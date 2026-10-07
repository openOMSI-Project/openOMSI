//! The small navigator's panel as the player shapes it: dragged by its corners and edges to
//! any size and shape and by the rest of it to any place, and laid out for that shape - the
//! map takes the room there is, the duty board goes under it or (on a wide panel) beside it,
//! and every text, icon and padding is drawn at a scale that follows the panel's size. Also
//! the shapes the other vehicles are drawn as on the maps. All of it plain geometry, without
//! a window or a GPU.

use glam::Vec2;
use omsi_ui::Rect;

/// The panel as designed, in points (scale 1): its width, the header's and the next-stop
/// bar's heights, and the least the map keeps.
pub(crate) const WIDTH: f32 = 360.0;
pub(crate) const HEAD: f32 = 34.0;
pub(crate) const NEXT: f32 = 46.0;
pub(crate) const MAP_MIN: f32 = 130.0;
/// The duty board's height the scale counts on (the trip, three stops and what comes next):
/// the board itself takes as many rows as fit.
pub(crate) const BOARD_NOMINAL: f32 = 200.0;
/// Beside the map (a wide panel): the board's column at the least, and the least the map
/// keeps beside it.
pub(crate) const SIDE_BOARD: f32 = 330.0;
pub(crate) const SIDE_MAP: f32 = 250.0;
/// The board goes beside the map only when that draws everything this much larger.
const BESIDE_GAIN: f32 = 1.1;
/// The scale's range against the navigator's own: a panel dragged very small keeps its
/// texts readable, a huge one does not blow them up beyond this.
pub(crate) const SCALE_RANGE: (f32, f32) = (0.6, 2.6);
/// A panel larger than its own size draws its texts larger by this power of how much
/// larger it is (the square root and a little more): they grow with it, the map takes most
/// of the room.
const GROWTH: f32 = 0.6;
/// The smallest panel (points at the interface's size).
pub(crate) const MIN_SIZE: [f32; 2] = [220.0, 150.0];
/// The board's handle on the next-stop bar (points, square).
pub(crate) const HANDLE: f32 = 24.0;

/// The scale (pixels a point) a panel `w` x `h` pixels is drawn at, `base` being the
/// navigator's own (the interface's pixels a point times `nav_scale`), with the duty board or
/// without; and whether the board goes beside the map. Smaller than its own size, the largest
/// at which everything fits - the panel's width against the design's, its height against what
/// it has to hold, the map at its least; larger, that grown more slowly than the panel.
pub(crate) fn scale(w: f32, h: f32, base: f32, board: bool) -> (f32, bool) {
    let stacked = (w / WIDTH).min(h / (HEAD + MAP_MIN + NEXT + if board { BOARD_NOMINAL } else { 0.0 }));
    let beside = (w / (SIDE_MAP + SIDE_BOARD)).min(h / (HEAD + (MAP_MIN + NEXT).max(BOARD_NOMINAL)));
    let side = board && beside > stacked * BESIDE_GAIN;
    let fit = if side { beside } else { stacked };
    let base = base.max(1e-3);
    let x = fit / base;
    let s = if x > 1.0 { base * x.powf(GROWTH) } else { fit };
    (s.clamp(base * SCALE_RANGE.0, base * SCALE_RANGE.1), side)
}

/// How tall the duty board may be in a panel `h` pixels tall at scale `s`: under the map, what
/// the header, the bar and the map at its least leave; beside it, all under the header.
pub(crate) fn board_room(h: f32, s: f32, beside: bool) -> f32 {
    if beside {
        h - (HEAD * s).round()
    } else {
        h - (HEAD * s).round() - (NEXT * s).round() - MAP_MIN * s
    }
}

/// Where the small navigator's parts go (the panel's own pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Layout {
    pub s: f32,
    pub head: Rect,
    pub map: Rect,
    /// The next-stop bar (all of it, the handle's place included).
    pub next: Rect,
    pub board: Option<Rect>,
    /// The duty board's handle at the bar's right end: a press shows or hides the board.
    pub handle: Option<Rect>,
    pub beside: bool,
}

impl Layout {
    /// The bar's part for its texts (left of the handle).
    pub(crate) fn next_text(&self) -> Rect {
        match self.handle {
            Some(h) => Rect::new(self.next.x, self.next.y, (h.x - 4.0 * self.s - self.next.x).max(0.0), self.next.h),
            None => self.next,
        }
    }
}

/// The parts of a panel `w` x `h` at scale `s`: the header across the top; the map under it
/// taking what is left; the next-stop bar under the map; the duty board (`board_h` pixels
/// tall, None: no board) under the bar - or beside the map and the bar, from the header down,
/// when `beside`; the board's handle (`handle`) at the bar's right end.
pub(crate) fn layout(w: f32, h: f32, s: f32, board_h: Option<f32>, beside: bool, handle: bool) -> Layout {
    let head = Rect::new(0.0, 0.0, w, (HEAD * s).round());
    let bar = (NEXT * s).round();
    let (map, next, board) = match board_h {
        // (beside the map the board has the column's height, whatever its rows ask)
        Some(_) if beside => {
            let bw = (w * 0.45).clamp(SIDE_BOARD * s, 420.0 * s).min(w - SIDE_MAP * s).max(0.0).round();
            let left = w - bw;
            let next = Rect::new(0.0, h - bar, left, bar);
            let map = Rect::new(0.0, head.bottom(), left, (next.y - head.bottom()).max(0.0));
            (map, next, Some(Rect::new(left, head.bottom(), bw, h - head.bottom())))
        }
        Some(bh) => {
            let bh = bh.min((h - head.h - bar).max(0.0));
            let next = Rect::new(0.0, h - bh - bar, w, bar);
            let map = Rect::new(0.0, head.bottom(), w, (next.y - head.bottom()).max(0.0));
            (map, next, (bh > 0.0).then(|| Rect::new(0.0, h - bh, w, bh)))
        }
        None => {
            let next = Rect::new(0.0, h - bar, w, bar);
            (Rect::new(0.0, head.bottom(), w, (next.y - head.bottom()).max(0.0)), next, None)
        }
    };
    let handle = handle.then(|| {
        let d = (HANDLE * s).round();
        Rect::new(next.right() - 8.0 * s - d, (next.center().y - d * 0.5).round(), d, d)
    });
    Layout { s, head, map, next, board, handle, beside: beside && board.is_some() }
}

/// What a press on the panel takes hold of: the panel (to move it) or some of its edges (to
/// size it by them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grip {
    Move,
    Size { left: bool, right: bool, top: bool, bottom: bool },
}

impl Grip {
    /// The mouse cursor for it (as `App::set_cursor_kind` has them): the arrows along the
    /// edges it moves.
    pub(crate) fn cursor(self) -> u8 {
        match self {
            Grip::Move => 3,
            Grip::Size { left, right, top, bottom } => match ((left || right), (top || bottom)) {
                (true, true) if (left && top) || (right && bottom) => 6,
                (true, true) => 7,
                (true, false) => 5,
                _ => 4,
            },
        }
    }
}

/// What a press at (`x`, `y`) takes hold of on a panel `rect` (x0, y0, x1, y1): an edge within
/// `edge` pixels of it, a corner - both its edges - within twice that along either, else the
/// panel. None: not on it.
pub(crate) fn grip_at(rect: [f32; 4], x: f32, y: f32, edge: f32) -> Option<Grip> {
    if !(x >= rect[0] && y >= rect[1] && x < rect[2] && y < rect[3]) {
        return None;
    }
    let corner = edge * 2.5;
    let (dl, dr, dt, db) = (x - rect[0], rect[2] - x, y - rect[1], rect[3] - y);
    let (mut left, mut right, mut top, mut bottom) = (dl < edge, dr < edge, dt < edge, db < edge);
    if left || right {
        top |= dt < corner;
        bottom |= db < corner;
    }
    if top || bottom {
        left |= dl < corner;
        right |= dr < corner;
    }
    // (a panel narrower than both edges' zones: the nearer edge)
    if left && right {
        (left, right) = (dl <= dr, dl > dr);
    }
    if top && bottom {
        (top, bottom) = (dt <= db, dt > db);
    }
    Some(if left || right || top || bottom { Grip::Size { left, right, top, bottom } } else { Grip::Move })
}

/// The panel `start` (x0, y0, x1, y1) with the edges `grip` holds moved by (`dx`, `dy`): never
/// smaller than `min`, never out of the window `window` (w, h); the other edges stay.
pub(crate) fn resized(start: [f32; 4], grip: Grip, dx: f32, dy: f32, min: [f32; 2], window: [f32; 2]) -> [f32; 4] {
    let Grip::Size { left, right, top, bottom } = grip else { return start };
    let mut r = start;
    let (min_w, min_h) = (min[0].min(window[0]), min[1].min(window[1]));
    if left {
        r[0] = (start[0] + dx).clamp(0.0, (start[2] - min_w).max(0.0));
    }
    if right {
        r[2] = (start[2] + dx).clamp((start[0] + min_w).min(window[0]), window[0]);
    }
    if top {
        r[1] = (start[1] + dy).clamp(0.0, (start[3] - min_h).max(0.0));
    }
    if bottom {
        r[3] = (start[3] + dy).clamp((start[1] + min_h).min(window[1]), window[1]);
    }
    r
}

/// A panel `w` x `h` kept between `min` and the window (`window`, w and h).
pub(crate) fn clamp_size(w: f32, h: f32, min: [f32; 2], window: [f32; 2]) -> (f32, f32) {
    (w.clamp(min[0].min(window[0]), window[0].max(1.0)).round(), h.clamp(min[1].min(window[1]), window[1].max(1.0)).round())
}

/// Where a panel `w` x `h` at `at` (a share of the room the window leaves it across and down)
/// has its top-left corner, and back: the share for a top-left corner at `p`.
pub(crate) fn place(at: [f32; 2], w: f32, h: f32, window: [f32; 2]) -> Vec2 {
    let room = Vec2::new((window[0] - w).max(0.0), (window[1] - h).max(0.0));
    Vec2::new((at[0].clamp(0.0, 1.0) * room.x).round(), (at[1].clamp(0.0, 1.0) * room.y).round())
}

pub(crate) fn share(p: Vec2, w: f32, h: f32, window: [f32; 2]) -> [f32; 2] {
    let share = |p: f32, r: f32| if r > 0.5 { (p / r).clamp(0.0, 1.0) } else { 0.0 };
    [share(p.x, window[0] - w), share(p.y, window[1] - h)]
}

// --- the other vehicles on the maps ---------------------------------------------------------

/// What a vehicle on the map is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyph {
    /// Its shape from above, at its own size or larger.
    Shape,
    /// A dot with a tip towards where it heads: the shape would be too small to read.
    Dot,
}

/// Seen from above, a vehicle `len_m` long is drawn as its shape - at least `min_px` long -
/// while that is not more than twice its size at `mpp` metres a pixel; further out, as a dot.
pub(crate) fn glyph(len_m: f32, mpp: f32, min_px: f32) -> Glyph {
    let true_px = len_m / mpp.max(1e-4);
    if true_px * 2.0 >= min_px { Glyph::Shape } else { Glyph::Dot }
}

/// How a shape is coloured: the vehicle's own colour, its windows, its roof, the joint of an
/// articulated one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Body,
    Glass,
    Roof,
    Joint,
}

/// A vehicle `len` x `width` metres from above, as convex outlines in metres about its middle
/// (x to its right, y ahead): a bus (or tram) with its windscreen, rear window and roof - two
/// joints where it is articulated - or a car with its windscreen and rear window.
pub(crate) fn vehicle_shape(len: f32, width: f32, bus: bool) -> Vec<(Tone, Vec<Vec2>)> {
    let (l, w) = (len.max(1.0) * 0.5, width.max(0.6) * 0.5);
    let body = |c_front: f32, c_rear: f32| {
        vec![
            Vec2::new(-w, -l + c_rear),
            Vec2::new(-w + c_rear, -l),
            Vec2::new(w - c_rear, -l),
            Vec2::new(w, -l + c_rear),
            Vec2::new(w, l - c_front),
            Vec2::new(w - c_front, l),
            Vec2::new(-w + c_front, l),
            Vec2::new(-w, l - c_front),
        ]
    };
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32| vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)];
    let mut out = Vec::new();
    if bus {
        let c = (w * 0.35).min(0.45);
        out.push((Tone::Body, body(c * 1.4, c)));
        let inset = (w * 0.22).min(0.3);
        // the windscreen, the rear window, the roof's hatches between
        out.push((Tone::Glass, quad(-w + inset, w - inset, l - 1.25f32.min(l * 0.12), l - 0.3f32.min(l * 0.04))));
        out.push((Tone::Glass, quad(-w + inset * 1.4, w - inset * 1.4, -l + 0.25f32.min(l * 0.03), -l + 0.75f32.min(l * 0.07))));
        out.push((Tone::Roof, quad(-w * 0.42, w * 0.42, -l + 1.6f32.min(l * 0.2), l - 2.2f32.min(l * 0.25))));
        // (an articulated bus or a tram: its joints, every 9 m or so)
        let parts = ((len / 9.5).round() as i32).clamp(1, 5);
        if len > 15.0 {
            for k in 1..parts {
                let y = -l + len * k as f32 / parts as f32;
                out.push((Tone::Joint, quad(-w, w, y - 0.35, y + 0.35)));
            }
        }
    } else {
        let c = w * 0.45;
        out.push((Tone::Body, body(c, c * 0.8)));
        let inset = w * 0.16;
        out.push((Tone::Glass, quad(-w + inset, w - inset, l * 0.12, l * 0.42)));
        out.push((Tone::Glass, quad(-w + inset * 1.3, w - inset * 1.3, -l * 0.78, -l * 0.55)));
    }
    out
}

/// A shape grown outwards by `d` (for its dark outline): each point moved away from the
/// middle, as far along each axis.
pub(crate) fn grown(shape: &[Vec2], d: f32) -> Vec<Vec2> {
    shape.iter().map(|p| *p + Vec2::new(p.x.signum() * d, p.y.signum() * d)).collect()
}

/// A shape turned to the compass heading `deg` (0 north, clockwise) in the world's x east,
/// y north.
pub(crate) fn turned(shape: &[Vec2], deg: f32) -> Vec<Vec2> {
    let (sn, cs) = deg.to_radians().sin_cos();
    // (ahead is (sin, cos), to the right (cos, -sin))
    shape.iter().map(|p| Vec2::new(p.x * cs + p.y * sn, -p.x * sn + p.y * cs)).collect()
}

/// The dot a vehicle is when its shape would be too small: a disc of radius 1 with a tip
/// ahead (y), for scaling to pixels.
pub(crate) fn dot_shape() -> (Vec<Vec2>, Vec<Vec2>) {
    let disc = (0..14).map(|k| {
        let a = std::f32::consts::TAU * k as f32 / 14.0;
        Vec2::new(a.cos(), a.sin())
    });
    (disc.collect(), vec![Vec2::new(0.0, 1.85), Vec2::new(-0.78, 0.62), Vec2::new(0.78, 0.62)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's own panel is drawn at its size; a larger one larger, more slowly than it
    /// grows; a tall one by its width, a short wide one by its height - and with the board,
    /// beside the map once that draws it larger; never beyond the scale's range.
    #[test]
    fn the_scale_follows_the_panels_shape() {
        let (s, side) = scale(360.0, HEAD + MAP_MIN + NEXT + BOARD_NOMINAL, 1.0, true);
        assert_eq!((s, side), (1.0, false));
        let twice = scale(720.0, 2.0 * (HEAD + MAP_MIN + NEXT + BOARD_NOMINAL), 1.0, true).0;
        assert!((twice - 2f32.powf(GROWTH)).abs() < 1e-4, "{twice}");
        // (the navigator's own size made larger: so are its texts, as far as they fit)
        assert_eq!(scale(720.0, 2.0 * (HEAD + MAP_MIN + NEXT + BOARD_NOMINAL), 2.0, true).0, 2.0);
        assert_eq!(scale(360.0, HEAD + MAP_MIN + NEXT + BOARD_NOMINAL, 1.5, true).0, 1.0, "no larger than fits");
        // tall and narrow: by the width, the map takes the height
        assert_eq!(scale(300.0, 1200.0, 1.0, true), (300.0 / WIDTH, false));
        // wide and short: the board beside the map
        let (s, side) = scale(1000.0, 300.0, 1.0, true);
        let fit = 300.0 / (HEAD + (MAP_MIN + NEXT).max(BOARD_NOMINAL));
        assert!(side && (s - fit.powf(GROWTH)).abs() < 1e-4, "{s}");
        assert!(!scale(1000.0, 300.0, 1.0, false).1, "no board, nothing beside");
        // (square: under it)
        assert!(!scale(500.0, 500.0, 1.0, true).1);
        // the range
        assert_eq!(scale(50.0, 50.0, 1.5, true).0, 1.5 * SCALE_RANGE.0);
        assert_eq!(scale(5000.0, 5000.0, 1.0, false).0, SCALE_RANGE.1);
        // smooth: a pixel more is hardly larger
        let (a, _) = scale(400.0, 500.0, 1.0, true);
        let (b, _) = scale(401.0, 500.0, 1.0, true);
        assert!((b - a).abs() < 0.01);
    }

    /// The parts fill the panel without overlapping: the header, the map, the bar and the board
    /// stacked - or the board beside - the handle inside the bar, the bar's texts left of it.
    #[test]
    fn the_layout_fills_any_shape() {
        for (w, h, board, beside) in [(360.0, 546.0, Some(243.0), false), (300.0, 900.0, Some(200.0), false), (1000.0, 300.0, Some(266.0), true), (360.0, 303.0, None, false), (220.0, 150.0, None, false)] {
            let (s, _) = scale(w, h, 1.0, board.is_some());
            let l = layout(w, h, s, board, beside, board.is_some());
            assert_eq!(l.head.y, 0.0);
            assert_eq!(l.map.y, l.head.bottom());
            assert_eq!(l.next.y, l.map.bottom());
            assert!(l.map.h >= 0.0 && l.map.w > 0.0);
            match l.board {
                Some(b) if beside => {
                    assert_eq!((b.x, b.y, b.right(), b.bottom()), (l.map.right(), l.head.bottom(), w, h));
                    assert_eq!(l.next.bottom(), h);
                }
                Some(b) => {
                    assert_eq!((b.y, b.bottom(), b.w), (l.next.bottom(), h, w));
                }
                None => assert_eq!(l.next.bottom(), h),
            }
            if let Some(hd) = l.handle {
                assert!(hd.x > l.next.x && hd.right() <= l.next.right() && hd.y >= l.next.y && hd.bottom() <= l.next.bottom() + 0.5);
                assert!(l.next_text().right() < hd.x);
            }
        }
        // (the design's own: the map 0.62 of the width tall, as ever)
        let l = layout(360.0, 34.0 + 223.0 + 46.0 + 243.0, 1.0, Some(243.0), false, true);
        assert_eq!(l.map.h, 223.0);
    }

    /// Edges and corners size the panel, the rest of it moves it; a corner's zone reaches
    /// further along its edges than an edge's.
    #[test]
    fn a_press_finds_the_edge_or_the_panel() {
        let r = [100.0, 100.0, 500.0, 400.0];
        assert_eq!(grip_at(r, 50.0, 200.0, 8.0), None);
        assert_eq!(grip_at(r, 300.0, 250.0, 8.0), Some(Grip::Move));
        assert_eq!(grip_at(r, 103.0, 250.0, 8.0), Some(Grip::Size { left: true, right: false, top: false, bottom: false }));
        assert_eq!(grip_at(r, 300.0, 396.0, 8.0), Some(Grip::Size { left: false, right: false, top: false, bottom: true }));
        assert_eq!(grip_at(r, 497.0, 385.0, 8.0), Some(Grip::Size { left: false, right: true, top: false, bottom: true }), "the corner");
        assert_eq!(grip_at(r, 110.0, 103.0, 8.0), Some(Grip::Size { left: true, right: false, top: true, bottom: false }));
        assert_eq!(Grip::Size { left: true, right: false, top: true, bottom: false }.cursor(), 6);
        assert_eq!(Grip::Size { left: false, right: true, top: true, bottom: false }.cursor(), 7);
        assert_eq!(Grip::Size { left: true, right: false, top: false, bottom: false }.cursor(), 5);
        assert_eq!(Grip::Size { left: false, right: false, top: false, bottom: true }.cursor(), 4);
    }

    /// Sizing by an edge moves that edge alone, held to the smallest size and the window.
    #[test]
    fn sizing_keeps_to_the_limits() {
        let r = [100.0, 100.0, 500.0, 400.0];
        let br = Grip::Size { left: false, right: true, top: false, bottom: true };
        assert_eq!(resized(r, br, 50.0, -20.0, [220.0, 150.0], [1920.0, 1080.0]), [100.0, 100.0, 550.0, 380.0]);
        assert_eq!(resized(r, br, -1000.0, -1000.0, [220.0, 150.0], [1920.0, 1080.0]), [100.0, 100.0, 320.0, 250.0]);
        assert_eq!(resized(r, br, 5000.0, 5000.0, [220.0, 150.0], [1920.0, 1080.0]), [100.0, 100.0, 1920.0, 1080.0]);
        let tl = Grip::Size { left: true, right: false, top: true, bottom: false };
        assert_eq!(resized(r, tl, -500.0, 40.0, [220.0, 150.0], [1920.0, 1080.0]), [0.0, 140.0, 500.0, 400.0]);
        assert_eq!(resized(r, Grip::Move, 10.0, 10.0, [220.0, 150.0], [1920.0, 1080.0]), r);
        assert_eq!(clamp_size(10.0, 5000.0, [220.0, 150.0], [1920.0, 1080.0]), (220.0, 1080.0));
        // a place and its share of the room, both ways
        let p = place([0.5, 1.0], 400.0, 300.0, [1920.0, 1080.0]);
        assert_eq!(p, Vec2::new(760.0, 780.0));
        assert_eq!(share(p, 400.0, 300.0, [1920.0, 1080.0]), [0.5, 1.0]);
    }

    /// A vehicle is its shape near, a dot far out; the shapes stay convex and inside their
    /// size, turned to where the vehicle heads.
    #[test]
    fn vehicles_are_shapes_until_too_small() {
        assert_eq!(glyph(12.0, 0.5, 14.0), Glyph::Shape);
        assert_eq!(glyph(12.0, 1.5, 14.0), Glyph::Shape, "a little enlarged");
        assert_eq!(glyph(12.0, 3.0, 14.0), Glyph::Dot);
        assert_eq!(glyph(4.4, 0.5, 9.0), Glyph::Shape);
        assert_eq!(glyph(4.4, 1.2, 9.0), Glyph::Dot);
        for (len, w, bus) in [(12.0, 2.55, true), (18.0, 2.55, true), (30.0, 2.4, true), (4.4, 1.8, false)] {
            let parts = vehicle_shape(len, w, bus);
            assert_eq!(parts[0].0, Tone::Body);
            assert_eq!(parts.iter().filter(|p| p.0 == Tone::Joint).count(), [0, 0, 1, 2][[12.0, 4.4, 18.0, 30.0].iter().position(|l| *l == len).unwrap()], "{len}");
            for (_, pts) in &parts {
                for p in pts {
                    assert!(p.x.abs() <= w * 0.5 + 1e-4 && p.y.abs() <= len * 0.5 + 1e-4, "{p} in {len} x {w}");
                }
                // convex, one way round
                let n = pts.len();
                let turns: Vec<f32> = (0..n).map(|i| (pts[(i + 1) % n] - pts[i]).perp_dot(pts[(i + 2) % n] - pts[(i + 1) % n])).collect();
                assert!(turns.iter().all(|t| *t >= -1e-4) || turns.iter().all(|t| *t <= 1e-4), "{pts:?}");
            }
        }
        // heading east: ahead is +x
        let t = turned(&[Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0)], 90.0);
        assert!((t[0] - Vec2::new(1.0, 0.0)).length() < 1e-5 && (t[1] - Vec2::new(0.0, -1.0)).length() < 1e-5, "{t:?}");
        assert!(grown(&[Vec2::new(1.0, -2.0)], 0.5)[0] == Vec2::new(1.5, -2.5));
    }
}
