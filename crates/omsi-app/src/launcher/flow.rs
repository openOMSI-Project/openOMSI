//! The Drive page laid out as Omsi-Hub lays out its setup ("the windscreen map"): the ground
//! is the map - on the bus step the bus - edge to edge, with nothing framing it; a step bar
//! runs along the top; each step's choice floats on the ground as a sheet on the left; and
//! the action that goes on stands in the bottom right corner, the same place on every step,
//! so that it is hit without being looked for. The launcher's other pages open in a wide
//! sheet under the same bar.
//!
//! The steps are the order a driver thinks in: who drives, how (a shift, a tour, or free),
//! where, on which day, what, and with which bus.

use super::drive;
use super::shiftsheet;
use super::state::hhmm;
use super::theme::*;
use super::ui::{id_of, ButtonKind};
use super::{Launcher, Page};
use glam::Vec2;
use omsi_ui::paint::{Align, Path};
use omsi_ui::{Atlas, Color, Fonts, Painter, Rect, Vertex, Weight};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Step {
    Profile,
    #[default]
    Mode,
    Map,
    /// Free drive only: where the bus starts, or the line it follows (`freedrive`).
    Start,
    Day,
    Duty,
    Bus,
}

/// The steps in the bar: name and icon.
const STEPS: [(Step, &str, &str); 7] = [
    (Step::Profile, "Profile", "person"),
    (Step::Mode, "Mode", "route"),
    (Step::Map, "Map", "map"),
    (Step::Start, "Start point", "location_on"),
    (Step::Day, "Day & weather", "partly_cloudy_day"),
    (Step::Duty, "Duty", "schedule"),
    (Step::Bus, "Bus", "directions_bus"),
];

/// The margin round the window's edge, the bar's height, where the sheets begin, and the
/// main action's size and place (from the window's bottom right corner).
pub(super) const EDGE_IN: f32 = 22.0;
const BAR_H: f32 = 40.0;
pub(super) const SHEET_TOP: f32 = 85.0;
pub(super) const ACTION_H: f32 = 62.0;
pub(super) const ACTION_BOTTOM: f32 = 31.0;
const ACTION_RIGHT: f32 = 24.0;

/// The steps a mode goes through: a free drive has no duty to choose but a place to start
/// (or a line to follow); a duty starts where its first trip does.
fn steps_of(l: &Launcher) -> Vec<(Step, &'static str, &'static str)> {
    let free = l.state.choice.free;
    STEPS.iter().copied().filter(|(s, _, _)| if free { *s != Step::Duty } else { *s != Step::Start }).collect()
}

pub(super) fn next_of(l: &Launcher, s: Step) -> Option<Step> {
    let steps = steps_of(l);
    let k = steps.iter().position(|x| x.0 == s)?;
    steps.get(k + 1).map(|x| x.0)
}

pub(super) fn previous_of(l: &Launcher, s: Step) -> Option<Step> {
    let steps = steps_of(l);
    let k = steps.iter().position(|x| x.0 == s)?;
    k.checked_sub(1).map(|k| steps[k].0)
}

/// The narrow sheet a step's choice lies on, left on the ground.
pub(super) fn sheet_rect(size: Vec2) -> Rect {
    let w = (size.x * 0.274).clamp(288.0, 420.0);
    Rect::new(EDGE_IN, SHEET_TOP, w, (size.y - SHEET_TOP - 30.0).max(200.0))
}

/// The wide sheet of the steps without a map under them (driver, mode) and of the other
/// pages: the window's width, the action row under it.
pub(super) fn wide_rect(size: Vec2, actions: bool) -> Rect {
    let bottom = if actions { ACTION_BOTTOM + ACTION_H + 24.0 } else { 22.0 };
    Rect::new(EDGE_IN, SHEET_TOP, size.x - 2.0 * EDGE_IN, (size.y - SHEET_TOP - bottom).max(200.0))
}

/// The widest the start's card and a page's sheet grow: on a large screen they stay in the
/// middle at this width, as Omsi-Hub's do, instead of stretching their rows across it.
const CARD_MAX_W: f32 = 1720.0;
const PAGE_MAX_W: f32 = 1480.0;

/// `r` no wider than `max`, in the middle of where it was.
fn centred(r: Rect, max: f32) -> Rect {
    let w = r.w.min(max);
    Rect::new(r.x + (r.w - w) * 0.5, r.y, w, r.h)
}

/// The main action in the bottom right corner.
fn action_rect(size: Vec2) -> Rect {
    let w = (size.x * 0.153).clamp(150.0, 240.0);
    Rect::new(size.x - ACTION_RIGHT - w, size.y - ACTION_BOTTOM - ACTION_H, w, ACTION_H)
}

/// A sheet on the ground: the one shadow, the one radius.
pub(super) fn sheet(l: &mut Launcher, r: Rect) {
    l.ui.p().shadow(r.inset(-2.0), SHEET_RADIUS, 24.0, Color::rgba(0, 0, 0, 0.45));
    l.ui.panel(r);
}

/// The start's card as glass, after Apple's liquid glass (Luc): a clear pane over the ground
/// - the route runs on under it unbroken - frosted white under dark words (or, dark, smoked
/// under white ones); light caught in its thickness all round its edge, a rim with the
/// light's glints at the upper left and the lower right, and a glow round it in the accent.
/// (A dark, smoky pane was tried first, then the ground magnified under it and a shadow under
/// it: Luc found them ugly - the route jumped at the pane's edge, the shadow showed through.)
pub(super) fn glass_sheet(l: &mut Launcher, r: Rect) {
    glass(l, r, SHEET_RADIUS, true);
}

/// A pane of that glass with corners of `radius` (the start's card, the top bar); `glow`:
/// with the accent's glow round it (not the bar).
fn glass(l: &mut Launcher, r: Rect, radius: f32, glow: bool) {
    let g = glass_look();
    l.ui.solid(r);
    let p = l.ui.p();
    // the glow: rings outside the pane, fading outwards (none under it: through the glass it
    // showed as a dark band inside its edge)
    if glow {
        let n = 20;
        let reach = 30.0;
        for i in 0..n {
            let t = i as f32 / n as f32;
            let d = reach * t;
            p.rounded_border(r.inset(-d - 1.0), radius + d + 1.0, reach / n as f32 + 0.5, accent().alpha(g.glow * (1.0 - t).powi(3)));
        }
    }
    p.rounded(r, radius, g.tint);
    // (the light the pane catches: white on the light glass, the accent on the dark one)
    let light = if g.shine_accent { accent() } else { Color::WHITE };
    // (the bar, without the glow, quieter still)
    let shine = if glow { g.shine } else { g.shine * 0.5 };
    let white = |a: f32| light.alpha(a * shine);
    // the sheen over the top, gone a little under half way down (none on the dark glass: a
    // white haze there)
    if g.sheen > 0.0 {
        let top = Rect::new(r.x, r.y, r.w, r.h * 0.45);
        p.rounded_gradient(top, radius, Color::rgba(255, 255, 255, 0.30 * g.sheen), Color::rgba(255, 255, 255, 0.0));
    }
    // the pane's thickness: light caught inside its edge all round, brightest at the rim
    let band = (r.h * 0.25).min(16.0);
    let n = 10;
    for i in 0..n {
        let d = band * i as f32 / n as f32;
        let a = 0.30 * (1.0 - i as f32 / n as f32).powi(2);
        p.rounded_border(r.inset(d), (radius - d).max(2.0), band / n as f32 + 0.5, white(a));
    }
    // the rim: a faint line outside it against the ground, a bright one on it
    p.rounded_border(r.inset(-1.0), radius + 1.0, 1.0, g.outline);
    p.rounded_border(r, radius, 1.5, white(0.85));
    // the light's glints: along the top and down the left, weaker along the bottom and up the
    // right (the light through the pane, caught on its far edge)
    let rad = radius;
    let mut glint = |a: Vec2, b: Vec2, w: f32, alpha: f32| p.stroke(&[a, b], w, white(alpha));
    glint(Vec2::new(r.x + rad, r.y + 1.5), Vec2::new(r.x + r.w * 0.6, r.y + 1.5), 2.0, 0.95);
    glint(Vec2::new(r.x + 1.5, r.y + rad), Vec2::new(r.x + 1.5, r.y + r.h * 0.45), 2.0, 0.80);
    glint(Vec2::new(r.x + r.w * 0.45, r.bottom() - 1.5), Vec2::new(r.right() - rad, r.bottom() - 1.5), 1.5, 0.70);
    glint(Vec2::new(r.right() - 1.5, r.y + r.h * 0.55), Vec2::new(r.right() - 1.5, r.bottom() - rad), 1.5, 0.60);
}

/// The glass's colours: light - a white frost, light enough to show the ground and thick
/// enough for dark words - or, with the setting "dark mode", dark - smoked glass under white
/// words, on the night ground.
pub(super) struct Glass {
    tint: Color,
    /// What lies on the glass (the record, the links), more under the mouse, and its rim.
    on: Color,
    on_hover: Color,
    edge: Color,
    /// The words on the glass: the ink, softer, quieter and quietest.
    pub(super) ink: Color,
    pub(super) ink_soft: Color,
    ink_dim: Color,
    ink_faint: Color,
    /// What the glass looks like behind the small logo's windscreen and lamps.
    ground: Color,
    /// The glow round the pane, in the accent: how strong at its edge.
    glow: f32,
    /// The faint line round the pane, against the ground.
    outline: Color,
    /// How strongly the light catches the pane (the rims, the glints), and whether in the
    /// accent rather than white; the sheen over its top.
    shine: f32,
    shine_accent: bool,
    sheen: f32,
}

/// The glass as the dark mode has it now.
pub(super) fn glass_look() -> Glass {
    if crate::accent::dark() {
        Glass {
            tint: Color::rgba(6, 12, 30, 0.55),
            on: Color::rgba(255, 255, 255, 0.05),
            on_hover: Color::rgba(255, 255, 255, 0.10),
            edge: Color::rgba(255, 255, 255, 0.07),
            ink: TEXT,
            ink_soft: TEXT_SOFT,
            ink_dim: TEXT_DIM,
            ink_faint: TEXT_FAINT,
            ground: Color::rgba(22, 32, 60, 1.0),
            // (low: blended in linear light, a little of the accent on the night blue shows
            // far brighter than its share)
            glow: 0.045,
            outline: Color::rgba(0, 0, 0, 0.35),
            // (white rims, glints and sheen on the dark pane looked harsh, a white haze (Luc):
            // the rims in the accent instead, no sheen)
            shine: 0.10,
            shine_accent: true,
            sheen: 0.0,
        }
    } else {
        Glass {
            tint: Color::rgba(255, 255, 255, 0.40),
            on: Color::rgba(255, 255, 255, 0.38),
            on_hover: Color::rgba(255, 255, 255, 0.62),
            edge: Color::rgba(255, 255, 255, 0.85),
            ink: Color::rgba(32, 30, 36, 1.0),
            ink_soft: Color::rgba(72, 68, 74, 1.0),
            ink_dim: Color::rgba(108, 102, 104, 1.0),
            ink_faint: Color::rgba(150, 142, 140, 1.0),
            ground: Color::rgba(250, 244, 236, 1.0),
            glow: 0.10,
            outline: Color::rgba(120, 80, 30, 0.14),
            shine: 1.0,
            shine_accent: false,
            sheen: 1.0,
        }
    }
}

/// A sheet's head: its icon, its title and the line under it. Returns the rest of the sheet.
pub(super) fn sheet_head(l: &mut Launcher, r: Rect, icon: &str, title: &str, sub: &str) -> Rect {
    let x = r.x + 20.0;
    l.ui.icon(icon, Vec2::new(x + 11.0, r.y + 34.0), 22.0, TEXT);
    l.ui.text_in(title, Rect::new(x + 32.0, r.y + 18.0, r.w - 72.0, 32.0), 26.0, Weight::Bold, TEXT, Align::Left);
    if !sub.is_empty() {
        l.ui.text_in(sub, Rect::new(x + 32.0, r.y + 50.0, r.w - 72.0, 18.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    Rect::new(r.x, r.y + 80.0, r.w, (r.h - 80.0).max(0.0))
}

/// A sheet's foot: a hairline and a quiet line with an info mark. Returns the space above it.
pub(super) fn sheet_foot(l: &mut Launcher, r: Rect, text: &str) -> Rect {
    let y = r.bottom() - 46.0;
    l.ui.p().rect(Rect::new(r.x, y, r.w, 1.0), HAIRLINE);
    l.ui.icon("info", Vec2::new(r.x + 26.0, y + 23.0), 14.0, TEXT_DIM);
    l.ui.text_in(text, Rect::new(r.x + 42.0, y, r.w - 60.0, 46.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    Rect::new(r.x, r.y, r.w, (y - r.y).max(0.0))
}

/// The yellow plate of a line number.
pub(super) fn line_plate(l: &mut Launcher, at: Vec2, line: &str, h: f32) -> f32 {
    let px = h * 0.6;
    let w = (l.ui.width(line, px, Weight::Black) + h * 0.8).max(h * 2.2);
    let r = Rect::new(at.x, at.y, w, h);
    l.ui.p().rounded(r, 6.0, LINE);
    l.ui.text_in(line, r, px, Weight::Black, ON_LINE, Align::Center);
    w
}

/// The line the bar's plate shows: the chosen tour's, or the composed duty's first.
fn chosen_line(l: &Launcher) -> Option<String> {
    if l.state.choice.free {
        return super::freedrive::chosen_plate(l);
    }
    if l.state.choice.composed {
        let leg = l.state.composed_legs()?.first()?.clone();
        let (line, tour, first, _) = omsi_launcher_lib::compose::Block::parse(&leg)?;
        let t = l.state.lines.iter().find(|x| x.name == line)?.tours.iter().find(|t| t.number == tour)?;
        let trip = t.trips.iter().find(|x| x.index == first)?;
        return Some(if trip.line.is_empty() { line } else { trip.line.clone() });
    }
    let line = l.state.choice.line.clone()?;
    let trip = l.state.tour().and_then(|t| t.trips.get(l.state.first_trip().unwrap_or(0)));
    Some(trip.map(|t| t.line.clone()).filter(|x| !x.is_empty()).unwrap_or(line))
}

/// The languages the bar has a flag for (the settings' codes, as Omsi-Hub has its four); the
/// rest of openOMSI's languages are in the list beside them.
pub(super) const FLAGS: [&str; 4] = ["ENG", "DEU", "FRA", "NLD"];

/// The step a page hangs under in the bar, as Omsi-Hub hangs its pages: the service record
/// under the driver, everything else under the mode, where its way in is.
fn step_of(page: Page) -> Step {
    if page == Page::Profile {
        Step::Profile
    } else {
        Step::Mode
    }
}

/// The bar along the top. On the start (the mode, where the launcher opens): the brand and
/// the version. On a step: the line's plate and the steps; on a page: the steps with the one
/// it hangs under, and the page's name after them. On the right always: home, the settings,
/// the running games, the version, and the languages as flags, the one spoken framed - the
/// other languages in a list beside them.
fn bar(l: &mut Launcher, step: Option<Step>, page_title: Option<&str>) {
    let size = l.ui.size;
    let r = Rect::new(EDGE_IN, EDGE_IN, size.x - 2.0 * EDGE_IN, BAR_H);
    // (glass, as the start's card)
    glass(l, r, RADIUS, false);
    let on_page = l.page != Page::Drive;
    let start = !on_page && step == Some(Step::Mode);
    // (a page reached from the start's buttons - Settings, Mods, Controls... - is no step of
    // the setup: its bar holds only the brand and the ways round, not the steps)
    let plain = start || on_page;
    let version = crate::startup::VERSION;
    // right: the languages, the version, the running games, the settings, home
    let mut rx = languages(l, r, r.right() - 10.0) - 12.0;
    if !plain {
        let vw = l.ui.width(version, 11.5, Weight::Regular);
        l.ui.text_in(version, Rect::new(rx - vw, r.y, vw, r.h), 11.5, Weight::Regular, glass_look().ink_dim, Align::Left);
        rx -= vw + 14.0;
    }
    let running = l.state.instances.iter().filter(|i| i.running).count();
    let mut icons: Vec<(&str, &str, Page)> = Vec::new();
    if running > 0 {
        icons.push(("sports_esports", "Sessions", Page::Sessions));
    }
    icons.push(("settings", "Settings", Page::Settings));
    // (the guided tour, `tour`, again)
    icons.push(("help", "Show the tour", Page::Drive));
    if !start {
        icons.push(("home", "Home", Page::Drive));
    }
    for (icon, tip, page) in icons {
        let hit = Rect::new(rx - 28.0, r.center().y - 13.0, 28.0, 26.0);
        let (h, _, clicked) = l.ui.interact(id_of(&format!("bar-{icon}")), hit);
        let help = icon == "help";
        let on = (l.page == page && page != Page::Drive) || (help && super::tour::active(l));
        if on {
            l.ui.p().rounded_border(hit, 6.0, 1.0, accent());
        }
        l.ui.icon(icon, hit.center(), 17.0, if on || h { glass_look().ink } else { glass_look().ink_soft });
        l.ui.tooltip(hit, tip);
        if help {
            super::tour::anchor("bar-help", hit);
        }
        if clicked && help {
            super::tour::start(l);
        } else if clicked {
            if page == Page::Drive {
                l.drive.step = Step::Mode;
            }
            l.go(page);
        }
        if page == Page::Sessions {
            l.ui.text_in(&running.to_string(), Rect::new(hit.right() - 8.0, r.y + 4.0, 14.0, 12.0), 10.0, Weight::Bold, OK, Align::Left);
        }
        rx = hit.x - 6.0;
        // (beside the settings: the palette, the accent colour chosen in a popover)
        if icon == "settings" {
            let hit = Rect::new(rx - 28.0, r.center().y - 13.0, 28.0, 26.0);
            super::accent_pick::bar_button(l, hit);
            rx = hit.x - 6.0;
        }
    }
    // left: the logo on the start and on a page, else the plate and the steps
    let mut x = r.x + 18.0;
    if plain {
        let (verts, at) = brand_inked(&mut l.ui.atlas, &l.ui.fonts, l.ui.scale, x - 4.0, r.center().y, BAR_BRAND_H, glass_look().ground, glass_look().ink);
        l.ui.p().verts.extend(verts);
        l.ui.text_in(version, Rect::new(at.right() + 12.0, r.y, 200.0, r.h), 11.5, Weight::Regular, glass_look().ink_dim, Align::Left);
        return;
    }
    match chosen_line(l).filter(|_| !on_page) {
        Some(line) => x += line_plate(l, Vec2::new(x, r.y + 8.0), &line, 24.0) + 22.0,
        None => x += 18.0,
    }
    let avail = (rx - 10.0 - x).max(0.0);
    let now = step.unwrap_or(Step::Mode);
    let end = step_list(l, Rect::new(x, r.y, avail, r.h), now);
    if let Some(title) = page_title.filter(|_| on_page) {
        // where one is: the page after the step it hangs under
        let word = omsi_ui::tr(title).to_uppercase();
        let room = (rx - 10.0 - end - 26.0).max(0.0);
        if room > 40.0 {
            l.ui.icon("chevron_right", Vec2::new(end + 9.0, r.center().y), 16.0, glass_look().ink_dim);
            l.ui.text_in(&word, Rect::new(end + 22.0, r.y, room, r.h), 12.0, Weight::Bold, glass_look().ink, Align::Left);
        }
    }
}

// --- the logo, small --------------------------------------------------------------------

/// How tall the bar's logo's ring is (pixels): with its line and stops it stands in the bar.
pub(super) const BAR_BRAND_H: f32 = 28.0;

/// The logo's colours (`assets/logos`): the ring and the line, the bus's and "open"'s ink,
/// "OMSI"'s orange, and the stops' - each on a white rim: the first red, the Haltestelle
/// yellow with green, the terminus grey with a teal smile.
const BRAND_INK: Color = Color::hex(0xEEF1F5);
const BRAND_ORANGE: Color = Color::hex(0xF58620);
const BRAND_RIM: Color = Color::WHITE;
const BRAND_RED: Color = Color::hex(0xE43334);
const BRAND_YELLOW: Color = Color::hex(0xF6D60D);
const BRAND_GREEN: Color = Color::hex(0x0F7E3A);
const BRAND_GREY: Color = Color::hex(0x464649);
const BRAND_TEAL: Color = Color::hex(0x14C6BC);
/// How far "OMSI" leans: the tangent of its slant (about 15 degrees).
const BRAND_SLANT: f32 = 0.26;
/// Where the ring ends at its lower right (radians clockwise from its right): the gap the
/// line leaves it through runs from there round to its foot.
const BRAND_RING_END: f32 = 0.436;

/// The small logo laid out round its ring's centre, in pixels: the logo's own measures
/// (`assets/logos/openomsi-wordmark-*.svg`, whose ring's radius is 167) as shares of the ring.
#[derive(Clone, Debug, PartialEq)]
struct Brand {
    /// The ring's radius (to the middle of its band) and its band's width - the line's too.
    r: f32,
    ring_w: f32,
    /// Each word's pen position, baseline and size.
    open: (f32, f32, f32),
    omsi: (f32, f32, f32),
    /// The line runs from the ring's foot to `line_end`; the stops on it (x) and their rims'
    /// radius.
    line_end: f32,
    stops: [f32; 3],
    stop_r: f32,
    /// What all of it covers.
    bounds: Rect,
}

impl Brand {
    /// The logo with its ring `h` pixels tall. Its words are sized by their capitals, so a
    /// launcher in another typeface (the classic one's Roboto) gets them as large.
    fn new(fonts: &Fonts, h: f32) -> Brand {
        // (the band at least two pixels: thinner it greys out)
        let ring_w = (h * 0.5 / 1.1377 * 0.275).max(2.0);
        let r = (h - ring_w) * 0.5;
        // the logo's "open" is x-high 0.647 of the ring, "OMSI" cap-high 1.018: in Hanken
        // Grotesk the capitals of "open"'s size stand 0.915 of the ring
        let cap_of = |w: Weight| fonts.cap_height(100.0, w).max(1.0) / 100.0;
        let open_px = r * 0.915 / cap_of(Weight::Bold);
        let omsi_px = r * 1.018 / cap_of(Weight::Black);
        let open_x = r * 1.473;
        let open_w = fonts.width_as_is("open", open_px, Weight::Bold);
        // (the logo's gap is 0.168 of the ring; the leaning "O" opens it a little more here)
        let omsi_x = open_x + open_w + r * 0.09;
        let omsi_w = fonts.width_as_is("OMSI", omsi_px, Weight::Black);
        let omsi_cap = r * 1.018;
        let omsi_base = r * 0.287;
        let line_end = omsi_x + omsi_w - r * 0.58;
        let stop_r = r * 0.365;
        let stops = [r * 0.563, open_x + open_w - r * 0.07, line_end];
        let half = h * 0.5;
        let right = (omsi_x + omsi_w + omsi_cap * BRAND_SLANT).max(line_end + stop_r);
        Brand {
            r,
            ring_w,
            open: (open_x, r * 0.19, open_px),
            omsi: (omsi_x, omsi_base, omsi_px),
            line_end,
            stops,
            stop_r,
            bounds: Rect::new(-half, -half, right + half, half + r + stop_r),
        }
    }

    /// Draws it with the ring's centre at `c`; `ground` is what it stands on (the bus's
    /// windscreen and lamps are of it).
    fn draw(&self, p: &mut Painter, atlas: &mut Atlas, fonts: &Fonts, c: Vec2, ground: Color, ink: Color) {
        let r = self.r;
        // the ring from its end at the lower right, up and round against the clock to its
        // foot, and on as the line under the words
        let mut route = Path::new(c + Vec2::from_angle(BRAND_RING_END) * r);
        route.arc_around(c, -(1.5 * std::f32::consts::PI + BRAND_RING_END)).line_to(c + Vec2::new(self.line_end, r));
        // (the line in the accent, as the large mark's: Luc)
        p.stroke(route.points(), self.ring_w, Color::hex(crate::accent::chosen()));
        // the bus's front as the small icon has it (`openomsi-small.svg`: no band, no
        // mirrors - each part a pixel or two), in the logo's units
        let k = r / 167.0;
        let part = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::new(c.x + x0 * k, c.y + y0 * k, (x1 - x0) * k, (y1 - y0) * k);
        for s in [-1.0f32, 1.0] {
            let (a, b) = (46.0 * s, 76.0 * s);
            p.rounded(part(a.min(b), 60.0, a.max(b), 100.0), 6.0 * k, ink);
        }
        p.rounded(part(-88.0, -80.0, 88.0, 76.0), 18.0 * k, ink);
        p.rounded(part(-68.0, -58.0, 68.0, 18.0), 8.0 * k, ground);
        for s in [-1.0f32, 1.0] {
            p.circle(c + Vec2::new(55.0 * s, 47.0) * k, 14.0 * k, ground);
        }
        // the words on whole pixels (text between them goes soft); "OMSI" leant - its
        // picture sheared about its baseline
        let scale = p.scale;
        let d = 1.0 / scale;
        let snap = |v: f32| (v * scale).round() * d;
        let pad = omsi_ui::text::PAD as f32 * d;
        let (x, base, px) = self.open;
        let s = atlas.text(fonts, "open", px * scale, Weight::Bold);
        p.sprite(s, Vec2::new(snap(c.x + x - pad), snap(c.y + base - s.ascent * d)), Vec2::new(s.w, s.h) * d, ink);
        let (x, base, px) = self.omsi;
        let s = atlas.text(fonts, "OMSI", px * scale, Weight::Black);
        let (x0, top, w, h, up) = (snap(c.x + x - pad), snap(c.y + base - s.ascent * d), s.w * d, s.h * d, s.ascent * d);
        let (lean_top, lean_foot) = (up * BRAND_SLANT, (h - up) * BRAND_SLANT);
        let v = |q: Vec2, u: f32, t: f32| Vertex { pos: [q.x, q.y, 0.0], uv: [u, t], color: BRAND_ORANGE.0, mode: [0.0, 1.0], ..Default::default() };
        let (a, b, cc, e) = (Vec2::new(x0 + lean_top, top), Vec2::new(x0 + w + lean_top, top), Vec2::new(x0 + w - lean_foot, top + h), Vec2::new(x0 - lean_foot, top + h));
        p.verts.extend([v(a, s.uv[0], s.uv[1]), v(b, s.uv[2], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(a, s.uv[0], s.uv[1]), v(cc, s.uv[2], s.uv[3]), v(e, s.uv[0], s.uv[3])]);
        // the stops, each on its white rim (shares of the rim's radius, as the logo's)
        for (n, &x) in self.stops.iter().enumerate() {
            let (at, s) = (c + Vec2::new(x, r), self.stop_r);
            p.circle(at, s, BRAND_RIM);
            match n {
                0 => {
                    p.circle(at, s * 0.84, BRAND_RED);
                    p.circle(at, s * 0.6, BRAND_RIM);
                    p.rect(Rect::new(at.x - s * 0.69, at.y - s * 0.13, s * 1.38, s * 0.26), BRAND_RED);
                }
                1 => {
                    p.circle(at, s * 0.9, BRAND_YELLOW);
                    p.arc(at, s * 0.68, s * 0.86, 0.0, std::f32::consts::TAU, BRAND_GREEN);
                    for q in [Rect::new(-0.33, -0.43, 0.16, 0.86), Rect::new(0.17, -0.43, 0.16, 0.86), Rect::new(-0.18, -0.08, 0.36, 0.16)] {
                        p.rect(Rect::new(at.x + q.x * s, at.y + q.y * s, q.w * s, q.h * s), BRAND_GREEN);
                    }
                }
                _ => {
                    p.circle(at, s * 0.89, BRAND_GREY);
                    p.arc(at, s * 0.42, s * 0.78, 0.5, std::f32::consts::PI - 0.5, BRAND_TEAL);
                }
            }
        }
    }
}

/// openOMSI's logo small, as the bar and the classic launcher's rail show it: the ring round
/// the bus's front, "open" and the leaning orange "OMSI" beside it, and the line under them
/// with its stops - the ring `h` tall, its left edge at `left`, the whole of it centred on
/// `mid_y`, on `ground`. Drawn into vertices of its own (both launchers' `Ui` keep their
/// painter behind the atlas it needs); returns them and where it stands.
pub(crate) fn brand(atlas: &mut Atlas, fonts: &Fonts, scale: f32, left: f32, mid_y: f32, h: f32, ground: Color) -> (Vec<Vertex>, Rect) {
    brand_inked(atlas, fonts, scale, left, mid_y, h, ground, BRAND_INK)
}

/// `brand` with "open" and the bus front in `ink` (dark on the light glass of the bar).
#[allow(clippy::too_many_arguments)]
fn brand_inked(atlas: &mut Atlas, fonts: &Fonts, scale: f32, left: f32, mid_y: f32, h: f32, ground: Color, ink: Color) -> (Vec<Vertex>, Rect) {
    let b = Brand::new(fonts, h);
    let c = Vec2::new(left - b.bounds.x, mid_y - b.bounds.center().y);
    let mut p = Painter::with_scale(scale);
    b.draw(&mut p, atlas, fonts, c, ground, ink);
    (p.verts, Rect::new(left, c.y + b.bounds.y, b.bounds.w, b.bounds.h))
}

/// The flags and the list of the other languages, from `right` leftwards in the bar `r`.
/// Returns where they begin.
fn languages(l: &mut Launcher, r: Rect, right: f32) -> f32 {
    let current = l.state.settings.get("language").and_then(|v| v.as_str()).unwrap_or("ENG").to_string();
    let names: Vec<String> = omsi_launcher_lib::LANGUAGES.iter().map(|x| x.1.to_string()).collect();
    let mut sel = omsi_launcher_lib::LANGUAGES.iter().position(|x| x.0 == current).unwrap_or(0);
    let more = Rect::new(right - 28.0, r.center().y - 13.0, 28.0, 26.0);
    let mut pick: Option<String> = None;
    if l.ui.menu("bar-languages", more, "language", "Other languages", !FLAGS.contains(&current.as_str()), &mut sel, &names) {
        pick = omsi_launcher_lib::LANGUAGES.get(sel).map(|x| x.0.to_string());
    }
    let mut x = more.x - 6.0;
    for code in FLAGS.iter().rev() {
        let b = Rect::new(x - 26.0, r.center().y - 10.0, 26.0, 20.0);
        let (_, _, clicked) = l.ui.interact(id_of(&format!("bar-flag-{code}")), b);
        let on = current == *code;
        let f = Rect::new(b.center().x - 9.0, b.center().y - 6.5, 18.0, 13.0);
        super::ui::flag(l.ui.p(), f, code);
        if on {
            l.ui.p().rounded_border(b, 6.0, 1.0, accent());
        }
        if let Some(name) = omsi_launcher_lib::LANGUAGES.iter().find(|x| x.0 == *code) {
            l.ui.tooltip(b, name.1);
        }
        if clicked && !on {
            pick = Some(code.to_string());
        }
        x = b.x - 2.0;
    }
    super::tour::anchor("bar-flags", Rect::new(x, r.y, right - x, r.h));
    if let Some(code) = pick {
        set_language(l, &code);
    }
    x
}

/// The interface (and the game) in another language: the setting the Settings page's
/// Language sets, saved as it is, and spoken at once.
pub(super) fn set_language(l: &mut Launcher, code: &str) {
    l.state.settings["language"] = serde_json::json!(code);
    l.state.settings_dirty = 0.3;
    crate::ui_language(code);
}

/// The steps as a route: done ones in ink and a way back, the current one blue and
/// underlined, the ones to come quiet. A narrow bar keeps the words of the current one only.
/// On a page a step leads back to the steps. Returns where the route ends.
fn step_list(l: &mut Launcher, r: Rect, now: Step) -> f32 {
    let on_page = l.page != Page::Drive;
    let steps = steps_of(l);
    let at = steps.iter().position(|s| s.0 == now).unwrap_or(0);
    let words: Vec<String> = steps.iter().map(|s| omsi_ui::tr(s.1).to_uppercase()).collect();
    let widths: Vec<f32> = words.iter().map(|w| l.ui.width(w, 12.0, Weight::Bold) + 24.0).collect();
    let total: f32 = widths.iter().sum();
    let joins = steps.len().saturating_sub(1) as f32;
    let join = ((r.w - total) / joins.max(1.0) - 16.0).clamp(0.0, 34.0);
    let narrow = total + joins * 8.0 > r.w;
    // (the route between the steps fills up to the current one as the underline slides there)
    let route = l.ui.spring(id_of("flow-route"), at as f32, super::ui::Feel::SLIDE);
    let mut under = None;
    let mut x = r.x;
    for (k, (step, _, icon)) in steps.iter().enumerate() {
        let word = if narrow && k != at { String::new() } else { words[k].clone() };
        let w = if word.is_empty() { 30.0 } else { widths[k] };
        let cell = Rect::new(x, r.y, w, r.h);
        let done = k < at;
        let current = k == at;
        let (h, _, clicked) = l.ui.interact(id_of(&format!("flow-step-{k}")), cell);
        // (a step to come can be gone to as well when the choice before it is made: the
        // launcher remembers the last duty, so the map and the bus are often chosen already)
        let open = done || current || l.state.map().is_some();
        if clicked && open {
            l.drive.step = *step;
            if on_page {
                l.go(Page::Drive);
            }
        }
        let c = if current {
            accent()
        } else if done || (h && open) {
            glass_look().ink
        } else {
            glass_look().ink_faint
        };
        l.ui.icon(icon, Vec2::new(cell.x + 9.0, cell.center().y), 15.0, c);
        if !word.is_empty() {
            l.ui.text_in(&word, Rect::new(cell.x + 22.0, cell.y, w - 22.0, cell.h), 12.0, Weight::Bold, c, Align::Left);
        }
        if current {
            under = Some((cell.x - 10.0, cell.right() + 4.0));
        }
        x += w;
        if k + 1 < steps.len() && !narrow && join > 4.0 {
            let line = Rect::new(x + 4.0, r.center().y - 1.0, join, 2.0);
            let done = (route - k as f32).clamp(0.0, 1.0);
            if done < 1.0 {
                l.ui.p().rect(line, glass_look().ink.alpha(0.15));
            }
            if done > 0.0 {
                l.ui.p().rect(Rect::new(line.x, line.y, line.w * done, line.h), accent());
            }
            x += join + 12.0;
        } else {
            x += 8.0;
        }
    }
    // the current step's underline, sliding from the step before to this one
    if let Some((x0, x1)) = under {
        let (a, b) = l.ui.slide_span(id_of("flow-underline"), x0, x1);
        l.ui.p().rounded(Rect::new(a, r.bottom() - 3.0, b - a, 3.0), 1.5, accent());
    }
    super::tour::anchor("bar-steps", Rect::new(r.x - 10.0, r.y, (x - r.x + 2.0).max(0.0), r.h));
    x
}

/// The action row: back on the left (from the sheet's edge on), the extra actions and the
/// main one on the right. Returns (back, main, the extra one clicked).
pub(super) fn actions(l: &mut Launcher, from_x: f32, back: bool, main: &str, main_icon: &str, extra: &[(&str, &str)]) -> (bool, bool, Option<usize>) {
    let size = l.ui.size;
    let go = action_rect(size);
    let mut clicked_back = false;
    let main_clicked = l.ui.button("flow-main", go, main, Some(main_icon), ButtonKind::Primary);
    super::tour::anchor("main-action", go);
    let mut x = go.x - 12.0;
    let mut extra_clicked = None;
    for (k, (label, icon)) in extra.iter().enumerate() {
        let w = l.ui.width(label, 14.5, Weight::Bold) + if icon.is_empty() { 44.0 } else { 70.0 };
        let r = Rect::new(x - w, go.y, w, go.h);
        if l.ui.button(&format!("flow-extra-{k}"), r, label, (!icon.is_empty()).then_some(*icon), ButtonKind::Normal) {
            extra_clicked = Some(k);
        }
        if *icon == "360" {
            super::tour::anchor("bus-look", r);
        }
        x = r.x - 12.0;
    }
    if back {
        let r = Rect::new(from_x, go.y, 100.0, go.h);
        if l.ui.button("flow-back", r, "Back", Some("chevron_left"), ButtonKind::Normal) {
            clicked_back = true;
        }
    }
    (clicked_back, main_clicked, extra_clicked)
}

/// The Drive page: the ground, the bar, the step's sheet and the actions.
pub fn draw(l: &mut Launcher) {
    let size = l.ui.size;
    let window = Rect::new(0.0, 0.0, size.x, size.y);
    // a free drive has no duty step, a duty no start point step
    if l.drive.step == Step::Duty && l.state.choice.free {
        l.drive.step = Step::Bus;
    }
    if l.drive.step == Step::Start && !l.state.choice.free {
        l.drive.step = Step::Day;
    }
    let step = l.drive.step;
    match step {
        Step::Profile => step_profile(l, window),
        Step::Mode => step_mode(l, window),
        // (the maps on a sheet of their own: their pictures as tiles, or the list beside the map)
        Step::Map => super::mapchoice::draw(l, window),
        Step::Start | Step::Day | Step::Duty => step_on_map(l, window, step),
        Step::Bus => step_bus(l, window),
    }
    bar(l, Some(step), None);
}

/// Another page in the wide sheet under the bar, on the start's ground: the sheet's head says
/// what it is and for what (once - the bar names it in passing), the way back in its corner.
/// The sheet stops short of the window's foot, where the status line is.
pub fn page(l: &mut Launcher, page: Page, title: &str) {
    // (the bus company is an application screen of its own: the whole window, its own bar -
    // Luc: "in plaats van een tegel")
    if page == Page::Company {
        super::company::screen(l);
        return;
    }
    let size = l.ui.size;
    ground_picture(l, Rect::new(0.0, 0.0, size.x, size.y));
    let r = centred(Rect::new(EDGE_IN, SHEET_TOP, size.x - 2.0 * EDGE_IN, (size.y - SHEET_TOP - 40.0).max(200.0)), PAGE_MAX_W);
    sheet(l, r);
    let title = if page == Page::Profile { "Service record" } else { title };
    let back = Rect::new(r.right() - 24.0 - 108.0, r.y + 20.0, 108.0, 40.0);
    if l.ui.button("page-back", back, "Back", Some("chevron_left"), ButtonKind::Normal) {
        // (the line editor is a page of the editor's - or, working for the bus company, of
        // the company's)
        l.go(match page {
            Page::Lines if l.pages.lines.for_company() => Page::Company,
            Page::Lines => Page::Editor,
            Page::Depots if l.pages.depots.from_lines() => Page::Lines,
            Page::Depots => Page::Editor,
            _ => Page::Drive,
        });
    }
    // what a page keeps in its head (the drivers on the service record), and its name and
    // what it is for in what is left of the head
    let tools = Rect::new(r.x + r.w * 0.4, back.y, (back.x - 12.0 - r.x - r.w * 0.4).max(0.0), back.h);
    let head_end = super::pages::head_tools(l, page, tools).min(back.x) - 16.0;
    let (icon, sub) = super::pages::about(page, l.pages.controls_tab);
    let x = r.x + 20.0;
    let w = (head_end - x - 32.0).max(0.0);
    l.ui.icon(icon, Vec2::new(x + 11.0, r.y + 34.0), 22.0, TEXT);
    l.ui.text_in(title, Rect::new(x + 32.0, r.y + 18.0, w, 32.0), 26.0, Weight::Bold, TEXT, Align::Left);
    l.ui.text_in(sub, Rect::new(x + 32.0, r.y + 50.0, w, 18.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    let inner = Rect::new(r.x + 28.0, r.y + 88.0, r.w - 56.0, r.h - 88.0 - 24.0);
    match page {
        Page::Multiplayer => super::multiplayer::draw(l, inner),
        Page::Profile => super::pages::profile(l, inner),
        Page::Settings => super::pages::settings(l, inner),
        Page::Controls => super::pages::controls(l, inner),
        Page::Sessions => super::pages::sessions(l, inner),
        Page::Mods => super::pages::mods(l, inner),
        Page::Tutorials => super::pages::tutorials(l, inner),
        Page::Timetable => super::timetable::draw(l, inner),
        Page::Setup => super::pages::setup(l, inner),
        Page::Editor => super::editor_hub::draw(l, inner),
        Page::Lines => super::lineeditor::draw(l, inner),
        Page::Depots => super::depoteditor::draw(l, inner),
        Page::Drive | Page::Livery | Page::Company => {}
    }
    bar(l, Some(step_of(page)), Some(title));
}

// --- the driver ---------------------------------------------------------------------------

/// Who drives: the drivers as tiles - a driver is a person, not a row in a table.
fn step_profile(l: &mut Launcher, window: Rect) {
    ground_picture(l, window);
    let size = l.ui.size;
    let r = wide_rect(size, true);
    sheet(l, r);
    let body = sheet_head(l, Rect::new(r.x + (r.w - 760.0).max(0.0) * 0.5, r.y, r.w.min(760.0), r.h), "person", "Bus drivers", "Who is driving today?");
    let names = l.state.profiles.clone();
    let body = sheet_foot(l, Rect::new(body.x, body.y, body.w, r.bottom() - body.y), &omsi_ui::tr("%{n} drivers on this computer").replace("%{n}", &names.len().to_string()));
    // (nobody on this computer yet: the new driver's tile is open from the start)
    if names.is_empty() && !l.pages.new_driver_open {
        super::pages::open_new_driver(l);
    }
    let adding = l.pages.new_driver_open;
    let count = names.len() + adding as usize;
    let (tw, th, gap) = (260.0, 160.0, 12.0);
    let per_row = (((body.w - 40.0 + gap) / (tw + gap)).floor() as usize).max(1);
    let rows = count.div_ceil(per_row).max(1);
    let grid_h = rows as f32 * (th + gap) - gap;
    let top = body.y + ((body.h - grid_h) * 0.5).max(10.0);
    let mut made = None;
    // (while the question whether to delete a driver is open, the tiles under it take nothing)
    let asking = l.pages.delete_driver.is_some();
    let held = asking.then(|| {
        let i = l.ui.input.clone();
        l.ui.input.mouse = Vec2::new(-1e4, -1e4);
        (l.ui.input.pressed, l.ui.input.released, l.ui.input.double_click) = (false, false, false);
        l.ui.input.keys.clear();
        i
    });
    let mut delete_asked = None;
    for k in 0..count {
        let (row, col) = (k / per_row, k % per_row);
        let in_row = (count - row * per_row).min(per_row);
        let row_w = in_row as f32 * (tw + gap) - gap;
        let base = Rect::new(body.x + (body.w - row_w) * 0.5 + col as f32 * (tw + gap), top + row as f32 * (th + gap), tw, th);
        let Some(name) = names.get(k) else {
            // the tile of the driver being made, after the others
            made = super::pages::new_driver_tile(&mut l.ui, base, &mut l.pages.new_driver);
            continue;
        };
        let on = *name == l.state.config.profile;
        // (a driver's tile moves under the mouse as the start's do; chosen, it turns blue and
        // the click's ripple runs over it)
        let t = l.ui.tile(id_of(&format!("driver-{k}")), base, SHEET_RADIUS);
        let tile = t.r;
        // (the blue comes in as quickly as a hover does: a click's change, not a jump)
        let blue = l.ui.anim(id_of(&format!("driver-{k}")) ^ 0xb1e, if on { 1.0 } else { 0.0 }, 0.06);
        l.ui.tile_shadow(&t);
        l.ui.p().rounded(tile, SHEET_RADIUS, PANEL.mix(HOVER, t.hover).mix(accent(), blue));
        l.ui.tile_light(&t, if on { 0.08 } else { 0.05 });
        if blue < 0.99 {
            l.ui.tile_edge(&t, 1.0, EDGE.alpha(1.0 - blue));
        }
        let mono = Rect::new(tile.x + 17.0, tile.y + 25.0, 56.0, 56.0);
        l.ui.p().rounded(mono, RADIUS, Color::WHITE.alpha(0.09 + 0.09 * blue));
        let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        l.ui.text_in(&initial, mono, 20.0, Weight::Bold, TEXT, Align::Center);
        l.ui.text_in(name, Rect::new(tile.x + 17.0, tile.y + 92.0, base.w - 34.0, 20.0), 15.0, Weight::Bold, TEXT, Align::Left);
        // (what the chosen driver has driven, as Omsi-Hub's tile says it: the others' files
        // are not read until one is chosen)
        if let Some(p) = l.state.profile.as_ref().filter(|p| on && p.name.eq_ignore_ascii_case(name)) {
            let line = format!("{} · {}", omsi_ui::tr("%{n} duties").replace("%{n}", &p.record.runs.to_string()), hours_text(p.hours));
            l.ui.text_in(&line, Rect::new(tile.x + 17.0, tile.y + 116.0, base.w - 34.0, 18.0), 12.5, Weight::Medium, TEXT, Align::Left);
        }
        // (under the mouse, or chosen: a bin in the corner asks to delete the driver)
        let bin = Vec2::new(tile.right() - 24.0, tile.y + 24.0);
        let on_bin = l.ui.input.mouse.distance(bin) <= 15.0;
        if t.hover > 0.3 || on {
            if l.ui.icon_button(&format!("driver-delete-{k}"), bin, 14.0, "delete", "Delete this driver") {
                delete_asked = Some(name.clone());
            }
        }
        if t.clicked && !on && !on_bin {
            l.state.config.profile = name.clone();
            let _ = omsi_launcher_lib::save_config(&l.state.config);
            l.state.load_profile();
            l.state.touched();
        }
    }
    match made {
        Some(true) => super::pages::create_driver(l),
        Some(false) => l.pages.new_driver_open = false,
        None => {}
    }
    if let Some(i) = held {
        l.ui.input = i;
    }
    if delete_asked.is_some() {
        l.pages.delete_driver = delete_asked;
    }
    delete_dialog(l, window);
    // (Omsi-Hub's order: New driver, Service record, the way on)
    let (_, next, extra) = actions(l, EDGE_IN, false, "Next step", "play_arrow", &[("Service record", ""), ("New driver", "")]);
    if next {
        l.drive.step = Step::Mode;
    }
    match extra {
        Some(0) => l.go(Page::Profile),
        Some(_) => super::pages::open_new_driver(l),
        None => {}
    }
}

/// The question whether to delete a driver, over the drivers step: their personnel file and
/// service record go, and that cannot be undone.
fn delete_dialog(l: &mut Launcher, window: Rect) {
    let Some(name) = l.pages.delete_driver.clone() else { return };
    l.ui.solid(window);
    l.ui.p().rect(window, Color::rgba(4, 7, 15, 0.62));
    let w = 460.0f32.min(window.w - 32.0);
    let text = omsi_ui::tr("Their personnel file and service record on this computer are deleted. This cannot be undone.").to_string();
    let text_h = l.ui.paragraph_height(&text, w - 48.0, 13.5, Weight::Regular);
    let h = 20.0 + 30.0 + 12.0 + text_h + 24.0 + 40.0 + 22.0;
    let r = Rect::new(window.center().x - w * 0.5, window.center().y - h * 0.5, w, h);
    l.ui.p().shadow(r.inset(-2.0), SHEET_RADIUS, 30.0, Color::rgba(0, 0, 0, 0.5));
    l.ui.panel(r);
    let x = r.x + 24.0;
    l.ui.icon("delete", Vec2::new(x + 11.0, r.y + 35.0), 22.0, DANGER);
    l.ui.text_in(&omsi_ui::tr("Delete %{name}?").replace("%{name}", &name), Rect::new(x + 32.0, r.y + 20.0, w - 80.0, 30.0), 19.0, Weight::Bold, TEXT, Align::Left);
    l.ui.paragraph(&text, Vec2::new(x, r.y + 62.0), w - 48.0, 13.5, Weight::Regular, TEXT_SOFT);
    let by = r.bottom() - 22.0 - 40.0;
    let escape = l.ui.input.keys.contains(&super::ui::Key::Escape);
    if l.ui.button("driver-delete-no", Rect::new(r.right() - 24.0 - 130.0 - 12.0 - 120.0, by, 120.0, 40.0), "Cancel", None, ButtonKind::Normal) || escape {
        l.pages.delete_driver = None;
    }
    if l.ui.button("driver-delete-yes", Rect::new(r.right() - 24.0 - 130.0, by, 130.0, 40.0), "Delete", Some("delete"), ButtonKind::Danger) {
        l.pages.delete_driver = None;
        super::pages::delete_driver(l, &name);
    }
}

fn hours_text(h: f64) -> String {
    super::pages::duration_text(h)
}

// --- the mode (the start) -----------------------------------------------------------------

/// How the driver wants to drive today: the start of the launcher. The ways to drive are
/// tiles with a photo each; under them the driver's record and the ways to everything else.
/// The sheet is as tall as what is on it: stretched to the window's bottom it left half a
/// screen of nothing under the buttons.
fn step_mode(l: &mut Launcher, window: Rect) {
    ground_picture(l, window);
    let size = l.ui.size;
    let pad = 34.0;
    let avail = wide_rect(size, false);
    // openOMSI's mark large over the greeting, in the middle (`intro::Logo`: its light runs
    // along the line under the mouse); the tiles give way to it on a lower window
    let logo_h = (avail.h * 0.13).clamp(64.0, 120.0);
    let head = logo_h + 16.0 + 92.0;
    // (under the tiles: the record beside four rows of links, the last the editor's)
    let bh = 280.0;
    let tile_h = ((avail.h - head - 22.0 - bh - 56.0) * 0.95).clamp(180.0, 340.0);
    let content_h = head + tile_h + 22.0 + bh;
    // (a card in the middle of the window, both ways, at most `CARD_MAX_W` wide)
    let avail = centred(avail, CARD_MAX_W);
    let h = (content_h + 56.0).min(avail.h);
    let r = Rect::new(avail.x, avail.y + ((avail.h - h) * 0.5).max(0.0), avail.w, h);
    glass_sheet(l, r);
    let inner = Rect::new(r.x + pad, r.y + 28.0, r.w - 2.0 * pad, r.h - 56.0);
    let name = l.state.profile.as_ref().map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| l.state.config.profile.clone());
    let hour = omsi_launcher_lib::local_now().map(|t| t.3).unwrap_or(12);
    let greeting = match hour {
        5..=11 => "Good morning %{name}",
        12..=17 => "Good afternoon %{name}",
        _ => "Good evening %{name}",
    };
    let logo_w = (inner.w * 0.5).min(620.0);
    let logo_r = Rect::new(inner.center().x - logo_w * 0.5, inner.y, logo_w, logo_h);
    let Launcher { ui, start_logo, .. } = l;
    start_logo.ink = Some(glass_look().ink);
    start_logo.draw(ui, "start-logo", logo_r);
    let gy = inner.y + logo_h + 16.0;
    l.ui.text_in(&omsi_ui::tr(greeting).replace("%{name}", &name), Rect::new(inner.x, gy, inner.w, 40.0), 32.0, Weight::Bold, glass_look().ink, Align::Center);
    l.ui.text_in("How do you want to drive today?", Rect::new(inner.x, gy + 46.0, inner.w, 20.0), 14.0, Weight::Regular, glass_look().ink_soft, Align::Center);
    // the ways to drive (kinds: 0 a tour, 1 a shift, 2 free)
    // (and a fourth tile, the bus company: no way to drive but a place of its own, never shown
    // as the chosen one - Omsi-Hub's bus company tile beside its modes; the editor is a long
    // button under the links)
    let modes: [(usize, &str, &str, &str, &str); 4] = [
        (1, "Work shift", "schedule", "mode-shift", "Choose how long you want to drive and when: openOMSI puts a shift together from the timetable's real trips, on with another line where lines meet."),
        (0, "Tour", "route", "mode-tour", "One bus's trips on one line, as OMSI's timetable dialog gives them: pick the line, the tour and the trip to start with."),
        (2, "Free drive", "map", "mode-free", "Only a map and a bus. The traffic and the timetable's buses drive around you; nothing is booked."),
        (4, "Bus company", "garage", "mode-company", "Your own transport company: buy, lease or rent buses, hire drivers and run lines day by day."),
    ];
    let c = &l.state.choice;
    let current = if c.free { 2 } else if c.composed { 1 } else { 0 };
    let ty = inner.y + head;
    let gap = 16.0;
    let tw = (inner.w - gap * (modes.len() as f32 - 1.0)) / modes.len() as f32;
    let mut chosen = None;
    for (k, (kind, title, icon, picture, text)) in modes.iter().enumerate() {
        let base = Rect::new(inner.x + k as f32 * (tw + gap), ty, tw, tile_h);
        // under the mouse the tile rises and grows a little, its photo comes closer and moves
        // against the mouse, a light follows the mouse over it and its edge lights up blue
        // (`Ui::tile`); the words are laid out in the tile as drawn but keep their size
        let t = l.ui.tile(id_of(&format!("mode-{kind}")), base, SHEET_RADIUS);
        let tile = t.r;
        let on = *kind == current;
        l.ui.tile_shadow(&t);
        if on {
            l.ui.p().shadow(tile.inset(-4.0), SHEET_RADIUS + 4.0, 22.0, accent().alpha(0.45));
        }
        match l.pictures.get(picture).copied() {
            Some((tex, w, hh)) => l.ui.tile_photo(&t, tile, SHEET_RADIUS, tex, w, hh),
            None => l.ui.p().rounded(tile, SHEET_RADIUS, FIELD),
        }
        // the text on the photo: the lower part darkened to the ground's colour - and once more
        // under the words (Luc's photos are bright and busy: a white bus under white words)
        l.ui.p().rounded_gradient(tile, SHEET_RADIUS, Color::rgba(9, 12, 24, 0.05), Color::rgba(9, 12, 24, 0.9));
        let low = Rect::new(tile.x, tile.y + tile.h * 0.3, tile.w, tile.h * 0.7);
        l.ui.p().rounded_gradient(low, SHEET_RADIUS, Color::rgba(9, 12, 24, 0.0), Color::rgba(9, 12, 24, 0.75));
        l.ui.tile_light(&t, 0.15);
        if on {
            l.ui.p().rounded_border(tile, SHEET_RADIUS, 2.5, accent());
        } else {
            l.ui.tile_edge(&t, 1.0, EDGE);
        }
        let (px, title_px) = if base.w < 300.0 { (12.5, 22.0) } else { (13.5, 26.0) };
        let text_h = l.ui.paragraph_height(text, base.w - 40.0, px, Weight::Medium);
        let badge = Rect::new(tile.x + 20.0, (tile.bottom() - 20.0 - text_h - 38.0 - 50.0).max(tile.y + 14.0), 42.0, 42.0);
        // (the mark warms to the route's blue under the mouse, the chosen one's is blue)
        l.ui.p().rounded(badge, RADIUS, if on { accent() } else { Color::WHITE.alpha(0.16).mix(accent(), 0.9 * t.hover) });
        l.ui.icon(icon, badge.center(), 21.0, Color::WHITE.mix(on_accent(), if on { 1.0 } else { 0.9 * t.hover }));
        // (widths from the tile where it lies: the words wrap and cut the same under the mouse)
        l.ui.text_in(title, Rect::new(tile.x + 20.0, badge.bottom() + 6.0, base.w - 40.0, 32.0), title_px, Weight::Bold, Color::WHITE, Align::Left);
        l.ui.paragraph(text, Vec2::new(tile.x + 20.0, badge.bottom() + 46.0), base.w - 40.0, px, Weight::Medium, Color::rgba(232, 235, 242, 0.92));
        if t.clicked {
            chosen = Some(*kind);
        }
    }
    super::tour::anchor("mode-tiles", Rect::new(inner.x, ty, inner.w, tile_h));
    if chosen == Some(4) {
        l.go(Page::Company);
    } else if let Some(kind) = chosen {
        l.state.choice.free = kind == 2;
        l.state.choice.composed = kind == 1;
        l.state.touched();
        l.drive.step = Step::Map;
    }
    // under the tiles: the record and the ways to the rest
    let by = ty + tile_h + 22.0;
    let rec = Rect::new(inner.x, by, inner.w * 0.44, bh);
    record_box(l, rec);
    // (every page of the launcher has its way in here: the service record is the box beside)
    let links: [(&str, &str, Page); 9] = [
        ("Driver: %{name}", "person", Page::Drive),
        ("Mods", "extension", Page::Mods),
        ("Settings", "settings", Page::Settings),
        ("Controls", "keyboard", Page::Controls),
        ("Timetable", "schedule", Page::Timetable),
        ("Multiplayer", "groups", Page::Multiplayer),
        ("Sessions", "sports_esports", Page::Sessions),
        ("Tutorials", "help", Page::Tutorials),
        ("Setup", "folder_open", Page::Setup),
    ];
    super::tour::anchor("start-links", Rect::new(inner.x, by, inner.w, bh));
    let lx = rec.right() + 18.0;
    let lw = (inner.right() - lx - 24.0) / 3.0;
    let lh = ((bh - 3.0 * 12.0) / 4.0).min(58.0);
    for (k, (label, icon, page)) in links.iter().enumerate() {
        let b = Rect::new(lx + (k % 3) as f32 * (lw + 12.0), by + (k / 3) as f32 * (lh + 12.0), lw, lh);
        let label = omsi_ui::tr(label).replace("%{name}", &name);
        if link(l, &format!("hub-{k}"), b, &label, icon) {
            if *page == Page::Drive {
                l.drive.step = Step::Profile;
            } else {
                l.go(*page);
            }
        }
    }
    // the editor: one long button under the links (lines of one's own, liveries, the
    // timetable, the map's objects)
    let eb = Rect::new(lx, by + 3.0 * (lh + 12.0), 3.0 * lw + 2.0 * 12.0, lh);
    super::tour::anchor("editor-tile", eb);
    if link(l, "hub-editor", eb, &omsi_ui::tr("Editor: lines, liveries, the timetable and the map's objects"), "construction") {
        l.go(Page::Editor);
    }
}

/// The start's box of the driver's record, Omsi-Hub's: what they have driven in four big
/// figures, how far it is to the next level, and the whole box the way to the service record.
fn record_box(l: &mut Launcher, rec: Rect) {
    let t = l.ui.tile(id_of("hub-record"), rec, SHEET_RADIUS);
    let (rec, hover, clicked) = (t.r, t.hover, t.clicked);
    l.ui.tile_shadow(&t);
    l.ui.p().rounded(rec, SHEET_RADIUS, glass_look().on.mix(glass_look().on_hover, 0.6 * hover));
    l.ui.tile_light(&t, 0.04);
    l.ui.tile_edge(&t, 1.0, glass_look().edge);
    l.ui.text_in(&omsi_ui::tr("Your service record").to_uppercase(), Rect::new(rec.x + 24.0, rec.y + 22.0, rec.w - 48.0, 16.0), 10.5, Weight::Bold, glass_look().ink_dim, Align::Left);
    let p = l.state.profile.as_ref();
    let stats: [(String, &str); 4] = [
        (p.map(|p| p.record.runs).unwrap_or(0).to_string(), "duties"),
        (hours_text(p.map(|p| p.hours).unwrap_or(0.0)), "driven"),
        (format!("{:.0}", p.map(|p| p.km).unwrap_or(0.0)), "km"),
        (p.map(|p| p.level).unwrap_or(1).to_string(), "level"),
    ];
    let mut sx = rec.x + 24.0;
    for (value, what) in stats {
        let vw = l.ui.width(&value, 28.0, Weight::Bold).max(l.ui.width(what, 12.5, Weight::Regular)) + 30.0;
        l.ui.text_in(&value, Rect::new(sx, rec.y + 52.0, vw, 34.0), 28.0, Weight::Bold, glass_look().ink, Align::Left);
        l.ui.text_in(what, Rect::new(sx, rec.y + 88.0, vw, 16.0), 12.5, Weight::Regular, glass_look().ink_dim, Align::Left);
        sx += vw;
    }
    if let Some(p) = l.state.profile.clone() {
        let line = omsi_ui::tr("Level %{level} · %{xp} of %{next} points").replace("%{level}", &p.level.to_string()).replace("%{xp}", &p.xp.to_string()).replace("%{next}", &p.next_level_xp.to_string());
        l.ui.text_in(&line, Rect::new(rec.x + 24.0, rec.y + 124.0, rec.w - 48.0, 18.0), 13.0, Weight::Medium, glass_look().ink_soft, Align::Left);
        let bar = Rect::new(rec.x + 24.0, rec.y + 150.0, (rec.w - 48.0).min(420.0), 6.0);
        l.ui.p().rounded(bar, bar.h * 0.5, glass_look().ink.alpha(0.10));
        l.ui.progress(bar, super::pages::level_progress(p.xp, p.level, p.next_level_xp), false);
    }
    // the way in, said at the box's foot
    let more = omsi_ui::tr("Open the service record");
    let mw = l.ui.width(&more, 13.0, Weight::Bold);
    let at = Rect::new(rec.right() - 24.0 - mw - 18.0, rec.bottom() - 40.0, mw + 18.0, 20.0);
    let ink = glass_look().ink_soft.mix(glass_look().ink, hover);
    l.ui.text_in(&more, Rect::new(at.x, at.y, mw, at.h), 13.0, Weight::Bold, ink, Align::Left);
    // (the arrow a step on, the way the box leads)
    l.ui.icon("chevron_right", Vec2::new(at.right() - 6.0 + 2.5 * t.rise.max(0.0), at.center().y), 16.0, ink);
    if clicked {
        l.go(Page::Profile);
    }
}

/// The ground of the steps without a map: Omsi-Hub's night city, darkened under the sheets.
pub(super) fn ground_picture(l: &mut Launcher, window: Rect) {
    paint_ground(l.ui.p(), window);
}

/// The launcher's ground: openOMSI's route in the accent colour (Luc's backgrounds, drawn
/// rather than pictures so that they stay sharp and follow any colour) - a ring round a bus
/// seen from the front, the line out of it with two stops, rising to the upper right.
fn paint_ground(p: &mut Painter, window: Rect) {
    let (ground, ink) = ground_colours(crate::accent::chosen());
    p.rect(window, ground);
    // (the drawing is 1672 x 941; it covers the window, held to the left - the ring with its
    // bus stays in view in a narrow window - and in the middle the other way)
    let k = (window.w / GROUND_W).max(window.h / GROUND_H);
    let o = Vec2::new(window.x, window.y + (window.h - GROUND_H * k) * 0.5);
    let k2 = k;
    let at = |x: f32, y: f32| o + Vec2::new(x, y) * k;
    let (c, r) = (Vec2::new(167.0, 528.0), 282.5);
    let on_ring = |deg: f32| c + Vec2::new(deg.to_radians().cos(), deg.to_radians().sin()) * r;
    // the route: from the ring's open end round over its top and down its left into the line,
    // along the line, up the bend and away to the upper right
    let e = on_ring(27.0);
    let mut path = Path::new(at(e.x, e.y));
    path.arc_around(at(c.x, c.y), (-242.0f32).to_radians());
    path.cubic_to(at(-24.2, 747.3), at(100.0, 757.0), at(175.0, 757.0));
    path.line_to(at(1060.0, 757.0));
    path.cubic_to(at(1118.0, 757.0), at(1180.3, 728.5), at(1210.3, 702.1));
    path.line_to(at(1752.0, 224.5));
    p.stroke(path.points(), 70.0 * k2, ink);
    // its two stops
    for (x, y) in [(340.0, 757.0), (1343.0, 585.0)] {
        p.circle(at(x, y), 52.0 * k2, ink);
        // (the hole twice, the second a polygon fanned from its edge: a sample on a seam of
        // the one let the route through as a hairline (Luc), the other has its seams elsewhere)
        let (c, r) = (at(x, y), 33.0 * k2);
        p.circle(c, r, ground);
        let ring: Vec<Vec2> = (0..64).map(|i| c + Vec2::from_angle((i as f32 + 0.5) * std::f32::consts::TAU / 64.0) * r).collect();
        p.convex(&ring, ground);
    }
    // the bus in the ring, from the front
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let a = at(x0, y0);
        let b = at(x1, y1);
        Rect::new(a.x, a.y, b.x - a.x, b.y - a.y)
    };
    p.rounded(rr(28.0, 392.0, 54.0, 463.0), 13.0 * k2, ink);
    p.rounded(rr(296.0, 392.0, 322.0, 463.0), 13.0 * k2, ink);
    p.rounded(rr(83.0, 560.0, 122.0, 617.0), 10.0 * k2, ink);
    p.rounded(rr(228.0, 560.0, 267.0, 617.0), 10.0 * k2, ink);
    p.rounded(rr(65.0, 338.0, 286.0, 590.0), 24.0 * k2, ink);
    p.rounded(rr(120.0, 357.0, 231.0, 376.0), 6.0 * k2, ground);
    p.rounded(rr(85.0, 393.0, 266.0, 511.0), 13.0 * k2, ground);
    for x in [103.0, 246.0] {
        p.circle(at(x, 551.0), 18.0 * k2, ground);
    }
}

/// The dark mode's ground: a deep night blue.
const NIGHT: Color = Color::rgba(10, 20, 46, 1.0);

/// The size of the launcher's ground drawing (Luc's backgrounds were made at this size).
const GROUND_W: f32 = 1672.0;
const GROUND_H: f32 = 941.0;

/// The ground's colours for the accent `rgb`: the ground and the route drawn on it - a light
/// ground under the route in the colour itself (the default orange on Luc's cream).
fn ground_colours(rgb: u32) -> (Color, Color) {
    if crate::accent::dark() {
        // (dark: a deep night blue (Luc), the route in the colour a little sunk into it)
        let [r, g, b] = crate::accent::unpack(rgb);
        return (NIGHT, Color::rgba(r, g, b, 1.0).mix(NIGHT, 0.25));
    }
    if rgb == crate::accent::DEFAULT {
        return (Color::rgba(252, 234, 208, 1.0), Color::rgba(247, 138, 18, 1.0));
    }
    let [r, g, b] = crate::accent::unpack(rgb);
    let light = |v: u8| (v as f32 + (255.0 - v as f32) * 0.55).round() as u8;
    (Color::rgba(light(r), light(g), light(b), 1.0), Color::rgba(r, g, b, 1.0))
}

/// A plain button on the start's sheet: an icon and a word, left aligned.
/// (Under the mouse it moves as the tiles do - `Ui::tile`. Its fill is the sheet's own colour
/// at rest, so that the shadow it gets when it rises does not show through it.)
fn link(l: &mut Launcher, name: &str, r: Rect, label: &str, icon: &str) -> bool {
    let t = l.ui.tile(id_of(name), r, RADIUS);
    let r = t.r;
    l.ui.tile_shadow(&t);
    l.ui.p().rounded(r, RADIUS, glass_look().on.mix(glass_look().on_hover, t.hover));
    l.ui.tile_light(&t, 0.05);
    l.ui.tile_edge(&t, 1.0, glass_look().edge);
    l.ui.icon(icon, Vec2::new(r.x + 24.0, r.center().y), 16.0, glass_look().ink_soft.mix(glass_look().ink, t.hover));
    l.ui.text_in(label, Rect::new(r.x + 44.0, r.y, r.w - 54.0, r.h), 14.0, Weight::Bold, glass_look().ink, Align::Left);
    t.clicked
}

// --- the steps on the map -----------------------------------------------------------------

/// Day and duty: the map is the ground, the sheet on its left (the map step is `mapchoice`).
fn step_on_map(l: &mut Launcher, window: Rect, step: Step) {
    if step == Step::Duty {
        step_duty(l, window);
        return;
    }
    let size = l.ui.size;
    let s = sheet_rect(size);
    // what the sheet leaves of the map: the route is framed in it
    let clear = Rect::new(s.right() + 20.0, SHEET_TOP, (size.x - s.right() - 40.0).max(160.0), (size.y - SHEET_TOP - ACTION_BOTTOM - ACTION_H - 20.0).max(160.0));
    // (a free drive shows the route of the line it follows, if any)
    l.mapview.want(if l.state.choice.free { super::freedrive::map_look(l) } else { drive::map_look(l) });
    l.map_background(window);
    // the roadbook beside the duty, on the map's right
    let mut book: Option<Rect> = None;
    if step == Step::Duty && !l.state.choice.free {
        let book_w = (size.x * 0.24).clamp(290.0, 380.0);
        if l.drive.book_open {
            let b = Rect::new(size.x - EDGE_IN - book_w, SHEET_TOP, book_w, (size.y - SHEET_TOP - ACTION_BOTTOM - ACTION_H - 24.0).max(200.0));
            sheet(l, b);
            drive::book_panel(l, b);
            book = Some(b);
        }
    }
    sheet(l, s);
    let (icon, title, sub) = match step {
        Step::Start => ("location_on", "Start point", super::freedrive::start_line(l)),
        Step::Day => ("partly_cloudy_day", "Day & weather", super::daytime::day_line(l)),
        _ => ("schedule", if l.state.choice.composed { "Shifts" } else { "Tours" }, drive::duty_of(l).1),
    };
    let body = sheet_head(l, s, icon, title, &sub);
    let inner = Rect::new(body.x + 18.0, body.y, body.w - 36.0, body.h - 18.0);
    match step {
        Step::Start => super::freedrive::start_panel(l, s, body),
        Step::Day => super::daytime::day_panel(l, s, body),
        _ => drive::duty_panel(l, inner, false),
    }
    // the map's names and the roadbook's handle, off the sheets
    let avoid: Vec<Rect> = [Some(s), book].into_iter().flatten().collect();
    if step == Step::Duty && !l.state.choice.free && book.is_none() {
        let h = drive::book_handle(l, Rect::new(clear.x, clear.y - 70.0, clear.w + 20.0, clear.h));
        let _ = h;
    }
    // (a free drive's own: the tour remembered from a duty is not what the map shows then)
    if l.state.choice.free {
        super::freedrive::map_marks(l, clear, &avoid);
    } else {
        drive::map_labels(l, clear, &avoid);
    }
    let previous = previous_of(l, step);
    let main = if step == Step::Duty || (step == Step::Day && l.state.choice.free) { "Next: the bus" } else { "Next step" };
    let (back, next, _) = actions(l, s.right() + 14.0, previous.is_some(), main, "play_arrow", &[]);
    if back {
        if let Some(p) = previous {
            l.drive.step = p;
        }
    }
    if next {
        if let Some(n) = next_of(l, step) {
            l.drive.step = n;
        }
    }
    // (an entry point clicked on the map in a free drive is its start from then on)
    let entry = l.state.choice.entry;
    l.map_interact(window, clear);
    if l.state.choice.free && l.state.choice.entry != entry {
        super::freedrive::entry_clicked(l, entry);
    }
}

/// The duty, as Omsi-Hub's duty step: the shifts (or the line's tours) on the sheet
/// (`shiftsheet`), the chosen one's route on the map - every trip of it in blue, its stops
/// signed - framed in what the sheets leave of it, the map's buttons in its top right corner,
/// and the roadbook beside it when the player opens it.
fn step_duty(l: &mut Launcher, window: Rect) {
    let size = l.ui.size;
    let s = sheet_rect(size);
    let map_bottom = size.y - ACTION_BOTTOM - ACTION_H - 20.0;
    let book_w = (size.x * 0.24).clamp(290.0, 380.0);
    let book = l.drive.book_open.then(|| Rect::new(size.x - EDGE_IN - book_w, SHEET_TOP, book_w, (map_bottom - 4.0 - SHEET_TOP).max(200.0)));
    // what the sheets leave of the map: the route is framed in it
    let right = book.map(|b| b.x).unwrap_or(size.x) - 20.0;
    let clear = Rect::new(s.right() + 20.0, SHEET_TOP, (right - s.right() - 20.0).max(160.0), (map_bottom - SHEET_TOP).max(160.0));
    super::tour::anchor("shift-map", clear);
    l.mapview.want(drive::map_look(l));
    l.map_background(window);
    if let Some(b) = book {
        sheet(l, b);
        drive::book_panel(l, b);
    }
    sheet(l, s);
    let (title, sub) = shiftsheet::head(l);
    let body = sheet_head(l, s, "schedule", &title, &sub);
    let foot = shiftsheet::foot(l);
    let body = sheet_foot(l, Rect::new(s.x, body.y, s.w, s.bottom() - body.y), &foot);
    shiftsheet::body(l, body);
    // the map's own buttons, right of the route (left of the roadbook when it is open)
    let tools_at = Vec2::new(book.map(|b| b.x - 14.0).unwrap_or(size.x - EDGE_IN), SHEET_TOP + 8.0);
    let tools = shiftsheet::map_tools(l, tools_at, book.is_none());
    // the names on the map, off the sheets, the buttons and the action row
    let actions_r = Rect::new(s.right(), action_rect(size).y, size.x - s.right(), ACTION_H);
    let avoid: Vec<Rect> = [Some(s), book, Some(actions_r)].into_iter().flatten().chain(tools).collect();
    drive::map_labels(l, Rect::new(0.0, SHEET_TOP - 8.0, size.x, size.y - SHEET_TOP + 8.0), &avoid);
    shiftsheet::scale_bar(l, Vec2::new(EDGE_IN - 10.0, size.y - 14.0));
    let extra: Vec<(&str, &str)> = if l.state.choice.composed { vec![("Find other shifts", "")] } else { Vec::new() };
    let (back, next, other) = actions(l, s.right() + 14.0, true, "Next step", "play_arrow", &extra);
    if back {
        if let Some(p) = previous_of(l, Step::Duty) {
            l.drive.step = p;
        }
    }
    if next {
        if let Some(n) = next_of(l, Step::Duty) {
            l.drive.step = n;
        }
    }
    if other == Some(0) {
        shiftsheet::other_shifts(l);
    }
    l.map_interact(window, clear);
}

/// The round mark of a row: a ring, and a dot in it when chosen (it pops in: `Ui::radio`).
pub(super) fn radio(ui: &mut super::ui::Ui, c: Vec2, on: bool) {
    ui.radio(c, on);
}

// --- the bus ------------------------------------------------------------------------------

/// The bus: the buses as tiles with their photos in a wide sheet - makers, models, versions
/// (see `buspick`) - and once one is chosen, the bus itself on the ground, turned by the
/// mouse, its livery and the rest on the sheet beside it. The bus that fits the duty best is
/// offered in a dialog over the step on the way in.
fn step_bus(l: &mut Launcher, window: Rect) {
    let size = l.ui.size;
    super::buspick::frame(l);
    // (the offer lies over the step: the step sees no mouse and no keys meanwhile; not while
    // the guided tour shows the step - it is made when the tour has gone)
    let offer = (super::buspick::offering(l) && !super::tour::active(l)).then(|| {
        let i = l.ui.input.clone();
        l.ui.input.mouse = Vec2::new(-1e4, -1e4);
        (l.ui.input.pressed, l.ui.input.released, l.ui.input.wheel) = (false, false, Vec2::ZERO);
        l.ui.input.keys.clear();
        l.ui.input.text.clear();
        i
    });
    let showroom = super::buspick::showing(l);
    let (title, sub) = super::buspick::heading(l);
    let from_x = if showroom {
        let s = sheet_rect(size);
        l.preview_full(window, super::buspick::stage(size, s));
        sheet(l, s);
        let body = sheet_head(l, s, "directions_bus", &title, &sub);
        super::buspick::bus_sheet(l, Rect::new(body.x + 18.0, body.y, body.w - 36.0, body.h - 18.0));
        s.right() + 14.0
    } else {
        ground_picture(l, window);
        let r = wide_rect(size, true);
        sheet(l, r);
        let col = Rect::new(r.x + (r.w - 1120.0).max(0.0) * 0.5, r.y, r.w.min(1120.0), r.h);
        let body = sheet_head(l, col, "directions_bus", &title, &sub);
        let foot = super::buspick::foot(l);
        let body = sheet_foot(l, Rect::new(r.x, body.y, r.w, r.bottom() - body.y), &foot);
        super::buspick::browse(l, Rect::new(col.x + 20.0, body.y, col.w - 40.0, body.h - 6.0), Rect::new(r.x, body.bottom(), r.w, r.bottom() - body.bottom()));
        EDGE_IN
    };
    let running = l.state.instances.iter().filter(|i| i.running).count();
    let label = if running > 0 && l.state.second_armed.map(|t| t.elapsed().as_secs() < 6).unwrap_or(false) {
        "Start another game"
    } else if l.state.choice.free || l.state.choice.line.is_none() || (l.state.choice.composed && l.state.composed_legs().is_none()) {
        "Drive"
    } else {
        "Start the duty"
    };
    let can_continue = l.state.joined_server.is_none() && l.state.has_last_situation();
    let mut extra: Vec<(&str, &str)> = if can_continue { vec![("Continue last game", "history")] } else { vec![] };
    // (on the tiles: the chosen bus in the showroom in one click)
    let look = !showroom && l.state.bus().is_some();
    if look {
        extra.push(("View in 3D", "360"));
    }
    let (back, go, extra_clicked) = actions(l, from_x, true, label, "play_arrow", &extra);
    // Back goes up through the tiles first (the showroom, a model, a maker), then to the step
    // before
    if back && !super::buspick::back(l) {
        if let Some(p) = previous_of(l, Step::Bus) {
            l.drive.step = p;
        }
    }
    if go {
        drive::start(l);
    }
    match extra_clicked {
        Some(0) if can_continue => l.state.launch_last_situation(),
        Some(_) if look => super::buspick::show_bus(l),
        _ => {}
    }
    // by the bus, when the duty starts, what it comes to in one line, above the action (the
    // tiles' sheet reaches down to the actions: no room there)
    if showroom {
        let (duty, when) = drive::duty_of(l);
        let go_r = action_rect(size);
        let line = if when.is_empty() { duty } else { format!("{duty} · {when}") };
        let tw = (go_r.right() - from_x - 40.0).max(0.0);
        let t = Rect::new(go_r.right() - tw, go_r.y - 30.0, tw, 20.0);
        let lw = l.ui.width(&line, 12.5, Weight::Medium).min(tw);
        l.ui.p().rounded(Rect::new(t.right() - lw - 16.0, t.y - 2.0, lw + 16.0, t.h + 4.0), 6.0, ON_MAP);
        l.ui.text_in(&line, Rect::new(t.x, t.y, t.w - 8.0, t.h), 12.5, Weight::Medium, TEXT_SOFT, Align::Right);
        l.showroom_pointer(window);
    }
    if let Some(i) = offer {
        l.ui.input = i;
        super::buspick::offer(l);
    }
    let _ = hhmm;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_small_logo_is_laid_out_as_the_logo() {
        for fonts in [Fonts::hanken(), Fonts::new()] {
            for h in [20.0, 24.0, 30.0, 60.0] {
                let b = Brand::new(&fonts, h);
                // the ring as tall as asked, its band two pixels at least
                assert!((b.r * 2.0 + b.ring_w - h).abs() < 1e-3 && b.ring_w >= 2.0, "{h}");
                // the words right of the ring, "OMSI" after "open" and a little lower
                assert!(b.open.0 > b.r + b.ring_w * 0.5 && b.omsi.0 > b.open.0 && b.omsi.1 > b.open.1, "{h}");
                // the stops on the line in their order, the first past the ring's foot, the
                // terminus at the line's end and under "OMSI"
                assert!(b.stops[0] > 0.0 && b.stops[0] < b.stops[1] && b.stops[1] < b.stops[2], "{h}");
                assert!(b.stops[2] == b.line_end && b.line_end > b.omsi.0, "{h}");
                // the H between the two words
                assert!(b.stops[1] > b.open.0 && b.stops[1] < b.omsi.0, "{h}");
                // all of it in its bounds: the ring's top to the stops' rims, the ring's left to
                // the terminus
                assert!((b.bounds.y + h * 0.5).abs() < 1e-3 && (b.bounds.x + h * 0.5).abs() < 1e-3, "{h}");
                assert!(b.bounds.bottom() >= b.r + b.stop_r - 1e-3 && b.bounds.right() >= b.line_end + b.stop_r - 1e-3, "{h}");
            }
            // it grows with its ring
            let (a, b) = (Brand::new(&fonts, 24.0), Brand::new(&fonts, 48.0));
            assert!((b.bounds.w / a.bounds.w - 2.0).abs() < 0.05);
        }
    }

    #[test]
    fn the_small_logo_stands_where_it_is_put() {
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(1024);
        let (verts, at) = brand(&mut atlas, &fonts, 1.0, 40.0, 20.0, 24.0, PANEL);
        assert!(!verts.is_empty());
        assert!((at.x - 40.0).abs() < 1e-3 && (at.center().y - 20.0).abs() < 1e-3);
        // the shapes lie in it; the words' pictures (their line's whole height, clear round
        // the letters) right of the ring, and in it but for that clear edge
        for v in &verts {
            let [x, y, _] = v.pos;
            if v.mode[1] == 0.0 {
                assert!(x > at.x - 0.5 && x < at.right() + 0.5 && y > at.y - 0.5 && y < at.bottom() + 0.5, "{:?} outside {at:?}", v.pos);
            } else {
                assert!(x > at.x + 24.0 && x < at.right() + 4.0 && y > at.y - 4.0 && y < at.bottom(), "{:?} outside {at:?}", v.pos);
            }
        }
    }
}