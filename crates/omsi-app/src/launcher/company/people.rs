//! The staff and the labour market: who works for the company (experience, licence, wage,
//! reliability, satisfaction and where they are today), a raise or a dismissal with notice;
//! the week's applicants with what they ask; and the licences and type trainings - who may
//! drive which bus, and the courses that open more (`co::licences`).

use super::super::theme::*;
use super::super::ui::{ButtonKind, Ui};
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{act, day_label, eur, grade, meter, Confirm, Dialog};
use glam::Vec2;
use omsi_launcher_lib::company::licences::{self, Endorsement, Lack};
use omsi_launcher_lib::company::staff::{self, Applicant};
use omsi_launcher_lib::company::training::{self, CourseKind};
use omsi_launcher_lib::company::{self as co, Company, Employee, Licence, Skills};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct PeopleView {
    tab: usize,
    /// The training matrix's group (licences, bus types, courses) and its first column shown
    /// (more columns than fit: paged).
    group: usize,
    first: usize,
}

/// The applicants' list (a popup's "Hire drivers").
pub(super) fn to_applicants(l: &mut Launcher) {
    l.company.people.tab = 1;
}

/// The staff's courses (a day report's "Train the drivers").
pub(super) fn to_courses(l: &mut Launcher) {
    l.company.tab = 2;
    l.company.people.tab = 2;
    l.company.people.group = 2;
    l.company.people.first = 0;
}

/// The type trainings of the fleet's models (a bus card's "Train drivers").
pub(super) fn to_training(l: &mut Launcher) {
    l.company.tab = 2;
    l.company.people.tab = 2;
    l.company.people.group = 1;
    l.company.people.first = 0;
}

/// What a driver lacks for a bus, in words.
pub(super) fn lack_text(c: &Company, x: &Lack) -> String {
    match x {
        Lack::Licence => omsi_ui::tr("No licence for this bus").into_owned(),
        Lack::Endorsement(e) => omsi_ui::tr(e.lacking()).into_owned(),
        Lack::Type(k) => omsi_ui::tr("No type training for the %{bus}").replace("%{bus}", &licences::type_name(c, k)),
    }
}

/// A bus joined the fleet: how many drivers may drive it, when not all.
pub(super) fn drivers_note(c: &Company, id: u32) -> Option<String> {
    let v = c.vehicle(id)?;
    let (q, m) = licences::qualified_drivers(c, v.kind, &v.bus);
    (m > 0 && q < m).then(|| omsi_ui::tr("%{q} of %{m} drivers may drive it: train more on the Staff page.").replace("%{q}", &q.to_string()).replace("%{m}", &m.to_string()))
}

/// The licence and the endorsements: "D", "D · G E".
fn licence_line(l: Licence, e: &[Endorsement]) -> String {
    let mut s = licence_text(l).to_string();
    if !e.is_empty() {
        s.push_str(" · ");
        s.push_str(&e.iter().map(|x| x.short()).collect::<Vec<_>>().join(" "));
    }
    s
}

/// The licence and the endorsements spelt out (a tooltip).
fn endorsements_tip(l: Licence, e: &[Endorsement]) -> String {
    let mut v = vec![omsi_ui::tr(if l == Licence::D1 { "D1: only midibuses" } else { "D: every bus" }).into_owned()];
    v.extend(e.iter().map(|x| format!("{}: {}", x.short(), omsi_ui::tr(x.label()))));
    v.join("\n")
}

/// The columns: name, experience, licence, wage, reliability, satisfaction or skills, status,
/// actions - as shares of the width.
const COLS: [f32; 8] = [0.20, 0.13, 0.07, 0.11, 0.09, 0.13, 0.15, 0.12];

fn cols(r: Rect) -> Vec<Rect> {
    let mut x = r.x;
    COLS.iter()
        .map(|w| {
            let c = Rect::new(x, r.y, r.w * w, r.h);
            x += r.w * w;
            c
        })
        .collect()
}

fn licence_text(l: Licence) -> &'static str {
    match l {
        Licence::D => "D",
        Licence::D1 => "D1",
    }
}

/// Where someone is today, and its colour.
fn status_of(c: &Company, e: &Employee, working: bool) -> (String, Color) {
    if let Some(u) = e.sick_until.as_deref().filter(|u| co::dates::between(&c.date, u) >= 0) {
        return (omsi_ui::tr("Ill until %{date}").replace("%{date}", &day_label(u)), WARN);
    }
    if let Some(u) = e.holiday_until.as_deref().filter(|u| co::dates::between(&c.date, u) >= 0) {
        return (omsi_ui::tr("On holiday until %{date}").replace("%{date}", &day_label(u)), TEXT_SOFT);
    }
    if let Some(u) = e.notice_until.as_deref() {
        return (omsi_ui::tr("Leaves after %{date}").replace("%{date}", &day_label(u)), DANGER.lighten(0.25));
    }
    if working {
        (omsi_ui::tr("Drives today").into_owned(), OK)
    } else if e.week_days >= staff::WEEK_DAYS {
        (omsi_ui::tr("Days off").into_owned(), TEXT_SOFT)
    } else {
        (omsi_ui::tr("Free today").into_owned(), TEXT_SOFT)
    }
}

fn head(ui: &mut Ui, r: Rect, labels: &[(&str, &str)]) {
    for (c, (t, tip)) in cols(r).iter().zip(labels) {
        ui.text_in(&omsi_ui::tr(t).to_uppercase(), Rect::new(c.x, c.y, c.w - 8.0, c.h), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Left);
        if !tip.is_empty() {
            ui.tooltip(Rect::new(c.x, c.y, c.w - 8.0, c.h), tip);
        }
    }
    ui.p().rect(Rect::new(r.x, r.bottom(), r.w, 1.0), HAIRLINE);
}

fn skills_text(s: &Skills) -> String {
    format!("{:.0} · {:.0} · {:.0}", s.driving, s.punctuality, s.service)
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let market = staff::applicants(&c);
    let labels = [omsi_ui::tr("Employees (%{n})").replace("%{n}", &c.staff.len().to_string()), omsi_ui::tr("Applicants (%{n})").replace("%{n}", &market.len().to_string()), omsi_ui::tr("Licences & training").into_owned()];
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut tab = l.company.people.tab;
    if l.ui.segmented("company-staff-tabs", Rect::new(area.x, area.y, 640.0f32.min(area.w), 40.0), &mut tab, &refs) {
        l.company.people.tab = tab;
    }
    // what the staff costs and what today asks
    let wages: i64 = c.staff.iter().map(staff::monthly_cost).sum();
    let duties: usize = l.company.plan.as_ref().map(|p| p.tours.iter().filter(|t| !t.by_player && !t.live && !t.tour.unplanned).map(|t| t.duties.len()).sum()).unwrap_or(0);
    let info = omsi_ui::tr("Wages %{amount} a month with the employer's share  ·  %{n} duties today").replace("%{amount}", &eur(wages)).replace("%{n}", &duties.to_string());
    l.ui.text_in(&info, Rect::new(area.x + 660.0, area.y, area.w - 660.0, 40.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Right);
    let body = Rect::new(area.x, area.y + 40.0 + 18.0, area.w, (area.h - 40.0 - 18.0 - 44.0).max(0.0));
    let foot = Rect::new(area.x, area.bottom() - 30.0, area.w, 30.0);
    if l.company.people.tab == 2 {
        training(l, body, &c);
        l.ui.text_in("A course begins today and the driver is away for its days. New drivers learn the fleet's models when they start; a model new to the company needs its type training first.", foot, kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    } else if l.company.people.tab == 1 {
        applicants(l, body, &c, market);
        let t = omsi_ui::tr("New applicants come on %{date}. Skills: driving · punctuality · service; a driver of little experience is a warning on an articulated bus or a double-decker.").replace("%{date}", &day_label(&co::network::next_monday(&c)));
        l.ui.text_in(&t, foot, kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    } else {
        employees(l, body, &c);
        l.ui.text_in("A driver works five days a week and has holidays and ill days: count about one and a half drivers for every duty of the day.", foot, kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    }
}

fn employees(l: &mut Launcher, area: Rect, c: &Company) {
    if c.staff.is_empty() {
        let h = l.ui.paragraph("Nobody works here yet. Every duty of a tour needs a driver: hire some among the week's applicants.", Vec2::new(area.x, area.y + 4.0), area.w.min(860.0), kit::BODY, Weight::Regular, TEXT_SOFT);
        let bw = Foot::width(&l.ui, "To the applicants", Some("groups"));
        if l.ui.button("company-to-applicants", Rect::new(area.x, area.y + h + 18.0, bw, 40.0), "To the applicants", Some("groups"), ButtonKind::Primary) {
            l.company.people.tab = 1;
        }
        return;
    }
    head(
        &mut l.ui,
        Rect::new(area.x + 8.0, area.y, area.w - 20.0, 20.0),
        &[("Name", ""), ("Experience", "0 - 100: grows with every day driven"), ("Licence", "D: every bus; D1: midibuses only. G: articulated, DD: double-deckers, E: electric buses"), ("Wage", "A month, before the employer's share"), ("Reliable", "How seldom they fall ill or come late"), ("Satisfaction", "Unhappy people leave: pay and hours count"), ("Today", ""), ("", "")],
    );
    let working: Vec<u32> = l.company.plan.as_ref().map(|p| p.tours.iter().filter(|t| !t.tour.unplanned).flat_map(|t| t.duties.iter()).filter_map(|d| d.driver).collect()).unwrap_or_default();
    let list = c.staff.clone();
    let mut action: Option<(u32, u8)> = None;
    let rows = Rect::new(area.x, area.y + 30.0, area.w, (area.h - 30.0).max(0.0));
    l.ui.scroll_area("company-staff", rows, &mut |ui, v| {
        let rh = 56.0;
        for (k, e) in list.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 12.0, rh - 4.0);
            if !ui.rect_visible(r) {
                continue;
            }
            ui.row(&format!("company-staff-row-{}", e.id), r, false);
            let cs = cols(Rect::new(r.x + 8.0, r.y, r.w - 8.0, r.h));
            ui.text_in(&e.name, Rect::new(cs[0].x, r.y + 5.0, cs[0].w - 8.0, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let since = omsi_ui::tr("%{age}, since %{date}").replace("%{age}", &e.age.to_string()).replace("%{date}", &day_label(&e.hired));
            ui.text_in(&since, Rect::new(cs[0].x, r.y + 29.0, cs[0].w - 8.0, 20.0), 13.0, Weight::Regular, TEXT_SOFT, Align::Left);
            ui.text_in(&format!("{:.0}", e.experience), Rect::new(cs[1].x, r.y, 34.0, r.h), kit::ROWS, Weight::Medium, TEXT, Align::Left);
            meter(ui, Rect::new(cs[1].x + 38.0, r.center().y - 3.0, cs[1].w - 54.0, 6.0), e.experience / 100.0, EARLY_SOFT);
            ui.text_in(&licence_line(e.licence, &e.endorsements), Rect::new(cs[2].x, cs[2].y, cs[2].w - 6.0, cs[2].h), kit::ROWS, Weight::Medium, TEXT, Align::Left);
            ui.tooltip(cs[2], &endorsements_tip(e.licence, &e.endorsements));
            ui.text_in(&eur(e.wage), cs[3], kit::ROWS, Weight::Medium, TEXT, Align::Left);
            ui.text_in(&format!("{:.0} %", e.reliability * 100.0), cs[4], kit::ROWS, Weight::Regular, TEXT, Align::Left);
            ui.text_in(&format!("{:.0}", e.satisfaction), Rect::new(cs[5].x, r.y, 34.0, r.h), kit::ROWS, Weight::Medium, grade(e.satisfaction), Align::Left);
            meter(ui, Rect::new(cs[5].x + 38.0, r.center().y - 3.0, cs[5].w - 54.0, 6.0), e.satisfaction / 100.0, grade(e.satisfaction));
            let (status, colour) = status_of(c, e, working.contains(&e.id));
            ui.text_in(&status, Rect::new(cs[6].x, r.y, cs[6].w - 8.0, r.h), kit::NOTE + 0.5, Weight::Medium, colour, Align::Left);
            let a = cs[7];
            if ui.icon_button(&format!("company-raise-{}", e.id), Vec2::new(a.x + 18.0, r.center().y), 17.0, "trending_up", "A raise of 5 %") {
                action = Some((e.id, 0));
            }
            if e.notice_until.is_some() {
                if !e.resigned && ui.icon_button(&format!("company-keep-{}", e.id), Vec2::new(a.x + 58.0, r.center().y), 17.0, "restart_alt", "Take the dismissal back") {
                    action = Some((e.id, 2));
                }
            } else if ui.icon_button(&format!("company-dismiss-{}", e.id), Vec2::new(a.x + 58.0, r.center().y), 17.0, "logout", "Dismiss: they work their notice and leave") {
                action = Some((e.id, 1));
            }
        }
        list.len() as f32 * rh
    });
    match action {
        Some((id, 0)) => {
            act(l, |c| {
                staff::raise(c, id, 0.05);
                Ok(())
            });
        }
        Some((id, 1)) => l.company.dialog = Some(Dialog::Confirm { what: Confirm::Dismiss(id) }),
        Some((id, _)) => {
            act(l, |c| staff::withdraw_notice(c, id));
        }
        None => {}
    }
}

/// A column of the training matrix: a licence or endorsement, a model's type training, or one
/// of the staff's courses (`training::CourseKind::STAFF`).
#[derive(Clone, PartialEq)]
enum Col {
    Licence(CourseKind),
    Type(String),
    Course(CourseKind),
}

impl Col {
    fn kind(&self) -> CourseKind {
        match self {
            Col::Licence(k) | Col::Course(k) => *k,
            Col::Type(_) => CourseKind::TypeTraining,
        }
    }

    fn subject(&self) -> &str {
        match self {
            Col::Type(t) => t,
            _ => "",
        }
    }

    fn title(&self, c: &Company) -> String {
        match self {
            Col::Type(t) => licences::type_name(c, t),
            _ => omsi_ui::tr(self.kind().label()).into_owned(),
        }
    }

    fn cost(&self, c: &Company) -> co::Cents {
        match self {
            Col::Course(k) => training::cost_of(c, *k),
            _ => licences::course_cost(c, self.kind()),
        }
    }

    fn has(&self, c: &Company, e: &Employee) -> bool {
        match self {
            Col::Course(k) => training::trained(c, Some(e.id), *k),
            _ => licences::has(e, self.kind(), self.subject()),
        }
    }

    fn booked<'a>(&self, c: &'a Company, e: &Employee) -> Option<&'a training::Course> {
        match self {
            Col::Course(k) => training::course_of(c, Some(e.id), *k).filter(|x| !x.done),
            _ => licences::booked(c, e.id, self.kind(), self.subject()),
        }
    }
}

/// The columns of a group of the matrix: 0 the licences, 1 the fleet's models, 2 the courses.
fn columns(c: &Company, group: usize) -> Vec<Col> {
    match group {
        0 => CourseKind::LICENCES.iter().map(|k| Col::Licence(*k)).collect(),
        1 => licences::fleet_types(c).into_iter().map(Col::Type).collect(),
        _ => CourseKind::STAFF.iter().map(|k| Col::Course(*k)).collect(),
    }
}

/// A course's cost and days.
fn course_terms(cost: co::Cents, k: CourseKind) -> String {
    let days = k.spec().days;
    let d = if days == 1 { omsi_ui::tr("1 day").into_owned() } else { omsi_ui::tr("%{n} days").replace("%{n}", &days.to_string()) };
    format!("{}  ·  {}", eur(cost), d)
}

/// The licences, the type trainings and the courses: a row for each driver, a column for
/// each licence, each model of the fleet or each course; booked per driver, or for all who
/// lack it.
fn training(l: &mut Launcher, area: Rect, c: &Company) {
    if c.staff.is_empty() {
        l.ui.paragraph("Nobody works here yet. Every duty of a tour needs a driver: hire some among the week's applicants.", Vec2::new(area.x, area.y + 4.0), area.w.min(860.0), kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let groups = [omsi_ui::tr("Licences").into_owned(), omsi_ui::tr("Bus types").into_owned(), omsi_ui::tr("Courses").into_owned()];
    let refs: Vec<&str> = groups.iter().map(String::as_str).collect();
    let mut group = l.company.people.group;
    if l.ui.segmented("company-training-groups", Rect::new(area.x, area.y, 420.0f32.min(area.w), 34.0), &mut group, &refs) {
        l.company.people.group = group;
        l.company.people.first = 0;
    }
    let area = Rect::new(area.x, area.y + 50.0, area.w, (area.h - 50.0).max(0.0));
    let all = columns(c, l.company.people.group);
    if all.is_empty() {
        l.ui.text_in("The fleet has no buses yet.", Rect::new(area.x, area.y, area.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    }
    let name_w = (area.w * 0.2).clamp(170.0, 240.0);
    let fit = (((area.w - name_w - 12.0) / 150.0).floor() as usize).max(1);
    let per = fit.min(all.len());
    let first = l.company.people.first.min(all.len() - per);
    let shown: Vec<Col> = all[first..first + per].to_vec();
    let col_w = (area.w - name_w - 12.0) / per as f32;
    // (who stays)
    let people: Vec<Employee> = c.staff.iter().filter(|e| e.notice_until.is_none()).cloned().collect();
    let mut book: Option<(Col, Option<u32>)> = None;
    let mut lock: Option<CourseKind> = None;
    // the heads: the course, what it costs, how many have it, and "Train all"
    let head_h = 100.0;
    if all.len() > per {
        let t = omsi_ui::tr("%{a}-%{b} of %{n}").replace("%{a}", &(first + 1).to_string()).replace("%{b}", &(first + per).to_string()).replace("%{n}", &all.len().to_string());
        l.ui.text_in(&t, Rect::new(area.x + 4.0, area.y + 60.0, name_w - 90.0, 24.0), kit::NOTE, Weight::Medium, TEXT_DIM, Align::Left);
        if first > 0 && l.ui.icon_button("company-training-prev", Vec2::new(area.x + name_w - 64.0, area.y + 72.0), 15.0, "chevron_left", "Earlier columns") {
            l.company.people.first = first.saturating_sub(per);
        }
        if first + per < all.len() && l.ui.icon_button("company-training-next", Vec2::new(area.x + name_w - 28.0, area.y + 72.0), 15.0, "chevron_right", "More columns") {
            l.company.people.first = (first + per).min(all.len() - per);
        }
    }
    kit::caps(&mut l.ui, Rect::new(area.x + 4.0, area.y, name_w - 12.0, 16.0), "Driver");
    let g = l.company.people.group;
    for (k, col) in shown.iter().enumerate() {
        let kind = col.kind();
        let x = area.x + name_w + k as f32 * col_w;
        let open = co::levels::unlocked(c, kind.spec().feature);
        let icon = if matches!(col, Col::Type(_)) { "directions_bus" } else { kind.icon() };
        l.ui.icon(icon, Vec2::new(x + 9.0, area.y + 9.0), 15.0, if open { accent_2() } else { TEXT_FAINT });
        l.ui.text_in(&col.title(c), Rect::new(x + 22.0, area.y, col_w - 28.0, 18.0), 14.0, Weight::Bold, TEXT, Align::Left);
        let terms = course_terms(col.cost(c), kind);
        let under = if matches!(col, Col::Type(_)) { format!("{}  ·  {terms}", omsi_ui::tr("Type training")) } else { terms };
        l.ui.text_in(&under, Rect::new(x + 4.0, area.y + 20.0, col_w - 10.0, 16.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
        let have = people.iter().filter(|e| col.has(c, e)).count();
        let count = omsi_ui::tr("%{n} of %{m} have it").replace("%{n}", &have.to_string()).replace("%{m}", &people.len().to_string());
        l.ui.text_in(&count, Rect::new(x + 4.0, area.y + 38.0, col_w - 10.0, 16.0), kit::NOTE, Weight::Medium, if have == people.len() { OK } else { TEXT_SOFT }, Align::Left);
        l.ui.tooltip(Rect::new(x, area.y, col_w, 56.0), &omsi_ui::tr(kind.effect()));
        let b = Rect::new(x + 2.0, area.y + 62.0, (col_w - 12.0).min(140.0), 28.0);
        if !open {
            let t = omsi_ui::tr("From level %{n}").replace("%{n}", &kind.spec().feature.level().to_string());
            if l.ui.button(&format!("company-train-lock-{g}-{k}-{first}"), b, &t, Some("lock"), ButtonKind::Ghost) {
                lock = Some(kind);
            }
        } else if have < people.len() && l.ui.button(&format!("company-train-all-{g}-{k}-{first}"), b, "Train all", Some("groups"), ButtonKind::Normal) {
            book = Some((col.clone(), None));
        }
    }
    l.ui.p().rect(Rect::new(area.x, area.y + head_h, area.w, 1.0), HAIRLINE);
    let rows = Rect::new(area.x, area.y + head_h + 6.0, area.w, (area.h - head_h - 6.0).max(0.0));
    let cc = c.clone();
    l.ui.scroll_area("company-licences", rows, &mut |ui, v| {
        let rh = 40.0;
        for (i, e) in people.iter().enumerate() {
            let row = Rect::new(v.x, v.y + i as f32 * rh, v.w - 12.0, rh - 2.0);
            if !ui.rect_visible(row) {
                continue;
            }
            ui.text_in(&e.name, Rect::new(row.x + 4.0, row.y, name_w - 70.0, row.h), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            ui.text_in(&licence_line(e.licence, &e.endorsements), Rect::new(row.x + name_w - 66.0, row.y, 60.0, row.h), kit::NOTE, Weight::Medium, TEXT_SOFT, Align::Left);
            let away = e.absent(&cc.date);
            for (k, col) in shown.iter().enumerate() {
                let kind = col.kind();
                let cell = Rect::new(row.x + name_w + k as f32 * col_w, row.y + 4.0, col_w - 10.0, row.h - 8.0);
                if col.has(&cc, e) {
                    ui.icon("check_circle", Vec2::new(cell.x + 10.0, cell.center().y), 15.0, OK);
                    continue;
                }
                match col.booked(&cc, e) {
                    Some(x) => {
                        ui.icon("schedule", Vec2::new(cell.x + 10.0, cell.center().y), 15.0, accent_2());
                        let t = omsi_ui::tr("Until %{date}").replace("%{date}", &day_label(&x.until));
                        ui.text_in(&t, Rect::new(cell.x + 24.0, cell.y, cell.w - 24.0, cell.h), kit::NOTE, Weight::Medium, accent_2(), Align::Left);
                    }
                    None if !co::levels::unlocked(&cc, kind.spec().feature) => {
                        ui.icon("lock", Vec2::new(cell.x + 10.0, cell.center().y), 14.0, TEXT_FAINT);
                    }
                    None if e.licence == Licence::D1 && matches!(kind, CourseKind::ArticulatedLicence | CourseKind::DoubleDeckerLicence) => {
                        ui.text_in("D licence first", Rect::new(cell.x + 4.0, cell.y, cell.w, cell.h), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
                    }
                    None if away => {
                        ui.text_in("Away", Rect::new(cell.x + 4.0, cell.y, cell.w, cell.h), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
                    }
                    None => {
                        if ui.button(&format!("company-train-{}-{g}-{k}-{first}", e.id), Rect::new(cell.x, cell.y, cell.w.min(110.0), cell.h), "Book", Some("add"), ButtonKind::Ghost) {
                            book = Some((col.clone(), Some(e.id)));
                        }
                    }
                }
            }
            ui.p().rect(Rect::new(row.x, row.bottom(), row.w, 1.0), HAIRLINE);
        }
        people.len() as f32 * rh
    });
    if let Some(kind) = lock {
        kit::show(l, kit::locked(c, kind.spec().feature));
    }
    let Some((col, who)) = book else { return };
    let kind = col.kind();
    let course = if matches!(col, Col::Type(_)) { format!("{} {}", omsi_ui::tr("Type training"), col.title(c)) } else { col.title(c) };
    let subject = col.subject().to_string();
    let staff_course = matches!(col, Col::Course(_));
    match who {
        Some(id) => {
            let done = if staff_course { act(l, |c| training::enrol(c, kind, Some(id))) } else { act(l, |c| licences::enrol(c, id, kind, &subject)) };
            if let Some(until) = done {
                l.state.set_status(omsi_ui::tr("Booked: %{course}, until %{date}.").replace("%{course}", &course).replace("%{date}", &day_label(&until)), false);
            }
        }
        None => {
            let cost = col.cost(c);
            if c.cash < cost {
                kit::show(l, kit::no_cash(c, cost));
                return;
            }
            let done = if staff_course { act(l, |c| Ok(training::enrol_all(c, kind))) } else { act(l, |c| Ok(licences::enrol_all(c, kind, &subject))) };
            let Some((n, spent)) = done else { return };
            if n == 0 {
                kit::show(l, kit::Popup::new("groups", "Nobody could be booked", omsi_ui::tr("Who lacks it is away today, on another course or leaving the company - or needs the D licence first."), "", None));
            } else {
                let t = omsi_ui::tr("Booked for %{n} drivers: %{course}, %{amount} together.").replace("%{n}", &n.to_string()).replace("%{course}", &course).replace("%{amount}", &eur(spent));
                l.state.set_status(t, false);
            }
        }
    }
}

fn applicants(l: &mut Launcher, area: Rect, c: &Company, market: Vec<Applicant>) {
    if market.is_empty() {
        l.ui.text_in("Nobody else applies this week.", Rect::new(area.x, area.y, area.w, 26.0), kit::BODY, Weight::Medium, TEXT_SOFT, Align::Left);
        return;
    }
    head(
        &mut l.ui,
        Rect::new(area.x + 8.0, area.y, area.w - 20.0, 20.0),
        &[("Name", ""), ("Experience", "0 - 100: grows with every day driven"), ("Licence", "D: every bus; D1: midibuses only. G: articulated, DD: double-deckers, E: electric buses"), ("Asks", "The wage a month they ask, more for each endorsement"), ("Reliable", "How seldom they fall ill or come late"), ("Skills", "Driving · punctuality · service"), ("Costs a month", "The wage with the employer's share"), ("", "")],
    );
    let mut hire: Option<usize> = None;
    let rows = Rect::new(area.x, area.y + 30.0, area.w, (area.h - 30.0).max(0.0));
    l.ui.scroll_area("company-applicants", rows, &mut |ui, v| {
        let rh = 56.0;
        for (k, a) in market.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 12.0, rh - 4.0);
            if !ui.rect_visible(r) {
                continue;
            }
            ui.row(&format!("company-applicant-row-{}", a.no), r, false);
            let cs = cols(Rect::new(r.x + 8.0, r.y, r.w - 8.0, r.h));
            ui.text_in(&a.name, Rect::new(cs[0].x, r.y + 5.0, cs[0].w - 8.0, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            ui.text_in(&omsi_ui::tr("%{n} years old").replace("%{n}", &a.age.to_string()), Rect::new(cs[0].x, r.y + 29.0, cs[0].w - 8.0, 20.0), 13.0, Weight::Regular, TEXT_SOFT, Align::Left);
            ui.text_in(&format!("{:.0}", a.experience), Rect::new(cs[1].x, r.y, 34.0, r.h), kit::ROWS, Weight::Medium, TEXT, Align::Left);
            meter(ui, Rect::new(cs[1].x + 38.0, r.center().y - 3.0, cs[1].w - 54.0, 6.0), a.experience / 100.0, EARLY_SOFT);
            let lic = licence_line(a.licence, &a.endorsements);
            ui.text_in(&lic, Rect::new(cs[2].x, cs[2].y, cs[2].w - 6.0, cs[2].h), kit::ROWS, Weight::Medium, if a.licence == Licence::D1 { WARN } else { TEXT }, Align::Left);
            ui.tooltip(cs[2], &endorsements_tip(a.licence, &a.endorsements));
            ui.text_in(&eur(a.wage), cs[3], kit::ROWS, Weight::Medium, TEXT, Align::Left);
            ui.text_in(&format!("{:.0} %", a.reliability * 100.0), cs[4], kit::ROWS, Weight::Regular, TEXT, Align::Left);
            ui.text_in(&skills_text(&a.skills), cs[5], kit::ROWS, Weight::Regular, TEXT, Align::Left);
            ui.text_in(&eur(co::economy::employer_cost(a.wage)), cs[6], kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Left);
            if ui.button(&format!("company-hire-{}", a.no), Rect::new(cs[7].x, r.y + 7.0, cs[7].w.min(120.0), r.h - 14.0), "Hire", None, ButtonKind::Normal) {
                hire = Some(k);
            }
        }
        market.len() as f32 * rh
    });
    if let Some(k) = hire {
        let a = market[k].clone();
        if act(l, |c| staff::hire(c, &a)).is_some() {
            l.state.set_status(omsi_ui::tr("%{name} works for %{company} from today.").replace("%{name}", &a.name).replace("%{company}", &c.name), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_launcher_lib::company::dealer::Extra;
    use omsi_launcher_lib::company::plan::Problem;

    #[test]
    fn the_licences_are_translated() {
        let mut keys: Vec<&str> = Endorsement::ALL.iter().flat_map(|e| [e.label(), e.lacking()]).collect();
        let courses = CourseKind::LICENCES.iter().chain([CourseKind::TypeTraining].iter());
        keys.extend(courses.flat_map(|k| [k.label(), k.effect()]));
        keys.extend([Problem::NoEndorsement.label(), Problem::NoTypeTraining.label(), Extra::Introduction.label(), "They need the D licence first.", "Which bus is the training for?"]);
        // (the staff's courses, and what opens the defensive driving)
        keys.extend(CourseKind::STAFF.iter().flat_map(|k| [k.label(), k.effect()]));
        keys.extend([co::levels::Feature::SafetyCourse.label(), "Licences", "Bus types", "Courses", "Train the drivers", "Bus %{bus} had an accident yesterday: damage %{amount}"]);
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in &keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
    }
}
