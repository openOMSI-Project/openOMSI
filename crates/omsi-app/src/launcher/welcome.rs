//! The welcome: what a player sees the first time the launcher opens, in place of the start
//! (the opening plays over it first). In the order Luc gave it: the language - every one
//! openOMSI speaks, as tiles with their flags, the choice spoken at once so that the next
//! screens are in it; a dialog over that asking for the new interface or the classic one; the
//! animations, on or off, with a little scene that shows what "on" means; the OMSI 2 folder
//! when none was found (nothing can be driven without it); the driver - a new one, or one
//! already on this computer (many players come from openOMSI and have one); and the way on -
//! a short tour of the launcher or straight to the start, with the game's own driving lessons
//! for whoever has never driven a bus in OMSI.
//!
//! It lies under a bar of its own steps (the setup's bar, with the welcome's steps), on the
//! start's ground, as one sheet in the middle of the window whose buttons stand under the
//! question: there is nothing else to do here than answer (Omsi-Hub's `Welkom`). Going on, the
//! next step slides in from the side it lies on and fades up out of the sheet, the sheet grows
//! or shrinks to what the step needs, and the dialog comes up out of a soft dark veil - all
//! sprung, and none of it with the setting "animations" off. Finishing, the window dips into
//! the ground's dark and the start comes up out of it.
//!
//! The setting `welcome_done` says whether it was seen: it is written when the player
//! finishes or skips it (the Settings page's General tab shows it again). A choice for the
//! classic launcher opens that one then: the launcher starts again (`restart_launcher`).
//!
//! For looking at it: `OMSI_LAUNCHER_WELCOME=driver` opens it at that step whatever the
//! setting says (language, interface, animations, folder, driver, ready), `=off` never.

use super::flow;
use super::theme::*;
use super::ui::{ease_in_out_cubic, ease_out_cubic, id_of, smoothstep, spring_step, ButtonKind, Feel, Input, Key, Ui};
use super::{mobile, Launcher, Page};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Painter, Rect, Weight};
use serde_json::json;
use std::f32::consts::{PI, TAU};

/// The steps of the welcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Language,
    /// The dialog over the language: the new interface or the classic one.
    Interface,
    Animations,
    /// Only when the OMSI 2 folder was not found or is not a whole installation.
    Folder,
    Driver,
    Ready,
}

const ALL: [Step; 6] = [Step::Language, Step::Interface, Step::Animations, Step::Folder, Step::Driver, Step::Ready];

impl Step {
    /// Its word and icon in the bar.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            Step::Language => ("Language", "language"),
            Step::Interface => ("Interface", "grid_view"),
            Step::Animations => ("Animations", "bolt"),
            Step::Folder => ("Game folder", "folder_open"),
            Step::Driver => ("Your driver", "person"),
            Step::Ready => ("Ready", "sports_score"),
        }
    }

    /// The step a word names (`OMSI_LAUNCHER_WELCOME`).
    fn parse(s: &str) -> Option<Step> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "language" | "1" => Step::Language,
            "interface" | "ui" | "2" => Step::Interface,
            "animations" | "3" => Step::Animations,
            "folder" => Step::Folder,
            "driver" | "profile" => Step::Driver,
            "ready" | "done" => Step::Ready,
            _ => return None,
        })
    }
}

/// The steps in their order: the folder's only when it has to be pointed to.
pub fn steps(folder: bool) -> Vec<Step> {
    ALL.into_iter().filter(|s| folder || *s != Step::Folder).collect()
}

/// What the sheet shows on a step: the interface's dialog lies over the language's sheet.
fn sheet_step(s: Step) -> Step {
    if s == Step::Interface {
        Step::Language
    } else {
        s
    }
}

/// What comes after the welcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Then {
    /// The start (also what "Skip" leads to).
    Start,
    /// The start, and the tour through the launcher on it.
    Tour,
    /// OMSI 2's driving lessons (the Tutorials page).
    Lessons,
}

/// Where the launcher goes once the welcome is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The classic launcher was chosen: this one closes and opens again as that one.
    Restart,
    /// This page (the start, the lessons, or Setup when there is still no game to drive),
    /// with the tour on it or not.
    Show { page: Page, tour: bool },
}

/// Where the launcher goes once the welcome is over: the classic launcher is a start of its
/// own; without an OMSI 2 to drive the Setup page says what is missing; else the start (with
/// the tour) or the lessons.
pub fn outcome(launcher_ui: &str, then: Then, folder_ok: bool) -> Outcome {
    if core::launcher_ui(launcher_ui) == "classic" {
        return Outcome::Restart;
    }
    match then {
        _ if !folder_ok => Outcome::Show { page: Page::Setup, tour: false },
        Then::Start => Outcome::Show { page: Page::Drive, tour: false },
        Then::Tour => Outcome::Show { page: Page::Drive, tour: true },
        Then::Lessons => Outcome::Show { page: Page::Tutorials, tour: false },
    }
}

/// Whether the welcome opens as the launcher starts, and at which step: when it was not seen
/// yet (`welcome_done`), or wherever `OMSI_LAUNCHER_WELCOME` (`asked`) says (`off`: never).
pub fn due(settings: &serde_json::Value, asked: Option<&str>) -> Option<Step> {
    match asked.map(|a| a.trim().to_ascii_lowercase()) {
        Some(a) if a == "off" || a == "0" => None,
        Some(a) if !a.is_empty() => Some(Step::parse(&a).unwrap_or(Step::Language)),
        _ => (!settings.get("welcome_done").and_then(|v| v.as_bool()).unwrap_or(false)).then_some(Step::Language),
    }
}

/// A value on a spring of its own: the welcome's motions outlive a frame unseen (`Ui::spring`
/// puts a value back where it goes when it was not drawn for a moment).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sprung {
    x: f32,
    v: f32,
}

impl Sprung {
    const fn at(x: f32) -> Sprung {
        Sprung { x, v: 0.0 }
    }

    /// `dt` seconds on towards `to` (there at once without animations).
    fn go(&mut self, to: f32, dt: f32, feel: Feel, motion: bool) -> f32 {
        if !motion {
            *self = Sprung::at(to);
            return to;
        }
        let (d, v) = spring_step(self.x - to, self.v, dt, feel);
        *self = if d.abs() < 1e-3 && v.abs() < 1e-2 { Sprung::at(to) } else { Sprung { x: to + d, v } };
        self.x
    }

    fn rests_at(&self, to: f32) -> bool {
        self.x == to && self.v == 0.0
    }
}

/// How a step comes in (a bit slower than a control's slide: a whole screen moves), the
/// dialog comes up (a little past and back), and a driver's pass pops in.
const ENTER: Feel = Feel { response: 0.42, damping: 0.88 };
const DIALOG: Feel = Feel { response: 0.34, damping: 0.74 };
const PASS: Feel = Feel { response: 0.3, damping: 0.7 };
/// How far a step slides in from its side, and the sheet rises at the first.
const SLIDE: f32 = 64.0;
const RISE: f32 = 28.0;
/// Finishing: the window goes dark this long, then the cover lifts off the page this long.
const LEAVE_DARK: f32 = 0.16;
const LIFT: f32 = 0.45;
const DARK: f32 = 0.94;
/// The widest the sheet grows (in the middle of a large window, never stretched), the bar's
/// height, and the heights of the sheet's foot and its buttons.
const SHEET_W: f32 = 980.0;
const BAR_H: f32 = 40.0;
const FOOT_H: f32 = 86.0;
/// The room over a step's body (under the head) and under it (over the foot's hairline).
const BODY_TOP: f32 = 8.0;
const BODY_BOTTOM: f32 = 28.0;
const BUTTON_H: f32 = 46.0;
/// The new driver's name field.
const NAME: &str = "welcome-driver-name";

/// The welcome's state (see the module).
pub struct Welcome {
    open: bool,
    step: Step,
    /// Whether the folder's step is one of the steps (decided as the welcome opens, so that
    /// the bar does not change under the player once the folder is right).
    folder: bool,
    /// The sheet's step coming in (0 to 1), and from which side: 1 the right (going on), -1
    /// the left (going back), 0 from below (the first).
    enter: Sprung,
    side: f32,
    /// The interface's dialog (0 shut, 1 open), and the sheet's height as drawn.
    dialog: Sprung,
    height: Option<Sprung>,
    /// What each step's body came to last frame (the sheet is sized by it).
    body_h: [f32; 6],
    /// Finishing: what comes after and how long the window has been going dark; then how
    /// long the cover has been lifting off the page under it.
    leave: Option<(Then, f32)>,
    lift: Option<f32>,
    /// The driver made in the welcome; the new driver's form open although one is chosen;
    /// the pass popping in; the personnel data read for a driver (see `personnel_in`).
    made: Option<String>,
    typing: bool,
    pass_in: Sprung,
    personnel: Option<(String, Option<(String, String)>)>,
    /// The name field wants the keyboard (the driver's step came up without a driver).
    focus_name: bool,
    /// The sheet's step last frame.
    seen: Option<Step>,
    /// The folder last judged, when, and what it is (see `verdict`).
    folder_seen: Option<(String, f32, Folder)>,
    /// openOMSI's mark in the sheet's head, alive under the mouse (see `intro::Logo`).
    logo: super::intro::Logo,
}

impl Welcome {
    pub fn closed() -> Welcome {
        Welcome { open: false, step: Step::Language, folder: false, enter: Sprung::at(0.0), side: 0.0, dialog: Sprung::at(0.0), height: None, body_h: [0.0; 6], leave: None, lift: None, made: None, typing: false, pass_in: Sprung::at(1.0), personnel: None, focus_name: false, seen: None, folder_seen: None, logo: Default::default() }
    }

    /// The welcome as the launcher starts (see `due`); `root` is the OMSI 2 folder set.
    pub fn at_start(settings: &serde_json::Value, root: &str) -> Welcome {
        let mut w = Welcome::closed();
        let asked = omsi_cfg::env::var("OMSI_LAUNCHER_WELCOME").ok();
        if let Some(step) = due(settings, asked.as_deref()) {
            log::info!("launcher: the welcome opens ({step:?})");
            w.open_at(step, folder_needed(root) || step == Step::Folder);
        }
        w
    }

    fn open_at(&mut self, step: Step, folder: bool) {
        *self = Welcome { open: true, step, folder, focus_name: step == Step::Driver, ..Welcome::closed() };
    }

    fn steps(&self) -> Vec<Step> {
        steps(self.folder)
    }

    fn index(&self, s: Step) -> usize {
        self.steps().iter().position(|x| *x == s).unwrap_or(0)
    }

    /// To another step: the sheet's slides in from the side it lies on.
    fn go_to(&mut self, to: Step) {
        if to == self.step {
            return;
        }
        let forward = self.index(to) > self.index(self.step);
        if sheet_step(to) != sheet_step(self.step) {
            self.enter = Sprung::at(0.0);
            self.side = if forward { 1.0 } else { -1.0 };
        }
        if to == Step::Driver {
            self.focus_name = true;
        }
        self.step = to;
    }

    fn next(&mut self) {
        let steps = self.steps();
        if let Some(n) = steps.get(self.index(self.step) + 1) {
            self.go_to(*n);
        }
    }

    fn back(&mut self) {
        let steps = self.steps();
        if let Some(k) = self.index(self.step).checked_sub(1) {
            self.go_to(steps[k]);
        }
    }
}

/// Whether the OMSI 2 folder set is not one to drive on (`state::root_problem` says why).
fn folder_needed(root: &str) -> bool {
    !omsi_cfg::missing_original_essentials(std::path::Path::new(root.trim())).is_empty()
}

/// Show the welcome again from its first step (the Settings page): `welcome_done` is false
/// until it is finished again. The page under it stays until then (no bus drives under it).
pub fn show_again(l: &mut Launcher) {
    l.state.settings["welcome_done"] = json!(false);
    l.state.settings_dirty = 0.3;
    let folder = folder_needed(&l.state.config.root);
    l.welcome.open_at(Step::Language, folder);
    log::info!("launcher: the welcome is shown again");
}

/// The input taken away from what lies under a dialog (or under the dark of the way out).
fn take_input(ui: &mut Ui) -> Input {
    let i = ui.input.clone();
    ui.input.mouse = Vec2::new(-1e4, -1e4);
    (ui.input.pressed, ui.input.released, ui.input.right_pressed, ui.input.double_click) = (false, false, false, false);
    ui.input.wheel = Vec2::ZERO;
    ui.input.keys.clear();
    ui.input.text.clear();
    i
}

// --- drawing -------------------------------------------------------------------------------

/// The welcome in place of the page while it is open (`covered`: the opening lies over the
/// window - the sheet comes in once it has gone). Returns whether it was drawn.
pub fn draw(l: &mut Launcher, covered: bool) -> bool {
    if !l.welcome.open {
        return false;
    }
    let (dt, motion) = (l.ui.dt, l.ui.motion);
    // the way out: the window goes dark, then the page comes up out of it (see `cover`)
    let mut dark = 0.0;
    if let Some((then, t)) = l.welcome.leave {
        let t = t + dt;
        if t >= LEAVE_DARK || !motion {
            l.welcome.leave = None;
            apply(l, then);
            if !l.welcome.open {
                return false;
            }
        } else {
            l.welcome.leave = Some((then, t));
            dark = DARK * ease_out_cubic(t / LEAVE_DARK);
        }
    }
    let size = l.ui.size;
    let window = Rect::new(0.0, 0.0, size.x, size.y);
    flow::ground_picture(l, window);
    // (a little darker than the start: the sheet is all there is to look at)
    l.ui.p().rect(window, GROUND.alpha(0.3));
    let interface = l.welcome.step == Step::Interface;
    let d = l.welcome.dialog.go(if interface { 1.0 } else { 0.0 }, dt, DIALOG, motion);
    // the dialog (and the way out) lie over the sheet and the bar: they see no mouse or keys
    let held = (interface || dark > 0.0).then(|| take_input(&mut l.ui));
    // (the sheet waits for the opening to go, then comes in)
    let k = if covered { l.welcome.enter.x } else { l.welcome.enter.go(1.0, dt, ENTER, motion) };
    sheet(l, k, covered);
    bar(l);
    if let Some(i) = held {
        l.ui.input = i;
    }
    if d > 0.004 {
        // (going away it takes no clicks: a second one went on twice)
        let held = (dark > 0.0 || !interface).then(|| take_input(&mut l.ui));
        dialog(l, d);
        if let Some(i) = held {
            l.ui.input = i;
        }
    }
    if dark > 0.0 {
        l.ui.solid(window);
        l.ui.p().rect(window, GROUND.alpha(dark));
    }
    let w = &l.welcome;
    if !w.enter.rests_at(1.0) || !w.dialog.rests_at(if interface { 1.0 } else { 0.0 }) || !w.pass_in.rests_at(1.0) || w.leave.is_some() || w.height.is_some_and(|h| h.v != 0.0) {
        l.ui.keep_moving();
    }
    true
}

/// Over the page once the welcome is over: the ground's dark lifting off it.
pub fn cover(l: &mut Launcher) {
    let Some(t) = l.welcome.lift else { return };
    let t = t + l.ui.dt;
    if t >= LIFT || !l.ui.motion {
        l.welcome.lift = None;
        return;
    }
    l.welcome.lift = Some(t);
    let size = l.ui.size;
    l.ui.p().rect(Rect::new(0.0, 0.0, size.x, size.y), GROUND.alpha(DARK * (1.0 - ease_in_out_cubic(t / LIFT))));
    l.ui.keep_moving();
}

/// The welcome is over (finished, or skipped): `welcome_done` is written and what comes after
/// it begins - at once without animations, else once the window has gone dark.
fn finish(l: &mut Launcher, then: Then) {
    if l.welcome.leave.is_some() {
        return;
    }
    if l.ui.motion {
        l.welcome.leave = Some((then, 0.0));
    } else {
        apply(l, then);
    }
}

fn apply(l: &mut Launcher, then: Then) {
    l.welcome.open = false;
    l.state.settings["welcome_done"] = json!(true);
    l.state.settings_dirty = 0.3;
    let ui_choice = l.state.settings.get("launcher_ui").and_then(|v| v.as_str()).unwrap_or("new").to_string();
    let to = outcome(&ui_choice, then, !folder_needed(&l.state.config.root));
    log::info!("launcher: the welcome is over ({then:?}): {to:?}");
    match to {
        Outcome::Restart => l.restart_launcher(),
        Outcome::Show { page, tour } => {
            // (the screen that lay under the welcome comes up out of the dark; another one
            // the bus brings, see `transition`)
            let same = l.page == page && (page != Page::Drive || l.drive.step == flow::Step::Mode);
            if page == Page::Drive {
                l.drive.step = flow::Step::Mode;
            }
            l.go(page);
            if same && l.ui.motion {
                l.welcome.lift = Some(0.0);
            }
            if tour {
                super::tour::start(l);
            }
        }
    }
}

/// The edge round the window: a phone has less to give.
fn edge() -> f32 {
    if mobile::mobile() {
        12.0
    } else {
        flow::EDGE_IN
    }
}

/// The bar along the top, as the setup's: the brand, the welcome's steps as a route, and
/// "Skip" on the right.
fn bar(l: &mut Launcher) {
    let size = l.ui.size;
    let e = edge();
    let r = Rect::new(e, e, size.x - 2.0 * e, BAR_H);
    l.ui.solid(r);
    l.ui.p().shadow(r.inset(-2.0), RADIUS, 20.0, Color::rgba(0, 0, 0, 0.4));
    l.ui.p().rounded(r, RADIUS, PANEL);
    l.ui.p().rounded_border(r, RADIUS, 1.0, EDGE);
    let word = omsi_ui::tr("Skip").to_string();
    let sw = l.ui.width(&word, 13.0, Weight::Medium) + 30.0;
    let skip = Rect::new(r.right() - 6.0 - sw, r.y + 5.0, sw, r.h - 10.0);
    if l.ui.button("welcome-skip", skip, "Skip", None, ButtonKind::Ghost) {
        finish(l, Then::Start);
    }
    let mut x = r.x + 18.0;
    if !mobile::mobile() {
        // (the brand as the launcher's bar has it: the new logo, small)
        let (verts, at) = super::flow::brand(&mut l.ui.atlas, &l.ui.fonts, l.ui.scale, x - 4.0, r.center().y, super::flow::BAR_BRAND_H, PANEL);
        l.ui.p().verts.extend(verts);
        x = at.right() + 34.0;
    }
    route(l, Rect::new(x, r.y, (skip.x - 14.0 - x).max(0.0), r.h));
}

/// The steps as the setup's route: done ones in ink (a way back), the current one blue and
/// underlined, those to come quiet. Narrow, only the current one has its word; narrower still,
/// it alone, with how far along it is.
fn route(l: &mut Launcher, r: Rect) {
    let steps = l.welcome.steps();
    let at = l.welcome.index(l.welcome.step);
    let words: Vec<String> = steps.iter().map(|s| omsi_ui::tr(s.label().0).to_uppercase()).collect();
    let widths: Vec<f32> = words.iter().map(|w| l.ui.width(w, 12.0, Weight::Bold) + 24.0).collect();
    let total: f32 = widths.iter().sum();
    let joins = steps.len().saturating_sub(1) as f32;
    let join = ((r.w - total) / joins.max(1.0) - 16.0).clamp(0.0, 40.0);
    let narrow = total + joins * 8.0 > r.w;
    let tight = narrow && widths[at] + joins * 38.0 > r.w;
    if tight {
        let (_, icon) = steps[at].label();
        l.ui.icon(icon, Vec2::new(r.x + 9.0, r.center().y), 15.0, accent());
        let w = l.ui.text_in(&words[at], Rect::new(r.x + 22.0, r.y, r.w - 60.0, r.h), 12.0, Weight::Bold, accent(), Align::Left);
        l.ui.text_in(&format!("{} / {}", at + 1, steps.len()), Rect::new(r.x + 32.0 + w, r.y, 50.0, r.h), 12.0, Weight::Medium, TEXT_DIM, Align::Left);
        return;
    }
    let filled = l.ui.spring(id_of("welcome-route"), at as f32, Feel::SLIDE);
    let mut under = None;
    let mut x = r.x;
    let mut go = None;
    for (k, step) in steps.iter().enumerate() {
        let word = if narrow && k != at { String::new() } else { words[k].clone() };
        let w = if word.is_empty() { 30.0 } else { widths[k] };
        let cell = Rect::new(x, r.y, w, r.h);
        let done = k < at;
        let (h, _, clicked) = l.ui.interact(id_of(&format!("welcome-step-{k}")), cell);
        if clicked && done {
            go = Some(*step);
        }
        let c = if k == at {
            accent()
        } else if done && h {
            TEXT.lighten(0.2)
        } else if done {
            TEXT
        } else {
            TEXT_FAINT
        };
        l.ui.icon(step.label().1, Vec2::new(cell.x + 9.0, cell.center().y), 15.0, c);
        if !word.is_empty() {
            l.ui.text_in(&word, Rect::new(cell.x + 22.0, cell.y, w - 22.0, cell.h), 12.0, Weight::Bold, c, Align::Left);
        }
        if k == at {
            under = Some((cell.x - 10.0, cell.right() + 4.0));
        }
        x += w;
        if k + 1 < steps.len() && !narrow && join > 4.0 {
            let line = Rect::new(x + 4.0, r.center().y - 1.0, join, 2.0);
            let part = (filled - k as f32).clamp(0.0, 1.0);
            if part < 1.0 {
                l.ui.p().rect(line, Color::WHITE.alpha(0.18));
            }
            if part > 0.0 {
                l.ui.p().rect(Rect::new(line.x, line.y, line.w * part, line.h), accent());
            }
            x += join + 12.0;
        } else {
            x += 8.0;
        }
    }
    if let Some((x0, x1)) = under {
        let (a, b) = l.ui.slide_span(id_of("welcome-underline"), x0, x1);
        l.ui.p().rounded(Rect::new(a, r.bottom() - 3.0, b - a, 3.0), 1.5, accent());
    }
    if let Some(s) = go {
        l.welcome.go_to(s);
    }
}

/// The step's eyebrow: "Step 2 of 5".
fn eyebrow(l: &Launcher, s: Step) -> String {
    let steps = l.welcome.steps();
    omsi_ui::tr("Step %{n} of %{total}").replace("%{n}", &(l.welcome.index(s) + 1).to_string()).replace("%{total}", &steps.len().to_string()).to_uppercase()
}

/// A step's title and the line under it.
fn heading(l: &Launcher, s: Step) -> (String, String) {
    let tr = |t: &str| omsi_ui::tr(t).to_string();
    match s {
        Step::Language | Step::Interface => (tr("Welcome to openOMSI"), tr("Choose the language of the launcher and the game. It applies at once, and you can change it any time in the settings.")),
        Step::Animations => (tr("Animations on or off?"), tr("Screens that slide in, tiles that rise under the mouse, a bus between the menus. Some like it lively, others calm.")),
        Step::Folder => (tr("Where is OMSI 2?"), tr("openOMSI drives on the maps and buses of the original OMSI 2. Point once to the folder you play from.")),
        Step::Driver => (tr("Who is driving?"), tr("Your duties, kilometres and level are kept per driver. Everything stays on this computer: there is nothing to sign in to.")),
        Step::Ready => {
            let name = chosen_driver(l);
            let title = match name {
                Some(n) => tr("Ready to go, %{name}").replace("%{name}", &n),
                None => tr("Ready to go"),
            };
            let sub = if classic(l) { tr("That is everything. The classic launcher opens as soon as the launcher has started again.") } else { tr("That is everything. Would you like a short tour of the launcher first?") };
            (title, sub)
        }
    }
}

/// Whether the classic launcher was chosen.
fn classic(l: &Launcher) -> bool {
    core::launcher_ui(l.state.settings.get("launcher_ui").and_then(|v| v.as_str()).unwrap_or("new")) == "classic"
}

/// The sheet: the step's head, its body (scrolling when the window is too low for it) and the
/// foot with Back and the way on. `k` is how far the step has come in.
fn sheet(l: &mut Launcher, k: f32, covered: bool) {
    let phone = mobile::mobile();
    let size = l.ui.size;
    let (dt, motion) = (l.ui.dt, l.ui.motion);
    let e = edge();
    let top = e + BAR_H + if phone { 12.0 } else { 22.0 };
    let avail = Rect::new(e, top, size.x - 2.0 * e, (size.y - top - if phone { e } else { 34.0 }).max(240.0));
    let w = avail.w.min(SHEET_W);
    let pad = if phone { 18.0 } else { 38.0 };
    let inner_w = w - 2.0 * pad;
    let step = sheet_step(l.welcome.step);
    let (title, sub) = heading(l, step);
    let title_px = if phone { 24.0 } else { 30.0 };
    // openOMSI's mark at the head's right; the title and its line keep clear of it
    let logo_w = (inner_w * 0.3).clamp(110.0, 260.0);
    let text_w = inner_w - logo_w - if phone { 12.0 } else { 28.0 };
    let title_h = l.ui.paragraph_height(&title, text_w, title_px, Weight::Bold) - title_px * 0.3;
    let sub_w = text_w.min(700.0);
    let sub_h = l.ui.paragraph_height(&sub, sub_w, 14.0, Weight::Regular);
    let head_h = 54.0 + title_h + 8.0 + sub_h + 22.0;
    let slot = ALL.iter().position(|s| *s == step).unwrap_or(0);
    // (a step not drawn yet keeps the sheet as it is for the frame it is measured in)
    let body_h = l.welcome.body_h[slot];
    let want = if body_h > 0.0 { (head_h + body_h + BODY_BOTTOM + FOOT_H).min(avail.h) } else { l.welcome.height.map(|h| h.x).unwrap_or((head_h + 320.0 + FOOT_H).min(avail.h)) };
    if covered || l.welcome.height.is_none() {
        l.welcome.height = Some(Sprung::at(want));
    }
    // (a new step: what had the keyboard on the last one has it no more)
    if l.welcome.seen != Some(step) {
        l.welcome.seen = Some(step);
        l.ui.focus = None;
    }
    let h = l.welcome.height.get_or_insert(Sprung::at(want)).go(want, dt, Feel::SLIDE, motion);
    let rise = if l.welcome.side == 0.0 { (1.0 - k) * RISE } else { 0.0 };
    let card = Rect::new(avail.x + (avail.w - w) * 0.5, avail.y + ((avail.h - h) * 0.5).max(0.0) + rise, w, h);
    flow::sheet(l, card);
    // the head and the body slide and fade in together; the foot stays where it is (Next is
    // pressed again where it was)
    let dx = l.welcome.side * (1.0 - k) * SLIDE;
    let x = card.x + pad + dx;
    let room = Rect::new(card.x, card.y, card.w, (card.h - FOOT_H).max(0.0));
    l.ui.push_clip(room, SHEET_RADIUS);
    let eb = eyebrow(l, step);
    l.ui.text_in(&eb, Rect::new(x, card.y + 26.0, inner_w, 16.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
    l.ui.paragraph(&title, Vec2::new(x, card.y + 50.0 - title_px * 0.15), text_w, title_px, Weight::Bold, TEXT);
    l.ui.paragraph(&sub, Vec2::new(x, card.y + 54.0 + title_h + 6.0), sub_w, 14.0, Weight::Regular, TEXT_SOFT);
    // (the mark slides with the head; under the mouse its light runs along the line)
    let logo_r = Rect::new(x + inner_w - logo_w, card.y + 18.0, logo_w, (head_h - 30.0).clamp(36.0, 96.0));
    let Launcher { ui, welcome, .. } = l;
    welcome.logo.draw(ui, "welcome-logo", logo_r);
    let view = Rect::new(card.x + 4.0, card.y + head_h - BODY_TOP, card.w - 8.0, (card.h - head_h - FOOT_H + BODY_TOP).max(0.0));
    let scroll_name = format!("welcome-body-{slot}");
    let off = l.ui.scroll.get(&id_of(&scroll_name)).copied().unwrap_or(0.0);
    l.ui.push_clip(view, 6.0);
    let body = Rect::new(x, view.y + BODY_TOP - off, inner_w, body_h.max(view.h - BODY_TOP - BODY_BOTTOM));
    let used = match step {
        Step::Language | Step::Interface => languages(l, body),
        Step::Animations => animations(l, body),
        Step::Folder => folder(l, body),
        Step::Driver => driver(l, body),
        Step::Ready => ready(l, body),
    };
    l.welcome.body_h[slot] = used;
    l.ui.pop_clip();
    // (the step coming up out of the sheet)
    if k < 0.999 {
        l.ui.p().rect(room, PANEL.alpha((1.0 - k).clamp(0.0, 1.0)));
    }
    l.ui.pop_clip();
    // (a hair under the view's height is no reason for a scrollbar: the sheet is sized to it)
    l.ui.scroll_keep(&scroll_name, view, BODY_TOP + used + BODY_BOTTOM - 0.5);
    foot(l, card, pad, step);
}

/// The sheet's foot: a hairline, Back on the left (not on the first step), the way on on the
/// right - its word and what it does are the step's.
fn foot(l: &mut Launcher, card: Rect, pad: f32, step: Step) {
    let phone = mobile::mobile();
    let y = card.bottom() - FOOT_H;
    l.ui.p().rect(Rect::new(card.x, y, card.w, 1.0), HAIRLINE);
    let by = y + (FOOT_H - BUTTON_H) * 0.5;
    if l.welcome.index(step) > 0 {
        let w = if phone { BUTTON_H } else { 116.0 };
        if l.ui.button("welcome-back", Rect::new(card.x + pad, by, w, BUTTON_H), if phone { "" } else { "Back" }, Some("chevron_left"), ButtonKind::Normal) {
            l.welcome.back();
        }
        l.ui.tooltip(Rect::new(card.x + pad, by, w, BUTTON_H), "Back");
    }
    // the steps without a text field go on with Enter as well
    let enter = l.ui.focus.is_none() && l.ui.input.keys.contains(&Key::Enter);
    let right = card.right() - pad;
    let button = |l: &mut Launcher, name: &str, label: &str, icon: &str, kind: ButtonKind| -> bool {
        let w = (l.ui.width(&omsi_ui::tr(label), 14.5, Weight::Bold) + 70.0).max(if phone { 120.0 } else { 170.0 });
        l.ui.button(name, Rect::new(right - w, by, w, BUTTON_H), label, Some(icon), kind)
    };
    match step {
        Step::Language | Step::Animations => {
            if button(l, "welcome-next", "Next", "chevron_right", ButtonKind::Primary) || (enter && l.welcome.step == step) {
                l.welcome.next();
            }
        }
        Step::Folder => {
            let ok = verdict(l) == Folder::Fine;
            if ok {
                if button(l, "welcome-folder-use", "Use this folder", "check", ButtonKind::Primary) || enter {
                    save_folder(l);
                    l.welcome.next();
                }
            } else if button(l, "welcome-folder-later", "Skip for now", "chevron_right", ButtonKind::Normal) {
                l.welcome.next();
            }
        }
        Step::Driver => {
            if button(l, "welcome-next", "Next", "chevron_right", ButtonKind::Primary) {
                driver_next(l);
            }
        }
        Step::Ready if classic(l) => {
            if button(l, "welcome-restart", "Restart in the classic interface", "restart_alt", ButtonKind::Primary) || enter {
                finish(l, Then::Start);
            }
        }
        Step::Ready => {
            // (a quiet way to the game's own lessons, for whoever never drove a bus in OMSI)
            if button(l, "welcome-lessons", "New to OMSI? Driving lessons", "help", ButtonKind::Ghost) {
                finish(l, Then::Lessons);
            }
            if enter {
                finish(l, Then::Start);
            }
        }
        Step::Interface => {}
    }
}

// --- the language ----------------------------------------------------------------------------

/// How many tiles go in a row `w` wide - none narrower than `min`, `gap` between them, at most
/// `most` - and how wide each then is.
fn grid(w: f32, min: f32, gap: f32, most: usize) -> (usize, f32) {
    let cols = (((w + gap) / (min + gap)).floor() as usize).clamp(1, most.max(1));
    (cols, ((w - gap * (cols as f32 - 1.0)) / cols as f32).max(0.0))
}

/// Every language openOMSI speaks as a tile with its flag and its own name; a click speaks it
/// at once. Returns the height used.
fn languages(l: &mut Launcher, r: Rect) -> f32 {
    let phone = mobile::mobile();
    let current = l.state.settings.get("language").and_then(|v| v.as_str()).unwrap_or("ENG").to_string();
    let gap = 10.0;
    let (cols, tw) = grid(r.w, if phone { 118.0 } else { 160.0 }, gap, 5);
    let th = if phone { 50.0 } else { 56.0 };
    let mut pick = None;
    for (k, (code, name, _, _)) in core::LANGUAGES.iter().enumerate() {
        let base = Rect::new(r.x + (k % cols) as f32 * (tw + gap), r.y + (k / cols) as f32 * (th + gap), tw, th);
        if !l.ui.rect_visible(base.inset(-8.0)) {
            continue;
        }
        let id = id_of(&format!("welcome-lang-{code}"));
        let t = l.ui.tile(id, base, RADIUS);
        let on = current == *code;
        let blue = l.ui.anim(id ^ 0xb1e, if on { 1.0 } else { 0.0 }, 0.06);
        l.ui.tile_shadow(&t);
        l.ui.p().rounded(t.r, RADIUS, FIELD.mix(HOVER, t.hover).mix(accent(), blue));
        l.ui.tile_light(&t, 0.05);
        if blue < 0.99 {
            l.ui.tile_edge(&t, 1.0, EDGE.alpha(1.0 - blue));
        }
        let f = Rect::new(t.r.x + 14.0, t.r.center().y - 10.0, 30.0, 20.0);
        if !paint_flag(l.ui.p(), f, code) {
            script_badge(&mut l.ui, f, code);
        }
        let right = if on { 30.0 } else { 10.0 };
        whole_name(&mut l.ui, name, Rect::new(f.right() + 12.0, t.r.y, t.r.right() - f.right() - 12.0 - right, t.r.h), if phone { 13.5 } else { 14.5 });
        if on {
            l.ui.icon("check", Vec2::new(t.r.right() - 18.0, t.r.center().y), 17.0, on_accent().alpha(blue));
        }
        if t.clicked && !on {
            pick = Some(code.to_string());
        }
    }
    if let Some(code) = pick {
        log::info!("launcher: the welcome speaks {code} from now on");
        flow::set_language(l, &code);
    }
    let rows = core::LANGUAGES.len().div_ceil(cols);
    rows as f32 * (th + gap) - gap
}

/// A language's own name in `r`, never cut: on two lines where it has a space, else smaller.
fn whole_name(ui: &mut Ui, name: &str, r: Rect, px: f32) {
    let w = ui.width(name, px, Weight::Bold);
    if w <= r.w {
        ui.text_in(name, r, px, Weight::Bold, TEXT, Align::Left);
    } else if name.contains(' ') && ui.paragraph_height(name, r.w, px - 1.0, Weight::Bold) <= 2.0 * (px - 1.0) * 1.38 + 0.5 {
        let h = ui.paragraph_height(name, r.w, px - 1.0, Weight::Bold);
        ui.paragraph(name, Vec2::new(r.x, r.center().y - h * 0.5 - (px - 1.0) * 0.12), r.w, px - 1.0, Weight::Bold, TEXT);
    } else {
        let small = (px * r.w / w.max(1.0)).max(10.0);
        ui.text_in(name, r, small, Weight::Bold, TEXT, Align::Left);
    }
}

/// A language's flag, painted (no flag emoji on Windows): the bar's four as the bar has them,
/// the others as simple as they still read at thirty points. False for a language that is
/// spoken under more than one flag or whose flag is disputed - it gets its script instead.
fn paint_flag(p: &mut Painter, r: Rect, code: &str) -> bool {
    if flow::FLAGS.contains(&code) {
        super::ui::flag(p, r, code);
        return true;
    }
    let rgb = |r: u8, g: u8, b: u8| Color::rgba(r, g, b, 1.0);
    let white = rgb(255, 255, 255);
    let rows = |p: &mut Painter, parts: &[(f32, Color)]| {
        let total: f32 = parts.iter().map(|x| x.0).sum();
        let mut y = r.y;
        for (share, c) in parts {
            let h = r.h * share / total;
            p.rect(Rect::new(r.x, y, r.w, h + 0.02), *c);
            y += h;
        }
    };
    let cols = |p: &mut Painter, parts: &[(f32, Color)]| {
        let total: f32 = parts.iter().map(|x| x.0).sum();
        let mut x = r.x;
        for (share, c) in parts {
            let w = r.w * share / total;
            p.rect(Rect::new(x, r.y, w + 0.02, r.h), *c);
            x += w;
        }
    };
    let c = r.center();
    match code {
        "RUS" => rows(p, &[(1.0, white), (1.0, rgb(0, 57, 166)), (1.0, rgb(213, 43, 30))]),
        "UKR" => rows(p, &[(1.0, rgb(0, 87, 183)), (1.0, rgb(255, 215, 0))]),
        "POL" => rows(p, &[(1.0, white), (1.0, rgb(220, 20, 60))]),
        // (the senyera: four red stripes on gold)
        "CAT" => {
            let (gold, red) = (rgb(252, 221, 9), rgb(218, 18, 26));
            rows(p, &[(1.0, gold), (1.0, red), (1.0, gold), (1.0, red), (1.0, gold), (1.0, red), (1.0, gold), (1.0, red), (1.0, gold)])
        }
        "CZE" => {
            rows(p, &[(1.0, white), (1.0, rgb(215, 20, 26))]);
            p.convex(&[Vec2::new(r.x, r.y), Vec2::new(r.x + r.w * 0.5, c.y), Vec2::new(r.x, r.bottom())], rgb(17, 69, 126));
        }
        "HUN" => rows(p, &[(1.0, rgb(205, 42, 62)), (1.0, white), (1.0, rgb(67, 111, 77))]),
        "ESP" => rows(p, &[(1.0, rgb(170, 21, 27)), (2.0, rgb(241, 191, 0)), (1.0, rgb(170, 21, 27))]),
        "ITA" => cols(p, &[(1.0, rgb(0, 146, 70)), (1.0, white), (1.0, rgb(206, 43, 55))]),
        "PTP" => {
            cols(p, &[(2.0, rgb(4, 106, 56)), (3.0, rgb(218, 41, 28))]);
            p.circle(Vec2::new(r.x + r.w * 0.4, c.y), r.h * 0.22, rgb(255, 204, 0));
            p.circle(Vec2::new(r.x + r.w * 0.4, c.y), r.h * 0.12, white);
        }
        "PTB" => {
            p.rect(r, rgb(0, 156, 59));
            p.convex(&[Vec2::new(r.x + r.w * 0.09, c.y), Vec2::new(c.x, r.y + r.h * 0.12), Vec2::new(r.right() - r.w * 0.09, c.y), Vec2::new(c.x, r.bottom() - r.h * 0.12)], rgb(255, 223, 0));
            p.circle(c, r.h * 0.23, rgb(0, 39, 118));
        }
        "TUR" => {
            p.rect(r, rgb(227, 10, 23));
            p.circle(Vec2::new(r.x + r.w * 0.37, c.y), r.h * 0.25, white);
            p.circle(Vec2::new(r.x + r.w * 0.41, c.y), r.h * 0.2, rgb(227, 10, 23));
            star(p, Vec2::new(r.x + r.w * 0.6, c.y), r.h * 0.13, white, -PI * 0.5);
        }
        "JPN" => {
            p.rect(r, white);
            p.circle(c, r.h * 0.3, rgb(188, 0, 45));
        }
        "KOR" => {
            p.rect(r, white);
            p.circle(c, r.h * 0.25, rgb(0, 71, 160));
            p.arc(c, 0.0, r.h * 0.25, PI, TAU, rgb(205, 46, 58));
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                p.rect(Rect::new(c.x + dx * r.w * 0.33 - 2.0, c.y + dy * r.h * 0.3 - 1.5, 4.0, 3.0), rgb(20, 20, 20));
            }
        }
        "THA" => rows(p, &[(1.0, rgb(165, 25, 49)), (1.0, white), (2.0, rgb(45, 42, 74)), (1.0, white), (1.0, rgb(165, 25, 49))]),
        "VIE" => {
            p.rect(r, rgb(218, 37, 29));
            star(p, c, r.h * 0.32, rgb(255, 255, 0), 0.0);
        }
        "IND" => rows(p, &[(1.0, rgb(206, 17, 38)), (1.0, white)]),
        "MSA" => {
            let stripes: Vec<(f32, Color)> = (0..14).map(|k| (1.0, if k % 2 == 0 { rgb(204, 0, 1) } else { white })).collect();
            rows(p, &stripes);
            let canton = Rect::new(r.x, r.y, r.w * 0.5, r.h * 8.0 / 14.0);
            p.rect(canton, rgb(1, 0, 102));
            let m = Vec2::new(canton.x + canton.w * 0.38, canton.center().y);
            p.circle(m, canton.h * 0.36, rgb(255, 204, 0));
            p.circle(m + Vec2::new(canton.h * 0.1, 0.0), canton.h * 0.3, rgb(1, 0, 102));
            star(p, Vec2::new(canton.x + canton.w * 0.74, m.y), canton.h * 0.22, rgb(255, 204, 0), 0.0);
        }
        "TGL" => {
            rows(p, &[(1.0, rgb(0, 56, 168)), (1.0, rgb(206, 17, 38))]);
            p.convex(&[Vec2::new(r.x, r.y), Vec2::new(r.x + r.h * 0.87, c.y), Vec2::new(r.x, r.bottom())], white);
            p.circle(Vec2::new(r.x + r.h * 0.3, c.y), r.h * 0.11, rgb(252, 209, 22));
        }
        "KAZ" => {
            p.rect(r, rgb(0, 175, 202));
            p.arc(c, r.h * 0.24, r.h * 0.31, 0.0, TAU, rgb(254, 197, 4));
            p.circle(c, r.h * 0.19, rgb(254, 197, 4));
        }
        "HIN" => {
            rows(p, &[(1.0, rgb(255, 153, 51)), (1.0, white), (1.0, rgb(19, 136, 8))]);
            p.arc(c, r.h * 0.1, r.h * 0.14, 0.0, TAU, rgb(0, 0, 128));
            p.circle(c, r.h * 0.035, rgb(0, 0, 128));
        }
        _ => return false,
    }
    // (an edge of light from inside, as the bar's flags have it)
    p.rounded_border(r, 2.0, 1.0, Color::WHITE.alpha(0.28));
    true
}

/// A five-pointed star round `c`, its outer points `outer` from it, one pointing at `turn`
/// (radians from straight up, clockwise).
fn star(p: &mut Painter, c: Vec2, outer: f32, col: Color, turn: f32) {
    let pts: Vec<Vec2> = (0..10)
        .map(|i| {
            let a = turn - PI * 0.5 + i as f32 * PI / 5.0;
            c + Vec2::new(a.cos(), a.sin()) * if i % 2 == 0 { outer } else { outer * 0.382 }
        })
        .collect();
    for i in 0..10 {
        p.tri(c, pts[i], pts[(i + 1) % 10], col, col, col);
    }
}

/// Where a language has no one flag: its script on a plate of the flag's size.
fn script_badge(ui: &mut Ui, r: Rect, code: &str) {
    let mark = match code {
        "ZHT" => "繁",
        "CHS" => "简",
        "BEL" => "Бел",
        _ => code,
    };
    ui.p().rounded(r, 3.0, Color::WHITE.alpha(0.12));
    ui.p().rounded_border(r, 3.0, 1.0, Color::WHITE.alpha(0.24));
    ui.text_in(mark, r, 11.5, Weight::Bold, TEXT, Align::Center);
}

// --- the interface (the dialog) ---------------------------------------------------------------

/// The dialog over the language: the new interface or the classic one, each with a picture of
/// it. `d` is how far it is open (a little past 1 as it comes up).
fn dialog(l: &mut Launcher, d: f32) {
    let phone = mobile::mobile();
    let size = l.ui.size;
    let a = d.clamp(0.0, 1.0);
    let window = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(window);
    // (blended in linear light: 0.82 darkens the dark ground about as much as half looks)
    l.ui.p().rect(window, GROUND.alpha(0.82 * a));
    // (wider than the sheet under it: it lies over the whole of it)
    let w = (size.x - if phone { 24.0 } else { 48.0 }).min(SHEET_W + 40.0);
    let pad = if phone { 18.0 } else { 32.0 };
    let inner = w - 2.0 * pad;
    let stacked = phone || inner < 600.0;
    let gap = if stacked { 12.0 } else { 20.0 };
    let cw = if stacked { inner } else { (inner - gap) * 0.5 };
    let pic_h = (cw - 20.0) / if phone { 2.6 } else if stacked { 2.3 } else { 1.6 };
    let lines = ["Omsi-Hub's look: you set up your duty step by step on the map, with your bus in 3D.", "openOMSI's own launcher: every page in one window, with the menu on the left."];
    let line_h = lines.iter().map(|t| l.ui.paragraph_height(t, cw - 40.0, 13.0, Weight::Regular)).fold(0.0, f32::max);
    let card_h = 10.0 + pic_h + 16.0 + 24.0 + 6.0 + line_h + 16.0;
    let cards_h = if stacked { 2.0 * card_h + gap } else { card_h };
    let title_px = if phone { 22.0 } else { 26.0 };
    let title = omsi_ui::tr("Which interface do you want to use?").to_string();
    let title_h = l.ui.paragraph_height(&title, inner, title_px, Weight::Bold) - title_px * 0.3;
    let sub = omsi_ui::tr("Both start the same game, and you can switch later.").to_string();
    let sub_h = l.ui.paragraph_height(&sub, inner, 14.0, Weight::Regular);
    let head_h = 50.0 + title_h + 8.0 + sub_h + 22.0;
    let h = (head_h + cards_h + FOOT_H).min(size.y - 24.0);
    // (a window too low for the cards - a phone - scrolls them)
    let room = (h - head_h - FOOT_H).max(0.0);
    // it comes up from a little lower and smaller, the veil darkening under it
    let s = 0.94 + 0.06 * d;
    let base = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5 + (1.0 - d) * 22.0, w, h);
    let shown = Rect::new(base.center().x - w * s * 0.5, base.center().y - h * s * 0.5, w * s, h * s);
    l.ui.p().shadow(shown.inset(-2.0), SHEET_RADIUS, 30.0, Color::rgba(0, 0, 0, 0.5 * a));
    // (solid almost at once: the sheet's words under it showed through the words on it)
    l.ui.p().rounded(shown, SHEET_RADIUS, PANEL.alpha((a * 4.0).min(1.0)));
    l.ui.p().rounded_border(shown, SHEET_RADIUS, 1.0, EDGE.alpha(0.08 * a));
    l.ui.push_clip(shown, SHEET_RADIUS);
    let x = base.x + pad;
    let eb = eyebrow(l, Step::Interface);
    l.ui.text_in(&eb, Rect::new(x, base.y + 24.0, inner, 16.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
    l.ui.paragraph(&title, Vec2::new(x, base.y + 48.0 - title_px * 0.15), inner, title_px, Weight::Bold, TEXT);
    l.ui.paragraph(&sub, Vec2::new(x, base.y + 50.0 + title_h + 8.0), inner, 14.0, Weight::Regular, TEXT_SOFT);
    let view = Rect::new(shown.x, base.y + head_h - 6.0, shown.w, room + 6.0);
    let scrolls = cards_h > room + 0.5;
    let off = if scrolls { l.ui.scroll.get(&id_of("welcome-ui-cards")).copied().unwrap_or(0.0) } else { 0.0 };
    if scrolls {
        l.ui.push_clip(view, 0.0);
    }
    let current = core::launcher_ui(l.state.settings.get("launcher_ui").and_then(|v| v.as_str()).unwrap_or("new"));
    let choices = [("new", "New interface", "ui-new", lines[0]), ("classic", "Classic interface", "ui-classic", lines[1])];
    let mut chosen = None;
    for (k, (value, title, picture, line)) in choices.iter().enumerate() {
        let at = if stacked { Vec2::new(x, base.y + head_h - off + k as f32 * (card_h + gap)) } else { Vec2::new(x + k as f32 * (cw + gap), base.y + head_h) };
        let id = id_of(&format!("welcome-ui-{value}"));
        let t = l.ui.tile(id, Rect::new(at.x, at.y, cw, card_h), SHEET_RADIUS);
        let on = current == *value;
        let sel = l.ui.anim(id ^ 0x5e1, if on { 1.0 } else { 0.0 }, 0.07);
        l.ui.tile_shadow(&t);
        if sel > 0.01 {
            l.ui.p().shadow(t.r.inset(-3.0), SHEET_RADIUS + 3.0, 22.0, accent().alpha(0.38 * sel));
        }
        l.ui.p().rounded(t.r, SHEET_RADIUS, FIELD.mix(HOVER, t.hover));
        let pic = Rect::new(t.r.x + 10.0, t.r.y + 10.0, t.r.w - 20.0, pic_h);
        match l.pictures.get(picture).copied() {
            Some((tex, pw, ph)) => l.ui.tile_photo(&t, pic, RADIUS, tex, pw, ph),
            None => l.ui.p().rounded(pic, RADIUS, GROUND),
        }
        l.ui.p().rounded_border(pic, RADIUS, 1.0, Color::WHITE.alpha(0.08));
        // (the chosen one's mark on its picture, popping in)
        if sel > 0.01 {
            let m = Vec2::new(pic.right() - 20.0, pic.y + 20.0);
            l.ui.p().circle(m, 13.0 * (0.6 + 0.4 * sel), accent().alpha(sel));
            l.ui.icon("check", m, 17.0, on_accent().alpha(sel));
        }
        let ty = pic.bottom() + 16.0;
        l.ui.radio(Vec2::new(t.r.x + 22.0, ty + 12.0), on);
        l.ui.text_in(title, Rect::new(t.r.x + 38.0, ty, t.r.w - 56.0, 24.0), 17.0, Weight::Bold, TEXT, Align::Left);
        l.ui.paragraph(line, Vec2::new(t.r.x + 20.0, ty + 30.0), cw - 40.0, 13.0, Weight::Regular, TEXT_DIM);
        l.ui.tile_light(&t, 0.05);
        if sel > 0.01 {
            l.ui.p().rounded_border(t.r, SHEET_RADIUS, 2.5, accent().alpha(sel));
        }
        if sel < 0.99 {
            l.ui.tile_edge(&t, 1.0, EDGE.alpha(1.0 - sel));
        }
        if t.clicked && !on {
            chosen = Some(*value);
        }
    }
    if scrolls {
        l.ui.pop_clip();
        l.ui.scroll_keep("welcome-ui-cards", view, cards_h + 12.0);
    }
    if let Some(v) = chosen {
        l.state.settings["launcher_ui"] = json!(v);
        l.state.settings_dirty = 0.3;
        log::info!("launcher: the welcome chose the {v} interface");
    }
    // (it comes up out of the panel's own colour)
    if d < 0.999 {
        l.ui.p().rect(shown, PANEL.alpha((1.0 - d).clamp(0.0, 1.0)));
    }
    l.ui.pop_clip();
    let fy = base.bottom() - FOOT_H;
    l.ui.p().rect(Rect::new(shown.x, fy, shown.w, 1.0), HAIRLINE.alpha(a));
    let by = fy + (FOOT_H - BUTTON_H) * 0.5;
    let bw = if phone { BUTTON_H } else { 116.0 };
    let escape = l.ui.input.keys.contains(&Key::Escape);
    if l.ui.button("welcome-ui-back", Rect::new(x, by, bw, BUTTON_H), if phone { "" } else { "Back" }, Some("chevron_left"), ButtonKind::Normal) || escape {
        l.welcome.back();
    }
    let nw = (l.ui.width(&omsi_ui::tr("Next"), 14.5, Weight::Bold) + 70.0).max(if phone { 120.0 } else { 170.0 });
    let enter = l.ui.input.keys.contains(&Key::Enter);
    if l.ui.button("welcome-ui-next", Rect::new(base.right() - pad - nw, by, nw, BUTTON_H), "Next", Some("chevron_right"), ButtonKind::Primary) || enter {
        l.welcome.next();
    }
}

// --- the animations --------------------------------------------------------------------------

/// The bus of the little scene at `t` seconds: how far along its route (0 to 1) and how much
/// of it is seen. It pulls away, stops at the middle stop, goes on to the last and fades out
/// there, and comes again at the first.
fn bus_at(t: f32) -> (f32, f32) {
    const CYCLE: f32 = 4.6;
    let c = t.rem_euclid(CYCLE);
    if c < 1.5 {
        (0.5 * ease_in_out_cubic(c / 1.5), (c / 0.3).min(1.0))
    } else if c < 2.2 {
        (0.5, 1.0)
    } else if c < 3.7 {
        (0.5 + 0.5 * ease_in_out_cubic((c - 2.2) / 1.5), 1.0)
    } else {
        (1.0, 1.0 - smoothstep((c - 3.7) / 0.6))
    }
}

/// The scene's three tiles at `t`: which one the mouse is over, where the mouse is between the
/// tiles (a tile's index, fractional on the way), and how high that tile has risen (0 to 1).
fn tiles_at(t: f32) -> (usize, f32, f32) {
    const SLOT: f32 = 1.15;
    let p = t.rem_euclid(3.0 * SLOT) / SLOT;
    let k = (p.floor() as usize).min(2);
    let u = p - k as f32;
    let from = if k == 0 { 2.0 } else { k as f32 - 1.0 };
    // (from the last tile back to the first the mouse sweeps back across the others)
    let pointer = from + (k as f32 - from) * ease_in_out_cubic((u / 0.32).min(1.0));
    let lift = smoothstep((u - 0.22) / 0.18) * (1.0 - smoothstep((u - 0.84) / 0.16));
    (k, pointer, lift)
}

/// The animations, on or off: two cards, each with a little scene - on, the mouse goes over
/// tiles that rise and a bus drives its route; off, all of it stands still. A click sets the
/// setting at once (the welcome itself then moves, or not). Returns the height used.
fn animations(l: &mut Launcher, r: Rect) -> f32 {
    let phone = mobile::mobile();
    let on_now = l.state.settings.get("animations").and_then(|v| v.as_bool()).unwrap_or(true);
    let stacked = phone || r.w < 560.0;
    let gap = 16.0;
    let cw = if stacked { r.w } else { (r.w - gap) * 0.5 };
    let scene_h = if phone { 104.0 } else { 140.0 };
    let cards = [(true, "Animations on", "Screens slide in, tiles rise under the mouse and a bus drives between the menus."), (false, "Animations off", "Everything stands still and each screen is there at once: calmer, and lighter for an older computer.")];
    let line_h = cards.iter().map(|c| l.ui.paragraph_height(c.2, cw - 40.0, 13.0, Weight::Regular)).fold(0.0, f32::max);
    let card_h = 12.0 + scene_h + 16.0 + 24.0 + 6.0 + line_h + 16.0;
    let mut set = None;
    for (k, (value, title, line)) in cards.iter().enumerate() {
        let at = if stacked { Vec2::new(r.x, r.y + k as f32 * (card_h + 12.0)) } else { Vec2::new(r.x + k as f32 * (cw + gap), r.y) };
        let id = id_of(&format!("welcome-anim-{value}"));
        let t = l.ui.tile(id, Rect::new(at.x, at.y, cw, card_h), SHEET_RADIUS);
        let on = on_now == *value;
        let sel = l.ui.anim(id ^ 0x5e1, if on { 1.0 } else { 0.0 }, 0.07);
        l.ui.tile_shadow(&t);
        if sel > 0.01 {
            l.ui.p().shadow(t.r.inset(-3.0), SHEET_RADIUS + 3.0, 22.0, accent().alpha(0.38 * sel));
        }
        l.ui.p().rounded(t.r, SHEET_RADIUS, FIELD.mix(HOVER, t.hover));
        let scene_r = Rect::new(t.r.x + 12.0, t.r.y + 12.0, t.r.w - 24.0, scene_h);
        // (the scene of "on" plays while animations are on, and under the mouse to show what
        // they would be - not on its own for whoever turned them off)
        let playing = *value && (l.ui.motion || t.hovered);
        let time = l.ui.time;
        scene(&mut l.ui, scene_r, if playing { Some(time) } else { None });
        if playing {
            l.ui.keep_moving();
        }
        let ty = scene_r.bottom() + 16.0;
        l.ui.radio(Vec2::new(t.r.x + 22.0, ty + 12.0), on);
        l.ui.text_in(title, Rect::new(t.r.x + 38.0, ty, t.r.w - 56.0, 24.0), 17.0, Weight::Bold, TEXT, Align::Left);
        l.ui.paragraph(line, Vec2::new(t.r.x + 20.0, ty + 30.0), cw - 40.0, 13.0, Weight::Regular, TEXT_DIM);
        l.ui.tile_light(&t, 0.05);
        if sel > 0.01 {
            l.ui.p().rounded_border(t.r, SHEET_RADIUS, 2.5, accent().alpha(sel));
        }
        if sel < 0.99 {
            l.ui.tile_edge(&t, 1.0, EDGE.alpha(1.0 - sel));
        }
        if t.clicked && !on {
            set = Some(*value);
        }
    }
    if let Some(v) = set {
        l.state.settings["animations"] = json!(v);
        l.state.settings_dirty = 0.3;
        log::info!("launcher: the welcome turned the animations {}", if v { "on" } else { "off" });
    }
    if stacked {
        2.0 * card_h + 12.0
    } else {
        card_h
    }
}

/// The little scene in `r`: three tiles with the mouse over one and a bus on a route with
/// three stops - moving at `time` (seconds), or standing still (None).
fn scene(ui: &mut Ui, r: Rect, time: Option<f32>) {
    ui.p().rounded(r, RADIUS, GROUND.mix(FIELD, 0.35));
    // (a map's roads under it, faint)
    ui.push_clip(r, RADIUS);
    let road = Color::WHITE.alpha(0.035);
    ui.p().line(Vec2::new(r.x + r.w * 0.55, r.y - 10.0), Vec2::new(r.x + r.w * 0.8, r.bottom() + 10.0), 9.0, road);
    ui.p().line(Vec2::new(r.x - 10.0, r.y + r.h * 0.5), Vec2::new(r.right() + 10.0, r.y + r.h * 0.38), 6.0, road);
    ui.p().line(Vec2::new(r.x + r.w * 0.12, r.bottom() + 10.0), Vec2::new(r.x + r.w * 0.3, r.y - 10.0), 5.0, road);
    ui.pop_clip();
    ui.p().rounded_border(r, RADIUS, 1.0, Color::WHITE.alpha(0.06));
    // the tiles, the one under the mouse risen
    let (gap, th) = (8.0, (r.h * 0.28).min(38.0));
    let tw = ((r.w - 36.0 - 2.0 * gap) / 3.0).min(th * 2.6);
    let tx = r.x + (r.w - 3.0 * tw - 2.0 * gap) * 0.5;
    let ty = r.y + 16.0;
    let (at, pointer, lift) = time.map(tiles_at).unwrap_or((0, 0.0, 0.0));
    for k in 0..3 {
        let base = Rect::new(tx + k as f32 * (tw + gap), ty, tw, th);
        let up = if k == at { lift } else { 0.0 };
        let tr = Rect::new(base.x - 1.5 * up, base.y - 3.0 * up, base.w + 3.0 * up, base.h + 2.0 * up);
        if up > 0.01 {
            ui.p().shadow(Rect::new(tr.x, tr.y + 3.0 * up, tr.w, tr.h), 6.0, 6.0 + 4.0 * up, Color::rgba(0, 0, 0, 0.45 * up));
        }
        ui.p().rounded(tr, 6.0, HOVER.mix(HOVER.lighten(0.08), up));
        ui.p().rounded_border(tr, 6.0, 1.0, Color::WHITE.alpha(0.1).mix(accent().lighten(0.25), up));
        ui.p().rounded(Rect::new(tr.x + 7.0, tr.y + tr.h * 0.5 - 2.0, tr.w * 0.45, 4.0), 2.0, Color::WHITE.alpha(0.22 + 0.2 * up));
    }
    if time.is_some() {
        // the mouse, gliding from tile to tile
        let px = tx + pointer * (tw + gap) + tw * 0.62;
        let py = ty + th * 0.62 - 3.0 * lift;
        let tip = Vec2::new(px, py);
        let arrow = [tip, tip + Vec2::new(0.0, 13.0), tip + Vec2::new(3.6, 9.8), tip + Vec2::new(9.0, 9.4)];
        let dark = [tip + Vec2::new(-1.2, -1.8), tip + Vec2::new(-1.2, 15.6), tip + Vec2::new(4.2, 11.4), tip + Vec2::new(11.6, 10.6)];
        ui.p().convex(&dark, Color::rgba(10, 14, 24, 0.9));
        ui.p().convex(&arrow, Color::WHITE);
    }
    // the route and its stops, the bus on it
    let y = r.y + r.h * 0.76;
    let (x0, x1) = (r.x + 26.0, r.right() - 26.0);
    ui.p().line(Vec2::new(x0, y), Vec2::new(x1, y), 7.0, Color::rgba(18, 58, 107, 1.0));
    ui.p().line(Vec2::new(x0, y), Vec2::new(x1, y), 4.0, accent());
    for s in [x0, (x0 + x1) * 0.5, x1] {
        ui.p().circle(Vec2::new(s, y), 6.0, Color::rgba(10, 107, 61, 1.0));
        ui.p().circle(Vec2::new(s, y), 4.4, LINE);
    }
    let (along, seen) = time.map(bus_at).unwrap_or((0.0, 1.0));
    mini_bus(ui.p(), Vec2::new(x0 + (x1 - x0) * along, y - 7.0), (r.h / 140.0).clamp(0.8, 1.0), seen);
}

/// A city bus seen from the side, its wheels standing on `at` (its middle), facing right.
fn mini_bus(p: &mut Painter, at: Vec2, s: f32, a: f32) {
    if a <= 0.01 {
        return;
    }
    let (w, h) = (50.0 * s, 19.0 * s);
    let body = Rect::new(at.x - w * 0.5, at.y - h, w, h);
    p.shadow(Rect::new(body.x + 2.0, body.bottom() - 2.0, body.w - 4.0, 4.0), 2.0, 4.0, Color::rgba(0, 0, 0, 0.5 * a));
    p.rounded(body, 4.5 * s, Color::rgba(238, 241, 246, a));
    // the windows, the windscreen a little deeper, the doors between
    p.rounded(Rect::new(body.x + 4.0 * s, body.y + 3.0 * s, body.w - 10.0 * s, 6.5 * s), 1.5 * s, Color::rgba(28, 40, 62, a));
    p.rounded(Rect::new(body.right() - 5.0 * s, body.y + 3.0 * s, 3.0 * s, 9.5 * s), 1.2 * s, Color::rgba(28, 40, 62, a));
    for dx in [0.33, 0.62] {
        p.rect(Rect::new(body.x + body.w * dx, body.y + 3.0 * s, 1.0 * s, body.h - 7.0 * s), Color::rgba(150, 160, 178, a));
    }
    p.rect(Rect::new(body.x + 2.0 * s, body.bottom() - 5.5 * s, body.w - 4.0 * s, 1.8 * s), accent().alpha(a));
    p.circle(Vec2::new(body.right() - 1.6 * s, body.bottom() - 6.5 * s), 1.3 * s, LINE.alpha(a));
    for wx in [body.x + 11.0 * s, body.right() - 12.0 * s] {
        p.circle(Vec2::new(wx, body.bottom()), 4.2 * s, Color::rgba(14, 18, 28, a));
        p.circle(Vec2::new(wx, body.bottom()), 1.7 * s, Color::rgba(160, 168, 184, a));
    }
}

// --- the folder ------------------------------------------------------------------------------

/// What the folder pointed to is.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Folder {
    Fine,
    /// None was given (nothing was found by itself).
    Empty,
    Missing,
    /// openOMSI's own folder instead of OMSI 2's.
    Own,
    /// An OMSI 2 that lacks these.
    Lacks(String),
}

/// What `root` is, from whether it exists, whether it is openOMSI's own folder and what of
/// the original's essentials it lacks (`omsi_cfg::missing_original_essentials`; the same as
/// `state::root_problem`, said for the welcome).
fn folder_verdict(root: &str, exists: bool, own: bool, missing: &[String]) -> Folder {
    if root.trim().is_empty() {
        Folder::Empty
    } else if !exists {
        Folder::Missing
    } else if own || missing.iter().any(|m| m.contains("content folder")) {
        Folder::Own
    } else if missing.is_empty() {
        Folder::Fine
    } else {
        Folder::Lacks(missing.iter().take(3).cloned().collect::<Vec<_>>().join(", "))
    }
}

/// The folder being pointed to: what was typed or chosen, else the one set.
fn folder_text(l: &Launcher) -> String {
    l.pages.setup_root.clone().unwrap_or_else(|| l.state.config.root.clone())
}

/// What the folder pointed to is - looked at again when the path changes, and every second
/// (a folder being copied meanwhile), not at every frame.
fn verdict(l: &mut Launcher) -> Folder {
    let root = folder_text(l);
    let now = l.ui.time;
    if let Some((seen, at, v)) = &l.welcome.folder_seen {
        if *seen == root && now - at < 1.0 {
            return v.clone();
        }
    }
    let p = std::path::Path::new(root.trim());
    let v = folder_verdict(&root, p.is_dir(), p.join("openomsi.exe").exists() || p.join("openomsi").is_file(), &omsi_cfg::missing_original_essentials(p));
    l.welcome.folder_seen = Some((root, now, v.clone()));
    v
}

/// The folder kept as the one to play from (as the Setup page's Save does): what was read of
/// the folders before is forgotten, and the content and the drivers are read from it.
fn save_folder(l: &mut Launcher) {
    let Some(root) = l.pages.setup_root.clone() else { return };
    l.state.config.root = root.trim().to_string();
    match core::save_config(&l.state.config) {
        Ok(()) => {
            omsi_cfg::content_changed();
            l.state.config = core::load_config();
            l.pages.setup_root = None;
            l.state.load_content();
            l.state.load_profiles();
            // (what the start said was missing is there now)
            l.state.set_status(String::new(), false);
            log::info!("launcher: the welcome set the OMSI 2 folder to {}", l.state.config.root);
        }
        Err(e) => l.state.set_status(format!("{e:#}"), true),
    }
}

/// The folder: its path (typed, or chosen with Browse), and what it is. Returns the height.
fn folder(l: &mut Launcher, r: Rect) -> f32 {
    let phone = mobile::mobile();
    let mut y = r.y;
    l.ui.heading(Rect::new(r.x, y, r.w, 26.0), "OMSI 2 folder", None);
    y += 28.0;
    // (the path and Browse side by side; on a phone the path has the width, Browse under it)
    let bw = if phone { 140.0 } else { 120.0 };
    let (field, browse) = if phone { (Rect::new(r.x, y, r.w, 44.0), Rect::new(r.x, y + 54.0, bw, 44.0)) } else { (Rect::new(r.x, y, r.w - bw - 10.0, 44.0), Rect::new(r.right() - bw, y, bw, 44.0)) };
    let mut root = folder_text(l);
    if l.ui.text_input("welcome-folder", field, &mut root, "C:\\Program Files (x86)\\Steam\\steamapps\\common\\OMSI 2", Some("folder_open")) {
        l.pages.setup_root = Some(root.clone());
    }
    if l.ui.button("welcome-folder-browse", browse, "Browse", None, ButtonKind::Normal) {
        if phone {
            l.browse(mobile::Purpose::Root, &root);
        } else if let Some(p) = core::pick_folder("The OMSI 2 folder (with maps and Vehicles in it)") {
            l.pages.setup_root = Some(p.to_string_lossy().to_string());
        }
    }
    y = browse.bottom() + 14.0;
    let v = verdict(l);
    let (icon, colour, text) = match &v {
        Folder::Fine => ("check_circle", OK, omsi_ui::tr("A complete OMSI 2: the maps, the buses and everything the game needs are there.").to_string()),
        Folder::Empty => ("warning", WARN, omsi_ui::tr("openOMSI did not find OMSI 2 by itself. Choose the folder with Omsi.exe in it.").to_string()),
        Folder::Missing => ("warning", WARN, omsi_ui::tr("That folder does not exist.").to_string()),
        Folder::Own => ("warning", WARN, omsi_ui::tr("That is openOMSI's own folder. Choose the original OMSI 2, the one with Omsi.exe in it.").to_string()),
        Folder::Lacks(what) => ("warning", WARN, omsi_ui::tr("Not a complete OMSI 2: it lacks %{what}.").replace("%{what}", what)),
    };
    l.ui.icon(icon, Vec2::new(r.x + 10.0, y + 9.0), 17.0, colour);
    y += l.ui.paragraph(&text, Vec2::new(r.x + 28.0, y), r.w - 28.0, 13.5, Weight::Medium, colour) + 14.0;
    y += l.ui.paragraph("Steam, the box version, a folder you moved yourself - all fine. openOMSI only reads it and never writes into it.", Vec2::new(r.x, y), r.w.min(700.0), 13.0, Weight::Regular, TEXT_DIM);
    y - r.y
}

// --- the driver ------------------------------------------------------------------------------

/// The driver chosen (in the launcher's config, and one of this computer's), or the one just
/// made (before the list of drivers has been read again).
fn chosen_driver(l: &Launcher) -> Option<String> {
    let current = l.state.config.profile.trim();
    if current.is_empty() {
        return None;
    }
    l.state.profiles.iter().find(|n| n.eq_ignore_ascii_case(current)).cloned().or_else(|| l.welcome.made.clone().filter(|m| m.eq_ignore_ascii_case(current)))
}

/// A driver's personnel number and code in the companion's `personnel.json` (`companion::
/// signon`, kept by driver key: the personnel file's name in lower case), if they were made
/// already - they are made at the first sign-on.
fn personnel_in(text: &str, driver: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let e = v.get("drivers")?.get(driver.trim().to_lowercase())?;
    let (number, code) = (e.get("number")?.as_str()?, e.get("code")?.as_str()?);
    let digits = |s: &str, n: std::ops::RangeInclusive<usize>| n.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    (digits(number, 3..=10) && digits(code, 3..=8)).then(|| (number.to_string(), code.to_string()))
}

/// `name`'s personnel data, read once for each driver shown.
fn personnel_of(l: &mut Launcher, name: &str) -> Option<(String, String)> {
    if l.welcome.personnel.as_ref().map(|p| p.0.as_str()) != Some(name) {
        let found = crate::lan::data_dir().and_then(|d| std::fs::read_to_string(d.join("personnel.json")).ok()).and_then(|t| personnel_in(&t, name));
        l.welcome.personnel = Some((name.to_string(), found));
    }
    l.welcome.personnel.as_ref().and_then(|p| p.1.clone())
}

/// Make the driver typed (as the drivers step does) and choose them; their pass pops in.
fn create(l: &mut Launcher) -> bool {
    let name = l.pages.new_driver.trim().to_string();
    if name.is_empty() {
        l.ui.focus = Some(id_of(NAME));
        l.state.set_status(omsi_ui::tr("Type a name for your driver first.").to_string(), false);
        return false;
    }
    super::pages::create_driver(l);
    if l.state.config.profile != name {
        return false;
    }
    log::info!("launcher: the welcome made the driver {name}");
    // (the pass shows the number and code to sign on with, made now as Omsi-Hub made them)
    l.welcome.personnel = Some((name.clone(), crate::companion::personnel_of_driver(&name)));
    l.welcome.made = Some(name);
    l.welcome.typing = false;
    l.welcome.pass_in = Sprung::at(0.0);
    true
}

/// Choose a driver already on this computer.
fn choose(l: &mut Launcher, name: &str) {
    l.state.config.profile = name.to_string();
    let _ = core::save_config(&l.state.config);
    l.state.load_profile();
    l.state.touched();
    l.welcome.typing = false;
    l.pages.new_driver.clear();
    l.welcome.pass_in = Sprung::at(0.0);
}

/// Next on the driver's step: with a name typed the driver is made first; without a driver
/// the field asks for one.
fn driver_next(l: &mut Launcher) {
    let typed = !l.pages.new_driver.trim().is_empty();
    if (l.welcome.typing || chosen_driver(l).is_none()) && typed {
        if create(l) {
            l.welcome.next();
        }
    } else if chosen_driver(l).is_some() {
        l.welcome.next();
    } else {
        l.ui.focus = Some(id_of(NAME));
        l.state.set_status(omsi_ui::tr("Type a name for your driver first.").to_string(), false);
    }
}

/// The driver: a new one (its name and Create) or the chosen one's pass, and beside it (under
/// it on a phone) the drivers already on this computer. Returns the height used.
fn driver(l: &mut Launcher, r: Rect) -> f32 {
    let phone = mobile::mobile();
    let names = l.state.profiles.clone();
    let chosen = chosen_driver(l);
    let form = l.welcome.typing || chosen.is_none();
    let two = !names.is_empty() && !phone && r.w >= 640.0;
    let lw = if two { ((r.w - 32.0) * 0.48).floor() } else { r.w };
    let left = Rect::new(r.x, r.y, lw, r.h);
    let lh = match chosen.clone().filter(|_| !form) {
        Some(name) => pass(l, left, &name),
        None => new_driver(l, left, chosen.is_some()),
    };
    if names.is_empty() {
        return lh;
    }
    let at = if two { Vec2::new(r.x + lw + 32.0, r.y) } else { Vec2::new(r.x, r.y + lh + 24.0) };
    let rh = driver_list(l, Rect::new(at.x, at.y, r.right() - at.x, r.h), &names, chosen.as_deref(), !two);
    if two {
        lh.max(rh)
    } else {
        lh + 24.0 + rh
    }
}

/// The new driver's name and Create (Enter makes them as well; Escape gives up when there is
/// a driver to go back to). Returns the height.
fn new_driver(l: &mut Launcher, r: Rect, can_cancel: bool) -> f32 {
    let mut y = r.y;
    l.ui.heading(Rect::new(r.x, y, r.w, 26.0), "New driver", None);
    y += 28.0;
    if l.welcome.focus_name {
        l.welcome.focus_name = false;
        l.ui.focus = Some(id_of(NAME));
    }
    let focused = l.ui.focus == Some(id_of(NAME));
    let enter = focused && l.ui.input.keys.contains(&Key::Enter);
    let escape = focused && l.ui.input.keys.contains(&Key::Escape);
    l.ui.text_input(NAME, Rect::new(r.x, y, r.w, 44.0), &mut l.pages.new_driver, "The new driver's name", Some("person"));
    y += 56.0;
    let cw = if can_cancel { (r.w - 10.0) * 0.5 } else { r.w };
    if l.ui.button("welcome-driver-create", Rect::new(r.x, y, cw, 44.0), "Create", Some("add"), ButtonKind::Primary) || enter {
        create(l);
    }
    if can_cancel && (l.ui.button("welcome-driver-cancel", Rect::new(r.x + cw + 10.0, y, cw, 44.0), "Cancel", Some("close"), ButtonKind::Normal) || escape) {
        l.welcome.typing = false;
        l.pages.new_driver.clear();
        l.ui.focus = None;
    }
    y += 58.0;
    let note = "The driver's name is the name of their personnel file, as in OMSI 2: the game adds every duty to it.";
    y += l.ui.paragraph(note, Vec2::new(r.x, y), r.w, 12.5, Weight::Regular, TEXT_DIM);
    y - r.y
}

/// The chosen driver's personnel pass, as Omsi-Hub hands it over to a new driver: their
/// initial and name, their level, and - when the companion has made them - the personnel
/// number and code they sign on with. Under it, a new driver after all. Returns the height.
fn pass(l: &mut Launcher, r: Rect, name: &str) -> f32 {
    let (dt, motion) = (l.ui.dt, l.ui.motion);
    let k = l.welcome.pass_in.go(1.0, dt, PASS, motion);
    let numbers = personnel_of(l, name);
    let level = l.state.profile.as_ref().filter(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.level).unwrap_or(1);
    let h = if numbers.is_some() { 214.0 } else { 196.0 };
    let card = Rect::new(r.x, r.y + (1.0 - k) * 12.0, r.w, h);
    l.ui.p().shadow(card.inset(-1.0), SHEET_RADIUS, 18.0, Color::rgba(0, 0, 0, 0.35 * k.clamp(0.0, 1.0)));
    l.ui.p().rounded(card, SHEET_RADIUS, FIELD);
    // its head: the company's band
    let band = Rect::new(card.x, card.y, card.w, 36.0);
    l.ui.push_clip(band, 0.0);
    l.ui.p().rounded(card, SHEET_RADIUS, accent());
    l.ui.pop_clip();
    let label = format!("OPENOMSI  ·  {}", omsi_ui::tr("Personnel pass").to_uppercase());
    l.ui.text_in(&label, Rect::new(band.x + 18.0, band.y, band.w - 60.0, band.h), 10.5, Weight::Bold, on_accent(), Align::Left);
    l.ui.icon("directions_bus", Vec2::new(band.right() - 24.0, band.center().y), 18.0, on_accent());
    let mono = Rect::new(card.x + 18.0, band.bottom() + 18.0, 60.0, 60.0);
    l.ui.p().rounded(mono, RADIUS, accent().alpha(0.28));
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    l.ui.text_in(&initial, mono, 24.0, Weight::Bold, TEXT, Align::Center);
    let tx = mono.right() + 16.0;
    l.ui.text_in(name, Rect::new(tx, mono.y + 4.0, card.right() - tx - 18.0, 28.0), 21.0, Weight::Bold, TEXT, Align::Left);
    let role = omsi_ui::tr("Bus driver · level %{level}").replace("%{level}", &level.to_string());
    l.ui.text_in(&role, Rect::new(tx, mono.y + 34.0, card.right() - tx - 18.0, 20.0), 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
    let fy = mono.bottom() + 16.0;
    l.ui.p().rect(Rect::new(card.x + 18.0, fy, card.w - 36.0, 1.0), HAIRLINE);
    match numbers {
        Some((number, code)) => {
            for (k, (what, value)) in [("Personnel number", number), ("Code", code)].iter().enumerate() {
                let x = card.x + 18.0 + k as f32 * (card.w * 0.5);
                l.ui.text_in(&omsi_ui::tr(what).to_uppercase(), Rect::new(x, fy + 12.0, card.w * 0.5 - 24.0, 14.0), 10.0, Weight::Bold, TEXT_DIM, Align::Left);
                l.ui.text_in(value, Rect::new(x, fy + 28.0, card.w * 0.5 - 24.0, 24.0), 20.0, Weight::Bold, TEXT, Align::Left);
            }
            l.ui.text_in("You sign on with these before every duty.", Rect::new(card.x + 18.0, fy + 56.0, card.w - 36.0, 16.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
        None => {
            l.ui.paragraph("Your personnel number and code are made when you first sign on for a duty.", Vec2::new(card.x + 18.0, fy + 10.0), card.w - 36.0, 12.5, Weight::Regular, TEXT_DIM);
        }
    }
    l.ui.p().rounded_border(card, SHEET_RADIUS, 1.0, EDGE);
    // (it pops in out of the sheet)
    if k < 0.999 {
        l.ui.p().rounded(card.inset(-1.0), SHEET_RADIUS, PANEL.alpha((1.0 - k).clamp(0.0, 1.0)));
    }
    let y = r.y + h + 12.0;
    let nw = l.ui.width(&omsi_ui::tr("New driver"), 13.0, Weight::Medium) + 52.0;
    if l.ui.button("welcome-driver-new", Rect::new(r.x, y, nw, 34.0), "New driver", Some("add"), ButtonKind::Ghost) {
        l.welcome.typing = true;
        l.welcome.focus_name = true;
        l.pages.new_driver.clear();
    }
    h + 12.0 + 34.0
}

/// The drivers on this computer as rows to choose (`all`: every one, the page scrolls - else
/// five, the list scrolls). Returns the height.
fn driver_list(l: &mut Launcher, r: Rect, names: &[String], chosen: Option<&str>, all: bool) -> f32 {
    let head = omsi_ui::tr("On this computer").to_uppercase();
    let hw = l.ui.text_in(&head, Rect::new(r.x, r.y + 2.0, r.w, 22.0), 11.0, Weight::Bold, TEXT_DIM, Align::Left);
    l.ui.text_in(&names.len().to_string(), Rect::new(r.x + hw + 8.0, r.y + 2.0, 40.0, 22.0), 11.0, Weight::Bold, TEXT_FAINT, Align::Left);
    let (row, gap) = (56.0, 8.0);
    let shown = if all { names.len() } else { names.len().min(5) };
    let view = Rect::new(r.x - 4.0, r.y + 30.0, r.w + 8.0, shown as f32 * (row + gap) - gap + 8.0);
    let off = if all { 0.0 } else { l.ui.scroll.get(&id_of("welcome-drivers")).copied().unwrap_or(0.0) };
    if !all {
        l.ui.push_clip(view, 6.0);
    }
    let mut pick = None;
    for (k, name) in names.iter().enumerate() {
        let base = Rect::new(r.x, view.y + 4.0 + k as f32 * (row + gap) - off, r.w, row);
        if !l.ui.rect_visible(base) {
            continue;
        }
        let id = id_of(&format!("welcome-driver-{name}"));
        let t = l.ui.tile(id, base, RADIUS);
        let on = chosen.is_some_and(|c| c.eq_ignore_ascii_case(name));
        let blue = l.ui.anim(id ^ 0xb1e, if on { 1.0 } else { 0.0 }, 0.06);
        l.ui.tile_shadow(&t);
        l.ui.p().rounded(t.r, RADIUS, FIELD.mix(HOVER, t.hover).mix(accent(), blue));
        l.ui.tile_light(&t, 0.05);
        if blue < 0.99 {
            l.ui.tile_edge(&t, 1.0, EDGE.alpha(1.0 - blue));
        }
        let mono = Rect::new(t.r.x + 10.0, t.r.y + 10.0, 36.0, 36.0);
        l.ui.p().rounded(mono, 8.0, Color::WHITE.alpha(0.09 + 0.09 * blue));
        let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        l.ui.text_in(&initial, mono, 15.0, Weight::Bold, TEXT, Align::Center);
        l.ui.text_in(name, Rect::new(mono.right() + 12.0, t.r.y, t.r.w - 100.0, t.r.h), 14.5, Weight::Bold, TEXT, Align::Left);
        if on {
            l.ui.icon("check", Vec2::new(t.r.right() - 22.0, t.r.center().y), 18.0, on_accent().alpha(blue));
        }
        if t.clicked && !on {
            pick = Some(name.clone());
        }
    }
    if !all {
        l.ui.pop_clip();
        l.ui.scroll_keep("welcome-drivers", view, names.len() as f32 * (row + gap) - gap + 8.0);
    }
    if let Some(name) = pick {
        log::info!("launcher: the welcome chose the driver {name}");
        choose(l, &name);
    }
    30.0 + view.h
}

// --- ready -----------------------------------------------------------------------------------

/// The way on: the tour or straight to the start, as tiles with a photo each (the start's
/// look); with the classic launcher chosen, a picture of it instead. Returns the height.
fn ready(l: &mut Launcher, r: Rect) -> f32 {
    let phone = mobile::mobile();
    if classic(l) {
        let w = r.w.min(560.0);
        let pic = Rect::new(r.x + (r.w - w) * 0.5, r.y, w, w / 1.6);
        match l.pictures.get("ui-classic").copied() {
            Some((tex, pw, ph)) => l.ui.image_cover(pic, tex, RADIUS, pw, ph),
            None => l.ui.p().rounded(pic, RADIUS, GROUND),
        }
        l.ui.p().rounded_border(pic, RADIUS, 1.0, Color::WHITE.alpha(0.1));
        let y = pic.bottom() + 14.0;
        l.ui.text_in("From now on openOMSI opens in its classic launcher.", Rect::new(r.x, y, r.w, 20.0), 13.0, Weight::Regular, TEXT_DIM, Align::Center);
        return y + 20.0 - r.y;
    }
    let stacked = phone || r.w < 560.0;
    let gap = 16.0;
    let tw = if stacked { r.w } else { (r.w - gap) * 0.5 };
    let th = if phone { 158.0 } else { 236.0 };
    let tiles = [(Then::Tour, "Take the tour", "route", "mode-tour", "A minute along the launcher: where you set up a duty, your service record and the settings."), (Then::Start, "Start right away", "play_arrow", "mode-free", "Straight to the start: choose how you want to drive today.")];
    let mut then = None;
    for (k, (what, title, icon, picture, text)) in tiles.iter().enumerate() {
        let base = if stacked { Rect::new(r.x, r.y + k as f32 * (th + 12.0), tw, th) } else { Rect::new(r.x + k as f32 * (tw + gap), r.y, tw, th) };
        let t = l.ui.tile(id_of(&format!("welcome-ready-{k}")), base, SHEET_RADIUS);
        l.ui.tile_shadow(&t);
        match l.pictures.get(picture).copied() {
            Some((tex, w, h)) => l.ui.tile_photo(&t, t.r, SHEET_RADIUS, tex, w, h),
            None => l.ui.p().rounded(t.r, SHEET_RADIUS, FIELD),
        }
        l.ui.p().rounded_gradient(t.r, SHEET_RADIUS, Color::rgba(9, 12, 24, 0.1), Color::rgba(9, 12, 24, 0.92));
        // (the words lie on the photo's lower half: a light photo - the tour's road in the
        // morning - darkened there once more)
        let low = Rect::new(t.r.x, t.r.y + t.r.h * 0.35, t.r.w, t.r.h * 0.65);
        l.ui.p().rounded_gradient(low, SHEET_RADIUS, Color::rgba(9, 12, 24, 0.0), Color::rgba(9, 12, 24, 0.6));
        l.ui.tile_light(&t, 0.15);
        l.ui.tile_edge(&t, 1.0, EDGE);
        let px = if phone { 12.5 } else { 13.5 };
        let text_h = l.ui.paragraph_height(text, base.w - 40.0, px, Weight::Medium);
        let badge = Rect::new(t.r.x + 20.0, (t.r.bottom() - 18.0 - text_h - 36.0 - 48.0).max(t.r.y + 12.0), 40.0, 40.0);
        l.ui.p().rounded(badge, RADIUS, if *what == Then::Start { accent() } else { Color::WHITE.alpha(0.16).mix(accent(), 0.9 * t.hover) });
        l.ui.icon(icon, badge.center(), 20.0, Color::WHITE.mix(on_accent(), if *what == Then::Start { 1.0 } else { 0.9 * t.hover }));
        l.ui.text_in(title, Rect::new(t.r.x + 20.0, badge.bottom() + 6.0, base.w - 40.0, 30.0), if phone { 19.0 } else { 23.0 }, Weight::Bold, Color::WHITE, Align::Left);
        l.ui.paragraph(text, Vec2::new(t.r.x + 20.0, badge.bottom() + 42.0), base.w - 40.0, px, Weight::Medium, Color::rgba(232, 235, 242, 0.92));
        if t.clicked {
            then = Some(*what);
        }
    }
    if let Some(t) = then {
        finish(l, t);
    }
    if stacked {
        2.0 * th + 12.0
    } else {
        th
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_step_is_only_there_when_the_folder_has_to_be_pointed_to() {
        assert_eq!(steps(false), vec![Step::Language, Step::Interface, Step::Animations, Step::Driver, Step::Ready]);
        assert_eq!(steps(true), vec![Step::Language, Step::Interface, Step::Animations, Step::Folder, Step::Driver, Step::Ready]);
    }

    #[test]
    fn it_opens_on_the_first_start_and_where_it_is_asked_to() {
        let mut s = core::settings_from_text(None);
        assert_eq!(due(&s, None), Some(Step::Language), "welcome_done is false until it was seen");
        s["welcome_done"] = json!(true);
        assert_eq!(due(&s, None), None);
        assert_eq!(due(&s, Some("driver")), Some(Step::Driver));
        assert_eq!(due(&s, Some("Ready")), Some(Step::Ready));
        assert_eq!(due(&s, Some("whatever")), Some(Step::Language));
        s["welcome_done"] = json!(false);
        assert_eq!(due(&s, Some("off")), None);
    }

    #[test]
    fn going_on_and_back_slides_from_the_side_the_step_lies_on() {
        let mut w = Welcome::closed();
        w.open_at(Step::Language, false);
        w.next();
        assert_eq!(w.step, Step::Interface);
        // (the dialog lies over the language: the sheet stays as it is)
        assert_eq!((w.enter, w.side), (Sprung::at(0.0), 0.0));
        w.enter = Sprung::at(1.0);
        w.next();
        assert_eq!((w.step, w.side, w.enter.x), (Step::Animations, 1.0, 0.0));
        w.next();
        assert_eq!(w.step, Step::Driver, "no folder step when the folder is right");
        assert!(w.focus_name, "the driver's name field wants the keyboard");
        w.back();
        assert_eq!((w.step, w.side), (Step::Animations, -1.0));
        let mut w = Welcome::closed();
        w.open_at(Step::Ready, false);
        w.next();
        assert_eq!(w.step, Step::Ready, "nothing after the last step");
        w.open_at(Step::Language, false);
        w.back();
        assert_eq!(w.step, Step::Language, "nothing before the first");
    }

    #[test]
    fn the_classic_launcher_is_a_restart_and_the_rest_a_page() {
        assert_eq!(outcome("classic", Then::Tour, true), Outcome::Restart);
        assert_eq!(outcome("Classic", Then::Start, false), Outcome::Restart);
        assert_eq!(outcome("new", Then::Start, true), Outcome::Show { page: Page::Drive, tour: false });
        assert_eq!(outcome("new", Then::Tour, true), Outcome::Show { page: Page::Drive, tour: true });
        assert_eq!(outcome("", Then::Lessons, true), Outcome::Show { page: Page::Tutorials, tour: false });
        assert_eq!(outcome("new", Then::Tour, false), Outcome::Show { page: Page::Setup, tour: false }, "without a game to drive, Setup says what is missing");
    }

    #[test]
    fn a_spring_of_its_own_comes_to_rest_and_is_there_at_once_without_animations() {
        let mut s = Sprung::at(0.0);
        let mut overshoot = false;
        for _ in 0..240 {
            overshoot |= s.go(1.0, 1.0 / 60.0, DIALOG, true) > 1.0;
        }
        assert!(s.rests_at(1.0), "{s:?}");
        assert!(overshoot, "the dialog comes up a little past and back");
        let mut s = Sprung::at(0.0);
        assert_eq!(s.go(1.0, 1.0 / 60.0, ENTER, false), 1.0);
        assert!(s.rests_at(1.0));
        let mut s = Sprung::at(0.0);
        let first = s.go(1.0, 1.0 / 60.0, ENTER, true);
        assert!(first > 0.0 && first < 0.2, "{first}");
    }

    #[test]
    fn the_language_tiles_fill_the_width_without_growing_too_narrow() {
        let (cols, w) = grid(908.0, 160.0, 10.0, 5);
        assert_eq!(cols, 5);
        assert!((w - 173.6).abs() < 1e-3);
        let (cols, w) = grid(251.0, 118.0, 10.0, 5);
        assert_eq!(cols, 2);
        assert!((w - 120.5).abs() < 1e-3);
        assert_eq!(grid(90.0, 118.0, 10.0, 5).0, 1, "at least one");
        assert_eq!(grid(4000.0, 160.0, 10.0, 5).0, 5, "at most `most`");
    }

    #[test]
    fn every_language_has_a_flag_or_its_script() {
        let mut p = Painter::new();
        let r = Rect::new(0.0, 0.0, 30.0, 20.0);
        let without: Vec<&str> = core::LANGUAGES.iter().map(|x| x.0).filter(|c| !paint_flag(&mut p, r, c)).collect();
        assert_eq!(without, vec!["BEL", "ZHT", "CHS"]);
    }

    #[test]
    fn the_folder_is_judged_as_the_setup_judges_it() {
        assert_eq!(folder_verdict(" ", false, false, &[]), Folder::Empty);
        assert_eq!(folder_verdict("D:/nowhere", false, false, &["maps".into()]), Folder::Missing);
        assert_eq!(folder_verdict("C:/openOMSI", true, true, &["Omsi.exe".into()]), Folder::Own);
        assert_eq!(folder_verdict("C:/x", true, false, &["the content folder".into()]), Folder::Own);
        assert_eq!(folder_verdict("C:/OMSI 2", true, false, &[]), Folder::Fine);
        assert_eq!(folder_verdict("C:/half", true, false, &["maps".into(), "Vehicles".into(), "Fonts".into(), "Sceneryobjects".into()]), Folder::Lacks("maps, Vehicles, Fonts".into()));
    }

    #[test]
    fn the_pass_shows_the_number_the_companion_made_and_nothing_made_up() {
        let file = r#"{ "drivers": { "anna": { "number": "482913", "code": "4821" }, "bad": { "number": "12ab", "code": "1" } } }"#;
        assert_eq!(personnel_in(file, "Anna"), Some(("482913".into(), "4821".into())), "kept under the name in lower case");
        assert_eq!(personnel_in(file, "Bert"), None);
        assert_eq!(personnel_in(file, "bad"), None, "a hand-edited entry that is no number");
        assert_eq!(personnel_in("not json", "anna"), None);
    }

    #[test]
    fn the_little_bus_drives_stops_and_comes_again() {
        assert_eq!(bus_at(0.0), (0.0, 0.0));
        assert_eq!(bus_at(1.8).0, 0.5, "it stands at the middle stop");
        let (along, seen) = bus_at(3.69);
        assert!(along > 0.99 && seen == 1.0);
        assert!(bus_at(4.59).1 < 0.01, "it fades at the last stop");
        assert_eq!(bus_at(4.6 + 1.8), bus_at(1.8), "and the same again");
        let mut last = 0.0;
        for k in 0..370 {
            let (a, _) = bus_at(k as f32 * 0.01);
            assert!(a >= last - 1e-6, "it never drives back");
            last = a;
        }
    }

    #[test]
    fn the_mouse_goes_from_tile_to_tile_and_each_rises_in_turn() {
        let (k, at, lift) = tiles_at(0.8);
        assert_eq!((k, at), (0, 0.0));
        assert!(lift > 0.9);
        let (k, at, lift) = tiles_at(1.15 + 0.1);
        assert_eq!(k, 1);
        assert!(at > 0.0 && at < 1.0, "on its way: {at}");
        assert!(lift < 0.01);
        assert_eq!(tiles_at(2.0 * 1.15 + 0.8).0, 2);
        let (again, first) = (tiles_at(3.0 * 1.15 + 0.8), tiles_at(0.8));
        assert_eq!(again.0, first.0);
        assert!((again.1 - first.1).abs() < 1e-4 && (again.2 - first.2).abs() < 1e-4);
    }
}
