//! The opening: when the launcher opens, openOMSI's mark draws itself - a blue ring round a bus
//! that opens into the route under the name, as a bus loop opens into the line. The pen draws
//! the ring from its open end round to its foot, the bus front springs up inside it once it has
//! closed, the pen runs on along the line and the stops pop up as it reaches them, "open" rises
//! letter by letter as the pen passes under it and "OMSI" lands along its slant, the terminus
//! rings when the pen arrives, and the mark settles and fades as the start page comes up under
//! it. About two and a half seconds, once per launch (not again when the launcher wakes after a
//! game), skipped by a click or any key (the page then comes up quickly), and not at all with
//! the setting "animations" off.
//!
//! The ring and the line are one flat blue stroke drawn up to a share of its length, a light at
//! the pen. Its timing is a pure function of the time since the first frame drawn (`look`) -
//! not since the process started: a debug build compiles its shaders for many seconds before
//! that.
//!
//! `OMSI_LAUNCHER_INTRO=1.2` holds the opening 1.2 s in, for pictures of it (a click or key
//! then skips it from there, as it would); `OMSI_LAUNCHER_INTRO=off` leaves it out.

use std::f32::consts::{PI, TAU};

use glam::Vec2;
use omsi_ui::paint::Path;
use omsi_ui::text::Style;
use omsi_ui::{Color, Fonts, Painter, Rect, Weight};

use super::theme::*;
use super::ui::{id_of, Input, Ui};

// The mark's colours, as the drawing has them.
/// The ring and the line.
/// "open" and the bus front; "OMSI".
const INK: Color = Color::rgba(236, 238, 242, 1.0);
const ORANGE: Color = Color::rgba(244, 134, 32, 1.0);
/// The stops: their white rims; the first one's red; the German stop's yellow and green; the
/// terminus's grey and the teal of its smile.
const RIM: Color = Color::rgba(252, 252, 254, 1.0);
const RED: Color = Color::rgba(229, 51, 52, 1.0);
const H_YELLOW: Color = Color::rgba(249, 215, 10, 1.0);
const H_GREEN: Color = Color::rgba(12, 121, 58, 1.0);
const BUS_GREY: Color = Color::rgba(68, 68, 70, 1.0);
const TEAL: Color = Color::rgba(1, 201, 190, 1.0);
/// The bus front's headlights when they flash.
const HEADLIGHT: Color = Color::rgba(255, 244, 200, 1.0);
/// The light at the pen: a white-blue point in a blue glow.
/// The route as the launcher's map draws it (`mapview`) under the stop sign the player looks
/// for in the game: a yellow disc, a green ring and a green H (`sign`).
const LINE_CASING: Color = Color::rgba(18, 58, 107, 1.0);
const SIGN: Color = Color::rgba(242, 194, 0, 1.0);
const SIGN_INK: Color = Color::rgba(10, 107, 61, 1.0);

/// The line's colour - the ring round the bus and the line under the name (Luc: in the
/// accent, orange unless another was chosen; it was the route's blue) - and its light as it
/// runs along: a pale tint of it, and its glow.
fn line_colour() -> Color {
    Color::hex(crate::accent::chosen())
}

fn pen() -> Color {
    line_colour().mix(Color::WHITE, 0.82)
}

fn pen_glow() -> Color {
    line_colour().mix(Color::WHITE, 0.35)
}

// The mark is laid out after the drawing it was made from, in that drawing's pixels (it is
// 2000 wide, its ring 174 round its middle); the mark is `RING_R` round at a 1440 x 900 window
// (`fit` sizes it to others).
const RING_R: f32 = 64.0;
const DRAWN_R: f32 = 174.0;
/// The ring's and the line's width; how far under the ring's middle the line runs (the ring
/// comes down to it flatter than round); where on the ring the pen sets down: this far
/// (radians) clockwise from its right - the ring is open between there and the line.
const STROKE: f32 = 46.5;
const FOOT: f32 = 150.0;
const RING_START: f32 = 0.32;
/// The stops' signs: the disc and its white rim's outer edge; the first stop this far right of
/// the ring's middle (the terminus stands under the end of the name, the H half way).
const SIGN_R: f32 = 55.0;
const RIM_R: f32 = 62.0;
const FIRST_STOP: f32 = 93.0;
/// The terminus: this far before the foot of the name's last letter.
const TERMINUS_BACK: f32 = 62.0;
/// "open": its capitals would be this tall (its small letters are 107), on a baseline this far
/// under the ring's middle, this far right of the ring's outer edge, its letters this much
/// closer than the face sets them; rounder than the face, as the drawing's (wider, and
/// thinned to its weight again).
const OPEN_CAP: f32 = 151.5;
const OPEN_BASE: f32 = 15.0;
const OPEN_GAP: f32 = 40.0;
const OPEN_TRACK: f32 = -5.0;
const OPEN_STYLE: Style = Style { slant: 0.0, bold: -0.006, stretch: 0.12 };
/// "OMSI": its capitals this tall, boldened and all, on a baseline a little lower than open's,
/// this much closer to it than the face would set it (the drawing has the words all but
/// touch), its letters this much further apart than the face sets them; leaning (the face has
/// no italic), wider and heavier than the face's heaviest.
const OMSI_CAP: f32 = 163.0;
const OMSI_BASE: f32 = 27.5;
const OMSI_CLOSER: f32 = 14.0;
const OMSI_TRACK: f32 = 3.0;
const OMSI_STYLE: Style = Style { slant: 0.27, bold: 0.025, stretch: 0.19 };
/// The bus front, round its middle (this far above the ring's): its body, rounded so much at
/// the top and at the foot; the destination band and the windscreen cut out of it (and how
/// round their corners are); the headlights; the wheels under it, the mirrors beside it.
const BUS_UP: f32 = 4.8;
const BODY: Rect = Rect::new(-73.5, -82.0, 147.0, 145.0);
const BODY_TOP: f32 = 22.0;
const BODY_FOOT: f32 = 9.0;
const BAND: Rect = Rect::new(-34.0, -72.0, 68.0, 13.0);
const BAND_ROUND: f32 = 4.5;
const SCREEN: Rect = Rect::new(-62.5, -50.0, 125.0, 69.0);
const SCREEN_ROUND: f32 = 9.0;
const LAMP_X: f32 = 48.25;
const LAMP_Y: f32 = 39.5;
const LAMP_R: f32 = 10.5;
/// (the square round each headlight the body is drawn in pieces of)
const LAMP_BOX: f32 = 14.0;
const WHEEL_X: f32 = 51.5;
const WHEEL: Rect = Rect::new(-11.5, 63.0, 23.0, 19.0);
const WHEEL_ROUND: f32 = 6.0;
const MIRROR_X: f32 = 85.0;
const MIRROR: Rect = Rect::new(-5.5, -47.0, 11.0, 37.0);
/// How high the bus front hops under the mouse.
const HOP_H: f32 = 13.0;

// The timeline, in seconds from the first frame drawn.
/// The ground's blue light comes up.
const GLOW_IN: f32 = 0.35;
/// The pen sets down, presses in, and takes this long over the ring and the whole line.
const DRAW_AT: f32 = 0.12;
const PEN_DOWN: f32 = 0.12;
const DRAW: f32 = 1.1;
/// A letter's rise (it starts as the pen passes under it, a little before), a stop's pop, the
/// bus front's, the ring going out from the terminus when the pen arrives.
const LETTER: f32 = 0.55;
const LETTER_LEAD: f32 = 0.03;
const POP: f32 = 0.45;
const BUS_POP: f32 = 0.55;
const PULSE: f32 = 0.7;
/// How far "OMSI"'s letters slide down their slant to land (of their capitals).
const SLIDE: f32 = 0.42;
/// The hand-over: the mark settles and fades, and a moment later the ground lifts off the page.
const LEAVE_AT: f32 = 1.95;
const MARK_OUT: f32 = 0.34;
const GROUND_DELAY: f32 = 0.14;
const GROUND_OUT: f32 = 0.55;
/// How much smaller the mark gets as it goes.
const SETTLE: f32 = 0.06;
/// Skipped: the same, quickly, from where it stood - the mark goes at once (eased out: it
/// answers the click), the ground a breath after it.
const QUICK_MARK: f32 = 0.12;
const QUICK_DELAY: f32 = 0.04;
const QUICK_GROUND: f32 = 0.26;
/// Below this the page answers the mouse again (it is plainly there).
const PASS_BELOW: f32 = 0.2;
/// The most the clock moves in one frame: a frame that came late (the first ones upload the
/// pictures) holds the line a moment rather than jumping it ahead.
const MAX_STEP: f32 = 0.05;

/// The whole opening, from the first frame to the page.
#[cfg_attr(not(test), allow(dead_code))]
const LENGTH: f32 = LEAVE_AT + GROUND_DELAY + GROUND_OUT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Not drawn yet: it starts with the first frame.
    Waiting,
    Playing,
    Done,
}

/// The quick way out (a click or a key): when it began, and how the mark and the ground stood.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Leave {
    at: f32,
    mark: f32,
    ground: f32,
    scale: f32,
}

pub struct Intro {
    state: State,
    clock: f32,
    leave: Option<Leave>,
    /// `OMSI_LAUNCHER_INTRO`: held at a moment (pictures), or left out.
    freeze: Option<f32>,
    off: bool,
    /// The mark, laid out once (in its own pixels; the window only scales it).
    mark: Option<Mark>,
}

impl Intro {
    pub fn new() -> Intro {
        let asked = omsi_cfg::env::var("OMSI_LAUNCHER_INTRO").ok();
        Intro {
            state: State::Waiting,
            clock: 0.0,
            leave: None,
            freeze: asked.as_deref().and_then(|v| v.trim().parse::<f32>().ok()),
            off: asked.is_some_and(|v| v.trim().eq_ignore_ascii_case("off")),
            mark: None,
        }
    }

    fn time(&self) -> f32 {
        self.freeze.unwrap_or(self.clock)
    }

    /// Before the page is drawn: the clock, a click or key that skips the opening, and the
    /// page's input taken away while the opening covers it (the page under it does not light
    /// up or take a click it cannot be seen to). Returns the input to put back once the page
    /// is drawn.
    pub fn begin(&mut self, ui: &mut Ui) -> Option<Input> {
        match self.state {
            State::Done => return None,
            // (the timeline starts at this first frame drawn)
            State::Waiting if self.off || !ui.motion => {
                self.state = State::Done;
                return None;
            }
            State::Waiting => {
                log::info!("launcher: the opening plays");
                self.state = State::Playing;
            }
            State::Playing if !ui.motion => {
                self.state = State::Done;
                return None;
            }
            State::Playing => self.clock += ui.dt.min(MAX_STEP),
        }
        let t = self.time();
        let (mark, ground, scale) = fade(t, self.leave);
        if ground <= 0.0 {
            log::info!("launcher: the opening is over after {t:.2} s{}", if self.leave.is_some() { " (skipped)" } else { "" });
            self.state = State::Done;
            return None;
        }
        if self.freeze.is_none() {
            ui.keep_moving();
        }
        if ground < PASS_BELOW {
            return None;
        }
        let i = &ui.input;
        if i.pressed || i.right_pressed || !i.keys.is_empty() || i.raw_key.is_some() || !i.text.is_empty() {
            // (held for a picture: it goes on from there as it would)
            if let Some(f) = self.freeze.take() {
                self.clock = f;
            }
            if self.leave.is_none() {
                self.leave = Some(Leave { at: t, mark, ground, scale });
            }
        }
        let real = ui.input.clone();
        let i = &mut ui.input;
        i.mouse = Vec2::new(-1e4, -1e4);
        (i.down, i.pressed, i.released, i.right_down, i.right_pressed, i.double_click) = (false, false, false, false, false, false);
        i.wheel = Vec2::ZERO;
        i.text.clear();
        i.keys.clear();
        i.raw_key = None;
        Some(real)
    }

    /// Over everything the launcher drew this frame.
    pub fn draw(&mut self, ui: &mut Ui) {
        if self.state != State::Playing {
            return;
        }
        let mark = self.mark.get_or_insert_with(|| Mark::new(&ui.fonts)).clone();
        let look = look(self.time(), self.leave, &mark.timing);
        let size = ui.size;
        let window = Rect::new(0.0, 0.0, size.x, size.y);
        let s = fit(size, mark.bounds);
        let k = s * look.scale;
        let centre = Vec2::new(size.x * 0.5, size.y * 0.48);
        let mid = mark.bounds.center();
        let at = |q: Vec2| centre + (q - mid) * k;
        // the ground, and a blue light on it round the mark (the blend is in linear light: the
        // opacities are turned into alphas that fade as evenly as they look - a dark ground at
        // four fifths showed the page half, a light mark at a third looked at half)
        let cover = 1.0 - (1.0 - look.ground).powf(2.2);
        ui.p().rect(window, GROUND.alpha(cover));
        // (faint: a tenth of blue lit the whole window)
        let light = accent().alpha(0.035 * look.glow * cover);
        ui.p().radial(centre, size.x.max(size.y) * 0.62, light, light.alpha(0.0));
        let a = look.mark.powf(2.2) * cover;
        if a <= 0.002 {
            return;
        }
        let u = mark.u;
        // the ring and the line, drawn up to the pen
        let route = mark.route(&at);
        let pen_at = route.length() * look.drawn;
        if look.drawn > 0.0 {
            ui.p().stroke(&route.part(0.0, pen_at), mark.stroke * k * look.pen, line_colour().alpha(a));
        }
        // the bus front, springing up once the ring has closed round it
        if look.bus > 0.0 {
            bus_front(ui.p(), at(mark.bus), u * k * look.bus, a, 0.0, INK);
        }
        // the stops, as the pen reaches them (each drawn at its size at rest, then only scaled);
        // the ring going out from the terminus
        for (stop, size) in mark.stops.iter().zip(look.stops) {
            if size > 0.0 {
                stop_sign(ui, stop.kind, at(stop.at), u * s, look.scale * size, a);
            }
        }
        if let (Some(p), Some(end)) = (look.pulse, mark.stops.last()) {
            pulse(ui.p(), at(end.at), u * k, p, a);
        }
        // the light at the pen
        if look.head > 0.0 && look.drawn > 0.0 {
            let (p, _) = route.at(pen_at);
            let h = look.head * a;
            ui.p().radial(p, mark.stroke * 1.75 * k, pen_glow().alpha(0.42 * h), pen_glow().alpha(0.0));
            ui.p().stroke(&[p], mark.stroke * k * look.pen * 0.55, pen().alpha(0.9 * h));
        }
        // the name: "open" rising into its place (cut off above the line: the p's tail comes up
        // from behind it), "OMSI" landing down its slant
        let cut = at(Vec2::new(0.0, mark.foot)).y - mark.stroke * 0.5 * k;
        ui.push_clip(Rect::new(0.0, 0.0, size.x, cut.max(0.0)), 0.0);
        for (l, (alpha, rest)) in mark.letters.iter().zip(&look.letters) {
            if *alpha <= 0.0 {
                continue;
            }
            let away = if l.style.slant > 0.0 {
                Vec2::new(l.style.slant, -1.0).normalize() * (rest * SLIDE * l.cap * k)
            } else {
                Vec2::new(0.0, rest * l.cap * 0.5 * k)
            };
            letter(ui, l, at(Vec2::new(l.x, l.base)), s, look.scale, away, l.color.alpha(alpha.powf(2.2) * a));
        }
        ui.pop_clip();
    }
}

// --- the mark at rest ------------------------------------------------------------------

/// openOMSI's mark where a page shows it (the start, the welcome): the opening's last frame,
/// and alive under the mouse - a light runs round the ring and along the line to the terminus,
/// the bus front hops and flashes its headlights when the light has gone round it, the stops
/// brighten and swell as it passes them and the terminus rings, as when the pen drew them;
/// while the mouse stays on it the line is a little lighter. With the setting "animations" off
/// it only lightens.
#[derive(Default)]
pub struct Logo {
    mark: Option<Mark>,
    /// When the light set off (on the interface's clock), and whether the mouse was on the
    /// mark last frame (the light sets off when it comes onto it, not for as long as it stays).
    light_from: Option<f32>,
    over: bool,
    /// The colour of "open" and of the bus front: white (on a dark ground) when none; the
    /// start's light glass gives its dark ink.
    pub ink: Option<Color>,
}

/// The light's run along the whole line (s), how much of the line it lights at once, and the
/// terminus's ring after it.
const LIGHT_RUN: f32 = 0.95;
const LIGHT_LEN: f32 = 0.24;
const LIGHT_RING: f32 = 0.6;
/// How long a stop stays lit after the light passed it, and how much it swells.
const STOP_LIT: f32 = 0.45;
const STOP_SWELL: f32 = 0.16;
/// The bus front's hop (and a smaller one after it), and its headlights' two flashes: when
/// each begins after the light has gone round it, and how long it lasts.
const HOP: f32 = 0.34;
const HOP_AGAIN: f32 = 0.2;
const FLASHES: [(f32, f32); 2] = [(0.0, 0.16), (0.22, 0.2)];

/// The light `t` seconds after it set off: the lit stretch (from, to, as shares of the line),
/// its strength, and the terminus's ring (0..1) once the light has arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Light {
    from: f32,
    to: f32,
    strength: f32,
    ring: Option<f32>,
}

fn light_at(t: f32) -> Option<Light> {
    if !(0.0..LIGHT_RUN + LIGHT_RING).contains(&t) {
        return None;
    }
    // the front runs from the ring's start past the terminus by a stretch, so the tail leaves
    // the line as well; eased, as a light thrown along it
    let u = ease_in_out((t / LIGHT_RUN).clamp(0.0, 1.0));
    let front = u * (1.0 + LIGHT_LEN);
    let strength = 1.0 - smoothstep(((t - LIGHT_RUN * 0.8) / (LIGHT_RUN * 0.3)).clamp(0.0, 1.0));
    let arrived = t - LIGHT_RUN * 0.82;
    Some(Light {
        from: (front - LIGHT_LEN).clamp(0.0, 1.0),
        to: front.clamp(0.0, 1.0),
        strength,
        ring: (arrived > 0.0 && arrived < LIGHT_RING).then(|| arrived / LIGHT_RING),
    })
}

/// How long ago the light's front passed `share` of the line, `t` seconds after it set off
/// (None: not yet, or the light is gone).
fn passed(t: f32, share: f32) -> Option<f32> {
    let l = light_at(t)?;
    if l.to < share {
        return None;
    }
    // (when the front passed it: the inverse of its easing, found by halving)
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if ease_in_out(mid) * (1.0 + LIGHT_LEN) < share {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(t - hi * LIGHT_RUN)
}

/// How lit a stop at `share` of the line is `t` seconds after the light set off (0..1): at
/// once when the front passes it, fading after.
fn stop_lit(t: f32, share: f32) -> f32 {
    passed(t, share).map_or(0.0, |since| 1.0 - ease_out_cubic((since / STOP_LIT).clamp(0.0, 1.0)))
}

/// The bus front `since` seconds after the light went round it: how high it is (of `HOP_H`: a
/// hop up and down and a smaller one after it) and how bright its headlights are (two flashes).
fn bus_hop(since: f32) -> (f32, f32) {
    let arc = |t: f32, len: f32| if (0.0..len).contains(&t) { 4.0 * (t / len) * (1.0 - t / len) } else { 0.0 };
    let hop = arc(since, HOP) + 0.25 * arc(since - HOP, HOP_AGAIN);
    let flash = FLASHES.iter().map(|&(at, len)| if (at..at + len).contains(&since) { (PI * (since - at) / len).sin() } else { 0.0 }).fold(0.0, f32::max);
    (hop, flash)
}

impl Logo {
    /// The mark fitted into `area` (centred, as large as it fits). Returns where it stands.
    pub fn draw(&mut self, ui: &mut Ui, name: &str, area: Rect) -> Rect {
        let mark = self.mark.get_or_insert_with(|| Mark::new(&ui.fonts)).clone();
        let k = (area.w / mark.bounds.w).min(area.h / mark.bounds.h).max(0.01);
        let centre = area.center();
        let mid = mark.bounds.center();
        let at = |q: Vec2| centre + (q - mid) * k;
        let r = Rect::new(centre.x - mark.bounds.w * k * 0.5, centre.y - mark.bounds.h * k * 0.5, mark.bounds.w * k, mark.bounds.h * k);
        let id = id_of(name);
        let (over, _, clicked) = ui.interact(id, r);
        if (over && !self.over) || clicked {
            self.light_from = Some(ui.time);
        }
        self.over = over;
        let warm = ui.anim(id ^ 0x1090, if over { 1.0 } else { 0.0 }, 0.16);
        let t = self.light_from.map(|f| ui.time - f).filter(|_| ui.motion);
        let light = t.and_then(light_at);
        if light.is_some() {
            ui.keep_moving();
        } else {
            self.light_from = None;
        }
        let u = mark.u;
        // the ring and the line, a little lighter while the mouse is on it
        let route = mark.route(&at);
        let len = route.length();
        let w = mark.stroke * k;
        ui.p().stroke(&route.part(0.0, len), w, line_colour().mix(pen(), 0.16 * warm));
        // the light: a bright stretch over the line, its glow at the front
        if let Some(l) = light {
            if l.to > l.from && l.strength > 0.0 {
                let (a, b) = (l.from * len, l.to * len);
                ui.p().stroke(&route.part(a, b), w * 0.62, pen().alpha(0.55 * l.strength));
                ui.p().stroke(&route.part((b - (b - a) * 0.35).max(a), b), w * 0.5, pen().alpha(0.85 * l.strength));
                let (front, _) = route.at(b);
                ui.p().radial(front, w * 2.0, pen_glow().alpha(0.45 * l.strength), pen_glow().alpha(0.0));
            }
        }
        // the bus front, hopping and flashing its lights once the light has gone round it
        let (hop, flash) = t.and_then(|t| passed(t, mark.timing.ring)).map_or((0.0, 0.0), bus_hop);
        bus_front(ui.p(), at(mark.bus) - Vec2::new(0.0, hop * HOP_H * u * k), u * k, 1.0, flash, self.ink.unwrap_or(INK));
        // the stops, lit and swelling as the light passes them; the terminus ringing after it
        for (stop, share) in mark.stops.iter().zip(mark.timing.stops) {
            let lit = t.map_or(0.0, |t| stop_lit(t, share));
            if lit > 0.0 {
                ui.p().radial(at(stop.at), RIM_R * u * 2.4 * k, pen_glow().alpha(0.4 * lit), pen_glow().alpha(0.0));
            }
            stop_sign(ui, stop.kind, at(stop.at), u * k, 1.0 + STOP_SWELL * lit, 1.0);
        }
        if let (Some(p), Some(end)) = (light.and_then(|l| l.ring), mark.stops.last()) {
            pulse(ui.p(), at(end.at), u * k, p, 1.0);
        }
        // the name on its baselines, on whole pixels (text between them goes soft)
        for l in &mark.letters {
            let color = if l.color == INK { self.ink.unwrap_or(INK) } else { l.color };
            letter(ui, l, at(Vec2::new(l.x, l.base)), k, 1.0, Vec2::ZERO, color);
        }
        r
    }
}

/// A letter of the name, its pen at `pen`: rasterised at its size in a mark `size` large (once
/// - a size a frame filled the atlas), drawn `grow` times that and moved by `away`; at rest
/// on whole pixels (text between them goes soft).
fn letter(ui: &mut Ui, l: &Letter, pen: Vec2, size: f32, grow: f32, away: Vec2, c: Color) {
    let px = l.px * size * ui.scale;
    let sp = if l.style == Style::default() { ui.atlas.text(&ui.fonts, &l.text, px, l.weight) } else { ui.atlas.text_styled(&ui.fonts, &l.text, px, l.weight, l.style) };
    let d = grow / ui.scale;
    let mut top = pen - Vec2::new(omsi_ui::text::PAD as f32 + l.style.lead(px), sp.ascent) * d;
    if grow == 1.0 {
        top = (top * ui.scale).round() / ui.scale;
    }
    ui.p().sprite(sp, top + away, Vec2::new(sp.w, sp.h) * d, c);
}

/// The ring going out from the terminus at `at` (`p` 0..1 of its way), `k` screen pixels to the
/// drawing's.
fn pulse(p: &mut Painter, at: Vec2, k: f32, t: f32, a: f32) {
    let r = (RIM_R * 1.08 + 80.0 * ease_out_cubic(t)) * k;
    let w = (6.5 - 3.2 * t) * k;
    p.arc(at, r - w * 0.5, r + w * 0.5, 0.0, TAU, line_colour().alpha(0.55 * (1.0 - t) * (1.0 - t) * a));
}

// (the tour's drawings draw their stops with it)
#[allow(dead_code)]
/// A stop sign at `at`, `r` round: on a rim of the casing, so it sits in the line; the
/// terminus ringed in the plate's yellow.
pub(super) fn sign(p: &mut Painter, at: Vec2, r: f32, terminus: bool, a: f32) {
    if r < 0.3 {
        return;
    }
    if terminus {
        p.arc(at, r * 1.62, r * 1.84, 0.0, std::f32::consts::TAU, LINE.alpha(a));
    }
    p.stroke(&[at], 2.0 * (r + (r * 0.16).max(1.2)), LINE_CASING.alpha(a));
    p.stroke(&[at], 2.0 * r, SIGN.alpha(a));
    p.arc(at, r * 0.73, r * 0.93, 0.0, std::f32::consts::TAU, SIGN_INK.alpha(a));
    // the H, in shares of the sign's radius
    let bar = |x0: f32, y0: f32, x1: f32, y1: f32| [at + Vec2::new(x0, y0) * r, at + Vec2::new(x1, y0) * r, at + Vec2::new(x1, y1) * r, at + Vec2::new(x0, y1) * r];
    for q in [bar(-0.42, -0.46, -0.2, 0.46), bar(0.2, -0.46, 0.42, 0.46), bar(-0.2, -0.09, 0.2, 0.09)] {
        p.convex(&q, SIGN_INK.alpha(a));
    }
}

// --- the bus front and the stops ---------------------------------------------------------

/// A box's outline with its top corners rounded by `top` and its foot's by `foot`, clockwise on
/// the screen from the top right (convex).
fn round_box(r: Rect, top: f32, foot: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(36);
    for (c, rad, from) in [
        (Vec2::new(r.right() - top, r.y + top), top, -0.5 * PI),
        (Vec2::new(r.right() - foot, r.bottom() - foot), foot, 0.0),
        (Vec2::new(r.x + foot, r.bottom() - foot), foot, 0.5 * PI),
        (Vec2::new(r.x + top, r.y + top), top, PI),
    ] {
        if rad <= 0.0 {
            out.push(c);
            continue;
        }
        for k in 0..=8 {
            out.push(c + Vec2::from_angle(from + 0.5 * PI * k as f32 / 8.0) * rad);
        }
    }
    out
}

/// What of a convex outline lies where `n` . p <= `d` (still convex). Two pieces cut apart by
/// the same line meet on exactly the same points, so no seam shows between them.
fn clip(poly: &[Vec2], n: Vec2, d: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n.dot(a) - d, n.dot(b) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// What of a convex outline lies between `x0` and `x1` and `y0` and `y1`.
fn slab(poly: &[Vec2], x0: f32, x1: f32, y0: f32, y1: f32) -> Vec<Vec2> {
    let p = clip(poly, Vec2::NEG_X, -x0);
    let p = clip(&p, Vec2::X, x1);
    let p = clip(&p, Vec2::NEG_Y, -y0);
    clip(&p, Vec2::Y, y1)
}

/// `n` points round `c` (a multiple of four, from the lower right: the square round the circle
/// has its corners in the four of them on its diagonals).
fn circle_pts(c: Vec2, r: f32, n: usize) -> Vec<Vec2> {
    (0..n).map(|k| c + Vec2::from_angle(0.25 * PI + TAU * k as f32 / n as f32) * r).collect()
}

/// How many points a circle `r` screen pixels round is drawn with.
fn points_for(r: f32) -> usize {
    ((r * 0.9) as usize).clamp(16, 96) / 4 * 4
}

fn disc(p: &mut Painter, c: Vec2, r: f32, n: usize, col: Color) {
    let pts = circle_pts(c, r, n);
    p.convex(&pts, col);
}

/// The ring between `r0` and `r1` round `c`, on the same points as a `disc` of either: they meet
/// without a seam, and without lying over each other (it is drawn see-through as it fades).
fn annulus(p: &mut Painter, c: Vec2, r0: f32, r1: f32, n: usize, col: Color) {
    let (i, o) = (circle_pts(c, r0, n), circle_pts(c, r1, n));
    for k in 0..n {
        let j = (k + 1) % n;
        p.tri(i[k], o[k], o[j], col, col, col);
        p.tri(i[k], o[j], i[j], col, col, col);
    }
}

/// The bus front with its middle at `c`, `k` screen pixels to the drawing's: white, rounded,
/// its windscreen, destination band and headlights cut out of it (in pieces that meet edge to
/// edge: what is cut out shows what lies under it, and it fades evenly), wheels under it,
/// mirrors beside it. `lamps` lights the headlights (0..1).
fn bus_front(p: &mut Painter, c: Vec2, k: f32, a: f32, lamps: f32, ink: Color) {
    if k <= 0.0 {
        return;
    }
    let col = ink.alpha(a);
    let at = |q: Vec2| c + q * k;
    let mut piece = |pts: Vec<Vec2>| {
        if pts.len() >= 3 {
            let pts: Vec<Vec2> = pts.into_iter().map(at).collect();
            p.convex(&pts, col);
        }
    };
    let body = round_box(BODY, BODY_TOP, BODY_FOOT);
    let (l, r, big) = (BODY.x - 1.0, BODY.right() + 1.0, 1e4);
    let lamp_top = LAMP_Y - LAMP_BOX;
    let lamp_foot = LAMP_Y + LAMP_BOX;
    // over the band, beside it, between it and the windscreen, beside that, under it down to
    // the headlights, beside and between them, and the foot
    piece(slab(&body, l, r, -big, BAND.y));
    piece(slab(&body, l, BAND.x, BAND.y, BAND.bottom()));
    piece(slab(&body, BAND.right(), r, BAND.y, BAND.bottom()));
    piece(slab(&body, l, r, BAND.bottom(), SCREEN.y));
    piece(slab(&body, l, SCREEN.x, SCREEN.y, SCREEN.bottom()));
    piece(slab(&body, SCREEN.right(), r, SCREEN.y, SCREEN.bottom()));
    piece(slab(&body, l, r, SCREEN.bottom(), lamp_top));
    piece(slab(&body, l, -LAMP_X - LAMP_BOX, lamp_top, lamp_foot));
    piece(slab(&body, -LAMP_X + LAMP_BOX, LAMP_X - LAMP_BOX, lamp_top, lamp_foot));
    piece(slab(&body, LAMP_X + LAMP_BOX, r, lamp_top, lamp_foot));
    piece(slab(&body, l, r, lamp_foot, big));
    // the wheels and the mirrors
    for side in [-1.0f32, 1.0] {
        let shift = |b: Rect, x: f32| Rect::new(b.x + x, b.y, b.w, b.h);
        piece(round_box(shift(WHEEL, side * WHEEL_X), 0.0, WHEEL_ROUND));
        piece(round_box(shift(MIRROR, side * MIRROR_X), MIRROR.w * 0.5, MIRROR.w * 0.5));
    }
    // the rounded corners of the band and the windscreen: what their rounding leaves of the box
    for (b, rad) in [(BAND, BAND_ROUND), (SCREEN, SCREEN_ROUND)] {
        let hole = round_box(b, rad, rad);
        let corners = [Vec2::new(b.right(), b.y), Vec2::new(b.right(), b.bottom()), Vec2::new(b.x, b.bottom()), Vec2::new(b.x, b.y)];
        for (q, corner) in corners.iter().enumerate() {
            let arc = &hole[q * 9..q * 9 + 9];
            for w in arc.windows(2) {
                p.tri(at(*corner), at(w[0]), at(w[1]), col, col, col);
            }
        }
    }
    // the square round each headlight, less the light: from each point of its circle out to
    // the square (the circle's points on the diagonals reach its corners)
    let n = points_for(LAMP_R * k);
    for side in [-1.0f32, 1.0] {
        let m = Vec2::new(side * LAMP_X, LAMP_Y);
        let ring = circle_pts(m, LAMP_R, n);
        let square: Vec<Vec2> = ring.iter().map(|&q| {
            let d = q - m;
            m + d * (LAMP_BOX / d.x.abs().max(d.y.abs()))
        }).collect();
        for i in 0..n {
            let j = (i + 1) % n;
            let (a0, a1, b0, b1) = (at(ring[i]), at(ring[j]), at(square[i]), at(square[j]));
            p.tri(a0, b0, b1, col, col, col);
            p.tri(a0, b1, a1, col, col, col);
        }
        if lamps > 0.0 {
            let lit = at(m);
            p.radial(lit, LAMP_R * 3.8 * k, HEADLIGHT.alpha(0.75 * lamps * a), HEADLIGHT.alpha(0.0));
            disc(p, lit, LAMP_R * k, n, HEADLIGHT.alpha(lamps * a));
        }
    }
}

/// A stop's sign at `at`, on its white rim: `rest` screen pixels to the drawing's at rest (what
/// its "BUS" is rasterised at), `grow` times that now (springing up, swelling).
fn stop_sign(ui: &mut Ui, kind: Kind, at: Vec2, rest: f32, grow: f32, a: f32) {
    let k = rest * grow;
    let r = SIGN_R * k;
    if r < 0.3 {
        return;
    }
    let n = points_for(RIM_R * k);
    let p = ui.p();
    annulus(p, at, r, RIM_R * k, n, RIM.alpha(a));
    // (in shares of the disc's radius, as the drawing has them)
    let q = |x: f32, y: f32| at + Vec2::new(x, y) * r;
    match kind {
        Kind::First => {
            // a red ring, a red bar across the white in it
            let (inner, bar) = (0.655 * r, 0.164 * r);
            annulus(p, at, inner, r, n, RED.alpha(a));
            let white = circle_pts(at, inner, n);
            p.convex(&clip(&white, Vec2::Y, at.y - bar), RIM.alpha(a));
            p.convex(&slab(&white, -1e9, 1e9, at.y - bar, at.y + bar), RED.alpha(a));
            p.convex(&clip(&white, Vec2::NEG_Y, -(at.y + bar)), RIM.alpha(a));
        }
        Kind::Halt => {
            // the German stop: a yellow disc, a green ring, a green H
            annulus(p, at, 0.936 * r, r, n, H_YELLOW.alpha(a));
            annulus(p, at, 0.745 * r, 0.936 * r, n, H_GREEN.alpha(a));
            disc(p, at, 0.745 * r, n, H_YELLOW.alpha(a));
            let bar = |x0: f32, y0: f32, x1: f32, y1: f32| [q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)];
            for b in [bar(-0.38, -0.464, -0.2, 0.464), bar(0.2, -0.464, 0.38, 0.464), bar(-0.2, -0.082, 0.2, 0.082)] {
                p.convex(&b, H_GREEN.alpha(a));
            }
        }
        Kind::Terminus => {
            // grey, "BUS" on it and a teal smile under that: the crescent between the circle of
            // the smile's line and a smaller one lower down, the line white over its top
            disc(p, at, r, n, BUS_GREY.alpha(a));
            let (smile_y, smile_r, low_y, low_r, line): (f32, f32, f32, f32, f32) = (-0.516, 1.079, 0.098, 0.821, 0.075);
            let top = smile_r + line * 0.5;
            // (where the two circles meet: the crescent's tips)
            let y = (top * top - low_r * low_r - smile_y * smile_y + low_y * low_y) / (2.0 * (low_y - smile_y));
            let tip = (low_r * low_r - (y - low_y) * (y - low_y)).max(0.0).sqrt();
            let m = 24;
            let xs: Vec<f32> = (0..=m).map(|i| -tip * (PI * i as f32 / m as f32).cos()).collect();
            let on = |cy: f32, rr: f32, x: f32| q(x, cy + (rr * rr - x * x).max(0.0).sqrt());
            for w in xs.windows(2) {
                let (u0, u1, l0, l1) = (on(smile_y, top, w[0]), on(smile_y, top, w[1]), on(low_y, low_r, w[0]), on(low_y, low_r, w[1]));
                p.tri(u0, l0, l1, TEAL.alpha(a), TEAL.alpha(a), TEAL.alpha(a));
                p.tri(u0, l1, u1, TEAL.alpha(a), TEAL.alpha(a), TEAL.alpha(a));
            }
            let end = 0.77;
            let smile: Vec<Vec2> = (0..=m).map(|i| on(smile_y, smile_r, -end + 2.0 * end * i as f32 / m as f32)).collect();
            p.stroke(&smile, line * r, RIM.alpha(a));
            // "BUS", its capitals centred a little over the middle, on whole pixels at rest
            let cap = 0.554 * SIGN_R * rest;
            let px = cap / cap_share(&ui.fonts, Weight::Black) * ui.scale;
            let sp = ui.atlas.text(&ui.fonts, "BUS", px, Weight::Black);
            let d = grow / ui.scale;
            let wide = sp.w - 2.0 * omsi_ui::text::PAD as f32;
            let mut top = at + Vec2::new(-wide * 0.5 * d - omsi_ui::text::PAD as f32 * d, (-0.214 * SIGN_R + 0.5 * 0.554 * SIGN_R) * k - sp.ascent * d);
            if grow == 1.0 {
                top = (top * ui.scale).round() / ui.scale;
            }
            ui.p().sprite(sp, top, Vec2::new(sp.w, sp.h) * d, RIM.alpha(a));
        }
    }
}

// --- the mark -------------------------------------------------------------------------

/// The stops' signs: the first (at the ring's foot), the German one half way, the terminus.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    First,
    Halt,
    Terminus,
}

#[derive(Clone, Debug)]
struct Stop {
    at: Vec2,
    kind: Kind,
}

/// One letter of the name, where it stands (its pen position on its baseline, in the mark's
/// pixels), how it is set, and its capitals' height (what it rises and slides by).
#[derive(Clone, Debug)]
struct Letter {
    text: String,
    x: f32,
    base: f32,
    px: f32,
    weight: Weight,
    style: Style,
    color: Color,
    cap: f32,
}

/// Where the pen gets to each thing, as shares of the whole stroke: each letter's middle, each
/// stop, and the ring's foot (the ring closed round the bus).
#[derive(Clone, Debug, PartialEq)]
struct Timing {
    letters: Vec<f32>,
    stops: [f32; 3],
    ring: f32,
}

/// The mark laid out in its own pixels (a 1440 x 900 window's): the ring round 0, 0 with the
/// bus front in it, the line from its foot on under the name, the name on its baselines over
/// the line - "open" white, "OMSI" orange, heavy and leaning - and the stops on the line: by
/// the ring, half way, and the terminus under the name's end.
#[derive(Clone, Debug)]
struct Mark {
    /// The mark's pixels to the drawing's.
    u: f32,
    r: f32,
    stroke: f32,
    foot: f32,
    line_end: f32,
    bus: Vec2,
    letters: Vec<Letter>,
    stops: [Stop; 3],
    bounds: Rect,
    timing: Timing,
}

impl Mark {
    fn new(fonts: &Fonts) -> Mark {
        let u = RING_R / DRAWN_R;
        let (r, stroke, foot) = (DRAWN_R * u, STROKE * u, FOOT * u);
        let mut letters = Vec::new();
        let mut x = r + stroke * 0.5 + OPEN_GAP * u;
        let mut name_end = 0.0;
        let words = [("open", Weight::Bold, OPEN_STYLE, OPEN_CAP, OPEN_BASE, OPEN_TRACK, INK), ("OMSI", Weight::Black, OMSI_STYLE, OMSI_CAP, OMSI_BASE, OMSI_TRACK, ORANGE)];
        for (n, (word, weight, style, cap, base, track, color)) in words.into_iter().enumerate() {
            if n > 0 {
                x -= OMSI_CLOSER * u;
            }
            // (the size that makes its capitals, boldened or thinned, as tall as asked)
            let cap = cap * u;
            let px = cap / (cap_share(fonts, weight) + 2.0 * style.bold);
            let chars: Vec<char> = word.chars().collect();
            for k in 0..chars.len() {
                // (where the letter's pen stands in the word: the word up to it, kerning and
                // all, less its own advance)
                let one = chars[k].to_string();
                let upto: String = chars[..=k].iter().collect();
                let lx = x + fonts.width_styled(&upto, px, weight, style) - fonts.width_styled(&one, px, weight, style) + track * u * k as f32;
                letters.push(Letter { text: one, x: lx, base: base * u, px, weight, style, color, cap });
            }
            x += fonts.width_styled(word, px, weight, style) + track * u * chars.len() as f32;
            name_end = x;
        }
        // the terminus under the last letter's foot, the H half way to it from the first stop
        let last = letters.last().expect("the name has letters");
        let line_end = name_end - OMSI_TRACK * u - TERMINUS_BACK * u;
        let first = FIRST_STOP * u;
        let stops = [
            Stop { at: Vec2::new(first, foot), kind: Kind::First },
            Stop { at: Vec2::new((first + line_end) * 0.5, foot), kind: Kind::Halt },
            Stop { at: Vec2::new(line_end, foot), kind: Kind::Terminus },
        ];
        let outer = r + stroke * 0.5;
        let top = (-outer).min(OMSI_BASE * u - last.cap);
        let right = (name_end + OMSI_STYLE.slant * last.cap).max(line_end + RIM_R * u);
        let bounds = Rect::new(-outer, top, right + outer, foot + RIM_R * u - top);
        let mut m = Mark { u, r, stroke, foot, line_end, bus: Vec2::new(0.0, -BUS_UP * u), letters, stops, bounds, timing: Timing { letters: Vec::new(), stops: [0.0; 3], ring: 0.0 } };
        // where the pen gets to each, as shares of the stroke (the same at every size)
        let route = m.route(&|q| q);
        let len = route.length();
        let ring = len - line_end;
        let along = |x: f32| ((ring + x) / len).clamp(0.0, 1.0);
        m.timing = Timing {
            letters: m.letters.iter().map(|l| along(l.x + fonts.width_styled(&l.text, l.px, l.weight, l.style) * 0.5)).collect(),
            stops: [along(m.stops[0].at.x), along(m.stops[1].at.x), 1.0],
            ring: ring / len,
        };
        m
    }

    /// The ring and the line on the screen (`at` places the mark's pixels there): from where the
    /// pen sets down up and round against the clock, down to the foot flatter than round, and
    /// from there on to the terminus.
    fn route(&self, at: &dyn Fn(Vec2) -> Vec2) -> Path {
        // (a quarter ellipse as a cubic: its handles this far along its tangents)
        let h = 0.552_284_8;
        let mut p = Path::new(at(Vec2::from_angle(RING_START) * self.r));
        p.arc_around(at(Vec2::ZERO), -(PI + RING_START))
            .cubic_to(at(Vec2::new(-self.r, h * self.foot)), at(Vec2::new(-h * self.r, self.foot)), at(Vec2::new(0.0, self.foot)))
            .line_to(at(Vec2::new(self.line_end, self.foot)));
        p
    }
}

/// A face's capitals' height, of its size (measured large: the measure is in whole pixels).
fn cap_share(fonts: &Fonts, weight: Weight) -> f32 {
    fonts.cap_height(1000.0, weight).max(1.0) / 1000.0
}

/// How large the mark is drawn in a window of `size` (1 at 1440 x 900): with the window, and a
/// phone's small one still gets a mark it reads - never wider than four fifths of the window.
fn fit(size: Vec2, bounds: Rect) -> f32 {
    let s = (size.x / 1440.0).min(size.y / 900.0);
    let small = (size.x * 0.42 / bounds.w).min(size.y * 0.3 / bounds.h).min(1.0);
    s.max(small).min(size.x * 0.8 / bounds.w).max(0.05)
}

// --- the timeline -----------------------------------------------------------------------

/// What the opening shows at a moment (opacities as the eye sees them; `draw` makes alphas of
/// them).
#[derive(Clone, Debug, PartialEq)]
struct Look {
    /// The ground over the page, and its blue light.
    ground: f32,
    glow: f32,
    /// The mark's opacity and size (1 at rest).
    mark: f32,
    scale: f32,
    /// How much of the stroke is drawn (0..1), the pen's press (its width, 0..1) and its light.
    drawn: f32,
    pen: f32,
    head: f32,
    /// The bus front's size (springing past 1, resting at 1).
    bus: f32,
    /// Each letter: its opacity, and how far it still is from its place (1 the whole way).
    letters: Vec<(f32, f32)>,
    /// Each stop's size (springing past 1, resting at 1).
    stops: [f32; 3],
    /// The ring going out from the terminus (0..1).
    pulse: Option<f32>,
}

/// The mark's and the ground's opacity and the mark's size at `t`: as the hand-over has them,
/// or the quick way out from where they stood.
fn fade(t: f32, leave: Option<Leave>) -> (f32, f32, f32) {
    let part = |from: f32, len: f32| ((t - from) / len).clamp(0.0, 1.0);
    match leave {
        None => {
            let m = part(LEAVE_AT, MARK_OUT);
            (1.0 - ease_in(m), 1.0 - standard(part(LEAVE_AT + GROUND_DELAY, GROUND_OUT)), 1.0 - SETTLE * ease_in_out(m))
        }
        Some(l) => {
            let m = part(l.at, QUICK_MARK);
            (l.mark * (1.0 - ease_out_cubic(m)), l.ground * (1.0 - standard(part(l.at + QUICK_DELAY, QUICK_GROUND))), l.scale - SETTLE * 0.5 * ease_out_cubic(m))
        }
    }
}

/// When the pen gets to `f` of the stroke (the inverse of its easing).
fn time_at(f: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..32 {
        let mid = (lo + hi) * 0.5;
        if draw_ease(mid) < f {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    DRAW_AT + hi * DRAW
}

/// The opening at `t` seconds: a pure function of the time, the way out taken (if any) and
/// where the pen meets each letter, stop and the ring's foot.
fn look(t: f32, leave: Option<Leave>, timing: &Timing) -> Look {
    let (mark, ground, scale) = fade(t, leave);
    // (skipped: what was on its way stays where it was while it all fades)
    let play = leave.map_or(t, |l| t.min(l.at));
    let u = ((play - DRAW_AT) / DRAW).clamp(0.0, 1.0);
    let since = |at: f32, len: f32| ((play - at) / len).clamp(0.0, 1.0);
    let pop = |f: f32, len: f32| {
        let at = time_at(f);
        if play <= at { 0.0 } else { spring(since(at, len)) }
    };
    let letters = timing
        .letters
        .iter()
        .map(|&f| {
            let p = since(time_at(f) - LETTER_LEAD, LETTER);
            (ease_out_cubic((p * 1.6).min(1.0)), 1.0 - ease_out_quint(p))
        })
        .collect();
    let arrived = DRAW_AT + DRAW;
    Look {
        ground,
        glow: ease_out_cubic((t / GLOW_IN).clamp(0.0, 1.0)),
        mark,
        scale,
        drawn: if play <= DRAW_AT { 0.0 } else { draw_ease(u) },
        pen: ease_out_cubic(since(DRAW_AT, PEN_DOWN)),
        head: smoothstep(since(DRAW_AT, 0.15)) * (1.0 - smoothstep(((u - 0.8) / 0.2).clamp(0.0, 1.0))),
        bus: pop(timing.ring, BUS_POP),
        letters,
        stops: timing.stops.map(|f| pop(f, POP)),
        pulse: (play > arrived && play < arrived + PULSE).then(|| since(arrived, PULSE)),
    }
}

// --- easing -----------------------------------------------------------------------------

/// CSS's `cubic-bezier(x1, y1, x2, y2)` at `x` (0..1): the curve's parameter for `x` found by
/// Newton's method, by halving where that does not settle.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let curve = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    };
    let slope = |a: f32, b: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
    };
    let mut t = x;
    for _ in 0..8 {
        let e = curve(x1, x2, t) - x;
        if e.abs() < 1e-6 {
            return curve(y1, y2, t);
        }
        let d = slope(x1, x2, t);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - e / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        t = (lo + hi) * 0.5;
        if curve(x1, x2, t) < x {
            lo = t;
        } else {
            hi = t;
        }
    }
    curve(y1, y2, (lo + hi) * 0.5)
}

/// The pen over the line: away gently, fastest under the name, slowing into the terminus.
fn draw_ease(u: f32) -> f32 {
    cubic_bezier(0.6, 0.0, 0.35, 1.0, u)
}

/// Leaving: slow away, then gone (Material's accelerate).
fn ease_in(u: f32) -> f32 {
    cubic_bezier(0.4, 0.0, 1.0, 1.0, u)
}

/// Material's standard curve: the ground lifting off the page.
fn standard(u: f32) -> f32 {
    cubic_bezier(0.4, 0.0, 0.2, 1.0, u)
}

fn ease_in_out(u: f32) -> f32 {
    cubic_bezier(0.45, 0.0, 0.55, 1.0, u)
}

fn ease_out_cubic(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

fn ease_out_quint(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(5)
}

fn smoothstep(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// A spring let go from 0 towards 1 (a stop popping up): a tenth over at its most, back and
/// still at `p` = 1 - the little left of its swing then is taken out evenly over the way, so
/// it ends on 1 exactly.
fn spring(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    let (zeta, omega) = (0.6f32, 9.0f32);
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    let raw = |t: f32| 1.0 - (-zeta * omega * t).exp() * ((wd * t).cos() + zeta * omega / wd * (wd * t).sin());
    raw(p) + (1.0 - raw(1.0)) * p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_run_from_0_to_1() {
        for f in [draw_ease, ease_in, standard, ease_in_out, ease_out_cubic, ease_out_quint, smoothstep] {
            assert_eq!((f(0.0), f(1.0)), (0.0, 1.0));
            let v: Vec<f32> = (0..=100).map(|k| f(k as f32 / 100.0)).collect();
            assert!(v.windows(2).all(|w| w[1] >= w[0] - 1e-5), "monotonic");
        }
        // CSS's own curves: linear is linear, a symmetric one is a half at a half
        assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, 0.3) - 0.3).abs() < 1e-4);
        assert!((cubic_bezier(0.42, 0.0, 0.58, 1.0, 0.5) - 0.5).abs() < 1e-4);
        // ease-out (0, 0, 0.58, 1) is well ahead at a quarter
        assert!(cubic_bezier(0.0, 0.0, 0.58, 1.0, 0.25) > 0.35);
    }

    #[test]
    fn the_logos_light_runs_the_line_once_and_rings_at_the_end() {
        assert_eq!(light_at(-0.1), None);
        let start = light_at(0.0).unwrap();
        assert_eq!((start.from, start.to), (0.0, 0.0));
        // the front only goes forward, the stretch it lights is never longer than its share
        let mut last = 0.0;
        for k in 0..=95 {
            let l = light_at(k as f32 * 0.01).unwrap();
            assert!(l.to >= last - 1e-5 && l.to - l.from <= LIGHT_LEN + 1e-4);
            last = l.to;
        }
        // at the end of its run the whole stretch has left the line, and the terminus rings
        let end = light_at(LIGHT_RUN).unwrap();
        assert_eq!((end.from, end.to), (1.0, 1.0));
        assert!(end.ring.is_some() && end.strength < 0.3);
        assert_eq!(light_at(LIGHT_RUN + LIGHT_RING + 0.01), None);
    }

    #[test]
    fn a_stop_lights_when_the_light_reaches_it() {
        // the stop half way: dark before the front gets there, lit at once, fading after
        assert_eq!(stop_lit(0.05, 0.5), 0.0);
        let first = (0..200).map(|k| k as f32 * 0.005).find(|&t| stop_lit(t, 0.5) > 0.0).unwrap();
        assert!(stop_lit(first, 0.5) > 0.9);
        assert!(stop_lit(first + STOP_LIT * 0.5, 0.5) < stop_lit(first, 0.5));
        assert_eq!(stop_lit(first + STOP_LIT + 0.01, 0.5), 0.0);
    }

    #[test]
    fn the_bus_hops_and_flashes_when_the_light_has_gone_round_it() {
        // nothing before the light has closed the ring, and nothing once it is over
        let ring = 0.38;
        assert_eq!(passed(0.05, ring), None);
        let at = (0..400).map(|k| k as f32 * 0.0025).find(|&t| passed(t, ring).is_some()).unwrap();
        assert!(passed(at, ring).unwrap() < 0.01);
        assert_eq!(bus_hop(-0.01), (0.0, 0.0));
        assert_eq!(bus_hop(HOP + HOP_AGAIN + 0.01), (0.0, 0.0));
        // a hop up and down, its top half way, a smaller one after; it lands in between
        let (top, _) = bus_hop(HOP * 0.5);
        assert!((top - 1.0).abs() < 1e-4);
        assert!(bus_hop(HOP).0 < 1e-4);
        let again = (1..100).map(|k| bus_hop(HOP + HOP_AGAIN * k as f32 / 100.0).0).fold(0.0, f32::max);
        assert!(again > 0.15 && again < 0.4, "{again}");
        // the headlights flash twice: on, off, on, off
        let f: Vec<f32> = (0..=60).map(|k| bus_hop(k as f32 * 0.01).1).collect();
        let ons = f.windows(2).filter(|w| w[0] < 0.5 && w[1] >= 0.5).count();
        assert_eq!(ons, 2, "{f:?}");
        // and it all happens while the light is still running (the logo keeps moving for it)
        assert!(at + HOP + HOP_AGAIN < LIGHT_RUN + LIGHT_RING);
    }

    #[test]
    fn a_stop_springs_up_and_rests() {
        assert_eq!(spring(0.0), 0.0);
        assert_eq!(spring(1.0), 1.0);
        let v: Vec<f32> = (0..=200).map(|k| spring(k as f32 / 200.0)).collect();
        let peak = v.iter().copied().fold(0.0, f32::max);
        assert!(peak > 1.05 && peak < 1.13, "over by {peak}");
        // and the last of its swing is too small to see
        assert!(v[180..].iter().all(|x| (x - 1.0).abs() < 0.02));
    }

    fn timing() -> Timing {
        Timing { letters: (0..8).map(|k| 0.45 + 0.065 * k as f32).collect(), stops: [0.4, 0.7, 1.0], ring: 0.37 }
    }

    #[test]
    fn the_opening_draws_holds_and_hands_over() {
        let tm = timing();
        let at = |t: f32| look(t, None, &tm);
        // nothing drawn at the first frame; the ground covers the page
        let first = at(0.0);
        assert_eq!((first.drawn, first.ground, first.mark, first.scale, first.bus), (0.0, 1.0, 1.0, 1.0, 0.0));
        assert!(first.letters.iter().all(|l| l.0 == 0.0) && first.stops == [0.0; 3]);
        // the pen only goes on, and arrives
        let v: Vec<f32> = (0..300).map(|k| at(k as f32 * 0.01).drawn).collect();
        assert!(v.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(at(DRAW_AT + DRAW).drawn, 1.0);
        // the bus front springs up once the ring has closed round it, not before
        let closed = time_at(tm.ring);
        assert_eq!(at(closed - 0.01).bus, 0.0);
        assert!(at(closed + 0.05).bus > 0.0);
        // a stop pops when the pen reaches it, not before
        let s1 = time_at(tm.stops[1]);
        assert_eq!(at(s1 - 0.01).stops[1], 0.0);
        assert!(at(s1 + 0.05).stops[1] > 0.0);
        // the letters come one after another
        let starts: Vec<f32> = tm.letters.iter().map(|f| time_at(*f)).collect();
        assert!(starts.windows(2).all(|w| w[1] > w[0]));
        // everything is in its place before the hand-over, exactly
        let rest = at(LEAVE_AT);
        assert!(rest.letters.iter().all(|l| *l == (1.0, 0.0)), "{:?}", rest.letters);
        assert_eq!((rest.stops, rest.bus), ([1.0; 3], 1.0));
        assert_eq!((rest.drawn, rest.head, rest.pulse, rest.mark, rest.ground), (1.0, 0.0, None, 1.0, 1.0));
        // then the mark goes first, the ground after it, and the page is there
        let mid = at(LEAVE_AT + 0.2);
        assert!(mid.mark < 1.0 && mid.ground > mid.mark && mid.scale < 1.0);
        assert_eq!(at(LENGTH).ground, 0.0);
        assert!((2.2..=2.8).contains(&LENGTH), "{LENGTH}");
        // and the ground only ever lifts
        let g: Vec<f32> = (0..300).map(|k| at(k as f32 * 0.01).ground).collect();
        assert!(g.windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn a_click_hands_over_quickly_from_where_it_stood() {
        let tm = timing();
        let t0 = 0.7;
        let (mark, ground, scale) = fade(t0, None);
        let leave = Some(Leave { at: t0, mark, ground, scale });
        let before = look(t0, None, &tm);
        // no jump at the click; the line stops where it was
        let now = look(t0, leave, &tm);
        assert_eq!((now.mark, now.ground, now.drawn), (before.mark, before.ground, before.drawn));
        assert_eq!(look(t0 + 0.1, leave, &tm).drawn, before.drawn);
        assert_eq!(look(t0 + QUICK_DELAY + QUICK_GROUND, leave, &tm).ground, 0.0);
        // the mark is gone before the ground is half lifted
        assert_eq!(look(t0 + QUICK_MARK, leave, &tm).mark, 0.0);
        assert!(look(t0 + QUICK_MARK, leave, &tm).ground > 0.5);
        // in the middle of the hand-over: on from the opacities it had, never back up
        let t1 = LEAVE_AT + 0.25;
        let (m1, g1, s1) = fade(t1, None);
        let l1 = Some(Leave { at: t1, mark: m1, ground: g1, scale: s1 });
        let v: Vec<f32> = (0..40).map(|k| fade(t1 + k as f32 * 0.01, l1).1).collect();
        assert!(v[0] == g1 && v.windows(2).all(|w| w[1] <= w[0]) && *v.last().unwrap() == 0.0);
    }

    #[test]
    fn the_mark_is_laid_out_and_fits() {
        let fonts = Fonts::hanken();
        let m = Mark::new(&fonts);
        // the letters in order, after the ring; "OMSI" leaning, "open" upright
        assert_eq!(m.letters.iter().map(|l| l.text.as_str()).collect::<String>(), "openOMSI");
        assert!(m.letters.windows(2).all(|w| w[1].x > w[0].x) && m.letters[0].x > m.r + m.stroke * 0.5);
        assert!(m.letters[..4].iter().all(|l| l.style.slant == 0.0) && m.letters[4..].iter().all(|l| l.style.slant > 0.2));
        // "OMSI" a little larger than "open", standing a little lower
        assert!(m.letters[4].cap > m.letters[0].cap && m.letters[4].base > m.letters[0].base);
        // the first stop by the ring, the H half way to the terminus, under the end of "open";
        // the terminus under the name's last letter
        assert!(m.stops[0].at.x > 0.0 && m.stops[0].at.x < m.r);
        assert!((m.stops[1].at.x - (m.stops[0].at.x + m.stops[2].at.x) * 0.5).abs() < 1e-3);
        assert!(m.stops[1].at.x > m.letters[3].x && m.stops[1].at.x < m.letters[4].x);
        assert!(m.stops[2].at.x > m.letters[7].x - 20.0 && m.stops[2].at.x == m.line_end);
        // the pen closes the ring, then meets the stops and the letters in order
        assert!(m.timing.letters.windows(2).all(|w| w[1] > w[0]));
        assert!(m.timing.ring < m.timing.stops[0] && m.timing.stops[0] < m.timing.letters[0]);
        assert!(m.timing.letters[3] < m.timing.stops[1] && m.timing.stops[1] < m.timing.letters[4] && m.timing.stops[2] == 1.0);
        // the name clear of the line, the ring's opening open: where the pen sets down is well
        // clear of the line it leaves by
        assert!(m.foot - m.stroke * 0.5 - m.letters[0].base > 0.3 * m.letters[0].cap);
        let start = Vec2::from_angle(RING_START) * m.r;
        assert!(m.foot - start.y > m.stroke + 4.0, "{start}");
        // the bus front inside the ring, clear of it
        let inside = m.r - m.stroke * 0.5;
        for c in round_box(BODY, BODY_TOP, BODY_FOOT).iter().chain(&round_box(Rect::new(MIRROR.x - MIRROR_X, MIRROR.y, MIRROR.w, MIRROR.h), 0.0, 0.0)) {
            assert!((m.bus + *c * m.u).length() < inside - 4.0, "{c}");
        }
        // the route: the ring, then the line; it passes through the foot where the ring ends
        let route = m.route(&|q| q);
        let ring = m.timing.ring * route.length();
        assert!((route.length() - ring - m.line_end).abs() < 1e-2);
        assert!(route.at(ring).0.distance(Vec2::new(0.0, m.foot)) < 0.5);
        assert!(ring > PI * m.r && ring < 1.7 * PI * m.r, "{ring}");
        // all of it in its bounds
        for s in &m.stops {
            assert!(m.bounds.contains(s.at + Vec2::splat(RIM_R * m.u * 0.99)) && m.bounds.contains(s.at - Vec2::splat(RIM_R * m.u * 0.99)));
        }
        assert!(m.bounds.x <= -m.r - m.stroke * 0.5 && m.bounds.y <= -m.r - m.stroke * 0.5);
        // at the launcher's sizes: in the window, centred, larger with it
        for (w, h) in [(1440.0, 900.0), (2560.0, 1347.0), (1350.0, 850.0), (820.0, 380.0)] {
            let s = fit(Vec2::new(w, h), m.bounds);
            let (bw, bh) = (m.bounds.w * s, m.bounds.h * s);
            assert!(bw <= w * 0.8 + 0.01 && bh < h * 0.5, "{w}x{h}: {bw}x{bh}");
            assert!(bw >= w * 0.25, "{w}x{h}: readable, {bw}");
        }
        assert_eq!(fit(Vec2::new(1440.0, 900.0), m.bounds), 1.0);
        assert!(fit(Vec2::new(2560.0, 1347.0), m.bounds) > 1.4);
    }

    /// The pieces of the bus front meet edge to edge and cover its body less what is cut out
    /// of it, nothing twice.
    #[test]
    fn the_bus_front_is_drawn_once_over() {
        let mut p = Painter::new();
        bus_front(&mut p, Vec2::ZERO, 1.0, 1.0, 0.0, INK);
        let area: f32 = p.verts.chunks(3).map(|t| {
            let (a, b, c) = (Vec2::new(t[0].pos[0], t[0].pos[1]), Vec2::new(t[1].pos[0], t[1].pos[1]), Vec2::new(t[2].pos[0], t[2].pos[1]));
            (b - a).perp_dot(c - a).abs() * 0.5
        }).sum();
        let poly = |pts: &[Vec2]| (0..pts.len()).map(|i| pts[i].perp_dot(pts[(i + 1) % pts.len()])).sum::<f32>().abs() * 0.5;
        let lamp = poly(&circle_pts(Vec2::ZERO, LAMP_R, points_for(LAMP_R)));
        let want = poly(&round_box(BODY, BODY_TOP, BODY_FOOT)) - poly(&round_box(BAND, BAND_ROUND, BAND_ROUND)) - poly(&round_box(SCREEN, SCREEN_ROUND, SCREEN_ROUND)) - 2.0 * lamp
            + 2.0 * poly(&round_box(WHEEL, 0.0, WHEEL_ROUND))
            + 2.0 * poly(&round_box(MIRROR, MIRROR.w * 0.5, MIRROR.w * 0.5));
        assert!((area / want - 1.0).abs() < 2e-3, "{area} {want}");
    }
}
