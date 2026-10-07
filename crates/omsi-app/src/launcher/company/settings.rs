//! The company's settings: its mark (a logo picture, or its monogram), its date (Luc: a
//! company date of one's own, later on as well) and how it buys its buses. Moving the date moves the company's clock to that day's midnight and
//! simulates nothing in between; it may not go back before the company's last booking
//! (`clock::move_to`).

use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::kit::{self, Foot};
use super::{act, day_label, Dialog};
use glam::Vec2;
use omsi_launcher_lib::company::{self as co, clock as ck, dealer::BuyingMode};
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};

pub(super) fn dialog(l: &mut Launcher) {
    let Some(Dialog::Settings { date }) = &l.company.dialog else { return };
    let date = date.clone();
    let Some(c) = l.company.company.clone() else {
        l.company.dialog = None;
        return;
    };
    let f = kit::frame(l, 760.0, 690.0, "settings", &omsi_ui::tr("The company's settings"));
    let inner = f.body;
    let mut y = inner.y;
    // its mark
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "The company's mark");
    y += 26.0;
    super::company_mark(l, Rect::new(inner.x, y, 68.0, 68.0), &c);
    let logo_text = c.logo.as_deref().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()).unwrap_or_else(|| omsi_ui::tr("No logo picture: the short name is the mark.").into_owned());
    l.ui.text_in(&logo_text, Rect::new(inner.x + 84.0, y + 2.0, inner.w - 84.0, 22.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    let lw = Foot::width(&l.ui, "Choose a logo picture", Some("photo_camera"));
    if l.ui.button("company-settings-logo", Rect::new(inner.x + 84.0, y + 30.0, lw, 36.0), "Choose a logo picture", Some("photo_camera"), ButtonKind::Normal) {
        if let Some(p) = omsi_launcher_lib::pick_file("Choose a logo picture") {
            let p = p.to_string_lossy().to_string();
            act(l, |c| {
                c.logo = Some(p);
                Ok(())
            });
        }
    }
    if c.logo.is_some() {
        let rw = Foot::width(&l.ui, "Remove the picture", Some("close"));
        if l.ui.button("company-settings-logo-remove", Rect::new(inner.x + 84.0 + lw + 10.0, y + 30.0, rw, 36.0), "Remove the picture", Some("close"), ButtonKind::Normal) {
            act(l, |c| {
                c.logo = None;
                Ok(())
            });
        }
    }
    y += 68.0 + 26.0;
    // how it buys
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "Buying buses");
    y += 26.0;
    let modes: Vec<String> = BuyingMode::ALL.iter().map(|m| omsi_ui::tr(m.label()).into_owned()).collect();
    let refs: Vec<&str> = modes.iter().map(String::as_str).collect();
    let mut mode = BuyingMode::ALL.iter().position(|m| *m == c.dealer.mode).unwrap_or(1);
    if l.ui.segmented("company-settings-buying", Rect::new(inner.x, y, 320.0f32.min(inner.w), 40.0), &mut mode, &refs) {
        let m = BuyingMode::ALL[mode];
        act(l, |c| {
            c.dealer.mode = m;
            Ok(())
        });
    }
    let say = if mode == 0 { "A model, a number, the list price: the buses are yours at once." } else { "Haggle with the dealer, agree on extras and sign a contract; new buses are delivered." };
    l.ui.paragraph(say, Vec2::new(inner.x + 340.0, y + 2.0), (inner.w - 340.0).max(120.0), kit::NOTE, Weight::Regular, TEXT_SOFT);
    y += 64.0;
    // its date
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "The company's date");
    y += 26.0;
    let now = omsi_ui::tr("Today it is %{date}, %{time}.").replace("%{date}", &day_label(&c.date)).replace("%{time}", &ck::hhmm(ck::now(&c)));
    l.ui.text_in(&now, Rect::new(inner.x, y, inner.w, 24.0), kit::BODY, Weight::Medium, TEXT, Align::Left);
    y += 34.0;
    let mut d = date;
    l.ui.date_field("company-settings-date", Rect::new(inner.x, y, 240.0, 40.0), &mut d);
    let earliest = ck::earliest_date(&c);
    if let Some(e) = &earliest {
        let t = omsi_ui::tr("Not before %{date}: the company's last booking.").replace("%{date}", &day_label(e));
        l.ui.text_in(&t, Rect::new(inner.x + 256.0, y, inner.w - 256.0, 40.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
    }
    y += 56.0;
    let warn = "Moving the date changes what the company finds there: the dealer's offers and the used market of that day, deliveries and contracts by their own dates, concessions and leases running to theirs. The days between are not simulated: they bring no money and no wear.";
    l.ui.icon("warning", Vec2::new(inner.x + 10.0, y + 11.0), 20.0, WARN);
    l.ui.paragraph(warn, Vec2::new(inner.x + 32.0, y), inner.w - 32.0, kit::BODY, Weight::Regular, TEXT_SOFT);
    let mut foot = Foot::new(&f);
    // (deleting it: asked again, it cannot be undone)
    if foot.left(l, "company-settings-delete", "Delete the company", Some("delete"), ButtonKind::Danger) {
        l.company.dialog = Some(super::Dialog::Confirm { what: super::Confirm::DeleteCompany(c.id.clone()) });
        return;
    }
    let moving = co::dates::parse(&d).is_some() && d != c.date;
    let label = omsi_ui::tr("Move to %{date}").replace("%{date}", &day_label(&d));
    let go = moving && foot.right(l, "company-settings-move", &label, Some("calendar_month"), ButtonKind::Primary);
    if foot.right(l, "company-settings-close", "Close", None, if moving { ButtonKind::Normal } else { ButtonKind::Primary }) || f.close {
        l.company.dialog = None;
        return;
    }
    if go {
        let to = d.clone();
        if act(l, |c| ck::move_to(c, &to)).is_some() {
            l.company.dialog = None;
            l.company.plan = None;
            l.state.set_status(omsi_ui::tr("The company is at %{date} now.").replace("%{date}", &day_label(&to)), false);
            return;
        }
    }
    l.company.dialog = Some(Dialog::Settings { date: d });
}
