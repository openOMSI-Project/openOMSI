//! The fleet and the dealer: the company's buses as tiles with their photos (the bus picker's,
//! `busphoto`), and the dealer (`dealer`: the showroom, the day's offers, the used market,
//! the orders); a bus of the fleet opens to its livery, a service, selling or giving it back.
//! A bus of the dealer can also be leased or rented here. The third tab is the advertising on
//! the buses (`adverts`): the advertisers' offers and the contracts signed.

use super::super::theme::*;
use super::super::ui::{id_of, ButtonKind, Ui};
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{act, day_label, eur, grade, grouped, meter, Confirm, Dialog};
use glam::Vec2;
use omsi_launcher_lib::company::dealer as dl;
use omsi_launcher_lib::company::market;
use omsi_launcher_lib::company::{self as co, BusKind, Company, Tenure, Vehicle};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct FleetView {
    pub(super) tab: usize,
    pub(super) selected: Option<u32>,
    pub(super) dealer: super::dealer::DealerView,
    pub(super) adverts: super::adverts::AdvertsView,
}

/// The photo of a bus in a livery, once it is there.
type Photo = Option<(usize, u32, u32)>;

pub fn draw(l: &mut Launcher, area: Rect) {
    let Some(c) = l.company.company.clone() else { return };
    // (the photos read and taken while the page is open, as on the bus step)
    super::super::busphoto::work(l);
    let labels = [omsi_ui::tr("Our buses (%{n})").replace("%{n}", &c.fleet.len().to_string()), omsi_ui::tr("Dealer").into_owned(), omsi_ui::tr("Advertising").into_owned()];
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut tab = l.company.fleet.tab.min(2);
    if l.ui.segmented("company-fleet-tabs", Rect::new(area.x, area.y, 560.0f32.min(area.w), 40.0), &mut tab, &refs) {
        l.company.fleet.tab = tab;
    }
    let body = Rect::new(area.x, area.y + 40.0 + 16.0, area.w, (area.h - 40.0 - 16.0).max(0.0));
    match l.company.fleet.tab {
        1 => super::dealer::draw(l, body),
        2 => super::adverts::draw(l, body, &c),
        _ => our_buses(l, body, &c),
    }
}

/// The grid's layout: columns, tile width, photo height and tile height.
pub(super) fn layout(w: f32) -> (usize, f32, f32, f32) {
    let gap = 14.0;
    let cols = (((w + gap) / (270.0 + gap)).floor() as usize).max(1);
    let tw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
    let ph = tw * 0.52;
    (cols, tw, ph, ph + 118.0)
}

/// A tile's ground and photo (the bus's initials while it is drawn). Returns (hovered,
/// clicked) and the room under the photo.
pub(super) fn tile(ui: &mut Ui, r: Rect, id: &str, name: &str, pic: Photo) -> (bool, Rect) {
    let (h, _, clicked) = ui.interact(id_of(id), r);
    let t = ui.anim(id_of(id) ^ 0x7e1, if h { 1.0 } else { 0.0 }, 0.08);
    let fill = FIELD.mix(HOVER, t);
    let ph = r.w * 0.52;
    ui.p().rounded(r, SHEET_RADIUS, fill);
    let under = Rect::new(r.x, r.y, r.w, ph + SHEET_RADIUS);
    match pic {
        Some((tex, w, hh)) => ui.image_cover(under, tex, SHEET_RADIUS, w, hh),
        None => {
            ui.p().rounded_gradient(under, SHEET_RADIUS, Color::rgba(38, 48, 70, 1.0), Color::rgba(24, 31, 46, 1.0));
            let mono = Rect::new(r.center().x - 28.0, r.y + ph * 0.5 - 28.0, 56.0, 56.0);
            ui.p().rounded(mono, RADIUS, Color::WHITE.alpha(0.09));
            ui.text_in(&super::super::buspick::initials(name), mono, 20.0, Weight::Bold, TEXT, Align::Center);
        }
    }
    ui.p().rect(Rect::new(r.x, r.y + ph, r.w, SHEET_RADIUS), fill);
    ui.p().rounded(Rect::new(r.x, r.y + ph, r.w, r.h - ph), SHEET_RADIUS, fill);
    ui.p().rounded_border(r, SHEET_RADIUS, 1.0, EDGE.mix(accent().alpha(0.7), t));
    (clicked, Rect::new(r.x + 16.0, r.y + ph + 12.0, r.w - 32.0, r.h - ph - 22.0))
}

/// A bus's paint names: the bus's own first (empty), then its liveries.
pub(super) fn liveries_of(l: &Launcher, bus: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    if let Some(v) = l.state.vehicles.iter().find(|v| v.file == bus) {
        out.extend(v.paints.iter().filter(|p| !p.eq_ignore_ascii_case(&v.default_paint)).cloned());
    }
    out
}

pub(super) fn livery_label(p: &str) -> String {
    if p.is_empty() {
        omsi_ui::tr("Its own livery").into_owned()
    } else {
        p.to_string()
    }
}

/// What a bus is to the company, in a few words, and its colour.
fn status_of(c: &Company, v: &Vehicle) -> (String, Color) {
    if v.in_workshop(&c.date) {
        return (omsi_ui::tr("In the workshop until %{date}").replace("%{date}", &day_label(v.workshop_until.as_deref().unwrap_or(""))), WARN);
    }
    if v.km >= v.next_service_km - 1_000.0 {
        return (omsi_ui::tr("Service due").into_owned(), WARN);
    }
    match &v.tenure {
        Tenure::Rented { until, .. } => (omsi_ui::tr("Rented until %{date}").replace("%{date}", &day_label(until)), EARLY_SOFT),
        Tenure::Leased { until, .. } => (omsi_ui::tr("Leased until %{date}").replace("%{date}", &day_label(until)), EARLY_SOFT),
        Tenure::Owned { .. } => (omsi_ui::tr("Ready").into_owned(), OK),
    }
}

fn our_buses(l: &mut Launcher, area: Rect, c: &Company) {
    // the buses on order
    let area = if let Some(first) = c.dealer.orders.iter().min_by_key(|o| dl::minutes_of(&o.delivery)) {
        let n: u32 = c.dealer.orders.iter().map(|o| o.contract.count).sum();
        let text = omsi_ui::tr("%{n} buses on order: the next arrive on %{date}.").replace("%{n}", &n.to_string()).replace("%{date}", &day_label(&dl::day_of(&first.delivery)));
        l.ui.icon("schedule", Vec2::new(area.x + 10.0, area.y + 12.0), 18.0, accent_2());
        l.ui.text_in(&text, Rect::new(area.x + 28.0, area.y, area.w - 28.0, 24.0), kit::BODY, Weight::Medium, TEXT, Align::Left);
        Rect::new(area.x, area.y + 38.0, area.w, (area.h - 38.0).max(0.0))
    } else {
        area
    };
    if c.fleet.is_empty() {
        let h = l.ui.paragraph("The fleet is empty. Buy buses at the dealer - new, second-hand or one of the day's offers - or lease one for years or rent one for a few days: each is a bus installed in your OMSI.", Vec2::new(area.x, area.y + 4.0), area.w.min(860.0), kit::BODY, Weight::Regular, TEXT_SOFT);
        let bw = Foot::width(&l.ui, "To the dealer", Some("directions_bus"));
        if l.ui.button("company-fleet-to-market", Rect::new(area.x, area.y + h + 18.0, bw, 40.0), "To the dealer", Some("directions_bus"), ButtonKind::Primary) {
            l.company.fleet.tab = 1;
        }
        return;
    }
    let (cols, tw, _, th) = layout(area.w - 12.0);
    let gap = 14.0;
    let root = l.state.config.root.clone();
    let now = l.ui.time;
    let fleet = c.fleet.clone();
    let mut open = None;
    let Launcher { ui, showroom, .. } = l;
    ui.scroll_area("company-fleet", area, &mut |ui, v| {
        for (k, bus) in fleet.iter().enumerate() {
            let r = Rect::new(v.x + (k % cols) as f32 * (tw + gap), v.y + (k / cols) as f32 * (th + gap), tw, th);
            if !ui.rect_visible(r) {
                continue;
            }
            let pic = showroom.photos.get(&root, &bus.bus, &bus.livery, now);
            let (clicked, info) = tile(ui, r, &format!("company-bus-{}", bus.id), &bus.name, pic);
            // the fleet number on the photo, as on the bus
            let nw = ui.width(&bus.number, 15.0, Weight::Black) + 20.0;
            let badge = Rect::new(r.x + 10.0, r.y + 10.0, nw, 28.0);
            ui.p().rounded(badge, 6.0, Color::rgba(9, 12, 24, 0.85));
            ui.text_in(&bus.number, badge, 15.0, Weight::Black, TEXT, Align::Center);
            // (the advert it carries, on the photo's other corner)
            if let Some(k) = co::adverts::advert_of(c, bus.id) {
                let t = format!("{}  ·  {}", omsi_ui::tr("Ad"), k.advertiser);
                let aw = (ui.width(&t, 12.0, Weight::Bold) + 20.0).min(r.w - nw - 30.0);
                let ab = Rect::new(r.right() - 10.0 - aw, r.y + 10.0, aw, 28.0);
                ui.p().rounded(ab, 6.0, Color::rgba(9, 12, 24, 0.85));
                ui.text_in(&t, ab.pad(8.0, 0.0), 12.0, Weight::Bold, accent_2(), Align::Center);
            }
            ui.text_in(&bus.name, Rect::new(info.x, info.y, info.w, 24.0), 16.0, Weight::Bold, TEXT, Align::Left);
            let sub = format!("{}  ·  {}", bus.plate, omsi_ui::tr(bus.kind.label()));
            ui.text_in(&sub, Rect::new(info.x, info.y + 25.0, info.w, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            let facts = format!("{} km  ·  {}", grouped(bus.km.round()), omsi_ui::tr("%{n} years").replace("%{n}", &format!("{:.0}", bus.age_years(&c.date))));
            ui.text_in(&facts, Rect::new(info.x, info.y + 47.0, info.w * 0.62, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            let mr = Rect::new(info.x + info.w * 0.66, info.y + 55.0, info.w * 0.34, 6.0);
            meter(ui, mr, bus.condition / 100.0, grade(bus.condition));
            ui.tooltip(Rect::new(mr.x, mr.y - 9.0, mr.w, 24.0), &omsi_ui::tr("Condition: %{n} of 100").replace("%{n}", &format!("{:.0}", bus.condition)));
            let (status, colour) = status_of(c, bus);
            ui.p().circle(Vec2::new(info.x + 5.0, info.y + 82.0), 4.0, colour);
            ui.text_in(&status, Rect::new(info.x + 16.0, info.y + 71.0, info.w - 16.0, 22.0), kit::NOTE, Weight::Medium, colour, Align::Left);
            if clicked {
                open = Some(bus.id);
            }
        }
        fleet.len().div_ceil(cols) as f32 * (th + gap)
    });
    if let Some(id) = open {
        let liveries = liveries_of(l, &c.vehicle(id).map(|v| v.bus.clone()).unwrap_or_default());
        let livery = c.vehicle(id).and_then(|v| liveries.iter().position(|p| *p == v.livery)).unwrap_or(0);
        l.company.fleet.selected = Some(id);
        l.company.dialog = Some(Dialog::Vehicle { id, livery });
    }
}

// --- the dialogs -----------------------------------------------------------------------------

/// A row of a price table: what, and the amount.
pub(super) fn price_row(ui: &mut Ui, r: Rect, label: &str, value: &str, strong: bool) {
    ui.text_in(label, Rect::new(r.x, r.y, r.w * 0.6, r.h), kit::ROWS, Weight::Regular, TEXT_SOFT, Align::Left);
    ui.text_in(value, Rect::new(r.x + r.w * 0.36, r.y, r.w * 0.64, r.h), if strong { 16.5 } else { kit::ROWS }, if strong { Weight::Bold } else { Weight::Medium }, TEXT, Align::Right);
    ui.p().rect(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), HAIRLINE);
}

/// Paint a livery of the company's own for `bus` in the livery studio (`vehicle`: the bus of
/// the fleet painted in it once it is saved): refused with the company's popup when it cannot
/// pay for the design (and the painting).
pub(super) fn design_livery(l: &mut Launcher, bus: &str, paint: &str, vehicle: Option<u32>) {
    let Some(c) = l.company.company.clone() else { return };
    let mut cost = co::livery::design_fee(&c);
    if let Some(v) = vehicle.and_then(|id| c.vehicle(id)) {
        cost += co::livery::paint_cost(&c, v);
    }
    if c.cash < cost {
        kit::show(l, kit::no_cash(&c, cost));
        return;
    }
    super::super::livery::open_for_company(l, bus, paint, &c.id, vehicle);
}

/// The photo and the livery choice on a dialog's left (the company's own liveries marked);
/// `design`: the way to paint one of its own (Some(the fleet's bus painted in it), or Some(None)
/// for a bus of the dealer's). Returns the livery chosen.
#[allow(clippy::too_many_arguments)]
pub(super) fn bus_side(l: &mut Launcher, r: Rect, bus: &str, name: &str, kind: BusKind, livery: usize, liveries: &[String], design: Option<Option<u32>>) -> usize {
    let root = l.state.config.root.clone();
    let now = l.ui.time;
    let paint = liveries.get(livery).cloned().unwrap_or_default();
    let pic = l.showroom.photos.get(&root, bus, &paint, now);
    let ph = r.w * 0.62;
    let photo = Rect::new(r.x, r.y, r.w, ph);
    match pic {
        Some((tex, w, h)) => l.ui.image_cover(photo, tex, RADIUS, w, h),
        None => {
            l.ui.p().rounded_gradient(photo, RADIUS, Color::rgba(38, 48, 70, 1.0), Color::rgba(24, 31, 46, 1.0));
            l.ui.text_in(&super::super::buspick::initials(name), photo, 24.0, Weight::Bold, TEXT, Align::Center);
        }
    }
    l.ui.text_in(kind.label(), Rect::new(r.x, photo.bottom() + 10.0, r.w, 22.0), kit::BODY, Weight::Medium, TEXT_SOFT, Align::Left);
    l.ui.label(Rect::new(r.x, photo.bottom() + 42.0, r.w, 20.0), "Livery");
    let ours = |p: &str| l.company.company.as_ref().is_some_and(|c| co::livery::design(c, p).is_some());
    let names: Vec<String> = liveries.iter().map(|p| if !p.is_empty() && ours(p) { format!("{}  ·  {}", livery_label(p), omsi_ui::tr("our livery")) } else { livery_label(p) }).collect();
    let mut k = livery.min(names.len().saturating_sub(1));
    l.ui.select("company-dialog-livery", Rect::new(r.x, photo.bottom() + 66.0, r.w, 40.0), &mut k, &names);
    match design {
        Some(vehicle) => {
            let fee = l.company.company.as_ref().map(co::livery::design_fee).unwrap_or(0);
            let label = omsi_ui::tr("Design a livery (%{amount})").replace("%{amount}", &eur(fee));
            let b = Rect::new(r.x, photo.bottom() + 116.0, r.w, 36.0);
            if l.ui.button("company-design-livery", b, &label, Some("livery_fill"), ButtonKind::Ghost) {
                design_livery(l, bus, &paint, vehicle);
            }
            l.ui.tooltip(b, "Paint the company's own livery for this bus in the livery studio: its design is paid when it is saved, a bus's painting when the bus is painted.");
        }
        None => {
            l.ui.paragraph("A house livery of your own comes with the livery studio.", Vec2::new(r.x, photo.bottom() + 118.0), r.w, kit::NOTE, Weight::Regular, TEXT_SOFT);
        }
    }
    k
}

pub fn dialog(l: &mut Launcher) {
    let Some(c) = l.company.company.clone() else { return };
    match l.company.dialog.take() {
        Some(Dialog::New { bus, how, days, livery }) => {
            let f = kit::frame(l, 920.0, 600.0, "directions_bus", &bus.name);
            let inner = f.body;
            let liveries = liveries_of(l, &bus.file);
            let side_w = 290.0;
            let side = Rect::new(inner.x, inner.y, side_w, inner.h);
            let livery = bus_side(l, side, &bus.file, &bus.name, bus.kind, livery, &liveries, None);
            let right = Rect::new(inner.x + side_w + 28.0, inner.y, inner.w - side_w - 28.0, inner.h);
            let labels: Vec<String> = ["Lease", "Rent"].iter().map(|s| omsi_ui::tr(s).into_owned()).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut how = how.min(1);
            l.ui.segmented("company-new-how", Rect::new(right.x, right.y, right.w, 40.0), &mut how, &refs);
            let r = co::economy::rules(c.difficulty);
            let mut y = right.y + 40.0 + 18.0;
            let rh = 34.0;
            let mut days = days;
            let action: String;
            let cost: i64;
            if how == 0 {
                let (monthly, months, residual) = market::lease_offer(&c, &bus);
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &omsi_ui::tr("Monthly rate"), &eur(monthly), true);
                y += rh;
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &omsi_ui::tr("Term"), &omsi_ui::tr("%{n} months").replace("%{n}", &months.to_string()), false);
                y += rh;
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &omsi_ui::tr("Residual value"), &eur(residual), false);
                y += rh + 12.0;
                l.ui.paragraph("No price now: the rate is booked at every month's end, and the bus goes back when the term ends. Insurance is the company's. A lease with a haggled rate comes with the dealer's contract.", Vec2::new(right.x, y), right.w, kit::BODY, Weight::Regular, TEXT_SOFT);
                cost = monthly;
                action = omsi_ui::tr("Lease for %{amount} a month").replace("%{amount}", &eur(monthly));
            } else {
                let daily = co::economy::rent_per_day(bus.kind, &r, c.price_index);
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &omsi_ui::tr("Per day"), &eur(daily), false);
                y += rh + 12.0;
                l.ui.slider("company-rent-days", Rect::new(right.x, y, right.w, 40.0), &mut days, 1.0, 60.0, 1.0, "Days", &|v| format!("{v:.0}"));
                y += 52.0;
                let n = days.round().max(1.0) as i64;
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &omsi_ui::tr("Together"), &eur(daily * n), true);
                y += rh + 12.0;
                l.ui.paragraph("Paid by the day at each day's close; the bus goes back after the last day. A rented bus is a few years old and kept well.", Vec2::new(right.x, y), right.w, kit::BODY, Weight::Regular, TEXT_SOFT);
                cost = daily * n;
                action = omsi_ui::tr("Rent for %{n} days").replace("%{n}", &n.to_string());
            }
            let mut foot = Foot::new(&f);
            let go = foot.right(l, "company-new-do", &action, Some("check_circle"), ButtonKind::Primary);
            if foot.right(l, "company-new-cancel", "Cancel", None, ButtonKind::Normal) || f.close {
                return;
            }
            if go {
                if c.cash < cost {
                    kit::show(l, kit::no_cash(&c, cost));
                } else {
                    let paint = liveries.get(livery).cloned().unwrap_or_default();
                    let n = days.round().max(1.0) as u32;
                    let done = act(l, |c| if how == 0 { market::lease(c, &bus, &paint) } else { market::rent(c, &bus, n, &paint) });
                    if let Some(id) = done {
                        joined(l, id);
                        return;
                    }
                }
            }
            l.company.dialog = Some(Dialog::New { bus, how, days, livery });
        }
        Some(Dialog::Vehicle { id, livery }) => {
            let Some(v) = c.vehicle(id).cloned() else { return };
            let f = kit::frame(l, 920.0, 640.0, "directions_bus", &format!("{}  ·  {}", v.number, v.name));
            let inner = f.body;
            let liveries = liveries_of(l, &v.bus);
            let side_w = 290.0;
            let side = Rect::new(inner.x, inner.y, side_w, inner.h);
            let mut chosen = bus_side(l, side, &v.bus, &v.name, v.kind, livery, &liveries, Some(Some(id)));
            if chosen != livery {
                let paint = liveries.get(chosen).cloned().unwrap_or_default();
                if co::livery::design(&c, &paint).is_some() {
                    // (the company's own livery: painted in the workshop, paid)
                    let cost = co::livery::paint_cost(&c, &v);
                    if act(l, |c| co::livery::paint(c, id, &paint)).is_some() {
                        l.state.set_status(omsi_ui::tr("Bus %{n} is painted in '%{name}' in the workshop (%{amount}).").replace("%{n}", &v.number).replace("%{name}", &paint).replace("%{amount}", &eur(cost)), false);
                    } else {
                        chosen = livery;
                    }
                } else {
                    act(l, |c| {
                        market::set_livery(c, id, &paint);
                        Ok(())
                    });
                }
            }
            let right = Rect::new(inner.x + side_w + 28.0, inner.y, inner.w - side_w - 28.0, inner.h);
            let rh = 34.0;
            let mut y = right.y;
            let rows: Vec<(String, String)> = vec![
                (omsi_ui::tr("Plate").into_owned(), v.plate.clone()),
                (omsi_ui::tr("Built").into_owned(), format!("{}  ({})", v.built.get(..4).unwrap_or(""), omsi_ui::tr("%{n} years").replace("%{n}", &super::num(v.age_years(&c.date), 1)))),
                (omsi_ui::tr("Kilometres").into_owned(), format!("{} km", grouped(v.km.round()))),
                (omsi_ui::tr("Next service").into_owned(), format!("{} km", grouped(v.next_service_km))),
                (omsi_ui::tr("Condition").into_owned(), format!("{:.0} / 100", v.condition)),
                (omsi_ui::tr("Breakdowns").into_owned(), v.breakdowns.to_string()),
                {
                    // (who may drive it: the licence, the endorsements, the type training)
                    let (q, m) = co::licences::qualified_drivers(&c, v.kind, &v.bus);
                    (omsi_ui::tr("Drivers").into_owned(), omsi_ui::tr("%{n} of %{m} may drive it").replace("%{n}", &q.to_string()).replace("%{m}", &m.to_string()))
                },
                (
                    omsi_ui::tr("Advert").into_owned(),
                    match co::adverts::advert_of(&c, id) {
                        Some(k) => format!("{}  ·  {}  ·  {}", k.advertiser, omsi_ui::tr(k.kind.label()), omsi_ui::tr("until %{date}").replace("%{date}", &day_label(&k.until))),
                        None => omsi_ui::tr("none").into_owned(),
                    },
                ),
                match &v.tenure {
                    Tenure::Owned { paid, .. } => (omsi_ui::tr("Bought for").into_owned(), format!("{}  ·  {} {}", eur(*paid), omsi_ui::tr("worth now"), eur(market::value_of(&c, &v)))),
                    Tenure::Leased { monthly, until, .. } => (omsi_ui::tr("Leased").into_owned(), format!("{} / {}  ·  {}", eur(*monthly), omsi_ui::tr("month"), day_label(until))),
                    Tenure::Rented { daily, until } => (omsi_ui::tr("Rented").into_owned(), format!("{} / {}  ·  {}", eur(*daily), omsi_ui::tr("day"), day_label(until))),
                },
            ];
            for (k, val) in rows {
                price_row(&mut l.ui, Rect::new(right.x, y, right.w, rh), &k, &val, false);
                y += rh;
            }
            let (status, colour) = status_of(&c, &v);
            l.ui.text_in(&status, Rect::new(right.x, y + 10.0, right.w, 26.0), kit::BODY, Weight::Bold, colour, Align::Left);
            let sell = match v.tenure {
                Tenure::Owned { .. } => "Sell",
                _ => "Give back",
            };
            let mut foot = Foot::new(&f);
            if foot.right(l, "company-bus-close", "Close", None, ButtonKind::Primary) || f.close {
                l.company.fleet.selected = None;
                return;
            }
            if foot.left(l, "company-bus-sell", sell, None, ButtonKind::Danger) {
                l.company.dialog = Some(Dialog::Confirm { what: Confirm::Sell(id) });
                return;
            }
            if foot.left(l, "company-bus-service", "Service tomorrow", Some("construction"), ButtonKind::Normal) && act(l, |c| market::service(c, id)).is_some() {
                l.state.set_status(omsi_ui::tr("Bus %{n} goes to the workshop tomorrow.").replace("%{n}", &v.number), false);
            }
            if foot.left(l, "company-bus-train", "Train drivers", Some("groups"), ButtonKind::Normal) {
                l.company.fleet.selected = None;
                l.company.dialog = None;
                super::people::to_training(l);
                return;
            }
            l.company.dialog = Some(Dialog::Vehicle { id, livery: chosen });
        }
        other => l.company.dialog = other,
    }
}

/// A bus joined the fleet: say so, and show the fleet.
fn joined(l: &mut Launcher, id: u32) {
    let number = l.company.company.as_ref().and_then(|c| c.vehicle(id)).map(|v| v.number.clone()).unwrap_or_default();
    let mut text = omsi_ui::tr("Bus %{n} joined the fleet.").replace("%{n}", &number);
    if let Some(note) = l.company.company.as_ref().and_then(|c| super::people::drivers_note(c, id)) {
        text = format!("{text} {note}");
    }
    l.state.set_status(text, false);
    l.company.dialog = None;
    l.company.fleet.tab = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fits_its_tiles() {
        let (cols, tw, ph, th) = layout(1100.0);
        assert_eq!(cols, 3);
        assert!(tw >= 270.0 && ph < tw && th > ph + 100.0);
    }
}
