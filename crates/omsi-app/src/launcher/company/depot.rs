//! The depot page: the yard drawn from above - the offices, the workshop hall with its bays,
//! the washing bay, the diesel station, the rows of parking spaces with the chargers, and the
//! buses where they stand tonight - the areas to build, and the workshop's jobs (the rules are
//! `omsi_launcher_lib::company::depot`'s).
//!
//! Calm, as the other company pages: what is built is drawn in the card's quiet greys with a
//! hairline, what could be built is a dashed plot, building work is amber, the buses wear the
//! company's colour. Only a bus under the mouse is lifted.

use super::super::ownlines;
use super::super::theme::*;
use super::super::ui::{id_of, ButtonKind, Ui};
use super::super::Launcher;
use super::kit;
use super::{act, changed, data, day_label, eur, figure, grade, meter, section};
use glam::Vec2;
use omsi_launcher_lib::company::depot::{self as dp, Area, JobKind};
use omsi_launcher_lib::company::{self as co, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use std::time::Instant;

#[derive(Default)]
pub struct DepotView {
    /// The bus picked on the plan or in the workshop's list.
    selected: Option<u32>,
    /// When the phone's orders were last looked for.
    orders_at: Option<Instant>,
    /// The company, day and lines the concession market was kept up for.
    market_for: Option<(String, String, usize)>,
}

/// What the company's pages do every frame besides drawing: the fleet map let go when its tab
/// is not shown, the phone's orders taken in (every two seconds), and the concession market
/// kept up with the company's day.
pub fn tick(l: &mut Launcher) {
    if l.company.tab != super::MAP_TAB {
        super::map::leave(l);
    }
    if l.company.closing || l.company.company.is_none() {
        return;
    }
    if l.company.depot.orders_at.is_none_or(|t| t.elapsed().as_secs_f32() >= 2.0) {
        l.company.depot.orders_at = Some(Instant::now());
        let data = data();
        let done = l.company.company.as_mut().map(|c| co::remote::take(&data, c)).unwrap_or_default();
        if !done.is_empty() {
            changed(l);
            match done.iter().find_map(|x| x.1.err()) {
                Some(e) => l.state.set_status(omsi_ui::tr("An order from the phone was refused: %{why}").replace("%{why}", &omsi_ui::tr(e)), true),
                None => l.state.set_status(omsi_ui::tr("Orders from the phone carried out: %{n}.").replace("%{n}", &done.len().to_string()), false),
            }
        }
    }
    let (Some(c), Some(t)) = (l.company.company.as_ref(), l.company.today.as_ref()) else { return };
    if t.map != c.map || t.date != c.date || t.error.is_some() {
        return;
    }
    let key = (c.id.clone(), c.date.clone(), c.lines.len());
    if l.company.depot.market_for.as_ref() == Some(&key) {
        return;
    }
    l.company.depot.market_for = Some(key);
    let lines = t.lines.clone();
    if l.company.company.as_mut().is_some_and(|c| co::concessions::refresh(c, &lines)) {
        changed(l);
    }
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    let gap = 16.0;
    let left_w = ((area.w - gap) * 0.56).max(420.0);
    let left = Rect::new(area.x, area.y, left_w, area.h);
    let right = Rect::new(area.x + left_w + gap, area.y, (area.w - left_w - gap).max(0.0), area.h);
    // the figures
    let held = c.fleet.iter().filter(|v| v.held_on(&c.date)).count();
    let spaces = c.site.spaces();
    let fh = kit::FIGURE_H;
    let fw = (left.w - 3.0 * 12.0) / 4.0;
    let fr = |k: usize| Rect::new(left.x + k as f32 * (fw + 12.0), left.y, fw, fh);
    let out = dp::outside(&c);
    let under = if out > 0 { omsi_ui::tr("%{n} in the street").replace("%{n}", &out.to_string()) } else { omsi_ui::tr("spaces in use").into_owned() };
    figure(&mut l.ui, fr(0), "Parking", &format!("{} / {}", held.min(spaces), spaces), &under, if out > 0 { WARN } else { TEXT });
    let busy = dp::bays_used(&c, &c.date);
    let waiting = c.site.jobs.iter().filter(|j| j.started.is_none()).count();
    let under = if waiting > 0 { omsi_ui::tr("Jobs waiting: %{n}").replace("%{n}", &waiting.to_string()) } else { omsi_ui::tr("bays in use").into_owned() };
    figure(&mut l.ui, fr(1), "Workshop", &format!("{} / {}", busy, c.site.bays()), &under, if busy > c.site.bays() { WARN } else { TEXT });
    figure(&mut l.ui, fr(2), "Cleanliness", &format!("{:.0} %", c.site.clean), &omsi_ui::tr("of the fleet"), grade(c.site.clean));
    figure(&mut l.ui, fr(3), "Upkeep", &eur(dp::upkeep_month(&c)), &omsi_ui::tr("a month for the buildings"), TEXT);
    plan(l, Rect::new(left.x, left.y + fh + 12.0, left.w, (left.h - fh - 12.0).max(0.0)), &c);
    let bh = (6.0 * 62.0 + 56.0f32).min(right.h * 0.62);
    buildings(l, Rect::new(right.x, right.y, right.w, bh), &c);
    workshop(l, Rect::new(right.x, right.y + bh + 12.0, right.w, (right.h - bh - 12.0).max(0.0)), &c);
}

// --- the plan ----------------------------------------------------------------------------------

const ASPHALT: Color = Color::rgba(25, 31, 44, 1.0);
const ROOF: Color = Color::rgba(38, 46, 62, 1.0);
const STREET: Color = Color::rgba(31, 37, 50, 1.0);

/// A plot that could be built on: a dashed outline.
fn dashed(ui: &mut Ui, r: Rect, c: Color) {
    let dash = 6.0;
    let mut seg = |a: Vec2, b: Vec2| {
        let len = (b - a).length();
        let n = (len / (dash * 2.0)).floor().max(1.0) as usize;
        let d = (b - a) / len.max(1e-3);
        for k in 0..n {
            let s = a + d * (k as f32 * dash * 2.0);
            let e = s + d * dash.min(len - k as f32 * dash * 2.0);
            ui.p().line(s, e, 1.0, c);
        }
    };
    let (tl, tr, br, bl) = (Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.right(), r.bottom()), Vec2::new(r.x, r.bottom()));
    seg(tl, tr);
    seg(tr, br);
    seg(br, bl);
    seg(bl, tl);
}

/// A building of the plan: its roof, its name, and how far it is built (an area not built is
/// a dashed plot; building work is amber with the day it is done).
fn building(ui: &mut Ui, r: Rect, c: &Company, area: Area, title: &str) -> bool {
    let built = c.site.level(area) > 0;
    let works = c.site.works_on(area).cloned();
    if built {
        ui.p().rounded(r, 6.0, ROOF);
        ui.p().rounded_border(r, 6.0, 1.0, HAIRLINE);
    } else {
        dashed(ui, r, TEXT_FAINT.alpha(0.7));
    }
    if let Some(w) = &works {
        ui.p().rounded_border(r.inset(2.0), 5.0, 1.5, WARN.alpha(0.8));
        let t = omsi_ui::tr("Ready %{date}").replace("%{date}", &day_label(&w.until));
        ui.text_in(&t, Rect::new(r.x + 8.0, r.bottom() - 18.0, r.w - 16.0, 14.0), kit::CAPS, Weight::Bold, WARN, Align::Left);
    }
    let head = if built { omsi_ui::tr(title).to_uppercase() } else { omsi_ui::tr("Free plot").to_uppercase() };
    ui.text_in(&head, Rect::new(r.x + 8.0, r.y + 4.0, r.w - 16.0, 14.0), 12.0, Weight::Bold, if built { TEXT_DIM } else { TEXT_FAINT }, Align::Left);
    built
}

/// A bus seen from above in a space: the company's colour, its number when there is room.
fn bus_top(ui: &mut Ui, r: Rect, number: &str, colour: Color, lifted: bool) {
    if lifted {
        ui.p().rounded(r.inset(-2.0), 4.0, TEXT);
    }
    ui.p().rounded(r, 3.0, colour);
    // (the roof's hatches: the front is up)
    ui.p().rect(Rect::new(r.x + 2.0, r.y + 3.0, r.w - 4.0, (r.h * 0.12).max(2.0)), Color::rgba(0, 0, 0, 0.22));
    if r.h >= 30.0 && r.w >= 14.0 {
        let ink = ownlines::ink_on(colour);
        ui.text_in(number, Rect::new(r.x - 6.0, r.y + r.h * 0.35, r.w + 12.0, r.h * 0.5), (r.w * 0.42).clamp(7.0, 11.0), Weight::Bold, ink, Align::Center);
    }
}

fn plan(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "The depot from above");
    if inner.h < 120.0 {
        return;
    }
    let colour = ownlines::colour_of(&c.colours[0]);
    let street_h = 30.0;
    let yard = Rect::new(inner.x, inner.y, inner.w, inner.h - street_h - 10.0);
    let street = Rect::new(inner.x - 6.0, yard.bottom() + 10.0, inner.w + 12.0, street_h);
    l.ui.p().rounded(yard, 8.0, ASPHALT);
    l.ui.p().rounded_border(yard, 8.0, 1.0, HAIRLINE);
    l.ui.p().rect(street, STREET);
    let mut x = street.x + 8.0;
    while x < street.right() - 8.0 {
        l.ui.p().rect(Rect::new(x, street.center().y - 0.75, 14.0, 1.5), TEXT_FAINT.alpha(0.5));
        x += 26.0;
    }
    // the gate
    let gate = Rect::new(yard.center().x - 30.0, yard.bottom() - 1.5, 60.0, 3.0);
    l.ui.p().rect(gate, ASPHALT);
    l.ui.p().rect(Rect::new(gate.x, gate.bottom() + 2.0, gate.w, street.y - gate.bottom() - 2.0), ASPHALT);

    let today = c.date.clone();
    let held: Vec<&co::Vehicle> = c.fleet.iter().filter(|v| v.held_on(&today)).collect();
    let mut in_ws: Vec<&co::Vehicle> = held.iter().copied().filter(|v| v.in_workshop(&today)).collect();
    in_ws.sort_by(|a, b| a.number.cmp(&b.number));
    let mut parked: Vec<&co::Vehicle> = held.iter().copied().filter(|v| !v.in_workshop(&today)).collect();
    parked.sort_by_key(|v| (v.number.len(), v.number.clone()));
    let mut spots: Vec<(Rect, u32)> = Vec::new();

    // the buildings along the top
    let pad = 12.0;
    let g = 10.0;
    let top = Rect::new(yard.x + pad, yard.y + pad, yard.w - 2.0 * pad, (yard.h * 0.34).max(80.0));
    let ow = top.w * 0.2;
    let ww = top.w * 0.48;
    let sw = top.w - ow - ww - 2.0 * g;
    let offices = Rect::new(top.x, top.y, ow, top.h);
    if building(&mut l.ui, offices, c, Area::Offices, "Offices") {
        let lv = c.site.offices as usize;
        // (its floors: a band a level)
        for k in 0..lv.min(3) {
            let fr = Rect::new(offices.x + 8.0, offices.y + 22.0 + k as f32 * 12.0, offices.w - 16.0, 8.0);
            l.ui.p().rounded(fr, 2.0, TEXT_FAINT.alpha(0.35));
        }
        let t = omsi_ui::tr("room for %{n}").replace("%{n}", &c.site.staff_room().to_string());
        l.ui.text_in(&t, Rect::new(offices.x + 8.0, offices.bottom() - 18.0, offices.w - 16.0, 14.0), kit::CAPS, Weight::Regular, TEXT_DIM, Align::Left);
    }
    // the hall: a bay a level, the buses being worked on in them, the ones waiting before it
    let hall = Rect::new(offices.right() + g, top.y, ww, top.h);
    building(&mut l.ui, hall, c, Area::Workshop, "Workshop");
    let max_bays = Area::Workshop.max() as usize;
    let bay_w = (hall.w - 16.0) / max_bays as f32;
    for k in 0..max_bays {
        let br = Rect::new(hall.x + 8.0 + k as f32 * bay_w + 2.0, hall.y + 22.0, bay_w - 4.0, hall.h - 30.0);
        if k < c.site.bays() {
            l.ui.p().rounded(br, 3.0, ASPHALT);
            // (the pit)
            l.ui.p().rect(Rect::new(br.center().x - 2.0, br.y + 6.0, 4.0, br.h - 12.0), Color::rgba(0, 0, 0, 0.35));
            if let Some(v) = in_ws.get(k) {
                let bw = (br.w * 0.62).min(26.0);
                let b = Rect::new(br.center().x - bw * 0.5, br.y + 6.0, bw, (br.h - 12.0).min(bw * 3.4));
                bus_top(&mut l.ui, b, &v.number, colour, l.company.depot.selected == Some(v.id));
                spots.push((b, v.id));
            }
        } else {
            dashed(&mut l.ui, br, TEXT_FAINT.alpha(0.35));
        }
    }
    for (k, v) in in_ws.iter().enumerate().skip(c.site.bays()).take(6) {
        let b = Rect::new(hall.x + 8.0 + (k - c.site.bays()) as f32 * 30.0, hall.bottom() + 4.0, 24.0, 14.0);
        l.ui.p().rounded(b, 3.0, WARN.alpha(0.8));
        spots.push((b, v.id));
    }
    // the washing bay and the diesel station
    let side = Rect::new(hall.right() + g, top.y, sw, top.h);
    let wash = Rect::new(side.x, side.y, side.w, (side.h - g) * 0.5);
    if building(&mut l.ui, wash, c, Area::Wash, "Washing bay") {
        let lane = Rect::new(wash.x + 10.0, wash.y + 22.0, wash.w - 20.0, wash.h - 30.0);
        l.ui.p().rounded(lane, 3.0, EARLY.alpha(0.12));
        if c.site.wash >= 2 {
            for k in 0..2 {
                let gx = lane.x + lane.w * (0.35 + 0.3 * k as f32);
                l.ui.p().rect(Rect::new(gx - 1.5, lane.y, 3.0, lane.h), EARLY_SOFT.alpha(0.6));
            }
        }
    }
    let fuel = Rect::new(side.x, wash.bottom() + g, side.w, (side.h - g) * 0.5);
    if building(&mut l.ui, fuel, c, Area::Fuel, "Diesel station") {
        let n = c.site.fuel as usize;
        for k in 0..n {
            let p = Vec2::new(fuel.x + 16.0 + k as f32 * 18.0, fuel.center().y + 6.0);
            l.ui.p().rounded(Rect::new(p.x - 5.0, p.y - 8.0, 10.0, 16.0), 2.0, TEXT_SOFT.alpha(0.7));
        }
    }

    // the parking: four rows of two blocks of twelve spaces, a level a block
    let park = Rect::new(yard.x + pad, top.bottom() + 26.0, yard.w - 2.0 * pad, (yard.bottom() - pad - top.bottom() - 26.0).max(40.0));
    let label = omsi_ui::tr(Area::Parking.label()).to_uppercase();
    l.ui.text_in(&label, Rect::new(park.x, park.y - 18.0, park.w * 0.5, 14.0), 12.0, Weight::Bold, TEXT_DIM, Align::Left);
    if let Some(w) = c.site.works_on(Area::Parking) {
        let t = omsi_ui::tr("More spaces ready %{date}").replace("%{date}", &day_label(&w.until));
        l.ui.text_in(&t, Rect::new(park.x + park.w * 0.5, park.y - 18.0, park.w * 0.5, 14.0), kit::CAPS, Weight::Bold, WARN, Align::Right);
    }
    let blocks = Area::Parking.max() as usize;
    let rows = blocks / 2;
    let bg = 14.0;
    let block_w = (park.w - bg) / 2.0;
    let block_h = ((park.h - (rows as f32 - 1.0) * 8.0) / rows as f32).max(16.0);
    let per = dp::SPACES_PER_LEVEL as usize;
    let slot_w = block_w / per as f32;
    let chargers = c.site.electric_served();
    let mut n = 0usize;
    for b in 0..blocks {
        let br = Rect::new(park.x + (b % 2) as f32 * (block_w + bg), park.y + (b / 2) as f32 * (block_h + 8.0), block_w, block_h);
        let built = b < c.site.parking as usize;
        let building_now = !built && b == c.site.parking as usize && c.site.works_on(Area::Parking).is_some();
        for s in 0..per {
            let sr = Rect::new(br.x + s as f32 * slot_w, br.y, slot_w, br.h);
            if built {
                l.ui.p().rect(Rect::new(sr.x, sr.y, 1.0, sr.h), TEXT_FAINT.alpha(0.55));
                if s + 1 == per {
                    l.ui.p().rect(Rect::new(sr.right() - 1.0, sr.y, 1.0, sr.h), TEXT_FAINT.alpha(0.55));
                }
                // (a charger at the head of every space it serves)
                if n < chargers {
                    l.ui.p().circle(Vec2::new(sr.center().x, sr.y + 3.0), 2.2, OK);
                }
                if let Some(v) = parked.get(n) {
                    let bw = (slot_w * 0.66).min(22.0);
                    let bh = (sr.h - 10.0).min(bw * 3.6);
                    let rect = Rect::new(sr.center().x - bw * 0.5, sr.y + 6.0, bw, bh);
                    bus_top(&mut l.ui, rect, &v.number, colour, l.company.depot.selected == Some(v.id));
                    spots.push((rect, v.id));
                }
                n += 1;
            }
        }
        if !built {
            dashed(&mut l.ui, br, if building_now { WARN.alpha(0.8) } else { TEXT_FAINT.alpha(0.3) });
        }
    }
    // the buses the yard has no space for: in the street
    for (k, v) in parked.iter().skip(n).enumerate().take(12) {
        let bw = 34.0;
        let rect = Rect::new(street.x + 10.0 + k as f32 * (bw + 6.0), street.y + 7.0, bw, street.h - 14.0);
        l.ui.p().rounded(rect.inset(-1.5), 3.0, WARN);
        l.ui.p().rounded(rect, 3.0, colour);
        spots.push((rect, v.id));
    }
    // a bus under the mouse: its card in the tooltip; a click picks it for the workshop
    let mouse = l.ui.input.mouse;
    if let Some((rect, id)) = spots.iter().find(|(r, _)| r.inset(-2.0).contains(mouse)).copied() {
        let (_, _, clicked) = l.ui.interact(id_of(&format!("depot-bus-{id}")), rect.inset(-2.0));
        if let Some(v) = c.vehicle(id) {
            let mut tip = format!("{} {}  ·  {} {:.0} %", v.number, v.name, omsi_ui::tr("condition"), v.condition);
            if v.in_workshop(&today) {
                tip.push_str(&format!("  ·  {}", omsi_ui::tr("in the workshop until %{date}").replace("%{date}", &day_label(v.workshop_until.as_deref().unwrap_or("")))));
            }
            l.ui.tooltip(rect, &tip);
        }
        l.ui.cursor = winit::window::CursorIcon::Pointer;
        if clicked {
            l.company.depot.selected = if l.company.depot.selected == Some(id) { None } else { Some(id) };
        }
    }
}

// --- the buildings ----------------------------------------------------------------------------

/// What a level of an area does, in words.
fn effect(c: &Company, a: Area) -> String {
    let s = &c.site;
    match a {
        Area::Parking => omsi_ui::tr("%{n} spaces").replace("%{n}", &s.spaces().to_string()),
        Area::Workshop => omsi_ui::tr("%{n} bays: buses worked on at once").replace("%{n}", &s.bays().to_string()),
        Area::Wash if s.wash == 0 => omsi_ui::tr("None: the fleet is cleaned by hand").into_owned(),
        Area::Wash => omsi_ui::tr("Washes %{n} buses a night").replace("%{n}", &s.washes().to_string()),
        Area::Fuel => omsi_ui::tr("Fills %{n} diesel buses a night").replace("%{n}", &s.diesel_served().to_string()),
        Area::Charging if s.charging == 0 => omsi_ui::tr("None: electric buses charge outside, dearer").into_owned(),
        Area::Charging => omsi_ui::tr("Charges %{n} electric buses a night").replace("%{n}", &s.electric_served().to_string()),
        Area::Offices => omsi_ui::tr("Room for %{n} people").replace("%{n}", &s.staff_room().to_string()),
    }
}

fn buildings(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Build");
    let mut build = None;
    let cc = c.clone();
    l.ui.scroll_area("depot-areas", inner, &mut |ui, v| {
        let rh = 62.0;
        for (k, a) in Area::ALL.iter().enumerate() {
            let r = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 6.0);
            if !ui.rect_visible(r) {
                continue;
            }
            if k > 0 {
                ui.p().rect(Rect::new(r.x, r.y - 3.0, r.w, 1.0), HAIRLINE);
            }
            ui.icon(a.icon(), Vec2::new(r.x + 15.0, r.y + 20.0), 22.0, accent_2());
            let lv = cc.site.level(*a);
            ui.text_in(a.label(), Rect::new(r.x + 38.0, r.y + 6.0, r.w - 230.0, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let lvl = omsi_ui::tr("level %{n} of %{max}").replace("%{n}", &lv.to_string()).replace("%{max}", &a.max().to_string());
            let tw = ui.width(&lvl, 12.0, Weight::Bold) + 16.0;
            let lx = r.x + 38.0 + ui.width(&omsi_ui::tr(a.label()), kit::ROWS, Weight::Bold) + 12.0;
            if lx + tw < r.right() - 190.0 {
                kit::tag(ui, Vec2::new(lx, r.y + 7.0), &lvl, TEXT_SOFT);
            }
            ui.text_in(&effect(&cc, *a), Rect::new(r.x + 38.0, r.y + 32.0, r.w - 230.0, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            let br = Rect::new(r.right() - 180.0, r.y + 9.0, 180.0, 38.0);
            if let Some(w) = cc.site.works_on(*a) {
                let t = omsi_ui::tr("Building until %{date}").replace("%{date}", &day_label(&w.until));
                ui.text_in(&t, br, kit::NOTE, Weight::Medium, WARN, Align::Right);
            } else if let Some((cost, days)) = dp::next_cost(&cc, *a) {
                let open = dp::area_allowed(&cc, *a);
                if ui.button(&format!("depot-build-{}", a.key()), br, &eur(cost), Some(if open { "add" } else { "lock" }), ButtonKind::Normal) {
                    build = Some((*a, cost));
                }
                let up = dp::step(*a, lv + 1).upkeep as f64 * cc.price_index;
                let tip = omsi_ui::tr("Build the next level: %{days} days of work, then %{amount} a month").replace("%{days}", &days.to_string()).replace("%{amount}", &eur(up.round() as i64));
                ui.tooltip(br, &tip);
            } else {
                ui.text_in("Complete", br, kit::NOTE, Weight::Medium, OK, Align::Right);
            }
        }
        Area::ALL.len() as f32 * rh
    });
    if let Some((a, cost)) = build {
        // (what is in the way is said first: the level that opens it, or the money)
        if !dp::area_allowed(c, a) {
            let f = if a == Area::Workshop { co::levels::Feature::Workshop } else { co::levels::Feature::ElectricBuses };
            kit::show(l, kit::locked(c, f));
        } else if c.cash < cost {
            kit::show(l, kit::no_cash(c, cost));
        } else if act(l, |c| dp::build(c, a)).is_some() {
            l.state.set_status(omsi_ui::tr("Building work has begun: %{what}.").replace("%{what}", &omsi_ui::tr(a.label())), false);
        }
    }
}

// --- the workshop -----------------------------------------------------------------------------

fn workshop(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Workshop");
    if inner.h < 30.0 {
        return;
    }
    let today = c.date.clone();
    // the jobs first, then every other bus by its condition (the one picked on the plan first)
    let jobs = c.site.jobs.clone();
    let selected = l.company.depot.selected;
    let mut buses: Vec<co::Vehicle> = c.fleet.iter().filter(|v| v.held_on(&today) && c.site.job_of(v.id).is_none()).cloned().collect();
    buses.sort_by(|a, b| (Some(b.id) == selected).cmp(&(Some(a.id) == selected)).then(a.condition.total_cmp(&b.condition)));
    if jobs.is_empty() && buses.is_empty() {
        l.ui.paragraph("No bus yet: the workshop services, repairs and overhauls the fleet's buses, one a bay.", Vec2::new(inner.x, inner.y), inner.w, kit::ROWS, Weight::Regular, TEXT_SOFT);
        return;
    }
    let cc = c.clone();
    let mut order: Option<(u32, JobKind)> = None;
    let mut cancel: Option<u32> = None;
    let mut pick: Option<u32> = None;
    l.ui.scroll_area("depot-workshop", inner, &mut |ui, v| {
        let rh = 56.0;
        let mut y = v.y;
        for j in &jobs {
            let r = Rect::new(v.x, y, v.w - 10.0, rh - 6.0);
            y += rh;
            if !ui.rect_visible(r) {
                continue;
            }
            let Some(bus) = cc.vehicle(j.vehicle) else { continue };
            let w = super::plate(ui, Vec2::new(r.x, r.y + 4.0), &bus.number, 20.0);
            ui.text_in(&format!("{}  ·  {}", omsi_ui::tr(j.kind.label()), bus.name), Rect::new(r.x + w + 10.0, r.y + 3.0, r.w - w - 140.0, 22.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let (what, colour) = match (&j.started, &j.until) {
                (Some(_), Some(u)) => (omsi_ui::tr("In a bay until %{date}").replace("%{date}", &day_label(u)), accent_2()),
                _ => (omsi_ui::tr("Waiting for a free bay").into_owned(), WARN),
            };
            ui.text_in(&what, Rect::new(r.x + w + 10.0, r.y + 27.0, r.w - w - 140.0, 20.0), kit::NOTE, Weight::Medium, colour, Align::Left);
            if j.cost > 0 {
                ui.text_in(&eur(j.cost), Rect::new(r.right() - 120.0, r.y + 2.0, 80.0, 18.0), kit::NOTE, Weight::Medium, TEXT_SOFT, Align::Right);
            }
            if j.started.is_none() && ui.icon_button(&format!("depot-job-cancel-{}", j.id), Vec2::new(r.right() - 16.0, r.y + 16.0), 14.0, "close", "Take the job back") {
                cancel = Some(j.id);
            }
        }
        if !jobs.is_empty() {
            ui.p().rect(Rect::new(v.x, y - 3.0, v.w - 10.0, 1.0), HAIRLINE);
        }
        for b in &buses {
            let r = Rect::new(v.x, y, v.w - 10.0, rh - 6.0);
            y += rh;
            if !ui.rect_visible(r) {
                continue;
            }
            let on = selected == Some(b.id);
            if ui.row(&format!("depot-bus-row-{}", b.id), Rect::new(r.x, r.y, r.w * 0.42, r.h), on) {
                pick = Some(b.id);
            }
            let w = super::plate(ui, Vec2::new(r.x + 6.0, r.y + 4.0), &b.number, 20.0);
            let ink = if on { on_accent() } else { TEXT };
            ui.text_in(&b.name, Rect::new(r.x + w + 14.0, r.y + 3.0, r.w * 0.42 - w - 18.0, 22.0), 14.0, Weight::Bold, ink, Align::Left);
            let due = b.km >= b.next_service_km - 1_000.0;
            let sub = if b.in_workshop(&today) {
                omsi_ui::tr("in the workshop").into_owned()
            } else if due {
                omsi_ui::tr("service due").into_owned()
            } else {
                format!("{:.0} %", b.condition)
            };
            ui.text_in(&sub, Rect::new(r.x + w + 14.0, r.y + 27.0, 110.0, 20.0), kit::NOTE, Weight::Medium, if due { WARN } else if on { on_accent() } else { TEXT_DIM }, Align::Left);
            let mr = Rect::new(r.x + w + 128.0, r.y + 35.0, (r.w * 0.42 - w - 136.0).max(10.0), 5.0);
            meter(ui, mr, b.condition / 100.0, grade(b.condition));
            ui.tooltip(Rect::new(mr.x, mr.y - 9.0, mr.w, 22.0), &omsi_ui::tr("Condition: %{n} of 100").replace("%{n}", &format!("{:.0}", b.condition)));
            let bw = ((r.w * 0.58 - 12.0) / 3.0).min(110.0);
            for (k, kind) in JobKind::ALL.iter().enumerate() {
                let br = Rect::new(r.right() - (3 - k) as f32 * (bw + 4.0), r.y + 8.0, bw, 34.0);
                if ui.button(&format!("depot-job-{}-{}", b.id, kind.key()), br, kind.label(), None, ButtonKind::Ghost) {
                    order = Some((b.id, *kind));
                }
                let cost = dp::job_cost(&cc, b.id, *kind);
                let tip = if cost > 0 {
                    omsi_ui::tr("%{days} days in a bay, %{amount}").replace("%{days}", &kind.days().to_string()).replace("%{amount}", &eur(cost))
                } else {
                    omsi_ui::tr("%{days} days in a bay; paid with the maintenance per kilometre").replace("%{days}", &kind.days().to_string())
                };
                ui.tooltip(br, &tip);
            }
        }
        y - v.y
    });
    if let Some(id) = pick {
        l.company.depot.selected = if selected == Some(id) { None } else { Some(id) };
    }
    if let Some(id) = cancel {
        act(l, |c| dp::cancel(c, id));
    }
    if let Some((id, kind)) = order {
        if act(l, |c| dp::order(c, id, kind)).is_some() {
            let started = l.company.company.as_ref().and_then(|c| c.site.job_of(id)).is_some_and(|j| j.started.is_some());
            l.state.set_status(if started { omsi_ui::tr("The bus is in the workshop tomorrow.") } else { omsi_ui::tr("The job waits for a free bay.") }.into_owned(), false);
        }
    }
}
