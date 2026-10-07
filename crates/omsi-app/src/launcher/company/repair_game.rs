//! The workshop task (the rules are `training::Job`'s): the bus from the side with its eight
//! parts marked on it; the player picks a part, checks it - which takes a moment - reads what
//! the check finds and deals with it, and finishes before the time runs out. At the end each
//! part says what was wrong and what was done, and the job is booked with what it saved.
//! Calm: nothing moves but the clock, the check's progress and what is under the mouse.

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit;
use super::{act, eur, section};
use glam::Vec2;
use omsi_launcher_lib::company::training::{self, Fix, Job, JobDone, JobKind, Part};
use omsi_launcher_lib::company::Company;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// Seconds a check takes.
const CHECK: f32 = 1.2;

pub struct Game {
    job: Job,
    bus: String,
    colours: [Color; 2],
    /// Seconds left.
    left: f32,
    selected: Option<Part>,
    /// The part being checked and how far (s).
    checking: Option<(Part, f32)>,
    /// The job done (the quality and what it saved), or why it could not be booked.
    result: Option<Result<JobDone, String>>,
}

/// Where each part sits on the bus seen from the side (shares of its width and height; the
/// front is on the right).
fn spot(p: Part) -> (f32, f32) {
    match p {
        Part::Lights => (0.965, 0.72),
        Part::Wipers => (0.93, 0.30),
        Part::Doors => (0.51, 0.50),
        Part::Engine => (0.06, 0.62),
        Part::Brakes => (0.80, 0.90),
        Part::Tyres => (0.21, 0.90),
        Part::Suspension => (0.30, 0.70),
        Part::Battery => (0.66, 0.80),
    }
}

impl Game {
    /// The task for a bus of the company (None: it is not in the fleet).
    pub fn new(c: &Company, vehicle: u32, kind: JobKind) -> Option<Game> {
        let v = c.vehicle(vehicle)?;
        Some(Game {
            job: Job::new(c, v, kind),
            bus: format!("{} {}", v.number, v.name),
            colours: [super::super::ownlines::colour_of(&c.colours[0]), super::super::ownlines::colour_of(&c.colours[1])],
            left: kind.seconds(),
            selected: Some(Part::Lights),
            checking: None,
            result: None,
        })
    }
}

/// The bus from the side into `r`, its parts as round marks; returns the part clicked.
fn draw_bus(ui: &mut Ui, r: Rect, g: &Game) -> Option<Part> {
    let body = Rect::new(r.x, r.y, r.w, r.h * 0.84);
    let main = g.colours[0];
    ui.p().rounded(body, r.h * 0.07, main);
    // the stripe of the second colour, the windows, the windscreen, the doors
    ui.p().rect(Rect::new(body.x + 6.0, body.y + body.h * 0.66, body.w - 12.0, body.h * 0.08), g.colours[1]);
    let glass = Color::rgba(28, 36, 52, 1.0);
    // (the doors, and the windows between them)
    let doors = [0.47, 0.79];
    let (dw, wy, wh) = (0.075, body.y + body.h * 0.14, body.h * 0.34);
    let mut x = 0.05;
    while x < 0.86 {
        let w = 0.08;
        if let Some(d) = doors.iter().find(|d| x + w > **d - 0.008 && x < **d + dw + 0.008) {
            x = d + dw + 0.014;
            continue;
        }
        if x + w <= 0.875 {
            ui.p().rounded(Rect::new(body.x + body.w * x, wy, body.w * w, wh), 4.0, glass);
        }
        x += w + 0.012;
    }
    ui.p().rounded(Rect::new(body.x + body.w * 0.885, body.y + body.h * 0.08, body.w * 0.1, body.h * 0.52), 6.0, glass);
    for dx in doors {
        let d = Rect::new(body.x + body.w * dx, body.y + body.h * 0.14, body.w * dw, body.h * 0.8);
        ui.p().rounded(d, 3.0, glass);
        ui.p().rect(Rect::new(d.center().x - 0.75, d.y, 1.5, d.h), main.alpha(0.6));
    }
    // the wheels
    for wx in [0.21, 0.80] {
        let c = Vec2::new(r.x + r.w * wx, r.y + r.h * 0.86);
        ui.p().circle(c, r.h * 0.14, Color::rgba(14, 16, 22, 1.0));
        ui.p().circle(c, r.h * 0.07, Color::rgba(110, 116, 128, 1.0));
    }
    // the marks
    let mut clicked = None;
    for ch in &g.job.checks {
        let (fx, fy) = spot(ch.part);
        let c = Vec2::new(r.x + r.w * fx, r.y + r.h * fy);
        let rad = 17.0;
        let hit = Rect::new(c.x - rad, c.y - rad, 2.0 * rad, 2.0 * rad);
        let (h, _, click) = ui.interact(super::super::ui::id_of(&format!("workshop-part-{:?}", ch.part)), hit);
        if click {
            clicked = Some(ch.part);
        }
        let selected = g.selected == Some(ch.part);
        let ring = if ch.done.is_some() { OK } else if ch.inspected { accent_2() } else { Color::WHITE };
        if selected {
            ui.p().circle(c, rad + 6.0, ring.alpha(0.25));
        }
        ui.p().circle(c, rad + if h { 2.0 } else { 0.0 }, Color::rgba(12, 16, 26, 0.92));
        ui.p().circle(c, rad - 2.5, ring.alpha(if ch.done.is_some() { 0.35 } else { 0.12 }));
        ui.icon(ch.part.icon(), c, 17.0, if ch.done.is_some() { OK } else { ring });
    }
    clicked
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(mut g) = l.company.career.game.take() else { return };
    let keep = if g.result.is_some() { results(l, area, &g) } else { play(l, area, &mut g) };
    if keep {
        l.company.career.game = Some(g);
    }
}

/// The task under way; returns whether it goes on.
fn play(l: &mut Launcher, area: Rect, g: &mut Game) -> bool {
    let dt = l.ui.dt.min(0.25);
    g.left = (g.left - dt).max(0.0);
    l.ui.keep_moving();
    // the head: the bus, what the job is, the time
    let kind = if g.job.kind == JobKind::Repair { "Repair job" } else { "Service job" };
    l.ui.text_in(&format!("{}  ·  {}", omsi_ui::tr(kind), g.bus), Rect::new(area.x, area.y, area.w * 0.5, 26.0), 18.0, Weight::Bold, TEXT, Align::Left);
    let done = g.job.checks.iter().filter(|c| c.done.is_some()).count();
    let t = omsi_ui::tr("%{n} of %{all} parts dealt with").replace("%{n}", &done.to_string()).replace("%{all}", &g.job.checks.len().to_string());
    l.ui.text_in(&t, Rect::new(area.x, area.y + 28.0, area.w * 0.5, 18.0), 14.0, Weight::Regular, TEXT_DIM, Align::Left);
    let total = g.job.kind.seconds();
    let clock = format!("{}:{:02}", (g.left as i32) / 60, (g.left as i32) % 60);
    let low = g.left < 15.0;
    let cw = 300.0f32.min(area.w * 0.3);
    let cx = area.right() - cw - 280.0;
    l.ui.text_in(&clock, Rect::new(cx, area.y, cw, 26.0), 20.0, Weight::Bold, if low { DANGER.lighten(0.2) } else { TEXT }, Align::Right);
    super::meter(&mut l.ui, Rect::new(cx, area.y + 34.0, cw, 6.0), (g.left / total) as f64, if low { DANGER } else { accent_2() });
    let mut finish = g.left <= 0.0;
    if l.ui.button("workshop-finish", Rect::new(area.right() - 264.0, area.y + 2.0, 150.0, 38.0), "Finish the job", Some("check_circle"), ButtonKind::Primary) {
        finish = true;
    }
    if l.ui.button("workshop-leave", Rect::new(area.right() - 104.0, area.y + 2.0, 104.0, 38.0), "Stop the job", None, ButtonKind::Ghost) {
        return false;
    }
    // the bus and its parts
    let top = area.y + 64.0;
    let side_w = 360.0f32.min(area.w * 0.36);
    let stage = Rect::new(area.x, top, area.w - side_w - 16.0, area.bottom() - top);
    l.ui.card(stage);
    let bw = (stage.w - 80.0).min((stage.h - 120.0) * 3.6);
    let bh = bw / 3.6;
    let bus = Rect::new(stage.x + (stage.w - bw) * 0.5, stage.y + (stage.h - bh) * 0.5 - 10.0, bw, bh);
    if let Some(p) = draw_bus(&mut l.ui, bus, g) {
        g.selected = Some(p);
    }
    l.ui.text_in("Pick a part on the bus, check it, and deal with what the check finds.", Rect::new(stage.x + 16.0, stage.bottom() - 34.0, stage.w - 32.0, 20.0), 14.0, Weight::Regular, TEXT_DIM, Align::Center);
    // the part picked
    let side = Rect::new(stage.right() + 16.0, top, side_w, stage.h);
    let Some(part) = g.selected else { return true };
    l.ui.card(side);
    let inner = Rect::new(side.x + 16.0, side.y + 14.0, side.w - 32.0, side.h - 28.0);
    let Some(ch) = g.job.checks.iter().find(|c| c.part == part).cloned() else { return true };
    let mut y = inner.y;
    l.ui.icon(part.icon(), Vec2::new(inner.x + 16.0, y + 18.0), 26.0, accent_2());
    l.ui.text_in(&omsi_ui::tr(part.label()), Rect::new(inner.x + 44.0, y + 4.0, inner.w - 44.0, 28.0), 20.0, Weight::Bold, TEXT, Align::Left);
    y += 52.0;
    match g.checking {
        Some((p, t)) if p == part => {
            let t = t + dt;
            l.ui.text_in("Checking…", Rect::new(inner.x, y, inner.w, 20.0), kit::ROWS, Weight::Medium, TEXT_SOFT, Align::Left);
            l.ui.progress(Rect::new(inner.x, y + 28.0, inner.w, 6.0), (t / CHECK).min(1.0), false);
            if t >= CHECK {
                g.job.inspect(part);
                g.checking = None;
            } else {
                g.checking = Some((p, t));
            }
            return finish_now(l, g, finish);
        }
        _ => {}
    }
    if !ch.inspected {
        l.ui.paragraph("Not checked yet.", Vec2::new(inner.x, y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        if l.ui.button("workshop-check", Rect::new(inner.x, y + 34.0, inner.w, 40.0), "Check it", Some("search"), ButtonKind::Primary) {
            g.checking = Some((part, 0.0));
        }
        return finish_now(l, g, finish);
    }
    // what the check found, and the four things to do
    l.ui.text_in(&omsi_ui::tr("The check finds").to_uppercase(), Rect::new(inner.x, y, inner.w, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
    let found = omsi_ui::tr(part.reading(ch.fault)).into_owned();
    let h = l.ui.paragraph(&found, Vec2::new(inner.x, y + 20.0), inner.w, 16.0, Weight::Medium, TEXT);
    y += 20.0 + h + 18.0;
    let bw = (inner.w - 10.0) * 0.5;
    for (k, fix) in Fix::ALL.iter().enumerate() {
        let b = Rect::new(inner.x + (k % 2) as f32 * (bw + 10.0), y + (k / 2) as f32 * 48.0, bw, 40.0);
        let chosen = ch.done == Some(*fix);
        if l.ui.button(&format!("workshop-fix-{k}"), b, fix.label(), if chosen { Some("check") } else { None }, if chosen { ButtonKind::Primary } else { ButtonKind::Normal }) {
            g.job.apply(part, *fix);
            // on to the next part not dealt with
            g.selected = g.job.checks.iter().map(|c| c.part).skip_while(|p| *p != part).skip(1).chain(g.job.checks.iter().map(|c| c.part)).find(|p| g.job.checks.iter().any(|c| c.part == *p && c.done.is_none())).or(Some(part));
        }
    }
    finish_now(l, g, finish)
}

/// Finish when asked (or at the bell): the job is booked. Always goes on (to the results).
fn finish_now(l: &mut Launcher, g: &mut Game, finish: bool) -> bool {
    if finish {
        let q = g.job.quality(g.left);
        let (id, kind) = (g.job.vehicle, g.job.kind);
        let mut err = None;
        let done = act(l, |c| training::finish_job(c, id, kind, q).inspect_err(|e| err = Some(*e)));
        g.result = Some(done.ok_or_else(|| omsi_ui::tr(err.unwrap_or("The job could not be booked.")).into_owned()));
    }
    true
}

/// The job done: how well, what each part was and what was done, what it saved.
fn results(l: &mut Launcher, area: Rect, g: &Game) -> bool {
    let mut keep = true;
    let r = Rect::new(area.x + (area.w - 760.0f32.min(area.w)) * 0.5, area.y, 760.0f32.min(area.w), area.h);
    let inner = section(&mut l.ui, r, if g.job.kind == JobKind::Repair { "Repair done" } else { "Service done" });
    let (right, wrong, missed, wasted) = g.job.tally();
    match g.result.as_ref() {
        Some(Ok(j)) => {
            let pct = (j.quality * 100.0).round() as i64;
            let ink = if pct >= 75 { OK } else if pct >= 40 { WARN } else { DANGER.lighten(0.2) };
            l.ui.text_in(&format!("{pct} %"), Rect::new(inner.x, inner.y, 160.0, 44.0), 36.0, Weight::Bold, ink, Align::Left);
            l.ui.text_in(&g.bus, Rect::new(inner.x + 170.0, inner.y + 2.0, inner.w - 170.0, 20.0), 16.0, Weight::Bold, TEXT, Align::Left);
            let saved = if j.saved >= 0 { omsi_ui::tr("Saved: %{amount}").replace("%{amount}", &eur(j.saved)) } else { omsi_ui::tr("The bus goes to the workshop after all; the parts cost %{amount}.").replace("%{amount}", &eur(-j.saved)) };
            l.ui.text_in(&saved, Rect::new(inner.x + 170.0, inner.y + 24.0, inner.w - 170.0, 20.0), kit::ROWS, Weight::Medium, if j.saved >= 0 { OK } else { WARN }, Align::Left);
        }
        Some(Err(e)) => {
            l.ui.text_in(e, Rect::new(inner.x, inner.y, inner.w, 22.0), 15.5, Weight::Medium, DANGER.lighten(0.2), Align::Left);
        }
        None => {}
    }
    let t = omsi_ui::tr("%{right} faults fixed right, %{wrong} the wrong way, %{missed} missed; %{wasted} good parts worked on for nothing").replace("%{right}", &right.to_string()).replace("%{wrong}", &wrong.to_string()).replace("%{missed}", &missed.to_string()).replace("%{wasted}", &wasted.to_string());
    l.ui.text_in(&t, Rect::new(inner.x, inner.y + 58.0, inner.w, 18.0), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
    // each part
    let mut y = inner.y + 90.0;
    for ch in &g.job.checks {
        if y + 30.0 > inner.bottom() - 46.0 {
            break;
        }
        let good = ch.fault.unwrap_or(Fix::Leave) == ch.done.unwrap_or(Fix::Leave) && (ch.done.is_some() || ch.fault.is_none());
        l.ui.icon(if good { "check_circle" } else { "error" }, Vec2::new(inner.x + 10.0, y + 14.0), 16.0, if good { OK } else { WARN });
        l.ui.text_in(&omsi_ui::tr(ch.part.label()), Rect::new(inner.x + 28.0, y, 120.0, 28.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
        l.ui.text_in(&omsi_ui::tr(ch.part.reading(ch.fault)), Rect::new(inner.x + 150.0, y, inner.w * 0.5, 28.0), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
        let did = match ch.done {
            Some(f) => omsi_ui::tr(f.label()).into_owned(),
            None => omsi_ui::tr("Not dealt with").into_owned(),
        };
        let want = ch.fault.map(|f| omsi_ui::tr(f.label()).into_owned());
        let text = match (&want, good) {
            (Some(w), false) => format!("{did}  →  {w}"),
            _ => did,
        };
        l.ui.text_in(&text, Rect::new(inner.right() - 220.0, y, 220.0, 28.0), 14.0, Weight::Medium, if good { TEXT_DIM } else { WARN }, Align::Right);
        l.ui.p().rect(Rect::new(inner.x, y + 29.0, inner.w, 1.0), HAIRLINE);
        y += 32.0;
    }
    if l.ui.button("workshop-done", Rect::new(inner.right() - 220.0, inner.bottom() - 38.0, 220.0, 38.0), "Back to the workshop", None, ButtonKind::Primary) {
        keep = false;
    }
    keep
}
