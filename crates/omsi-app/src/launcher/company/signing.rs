//! Signing a contract (Luc's wish): before a contract is booked, the paper comes into view, a
//! fountain pen goes over its signature line and the ink of the signature draws itself behind
//! its nib - the signature drawn in the field, or one written from the name typed - and the
//! paper is stamped "Signed". About two seconds; a click or Escape skips to its end; with the
//! setting "Animations" off the signed paper is shown at once.
//!
//! The signature's path is plain maths (`path_of`, `scribble`, `along`): strokes in a unit
//! box, walked by their length.

use super::super::theme::*;
use super::super::ui::Key;
use super::super::Launcher;
use super::kit;
use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// Seconds the whole takes, and when its parts are: the paper in, the pen writing, the pen
/// away, the stamp.
pub(super) const LENGTH: f32 = 2.3;
const PAPER_IN: f32 = 0.3;
const WRITE: (f32, f32) = (0.3, 1.55);
const STAMP: f32 = 1.65;
/// With the animations off, how long the signed paper is shown.
const STILL: f32 = 0.8;

/// A signing under way: when it began, the signature's strokes (a unit box), whose it is.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Signing {
    pub start: f32,
    pub strokes: Vec<Vec<Vec2>>,
    pub name: String,
    pub title: String,
}

impl Signing {
    /// A signing beginning now, of the strokes drawn in the field (0..1 of it) or else the
    /// name typed.
    pub fn new(now: f32, drawn: &[Vec<[f32; 2]>], name: &str, title: &str) -> Signing {
        Signing { start: now, strokes: path_of(drawn, name), name: name.trim().to_string(), title: title.to_string() }
    }
}

/// What a frame of the signing came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum State {
    Running,
    /// Over (or skipped): the contract is to be booked now.
    Done,
}

/// The signature's strokes in a unit box: the strokes drawn (their box filled, its shape
/// kept), else a scribble of the name.
pub(super) fn path_of(drawn: &[Vec<[f32; 2]>], name: &str) -> Vec<Vec<Vec2>> {
    let strokes: Vec<Vec<Vec2>> = drawn.iter().filter(|s| s.len() > 1).map(|s| s.iter().map(|p| Vec2::new(p[0], p[1])).collect()).collect();
    if strokes.is_empty() {
        return scribble(name);
    }
    // (into the unit box, its shape kept, in the middle of its height)
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in strokes.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let span = (hi - lo).max(Vec2::splat(1e-3));
    let k = (1.0 / span.x).min(0.6 / span.y);
    let off = Vec2::new(0.0, (1.0 - span.y * k) * 0.5);
    strokes.into_iter().map(|s| s.into_iter().map(|p| (p - lo) * k + off).collect()).collect()
}

/// A flowing scribble of a name, as a hand writes it fast: a loop or a hump a letter (its
/// height and width from the letter), joined in one stroke, a flourish under it. The same name
/// gives the same signature.
pub(super) fn scribble(name: &str) -> Vec<Vec<Vec2>> {
    let letters: Vec<char> = name.chars().filter(|c| c.is_alphanumeric()).take(14).collect();
    let letters = if letters.is_empty() { "Signed".chars().collect() } else { letters };
    let n = letters.len() as f32;
    let mut pts: Vec<Vec2> = Vec::new();
    let mut x = 0.02;
    let step = 0.9 / n;
    for (k, ch) in letters.iter().enumerate() {
        let code = *ch as u32;
        let tall = ch.is_uppercase() || "bdfhklt".contains(ch.to_ascii_lowercase()) || k == 0;
        let deep = "gjpqy".contains(ch.to_ascii_lowercase());
        let top = if tall { 0.1 + (code % 3) as f32 * 0.03 } else { 0.32 + (code % 5) as f32 * 0.02 };
        let bottom = 0.62;
        let w = step * (0.8 + (code % 4) as f32 * 0.1);
        // a hump up to the letter's top and down, with a little loop for the tall ones
        let samples = 20;
        for i in 0..=samples {
            let t = i as f32 / samples as f32;
            let a = t * std::f32::consts::TAU;
            let lean = 0.18 * (0.5 - t);
            // (a hump up to the letter's top; a tall one loops a little, a deep one dips below
            // the line after it)
            let tail = if deep && t > 0.5 { 0.22 * (std::f32::consts::TAU * (t - 0.5)).sin().max(0.0) } else { 0.0 };
            let y = bottom - (bottom - top) * (0.5 - 0.5 * a.cos()) + if tall { 0.06 * (2.0 * a).sin() } else { 0.0 } + tail;
            pts.push(Vec2::new(x + w * t + lean * (bottom - y), y));
        }
        x += w * 0.92;
    }
    // squeezed into the box's width
    let right = pts.iter().map(|p| p.x).fold(0.0, f32::max).max(0.1);
    for p in pts.iter_mut() {
        p.x = 0.02 + (p.x - 0.02) * 0.94 / right;
    }
    // the flourish: from the last letter back under the name
    let end = *pts.last().unwrap_or(&Vec2::new(0.9, 0.6));
    let mut flourish = vec![end];
    for i in 1..=28 {
        let t = i as f32 / 28.0;
        // (down from the last letter to under the name, gently, then along it)
        let down = (t * 4.0).min(1.0);
        let down = down * down * (3.0 - 2.0 * down);
        flourish.push(Vec2::new(end.x - (end.x - 0.08) * t, end.y + (0.82 - end.y) * down + 0.06 * (t * std::f32::consts::PI).sin()));
    }
    vec![pts, flourish]
}

/// The strokes' length in all.
pub(super) fn length(strokes: &[Vec<Vec2>]) -> f32 {
    strokes.iter().map(|s| s.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f32>()).sum()
}

/// The strokes as far as `share` (0..1) of their length, and where the pen is there.
pub(super) fn along(strokes: &[Vec<Vec2>], share: f32) -> (Vec<Vec<Vec2>>, Vec2) {
    let total = length(strokes);
    let mut left = total * share.clamp(0.0, 1.0);
    let mut out: Vec<Vec<Vec2>> = Vec::new();
    let mut tip = strokes.first().and_then(|s| s.first()).copied().unwrap_or(Vec2::ZERO);
    for s in strokes {
        let Some(first) = s.first() else { continue };
        let mut part = vec![*first];
        tip = *first;
        for w in s.windows(2) {
            let d = (w[1] - w[0]).length();
            if left >= d {
                left -= d;
                part.push(w[1]);
                tip = w[1];
            } else {
                let p = w[0] + (w[1] - w[0]) * (left / d.max(1e-6));
                part.push(p);
                tip = p;
                left = 0.0;
                break;
            }
        }
        out.push(part);
        if left <= 0.0 {
            break;
        }
    }
    (out, tip)
}

fn ease(t: f32) -> f32 {
    super::super::ui::ease_in_out_cubic(t.clamp(0.0, 1.0))
}

/// A point turned by `a` about `c`.
fn turn(p: Vec2, c: Vec2, a: f32) -> Vec2 {
    let (s, co) = a.sin_cos();
    let d = p - c;
    c + Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co)
}

/// A rectangle turned by `a` about its middle, as a polygon.
fn turned(r: Rect, a: f32) -> [Vec2; 4] {
    turned_about(r, r.center(), a)
}

/// A rectangle turned by `a` about `c`, as a polygon.
fn turned_about(r: Rect, c: Vec2, a: f32) -> [Vec2; 4] {
    [turn(Vec2::new(r.x, r.y), c, a), turn(Vec2::new(r.right(), r.y), c, a), turn(Vec2::new(r.right(), r.bottom()), c, a), turn(Vec2::new(r.x, r.bottom()), c, a)]
}

/// Words on the turned paper: `left` the middle of their left end as the paper lies flat,
/// `c` the paper's middle it is turned about.
#[allow(clippy::too_many_arguments)]
fn words(l: &mut Launcher, text: &str, px: f32, weight: Weight, left: Vec2, c: Vec2, angle: f32, colour: Color) {
    let w = l.ui.width(text, px, weight);
    l.ui.text_rotated(text, px, weight, turn(left + Vec2::new(w * 0.5, 0.0), c, angle), angle, colour);
}

/// The fountain pen with its nib at `tip`, leaning `lean` (radians) to the right: its soft
/// shadow, barrel, cap ring and nib.
fn pen(l: &mut Launcher, tip: Vec2, lean: f32, lift: f32) {
    let up = Vec2::new(lean.sin(), -lean.cos());
    let side = Vec2::new(-up.y, up.x);
    let at = |along: f32, across: f32| tip + up * along + side * across;
    // the shadow on the paper, off to the lower right as the pen lifts
    let sh = Vec2::new(10.0 + 14.0 * lift, 12.0 + 10.0 * lift);
    let shadow = [at(14.0, -6.0) + sh, at(14.0, 6.0) + sh, at(150.0, 8.0) + sh, at(150.0, -8.0) + sh];
    l.ui.p().convex(&shadow, Color::rgba(0, 0, 0, 0.16 - 0.06 * lift));
    // the nib: a pointed steel plate with its slit
    let nib = [at(0.0, 0.0), at(16.0, -5.0), at(24.0, -6.0), at(24.0, 6.0), at(16.0, 5.0)];
    l.ui.p().convex(&nib, Color::rgba(200, 205, 214, 1.0));
    l.ui.p().line(at(2.0, 0.0), at(15.0, 0.0), 1.0, Color::rgba(60, 64, 72, 1.0));
    // the grip and the barrel, the barrel's light along one side
    let grip = [at(24.0, -6.5), at(24.0, 6.5), at(46.0, 7.5), at(46.0, -7.5)];
    l.ui.p().convex(&grip, Color::rgba(28, 30, 38, 1.0));
    let barrel = [at(46.0, -8.5), at(46.0, 8.5), at(150.0, 8.5), at(150.0, -8.5)];
    l.ui.p().convex(&barrel, Color::rgba(22, 34, 64, 1.0));
    let light = [at(50.0, -6.0), at(50.0, -3.0), at(146.0, -3.0), at(146.0, -6.0)];
    l.ui.p().convex(&light, Color::rgba(255, 255, 255, 0.14));
    let ring = [at(46.0, -8.8), at(46.0, 8.8), at(52.0, 8.8), at(52.0, -8.8)];
    l.ui.p().convex(&ring, Color::rgba(214, 178, 96, 1.0));
    l.ui.p().circle(at(150.0, 0.0), 8.5, Color::rgba(22, 34, 64, 1.0));
}

/// A frame of the signing over the contract's dialog. Returns whether it is over.
pub(super) fn draw(l: &mut Launcher, s: &Signing) -> State {
    let motion = l.ui.motion;
    let t = l.ui.time - s.start;
    let skip = l.ui.input.pressed || l.ui.input.keys.contains(&Key::Escape) || l.ui.input.keys.contains(&Key::Enter);
    let end = if motion { LENGTH } else { STILL };
    if t >= end || (skip && t > 0.05) {
        return State::Done;
    }
    l.ui.keep_moving();
    let t = if motion { t } else { LENGTH };
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, Color::rgba(0, 0, 0, 0.5 * ease(t / PAPER_IN)));
    // the paper: in from below, a little turned, settling
    let pw = 560.0f32.min(size.x - 40.0);
    let ph = pw * 0.62;
    let come = ease(t / PAPER_IN);
    let rustle = if motion { (t * 9.0).sin() * 0.006 * (1.0 - (t / 1.2).min(1.0)) } else { 0.0 };
    let angle = -0.035 + 0.02 * come + rustle;
    let paper = Rect::new((size.x - pw) * 0.5, (size.y - ph) * 0.5 + 60.0 * (1.0 - come), pw, ph);
    let c = paper.center();
    let shadow = turned(Rect::new(paper.x + 6.0, paper.y + 10.0, paper.w, paper.h), angle);
    l.ui.p().convex(&shadow, Color::rgba(0, 0, 0, 0.35 * come));
    l.ui.p().convex(&turned(paper, angle), Color::rgba(247, 244, 236, come));
    // its words: the title and lines of text, a signature line
    let ink = Color::rgba(40, 44, 56, 0.9 * come);
    words(l, &s.title, kit::HEAD, Weight::Bold, Vec2::new(paper.x + 34.0, paper.y + 38.0), c, angle, ink);
    for k in 0..6 {
        let w = paper.w - 68.0 - if k == 5 { 160.0 } else { (k % 3) as f32 * 30.0 };
        let y = paper.y + 70.0 + k as f32 * 18.0;
        l.ui.p().convex(&turned_about(Rect::new(paper.x + 34.0, y, w, 4.0), c, angle), Color::rgba(60, 64, 80, 0.16 * come));
    }
    let line_y = paper.bottom() - 70.0;
    let sig = Rect::new(paper.x + 34.0, line_y - 70.0, paper.w * 0.56, 76.0);
    let a = turn(Vec2::new(sig.x, line_y), c, angle);
    let b = turn(Vec2::new(sig.right(), line_y), c, angle);
    l.ui.p().line(a, b, 1.2, Color::rgba(60, 64, 80, 0.5 * come));
    words(l, &omsi_ui::tr("Signed for the company"), 12.0, Weight::Medium, Vec2::new(sig.x, line_y + 42.0), c, angle, Color::rgba(60, 64, 80, 0.7 * come));
    // the ink, as far as the pen has come
    let w = ((t - WRITE.0) / (WRITE.1 - WRITE.0)).clamp(0.0, 1.0);
    let share = ease(w);
    let (done, tip) = along(&s.strokes, share);
    let map = |p: Vec2| turn(Vec2::new(sig.x + p.x * sig.w, sig.y + p.y * sig.h), c, angle);
    let settle = ((t - WRITE.1) / 0.4).clamp(0.0, 1.0);
    let ink_c = Color::rgba(22, 46, 120, 0.82 + 0.18 * settle);
    for stroke in &done {
        let pts: Vec<Vec2> = stroke.iter().map(|p| map(*p)).collect();
        if pts.len() > 1 {
            l.ui.p().stroke(&pts, 2.4, ink_c);
        }
    }
    // the pen: down while writing, then up and away
    let away = ((t - WRITE.1) / 0.35).clamp(0.0, 1.0);
    if t > PAPER_IN * 0.6 && away < 1.0 {
        let start = map(s.strokes.first().and_then(|x| x.first()).copied().unwrap_or(Vec2::ZERO));
        let at = if w <= 0.0 { start + Vec2::new(30.0, -40.0) * (1.0 - ((t - PAPER_IN * 0.6) / (WRITE.0 - PAPER_IN * 0.6)).clamp(0.0, 1.0)) } else { map(tip) };
        let lift = ease(away);
        let at = at + Vec2::new(60.0, -90.0) * lift;
        // (the nib trembles a little as it writes)
        let jitter = if motion && w > 0.0 && w < 1.0 { Vec2::new((t * 63.0).sin(), (t * 71.0).cos()) * 0.6 } else { Vec2::ZERO };
        pen(l, at + jitter, 0.62 + 0.05 * (t * 5.0).sin() * (1.0 - lift), lift);
    }
    // the stamp
    let st = ((t - STAMP) / 0.25).clamp(0.0, 1.0);
    if st > 0.0 {
        let k = 1.0 + 0.35 * (1.0 - ease(st));
        let sw = 170.0 * k;
        let sh = 56.0 * k;
        let centre = turn(Vec2::new(paper.right() - 120.0, line_y - 34.0), c, angle);
        let r = Rect::new(centre.x - sw * 0.5, centre.y - sh * 0.5, sw, sh);
        let green = Color::rgba(26, 138, 79, 0.85 * st);
        l.ui.p().rounded_border(r, 10.0, 3.0, green);
        l.ui.icon("check_circle", Vec2::new(r.x + 30.0 * k, r.center().y), 24.0 * k, green);
        l.ui.text_in(&omsi_ui::tr("Signed").to_uppercase(), Rect::new(r.x + 46.0 * k, r.y, r.w - 52.0 * k, r.h), 22.0 * k, Weight::Black, green, Align::Center);
    }
    if !s.name.is_empty() {
        words(l, &s.name, 13.5, Weight::Bold, Vec2::new(sig.x, line_y + 20.0), c, angle, Color::rgba(40, 44, 56, 0.85 * come));
    }
    l.ui.text_in(&omsi_ui::tr("Click to skip"), Rect::new(0.0, size.y - 64.0, size.x, 24.0), kit::NOTE, Weight::Medium, TEXT_DIM.alpha(come), Align::Center);
    State::Running
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(strokes: &[Vec<Vec2>]) -> bool {
        strokes.iter().flatten().all(|p| (-0.01..=1.01).contains(&p.x) && (-0.01..=1.01).contains(&p.y))
    }

    #[test]
    fn a_name_is_written_as_a_flowing_scribble() {
        let a = scribble("Luc Ruigrok");
        assert_eq!(a, scribble("Luc Ruigrok"), "the same name, the same signature");
        assert_ne!(a, scribble("Joost Wagner"));
        assert!(inside(&a));
        assert_eq!(a.len(), 2, "the name and its flourish");
        // a longer name writes more, yet stays in the box; no name still signs
        assert!(length(&scribble("Maximiliane Oberhuber")) > length(&scribble("Al")));
        assert!(inside(&scribble("Maximiliane Oberhuber")) && !scribble("").is_empty());
        // the strokes are smooth: no jump longer than a letter
        for s in &a {
            let worst = s.windows(2).map(|w| (w[1] - w[0]).length()).fold(0.0, f32::max);
            assert!(worst < 0.2, "{worst} {:?}", s.windows(2).find(|w| (w[1] - w[0]).length() >= 0.2));
        }
    }

    #[test]
    fn a_drawn_signature_fills_the_box_and_keeps_its_shape() {
        let drawn = vec![vec![[0.2, 0.4], [0.4, 0.5]], vec![[0.4, 0.5], [0.6, 0.45]], vec![[0.9, 0.9]]];
        let s = path_of(&drawn, "ignored");
        assert_eq!(s.len(), 2, "a dot is no stroke");
        assert!(inside(&s));
        let xs: Vec<f32> = s.iter().flatten().map(|p| p.x).collect();
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min).abs() < 1e-4 && (xs.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-4);
        // nothing drawn: the name's scribble
        assert_eq!(path_of(&[], "Luc"), scribble("Luc"));
    }

    #[test]
    fn the_pen_walks_the_strokes_by_their_length() {
        let s = vec![vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0)], vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0)]];
        assert!((length(&s) - 2.0).abs() < 1e-6);
        let (part, tip) = along(&s, 0.25);
        assert_eq!(part.len(), 1);
        assert!((tip - Vec2::new(0.5, 0.0)).length() < 1e-5);
        let (part, tip) = along(&s, 0.75);
        assert_eq!(part.len(), 2);
        assert!((tip - Vec2::new(0.5, 1.0)).length() < 1e-5);
        let (all, tip) = along(&s, 1.0);
        assert_eq!(all, s);
        assert!((tip - Vec2::new(1.0, 1.0)).length() < 1e-5);
        assert!(along(&[], 0.5).0.is_empty());
    }
}
