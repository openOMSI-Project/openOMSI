//! The Home page: what the launcher opens with. A line with three stops over the greeting,
//! the ways to drive and the workshop's tools as a row of picture cards that slides (the
//! wheel, the arrow keys, a click on a card at the side), and under it a row of chips: the
//! driver, the mods and the other pages.

use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

use super::theme::*;
use super::ui::{id_of, Key};
use super::{Launcher, Page};

/// What a card leads to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Duty,
    Free,
    Online,
    Timetable,
    Gallery,
    Lines,
    Livery,
}

/// The picture a card draws (see `art`).
#[derive(Clone, Copy, PartialEq)]
enum Art {
    City,
    Country,
    Crowd,
    Board,
    Fleet,
    Plan,
    Paint,
}

struct Card {
    mode: Mode,
    title: &'static str,
    icon: &'static str,
    text: &'static str,
    art: Art,
}

const CARDS: [Card; 7] = [
    Card { mode: Mode::Duty, title: "Drive a duty", icon: "directions_bus", text: "A line and a tour from the timetable, its times kept stop by stop.", art: Art::City },
    Card { mode: Mode::Free, title: "Free drive", icon: "explore", text: "Only a map and a bus: go where the road takes you, nothing is booked.", art: Art::Country },
    Card { mode: Mode::Online, title: "Multiplayer", icon: "groups", text: "Drive with friends by a code, or join a server that is always on.", art: Art::Crowd },
    Card { mode: Mode::Timetable, title: "Timetable", icon: "departure_board", text: "The map's lines, their tours and when the buses go.", art: Art::Board },
    Card { mode: Mode::Gallery, title: "Bus gallery", icon: "photo_library", text: "Every bus you have, pictured: pick one and look at it from all sides.", art: Art::Fleet },
    Card { mode: Mode::Lines, title: "Line editor", icon: "route", text: "Lines of your own: click the stops, the way is found over the roads.", art: Art::Plan },
    Card { mode: Mode::Livery, title: "Livery studio", icon: "format_paint", text: "Paint a bus: colours, stripes, a name and a logo, seen on the model.", art: Art::Paint },
];

/// The chips under the cards: a page, its icon and its name.
const CHIPS: [(Page, &str, &str); 5] = [
    (Page::Mods, "extension", "Mods"),
    (Page::Settings, "tune", "Settings"),
    (Page::Controls, "keyboard", "Controls"),
    (Page::Sessions, "sports_esports", "Sessions"),
    (Page::Tutorials, "help", "Tutorials"),
];

/// The greeting for an hour of the day.
fn greeting(hour: i32) -> &'static str {
    match hour {
        5..=10 => "Good morning",
        11..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Good night",
    }
}

fn hours(h: f64) -> String {
    let m = (h * 60.0).round().max(0.0) as i64;
    format!("{}h {:02}m", m / 60, m % 60)
}

/// The card shown in the middle: kept between frames (the wheel and the arrow keys move it).
pub struct HomeView {
    pub focus: usize,
    /// The intro: how far it has run, and whether it is over (see `intro`).
    intro: f32,
    intro_done: bool,
}

impl Default for HomeView {
    /// (the second card in the middle: the first stands beside it, the row reads both ways)
    fn default() -> Self {
        HomeView { focus: 1, intro: 0.0, intro_done: false }
    }
}

/// Where the row is: the focused card's left edge put so that it stands in the middle.
fn row_x(centre: f32, card_w: f32, gap: f32, at: f32) -> f32 {
    centre - card_w * 0.5 - at * (card_w + gap)
}

pub fn draw(l: &mut Launcher, area: Rect) {
    if !l.home_asked {
        l.home_asked = true;
        l.state.load_profile();
    }
    let size = l.ui.size;
    let tall = area.h >= 620.0;
    let card_h = if tall { (area.h * 0.3).clamp(190.0, 250.0) } else { 170.0 };
    let card_w = (card_h * 1.12).min(area.w * 0.42);
    let gap = 18.0;

    // the stage: a glow behind the cards and a line through it that runs the window's width
    // (the whole of it - the line, the greeting, the cards, the chips - stands a little above
    // the middle of the page)
    let head = if tall { 136.0 } else { 104.0 };
    let block = head + card_h + 112.0;
    let band_y = area.y + ((area.h - block) * 0.42).max(0.0) + head;
    let mid = band_y + card_h * 0.5;
    l.ui.p().gradient(Rect::new(0.0, area.y - 30.0, size.x, mid - area.y + 30.0), BACKDROP(), BACKDROP().mix(ACCENT(), 0.10));
    l.ui.p().gradient(Rect::new(0.0, mid, size.x, (area.bottom() - mid).max(0.0)), BACKDROP().mix(ACCENT(), 0.10), BACKDROP());
    l.ui.p().gradient_h(Rect::new(0.0, mid - 1.0, size.x * 0.5, 2.0), ACCENT().alpha(0.0), ACCENT().alpha(0.85));
    l.ui.p().gradient_h(Rect::new(size.x * 0.5, mid - 1.0, size.x * 0.5, 2.0), ACCENT().alpha(0.85), ACCENT().alpha(0.0));
    l.ui.p().gradient_h(Rect::new(0.0, mid - 4.0, size.x * 0.5, 8.0), ACCENT().alpha(0.0), ACCENT().alpha(0.16));
    l.ui.p().gradient_h(Rect::new(size.x * 0.5, mid - 4.0, size.x * 0.5, 8.0), ACCENT().alpha(0.16), ACCENT().alpha(0.0));

    // the line with its three stops, the greeting under it
    let cx = area.x + area.w * 0.5;
    let line_y = band_y - head + 18.0;
    let t = l.page_t;
    // (a light running along the line through the cards, every few seconds)
    let sweep = (l.ui.time * 0.22).fract();
    let sx = -300.0 + (size.x + 600.0) * sweep;
    l.ui.p().gradient_h(Rect::new(sx - 240.0, mid - 2.0, 240.0, 4.0), ACCENT().alpha(0.0), ACCENT().lighten(0.4));
    l.ui.p().gradient_h(Rect::new(sx, mid - 2.0, 120.0, 4.0), ACCENT().lighten(0.4), ACCENT().alpha(0.0));
    stop_line(l, Vec2::new(cx, line_y), (area.w * 0.2).clamp(150.0, 260.0), t);
    let name = l.state.profile.as_ref().map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| l.state.config.profile.clone());
    let hour = omsi_launcher_lib::local_now().map(|t| t.3).unwrap_or(12);
    let hello = if name.is_empty() { omsi_ui::tr(greeting(hour)).to_string() } else { format!("{} {name}", omsi_ui::tr(greeting(hour))) };
    let a = appear(t, 0.15, 0.5);
    l.ui.text_in(&hello, Rect::new(area.x, line_y + 22.0 + 14.0 * (1.0 - a), area.w, 40.0), 30.0, Weight::Bold, TEXT().alpha(a), Align::Center);
    let a = appear(t, 0.25, 0.5);
    l.ui.text_in("How do you want to drive today?", Rect::new(area.x, line_y + 62.0 + 10.0 * (1.0 - a), area.w, 22.0), 14.0, Weight::Regular, TEXT_DIM().alpha(a), Align::Center);

    // the row of cards: the focused one in the middle, the wheel and the arrow keys move it
    let row = Rect::new(0.0, band_y - 10.0, size.x, card_h + 20.0);
    if l.ui.hover(row) && l.ui.input.wheel != Vec2::ZERO {
        let w = if l.ui.input.wheel.x.abs() > l.ui.input.wheel.y.abs() { -l.ui.input.wheel.x } else { -l.ui.input.wheel.y };
        if w > 0.0 {
            l.home.focus = (l.home.focus + 1).min(CARDS.len() - 1);
        } else if w < 0.0 {
            l.home.focus = l.home.focus.saturating_sub(1);
        }
    }
    let keys = l.ui.input.keys.clone();
    if l.ui.focus.is_none() {
        for k in keys {
            match k {
                Key::Right => l.home.focus = (l.home.focus + 1).min(CARDS.len() - 1),
                Key::Left => l.home.focus = l.home.focus.saturating_sub(1),
                Key::Enter => open(l, CARDS[l.home.focus].mode),
                _ => {}
            }
        }
    }
    let at = l.ui.anim(id_of("home-row"), l.home.focus as f32, 0.12);
    let x0 = row_x(cx, card_w, gap, at);
    for (k, c) in CARDS.iter().enumerate() {
        let r = Rect::new(x0 + k as f32 * (card_w + gap), band_y, card_w, card_h);
        if r.right() < -20.0 || r.x > size.x + 20.0 {
            continue;
        }
        // how far from the middle: the far ones fade into the backdrop
        let off = ((r.center().x - cx) / (card_w + gap)).abs();
        let fade = (1.0 - (off - 1.2).max(0.0) * 0.55).clamp(0.0, 1.0);
        // (the cards come in one after the other, from the middle out)
        let a = appear(t, 0.3 + off.min(4.0) * 0.08, 0.55);
        let fade = fade * a;
        if fade <= 0.02 {
            continue;
        }
        let r = Rect::new(r.x, r.y + 36.0 * (1.0 - a), r.w, r.h);
        if card(l, r, k, c, k == l.home.focus, fade) {
            if k == l.home.focus {
                open(l, c.mode);
            } else {
                l.home.focus = k;
            }
        }
    }
    // the dots under the row: which card is in the middle
    let dots_y = band_y + card_h + 22.0;
    let dw = 18.0;
    let dx = cx - dw * (CARDS.len() as f32 - 1.0) * 0.5;
    for k in 0..CARDS.len() {
        let c = Vec2::new(dx + k as f32 * dw, dots_y);
        let r = Rect::new(c.x - 8.0, c.y - 8.0, 16.0, 16.0);
        let (h, _, clicked) = l.ui.interact(id_of(&format!("home-dot-{k}")), r);
        if clicked {
            l.home.focus = k;
        }
        let on = k == l.home.focus;
        l.ui.p().circle(c, if on { 4.5 } else { 3.2 }, if on { ACCENT() } else if h { TEXT_DIM() } else { TRACK() });
    }

    // the chips: the driver first, then the other pages
    let chips_y = dots_y + 24.0;
    chips(l, Rect::new(area.x, chips_y, area.w, 44.0), t);
    signature(l, area, t);
}

/// The way a card goes.
fn open(l: &mut Launcher, mode: Mode) {
    match mode {
        Mode::Duty | Mode::Free => {
            let free = mode == Mode::Free;
            if l.state.choice.free != free {
                l.state.choice.free = free;
                l.state.touched();
            }
            l.drive.tab = 0;
            l.go(Page::Drive);
        }
        Mode::Online => l.go(Page::Multiplayer),
        Mode::Timetable => l.go(Page::Timetable),
        Mode::Gallery => l.go(Page::Buses),
        Mode::Lines => l.go(Page::Lines),
        Mode::Livery => l.go(Page::Livery),
    }
}

/// A line along the top with three stops on it: it grows out of the middle when the page
/// opens, the stops pop up on it, and a little bus runs along it from stop to stop.
fn stop_line(l: &mut Launcher, c: Vec2, half: f32, since: f32) {
    let y = c.y;
    let grow = appear(since, 0.0, 0.6);
    let hw = (half + 40.0) * grow;
    l.ui.p().gradient_h(Rect::new(c.x - hw, y - 2.5, (hw - half * grow).max(0.0), 5.0), ACCENT().alpha(0.0), ACCENT());
    l.ui.p().rect(Rect::new(c.x - half * grow, y - 2.5, half * 2.0 * grow, 5.0), ACCENT());
    l.ui.p().gradient_h(Rect::new(c.x + half * grow, y - 2.5, (hw - half * grow).max(0.0), 5.0), ACCENT(), ACCENT().alpha(0.0));
    let t = l.ui.time;
    for (k, col) in [DANGER(), ACCENT_2(), OK()].into_iter().enumerate() {
        // (popping up a little over their size, then settling)
        let x = ((since - 0.25 - k as f32 * 0.12) / 0.35).clamp(0.0, 1.0);
        if x <= 0.0 {
            continue;
        }
        let pop = 1.0 + (x * std::f32::consts::PI).sin() * 0.25 * (1.0 - x * 0.5);
        let p = Vec2::new(c.x + (k as f32 - 1.0) * half, y);
        let pulse = 0.5 + 0.5 * (t * 1.6 - k as f32 * 0.9).sin();
        l.ui.p().circle(p, (15.0 + 3.0 * pulse) * pop, col.alpha(0.14 * x));
        l.ui.p().circle(p, 12.0 * pop * x.min(1.0), RAIL());
        l.ui.p().circle(p, 10.0 * pop * x, col);
        l.ui.p().circle(p, 5.0 * pop * x, RAIL());
    }
    // the bus: from one stop to the next, a moment's halt at each, there and back
    if since > 1.0 {
        let cycle = 7.0;
        let ph = (t % cycle) / cycle;
        // 0..0.5 out, 0.5..1 back; within each half: two legs, each with a halt
        let (half_ph, back) = if ph < 0.5 { (ph * 2.0, false) } else { ((ph - 0.5) * 2.0, true) };
        let leg = (half_ph * 2.0).min(1.999);
        let in_leg = leg.fract();
        let moving = ((in_leg - 0.25) / 0.75).clamp(0.0, 1.0);
        let e = moving * moving * (3.0 - 2.0 * moving);
        let u = (leg.floor() + e) / 2.0;
        let u = if back { 1.0 - u } else { u };
        let p = Vec2::new(c.x - half + u * half * 2.0, y - 14.0);
        let a = appear(since, 1.0, 0.4);
        l.ui.icon("directions_bus", p, 18.0, TEXT().alpha(a));
        if moving > 0.0 && moving < 1.0 {
            let dir = if back { 1.0 } else { -1.0 };
            for k in 0..3 {
                let sx = p.x + dir * (12.0 + k as f32 * 7.0);
                l.ui.p().line(Vec2::new(sx, p.y - 4.0 + k as f32 * 4.0), Vec2::new(sx + dir * 8.0, p.y - 4.0 + k as f32 * 4.0), 1.5, TEXT_DIM().alpha(0.6 * a));
            }
        }
    }
}

/// The designer's mark, bottom right: a small badge and the name.
fn signature(l: &mut Launcher, area: Rect, since: f32) {
    let a = appear(since, 0.9, 0.6);
    if a <= 0.01 {
        return;
    }
    let by = omsi_ui::tr("Design by").to_string();
    let w1 = l.ui.width(&by, 11.0, Weight::Regular);
    let w2 = l.ui.width(DESIGNER, 12.5, Weight::Bold);
    let w = 34.0 + w1 + 6.0 + w2 + 14.0;
    let r = Rect::new(area.right() - w, area.bottom() - 34.0 + 8.0 * (1.0 - a), w, 30.0);
    let (h, _, _) = l.ui.interact(id_of("home-signature"), r);
    let t = l.ui.anim(id_of("home-signature"), if h { 1.0 } else { 0.0 }, 0.1);
    l.ui.p().rounded(r, 15.0, PANEL().alpha((0.6 + 0.3 * t) * a));
    l.ui.p().rounded_border(r, 15.0, 1.0, EDGE().mix(ACCENT(), t).alpha(a));
    mark(l, Vec2::new(r.x + 16.0, r.center().y), 10.0, a, l.ui.time * (0.6 + 2.0 * t));
    l.ui.text_in(&by, Rect::new(r.x + 32.0, r.y, w1 + 4.0, r.h), 11.0, Weight::Regular, TEXT_DIM().alpha(a), Align::Left);
    l.ui.text_in(DESIGNER, Rect::new(r.x + 32.0 + w1 + 6.0, r.y, w2 + 4.0, r.h), 12.5, Weight::Bold, TEXT().mix(ACCENT_2(), t).alpha(a), Align::Left);
    l.ui.tooltip(r, &format!("{} {DESIGNER}: {}", omsi_ui::tr("Design by"), omsi_ui::tr("the looks, the Home page, the bus gallery, the livery studio and the line editor")));
}

/// The designer's badge: a ring in the look's two colours turning round an "S".
pub fn mark(l: &mut Launcher, c: Vec2, r: f32, a: f32, turn: f32) {
    l.ui.p().circle(c, r, RAIL().alpha(a));
    l.ui.p().arc(c, r - 2.5, r, turn, turn + std::f32::consts::PI, ACCENT().alpha(a));
    l.ui.p().arc(c, r - 2.5, r, turn + std::f32::consts::PI, turn + std::f32::consts::TAU, ACCENT_2().alpha(a));
    l.ui.text_in("S", Rect::new(c.x - r, c.y - r, r * 2.0, r * 2.0), r * 1.15, Weight::Black, TEXT().alpha(a), Align::Center);
}

/// How long the intro runs (seconds).
const INTRO: f32 = 2.6;

/// The launcher's first moments: a bus drives through the dark along the line, the name
/// and the designer's mark come up, and the whole of it fades into the Home page. A click
/// or a key ends it at once; it runs once, when the launcher opens on Home.
pub fn intro(l: &mut Launcher) {
    if l.home.intro_done {
        return;
    }
    if l.page != Page::Home || l.ui.input.pressed || !l.ui.input.keys.is_empty() {
        l.home.intro_done = true;
        l.ui.input.pressed = false;
        l.ui.input.keys.clear();
        return;
    }
    l.home.intro += l.ui.dt;
    let t = l.home.intro;
    if t >= INTRO {
        l.home.intro_done = true;
        // (the Home page comes in from here, not from behind the intro)
        l.page_t = 0.0;
        return;
    }
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    let out = 1.0 - ((t - (INTRO - 0.5)) / 0.5).clamp(0.0, 1.0);
    l.ui.solid(full);
    l.ui.p().rect(full, BACKDROP().alpha(out));
    let mid = size.y * 0.55;
    // the road: a line drawn across, the bus on it with its light streaks
    let line = appear(t, 0.0, 0.5);
    l.ui.p().gradient_h(Rect::new(0.0, mid, size.x * line, 2.0), ACCENT().alpha(0.0), ACCENT().alpha(0.9 * out));
    l.ui.p().gradient(Rect::new(0.0, mid - 60.0, size.x, 60.0), BACKDROP().alpha(0.0), ACCENT().alpha(0.06 * out));
    let drive = ((t - 0.2) / 1.6).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - drive).powi(2);
    let len = (size.x * 0.13).clamp(110.0, 190.0);
    let x = -len + (size.x * 0.5 + len * 0.5) * e;
    for k in 0..5 {
        let y = mid - 8.0 - k as f32 * 9.0;
        let w = 80.0 + 70.0 * ((k * 37 % 5) as f32 / 4.0);
        l.ui.p().gradient_h(Rect::new(x - w - 10.0 - k as f32 * 14.0, y, w, 2.0), TEXT().alpha(0.0), TEXT().alpha(0.35 * out * (1.0 - e * 0.7)));
    }
    bus(l, Vec2::new(x, mid), len, TEXT().alpha(out), PANEL().mix(ACCENT(), 0.35).alpha(out));
    // the name, and the designer's mark under it
    let a = appear(t, 1.0, 0.5) * out;
    let cy = mid - len * 0.28 - 70.0;
    let w1 = l.ui.width("open", 40.0, Weight::Regular);
    let w2 = l.ui.width("OMSI", 40.0, Weight::Black);
    let x0 = size.x * 0.5 - (w1 + w2) * 0.5;
    l.ui.text("open", Vec2::new(x0, cy + 12.0 * (1.0 - a)), 40.0, Weight::Regular, TEXT().alpha(a), Align::Left);
    l.ui.text("OMSI", Vec2::new(x0 + w1, cy + 12.0 * (1.0 - a)), 40.0, Weight::Black, ACCENT_2().alpha(a), Align::Left);
    let b = appear(t, 1.35, 0.5) * out;
    let by = format!("{} {DESIGNER}", omsi_ui::tr("Design by"));
    let bw = l.ui.width(&by, 13.0, Weight::Medium) + 30.0;
    mark(l, Vec2::new(size.x * 0.5 - bw * 0.5 + 9.0, cy + 34.0), 9.0, b, t * 2.0);
    l.ui.text_in(&by, Rect::new(size.x * 0.5 - bw * 0.5 + 24.0, cy + 24.0, bw, 20.0), 13.0, Weight::Medium, TEXT_DIM().alpha(b), Align::Left);
}

/// One card: its picture, the icon and the name over it, a line about it. True when clicked.
fn card(l: &mut Launcher, r: Rect, k: usize, c: &Card, focused: bool, fade: f32) -> bool {
    let id = id_of(&format!("home-card-{k}"));
    let (h, held, clicked) = l.ui.interact(id, r);
    let t = l.ui.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
    let f = l.ui.anim(id ^ 9, if focused { 1.0 } else { 0.0 }, 0.12);
    let lift = 6.0 * f + 3.0 * t;
    let r = if held { r.inset(1.0) } else { Rect::new(r.x, r.y - lift, r.w, r.h) };
    l.ui.solid(r);
    if f > 0.01 {
        l.ui.p().shadow(Rect::new(r.x, r.y + 10.0, r.w, r.h), RADIUS + 4.0, 34.0, SHADOW().alpha(f * fade));
        l.ui.p().shadow(r, RADIUS + 4.0, 22.0, ACCENT().alpha(0.28 * f * fade));
    }
    l.ui.p().rounded(r, RADIUS + 4.0, PANEL().alpha(fade));
    l.ui.push_clip(Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0), RADIUS + 3.0);
    art(l, r, c.art, fade, t);
    // the words over the foot of the picture, on a shade that keeps them readable
    let foot = Rect::new(r.x, r.y + r.h * 0.42, r.w, r.h * 0.58);
    l.ui.p().gradient(foot, PANEL().alpha(0.0), PANEL().alpha(0.94 * fade));
    l.ui.pop_clip();

    let badge = Rect::new(r.x + 16.0, r.y + r.h * 0.5, 30.0, 30.0);
    l.ui.p().rounded(badge, 8.0, LIFT().alpha(0.16 * fade));
    l.ui.p().rounded_border(badge, 8.0, 1.0, LIFT().alpha(0.22 * fade));
    l.ui.icon(c.icon, badge.center(), 17.0, TEXT().alpha(fade));
    let ty = badge.bottom() + 8.0;
    l.ui.text_in(c.title, Rect::new(r.x + 16.0, ty, r.w - 32.0, 24.0), 17.0, Weight::Bold, TEXT().alpha(fade), Align::Left);
    if r.bottom() - ty > 54.0 {
        l.ui.push_clip(Rect::new(r.x, ty + 26.0, r.w, r.bottom() - ty - 32.0), 0.0);
        l.ui.paragraph(c.text, Vec2::new(r.x + 16.0, ty + 27.0), r.w - 32.0, 11.5, Weight::Regular, TEXT_SOFT().alpha(fade));
        l.ui.pop_clip();
    }
    let edge = EDGE().mix(ACCENT(), (f + t * 0.6).min(1.0));
    l.ui.p().rounded_border(r, RADIUS + 4.0, 1.0 + f, edge.alpha(edge.0[3].max(0.12) * fade));
    clicked
}

/// The sky of a card: the look's own colours, light at the horizon.
fn sky(l: &mut Launcher, r: Rect, top: Color, bottom: Color) {
    l.ui.p().gradient(Rect::new(r.x, r.y, r.w, r.h * 0.62), top, bottom);
    l.ui.p().rect(Rect::new(r.x, r.y + r.h * 0.62, r.w, r.h * 0.38), bottom.mix(PANEL(), 0.5));
}

/// A row of houses and towers along `base`, their heights from a seed.
fn skyline(l: &mut Launcher, r: Rect, base: f32, max_h: f32, c: Color, seed: u32) {
    let mut x = r.x - 6.0;
    let mut s = seed.wrapping_mul(2654435761);
    while x < r.right() {
        s = s.wrapping_mul(1103515245).wrapping_add(12345);
        let w = 10.0 + (s >> 16 & 15) as f32 * 1.4;
        let h = max_h * (0.3 + (s >> 8 & 255) as f32 / 255.0 * 0.7);
        l.ui.p().rect(Rect::new(x, base - h, w - 2.0, h), c);
        if s & 7 == 0 {
            // a tower with a spire
            l.ui.p().convex(&[Vec2::new(x + 2.0, base - h), Vec2::new(x + w * 0.5 - 1.0, base - h - max_h * 0.35), Vec2::new(x + w - 4.0, base - h)], c);
        }
        x += w;
    }
}

/// A bus seen from the side, `len` long, its front to the right.
fn bus(l: &mut Launcher, at: Vec2, len: f32, body: Color, glass: Color) {
    let h = len * 0.28;
    let b = Rect::new(at.x, at.y - h, len, h);
    l.ui.p().rounded(b, h * 0.14, body);
    let wy = b.y + h * 0.16;
    let wh = h * 0.36;
    let mut x = b.x + len * 0.05;
    while x < b.right() - len * 0.12 {
        l.ui.p().rounded(Rect::new(x, wy, len * 0.11, wh), 1.5, glass);
        x += len * 0.125;
    }
    l.ui.p().rounded(Rect::new(b.right() - len * 0.075, wy, len * 0.06, h * 0.62), 1.5, glass);
    for wx in [0.2, 0.78] {
        let c = Vec2::new(b.x + len * wx, b.bottom());
        l.ui.p().circle(c, h * 0.17, Color::rgba(14, 16, 22, 1.0));
        l.ui.p().circle(c, h * 0.08, Color::rgba(120, 124, 132, 1.0));
    }
}

/// The picture of a card, drawn in the look's colours (no photographs: shapes).
fn art(l: &mut Launcher, r: Rect, art: Art, fade: f32, hover: f32) {
    let a = |c: Color| c.alpha(c.0[3] * fade);
    let pic = Rect::new(r.x, r.y, r.w, r.h * 0.72);
    let ground = pic.y + pic.h * 0.66;
    match art {
        Art::City => {
            sky(l, pic, a(ACCENT().mix(BACKDROP(), 0.55)), a(ACCENT().mix(TEXT(), 0.25).mix(PANEL(), 0.35)));
            skyline(l, pic, ground, pic.h * 0.5, a(PANEL().mix(ACCENT(), 0.25)), 3);
            skyline(l, pic, ground, pic.h * 0.32, a(PANEL().mix(ACCENT(), 0.12)), 11);
            // the route in the sky with its stops, as on the line map
            let pts = [(0.0, 0.62), (0.24, 0.46), (0.5, 0.52), (0.78, 0.28), (1.0, 0.22)];
            let p = |q: (f32, f32)| Vec2::new(pic.x + pic.w * q.0, pic.y + pic.h * q.1);
            for s in pts.windows(2) {
                l.ui.p().line(p(s[0]), p(s[1]), 3.0, a(ACCENT_2().alpha(0.85)));
            }
            for q in &pts[1..4] {
                l.ui.p().circle(p(*q), 5.0, a(ACCENT_2()));
                l.ui.p().circle(p(*q), 2.4, a(PANEL()));
            }
            bus(l, Vec2::new(pic.x + pic.w * (0.08 + 0.04 * hover), ground + 2.0), pic.w * 0.5, a(TEXT().mix(ACCENT(), 0.1)), a(PANEL().mix(ACCENT(), 0.3)));
        }
        Art::Country => {
            sky(l, pic, a(OK().mix(BACKDROP(), 0.6)), a(OK().mix(TEXT(), 0.35).mix(PANEL(), 0.3)));
            // hills, a road winding to the horizon
            let hill = |l: &mut Launcher, y: f32, amp: f32, c: Color, ph: f32| {
                let n = 24;
                for i in 0..n {
                    let x0 = pic.x + pic.w * i as f32 / n as f32;
                    let x1 = pic.x + pic.w * (i + 1) as f32 / n as f32;
                    let y0 = y - amp * ((i as f32 * 0.5 + ph).sin() * 0.5 + 0.5);
                    let y1 = y - amp * (((i + 1) as f32 * 0.5 + ph).sin() * 0.5 + 0.5);
                    l.ui.p().convex(&[Vec2::new(x0, y0), Vec2::new(x1, y1), Vec2::new(x1, pic.bottom()), Vec2::new(x0, pic.bottom())], c);
                }
            };
            hill(l, ground - pic.h * 0.12, pic.h * 0.16, a(OK().mix(PANEL(), 0.62)), 0.0);
            hill(l, ground, pic.h * 0.10, a(OK().mix(PANEL(), 0.45)), 2.0);
            let road = [Vec2::new(pic.x + pic.w * 0.62, ground - pic.h * 0.2), Vec2::new(pic.x + pic.w * 0.66, ground - pic.h * 0.2), Vec2::new(pic.x + pic.w * 0.95, pic.bottom()), Vec2::new(pic.x + pic.w * 0.35, pic.bottom())];
            l.ui.p().convex(&road, a(ROAD()));
            for k in 0..4 {
                let t0 = k as f32 / 4.0 + 0.05;
                let lerp = |t: f32| road[0].lerp(road[3], t).lerp(road[1].lerp(road[2], t), 0.5);
                l.ui.p().line(lerp(t0), lerp(t0 + 0.1), 1.5, a(TEXT().alpha(0.6)));
            }
        }
        Art::Crowd => {
            sky(l, pic, a(ACCENT_2().mix(BACKDROP(), 0.6)), a(ACCENT_2().mix(TEXT(), 0.2).mix(PANEL(), 0.35)));
            skyline(l, pic, ground, pic.h * 0.38, a(PANEL().mix(ACCENT_2(), 0.18)), 7);
            // three buses on three lines, linked
            let ys = [0.30, 0.48, 0.66];
            for (k, y) in ys.iter().enumerate() {
                let x = pic.x + pic.w * (0.12 + k as f32 * 0.24);
                bus(l, Vec2::new(x, pic.y + pic.h * y + 8.0), pic.w * 0.3, a([ACCENT(), ACCENT_2(), OK()][k].mix(TEXT(), 0.2)), a(PANEL().alpha(0.8)));
            }
        }
        Art::Board => {
            sky(l, pic, a(DANGER().mix(BACKDROP(), 0.62)), a(DANGER().mix(TEXT(), 0.15).mix(PANEL(), 0.4)));
            // a departure board
            let b = Rect::new(pic.x + pic.w * 0.12, pic.y + pic.h * 0.14, pic.w * 0.76, pic.h * 0.58);
            l.ui.p().rounded(b, 6.0, a(RAIL()));
            l.ui.p().rounded_border(b, 6.0, 1.0, a(EDGE()));
            for i in 0..4 {
                let y = b.y + 10.0 + i as f32 * (b.h - 20.0) / 4.0;
                l.ui.p().rounded(Rect::new(b.x + 10.0, y, 22.0, 9.0), 2.0, a(ACCENT_2()));
                l.ui.p().rect(Rect::new(b.x + 40.0, y + 2.0, b.w * (0.35 + 0.08 * (i % 2) as f32), 5.0), a(TEXT_SOFT().alpha(0.6)));
                l.ui.p().rect(Rect::new(b.right() - 40.0, y + 2.0, 28.0, 5.0), a(ACCENT_2().alpha(0.75)));
            }
        }
        Art::Fleet => {
            sky(l, pic, a(ACCENT().mix(BACKDROP(), 0.7)), a(TEXT_DIM().mix(PANEL(), 0.3)));
            // a hall with buses in a row
            l.ui.p().rect(Rect::new(pic.x, ground - 2.0, pic.w, 2.0), a(TEXT_FAINT()));
            for k in 0..3 {
                let len = pic.w * 0.42;
                let x = pic.x + pic.w * (-0.08 + k as f32 * 0.36);
                bus(l, Vec2::new(x, ground - 2.0 - k as f32 * 0.0), len, a([TEXT(), ACCENT_2(), DANGER()][k].mix(PANEL(), 0.15)), a(PANEL().mix(ACCENT(), 0.25)));
            }
            for k in 0..6 {
                let x = pic.x + pic.w * k as f32 / 5.0;
                l.ui.p().line(Vec2::new(x, pic.y), Vec2::new(pic.x + pic.w * 0.5 + (x - pic.x - pic.w * 0.5) * 1.6, ground - pic.h * 0.5), 1.0, a(LIFT().alpha(0.06)));
            }
        }
        Art::Plan => {
            l.ui.p().rect(pic, a(RAIL()));
            // a street grid and a line drawn over it
            let step = 22.0;
            let mut x = pic.x + 8.0;
            while x < pic.right() {
                l.ui.p().rect(Rect::new(x, pic.y, 1.0, pic.h), a(ROAD().alpha(0.7)));
                x += step;
            }
            let mut y = pic.y + 6.0;
            while y < pic.bottom() {
                l.ui.p().rect(Rect::new(pic.x, y, pic.w, 1.0), a(ROAD().alpha(0.7)));
                y += step;
            }
            let pts = [(0.06, 0.82), (0.06, 0.5), (0.38, 0.5), (0.38, 0.22), (0.72, 0.22), (0.72, 0.62), (0.96, 0.62)];
            let p = |q: (f32, f32)| Vec2::new(pic.x + pic.w * q.0, pic.y + pic.h * q.1);
            for s in pts.windows(2) {
                l.ui.p().line(p(s[0]), p(s[1]), 4.0, a(ACCENT()));
            }
            for q in [pts[0], pts[2], pts[4], pts[6]] {
                l.ui.p().circle(p(q), 6.0, a(TEXT()));
                l.ui.p().circle(p(q), 3.5, a(ACCENT()));
            }
        }
        Art::Paint => {
            sky(l, pic, a(TEXT_DIM().mix(BACKDROP(), 0.5)), a(TEXT_SOFT().mix(PANEL(), 0.25)));
            // a bus half painted, the brush's stroke still on it
            let len = pic.w * 0.82;
            let at = Vec2::new(pic.x + pic.w * 0.09, ground + 4.0);
            bus(l, at, len, a(TEXT().mix(PANEL(), 0.1)), a(PANEL().mix(ACCENT(), 0.25)));
            let h = len * 0.28;
            let split = at.x + len * (0.45 + 0.25 * hover);
            l.ui.p().rect(Rect::new(at.x + 3.0, at.y - h * 0.42, split - at.x - 3.0, h * 0.30), a(ACCENT()));
            l.ui.p().rect(Rect::new(at.x + 3.0, at.y - h * 0.12, split - at.x - 3.0, h * 0.06), a(ACCENT_2()));
            l.ui.p().circle(Vec2::new(split, at.y - h * 0.27), 6.0, a(ACCENT()));
        }
    }
}

/// The chips under the cards: the driver (level and hours with them), then the other pages.
fn chips(l: &mut Launcher, r: Rect, since: f32) {
    let p = l.state.profile.clone().filter(|p| p.exists);
    let driver = match &p {
        Some(p) => format!("{}: {}  ·  {} {}  ·  {}", omsi_ui::tr("Driver"), p.name, omsi_ui::tr("Level"), p.level, hours(p.hours)),
        None => omsi_ui::tr("Create a driver").to_string(),
    };
    let mut items: Vec<(Page, &str, String)> = vec![(Page::Profile, "account_circle", driver)];
    for (page, icon, name) in CHIPS {
        items.push((page, icon, omsi_ui::tr(name).to_string()));
    }
    let widths: Vec<f32> = items.iter().map(|(_, _, t)| l.ui.width(t, 13.0, Weight::Medium) + 54.0).collect();
    let gap = 10.0;
    let total: f32 = widths.iter().sum::<f32>() + gap * (items.len() - 1) as f32;
    // (a narrow window: as many as fit, the driver always)
    let mut x = r.x + ((r.w - total) * 0.5).max(0.0);
    for (k, ((page, icon, text), w)) in items.iter().zip(widths).enumerate() {
        if x + w > r.right() && k > 0 {
            break;
        }
        // (one after the other, rising into place)
        let a = appear(since, 0.6 + k as f32 * 0.06, 0.45);
        let c = Rect::new(x, r.y + 16.0 * (1.0 - a), w, r.h);
        x += w + gap;
        if a <= 0.01 {
            continue;
        }
        let id = id_of(&format!("home-chip-{k}"));
        let (h, _, clicked) = l.ui.interact(id, c);
        let t = l.ui.anim(id, if h { 1.0 } else { 0.0 }, 0.07);
        let c = Rect::new(c.x, c.y - 2.0 * t, c.w, c.h);
        l.ui.solid(c);
        l.ui.p().rounded(c, RADIUS, PANEL().mix(HOVER(), t).alpha(a));
        l.ui.p().rounded_border(c, RADIUS, 1.0, EDGE().mix(ACCENT(), t * 0.8).alpha(a));
        l.ui.icon(icon, Vec2::new(c.x + 22.0, c.center().y), 18.0, if k == 0 { ACCENT_2().alpha(a) } else { TEXT_SOFT().mix(TEXT(), t).alpha(a) });
        l.ui.text_in(text, Rect::new(c.x + 40.0, c.y, c.w - 48.0, c.h), 13.0, Weight::Medium, TEXT().alpha(a), Align::Left);
        if clicked {
            l.go(*page);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hour_has_a_greeting() {
        assert_eq!(greeting(7), "Good morning");
        assert_eq!(greeting(13), "Good afternoon");
        assert_eq!(greeting(21), "Good evening");
        assert_eq!(greeting(2), "Good night");
        assert_eq!(hours(1.5), "1h 30m");
    }

    #[test]
    fn the_focused_card_stands_in_the_middle() {
        let (w, g) = (200.0, 20.0);
        for at in [0.0, 1.0, 3.0] {
            let x = row_x(700.0, w, g, at) + at * (w + g);
            assert!((x + w * 0.5 - 700.0).abs() < 1e-3);
        }
    }

    #[test]
    fn every_card_leads_somewhere_of_its_own() {
        for (i, a) in CARDS.iter().enumerate() {
            for b in &CARDS[i + 1..] {
                assert_ne!(a.mode, b.mode);
            }
        }
    }
}
