//! The company's Career tab (the rules are `omsi_launcher_lib::company`'s `career`,
//! `levels`, `training` and `rankings`): the player's own duties of the week, each a click
//! from driving it ("My duties", `plan::my_duties`); the driver's level, rank and licences with the
//! driving test, the company's level and what it opens; the training courses of the staff
//! and of the player; the workshop's jobs the player can do himself (`repair_game`); the
//! rankings among the map's other companies and drivers; and the statistics of the player's
//! trips and the company's tours.

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit;
use super::{act, day_label, eur, figure, grouped, section};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::career::{self as dc, DriverCareer, LicenceClass};
use omsi_launcher_lib::company::plan::{BusOf, DayPlan, MyDuty};
use omsi_launcher_lib::company as co;
use omsi_launcher_lib::company::levels::{self, Feature};
use omsi_launcher_lib::company::rankings::{self, Entry};
use omsi_launcher_lib::company::training::{self, CourseKind, JobKind};
use omsi_launcher_lib::company::{BusSize, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

pub(super) const PARTS: [&str; 6] = ["My duties", "Progress", "Training", "Workshop", "Rankings", "Statistics"];
/// The parts by their place in `PARTS`.
pub(super) const DUTIES: usize = 0;
pub(super) const PROGRESS: usize = 1;
pub(super) const TRAINING: usize = 2;
pub(super) const WORKSHOP: usize = 3;

#[derive(Default)]
pub struct CareerView {
    pub(super) part: usize,
    driver: Option<Driver>,
    pub(super) game: Option<super::repair_game::Game>,
}

/// The driver's career as read: whose, the career file, the trips and what they come to.
struct Driver {
    name: String,
    career: DriverCareer,
    trips: Vec<core::TripRun>,
    summary: dc::Summary,
    /// When it was read (the launcher's clock).
    read: f32,
}

/// The size of a bus file: the market's kind, the fleet's, else a guess from its name.
fn size_of(l: &Launcher, bus: &str) -> BusSize {
    let same = |a: &str| a.replace('\\', "/").eq_ignore_ascii_case(&bus.replace('\\', "/"));
    if let Some(b) = l.company.market.as_ref().and_then(|m| m.iter().find(|b| same(&b.file))) {
        return b.kind.size;
    }
    if let Some(v) = l.company.company.as_ref().and_then(|c| c.fleet.iter().find(|v| same(&v.bus))) {
        return v.kind.size;
    }
    core::company::market::guess_kind(&[bus], &[], false, None, None).size
}

/// The driver's career, read again when the driver changed or after a while (a game may
/// have added trips); a driving test booked is judged when its trip is there.
fn driver(l: &mut Launcher) -> Option<&Driver> {
    let name = l.state.config.profile.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let stale = l.company.career.driver.as_ref().is_none_or(|d| d.name != name || l.ui.time - d.read > 8.0 || l.ui.time < d.read);
    if stale {
        let data = core::data_dir();
        let trips = core::trips_of(&data, &name);
        let mut career = dc::load(&data, &name);
        let sizes: Vec<(String, BusSize)> = trips.iter().map(|t| (t.bus.clone(), size_of(l, &t.bus))).collect();
        let size = |bus: &str| sizes.iter().find(|x| x.0 == bus).map(|x| x.1).unwrap_or(BusSize::Solo);
        if let Some(exam) = dc::check_exam(&mut career, &trips, &size) {
            let _ = dc::save(&data, &career);
            let text = if exam.passed { "You passed the driving test: the %{class} licence is yours." } else { "You failed the driving test. Book it again when you are ready." };
            l.state.set_status(omsi_ui::tr(text).replace("%{class}", &omsi_ui::tr(exam.class.label())), !exam.passed);
        }
        let summary = dc::summary(&trips, &career);
        l.company.career.driver = Some(Driver { name, career, trips, summary, read: l.ui.time });
    }
    l.company.career.driver.as_ref()
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let labels: Vec<String> = PARTS.iter().map(|t| omsi_ui::tr(t).into_owned()).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut part = l.company.career.part;
    let seg_w = 740.0f32.min(area.w);
    super::super::tour::anchor("company-career-parts", Rect::new(area.x, area.y, seg_w, ROW));
    if l.ui.segmented("company-career-parts", Rect::new(area.x, area.y, seg_w, ROW), &mut part, &refs) {
        l.company.career.part = part;
    }
    // the company's level beside
    let lv = levels::level(&c);
    let tag = format!("{}  ·  {}", omsi_ui::tr("Level %{n}").replace("%{n}", &lv.to_string()), omsi_ui::tr(levels::title_of(lv)));
    l.ui.text_in(&tag, Rect::new(area.x + seg_w + 20.0, area.y, (area.w - seg_w - 20.0).max(0.0), ROW), kit::ROWS, Weight::Medium, TEXT_DIM, Align::Right);
    let body = Rect::new(area.x, area.y + ROW + 16.0, area.w, (area.h - ROW - 16.0).max(0.0));
    if l.company.career.part == WORKSHOP && l.company.career.game.is_some() {
        super::repair_game::draw(l, body);
        return;
    }
    match l.company.career.part {
        PROGRESS => progress_part(l, body, &c),
        TRAINING => training_part(l, body, &c),
        WORKSHOP => workshop_part(l, body, &c),
        4 => rankings_part(l, body, &c),
        5 => statistics_part(l, body, &c),
        _ => duties_part(l, body, &c),
    }
}

// --- my duties ----------------------------------------------------------------------------------

/// The player's own duties of the company's next seven days (`plan::my_duties`, from the
/// planning's plans of those days): the plans read so far, the duties - the next first -, and
/// whether the whole week is read.
pub(super) fn my_duties(l: &mut Launcher, c: &Company) -> (Vec<DayPlan>, Vec<MyDuty>, bool) {
    super::planning::work(l, c);
    let mut plans = Vec::new();
    let mut whole = true;
    for k in 0..7 {
        let date = co::dates::add(&c.date, k);
        match super::planning::plan_of(l, c, &date) {
            Some(Ok(p)) => plans.push(p),
            Some(Err(_)) => {}
            None => whole = false,
        }
    }
    // (minutes since the company's day began: a duty over by then is done with)
    let now = (co::clock::now(c) - co::clock::moment(&c.date, 0)).clamp(0, 48 * 60) as i32;
    let duties = co::plan::my_duties(&plans, &c.date, now);
    (plans, duties, whole)
}

/// Drive one of the player's duties: the Drive page set to exactly it (`planning::drive`).
pub(super) fn drive_duty(l: &mut Launcher, c: &Company, plans: &[DayPlan], d: &MyDuty) {
    if let Some(p) = plans.iter().find(|p| p.date == d.date) {
        super::planning::drive(l, c, p, d.tour, d.duty);
    }
}

/// What a duty is driven with: the fleet bus planned (its number and name), a rental bus, or
/// none yet.
fn bus_words(c: &Company, d: &MyDuty) -> (String, Color) {
    match d.bus {
        Some(BusOf::Own(id)) => match c.vehicle(id) {
            Some(v) => (format!("{} {}", v.number, v.name), TEXT_SOFT),
            None => (omsi_ui::tr("No bus planned yet").into_owned(), WARN),
        },
        Some(BusOf::Rental) => (omsi_ui::tr("Rental bus").into_owned(), TEXT_SOFT),
        None => (omsi_ui::tr("No bus planned yet").into_owned(), WARN),
    }
}

/// "My duties": the duties the planning has the player drive himself in the coming week, the
/// next on top, each a click from the Drive page set to it; without any, how to get one.
fn duties_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    let side_w = (area.w * 0.3).clamp(260.0, 380.0);
    let list_r = Rect::new(area.x, area.y, area.w - side_w - gap, area.h);
    let side = Rect::new(list_r.right() + gap, area.y, side_w, area.h);
    let (plans, duties, whole) = my_duties(l, c);
    let inner = section(&mut l.ui, list_r, "My duties");
    if duties.is_empty() {
        if !whole {
            l.ui.text_in("Reading the timetable of the week…", Rect::new(inner.x, inner.y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        } else {
            let mid = inner.center();
            l.ui.icon("event", Vec2::new(mid.x, mid.y - 70.0), 34.0, TEXT_DIM);
            l.ui.text_in("No duty of yours in the coming week", Rect::new(inner.x, mid.y - 40.0, inner.w, 26.0), kit::HEAD, Weight::Bold, TEXT_SOFT, Align::Center);
            let text = "Plan yourself as a driver in the Planning: choose a duty there and \"You\". It is yours on that weekday every week.";
            let w = inner.w.min(520.0);
            l.ui.paragraph(text, Vec2::new(mid.x - w * 0.5, mid.y - 6.0), w, kit::NOTE, Weight::Regular, TEXT_DIM);
            let bw = kit::Foot::width(&l.ui, "To the planning", Some("event"));
            if l.ui.button("career-to-planning", Rect::new(mid.x - bw * 0.5, mid.y + 50.0, bw, 38.0), "To the planning", Some("event"), ButtonKind::Primary) {
                l.company.tab = 5;
            }
        }
    } else {
        let mut go: Option<usize> = None;
        let lines = c.lines.clone();
        let rows: Vec<(MyDuty, String, Color)> = duties.iter().map(|d| {
            let (b, col) = bus_words(c, d);
            (d.clone(), b, col)
        }).collect();
        l.ui.scroll_area("career-duties", inner, &mut |ui, v| {
            let rh = 84.0;
            for (k, (d, bus, bus_c)) in rows.iter().enumerate() {
                let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 8.0);
                if !ui.rect_visible(r) {
                    continue;
                }
                let next = k == 0;
                if next {
                    ui.p().rounded(r, RADIUS, accent().alpha(0.10));
                }
                ui.p().rounded_border(r, RADIUS, 1.0, if next { accent().alpha(0.5) } else { HAIRLINE });
                // the day and the time
                let tw = 190.0f32.min(r.w * 0.3);
                ui.text_in(&day_label(&d.date), Rect::new(r.x + 16.0, r.y + 12.0, tw, 22.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
                let time = format!("{} – {}", co::clock::hhmm(d.from as i64), co::clock::hhmm(d.to as i64));
                ui.text_in(&time, Rect::new(r.x + 16.0, r.y + 38.0, tw, 24.0), 19.0, Weight::Bold, if next { accent() } else { TEXT_SOFT }, Align::Left);
                // the line, the tour, the bus
                let x = r.x + 16.0 + tw + 12.0;
                let pw = match lines.iter().find(|x| x.name.eq_ignore_ascii_case(&d.line)) {
                    Some(cl) => super::line_plate(ui, Vec2::new(x, r.y + 12.0), cl, 24.0),
                    None => super::plate(ui, Vec2::new(x, r.y + 12.0), &d.number, 24.0),
                };
                let bw = 120.0;
                // (the next one says so, before its button)
                let up = omsi_ui::tr("Up next").into_owned();
                let tag_w = if next { ui.width(&up, 12.0, Weight::Bold) + 16.0 + 10.0 } else { 0.0 };
                let room = r.right() - bw - 24.0 - tag_w;
                let words = omsi_ui::tr("Tour %{t} · %{n} trips").replace("%{t}", &d.tour_no).replace("%{n}", &d.trips.to_string());
                ui.text_in(&words, Rect::new(x + pw + 10.0, r.y + 12.0, (room - x - pw - 10.0).max(20.0), 24.0), kit::ROWS, Weight::Medium, TEXT, Align::Left);
                ui.icon("directions_bus", Vec2::new(x + 9.0, r.y + 52.0), 16.0, *bus_c);
                ui.text_in(bus, Rect::new(x + 24.0, r.y + 41.0, (r.right() - bw - 24.0 - x - 24.0).max(20.0), 22.0), kit::NOTE, Weight::Regular, *bus_c, Align::Left);
                if next {
                    kit::tag(ui, Vec2::new(room, r.y + 12.0), &up, accent());
                }
                let b = Rect::new(r.right() - bw - 14.0, r.y + (r.h - 38.0) * 0.5, bw, 38.0);
                if ui.button(&format!("career-drive-{k}"), b, "Drive", Some("play_arrow"), if next { ButtonKind::Primary } else { ButtonKind::Normal }) {
                    go = Some(k);
                }
                ui.tooltip(b, "Sets the drive to this duty - the map, the line and tour, the company's bus in its livery, the day and the time - and opens its start: you start it there.");
            }
            rows.len() as f32 * rh
        });
        if let Some(k) = go {
            drive_duty(l, c, &plans, &duties[k]);
            return;
        }
    }
    let inner = section(&mut l.ui, side, "How it works");
    let text = "The duties here are those the Planning gives you yourself (\"You\" as the driver): the same weekday every week. \"Drive\" opens the start of the drive with everything set; what you drive counts for the company as measured when its day is closed, and a duty of yours not driven is open for the central to fill.";
    l.ui.paragraph(text, Vec2::new(inner.x, inner.y), inner.w, kit::NOTE + 0.5, Weight::Regular, TEXT_SOFT);
}

// --- progress -----------------------------------------------------------------------------------

/// A bar of progress with the points written under it.
fn level_bar(ui: &mut Ui, r: Rect, xp: i64, floor: i64, next: Option<i64>, c: Color) {
    let share = next.map(|n| ((xp - floor) as f64 / (n - floor).max(1) as f64).clamp(0.0, 1.0)).unwrap_or(1.0);
    let text = match next {
        Some(n) => omsi_ui::tr("To the next level: %{xp} of %{next} points").replace("%{xp}", &grouped(xp as f64)).replace("%{next}", &grouped(n as f64)),
        None => omsi_ui::tr("%{xp} points: the highest level").replace("%{xp}", &grouped(xp as f64)),
    };
    kit::bar(ui, "", Rect::new(r.x, r.y, r.w, kit::BAR_H), share, c, &text, &format!("{:.0} %", share * 100.0), &omsi_ui::tr("Points come with every day closed and every trip driven: tours run, trips on time, passengers carried, a good reputation"));
}

fn progress_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    let mut top = area.y;
    // a level reached and not yet said
    let lv = levels::level(c);
    if lv > c.progress.level_seen.max(1) {
        let r = Rect::new(area.x, top, area.w, 56.0);
        l.ui.p().rounded(r, RADIUS, accent_2().alpha(0.14));
        l.ui.p().rounded(Rect::new(r.x, r.y, 4.0, r.h), 2.0, accent_2());
        l.ui.icon("emoji_events", Vec2::new(r.x + 28.0, r.center().y), 22.0, accent_2());
        let opens: Vec<String> = (c.progress.level_seen.max(1) + 1..=lv).flat_map(levels::opens_at).map(|f| omsi_ui::tr(f.label()).into_owned()).collect();
        let head = omsi_ui::tr("Your company reached level %{n}: %{title}").replace("%{n}", &lv.to_string()).replace("%{title}", &omsi_ui::tr(levels::title_of(lv)));
        l.ui.text_in(&head, Rect::new(r.x + 52.0, r.y + 8.0, r.w - 200.0, 20.0), 16.0, Weight::Bold, TEXT, Align::Left);
        let sub = if opens.is_empty() { String::new() } else { omsi_ui::tr("New: %{what}").replace("%{what}", &opens.join(", ")) };
        l.ui.text_in(&sub, Rect::new(r.x + 52.0, r.y + 29.0, r.w - 200.0, 18.0), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
        if l.ui.button("company-level-seen", Rect::new(r.right() - 130.0, r.y + 11.0, 116.0, 34.0), "Wonderful", None, ButtonKind::Normal) {
            act(l, |c| {
                levels::take_new_level(c);
                Ok(())
            });
        }
        top += 56.0 + gap;
    }
    let half = (area.w - gap) * 0.5;
    let left = Rect::new(area.x, top, half, area.bottom() - top);
    let right = Rect::new(area.x + half + gap, top, half, area.bottom() - top);
    let head_h = 222.0;
    // the driver
    let inner = section(&mut l.ui, Rect::new(left.x, left.y, left.w, head_h), "You, the driver");
    let profile = l.state.config.profile.clone();
    match driver(l).map(|d| (d.summary.clone(), d.career.clone())) {
        None => {
            l.ui.paragraph("Choose a driver on the start page: the career is the driver's own.", Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        }
        Some((s, career)) => {
            l.ui.text_in(&profile, Rect::new(inner.x, inner.y - 2.0, inner.w * 0.6, 18.0), 14.0, Weight::Medium, TEXT_DIM, Align::Left);
            l.ui.text_in(&omsi_ui::tr(s.rank), Rect::new(inner.x, inner.y + 16.0, inner.w * 0.7, 30.0), 22.0, Weight::Bold, TEXT, Align::Left);
            let lvl = omsi_ui::tr("Level %{n}").replace("%{n}", &s.progress.level.to_string());
            let bw = l.ui.width(&lvl, kit::NOTE, Weight::Bold) + 20.0;
            let b = Rect::new(inner.right() - bw, inner.y + 20.0, bw, 24.0);
            l.ui.p().rounded(b, 12.0, accent_2().alpha(0.16));
            l.ui.text_in(&lvl, b, kit::NOTE, Weight::Bold, accent_2(), Align::Center);
            level_bar(&mut l.ui, Rect::new(inner.x, inner.y + 56.0, inner.w, 30.0), s.progress.xp, s.progress.floor, s.progress.next, accent_2());
            let facts = [
                ("Trips", s.trips.to_string()),
                ("Kilometres", grouped(s.km.round())),
                ("On time", s.punctuality.map_or("–".into(), |p| format!("{p:.0} %"))),
                ("Average score", s.average_score.map_or("–".into(), |a| format!("{a:.0}"))),
            ];
            let fw = inner.w / 4.0;
            for (k, (label, value)) in facts.iter().enumerate() {
                let x = inner.x + k as f32 * fw;
                l.ui.text_in(&omsi_ui::tr(label).to_uppercase(), Rect::new(x, inner.y + 112.0, fw - 8.0, 16.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
                l.ui.text_in(value, Rect::new(x, inner.y + 132.0, fw - 8.0, 28.0), 20.0, Weight::Bold, TEXT, Align::Left);
            }
            licences(l, Rect::new(left.x, left.y + head_h + gap, left.w, left.h - head_h - gap), &s, &career);
        }
    }
    // the company
    let inner = section(&mut l.ui, Rect::new(right.x, right.y, right.w, head_h), "The company");
    let (xp, floor, next) = levels::progress(c);
    l.ui.text_in(&c.name, Rect::new(inner.x, inner.y - 2.0, inner.w * 0.6, 18.0), 14.0, Weight::Medium, TEXT_DIM, Align::Left);
    l.ui.text_in(&omsi_ui::tr(levels::title_of(lv)), Rect::new(inner.x, inner.y + 16.0, inner.w * 0.7, 30.0), 22.0, Weight::Bold, TEXT, Align::Left);
    let lvl = omsi_ui::tr("Level %{n}").replace("%{n}", &lv.to_string());
    let bw = l.ui.width(&lvl, kit::NOTE, Weight::Bold) + 20.0;
    let b = Rect::new(inner.right() - bw, inner.y + 20.0, bw, 24.0);
    l.ui.p().rounded(b, 12.0, OK.alpha(0.16));
    l.ui.text_in(&lvl, b, kit::NOTE, Weight::Bold, OK, Align::Center);
    level_bar(&mut l.ui, Rect::new(inner.x, inner.y + 56.0, inner.w, 30.0), xp, floor, next, OK);
    let facts = [
        ("Fines paid", eur(c.progress.fines), if c.progress.fines > 0 { WARN } else { TEXT }),
        ("Bonuses", eur(c.progress.bonuses), if c.progress.bonuses > 0 { OK } else { TEXT }),
        ("Own work saved", eur(c.progress.saved), TEXT),
        ("Concessions", levels::max_concessions(c).to_string(), TEXT),
    ];
    let fw = inner.w / 4.0;
    for (k, (label, value, colour)) in facts.iter().enumerate() {
        let x = inner.x + k as f32 * fw;
        l.ui.text_in(&omsi_ui::tr(label).to_uppercase(), Rect::new(x, inner.y + 112.0, fw - 8.0, 16.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
        l.ui.text_in(value, Rect::new(x, inner.y + 132.0, fw - 8.0, 28.0), 20.0, Weight::Bold, *colour, Align::Left);
    }
    features(l, Rect::new(right.x, right.y + head_h + gap, right.w, right.h - head_h - gap), c);
}

/// The licences: held, the test booked, the level a test wants, or the button to book it.
fn licences(l: &mut Launcher, r: Rect, s: &dc::Summary, career: &DriverCareer) {
    let inner = section(&mut l.ui, r, "Licences");
    let mut y = inner.y;
    let mut book: Option<LicenceClass> = None;
    let mut cancel = false;
    for class in LicenceClass::ALL {
        let row = Rect::new(inner.x, y, inner.w, 40.0);
        if row.bottom() > inner.bottom() + 8.0 {
            break;
        }
        let held = career.holds(class);
        l.ui.icon(if held { "check_circle" } else { "lock" }, Vec2::new(row.x + 10.0, row.center().y), 17.0, if held { OK } else { TEXT_FAINT });
        l.ui.text_in(&omsi_ui::tr(class.label()), Rect::new(row.x + 28.0, row.y, row.w * 0.4, row.h), 15.5, Weight::Bold, if held { TEXT } else { TEXT_SOFT }, Align::Left);
        let x = row.x + row.w * 0.42;
        let w = row.w - row.w * 0.42;
        if held {
            let t = if class.basic() { "Held from the start" } else { "Won with the driving test" };
            l.ui.text_in(t, Rect::new(x, row.y, w, row.h), 14.0, Weight::Regular, TEXT_DIM, Align::Left);
        } else if career.booked.as_ref().is_some_and(|b| b.class == class) {
            l.ui.text_in("Booked: drive a trip to a timetable with this bus", Rect::new(x, row.y, w - 96.0, row.h), 14.0, Weight::Medium, accent_2(), Align::Left);
            if l.ui.button(&format!("company-exam-cancel-{class:?}"), Rect::new(row.right() - 90.0, row.y + 4.0, 90.0, 32.0), "Cancel", None, ButtonKind::Ghost) {
                cancel = true;
            }
        } else if s.progress.level < class.level() {
            let t = omsi_ui::tr("The test from driver level %{n}").replace("%{n}", &class.level().to_string());
            l.ui.text_in(&t, Rect::new(x, row.y, w, row.h), 14.0, Weight::Regular, TEXT_DIM, Align::Left);
        } else if career.booked.is_none() {
            l.ui.text_in("Ready for the test", Rect::new(x, row.y, w - 150.0, row.h), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
            if l.ui.button(&format!("company-exam-{class:?}"), Rect::new(row.right() - 140.0, row.y + 4.0, 140.0, 32.0), "Book the test", Some("badge"), ButtonKind::Normal) {
                book = Some(class);
            }
        } else {
            l.ui.text_in("Another test is booked", Rect::new(x, row.y, w, row.h), 14.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
        l.ui.p().rect(Rect::new(row.x, row.bottom(), row.w, 1.0), HAIRLINE);
        y += 42.0;
    }
    // the last test, and what a test asks
    if let Some(e) = career.exams.first() {
        if y + 22.0 <= inner.bottom() + 8.0 {
            let t = omsi_ui::tr(if e.passed { "Last test: %{class}, passed (%{score} points)" } else { "Last test: %{class}, failed (%{score} points)" }).replace("%{class}", &omsi_ui::tr(e.class.label())).replace("%{score}", &e.score.to_string());
            l.ui.text_in(&t, Rect::new(inner.x, y + 4.0, inner.w, 18.0), 14.0, Weight::Medium, if e.passed { OK } else { WARN }, Align::Left);
            y += 24.0;
        }
    }
    if y + 36.0 <= inner.bottom() + 8.0 {
        l.ui.paragraph("A test is one trip to a timetable with a bus of its kind: driven to the end, on average within three minutes, at most five rough moments, and no red light, camera or collision.", Vec2::new(inner.x, y + 6.0), inner.w, kit::NOTE, Weight::Regular, TEXT_SOFT);
    }
    if book.is_some() || cancel {
        let data = core::data_dir();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Some(d) = l.company.career.driver.as_mut() {
            let result = match book {
                Some(class) => dc::book_exam(&mut d.career, class, d.summary.progress.level, now),
                None => {
                    dc::cancel_exam(&mut d.career);
                    Ok(())
                }
            };
            match result {
                Err(e) => kit::refuse(l, e),
                Ok(()) => match dc::save(&data, &d.career) {
                    Ok(()) => {
                        if let Some(class) = book {
                            l.state.set_status(omsi_ui::tr("The %{class} test is booked: your next trip to a timetable with such a bus is the test.").replace("%{class}", &omsi_ui::tr(class.label())), false);
                        }
                    }
                    Err(e) => l.state.set_status(format!("{e:#}"), true),
                },
            }
        }
    }
}

/// What the levels open: each feature, open or the level it wants.
fn features(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "What the levels open");
    let lv = levels::level(c);
    let mut list: Vec<Feature> = Feature::ALL.to_vec();
    list.sort_by_key(|f| f.level());
    l.ui.scroll_area("company-features", inner, &mut |ui, v| {
        let rh = 40.0;
        for (k, f) in list.iter().enumerate() {
            let row = Rect::new(v.x, v.y + k as f32 * rh, v.w - 12.0, rh - 2.0);
            if !ui.rect_visible(row) {
                continue;
            }
            let open = lv >= f.level();
            ui.icon(f.icon(), Vec2::new(row.x + 11.0, row.center().y), 17.0, if open { OK } else { TEXT_FAINT });
            ui.text_in(&omsi_ui::tr(f.label()), Rect::new(row.x + 30.0, row.y, row.w * 0.6, row.h), kit::ROWS, Weight::Medium, if open { TEXT } else { TEXT_DIM }, Align::Left);
            let state = if open { omsi_ui::tr("Unlocked").into_owned() } else { omsi_ui::tr("Level %{n}").replace("%{n}", &f.level().to_string()) };
            ui.text_in(&state, Rect::new(row.right() - 120.0, row.y, 116.0, row.h), 14.0, Weight::Bold, if open { OK } else { TEXT_FAINT }, Align::Right);
            ui.p().rect(Rect::new(row.x, row.bottom(), row.w, 1.0), HAIRLINE);
        }
        list.len() as f32 * rh
    });
}

// --- training -----------------------------------------------------------------------------------

/// What a course's head says: its cost and days.
fn course_terms(c: &Company, k: CourseKind) -> String {
    let days = k.spec().days;
    let d = if days == 1 { omsi_ui::tr("1 day").into_owned() } else { omsi_ui::tr("%{n} days").replace("%{n}", &days.to_string()) };
    format!("{}  ·  {}", eur(training::cost_of(c, k)), d)
}

fn training_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    // the player's own courses
    let top_h = 176.0;
    let inner = section(&mut l.ui, Rect::new(area.x, area.y, area.w, top_h), "Your own training");
    let cw = (inner.w - gap) * 0.5;
    let mut enrol: Option<(CourseKind, Option<u32>)> = None;
    for (k, kind) in CourseKind::PLAYER.iter().enumerate() {
        let r = Rect::new(inner.x + k as f32 * (cw + gap), inner.y, cw, inner.h);
        l.ui.p().rounded(r, RADIUS, FIELD.alpha(0.6));
        l.ui.icon(kind.icon(), Vec2::new(r.x + 24.0, r.y + 26.0), 20.0, accent_2());
        l.ui.text_in(&omsi_ui::tr(kind.label()), Rect::new(r.x + 48.0, r.y + 12.0, r.w - 210.0, 24.0), 16.0, Weight::Bold, TEXT, Align::Left);
        l.ui.text_in(&course_terms(c, *kind), Rect::new(r.x + 48.0, r.y + 38.0, r.w - 210.0, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
        l.ui.text_in(&omsi_ui::tr(kind.effect()), Rect::new(r.x + 18.0, r.y + 72.0, r.w - 36.0, 22.0), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
        let b = Rect::new(r.right() - 150.0, r.y + 12.0, 136.0, 34.0);
        match training::course_of(c, None, *kind) {
            Some(x) if x.done => {
                l.ui.text_in("Course completed", b, kit::ROWS, Weight::Bold, OK, Align::Right);
            }
            Some(x) => {
                let t = omsi_ui::tr("Until %{date}").replace("%{date}", &day_label(&x.until));
                l.ui.text_in(&t, b, 14.0, Weight::Medium, accent_2(), Align::Right);
            }
            None if !levels::unlocked(c, kind.spec().feature) => {
                let t = omsi_ui::tr("Company level %{n}").replace("%{n}", &kind.spec().feature.level().to_string());
                l.ui.text_in(&t, b, 14.0, Weight::Medium, TEXT_DIM, Align::Right);
            }
            None => {
                if l.ui.button(&format!("company-course-me-{kind:?}"), b, "Book the course", None, ButtonKind::Normal) {
                    enrol = Some((*kind, None));
                }
            }
        }
    }
    // the staff's courses: a row for each person, a column for each course
    let y = area.y + top_h + gap;
    let r = Rect::new(area.x, y, area.w, area.bottom() - y - 30.0);
    let inner = section(&mut l.ui, r, "Courses for your staff");
    let kinds = CourseKind::STAFF;
    let name_w = (inner.w * 0.22).max(160.0);
    let col_w = (inner.w - name_w - 12.0) / kinds.len() as f32;
    for (k, kind) in kinds.iter().enumerate() {
        let x = inner.x + name_w + k as f32 * col_w;
        l.ui.icon(kind.icon(), Vec2::new(x + 9.0, inner.y + 9.0), 15.0, if levels::unlocked(c, kind.spec().feature) { accent_2() } else { TEXT_FAINT });
        l.ui.text_in(&omsi_ui::tr(kind.label()), Rect::new(x + 22.0, inner.y, col_w - 26.0, 18.0), 14.0, Weight::Bold, TEXT, Align::Left);
        l.ui.text_in(&course_terms(c, *kind), Rect::new(x + 22.0, inner.y + 18.0, col_w - 26.0, 15.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        l.ui.tooltip(Rect::new(x, inner.y, col_w, 34.0), &omsi_ui::tr(kind.effect()));
    }
    l.ui.p().rect(Rect::new(inner.x, inner.y + 40.0, inner.w, 1.0), HAIRLINE);
    let rows = Rect::new(inner.x, inner.y + 46.0, inner.w, (inner.h - 46.0).max(0.0));
    // (who stays: id, name, away today)
    let people: Vec<(u32, String, bool)> = c.staff.iter().filter(|e| e.notice_until.is_none()).map(|e| (e.id, e.name.clone(), e.absent(&c.date))).collect();
    if people.is_empty() {
        l.ui.text_in("Nobody works here yet.", Rect::new(rows.x, rows.y, rows.w, 22.0), kit::ROWS, Weight::Regular, TEXT_DIM, Align::Left);
    }
    let cc = c.clone();
    l.ui.scroll_area("company-courses", rows, &mut |ui, v| {
        let rh = 40.0;
        for (i, (id, name, away)) in people.iter().enumerate() {
            let row = Rect::new(v.x, v.y + i as f32 * rh, v.w - 12.0, rh - 2.0);
            if !ui.rect_visible(row) {
                continue;
            }
            ui.text_in(name, Rect::new(row.x + 4.0, row.y, name_w - 12.0, row.h), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            for (k, kind) in kinds.iter().enumerate() {
                let cell = Rect::new(row.x + name_w + k as f32 * col_w, row.y + 4.0, col_w - 10.0, row.h - 8.0);
                match training::course_of(&cc, Some(*id), *kind) {
                    Some(x) if x.done => {
                        ui.icon("check_circle", Vec2::new(cell.x + 10.0, cell.center().y), 15.0, OK);
                        ui.text_in("Course completed", Rect::new(cell.x + 24.0, cell.y, cell.w - 24.0, cell.h), 14.0, Weight::Medium, OK, Align::Left);
                    }
                    Some(x) => {
                        ui.icon("schedule", Vec2::new(cell.x + 10.0, cell.center().y), 15.0, accent_2());
                        let t = omsi_ui::tr("Until %{date}").replace("%{date}", &day_label(&x.until));
                        ui.text_in(&t, Rect::new(cell.x + 24.0, cell.y, cell.w - 24.0, cell.h), kit::NOTE, Weight::Medium, accent_2(), Align::Left);
                    }
                    None if !levels::unlocked(&cc, kind.spec().feature) => {
                        ui.icon("lock", Vec2::new(cell.x + 10.0, cell.center().y), 14.0, TEXT_FAINT);
                    }
                    None if *away => {
                        ui.text_in("Away", Rect::new(cell.x + 4.0, cell.y, cell.w, cell.h), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
                    }
                    None => {
                        if ui.button(&format!("company-course-{id}-{kind:?}"), Rect::new(cell.x, cell.y, cell.w.min(110.0), cell.h), "Book", None, ButtonKind::Ghost) {
                            enrol = Some((*kind, Some(*id)));
                        }
                    }
                }
            }
            ui.p().rect(Rect::new(row.x, row.bottom(), row.w, 1.0), HAIRLINE);
        }
        people.len() as f32 * rh
    });
    l.ui.text_in("A course begins today: the person is away for its days, and has learnt it when its last day is closed.", Rect::new(area.x, area.bottom() - 22.0, area.w, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
    if let Some((kind, who)) = enrol {
        if let Some(until) = act(l, |c| training::enrol(c, kind, who)) {
            l.state.set_status(omsi_ui::tr("Booked: %{course}, until %{date}.").replace("%{course}", &omsi_ui::tr(kind.label())).replace("%{date}", &day_label(&until)), false);
        }
    }
}

// --- workshop -----------------------------------------------------------------------------------

fn workshop_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    if !levels::unlocked(c, Feature::Workshop) {
        let inner = section(&mut l.ui, Rect::new(area.x, area.y, area.w.min(760.0), 150.0), "The workshop");
        let t = omsi_ui::tr("Your company opens its own workshop at level %{n}. Until then the buses go to a contract workshop.").replace("%{n}", &Feature::Workshop.level().to_string());
        l.ui.paragraph(&t, Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    }
    let service = training::player_can(c, CourseKind::PlayerService);
    let repairs = training::player_can(c, CourseKind::PlayerRepairs);
    let list_w = (area.w - gap) * 0.62;
    let inner = section(&mut l.ui, Rect::new(area.x, area.y, list_w, area.h), "Jobs you can do today");
    let mut jobs: Vec<(u32, JobKind, String, String, Option<i64>, bool)> = Vec::new();
    for v in &c.fleet {
        if let Some(bill) = training::repair_due(c, v) {
            jobs.push((v.id, JobKind::Repair, format!("{} {}", v.number, v.name), omsi_ui::tr("Broken down: the repair bill was %{amount}").replace("%{amount}", &eur(bill)), Some(bill * 55 / 100), repairs));
        } else if !v.in_workshop(&c.date) && (v.km >= v.next_service_km - 5_000.0 || v.condition < 90.0) {
            let done = c.progress.jobs.iter().any(|j| j.vehicle == v.id && j.date == c.date);
            let what = omsi_ui::tr("Condition %{n} %, next service at %{km} km").replace("%{n}", &format!("{:.0}", v.condition)).replace("%{km}", &grouped(v.next_service_km));
            jobs.push((v.id, JobKind::Service, format!("{} {}", v.number, v.name), what, Some(training::service_labour(c)), service && !done));
        }
    }
    let mut start: Option<(u32, JobKind)> = None;
    let mut blocked: Option<JobKind> = None;
    if jobs.is_empty() {
        l.ui.paragraph("No bus needs you today: none is due for its service, none broke down.", Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
    } else {
        l.ui.scroll_area("company-jobs", inner, &mut |ui, v| {
            let rh = 58.0;
            for (k, (id, kind, name, what, save, can)) in jobs.iter().enumerate() {
                let row = Rect::new(v.x, v.y + k as f32 * rh, v.w - 12.0, rh - 4.0);
                if !ui.rect_visible(row) {
                    continue;
                }
                ui.p().rounded(row, RADIUS, FIELD.alpha(0.5));
                let (icon, ink) = if *kind == JobKind::Repair { ("construction", WARN) } else { ("tune", accent_2()) };
                ui.icon(icon, Vec2::new(row.x + 22.0, row.center().y), 19.0, ink);
                ui.text_in(name, Rect::new(row.x + 44.0, row.y + 8.0, row.w - 220.0, 20.0), 15.5, Weight::Bold, TEXT, Align::Left);
                ui.text_in(what, Rect::new(row.x + 44.0, row.y + 28.0, row.w - 220.0, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
                if let Some(s) = save {
                    let t = omsi_ui::tr("up to %{amount}").replace("%{amount}", &eur(*s));
                    ui.text_in(&t, Rect::new(row.right() - 280.0, row.y, 120.0, row.h), 14.0, Weight::Medium, OK, Align::Right);
                }
                let b = Rect::new(row.right() - 150.0, row.y + 10.0, 138.0, row.h - 20.0);
                if ui.button(&format!("company-job-{id}-{kind:?}"), b, if *kind == JobKind::Repair { "Repair it" } else { "Service it" }, if *can { None } else { Some("lock") }, ButtonKind::Normal) {
                    if *can {
                        start = Some((*id, *kind));
                    } else {
                        blocked = Some(*kind);
                    }
                }
                if !*can {
                    ui.tooltip(b, if *kind == JobKind::Repair { "The repairs course first" } else { "The workshop course first" });
                }
            }
            jobs.len() as f32 * rh
        });
    }
    // what the player did lately, and how the job goes
    let side = Rect::new(area.x + list_w + gap, area.y, area.w - list_w - gap, area.h);
    let inner = section(&mut l.ui, side, "How it works");
    let h = l.ui.paragraph("Each part of the bus is checked and dealt with: replace it, adjust it, top it up, or leave it as it is. Read what the check finds - and mind the time. The better the work, the more it saves; a service done badly sends the bus to the workshop after all.", Vec2::new(inner.x, inner.y), inner.w, 14.0, Weight::Regular, TEXT_SOFT);
    let mut y = inner.y + h + 18.0;
    if !(service && repairs) {
        if l.ui.button("company-to-training", Rect::new(inner.x, y, 200.0f32.min(inner.w), 34.0), "To the courses", Some("badge"), ButtonKind::Normal) {
            l.company.career.part = TRAINING;
        }
        y += 48.0;
    }
    l.ui.text_in(&omsi_ui::tr("Done lately").to_uppercase(), Rect::new(inner.x, y, inner.w, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
    y += 22.0;
    if c.progress.jobs.is_empty() {
        l.ui.text_in("No job yet.", Rect::new(inner.x, y, inner.w, 18.0), 14.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    for j in c.progress.jobs.iter().rev().take(8) {
        if y + 22.0 > inner.bottom() + 6.0 {
            break;
        }
        let what = if j.repair { omsi_ui::tr("Repair job") } else { omsi_ui::tr("Service job") };
        let t = format!("{}  ·  {} {}  ·  {:.0} %", day_label(&j.date), what, j.number, j.quality * 100.0);
        l.ui.text_in(&t, Rect::new(inner.x, y, inner.w - 90.0, 20.0), 14.0, Weight::Regular, TEXT_SOFT, Align::Left);
        l.ui.text_in(&eur(j.saved), Rect::new(inner.right() - 90.0, y, 90.0, 20.0), 14.0, Weight::Bold, if j.saved >= 0 { OK } else { DANGER.lighten(0.2) }, Align::Right);
        y += 24.0;
    }
    if let Some((id, kind)) = start {
        l.company.career.game = super::repair_game::Game::new(c, id, kind);
    }
    if let Some(kind) = blocked {
        let (what, course) = if kind == JobKind::Repair { ("Repairs need the repairs course: what is broken is found and mended the right way.", "Book the repairs course under Training: it takes a few days.") } else { ("A service needs the workshop course: which part wants what.", "Book the workshop course under Training: it takes a few days.") };
        kit::show(l, kit::Popup::new("badge", "A course first", omsi_ui::tr(what), omsi_ui::tr(course), Some(kit::Go::Courses)));
    }
}

// --- rankings -----------------------------------------------------------------------------------

/// A table of places: the place, the name (the player's marked), the level, the points, the
/// quality and the count.
fn table(ui: &mut Ui, r: Rect, rows: &[(usize, Entry)], quality: &str, count: &str) {
    let cols = [0.08, 0.42, 0.12, 0.16, 0.12, 0.10];
    let xs: Vec<f32> = cols.iter().scan(r.x, |x, w| {
        let at = *x;
        *x += r.w * w;
        Some(at)
    }).collect();
    let heads = ["#", "Name", "Level", "Points", quality, count];
    for (k, h) in heads.iter().enumerate() {
        let align = if k >= 2 { Align::Right } else { Align::Left };
        ui.text_in(&omsi_ui::tr(h).to_uppercase(), Rect::new(xs[k], r.y, r.w * cols[k] - 8.0, 14.0), kit::CAPS, Weight::Bold, TEXT_DIM, align);
    }
    ui.p().rect(Rect::new(r.x, r.y + 20.0, r.w, 1.0), HAIRLINE);
    let rh = 36.0;
    for (i, (place, e)) in rows.iter().enumerate() {
        let y = r.y + 26.0 + i as f32 * rh;
        if y + rh > r.bottom() + 4.0 {
            break;
        }
        let row = Rect::new(r.x - 6.0, y, r.w + 12.0, rh - 4.0);
        if e.you {
            ui.p().rounded(row, RADIUS, accent_2().alpha(0.14));
        }
        let medal = match place {
            1 => Some(LINE),
            2 => Some(Color::rgba(200, 206, 216, 1.0)),
            3 => Some(Color::rgba(205, 140, 80, 1.0)),
            _ => None,
        };
        match medal {
            Some(m) => {
                ui.p().circle(Vec2::new(xs[0] + 10.0, row.center().y), 11.0, m.alpha(0.22));
                ui.text_in(&place.to_string(), Rect::new(xs[0], row.y, 20.0, row.h), 14.0, Weight::Bold, m, Align::Center);
            }
            None => {
                ui.text_in(&place.to_string(), Rect::new(xs[0], row.y, 20.0, row.h), 14.0, Weight::Medium, TEXT_DIM, Align::Center);
            }
        }
        ui.text_in(&e.name, Rect::new(xs[1], row.y, r.w * cols[1] - 8.0, row.h), 13.5, if e.you { Weight::Bold } else { Weight::Medium }, if e.you { TEXT } else { TEXT_SOFT }, Align::Left);
        ui.text_in(&e.level.to_string(), Rect::new(xs[2], row.y, r.w * cols[2] - 8.0, row.h), kit::ROWS, Weight::Medium, TEXT_SOFT, Align::Right);
        ui.text_in(&grouped(e.points as f64), Rect::new(xs[3], row.y, r.w * cols[3] - 8.0, row.h), kit::ROWS, Weight::Bold, TEXT, Align::Right);
        ui.text_in(&format!("{:.0}", e.quality), Rect::new(xs[4], row.y, r.w * cols[4] - 8.0, row.h), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Right);
        ui.text_in(&e.count.to_string(), Rect::new(xs[5], row.y, r.w * cols[5] - 8.0, row.h), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Right);
    }
}

fn rankings_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    let half = (area.w - gap) * 0.5;
    let map = if c.map_name.is_empty() { super::super::state::short_map(&c.map) } else { c.map_name.clone() };
    let companies = rankings::company_table(c);
    let title = omsi_ui::tr("Companies of %{map}").replace("%{map}", &map);
    let inner = section(&mut l.ui, Rect::new(area.x, area.y, half, area.h - 30.0), &title);
    let place = rankings::place_of(&companies).unwrap_or(0);
    l.ui.text_in(&omsi_ui::tr("Place %{n} of %{all}").replace("%{n}", &place.to_string()).replace("%{all}", &companies.len().to_string()), Rect::new(inner.x, inner.y - 30.0, inner.w, 14.0), kit::NOTE, Weight::Bold, accent_2(), Align::Right);
    table(&mut l.ui, inner, &companies, "On time", "Buses");
    let right = Rect::new(area.x + half + gap, area.y, half, area.h - 30.0);
    let inner = section(&mut l.ui, right, "Drivers");
    let profile = l.state.config.profile.clone();
    match driver(l).map(|d| d.summary.clone()) {
        Some(s) => {
            let drivers = rankings::driver_table(&c.map, &profile, &s);
            let place = rankings::place_of(&drivers).unwrap_or(0);
            l.ui.text_in(&omsi_ui::tr("Place %{n} of %{all}").replace("%{n}", &place.to_string()).replace("%{all}", &drivers.len().to_string()), Rect::new(inner.x, inner.y - 30.0, inner.w, 14.0), kit::NOTE, Weight::Bold, accent_2(), Align::Right);
            table(&mut l.ui, inner, &drivers, "Score", "Trips");
        }
        None => {
            l.ui.text_in("Choose a driver on the start page.", Rect::new(inner.x, inner.y, inner.w, 20.0), kit::ROWS, Weight::Regular, TEXT_DIM, Align::Left);
        }
    }
    l.ui.text_in("The other companies and drivers of the map are the game's own; in multiplayer the table will hold the real players.", Rect::new(area.x, area.bottom() - 22.0, area.w, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
}

// --- statistics ---------------------------------------------------------------------------------

/// Bars of 0 - 100 (the newest on the right), each coloured by its value; a tooltip each.
fn score_chart(ui: &mut Ui, r: Rect, values: &[(u32, String)], empty: &str) {
    if values.is_empty() {
        ui.paragraph(empty, Vec2::new(r.x, r.y + 4.0), r.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    }
    let plot = Rect::new(r.x + 30.0, r.y + 4.0, r.w - 30.0, r.h - 10.0);
    for (v, y) in [(100.0, plot.y), (50.0, plot.y + plot.h * 0.5), (0.0, plot.bottom())] {
        ui.p().rect(Rect::new(plot.x, y, plot.w, 1.0), Color::WHITE.alpha(if v == 0.0 { 0.18 } else { 0.06 }));
        ui.text_in(&format!("{v:.0}"), Rect::new(r.x, y - 7.0, 24.0, 14.0), kit::CAPS, Weight::Regular, TEXT_DIM, Align::Right);
    }
    let n = 30usize;
    let slot = plot.w / n as f32;
    let bw = (slot * 0.62).max(2.0);
    let shown: Vec<&(u32, String)> = values.iter().rev().take(n).rev().collect();
    let start = plot.x + (n - shown.len()) as f32 * slot;
    for (k, (v, tip)) in shown.iter().enumerate() {
        let x = start + k as f32 * slot + (slot - bw) * 0.5;
        let h = (plot.h * (*v).min(100) as f32 / 100.0).max(2.0);
        let col = if *v >= 75 { OK } else if *v >= 50 { WARN } else { DANGER };
        let cell = Rect::new(start + k as f32 * slot, plot.y, slot, plot.h);
        let hover = ui.hover(cell);
        ui.p().rounded(Rect::new(x, plot.bottom() - h, bw, h), 2.0f32.min(bw * 0.4), if hover { col.lighten(0.25) } else { col.alpha(0.85) });
        if hover {
            ui.tooltip(cell, tip);
        }
    }
}

fn statistics_part(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 12.0;
    let half = (area.w - gap) * 0.5;
    // the player's trips
    let left = Rect::new(area.x, area.y, half, area.h);
    let fh = kit::FIGURE_H;
    match driver(l).map(|d| (d.summary.clone(), d.trips.clone())) {
        Some((s, trips)) => {
            let fw = (half - 3.0 * gap) / 4.0;
            let figures = [
                ("Trips", s.trips.to_string(), omsi_ui::tr("%{n} hours").replace("%{n}", &super::num(s.hours, 1)), TEXT),
                ("Kilometres", grouped(s.km.round()), omsi_ui::tr("%{n} passengers").replace("%{n}", &grouped(s.passengers as f64)), TEXT),
                ("On time", s.punctuality.map_or("–".into(), |p| format!("{p:.0} %")), omsi_ui::tr("%{n} excellent trips").replace("%{n}", &s.excellent.to_string()), TEXT),
                ("Fines", eur(s.fines), omsi_ui::tr("%{r} red · %{c} cameras").replace("%{r}", &s.red_lights.to_string()).replace("%{c}", &s.speeding.to_string()), if s.fines > 0 { WARN } else { TEXT }),
            ];
            for (k, (label, value, under, colour)) in figures.iter().enumerate() {
                figure(&mut l.ui, Rect::new(left.x + k as f32 * (fw + gap), left.y, fw, fh), label, value, under, *colour);
            }
            let r = Rect::new(left.x, left.y + fh + gap, left.w, left.h - fh - gap);
            let inner = section(&mut l.ui, r, "Your last 30 trips, judged");
            let values: Vec<(u32, String)> = trips
                .iter()
                .rev()
                .map(|t| {
                    let e = dc::evaluate(t);
                    let line = if t.line.trim().is_empty() { omsi_ui::tr("Free drive").into_owned() } else { omsi_ui::tr("Line %{n}").replace("%{n}", &t.line) };
                    (e.score, format!("{line}  ·  {}  ·  {} ({})", t.terminus, e.score, omsi_ui::tr(e.grade.label())))
                })
                .collect();
            score_chart(&mut l.ui, Rect::new(inner.x, inner.y, inner.w, (inner.h - 60.0).max(80.0)), &values, "Drive a trip to see it judged here.");
            let avg = s.average_score.map_or("–".into(), |a| format!("{a:.0}"));
            let best = s.best_score.map_or("–".into(), |b| b.to_string());
            let t = omsi_ui::tr("Average %{avg}  ·  best %{best}  ·  %{n} trips seen by the drive watch").replace("%{avg}", &avg).replace("%{best}", &best).replace("%{n}", &s.watched.to_string());
            l.ui.text_in(&t, Rect::new(inner.x, inner.bottom() - 22.0, inner.w, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        }
        None => {
            l.ui.text_in("Choose a driver on the start page.", Rect::new(left.x, left.y, left.w, 20.0), kit::ROWS, Weight::Regular, TEXT_DIM, Align::Left);
        }
    }
    // the company's
    let right = Rect::new(area.x + half + gap, area.y, half, area.h);
    let fw = (half - 3.0 * gap) / 4.0;
    let days = c.history.len();
    let pax: u64 = c.history.iter().map(|d| d.passengers as u64).sum();
    let km: f64 = c.history.iter().map(|d| d.km).sum();
    let judged = &c.progress.judged;
    let avg = (!judged.is_empty()).then(|| judged.iter().map(|j| j.score as f64).sum::<f64>() / judged.len() as f64);
    let figures = [
        ("Days run", days.to_string(), omsi_ui::tr("since %{date}").replace("%{date}", &day_label(&c.founded)), TEXT),
        ("Passengers", grouped(pax as f64), format!("{} km", grouped(km.round())), TEXT),
        ("Your tours", judged.len().to_string(), avg.map_or(String::new(), |a| omsi_ui::tr("average %{n}").replace("%{n}", &format!("{a:.0}"))), TEXT),
        ("Reputation", format!("{:.0}", c.reputation), omsi_ui::tr("of 100").into_owned(), super::grade(c.reputation + 15.0)),
    ];
    for (k, (label, value, under, colour)) in figures.iter().enumerate() {
        figure(&mut l.ui, Rect::new(right.x + k as f32 * (fw + gap), right.y, fw, fh), label, value, under, *colour);
    }
    let r = Rect::new(right.x, right.y + fh + gap, right.w, right.h - fh - gap);
    let inner = section(&mut l.ui, r, "Your tours for the company, judged");
    let values: Vec<(u32, String)> = judged
        .iter()
        .map(|j| {
            let mut t = format!("{}  ·  {}  ·  {}", day_label(&j.date), omsi_ui::tr("Line %{n}").replace("%{n}", &j.line), j.score);
            if j.fines > 0 {
                t.push_str(&format!("  ·  {} {}", omsi_ui::tr("Fines"), eur(j.fines)));
            }
            if j.bonus > 0 {
                t.push_str(&format!("  ·  {} {}", omsi_ui::tr("Bonus"), eur(j.bonus)));
            }
            (j.score, t)
        })
        .collect();
    score_chart(&mut l.ui, Rect::new(inner.x, inner.y, inner.w, (inner.h - 60.0).max(80.0)), &values, "Drive the company's lines yourself: each trip is judged when the day is closed.");
    let t = omsi_ui::tr("Fines %{fines}  ·  bonuses %{bonus}  ·  saved by own work %{saved}").replace("%{fines}", &eur(c.progress.fines)).replace("%{bonus}", &eur(c.progress.bonuses)).replace("%{saved}", &eur(c.progress.saved));
    l.ui.text_in(&t, Rect::new(inner.x, inner.bottom() - 22.0, inner.w, 18.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
}
