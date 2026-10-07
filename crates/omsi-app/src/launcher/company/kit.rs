//! The company pages' shared look (Luc: "sommige tekst is erg klein en niet goed leesbaar"):
//! the sizes their words are read at, the frame every dialog has with its ways out (a cross in
//! its head, Close or Cancel in its foot, Escape), the row of buttons along a dialog's foot,
//! bars that say what they measure, and the popup that says why something cannot be done and
//! what opens it.

use super::super::theme::*;
use super::super::ui::{ButtonKind, Key, Ui};
use super::super::Launcher;
use super::eur;
use glam::Vec2;
use omsi_launcher_lib::company::{self as co, finance, levels, Company};
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

// --- sizes (at interface scale 1) ---------------------------------------------------------------

/// A dialog's title.
pub(super) const TITLE: f32 = 21.0;
/// A name or figure that leads a section.
pub(super) const HEAD: f32 = 17.0;
/// Running text.
pub(super) const BODY: f32 = 15.0;
/// A table's rows and a list's lines.
pub(super) const ROWS: f32 = 14.5;
/// What is said under or beside: notes, a row's second line.
pub(super) const NOTE: f32 = 13.5;
/// A heading in capitals.
pub(super) const CAPS: f32 = 12.5;
/// The controls' words (`Ui::widget_px` while the pages draw).
pub(super) const CONTROL: f32 = 14.0;
/// What is said in the dealer's talk.
pub(super) const CHAT: f32 = 16.0;
/// A figure card's height (`super::figure`).
pub(super) const FIGURE_H: f32 = 100.0;
/// A dialog's buttons.
pub(super) const BUTTON_H: f32 = 40.0;

/// A heading in capitals.
pub(super) fn caps(ui: &mut Ui, r: Rect, text: &str) {
    ui.text_in(&omsi_ui::tr(text).to_uppercase(), Rect::new(r.x, r.y, r.w, CAPS + 4.0), CAPS, Weight::Bold, TEXT_DIM, Align::Left);
}

/// A small coloured tag with its words; returns its width.
pub(super) fn tag(ui: &mut Ui, at: Vec2, text: &str, c: Color) -> f32 {
    let w = ui.width(text, 12.0, Weight::Bold) + 16.0;
    let r = Rect::new(at.x, at.y, w, 22.0);
    ui.p().rounded(r, 6.0, c.alpha(0.16));
    ui.text_in(text, r, 12.0, Weight::Bold, c, Align::Center);
    w
}

/// An amount with its sign and colour: what comes in green "+€894", what goes out red "−€11,175".
pub(super) fn signed(c: co::Cents) -> (String, Color) {
    if c > 0 {
        (format!("+{}", eur(c)), OK)
    } else if c < 0 {
        (format!("−{}", eur(-c)), DANGER.lighten(0.25))
    } else {
        (eur(0), TEXT_DIM)
    }
}

// --- the dialog's frame -------------------------------------------------------------------------

/// A dialog laid out: the room between its head and its foot, its foot's row, and whether it
/// is to be left (its cross clicked, or Escape).
pub(super) struct Frame {
    pub body: Rect,
    pub foot: Rect,
    pub close: bool,
}

/// A dialog's panel in the middle of the window, as wide and high as asked (the window's less
/// a margin at most): its icon and title, the cross that closes it.
pub(super) fn frame(l: &mut Launcher, w: f32, h: f32, icon: &str, title: &str) -> Frame {
    frame_with(l, w, h, icon, title, true)
}

/// `frame`, the cross left out when `closable` is false (a decision the clock waits for).
pub(super) fn frame_with(l: &mut Launcher, w: f32, h: f32, icon: &str, title: &str, closable: bool) -> Frame {
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, Color::rgba(0, 0, 0, 0.62));
    let w = (size.x - 32.0).min(w);
    let h = (size.y - 32.0).min(h);
    let r = Rect::new(((size.x - w) * 0.5).round(), ((size.y - h) * 0.5).round(), w, h);
    l.ui.p().shadow(r.inset(-2.0), SHEET_RADIUS, 26.0, Color::rgba(0, 0, 0, 0.5));
    l.ui.panel(r);
    let head_y = r.y + 34.0;
    l.ui.icon(icon, Vec2::new(r.x + 40.0, head_y), 24.0, accent_2());
    let tw = r.w - 60.0 - if closable { 64.0 } else { 28.0 };
    l.ui.text_in(title, Rect::new(r.x + 60.0, head_y - 16.0, tw, 32.0), TITLE, Weight::Bold, TEXT, Align::Left);
    let mut close = false;
    if closable {
        close = l.ui.icon_button(&format!("dialog-x-{icon}"), Vec2::new(r.right() - 36.0, head_y), 18.0, "close", "Close");
        close |= l.ui.input.keys.contains(&Key::Escape);
    }
    let foot = Rect::new(r.x + 28.0, r.bottom() - 24.0 - BUTTON_H, r.w - 56.0, BUTTON_H);
    let body = Rect::new(r.x + 28.0, r.y + 70.0, r.w - 56.0, (foot.y - 18.0 - r.y - 70.0).max(0.0));
    Frame { body, foot, close }
}

/// The buttons along a dialog's foot: from its right end leftwards (the main one first, then
/// the others, the way out last), and from its left end rightwards; each as wide as its words.
pub(super) struct Foot {
    r: Rect,
    left: f32,
    right: f32,
}

impl Foot {
    pub fn new(f: &Frame) -> Foot {
        Foot { r: f.foot, left: f.foot.x, right: f.foot.right() }
    }

    /// A button's width for its words (and icon).
    pub fn width(ui: &Ui, label: &str, icon: Option<&str>) -> f32 {
        (ui.width(label, CONTROL, Weight::Bold) + if icon.is_some() { CONTROL * 1.3 + 6.0 } else { 0.0 } + 44.0).max(116.0)
    }

    /// A button at the right; returns whether it was pressed.
    pub fn right(&mut self, l: &mut Launcher, name: &str, label: &str, icon: Option<&str>, kind: ButtonKind) -> bool {
        let w = Foot::width(&l.ui, label, icon);
        let b = Rect::new(self.right - w, self.r.y, w, self.r.h);
        self.right = b.x - 10.0;
        l.ui.button(name, b, label, icon, kind)
    }

    /// A button at the left; returns whether it was pressed.
    pub fn left(&mut self, l: &mut Launcher, name: &str, label: &str, icon: Option<&str>, kind: ButtonKind) -> bool {
        let w = Foot::width(&l.ui, label, icon);
        let b = Rect::new(self.left, self.r.y, w, self.r.h);
        self.left = b.right() + 10.0;
        l.ui.button(name, b, label, icon, kind)
    }

    /// The room left between the buttons (for a short note).
    pub fn rest(&self) -> Rect {
        Rect::new(self.left + 4.0, self.r.y, (self.right - self.left - 8.0).max(0.0), self.r.h)
    }
}

// --- bars that say what they measure --------------------------------------------------------------

/// What a share's colour says: all of it green, part amber, none red; grey when it does not
/// count (`idle`: a line not planned yet).
pub(super) fn share_colour(frac: f64, idle: bool) -> Color {
    if idle {
        TEXT_FAINT
    } else if frac >= 0.999 {
        OK
    } else if frac > 0.0 {
        WARN
    } else {
        DANGER.lighten(0.15)
    }
}

/// A bar with its words over it ("Covered today: 1 of 3 tours · 33 %"), its tooltip with the
/// details; returns whether it was clicked (`name` empty: not clickable).
#[allow(clippy::too_many_arguments)]
pub(super) fn bar(ui: &mut Ui, name: &str, r: Rect, frac: f64, c: Color, label: &str, right: &str, tip: &str) -> bool {
    let line_h = NOTE + 6.0;
    let clicked = if name.is_empty() {
        false
    } else {
        let (h, _, clicked) = ui.interact(super::super::ui::id_of(name), r);
        if h {
            ui.p().rounded(r.inset(-4.0), 6.0, Color::WHITE.alpha(0.04));
            ui.cursor = winit::window::CursorIcon::Pointer;
        }
        clicked
    };
    ui.text_in(label, Rect::new(r.x, r.y, r.w * 0.72, line_h), NOTE, Weight::Medium, TEXT_SOFT, Align::Left);
    if !right.is_empty() {
        ui.text_in(right, Rect::new(r.x + r.w * 0.5, r.y, r.w * 0.5, line_h), NOTE, Weight::Bold, c, Align::Right);
    }
    let track = Rect::new(r.x, r.y + line_h + 3.0, r.w, 6.0);
    super::meter(ui, track, frac, c);
    if !tip.is_empty() {
        ui.tooltip(r, tip);
    }
    clicked
}

/// How high `bar` is.
pub(super) const BAR_H: f32 = NOTE + 6.0 + 3.0 + 6.0;

// --- the popup for what cannot be done --------------------------------------------------------------

/// Where a popup's second button goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Go {
    /// The career's progress: the company's level and what it opens.
    Progress,
    /// The depot, to build.
    Depot,
    /// A loan's dialog.
    Loan,
    Finances,
    /// The career's training courses.
    Courses,
    Planning,
    Lines,
    Dealer,
    /// The staff's applicants.
    Hire,
    /// The staff's courses on the Staff page.
    Training,
    /// The licences and type trainings on the Staff page.
    Licences,
}

impl Go {
    pub fn label(self) -> &'static str {
        match self {
            Go::Progress => "Show my progress",
            Go::Depot => "Build parking spaces",
            Go::Loan => "Take a loan",
            Go::Finances => "Show the finances",
            Go::Courses => "To the courses",
            Go::Planning => "Open the planning",
            Go::Lines => "To the lines",
            Go::Dealer => "To the dealer",
            Go::Hire => "Hire drivers",
            Go::Training => "Train the drivers",
            Go::Licences => "Train drivers",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Go::Progress => "emoji_events",
            Go::Depot => "garage",
            Go::Loan | Go::Finances => "payments",
            Go::Courses => "badge",
            Go::Planning => "event",
            Go::Lines => "route",
            Go::Dealer => "directions_bus",
            Go::Hire => "groups",
            Go::Training => "badge",
            Go::Licences => "key",
        }
    }
}

/// Something that cannot be done, said plainly: what, why, and what opens it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Popup {
    pub icon: &'static str,
    pub title: String,
    pub text: String,
    /// What opens it (empty: nothing to say).
    pub unlock: String,
    pub go: Option<Go>,
}

impl Popup {
    pub fn new(icon: &'static str, title: &str, text: impl Into<String>, unlock: impl Into<String>, go: Option<Go>) -> Popup {
        Popup { icon, title: omsi_ui::tr(title).into_owned(), text: text.into(), unlock: unlock.into(), go }
    }
}

/// The company's level in words: "you are level 1 (430 of 600 points)".
fn level_words(c: &Company) -> String {
    let lv = levels::level(c);
    let (xp, _, next) = levels::progress(c);
    match next {
        Some(n) => omsi_ui::tr("Your company is level %{lv} (%{xp} of %{next} points).").replace("%{lv}", &lv.to_string()).replace("%{xp}", &super::grouped(xp as f64)).replace("%{next}", &super::grouped(n as f64)),
        None => omsi_ui::tr("Your company is level %{lv}.").replace("%{lv}", &lv.to_string()),
    }
}

/// A feature the company's level does not open yet.
pub(super) fn locked(c: &Company, f: levels::Feature) -> Popup {
    let text = omsi_ui::tr("%{what} open at company level %{n}.").replace("%{what}", &omsi_ui::tr(f.label())).replace("%{n}", &f.level().to_string());
    Popup::new(
        "lock",
        "Not open yet",
        format!("{text} {}", level_words(c)),
        omsi_ui::tr("The company earns points with every day closed: tours run, trips on time, passengers carried and a good reputation."),
        Some(Go::Progress),
    )
}

/// Not enough cash for `amount` (0: not known).
pub(super) fn no_cash(c: &Company, amount: co::Cents) -> Popup {
    let mut text = omsi_ui::tr("The company has %{cash} in cash.").replace("%{cash}", &eur(c.cash));
    if amount > 0 {
        text.push(' ');
        text.push_str(&omsi_ui::tr("This costs %{amount}: %{short} more than there is.").replace("%{amount}", &eur(amount)).replace("%{short}", &eur((amount - c.cash).max(0))));
    }
    let room = finance::credit(c, 0).left;
    let (unlock, go) = if room >= finance::SMALLEST_LOAN {
        (omsi_ui::tr("The bank lends up to %{amount} more. Or sell a bus, or wait for the month's income.").replace("%{amount}", &eur(room)), Go::Loan)
    } else {
        (omsi_ui::tr("The bank lends nothing more now: sell a bus, or wait for the month's income.").into_owned(), Go::Finances)
    };
    Popup::new("payments", "Not enough cash", text, unlock, Some(go))
}

/// The bank does not lend `amount` (0: not known).
pub(super) fn no_credit(c: &Company, amount: co::Cents) -> Popup {
    let cr = finance::credit(c, 0);
    let mut text = omsi_ui::tr("The bank lends at most %{left} more now (%{debt} of %{limit} used).").replace("%{left}", &eur(cr.left)).replace("%{debt}", &eur(cr.debt)).replace("%{limit}", &eur(cr.limit));
    if amount > 0 {
        text = format!("{}  {}", omsi_ui::tr("You asked for %{amount}.").replace("%{amount}", &eur(amount)), text);
    }
    Popup::new("payments", "The bank says no", text, omsi_ui::tr("Pay loans back, or let the fleet and the results grow: the credit room grows with them."), Some(Go::Finances))
}

/// What a refusal of the company's rules (`act`) means, and what opens it.
pub(super) fn refusal(c: &Company, reason: &str) -> Popup {
    use levels::Feature;
    let said = omsi_ui::tr(reason).into_owned();
    match reason {
        "Articulated buses open at a higher company level." => locked(c, Feature::ArticulatedBuses),
        "Double-deckers open at a higher company level." => locked(c, Feature::DoubleDeckers),
        "Rear adverts open at a higher company level." => locked(c, Feature::RearAdverts),
        "Side adverts open at a higher company level." => locked(c, Feature::SideAdverts),
        "Full wraps open at a higher company level." => locked(c, Feature::FullWraps),
        "Electric buses open at a higher company level." => locked(c, Feature::ElectricBuses),
        "Your company cannot build this yet." | "Your company's level does not offer this course yet." => Popup::new("lock", "Not open yet", format!("{said} {}", level_words(c)), omsi_ui::tr("The company earns points with every day closed: tours run, trips on time, passengers carried and a good reputation."), Some(Go::Progress)),
        "Not enough cash." | "Not enough money for the course." => no_cash(c, 0),
        "The bank does not lend that much." => no_credit(c, 0),
        "The company cannot pay for the line: take a loan on the Finances page, or make it smaller." | "The company cannot pay for the change: take a loan on the Finances page." => {
            let p = no_cash(c, 0);
            Popup { text: format!("{said} {}", p.text), ..p }
        }
        "The depot has no room for another bus: build more parking spaces." | "The depot has no room for so many buses: build more parking spaces." => {
            let held = c.fleet.iter().filter(|v| v.held_on(&c.date)).count();
            let text = omsi_ui::tr("The depot has %{spaces} parking spaces and holds %{buses} buses; a few more may stand in the street.").replace("%{spaces}", &c.site.spaces().to_string()).replace("%{buses}", &held.to_string());
            Popup::new("garage", "No room in the depot", text, omsi_ui::tr("Build more parking spaces at the depot: each level gives twelve."), Some(Go::Depot))
        }
        "You need the repairs course first." => Popup::new("badge", "A course first", said, omsi_ui::tr("Book the repairs course under Training."), Some(Go::Courses)),
        "Your driver level is too low for this test." => Popup::new("badge", "Not yet", said, omsi_ui::tr("Your driver level grows with every trip you drive to a timetable."), Some(Go::Progress)),
        "Sign the contract first." => Popup::new("livery_pen", "Not signed yet", omsi_ui::tr("Sign in the field with the mouse, or type your name beside it."), "", None),
        "This tender is closed." | "This tender has not opened yet." | "Another bid leads with more: bid at least the least shown." | "That is the buy-out price: buy the line instead." | "There is no such tender." => Popup::new("receipt_long", "The bid cannot be placed", said, "", None),
        "The dealer does not want to talk to you for now." => Popup::new("forum", "The dealer will not talk", said, omsi_ui::tr("Come back another day - or buy at the list price."), None),
        "This timetable carries no passengers of its own: depot runs, empty runs or other traffic." => Popup::new(
            "route",
            "Not a line with passengers",
            said,
            omsi_ui::tr("Depot and empty runs are part of the tours of the lines that start or end there: take those lines on and these runs are driven with them, as empty kilometres. Choose a line with passengers in the list."),
            None,
        ),
        _ => Popup::new("error", "This cannot be done", said, "", None),
    }
}

/// Show a popup over the company's pages.
pub(super) fn show(l: &mut Launcher, p: Popup) {
    l.company.popup = Some(p);
}

/// Say a refusal of the company's rules in a popup.
pub(super) fn refuse(l: &mut Launcher, reason: &str) {
    let p = match l.company.company.as_ref() {
        Some(c) => refusal(c, reason),
        None => Popup::new("error", "This cannot be done", omsi_ui::tr(reason).into_owned(), "", None),
    };
    show(l, p);
}

/// The popup, over everything: its icon, title, why, what opens it; OK and where to go.
pub(super) fn draw_popup(l: &mut Launcher) {
    let Some(p) = l.company.popup.clone() else { return };
    let w = 560.0f32.min(l.ui.size.x - 32.0);
    // (the body's width, as the frame lays it out)
    let tw = w - 56.0;
    let text_h = l.ui.paragraph_height(&p.text, tw, BODY, Weight::Regular);
    let unlock_h = if p.unlock.is_empty() { 0.0 } else { l.ui.paragraph_height(&p.unlock, tw - 52.0, NOTE + 0.5, Weight::Medium) + 26.0 };
    let h = 70.0 + text_h + 16.0 + unlock_h + 10.0 + BUTTON_H + 26.0;
    let f = frame(l, w, h, p.icon, &p.title);
    let x = f.body.x;
    let mut y = f.body.y;
    y += l.ui.paragraph(&p.text, Vec2::new(x, y), f.body.w, BODY, Weight::Regular, TEXT_SOFT) + 16.0;
    if !p.unlock.is_empty() {
        let bh = unlock_h - 8.0;
        let r = Rect::new(x, y, f.body.w, bh);
        l.ui.p().rounded(r, RADIUS, accent_2().alpha(0.12));
        l.ui.p().rounded(Rect::new(r.x, r.y, 4.0, r.h), 2.0, accent_2());
        l.ui.icon("key", Vec2::new(r.x + 22.0, r.y + 21.0), 18.0, accent_2());
        l.ui.paragraph(&p.unlock, Vec2::new(r.x + 40.0, r.y + 9.0), r.w - 52.0, NOTE + 0.5, Weight::Medium, TEXT);
    }
    let mut foot = Foot::new(&f);
    let mut done = f.close || l.ui.input.keys.contains(&Key::Enter);
    let mut go = None;
    if let Some(g) = p.go {
        if foot.right(l, "company-popup-go", g.label(), Some(g.icon()), ButtonKind::Primary) {
            go = Some(g);
        }
        done |= foot.right(l, "company-popup-ok", "OK", None, ButtonKind::Normal);
    } else {
        done |= foot.right(l, "company-popup-ok", "OK", None, ButtonKind::Primary);
    }
    if done || go.is_some() {
        l.company.popup = None;
    }
    if let Some(g) = go {
        super::go(l, g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_launcher_lib::company::{found, Difficulty, Founding};

    #[test]
    fn a_refusal_says_why_and_what_opens_it() {
        let c = found(&Founding { name: "Kit".into(), difficulty: Difficulty::Realistic, date: "2024-03-04".into(), ..Default::default() }, "Luc");
        let p = refusal(&c, "Articulated buses open at a higher company level.");
        assert_eq!(p.go, Some(Go::Progress));
        assert!(p.text.contains("level 2") && p.text.contains("of 600 points"), "{}", p.text);
        let p = refusal(&c, "The depot has no room for another bus: build more parking spaces.");
        assert_eq!(p.go, Some(Go::Depot));
        let p = no_cash(&c, c.cash + 100_00);
        assert!(p.text.contains(&eur(100_00)) && p.go == Some(Go::Loan));
        assert_eq!(refusal(&c, "Something new.").go, None);
        assert_eq!(signed(894_00).0, format!("+{}", eur(894_00)));
        assert!(signed(-5_00).0.starts_with('−'));
    }
}
