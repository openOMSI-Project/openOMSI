//! The company's lines: those it runs, with today's tours and how they are covered (a bus for
//! each tour, a driver for each duty - as planned: only what is planned runs), and the lines
//! it can add - the map's timetable lines and the player's own from the line editor (the
//! switch of `ownlines`). Adding one shows first what it needs against what the company has
//! (`add_dialog`). "Make a new line" opens the line editor for the company
//! (`lineeditor::open_for_company`): a line made there is confirmed and paid for, and an own
//! line of the company's is changed there too. A line chosen shows itself in public above its
//! tours (`line_card`): its title, the depot file its buses carry, and its advertising.

use super::super::ownlines;
use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{act, line_plate, plate, section, Confirm, Dialog};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::company::{self as co, Company};
use omsi_launcher_lib::service::ServiceKind;
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct LinesView {
    selected: Option<String>,
    mine: bool,
    /// The public title being typed: for which line, and the text.
    title: Option<(String, String)>,
    /// The termini of the depot files looked at (`hof_termini`), by name (lower case).
    hofs: HashMap<String, Option<Arc<HashSet<String>>>>,
}

fn hhmm(minutes: i32) -> String {
    super::super::state::hhmm(minutes as f64 * 60.0)
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let gap = 16.0;
    // (what a line earns, under both columns)
    let r = co::economy::rules(c.difficulty);
    let pay = co::economy::compensation_per_km(&r, c.reputation, c.contract_index);
    let foot = omsi_ui::tr("A single ticket of the fare association costs %{single} (a passenger pays %{fare} on average, with day and season tickets); the authority pays %{km} for every kilometre run, more the better your reputation.").replace("%{single}", &super::eur_cents(co::fares::association_fare(&c) as f64)).replace("%{fare}", &super::eur_cents(r.fare as f64)).replace("%{km}", &super::eur_cents(pay));
    l.ui.text_in(&foot, Rect::new(area.x, area.bottom() - 24.0, area.w, 24.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
    let area = Rect::new(area.x, area.y, area.w, (area.h - 36.0).max(0.0));
    let left_w = ((area.w - gap) * 0.52).max(320.0);
    ours(l, Rect::new(area.x, area.y, left_w, area.h), &c);
    let right = Rect::new(area.x + left_w + gap, area.y, area.w - left_w - gap, area.h);
    match l.company.lines.selected.clone().filter(|s| c.lines.iter().any(|x| &x.name == s)) {
        Some(name) => tours_of(l, right, &c, &name),
        None => to_add(l, right, &c),
    }
}

/// Planning, with a line in view.
fn to_planning(l: &mut Launcher, line: &str) {
    l.company.tab = 5;
    l.company.planning.day = 0;
    l.company.planning.focus = Some(line.to_string());
}

/// The lines the company runs.
fn ours(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "The company's lines");
    if c.lines.is_empty() {
        l.ui.paragraph("The company runs no line yet. Add one of the map's lines, or one of your own from the line editor: every tour it has on a day is then the company's to plan and run.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let plan = l.company.plan.clone();
    let list = c.lines.clone();
    let selected = l.company.lines.selected.clone();
    let now = co::clock::now(c);
    let cc = c.clone();
    let mut pick = None;
    let mut remove = None;
    let mut edit = None;
    let mut planning = None;
    let mut fare = None;
    l.ui.scroll_area("company-lines", inner, &mut |ui, v| {
        let rh = 104.0;
        for (k, line) in list.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 8.0);
            if !ui.rect_visible(r) {
                continue;
            }
            let on = selected.as_deref() == Some(line.name.as_str());
            if ui.row(&format!("company-line-{}", line.name), r, on) {
                pick = Some(line.name.clone());
            }
            let ink = if on { on_accent() } else { TEXT };
            let w = line_plate(ui, Vec2::new(r.x + 12.0, r.y + 12.0), line, 26.0);
            // (its public title when it has one; where it goes in the tooltip then)
            let caption = if !line.title.trim().is_empty() { line.title.trim().to_string() } else if line.caption.is_empty() { line.name.clone() } else { line.caption.clone() };
            // the row's tools at its right: own, its fare, edit, stop
            let mut tx = r.right() - 14.0;
            let close_c = Vec2::new(tx - 14.0, r.y + 25.0);
            if ui.icon_button(&format!("company-line-remove-{}", line.name), close_c, 16.0, "close", "Stop running this line") {
                remove = Some(line.name.clone());
            }
            tx -= 38.0;
            if let Some(p) = line.plan.as_ref() {
                if ui.icon_button(&format!("company-line-edit-{}", line.name), Vec2::new(tx - 14.0, r.y + 25.0), 16.0, "route", "Edit route: change the line in the line editor") {
                    edit = Some(p.line_id);
                }
                tx -= 38.0;
            }
            let fare_t = co::fares::fare_of(&cc, line);
            let fl = format!("{} {}", omsi_ui::tr("Fare"), super::eur_cents(fare_t as f64));
            let fw = ui.width(&fl, 13.0, Weight::Bold) + 22.0;
            let fr = Rect::new(tx - fw, r.y + 11.0, fw, 28.0);
            if ui.button(&format!("company-line-fare-{}", line.name), fr, &fl, None, ButtonKind::Ghost) {
                fare = Some(line.name.clone());
            }
            ui.tooltip(fr, &omsi_ui::tr("The single ticket this line asks: change it, and see what it does to passengers and fares"));
            tx = fr.x - 8.0;
            // (a change waiting for its day)
            if let Some(p) = line.pending.as_ref() {
                let t = omsi_ui::tr("New timetable from %{date}").replace("%{date}", &super::day_label(&p.from));
                let tw = ui.width(&t, 12.0, Weight::Bold) + 16.0;
                let at = Vec2::new(tx - tw, r.y + 14.0);
                kit::tag(ui, at, &t, if on { on_accent() } else { WARN });
                ui.tooltip(Rect::new(at.x, at.y, tw, 22.0), &omsi_ui::tr("Saved in the line editor: today's tours run as they are; from that day the line runs its new timetable and its changed tours are planned anew."));
                tx = at.x - 8.0;
            }
            if line.own {
                // (its kind of service, a regular one "own line"; the buses it asks for in the
                // tooltip)
                let kind = co::ownline::kind_of(line);
                let t = if kind == ServiceKind::Regular { omsi_ui::tr("Own line") } else { omsi_ui::tr(kind.label()) };
                let tw = ui.width(&t, 12.0, Weight::Bold) + 16.0;
                let at = Vec2::new(tx - tw, r.y + 14.0);
                kit::tag(ui, at, &t, if on { on_accent() } else { accent_2() });
                let mut tip = omsi_ui::tr(if kind == ServiceKind::Regular { "Your own line, made in the line editor: no concession, a licence a month" } else { kind.note() }).into_owned();
                if let Some(v) = line.plan.as_ref().map(|p| &p.vehicles).filter(|v| !v.open()) {
                    tip.push_str(&format!("\n{}: {}", omsi_ui::tr("Buses on this line"), v.summary(&|s| omsi_ui::tr(s).into_owned())));
                }
                ui.tooltip(Rect::new(at.x, at.y, tw, 22.0), &tip);
                tx = at.x - 8.0;
            }
            ui.text_in(&caption, Rect::new(r.x + w + 24.0, r.y + 10.0, (tx - r.x - w - 30.0).max(30.0), 30.0), kit::ROWS, Weight::Bold, ink, Align::Left);
            // how it is covered today, as planned
            let in_service = co::network::in_service(line, now);
            let (covered, all) = plan.as_ref().map(|p| p.planned_of(&line.name)).unwrap_or((0, line.tours as usize));
            let frac = if all > 0 { covered as f64 / all as f64 } else { 0.0 };
            let (label, right, tip) = if !in_service {
                let waits = line.service_from.is_some_and(|s| s > now);
                (
                    if waits { omsi_ui::tr("In service from %{when}").replace("%{when}", &line.service_from.map(|s| format!("{} {}", super::day_label(&co::clock::date_of(s)), co::clock::hhmm(s))).unwrap_or_default()) } else { omsi_ui::tr("Not planned yet: %{c} of %{t} tours planned").replace("%{c}", &covered.to_string()).replace("%{t}", &all.to_string()) },
                    omsi_ui::tr("Plan it").into_owned(),
                    omsi_ui::tr("The line runs only once it is planned and its service is started. Click for the planning.").into_owned(),
                )
            } else {
                let open = all - covered;
                (
                    omsi_ui::tr("Covered today: %{c} of %{t} tours").replace("%{c}", &covered.to_string()).replace("%{t}", &all.to_string()),
                    if all > 0 { format!("{:.0} %", frac * 100.0) } else { String::new() },
                    if open == 0 { omsi_ui::tr("Every tour of today has its bus and drivers. Click for the planning.").into_owned() } else { omsi_ui::tr("%{n} tours of today lack a bus or a driver: they are dropped, with the contract's penalty. Click for the planning.").replace("%{n}", &open.to_string()) },
                )
            };
            let colour = kit::share_colour(frac, !in_service);
            let br = Rect::new(r.x + 14.0, r.y + 48.0, r.w * 0.66, kit::BAR_H);
            if kit::bar(ui, &format!("company-line-bar-{}", line.name), br, frac, colour, &label, &right, &tip) {
                planning = Some(line.name.clone());
            }
            let km = omsi_ui::tr("%{km} km a day").replace("%{km}", &super::grouped(line.km));
            ui.text_in(&km, Rect::new(br.right() + 16.0, br.y, r.right() - br.right() - 30.0, 20.0), kit::NOTE, Weight::Regular, if on { on_accent() } else { TEXT_DIM }, Align::Right);
        }
        list.len() as f32 * rh
    });
    if let Some(id) = edit {
        super::super::lineeditor::open_for_company(l, Some(id));
    } else if let Some(name) = remove {
        l.company.dialog = Some(Dialog::Confirm { what: Confirm::RemoveLine(name) });
    } else if let Some(name) = planning {
        to_planning(l, &name);
    } else if let Some(name) = fare {
        let f = c.lines.iter().find(|x| x.name == name).map(|x| co::fares::fare_of(c, x)).unwrap_or(0);
        l.company.dialog = Some(Dialog::Fare { line: name, fare: f as f32 / 100.0 });
    } else if let Some(name) = pick {
        l.company.lines.selected = if selected.as_deref() == Some(name.as_str()) { None } else { Some(name) };
    }
}

/// A line's tours today: the bus, the drivers, and whether it runs - as planned.
fn tours_of(l: &mut Launcher, r: Rect, c: &Company, name: &str) {
    let Some(line) = c.lines.iter().find(|x| x.name == name).cloned() else { return };
    // the line in public above, its tours of today under it
    let card_h = 214.0f32.min(r.h * 0.45);
    line_card(l, Rect::new(r.x, r.y, r.w, card_h), c, &line);
    let r = Rect::new(r.x, r.y + card_h + 12.0, r.w, (r.h - card_h - 12.0).max(80.0));
    let title = omsi_ui::tr("Line %{n} today").replace("%{n}", &line.number);
    let inner = section(&mut l.ui, r, &title);
    let pw = Foot::width(&l.ui, "Open the planning", Some("event"));
    if l.ui.button("company-line-plan", Rect::new(r.right() - pw - 12.0, r.y + 8.0, pw, 32.0), "Open the planning", Some("event"), ButtonKind::Ghost) {
        to_planning(l, name);
    }
    let aw = Foot::width(&l.ui, "Add lines", Some("add"));
    if l.ui.button("company-line-back", Rect::new(r.right() - pw - aw - 20.0, r.y + 8.0, aw, 32.0), "Add lines", Some("add"), ButtonKind::Ghost) {
        l.company.lines.selected = None;
    }
    let Some(plan) = l.company.plan.clone() else {
        l.ui.text_in("Reading the timetable…", Rect::new(inner.x, inner.y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    };
    let tours: Vec<co::day::TourPlan> = plan.tours.into_iter().filter(|t| t.tour.line.eq_ignore_ascii_case(name)).collect();
    if tours.is_empty() {
        l.ui.paragraph("The line has no tour on this day (its timetable runs on other days).", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let fleet: Vec<(u32, String)> = c.fleet.iter().map(|v| (v.id, v.number.clone())).collect();
    let staff: Vec<(u32, String)> = c.staff.iter().map(|e| (e.id, e.name.clone())).collect();
    let cc = c.clone();
    l.ui.scroll_area("company-line-tours", inner, &mut |ui, v| {
        let rh = 58.0;
        for (k, t) in tours.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 4.0);
            if !ui.rect_visible(r) {
                continue;
            }
            ui.p().rect(Rect::new(r.x, r.bottom(), r.w, 1.0), HAIRLINE);
            let head = format!("{} {}", omsi_ui::tr("Tour"), t.tour.tour);
            ui.text_in(&head, Rect::new(r.x, r.y + 4.0, 120.0, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let mut when = format!("{} – {}  ·  {:.0} km", hhmm(t.tour.from()), hhmm(t.tour.to()), t.tour.km());
            // (the size of bus the tour asks for)
            if let Some(size) = co::ownline::wanted(&cc, &t.tour) {
                when.push_str("  ·  ");
                when.push_str(&omsi_ui::tr(co::BusKind { size, drive: co::Drive::Diesel }.label()));
            }
            ui.text_in(&when, Rect::new(r.x, r.y + 28.0, 250.0, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
            let bus = t.bus.and_then(|b| fleet.iter().find(|f| f.0 == b)).map(|f| omsi_ui::tr("Bus %{n}").replace("%{n}", &f.1));
            let drivers: Vec<String> = t.duties.iter().filter_map(|d| d.driver.and_then(|id| staff.iter().find(|s| s.0 == id)).map(|s| s.1.clone())).collect();
            let (what, colour) = if t.by_player {
                (omsi_ui::tr("Driven by you").into_owned(), accent_2())
            } else if t.live {
                (omsi_ui::tr("Reported by the game").into_owned(), accent_2())
            } else if t.tour.unplanned && t.covered() {
                (omsi_ui::tr("Planned - the line is not in service yet").into_owned(), TEXT_SOFT)
            } else if t.tour.unplanned {
                (omsi_ui::tr("Not planned - the line is not in service yet").into_owned(), TEXT_DIM)
            } else if t.bus.is_none() {
                (omsi_ui::tr("No bus planned: dropped").into_owned(), DANGER.lighten(0.25))
            } else if drivers.len() < t.duties.len() {
                (omsi_ui::tr("%{n} duties without a driver").replace("%{n}", &(t.duties.len() - drivers.len()).to_string()), WARN)
            } else {
                (omsi_ui::tr("Covered").into_owned(), OK)
            };
            let x = r.x + 260.0;
            let mut who = bus.unwrap_or_default();
            if !drivers.is_empty() {
                if !who.is_empty() {
                    who.push_str("  ·  ");
                }
                who.push_str(&drivers.join(", "));
            }
            ui.text_in(&who, Rect::new(x, r.y + 4.0, r.right() - x, 24.0), kit::ROWS, Weight::Medium, TEXT_SOFT, Align::Left);
            ui.text_in(&what, Rect::new(x, r.y + 28.0, r.right() - x, 20.0), kit::NOTE, Weight::Bold, colour, Align::Left);
        }
        tours.len() as f32 * rh
    });
}

/// The termini a depot file can show (folded as the bus step compares them: `buspick::fold`);
/// None: the file was not found beside any bus. Read once a depot file.
fn hof_termini(v: &mut LinesView, hof: &str, map: &str) -> Option<Arc<HashSet<String>>> {
    let key = hof.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    v.hofs
        .entry(key)
        .or_insert_with(|| {
            let (bases, _) = core::depot_roots();
            core::linehof::first_depot(hof.trim(), &bases, &core::lines::map_folder(map)).map(|d| Arc::new(super::super::buspick::depot_of(&d.hof).termini))
        })
        .clone()
}

/// The line in public (Luc: "Shuttleverkehr Altenfeld - Wurzbach" bovenaan, in de reclame): its
/// title - on its card in the list and in its advertising -, the depot file the company's buses
/// carry on it (the map's, or another of the installed buses'), whether that file knows where
/// the line's trips go, and the line as its advertising shows it.
fn line_card(l: &mut Launcher, r: Rect, c: &Company, line: &co::CompanyLine) {
    let inner = section(&mut l.ui, r, "The line in public");
    let half = (inner.w - 20.0) * 0.5;
    let x2 = inner.x + half + 20.0;
    // the title, kept when "Save" is pressed
    if l.company.lines.title.as_ref().is_none_or(|t| t.0 != line.name) {
        l.company.lines.title = Some((line.name.clone(), line.title.clone()));
    }
    let mut text = l.company.lines.title.as_ref().map(|t| t.1.clone()).unwrap_or_default();
    kit::caps(&mut l.ui, Rect::new(inner.x, inner.y - 4.0, half, 16.0), "Public title");
    let bw = 70.0;
    let dirty = text.trim() != line.title.trim();
    let tw = if dirty { half - bw - 6.0 } else { half };
    let hint = if line.caption.trim().is_empty() { "Shuttleverkehr Altenfeld - Wurzbach" } else { line.caption.trim() };
    l.ui.text_input("company-line-title", Rect::new(inner.x, inner.y + 16.0, tw, 34.0), &mut text, hint, None);
    let mut save = false;
    if dirty && l.ui.button("company-line-title-save", Rect::new(inner.x + half - bw, inner.y + 16.0, bw, 34.0), "Save", None, ButtonKind::Primary) {
        save = true;
    }
    if let Some(t) = l.company.lines.title.as_mut() {
        t.1 = text.clone();
    }
    // the depot file
    kit::caps(&mut l.ui, Rect::new(inner.x, inner.y + 60.0, half, 16.0), "Depot file of its buses");
    let depots: Vec<String> = l.state.maps.iter().find(|m| m.file.eq_ignore_ascii_case(&c.map)).map(|m| super::wizard::depots_for(m, &l.state.vehicles)).unwrap_or_default();
    let mut options = vec![omsi_ui::tr("The company's: %{hof}").replace("%{hof}", &c.depot)];
    options.extend(depots.iter().filter(|d| !d.eq_ignore_ascii_case(&c.depot)).cloned());
    let mut k = if line.hof.trim().is_empty() { 0 } else { options.iter().skip(1).position(|o| o.eq_ignore_ascii_case(line.hof.trim())).map(|i| i + 1).unwrap_or(0) };
    let mut hof_pick = None;
    if l.ui.select("company-line-hof", Rect::new(inner.x, inner.y + 80.0, half, 34.0), &mut k, &options) {
        hof_pick = Some(if k == 0 { String::new() } else { options[k].clone() });
    }
    // whether it knows where the line goes
    let hof = line.hof_or(&c.depot).to_string();
    let termini: Vec<String> = super::map_lines(&l.company).and_then(|ls| ls.iter().find(|x| x.name.eq_ignore_ascii_case(&line.name))).map(|x| x.termini.clone()).unwrap_or_default();
    let known = hof_termini(&mut l.company.lines, &hof, &c.map);
    let (words, colour) = match &known {
        None => (omsi_ui::tr("The depot file %{hof} was not found beside any bus.").replace("%{hof}", &hof), WARN),
        Some(set) => {
            let missing: Vec<&String> = termini.iter().filter(|t| !set.contains(&super::super::buspick::fold(t))).collect();
            if termini.is_empty() {
                (String::new(), TEXT_DIM)
            } else if missing.is_empty() {
                (omsi_ui::tr("It knows every destination of the line.").into_owned(), OK)
            } else {
                let list = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
                (omsi_ui::tr("Not in it: %{list}. Own lines bring theirs when saved in the line editor.").replace("%{list}", &list), WARN)
            }
        }
    };
    l.ui.paragraph(&words, Vec2::new(inner.x, inner.y + 122.0), half, kit::NOTE, Weight::Regular, colour);
    // as its advertising shows it
    kit::caps(&mut l.ui, Rect::new(x2, inner.y - 4.0, half, 16.0), "As advertised");
    let ad = Rect::new(x2, inner.y + 16.0, half, (inner.bottom() - inner.y - 16.0).max(60.0));
    l.ui.p().rounded(ad, RADIUS, super::super::ownlines::colour_of(if line.colour.trim().is_empty() { &c.colours[0] } else { &line.colour }).alpha(0.16));
    let pw = super::line_plate(&mut l.ui, Vec2::new(ad.x + 14.0, ad.y + 14.0), line, 30.0);
    let shown = if text.trim().is_empty() { line.public_name().to_string() } else { text.trim().to_string() };
    let title_h = l.ui.paragraph(&shown, Vec2::new(ad.x + 24.0 + pw, ad.y + 10.0), ad.w - pw - 38.0, 17.0, Weight::Bold, TEXT);
    let mut under: Vec<String> = Vec::new();
    if !line.title.trim().is_empty() && !line.caption.trim().is_empty() {
        under.push(line.caption.clone());
    }
    let kind = co::ownline::kind_of(line);
    if kind != ServiceKind::Regular {
        under.push(omsi_ui::tr(kind.label()).into_owned());
    }
    under.push(c.name.clone());
    l.ui.paragraph(&under.join("  ·  "), Vec2::new(ad.x + 24.0 + pw, ad.y + 14.0 + title_h), ad.w - pw - 38.0, kit::NOTE, Weight::Regular, TEXT_SOFT);
    let name = line.name.clone();
    if let Some(h) = hof_pick {
        act(l, |c| {
            if let Some(x) = c.lines.iter_mut().find(|x| x.name == name) {
                x.hof = h;
            }
            Ok(())
        });
    }
    if save {
        act(l, |c| {
            if let Some(x) = c.lines.iter_mut().find(|x| x.name == name) {
                x.title = text.trim().to_string();
            }
            Ok(())
        });
    }
}

/// The lines that can be added: the map's or the player's own - or a new one, made in the
/// line editor for the company.
fn to_add(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Add a line");
    let mw = Foot::width(&l.ui, "Make a new line", Some("route"));
    let make = Rect::new(r.right() - mw - 12.0, r.y + 8.0, mw, 32.0);
    if l.ui.button("company-line-make", make, "Make a new line", Some("route"), ButtonKind::Ghost) {
        super::super::lineeditor::open_for_company(l, None);
        return;
    }
    l.ui.tooltip(make, "Draw a line of the company's own in the line editor: it shows what the line costs and brings, and you confirm and pay for it there");
    let Some(today) = l.company.today.as_ref() else {
        l.ui.text_in("Reading the timetable…", Rect::new(inner.x, inner.y, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    };
    if let Some(e) = &today.error {
        l.ui.paragraph(e, Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, WARN);
        return;
    }
    let all: Vec<core::LineInfo> = today.lines.clone();
    let own = l.company.own.clone();
    let (maps, mine) = ownlines::split(&all, &own);
    let showing_mine = ownlines::showing_mine(l.company.lines.mine, mine.len());
    match ownlines::switch(&mut l.ui, "company-lines-switch", Rect::new(inner.x, inner.y, inner.w.min(460.0), 40.0), l.company.lines.mine, (maps.len(), mine.len())) {
        ownlines::Switched::To(m) => l.company.lines.mine = m,
        ownlines::Switched::Hint => l.state.set_status(omsi_ui::tr(ownlines::NONE_YET).into_owned(), false),
        ownlines::Switched::No => {}
    }
    let list: Vec<core::LineInfo> = if showing_mine { mine.into_iter().cloned().collect() } else { maps.into_iter().cloned().collect() };
    let list: Vec<core::LineInfo> = list.into_iter().filter(|x| !c.lines.iter().any(|y| y.name.eq_ignore_ascii_case(&x.name))).collect();
    // (the map's depot runs, empty runs, test drives and specials are no lines of their own:
    // they go with the tours that need them, or are other contracts; nor is other operators'
    // traffic the company's - `specials`. The same rule as Apply's, `concessions::is_line`.)
    let n = list.len();
    let list: Vec<core::LineInfo> = list.into_iter().filter(|x| showing_mine || co::concessions::is_line(&c, x)).collect();
    let hidden = n - list.len();
    let rows = Rect::new(inner.x, inner.y + 40.0 + 14.0, inner.w, (inner.h - 40.0 - 14.0 - if hidden > 0 { 28.0 } else { 0.0 }).max(0.0));
    if hidden > 0 {
        let t = omsi_ui::tr("%{n} timetables of the map are not the company's to run: depot and empty runs (they go with the tours that need them), specials and other operators' traffic.").replace("%{n}", &hidden.to_string());
        l.ui.text_in(&t, Rect::new(inner.x, inner.bottom() - 22.0, inner.w, 22.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if list.is_empty() {
        l.ui.text_in("The company runs all of them already.", Rect::new(rows.x, rows.y, rows.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    }
    let mut add = None;
    // (on Realistic and Hard a map line is applied for: its concession's tender)
    let direct = co::concessions::may_add_directly(c);
    let depot = c.depot.clone();
    let bids: Vec<(String, Option<i64>)> = c.concessions.tenders.iter().filter(|t| t.open()).map(|t| (t.line.to_lowercase(), t.offers.last().map(|o| o.1))).collect();
    l.ui.scroll_area("company-lines-add", rows, &mut |ui, v| {
        let rh = 62.0;
        for (k, line) in list.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 6.0);
            if !ui.rect_visible(r) {
                continue;
            }
            ui.row(&format!("company-add-row-{}", line.name), r, false);
            let o = core::lines::own_line_of(&line.name, &own);
            let w = match &o {
                Some(o) => ownlines::plate(ui, Vec2::new(r.x + 10.0, r.y + 15.0), &o.number, &o.colour, 24.0),
                None => plate(ui, Vec2::new(r.x + 10.0, r.y + 15.0), &co::specials::number_of(line, &[&depot]), 24.0),
            };
            let caption = match &o {
                Some(o) => ownlines::caption_of(o, line),
                None => format!("{}  ·  {}", line.name, co::specials::caption_of(line, &[&depot])),
            };
            let (label, icon) = if direct || o.is_some() {
                ("Add…", "add")
            } else if bids.iter().find(|b| b.0 == line.name.to_lowercase()).is_some_and(|t| t.1.is_some()) {
                ("Bid made", "receipt_long")
            } else {
                ("Apply…", "receipt_long")
            };
            let bw = Foot::width(ui, label, Some(icon));
            ui.text_in(&caption, Rect::new(r.x + w + 22.0, r.y + 6.0, r.w - w - bw - 40.0, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let runs = line.tours.iter().filter(|t| t.runs).count();
            let km: f64 = line.tours.iter().filter(|t| t.runs).flat_map(|t| t.trips.iter()).map(|t| t.km).sum();
            let sub = omsi_ui::tr("%{n} tours today  ·  %{km} km").replace("%{n}", &runs.to_string()).replace("%{km}", &format!("{km:.0}"));
            ui.text_in(&sub, Rect::new(r.x + w + 22.0, r.y + 30.0, r.w - w - bw - 40.0, 20.0), kit::NOTE, Weight::Regular, TEXT_DIM, Align::Left);
            let br = Rect::new(r.right() - bw - 8.0, r.y + 10.0, bw, 36.0);
            if ui.button(&format!("company-add-{}", line.name), br, label, Some(icon), ButtonKind::Normal) {
                add = Some(k);
            }
            if !direct && o.is_none() {
                ui.tooltip(br, "A line of the map is run under a concession: bid in its tender, and the line is the company's if the bid wins");
            } else {
                ui.tooltip(br, "See what the line needs against what the company has, then add it");
            }
        }
        list.len() as f32 * rh
    });
    if let Some(k) = add {
        l.company.dialog = Some(Dialog::AddLine { name: list[k].name.clone() });
    }
}

/// Taking a line on: its tours, the buses at its busiest (by size) and the drivers it needs,
/// against what the company has beside its lines in service - with a warning when it is short.
/// The line comes in not planned: it runs once it is planned and its service started.
pub(super) fn add_dialog(l: &mut Launcher) {
    let Some(super::Dialog::AddLine { name }) = &l.company.dialog else { return };
    let name = name.clone();
    let Some(c) = l.company.company.clone() else { return };
    let Some(all) = l.company.today.as_ref().map(|t| t.lines.clone()) else {
        l.company.dialog = None;
        return;
    };
    let Some(line) = all.iter().find(|x| x.name == name).cloned() else {
        l.company.dialog = None;
        return;
    };
    let own = core::lines::own_line_of(&line.name, &l.company.own);
    let direct = co::concessions::may_add_directly(&c) || own.is_some();
    let number = own.as_ref().map(|o| o.number.clone()).unwrap_or_else(|| co::specials::number_of(&line, &[&c.depot]));
    let title = if direct { omsi_ui::tr("Add line %{n}") } else { omsi_ui::tr("Apply for line %{n}") }.replace("%{n}", &number);
    let n = co::network::needs(&c, &line, &all, &c.date);
    let f = kit::frame(l, 760.0, 600.0, "route", &title);
    let inner = f.body;
    let mut y = inner.y;
    let intro = if direct { "What the line asks of the company on a day like today, against what it has. It comes in not planned: it runs once its tours are planned and its service is started on the Planning page." } else { "What the line asks of the company on a day like today, against what it has. Its concession is auctioned: if your bid wins, the line comes in not planned - it runs once its tours are planned and its service is started." };
    y += l.ui.paragraph(intro, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT) + 18.0;
    let (short_b, short_d) = n.short();
    let row = |l: &mut Launcher, y: &mut f32, what: &str, need: String, have: String, ok: bool| {
        l.ui.text_in(what, Rect::new(inner.x, *y, inner.w * 0.42, 30.0), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Left);
        l.ui.text_in(&need, Rect::new(inner.x + inner.w * 0.42, *y, inner.w * 0.22, 30.0), kit::ROWS, Weight::Bold, TEXT, Align::Right);
        l.ui.text_in(&have, Rect::new(inner.x + inner.w * 0.64, *y, inner.w * 0.36, 30.0), kit::ROWS, Weight::Bold, if ok { OK } else { WARN }, Align::Right);
        l.ui.p().rect(Rect::new(inner.x, *y + 30.0, inner.w, 1.0), HAIRLINE);
        *y += 34.0;
    };
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w * 0.4, 16.0), "The line needs");
    l.ui.text_in(&omsi_ui::tr("Needs").to_uppercase(), Rect::new(inner.x + inner.w * 0.42, y, inner.w * 0.22, 16.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Right);
    l.ui.text_in(&omsi_ui::tr("The company has free").to_uppercase(), Rect::new(inner.x + inner.w * 0.64, y, inner.w * 0.36, 16.0), kit::CAPS, Weight::Bold, TEXT_DIM, Align::Right);
    y += 26.0;
    let runs = line.tours.iter().filter(|t| t.runs).count();
    row(l, &mut y, &omsi_ui::tr("Tours today"), runs.to_string(), String::new(), true);
    let free_b = n.have_buses.saturating_sub(n.busy_buses);
    row(l, &mut y, &omsi_ui::tr("Buses at its busiest"), n.buses.to_string(), omsi_ui::tr("%{n} of %{all}").replace("%{n}", &free_b.to_string()).replace("%{all}", &n.have_buses.to_string()), short_b == 0);
    for (size, k) in &n.sizes {
        let have = c.fleet.iter().filter(|v| v.held_on(&c.date) && v.kind.size == *size).count();
        row(l, &mut y, &format!("    {}", omsi_ui::tr(co::BusKind { size: *size, drive: co::Drive::Diesel }.label())), k.to_string(), have.to_string(), have >= *k);
    }
    let free_d = n.have_drivers.saturating_sub(n.busy_drivers);
    row(l, &mut y, &omsi_ui::tr("Drivers (%{n} duties, about one and a half a duty)").replace("%{n}", &n.duties.to_string()), n.drivers.to_string(), omsi_ui::tr("%{n} of %{all}").replace("%{n}", &free_d.to_string()).replace("%{all}", &n.have_drivers.to_string()), short_d == 0);
    y += 10.0;
    if short_b > 0 || short_d > 0 {
        let t = omsi_ui::tr("Short for this line: %{b} buses and %{d} drivers. Buy or rent buses and hire drivers before you start its service.").replace("%{b}", &short_b.to_string()).replace("%{d}", &short_d.to_string());
        l.ui.icon("warning", Vec2::new(inner.x + 10.0, y + 11.0), 20.0, WARN);
        l.ui.paragraph(&t, Vec2::new(inner.x + 30.0, y), inner.w - 30.0, kit::BODY, Weight::Medium, WARN);
    } else if n.buses > 0 {
        l.ui.icon("check_circle", Vec2::new(inner.x + 10.0, y + 11.0), 20.0, OK);
        l.ui.paragraph("The company has the buses and drivers for it.", Vec2::new(inner.x + 30.0, y), inner.w - 30.0, kit::BODY, Weight::Medium, OK);
    }
    let mut foot = Foot::new(&f);
    let label = if direct { "Add the line" } else { "Apply for its concession" };
    let go = foot.right(l, "company-addline-go", label, Some(if direct { "add" } else { "receipt_long" }), ButtonKind::Primary);
    if foot.right(l, "company-addline-cancel", "Cancel", None, ButtonKind::Normal) || f.close {
        l.company.dialog = None;
        return;
    }
    if short_b > 0 && foot.left(l, "company-addline-dealer", "To the dealer", Some("directions_bus"), ButtonKind::Normal) {
        super::go(l, kit::Go::Dealer);
        return;
    }
    if short_d > 0 && foot.left(l, "company-addline-hire", "Hire drivers", Some("groups"), ButtonKind::Normal) {
        super::go(l, kit::Go::Hire);
        return;
    }
    if go {
        if !direct {
            // (its auction opens now: to the concessions, where it is bid on)
            if let Some(id) = act(l, |c| co::concessions::apply(c, &line)) {
                l.company.dialog = None;
                l.company.tenders.selected = Some(id);
                l.company.tab = super::CONCESSIONS_TAB;
            }
            return;
        }
        if act(l, |c| co::network::add_line(c, &line, own.as_ref())).is_some() {
            l.company.dialog = None;
            l.company.lines.selected = Some(line.name.clone());
            let n = l.company.company.as_ref().and_then(|c| c.lines.last()).map(|x| x.number.clone()).unwrap_or_default();
            l.state.set_status(omsi_ui::tr("Line %{n} is the company's: plan its tours and start its service on the Planning page.").replace("%{n}", &n), false);
        }
    }
}
