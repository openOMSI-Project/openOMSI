//! The company's overview: its figures (cash, the month's result, fleet, staff, punctuality,
//! reputation), the last thirty days in a chart, today's tours and what wants attention, and
//! the last day closed.

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit;
use super::{day_label, eur, grouped, section};
use glam::Vec2;
use omsi_launcher_lib::company::{self as co, Alert, Company, DayRecord};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let gap = 14.0;
    // the figures
    let month = c.month(&co::dates::month_of(&c.date));
    let ready = c.fleet.iter().filter(|v| v.held_on(&c.date) && !v.in_workshop(&c.date)).count();
    let here = c.staff.iter().filter(|e| e.employed_on(&c.date) && !e.absent(&c.date)).count();
    // (punctuality only once a trip was run and timed)
    let timed = c.history.iter().any(|d| d.punctuality.is_some());
    let fw = (area.w - 5.0 * gap) / 6.0;
    let fh = kit::FIGURE_H;
    let figures: [(&str, String, String, Color, &str); 6] = [
        ("Cash", eur(c.cash), if c.debt() > 0 { omsi_ui::tr("Loans: %{amount}").replace("%{amount}", &eur(c.debt())) } else { String::new() }, if c.cash >= 0 { TEXT } else { DANGER.lighten(0.2) }, "The money the company has now"),
        ("This month", kit::signed(month.result()).0, super::month_label(&month.month), if month.result() >= 0 { OK } else { DANGER.lighten(0.2) }, "What running the lines brought this month, less what it cost (buying and loans apart)"),
        ("Fleet", c.fleet.len().to_string(), omsi_ui::tr("%{n} ready today").replace("%{n}", &ready.to_string()), TEXT, "Buses of the company, and how many are not in the workshop today"),
        ("Staff", c.staff.len().to_string(), omsi_ui::tr("%{n} at work today").replace("%{n}", &here.to_string()), TEXT, "People on the payroll, and how many are not ill or on holiday today"),
        ("On time", if timed { format!("{:.0} %", c.punctuality) } else { "–".to_string() }, omsi_ui::tr("of the trips, lately").into_owned(), if timed { super::grade(c.punctuality * 1.2 - 20.0) } else { TEXT }, "Trips at most three minutes late, over the last days"),
        ("Reputation", format!("{:.0}", c.reputation), omsi_ui::tr("of 100").into_owned(), super::grade(c.reputation + 15.0), "What passengers and the authority think of the company: trips run on time raise it, trips dropped lower it"),
    ];
    super::super::tour::anchor("company-figures", Rect::new(area.x, area.y, area.w, fh));
    for (k, (label, value, under, colour, tip)) in figures.iter().enumerate() {
        let r = Rect::new(area.x + k as f32 * (fw + gap), area.y, fw, fh);
        super::figure(&mut l.ui, r, label, value, under, *colour);
        l.ui.tooltip(r, tip);
    }
    let y = area.y + fh + gap;
    let left_w = (area.w - gap) * 0.62;
    let right_w = area.w - gap - left_w;
    let h1 = ((area.bottom() - y - gap) * 0.52).max(220.0);
    // the last thirty days
    let span = co::dates::parse(&c.date).map_or(30, |t| chart_span(&c.history, t));
    let title = omsi_ui::tr("The last %{n} days").replace("%{n}", &span.to_string());
    let chart = section(&mut l.ui, Rect::new(area.x, y, left_w, h1), &title);
    chart_of(&mut l.ui, chart, &c.history, &c.date);
    // today
    today(l, Rect::new(area.x + left_w + gap, y, right_w, h1), &c);
    // the company's day as it went, the last day closed, and the companies
    let y2 = y + h1 + gap;
    let h2 = (area.bottom() - y2).max(140.0);
    let mut minor = l.company.feed_minor;
    super::clock::feed(l, Rect::new(area.x, y2, left_w, h2), &c, &mut minor);
    l.company.feed_minor = minor;
    let h3 = (h2 * 0.6).max(170.0);
    last_day(l, Rect::new(area.x + left_w + gap, y2, right_w, h3), &c);
    companies(l, Rect::new(area.x + left_w + gap, y2 + h3 + gap, right_w, (h2 - h3 - gap).max(80.0)), &c);
}

/// "12 Oct": a date short, for an axis.
fn short_date(date: &str) -> String {
    let Some(d) = co::dates::parse(date) else { return date.to_string() };
    let (_, m, day) = co::dates::civil_from_days(d);
    format!("{} {}", day, omsi_ui::tr(super::MONTHS[(m as usize).clamp(1, 12) - 1]))
}

/// A round step for an axis of `span` with about `n` steps (1, 2 or 5 times a power of ten).
pub(super) fn nice_step(span: f64, n: f64) -> f64 {
    let raw = (span / n).max(1.0);
    let p = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|k| k * p).find(|s| *s >= raw).unwrap_or(10.0 * p)
}

/// Which of `n` day slots get a date under them: every `every`-th counted back from the last,
/// so that labels `label_w` wide never overlap in slots `slot` wide.
pub(super) fn tick_every(slot: f32, label_w: f32) -> usize {
    ((label_w + 12.0) / slot.max(1.0)).ceil().max(1.0) as usize
}

/// How many days the chart shows up to yesterday: thirty, or - for a young company - the days
/// since its first closed day, at least a week, so that a few bars are not lost in a month.
pub(super) fn chart_span(history: &[DayRecord], today: i64) -> i64 {
    let oldest = history.iter().filter_map(|d| co::dates::parse(&d.date)).filter(|x| *x < today).min();
    oldest.map_or(30, |o| (today - o).clamp(7, 30))
}

/// The days' results as bars around the zero line (green above, red under) on the last thirty
/// days ending yesterday - each bar at its own day, a value scale on the left, short dates
/// under that never overlap, the full date in the tooltip.
fn chart_of(ui: &mut Ui, r: Rect, history: &[DayRecord], today: &str) {
    let Some(t) = co::dates::parse(today) else { return };
    let span = chart_span(history, t);
    let first = t - span;
    let days: Vec<&DayRecord> = history.iter().filter(|d| co::dates::parse(&d.date).is_some_and(|x| x >= first && x < t)).collect();
    if days.is_empty() {
        let c = r.center();
        ui.icon("signal_cellular_alt", Vec2::new(c.x, c.y - 30.0), 34.0, TEXT_DIM);
        ui.text_in("No day closed yet", Rect::new(r.x, c.y - 4.0, r.w, 24.0), kit::HEAD, Weight::Bold, TEXT_SOFT, Align::Center);
        ui.text_in("Each closed day's result appears here as a bar: green for a profit, red for a loss.", Rect::new(r.x, c.y + 22.0, r.w, 22.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Center);
        return;
    }
    let hi = days.iter().map(|d| d.result).max().unwrap_or(0).max(0) as f64;
    let lo = days.iter().map(|d| d.result).min().unwrap_or(0).min(0) as f64;
    let step = nice_step((hi - lo).max(100_00.0), 4.0);
    let top = (hi / step).ceil() * step;
    let bottom = (lo / step).floor() * step;
    let (top, bottom) = if top == bottom { (step, 0.0) } else { (top, bottom) };
    let label_w = [top, bottom].iter().map(|v| ui.width(&eur(*v as i64), kit::NOTE, Weight::Regular)).fold(0.0, f32::max) + 14.0;
    let plot = Rect::new(r.x + label_w, r.y + 8.0, r.w - label_w - 4.0, r.h - 40.0);
    let y_of = |v: f64| plot.y + plot.h * ((top - v) / (top - bottom)) as f32;
    // the scale
    let mut v = bottom;
    while v <= top + 1.0 {
        let y = y_of(v);
        ui.p().rect(Rect::new(plot.x, y, plot.w, 1.0), Color::WHITE.alpha(if v.abs() < 1.0 { 0.22 } else { 0.06 }));
        ui.text_in(&eur(v as i64), Rect::new(r.x, y - 9.0, label_w - 10.0, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Right);
        v += step;
    }
    // the days, each in its slot
    let slot = plot.w / span as f32;
    let bw = (slot * 0.64).clamp(2.0, 56.0);
    let zero = y_of(0.0);
    for d in &days {
        let Some(x) = co::dates::parse(&d.date) else { continue };
        let k = (x - first) as f32;
        let cell = Rect::new(plot.x + k * slot, plot.y, slot, plot.h);
        let bx = cell.x + (slot - bw) * 0.5;
        let y = y_of(d.result as f64);
        let bar = if d.result >= 0 { Rect::new(bx, y, bw, (zero - y).max(1.5)) } else { Rect::new(bx, zero, bw, (y - zero).max(1.5)) };
        let hover = ui.hover(cell);
        let col = if d.result >= 0 { OK } else { DANGER };
        ui.p().rounded(bar, 2.0f32.min(bw * 0.4), if hover { col.lighten(0.25) } else { col.alpha(0.85) });
        if hover {
            let text = format!("{}\n{}  ·  {}", day_label(&d.date), kit::signed(d.result).0, omsi_ui::tr("%{c} of %{t} tours").replace("%{c}", &(d.tours - d.dropped_tours).to_string()).replace("%{t}", &d.tours.to_string()));
            ui.tooltip(cell, &text);
        }
    }
    // the dates: every so many days counted back from yesterday, never on top of each other
    let sample = short_date(&co::dates::fmt(t - 1));
    let every = tick_every(slot, ui.width(&sample, kit::NOTE, Weight::Regular));
    for k in (0..span).rev().step_by(every) {
        let x = plot.x + (k as f32 + 0.5) * slot;
        let label = short_date(&co::dates::fmt(first + k as i64));
        ui.p().rect(Rect::new(x, plot.bottom(), 1.0, 4.0), Color::WHITE.alpha(0.2));
        ui.text_in(&label, Rect::new(x - 60.0, plot.bottom() + 8.0, 120.0, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Center);
    }
}

/// Today: the tours and how many are covered, and what wants attention.
fn today(l: &mut Launcher, r: Rect, c: &Company) {
    super::super::tour::anchor("company-today", r);
    let inner = section(&mut l.ui, r, "Today");
    let mut y = inner.y;
    let plan = l.company.plan.clone();
    match &plan {
        Some(p) if p.tours.iter().any(|t| !t.tour.unplanned) => {
            let n = p.tours.iter().filter(|t| !t.tour.unplanned).count();
            let covered = n - p.uncovered();
            let frac = covered as f64 / n as f64;
            let label = omsi_ui::tr("Covered today: %{c} of %{t} tours").replace("%{c}", &covered.to_string()).replace("%{t}", &n.to_string());
            let (buses, duties) = p.short_of();
            let tip = if covered == n { omsi_ui::tr("Every tour in service has a bus and its drivers.").into_owned() } else { omsi_ui::tr("Open: %{b} tours without a bus, %{d} duties without a driver. Click for the planning.").replace("%{b}", &buses.to_string()).replace("%{d}", &duties.to_string()) };
            if kit::bar(&mut l.ui, "company-today-bar", Rect::new(inner.x, y, inner.w, kit::BAR_H), frac, kit::share_colour(frac, false), &label, &format!("{:.0} %", frac * 100.0), &tip) {
                l.company.tab = 5;
            }
            y += kit::BAR_H + 18.0;
        }
        Some(_) => {
            l.ui.text_in("No tours run today.", Rect::new(inner.x, y, inner.w, 24.0), kit::BODY, Weight::Medium, TEXT_SOFT, Align::Left);
            y += 34.0;
        }
        None => {
            let t = if l.company.today.as_ref().is_some_and(|t| t.error.is_some()) { "The map's timetable could not be read." } else { "Reading the timetable…" };
            l.ui.text_in(t, Rect::new(inner.x, y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 34.0;
        }
    }
    // the player's own next duty, a click from driving it (`career::my_duties`)
    let (plans, mine, whole) = super::career::my_duties(l, c);
    let row = Rect::new(inner.x, y, inner.w, 38.0);
    match mine.first() {
        Some(d) => {
            let bw = 104.0;
            let when = format!("{} {}", super::day_label(&d.date), co::clock::hhmm(d.from as i64));
            let text = omsi_ui::tr("Your next duty: %{when}, line %{n}, tour %{t}").replace("%{when}", &when).replace("%{n}", &d.number).replace("%{t}", &d.tour_no);
            // (the words open all of them: the Career tab's "My duties")
            let words = Rect::new(row.x - 6.0, row.y, row.w - bw - 24.0, row.h);
            if l.ui.row("company-my-duties", words, false) {
                l.company.tab = 6;
                l.company.career.part = super::career::DUTIES;
            }
            l.ui.tooltip(words, "All your duties: Career, My duties");
            l.ui.icon("person", Vec2::new(row.x + 9.0, row.center().y), 18.0, accent());
            l.ui.text_in(&text, Rect::new(row.x + 28.0, row.y, row.w - bw - 36.0, row.h), kit::ROWS, Weight::Medium, TEXT, Align::Left);
            if l.ui.button("company-next-duty", Rect::new(row.right() - bw, row.y + 2.0, bw, 34.0), "Drive", Some("play_arrow"), ButtonKind::Primary) {
                let d = d.clone();
                super::career::drive_duty(l, c, &plans, &d);
                return;
            }
            y += 46.0;
        }
        None if whole => {
            let bw = kit::Foot::width(&l.ui, "Plan a duty", Some("event"));
            l.ui.icon("person", Vec2::new(row.x + 9.0, row.center().y), 18.0, TEXT_DIM);
            l.ui.text_in("No duty of yours is planned.", Rect::new(row.x + 28.0, row.y, row.w - bw - 36.0, row.h), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Left);
            if l.ui.button("company-plan-duty", Rect::new(row.right() - bw, row.y + 2.0, bw, 34.0), "Plan a duty", Some("event"), ButtonKind::Normal) {
                l.company.tab = 5;
            }
            y += 46.0;
        }
        None => {}
    }
    let mut alerts = co::alerts(c, plan.as_ref());
    if let Some(a) = super::planning::tomorrow(l, c).as_ref().and_then(co::tomorrow_alert) {
        alerts.push(a);
    }
    if alerts.is_empty() {
        l.ui.text_in("Nothing wants your attention.", Rect::new(inner.x, y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    }
    let row_h = 38.0;
    for (k, a) in alerts.iter().enumerate() {
        if y + row_h > inner.bottom() + 8.0 {
            break;
        }
        let (icon, text, colour, tab) = alert_text(a);
        let row = Rect::new(inner.x - 8.0, y, inner.w + 16.0, row_h);
        if l.ui.row(&format!("company-alert-{k}"), row, false) {
            l.company.tab = tab;
        }
        l.ui.icon(icon, Vec2::new(inner.x + 9.0, y + row_h * 0.5), 18.0, colour);
        l.ui.text_in(&text, Rect::new(inner.x + 30.0, y, inner.w - 46.0, row_h), kit::ROWS, Weight::Regular, TEXT, Align::Left);
        l.ui.icon("chevron_right", Vec2::new(inner.right() - 4.0, y + row_h * 0.5), 18.0, TEXT_DIM);
        l.ui.tooltip(row, &text);
        y += row_h + 2.0;
    }
}

/// An alert's icon, words, colour and the tab that helps.
fn alert_text(a: &Alert) -> (&'static str, String, Color, usize) {
    match a {
        Alert::NoLines => ("route", omsi_ui::tr("The company runs no line yet: add one on the Lines page.").into_owned(), WARN, 3),
        Alert::NoBuses => ("directions_bus", omsi_ui::tr("There is no bus in the fleet: buy, lease or rent one.").into_owned(), WARN, 1),
        Alert::NoDrivers => ("groups", omsi_ui::tr("Nobody works here yet: hire drivers on the Staff page.").into_owned(), WARN, 2),
        Alert::Uncovered { tours, buses, duties } => {
            let mut t = omsi_ui::tr("%{n} tours are not covered today").replace("%{n}", &tours.to_string());
            if *buses > 0 {
                t.push_str(&omsi_ui::tr(": %{n} without a bus").replace("%{n}", &buses.to_string()));
            }
            if *duties > 0 {
                t.push_str(&omsi_ui::tr(", %{n} duties without a driver").replace("%{n}", &duties.to_string()));
            }
            ("warning", t, DANGER.lighten(0.2), 5)
        }
        Alert::NotPlanned(lines) => ("event", omsi_ui::tr("Not in service yet: line %{n}. Plan its tours and start its service.").replace("%{n}", &lines.join(", ")), WARN, 5),
        Alert::Tomorrow { tours, buses, duties } => (
            "event",
            omsi_ui::tr("Tomorrow %{n} tours are not covered: %{b} without a bus, %{d} duties without a driver.").replace("%{n}", &tours.to_string()).replace("%{b}", &buses.to_string()).replace("%{d}", &duties.to_string()),
            WARN,
            5,
        ),
        Alert::LowCash => ("payments", omsi_ui::tr("Cash is low: less than a month's wages.").into_owned(), DANGER.lighten(0.2), 4),
        Alert::ServiceDue(n) => ("construction", omsi_ui::tr("%{n} buses are due for their service.").replace("%{n}", &n.to_string()), WARN, 1),
        Alert::Unhappy(n) => ("person", omsi_ui::tr("%{n} employees are unhappy and may leave.").replace("%{n}", &n.to_string()), WARN, 2),
        Alert::GoingBack { number, until } => ("event", omsi_ui::tr("Bus %{n} goes back on %{date}.").replace("%{n}", number).replace("%{date}", &day_label(until)), TEXT_SOFT, 1),
    }
}

/// The last day closed, in a few words, and the way to its report.
fn last_day(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "The last day closed");
    let Some(rep) = c.last_report.clone() else {
        l.ui.paragraph("\"Simulate to tomorrow\" at the top runs the rest of the day and closes it at midnight: the tours are run, the money booked, and the next day begins. What you drove yourself on the company's lines counts as measured.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    };
    l.ui.text_in(&day_label(&rep.date), Rect::new(inner.x, inner.y, inner.w * 0.55, 26.0), kit::HEAD, Weight::Bold, TEXT, Align::Left);
    let (res, col) = kit::signed(rep.result);
    l.ui.text_in(&res, Rect::new(inner.x + inner.w * 0.45, inner.y, inner.w * 0.55, 26.0), 19.0, Weight::Bold, col, Align::Right);
    l.ui.tooltip(Rect::new(inner.x + inner.w * 0.45, inner.y, inner.w * 0.55, 26.0), &format!("{} {}  ·  {} {}", kit::signed(rep.income).0, omsi_ui::tr("in"), kit::signed(-rep.expenses).0, omsi_ui::tr("out")));
    let facts = [
        omsi_ui::tr("%{c} of %{t} tours").replace("%{c}", &rep.covered.to_string()).replace("%{t}", &rep.tours.to_string()),
        omsi_ui::tr("%{n} trips dropped").replace("%{n}", &rep.dropped.to_string()),
        omsi_ui::tr("%{n} passengers").replace("%{n}", &grouped(rep.passengers as f64)),
        format!("{} km", grouped(rep.km.round())),
    ];
    // (two by two: the column is narrow; the button under them, or beside them when the card is
    // low)
    let bw = kit::Foot::width(&l.ui, "Show the report", Some("receipt_long"));
    let beside = inner.h < 132.0;
    let fw = if beside { (inner.w - bw - 16.0) / 2.0 } else { inner.w / 2.0 };
    for (k, f) in facts.iter().enumerate() {
        l.ui.text_in(f, Rect::new(inner.x + (k % 2) as f32 * fw, inner.y + 34.0 + (k / 2) as f32 * 24.0, fw - 8.0, 22.0), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Left);
    }
    let at = if beside { Rect::new(inner.right() - bw, inner.y + 38.0, bw, 38.0) } else { Rect::new(inner.x, inner.y + 92.0, bw, 38.0) };
    if l.ui.button("company-show-report", at, "Show the report", Some("receipt_long"), ButtonKind::Normal) {
        l.company.reports = Some(vec![rep]);
    }
}

/// The driver's companies: which one is open, and founding another.
fn companies(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Your companies");
    let list = l.company.companies.clone().unwrap_or_default();
    let bw = kit::Foot::width(&l.ui, "Found another company", Some("add")).min(inner.w * 0.5);
    let y = inner.y - 4.0;
    if list.len() > 1 {
        let names: Vec<String> = list.iter().map(|x| format!("{}  ·  {}", x.name, x.map_name)).collect();
        let mut k = list.iter().position(|x| x.id == c.id).unwrap_or(0);
        if l.ui.select("company-pick", Rect::new(inner.x, y, (inner.w - bw - 12.0).max(80.0), 38.0), &mut k, &names) {
            l.company.company = list.get(k).cloned();
            l.company.plan = None;
        }
    } else {
        let text = omsi_ui::tr("Founded on %{date}.").replace("%{date}", &day_label(&c.founded));
        l.ui.text_in(&text, Rect::new(inner.x, y, (inner.w - bw - 12.0).max(80.0), 38.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
    }
    if l.ui.button("company-found-another", Rect::new(inner.right() - bw, y, bw, 38.0), "Found another company", Some("add"), ButtonKind::Normal) {
        l.company.wizard = Some(super::wizard::Wizard::new(l));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chart_has_round_steps_and_dates_that_do_not_meet() {
        assert_eq!(nice_step(1_000_00.0, 4.0), 500_00.0);
        assert_eq!(nice_step(12_345_00.0, 4.0), 500_000.0);
        assert_eq!(nice_step(3.0, 4.0), 1.0);
        // slots of 20 px and labels 50 px wide: a date every fourth day
        assert_eq!(tick_every(20.0, 50.0), 4);
        assert_eq!(tick_every(80.0, 50.0), 1);
        assert_eq!(short_date("2005-10-12"), "12 Oct");
    }

    #[test]
    fn a_young_company_sees_its_few_days_wide() {
        let day = |d: &str| DayRecord { date: d.into(), ..Default::default() };
        let t = co::dates::parse("2024-03-05").unwrap();
        assert_eq!(chart_span(&[], t), 30);
        assert_eq!(chart_span(&[day("2024-03-04")], t), 7);
        assert_eq!(chart_span(&[day("2024-02-20"), day("2024-03-04")], t), 14);
        assert_eq!(chart_span(&[day("2023-12-01"), day("2024-03-04")], t), 30);
    }
}
