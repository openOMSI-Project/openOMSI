//! The company's clock on its pages: the company's now in the sheet's head with the steps to
//! simulate (+1 h, "Simulate to tomorrow", the others in the time dialog), the play mode that
//! runs the clock while the page is open (×60, ×600), the decisions a simulation waits for,
//! and the feed of what happened (the rules are `omsi_launcher_lib::company::clock`'s).
//!
//! A step within the company's day runs at once, on the page's own copy of the day's
//! timetable; a step past midnight closes days and runs on a thread of its own, as "Close the
//! day" did. While the game runs on the company's map its clock leads: the steps are not
//! offered, the company's day goes along with the game's time and what the game reports of
//! the company's buses is told as it comes.

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{changed, data, day_label, eur, spawn, Dialog, Msg};
use glam::Vec2;
use omsi_launcher_lib::company::clock::{self as ck, Ask, Choice, FeedItem, Level, Run, Step, DAY};
use omsi_launcher_lib::company::store::Disk;
use omsi_launcher_lib::company::{self as co, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::time::Instant;

pub struct ClockView {
    /// Playing: company minutes per second of ours (0: the clock stands).
    pub(super) speed: f64,
    acc: f64,
    last: Option<Instant>,
    /// Changes of the clock not saved yet (a playing clock is saved now and then).
    dirty: Option<Instant>,
    /// The game: when the clock last followed it, and the live events told (of which
    /// company and day).
    follow_at: Option<Instant>,
    live_seen: usize,
    live_for: (String, String),
    /// The time dialog's "until": a minute of the day.
    pub(super) until: f32,
}

impl Default for ClockView {
    fn default() -> Self {
        ClockView { speed: 0.0, acc: 0.0, last: None, dirty: None, follow_at: None, live_seen: 0, live_for: Default::default(), until: 16.0 * 60.0 }
    }
}

/// The game runs on the company's map: its clock leads (the minute of its day).
fn game_minute(l: &Launcher, c: &Company) -> Option<i64> {
    let (secs, in_game) = super::map::time_of_day(l, c);
    in_game.then_some((secs / 60.0).floor() as i64)
}

/// A step may be taken now (no other running, no game leading).
fn may_step(l: &Launcher) -> bool {
    !l.company.closing && !l.state.in_game() && l.company.company.as_ref().is_some_and(|c| c.clock.ask.is_none())
}

/// Simulate to `to` (the clock's minutes): within the company's day at once, past its midnight
/// on a thread. `quick`: nothing waits, the dispatcher decides.
pub(super) fn simulate(l: &mut Launcher, to: i64, quick: bool) {
    let Some(c) = l.company.company.clone() else { return };
    if l.company.closing || l.state.in_game() {
        return;
    }
    let midnight = (ck::now(&c).div_euclid(DAY) + 1) * DAY;
    let lines = l.company.today.as_ref().filter(|t| t.map == c.map && t.date == c.date && t.error.is_none()).map(|t| t.lines.clone());
    if to < midnight {
        if let Some(lines) = lines {
            let mut c = c;
            let d = data();
            let mut w = Disk::new(&d).knowing(&c.map.clone(), &c.date.clone(), lines);
            match ck::advance(&mut c, to, &mut w, quick) {
                Ok(run) => adopt(l, c, run),
                Err(e) => {
                    l.company.clock.speed = 0.0;
                    l.state.set_status(e, true);
                }
            }
            return;
        }
    }
    l.company.closing = true;
    spawn(&l.company.tx, move || {
        let d = data();
        let mut c = c;
        let w = lines.map(|x| Disk::new(&d).knowing(&c.map.clone(), &c.date.clone(), x));
        match co::store::simulate(&d, &mut c, to, quick, w) {
            Ok(run) => Msg::Simulated(Ok((c, run))),
            Err(e) => Msg::Simulated(Err(format!("{e:#}"))),
        }
    });
}

/// The company after a simulation: its days' reports shown, the decision it waits for asked,
/// saved (a playing clock's quiet minutes now and then).
pub(super) fn adopt(l: &mut Launcher, c: Company, run: Run) {
    let quiet = run.reports.is_empty() && !run.stopped && c.clock.feed.iter().rev().take(run.told).all(|f| f.level == Level::Minor);
    l.company.company = Some(c);
    if !run.reports.is_empty() {
        l.company.reports = Some(run.reports);
    }
    if quiet && l.company.clock.speed > 0.0 {
        let view = &mut l.company;
        if let (Some(c), Some(list)) = (view.company.as_ref(), view.companies.as_mut()) {
            if let Some(x) = list.iter_mut().find(|x| x.id == c.id) {
                *x = c.clone();
            }
        }
        if view.clock.dirty.is_none() {
            view.clock.dirty = Some(Instant::now());
        }
        return;
    }
    l.company.clock.dirty = None;
    changed(l);
}

/// Save what a playing clock left unsaved.
fn flush(l: &mut Launcher) {
    if l.company.clock.dirty.take().is_some() {
        changed(l);
    }
}

/// Every frame on the company's pages: the playing clock, and the game's clock leading.
pub fn tick(l: &mut Launcher, modal: bool) {
    let now = Instant::now();
    let dt = l.company.clock.last.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0).min(0.5);
    l.company.clock.last = Some(now);
    let Some(c) = l.company.company.clone() else { return };
    // the game leads
    if l.state.in_game() {
        l.company.clock.speed = 0.0;
        if l.company.clock.follow_at.is_none_or(|t| t.elapsed().as_secs_f32() >= 2.0) {
            l.company.clock.follow_at = Some(now);
            follow(l, &c);
        }
        return;
    }
    if l.company.clock.dirty.is_some_and(|t| t.elapsed().as_secs_f32() >= 8.0) || (l.company.clock.speed == 0.0 && l.company.clock.dirty.is_some()) {
        flush(l);
    }
    if l.company.clock.speed <= 0.0 || modal || l.company.closing {
        return;
    }
    if c.clock.ask.is_some() {
        l.company.clock.speed = 0.0;
        return;
    }
    l.ui.keep_moving();
    let view = &mut l.company.clock;
    view.acc += dt * view.speed;
    if view.acc < 1.0 {
        return;
    }
    let steps = view.acc.floor() as i64;
    view.acc -= steps as f64;
    simulate(l, ck::now(&c) + steps, false);
}

/// The game's minute taken over, and what it reported told.
fn follow(l: &mut Launcher, c: &Company) {
    let Some(minute) = game_minute(l, c) else { return };
    let mut c = c.clone();
    let key = (c.id.clone(), c.date.clone());
    if l.company.clock.live_for != key {
        l.company.clock.live_for = key;
        l.company.clock.live_seen = 0;
    }
    let d = data();
    let live = co::store::peek_live(&d, &c.id, l.company.clock.live_seen);
    let mut told = 0;
    if !live.is_empty() {
        l.company.clock.live_seen += live.len();
        told += ck::tell_live(&mut c, &live);
    }
    let lines = l.company.today.as_ref().filter(|t| t.map == c.map && t.date == c.date && t.error.is_none()).map(|t| t.lines.clone());
    if let Some(lines) = lines {
        let mut w = Disk::new(&d).knowing(&c.map.clone(), &c.date.clone(), lines);
        match ck::follow(&mut c, minute, &mut w) {
            Ok(run) => told += run.told,
            Err(e) => log::warn!("company: following the game's clock: {e}"),
        }
    }
    if told > 0 || ck::now(&c) != ck::now(l.company.company.as_ref().unwrap_or(&c)) {
        l.company.company = Some(c);
        changed(l);
    }
}

/// The company's now as the head shows it.
pub(super) fn now_label(c: &Company) -> String {
    format!("{}  ·  {}", day_label(&c.date), ck::hhmm(ck::now(c)))
}

/// The head: the company's day and its time, the play buttons, +1 h, "Simulate to tomorrow"
/// and the time dialog. Returns where it begins.
pub fn head(l: &mut Launcher, r: Rect) -> f32 {
    let Some(c) = l.company.company.clone() else { return r.right() };
    let in_game = l.state.in_game();
    let ok = may_step(l);
    let gap = 8.0;
    let menu = Rect::new(r.right() - 42.0, r.y, 42.0, r.h);
    let tomorrow_w = Foot::width(&l.ui, "Simulate to tomorrow", Some("nights_stay"));
    let tomorrow = Rect::new(menu.x - gap - tomorrow_w, r.y, tomorrow_w, r.h);
    let hour = Rect::new(tomorrow.x - gap - 70.0, r.y, 70.0, r.h);
    let mut x = hour.x;
    if l.ui.button("company-time", menu, "", Some("schedule"), ButtonKind::Normal) && !l.company.closing {
        l.company.dialog = Some(Dialog::Time);
    }
    l.ui.tooltip(menu, "More steps, the clock's speed and the dispatcher");
    if l.ui.button("company-tomorrow", tomorrow, "Simulate to tomorrow", Some("nights_stay"), ButtonKind::Primary) {
        if ok {
            let to = ck::target(&c, Step::Midnight);
            simulate(l, to, false);
        } else {
            busy(l);
        }
    }
    l.ui.tooltip(tomorrow, if in_game { "The game leads the company's clock while it runs" } else { "Simulate the rest of the day: the tours run, what happens is told, and the day is closed at midnight" });
    if l.ui.button("company-hour", hour, "+1 h", None, ButtonKind::Normal) {
        if ok {
            let to = ck::target(&c, Step::Minutes(60));
            simulate(l, to, false);
        } else {
            busy(l);
        }
    }
    l.ui.tooltip(hour, "Simulate one hour of the company's time");
    // the play buttons, where there is room
    if r.w >= 640.0 {
        for (k, (label, speed)) in [("×600", 600.0 / 60.0), ("×60", 1.0)].into_iter().enumerate() {
            let w = if k == 0 { 70.0 } else { 62.0 };
            let b = Rect::new(x - gap - w, r.y, w, r.h);
            x = b.x;
            let on = (l.company.clock.speed - speed).abs() < 1e-6;
            if l.ui.button(&format!("company-play-{k}"), b, label, None, if on { ButtonKind::Primary } else { ButtonKind::Ghost }) {
                if ok || on {
                    play(l, if on { 0.0 } else { speed });
                } else {
                    busy(l);
                }
            }
            l.ui.tooltip(b, if on { "Stop the clock" } else if k == 0 { "Run the company's clock at ten minutes a second while this page is open" } else { "Run the company's clock at a minute a second while this page is open" });
        }
    }
    // the day and the time
    let date = day_label(&c.date);
    let time = ck::hhmm(ck::now(&c));
    let label = omsi_ui::tr(if in_game { "Company time · the game's" } else { "Company time" }).to_uppercase();
    let tw = l.ui.width(&time, 22.0, Weight::Bold);
    let dw = (l.ui.width(&date, 14.0, Weight::Medium) + tw + 12.0).max(l.ui.width(&label, 11.5, Weight::Bold)) + 8.0;
    let dx = x - 16.0 - dw;
    if dx < r.x {
        return x;
    }
    l.ui.text_in(&label, Rect::new(dx, r.y - 3.0, dw, 15.0), 11.5, Weight::Bold, if in_game { OK } else { TEXT_DIM }, Align::Right);
    l.ui.text_in(&time, Rect::new(dx, r.y + 12.0, dw, 28.0), 22.0, Weight::Bold, if l.company.clock.speed > 0.0 { accent_2() } else { TEXT }, Align::Right);
    l.ui.text_in(&date, Rect::new(dx, r.y + 14.0, dw - tw - 12.0, 26.0), 14.0, Weight::Medium, TEXT_SOFT, Align::Right);
    dx
}

/// Why the clock cannot be stepped now, in a popup.
pub(super) fn busy(l: &mut Launcher) {
    let p = if l.state.in_game() {
        kit::Popup::new("sports_esports", "The game leads the clock", omsi_ui::tr("The game runs on the company's map: the company's day goes along with the game's time."), omsi_ui::tr("Simulating is offered again when the game is closed."), None)
    } else if l.company.company.as_ref().is_some_and(|c| c.clock.ask.is_some()) {
        kit::Popup::new("help", "A decision waits", omsi_ui::tr("The company's clock waits for your answer first."), "", None)
    } else {
        kit::Popup::new("timer", "The clock is running", omsi_ui::tr("The company's time is being simulated: wait a moment."), "", None)
    };
    kit::show(l, p);
}

/// The clock runs (company minutes a second) or stands.
pub(super) fn play(l: &mut Launcher, speed: f64) {
    l.company.clock.speed = speed;
    l.company.clock.acc = 0.0;
    l.company.clock.last = Some(Instant::now());
    if speed == 0.0 {
        flush(l);
    }
}

/// The time dialog: every step, "until", the clock's speed, the dispatcher, the quick close.
pub fn time_dialog(l: &mut Launcher) {
    let Some(c) = l.company.company.clone() else {
        l.company.dialog = None;
        return;
    };
    let title = omsi_ui::tr("Company time: %{now}").replace("%{now}", &now_label(&c));
    let in_game = l.state.in_game();
    let f = kit::frame(l, 780.0, if in_game { 680.0 } else { 620.0 }, "schedule", &title);
    let inner = f.body;
    let ok = may_step(l);
    let mut y = inner.y;
    if in_game {
        y += l.ui.paragraph("The game runs on the company's map: its clock leads, and the company's day goes along with it. Simulating is offered again when the game is closed.", Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, WARN) + 12.0;
    }
    let heading = |l: &mut Launcher, y: f32, t: &str| {
        kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), t);
    };
    heading(l, y, "Simulate");
    y += 26.0;
    let steps: [(&str, Step); 9] = [
        ("+15 min", Step::Minutes(15)),
        ("+1 h", Step::Minutes(60)),
        ("+6 h", Step::Minutes(360)),
        ("To the morning", Step::Morning),
        ("To tomorrow", Step::Midnight),
        ("+1 day", Step::Days(1)),
        ("+2 days", Step::Days(2)),
        ("+3 days", Step::Days(3)),
        ("+7 days", Step::Days(7)),
    ];
    let cols = 5;
    let bw = (inner.w - (cols - 1) as f32 * 8.0) / cols as f32;
    let mut chosen: Option<Step> = None;
    for (k, (label, step)) in steps.iter().enumerate() {
        let b = Rect::new(inner.x + (k % cols) as f32 * (bw + 8.0), y + (k / cols) as f32 * 46.0, bw, 38.0);
        if l.ui.button(&format!("company-step-{k}"), b, label, None, ButtonKind::Normal) {
            if ok {
                chosen = Some(*step);
            } else {
                busy(l);
            }
        }
        let to = ck::target(&c, *step);
        l.ui.tooltip(b, &format!("{}  {}", day_label(&ck::date_of(to)), ck::hhmm(to)));
    }
    y += 2.0 * 46.0 + 10.0;
    // until a time
    let mut m = l.company.clock.until;
    let fmt = |v: f32| ck::hhmm(v as i64);
    let label = omsi_ui::tr("Simulate until %{time}").replace("%{time}", &ck::hhmm(m as i64));
    let gw = Foot::width(&l.ui, &label, Some("schedule")) + 10.0;
    l.ui.slider("company-until", Rect::new(inner.x, y, inner.w - gw - 14.0, 40.0), &mut m, 0.0, (DAY - 15) as f32, 15.0, "Until", &fmt);
    l.company.clock.until = m;
    if l.ui.button("company-until-go", Rect::new(inner.right() - gw, y, gw, 40.0), &label, Some("schedule"), ButtonKind::Normal) {
        if ok {
            chosen = Some(Step::Until(m as i64));
        } else {
            busy(l);
        }
    }
    y += 40.0 + 24.0;
    heading(l, y, "Run the clock");
    y += 26.0;
    let speeds = [("Stop", 0.0), ("×60", 1.0), ("×600", 10.0)];
    let sw = 110.0;
    for (k, (label, speed)) in speeds.iter().enumerate() {
        let on = (l.company.clock.speed - speed).abs() < 1e-6;
        if l.ui.button(&format!("company-speed-{k}"), Rect::new(inner.x + k as f32 * (sw + 8.0), y, sw, 38.0), label, None, if on { ButtonKind::Primary } else { ButtonKind::Normal }) {
            if ok || *speed == 0.0 {
                play(l, *speed);
            } else {
                busy(l);
            }
        }
    }
    l.ui.paragraph(&omsi_ui::tr("×60: a minute a second; it stops for what needs you."), Vec2::new(inner.x + 3.0 * (sw + 8.0) + 10.0, y + 2.0), inner.w - 3.0 * (sw + 8.0) - 10.0, kit::NOTE, Weight::Regular, TEXT_SOFT);
    y += 38.0 + 24.0;
    heading(l, y, "Decisions");
    y += 26.0;
    let mut auto = c.clock.dispatcher;
    if l.ui.toggle("company-dispatcher", Rect::new(inner.x, y, inner.w, ROW), &mut auto, "The dispatcher decides breakdowns (a rental bus while the cash allows)") {
        if let Some(c) = l.company.company.as_mut() {
            c.clock.dispatcher = auto;
        }
        changed(l);
    }
    y += ROW + 8.0;
    let _ = y;
    // the quick close, and done
    let mut foot = Foot::new(&f);
    if foot.left(l, "company-quick-close", "Close the day quickly", Some("check_circle"), ButtonKind::Normal) {
        if ok {
            let to = ck::target(&c, Step::Midnight);
            l.company.dialog = None;
            simulate(l, to, true);
            return;
        }
        busy(l);
    }
    l.ui.tooltip(Rect::new(f.foot.x, f.foot.y, 260.0, f.foot.h), "The rest of the day at once: nothing stops, the dispatcher decides");
    if foot.right(l, "company-time-done", "Close", None, ButtonKind::Primary) || f.close {
        l.company.dialog = None;
    }
    if let Some(step) = chosen {
        l.company.dialog = None;
        let to = ck::target(&c, step);
        simulate(l, to, false);
    }
}

/// A feed line's words, with its places filled.
pub(super) fn text(f: &FeedItem) -> String {
    let mut t = omsi_ui::tr(&f.text).into_owned();
    for (k, v) in &f.args {
        let v = match k.as_str() {
            "amount" => v.parse::<i64>().map(eur).unwrap_or_else(|_| v.clone()),
            "date" => day_label(v),
            "why" | "job" | "area" => omsi_ui::tr(v).into_owned(),
            "bus" if v == "rental" => omsi_ui::tr("rental bus").into_owned(),
            _ => v.clone(),
        };
        t = t.replace(&format!("%{{{k}}}"), &v);
    }
    t
}

pub(super) fn colour(level: Level) -> Color {
    match level {
        Level::Minor => TEXT_DIM,
        Level::Info => TEXT_SOFT,
        Level::Good => OK,
        Level::Warn => WARN,
        Level::Bad => DANGER.lighten(0.25),
    }
}

/// The feed: what happened, the newest first (the ordinary run of the day only when asked).
pub fn feed(l: &mut Launcher, r: Rect, c: &Company, minor: &mut bool) {
    let inner = super::section(&mut l.ui, r, "The company's day");
    let tw = 170.0;
    let mut m = *minor;
    let tr = Rect::new(inner.right() - tw, r.y + 9.0, tw, 26.0);
    if l.ui.toggle("company-feed-minor", tr, &mut m, "All tours") {
        *minor = m;
    }
    l.ui.tooltip(tr, "Show every tour leaving and coming back too");
    let items: Vec<FeedItem> = c.clock.feed.iter().rev().filter(|f| *minor || f.level != Level::Minor).take(120).cloned().collect();
    if items.is_empty() && c.clock.feed.iter().any(|f| f.at >= ck::moment(&c.date, 0)) {
        l.ui.paragraph("The day runs as planned so far. \"All tours\" shows every tour leaving and coming back.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    if items.is_empty() {
        l.ui.paragraph("Nothing happened yet. Simulate the company's time at the top: by the hour, to the morning, to tomorrow or days at once; what happens is told here.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let today = c.date.clone();
    l.ui.scroll_area("company-feed", inner, &mut |ui: &mut Ui, v: Rect| {
        let mut y = v.y;
        let mut day = String::new();
        for f in &items {
            let d = ck::date_of(f.at);
            if d != day {
                day = d.clone();
                if d != today {
                    ui.text_in(&day_label(&d).to_uppercase(), Rect::new(v.x, y + 6.0, v.w, 16.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
                    y += 28.0;
                }
            }
            let t = text(f);
            ui.text_in(&ck::hhmm(f.at), Rect::new(v.x, y, 52.0, 21.0), kit::ROWS, Weight::Bold, TEXT_DIM, Align::Left);
            ui.p().circle(Vec2::new(v.x + 62.0, y + 10.5), 4.0, colour(f.level));
            let h = ui.paragraph(&t, Vec2::new(v.x + 76.0, y), v.w - 86.0, kit::ROWS, Weight::Regular, if f.level == Level::Minor { TEXT_DIM } else { TEXT });
            y += h.max(21.0) + 8.0;
        }
        y - v.y + 6.0
    });
}

/// What the simulation waits for: a breakdown (a rental bus or the trips dropped), a tender
/// opened or the player's bid beaten (to the auction, or on), the morning's disruptions.
pub fn ask_dialog(l: &mut Launcher) {
    let Some(c) = l.company.company.clone() else { return };
    let Some(ask) = c.clock.ask.clone() else { return };
    let when = ck::hhmm(ck::now(&c));
    let mut answer: Option<(Option<Choice>, bool)> = None;
    match &ask {
        Ask::Breakdown { index } => {
            let Some(b) = c.clock.today.as_ref().and_then(|s| s.breaks.get(*index)).cloned() else {
                answer = Some((None, true));
                return finish(l, answer);
            };
            let title = omsi_ui::tr("%{time}  Bus %{bus} broke down").replace("%{time}", &when).replace("%{bus}", &b.bus);
            let cost = ck::rental_cost(&c, *index);
            let back = ck::hhmm(ck::moment(&c.date, (b.at + ck::RENTAL_MINUTES) as i64));
            let t = omsi_ui::tr("It was on line %{n}; it goes to the workshop. A rental bus can take over its trips from %{time} for %{amount}; else its trips are dropped for the rest of the day, with the contract's penalty.").replace("%{n}", &b.line).replace("%{time}", &back).replace("%{amount}", &eur(cost));
            let th = l.ui.paragraph_height(&t, 640.0 - 56.0, kit::BODY, Weight::Regular);
            // (a decision the clock waits for: no cross, both answers lead on)
            let f = kit::frame_with(l, 640.0, 70.0 + th + 20.0 + ROW + 24.0 + kit::BUTTON_H + 26.0, "warning", &title, false);
            l.ui.paragraph(&t, Vec2::new(f.body.x, f.body.y), f.body.w, kit::BODY, Weight::Regular, TEXT_SOFT);
            let mut auto = c.clock.dispatcher;
            if l.ui.toggle("company-ask-dispatcher", Rect::new(f.body.x, f.body.y + th + 16.0, f.body.w, ROW), &mut auto, "From now on the dispatcher decides") {
                if let Some(c) = l.company.company.as_mut() {
                    c.clock.dispatcher = auto;
                }
            }
            let mut foot = Foot::new(&f);
            let label = omsi_ui::tr("Rental bus, %{amount}").replace("%{amount}", &eur(cost));
            if foot.right(l, "company-ask-rental", &label, Some("directions_bus"), ButtonKind::Primary) {
                answer = Some((Some(Choice::Rental), true));
            }
            if foot.right(l, "company-ask-drop", "Drop its trips", None, ButtonKind::Normal) {
                answer = Some((Some(Choice::Drop), true));
            }
        }
        Ask::Tender { id, outbid } => {
            let Some(t) = c.concessions.tenders.iter().find(|t| t.id == *id).cloned() else {
                return finish(l, Some((None, true)));
            };
            let title = if *outbid { omsi_ui::tr("%{time}  Your bid for line %{n} is beaten") } else { omsi_ui::tr("%{time}  Line %{n} is out to tender") }.replace("%{time}", &when).replace("%{n}", &t.number);
            let left = (t.closes_at - ck::now(&c)).max(0);
            let mut s = omsi_ui::tr("Bids until %{time} (%{left} from now). The least bid now: %{amount}; buying it outright: %{buy}.").replace("%{time}", &ck::hhmm(t.closes_at)).replace("%{left}", &format!("{}:{:02} h", left / 60, left % 60)).replace("%{amount}", &eur(co::concessions::min_bid(&c, &t))).replace("%{buy}", &eur(t.buy_out()));
            if *outbid {
                let placed = co::concessions::bids(&c, &t, ck::now(&c));
                if let Some(p) = placed.last() {
                    s = format!("{}  {}", omsi_ui::tr("%{who} bids %{amount}.").replace("%{who}", &co::concessions::bidder_name(&c, &t, p.who)).replace("%{amount}", &eur(p.amount)), s);
                }
            }
            let th = l.ui.paragraph_height(&s, 640.0 - 56.0, kit::BODY, Weight::Regular);
            let f = kit::frame(l, 640.0, 70.0 + th + 24.0 + kit::BUTTON_H + 26.0, "payments", &title);
            l.ui.paragraph(&s, Vec2::new(f.body.x, f.body.y), f.body.w, kit::BODY, Weight::Regular, TEXT_SOFT);
            let mut foot = Foot::new(&f);
            if foot.right(l, "company-ask-auction", "To the auction", Some("payments"), ButtonKind::Primary) {
                l.company.tab = super::CONCESSIONS_TAB;
                l.company.tenders.selected = Some(*id);
                answer = Some((None, false));
            }
            if foot.right(l, "company-ask-on", if *outbid { "Let it go" } else { "Go on" }, None, ButtonKind::Normal) {
                answer = Some((None, true));
            }
            if f.close && answer.is_none() {
                answer = Some((None, false));
            }
        }
        Ask::Morning { count } => {
            let title = omsi_ui::tr("%{time}  The morning").replace("%{time}", &when);
            let t = omsi_ui::tr("%{n} things fell out this morning: drivers late, buses that did not start. The central fills them as it can; the planning shows them and lets you choose.").replace("%{n}", &count.to_string());
            let th = l.ui.paragraph_height(&t, 640.0 - 56.0, kit::BODY, Weight::Regular);
            let f = kit::frame(l, 640.0, 70.0 + th + 24.0 + kit::BUTTON_H + 26.0, "wb_twilight", &title);
            l.ui.paragraph(&t, Vec2::new(f.body.x, f.body.y), f.body.w, kit::BODY, Weight::Regular, TEXT_SOFT);
            let mut foot = Foot::new(&f);
            if foot.right(l, "company-ask-plan", "Open the planning", Some("event"), ButtonKind::Primary) {
                l.company.tab = 5;
                answer = Some((None, false));
            }
            if foot.right(l, "company-ask-on", "Go on", None, ButtonKind::Normal) {
                answer = Some((None, true));
            }
            if f.close && answer.is_none() {
                answer = Some((None, false));
            }
        }
        Ask::Notice { text, args } => {
            let item = FeedItem { at: ck::now(&c), text: text.clone(), args: args.clone(), level: Level::Bad };
            let t = super::clock::text(&item);
            let th = l.ui.paragraph_height(&t, 600.0 - 56.0, kit::BODY, Weight::Regular);
            let f = kit::frame(l, 600.0, 70.0 + th + 24.0 + kit::BUTTON_H + 26.0, "info", &when);
            l.ui.paragraph(&t, Vec2::new(f.body.x, f.body.y), f.body.w, kit::BODY, Weight::Regular, TEXT_SOFT);
            let mut foot = Foot::new(&f);
            if foot.right(l, "company-ask-ok", "Go on", None, ButtonKind::Primary) {
                answer = Some((None, true));
            }
            if f.close && answer.is_none() {
                answer = Some((None, false));
            }
        }
    }
    finish(l, answer);
}

/// The answer given: applied, and the simulation goes on where it was going (`on`), unless the
/// clock plays (it goes on by itself) or the player went to look.
fn finish(l: &mut Launcher, answer: Option<(Option<Choice>, bool)>) {
    let Some((choice, on)) = answer else { return };
    let Some(c) = l.company.company.as_mut() else { return };
    let mut run = Run::default();
    let to = ck::answer(c, choice, &mut run);
    changed(l);
    if on && l.company.clock.speed == 0.0 {
        if let Some(to) = to {
            simulate(l, to, false);
        }
    }
}
