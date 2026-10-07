//! The concession market: the map's lines the authority puts out to tender, with their week
//! (tours, kilometres, what they bring in), and the concessions the company holds with their
//! terms (the rules are `omsi_launcher_lib::company::concessions`'). A tender is an auction of
//! a few hours of the company's time: the panel beside the list shows the one picked as it
//! runs - the rivals with their character and their bids as they come with the clock, the
//! least bid that leads now and the chance it has, buying the line outright - and when it
//! closed, who won and for how much.
//!
//! The lines' weeks are read from the timetable of the seven days from the company's date on
//! a thread of their own (the page says so meanwhile).

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit;
use super::{act, day_label, eur, eur_cents, plate, section};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::auction::Who;
use omsi_launcher_lib::company::clock::{self as ck, Step};
use omsi_launcher_lib::company::concessions::{self as cn, Outcome, Tender, Week};
use omsi_launcher_lib::company::{dates, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

#[derive(Default)]
pub struct TendersView {
    /// The lines' weeks, for the map and the first day they were read for.
    weeks: HashMap<String, Week>,
    weeks_for: Option<(String, String)>,
    reading: Option<Receiver<HashMap<String, Week>>>,
    /// The tender in the auction panel.
    pub(super) selected: Option<u32>,
    /// The sum on the bid slider (euros), and for which tender.
    amount: f32,
    amount_for: Option<u32>,
}

/// The weeks of every map line, read in the background once per map and week.
fn weeks(l: &mut Launcher, c: &Company) {
    let view = &mut l.company.tenders;
    if let Some(rx) = view.reading.as_ref() {
        if let Ok(w) = rx.try_recv() {
            view.weeks = w;
            view.reading = None;
        }
    }
    let monday = co_monday(&c.date);
    let key = (c.map.clone(), monday.clone());
    if view.weeks_for.as_ref() == Some(&key) {
        return;
    }
    view.weeks_for = Some(key);
    let (tx, rx) = channel();
    view.reading = Some(rx);
    let map = c.map.clone();
    let _ = std::thread::Builder::new().name("company weeks".into()).spawn(move || {
        let days: Vec<Vec<core::LineInfo>> = (0..7).filter_map(|k| core::list_lines(&map, &dates::add(&monday, k)).ok()).collect();
        let mut out = HashMap::new();
        for l in days.first().map(|d| d.iter().map(|l| l.name.clone()).collect::<Vec<_>>()).unwrap_or_default() {
            out.insert(l.to_lowercase(), cn::week_of(&days, &l));
        }
        let _ = tx.send(out);
    });
}

/// The Monday of a date's week (the week is read from it).
fn co_monday(date: &str) -> String {
    let d = dates::parse(date).unwrap_or(0);
    dates::fmt(d - dates::weekday(d) as i64)
}

fn week_of(l: &Launcher, line: &str) -> Option<Week> {
    l.company.tenders.weeks.get(&line.to_lowercase()).copied()
}

/// When a minute of the clock is: its time, and its day when it is not today.
fn when(c: &Company, m: i64) -> String {
    let d = ck::date_of(m);
    if d == c.date {
        ck::hhmm(m)
    } else {
        format!("{} {}", day_label(&d), ck::hhmm(m))
    }
}

/// "2:10 h".
fn span(minutes: i64) -> String {
    let m = minutes.max(0);
    format!("{}:{:02} h", m / 60, m % 60)
}

/// A tender's state in a few words, and its colour.
fn state_of(c: &Company, t: &Tender) -> (String, Color) {
    let now = ck::now(c);
    match &t.outcome {
        None if now < t.opens_at => (omsi_ui::tr("Opens %{when}").replace("%{when}", &when(c, t.opens_at)), TEXT_SOFT),
        None => {
            let placed = cn::bids(c, t, now);
            let lead = match placed.last() {
                Some(p) if p.who == Who::Player => omsi_ui::tr("you lead with %{amount}").replace("%{amount}", &eur(p.amount)),
                Some(p) => omsi_ui::tr("%{who} leads with %{amount}").replace("%{who}", &cn::bidder_name(c, t, p.who)).replace("%{amount}", &eur(p.amount)),
                None => omsi_ui::tr("no bid yet").into_owned(),
            };
            let mine = !t.offers.is_empty();
            let colour = match placed.last() {
                Some(p) if p.who == Who::Player => OK,
                _ if mine => WARN,
                _ => accent_2(),
            };
            (format!("{}  ·  {}", omsi_ui::tr("Running until %{time}").replace("%{time}", &ck::hhmm(t.closes_at)), lead), colour)
        }
        Some(Outcome::Won { amount, bought, .. }) => (omsi_ui::tr(if *bought { "Bought for %{amount}" } else { "Won for %{amount}" }).replace("%{amount}", &eur(*amount)), OK),
        Some(Outcome::Lost { winner, amount, .. }) => (omsi_ui::tr("Lost to %{who} for %{amount}").replace("%{who}", winner).replace("%{amount}", &eur(*amount)), WARN),
        Some(Outcome::NoBid { winner, amount }) if !winner.is_empty() => (omsi_ui::tr("Went to %{who} for %{amount}").replace("%{who}", winner).replace("%{amount}", &eur(*amount)), TEXT_FAINT),
        Some(Outcome::NoBid { .. }) => (omsi_ui::tr("Nobody bid").into_owned(), TEXT_FAINT),
    }
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    weeks(l, &c);
    let gap = 16.0;
    let q = cn::quality(&c, false);
    let foot = omsi_ui::tr("An auction runs a few hours of the company's time; the best offer when it closes wins, weighed with the bidder's name - yours is %{q} of 100, from your reputation and punctuality.").replace("%{q}", &format!("{q:.0}"));
    l.ui.text_in(&foot, Rect::new(area.x, area.bottom() - 22.0, area.w, 22.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
    let area = Rect::new(area.x, area.y, area.w, (area.h - 32.0).max(0.0));
    let left_w = ((area.w - gap) * 0.5).max(360.0);
    let list_h = ((area.h - gap) * 0.58).max(200.0);
    tenders(l, Rect::new(area.x, area.y, left_w, list_h), &c);
    held(l, Rect::new(area.x, area.y + list_h + gap, left_w, (area.h - list_h - gap).max(0.0)), &c);
    auction(l, Rect::new(area.x + left_w + gap, area.y, area.w - left_w - gap, area.h), &c);
}

/// The tenders: announced and running first, then the results; one picked for the panel.
fn tenders(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Tenders");
    let top = if cn::may_add_directly(c) {
        l.ui.paragraph("On an easy economy the map's lines are taken on directly on the Lines page and their concessions renew by themselves; tenders are offered all the same, for the practice.", Vec2::new(inner.x, inner.y), inner.w, 14.0, Weight::Regular, TEXT_DIM);
        48.0
    } else {
        0.0
    };
    let rows = Rect::new(inner.x, inner.y + top, inner.w, (inner.h - top).max(0.0));
    let mut list: Vec<Tender> = c.concessions.tenders.clone();
    let now = ck::now(c);
    list.sort_by(|a, b| b.open().cmp(&a.open()).then(if a.open() { a.opens_at.cmp(&b.opens_at) } else { b.closes_at.cmp(&a.closes_at) }));
    if list.is_empty() {
        let t = if l.company.today.is_none() { "Reading the timetable…" } else { "No line is out to tender now. A new round comes every four weeks; a line of the map can also be applied for on the Lines page." };
        l.ui.paragraph(t, Vec2::new(rows.x, rows.y), rows.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    }
    // (the panel shows the one picked, else the first running or announced)
    if l.company.tenders.selected.is_none_or(|id| !list.iter().any(|t| t.id == id)) {
        l.company.tenders.selected = list.first().map(|t| t.id);
    }
    let selected = l.company.tenders.selected;
    let weeks: Vec<Option<Week>> = list.iter().map(|t| t.week.or_else(|| week_of(l, &t.line))).collect();
    let reading = l.company.tenders.reading.is_some();
    let cc = c.clone();
    let mut pick: Option<u32> = None;
    l.ui.scroll_area("company-tenders", rows, &mut |ui: &mut Ui, v: Rect| {
        let rh = 88.0;
        for (k, t) in list.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 6.0);
            if !ui.rect_visible(r) {
                continue;
            }
            if ui.row(&format!("company-tender-{}", t.id), r, false) {
                pick = Some(t.id);
            }
            if selected == Some(t.id) {
                ui.p().rounded(r, 8.0, Color::WHITE.alpha(0.05));
                ui.p().rounded(Rect::new(r.x, r.y + 10.0, 3.0, r.h - 20.0), 1.5, accent());
            }
            let dim = !t.open();
            let x = r.x + 10.0;
            let w = plate(ui, Vec2::new(x, r.y + 10.0), &t.number, 26.0);
            let caption = if t.caption.is_empty() { t.line.clone() } else { t.caption.clone() };
            ui.text_in(&caption, Rect::new(x + w + 12.0, r.y + 8.0, r.w - w - 120.0, 22.0), kit::ROWS, Weight::Bold, if dim { TEXT_DIM } else { TEXT }, Align::Left);
            let figures = match weeks[k] {
                Some(wk) if wk.km < 1.0 => omsi_ui::tr("%{t} tours, %{n} trips a week").replace("%{t}", &wk.tours.to_string()).replace("%{n}", &wk.trips.to_string()),
                Some(wk) => omsi_ui::tr("%{t} tours, %{km} km a week  ·  about %{amount}").replace("%{t}", &wk.tours.to_string()).replace("%{km}", &format!("{:.0}", wk.km)).replace("%{amount}", &eur(cn::week_revenue(&cc, &wk, 1.0))),
                None if reading => omsi_ui::tr("Reading its week…").into_owned(),
                None => omsi_ui::tr("%{t} tours, %{km} km on the day it was offered").replace("%{t}", &t.day_tours.to_string()).replace("%{km}", &format!("{:.0}", t.day_km)),
            };
            ui.text_in(&figures, Rect::new(x + w + 12.0, r.y + 34.0, r.w - w - 30.0, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
            let (state, colour) = state_of(&cc, t);
            ui.text_in(&state, Rect::new(x + w + 12.0, r.y + 58.0, r.w - w - 30.0, 20.0), kit::NOTE, Weight::Medium, colour, Align::Left);
            if t.renewal {
                ui.badge(Vec2::new(r.right() - 90.0, r.y + 8.0), &omsi_ui::tr("renewal").to_uppercase(), accent_2());
            } else if t.running(now) {
                ui.badge(Vec2::new(r.right() - 70.0, r.y + 8.0), &omsi_ui::tr("live").to_uppercase(), OK);
            }
        }
        list.len() as f32 * rh
    });
    if let Some(id) = pick {
        l.company.tenders.selected = Some(id);
    }
}

fn held(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Your concessions");
    let held = c.concessions.held.clone();
    let own: Vec<&omsi_launcher_lib::company::CompanyLine> = c.lines.iter().filter(|x| x.own).collect();
    if held.is_empty() && own.is_empty() {
        l.ui.paragraph("The company holds no concession yet. Win a tender, or apply for a line of the map on the Lines page.", Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    }
    let today = c.date.clone();
    let lic = eur((cn::LICENCE as f64 * c.price_index).round() as i64);
    let own: Vec<(String, String)> = own.iter().map(|x| (x.number.clone(), x.caption.clone())).collect();
    let per_km = cn::reference_per_km(c);
    l.ui.scroll_area("company-concessions", inner, &mut |ui: &mut Ui, v: Rect| {
        let rh = 60.0;
        let mut y = v.y;
        for h in &held {
            let r = Rect::new(v.x, y, v.w - 10.0, rh - 6.0);
            y += rh;
            if !ui.rect_visible(r) {
                continue;
            }
            let w = plate(ui, Vec2::new(r.x, r.y + 4.0), &h.number, 22.0);
            let left = dates::between(&today, &h.until);
            let until = if dates::between(&today, &h.from) > 0 { omsi_ui::tr("from %{date}").replace("%{date}", &day_label(&h.from)) } else { omsi_ui::tr("until %{date}").replace("%{date}", &day_label(&h.until)) };
            let colour = if left <= cn::RENEW_BEFORE { WARN } else { TEXT };
            ui.text_in(&until, Rect::new(r.x + w + 12.0, r.y + 2.0, r.w - w - 12.0, 22.0), kit::ROWS, Weight::Bold, colour, Align::Left);
            let price = omsi_ui::tr("%{p} % of the reference: %{km} a kilometre").replace("%{p}", &format!("{:.0}", h.price * 100.0)).replace("%{km}", &eur_cents(per_km * h.price));
            ui.text_in(&price, Rect::new(r.x + w + 12.0, r.y + 27.0, r.w - w - 12.0, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
            if h.direct {
                ui.badge(Vec2::new(r.right() - 80.0, r.y + 4.0), &omsi_ui::tr("direct").to_uppercase(), TEXT_DIM);
            }
        }
        for (number, caption) in &own {
            let r = Rect::new(v.x, y, v.w - 10.0, rh - 6.0);
            y += rh;
            if !ui.rect_visible(r) {
                continue;
            }
            let w = plate(ui, Vec2::new(r.x, r.y + 4.0), number, 22.0);
            ui.text_in(caption, Rect::new(r.x + w + 12.0, r.y + 2.0, r.w - w - 12.0, 22.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let t = omsi_ui::tr("Your own line: no concession, a licence of %{amount} a month").replace("%{amount}", &lic);
            ui.text_in(&t, Rect::new(r.x + w + 12.0, r.y + 27.0, r.w - w - 12.0, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        }
        y - v.y
    });
}

/// The auction of the tender picked: the bidders and their bids as the clock brings them, the
/// least bid that leads now with its chance, buying outright; or how it ended.
fn auction(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Auction");
    let Some(t) = l.company.tenders.selected.and_then(|id| c.concessions.tenders.iter().find(|t| t.id == id)).cloned() else {
        l.ui.paragraph("Pick a tender on the left: its auction is shown here as it runs.", Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    };
    let now = ck::now(c);
    let mut y = inner.y;
    let w = plate(&mut l.ui, Vec2::new(inner.x, y + 2.0), &t.number, 26.0);
    let caption = if t.caption.is_empty() { t.line.clone() } else { t.caption.clone() };
    l.ui.text_in(&caption, Rect::new(inner.x + w + 12.0, y, inner.w - w - 12.0, 30.0), 16.0, Weight::Bold, TEXT, Align::Left);
    y += 38.0;
    let (state, colour) = state_of(c, &t);
    l.ui.text_in(&state, Rect::new(inner.x, y, inner.w, 18.0), kit::ROWS, Weight::Medium, colour, Align::Left);
    y += 26.0;
    // the frame: the line's worth, the least bid now, buying it outright
    let least = cn::min_bid(c, &t);
    let fw = (inner.w - 16.0) / 3.0;
    let figure = |l: &mut Launcher, k: usize, label: &str, value: String, col: Color| {
        let x = inner.x + k as f32 * (fw + 8.0);
        l.ui.text_in(&omsi_ui::tr(label).to_uppercase(), Rect::new(x, y, fw, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
        l.ui.text_in(&value, Rect::new(x, y + 18.0, fw, 28.0), 19.0, Weight::Bold, col, Align::Left);
    };
    figure(l, 0, "The line's worth", eur(t.value), TEXT);
    figure(l, 1, if t.open() { "Least bid now" } else { "Term" }, if t.open() { eur(least) } else { omsi_ui::tr("%{n} weeks").replace("%{n}", &t.weeks.to_string()) }, TEXT);
    figure(l, 2, "Buy outright", eur(t.buy_out()), accent_2());
    y += 58.0;
    // its time
    let len = (t.closes_at - t.opens_at).max(1);
    let frac = ((now - t.opens_at) as f32 / len as f32).clamp(0.0, 1.0);
    l.ui.progress(Rect::new(inner.x, y, inner.w, 6.0), if t.open() { frac } else { 1.0 }, false);
    y += 12.0;
    let times = if !t.open() {
        omsi_ui::tr("Closed at %{when}").replace("%{when}", &when(c, t.closes_at))
    } else if now < t.opens_at {
        omsi_ui::tr("Opens %{when}, runs %{len}").replace("%{when}", &when(c, t.opens_at)).replace("%{len}", &span(len))
    } else {
        omsi_ui::tr("Closes at %{time}: %{left} left").replace("%{time}", &ck::hhmm(t.closes_at)).replace("%{left}", &span(t.closes_at - now))
    };
    l.ui.text_in(&times, Rect::new(inner.x, y, inner.w, 16.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
    y += 26.0;
    // the bidders
    let placed = cn::bids(c, &t, if t.open() { now } else { t.closes_at });
    let leader = placed.last().map(|p| p.who);
    let rh = 34.0;
    let mut rows: Vec<(String, String, Option<i64>, bool)> = t
        .bidders
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let r = cn::rival(c, *id)?;
            let last = placed.iter().rev().find(|p| p.who == Who::Rival(i)).map(|p| p.amount);
            Some((r.name.clone(), omsi_ui::tr(r.character.label()).into_owned(), last, leader == Some(Who::Rival(i))))
        })
        .collect();
    let mine = placed.iter().rev().find(|p| p.who == Who::Player).map(|p| p.amount);
    rows.push((omsi_ui::tr("You").into_owned(), omsi_ui::tr("your name counts ×%{w}").replace("%{w}", &format!("{:.2}", cn::weight(c, &t))), mine, leader == Some(Who::Player)));
    for (name, kind, last, leads) in &rows {
        if *leads {
            l.ui.p().rounded(Rect::new(inner.x - 6.0, y - 2.0, inner.w + 12.0, rh - 2.0), 6.0, Color::WHITE.alpha(0.05));
        }
        l.ui.p().circle(Vec2::new(inner.x + 4.0, y + rh * 0.5 - 1.0), 3.5, if *leads { OK } else { TEXT_FAINT });
        l.ui.text_in(name, Rect::new(inner.x + 16.0, y, inner.w * 0.45, rh - 4.0), kit::ROWS, Weight::Medium, TEXT, Align::Left);
        l.ui.text_in(kind, Rect::new(inner.x + 16.0 + inner.w * 0.45, y, inner.w * 0.3, rh - 4.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        let v = last.map(eur).unwrap_or_else(|| omsi_ui::tr("no bid").into_owned());
        l.ui.text_in(&v, Rect::new(inner.right() - 130.0, y, 130.0, rh - 4.0), 13.0, if *leads { Weight::Bold } else { Weight::Regular }, if last.is_some() { TEXT } else { TEXT_FAINT }, Align::Right);
        y += rh;
    }
    y += 8.0;
    // what to do
    let actions_h = if t.running(now) { 168.0 } else { 52.0 };
    let log_h = (inner.bottom() - actions_h - y).max(0.0);
    if log_h > 40.0 {
        let log: Vec<(String, String, i64, bool)> = placed.iter().rev().take(30).map(|p| (ck::hhmm(p.at), if p.who == Who::Player { omsi_ui::tr("You").into_owned() } else { cn::bidder_name(c, &t, p.who) }, p.amount, p.who == Who::Player)).collect();
        l.ui.text_in(&omsi_ui::tr("Bids").to_uppercase(), Rect::new(inner.x, y, inner.w, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
        let area = Rect::new(inner.x, y + 18.0, inner.w, log_h - 22.0);
        if log.is_empty() {
            l.ui.text_in(&omsi_ui::tr("No bid yet."), Rect::new(area.x, area.y, area.w, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        } else {
            l.ui.scroll_area("company-auction-log", area, &mut |ui: &mut Ui, v: Rect| {
                for (k, (at, who, amount, me)) in log.iter().enumerate() {
                    let yy = v.y + k as f32 * 26.0;
                    ui.text_in(at, Rect::new(v.x, yy, 56.0, 24.0), kit::NOTE, Weight::Bold, TEXT_DIM, Align::Left);
                    ui.text_in(who, Rect::new(v.x + 60.0, yy, v.w * 0.6, 24.0), kit::NOTE, Weight::Regular, if *me { accent_2() } else { TEXT_SOFT }, Align::Left);
                    ui.text_in(&eur(*amount), Rect::new(v.right() - 150.0, yy, 140.0, 24.0), kit::NOTE, Weight::Medium, TEXT, Align::Right);
                }
                log.len() as f32 * 26.0
            });
        }
    }
    let ay = inner.bottom() - actions_h;
    let ok = !l.company.closing && !l.state.in_game() && c.clock.ask.is_none();
    if t.running(now) {
        let id = t.id;
        let hi = (t.buy_out() - omsi_launcher_lib::company::auction::ROUND) as f32 / 100.0;
        let lo = (least as f32 / 100.0).min(hi);
        let view = &mut l.company.tenders;
        if view.amount_for != Some(id) || view.amount < lo {
            view.amount_for = Some(id);
            view.amount = lo;
        }
        let mut a = view.amount.clamp(lo, hi);
        let fmt = |v: f32| eur((v as f64 * 100.0).round() as i64);
        l.ui.slider("company-auction-amount", Rect::new(inner.x, ay, inner.w, ROW), &mut a, lo, hi.max(lo + 100.0), 100.0, "Your bid", &fmt);
        l.company.tenders.amount = a;
        let amount = (a as f64 * 100.0).round() as i64;
        let chance = cn::chance(c, &t, amount);
        let col = if chance >= 0.7 { OK } else if chance >= 0.35 { WARN } else { DANGER.lighten(0.25) };
        let mut s = omsi_ui::tr("Chance to win, as it looks now: about %{p} %").replace("%{p}", &format!("{:.0}", chance * 100.0));
        if !t.fee_paid && cn::fee(c, &t) > 0 {
            s = format!("{s}  ·  {}", omsi_ui::tr("taking part costs %{amount}").replace("%{amount}", &eur(cn::fee(c, &t))));
        }
        l.ui.text_in(&s, Rect::new(inner.x, ay + ROW + 6.0, inner.w, 22.0), kit::NOTE, Weight::Medium, col, Align::Left);
        let by = ay + ROW + 36.0;
        let bw = (inner.w - 8.0) / 2.0;
        let label = omsi_ui::tr("Bid %{amount}").replace("%{amount}", &eur(amount));
        if l.ui.button("company-auction-bid", Rect::new(inner.x, by, bw, 40.0), &label, Some("payments"), ButtonKind::Primary) {
            if !ok {
                super::clock::busy(l);
            } else if act(l, |c| cn::bid(c, id, amount)).is_some() {
                l.state.set_status(omsi_ui::tr("Your bid is in. The rivals answer as the clock goes on."), false);
            }
        }
        let label = omsi_ui::tr("Buy outright, %{amount}").replace("%{amount}", &eur(t.buy_out()));
        if l.ui.button("company-auction-buy", Rect::new(inner.x + bw + 8.0, by, bw, 40.0), &label, Some("check_circle"), ButtonKind::Normal) {
            if !ok {
                super::clock::busy(l);
            } else if act(l, |c| cn::buy_out(c, id)).is_some() {
                l.state.set_status(omsi_ui::tr("Line %{n} is yours: plan its tours and start its service on the Planning page.").replace("%{n}", &t.number), false);
            }
        }
        l.ui.tooltip(Rect::new(inner.x + bw + 8.0, by, bw, 40.0), "Certain: the auction ends at once in your favour");
        let cy = by + 50.0;
        let cw = (inner.w - 8.0) / 2.0;
        if l.ui.button("company-auction-15", Rect::new(inner.x, cy, cw, 36.0), "+15 min", Some("timer"), ButtonKind::Ghost) {
            if ok {
                super::clock::simulate(l, ck::target(c, Step::Minutes(15)), false);
            } else {
                super::clock::busy(l);
            }
        }
        if l.ui.button("company-auction-close", Rect::new(inner.x + cw + 8.0, cy, cw, 36.0), "To its close", Some("sports_score"), ButtonKind::Ghost) {
            if ok {
                super::clock::simulate(l, t.closes_at, false);
            } else {
                super::clock::busy(l);
            }
        }
    } else if t.open() {
        if l.ui.button("company-auction-wait", Rect::new(inner.x, ay + 4.0, inner.w, 40.0), "Simulate until it opens", Some("schedule"), ButtonKind::Normal) {
            if ok {
                super::clock::simulate(l, t.opens_at, false);
            } else {
                super::clock::busy(l);
            }
        }
    } else if let Some(Outcome::Lost { winner, amount, .. } | Outcome::NoBid { winner, amount }) = &t.outcome {
        if !winner.is_empty() {
            let s = omsi_ui::tr("%{who} won the line for %{amount}.").replace("%{who}", winner).replace("%{amount}", &eur(*amount));
            l.ui.text_in(&s, Rect::new(inner.x, ay + 12.0, inner.w, 20.0), kit::ROWS, Weight::Medium, TEXT_SOFT, Align::Left);
        }
    } else if let Some(Outcome::Won { .. }) = &t.outcome {
        let s = match cn::of_line(c, &t.line) {
            Some(h) if dates::between(&c.date, &h.from) > 0 => omsi_ui::tr("The company runs the line from %{date}.").replace("%{date}", &day_label(&h.from)),
            Some(h) => omsi_ui::tr("The concession runs until %{date}.").replace("%{date}", &day_label(&h.until)),
            None => String::new(),
        };
        l.ui.text_in(&s, Rect::new(inner.x, ay + 12.0, inner.w, 20.0), kit::ROWS, Weight::Medium, OK, Align::Left);
    }
}
