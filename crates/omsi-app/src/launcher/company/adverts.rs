//! Advertising on the company's buses (the rules are `company::adverts`'): the week's offers of
//! the advertisers - a kind the company's level does not open yet shown locked, with what opens
//! it -, the contract of an offer, signed with the same paper and pen as the dealer's
//! (`dealer::parties`, `dealer::signature`, `signing`), and the contracts running, each to look
//! at or to end early (pressed twice: it costs).

use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::dealer::{self, Sheet};
use super::fleet::price_row;
use super::kit::{self, Foot};
use super::signing::{self, Signing, State};
use super::{act, day_label, eur, section};
use glam::Vec2;
use omsi_launcher_lib::company::adverts::{self as ad, AdContract, AdKind};
use omsi_launcher_lib::company::levels;
use omsi_launcher_lib::company::Company;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct AdvertsView {
    /// "End early" pressed once on this contract: pressed again it ends.
    end_armed: Option<u32>,
}

fn kind_colour(k: AdKind) -> Color {
    match k {
        AdKind::Rear => accent_2(),
        AdKind::Sides => EARLY_SOFT,
        AdKind::FullWrap => OK,
    }
}

/// The advertising page: the week's offers on the left, the contracts running on the right.
pub fn draw(l: &mut Launcher, area: Rect, c: &Company) {
    let gap = 16.0;
    let left_w = ((area.w - gap) * 0.55).max(360.0);
    offers(l, Rect::new(area.x, area.y, left_w, area.h), c);
    running(l, Rect::new(area.x + left_w + gap, area.y, area.w - left_w - gap, area.h), c);
}

fn offers(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Advertisers' offers this week");
    let list = ad::offers(c);
    let free = ad::free_buses(c).len();
    let note = omsi_ui::tr("%{n} of your buses carry no advert yet. Advertisers pay by the month, more for buses many people see.").replace("%{n}", &free.to_string());
    let nh = l.ui.paragraph(&note, Vec2::new(inner.x, inner.y), inner.w, kit::NOTE + 0.5, Weight::Regular, TEXT_SOFT);
    let top = inner.y + nh + 12.0;
    if list.is_empty() {
        l.ui.text_in("No more offers this week: new ones come on Monday.", Rect::new(inner.x, top, inner.w, 24.0), kit::BODY, Weight::Regular, TEXT_SOFT, Align::Left);
        return;
    }
    let rh = 96.0;
    let mut open: Option<usize> = None;
    let mut locked: Option<usize> = None;
    let cc = c.clone();
    l.ui.scroll_area("company-ad-offers", Rect::new(inner.x, top, inner.w, (inner.bottom() - top).max(40.0)), &mut |ui, v| {
        for (k, o) in list.iter().enumerate() {
            let rr = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 10.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            let lock = !levels::unlocked(&cc, o.kind.feature());
            ui.p().rounded(rr, RADIUS, FIELD);
            ui.p().rounded(Rect::new(rr.x, rr.y, 4.0, rr.h), 2.0, kind_colour(o.kind));
            ui.text_in(&o.advertiser, Rect::new(rr.x + 18.0, rr.y + 10.0, rr.w * 0.55, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            ui.text_in(&omsi_ui::tr(&o.trade), Rect::new(rr.x + 18.0, rr.y + 34.0, rr.w * 0.55, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            let terms = omsi_ui::tr("%{kind} on %{n} buses for %{m} months").replace("%{kind}", &omsi_ui::tr(o.kind.label())).replace("%{n}", &o.buses.to_string()).replace("%{m}", &o.months.to_string());
            ui.text_in(&terms, Rect::new(rr.x + 18.0, rr.y + 58.0, rr.w * 0.6, 20.0), kit::NOTE, Weight::Medium, kind_colour(o.kind), Align::Left);
            let money = omsi_ui::tr("%{amount} a month").replace("%{amount}", &eur(o.monthly()));
            ui.text_in(&money, Rect::new(rr.right() - 230.0, rr.y + 10.0, 214.0, 24.0), kit::ROWS, Weight::Bold, if lock { TEXT_DIM } else { TEXT }, Align::Right);
            let all = omsi_ui::tr("%{amount} in all").replace("%{amount}", &eur(o.total()));
            ui.text_in(&all, Rect::new(rr.right() - 230.0, rr.y + 34.0, 214.0, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Right);
            let b = Rect::new(rr.right() - 196.0, rr.y + 54.0, 180.0, 30.0);
            if lock {
                let label = omsi_ui::tr("From level %{n}").replace("%{n}", &o.kind.feature().level().to_string());
                if ui.button(&format!("company-ad-lock-{}", o.no), b, &label, Some("lock"), ButtonKind::Ghost) {
                    locked = Some(k);
                }
            } else if ui.button(&format!("company-ad-open-{}", o.no), b, "To the contract", Some("description"), ButtonKind::Primary) {
                open = Some(k);
            }
        }
        list.len() as f32 * rh
    });
    if let Some(k) = locked {
        kit::show(l, kit::locked(c, list[k].kind.feature()));
    } else if let Some(k) = open {
        match ad::draft(c, &list[k]) {
            Ok(contract) => dealer::open(l, Sheet::Advert { contract, strokes: Vec::new(), readonly: false }),
            Err(e) => kit::refuse(l, e),
        }
    }
}

fn running(l: &mut Launcher, r: Rect, c: &Company) {
    let inner = section(&mut l.ui, r, "Advertising contracts");
    let list: Vec<AdContract> = c.adverts.contracts.iter().filter(|k| k.running()).cloned().collect();
    if list.is_empty() {
        l.ui.paragraph("No advert on the buses yet. Sign an advertiser's offer: the money comes at every month's end, from the month after the signing.", Vec2::new(inner.x, inner.y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
        return;
    }
    let armed = l.company.fleet.adverts.end_armed;
    let rh = 104.0;
    let mut look: Option<usize> = None;
    let mut end: Option<u32> = None;
    let cc = c.clone();
    l.ui.scroll_area("company-ad-running", inner, &mut |ui, v| {
        for (k, x) in list.iter().enumerate() {
            let rr = Rect::new(v.x, v.y + k as f32 * rh, v.w - 10.0, rh - 10.0);
            if !ui.rect_visible(rr) {
                continue;
            }
            ui.p().rounded(rr, RADIUS, FIELD);
            ui.p().rounded(Rect::new(rr.x, rr.y, 4.0, rr.h), 2.0, kind_colour(x.kind));
            ui.text_in(&x.advertiser, Rect::new(rr.x + 18.0, rr.y + 8.0, rr.w * 0.6, 24.0), kit::ROWS, Weight::Bold, TEXT, Align::Left);
            let buses: Vec<String> = x.buses.iter().filter_map(|id| cc.vehicle(*id)).map(|v| v.number.clone()).collect();
            let what = format!("{}  ·  {}", omsi_ui::tr(x.kind.label()), omsi_ui::tr("buses %{list}").replace("%{list}", &buses.join(", ")));
            ui.text_in(&what, Rect::new(rr.x + 18.0, rr.y + 32.0, rr.w - 36.0, 20.0), kit::NOTE, Weight::Medium, kind_colour(x.kind), Align::Left);
            let until = omsi_ui::tr("%{amount} a month, until %{date}").replace("%{amount}", &eur(x.monthly())).replace("%{date}", &day_label(&x.until));
            ui.text_in(&until, Rect::new(rr.x + 18.0, rr.y + 54.0, rr.w - 36.0, 20.0), kit::NOTE, Weight::Regular, TEXT_SOFT, Align::Left);
            if ui.button(&format!("company-ad-look-{}", x.no), Rect::new(rr.right() - 330.0, rr.y + 8.0, 130.0, 30.0), "The contract", None, ButtonKind::Ghost) {
                look = Some(k);
            }
            let pen = ad::penalty(x);
            let label = if armed == Some(x.no) { omsi_ui::tr("Press again: %{amount}").replace("%{amount}", &eur(pen)) } else { omsi_ui::tr("End early (%{amount})").replace("%{amount}", &eur(pen)) };
            let b = Rect::new(rr.right() - 192.0, rr.y + 8.0, 180.0, 30.0);
            if ui.button(&format!("company-ad-end-{}", x.no), b, &label, None, if armed == Some(x.no) { ButtonKind::Danger } else { ButtonKind::Ghost }) {
                end = Some(x.no);
            }
            ui.tooltip(b, "Ending a contract early costs three months' fee (or what is left, if less); its buses are free for another advert.");
        }
        list.len() as f32 * rh
    });
    if let Some(k) = look {
        dealer::open(l, Sheet::Advert { contract: list[k].clone(), strokes: Vec::new(), readonly: true });
    }
    if let Some(no) = end {
        if armed == Some(no) {
            l.company.fleet.adverts.end_armed = None;
            if let Some(fee) = act(l, |c| ad::end_early(c, no)) {
                l.state.set_status(omsi_ui::tr("The advertising contract is ended: %{amount} paid to the advertiser.").replace("%{amount}", &eur(fee)), false);
            }
        } else {
            l.company.fleet.adverts.end_armed = Some(no);
        }
    }
}

/// The advertising contract: the advertiser and the company, the terms, the buses that carry
/// it, and the signature; signed, the pen goes over the paper and the contract is the
/// company's.
pub(super) fn contract_sheet(l: &mut Launcher, c: &Company, contract: AdContract, strokes: Vec<Vec<Vec2>>, readonly: bool, esc: bool) -> Option<Sheet> {
    let signing = l.company.fleet.dealer.signing.clone();
    let held = signing.as_ref().map(|_| super::mask(&mut l.ui));
    let esc = esc && signing.is_none();
    let title = if readonly { omsi_ui::tr("Advertising contract no. %{no}").replace("%{no}", &contract.no.to_string()) } else { omsi_ui::tr("Advertising contract").into_owned() };
    let f = kit::frame(l, 980.0, 720.0, "campaign", &title);
    let inner = f.body;
    let (mut k, mut strokes) = (contract, strokes);
    let home = if c.map_name.is_empty() { c.depot.clone() } else { format!("{}  ·  {}", c.map_name, c.depot) };
    let trade = omsi_ui::tr(&k.trade).into_owned();
    let y = dealer::parties(l, inner, ("Advertiser", &k.advertiser, &trade), ("Bus company", &c.name, &home));
    let gap = 22.0;
    let cw = (inner.w - gap) / 2.0;
    // the terms
    kit::caps(&mut l.ui, Rect::new(inner.x, y, cw, 16.0), "The advert");
    let mut ly = y + 24.0;
    let rows: Vec<(String, String)> = vec![
        (omsi_ui::tr("Where").into_owned(), omsi_ui::tr(k.kind.label()).into_owned()),
        (omsi_ui::tr("Buses").into_owned(), k.buses.len().to_string()),
        (omsi_ui::tr("Term").into_owned(), omsi_ui::tr("%{n} months").replace("%{n}", &k.months.to_string())),
        (omsi_ui::tr("From").into_owned(), day_label(&k.from)),
        (omsi_ui::tr("Until").into_owned(), day_label(&k.until)),
        (omsi_ui::tr("A bus a month").into_owned(), eur(k.per_bus)),
        (omsi_ui::tr("A month in all").into_owned(), eur(k.monthly())),
        (omsi_ui::tr("Over the term").into_owned(), eur(k.monthly() * k.months as i64)),
    ];
    for (a, b) in rows {
        price_row(&mut l.ui, Rect::new(inner.x, ly, cw, 30.0), &a, &b, false);
        ly += 30.0;
    }
    ly += 8.0;
    let terms = omsi_ui::tr("Paid at every month's end, from the month after the signing. Ending it early costs three months' fee (%{amount}). The advertiser pays the printing and the foil.").replace("%{amount}", &eur(ad::penalty(&k)));
    l.ui.paragraph(&terms, Vec2::new(inner.x, ly), cw, kit::NOTE + 0.5, Weight::Regular, TEXT_SOFT);
    // the buses that carry it
    let rx = inner.x + cw + gap;
    kit::caps(&mut l.ui, Rect::new(rx, y, cw, 16.0), "The buses that carry it");
    let mut by = y + 24.0;
    for id in &k.buses {
        let Some(v) = c.vehicle(*id) else { continue };
        price_row(&mut l.ui, Rect::new(rx, by, cw, 30.0), &format!("{}  {}", v.number, v.name), &omsi_ui::tr(v.kind.label()), false);
        by += 30.0;
        if by > inner.bottom() - dealer::SIGNATURE_H - 40.0 {
            break;
        }
    }
    // the signature
    let day = if readonly { k.signed_at.clone() } else { c.date.clone() };
    let kept = dealer::signature(l, inner, &mut strokes, &mut k.signed_by, &k.strokes.clone(), readonly, &day);
    let mut foot = Foot::new(&f);
    if readonly {
        if foot.right(l, "company-ad-contract-close", "Close", None, ButtonKind::Primary) || esc || f.close {
            return None;
        }
        return Some(Sheet::Advert { contract: k, strokes, readonly });
    }
    k.strokes = kept;
    let signed = k.is_signed();
    let sign = foot.right(l, "company-ad-sign", "Sign the advertising contract", Some("check_circle"), ButtonKind::Primary);
    if foot.right(l, "company-ad-contract-back", "Back", Some("chevron_left"), ButtonKind::Normal) || esc || (f.close && signing.is_none()) {
        return None;
    }
    if !signed {
        l.ui.text_in("Sign the contract first.", foot.rest(), kit::BODY, Weight::Medium, TEXT_SOFT, Align::Left);
    }
    let mut book = false;
    if let (Some(s), Some(i)) = (signing.as_ref(), held) {
        l.ui.input = i;
        if signing::draw(l, s) == State::Done {
            l.company.fleet.dealer.signing = None;
            book = true;
        }
    } else if sign {
        if signed {
            l.company.fleet.dealer.signing = Some(Signing::new(l.ui.time, &k.strokes, &k.signed_by, &omsi_ui::tr("Advertising contract")));
        } else {
            kit::refuse(l, "Sign the contract first.");
        }
    }
    if book {
        if let Some(no) = act(l, |c| ad::sign(c, &k)) {
            l.state.set_status(omsi_ui::tr("Advertising contract %{no} is signed: %{amount} a month from next month.").replace("%{no}", &no.to_string()).replace("%{amount}", &eur(k.monthly())), false);
            return None;
        }
    }
    Some(Sheet::Advert { contract: k, strokes, readonly })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_advertising_is_translated() {
        let mut keys: Vec<&str> = AdKind::ALL.iter().map(|k| k.label()).collect();
        keys.extend(AdKind::ALL.iter().map(|k| k.feature().label()));
        keys.extend(AdKind::ALL.iter().map(|k| k.locked_reason()));
        keys.extend(ad::ADVERTISERS.iter().map(|a| a.1));
        keys.extend(["Advertising", "Liveries and painting", "Advertising contract", "Sign the advertising contract"]);
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in &keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
    }
}
