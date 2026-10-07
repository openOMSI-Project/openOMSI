//! The bank (the rules are `company::finance`'s): a loan's dialog - how much, over how long,
//! what it costs a month and in all, what it does to the cash and to what the bank still
//! lends - and its contract, signed in the same way as the dealer's (`dealer::parties`,
//! `dealer::signature`, and the pen of `signing`) before anything is booked; and paying a loan
//! back early, with its fee. A loan the dealer's quick buy or contract asks for comes here
//! first, and both are signed at once (`finance::with_loan`).

use super::super::theme::*;
use super::super::ui::ButtonKind;
use super::super::Launcher;
use super::dealer::{self, Sheet, Then};
use super::fleet::price_row;
use super::kit::{self, Foot};
use super::signing::{self, Signing, State};
use super::{act, day_label, eur};
use glam::Vec2;
use omsi_launcher_lib::company::dealer as dl;
use omsi_launcher_lib::company::finance::{self as fi, LoanContract, Room};
use omsi_launcher_lib::company::market::Payment;
use omsi_launcher_lib::company::Company;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// The colour of the credit room.
pub(super) fn room_colour(r: Room) -> Color {
    match r {
        Room::Plenty => OK,
        Room::Little => WARN,
        Room::None => DANGER.lighten(0.15),
    }
}

/// "%{rate} %" with one decimal ("4.5 %", "0 %").
pub(super) fn percent(rate: f64) -> String {
    let v = rate * 100.0;
    if (v - v.round()).abs() < 0.05 {
        format!("{} %", super::num(v, 0))
    } else {
        format!("{} %", super::num(v, 1))
    }
}

/// Open the loan's dialog from the finances page.
pub(super) fn open_loan(l: &mut Launcher) {
    let Some(c) = l.company.company.as_ref() else { return };
    let left = fi::credit(c, 0).left;
    let amount = (100_000_00i64).min(left).max(fi::SMALLEST_LOAN) / 100;
    dealer::open(l, Sheet::Loan { amount: amount as f32, term: usize::MAX, purpose: String::new(), collateral: 0, fixed: false, then: Then::Nothing });
}

/// The rows of a sheet: what, and the amount (in its colour when it has one).
fn rows(l: &mut Launcher, r: Rect, y: &mut f32, list: &[(String, String, Option<Color>)]) {
    let rh = 32.0;
    for (a, b, colour) in list {
        price_row(&mut l.ui, Rect::new(r.x, *y, r.w, rh), a, b, false);
        if let Some(c) = colour {
            // (the value in its colour over the plain one)
            l.ui.p().rect(Rect::new(r.x + r.w * 0.45, *y + 2.0, r.w * 0.55, rh - 4.0), PANEL);
            l.ui.text_in(b, Rect::new(r.x + r.w * 0.4, *y, r.w * 0.6, rh), kit::ROWS, Weight::Bold, *c, Align::Right);
        }
        *y += rh;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn loan_sheet(l: &mut Launcher, c: &Company, amount: f32, term: usize, purpose: String, collateral: i64, fixed: bool, then: Then, esc: bool) -> Option<Sheet> {
    let f = kit::frame(l, 860.0, if fixed { 640.0 } else { 740.0 }, "payments", &omsi_ui::tr("A loan from the bank"));
    let inner = f.body;
    let cr = fi::credit(c, collateral);
    let terms = fi::terms_offered(c);
    let longest = terms.len().saturating_sub(1);
    let mut term = if term == usize::MAX { longest } else { term.min(longest) };
    let mut amount = amount;
    let mut y = inner.y;
    if fixed {
        let t = omsi_ui::tr("For %{what}: the bank lends what is to be paid, and the buses are its security.").replace("%{what}", &purpose);
        y += l.ui.paragraph(&t, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT) + 10.0;
    }
    // how much
    kit::caps(&mut l.ui, Rect::new(inner.x, y, inner.w, 16.0), "Amount");
    let cents = (amount.round() as i64) * 100;
    l.ui.text_in(&eur(cents), Rect::new(inner.x, y + 18.0, inner.w * 0.5, 42.0), 34.0, Weight::Bold, TEXT, Align::Left);
    let left_t = omsi_ui::tr("The bank lends up to %{amount} more").replace("%{amount}", &eur(cr.left));
    l.ui.text_in(&left_t, Rect::new(inner.x + inner.w * 0.4, y + 28.0, inner.w * 0.6, 26.0), kit::BODY, Weight::Medium, room_colour(fi::room(&cr)), Align::Right);
    y += 70.0;
    if !fixed {
        let max = (cr.left.max(fi::SMALLEST_LOAN) / 100) as f32;
        l.ui.slider("company-loan-amount", Rect::new(inner.x, y, inner.w, 36.0), &mut amount, (fi::SMALLEST_LOAN / 100) as f32, max, 5_000.0, "", &|v| format!("{:.0}k", v / 1000.0));
        y += 46.0;
        let presets: Vec<i64> = [50_000_00i64, 100_000_00, 250_000_00, 500_000_00].into_iter().filter(|p| *p <= cr.left).collect();
        let mut labels: Vec<String> = presets.iter().map(|p| eur(*p)).collect();
        labels.push(omsi_ui::tr("All that is left").into_owned());
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut k = presets.iter().position(|p| *p == cents).unwrap_or(if cents == cr.left { presets.len() } else { usize::MAX });
        if l.ui.chips("company-loan-presets", Rect::new(inner.x, y, inner.w, 34.0), &mut k, &refs) {
            amount = (presets.get(k).copied().unwrap_or(cr.left) / 100) as f32;
        }
        y += l.ui.chips_height(inner.w, 34.0, &refs) + 16.0;
    }
    // over how long
    l.ui.label(Rect::new(inner.x, y, inner.w, 20.0), "Term");
    y += 24.0;
    let labels: Vec<String> = terms.iter().map(|m| omsi_ui::tr("%{n} months").replace("%{n}", &m.to_string())).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    l.ui.chips("company-loan-term", Rect::new(inner.x, y, inner.w, 34.0), &mut term, &refs);
    y += l.ui.chips_height(inner.w, 34.0, &refs) + 16.0;
    // what it costs, and what it does
    let cents = (amount.round() as i64) * 100;
    let months = terms.get(term).copied().unwrap_or(12);
    let k = fi::draft_loan(c, cents, months, &purpose, collateral);
    let after = (cr.left - cents).max(0);
    let after_room = fi::room(&fi::Credit { left: after, ..cr });
    let rate = if cr.discount > 0.0 { omsi_ui::tr("%{rate} a year (%{discount} less for the company's level)").replace("%{rate}", &percent(k.rate)).replace("%{discount}", &percent(cr.discount)) } else { omsi_ui::tr("%{rate} a year").replace("%{rate}", &percent(k.rate)) };
    let list = vec![
        (omsi_ui::tr("Interest").into_owned(), rate, None),
        (omsi_ui::tr("Monthly payment").into_owned(), eur(k.monthly), Some(TEXT)),
        (omsi_ui::tr("Paid back in all").into_owned(), eur(k.total), None),
        (omsi_ui::tr("Of which interest").into_owned(), eur(k.interest()), None),
        (omsi_ui::tr("First payment").into_owned(), day_label(&k.first_rate), None),
        (omsi_ui::tr("Cash afterwards").into_owned(), if fixed { eur(c.cash) } else { eur(c.cash + cents) }, None),
        (omsi_ui::tr("Credit room afterwards").into_owned(), eur(after), Some(room_colour(after_room))),
    ];
    rows(l, Rect::new(inner.x, y, inner.w, 0.0), &mut y, &list);
    let ok = cents >= fi::SMALLEST_LOAN.min(cents.max(1)) && cents > 0 && cents <= cr.left;
    let mut foot = Foot::new(&f);
    let draw_up = foot.right(l, "company-loan-draw-up", "Draw up the loan contract", Some("description"), ButtonKind::Primary);
    if foot.right(l, "company-loan-cancel", "Cancel", None, ButtonKind::Normal) || esc || f.close {
        return back_from_loan(then);
    }
    if draw_up {
        if ok {
            return Some(Sheet::LoanContract { contract: k, strokes: Vec::new(), readonly: false, collateral, then });
        }
        kit::show(l, kit::no_credit(c, cents));
    }
    Some(Sheet::Loan { amount, term, purpose, collateral, fixed, then })
}

/// Leaving a loan's dialog: back to the purchase that asked for it.
fn back_from_loan(then: Then) -> Option<Sheet> {
    match then {
        Then::Purchase(k) => Some(Sheet::Contract { contract: *k, strokes: Vec::new(), readonly: false }),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn loan_contract_sheet(l: &mut Launcher, c: &Company, contract: LoanContract, strokes: Vec<Vec<Vec2>>, readonly: bool, collateral: i64, then: Then, esc: bool) -> Option<Sheet> {
    // (being signed: the contract under the pen takes nothing)
    let signing = l.company.fleet.dealer.signing.clone();
    let held = signing.as_ref().map(|_| super::mask(&mut l.ui));
    let esc = esc && signing.is_none();
    let title = if readonly { omsi_ui::tr("Loan contract no. %{no}").replace("%{no}", &contract.no.to_string()) } else { omsi_ui::tr("Loan contract").into_owned() };
    let f = kit::frame(l, 1020.0, 820.0, "description", &title);
    let inner = f.body;
    let (mut k, mut strokes) = (contract, strokes);
    let home = if c.map_name.is_empty() { c.depot.clone() } else { format!("{}  ·  {}", c.map_name, c.depot) };
    let y = dealer::parties(l, inner, ("Lender", &k.lender, "Business bank"), ("Borrower", &k.borrower, &home));
    let gap = 22.0;
    let cw = (inner.w - gap) / 2.0;
    // the terms
    let left = Rect::new(inner.x, y, cw, 0.0);
    let mut ly = y;
    kit::caps(&mut l.ui, Rect::new(left.x, ly, left.w, 16.0), "The loan");
    ly += 24.0;
    let early = if k.early_fee <= 0.0 { omsi_ui::tr("at any time, free of charge").into_owned() } else { omsi_ui::tr("at any time, a fee of %{rate} of what is paid back").replace("%{rate}", &percent(k.early_fee)) };
    let purpose = if k.purpose.trim().is_empty() { omsi_ui::tr("The company's needs").into_owned() } else { k.purpose.clone() };
    let list = vec![
        (omsi_ui::tr("Amount").into_owned(), eur(k.amount), Some(TEXT)),
        (omsi_ui::tr("Purpose").into_owned(), purpose, None),
        (omsi_ui::tr("Interest").into_owned(), omsi_ui::tr("%{rate} a year").replace("%{rate}", &percent(k.rate)), None),
        (omsi_ui::tr("Term").into_owned(), omsi_ui::tr("%{n} months").replace("%{n}", &k.months.to_string()), None),
        (omsi_ui::tr("Monthly payment").into_owned(), eur(k.monthly), None),
        (omsi_ui::tr("Paid back in all").into_owned(), eur(k.total), None),
        (omsi_ui::tr("First payment").into_owned(), day_label(&k.first_rate), None),
    ];
    rows(l, left, &mut ly, &list);
    ly += 10.0;
    let security = omsi_ui::tr("Security: the company's buses, worth %{amount}.").replace("%{amount}", &eur(k.security));
    ly += l.ui.paragraph(&security, Vec2::new(left.x, ly), left.w, kit::NOTE + 0.5, Weight::Regular, TEXT_SOFT);
    let early = omsi_ui::tr("Paying back early: %{terms}.").replace("%{terms}", &early);
    l.ui.paragraph(&early, Vec2::new(left.x, ly + 2.0), left.w, kit::NOTE + 0.5, Weight::Regular, TEXT_SOFT);
    // the repayment schedule
    let right = Rect::new(inner.x + cw + gap, y, cw, 0.0);
    kit::caps(&mut l.ui, Rect::new(right.x, y, right.w, 16.0), "Repayment");
    let plan = fi::schedule(k.amount, k.rate, k.months);
    let cols = [("Month", 0.12), ("Payment", 0.22), ("Interest", 0.22), ("Repaid", 0.22), ("Owed", 0.22)];
    let mut ry = y + 24.0;
    let mut x = right.x;
    for (h, w) in cols {
        l.ui.text_in(h, Rect::new(x, ry, right.w * w - 6.0, 20.0), 13.0, Weight::Bold, TEXT_DIM, if h == "Month" { Align::Left } else { Align::Right });
        x += right.w * w;
    }
    ry += 26.0;
    let shown: Vec<Option<&fi::Instalment>> = if plan.len() <= 9 { plan.iter().map(Some).collect() } else { plan.iter().take(6).map(Some).chain([None]).chain(plan.iter().rev().take(2).rev().map(Some)).collect() };
    for i in shown {
        let rr = Rect::new(right.x, ry, right.w, 24.0);
        match i {
            Some(i) => {
                let cells = [i.month.to_string(), eur(i.payment), eur(i.interest), eur(i.principal), eur(i.remaining)];
                let mut x = rr.x;
                for (n, (_, w)) in cols.iter().enumerate() {
                    l.ui.text_in(&cells[n], Rect::new(x, ry, right.w * w - 6.0, 24.0), 13.5, Weight::Regular, if n == 0 { TEXT_DIM } else { TEXT_SOFT }, if n == 0 { Align::Left } else { Align::Right });
                    x += right.w * w;
                }
            }
            None => {
                l.ui.text_in("…", rr, 14.0, Weight::Bold, TEXT_DIM, Align::Center);
            }
        }
        ry += 24.0;
    }
    l.ui.p().rect(Rect::new(right.x, ry + 2.0, right.w, 1.0), HAIRLINE);
    let interest: i64 = plan.iter().map(|i| i.interest).sum();
    l.ui.text_in(&omsi_ui::tr("Interest in all: %{amount}").replace("%{amount}", &eur(interest)), Rect::new(right.x, ry + 8.0, right.w, 22.0), kit::NOTE + 0.5, Weight::Medium, TEXT_SOFT, Align::Right);
    // the signature
    let day = if readonly { k.signed_at.clone() } else { c.date.clone() };
    let kept = dealer::signature(l, inner, &mut strokes, &mut k.signed_by, &k.strokes.clone(), readonly, &day);
    let mut foot = Foot::new(&f);
    if readonly {
        if foot.right(l, "company-loan-contract-close", "Close", None, ButtonKind::Primary) || esc || f.close {
            return None;
        }
        return Some(Sheet::LoanContract { contract: k, strokes, readonly, collateral, then });
    }
    k.strokes = kept;
    let signed = k.is_signed();
    let sign = foot.right(l, "company-loan-sign", "Sign the loan contract", Some("check_circle"), ButtonKind::Primary);
    if foot.right(l, "company-loan-contract-back", "Back", Some("chevron_left"), ButtonKind::Normal) || esc || (f.close && signing.is_none()) {
        let fixed = !matches!(then, Then::Nothing);
        let term = fi::terms_offered(c).iter().position(|m| *m == k.months).unwrap_or(usize::MAX);
        return Some(Sheet::Loan { amount: (k.amount / 100) as f32, term, purpose: k.purpose.clone(), collateral, fixed, then });
    }
    if !signed {
        l.ui.text_in("Sign the contract first.", foot.rest(), kit::BODY, Weight::Medium, TEXT_SOFT, Align::Left);
    }
    // the pen goes over the paper; the contract is booked when it is done
    let mut book = false;
    if let (Some(s), Some(i)) = (signing.as_ref(), held) {
        l.ui.input = i;
        if signing::draw(l, s) == State::Done {
            l.company.fleet.dealer.signing = None;
            book = true;
        }
    } else if sign {
        if signed {
            l.company.fleet.dealer.signing = Some(Signing::new(l.ui.time, &k.strokes, &k.signed_by, &omsi_ui::tr("Loan contract")));
        } else {
            kit::refuse(l, "Sign the contract first.");
        }
    }
    if book {
        let listings = l.company.fleet.dealer.listings.clone().unwrap_or_default();
        let now = dl::now_of(c);
        match then {
            Then::Nothing => {
                if let Some(id) = act(l, |c| fi::sign_loan(c, &k, collateral)) {
                    l.state.set_status(omsi_ui::tr("Loan contract %{no} is signed: the bank paid %{amount} into the cash.").replace("%{no}", &id.to_string()).replace("%{amount}", &eur(k.amount)), false);
                    return None;
                }
            }
            Then::Purchase(ref p) => {
                if let Some((_, done)) = act(l, |c| dl::sign_financed(c, p, &k, &listings, &now)) {
                    dealer::signed_status(l, done);
                    return None;
                }
            }
            Then::Quick { ref listing, count, ref livery } => {
                if let Some((_, ids)) = act(l, |c| fi::with_loan(c, &k, collateral, |c| dl::quick_buy(c, listing, count, Payment::Cash, livery))) {
                    dealer::joined(l, &ids);
                    return None;
                }
            }
            Then::QuickOffer { ref offer, count, ref livery } => {
                if let Some((_, ids)) = act(l, |c| fi::with_loan(c, &k, collateral, |c| dl::quick_buy_offer(c, offer, count, Payment::Cash, livery, &listings))) {
                    dealer::joined(l, &ids);
                    return None;
                }
            }
        }
    }
    Some(Sheet::LoanContract { contract: k, strokes, readonly, collateral, then })
}

/// Paying a loan back early: what it costs, and what is left.
pub(super) fn repay_sheet(l: &mut Launcher, c: &Company, id: u32, amount: i64, esc: bool) -> Option<Sheet> {
    let loan = c.loans.iter().find(|x| x.id == id).cloned()?;
    let f = kit::frame(l, 620.0, 440.0, "payments", &omsi_ui::tr("Pay back early"));
    let inner = f.body;
    let amount = amount.min(loan.remaining);
    let fee = fi::early_fee(c, amount);
    let what = if loan.purpose.is_empty() { omsi_ui::tr("Loan").into_owned() } else { loan.purpose.clone() };
    let mut y = inner.y;
    let fee_label = omsi_ui::tr("Early repayment fee (%{rate})").replace("%{rate}", &percent(fi::early_fee_rate(c.difficulty)));
    let list = vec![
        (omsi_ui::tr("Loan").into_owned(), what, None),
        (omsi_ui::tr("Paid back now").into_owned(), eur(amount), None),
        (fee_label, eur(fee), if fee > 0 { Some(WARN) } else { None }),
        (omsi_ui::tr("From the cash").into_owned(), eur(amount + fee), Some(TEXT)),
        (omsi_ui::tr("Still owed afterwards").into_owned(), eur(loan.remaining - amount), None),
    ];
    rows(l, inner, &mut y, &list);
    y += 10.0;
    let note = if amount >= loan.remaining { "The loan is paid off: no more rates." } else { "The monthly payment stays the same: the loan ends sooner." };
    l.ui.paragraph(note, Vec2::new(inner.x, y), inner.w, kit::BODY, Weight::Regular, TEXT_SOFT);
    let ok = c.cash >= amount + fee;
    let mut foot = Foot::new(&f);
    let label = omsi_ui::tr("Pay back %{amount}").replace("%{amount}", &eur(amount + fee));
    let pay = foot.right(l, "company-repay-do", &label, Some("check_circle"), ButtonKind::Primary);
    if foot.right(l, "company-repay-cancel", "Cancel", None, ButtonKind::Normal) || esc || f.close {
        return None;
    }
    if pay {
        if !ok {
            kit::show(l, kit::no_cash(c, amount + fee));
        } else if act(l, |c| fi::repay(c, id, amount)).is_some() {
            l.state.set_status(omsi_ui::tr("Paid back: %{amount}.").replace("%{amount}", &eur(amount + fee)), false);
            return None;
        }
    }
    Some(Sheet::Repay { id, amount })
}

/// The credit room on the finances page: how much more the bank lends, in its colour, with
/// what it is made of and the share used. Returns its height.
pub(super) fn credit_figure(l: &mut Launcher, r: Rect, c: &Company) -> f32 {
    let cr = fi::credit(c, 0);
    let room = fi::room(&cr);
    let colour = room_colour(room);
    let narrow = r.w < 520.0;
    let lw = if narrow { r.w } else { r.w * 0.46 };
    kit::caps(&mut l.ui, Rect::new(r.x, r.y, lw, 16.0), "Credit room");
    let value = if room == Room::None { omsi_ui::tr("%{amount} left").replace("%{amount}", &eur(cr.left)) } else { eur(cr.left) };
    l.ui.text_in(&value, Rect::new(r.x, r.y + 18.0, lw, 40.0), 32.0, Weight::Bold, colour, Align::Left);
    let say = match room {
        Room::Plenty => omsi_ui::tr("the bank lends this much more"),
        Room::Little => omsi_ui::tr("little left: the bank is careful"),
        Room::None => omsi_ui::tr("used up: the bank lends nothing more"),
    };
    l.ui.text_in(&say, Rect::new(r.x, r.y + 60.0, lw, 22.0), kit::NOTE + 0.5, Weight::Medium, colour, Align::Left);
    let used = if cr.limit > 0 { (cr.debt as f64 / cr.limit as f64).clamp(0.0, 1.0) } else { 1.0 };
    let of = omsi_ui::tr("%{debt} of %{limit} used").replace("%{debt}", &eur(cr.debt)).replace("%{limit}", &eur(cr.limit));
    let br = Rect::new(r.x, r.y + 88.0, lw - 10.0, kit::BAR_H);
    kit::bar(&mut l.ui, "", br, 1.0 - used, colour, &of, &format!("{:.0} % {}", (1.0 - used) * 100.0, omsi_ui::tr("free")), &omsi_ui::tr("What the bank still lends, of all it would lend the company: its base, a share of the fleet's value and six months of the result, less what is owed."));
    if narrow {
        return 88.0 + kit::BAR_H + 6.0;
    }
    // what it is made of
    let rx = r.x + lw + 16.0;
    let rw = r.w - lw - 16.0;
    let mut y = r.y;
    let lines = [
        (omsi_ui::tr("Base for a %{difficulty} company").replace("%{difficulty}", &omsi_ui::tr(c.difficulty.label()).to_lowercase()), eur(cr.base)),
        (omsi_ui::tr("%{share} of the fleet's %{value}").replace("%{share}", &percent(cr.share)).replace("%{value}", &eur(cr.fleet_value)), eur((cr.share * cr.fleet_value as f64).round() as i64)),
        (omsi_ui::tr("6 months of the result (%{amount} a month)").replace("%{amount}", &eur(cr.income)), eur(fi::INCOME_MONTHS * cr.income)),
        (omsi_ui::tr("Owed already").into_owned(), format!("- {}", eur(cr.debt))),
    ];
    for (a, b) in lines {
        l.ui.text_in(&a, Rect::new(rx, y, rw * 0.68, 24.0), 13.0, Weight::Regular, TEXT_SOFT, Align::Left);
        l.ui.text_in(&b, Rect::new(rx + rw * 0.6, y, rw * 0.4, 24.0), 13.0, Weight::Medium, TEXT, Align::Right);
        y += 24.0;
    }
    let rate = if cr.discount > 0.0 { omsi_ui::tr("Interest %{rate} a year (%{discount} less for the company's level)").replace("%{rate}", &percent(cr.rate)).replace("%{discount}", &percent(cr.discount)) } else { omsi_ui::tr("Interest %{rate} a year").replace("%{rate}", &percent(cr.rate)) };
    l.ui.text_in(&rate, Rect::new(rx, y + 4.0, rw, 22.0), 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
    (y + 26.0 - r.y).max(88.0 + kit::BAR_H) + 6.0
}
