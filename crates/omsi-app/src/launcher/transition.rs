//! The bus between menus: when the player goes to another page or another step of the setup,
//! a cover comes up over the whole window in a tenth of a second - over the bar too: the new
//! page is not seen yet - a city bus drives across it, and only once the bus and the air it
//! leaves behind are out of the window does the new page pop up: the cover opens from the
//! middle, a hole of the window's shape with a soft edge that grows past the corners in under
//! a quarter of a second. After Omsi-Hub's "busrit", as Luc asked for it: the next page is
//! shown when the bus has crossed the screen. The cover is a screen of its own in the
//! launcher's look - the ground's night blue, a soft light in the middle and the corners a
//! little deeper - with the thin road the bus drives on: the way ahead quiet, the way driven
//! in the route's blue. While it hides the page, the page gets no clicks, keys, wheel or
//! typing and does not see the mouse (it cannot be seen); all of it is the page's again as
//! the cover opens.
//!
//! A change is found in one place, the screen at the end of a frame against the one at the
//! end of the frame before (`after_page`), not at every place that changes the page or the
//! step. Going back (to an earlier step, or from a page to the start) the bus comes from the
//! right, facing left. The bus and the cover are a function of the time since the change only
//! (`Pass`, `Cover`, `motion`): a frame looks the same whatever the frame rate, and the tests
//! can look at any moment of it. A change during a pass does not blink: the cover stays up -
//! still coming up, it goes on from where it stood; already opening, it closes again - and the
//! bus that was on its way fades out as it drives on while a new one sets off. With the
//! setting "animations" off there is no cover and no bus: the new screen at once.
//!
//! For looking at it frame by frame: `OMSI_LAUNCHER_TRANSITION_AT=0.35` stops every pass at
//! that many seconds in (it then never ends, and while the cover is up at that moment the page
//! gets no clicks - a script's `page` still goes to another page), and
//! `OMSI_LAUNCHER_TRANSITION_SLOW=8` runs every pass eight times slower.

use super::flow::Step;
use super::phone::{Sheet, Tab};
use super::theme::*;
use super::ui::{ease_out_cubic, smoothstep, Input, Ui};
use super::{mobile, Launcher, Page};
use glam::Vec2;
use omsi_ui::{Color, Painter, Rect};
use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

/// The bus's length in its own drawing (points at scale 1).
const LEN: f32 = 300.0;
/// The air the bus leaves behind it as long as it is fast: each streak's height in the
/// drawing, its share of the full reach and how far behind the rear it begins - and the full
/// reach, the way the bus goes in this long (seconds). What else of the bus lies behind its
/// rear (the shadow's blur, the body's pitch), in the drawing's points.
const STREAKS: [(f32, f32, f32); 3] = [(64.0, 1.0, 10.0), (45.0, 0.62, 24.0), (24.0, 0.82, 14.0)];
const STREAK_TIME: f32 = 0.055;
const BEHIND: f32 = 4.0;
/// The share of its top speed the bus comes in with, and the share of the crossing it speeds
/// up in: it is moving when it appears (a bus starting from standing off screen would leave
/// the cover empty for a fifth of a second), and pulls away visibly while it crosses the
/// first part.
const START: f32 = 0.35;
const RAMP: f32 = 0.42;
/// A bus whose pass another change took over fades out in this long while it drives on.
const GHOST_FADE: f32 = 0.2;
/// The cover comes up in this long (seconds), counted from the frame of the change - which
/// already shows it beginning: a click is answered at once.
const COVER_IN: f32 = 0.07;
const FIRST_FRAME: f32 = 1.0 / 60.0;
/// The pop: how long the cover takes to open once the bus is out (seconds), and the share of
/// that after which what is left of it fades as well (the corners, reached last, go with it).
const POP: f32 = 0.16;
const FADE_FROM: f32 = 0.35;
/// The hole the cover opens with: its corners' radius (a share of its smaller half), and its
/// soft edge (a share of the window's smaller side).
const HOLE_ROUND: f32 = 0.5;
const SOFT: f32 = 0.08;
/// The cover's light in the middle and its deeper corners (the ground's night blue between),
/// how far the light reaches and where the corners begin to sink (shares of the way from the
/// middle to a corner), and how far they sink.
const COVER_LIGHT: Color = Color::rgba(19, 31, 62, 1.0);
const COVER_DEEP: Color = Color::rgba(3, 5, 11, 1.0);
const LIGHT_REACH: f32 = 0.8;
const SINK_FROM: f32 = 0.35;
const SINK: f32 = 0.7;
/// How the cover is drawn: rays from its middle (and one through each corner of the window),
/// on each the soft edge's rings and then these shares of the rest of the way to the edge.
const RAYS: usize = 128;
const SOFT_RINGS: usize = 6;
const OUTER: [f32; 9] = [0.05, 0.12, 0.2, 0.3, 0.42, 0.55, 0.7, 0.85, 1.0];
/// The road: how far under the window's middle (the drawing's points: the bus stands in the
/// middle), its thickness and the glow of its driven part on each side, the share of the
/// width at each end over which it comes out of the dark, and the steps it is drawn in.
const ROAD_DROP: f32 = 41.0;
const ROAD_W: f32 = 2.0;
const ROAD_GLOW: f32 = 6.0;
const ROAD_FADE: f32 = 0.12;
const ROAD_STEP: f32 = 8.0;
/// The body's suspension: its frequency, its damping (a little overshoot) and the pitch at
/// the hardest pull (radians, about 1.5 degrees).
const SPRING_HZ: f32 = 2.4;
const DAMPING: f32 = 0.36;
const PITCH: f32 = 0.026;
/// The drawing's wheels: radius, and where the two axles are from the rear.
const WHEEL_R: f32 = 14.0;
const AXLES: [f32; 2] = [74.0, 224.0];
/// The fastest the hubs are seen to turn (radians a second): faster, a five-bolt hub strobes
/// and seems to turn backwards; they keep this rate, and the rim blurs instead.
const HUB_TURN: f32 = 15.0;
/// Where the mouse is while the page under the cover is not to see it: far off any window.
const AWAY: Vec2 = Vec2::new(-3.0e4, -3.0e4);

/// Which screen is shown: a step of the setup, another page, or what a phone shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Step(Step),
    Page(Page),
    Phone(Tab, Option<Page>, Option<Sheet>),
}

impl Screen {
    fn of(l: &Launcher) -> Screen {
        if mobile::mobile() {
            Screen::Phone(l.phone.tab, l.phone.page, l.phone.sheet)
        } else if l.page == Page::Drive {
            Screen::Step(l.drive.step)
        } else {
            Screen::Page(l.page)
        }
    }

    /// Where the screen lies on the way from the driver to the bus: going to a place before
    /// this one is going back. A page lies just after the step it hangs under in the bar (the
    /// service record under the driver, the others under the mode, as `flow::step_of` hangs
    /// them); a phone's page or sheet after its tab.
    fn place(self) -> (u8, u8) {
        let step = |s: Step| match s {
            Step::Profile => 0,
            Step::Mode => 1,
            Step::Map => 2,
            Step::Start => 3,
            Step::Day => 4,
            Step::Duty => 5,
            Step::Bus => 6,
        };
        match self {
            Screen::Step(s) => (step(s), 0),
            Screen::Page(Page::Profile) => (0, 1),
            Screen::Page(_) => (1, 1),
            Screen::Phone(tab, page, sheet) => (tab as u8, page.is_some() as u8 + 2 * sheet.is_some() as u8),
        }
    }
}

/// One pass on its way: how long it has been going, which way, how the cover stood when it
/// set off, and - once another change took over - how long it had been going then.
#[derive(Clone, Copy, Debug)]
struct Run {
    age: f32,
    back: bool,
    from: Cover,
    ghost: Option<f32>,
}

/// The bus between menus (see the module): the screen last frame and the passes on their way,
/// the one that holds the cover last.
pub struct Transition {
    last: Option<Screen>,
    runs: Vec<Run>,
    /// Where the mouse was while the cover hid the page (see `before_page`).
    mouse: Option<Vec2>,
    /// `OMSI_LAUNCHER_TRANSITION_AT` and `OMSI_LAUNCHER_TRANSITION_SLOW` (see the module).
    freeze: Option<f32>,
    slow: f32,
    /// The plain change of screen (the bus between pages off, its default): the screen seen
    /// last, and how long ago the new one came (seconds) while it still fades in.
    fade_last: Option<Screen>,
    fade: Option<f32>,
}

/// How long a new screen takes to fade in from the ground (seconds).
const FADE_S: f32 = 0.24;

/// How much of the ground still lies over a screen that came `age` seconds ago (1: all of
/// it, 0: none): quick at first, settling softly.
fn fade_cover(age: f32) -> f32 {
    1.0 - ease_out_cubic((age / FADE_S).clamp(0.0, 1.0))
}

impl Transition {
    pub fn new() -> Transition {
        let num = |name: &str| omsi_cfg::env::var(name).ok().and_then(|v| v.trim().parse::<f32>().ok());
        Transition { last: None, runs: Vec::new(), mouse: None, freeze: num("OMSI_LAUNCHER_TRANSITION_AT").map(|t| t.max(0.0)), slow: num("OMSI_LAUNCHER_TRANSITION_SLOW").unwrap_or(1.0).max(0.01), fade_last: None, fade: None }
    }

    /// The pass that holds the cover, if one is on its way.
    fn lead(&self) -> Option<&Run> {
        self.runs.last().filter(|r| r.ghost.is_none())
    }

    /// Time goes on for the passes on their way (`dt` seconds of the launcher's clock).
    fn advance(&mut self, dt: f32) {
        for r in &mut self.runs {
            r.age = match self.freeze {
                Some(t) => t,
                None => r.age + dt / self.slow,
            };
        }
    }

    /// Whether the cover hides the page: from the change until the bus is out of the window.
    /// Meanwhile the page gets no input at all (a double click on the way on would otherwise
    /// go on twice, the second time on a screen not yet seen); as the cover opens it has it
    /// again.
    fn covering(&self, pass: &Pass) -> bool {
        self.lead().is_some_and(|r| r.age < pass.dur)
    }

    /// The screen this frame is `now`: a pass over a window crossed as `pass` starts when it
    /// is another than last frame's - if passes are `wanted` at all (none without animations:
    /// one on its way goes at once). It takes the cover over as the one before left it, and
    /// the bus on its way drives on and fades out (the one before that, if any, goes at once).
    fn see(&mut self, now: Screen, pass: &Pass, wanted: bool) {
        let was = self.last.replace(now);
        if !wanted {
            self.runs.clear();
            return;
        }
        let Some(was) = was.filter(|w| *w != now) else { return };
        // (stood still for a picture, every pass looks as one from a quiet screen)
        let from = match self.lead() {
            Some(r) if self.freeze.is_none() => pass.cover(r.age, r.from),
            _ => Cover::NONE,
        };
        for r in &mut self.runs {
            r.ghost.get_or_insert(r.age);
        }
        // (stood still for a picture, a fading bus would never fade: only the new one stays)
        let keep = if self.freeze.is_some() { 0 } else { 1 };
        if self.runs.len() > keep {
            self.runs.drain(..self.runs.len() - keep);
        }
        let age = self.freeze.unwrap_or(0.0);
        log::debug!("launcher: a bus from {was:?} to {now:?}");
        self.runs.push(Run { age, back: now.place() < was.place(), from, ghost: None });
    }

    /// The passes that are over go, and the buses that have faded out or left the window.
    fn tidy(&mut self, pass: &Pass) {
        if self.freeze.is_some() {
            return;
        }
        self.runs.retain(|r| match r.ghost {
            None => r.age < pass.end(),
            Some(g) => r.age < pass.dur && r.age - g < GHOST_FADE,
        });
    }
}

/// Takes from `i` what a page under the cover must not have, as it cannot be seen: clicks,
/// keys, the wheel, typing - and the mouse, put away (no hover, no pointer, no tooltip on it).
/// A button held stays held: without a press nothing on the page starts to drag. Returns where
/// the mouse was.
fn hold(i: &mut Input) -> Vec2 {
    let at = i.mouse;
    i.mouse = AWAY;
    (i.pressed, i.released, i.right_pressed, i.double_click) = (false, false, false, false);
    i.wheel = Vec2::ZERO;
    i.text.clear();
    i.keys.clear();
    i.raw_key = None;
    at
}

/// The mouse back where `hold` found it - unless it has moved since.
fn give_back(i: &mut Input, at: Vec2) {
    if i.mouse == AWAY {
        i.mouse = at;
    }
}

/// Called before the page is drawn: the clock of the passes goes on, and while the cover hides
/// the page, the page gets no input.
pub fn before_page(l: &mut Launcher) {
    let pass = Pass::new(l.ui.size, mobile::mobile());
    let tr = &mut l.transition;
    tr.advance(l.ui.dt);
    // (a copy of the input taken while the mouse was away - a dialog's, see
    // `Launcher::draw_ui` - can have left it away)
    if let Some(at) = tr.mouse.take() {
        give_back(&mut l.ui.input, at);
    }
    if tr.covering(&pass) {
        tr.mouse = Some(hold(&mut l.ui.input));
    }
}

/// Called once the page (and its bar) is drawn, under the dialogs and the dropdowns: a change
/// of screen starts a pass, and the cover, its road and the buses on their way are drawn over
/// the page.
pub fn after_page(l: &mut Launcher) {
    // (the page is drawn: what lies over the cover - the tour, a dialog - has the mouse again)
    if let Some(at) = l.transition.mouse {
        give_back(&mut l.ui.input, at);
    }
    let size = l.ui.size;
    let pass = Pass::new(size, mobile::mobile());
    // (none with the animations off; none in the picture a game leaves in the window either,
    // see `Launcher::frame`)
    // (and only when the player chose the bus between pages: it is off by default)
    let bus = l.state.settings.get("page_bus").and_then(|v| v.as_bool()).unwrap_or(false);
    let wanted = l.ui.motion && bus && !(l.state.in_game() && !l.awake());
    let now = Screen::of(l);
    // without the bus, a new screen fades in from the ground (with the animations on)
    let quiet = l.ui.motion && !bus && !(l.state.in_game() && !l.awake());
    let changed = l.transition.fade_last.replace(now).is_some_and(|was| was != now);
    if quiet && changed {
        l.transition.fade = Some(0.0);
    } else if !quiet {
        l.transition.fade = None;
    }
    if let Some(age) = l.transition.fade {
        let cover = fade_cover(age);
        if cover <= 0.001 {
            l.transition.fade = None;
        } else {
            l.ui.keep_moving();
            l.ui.p().rect(Rect::new(0.0, 0.0, size.x, size.y), GROUND.alpha(cover));
            l.transition.fade = Some(age + l.ui.dt / l.transition.slow);
        }
    }
    l.transition.see(now, &pass, wanted);
    l.transition.tidy(&pass);
    if l.transition.runs.is_empty() {
        return;
    }
    l.ui.keep_moving();
    let runs = l.transition.runs.clone();
    paint(&mut l.ui, &pass, size, size.y * 0.5 + ROAD_DROP * pass.k, &runs);
}

/// One pass of the bus over a window: how big the bus is, how far it goes and how fast.
#[derive(Clone, Copy, Debug)]
struct Pass {
    w: f32,
    /// The drawing's scale (1: the bus is `LEN` points long), and the bus's length.
    k: f32,
    len: f32,
    /// The crossing - until the bus and the air behind it are out of the window, when the
    /// cover opens - and the first part of it, in which the bus speeds up (seconds).
    dur: f32,
    ramp: f32,
    /// Its speed as it comes in, and once it is up to speed (points a second).
    v0: f32,
    vmax: f32,
}

impl Pass {
    /// A pass over a window of `size` points: a little longer and with a slightly larger bus
    /// on a large screen; on a phone a smaller bus and a shorter pass.
    fn new(size: Vec2, phone: bool) -> Pass {
        let k = if phone { 0.62 } else { (size.y / 900.0).max(0.0).sqrt().clamp(1.0, 1.25) };
        let len = LEN * k;
        // (quick, at Luc's asking: a little over half a second across a 1440 window - the bus
        // is there to say the page changes, not to keep the player waiting for it)
        let dur = if phone { 0.45 } else { (0.55 * (size.x.max(1.0) / 1440.0).powf(0.25)).clamp(0.52, 0.62) };
        let ramp = dur * RAMP;
        // (the front goes from the window's edge, where nothing of the bus is seen yet, until
        // the bus and what trails it are out of the window at the other side; the air behind
        // it reaches further the faster it goes, and it goes the faster the further it has to:
        // a few rounds find both)
        let span = START * dur + (1.0 - START) * (dur - 0.5 * ramp);
        let way = size.x.max(0.0) + len;
        let mut vmax = way / span;
        for _ in 0..8 {
            vmax = (way + trail(vmax, k)) / span;
        }
        Pass { w: size.x, k, len, dur, ramp, v0: START * vmax, vmax }
    }

    /// The whole pass: the crossing, and the cover opening after it.
    fn end(&self) -> f32 {
        self.dur + POP
    }

    /// How far the front has come `t` seconds in (from where the bus is not yet seen).
    fn front(&self, t: f32) -> f32 {
        let t = t.max(0.0);
        let dv = self.vmax - self.v0;
        if t < self.ramp {
            // (the integral of the speed below)
            let u = t / self.ramp;
            self.v0 * t + dv * self.ramp * (u.powi(3) - 0.5 * u.powi(4))
        } else {
            self.v0 * self.ramp + 0.5 * dv * self.ramp + self.vmax * (t - self.ramp)
        }
    }

    /// The speed `t` seconds in: up from `v0` to `vmax` along a smoothstep, so that it pulls
    /// away without a jolt and reaches its speed without one.
    fn speed(&self, t: f32) -> f32 {
        self.v0 + (self.vmax - self.v0) * smoothstep(t / self.ramp)
    }

    /// How hard it speeds up `t` seconds in (points a second, a second).
    fn accel(&self, t: f32) -> f32 {
        if t <= 0.0 || t >= self.ramp {
            return 0.0;
        }
        let u = t / self.ramp;
        (self.vmax - self.v0) / self.ramp * 6.0 * u * (1.0 - u)
    }

    /// The cover `t` seconds in, for a pass that found it as `from` (a change during a pass:
    /// it goes on from there, no dip). It comes up fast, easing out - and a hole it had opened
    /// closes again as fast; it stands while the bus crosses; and once the bus is out it opens.
    fn cover(&self, t: f32, from: Cover) -> Cover {
        let t = t.max(0.0);
        if t < self.dur {
            let rise = ease_out_cubic((t + FIRST_FRAME) / COVER_IN);
            Cover { up: from.up + (1.0 - from.up) * rise, open: from.open * (1.0 - rise) }
        } else if t >= self.end() {
            // (open at the end whatever the rounding of dur + POP - POP)
            Cover { up: 1.0, open: 1.0 }
        } else {
            Cover { up: 1.0, open: ((t - self.dur) / POP).min(1.0) }
        }
    }

    /// Where on the window a distance along the way is.
    fn x_of(&self, back: bool, d: f32) -> f32 {
        if back {
            self.w - d
        } else {
            d
        }
    }
}

/// How far behind the bus's rear what is drawn of it reaches at the speed `v` (on the window,
/// the drawing at the scale `k`): the longest of its streaks, and the shadow's blur and the
/// body's pitch.
fn trail(v: f32, k: f32) -> f32 {
    STREAKS.iter().map(|&(_, share, gap)| gap * k + share * v * STREAK_TIME).fold(0.0, f32::max) + BEHIND * k
}

/// How the cover stands: how far it has come up (0: not at all, 1: wholly), and how far it
/// has opened again (0: closed, 1: gone).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cover {
    up: f32,
    open: f32,
}

impl Cover {
    /// None: the screen before a pass.
    const NONE: Cover = Cover { up: 0.0, open: 0.0 };
}

/// The cover over a window as it stands: its middle and half the window; how much of it is
/// there at all (coming up, or fading as it opens); and the hole it opens with - a box of the
/// window's shape `s` times its size, its corners rounded, its edge `soft` points wide.
#[derive(Clone, Copy, Debug)]
struct Hole {
    mid: Vec2,
    half: Vec2,
    s: f32,
    round: f32,
    soft: f32,
    up: f32,
}

impl Hole {
    /// The cover over a window of `size` as `c` has it: the hole is there at once and grows,
    /// settling softly (see `opening`), and once it is well open what is left fades as well.
    fn new(size: Vec2, c: Cover) -> Hole {
        let half = (0.5 * size).max(Vec2::ONE);
        let open = c.open.clamp(0.0, 1.0);
        let fade = 1.0 - smoothstep((open - FADE_FROM) / (1.0 - FADE_FROM));
        let mut hole = Hole { mid: 0.5 * size, half, s: 0.0, round: HOLE_ROUND * half.x.min(half.y), soft: SOFT * 2.0 * half.x.min(half.y), up: c.up.clamp(0.0, 1.0) * fade };
        hole.s = opening(open) * hole.wide_open();
        hole
    }

    /// How far `q` lies outside the hole's edge (points; inside, below 0).
    fn outside(&self, q: Vec2) -> f32 {
        sd_box(q - self.mid, self.half * self.s, self.round * self.s)
    }

    /// The size at which the whole window lies in the clear middle of the hole: its corners a
    /// soft edge inside it. (The corner comes further in as the hole grows: halving the
    /// sizes finds it.)
    fn wide_open(&self) -> f32 {
        let (mut lo, mut hi) = (0.0, 4.0);
        for _ in 0..40 {
            let m = 0.5 * (lo + hi);
            if sd_box(self.half, self.half * m, self.round * m) > -self.soft {
                lo = m;
            } else {
                hi = m;
            }
        }
        hi
    }

    /// How much the cover hides of what lies at `q` (0..1): nothing in the hole's clear
    /// middle, all of it beyond its soft edge - as the eye sees it: the launcher blends in
    /// linear light, where an alpha of a half hides far less than half (see `alpha_for`).
    fn cover_at(&self, q: Vec2) -> f32 {
        alpha_for(self.up * smoothstep((self.outside(q) + self.soft) / self.soft))
    }
}

/// The hole's size (a share of the size at which the window is clear) as far as it has
/// opened: what it shows grows a little less from each frame to the next - the first frame
/// already a third of the way, a pop, and then settling into the window's corners, so that the
/// whole of the opening is seen (eased by the size alone, the hole was past all but the
/// corners halfway).
fn opening(open: f32) -> f32 {
    (1.0 - (1.0 - open.clamp(0.0, 1.0)).powf(1.5)).sqrt()
}

/// The alpha that hides `share` of what lies under it as the eye sees it: the launcher blends
/// in linear light, where a dark cover at half alpha leaves a light colour on the screen at
/// about three quarters of its brightness. The alpha goes through the screen's curve (a gamma
/// of 2.2, near enough) so that a cover half up looks half up.
fn alpha_for(share: f32) -> f32 {
    1.0 - (1.0 - share.clamp(0.0, 1.0)).powf(2.2)
}

/// How far `p` lies outside a box round the origin with half sizes `b`, its corners rounded
/// by `r` (inside, below 0). With no size it is the distance from the origin.
fn sd_box(p: Vec2, b: Vec2, r: f32) -> f32 {
    let r = r.min(b.x).min(b.y).max(0.0);
    let q = p.abs() - (b - Vec2::splat(r));
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - r
}

/// How far from the middle along `d` (a unit direction, both parts at least 0) the edge of a
/// box with half sizes `b`, its corners rounded by `r`, lies: on a side, or round a corner.
fn ray_box(d: Vec2, b: Vec2, r: f32) -> f32 {
    if b.x <= 0.0 || b.y <= 0.0 {
        return 0.0;
    }
    let r = r.clamp(0.0, b.x.min(b.y));
    let tx = if d.x > 1e-6 { b.x / d.x } else { f32::INFINITY };
    let ty = if d.y > 1e-6 { b.y / d.y } else { f32::INFINITY };
    let t = tx.min(ty);
    let q = d * t;
    if q.x <= b.x - r || q.y <= b.y - r {
        return t;
    }
    // (round the corner: where the ray leaves the circle the corner is a quarter of)
    let c = b - Vec2::splat(r);
    let dc = d.dot(c);
    dc + (dc * dc - c.length_squared() + r * r).max(0.0).sqrt()
}

/// The cover's points: along rays from its middle - evenly round, and one through each corner
/// of the window, so that the outermost points go round the window's edge exactly - first the
/// rings of the hole's soft edge (where the hole is not yet that large, at the middle), then on
/// to the window's edge. A ring the window cuts off lies on its edge.
fn mesh(hole: &Hole) -> Vec<Vec<Vec2>> {
    let corner = hole.half.y.atan2(hole.half.x);
    let mut angles: Vec<f32> = (0..RAYS).map(|i| TAU * i as f32 / RAYS as f32).chain([corner, PI - corner, PI + corner, TAU - corner]).collect();
    angles.sort_by(f32::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let (b, r) = (hole.half * hole.s, hole.round * hole.s);
    angles
        .into_iter()
        .map(|a| {
            let d = Vec2::from_angle(a);
            let u = d.abs();
            let edge = (hole.half.x / u.x.max(1e-6)).min(hole.half.y / u.y.max(1e-6));
            let mut ts: Vec<f32> = (0..SOFT_RINGS)
                .map(|i| {
                    let off = hole.soft * (i as f32 / (SOFT_RINGS - 1) as f32 - 1.0);
                    ray_box(u, (b + Vec2::splat(off)).max(Vec2::ZERO), (r + off).max(0.0)).min(edge)
                })
                .collect();
            let t0 = ts[SOFT_RINGS - 1];
            ts.extend(OUTER.iter().map(|f| t0 + (edge - t0) * f));
            ts.into_iter().map(|t| hole.mid + d * t).collect()
        })
        .collect()
}

/// The cover's colour at `q`, before it is see-through: the ground's night blue, lit softly in
/// the middle and sinking a little towards the corners, along ellipses that fit the window.
fn shade(hole: &Hole, q: Vec2) -> Color {
    let d = ((q - hole.mid) / hole.half).length() * FRAC_1_SQRT_2;
    let lit = 1.0 - smoothstep(d / LIGHT_REACH);
    let sunk = smoothstep((d - SINK_FROM) / (1.0 - SINK_FROM));
    GROUND.mix(COVER_LIGHT, lit).mix(COVER_DEEP, SINK * sunk)
}

/// The cover over the window, with the hole it opens with: each piece shaded from its colour
/// and its alpha at its corners.
fn cover(p: &mut Painter, hole: &Hole) {
    if hole.up < 0.002 || hole.mid.x <= 0.0 || hole.mid.y <= 0.0 {
        return;
    }
    let rays = mesh(hole);
    let cols: Vec<Vec<Color>> = rays.iter().map(|ray| ray.iter().map(|q| shade(hole, *q).alpha(hole.cover_at(*q))).collect()).collect();
    for j in 0..rays.len() {
        let k = (j + 1) % rays.len();
        for i in 0..rays[j].len() - 1 {
            let (a, b, c, d) = (rays[j][i], rays[k][i], rays[k][i + 1], rays[j][i + 1]);
            // (rings the hole or the window laid on one another: nothing between them)
            if a.distance_squared(d) < 1e-6 && b.distance_squared(c) < 1e-6 {
                continue;
            }
            let (ca, cb, cc, cd) = (cols[j][i], cols[k][i], cols[k][i + 1], cols[j][i + 1]);
            p.tri(a, b, c, ca, cb, cc);
            p.tri(a, c, d, ca, cc, cd);
        }
    }
}

/// The road the bus drives on, across the cover at the height `y` and as much seen at each
/// place as the cover is there: ahead of the bus quiet, as the bar's steps to come; under and
/// behind it in the route's blue, as the steps done, with a little of its light round it. It
/// comes out of the dark at the window's edges.
fn road(p: &mut Painter, pass: &Pass, hole: &Hole, y: f32, run: &Run) {
    let w = pass.w;
    if hole.up < 0.002 || w <= 0.0 {
        return;
    }
    let front = pass.x_of(run.back, pass.front(run.age).clamp(0.0, w));
    let (behind, ahead) = if run.back { ((front, w), (0.0, front)) } else { ((0.0, front), (front, w)) };
    let (th, glow) = ((ROAD_W * pass.k).max(1.5), ROAD_GLOW * pass.k);
    let lit = accent().alpha(0.16);
    let quiet = TEXT.alpha(0.14);
    let seen = |x: f32| smoothstep(x / (ROAD_FADE * w)) * smoothstep((w - x) / (ROAD_FADE * w)) * hole.cover_at(Vec2::new(x, y));
    strip(p, &seen, ahead, (y, y + th), (quiet, quiet));
    strip(p, &seen, behind, (y - glow, y), (lit.alpha(0.0), lit));
    strip(p, &seen, behind, (y, y + th), (accent(), accent()));
    strip(p, &seen, behind, (y + th, y + th + glow), (lit, lit.alpha(0.0)));
}

/// A stretch of the road from `x0` to `x1` between the heights `y0` and `y1`, `c0` along its
/// top and `c1` along its foot, as much of it seen at each place as `seen` says (in steps: the
/// hole and the window's edges vary along it).
fn strip(p: &mut Painter, seen: &dyn Fn(f32) -> f32, (x0, x1): (f32, f32), (y0, y1): (f32, f32), (c0, c1): (Color, Color)) {
    if x1 - x0 < 0.01 {
        return;
    }
    let n = ((x1 - x0) / ROAD_STEP).ceil().max(1.0) as usize;
    let at = |i: usize| x0 + (x1 - x0) * i as f32 / n as f32;
    for i in 0..n {
        let (a, b) = (at(i), at(i + 1));
        let (sa, sb) = (seen(a), seen(b));
        if sa < 1e-3 && sb < 1e-3 {
            continue;
        }
        let (ta, tb, fa, fb) = (Vec2::new(a, y0), Vec2::new(b, y0), Vec2::new(a, y1), Vec2::new(b, y1));
        p.tri(ta, tb, fb, c0.alpha(sa), c0.alpha(sb), c1.alpha(sb));
        p.tri(ta, fb, fa, c0.alpha(sa), c1.alpha(sb), c1.alpha(sa));
    }
}

/// The body's pitch (radians, nose up) and how far the wheels' hubs have turned (radians)
/// `t` seconds in. The body hangs on a damped spring that the bus's speeding up pulls on: it
/// sits back as the bus pulls away, rocks forward past level once it is up to speed, and
/// settles. The hubs turn with the road as long as the eye can follow them, then at a rate it
/// can (see `HUB_TURN`).
fn motion(pass: &Pass, t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 2.0 * pass.dur);
    let n = ((t * 480.0).ceil() as usize).max(1);
    let h = t / n as f32;
    let omega = TAU * SPRING_HZ;
    let hardest = 1.5 * (pass.vmax - pass.v0) / pass.ramp;
    let (mut pitch, mut rate, mut turn) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let s = (i as f32 + 0.5) * h;
        let pull = PITCH * pass.accel(s) / hardest;
        rate += (omega * omega * (pull - pitch) - 2.0 * DAMPING * omega * rate) * h;
        pitch += rate * h;
        turn += (pass.speed(s) / (WHEEL_R * pass.k)).min(HUB_TURN) * h;
    }
    (pitch, turn)
}

/// The cover, its road and the buses of `runs` over the page (the last one holds the cover;
/// a bus out of the window is not drawn).
fn paint(ui: &mut Ui, pass: &Pass, size: Vec2, road_y: f32, runs: &[Run]) {
    let p = ui.p();
    if let Some(lead) = runs.last().filter(|r| r.ghost.is_none()) {
        let hole = Hole::new(size, pass.cover(lead.age, lead.from));
        cover(p, &hole);
        road(p, pass, &hole, road_y, lead);
    }
    for r in runs.iter().filter(|r| r.age < pass.dur) {
        let haze = r.ghost.map(|g| ((r.age - g) / GHOST_FADE).clamp(0.0, 1.0)).unwrap_or(0.0);
        bus(p, pass, road_y, r, smoothstep(haze));
    }
}

/// The bus as drawn this frame: where its rear stands on the road (on the window), which way
/// it faces, its scale, the body's pitch, and how far it has faded out (a bus whose pass
/// another change took over).
struct Body {
    at: Vec2,
    dir: f32,
    k: f32,
    pitch: f32,
    haze: f32,
}

impl Body {
    /// A point of the body, in the drawing's points (from the rear on the road, up is up),
    /// on the window: the body pitches about its middle at the axles' height.
    fn body(&self, x: f32, y: f32) -> Vec2 {
        let (px, py) = (0.5 * LEN, WHEEL_R);
        let (s, c) = self.pitch.sin_cos();
        let (dx, dy) = (x - px, y - py);
        self.road(px + dx * c - dy * s, py + dx * s + dy * c)
    }

    /// A point that does not pitch (the wheels, the shadow, the air behind the bus).
    fn road(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(self.at.x + self.dir * x * self.k, self.at.y - y * self.k)
    }

    /// `c` as far faded as the bus has: into the cover - towards the ground's colour and
    /// see-through together, so that its parts, by then alike, hardly show through each other.
    fn ink(&self, c: Color) -> Color {
        c.mix(GROUND.alpha(c.0[3]), self.haze).alpha(1.0 - self.haze)
    }

    /// A convex shape of the body, shaded from `low` at its foot to `high` at its head.
    fn fill(&self, p: &mut Painter, pts: &[Vec2], low: Color, high: Color) {
        if pts.len() < 3 {
            return;
        }
        let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.y), b.max(q.y)));
        let col = |q: Vec2| self.ink(low.mix(high, ((q.y - y0) / (y1 - y0).max(0.01)).clamp(0.0, 1.0)));
        let m = pts.iter().copied().sum::<Vec2>() / pts.len() as f32;
        let on = |q: Vec2| self.body(q.x, q.y);
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            p.tri(on(m), on(a), on(b), col(m), col(a), col(b));
        }
    }

    /// A box of the body in one colour.
    fn quad(&self, p: &mut Painter, x0: f32, y0: f32, x1: f32, y1: f32, c: Color) {
        self.fill(p, &[Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1)], c, c);
    }
}

/// A box in the drawing's points (up is up) with its corners rounded by `r`: rear foot, front
/// foot, front head, rear head.
fn rounded_box(x0: f32, y0: f32, x1: f32, y1: f32, r: [f32; 4]) -> Vec<Vec2> {
    let corners = [(Vec2::new(x0, y0), Vec2::new(1.0, 1.0), PI), (Vec2::new(x1, y0), Vec2::new(-1.0, 1.0), 1.5 * PI), (Vec2::new(x1, y1), Vec2::new(-1.0, -1.0), 0.0), (Vec2::new(x0, y1), Vec2::new(1.0, -1.0), 0.5 * PI)];
    let mut out = Vec::new();
    for ((at, inward, a0), rad) in corners.into_iter().zip(r) {
        if rad < 0.01 {
            out.push(at);
            continue;
        }
        let c = at + inward * rad;
        let n = ((rad * 0.7) as usize).clamp(3, 10);
        for i in 0..=n {
            let a = a0 + 0.5 * PI * i as f32 / n as f32;
            out.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    }
    out
}

/// The part of a circle round `c` (radius `r`) above the height `floor`.
fn segment_above(c: Vec2, r: f32, floor: f32) -> Vec<Vec2> {
    let a = ((floor - c.y) / r).clamp(-1.0, 1.0).asin();
    (0..=20).map(|i| a + (PI - 2.0 * a) * i as f32 / 20.0).map(|t| c + Vec2::new(t.cos(), t.sin()) * r).collect()
}

const BODY_LOW: Color = Color::rgba(208, 214, 225, 1.0);
const BODY_HIGH: Color = Color::rgba(248, 249, 252, 1.0);
const GLASS_LOW: Color = Color::rgba(11, 16, 27, 1.0);
const GLASS_HIGH: Color = Color::rgba(42, 54, 77, 1.0);
const FRAME: Color = Color::rgba(178, 186, 201, 1.0);
const ARCH: Color = Color::rgba(7, 9, 15, 1.0);
const TYRE: Color = Color::rgba(15, 18, 26, 1.0);
const RIM: Color = Color::rgba(150, 159, 176, 1.0);
const HUB: Color = Color::rgba(206, 211, 221, 1.0);
const BOLT: Color = Color::rgba(70, 77, 92, 1.0);
const LAMP: Color = Color::rgba(255, 246, 222, 1.0);
const BEAM: Color = Color::rgba(214, 228, 255, 1.0);
const DARK: Color = Color::rgba(24, 30, 43, 1.0);

/// One bus of a pass: its shadow and the air it leaves behind, the light ahead of it, the body
/// - white, a band of glass, two doors, the route's blue along its foot, the line's yellow on
/// its display - and its two wheels.
fn bus(p: &mut Painter, pass: &Pass, road_y: f32, run: &Run, haze: f32) {
    let t = run.age;
    let front = pass.front(t);
    let (pitch, turn) = motion(pass, t);
    let dir = if run.back { -1.0 } else { 1.0 };
    let b = Body { at: Vec2::new(pass.x_of(run.back, front - pass.len), road_y), dir, k: pass.k, pitch, haze };
    let fade = 1.0 - haze;
    let k = pass.k;
    // the shadow it stands in
    let (s0, s1) = (b.road(8.0, 0.0), b.road(LEN - 8.0, 0.0));
    p.shadow(Rect::new(s0.x.min(s1.x), road_y - 2.5 * k, (s1.x - s0.x).abs(), 5.0 * k), 2.5 * k, 18.0 * k, Color::rgba(0, 0, 0, 0.5 * fade));
    // the air behind it, as long as it is fast
    let v = pass.speed(t);
    let rush = smoothstep((v / pass.vmax - 0.45) / 0.45) * fade;
    if rush > 0.01 {
        let reach = v * STREAK_TIME / k;
        for (y, share, gap) in STREAKS {
            let (tail, head) = (b.road(-gap - reach * share, y), b.road(-gap, y));
            let th = 1.6 * k;
            let r = Rect::new(tail.x.min(head.x), head.y - 0.5 * th, (head.x - tail.x).abs(), th);
            let c = TEXT.alpha(0.3 * rush);
            if run.back {
                p.gradient_h(r, c, c.alpha(0.0));
            } else {
                p.gradient_h(r, c.alpha(0.0), c);
            }
        }
    }
    // the light ahead of it on the road: soft at its edges, and falling off faster than
    // straight (blended in linear light, a straight fade of a light colour holds its
    // brightness and then stops short, an edge where the light ends)
    let n = 8;
    for i in 0..n {
        let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let at = |u: f32| (LEN - 2.0 + 160.0 * u, 17.5 - 13.0 * u, 3.0 + 14.0 * u);
        let glow = |u: f32| BEAM.alpha(0.09 * fade * (1.0 - u).powi(3));
        let ((x0, y0, h0), (x1, y1, h1)) = (at(u0), at(u1));
        let (c0, c1) = (glow(u0), glow(u1));
        let (e0, e1) = (c0.alpha(0.0), c1.alpha(0.0));
        let (m0, m1) = (b.body(x0, y0), b.road(x1, y1));
        let m0 = if i == 0 { m0 } else { b.road(x0, y0) };
        for side in [-1.0, 1.0] {
            let (o0, o1) = (b.road(x0, y0 + side * h0), b.road(x1, y1 + side * h1));
            p.tri(m0, o0, o1, c0, e0, e1);
            p.tri(m0, o1, m1, c0, e1, c1);
        }
    }

    // the body: white, darker towards its foot
    b.fill(p, &rounded_box(0.0, 6.0, LEN, 78.0, [4.0, 6.0, 20.0, 10.0]), BODY_LOW, BODY_HIGH);
    // the roof's air conditioning
    b.fill(p, &rounded_box(104.0, 77.0, 214.0, 82.5, [0.0, 0.0, 2.5, 2.5]), Color::rgba(214, 220, 230, 1.0), Color::rgba(232, 236, 243, 1.0));
    // the route's blue along its foot
    b.fill(p, &rounded_box(0.0, 6.0, LEN, 17.0, [4.0, 6.0, 0.0, 0.0]), accent_deep(), accent());
    // the band of side windows, the windscreen round the front, and the light in the glass
    b.fill(p, &rounded_box(9.0, 40.0, 272.0, 68.0, [3.0, 0.0, 0.0, 6.0]), GLASS_LOW, GLASS_HIGH);
    let mut screen = vec![Vec2::new(275.0, 22.0), Vec2::new(297.6, 22.0)];
    screen.extend((0..=10).map(|i| Vec2::new(280.0, 58.0) + Vec2::from_angle(0.5 * PI * i as f32 / 10.0) * 17.6));
    screen.push(Vec2::new(275.0, 75.6));
    b.fill(p, &screen, GLASS_LOW, GLASS_HIGH);
    for (x, w) in [(28.0, 7.0), (39.0, 3.0), (172.0, 7.0)] {
        b.fill(p, &[Vec2::new(x, 40.0), Vec2::new(x + w, 40.0), Vec2::new(x + w + 12.0, 68.0), Vec2::new(x + 12.0, 68.0)], Color::WHITE.alpha(0.07), Color::WHITE.alpha(0.07));
    }
    b.fill(p, &[Vec2::new(278.0, 26.0), Vec2::new(283.0, 26.0), Vec2::new(292.0, 64.0), Vec2::new(287.0, 64.0)], Color::WHITE.alpha(0.08), Color::WHITE.alpha(0.08));
    // the window posts
    for x in [47.0, 89.0, 168.0, 197.0] {
        b.quad(p, x, 40.0, x + 2.6, 68.0, BODY_HIGH.mix(BODY_LOW, 0.3));
    }
    // two double doors: glass to the floor in a frame, the leaves meeting in the middle - on
    // the side they are on: going back, the bus shows its other side, windows only
    let doors: &[f32] = if run.back { &[] } else { &[131.0, 240.0] };
    if run.back {
        for x in [131.0, 240.0] {
            b.quad(p, x, 40.0, x + 2.6, 68.0, BODY_HIGH.mix(BODY_LOW, 0.3));
        }
    }
    for &x0 in doors {
        let x1 = x0 + 31.0;
        b.fill(p, &rounded_box(x0, 7.0, x1, 68.0, [0.0; 4]), GLASS_LOW, GLASS_HIGH);
        let c = FRAME;
        b.quad(p, x0, 7.0, x0 + 1.8, 68.0, c);
        b.quad(p, x1 - 1.8, 7.0, x1, 68.0, c);
        b.quad(p, x0, 66.4, x1, 68.0, c);
        b.quad(p, x0 + 0.5 * (x1 - x0) - 0.7, 7.0, x0 + 0.5 * (x1 - x0) + 0.7, 68.0, c);
        b.quad(p, x0, 33.0, x1, 34.2, c);
    }
    // the side display: the line in the line's yellow
    b.quad(p, 200.0, 57.0, 236.0, 65.6, DARK.darken(0.4));
    for (x0, x1) in [(202.5, 206.0), (208.0, 215.5), (217.5, 221.0), (223.0, 233.5)] {
        b.quad(p, x0, 59.6, x1, 63.0, LINE);
    }
    // the lamps: the head lamp at the front's foot, the tail lamp at the rear
    b.fill(p, &rounded_box(292.5, 14.0, 299.4, 21.0, [0.0, 2.5, 2.0, 0.0]), LAMP, LAMP);
    b.fill(p, &rounded_box(0.6, 22.0, 4.2, 38.0, [1.0, 0.0, 0.0, 1.0]), LATE, LATE.lighten(0.15));
    // the mirror on its arm in front of the windscreen
    p.line(b.body(298.0, 66.0), b.body(305.0, 69.5), 1.5 * k, b.ink(DARK));
    b.fill(p, &rounded_box(303.5, 56.0, 307.5, 70.5, [1.5; 4]), DARK, DARK.lighten(0.12));

    // the wheels in their arches; the hubs turn, and blur as the bus goes faster
    let spin = smoothstep((v / (WHEEL_R * k) - HUB_TURN) / (4.0 * HUB_TURN)) * 0.65;
    for ax in AXLES {
        b.fill(p, &segment_above(Vec2::new(ax, WHEEL_R), WHEEL_R + 4.5, 6.0), ARCH, ARCH);
        let c = b.road(ax, WHEEL_R);
        let r = WHEEL_R * k;
        p.circle(c, r, b.ink(TYRE));
        p.arc(c, r * 0.8, r * 0.86, 0.0, TAU, b.ink(Color::rgba(38, 44, 58, 1.0)));
        p.circle(c, r * 0.6, b.ink(RIM));
        for i in 0..5 {
            let a = -turn + TAU * i as f32 / 5.0;
            let q = b.road(ax + a.cos() * WHEEL_R * 0.43, WHEEL_R + a.sin() * WHEEL_R * 0.43);
            p.circle(q, r * 0.085, b.ink(BOLT).alpha(1.0 - spin));
        }
        p.arc(c, r * 0.33, r * 0.53, 0.0, TAU, b.ink(RIM.mix(BOLT, 0.35)).alpha(spin));
        p.circle(c, r * 0.28, b.ink(HUB));
        p.circle(c, r * 0.1, b.ink(BOLT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new screen fades in from the ground: all of it at first, none after `FADE_S`, and
    /// less with every moment between.
    #[test]
    fn a_new_screen_fades_in() {
        assert_eq!(fade_cover(0.0), 1.0);
        assert_eq!(fade_cover(FADE_S), 0.0);
        let mut last = 1.0;
        for k in 1..10 {
            let c = fade_cover(FADE_S * k as f32 / 10.0);
            assert!(c < last && c > 0.0, "{k}: {c}");
            last = c;
        }
    }
    use super::super::ui::Key;

    const DESKTOP: Vec2 = Vec2::new(1440.0, 900.0);
    const WIDE: Vec2 = Vec2::new(2560.0, 1347.0);
    const PHONE: Vec2 = Vec2::new(420.0, 900.0);
    const SIZES: [(Vec2, bool); 5] = [(DESKTOP, false), (WIDE, false), (Vec2::new(1080.0, 680.0), false), (Vec2::new(1067.0, 667.0), true), (PHONE, true)];

    /// How far the last of the bus that is drawn - the far end of the air behind it - has
    /// come `t` seconds in.
    fn last_seen(p: &Pass, t: f32) -> f32 {
        p.front(t) - p.len - trail(p.speed(t), p.k)
    }

    #[test]
    fn a_pass_crosses_in_under_a_second_and_takes_what_trails_the_bus_out_too() {
        for (size, phone) in SIZES {
            let p = Pass::new(size, phone);
            assert!((0.4..=0.65).contains(&p.dur), "{size}: {}", p.dur);
            assert!(p.front(0.0).abs() < 1e-3);
            let travel = size.x + p.len + trail(p.vmax, p.k);
            assert!((p.front(p.dur) - travel).abs() < 0.5, "{size}: {} of {travel}", p.front(p.dur));
            // it never stands still or goes back, and it never jumps
            let mut last = -1.0;
            for i in 0..=400 {
                let t = p.dur * i as f32 / 400.0;
                let d = p.front(t);
                assert!(d > last, "{size} at {t}");
                assert!(d - last < p.vmax * p.dur / 400.0 + 0.5 || i == 0);
                last = d;
            }
            // the bus fits the window it crosses
            assert!(p.len < 0.6 * size.x, "{size}");
            // what trails it is the streaks at its top speed, with a little room
            assert!(trail(p.vmax, p.k) > STREAK_TIME * p.vmax && trail(p.vmax, p.k) < STREAK_TIME * p.vmax + 30.0 * p.k);
        }
        assert!((Pass::new(DESKTOP, false).dur - 0.55).abs() < 1e-3);
        assert!(Pass::new(DESKTOP, true).dur < Pass::new(DESKTOP, false).dur);
    }

    #[test]
    fn the_speed_is_smooth_and_matches_the_way() {
        let p = Pass::new(DESKTOP, false);
        assert!((p.speed(0.0) - p.v0).abs() < 1e-3);
        assert!((p.speed(p.ramp - 1e-4) - p.vmax).abs() < 1.0);
        assert!((p.speed(p.dur) - p.vmax).abs() < 1e-3);
        assert!(p.accel(0.0).abs() < 1e-3 && p.accel(p.ramp).abs() < 1e-3 && p.accel(0.5 * p.ramp) > 0.0);
        // the way is what the speed adds up to
        let h = 1e-3;
        for t in [0.05, 0.2, p.ramp - 0.01, p.ramp + 0.01, 0.7] {
            let v = (p.front(t + h) - p.front(t - h)) / (2.0 * h);
            assert!((v - p.speed(t)).abs() < 0.01 * p.vmax, "at {t}: {v} against {}", p.speed(t));
        }
    }

    #[test]
    fn the_cover_comes_up_fast_stays_up_while_the_bus_crosses_and_opens_once_it_is_out() {
        for (size, phone) in SIZES {
            let p = Pass::new(size, phone);
            let c = |t: f32| p.cover(t, Cover::NONE);
            // the frame of the change shows it beginning (a click is answered at once), the
            // next one most of the way, and it is up in a tenth of a second
            assert!((0.3..0.6).contains(&c(0.0).up), "{}", c(0.0).up);
            assert!(c(1.0 / 60.0).up > 0.65);
            assert_eq!(c(COVER_IN - FIRST_FRAME), Cover { up: 1.0, open: 0.0 });
            // coming up it only comes up, then it stands closed while the bus crosses, and
            // then it only opens - at 60 frames a second never a jolt while it comes up
            let mut last = c(0.0);
            for i in 1..=2000 {
                let t = p.end() * i as f32 / 2000.0;
                let now = c(t);
                if t < p.dur {
                    assert!(now.up >= last.up && now.open == 0.0, "{size} at {t}");
                    assert!(t < COVER_IN || now.up == 1.0, "{size} at {t}");
                } else {
                    assert!(now.up == 1.0 && now.open >= last.open, "{size} at {t}");
                }
                last = now;
            }
            assert_eq!(c(p.end()).open, 1.0);
            assert_eq!(c(p.end() + 1.0).open, 1.0);
            // it opens as the last of the bus - the air behind it - leaves the window, not later
            assert!((last_seen(&p, p.dur) - size.x).abs() < 0.5, "{size}: {}", last_seen(&p, p.dur));
            assert!(last_seen(&p, p.dur - 0.03) < size.x - 20.0, "{size}");
            // and the bus crosses the middle well inside the crossing
            let middle = (0..=1000).map(|i| p.dur * i as f32 / 1000.0).find(|t| p.front(*t) - 0.5 * p.len >= 0.5 * size.x).unwrap();
            assert!(middle > COVER_IN && middle < 0.8 * p.dur, "{size}: {middle}");
            // it opens in about a sixth of a second
            assert!((0.14..=0.18).contains(&(p.end() - p.dur)));
        }
        // a whole pass: under four fifths of a second on a desktop, less on a phone
        assert!((0.65..=0.8).contains(&Pass::new(DESKTOP, false).end()));
        assert!((0.65..=0.8).contains(&Pass::new(WIDE, false).end()));
        assert!(Pass::new(PHONE, true).end() < 0.65);
    }

    /// Places on a window: its middle, the middles of its sides, its corners.
    fn places(size: Vec2) -> ([Vec2; 1], [Vec2; 4], [Vec2; 4]) {
        let (w, h) = (size.x, size.y);
        ([0.5 * size], [Vec2::new(0.5 * w, 0.0), Vec2::new(w, 0.5 * h), Vec2::new(0.5 * w, h), Vec2::new(0.0, 0.5 * h)], [Vec2::ZERO, Vec2::new(w, 0.0), size, Vec2::new(0.0, h)])
    }

    #[test]
    fn the_cover_pops_open_from_the_middle_and_clears_the_window() {
        for (size, phone) in SIZES {
            let p = Pass::new(size, phone);
            let at = |t: f32, q: Vec2| Hole::new(size, p.cover(t, Cover::NONE)).cover_at(q);
            let (mid, sides, corners) = places(size);
            let all: Vec<Vec2> = mid.iter().chain(&sides).chain(&corners).copied().collect();
            // up and closed until the bus is out: everything hidden whole
            for t in [COVER_IN, 0.5 * p.dur, p.dur - 1e-3] {
                for q in &all {
                    assert!(at(t, *q) > 0.9999, "{size} at {t}, {q}");
                }
            }
            // it opens from the middle: the middle clears first, the sides' middles next, the
            // corners last - by the end of the pop
            let clear = |q: Vec2| (0..=1000).map(|i| p.dur + POP * i as f32 / 1000.0).find(|t| at(*t, q) < 0.02).unwrap_or(f32::MAX);
            let side = sides.iter().map(|q| clear(*q)).fold(0.0, f32::max);
            assert!(clear(mid[0]) < sides.iter().map(|q| clear(*q)).fold(f32::MAX, f32::min), "{size}");
            assert!(corners.iter().all(|q| clear(*q) > side && clear(*q) <= p.end()), "{size}");
            // a pop: a third of the way in, the middle half of the window is clear
            let t = p.dur + POP / 3.0;
            for i in 0..=8 {
                for j in 0..=8 {
                    let q = size * (Vec2::new(i as f32, j as f32) / 16.0 + 0.25);
                    assert!(at(t, q) < 0.05, "{size} at {q}: {}", at(t, q));
                }
            }
            // no place is hidden again as it opens, and once it is over nothing is hidden
            for q in &all {
                let mut last = 1.0;
                for i in 0..=200 {
                    let a = at(p.dur + POP * i as f32 / 200.0, *q);
                    assert!(a <= last + 1e-6, "{size} at {q}");
                    last = a;
                }
                assert!(at(p.end(), *q) < 1e-6);
            }
        }
    }

    #[test]
    fn the_pop_is_seen_the_whole_way_fast_at_first_and_settling() {
        assert_eq!(opening(0.0), 0.0);
        assert_eq!(opening(1.0), 1.0);
        for (size, phone) in SIZES {
            let p = Pass::new(size, phone);
            // how much of the window the cover hides, frame by frame at 60 a second
            let hidden = |t: f32| {
                let hole = Hole::new(size, p.cover(t, Cover::NONE));
                let (n, m) = (40, 25);
                (0..n * m).map(|i| hole.cover_at(size * Vec2::new((i % n) as f32 + 0.5, (i / n) as f32 + 0.5) / Vec2::new(n as f32, m as f32))).sum::<f32>() / (n * m) as f32
            };
            let frames: Vec<f32> = (0..=(POP * 60.0).ceil() as usize).map(|f| hidden(p.dur + f as f32 / 60.0)).collect();
            // the first frame already opens it (a pop), no frame shows a great deal at once,
            // and past half the pop something is still going
            assert!(frames[0] > 0.999 && frames[1] < 0.95, "{size}: {frames:?}");
            assert!(frames.windows(2).all(|w| w[1] <= w[0] + 1e-6 && w[0] - w[1] < 0.2), "{size}: {frames:?}");
            assert!(hidden(p.dur + 0.6 * POP) > 0.015, "{size}: {frames:?}");
        }
    }

    #[test]
    fn the_cover_lies_once_over_the_whole_window_round_its_clear_middle() {
        for (size, phone) in SIZES {
            let window = size.x * size.y;
            let _ = phone;
            for open in [0.0, 0.03, 0.15, 0.4, 0.7] {
                let hole = Hole::new(size, Cover { up: 1.0, open });
                let rays = mesh(&hole);
                let mut ui = Ui::new();
                ui.begin(size, 1.0, 1.0 / 60.0);
                cover(ui.p(), &hole);
                let pieces = tris(ui.p());
                // the pieces and the clear middle they go round add up to the window: every
                // point of it under one piece, or clear
                let area: f32 = pieces.iter().map(|(q, _)| 0.5 * (q[1] - q[0]).perp_dot(q[2] - q[0]).abs()).sum();
                let inner: Vec<Vec2> = rays.iter().map(|r| r[0]).collect();
                let middle = 0.5 * (0..inner.len()).map(|i| inner[i].perp_dot(inner[(i + 1) % inner.len()])).sum::<f32>().abs();
                assert!((area + middle - window).abs() < 2e-3 * window, "{size} open {open}: {area} + {middle} against {window}");
                assert!(pieces.iter().all(|(q, _)| q.iter().all(|v| v.x >= -1e-2 && v.y >= -1e-2 && v.x <= size.x + 1e-2 && v.y <= size.y + 1e-2)));
                // the middle it goes round is clear indeed (where the hole has one), and
                // closed there is none
                assert!(inner.iter().filter(|q| q.distance(0.5 * size) > 0.01).all(|q| hole.cover_at(*q) < 1e-4), "{size} open {open}");
                if open == 0.0 {
                    assert!(middle < 1e-3);
                    assert!(pieces.iter().all(|(_, a)| a.iter().all(|a| *a > 0.9999)));
                }
                // the outermost points go round the window's edge
                for r in &rays {
                    let q = *r.last().unwrap();
                    assert!(q.x.abs() < 0.05 || (q.x - size.x).abs() < 0.05 || q.y.abs() < 0.05 || (q.y - size.y).abs() < 0.05, "{size}: {q}");
                }
            }
        }
    }

    #[test]
    fn the_cover_is_the_ground_lit_softly_in_the_middle_and_deeper_in_the_corners() {
        let lum = |c: Color| 0.2126 * c.0[0] + 0.7152 * c.0[1] + 0.0722 * c.0[2];
        for (size, _) in SIZES {
            let hole = Hole::new(size, Cover { up: 1.0, open: 0.0 });
            let (mid, corner) = (shade(&hole, 0.5 * size), shade(&hole, Vec2::ZERO));
            assert!(lum(mid) > lum(GROUND) && lum(corner) < lum(GROUND), "{size}");
            // darker outwards, without a step
            let mut last = lum(mid);
            for i in 0..=60 {
                let l = lum(shade(&hole, 0.5 * size * (1.0 - i as f32 / 60.0)));
                assert!(l <= last + 1e-6 && last - l < 0.006, "{size} at {i}");
                last = l;
            }
            // still the night: a dark blue, never a lit screen
            assert!(mid.0[2] > mid.0[0] && mid.0[2] < 0.3 && lum(mid) < 0.15);
        }
        // half up, it hides half as the eye sees it: a light text under a dark cover, blended
        // in linear light, keeps about half its brightness
        let lin = |c: f32| ((c + 0.055) / 1.055).powf(2.4);
        let srgb = |l: f32| 1.055 * l.powf(1.0 / 2.4) - 0.055;
        for c in [0.5, 0.91] {
            let seen = srgb(lin(c) * (1.0 - alpha_for(0.5)));
            assert!((seen - 0.5 * c).abs() < 0.03, "{c}: {seen}");
        }
        assert!(alpha_for(0.0).abs() < 1e-6 && alpha_for(1.0) > 0.999);
    }

    #[test]
    fn the_road_is_quiet_ahead_lit_behind_and_hides_with_the_cover() {
        let pass = Pass::new(DESKTOP, false);
        let y = 500.0;
        for back in [false, true] {
            let run = Run { age: 0.5 * pass.dur, back, from: Cover::NONE, ghost: None };
            let front = pass.x_of(back, pass.front(run.age));
            let hole = Hole::new(DESKTOP, pass.cover(run.age, run.from));
            let mut p = Painter::new();
            road(&mut p, &pass, &hole, y, &run);
            let pieces: Vec<([Vec2; 3], [f32; 4])> = p.verts.chunks(3).map(|t| ([0, 1, 2].map(|i| Vec2::new(t[i].pos[0], t[i].pos[1])), t[0].color)).collect();
            // on the line itself (not its glow): the accent on the side the bus came from, quiet
            // ahead
            let on_line = |x: f32| pieces.iter().filter(|(q, _)| q.iter().all(|v| v.y >= y - 0.01) && q.iter().all(|v| v.y <= y + 2.01)).find(|(q, _)| q.iter().map(|v| v.x).fold(f32::MAX, f32::min) <= x && q.iter().map(|v| v.x).fold(f32::MIN, f32::max) >= x).map(|(_, c)| *c);
            let (driven, to_go) = if back { (front + 200.0, front - 200.0) } else { (front - 200.0, front + 200.0) };
            let blue = on_line(driven).unwrap();
            let quiet = on_line(to_go).unwrap();
            let accent = crate::accent::base().0;
            assert!((0..3).all(|i| (blue[i] - accent[i]).abs() < 0.02) && blue[3] > 0.95, "{blue:?}");
            assert!((quiet[0] - quiet[2]).abs() < 0.1 && quiet[3] < 0.2, "{quiet:?}");
            // it comes out of the dark at the window's edges
            assert!(pieces.iter().all(|(q, _)| q.iter().all(|v| v.x >= 0.0 && v.x <= DESKTOP.x)));
        }
        // the cover open in the middle: no road there
        let run = Run { age: pass.dur + 0.1, back: false, from: Cover::NONE, ghost: None };
        let hole = Hole::new(DESKTOP, pass.cover(run.age, run.from));
        let mut p = Painter::new();
        road(&mut p, &pass, &hole, 0.5 * DESKTOP.y, &run);
        assert!(p.verts.iter().filter(|v| (v.pos[0] - 0.5 * DESKTOP.x).abs() < 100.0).all(|v| v.color[3] < 1e-3));
    }

    /// The triangles a painter holds: their corners and the alpha at each.
    fn tris(p: &Painter) -> Vec<([Vec2; 3], [f32; 3])> {
        p.verts.chunks(3).map(|t| ([0, 1, 2].map(|i| Vec2::new(t[i].pos[0], t[i].pos[1])), [0, 1, 2].map(|i| t[i].color[3]))).collect()
    }

    #[test]
    fn the_body_sits_back_pulling_away_and_settles() {
        let p = Pass::new(DESKTOP, false);
        let (pull, _) = motion(&p, 0.5 * p.ramp);
        assert!(pull > 0.2 * PITCH, "{pull}");
        let mut most = 0.0f32;
        let mut least = 0.0f32;
        for i in 0..=100 {
            let (pitch, _) = motion(&p, p.dur * i as f32 / 100.0);
            most = most.max(pitch);
            least = least.min(pitch);
        }
        // a little overshoot past level once up to speed, never a lurch
        assert!(most < 2.0 * PITCH && most > 0.5 * PITCH, "{most}");
        assert!(least < 0.0 && least > -most, "{least}");
        let (end, _) = motion(&p, p.dur);
        assert!(end.abs() < 0.5 * most, "{end}");
    }

    #[test]
    fn the_hubs_turn_forwards_at_a_rate_the_eye_can_follow() {
        let p = Pass::new(DESKTOP, false);
        let mut last = 0.0;
        for i in 1..=60 {
            let t = p.dur * i as f32 / 60.0;
            let (_, turn) = motion(&p, t);
            assert!(turn > last);
            // at 60 frames a second a bolt moves less than a fifth of the way to the next
            assert!(turn - last <= HUB_TURN * p.dur / 60.0 + 1e-3);
            assert!(HUB_TURN / 60.0 < 0.2 * TAU / 5.0);
            last = turn;
        }
    }

    #[test]
    fn going_back_is_going_to_an_earlier_place() {
        let back = |a: Screen, b: Screen| b.place() < a.place();
        assert!(!back(Screen::Step(Step::Mode), Screen::Step(Step::Map)));
        assert!(back(Screen::Step(Step::Bus), Screen::Step(Step::Mode)));
        assert!(back(Screen::Step(Step::Mode), Screen::Step(Step::Profile)));
        assert!(!back(Screen::Step(Step::Mode), Screen::Page(Page::Settings)));
        assert!(back(Screen::Page(Page::Settings), Screen::Step(Step::Mode)));
        assert!(!back(Screen::Page(Page::Settings), Screen::Step(Step::Map)));
        assert!(!back(Screen::Page(Page::Settings), Screen::Page(Page::Mods)));
        assert!(back(Screen::Page(Page::Profile), Screen::Step(Step::Profile)));
        assert!(!back(Screen::Step(Step::Profile), Screen::Page(Page::Profile)));
        assert!(!back(Screen::Phone(Tab::Play, None, None), Screen::Phone(Tab::Play, None, Some(Sheet::Map))));
        assert!(back(Screen::Phone(Tab::More, Some(Page::Settings), None), Screen::Phone(Tab::More, None, None)));
        assert!(back(Screen::Phone(Tab::Mods, None, None), Screen::Phone(Tab::Play, None, None)));
    }

    fn quiet() -> Transition {
        Transition { last: None, runs: Vec::new(), mouse: None, freeze: None, slow: 1.0, fade_last: None, fade: None }
    }

    /// A frame of the launcher with the screen `now`: the clock goes on, the change is seen,
    /// what is over goes.
    fn frame(tr: &mut Transition, pass: &Pass, now: Screen) {
        tr.advance(1.0 / 60.0);
        tr.see(now, pass, true);
        tr.tidy(pass);
    }

    #[test]
    fn a_change_hides_the_page_until_the_bus_is_out_and_gives_it_back_as_the_cover_opens() {
        for (size, phone) in SIZES {
            let pass = Pass::new(size, phone);
            let mut tr = quiet();
            // the first frame is no change (the launcher opening is the intro's)
            tr.see(Screen::Step(Step::Mode), &pass, true);
            assert!(tr.runs.is_empty());
            tr.see(Screen::Step(Step::Mode), &pass, true);
            assert!(tr.runs.is_empty());
            tr.see(Screen::Step(Step::Map), &pass, true);
            assert_eq!(tr.runs.len(), 1);
            assert!(!tr.runs[0].back && tr.runs[0].from == Cover::NONE);
            assert!(tr.covering(&pass));
            let mut t = 0.0;
            while t < pass.end() + 0.2 {
                frame(&mut tr, &pass, Screen::Step(Step::Map));
                t += 1.0 / 60.0;
                assert_eq!(tr.covering(&pass), t < pass.dur, "{size} at {t}");
                assert_eq!(tr.runs.is_empty(), t >= pass.end(), "{size} at {t}");
            }
        }
    }

    #[test]
    fn under_the_cover_the_page_gets_no_click_key_wheel_or_mouse_and_then_the_mouse_back() {
        let mut i = Input { mouse: Vec2::new(300.0, 200.0), down: true, pressed: true, released: true, right_pressed: true, double_click: true, wheel: Vec2::new(0.0, 3.0), raw_key: Some(winit::keyboard::KeyCode::KeyA), ..Default::default() };
        i.text.push('a');
        i.keys.push(Key::Enter);
        let at = hold(&mut i);
        assert!(!i.pressed && !i.released && !i.right_pressed && !i.double_click);
        assert!(i.wheel == Vec2::ZERO && i.text.is_empty() && i.keys.is_empty() && i.raw_key.is_none());
        assert!(!Rect::new(-1e4, -1e4, 2e4, 2e4).contains(i.mouse));
        // (a button held stays held: nothing starts to drag without a press)
        assert!(i.down);
        give_back(&mut i, at);
        assert_eq!(i.mouse, Vec2::new(300.0, 200.0));
        // moved while the page could not see it: where it went
        let at = hold(&mut i);
        i.mouse = Vec2::new(5.0, 6.0);
        give_back(&mut i, at);
        assert_eq!(i.mouse, Vec2::new(5.0, 6.0));
    }

    #[test]
    fn with_the_animations_off_there_is_no_cover_and_no_bus() {
        let pass = Pass::new(DESKTOP, false);
        let mut tr = quiet();
        tr.see(Screen::Step(Step::Mode), &pass, false);
        tr.see(Screen::Step(Step::Map), &pass, false);
        assert!(tr.runs.is_empty() && !tr.covering(&pass));
        // turned off during a pass: it goes at once, and the page has its input
        tr.see(Screen::Step(Step::Mode), &pass, true);
        assert!(tr.covering(&pass));
        tr.see(Screen::Step(Step::Mode), &pass, false);
        assert!(tr.runs.is_empty() && !tr.covering(&pass));
    }

    #[test]
    fn a_change_during_a_pass_keeps_the_cover_up_and_the_bus_on_its_way_fades() {
        let pass = Pass::new(DESKTOP, false);
        let (mid, sides, corners) = places(DESKTOP);
        let probes: Vec<Vec2> = mid.iter().chain(&sides).chain(&corners).copied().chain([Vec2::new(400.0, 300.0)]).collect();
        let hidden = |tr: &Transition| -> Vec<f32> {
            let r = tr.lead().unwrap();
            let hole = Hole::new(DESKTOP, pass.cover(r.age, r.from));
            probes.iter().map(|q| hole.cover_at(*q)).collect()
        };
        // a change coming up, while the bus crosses, as the cover opens, nearly open
        for at in [0.03, 0.5 * pass.dur, pass.dur + 0.25 * POP, pass.dur + 0.7 * POP] {
            let mut tr = quiet();
            tr.see(Screen::Step(Step::Mode), &pass, true);
            tr.see(Screen::Step(Step::Map), &pass, true);
            while tr.runs[0].age < at {
                frame(&mut tr, &pass, Screen::Step(Step::Map));
            }
            let before = hidden(&tr);
            tr.see(Screen::Step(Step::Mode), &pass, true);
            tr.tidy(&pass);
            // the new bus comes back from the right; the one on its way, if still in sight,
            // drives on
            assert!(tr.lead().is_some_and(|r| r.back && r.age == 0.0), "at {at}");
            assert_eq!(tr.runs.len(), if at < pass.dur { 2 } else { 1 }, "at {at}");
            assert!(tr.covering(&pass));
            // no dip: every place at least as hidden as it was, and more and more until all
            // of it is
            let mut last = before;
            for _ in 0..8 {
                let now = hidden(&tr);
                assert!(now.iter().zip(&last).all(|(n, l)| *n >= l - 1e-5), "at {at}: {now:?} after {last:?}");
                last = now;
                frame(&mut tr, &pass, Screen::Step(Step::Mode));
            }
            assert!(last.iter().all(|a| *a > 0.9999), "at {at}: {last:?}");
            // the bus that was on its way fades out and is gone
            for _ in 0..12 {
                frame(&mut tr, &pass, Screen::Step(Step::Mode));
            }
            assert_eq!(tr.runs.len(), 1, "at {at}");
        }
        // a third change keeps only the last two buses
        let mut tr = quiet();
        tr.see(Screen::Step(Step::Mode), &pass, true);
        tr.see(Screen::Step(Step::Map), &pass, true);
        frame(&mut tr, &pass, Screen::Step(Step::Map));
        tr.see(Screen::Step(Step::Mode), &pass, true);
        for _ in 0..10 {
            frame(&mut tr, &pass, Screen::Step(Step::Mode));
        }
        tr.see(Screen::Page(Page::Settings), &pass, true);
        assert_eq!(tr.runs.len(), 2);
        assert!(tr.lead().is_some_and(|r| !r.back && r.from.up == 1.0));
    }

    #[test]
    fn stood_still_for_a_picture_a_pass_looks_as_one_from_a_quiet_screen() {
        let pass = Pass::new(DESKTOP, false);
        let mut tr = Transition { last: None, runs: Vec::new(), mouse: None, freeze: Some(0.05), slow: 1.0, fade_last: None, fade: None };
        tr.see(Screen::Step(Step::Mode), &pass, true);
        tr.see(Screen::Step(Step::Map), &pass, true);
        tr.advance(1.0 / 60.0);
        tr.see(Screen::Step(Step::Duty), &pass, true);
        assert_eq!(tr.runs.len(), 1);
        assert!(tr.runs[0].from == Cover::NONE && tr.runs[0].age == 0.05);
        tr.advance(5.0);
        tr.tidy(&pass);
        assert_eq!(tr.runs.len(), 1);
        assert!(tr.covering(&pass));
        // held in the pop, the page has its input
        let mut tr = Transition { last: None, runs: Vec::new(), mouse: None, freeze: Some(pass.dur + 0.1), slow: 1.0, fade_last: None, fade: None };
        tr.see(Screen::Step(Step::Mode), &pass, true);
        tr.see(Screen::Step(Step::Map), &pass, true);
        tr.advance(1.0 / 60.0);
        assert!(!tr.covering(&pass) && tr.runs.len() == 1);
    }

    #[test]
    fn an_interrupted_bus_fades_out_whole() {
        let at = |haze: f32| Body { at: Vec2::ZERO, dir: 1.0, k: 1.0, pitch: 0.0, haze };
        assert_eq!(at(0.0).ink(LINE), LINE);
        assert!(at(1.0).ink(LINE).0[3] < 1e-6);
        let half = at(0.5).ink(LINE);
        assert!((half.0[3] - 0.5).abs() < 1e-6 && half.0[2] < LINE.0[2] && half.0[0] < LINE.0[0]);
        // its see-through parts stay as see-through as they were, relatively
        assert!((at(0.5).ink(LINE.alpha(0.4)).0[3] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn every_moment_of_a_pass_draws_whole_numbers() {
        for (size, phone) in SIZES {
            let pass = Pass::new(size, phone);
            for back in [false, true] {
                for i in 0..=24 {
                    let t = pass.end() * i as f32 / 24.0;
                    let mut ui = Ui::new();
                    ui.begin(size, 1.0, 1.0 / 60.0);
                    let runs = [Run { age: (t - 0.3).max(0.0), back: !back, from: Cover::NONE, ghost: Some(0.1) }, Run { age: t, back, from: Cover { up: 0.4, open: 0.3 }, ghost: None }];
                    paint(&mut ui, &pass, size, size.y * 0.5 + ROAD_DROP * pass.k, &runs);
                    let verts = &ui.p().verts;
                    assert!(!verts.is_empty() || t >= pass.dur);
                    assert!(verts.iter().all(|v| v.pos.iter().chain(v.color.iter()).all(|x| x.is_finite())), "{size} at {t}");
                    assert!(verts.iter().all(|v| (0.0..=1.0).contains(&v.color[3])), "{size} at {t}");
                }
            }
        }
    }

    #[test]
    fn the_drawing_holds_together() {
        // the arches stay in the body, the wheels clear of the doors (131-162, 240-271)
        for ax in AXLES {
            let arch = segment_above(Vec2::new(ax, WHEEL_R), WHEEL_R + 4.5, 6.0);
            assert!(arch.iter().all(|q| q.y >= 6.0 - 1e-3 && q.x > 0.0 && q.x < LEN));
            for (d0, d1) in [(131.0, 162.0), (240.0, 271.0)] {
                assert!(ax + WHEEL_R < d0 || ax - WHEEL_R > d1, "axle {ax}");
            }
        }
        let b = rounded_box(0.0, 6.0, LEN, 78.0, [4.0, 6.0, 20.0, 10.0]);
        assert!(b.iter().all(|q| q.x >= -1e-3 && q.x <= LEN + 1e-3 && q.y >= 6.0 - 1e-3 && q.y <= 78.0 + 1e-3));
        // the body pitches about its middle: a nose-up pitch lifts the front, drops the rear,
        // whichever way the bus faces
        for dir in [1.0, -1.0] {
            let body = Body { at: Vec2::new(500.0, 400.0), dir, k: 1.0, pitch: 0.02, haze: 0.0 };
            let (rear, nose) = (body.body(0.0, WHEEL_R), body.body(LEN, WHEEL_R));
            assert!(nose.y < 400.0 - WHEEL_R && rear.y > 400.0 - WHEEL_R);
            assert!((nose.x - rear.x).signum() == dir);
        }
        // a hole's edge found along a ray lies on that edge
        for (b, r) in [(Vec2::new(300.0, 200.0), 100.0), (Vec2::new(300.0, 200.0), 0.0), (Vec2::new(50.0, 80.0), 50.0)] {
            for i in 0..=32 {
                let d = Vec2::from_angle(0.5 * PI * i as f32 / 32.0);
                let t = ray_box(d.abs(), b, r);
                assert!(sd_box(d * t, b, r).abs() < 1e-2, "{b} {r} along {d}: {}", sd_box(d * t, b, r));
            }
        }
    }
}
