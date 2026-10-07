//! The Depot page: the player's own buses, each with its plate, its counters and its wear.

use super::theme::*;
use super::ui::ButtonKind;
use super::{Launcher, Page};
use glam::Vec2;
use omsi_launcher_lib::depot::DepotBus;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct DepotView {
    /// The bus shown on the right.
    pub selected: String,
    /// The bus families of the add form, and the content they were built from.
    families: std::sync::Arc<Vec<super::drive::BusManufacturer>>,
    families_key: (usize, usize),
    /// The bus being put together, while the form is open.
    pub adding: Option<Draft>,
    /// The name and plate being typed, for the bus they belong to.
    pub editing: Option<(String, String, String)>,
    pub confirm_remove: Option<std::time::Instant>,
}

#[derive(Default, Clone)]
pub struct Draft {
    pub filter: String,
    /// The bus family open in the list, and whether the chosen one was looked for yet.
    pub expanded: Option<String>,
    pub list_ready: bool,
    pub list_h: f32,
    pub bus: String,
    pub paint: String,
    pub name: String,
    pub plate: String,
    pub number: String,
}

/// The bus and livery the showroom shows on this page: the one being added, else the one selected.
pub fn look(l: &Launcher) -> Option<(String, String)> {
    if let Some(d) = l.depot.adding.as_ref().filter(|d| !d.bus.is_empty()) {
        return Some((d.bus.clone(), d.paint.clone()));
    }
    let b = l.state.depot.get(&l.depot.selected).or_else(|| l.state.depot.buses.first())?;
    Some((b.bus.clone(), b.paint.clone()))
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Depot", "Your own buses: each keeps its number plate, its kilometres and the state you left it in.");
    if l.depot.selected.is_empty() || l.state.depot.get(&l.depot.selected).is_none() {
        l.depot.selected = l.state.depot.buses.first().map(|b| b.id.clone()).unwrap_or_default();
    }
    let (left, right, preview) = if body.w < 760.0 {
        let list = Rect::new(body.x, body.y, body.w, (body.h * 0.42).max(260.0));
        (list, Rect::new(body.x, list.bottom() + GAP, body.w, 560.0), false)
    } else {
        let col_w = (body.w * 0.32).clamp(320.0, 420.0);
        (Rect::new(body.x, body.y, col_w, body.h), Rect::new(body.x + col_w + GAP * 2.0, body.y, body.w - col_w - GAP * 2.0, body.h), true)
    };
    fleet(l, left);
    if l.depot.adding.is_some() {
        add_form(l, right, preview);
    } else {
        details(l, right, preview);
    }
}

fn fleet(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = l.ui.heading(Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, r.h - 28.0), "Fleet", Some("garage"));
    let list = Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h - ROW - 14.0);
    let buses = l.state.depot.buses.clone();
    let selected = if l.depot.adding.is_some() { String::new() } else { l.depot.selected.clone() };
    let mut pick = None;
    l.ui.scroll_area("depot-fleet", list, &mut |ui, v| {
        if buses.is_empty() {
            let h = ui.paragraph("No bus of your own yet: add one with the button below.", Vec2::new(v.x + 8.0, v.y + 4.0), v.w - 16.0, 13.0, Weight::Regular, TEXT_DIM);
            return h + 8.0;
        }
        let rh = 64.0;
        for (k, b) in buses.iter().enumerate() {
            let row = Rect::new(v.x + 6.0, v.y + k as f32 * rh, v.w - 16.0, rh - 6.0);
            if !ui.rect_visible(row) {
                continue;
            }
            if ui.row(&format!("depot-bus-{}", b.id), row, b.id == selected) {
                pick = Some(b.id.clone());
            }
            ui.icon("directions_bus", Vec2::new(row.x + 22.0, row.center().y), 22.0, if b.id == selected { ACCENT } else { TEXT_DIM });
            ui.text_in(&b.name, Rect::new(row.x + 46.0, row.y + 8.0, row.w - 140.0, 20.0), 13.5, Weight::Bold, TEXT, Align::Left);
            ui.text_in(&identity(b), Rect::new(row.x + 46.0, row.y + 30.0, row.w - 140.0, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            ui.text_in(&format!("{:.0} km", b.odometer().unwrap_or(b.metres / 1000.0)), Rect::new(row.right() - 96.0, row.y + 8.0, 86.0, 20.0), 12.5, Weight::Bold, TEXT_SOFT, Align::Right);
            if let Some(f) = b.fuel() {
                ui.text_in(&format!("{f:.0} L"), Rect::new(row.right() - 96.0, row.y + 30.0, 86.0, 18.0), 11.5, Weight::Medium, if f < 25.0 { WARN } else { TEXT_DIM }, Align::Right);
            }
        }
        buses.len() as f32 * rh
    });
    if let Some(id) = pick {
        l.depot.selected = id;
        l.depot.adding = None;
        l.depot.editing = None;
        l.depot.confirm_remove = None;
    }
    let add = Rect::new(inner.x, inner.bottom() - ROW, inner.w, ROW);
    if l.ui.button("depot-add", add, "Add a bus", Some("add"), if buses.is_empty() { ButtonKind::Primary } else { ButtonKind::Normal }) && l.depot.adding.is_none() {
        let plate = l.state.depot.new_plate(seed());
        l.depot.adding = Some(Draft { plate, ..Default::default() });
    }
}

fn add_form(l: &mut Launcher, r: Rect, preview: bool) {
    let mut y = r.y;
    if preview {
        let view = Rect::new(r.x, r.y, r.w, (r.h * 0.36).clamp(180.0, 340.0));
        if l.depot.adding.as_ref().is_some_and(|d| !d.bus.is_empty()) {
            l.preview_full(view, 0.5);
            l.showroom_pointer(view);
        } else {
            l.ui.p().rounded(view, RADIUS, FIELD);
            l.ui.text_in("Choose a bus in the list", Rect::new(view.x, view.center().y - 12.0, view.w, 24.0), 13.0, Weight::Regular, TEXT_FAINT, Align::Center);
        }
        l.ui.p().rounded_border(view, RADIUS, 1.0, EDGE);
        y = view.bottom() + GAP;
    }
    let card = Rect::new(r.x, y, r.w, r.bottom() - y);
    l.ui.panel(card);
    let inner = l.ui.heading(Rect::new(card.x + 18.0, card.y + 14.0, card.w - 36.0, card.h - 28.0), "New bus", Some("add"));
    let Some(mut d) = l.depot.adding.clone() else { return };
    let half = (inner.w - GAP * 2.0) * 0.5;
    // the models
    let left = Rect::new(inner.x, inner.y, half, inner.h);
    let search_changed = l.ui.text_input("depot-filter", Rect::new(left.x, left.y, left.w, ROW), &mut d.filter, "Search buses…", Some("search"));
    let key = (l.state.vehicles.len(), l.state.fresh.len());
    if key != l.depot.families_key || (l.depot.families.is_empty() && !l.state.vehicles.is_empty()) {
        let fresh = l.state.fresh.keys().cloned().collect();
        l.depot.families = std::sync::Arc::new(super::drive::build_bus_manufacturers(&l.state.vehicles, None, &fresh));
        l.depot.families_key = key;
    }
    let families = l.depot.families.clone();
    let q = omsi_launcher_lib::display_bus_name(d.filter.trim()).to_lowercase();
    let visible: Vec<&super::drive::BusManufacturer> = families.iter().filter(|m| super::drive::manufacturer_matches(m, &q)).collect();
    let list = Rect::new(left.x, left.y + ROW + 8.0, left.w, left.h - ROW - 8.0);
    l.ui.p().rounded(list, RADIUS, FIELD);
    l.ui.p().rounded_border(list, RADIUS, 1.0, EDGE);
    let open_at = (!d.list_ready && !families.is_empty()).then(|| families.iter().position(|m| m.variants.iter().any(|v| v.file == d.bus))).flatten();
    if let Some(i) = open_at {
        d.expanded = Some(families[i].key.clone());
    }
    if search_changed && !q.is_empty() {
        d.expanded = visible.first().map(|m| m.key.clone());
    }
    let chosen = d.bus.clone();
    let expanded = d.expanded.clone();
    let loading = l.state.loading_content;
    let mut picked = super::drive::FamilyPick::default();
    l.ui.scroll_area("depot-models", list, &mut |ui, view| super::drive::family_rows(ui, view, "depot", &visible, expanded.as_deref(), &chosen, &q, None, loading, &mut picked));
    if let Some(i) = open_at {
        l.ui.scroll_to("depot-models", 6.0 + i as f32 * 58.0, 120.0, list.h);
    }
    // (the buses come in batches, and the window takes its size after the first frames)
    let settled = (d.list_h - list.h).abs() < 0.5;
    d.list_h = list.h;
    d.list_ready |= settled && (open_at.is_some() || (!loading && !families.is_empty()));
    if let Some(k) = picked.toggle {
        d.expanded = if d.expanded.as_ref() == Some(&k) { None } else { Some(k) };
    }
    if let Some(file) = picked.pick {
        if file != d.bus {
            d.number = l.state.vehicles.iter().find(|v| v.file == file).and_then(|v| v.numbers.first().map(|n| n.0.clone())).unwrap_or_default();
            d.paint.clear();
            d.bus = file;
        }
    }
    // its livery, name, plate and number
    let right = Rect::new(inner.x + half + GAP * 2.0, inner.y, half, inner.h);
    let label_w = 150.0;
    let field = |y: f32| Rect::new(right.x + label_w, y, right.w - label_w, ROW);
    let mut y = right.y;
    let vehicle = l.state.vehicles.iter().find(|v| v.file == d.bus).cloned();
    if let Some(v) = &vehicle {
        let paints: Vec<String> = std::iter::once(super::drive::default_livery_label(v).to_string()).chain(v.paints.iter().cloned()).collect();
        let mut sel = v.paints.iter().position(|p| *p == d.paint).map(|i| i + 1).unwrap_or(0);
        l.ui.label(Rect::new(right.x, y, label_w, ROW), "Livery");
        if l.ui.select("depot-paint", field(y), &mut sel, &paints) {
            d.paint = if sel == 0 { String::new() } else { v.paints[sel - 1].clone() };
        }
        y += ROW + 10.0;
    }
    l.ui.label(Rect::new(right.x, y, label_w, ROW), "Name");
    l.ui.text_input("depot-new-name", field(y), &mut d.name, &vehicle.as_ref().map(|v| v.name.clone()).unwrap_or_else(|| omsi_ui::tr("Name (e.g. my Citaro)").into_owned()), Some("badge"));
    y += ROW + 10.0;
    l.ui.label(Rect::new(right.x, y, label_w, ROW), "Number plate");
    let pf = field(y);
    l.ui.text_input("depot-new-plate", Rect::new(pf.x, pf.y, pf.w - ROW - 8.0, ROW), &mut d.plate, "AB-123-CD", Some("badge"));
    let dice = Rect::new(pf.right() - ROW, y, ROW, ROW);
    if l.ui.button("depot-new-plate-roll", dice, "", Some("autorenew"), ButtonKind::Normal) {
        d.plate = l.state.depot.new_plate(seed());
    }
    l.ui.tooltip(dice, "Another plate");
    y += ROW + 10.0;
    if let Some(v) = vehicle.as_ref().filter(|v| !v.numbers.is_empty()) {
        let options: Vec<String> = v.numbers.iter().map(|(n, p)| if p.trim().is_empty() { n.clone() } else { format!("{n}  ({})", p.trim()) }).collect();
        let mut sel = v.numbers.iter().position(|(n, _)| *n == d.number).unwrap_or(0);
        l.ui.label(Rect::new(right.x, y, label_w, ROW), "Fleet number");
        if l.ui.select("depot-new-number", field(y), &mut sel, &options) {
            d.number = v.numbers[sel].0.clone();
        }
    }
    let by = right.bottom() - ROW;
    let cancel_w = (right.w * 0.36).max(110.0);
    let mut done = false;
    if l.ui.button("depot-add-cancel", Rect::new(right.x, by, cancel_w, ROW), "Cancel", None, ButtonKind::Ghost) {
        done = true;
    }
    let add = Rect::new(right.x + cancel_w + GAP, by, right.w - cancel_w - GAP, ROW);
    if l.ui.button("depot-add-confirm", add, "Add to the depot", Some("garage"), ButtonKind::Primary) {
        if let Some(v) = &vehicle {
            let name = if d.name.trim().is_empty() { v.name.clone() } else { d.name.trim().to_string() };
            let id = l.state.add_to_depot(DepotBus { name, bus: d.bus.clone(), paint: d.paint.clone(), plate: d.plate.trim().to_string(), number: d.number.clone(), ..Default::default() });
            l.depot.selected = id;
            l.state.set_status("Bus added to your depot.", false);
            done = true;
        } else {
            l.state.set_status("Choose a bus in the list", true);
        }
    }
    l.depot.adding = if done { None } else { Some(d) };
}

fn details(l: &mut Launcher, r: Rect, preview: bool) {
    let Some(b) = l.state.depot.get(&l.depot.selected).cloned() else {
        l.ui.panel(r);
        l.ui.icon("garage", Vec2::new(r.center().x, r.center().y - 28.0), 40.0, TEXT_FAINT);
        l.ui.text_in("Your buses show up here once added.", Rect::new(r.x + 20.0, r.center().y + 4.0, r.w - 40.0, 24.0), 14.0, Weight::Medium, TEXT_DIM, Align::Center);
        return;
    };
    let mut y = r.y;
    if preview {
        let view = Rect::new(r.x, r.y, r.w, (r.h * 0.46).clamp(200.0, 420.0));
        l.preview_full(view, 0.5);
        l.ui.p().rounded_border(view, RADIUS, 1.0, EDGE);
        l.showroom_pointer(view);
        y = view.bottom() + GAP;
    }
    let card = Rect::new(r.x, y, r.w, r.bottom() - y);
    l.ui.panel(card);
    let inner = Rect::new(card.x + 18.0, card.y + 16.0, card.w - 36.0, card.h - 32.0);
    // name and plate, written when typed
    let (mut name, mut plate) = match &l.depot.editing {
        Some((id, n, p)) if *id == b.id => (n.clone(), p.clone()),
        _ => (b.name.clone(), b.plate.clone()),
    };
    let half = (inner.w - GAP) * 0.5;
    let n_changed = l.ui.text_input("depot-name", Rect::new(inner.x, inner.y, half, ROW), &mut name, "Name", Some("directions_bus"));
    let p_changed = l.ui.text_input("depot-plate", Rect::new(inner.x + half + GAP, inner.y, half, ROW), &mut plate, "Number plate", Some("badge"));
    if n_changed || p_changed {
        l.depot.editing = Some((b.id.clone(), name.clone(), plate.clone()));
        if let Some(x) = l.state.depot.get_mut(&b.id) {
            if !name.trim().is_empty() {
                x.name = name.trim().to_string();
            }
            x.plate = plate.trim().to_string();
        }
        l.state.save_depot();
    }
    let model = l.state.vehicles.iter().find(|v| v.file == b.bus).map(|v| v.name.clone()).unwrap_or_else(|| b.bus.rsplit('/').next().unwrap_or("").to_string());
    let paint = if b.paint.is_empty() { omsi_ui::tr("standard livery").into_owned() } else { b.paint.clone() };
    let number = if b.number.is_empty() { String::new() } else { format!(" · {} {}", omsi_ui::tr("fleet number"), b.number) };
    l.ui.text_in(&format!("{model} · {paint}{number}"), Rect::new(inner.x, inner.y + ROW + 8.0, inner.w, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
    // the counters
    let km = b.metres / 1000.0;
    let clock = b.odometer().unwrap_or(km);
    let per_100 = if km > 5.0 && b.fuel_used > 0.0 { format!("{:.1} L", b.fuel_used / km * 100.0) } else { "-".into() };
    let damage = b.damage();
    let state = if damage <= 0.0 { omsi_ui::tr("no damage") } else if damage < 100.0 { omsi_ui::tr("light damage") } else { omsi_ui::tr("damaged") };
    let tiles = [
        ("route", format!("{clock:.0} km"), "on the clock"),
        ("schedule", hours(b.seconds / 3600.0), "hours driven"),
        ("history", b.sessions.to_string(), "runs"),
        ("location_on", b.stops.to_string(), "stops served"),
        ("local_gas_station", b.fuel().map(|f| format!("{f:.0} L")).unwrap_or_else(|| "-".into()), "fuel left"),
        ("speed", per_100, "per 100 km"),
        ("water_drop", b.dirt().map(|d| format!("{:.0} %", d * 100.0)).unwrap_or_else(|| "-".into()), "dirt"),
        ("build", state.into_owned(), "body"),
        ("confirmation_number", b.tickets.to_string(), "tickets sold"),
        ("payments", format!("{:.2}", b.takings), "takings"),
        ("warning", b.crashes.to_string(), "crashes"),
        ("event", if b.last_used == 0 { omsi_ui::tr("never").into_owned() } else { super::pages::chrono_like(b.last_used) }, "last run"),
    ];
    let foot_h = ROW + 12.0;
    let grid = Rect::new(inner.x, inner.y + ROW + 36.0, inner.w, inner.bottom() - inner.y - ROW - 36.0 - foot_h);
    let cols = if grid.w > 700.0 { 4 } else { 3 };
    let cw = (grid.w - GAP * (cols as f32 - 1.0)) / cols as f32;
    let ch = 62.0;
    for (k, (icon, v, label)) in tiles.iter().enumerate() {
        let (cx, cy) = ((k % cols) as f32, (k / cols) as f32);
        let t = Rect::new(grid.x + cx * (cw + GAP), grid.y + cy * (ch + 8.0), cw, ch);
        if t.bottom() > grid.bottom() {
            break;
        }
        l.ui.p().rounded(t, 10.0, Color::WHITE.alpha(0.04));
        l.ui.icon(icon, Vec2::new(t.x + 20.0, t.y + 20.0), 17.0, ACCENT);
        l.ui.text_in(v, Rect::new(t.x + 36.0, t.y + 6.0, t.w - 42.0, 26.0), 16.0, Weight::Black, TEXT, Align::Left);
        l.ui.text_in(label, Rect::new(t.x + 12.0, t.y + 38.0, t.w - 18.0, 16.0), 11.0, Weight::Medium, TEXT_DIM, Align::Left);
    }
    // driving it, or letting it go
    let fy = inner.bottom() - ROW;
    let go = Rect::new(inner.x, fy, (inner.w * 0.5).min(300.0), ROW);
    if l.ui.button("depot-drive", go, "Drive this bus", Some("play_arrow"), ButtonKind::Primary) {
        l.state.take_depot_bus(&b.id);
        l.go(Page::Drive);
        l.state.set_status(omsi_ui::tr("%{name} is ready under Drive: choose the time and the duty.").replace("%{name}", &b.name), false);
    }
    let armed = l.depot.confirm_remove.is_some_and(|t| t.elapsed().as_secs() < 4);
    let rm = Rect::new(inner.right() - 220.0, fy, 220.0, ROW);
    if l.ui.button("depot-remove", rm, if armed { "Click again to remove" } else { "Remove from the depot" }, Some("delete"), ButtonKind::Danger) {
        if armed {
            l.state.depot.remove(&b.id);
            l.state.save_depot();
            l.depot.selected.clear();
            l.depot.confirm_remove = None;
        } else {
            l.depot.confirm_remove = Some(std::time::Instant::now());
        }
    }
}

/// Plate, fleet number: what tells this bus from the others of its type.
fn identity(b: &DepotBus) -> String {
    let mut parts = Vec::new();
    if !b.plate.trim().is_empty() {
        parts.push(b.plate.trim().to_string());
    }
    if !b.number.trim().is_empty() {
        parts.push(format!("#{}", b.number.trim()));
    }
    if parts.is_empty() {
        parts.push(b.bus.rsplit('/').next().unwrap_or("").trim_end_matches(".bus").to_string());
    }
    parts.join(" · ")
}

fn hours(h: f64) -> String {
    format!("{}:{:02} h", h.floor() as i64, ((h - h.floor()) * 60.0).round() as i64)
}
