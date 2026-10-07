//! A line's fare (the rules are `omsi_launcher_lib::company::fares`'): its single ticket set
//! within the band the line allows, and before it is set what it does - a small chart of the
//! price against the passengers a day and the fares a day, the fare now and the one that
//! brings the most marked, the first days' answer and the one after a few weeks said.

use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{act, eur, eur_cents, grouped, Dialog};
use glam::Vec2;
use omsi_launcher_lib::company::fares as fr;
use omsi_launcher_lib::company::{self as co, Company, CompanyLine};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// A line's passengers a day at the association's fare, from today's tours (None: the
/// timetable is not read yet, or the line has no tour today).
pub(super) fn base_passengers(l: &Launcher, c: &Company, line: &CompanyLine) -> Option<f64> {
    let plan = l.company.plan.as_ref()?;
    let r = co::economy::rules(c.difficulty);
    let mood = 0.85 + 0.3 * c.reputation / 100.0;
    let mut sum = 0.0;
    let mut any = false;
    for t in plan.tours.iter().filter(|t| t.tour.line.eq_ignore_ascii_case(&line.name)) {
        for trip in t.tour.trips.iter().filter(|x| x.counts()) {
            any = true;
            let own = line.plan.as_ref().and_then(|_| co::ownline::trip_boardings(line, &c.date, trip.dep));
            sum += own.unwrap_or_else(|| co::economy::passengers_for(trip.km, trip.dep, &r)) * mood;
        }
    }
    any.then_some(sum)
}

pub(super) fn dialog(l: &mut Launcher) {
    let Some(Dialog::Fare { line, fare }) = &l.company.dialog else { return };
    let (line, fare) = (line.clone(), *fare);
    let Some(c) = l.company.company.clone() else { return };
    let Some(cl) = c.lines.iter().find(|x| x.name == line).cloned() else {
        l.company.dialog = None;
        return;
    };
    let title = omsi_ui::tr("The fare of line %{n}").replace("%{n}", &cl.number);
    let f = kit::frame(l, 860.0, 640.0, "confirmation_number", &title);
    let inner = f.body;
    let (lo, hi) = fr::band(&c, &cl);
    let p0 = fr::association_fare(&c);
    let best = fr::best_fare(&c, &cl);
    let mut y = inner.y;
    let band = if cl.own {
        omsi_ui::tr("Your own line: its fare is yours to set, from %{lo} to %{hi}. The fare association's single ticket is %{p0}.")
    } else {
        omsi_ui::tr("A concession line: the fare association allows %{lo} to %{hi} round its single ticket of %{p0}.")
    }
    .replace("%{lo}", &eur_cents(lo as f64))
    .replace("%{hi}", &eur_cents(hi as f64))
    .replace("%{p0}", &eur_cents(p0 as f64));
    y += l.ui.paragraph(&band, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT) + 14.0;
    // the price
    let mut v = fare.clamp(lo as f32 / 100.0, hi as f32 / 100.0);
    l.ui.slider("company-fare", Rect::new(inner.x, y, inner.w, 40.0), &mut v, lo as f32 / 100.0, hi as f32 / 100.0, 0.1, "Single ticket", &|x| eur_cents((x as f64 * 100.0).round()));
    let cents = ((v as f64 * 10.0).round() * 10.0) as i64;
    y += 54.0;
    // what it does
    let base = base_passengers(l, &c, &cl);
    let chart = Rect::new(inner.x, y, inner.w, 230.0);
    match base {
        Some(base) if base > 0.0 => {
            curves(l, chart, &c, base, (lo, hi), cents, best, p0);
            y = chart.bottom() + 16.0;
            let (pax, rev) = fr::estimate(&c, base, cents);
            let (pax0, rev0) = fr::estimate(&c, base, p0);
            let first = base * fr::short_run_share(fr::offset(&c, cents));
            let change = |a: f64, b: f64| if b > 0.0 { format!("{:+.0} %", (a / b - 1.0) * 100.0) } else { "–".into() };
            let t = omsi_ui::tr("At %{fare}: about %{pax} passengers a day (%{dp} against the association's fare) and %{rev} of fares a day (%{dr}), once they have got used to it - a few weeks. The first days: about %{first} passengers.")
                .replace("%{fare}", &eur_cents(cents as f64))
                .replace("%{pax}", &grouped(pax))
                .replace("%{dp}", &change(pax, pax0))
                .replace("%{rev}", &eur(rev.round() as i64))
                .replace("%{dr}", &change(rev, rev0))
                .replace("%{first}", &grouped(first));
            y += l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT) + 10.0;
        }
        _ => {
            y += l.ui.paragraph("The line has no tour today: what the fare does is shown on a day it runs.", Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT) + 10.0;
        }
    }
    let x = fr::offset(&c, cents);
    if fr::reputation_cost(x) > 0.0 {
        let t = omsi_ui::tr("Far above the association's fare: every day of it costs reputation as well.");
        l.ui.icon("warning", Vec2::new(inner.x + 10.0, y + 11.0), 20.0, WARN);
        y += l.ui.paragraph(&t, Vec2::new(inner.x + 30.0, y), inner.w - 30.0, kit::BODY, Weight::Medium, WARN) + 8.0;
    }
    let share = cl.demand.share();
    if (share - 1.0).abs() > 0.005 {
        let t = omsi_ui::tr("Its passengers now: %{p} of what the association's fare would bring.").replace("%{p}", &format!("{:.0} %", share * 100.0));
        l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, kit::NOTE, Weight::Regular, TEXT_SOFT);
    }
    let mut foot = Foot::new(&f);
    let set_label = omsi_ui::tr("Set %{fare}").replace("%{fare}", &eur_cents(cents as f64));
    let set = foot.right(l, "company-fare-set", &set_label, Some("check_circle"), ButtonKind::Primary);
    if foot.right(l, "company-fare-cancel", "Cancel", None, ButtonKind::Normal) || f.close {
        l.company.dialog = None;
        return;
    }
    let mut v = v;
    if foot.left(l, "company-fare-p0", "The association's fare", None, ButtonKind::Ghost) {
        v = p0 as f32 / 100.0;
    }
    if foot.left(l, "company-fare-best", "The most fares", None, ButtonKind::Ghost) {
        v = best as f32 / 100.0;
    }
    if set {
        let name = cl.name.clone();
        if let Some(done) = act(l, |c| fr::set_fare(c, &name, cents)) {
            l.company.dialog = None;
            l.state.set_status(omsi_ui::tr("Line %{n}: the single ticket costs %{fare} from now on.").replace("%{n}", &cl.number).replace("%{fare}", &eur_cents(done as f64)), false);
            return;
        }
    }
    l.company.dialog = Some(Dialog::Fare { line, fare: v });
}

/// The chart: the price across the band, the passengers a day (blue) and the fares a day
/// (green) in the long run, the fare chosen and the one that brings the most marked.
#[allow(clippy::too_many_arguments)]
fn curves(l: &mut Launcher, r: Rect, c: &Company, base: f64, (lo, hi): (i64, i64), now: i64, best: i64, p0: i64) {
    l.ui.card(r);
    let plot = Rect::new(r.x + 70.0, r.y + 52.0, r.w - 140.0, r.h - 86.0);
    let n = 48;
    let at = |k: usize| lo + ((hi - lo) as f64 * k as f64 / n as f64).round() as i64;
    let pts: Vec<(i64, f64, f64)> = (0..=n).map(|k| {
        let p = at(k);
        let (pax, rev) = fr::estimate(c, base, p);
        (p, pax, rev)
    }).collect();
    let max_pax = pts.iter().map(|x| x.1).fold(1.0, f64::max) * 1.08;
    let max_rev = pts.iter().map(|x| x.2).fold(1.0, f64::max) * 1.08;
    let x_of = |p: i64| plot.x + plot.w * ((p - lo) as f32 / (hi - lo).max(1) as f32);
    let pax_c = Color::rgba(122, 168, 255, 1.0);
    let rev_c = OK;
    for k in 0..=4 {
        let y = plot.y + plot.h * k as f32 / 4.0;
        l.ui.p().rect(Rect::new(plot.x, y, plot.w, 1.0), Color::WHITE.alpha(if k == 4 { 0.2 } else { 0.06 }));
        let f = 1.0 - k as f64 / 4.0;
        l.ui.text_in(&grouped(max_pax * f), Rect::new(r.x + 6.0, y - 9.0, 58.0, 18.0), 12.5, Weight::Regular, pax_c, Align::Right);
        l.ui.text_in(&eur((max_rev * f).round() as i64), Rect::new(plot.right() + 6.0, y - 9.0, 64.0, 18.0), 12.5, Weight::Regular, rev_c, Align::Left);
    }
    let line = |l: &mut Launcher, f: &dyn Fn(&(i64, f64, f64)) -> f64, max: f64, col: Color| {
        let ps: Vec<Vec2> = pts.iter().map(|x| Vec2::new(x_of(x.0), plot.bottom() - plot.h * (f(x) / max) as f32)).collect();
        l.ui.p().stroke(&ps, 2.5, col);
    };
    line(l, &|x| x.1, max_pax, pax_c);
    line(l, &|x| x.2, max_rev, rev_c);
    // the marks: the association's fare, the most fares, the one chosen
    for (p, label, col) in [(p0, omsi_ui::tr("Association"), TEXT_DIM), (best, omsi_ui::tr("Most fares"), rev_c)] {
        let x = x_of(p);
        let mut y = plot.y;
        while y < plot.bottom() {
            l.ui.p().rect(Rect::new(x, y, 1.0, 5.0), col.alpha(0.7));
            y += 9.0;
        }
        // (its words beside the mark, inside the plot: to the right of it, or to its left at the end)
        let lw = l.ui.width(&label, 12.5, Weight::Bold);
        let lx = if x + 8.0 + lw > plot.right() { x - 8.0 - lw } else { x + 8.0 };
        l.ui.text_in(&label, Rect::new(lx, plot.y + 2.0, lw + 4.0, 18.0), 12.5, Weight::Bold, col, Align::Left);
    }
    let (pax, rev) = fr::estimate(c, base, now);
    let x = x_of(now);
    l.ui.p().rect(Rect::new(x - 1.0, plot.y, 2.0, plot.h), accent().alpha(0.8));
    l.ui.p().circle(Vec2::new(x, plot.bottom() - plot.h * (pax / max_pax) as f32), 5.5, pax_c);
    l.ui.p().circle(Vec2::new(x, plot.bottom() - plot.h * (rev / max_rev) as f32), 5.5, rev_c);
    // the axis
    l.ui.text_in(&eur_cents(lo as f64), Rect::new(plot.x - 40.0, plot.bottom() + 8.0, 80.0, 18.0), 12.5, Weight::Regular, TEXT_DIM, Align::Center);
    l.ui.text_in(&eur_cents(hi as f64), Rect::new(plot.right() - 40.0, plot.bottom() + 8.0, 80.0, 18.0), 12.5, Weight::Regular, TEXT_DIM, Align::Center);
    l.ui.text_in(&eur_cents(now as f64), Rect::new(x - 40.0, plot.bottom() + 8.0, 80.0, 18.0), 13.0, Weight::Bold, accent(), Align::Center);
    // the legend over the plot: what each line is
    let mut lx = r.x + 16.0;
    for (t, col) in [(omsi_ui::tr("Passengers a day"), pax_c), (omsi_ui::tr("Fares a day"), rev_c)] {
        l.ui.p().rounded(Rect::new(lx, r.y + 17.0, 16.0, 4.0), 2.0, col);
        let w = l.ui.width(&t, 13.0, Weight::Bold);
        l.ui.text_in(&t, Rect::new(lx + 22.0, r.y + 10.0, w + 4.0, 18.0), 13.0, Weight::Bold, col, Align::Left);
        lx += 22.0 + w + 26.0;
    }
}

/// The Finances page's card of the fares: every line's single ticket, how its passengers take
/// it, and the way to change it.
pub(super) fn card(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = super::section(&mut l.ui, r, "Ticket prices");
    if c.lines.is_empty() {
        l.ui.paragraph("The company runs no line yet: each line's single ticket is set here and on the Lines page.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let p0 = fr::association_fare(c);
    let lines = c.lines.clone();
    let cc = c.clone();
    let mut open: Option<String> = None;
    l.ui.scroll_area("company-fares", inner, &mut |ui, v| {
        let rh = 44.0;
        for (k, cl) in lines.iter().enumerate() {
            let row = Rect::new(v.x, v.y + k as f32 * rh, v.w - 8.0, rh - 4.0);
            if ui.row(&format!("company-fares-{}", cl.name), row, false) {
                open = Some(cl.name.clone());
            }
            let w = super::line_plate(ui, Vec2::new(row.x + 6.0, row.y + 9.0), cl, 22.0);
            let fare = fr::fare_of(&cc, cl);
            ui.text_in(&eur_cents(fare as f64), Rect::new(row.x + w + 18.0, row.y, 90.0, row.h), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let rel = fare as f64 / p0.max(1) as f64 - 1.0;
            let vs = if rel.abs() < 0.005 { omsi_ui::tr("the association's").into_owned() } else { format!("{:+.0} %", rel * 100.0) };
            ui.text_in(&vs, Rect::new(row.x + w + 110.0, row.y, 130.0, row.h), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            let share = cl.demand.share();
            let (t, col) = if (share - 1.0).abs() < 0.005 { (omsi_ui::tr("passengers as usual").into_owned(), TEXT_DIM) } else { (omsi_ui::tr("%{p} passengers").replace("%{p}", &format!("{:+.0} %", (share - 1.0) * 100.0)), if share > 1.0 { OK } else { WARN }) };
            ui.text_in(&t, Rect::new(row.right() - 210.0, row.y, 180.0, row.h), kit::NOTE, Weight::Medium, col, Align::Right);
            ui.icon("chevron_right", Vec2::new(row.right() - 12.0, row.center().y), 18.0, TEXT_DIM);
            ui.tooltip(row, &omsi_ui::tr("Change the fare: see what it does before it is set"));
        }
        lines.len() as f32 * rh
    });
    if let Some(name) = open {
        let f = c.lines.iter().find(|x| x.name == name).map(|x| fr::fare_of(c, x)).unwrap_or(p0);
        l.company.dialog = Some(Dialog::Fare { line: name, fare: f as f32 / 100.0 });
    }
}
