//! Signing on in the navigator: Omsi-Hub's phone - the one in its overlay - in openOMSI's own
//! interface, so that the driver needs no phone for it.
//!
//! Omsi-Hub's driver signed on with a personnel number and a code, signed the duty order, and
//! only then had the duty and its codes; a break was timed on the phone, and the same phone
//! could be a real one or a tablet. The game has all of that ([`crate::companion`]); this is
//! the page for it in the navigator (a button in the city map's header brings it, and until
//! the duty is signed the navigator is this page by itself - Omsi-Hub's duty panel said "sign
//! on first" until then):
//!
//! * not signed on: the keypad - the number, then the code, checked as soon as there are
//!   enough digits (no key to confirm: one hand is on the wheel) - and below it the driver's
//!   pass with the number and the code (Omsi-Hub's `Dienstpas`: there is nothing to protect,
//!   and typing them over is work enough);
//! * signed on without a duty: the duty menu, the game's "Line and tour..." - a line, then
//!   the tour, taken on at once - or driving without a duty;
//! * a duty waiting: its order (line, tour, times, trips), to accept - the IBIS gets the
//!   duty's codes then;
//! * at work: the duty (or the free drive), the break timed in the game's time against the
//!   layover at the terminus, another duty, signing off;
//! * always, at the bottom: phones and tablets - whether the companion is on (and how to turn
//!   it on), the QR code a device scans to pair, the address and pairing code to type on one
//!   instead, the devices paired.
//!
//! While a duty waits to be signed for, the page is the navigator's whole interface rather
//! than a part of it (Omsi-Hub's duty panel was "sign on first" and nothing else until then):
//! the small navigator is the page from edge to edge ([`Room::Panel`]: the pass as one line,
//! the devices as one block where there is room) and the city map's view is the page in two
//! cards, the devices' beside it ([`Room::Full`]); signed, the duty board and the duty sheet
//! come. At work the page is a column beside the city map again ([`Room::Column`]).
//!
//! The state is the game's: signing on here or on a phone shows on both. Everything is
//! worked out by pure functions from the companion's state into items, their places and what
//! a press on each does, and only then drawn - the page is tested without a window.

use std::sync::Arc;

use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Fonts, Painter, Rect, Weight};

use crate::companion::{Attempt, CompanionState, Request, Stage};
use crate::nav_duty::{accent, accent_ink, hhmm, tr_with, DutyState, Pen, EDGE, FIELD, HAIRLINE, LATE_INK, ON_TIME, ON_TIME_INK, SHEET, TEXT, TEXT_DIM, TEXT_FAINT, TEXT_SOFT};

fn tr(key: &str) -> String {
    omsi_ui::tr(key).into_owned()
}

/// The pass as one line: "Personnel no. 482913 · code 5821".
fn pass_line(st: &CompanionState) -> String {
    tr_with("Personnel no. %{number} · code %{code}", &[("number", st.personnel_number.clone()), ("code", st.personnel_code.clone())])
}

/// The duty waits to be signed for: the game knows the driver, who has not signed on yet or
/// not signed the duty order. Until then the navigator is the sign-on page and nothing else.
pub(crate) fn waiting(st: &CompanionState) -> bool {
    !st.personnel_number.is_empty() && matches!(st.stage, Stage::SignOn | Stage::DutyOrder)
}

/// Where the page is shown, and so how much it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Room {
    /// A column beside the city map (at work: the break, another duty, signing off).
    #[default]
    Column,
    /// The whole small navigator, while the duty waits: the pass as one line under the
    /// keypad, the devices as one block beside the QR code (left out when there is no room).
    Panel,
    /// The city map's whole view, while the duty waits: the page in a card, the pass large,
    /// and the devices in a card of their own beside it.
    Full,
}

/// The pairing QR code ([`crate::companion::pair_qr`]): its side in modules and the dark ones,
/// row by row. The quiet zone round it is the drawing's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Qr {
    pub side: usize,
    pub dark: Vec<bool>,
}

impl Qr {
    /// A code from `pair_qr`'s answer; None for one that is no square of modules (a QR code is
    /// 21 modules at the least).
    pub(crate) fn new(side: usize, dark: Vec<bool>) -> Option<Qr> {
        (side >= 21 && dark.len() == side * side).then_some(Qr { side, dark })
    }
}

// --- what a press does -----------------------------------------------------------------

/// What a press on the page does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The keypad: a digit, all typed away, the last one away.
    Digit(u8),
    Clear,
    Erase,
    /// Sign the duty order.
    Accept,
    /// Drive without a duty.
    Free,
    /// Start or end the break.
    Break(bool),
    SignOff,
    /// The duty menu opened over the order or the duty (for another one), and closed again.
    OpenMenu,
    CloseMenu,
    /// A line of the duty menu opened, back to its lines, a tour of the line taken on.
    Line(usize),
    Lines,
    Tour(usize),
    /// Another pairing code; every device unpaired.
    NewCode,
    Forget,
    /// The address and the pairing code uncovered for a while (true), or covered again.
    Reveal(bool),
}

/// What a press asks of the game (the navigator does it with the companion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    SignOn { number: String, code: Option<String> },
    SignOff,
    Free,
    Ask(Request),
    NewCode,
    Forget,
    Reveal(bool),
}

/// The page's own state: what is typed on the keypad, where in the duty menu it is, how far
/// it is scrolled. (Whether the driver is signed on is the game's.)
#[derive(Debug, Clone, Default)]
pub(crate) struct Phone {
    /// The number was right: the code is typed now.
    pub(crate) code_step: bool,
    /// The number that was right (it goes along with the code).
    number: String,
    pub(crate) typed: String,
    /// What was sent, waiting for its answer.
    sent: String,
    /// The last try was wrong: the boxes are red.
    pub(crate) wrong: bool,
    /// The duty menu opened for another duty.
    pub(crate) menu: bool,
    /// The menu's line opened.
    pub(crate) line: Option<usize>,
    /// The menu's lines were asked for.
    asked: bool,
    /// The stage the page was last shown at.
    seen: Option<Stage>,
    /// How far the page is scrolled (pixels).
    pub(crate) scroll: f32,
}

impl Phone {
    /// The stage now: another one (signed on or off here or on a phone, a duty taken on or
    /// signed) starts the page afresh.
    pub(crate) fn follow(&mut self, st: &CompanionState) {
        if self.seen != Some(st.stage) {
            if self.seen.is_some() {
                *self = Phone::default();
            }
            self.seen = Some(st.stage);
        }
    }

    /// The duty menu shows: signed on without a duty, or opened for another one.
    fn menu_open(&self, st: &CompanionState) -> bool {
        st.signed_on && (st.stage == Stage::DutyMenu || self.menu)
    }

    /// What the page needs of the game before it can show: the duty menu's lines, once.
    pub(crate) fn wants(&mut self, st: &CompanionState) -> Option<Request> {
        if self.menu_open(st) && self.line.is_none() && !self.asked {
            self.asked = true;
            return Some(Request::Lines);
        }
        None
    }

    /// How many digits the keypad takes now: the number's, then the code's (six and four
    /// while the game has not said).
    fn wanted(&self, st: &CompanionState) -> usize {
        let n = if self.code_step { st.personnel_code.len() } else { st.personnel_number.len() };
        if n > 0 {
            n
        } else if self.code_step {
            4
        } else {
            6
        }
    }

    /// A press: what it changes on the page, and what it asks of the game.
    pub(crate) fn press(&mut self, a: Action, st: &CompanionState) -> Option<Call> {
        match a {
            Action::Digit(d) => {
                let n = self.wanted(st);
                if st.signed_on || d > 9 || self.typed.len() >= n {
                    return None;
                }
                self.wrong = false;
                self.typed.push(char::from(b'0' + d));
                if self.typed.len() < n {
                    return None;
                }
                // enough digits: checked at once, as on Omsi-Hub's keypad
                self.sent = std::mem::take(&mut self.typed);
                Some(if self.code_step { Call::SignOn { number: self.number.clone(), code: Some(self.sent.clone()) } } else { Call::SignOn { number: self.sent.clone(), code: None } })
            }
            Action::Clear => {
                self.typed.clear();
                None
            }
            Action::Erase => {
                self.typed.pop();
                None
            }
            Action::Accept => Some(Call::Ask(Request::Accept)),
            Action::Free => Some(Call::Free),
            Action::Break(on) => Some(Call::Ask(Request::Break(on))),
            Action::SignOff => {
                *self = Phone { seen: self.seen, ..Phone::default() };
                Some(Call::SignOff)
            }
            Action::OpenMenu => {
                self.menu = true;
                self.line = None;
                self.asked = false;
                self.scroll = 0.0;
                None
            }
            Action::CloseMenu => {
                self.menu = false;
                self.line = None;
                self.scroll = 0.0;
                None
            }
            Action::Line(k) => {
                self.line = Some(k);
                self.scroll = 0.0;
                Some(Call::Ask(Request::Tours(k)))
            }
            Action::Lines => {
                self.line = None;
                None
            }
            Action::Tour(k) => {
                // (back to the lines meanwhile: when it worked the duty order follows, when
                // not the lines say so)
                let line = self.line.take()?;
                self.menu = false;
                Some(Call::Ask(Request::Pick(line, k)))
            }
            Action::NewCode => Some(Call::NewCode),
            Action::Forget => Some(Call::Forget),
            Action::Reveal(on) => Some(Call::Reveal(on)),
        }
    }

    /// The answer to a try at signing on: the code next, signed on (the stage shows it from
    /// the next frame on), or wrong - the boxes empty and red on the same step.
    pub(crate) fn attempted(&mut self, a: Attempt) {
        let sent = std::mem::take(&mut self.sent);
        match a {
            Attempt::Number => {
                self.number = sent;
                self.code_step = true;
                self.wrong = false;
            }
            Attempt::SignedOn => {
                self.code_step = false;
                self.number.clear();
                self.wrong = false;
            }
            Attempt::Wrong => self.wrong = true,
        }
    }
}

// --- the page as items ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    Dim,
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// The one thing to do (blue).
    Main,
    Plain,
    /// Outlined: back, sign off, the devices.
    Quiet,
}

/// What a row of the duty menu begins with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lead {
    /// A line: its plate.
    Plate(String),
    /// A tour: when it leaves.
    Time(String),
}

/// A part of the page, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    /// Text, wrapped to the page.
    Text(String, Tone),
    /// A small heading in capitals.
    Caption(String),
    /// The driver's pass: number and code, large.
    Pass { number: String, code: String },
    /// The pass as one small line.
    PassLine(String),
    /// A box for each digit: how many, those typed (dots for the code), wrong.
    Boxes { len: usize, typed: String, hide: bool, wrong: bool },
    Keypad,
    /// A line of the duty order.
    Field { label: String, value: String },
    Button { label: String, icon: Option<&'static str>, action: Action, style: Style },
    /// A line or tour of the duty menu.
    Choice { lead: Lead, text: String, action: Action },
    /// The devices' part.
    Section { icon: &'static str, title: String },
    /// The duty at work: its line (None: driving without one), what, and its span.
    Duty { line: Option<String>, title: String, sub: String },
    /// The break: minutes (of the break, or planned), how much of the planned is gone, what
    /// it means, and whether it is within it (None: no break running).
    Break { minutes: i64, part: f32, label: String, within: Option<bool> },
    /// The next trip: when it leaves, its line (None: an empty run), where to, when it ends.
    Trip { departure: f64, line: Option<String>, terminus: String, arrival: f64 },
    /// Where a device opens the page.
    Address(String),
    /// The pairing code, a box a digit.
    Code(String),
    /// The QR code a phone or tablet scans to pair, at most `max` points wide, with a line
    /// under it saying so.
    Qr { qr: Arc<Qr>, max: f32 },
    /// The devices as one block (the small navigator): the QR code when there is one, and
    /// where a device opens the page with the code it pairs with.
    Pairing { qr: Option<Arc<Qr>>, address: String, code: String },
    /// The address, the pairing code and the QR code covered (for streaming): a blank in
    /// their place.
    Hidden,
    /// A device seen lately, and what it does.
    Device { name: String, what: String },
    Gap(f32),
}

/// The page: its head (icon, title, the driver, signed on or not) and its items.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Page {
    pub icon: &'static str,
    pub title: String,
    pub sub: String,
    pub signed_on: bool,
    pub items: Vec<Item>,
    /// Where the devices' part begins in `items` (the full view gives it a card of its own,
    /// the small navigator leaves it out when there is no room for it).
    pub devices: usize,
}

/// The page for the companion's state `st`, the page's own `ui`, the duty as the navigator
/// has it and the game's time of day (s), as a column beside the city map without a QR code
/// (the tests' short way to [`page_in`]).
#[cfg(test)]
pub(crate) fn page(st: &CompanionState, ui: &Phone, duty: Option<&DutyState>, time: f64) -> Page {
    page_in(st, ui, duty, time, Room::Column, None)
}

/// The page as [`page`] has it, shown in `room`, with the pairing QR code when the companion
/// has one.
pub(crate) fn page_in(st: &CompanionState, ui: &Phone, duty: Option<&DutyState>, time: f64, room: Room, qr: Option<&Arc<Qr>>) -> Page {
    let mut items = Vec::new();
    let (icon, title) = if !st.signed_on {
        sign_on_items(st, ui, room, &mut items);
        ("badge", tr("Sign on"))
    } else {
        // (the pass as a small line from now on, for whoever signs on again after a restart:
        // Omsi-Hub's `pasregel`; not when signing on is not asked for)
        if !st.personnel_number.is_empty() && !st.auto_sign_on {
            items.push(Item::PassLine(pass_line(st)));
        }
        if ui.menu_open(st) {
            menu_items(st, ui, &mut items);
            ("departure_board", tr("Choose a duty"))
        } else if st.stage == Stage::DutyOrder {
            order_items(st, &mut items);
            ("description", tr("Duty assignment"))
        } else {
            duty_items(st, duty, time, &mut items);
            if st.auto_sign_on { ("badge", tr("Duty")) } else { ("badge", tr("Signed on")) }
        }
    };
    let devices = items.len();
    match room {
        Room::Panel => pairing_items(st, qr, &mut items),
        Room::Column | Room::Full => device_items(st, qr, room, &mut items),
    }
    Page { icon, title, sub: st.driver.trim().to_string(), signed_on: st.signed_on, items, devices }
}

/// "Sign off" - not when signing on is not asked for (the setting `nav_signon` off): nobody
/// signed on to sign off from.
fn sign_off_item(st: &CompanionState, items: &mut Vec<Item>) {
    if !st.auto_sign_on {
        items.push(button("Sign off", Some("logout"), Action::SignOff, Style::Quiet));
    }
}

fn button(label: &str, icon: Option<&'static str>, action: Action, style: Style) -> Item {
    Item::Button { label: tr(label), icon, action, style }
}

fn sign_on_items(st: &CompanionState, ui: &Phone, room: Room, items: &mut Vec<Item>) {
    items.push(Item::Text(tr(if ui.code_step { "Your code" } else { "Your personnel number" }), Tone::Plain));
    items.push(Item::Boxes { len: ui.wanted(st), typed: ui.typed.clone(), hide: ui.code_step, wrong: ui.wrong });
    if ui.wrong {
        items.push(Item::Text(tr("Not known here. Your number and code are on your pass below."), Tone::Bad));
    }
    items.push(Item::Keypad);
    if !st.personnel_number.is_empty() {
        // (the small navigator has room for the pass as one line)
        if room == Room::Panel {
            items.push(Item::Gap(4.0));
            items.push(Item::PassLine(pass_line(st)));
        } else {
            items.push(Item::Caption(tr("Your pass")));
            items.push(Item::Pass { number: st.personnel_number.clone(), code: st.personnel_code.clone() });
        }
    }
}

fn menu_items(st: &CompanionState, ui: &Phone, items: &mut Vec<Item>) {
    let loading = || Item::Text(tr("Loading…"), Tone::Dim);
    match ui.line {
        None => match st.menu.lines.as_ref() {
            None => items.push(loading()),
            Some(lines) if lines.is_empty() => items.push(Item::Text(tr("No timetable on this map"), Tone::Dim)),
            Some(lines) => items.extend(lines.iter().enumerate().map(|(k, (name, label))| Item::Choice { lead: Lead::Plate(name.trim().to_string()), text: label.clone(), action: Action::Line(k) })),
        },
        Some(k) => {
            // (the line opened, as a way back to the lines)
            let label = st.menu.lines.as_ref().and_then(|l| l.get(k)).map(|l| l.1.clone()).unwrap_or_else(|| tr("Back"));
            items.push(Item::Button { label, icon: Some("chevron_left"), action: Action::Lines, style: Style::Quiet });
            items.push(Item::Text(tr("Tap the tour you drive: you take it on at once, and its codes go to the IBIS."), Tone::Dim));
            match st.menu.tours.as_ref() {
                Some((at, tours)) if *at == k && tours.is_empty() => items.push(Item::Text(tr("No tours on this line."), Tone::Dim)),
                Some((at, tours)) if *at == k => items.extend(tours.iter().enumerate().map(|(n, (what, when))| Item::Choice { lead: Lead::Time(when.clone()), text: what.clone(), action: Action::Tour(n) })),
                _ => items.push(loading()),
            }
        }
    }
    if st.menu.failed {
        items.push(Item::Text(tr("That did not work. Try again."), Tone::Bad));
    }
    items.push(Item::Gap(6.0));
    if st.stage == Stage::DutyMenu {
        items.push(button("Drive without a duty", Some("directions_bus"), Action::Free, Style::Plain));
    }
    if ui.menu {
        items.push(button("Back", None, Action::CloseMenu, Style::Plain));
    }
    sign_off_item(st, items);
}

fn order_items(st: &CompanionState, items: &mut Vec<Item>) {
    let d = st.duty.clone().unwrap_or_default();
    let dash = |v: &str| if v.trim().is_empty() { "—".to_string() } else { v.trim().to_string() };
    let field = |label: &str, value: String| Item::Field { label: tr(label), value };
    items.push(field("Line", dash(if d.lines.trim().is_empty() { d.line.as_str() } else { d.lines.as_str() })));
    items.push(field("Tour", dash(d.tour.as_str())));
    items.push(field("Departure", hhmm(d.start)));
    items.push(field("Back at", hhmm(d.end)));
    items.push(field("Trips", d.trips.to_string()));
    items.push(Item::Gap(8.0));
    items.push(Item::Text(tr("The duty's codes go to the IBIS as soon as you accept it."), Tone::Dim));
    items.push(button("Accept duty", Some("check"), Action::Accept, Style::Main));
    items.push(button("Choose another duty", None, Action::OpenMenu, Style::Plain));
    sign_off_item(st, items);
}

fn duty_items(st: &CompanionState, duty: Option<&DutyState>, time: f64, items: &mut Vec<Item>) {
    match st.duty.as_ref() {
        Some(d) => {
            let line = Some(if d.lines.trim().is_empty() { d.line.trim() } else { d.lines.trim() }).filter(|l| !l.is_empty()).map(str::to_string);
            let trips = tr_with(if d.trips == 1 { "%{n} trip" } else { "%{n} trips" }, &[("n", d.trips.to_string())]);
            items.push(Item::Duty { line, title: tr_with("Tour %{tour}", &[("tour", d.tour.trim().to_string())]), sub: format!("{} – {}  ·  {trips}", hhmm(d.start), hhmm(d.end)) });
        }
        None => items.push(Item::Duty { line: None, title: tr("Driving without a duty"), sub: String::new() }),
    }
    break_items(st, duty, time, items);
    items.push(Item::Gap(4.0));
    items.push(button(if st.duty.is_some() { "Choose another duty" } else { "Choose a duty" }, Some("departure_board"), Action::OpenMenu, Style::Plain));
    sign_off_item(st, items);
}

/// Omsi-Hub's break: timed in the game's time (OMSI may run faster or slower than the wall
/// clock) against the layover at the terminus before the next trip.
fn break_items(st: &CompanionState, duty: Option<&DutyState>, time: f64, items: &mut Vec<Item>) {
    let next = duty.and_then(|d| Some((d.trips.get(d.trip)?, d.trips.get(d.trip + 1)?)));
    let planned = next.map_or(0, |(a, b)| ((b.departure - a.end) / 60.0).round().max(0.0) as i64);
    let spent = st.break_since.map(|since| ((time - since).rem_euclid(86_400.0) / 60.0).floor() as i64);
    let (minutes, part, label, within) = match spent {
        None => (planned, 0.0, if planned > 0 { tr("Scheduled break at the terminus") } else { String::new() }, None),
        Some(m) if next.is_some() => {
            let left = planned - m;
            let label = if left >= 0 { tr_with("%{minutes} min left", &[("minutes", left.to_string())]) } else { tr_with("%{minutes} min over", &[("minutes", (-left).to_string())]) };
            (m, (m as f32 / planned.max(1) as f32).min(1.0), label, Some(left >= 0))
        }
        Some(m) => (m, 1.0, tr("On break"), Some(true)),
    };
    items.push(Item::Caption(tr("Break")));
    items.push(Item::Break { minutes, part, label, within });
    match spent {
        Some(_) => items.push(button("End break", Some("play_arrow"), Action::Break(false), Style::Plain)),
        None => items.push(button("Start break", Some("pause"), Action::Break(true), Style::Main)),
    }
    if let Some((_, b)) = next {
        items.push(Item::Caption(tr("Next trip")));
        items.push(Item::Trip { departure: b.departure, line: Some(b.line.trim().to_string()).filter(|l| !l.is_empty()), terminus: b.terminus.trim().to_string(), arrival: b.end });
    }
}

/// Where a device opens the page, as the driver types it ("192.168.1.20:47811"; the
/// Cloudflare tunnel's with its `https://`).
fn address(a: &str) -> String {
    crate::companion::shown_address(a)
}

/// The Cloudflare tunnel, when it is asked for: how far it is.
fn tunnel_items(st: &CompanionState, items: &mut Vec<Item>) {
    use crate::companion::TunnelState as T;
    match &st.tunnel {
        None => {}
        Some(T::Ready(_)) => items.push(Item::Text(tr("Through Cloudflare: reachable outside your network too."), Tone::Dim)),
        Some(T::Starting) => items.push(Item::Text(tr("Connecting through Cloudflare…"), Tone::Dim)),
        Some(T::Missing) => items.push(Item::Text(tr("Cloudflare: cloudflared is not installed. Download it in the launcher under Settings › General."), Tone::Bad)),
        Some(T::Failed) => items.push(Item::Text(tr("The Cloudflare tunnel could not start. Trying again…"), Tone::Bad)),
    }
}

/// The address and the codes covered for streaming, and the button that uncovers them; or,
/// uncovered for a while, the button that covers them again.
fn hidden_items(st: &CompanionState, items: &mut Vec<Item>) {
    if st.hidden {
        items.push(Item::Hidden);
        items.push(button("Show for 30 seconds", Some("visibility"), Action::Reveal(true), Style::Quiet));
    } else if st.hide {
        items.push(button("Hide again", Some("lock"), Action::Reveal(false), Style::Quiet));
    }
}

/// The companion is listening with an address and a code: a device can pair.
fn pairable(st: &CompanionState) -> bool {
    st.enabled && st.error.is_none() && st.listening.is_some() && !st.pairing_code.is_empty() && !st.addresses.is_empty()
}

/// Phones and tablets: on or how to turn it on, the QR code to scan, where a device opens the
/// page and the code it pairs with, the devices.
fn device_items(st: &CompanionState, qr: Option<&Arc<Qr>>, room: Room, items: &mut Vec<Item>) {
    items.push(Item::Section { icon: "wifi_tethering", title: tr("Phone & tablet") });
    if !st.enabled {
        items.push(Item::Text(tr("Phone & tablet is off. Turn it on in the settings to have the duty and the bus's screens on a phone or tablet."), Tone::Dim));
        return;
    }
    if let Some(e) = st.error.as_ref() {
        items.push(Item::Text(tr_with("The phone companion could not start: %{error}", &[("error", e.clone())]), Tone::Bad));
        return;
    }
    if st.listening.is_none() || st.pairing_code.is_empty() {
        items.push(Item::Text(tr("Starting…"), Tone::Dim));
        return;
    }
    tunnel_items(st, items);
    if st.addresses.is_empty() {
        items.push(Item::Text(tr("This computer has no network address. Is it on Wi-Fi or a network cable?"), Tone::Bad));
    } else if st.hidden {
        hidden_items(st, items);
    } else {
        // (the code to scan first: typing the address over is the way without a camera)
        if let Some(q) = qr {
            items.push(Item::Qr { qr: q.clone(), max: if room == Room::Full { 232.0 } else { 188.0 } });
        }
        let public = matches!(st.tunnel, Some(crate::companion::TunnelState::Ready(_)));
        items.push(Item::Text(tr(if public { "On a phone or tablet, open" } else { "On a phone or tablet on the same network, open" }), Tone::Dim));
        items.extend(st.addresses.iter().take(3).map(|a| Item::Address(address(a))));
        items.push(Item::Text(tr("and enter the pairing code"), Tone::Dim));
        items.push(Item::Code(st.pairing_code.clone()));
        hidden_items(st, items);
    }
    for d in &st.devices {
        let what = d.screen.as_ref().and_then(|id| st.screens.iter().find(|s| &s.0 == id)).map(|s| tr_with("Shows %{screen}", &[("screen", s.1.clone())])).unwrap_or_else(|| tr("Connected"));
        items.push(Item::Device { name: d.name.clone(), what });
    }
    items.push(Item::Text(if st.paired == 0 { tr("No device paired yet.") } else { tr_with("Paired devices: %{n}", &[("n", st.paired.to_string())]) }, Tone::Dim));
    items.push(button("New pairing code", Some("refresh"), Action::NewCode, Style::Quiet));
    if st.paired > 0 {
        items.push(button("Forget devices", Some("delete"), Action::Forget, Style::Quiet));
    }
}

/// The devices on the small navigator: one block to pair with, while a device can (a
/// companion that is off or starting says nothing there - the city map's view says it).
fn pairing_items(st: &CompanionState, qr: Option<&Arc<Qr>>, items: &mut Vec<Item>) {
    if pairable(st) {
        if st.hidden {
            hidden_items(st, items);
        } else {
            items.push(Item::Pairing { qr: qr.cloned(), address: address(&st.addresses[0]), code: st.pairing_code.clone() });
        }
    }
}

// --- where the items go ----------------------------------------------------------------------

/// The page's head above the part that scrolls (at scale 1).
pub(crate) const HEAD: f32 = 66.0;
/// Space between the page's edge and its items (at scale 1).
pub(crate) const PAD: f32 = 16.0;
const TEXT_PX: f32 = 12.5;
const TEXT_LINE: f32 = 17.0;
const KEY_H: f32 = 48.0;
const KEY_GAP: f32 = 8.0;

/// An item's place on the page (pixels from the page's top left, unscrolled) and, for text,
/// its lines.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    pub rect: Rect,
    pub lines: Vec<String>,
}

fn text_weight(tone: Tone) -> Weight {
    if tone == Tone::Bad { Weight::Bold } else { Weight::Medium }
}

/// `text` broken into lines no wider than `w` pixels at `px` (a word longer than that keeps
/// its line, and is cut with an ellipsis when it is drawn).
pub(crate) fn wrap(fonts: &Fonts, text: &str, px: f32, weight: Weight, w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let longer = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && fonts.width(&longer, px, weight) > w {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = longer;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The items one under the other in a page `w` pixels wide at scale `s`; and how tall it all
/// is.
pub(crate) fn layout(items: &[Item], w: f32, s: f32, fonts: &Fonts) -> (Vec<Placed>, f32) {
    let mut y = 0.0;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let mut lines = Vec::new();
        let h = match item {
            Item::Text(t, tone) => {
                lines = wrap(fonts, t, TEXT_PX * s, text_weight(*tone), w);
                (lines.len() as f32 * TEXT_LINE + 8.0) * s
            }
            // (the code in whole pixels a module: its height is no multiple of the scale's)
            Item::Qr { qr, max } => qr_px(qr.side, (*max * s).min(w)) + QR_CAPTION * s,
            Item::Pairing { qr, .. } => pairing_height(qr.as_deref(), w, s),
            other => {
                s * match other {
                    Item::Caption(_) => 26.0,
                    Item::Pass { .. } => 66.0,
                    Item::PassLine(_) => 32.0,
                    Item::Boxes { .. } => 58.0,
                    Item::Keypad => 4.0 * KEY_H + 3.0 * KEY_GAP + 8.0,
                    Item::Field { .. } => 32.0,
                    Item::Button { .. } | Item::Choice { .. } => 48.0,
                    Item::Section { .. } => 50.0,
                    Item::Duty { .. } => 70.0,
                    Item::Break { .. } => 92.0,
                    Item::Trip { .. } => 46.0,
                    Item::Address(_) => 28.0,
                    Item::Code(_) => 56.0,
                    Item::Hidden => 64.0,
                    Item::Device { .. } => 40.0,
                    Item::Gap(g) => *g,
                    Item::Text(..) | Item::Qr { .. } | Item::Pairing { .. } => 0.0,
                }
            }
        };
        out.push(Placed { rect: Rect::new(0.0, y, w, h), lines });
        y += h;
    }
    (out, y)
}

// --- the QR code ------------------------------------------------------------------------------

/// The light modules round the code that a reader needs to find it (the standard's four).
pub(crate) const QR_QUIET: usize = 4;
/// Above the code and under it, its line ("Scan with your phone or tablet") (at scale 1).
const QR_CAPTION: f32 = 4.0 + 8.0 + 18.0 + 4.0;
/// The darkest ink on the whitest white: a phone's camera reads it in a dim cab.
const QR_INK: Color = Color::rgba(0, 0, 0, 1.0);

/// Pixels a module of a code `side` modules across, the quiet zone round it, at most `max`
/// pixels in all: whole pixels (a module that is not cut into the pixels stays sharp), two at
/// the least.
pub(crate) fn qr_module(side: usize, max: f32) -> f32 {
    (max / (side + 2 * QR_QUIET) as f32).floor().max(2.0)
}

/// How wide the code is drawn in at most `max` pixels, the quiet zone with it.
pub(crate) fn qr_px(side: usize, max: f32) -> f32 {
    qr_module(side, max) * (side + 2 * QR_QUIET) as f32
}

/// The dark modules as runs along each row (one box a run: fewer triangles, and no seam
/// between two modules side by side), the quiet zone's top left at `at` - put on a whole
/// pixel - and `m` pixels a module.
pub(crate) fn qr_runs(q: &Qr, at: Vec2, m: f32) -> Vec<Rect> {
    let o = at.round() + Vec2::splat(QR_QUIET as f32 * m);
    let mut out = Vec::new();
    for (y, row) in q.dark.chunks(q.side.max(1)).enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            out.push(Rect::new(o.x + start as f32 * m, o.y + y as f32 * m, (x - start) as f32 * m, m));
        }
    }
    out
}

/// Draw the code: a white square, the quiet zone, with the dark modules on it; its top left
/// at `at` (put on a whole pixel), `m` pixels a module.
pub(crate) fn draw_qr(p: &mut Painter, q: &Qr, at: Vec2, m: f32) {
    let side = (q.side + 2 * QR_QUIET) as f32 * m;
    let at = at.round();
    // (rounded within the quiet zone: the code's own corners stay square)
    p.rounded(Rect::new(at.x, at.y, side, side), (1.5 * m).min(8.0), Color::WHITE);
    for r in qr_runs(q, at, m) {
        p.rect(r, QR_INK);
    }
}

/// The small navigator's block for the devices: the code (at most 124 points, and two fifths
/// of the width) beside four lines, or two lines without a code.
const PAIRING_QR: f32 = 124.0;

fn pairing_qr_px(q: &Qr, w: f32, s: f32) -> (f32, f32) {
    let m = qr_module(q.side, (PAIRING_QR * s).min(w * 0.42));
    (m, m * (q.side + 2 * QR_QUIET) as f32)
}

fn pairing_height(qr: Option<&Qr>, w: f32, s: f32) -> f32 {
    match qr {
        Some(q) => 14.0 * s + pairing_qr_px(q, w, s).1.max(78.0 * s) + 4.0 * s,
        None => (14.0 + 44.0 + 4.0) * s,
    }
}

/// The keypad's twelve keys in its place `r`: 1-9, then Clear, 0 and the key that takes the
/// last digit away (Omsi-Hub's `cijferblok`).
pub(crate) fn keypad_keys(r: Rect, s: f32) -> Vec<(Rect, Action)> {
    let (gap, h) = (KEY_GAP * s, KEY_H * s);
    let w = (r.w - 2.0 * gap) / 3.0;
    let order = [Action::Digit(1), Action::Digit(2), Action::Digit(3), Action::Digit(4), Action::Digit(5), Action::Digit(6), Action::Digit(7), Action::Digit(8), Action::Digit(9), Action::Clear, Action::Digit(0), Action::Erase];
    order.iter().enumerate().map(|(k, a)| (Rect::new(r.x + (k % 3) as f32 * (w + gap), r.y + 4.0 * s + (k / 3) as f32 * (h + gap), w, h), *a)).collect()
}

fn button_rect(r: Rect, s: f32) -> Rect {
    Rect::new(r.x, r.y + 4.0 * s, r.w, r.h - 8.0 * s)
}

fn choice_rect(r: Rect, s: f32) -> Rect {
    Rect::new(r.x, r.y + 3.0 * s, r.w, r.h - 6.0 * s)
}

/// Where a press does something: the keys, the buttons, the duty menu's rows (in the page's
/// own pixels, as [`layout`] placed them).
pub(crate) fn hits(items: &[Item], placed: &[Placed], s: f32) -> Vec<(Rect, Action)> {
    let mut out = Vec::new();
    for (item, p) in items.iter().zip(placed) {
        match item {
            Item::Keypad => out.extend(keypad_keys(p.rect, s)),
            Item::Button { action, .. } => out.push((button_rect(p.rect, s), *action)),
            Item::Choice { action, .. } => out.push((choice_rect(p.rect, s), *action)),
            _ => {}
        }
    }
    out
}

/// The keypad's action for a key of the keyboard: digits (the number row and the numpad),
/// Backspace and Delete.
pub(crate) fn key_action(code: winit::keyboard::KeyCode) -> Option<Action> {
    use winit::keyboard::KeyCode as K;
    let digit = match code {
        K::Digit0 | K::Numpad0 => 0,
        K::Digit1 | K::Numpad1 => 1,
        K::Digit2 | K::Numpad2 => 2,
        K::Digit3 | K::Numpad3 => 3,
        K::Digit4 | K::Numpad4 => 4,
        K::Digit5 | K::Numpad5 => 5,
        K::Digit6 | K::Numpad6 => 6,
        K::Digit7 | K::Numpad7 => 7,
        K::Digit8 | K::Numpad8 => 8,
        K::Digit9 | K::Numpad9 => 9,
        K::Backspace => return Some(Action::Erase),
        K::Delete | K::NumpadClear => return Some(Action::Clear),
        _ => return None,
    };
    Some(Action::Digit(digit))
}

// --- drawing ---------------------------------------------------------------------------------

/// A card's frame: its shadow, the sheet and its edge (the left column, the full view's cards).
fn draw_frame(pen: &mut Pen, r: Rect, s: f32) {
    pen.p.shadow(r, 12.0 * s, 18.0 * s, Color::rgba(0, 0, 0, 0.45));
    pen.p.rounded(r, 12.0 * s, SHEET.alpha(0.96));
    pen.p.rounded_border(r, 12.0 * s, 1.0, EDGE);
}

/// The chip in the page's head: signed on (green) or not.
fn chip(page: &Page) -> (&'static str, Color, Color) {
    if page.signed_on { ("Signed on", ON_TIME, Color::WHITE) } else { ("Not signed on", FIELD, TEXT_SOFT) }
}

/// A head along the top of `r`: the icon and title, a line under them, the chip on the right
/// (text, fill, ink), and a hairline under it all. Returns the rect under the head.
pub(crate) fn draw_head_in(pen: &mut Pen, icon: &str, title: &str, sub: &str, chip: Option<(&str, Color, Color)>, r: Rect, s: f32) -> Rect {
    let (x0, x1) = (r.x + PAD * s, r.right() - PAD * s);
    pen.p.icon(pen.atlas, icon, Vec2::new(x0 + 10.0 * s, r.y + 27.0 * s), 20.0 * s, TEXT);
    let mut right = x1;
    if let Some((text, fill, ink)) = chip {
        let px = 11.5 * s;
        let text = omsi_ui::tr(text);
        let cw = pen.width(&text, px, Weight::Bold) + 18.0 * s;
        let c = Rect::new(x1 - cw, r.y + 16.5 * s, cw, 21.0 * s);
        pen.p.rounded(c, 10.5 * s, fill);
        pen.text_in(&text, px, Weight::Bold, c, Align::Center, ink);
        right = c.x - 8.0 * s;
    }
    let tx = x0 + 28.0 * s;
    pen.text_in(title, 18.0 * s, Weight::Bold, Rect::new(tx, r.y + 15.0 * s, (right - tx).max(0.0), 24.0 * s), Align::Left, TEXT);
    pen.text_in(sub, 12.0 * s, Weight::Medium, Rect::new(tx, r.y + 39.0 * s, (x1 - tx).max(0.0), 18.0 * s), Align::Left, TEXT_DIM);
    pen.p.rect(Rect::new(r.x, r.y + HEAD * s - 1.0, r.w, 1.0), HAIRLINE);
    Rect::new(r.x, r.y + HEAD * s, r.w, (r.h - HEAD * s - 8.0 * s).max(0.0))
}

/// The page's head in `r` with no frame (the small navigator draws its own): the icon and
/// title, the driver, and whether they are signed on. Returns the rect the items go in.
pub(crate) fn draw_page_head(pen: &mut Pen, page: &Page, r: Rect, s: f32) -> Rect {
    draw_head_in(pen, page.icon, &page.title, &page.sub, Some(chip(page)), r, s)
}

/// Draw the page's frame and head into `r` (the left column of the city map): the icon and
/// title, the driver, and whether they are signed on. Returns the rect the items go in.
pub(crate) fn draw_head(pen: &mut Pen, page: &Page, r: Rect, s: f32) -> Rect {
    draw_frame(pen, r, s);
    draw_page_head(pen, page, r, s)
}

// --- the whole navigator as the page ---------------------------------------------------------

/// Space between the head and the first item, and under the last (at scale 1).
pub(crate) const TOP: f32 = 6.0;
const BOTTOM: f32 = 14.0;

/// Where the small navigator's items begin (at scale `s`): under its head, inside its edge.
pub(crate) fn panel_origin(s: f32) -> Vec2 {
    Vec2::new(PAD * s, (HEAD + TOP) * s)
}

/// The smallest the small navigator's page is made to fit its window (a share of its scale):
/// smaller, it scrolls instead.
const PANEL_SMALLEST: f32 = 0.75;

/// The page as the small navigator shows it ([`Room::Panel`]), `w` pixels wide at scale `s`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fitted {
    pub page: Page,
    /// Where its items go (from [`panel_origin`]).
    pub placed: Vec<Placed>,
    /// The scale it is drawn at.
    pub s: f32,
    /// How tall the navigator is (at most the room it has), and how tall the page would be:
    /// taller, it scrolls under its head.
    pub height: f32,
    pub content: f32,
}

/// The page as the small navigator shows it, `w` pixels wide at scale `s` and at most `room`
/// pixels tall: the devices left out when they do not fit, then the page made smaller (to
/// three quarters at the most: a small window, a large interface), and what still does not
/// fit scrolls (a long duty menu).
pub(crate) fn fit_panel(mut page: Page, w: f32, s: f32, room: f32, fonts: &Fonts) -> Fitted {
    let measure = |page: &Page, s: f32| {
        let (placed, h) = layout(&page.items, w - 2.0 * PAD * s, s, fonts);
        (placed, ((HEAD + TOP + BOTTOM) * s + h).ceil())
    };
    let (placed, h) = measure(&page, s);
    if h <= room {
        return Fitted { page, placed, s, height: h, content: h };
    }
    page.items.truncate(page.devices);
    let (mut placed, mut h) = measure(&page, s);
    let (mut k, smallest) = (s, s * PANEL_SMALLEST);
    // (smaller text wraps into fewer lines: a step or two finds the scale that fits)
    for _ in 0..4 {
        if h <= room || k <= smallest {
            break;
        }
        k = (k * (room / h).clamp(0.5, 0.99)).max(smallest);
        (placed, h) = measure(&page, k);
    }
    Fitted { page, placed, s: k, height: h.min(room.max((HEAD + TOP + BOTTOM) * k)), content: h }
}

/// Where a press on the small navigator's page does what (the panel's pixels), its items
/// scrolled up by `scroll` under the head: those out of sight are none.
pub(crate) fn panel_hits(f: &Fitted, scroll: f32) -> Vec<(Rect, Action)> {
    let o = panel_origin(f.s) - Vec2::new(0.0, scroll);
    let view = Rect::new(0.0, HEAD * f.s, f32::MAX, (f.height - HEAD * f.s).max(0.0));
    hits(&f.page.items, &f.placed, f.s).into_iter().map(|(r, a)| (Rect::new(r.x + o.x, r.y + o.y, r.w, r.h), a)).filter(|(r, _)| view.contains(r.center())).collect()
}

/// Where the small navigator's items show (`panel`'s pixels): under its head, to its foot.
pub(crate) fn panel_view(f: &Fitted, panel: Rect) -> Rect {
    Rect::new(panel.x, panel.y + HEAD * f.s, panel.w, (f.height - HEAD * f.s).max(0.0))
}

/// The most the small navigator's page scrolls.
pub(crate) fn panel_scroll_max(f: &Fitted) -> f32 {
    (f.content - f.height).max(0.0)
}

/// Draw the small navigator's head into `panel` (its background is the navigator's), and a
/// scroll bar beside its items when they scroll (`scroll` of the most there is).
pub(crate) fn draw_panel_head(pen: &mut Pen, f: &Fitted, panel: Rect, scroll: f32) {
    draw_page_head(pen, &f.page, panel, f.s);
    let max = panel_scroll_max(f);
    if max > 0.0 {
        let view = panel_view(f, panel);
        let thumb = (view.h * view.h / (view.h + max)).max(24.0 * f.s);
        let y = view.y + (view.h - thumb) * (scroll / max).clamp(0.0, 1.0);
        pen.p.rounded(Rect::new(view.right() - 5.0 * f.s, y, 3.0 * f.s, thumb), 1.5 * f.s, Color::WHITE.alpha(0.2));
    }
}

/// Draw the small navigator's items into `panel`, scrolled up by `scroll` (drawn in a layer
/// clipped to [`panel_view`]).
pub(crate) fn draw_panel_items(pen: &mut Pen, f: &Fitted, panel: Rect, scroll: f32) {
    let o = Vec2::new(panel.x, panel.y - scroll) + panel_origin(f.s);
    draw_items(pen, &f.page.items, &f.placed, o, panel_view(f, panel), f.s);
}

/// The full view's two cards ([`Room::Full`]): the page's own and the devices' (the page's
/// part from [`Page::devices`], its section the card's head), where their items go, and how
/// tall it all is with the margins - for the scroll.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Full {
    /// The cards (window pixels, unscrolled).
    pub main: Rect,
    pub devices: Option<Rect>,
    pub main_placed: Vec<Placed>,
    pub dev_placed: Vec<Placed>,
    pub height: f32,
}

/// The cards' widths and the space between them and round them (at scale 1).
const CARD_MAIN: f32 = 400.0;
const CARD_DEVICES: f32 = 360.0;
const CARD_GAP: f32 = 24.0;

/// The page's items for its own card, and the devices' items under their card's head.
fn split(page: &Page) -> (&[Item], Option<(&'static str, &str)>, &[Item]) {
    let (main, rest) = page.items.split_at(page.devices.min(page.items.len()));
    match rest.split_first() {
        Some((Item::Section { icon, title }, body)) => (main, Some((icon, title.as_str())), body),
        _ => (main, None, rest),
    }
}

/// Where the full view's cards go in `body` (the city map's view under its header, window
/// pixels) at scale `s`: side by side when the body is wide enough for both, else one under
/// the other; across in the middle, and down the middle when it all fits (else from the top:
/// it scrolls).
pub(crate) fn full(page: &Page, body: Rect, s: f32, fonts: &Fonts) -> Full {
    let (main_items, devices, dev_items) = split(page);
    let margin = CARD_GAP * s;
    let room = (body.w - 2.0 * margin).max(0.0);
    let (gap, mw, dw) = (CARD_GAP * s, CARD_MAIN * s, CARD_DEVICES * s);
    let beside = devices.is_some() && mw + gap + dw <= room;
    let mw = if beside { mw } else { mw.min(room) };
    let dw = if beside { dw } else { mw };
    let (main_placed, mh) = layout(main_items, mw - 2.0 * PAD * s, s, fonts);
    let (dev_placed, dh) = layout(dev_items, dw - 2.0 * PAD * s, s, fonts);
    let card = |h: f32| ((HEAD + TOP + BOTTOM) * s + h).ceil();
    let (mh, dh) = (card(mh), card(dh));
    let inner = match (devices, beside) {
        (None, _) => mh,
        (Some(_), true) => mh.max(dh),
        (Some(_), false) => mh + gap + dh,
    };
    let height = inner + 2.0 * margin;
    let top = (body.y + ((body.h - height) * 0.5).max(0.0) + margin).round();
    let x = (body.x + (body.w - if beside { mw + gap + dw } else { mw }) * 0.5).round();
    let main = Rect::new(x, top, mw, mh);
    let devices = devices.map(|_| if beside { Rect::new(x + mw + gap, top, dw, dh) } else { Rect::new(x, top + mh + gap, dw, dh) });
    Full { main, devices, main_placed, dev_placed, height }
}

/// Where a card's items begin, its top left at `card` and scrolled up by `scroll` pixels.
fn card_origin(card: Rect, scroll: f32, s: f32) -> Vec2 {
    Vec2::new(card.x + PAD * s, card.y - scroll + (HEAD + TOP) * s)
}

/// Draw the full view's cards scrolled up by `scroll` pixels; those parts outside `view`
/// are left out (the layer drawn in clips the rest).
pub(crate) fn draw_full(pen: &mut Pen, page: &Page, f: &Full, scroll: f32, view: Rect, s: f32) {
    let (main_items, devices, dev_items) = split(page);
    let main = Rect::new(f.main.x, f.main.y - scroll, f.main.w, f.main.h);
    draw_head(pen, page, main, s);
    draw_items(pen, main_items, &f.main_placed, card_origin(f.main, scroll, s), view, s);
    if let (Some(card), Some((icon, title))) = (f.devices, devices) {
        let r = Rect::new(card.x, card.y - scroll, card.w, card.h);
        draw_frame(pen, r, s);
        draw_head_in(pen, icon, title, "", None, r, s);
        draw_items(pen, dev_items, &f.dev_placed, card_origin(card, scroll, s), view, s);
    }
}

/// Where a press does something in the full view, scrolled up by `scroll` (window pixels).
pub(crate) fn full_hits(page: &Page, f: &Full, scroll: f32, s: f32) -> Vec<(Rect, Action)> {
    let (main_items, _, dev_items) = split(page);
    let at = |hits: Vec<(Rect, Action)>, o: Vec2| hits.into_iter().map(move |(r, a)| (Rect::new(r.x + o.x, r.y + o.y, r.w, r.h), a));
    let mut out: Vec<(Rect, Action)> = at(hits(main_items, &f.main_placed, s), card_origin(f.main, scroll, s)).collect();
    if let Some(card) = f.devices {
        out.extend(at(hits(dev_items, &f.dev_placed, s), card_origin(card, scroll, s)));
    }
    out
}

/// Draw the items placed by [`layout`] with the page's top left at `origin` (scrolled: it may
/// be above the view); those wholly outside `view` are left out (the rest is clipped by the
/// layer drawn in).
pub(crate) fn draw_items(pen: &mut Pen, items: &[Item], placed: &[Placed], origin: Vec2, view: Rect, s: f32) {
    for (item, p) in items.iter().zip(placed) {
        let r = Rect::new(origin.x + p.rect.x, origin.y + p.rect.y, p.rect.w, p.rect.h);
        if r.bottom() < view.y || r.y > view.bottom() {
            continue;
        }
        draw_item(pen, item, &p.lines, r, s);
    }
}

fn draw_item(pen: &mut Pen, item: &Item, lines: &[String], r: Rect, s: f32) {
    let cy = r.center().y;
    match item {
        Item::Text(_, tone) => {
            let c = match tone {
                Tone::Plain => TEXT,
                Tone::Dim => TEXT_DIM,
                Tone::Bad => LATE_INK,
            };
            for (k, line) in lines.iter().enumerate() {
                pen.text_in(line, TEXT_PX * s, text_weight(*tone), Rect::new(r.x, r.y + (2.0 + k as f32 * TEXT_LINE) * s, r.w, TEXT_LINE * s), Align::Left, c);
            }
        }
        Item::Caption(t) => {
            pen.text_in(&t.to_uppercase(), 9.5 * s, Weight::Bold, Rect::new(r.x, r.y + 8.0 * s, r.w, 14.0 * s), Align::Left, TEXT_DIM);
        }
        Item::Pass { number, code } => {
            let gap = 8.0 * s;
            let tw = (r.w - gap) / 2.0;
            for (k, (value, label)) in [(number, "Personnel number"), (code, "Code")].into_iter().enumerate() {
                let t = Rect::new(r.x + k as f32 * (tw + gap), r.y + 2.0 * s, tw, 56.0 * s);
                pen.p.rounded(t, 8.0 * s, FIELD.alpha(0.7));
                pen.text_in(value, 21.0 * s, Weight::Bold, Rect::new(t.x + 12.0 * s, t.y + 7.0 * s, t.w - 24.0 * s, 26.0 * s), Align::Left, TEXT);
                pen.text_in(&omsi_ui::tr(label).to_uppercase(), 9.5 * s, Weight::Bold, Rect::new(t.x + 12.0 * s, t.y + 35.0 * s, t.w - 24.0 * s, 14.0 * s), Align::Left, TEXT_DIM);
            }
        }
        Item::PassLine(t) => {
            let b = Rect::new(r.x, r.y + 2.0 * s, r.w, 24.0 * s);
            pen.p.rounded(b, 6.0 * s, FIELD.alpha(0.45));
            pen.p.icon(pen.atlas, "badge", Vec2::new(b.x + 14.0 * s, b.center().y), 14.0 * s, TEXT_DIM);
            pen.text_in(t, 12.0 * s, Weight::Medium, Rect::new(b.x + 28.0 * s, b.y, b.w - 36.0 * s, b.h), Align::Left, TEXT_SOFT);
        }
        Item::Boxes { len, typed, hide, wrong } => {
            let n = (*len).max(1);
            let gap = 8.0 * s;
            let bw = (36.0 * s).min((r.w - gap * (n - 1) as f32) / n as f32);
            let x = r.x + (r.w - (bw * n as f32 + gap * (n - 1) as f32)) * 0.5;
            let at = typed.chars().count();
            for (k, ch) in typed.chars().map(Some).chain(std::iter::repeat(None)).take(n).enumerate() {
                let b = Rect::new(x + k as f32 * (bw + gap), r.y + 5.0 * s, bw, 46.0 * s);
                pen.p.rounded(b, 7.0 * s, FIELD);
                let (edge, t) = if *wrong { (LATE_INK, 1.5 * s) } else if k == at { (accent(), 1.5 * s) } else { (EDGE, 1.0) };
                pen.p.rounded_border(b, 7.0 * s, t.max(1.0), edge);
                match ch {
                    Some(_) if *hide => pen.p.circle(b.center(), 5.0 * s, TEXT),
                    Some(ch) => {
                        pen.text_in(&ch.to_string(), 22.0 * s, Weight::Bold, b, Align::Center, TEXT);
                    }
                    None => {}
                }
            }
        }
        Item::Keypad => {
            for (k, a) in keypad_keys(r, s) {
                let quiet = !matches!(a, Action::Digit(_));
                pen.p.rounded(k, 9.0 * s, if quiet { FIELD.alpha(0.45) } else { FIELD });
                match a {
                    Action::Digit(d) => {
                        pen.text_in(&d.to_string(), 22.0 * s, Weight::Bold, k, Align::Center, TEXT);
                    }
                    Action::Clear => {
                        pen.text_in("Clear", 12.5 * s, Weight::Medium, k, Align::Center, TEXT_SOFT);
                    }
                    _ => pen.p.icon(pen.atlas, "arrow_back", k.center(), 20.0 * s, TEXT_SOFT),
                }
            }
        }
        Item::Field { label, value } => {
            pen.text_in(label, 12.5 * s, Weight::Medium, Rect::new(r.x, r.y, r.w * 0.45, r.h), Align::Left, TEXT_DIM);
            pen.text_in(value, 14.5 * s, Weight::Bold, Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), Align::Right, TEXT);
            pen.p.rect(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), HAIRLINE);
        }
        Item::Button { label, icon, style, .. } => {
            let b = button_rect(r, s);
            let ink = match style {
                Style::Main => {
                    pen.p.rounded(b, 9.0 * s, accent());
                    Color::WHITE
                }
                Style::Plain => {
                    pen.p.rounded(b, 9.0 * s, FIELD);
                    TEXT
                }
                Style::Quiet => {
                    pen.p.rounded_border(b, 9.0 * s, 1.0, Color::WHITE.alpha(0.16));
                    TEXT_SOFT
                }
            };
            let px = 13.5 * s;
            let iw = if icon.is_some() { 26.0 * s } else { 0.0 };
            let label = pen.fonts.fit(label, px, Weight::Bold, (b.w - 24.0 * s - iw).max(0.0));
            let tw = pen.width(&label, px, Weight::Bold);
            let x = b.center().x - (iw + tw) * 0.5;
            if let Some(icon) = icon {
                pen.p.icon(pen.atlas, icon, Vec2::new(x + 9.0 * s, b.center().y), 18.0 * s, ink);
            }
            pen.text_in(&label, px, Weight::Bold, Rect::new(x + iw, b.y, tw + 2.0 * s, b.h), Align::Left, ink);
        }
        Item::Choice { lead, text, .. } => {
            let b = choice_rect(r, s);
            pen.p.rounded(b, 8.0 * s, FIELD.alpha(0.55));
            let cy = b.center().y;
            let lw = match lead {
                Lead::Plate(l) => pen.plate(Some(l.as_str()), b.x + 10.0 * s, cy, s),
                Lead::Time(t) => {
                    pen.text_in(t, 13.5 * s, Weight::Bold, Rect::new(b.x + 12.0 * s, b.y, 44.0 * s, b.h), Align::Left, TEXT);
                    44.0 * s
                }
            };
            let x = b.x + 10.0 * s + lw + 10.0 * s;
            pen.text_in(text, 13.0 * s, Weight::Medium, Rect::new(x, b.y, (b.right() - 30.0 * s - x).max(0.0), b.h), Align::Left, TEXT);
            pen.p.icon(pen.atlas, "chevron_right", Vec2::new(b.right() - 16.0 * s, cy), 18.0 * s, TEXT_DIM);
        }
        Item::Section { icon, title } => {
            pen.p.rect(Rect::new(r.x, r.y + 12.0 * s, r.w, 1.0), HAIRLINE);
            pen.p.icon(pen.atlas, icon, Vec2::new(r.x + 9.0 * s, r.y + 34.0 * s), 17.0 * s, TEXT);
            pen.text_in(title, 14.0 * s, Weight::Bold, Rect::new(r.x + 26.0 * s, r.y + 24.0 * s, r.w - 26.0 * s, 20.0 * s), Align::Left, TEXT);
        }
        Item::Duty { line, title, sub } => {
            let b = Rect::new(r.x, r.y + 4.0 * s, r.w, 60.0 * s);
            pen.p.rounded(b, 10.0 * s, FIELD.alpha(0.7));
            let cy = b.y + if sub.is_empty() { b.h * 0.5 } else { 21.0 * s };
            let lw = match line {
                Some(l) => pen.plate(Some(l.as_str()), b.x + 12.0 * s, cy, s),
                None => {
                    pen.p.icon(pen.atlas, "directions_bus", Vec2::new(b.x + 24.0 * s, cy), 20.0 * s, TEXT_SOFT);
                    24.0 * s
                }
            };
            let x = b.x + 12.0 * s + lw + 10.0 * s;
            pen.text_in(title, 15.0 * s, Weight::Bold, Rect::new(x, cy - 10.0 * s, (b.right() - 12.0 * s - x).max(0.0), 20.0 * s), Align::Left, TEXT);
            if !sub.is_empty() {
                pen.text_in(sub, 11.5 * s, Weight::Medium, Rect::new(b.x + 12.0 * s, b.y + 37.0 * s, b.w - 24.0 * s, 16.0 * s), Align::Left, TEXT_DIM);
            }
        }
        Item::Break { minutes, part, label, within } => {
            // (Omsi-Hub's ring: it fills as the break does, green within it, red over it)
            let c = Vec2::new(r.x + 40.0 * s, r.y + 46.0 * s);
            pen.p.arc(c, 31.0 * s, 37.0 * s, 0.0, std::f32::consts::TAU, FIELD);
            let tint = match within {
                Some(true) => ON_TIME_INK,
                Some(false) => LATE_INK,
                None => accent_ink(),
            };
            if *part > 0.0 {
                let a0 = -std::f32::consts::FRAC_PI_2;
                pen.p.arc(c, 31.0 * s, 37.0 * s, a0, a0 + std::f32::consts::TAU * part.clamp(0.0, 1.0), tint);
            }
            pen.text_in(&minutes.to_string(), 22.0 * s, Weight::Bold, Rect::new(c.x - 28.0 * s, c.y - 17.0 * s, 56.0 * s, 24.0 * s), Align::Center, TEXT);
            pen.text_in("min", 10.0 * s, Weight::Medium, Rect::new(c.x - 28.0 * s, c.y + 7.0 * s, 56.0 * s, 14.0 * s), Align::Center, TEXT_DIM);
            let x = r.x + 92.0 * s;
            pen.text_in(label, 13.5 * s, Weight::Bold, Rect::new(x, c.y - 10.0 * s, (r.right() - x).max(0.0), 20.0 * s), Align::Left, if within.is_some() { tint } else { TEXT_SOFT });
        }
        Item::Trip { departure, line, terminus, arrival } => {
            pen.text_in(&hhmm(*departure), 13.5 * s, Weight::Bold, Rect::new(r.x, cy - 9.0 * s, 44.0 * s, 18.0 * s), Align::Left, TEXT);
            let at = hhmm(*arrival);
            let aw = pen.width(&at, 13.0 * s, Weight::Medium) + 2.0 * s;
            pen.text_in(&at, 13.0 * s, Weight::Medium, Rect::new(r.right() - aw, cy - 9.0 * s, aw, 18.0 * s), Align::Right, TEXT_DIM);
            let px = r.x + 48.0 * s;
            let pw = pen.plate(line.as_deref(), px, cy, s * 0.92);
            let x = px + pw + 8.0 * s;
            pen.text_in(terminus, 13.5 * s, Weight::Bold, Rect::new(x, cy - 9.0 * s, (r.right() - aw - 8.0 * s - x).max(0.0), 18.0 * s), Align::Left, TEXT);
        }
        Item::Address(a) => {
            pen.p.icon(pen.atlas, "link", Vec2::new(r.x + 9.0 * s, cy), 16.0 * s, accent_ink());
            pen.text_in(a, 15.0 * s, Weight::Bold, Rect::new(r.x + 26.0 * s, r.y, r.w - 26.0 * s, r.h), Align::Left, TEXT);
        }
        Item::Code(code) => {
            let (bw, gap) = (32.0 * s, 6.0 * s);
            for (k, ch) in code.chars().enumerate() {
                let b = Rect::new(r.x + k as f32 * (bw + gap), r.y + 6.0 * s, bw, 42.0 * s);
                if b.right() > r.right() + 0.5 {
                    break;
                }
                pen.p.rounded(b, 7.0 * s, FIELD);
                pen.p.rounded_border(b, 7.0 * s, 1.0, EDGE);
                pen.text_in(&ch.to_string(), 22.0 * s, Weight::Bold, b, Align::Center, TEXT);
            }
        }
        Item::Qr { qr, max } => {
            let m = qr_module(qr.side, (*max * s).min(r.w));
            let side = m * (qr.side + 2 * QR_QUIET) as f32;
            let at = Vec2::new(r.x + (r.w - side) * 0.5, r.y + 4.0 * s).round();
            draw_qr(pen.p, qr, at, m);
            pen.text_in("Scan with your phone or tablet", 12.5 * s, Weight::Bold, Rect::new(r.x, at.y + side + 8.0 * s, r.w, 18.0 * s), Align::Center, TEXT_SOFT);
        }
        Item::Pairing { qr, address, code } => {
            pen.p.rect(Rect::new(r.x, r.y + 6.0 * s, r.w, 1.0), HAIRLINE);
            let mut y = r.y + 14.0 * s;
            let (tx, lines) = match qr.as_deref() {
                Some(q) => {
                    let (m, side) = pairing_qr_px(q, r.w, s);
                    draw_qr(pen.p, q, Vec2::new(r.x, y), m);
                    let tx = r.x + side + 12.0 * s;
                    // (the four lines down the middle of the code beside them)
                    y += ((side - 77.0 * s) * 0.5).max(0.0);
                    (tx, vec![("Scan with your phone or tablet".to_string(), 11.5 * s, Weight::Medium, TEXT_DIM), (address.clone(), 13.0 * s, Weight::Bold, TEXT), (tr_with("Pairing code %{code}", &[("code", code.clone())]), 12.5 * s, Weight::Bold, accent_ink())])
                }
                None => {
                    pen.p.icon(pen.atlas, "wifi_tethering", Vec2::new(r.x + 9.0 * s, y + 10.0 * s), 17.0 * s, TEXT);
                    (r.x + 26.0 * s, vec![(format!("{address}  ·  {}", tr_with("Pairing code %{code}", &[("code", code.clone())])), 12.5 * s, Weight::Bold, TEXT_SOFT)])
                }
            };
            let tw = (r.right() - tx).max(0.0);
            pen.text_in("Phone & tablet", 13.5 * s, Weight::Bold, Rect::new(tx, y, tw, 20.0 * s), Align::Left, TEXT);
            for (k, (t, px, weight, c)) in lines.iter().enumerate() {
                pen.text_in(t, *px, *weight, Rect::new(tx, y + (22.0 + k as f32 * 19.0) * s, tw, 18.0 * s), Align::Left, *c);
            }
        }
        Item::Hidden => {
            // (a covered field where the address and the codes would be)
            let b = Rect::new(r.x, r.y + 4.0 * s, r.w, r.h - 8.0 * s);
            pen.p.rounded(b, 10.0 * s, FIELD.alpha(0.7));
            pen.p.rounded_border(b, 10.0 * s, 1.0, EDGE);
            pen.p.icon(pen.atlas, "lock", Vec2::new(b.x + 22.0 * s, b.y + b.h * 0.5), 18.0 * s, TEXT_DIM);
            let tx = b.x + 42.0 * s;
            let tw = (b.right() - 10.0 * s - tx).max(0.0);
            pen.text_in(&tr("Address and pairing code hidden"), 13.5 * s, Weight::Bold, Rect::new(tx, b.y + 9.0 * s, tw, 18.0 * s), Align::Left, TEXT);
            pen.text_in(&tr("Covered for streaming. Nobody sees them on screen."), 11.0 * s, Weight::Medium, Rect::new(tx, b.y + 29.0 * s, tw, 16.0 * s), Align::Left, TEXT_DIM);
        }
        Item::Device { name, what } => {
            pen.p.icon(pen.atlas, "smartphone", Vec2::new(r.x + 9.0 * s, cy), 15.0 * s, TEXT_DIM);
            pen.text_in(name, 13.0 * s, Weight::Bold, Rect::new(r.x + 26.0 * s, r.y + 4.0 * s, r.w - 26.0 * s, 17.0 * s), Align::Left, TEXT);
            pen.text_in(what, 11.0 * s, Weight::Medium, Rect::new(r.x + 26.0 * s, r.y + 21.0 * s, r.w - 26.0 * s, 14.0 * s), Align::Left, TEXT_FAINT.lighten(0.25));
        }
        Item::Gap(_) => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::companion::{DeviceInfo, DutyMenu, DutyOrder};
    use omsi_ui::{Atlas, Painter};

    /// Hans, not signed on yet, the companion off.
    pub(crate) fn hans() -> CompanionState {
        CompanionState { driver: "Hans Müller".into(), personnel_number: "482913".into(), personnel_code: "5821".into(), ..CompanionState::default() }
    }

    fn order() -> DutyOrder {
        DutyOrder { line: "35".into(), tour: "1".into(), lines: "35 / 25".into(), start: 11.0 * 3600.0 + 26.0 * 60.0, end: 12.0 * 3600.0 + 61.0 * 60.0, trips: 3 }
    }

    /// Signed on at `stage`.
    pub(crate) fn at(stage: Stage) -> CompanionState {
        let duty = matches!(stage, Stage::DutyOrder | Stage::OnDuty).then(order);
        CompanionState { signed_on: true, stage, accepted: stage == Stage::OnDuty, duty, ..hans() }
    }

    /// The companion on, listening, with a device watching the IBIS.
    pub(crate) fn listening(st: CompanionState) -> CompanionState {
        CompanionState {
            enabled: true,
            listening: Some(47811),
            addresses: vec!["http://192.168.1.20:47811/".into(), "http://10.0.0.5:47811/".into()],
            pairing_code: "731904".into(),
            devices: vec![DeviceInfo { name: "Safari on iPad".into(), seconds_ago: 1.0, screen: Some("s0".into()) }],
            paired: 1,
            screens: vec![("s0".into(), "IBIS".into())],
            ..st
        }
    }

    /// Streaming: the address, the pairing code and the QR code are covered on every view of
    /// the page, "Show" uncovers them (and "Hide again" covers them at once); the tunnel's
    /// state is said, and its address shown in place of the home network's.
    #[test]
    fn the_address_and_codes_are_covered_while_streaming_until_shown() {
        let qr = made_up_qr(25);
        let shows_secrets = |p: &Page| p.items.iter().any(|i| matches!(i, Item::Address(_) | Item::Code(_) | Item::Qr { .. } | Item::Pairing { .. }));
        let covered = CompanionState { hide: true, hidden: true, ..listening(hans()) };
        for room in [Room::Column, Room::Full, Room::Panel] {
            let p = page_in(&covered, &Phone::default(), None, 0.0, room, Some(&qr));
            assert!(!shows_secrets(&p), "{room:?} shows the address or a code while streaming");
            assert!(p.items.contains(&Item::Hidden), "{room:?}");
            assert!(actions(&p).contains(&Action::Reveal(true)), "{room:?} has no Show");
            assert!(!texts(&p).iter().any(|t| t.contains("731904") || t.contains("192.168")), "{room:?}");
        }
        // shown for a while: all of it, and a way to cover it again
        let shown = CompanionState { hide: true, hidden: false, ..listening(hans()) };
        let p = page_in(&shown, &Phone::default(), None, 0.0, Room::Column, Some(&qr));
        assert!(shows_secrets(&p) && !p.items.contains(&Item::Hidden));
        assert!(actions(&p).contains(&Action::Reveal(false)));
        // the setting off: shown, and no button about it
        let plain = page_in(&listening(hans()), &Phone::default(), None, 0.0, Room::Column, Some(&qr));
        assert!(shows_secrets(&plain) && !actions(&plain).iter().any(|a| matches!(a, Action::Reveal(_))));
        let mut ui = Phone::default();
        assert_eq!(ui.press(Action::Reveal(true), &covered), Some(Call::Reveal(true)));
        // the tunnel: its address in place of the network's, and what it is doing
        use crate::companion::TunnelState;
        let url = "https://quiet-river.trycloudflare.com";
        let public = CompanionState { addresses: vec![url.into()], tunnel: Some(TunnelState::Ready(url.into())), ..listening(hans()) };
        let p = page(&public, &Phone::default(), None, 0.0);
        assert!(p.items.contains(&Item::Address(url.into())), "the https address as it is typed");
        assert!(texts(&p).contains(&"On a phone or tablet, open".to_string()));
        let missing = page(&CompanionState { tunnel: Some(TunnelState::Missing), ..listening(hans()) }, &Phone::default(), None, 0.0);
        assert!(missing.items.iter().any(|i| matches!(i, Item::Text(t, Tone::Bad) if t.contains("cloudflared"))));
    }

    /// A made-up code of `side` modules: the three finder squares in their corners, and a
    /// scatter of modules between them (no real code: the drawing is what is tested).
    pub(crate) fn made_up_qr(side: usize) -> Arc<Qr> {
        let finder = |x: usize, y: usize| {
            let corner = |cx: usize, cy: usize| {
                let (dx, dy) = (x.wrapping_sub(cx), y.wrapping_sub(cy));
                (dx < 7 && dy < 7).then(|| dx == 0 || dx == 6 || dy == 0 || dy == 6 || ((2..=4).contains(&dx) && (2..=4).contains(&dy)))
            };
            corner(0, 0).or(corner(side - 7, 0)).or(corner(0, side - 7))
        };
        let dark = (0..side * side).map(|k| finder(k % side, k / side).unwrap_or((k * 7919 + k / side * 31) % 5 < 2)).collect();
        Arc::new(Qr::new(side, dark).unwrap())
    }

    fn actions(p: &Page) -> Vec<Action> {
        p.items
            .iter()
            .filter_map(|i| match i {
                Item::Button { action, .. } | Item::Choice { action, .. } => Some(*action),
                _ => None,
            })
            .collect()
    }

    fn texts(p: &Page) -> Vec<String> {
        p.items.iter().filter_map(|i| if let Item::Text(t, _) = i { Some(t.clone()) } else { None }).collect()
    }

    /// The number first, checked as soon as it is complete, then the code typed blind; a
    /// wrong one empties the boxes and turns them red on the same step.
    #[test]
    fn signing_on_takes_the_number_then_the_code() {
        let st = hans();
        let mut ui = Phone::default();
        ui.follow(&st);
        for d in [4, 8, 2, 9, 1] {
            assert_eq!(ui.press(Action::Digit(d), &st), None);
        }
        ui.press(Action::Erase, &st);
        ui.press(Action::Digit(1), &st);
        assert_eq!(ui.typed, "48291");
        assert_eq!(ui.press(Action::Digit(3), &st), Some(Call::SignOn { number: "482913".into(), code: None }));
        assert_eq!(ui.typed, "");
        ui.attempted(Attempt::Number);
        assert!(ui.code_step);
        let p = page(&st, &ui, None, 0.0);
        assert!(p.items.contains(&Item::Boxes { len: 4, typed: String::new(), hide: true, wrong: false }));
        for d in [1, 1, 1] {
            ui.press(Action::Digit(d), &st);
        }
        assert_eq!(ui.press(Action::Digit(1), &st), Some(Call::SignOn { number: "482913".into(), code: Some("1111".into()) }));
        ui.attempted(Attempt::Wrong);
        let p = page(&st, &ui, None, 0.0);
        assert!(p.items.contains(&Item::Boxes { len: 4, typed: String::new(), hide: true, wrong: true }));
        assert!(texts(&p).iter().any(|t| t.contains("pass below")));
        // (signed on, the keypad takes nothing)
        for d in [5, 8, 2, 1] {
            ui.press(Action::Digit(d), &st);
        }
        ui.attempted(Attempt::SignedOn);
        assert!(!ui.code_step && !ui.wrong);
        assert_eq!(ui.press(Action::Digit(1), &at(Stage::DutyMenu)), None);
    }

    /// Not signed on: the keypad, and the pass with the number and the code under it (as
    /// Omsi-Hub's duty panel showed it until the duty was signed).
    #[test]
    fn before_signing_on_the_page_is_the_keypad_and_the_pass() {
        let st = hans();
        let p = page(&st, &Phone::default(), None, 0.0);
        assert_eq!((p.title.as_str(), p.sub.as_str(), p.signed_on), ("Sign on", "Hans Müller", false));
        assert!(p.items.contains(&Item::Keypad));
        assert!(p.items.contains(&Item::Boxes { len: 6, typed: String::new(), hide: false, wrong: false }));
        assert!(p.items.contains(&Item::Pass { number: "482913".into(), code: "5821".into() }));
        // nothing to accept or choose before signing on
        assert!(!actions(&p).iter().any(|a| matches!(a, Action::Accept | Action::OpenMenu | Action::Free | Action::SignOff)));
        assert!(waiting(&st), "the navigator is the page until the duty is signed for");
        assert!(!waiting(&CompanionState { personnel_number: String::new(), ..hans() }), "not before the game knows the driver");
    }

    /// Signed on without a duty: the duty menu - the lines, asked for once; a line's tours;
    /// a tour taken on - or driving without a duty; the pass as a small line.
    #[test]
    fn the_duty_menu_lists_the_lines_and_takes_on_a_tour() {
        let mut st = at(Stage::DutyMenu);
        let mut ui = Phone::default();
        ui.follow(&st);
        assert_eq!(ui.wants(&st), Some(Request::Lines));
        assert_eq!(ui.wants(&st), None, "asked once");
        let p = page(&st, &ui, None, 0.0);
        assert_eq!(p.title, "Choose a duty");
        assert_eq!(p.items[0], Item::PassLine("Personnel no. 482913 · code 5821".into()));
        assert!(texts(&p).contains(&"Loading…".to_string()));
        st.menu = DutyMenu { lines: Some(vec![("35".into(), "35 Hauptbahnhof".into()), ("25".into(), "25 Eichstedt".into())]), ..DutyMenu::default() };
        let p = page(&st, &ui, None, 0.0);
        assert_eq!(actions(&p), [Action::Line(0), Action::Line(1), Action::Free, Action::SignOff]);
        assert_eq!(ui.press(Action::Line(1), &st), Some(Call::Ask(Request::Tours(1))));
        st.menu.tours = Some((1, vec![("Tour 3 › Hafen".into(), "05:42".into()), ("Tour 4 › Markt".into(), "06:12".into())]));
        let p = page(&st, &ui, None, 0.0);
        assert!(p.items.contains(&Item::Choice { lead: Lead::Time("06:12".into()), text: "Tour 4 › Markt".into(), action: Action::Tour(1) }));
        assert_eq!(actions(&p)[0], Action::Lines);
        assert_eq!(ui.press(Action::Tour(1), &st), Some(Call::Ask(Request::Pick(1, 1))));
        assert_eq!(ui.line, None);
        // it did not work: back at the lines, saying so
        st.menu.failed = true;
        assert!(texts(&page(&st, &ui, None, 0.0)).contains(&"That did not work. Try again.".to_string()));
        assert!(!waiting(&st), "signed on, no duty to sign");
    }

    /// A duty waiting: its order with the times, accepted with one press; another duty can
    /// be chosen first (the menu opens over the order, with a way back).
    #[test]
    fn the_duty_order_is_accepted_or_another_duty_chosen() {
        let st = at(Stage::DutyOrder);
        let mut ui = Phone::default();
        ui.follow(&st);
        let p = page(&st, &ui, None, 0.0);
        assert_eq!(p.title, "Duty assignment");
        let fields: Vec<(String, String)> = p.items.iter().filter_map(|i| if let Item::Field { label, value } = i { Some((label.clone(), value.clone())) } else { None }).collect();
        assert_eq!(fields, [("Line".into(), "35 / 25".into()), ("Tour".into(), "1".into()), ("Departure".into(), "11:26".into()), ("Back at".into(), "13:01".into()), ("Trips".into(), "3".into())]);
        assert_eq!(actions(&p), [Action::Accept, Action::OpenMenu, Action::SignOff]);
        assert_eq!(ui.press(Action::Accept, &st), Some(Call::Ask(Request::Accept)));
        assert_eq!(ui.wants(&st), None);
        ui.press(Action::OpenMenu, &st);
        assert_eq!(ui.wants(&st), Some(Request::Lines));
        let p = page(&st, &ui, None, 0.0);
        assert_eq!(p.title, "Choose a duty");
        assert_eq!(actions(&p), [Action::CloseMenu, Action::SignOff], "no driving freely with a duty there");
        assert!(waiting(&st), "the duty order is part of signing on");
        // the duty signed (here or on a phone): the page starts afresh at work
        ui.follow(&at(Stage::OnDuty));
        assert!(!ui.menu);
    }

    /// Signing on not asked for (the setting `nav_signon` off): the duty is driven at once -
    /// nothing waits, and the page (opened on purpose) has the duty but no pass and no
    /// signing off.
    #[test]
    fn without_signing_on_nothing_waits_and_nothing_signs_off() {
        let st = CompanionState { auto_sign_on: true, ..at(Stage::OnDuty) };
        assert!(!waiting(&st));
        let mut ui = Phone::default();
        ui.follow(&st);
        let p = page(&st, &ui, None, 0.0);
        assert_eq!(p.title, "Duty");
        assert!(!actions(&p).contains(&Action::SignOff), "{:?}", actions(&p));
        assert!(actions(&p).contains(&Action::OpenMenu), "another duty can still be chosen");
        assert!(!p.items.iter().any(|i| matches!(i, Item::PassLine(_) | Item::Pass { .. } | Item::Keypad)));
        // with signing on asked for, the same duty has both
        let asked = page(&at(Stage::OnDuty), &ui, None, 0.0);
        assert!(actions(&asked).contains(&Action::SignOff));
        assert!(asked.items.iter().any(|i| matches!(i, Item::PassLine(_))));
    }

    /// At work: the duty, the break against the layover at the terminus, another duty and
    /// signing off. The break runs in the game's time.

    #[test]
    fn at_work_the_break_is_timed_against_the_layover() {
        let (trips, tours) = crate::nav_duty::tests::duty();
        // under way on trip 1 (ends 11:46); trip 2 leaves 12:01: a 15 minute layover
        let d = crate::nav_duty::tests::state(&trips, &tours, 0, 3, false, 0.0);
        let mut st = at(Stage::OnDuty);
        let ui = Phone::default();
        let p = page(&st, &ui, Some(&d), 11.0 * 3600.0 + 50.0 * 60.0);
        assert_eq!(p.title, "Signed on");
        assert!(p.items.iter().any(|i| matches!(i, Item::Duty { line: Some(l), title, sub } if l == "35 / 25" && title == "Tour 1" && sub.starts_with("11:26 – 13:01"))));
        assert!(p.items.contains(&Item::Break { minutes: 15, part: 0.0, label: "Scheduled break at the terminus".into(), within: None }));
        assert!(p.items.iter().any(|i| matches!(i, Item::Trip { terminus, .. } if terminus == "Hauptbahnhof")));
        assert_eq!(actions(&p), [Action::Break(true), Action::OpenMenu, Action::SignOff]);
        // ten minutes into it, then twenty
        st.break_since = Some(11.0 * 3600.0 + 47.0 * 60.0);
        let p = page(&st, &ui, Some(&d), 11.0 * 3600.0 + 57.0 * 60.0 + 30.0);
        assert!(p.items.contains(&Item::Break { minutes: 10, part: 10.0 / 15.0, label: "%{minutes} min left".replace("%{minutes}", "5"), within: Some(true) }));
        assert_eq!(actions(&p)[0], Action::Break(false));
        let p = page(&st, &ui, Some(&d), 12.0 * 3600.0 + 7.0 * 60.0);
        assert!(p.items.contains(&Item::Break { minutes: 20, part: 1.0, label: "5 min over".into(), within: Some(false) }));
        // across midnight the minutes go on
        st.break_since = Some(86_000.0);
        assert!(page(&st, &ui, Some(&d), 400.0).items.iter().any(|i| matches!(i, Item::Break { minutes: 13, .. })));
        // driving without a duty: no layover to time against
        let free = CompanionState { duty: None, free: true, break_since: None, ..at(Stage::OnDuty) };
        let p = page(&free, &ui, None, 0.0);
        assert!(p.items.contains(&Item::Duty { line: None, title: "Driving without a duty".into(), sub: String::new() }));
        assert!(p.items.contains(&Item::Break { minutes: 0, part: 0.0, label: String::new(), within: None }));
        assert_eq!(actions(&p), [Action::Break(true), Action::OpenMenu, Action::SignOff]);
        assert!(!waiting(&st));
    }

    /// The devices: off, how to turn it on; on, where to go and the code, the devices.
    #[test]
    fn the_devices_say_how_to_connect() {
        let off = page(&hans(), &Phone::default(), None, 0.0);
        assert!(off.items.contains(&Item::Section { icon: "wifi_tethering", title: "Phone & tablet".into() }));
        assert!(texts(&off).last().is_some_and(|t| t.contains("Turn it on in the settings")));
        assert!(!actions(&off).contains(&Action::NewCode));
        let on = page(&listening(hans()), &Phone::default(), None, 0.0);
        assert!(on.items.contains(&Item::Address("192.168.1.20:47811".into())));
        assert!(on.items.contains(&Item::Address("10.0.0.5:47811".into())));
        assert!(on.items.contains(&Item::Code("731904".into())));
        assert!(on.items.contains(&Item::Device { name: "Safari on iPad".into(), what: "Shows %{screen}".replace("%{screen}", "IBIS") }));
        assert!(texts(&on).contains(&"Paired devices: 1".to_string()));
        assert!(actions(&on).ends_with(&[Action::NewCode, Action::Forget]));
        let none = page(&CompanionState { paired: 0, devices: Vec::new(), ..listening(hans()) }, &Phone::default(), None, 0.0);
        assert!(actions(&none).ends_with(&[Action::NewCode]));
        let mut ui = Phone::default();
        assert_eq!(ui.press(Action::Forget, &hans()), Some(Call::Forget));
        assert_eq!(ui.press(Action::NewCode, &hans()), Some(Call::NewCode));
        let broken = page(&CompanionState { enabled: true, error: Some("port taken".into()), ..hans() }, &Phone::default(), None, 0.0);
        assert!(broken.items.contains(&Item::Text("The phone companion could not start: port taken".into(), Tone::Bad)));
        let starting = page(&CompanionState { enabled: true, ..hans() }, &Phone::default(), None, 0.0);
        assert!(texts(&starting).contains(&"Starting…".to_string()));
        // the QR code (when the companion has one) first, the address to type after it
        let qr = made_up_qr(25);
        let with = page_in(&listening(hans()), &Phone::default(), None, 0.0, Room::Column, Some(&qr));
        let at = |p: &Page, f: &dyn Fn(&Item) -> bool| p.items.iter().position(f);
        let (q, a) = (at(&with, &|i| matches!(i, Item::Qr { .. })), at(&with, &|i| matches!(i, Item::Address(_))));
        assert!(q.is_some() && q < a && q > Some(with.devices), "{:?}", with.items);
        assert_eq!(with.items[with.devices], Item::Section { icon: "wifi_tethering", title: "Phone & tablet".into() });
        // (none while a device cannot pair: off, or no address)
        assert!(at(&page_in(&hans(), &Phone::default(), None, 0.0, Room::Column, Some(&qr)), &|i| matches!(i, Item::Qr { .. })).is_none());
        assert!(at(&page_in(&CompanionState { addresses: Vec::new(), ..listening(hans()) }, &Phone::default(), None, 0.0, Room::Full, Some(&qr)), &|i| matches!(i, Item::Qr { .. })).is_none());
    }

    /// The small navigator is the page from edge to edge while the duty waits: the keypad,
    /// the pass as one line under it, and the devices as one block - the QR code beside where
    /// to go and the code - which a short window leaves out before the page is made smaller.
    #[test]
    fn the_small_navigator_is_the_page() {
        let fonts = Fonts::hanken();
        let qr = made_up_qr(29);
        let st = listening(hans());
        let p = page_in(&st, &Phone::default(), None, 0.0, Room::Panel, Some(&qr));
        assert!(p.items.contains(&Item::Keypad));
        assert!(p.items.contains(&Item::PassLine("Personnel no. 482913 · code 5821".into())));
        assert!(!p.items.iter().any(|i| matches!(i, Item::Pass { .. } | Item::Section { .. } | Item::Button { .. })), "{:?}", p.items);
        assert_eq!(&p.items[p.devices..], [Item::Pairing { qr: Some(qr.clone()), address: "192.168.1.20:47811".into(), code: "731904".into() }]);
        // (a companion that is off says nothing here)
        let off = page_in(&hans(), &Phone::default(), None, 0.0, Room::Panel, None);
        assert_eq!(off.devices, off.items.len());
        // room enough: all of it at the panel's scale
        let (w, s) = (360.0 * 1.2, 1.2);
        let all = fit_panel(p.clone(), w, s, 2000.0, &fonts);
        assert_eq!((all.page.items.len(), all.s, all.height), (p.items.len(), s, all.content));
        assert!((all.height - ((HEAD + TOP + BOTTOM) * s + all.placed.last().unwrap().rect.bottom()).ceil()).abs() < 1e-3);
        assert_eq!(panel_scroll_max(&all), 0.0);
        // a little short: the devices go first
        let less = fit_panel(p.clone(), w, s, all.height - 20.0, &fonts);
        assert_eq!((less.page.items.len(), less.s), (p.devices, s));
        assert!(less.height <= all.height - 20.0 && less.height == less.content);
        // short: the page made smaller to fit
        let small = fit_panel(p.clone(), w, s, 470.0, &fonts);
        assert!(small.s < s && small.s >= s * PANEL_SMALLEST && small.height <= 470.0 && small.height == small.content, "{} {}", small.s, small.height);
        assert!(small.page.items.contains(&Item::Keypad));
        // very short: three quarters, and the rest scrolls - its keys out of sight are none
        let tiny = fit_panel(p.clone(), w, s, 300.0, &fonts);
        assert!((tiny.s - s * PANEL_SMALLEST).abs() < 1e-4 && tiny.height == 300.0 && tiny.content > 300.0, "{} {} {}", tiny.s, tiny.height, tiny.content);
        let max = panel_scroll_max(&tiny);
        let top = panel_hits(&tiny, 0.0);
        let bottom = panel_hits(&tiny, max);
        assert!(top.iter().any(|h| h.1 == Action::Digit(1)) && !top.iter().any(|h| h.1 == Action::Erase));
        assert!(bottom.iter().any(|h| h.1 == Action::Erase));
        for (r, _) in top.iter().chain(&bottom) {
            assert!(r.center().y >= HEAD * tiny.s && r.center().y <= tiny.height);
        }
        // the order to sign on the small navigator too: accepted there
        let order = page_in(&listening(at(Stage::DutyOrder)), &Phone::default(), None, 0.0, Room::Panel, None);
        assert_eq!(actions(&order), [Action::Accept, Action::OpenMenu, Action::SignOff]);
        // its presses land on their keys, inside the panel
        let f = fit_panel(p, w, s, 2000.0, &fonts);
        let hits = panel_hits(&f, 0.0);
        assert_eq!(hits.len(), 12, "the keypad's keys");
        assert!(hits.iter().all(|(r, _)| r.x >= 0.0 && r.right() <= w && r.y >= HEAD * f.s && r.bottom() <= f.height));
        let five = hits.iter().find(|h| h.1 == Action::Digit(5)).unwrap().0;
        assert_eq!(hits.iter().find(|h| h.0.contains(five.center())).map(|h| h.1), Some(Action::Digit(5)));
    }

    /// The city map's whole view: the page's card and the devices' beside it when the view is
    /// wide enough, one under the other when not; in the middle; a press lands on its key in
    /// the window's pixels, scrolled or not.
    #[test]
    fn the_full_view_puts_the_devices_beside_the_page() {
        let fonts = Fonts::hanken();
        let qr = made_up_qr(25);
        let st = listening(hans());
        let p = page_in(&st, &Phone::default(), None, 0.0, Room::Full, Some(&qr));
        let s: f32 = 1.2;
        let body = Rect::new(0.0, 53.0, 1536.0, 832.0);
        let f = full(&p, body, s, &fonts);
        let d = f.devices.unwrap();
        assert!((f.main.y - d.y).abs() < 1e-3 && d.x >= f.main.right() + 20.0 * s, "{f:?}");
        let middle = (f.main.x + d.right()) * 0.5;
        assert!((middle - body.center().x).abs() <= 1.0, "across in the middle");
        assert!(f.main.y >= body.y && d.bottom() <= body.bottom(), "{:?} {:?} in {body:?}", f.main, d);
        assert!(f.dev_placed.len() + f.main_placed.len() + 1 == p.items.len(), "(the section is the card's head)");
        // narrow: one under the other, as wide
        let narrow = full(&p, Rect::new(0.0, 53.0, 600.0, 832.0), s, &fonts);
        let nd = narrow.devices.unwrap();
        assert!(nd.y >= narrow.main.bottom() && (nd.x - narrow.main.x).abs() < 1e-3 && (nd.w - narrow.main.w).abs() < 1e-3);
        assert!(narrow.height > 832.0, "it scrolls");
        assert!((narrow.main.y - (53.0 + CARD_GAP * s)).abs() < 1.0, "from the top");
        // the presses: the keypad's 5, and the devices' card's button, in window pixels
        for scroll in [0.0, 37.0] {
            let hits = full_hits(&p, &f, scroll, s);
            let five = hits.iter().find(|h| h.1 == Action::Digit(5)).unwrap().0;
            assert!(f.main.contains(Vec2::new(five.center().x, five.center().y + scroll)));
            let code = hits.iter().find(|h| h.1 == Action::NewCode).unwrap().0;
            assert!(d.contains(Vec2::new(code.center().x, code.center().y + scroll)));
        }
    }

    /// The QR code in whole pixels a module (sharp), with its quiet zone round it on a white
    /// square, and every dark module drawn - in runs along the rows - and nothing else.
    #[test]
    fn a_made_up_qr_code_draws_sharp_and_whole() {
        let q = made_up_qr(29);
        assert_eq!(Qr::new(5, vec![true; 25]), None, "no QR code is that small");
        assert_eq!(Qr::new(21, vec![true; 20]), None);
        for max in [74.0, 150.0, 188.0 * 1.37, 232.0 * 2.0] {
            let m = qr_module(q.side, max);
            assert!(m.fract() == 0.0 && m >= 2.0 && qr_px(q.side, max) <= max.max(2.0 * 37.0), "{max}: {m}");
            let at = Vec2::new(20.4, 31.6);
            let runs = qr_runs(&q, at, m);
            let o = at.round() + Vec2::splat(4.0 * m);
            // (every run on whole pixels, inside the code, the quiet zone free)
            for r in &runs {
                assert!([r.x, r.y, r.w, r.h].iter().all(|v| v.fract() == 0.0), "{r:?}");
                assert!(r.x >= o.x && r.y >= o.y && r.right() <= o.x + 29.0 * m && r.bottom() <= o.y + 29.0 * m);
            }
            // (and every dark module in one of them)
            let dark = q.dark.iter().filter(|d| **d).count();
            assert_eq!(runs.iter().map(|r| (r.w / m) as usize).sum::<usize>(), dark);
            for (k, d) in q.dark.iter().enumerate() {
                let c = o + Vec2::new((k % 29) as f32 + 0.5, (k / 29) as f32 + 0.5) * m;
                assert_eq!(runs.iter().any(|r| r.contains(c)), *d, "module {k}");
            }
            // drawn: the white square first, then the dark runs
            let mut p = Painter::new();
            draw_qr(&mut p, &q, at, m);
            let side = 37.0 * m;
            assert!(p.verts.iter().all(|v| v.pos[0] >= 20.0 && v.pos[0] <= 20.0 + side && v.pos[1] >= 32.0 && v.pos[1] <= 32.0 + side));
            assert!(p.verts.iter().filter(|v| v.color == QR_INK.0).count() == runs.len() * 6);
            assert!(p.verts.iter().filter(|v| v.color != QR_INK.0).all(|v| v.color == Color::WHITE.0));
        }
    }

    /// Signing off here forgets what the page had, and asks the game to sign off.
    #[test]
    fn signing_off_starts_afresh() {
        let st = at(Stage::OnDuty);
        let mut ui = Phone::default();
        ui.follow(&st);
        ui.press(Action::OpenMenu, &st);
        ui.scroll = 300.0;
        assert_eq!(ui.press(Action::SignOff, &st), Some(Call::SignOff));
        assert!(!ui.menu && ui.scroll == 0.0);
        assert_eq!(ui.press(Action::Free, &st), Some(Call::Free));
        assert_eq!(ui.press(Action::Break(true), &st), Some(Call::Ask(Request::Break(true))));
    }

    /// Every key, button and row has a place of its own on the page, inside it; a press in
    /// the middle of the keypad's 5 is a 5. The keyboard's digits work the keypad too.
    #[test]
    fn every_press_lands_on_its_item() {
        let fonts = Fonts::hanken();
        let qr = made_up_qr(25);
        for (st, ui) in [(listening(hans()), Phone::default()), (listening(at(Stage::DutyOrder)), Phone::default()), (listening(at(Stage::OnDuty)), Phone::default())] {
            for s in [0.95, 1.4, 2.0] {
                let p = page_in(&st, &ui, None, 0.0, Room::Column, Some(&qr));
                let w = (340.0 - 2.0 * PAD) * s;
                let (placed, height) = layout(&p.items, w, s, &fonts);
                assert_eq!(placed.len(), p.items.len());
                assert!((placed.last().map_or(0.0, |l| l.rect.bottom()) - height).abs() < 1e-3);
                let hits = hits(&p.items, &placed, s);
                for (i, (a, x)) in hits.iter().enumerate() {
                    assert!(a.x >= -0.01 && a.right() <= w + 0.01 && a.y >= 0.0 && a.bottom() <= height + 0.01, "{x:?} {a:?} at {s}");
                    for (b, y) in &hits[i + 1..] {
                        let apart = a.right() <= b.x + 0.01 || b.right() <= a.x + 0.01 || a.bottom() <= b.y + 0.01 || b.bottom() <= a.y + 0.01;
                        assert!(apart, "{x:?} {a:?} on {y:?} {b:?}");
                    }
                }
                if !st.signed_on {
                    let five = hits.iter().find(|h| h.1 == Action::Digit(5)).unwrap().0;
                    assert_eq!(hits.iter().find(|h| h.0.contains(five.center())).map(|h| h.1), Some(Action::Digit(5)));
                }
            }
        }
        assert_eq!(key_action(winit::keyboard::KeyCode::Numpad7), Some(Action::Digit(7)));
        assert_eq!(key_action(winit::keyboard::KeyCode::Digit0), Some(Action::Digit(0)));
        assert_eq!(key_action(winit::keyboard::KeyCode::Backspace), Some(Action::Erase));
        assert_eq!(key_action(winit::keyboard::KeyCode::KeyA), None);
        // long text wraps to the page
        let lines = wrap(&fonts, "Phone & tablet is off. Turn it on in the settings to have the duty and the bus's screens on a phone or tablet.", 12.5, Weight::Medium, 300.0);
        assert!(lines.len() >= 2 && lines.iter().all(|l| fonts.width(l, 12.5, Weight::Medium) <= 300.0));
    }

    /// The page draws inside its column (the items across its width; the column clips them
    /// below and above as they scroll).
    #[test]
    fn the_page_draws_within_its_column() {
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(1024);
        let (trips, tours) = crate::nav_duty::tests::duty();
        let d = crate::nav_duty::tests::state(&trips, &tours, 0, 3, false, 0.0);
        let mut menu = at(Stage::DutyMenu);
        menu.menu = DutyMenu { lines: Some(vec![("35".into(), "35 Hauptbahnhof".into())]), ..DutyMenu::default() };
        let mut wrong = Phone { wrong: true, typed: String::new(), ..Phone::default() };
        wrong.follow(&hans());
        let qr = made_up_qr(33);
        for (st, ui) in [(listening(hans()), Phone::default()), (hans(), wrong), (menu, Phone::default()), (listening(at(Stage::DutyOrder)), Phone::default()), (CompanionState { break_since: Some(42_000.0), ..listening(at(Stage::OnDuty)) }, Phone::default())] {
            for s in [0.95, 1.0, 1.6] {
                let p = page_in(&st, &ui, Some(&d), 43_000.0, Room::Column, Some(&qr));
                let column = Rect::new(12.0 * s, 56.0 * s, 340.0 * s, 4000.0 * s);
                let mut painter = Painter::new();
                let mut pen = Pen { p: &mut painter, atlas: &mut atlas, fonts: &fonts };
                let view = draw_head(&mut pen, &p, column, s);
                let (placed, height) = layout(&p.items, view.w - 2.0 * PAD * s, s, &fonts);
                assert!(height < view.h);
                draw_items(&mut pen, &p.items, &placed, Vec2::new(view.x + PAD * s, view.y + 6.0 * s), view, s);
                assert!(!painter.verts.is_empty());
                // (the shadow reaches a little beyond the column, as the duty sheet's does)
                let room = column.inset(-10.0 * s);
                for v in &painter.verts {
                    let (x, y) = (v.pos[0] + v.ext[0] * v.width[0], v.pos[1] + v.ext[1] * v.width[0]);
                    assert!(x >= room.x && x <= room.right() && y >= room.y && y <= room.bottom(), "({x}, {y}) outside {column:?} at {s}: {}", p.title);
                }
            }
        }
    }

    /// Every text of the page is in the tables for the languages that matter most, with its
    /// placeholders.
    #[test]
    fn the_page_is_translated() {
        let keys = [
            "Your pass",
            "Personnel number",
            "Code",
            "Not known here. Your number and code are on your pass below.",
            "Signed on",
            "Not signed on",
            "Tour %{tour}",
            "Driving without a duty",
            "On break",
            "No tours on this line.",
            "Phone & tablet",
            "Phone & tablet is off. Turn it on in the settings to have the duty and the bus's screens on a phone or tablet.",
            "Starting…",
            "This computer has no network address. Is it on Wi-Fi or a network cable?",
            "On a phone or tablet on the same network, open",
            "and enter the pairing code",
            "Shows %{screen}",
            "Connected",
            "No device paired yet.",
            "Paired devices: %{n}",
            "New pairing code",
            "Forget devices",
            "Scan with your phone or tablet",
            "Pairing code %{code}",
            // (the companion's and the game's, used here too)
            "Sign on",
            "Your personnel number",
            "Your code",
            "Clear",
            "Choose a duty",
            "Choose another duty",
            "Drive without a duty",
            "Duty assignment",
            "Accept duty",
            "Sign off",
            "Loading…",
            "No timetable on this map",
            "Back",
            "Line",
            "Tour",
            "Departure",
            "Back at",
            "Trips",
            "Break",
            "Start break",
            "End break",
            "Next trip",
            "Scheduled break at the terminus",
            "%{minutes} min left",
            "%{minutes} min over",
            "Personnel no. %{number} · code %{code}",
            "The phone companion could not start: %{error}",
        ];
        for language in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for key in keys {
                let t = crate::_rust_i18n_try_translate(language, key);
                assert!(t.as_ref().is_some_and(|t| !t.trim().is_empty()), "{language}: {key}");
                for var in ["%{tour}", "%{screen}", "%{n}", "%{minutes}", "%{number}", "%{code}", "%{error}"] {
                    assert_eq!(key.contains(var), t.as_ref().unwrap().contains(var), "{language}: {key}");
                }
            }
        }
        assert_eq!(crate::_rust_i18n_try_translate("nl", "Not signed on").as_deref(), Some("Niet aangemeld"));
    }

    /// Pictures of the page - the keypad with the pass and a QR code (the companion on), a
    /// wrong code, the duty menu, the duty order, at work on a break - as the city map's column;
    /// the small navigator as the page (the keypad with the devices, without them, the duty
    /// order); and the city map's whole view as the page, wide and narrow; in Dutch:
    /// `OMSI_NAV_SIGNON_PREVIEW=<folder> cargo test -p omsi-app --lib nav_signon -- --ignored`.
    #[test]
    #[ignore]
    fn preview_pictures() {
        let Ok(dir) = std::env::var("OMSI_NAV_SIGNON_PREVIEW") else { return };
        crate::ui_language("NLD");
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(2048);
        let all = Rect::new(0.0, 0.0, 1e5, 1e5);
        let s = 1.6;
        let qr = made_up_qr(29);
        let (trips, tours) = crate::nav_duty::tests::duty();
        let d = crate::nav_duty::tests::state(&trips, &tours, 0, 5, false, 40.0);
        let mut typing = Phone { typed: "48".into(), ..Phone::default() };
        typing.follow(&hans());
        let mut wrong = Phone { code_step: true, wrong: true, ..Phone::default() };
        wrong.follow(&hans());
        let mut menu = listening(at(Stage::DutyMenu));
        menu.menu = DutyMenu { lines: Some(vec![("35".into(), "35 Hauptbahnhof – Diakonissenkrankenhaus".into()), ("25".into(), "25 Eichstedt".into()), ("5E".into(), "5E Markt".into())]), ..DutyMenu::default() };
        let mut tours_ui = Phone { line: Some(0), ..Phone::default() };
        tours_ui.follow(&menu);
        let mut tours_st = menu.clone();
        tours_st.menu.tours = Some((0, vec![("Omloop 1 › Diakonissenkrankenhaus".into(), "11:26".into()), ("Omloop 2 › Hauptbahnhof".into(), "11:56".into()), ("Omloop 3 › Park".into(), "12:26".into())]));
        let on_break = CompanionState { break_since: Some(11.0 * 3600.0 + 47.0 * 60.0), ..listening(at(Stage::OnDuty)) };
        let pages = [
            (listening(hans()), typing.clone()),
            (CompanionState { error: None, ..hans() }, wrong),
            (menu, Phone::default()),
            (tours_st, tours_ui),
            (listening(at(Stage::DutyOrder)), Phone::default()),
            (on_break, Phone::default()),
        ];
        let (cw, ch) = (340.0 * s, 1160.0 * s);
        let mut img = image::RgbaImage::from_pixel(((cw + 30.0) * pages.len() as f32 + 30.0) as u32, (ch + 40.0) as u32, image::Rgba([12, 16, 26, 255]));
        for (k, (st, ui)) in pages.iter().enumerate() {
            let p = page_in(st, ui, Some(&d), 11.0 * 3600.0 + 57.0 * 60.0, Room::Column, Some(&qr));
            let column = Rect::new(30.0 + k as f32 * (cw + 30.0), 20.0, cw, ch);
            let mut head = Painter::new();
            let view = draw_head(&mut Pen { p: &mut head, atlas: &mut atlas, fonts: &fonts }, &p, column, s);
            crate::nav_duty::tests::raster(&head.verts, &atlas, &mut img, all);
            let (placed, _) = layout(&p.items, view.w - 2.0 * PAD * s, s, &fonts);
            let mut body = Painter::new();
            draw_items(&mut Pen { p: &mut body, atlas: &mut atlas, fonts: &fonts }, &p.items, &placed, Vec2::new(view.x + PAD * s, view.y + 6.0 * s), view, s);
            crate::nav_duty::tests::raster(&body.verts, &atlas, &mut img, view);
        }
        img.save(format!("{dir}/signon.png")).unwrap();

        // the small navigator as the page, over a stand-in for the cab: with the devices, in a
        // window too short for them, and the duty order (at 1.2: a 1080p window's navigator)
        let s: f32 = 1.2;
        let pw = (360.0 * s).round();
        let panels = [(listening(hans()), typing, 2000.0), (listening(hans()), Phone::default(), 560.0), (listening(at(Stage::DutyOrder)), Phone::default(), 2000.0)];
        let fitted: Vec<_> = panels.iter().map(|(st, ui, room)| fit_panel(page_in(st, ui, None, 0.0, Room::Panel, Some(&qr)), pw, s, *room, &fonts)).collect();
        let tallest = fitted.iter().map(|f| f.height).fold(0.0, f32::max);
        let mut img = image::RgbaImage::from_pixel(((pw + 30.0) * panels.len() as f32 + 30.0) as u32, (tallest + 40.0) as u32, image::Rgba([70, 84, 96, 255]));
        for (k, f) in fitted.iter().enumerate() {
            let r = Rect::new(30.0 + k as f32 * (pw + 30.0), 20.0, pw, f.height);
            let mut p = Painter::new();
            p.rounded(r, 7.0 * s, SHEET.alpha(0.94));
            draw_panel_head(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, f, r, 0.0);
            crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, r);
            let mut items = Painter::new();
            draw_panel_items(&mut Pen { p: &mut items, atlas: &mut atlas, fonts: &fonts }, f, r, 0.0);
            crate::nav_duty::tests::raster(&items.verts, &atlas, &mut img, panel_view(f, r));
        }
        img.save(format!("{dir}/panel.png")).unwrap();

        // the city map's whole view (a 1080p window's: 1536 x 886, the interface at 1.17),
        // then a narrow one that scrolls
        for (name, w, h, page_st) in [("full", 1536.0f32, 886.0f32, listening(hans())), ("full-order", 1536.0, 886.0, listening(at(Stage::DutyOrder))), ("full-narrow", 760.0, 700.0, listening(hans()))] {
            let s = (h / 760.0).clamp(0.95, 2.0);
            let mut img = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([12, 16, 26, 255]));
            let mut head = Painter::new();
            head.rect(Rect::new(0.0, 0.0, w, 44.0 * s), Color::rgba(20, 26, 38, 0.97));
            let body = Rect::new(0.0, 44.0 * s, w, h - 44.0 * s);
            let page = page_in(&page_st, &Phone { typed: "4829".into(), ..Phone::default() }, None, 0.0, Room::Full, Some(&qr));
            let f = full(&page, body, s, &fonts);
            let mut p = Painter::new();
            draw_full(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &page, &f, 0.0, body, s);
            crate::nav_duty::tests::raster(&head.verts, &atlas, &mut img, all);
            crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, body);
            img.save(format!("{dir}/{name}.png")).unwrap();
        }
    }
}
