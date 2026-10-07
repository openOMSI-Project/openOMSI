//! The guided tour for a player new to the launcher, after Omsi-Hub's "rondleiding": the
//! window darkens except for a rounded spotlight round the part being talked about, and a
//! speech bubble beside it - a small arrow at the spotlight - says what it is for in two or
//! three sentences, with the way on, back and out (also the arrow keys, Enter and Escape).
//! The spotlight and the bubble glide from stop to stop on springs; with the setting
//! "animations" off they are where they go at once.
//!
//! The tour walks the setup itself: for each stop it puts the launcher on the step (and in
//! the way of driving) where that part is, and at the end - or when it is skipped - the
//! player's own page, step and way of driving are back. It chooses nothing for the player:
//! without a chosen map the stops that need one are left out, rather than a map chosen. It
//! never starts a game: while it runs the page under it gets no mouse and no keys. Its last
//! stops are about the game itself - the navigator, the city map and signing on, the phone
//! and the tablet - as cards in the middle with a drawing each, since nothing in the launcher
//! shows them. The bus that drives between two screens stays away while the tour changes
//! them: it would cover what the spotlight is about to show.
//!
//! The parts register themselves while they are drawn (`anchor`, one line where each one is
//! drawn), so the tour never works out another module's layout. A stop whose part is not
//! drawn after all (the bus in the showroom, a list still empty) is left out as the tour
//! comes to it. A phone has no setup steps to walk: there the tour is its cards alone.
//!
//! The bus company has a tour of its own (`start_company`, Luc: "een welkomstscherm voor de
//! busbedrijfmodus, met een tutorial"): a welcome card with what the mode is, then its pages
//! one by one - the clock, the tabs, the overview, the fleet and the dealer, the staff, the
//! lines, the planning, the money and the career with "My duties" - each with its part lit.
//! The company remembers per driver where it was left (`company::tutorial`), and its "?" goes
//! on from there.
//!
//! `OMSI_LAUNCHER_TOUR=1` starts the tour with the launcher, `=7` at its seventh stop (for
//! pictures of it).

use std::cell::RefCell;

use glam::Vec2;
use omsi_ui::paint::{Align, Path};
use omsi_ui::{Color, Painter, Rect, Weight};

use super::flow::Step;
use super::theme::*;
use super::ui::{ease_in_out_cubic, ease_out_cubic, id_of, smoothstep, ButtonKind, Feel, Input, Key, Ui};
use super::{intro, mobile, Launcher, Page};

// --- the parts pointed at ------------------------------------------------------------------

thread_local! {
    /// This frame's parts the tour can point at, by name (see `anchor`): cleared before the
    /// page is drawn, read once it is.
    static ANCHORS: RefCell<Vec<(&'static str, Rect)>> = const { RefCell::new(Vec::new()) };
}

/// Where a part the tour can point at was drawn this frame; `key` is its name in
/// `Stop::target`. Called by whatever draws the part - also inside a scroll area's closure,
/// which only has the `Ui` (hence a list of the thread the launcher draws on, not a field of
/// the launcher). The first of a name drawn in a frame counts.
pub fn anchor(key: &'static str, r: Rect) {
    ANCHORS.with(|a| {
        let mut a = a.borrow_mut();
        if !a.iter().any(|(k, _)| *k == key) {
            a.push((key, r));
        }
    });
}

fn anchored(key: &str) -> Option<Rect> {
    ANCHORS.with(|a| a.borrow().iter().find(|(k, _)| *k == key).map(|(_, r)| *r))
}

fn clear_anchors() {
    ANCHORS.with(|a| a.borrow_mut().clear());
}

// --- the stops -----------------------------------------------------------------------------

/// The tour's stops, in their order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Welcome,
    Modes,
    Record,
    Languages,
    Steps,
    MapViews,
    MapTile,
    StartAt,
    MainAction,
    Day,
    ShiftLength,
    ShiftList,
    ShiftRoute,
    FreeStarts,
    FreeLine,
    Buses,
    LookIn3d,
    Navigator,
    CityMap,
    Companion,
    /// The end on a desktop: the "?" that shows the tour again.
    Done,
    /// The end on a phone, as a card (no bar with a "?" there).
    Goodbye,
    /// The bus company's tour (`COMPANY_STOPS`): its welcome, the founding wizard (no company
    /// yet), its clock, its tabs and pages, and its "?".
    CoWelcome,
    CoFound,
    CoClock,
    CoTabs,
    CoOverview,
    CoToday,
    CoFleet,
    CoStaff,
    CoLines,
    CoPlanning,
    CoFinances,
    CoCareer,
    CoDone,
}

use Stop::*;

/// The bus company's tour, in its order.
const COMPANY_STOPS: [Stop; 13] = [CoWelcome, CoFound, CoClock, CoTabs, CoOverview, CoToday, CoFleet, CoStaff, CoLines, CoPlanning, CoFinances, CoCareer, CoDone];

const STOPS: [Stop; 22] = [Welcome, Modes, Record, Languages, Steps, MapViews, MapTile, StartAt, MainAction, Day, ShiftLength, ShiftList, ShiftRoute, FreeStarts, FreeLine, Buses, LookIn3d, Navigator, CityMap, Companion, Done, Goodbye];

/// What a stop points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    /// A card in the middle with a drawing: what it is about is in the game, not here.
    Card(Picture),
    /// The part drawn under this name.
    Part(&'static str),
    /// The first of the two that is drawn.
    Either(&'static str, &'static str),
    /// The two together.
    Both(&'static str, &'static str),
    /// The shift's route: its stops as the map shows them, in what the sheets leave of the map.
    Route,
}

/// A card's drawing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Picture {
    Welcome,
    Navigator,
    CityMap,
    Companion,
    Arrived,
}

/// The way of driving a stop needs (a shift has the duty step, a free drive the start point).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Shift,
    Free,
}

/// Where the launcher has to be for a stop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Place {
    /// Where the stop before left it (a card about the game).
    Keep,
    /// The player's own page, step and way of driving.
    Home,
    /// This step of the setup, in this way of driving (or in the one it is in).
    On(Step, Option<Mode>),
    /// The bus company, on this tab.
    Company(usize),
}

impl Stop {
    /// The chapter it is in, its title and what it says (English: the keys of `app.yml`).
    fn words(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Welcome => ("Guided tour", "Welcome aboard", "openOMSI puts real duties together from the timetables of your own OMSI maps and gets the game ready for you. In a minute you'll know where everything is."),
            Modes => ("The start", "How do you want to drive?", "A work shift is put together from the timetable's real trips. A tour is one bus's trips on one line, as OMSI gives them. And a free drive is just a map and a bus."),
            Record => ("The start", "Your record and the rest", "On the left, your service record: what you have driven and how close the next level is. On the right, the way to everything else: mods, settings, controls, timetables and more."),
            Languages => ("The start", "Your language", "A flag switches the language at once; the globe beside them has all the others. The game speaks it too."),
            Steps => ("The map", "Step by step", "Up here are the steps, in the order a driver thinks: who drives, how, where, when, what and with which bus. A step you have been to is one click away."),
            MapViews => ("The map", "Tiles or a list", "Every map you have in OMSI is here: as tiles with their picture, or as a list with the map beside it. You switch between them here."),
            MapTile => ("The map", "A map", "A tile shows how many tours a map has, which year it is set in and at how many places the bus can start. Click it and it's yours."),
            StartAt => ("The map", "Where the bus starts", "On Automatic the bus is put down nearest to your duty's first stop. Or pick a place yourself: the orange marks on the map are the same places."),
            MainAction => ("The map", "On you go", "Bottom right, on every step, is the button that takes you on. On the last step it starts the game with your duty."),
            Day => ("The day", "Day and weather", "Choose the date, the time and the weather. The timetable runs as it does on that day: on a Sunday there are often fewer buses."),
            ShiftLength => ("The duty", "How long do you want to drive?", "Say how long your shift is and in which part of the day it starts. openOMSI looks for shifts like that among the timetable's real trips."),
            ShiftList => ("The duty", "Your shifts", "These are the shifts that fit. Click one to see all its trips, with the breaks in between; Find other shifts draws new ones."),
            ShiftRoute => ("The duty", "Your route", "On the map, in blue, is where the shift you pick takes you, with every stop on the way. Drag to look around; the wheel zooms."),
            FreeStarts => ("Free drive", "Where do you start?", "On a free drive you choose where the bus starts: at a stop where trips begin, at one on the way, or at one of the map's entry points. The traffic and the timetable's buses drive around you."),
            FreeLine => ("Free drive", "Choose a line yourself", "Want to follow a line after all? Pick it here: its route is on the map, and the navigator and the IBIS know the line. Nothing is booked."),
            Buses => ("The bus", "Choose your bus", "First the maker, then the model and the version, each with a photo. Up here you see where you are: click it to go back up. The bus that fits your duty best says so on its tile."),
            LookIn3d => ("The bus", "View in 3D", "Your bus in the showroom: turn it with the mouse, zoom with the wheel, and choose its livery, fleet number and plate."),
            Navigator => ("In the game", "Your duty on the navigator", "Shift+N switches the navigator between the map, the map with your duty board, and off. The board shows the trip you are on, the stops ahead and whether you are on time."),
            CityMap => ("In the game", "The city map and signing on", "Shift+M, or a click on the navigator, opens the city map with your route. Beside it you sign on for your duty with your personnel number and code: they are on your driver's pass, right there."),
            Companion => ("In the game", "Phone & tablet", "Signing on, the duty menu and the bus's screens can also be on a phone or tablet in your network. Turn it on under Settings › General › Phone & tablet; the game shows the address and the code when you drive."),
            Done => ("Guided tour", "Ready to go", "This question mark shows the tour again. Have a good ride!"),
            Goodbye => ("Guided tour", "Ready to go", "That's the tour. Have a good ride!"),
            CoWelcome => ("Bus company", "Your own bus company", "Here you run a transport company on one of your maps: buy buses, hire drivers, take on lines or make your own. It runs in a time of its own, and what you drive yourself counts for it."),
            CoFound => ("Bus company", "Found it first", "Give the company a name, its colours, its home map with a depot and how hard its economy is. Once it runs, this tour shows you round its pages."),
            CoClock => ("Its time", "The company's own clock", "The company runs in a time of its own: let it run here, or jump on to the morning, to tomorrow or by days. At midnight the day is closed: the tours are run and the money is booked."),
            CoTabs => ("Its pages", "Everything in tabs", "Each part of the company has a page here: the buses, the staff, the lines, the money, the planning and more."),
            CoOverview => ("Overview", "The figures", "The cash, this month's result, the fleet, the staff, punctuality and reputation at a glance."),
            CoToday => ("Overview", "What wants your attention", "Today's tours and how many are covered, your own next duty, and what needs doing. A click takes you to the page that helps."),
            CoFleet => ("Fleet", "Buses and the dealer", "Your buses with their condition and their livery. At the dealer you buy new or used ones, haggle, lease or rent, and sign the contract."),
            CoStaff => ("Staff", "Drivers and the others", "Hire drivers on the labour market and keep them content. Their wages, licences, holidays and days ill all count."),
            CoLines => ("Lines", "What the company runs", "Take on the map's lines or make your own in the line editor: its kind, its buses, its timetable. A line runs once it is planned."),
            CoPlanning => ("Planning", "Who drives what", "The week as a chart: drag drivers and buses onto the tours and duties, or let the dispatcher fill the roster. Plan yourself as a driver too."),
            CoFinances => ("Finances", "The money", "Every booking, month by month: fares, the authority's money, wages, fuel and repairs. Loans come from the bank here."),
            CoCareer => ("Career", "Your duties and your progress", "My duties lists the duties planned for you, each a click from driving it. Here too: the company's level, the training courses and the rankings."),
            CoDone => ("Bus company", "Off to work", "This question mark shows the tour again. Good luck with your company!"),
        }
    }

    fn target(self) -> Target {
        match self {
            Welcome => Target::Card(Picture::Welcome),
            Modes => Target::Part("mode-tiles"),
            Record => Target::Part("start-links"),
            Languages => Target::Part("bar-flags"),
            Steps => Target::Part("bar-steps"),
            MapViews => Target::Part("map-views"),
            // (the chosen map's tile, else the first one in view)
            MapTile => Target::Either("map-tile", "map-tile-any"),
            StartAt => Target::Part("map-start"),
            MainAction => Target::Part("main-action"),
            Day => Target::Part("day-sheet"),
            ShiftLength => Target::Part("shift-length"),
            ShiftList => Target::Part("shift-list"),
            ShiftRoute => Target::Route,
            FreeStarts => Target::Part("free-starts"),
            FreeLine => Target::Part("free-line"),
            Buses => Target::Both("bus-crumbs", "bus-grid"),
            LookIn3d => Target::Part("bus-look"),
            Navigator => Target::Card(Picture::Navigator),
            CityMap => Target::Card(Picture::CityMap),
            Companion => Target::Card(Picture::Companion),
            Done => Target::Part("bar-help"),
            Goodbye => Target::Card(Picture::Arrived),
            CoWelcome => Target::Card(Picture::Welcome),
            CoFound => Target::Part("company-wizard"),
            CoClock => Target::Part("company-clock"),
            CoTabs => Target::Part("company-tabs"),
            CoOverview => Target::Part("company-figures"),
            CoToday => Target::Part("company-today"),
            CoFleet | CoStaff | CoLines | CoPlanning | CoFinances => Target::Part("company-page"),
            CoCareer => Target::Part("company-career-parts"),
            CoDone => Target::Part("company-help"),
        }
    }

    fn place(self) -> Place {
        match self {
            Welcome | Modes | Record | Languages => Place::On(Step::Mode, None),
            Steps | MapViews | MapTile | StartAt | MainAction => Place::On(Step::Map, None),
            Day => Place::On(Step::Day, None),
            ShiftLength | ShiftList | ShiftRoute => Place::On(Step::Duty, Some(Mode::Shift)),
            FreeStarts | FreeLine => Place::On(Step::Start, Some(Mode::Free)),
            Buses | LookIn3d => Place::On(Step::Bus, None),
            Navigator | CityMap | Companion | Goodbye => Place::Keep,
            Done | CoDone => Place::Home,
            CoWelcome | CoFound | CoClock | CoTabs | CoOverview | CoToday => Place::Company(0),
            CoFleet => Place::Company(1),
            CoStaff => Place::Company(2),
            CoLines => Place::Company(3),
            CoFinances => Place::Company(4),
            CoPlanning => Place::Company(5),
            CoCareer => Place::Company(6),
        }
    }

    /// Whether the stop has something to show in the launcher as it is.
    fn shown(self, c: &Ctx) -> bool {
        match self {
            // (the company's tour: the founding wizard before there is a company, its pages
            // once there is one)
            CoWelcome => true,
            CoFound => !c.company,
            CoClock | CoTabs | CoOverview | CoToday | CoFleet | CoStaff | CoLines | CoPlanning | CoFinances | CoCareer | CoDone => c.company,
            Welcome | Navigator | CityMap | Companion => true,
            Goodbye => c.phone,
            _ if c.phone => false,
            Done => true,
            StartAt => c.map && c.entries,
            // (on a server the day is the server's: the sheet only says so)
            Day => c.map && !c.server,
            ShiftLength | ShiftList | ShiftRoute | FreeLine => c.map,
            FreeStarts => c.map && !c.own_line,
            Buses => !c.showroom,
            LookIn3d => !c.showroom && c.bus,
            _ => true,
        }
    }
}

/// What the launcher has when the tour starts, as far as the stops care.
#[derive(Clone, Copy, Default, Debug)]
struct Ctx {
    phone: bool,
    /// A map is chosen (and installed).
    map: bool,
    /// It has entry points to choose from (not on a server: the server's map starts where it says).
    entries: bool,
    /// A bus is chosen.
    bus: bool,
    /// The bus step shows the chosen bus in the showroom rather than the tiles.
    showroom: bool,
    /// The free drive follows a line of the player's own choosing (its stops are not shown).
    own_line: bool,
    server: bool,
    /// A bus company is open (not the founding wizard).
    company: bool,
}

impl Ctx {
    fn of(l: &Launcher) -> Ctx {
        let server = super::drive::joined_server_name(l).is_some();
        let map = l.state.map();
        Ctx {
            phone: mobile::mobile(),
            map: map.is_some(),
            entries: !server && map.is_some_and(|m| !m.entry_points.is_empty()),
            bus: l.state.bus().is_some(),
            showroom: super::buspick::showing(l),
            own_line: l.state.choice.own_line,
            server,
            company: l.company.company.is_some() && l.company.wizard.is_none(),
        }
    }
}

/// The stops the tour makes in the launcher as it is.
fn plan(c: &Ctx) -> Vec<Stop> {
    STOPS.iter().copied().filter(|s| s.shown(c)).collect()
}

/// The stops of the bus company's tour as the company is (founded or not).
fn company_plan(c: &Ctx) -> Vec<Stop> {
    COMPANY_STOPS.iter().copied().filter(|s| s.shown(c)).collect()
}

// --- going round ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Command {
    Next,
    Back,
    Skip,
}

/// What the keys do: on (the right arrow, Enter), back (the left arrow), out (Escape).
fn key_command(keys: &[Key]) -> Option<Command> {
    keys.iter().find_map(|k| match k {
        Key::Right | Key::Enter => Some(Command::Next),
        Key::Left => Some(Command::Back),
        Key::Escape => Some(Command::Skip),
        _ => None,
    })
}

/// The stop a command takes the tour to from stop `at` of `len`, or None when it is over.
fn after(cmd: Command, at: usize, len: usize) -> Option<usize> {
    match cmd {
        Command::Next => (at + 1 < len).then_some(at + 1),
        Command::Back => Some(at.saturating_sub(1).min(len.saturating_sub(1))),
        Command::Skip => None,
    }
}

/// The stop at `at` was left out (its part is not drawn) and `len` are left: where the tour
/// goes on - the next one going forward, the one before going back (the first when there is
/// none) - or None when it is over.
fn without(at: usize, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if forward {
        (at < len).then_some(at)
    } else {
        Some(at.saturating_sub(1).min(len - 1))
    }
}

/// The player's own place, put back at the end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Home {
    page: Page,
    step: Step,
    free: bool,
    composed: bool,
    /// The bus company's tab.
    tab: usize,
}

/// The spotlight's ring when it lands: when it went out, and whether the spotlight was on its
/// way the frame before.
#[derive(Default)]
struct Glide {
    ping: Option<f32>,
    travelling: bool,
}

/// A tour on its way.
struct Run {
    plan: Vec<Stop>,
    at: usize,
    forward: bool,
    /// The stop whose words the bubble shows: the one before, until the part this one is
    /// about has been drawn.
    shown: Option<Stop>,
    /// Where that part is (None: a card).
    part: Option<Rect>,
    /// Frames the stop has waited for its part.
    waited: u8,
    home: Home,
    /// The tour changed the way of driving (the choice is saved again with the player's own
    /// at the end: something may have saved it meanwhile).
    moved_mode: bool,
    /// Seconds since it opened, since the words changed, since it began to close.
    age: f32,
    words_age: f32,
    closing: Option<f32>,
    glide: Glide,
    /// The bus company's tour (its end is remembered for the driver: `company::tutorial`).
    company: bool,
}

/// The tour: the one running, if any, and the frame drawn without the bus between screens.
#[derive(Default)]
pub struct Tour {
    run: Option<Run>,
    /// The tour changed the screen: the next frame is drawn without the bus (see the module).
    quiet: bool,
    /// The setting "animations" while such a frame is drawn, put back after the page.
    motion: Option<bool>,
    /// `OMSI_LAUNCHER_TOUR` was looked at (see `begin`).
    asked: bool,
}

/// Whether the tour is running (and not on its way out).
pub fn active(l: &Launcher) -> bool {
    l.tour.run.as_ref().is_some_and(|r| r.closing.is_none())
}

/// Start the tour from its first stop (again, if it was running).
pub fn start(l: &mut Launcher) {
    let plan = plan(&Ctx::of(l));
    begin_run(l, plan, 0, false);
}

/// Start the bus company's tour at its stop `at` (0: its welcome; one remembered from the
/// last time it was left goes on from there).
pub fn start_company(l: &mut Launcher, at: usize) {
    let plan = company_plan(&Ctx::of(l));
    begin_run(l, plan, at, true);
}

fn begin_run(l: &mut Launcher, plan: Vec<Stop>, at: usize, company: bool) {
    // (started again while it runs: home is still where the player was)
    let home = match l.tour.run.as_ref() {
        Some(r) => r.home,
        None => Home { page: l.page, step: l.drive.step, free: l.state.choice.free, composed: l.state.choice.composed, tab: l.company.tab },
    };
    let moved_mode = l.tour.run.as_ref().is_some_and(|r| r.moved_mode && r.closing.is_none());
    log::info!("launcher: the {} starts ({} stops, at {})", if company { "company's tour" } else { "guided tour" }, plan.len(), at + 1);
    let at = at.min(plan.len().saturating_sub(1));
    let Some(first) = plan.get(at).copied() else { return };
    l.ui.focus = None;
    l.tour.run = Some(Run { plan, at, forward: true, shown: None, part: None, waited: 0, home, moved_mode, age: 0.0, words_age: 0.0, closing: None, glide: Glide::default(), company });
    go_to(l, first.place());
}

/// Before the page is drawn: the parts of the last frame are forgotten, a frame after the
/// tour changed the screen is drawn without animations (no bus drives across), and while the
/// tour runs the page gets no mouse and no keys. Returns what the tour takes for itself.
pub(super) fn begin(l: &mut Launcher) -> Option<Input> {
    clear_anchors();
    // (`OMSI_LAUNCHER_TOUR=1` starts the tour with the launcher, `=7` at its seventh stop:
    // for pictures of it, and for a phone, whose launcher has no "?" in a bar)
    if !std::mem::replace(&mut l.tour.asked, true) {
        if let Some(n) = omsi_cfg::env::var("OMSI_LAUNCHER_TOUR").ok().and_then(|v| v.trim().parse::<usize>().ok()) {
            start(l);
            if let Some(run) = l.tour.run.as_mut() {
                run.at = n.saturating_sub(1).min(run.plan.len() - 1);
                let place = run.plan[run.at].place();
                go_to(l, place);
            }
        }
    }
    if std::mem::take(&mut l.tour.quiet) {
        l.tour.motion = Some(l.ui.motion);
        l.ui.motion = false;
    }
    if !active(l) {
        return None;
    }
    let taken = l.ui.input.clone();
    let i = &mut l.ui.input;
    i.mouse = Vec2::new(-1e4, -1e4);
    (i.pressed, i.released, i.down, i.right_pressed, i.right_down, i.double_click) = (false, false, false, false, false, false);
    i.wheel = Vec2::ZERO;
    i.keys.clear();
    i.text.clear();
    i.raw_key = None;
    Some(taken)
}

/// Once the page and the bus between screens are drawn: the dark, the spotlight and the
/// bubble over them, with what the tour took for itself.
pub(super) fn draw(l: &mut Launcher, taken: Option<Input>) {
    if let Some(i) = taken {
        l.ui.input = i;
    }
    if let Some(m) = l.tour.motion.take() {
        l.ui.motion = m;
    }
    let dt = l.ui.dt;
    let Some(run) = l.tour.run.as_mut() else { return };
    run.age += dt;
    run.words_age += dt;
    if let Some(c) = run.closing.as_mut() {
        *c += dt;
        if *c >= CLOSE_S {
            l.tour.run = None;
            return;
        }
    }
    let closing = run.closing.is_some();
    // the part the stop is about (while it is not drawn yet, the one before stays shown)
    if let Some(stop) = run.plan.get(run.at).copied().filter(|_| !closing) {
        let found = find(l, stop.target());
        let Some(run) = l.tour.run.as_mut() else { return };
        match found {
            Some(part) => {
                run.waited = 0;
                run.part = part;
                if run.shown != Some(stop) {
                    run.shown = Some(stop);
                    run.words_age = 0.0;
                }
            }
            None => {
                run.waited += 1;
                if run.waited >= WAIT_FRAMES {
                    log::info!("launcher: the guided tour leaves out {stop:?} (not drawn)");
                    leave_out(l);
                }
            }
        }
    }
    let motion = l.ui.motion;
    let name = welcome_name(l);
    let Some(run) = l.tour.run.as_mut() else { return };
    let Some(shown) = run.shown.or_else(|| run.plan.get(run.at).copied()) else {
        l.tour.run = None;
        return;
    };
    let closing = run.closing.is_some();
    let at = run.plan.iter().position(|s| *s == shown).unwrap_or(run.at);
    let (chapter, title, text) = shown.words();
    let title = match (shown, name) {
        (Welcome, Some(n)) => omsi_ui::tr("Welcome aboard, %{name}").replace("%{name}", &n),
        (CoWelcome, Some(n)) => omsi_ui::tr("Your own bus company, %{name}").replace("%{name}", &n),
        _ => title.to_string(),
    };
    let company = run.company;
    let open = match run.closing {
        Some(c) => 1.0 - smoothstep(c / CLOSE_S),
        None if motion => ease_out_cubic(run.age / OPEN_S),
        None => 1.0,
    };
    let words = if motion { (run.words_age / WORDS_S).min(1.0) } else { 1.0 };
    let view = View {
        chapter,
        title,
        text,
        picture: match shown.target() {
            Target::Card(p) => Some(p),
            _ => None,
        },
        part: run.part,
        at,
        total: run.plan.len(),
        first: at == 0,
        last: at + 1 >= run.plan.len(),
        phone: mobile::mobile(),
        open,
        words,
        closing,
        company,
    };
    if open < 1.0 || words < 1.0 {
        l.ui.keep_moving();
    }
    let Launcher { ui, tour, .. } = l;
    let Some(run) = tour.run.as_mut() else { return };
    let cmd = overlay(ui, &view, &mut run.glide);
    if closing {
        return;
    }
    // (what the tour had is used up: nothing drawn after it acts on it, and a phone's page
    // does not scroll behind it)
    let i = &mut l.ui.input;
    (i.pressed, i.released, i.double_click) = (false, false, false);
    i.wheel = Vec2::ZERO;
    i.keys.clear();
    i.text.clear();
    if let Some(c) = cmd {
        act(l, c);
    }
}

/// Where a stop's part is this frame: Some(None) for a card, None when it is not drawn.
fn find(l: &Launcher, t: Target) -> Option<Option<Rect>> {
    match t {
        Target::Card(_) => Some(None),
        Target::Part(k) => anchored(k).map(Some),
        Target::Either(a, b) => anchored(a).or_else(|| anchored(b)).map(Some),
        Target::Both(a, b) => match (anchored(a), anchored(b)) {
            (Some(x), Some(y)) => Some(Some(union(x, y))),
            (Some(x), None) | (None, Some(x)) => Some(Some(x)),
            (None, None) => None,
        },
        Target::Route => anchored("shift-map").map(|map| {
            let stops: Vec<Vec2> = l.mapview.placed_stops().iter().map(|s| l.mapview.project(s.at)).collect();
            Some(route_box(&stops, map).unwrap_or(map))
        }),
    }
}

/// The driver's name for the welcome, if there is one.
fn welcome_name(l: &Launcher) -> Option<String> {
    let name = l.state.profile.as_ref().map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| l.state.config.profile.clone());
    (!name.trim().is_empty()).then_some(name)
}

/// A button or a key: on, back or out.
fn act(l: &mut Launcher, cmd: Command) {
    let Some(run) = l.tour.run.as_mut() else { return };
    let completed = cmd == Command::Next;
    match after(cmd, run.at, run.plan.len()) {
        Some(k) if k != run.at => {
            run.forward = k > run.at;
            run.at = k;
            run.waited = 0;
            let place = run.plan[k].place();
            go_to(l, place);
        }
        Some(_) => {}
        None => finish(l, completed),
    }
}

/// The stop now is left out: on to the next one the way the tour is going.
fn leave_out(l: &mut Launcher) {
    let Some(run) = l.tour.run.as_mut() else { return };
    run.plan.remove(run.at);
    let forward = run.forward;
    match without(run.at, run.plan.len(), run.forward) {
        Some(k) => {
            run.at = k;
            run.waited = 0;
            let place = run.plan[k].place();
            go_to(l, place);
        }
        // (left out at its end going forward: it is done, not skipped)
        None => finish(l, forward),
    }
}

/// The end (`completed`: the last stop gone through, else skipped): the player's own place
/// back, the dark goes, and the company's tour remembers where it was left.
fn finish(l: &mut Launcher, completed: bool) {
    go_to(l, Place::Home);
    let motion = l.ui.motion;
    let Some(run) = l.tour.run.as_mut() else { return };
    let moved = run.moved_mode;
    if run.company {
        let (at, profile) = (run.at, l.state.config.profile.clone());
        super::company::tutorial::left(&profile, at, completed);
    }
    let Some(run) = l.tour.run.as_mut() else { return };
    log::info!("launcher: the guided tour ends at stop {} of {}", run.at + 1, run.plan.len());
    if motion {
        run.closing = Some(0.0);
    } else {
        l.tour.run = None;
    }
    if moved {
        // (saved again as the player had it: the tour's way of driving may have been saved
        // meanwhile)
        l.state.touched();
    }
}

/// The launcher where a stop needs it (a phone: nowhere, its tour is cards).
fn go_to(l: &mut Launcher, place: Place) {
    if mobile::mobile() {
        return;
    }
    let Some(home) = l.tour.run.as_ref().map(|r| r.home) else { return };
    let before = (l.page, l.drive.step, l.company.tab);
    match place {
        Place::Keep => {}
        Place::Home => {
            l.go(home.page);
            l.drive.step = home.step;
            l.company.tab = home.tab;
            set_mode(l, home.free, home.composed);
        }
        Place::Company(tab) => {
            l.go(Page::Company);
            l.company.tab = tab;
        }
        Place::On(step, mode) => {
            l.go(Page::Drive);
            l.drive.step = step;
            match mode {
                Some(Mode::Shift) => set_mode(l, false, true),
                Some(Mode::Free) => set_mode(l, true, false),
                None => {}
            }
        }
    }
    if (l.page, l.drive.step, l.company.tab) != before {
        l.tour.quiet = true;
    }
}

/// The way of driving (as the start's tiles set it, but not saved: it is the tour's).
fn set_mode(l: &mut Launcher, free: bool, composed: bool) {
    let c = &mut l.state.choice;
    if (c.free, c.composed) != (free, composed) {
        (c.free, c.composed) = (free, composed);
        if let Some(run) = l.tour.run.as_mut() {
            run.moved_mode = true;
        }
    }
}

// --- the spotlight and the bubble ----------------------------------------------------------

/// The dark over the window, Omsi-Hub's.
const DIM: Color = Color::rgba(4, 8, 18, 0.66);
/// Room round a part inside the spotlight, the spotlight's corners, how soft its edge is.
const ROOM: f32 = 8.0;
const HOLE_RADIUS: f32 = 16.0;
const FEATHER: f32 = 12.0;
/// The points on a rounded corner of the spotlight (always as many, whatever its size: its
/// soft edge joins two outlines point by point).
const CORNER: usize = 8;
/// The bubble's width, a card's, a card's drawing at most, and the room inside both.
const BUBBLE_W: f32 = 372.0;
const CARD_W: f32 = 560.0;
const PICTURE_H: f32 = 196.0;
const PAD: f32 = 22.0;
/// The type: the title, the words.
const TITLE_PX: f32 = 20.0;
const TEXT_PX: f32 = 13.5;
/// Between the spotlight and the bubble (the arrow is in it), and from the window's edge.
const GAP: f32 = 18.0;
const MARGIN: f32 = 16.0;
/// The arrow's base and height.
const ARROW_W: f32 = 18.0;
const ARROW_H: f32 = 9.0;
const BUTTON_H: f32 = 36.0;
/// How the spotlight and the bubble move: a slide with a touch of overshoot, a little slower
/// than a control's (they cross the window).
const GLIDE: Feel = Feel { response: 0.44, damping: 0.82 };
/// The dark coming in and going, and a stop's words coming in (seconds).
const OPEN_S: f32 = 0.3;
const CLOSE_S: f32 = 0.32;
const WORDS_S: f32 = 0.22;
/// The ring that goes out from the spotlight once it has landed: how long, how far.
const PING_S: f32 = 0.7;
const PING_REACH: f32 = 14.0;
/// Frames a stop waits for its part to be drawn before it is left out.
const WAIT_FRAMES: u8 = 3;

/// Which side of the spotlight the bubble is on (over it: the part fills the window).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Below,
    Above,
    Right,
    Left,
    Over,
}

/// The spotlight round a part: a little room round it, inside the window.
fn hole_round(part: Rect, screen: Vec2) -> Rect {
    let r = part.inset(-ROOM);
    let (x0, y0) = (r.x.max(4.0), r.y.max(4.0));
    let (x1, y1) = (r.right().min(screen.x - 4.0), r.bottom().min(screen.y - 4.0));
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Where a bubble of `size` goes beside the spotlight: under it if there is room, else above,
/// else to its right, else to its left - always whole in the window, in line with the
/// spotlight's middle as far as it can be. Beside a part of a sheet on the left it goes to
/// its right first, over the map, so that the rest of the sheet stays in view. A part that
/// leaves no room round it gets the bubble over its bottom right corner.
fn place(hole: Rect, size: Vec2, screen: Vec2) -> (Rect, Side) {
    let fit_x = |x: f32| x.clamp(MARGIN, (screen.x - MARGIN - size.x).max(MARGIN));
    let fit_y = |y: f32| y.clamp(MARGIN, (screen.y - MARGIN - size.y).max(MARGIN));
    let (cx, cy) = (fit_x(hole.center().x - size.x * 0.5), fit_y(hole.center().y - size.y * 0.5));
    let below = (hole.bottom() + GAP + size.y <= screen.y - MARGIN).then(|| (Rect::new(cx, hole.bottom() + GAP, size.x, size.y), Side::Below));
    let above = (hole.y - GAP - size.y >= MARGIN).then(|| (Rect::new(cx, hole.y - GAP - size.y, size.x, size.y), Side::Above));
    let right = (hole.right() + GAP + size.x <= screen.x - MARGIN).then(|| (Rect::new(hole.right() + GAP, cy, size.x, size.y), Side::Right));
    let left = (hole.x - GAP - size.x >= MARGIN).then(|| (Rect::new(hole.x - GAP - size.x, cy, size.x, size.y), Side::Left));
    let order = if hole.right() < screen.x * 0.4 { [right, below, above, left] } else { [below, above, right, left] };
    order.into_iter().flatten().next().unwrap_or_else(|| (Rect::new(fit_x(hole.right() - 24.0 - size.x), fit_y(hole.bottom() - 24.0 - size.y), size.x, size.y), Side::Over))
}

/// The bubble's arrow: its base on the side facing the spotlight (a little over the bubble's
/// edge, hiding the hairline there), its tip towards the spotlight's middle, as near to it as
/// the bubble's corners let it be.
fn arrow(b: Rect, side: Side, hole: Rect) -> Option<[Vec2; 3]> {
    let half = ARROW_W * 0.5;
    let inside = 1.2;
    let along = |lo: f32, hi: f32, at: f32| {
        let (a, z) = (lo + SHEET_RADIUS + half, hi - SHEET_RADIUS - half);
        if z <= a {
            (lo + hi) * 0.5
        } else {
            at.clamp(a, z)
        }
    };
    match side {
        Side::Below => {
            let x = along(b.x, b.right(), hole.center().x);
            Some([Vec2::new(x - half, b.y + inside), Vec2::new(x, b.y - ARROW_H), Vec2::new(x + half, b.y + inside)])
        }
        Side::Above => {
            let x = along(b.x, b.right(), hole.center().x);
            Some([Vec2::new(x + half, b.bottom() - inside), Vec2::new(x, b.bottom() + ARROW_H), Vec2::new(x - half, b.bottom() - inside)])
        }
        Side::Right => {
            let y = along(b.y, b.bottom(), hole.center().y);
            Some([Vec2::new(b.x + inside, y + half), Vec2::new(b.x - ARROW_H, y), Vec2::new(b.x + inside, y - half)])
        }
        Side::Left => {
            let y = along(b.y, b.bottom(), hole.center().y);
            Some([Vec2::new(b.right() - inside, y - half), Vec2::new(b.right() + ARROW_H, y), Vec2::new(b.right() - inside, y + half)])
        }
        Side::Over => None,
    }
}

/// The smallest rect holding both.
fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(x0, y0, a.right().max(b.right()) - x0, a.bottom().max(b.bottom()) - y0)
}

/// The part of the map a route takes: round its stops in `within` (what the sheets leave of
/// the map), with room round them, and never smaller than a hand (one stop in view is not a
/// dot). None when none of them is in it.
fn route_box(stops: &[Vec2], within: Rect) -> Option<Rect> {
    let mut inside = stops.iter().copied().filter(|p| within.contains(*p));
    let first = inside.next()?;
    let (lo, hi) = inside.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let c = (lo + hi) * 0.5;
    let half = ((hi - lo) * 0.5 + Vec2::splat(28.0)).max(Vec2::new(80.0, 60.0));
    let lo = (c - half).max(Vec2::new(within.x, within.y));
    let hi = (c + half).min(Vec2::new(within.right(), within.bottom()));
    Some(Rect::new(lo.x, lo.y, (hi.x - lo.x).max(0.0), (hi.y - lo.y).max(0.0)))
}

/// The outline of a rounded box, `CORNER` + 1 points a corner, clockwise from the top of
/// its top right corner.
fn outline(r: Rect, radius: f32) -> Vec<Vec2> {
    let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let corners = [
        (Vec2::new(r.right() - rad, r.y + rad), -90.0f32),
        (Vec2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Vec2::new(r.x + rad, r.bottom() - rad), 90.0),
        (Vec2::new(r.x + rad, r.y + rad), 180.0),
    ];
    let mut out = Vec::with_capacity(4 * (CORNER + 1));
    for (c, a0) in corners {
        for k in 0..=CORNER {
            let a = (a0 + 90.0 * k as f32 / CORNER as f32).to_radians();
            out.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    }
    out
}

/// The dark over `screen` with the spotlight `hole` left out: clear at the spotlight's edge,
/// `c` a feather's width out from it, and `c` over everything further.
fn dim(p: &mut Painter, screen: Rect, hole: Rect, radius: f32, c: Color) {
    let outer = hole.inset(-FEATHER);
    let (a, b) = (outline(hole, radius), outline(outer, radius + FEATHER));
    let clear = c.alpha(0.0);
    let n = a.len();
    for k in 0..n {
        let j = (k + 1) % n;
        p.tri(a[k], b[k], b[j], clear, c, c);
        p.tri(a[k], b[j], a[j], clear, c, clear);
    }
    // the window round the soft edge's box
    let (x0, y0, x1, y1) = (screen.x, screen.y, screen.right(), screen.bottom());
    for r in [
        Rect::new(x0, y0, x1 - x0, outer.y - y0),
        Rect::new(x0, outer.bottom(), x1 - x0, y1 - outer.bottom()),
        Rect::new(x0, outer.y, outer.x - x0, outer.h),
        Rect::new(outer.right(), outer.y, x1 - outer.right(), outer.h),
    ] {
        if r.w > 0.0 && r.h > 0.0 {
            p.rect(r, c);
        }
    }
    // and the box's corners outside its rounded ones
    let corners = [Vec2::new(outer.right(), outer.y), Vec2::new(outer.right(), outer.bottom()), Vec2::new(outer.x, outer.bottom()), Vec2::new(outer.x, outer.y)];
    for (q, corner) in corners.into_iter().enumerate() {
        let arc = &b[q * (CORNER + 1)..(q + 1) * (CORNER + 1)];
        for k in 0..CORNER {
            p.tri(corner, arc[k], arc[k + 1], c, c, c);
        }
    }
}

/// A soft light going out from a rounded box's edge, `width` wide.
fn halo(p: &mut Painter, r: Rect, radius: f32, width: f32, c: Color) {
    let (a, b) = (outline(r, radius), outline(r.inset(-width), radius + width));
    let clear = c.alpha(0.0);
    let n = a.len();
    for k in 0..n {
        let j = (k + 1) % n;
        p.tri(a[k], b[k], b[j], c, clear, clear);
        p.tri(a[k], b[j], a[j], c, clear, c);
    }
}

/// A rect on springs (`GLIDE`), each edge its own.
fn glide_rect(ui: &mut Ui, name: &str, to: Rect) -> Rect {
    let id = id_of(name);
    let x = ui.spring(id ^ 1, to.x, GLIDE);
    let y = ui.spring(id ^ 2, to.y, GLIDE);
    let w = ui.spring(id ^ 3, to.w, GLIDE).max(0.0);
    let h = ui.spring(id ^ 4, to.h, GLIDE).max(0.0);
    Rect::new(x, y, w, h)
}

/// How far apart two rects are, by the edge furthest from its place.
fn apart(a: Rect, b: Rect) -> f32 {
    (a.x - b.x).abs().max((a.y - b.y).abs()).max((a.right() - b.right()).abs()).max((a.bottom() - b.bottom()).abs())
}

/// What the overlay shows this frame (worked out from the tour and the launcher by `draw`).
struct View {
    chapter: &'static str,
    title: String,
    text: &'static str,
    picture: Option<Picture>,
    /// The part pointed at (None: a card in the middle).
    part: Option<Rect>,
    at: usize,
    total: usize,
    first: bool,
    last: bool,
    phone: bool,
    /// How far the dark has come in (0 to 1; on its way down while the tour closes).
    open: f32,
    /// How far the stop's words have come in (0 to 1).
    words: f32,
    closing: bool,
    /// The bus company's tour (its last button says so).
    company: bool,
}

/// The bubble's insides, measured.
struct Lay {
    pad: f32,
    picture_h: f32,
    title_px: f32,
    title_h: f32,
    h: f32,
}

fn layout(ui: &Ui, v: &View, w: f32) -> Lay {
    let pad = if v.phone { 18.0 } else { PAD };
    let inner = w - 2.0 * pad;
    let picture_h = if v.picture.is_some() { (PICTURE_H * (w - 20.0) / (CARD_W - 20.0)).clamp(120.0, PICTURE_H) } else { 0.0 };
    let title_px = if v.phone { 18.5 } else { TITLE_PX };
    let title_h = ui.paragraph_height(&v.title, inner, title_px, Weight::Bold);
    let text_h = ui.paragraph_height(v.text, inner, TEXT_PX, Weight::Regular);
    let top = if picture_h > 0.0 { 10.0 + picture_h + 18.0 } else { pad };
    Lay { pad, picture_h, title_px, title_h, h: top + 36.0 + title_h + 6.0 + text_h + 20.0 + BUTTON_H + pad }
}

/// The dark with the spotlight, and the bubble (or the card) with its words and buttons.
/// Returns what a button or a key asked for.
fn overlay(ui: &mut Ui, v: &View, g: &mut Glide) -> Option<Command> {
    let size = ui.size;
    let screen = Rect::new(0.0, 0.0, size.x, size.y);
    let card = v.part.is_none();
    let w = (if card { CARD_W } else { BUBBLE_W }).min(size.x - 2.0 * MARGIN).max(160.0);
    let lay = layout(ui, v, w);
    // where the spotlight and the bubble are going: round the part, the bubble beside it; a
    // card in the middle with the spotlight shrunk to a point under it (from there it goes
    // to the next part, rather than coming out of nothing); closing, the spotlight opens up
    let part_hole = v.part.map(|p| hole_round(p, size));
    let hole_to = match (v.closing, part_hole) {
        (true, _) => screen.inset(-3.0 * FEATHER),
        (false, Some(h)) => h,
        (false, None) => Rect::new(size.x * 0.5, size.y * 0.5, 0.0, 0.0),
    };
    let (bubble_to, side) = match part_hole {
        Some(h) => place(h, Vec2::new(w, lay.h), size),
        None => (Rect::new((size.x - w) * 0.5, ((size.y - lay.h) * 0.5).max(MARGIN), w, lay.h), Side::Over),
    };
    let hole = glide_rect(ui, "tour-hole", hole_to);
    let bubble = glide_rect(ui, "tour-bubble", bubble_to);
    // (a ring goes out from the spotlight once, as it lands on a part)
    let now = ui.time;
    let travelling = apart(hole, hole_to) > 1.5;
    if g.travelling && !travelling && part_hole.is_some() && !v.closing {
        g.ping = Some(now);
    }
    g.travelling = travelling;
    if !v.closing {
        ui.solid(screen);
    }
    let open = v.open;
    let radius = HOLE_RADIUS.min(hole.w * 0.5).min(hole.h * 0.5);
    let ring = if v.closing { 0.0 } else { smoothstep(hole.w.min(hole.h) / 28.0) * open };
    let pinging = g.ping.map(|t0| (now - t0) / PING_S).filter(|k| *k < 1.0 && ui.motion && ring > 0.01);
    if pinging.is_some() {
        ui.keep_moving();
    } else {
        g.ping = None;
    }
    let p = ui.p();
    dim(p, screen, hole, radius, DIM.alpha(open));
    if ring > 0.01 {
        halo(p, hole, radius, 10.0, accent().alpha(0.22 * ring));
        p.rounded_border(hole, radius, 2.0, accent().alpha(ring));
    }
    if let Some(k) = pinging {
        let e = ease_out_cubic(k);
        p.rounded_border(hole.inset(-PING_REACH * e), radius + PING_REACH * e, 2.0, accent().alpha(0.5 * (1.0 - e) * ring));
    }
    // the bubble, rising a little as the dark comes in
    let b = Rect::new(bubble.x, bubble.y + 8.0 * (1.0 - open), bubble.w, bubble.h);
    p.shadow(Rect::new(b.x, b.y + 16.0, b.w, b.h).inset(6.0), SHEET_RADIUS, 48.0, Color::rgba(0, 0, 0, 0.5 * open));
    p.rounded(b, SHEET_RADIUS, PANEL.alpha(open));
    p.rounded_border(b, SHEET_RADIUS, 1.0, Color::WHITE.alpha(0.1 * open));
    // the arrow at the spotlight, once the bubble has come to its place
    let settled = 1.0 - ((apart(bubble, bubble_to).max(apart(hole, hole_to)) - 2.0) / 28.0).clamp(0.0, 1.0);
    if let Some(tri) = arrow(b, side, hole).filter(|_| !card && !v.closing) {
        let a = settled * open;
        if a > 0.01 {
            p.convex(&tri, PANEL.alpha(a));
            p.stroke(&tri, 1.0, Color::WHITE.alpha(0.1 * a));
        }
    }
    ui.push_clip(b, SHEET_RADIUS);
    let clicked = bubble_inside(ui, b, v, &lay);
    ui.pop_clip();
    if v.closing {
        return None;
    }
    clicked.or_else(|| key_command(&ui.input.keys))
}

/// The bubble's insides: the drawing (a card), where in the tour it is, the title, the words,
/// and the buttons. Returns the button clicked.
fn bubble_inside(ui: &mut Ui, b: Rect, v: &View, lay: &Lay) -> Option<Command> {
    let (open, words) = (v.open, ease_out_cubic(v.words) * v.open);
    let pad = lay.pad;
    let mut y = b.y + pad;
    if let Some(pic) = v.picture {
        let r = Rect::new(b.x + 10.0, b.y + 10.0, b.w - 20.0, lay.picture_h);
        ui.push_clip(r, RADIUS);
        picture(ui, pic, r, words);
        ui.pop_clip();
        y = r.bottom() + 18.0;
    }
    let x = b.x + pad;
    let inner = b.w - 2.0 * pad;
    // (the words come in from a little lower)
    let dy = 6.0 * (1.0 - ease_out_cubic(v.words));
    let chapter = omsi_ui::tr(v.chapter).to_uppercase();
    ui.text_in(&chapter, Rect::new(x, y + dy, inner * 0.7, 16.0), 10.5, Weight::Bold, accent_2().alpha(words), Align::Left);
    let count = format!("{} / {}", v.at + 1, v.total);
    ui.text_in(&count, Rect::new(x + inner * 0.5, y, inner * 0.5, 16.0), 11.5, Weight::Medium, TEXT_DIM.alpha(open), Align::Right);
    // how far along: a line filling up
    let track = Rect::new(x, y + 24.0, inner, 2.0);
    let done = ui.spring(id_of("tour-progress"), (v.at + 1) as f32 / v.total.max(1) as f32, Feel::SLIDE).clamp(0.0, 1.0);
    ui.p().rounded(track, 1.0, Color::WHITE.alpha(0.08 * open));
    ui.p().rounded(Rect::new(track.x, track.y, track.w * done, track.h), 1.0, accent().alpha(open));
    let ty = y + 36.0 + dy;
    ui.paragraph(&v.title, Vec2::new(x, ty), inner, lay.title_px, Weight::Bold, TEXT.alpha(words));
    ui.paragraph(v.text, Vec2::new(x, ty + lay.title_h + 6.0), inner, TEXT_PX, Weight::Regular, TEXT_SOFT.alpha(words));
    // the buttons, once the bubble is more there than not; laid out from the top as the words
    // are (growing into a card, the bubble shows them as it grows instead of pushing its
    // buttons over the words)
    if v.closing || open < 0.5 {
        return None;
    }
    let row = b.y + lay.h - pad - BUTTON_H;
    let next = if v.first && !v.last {
        "Show me around"
    } else if v.last && v.company {
        "Off to work"
    } else if v.last {
        "Let's drive"
    } else {
        "Next"
    };
    let nw = (ui.width(next, 13.0, Weight::Bold) + 36.0).max(92.0);
    let next_r = Rect::new(b.right() - pad - nw, row, nw, BUTTON_H);
    let mut cmd = None;
    if ui.button("tour-next", next_r, next, None, ButtonKind::Primary) {
        cmd = Some(Command::Next);
    }
    if !v.first {
        let bw = ui.width("Back", 13.0, Weight::Bold) + 56.0;
        if ui.button("tour-back", Rect::new(next_r.x - 8.0 - bw, row, bw, BUTTON_H), "Back", Some("chevron_left"), ButtonKind::Normal) {
            cmd = Some(Command::Back);
        }
    }
    if !v.last {
        let sw = ui.width("Skip", 13.0, Weight::Medium) + 28.0;
        if ui.button("tour-skip", Rect::new(x - 14.0, row, sw, BUTTON_H), "Skip", None, ButtonKind::Ghost) {
            cmd = Some(Command::Skip);
        }
    }
    cmd
}

// --- the cards' drawings -------------------------------------------------------------------
//
// Each card's drawing is a small illustration in the launcher's own language: the night city
// of its map (blocks a shade lighter than the ground, main streets on a casing, a river and a
// park), the route as the map and openOMSI's mark draw it - the blue band on its casing, the
// stops' signs, the terminus ringed in the plate's yellow - a bus seen from above, and the
// game's screens as small devices with some depth to them. Each is a whole picture standing
// still (the setting "animations" off) and has one calm motion when it is on, worked out by a
// pure function of the time since the card came (`ride`, `celebration`, `board_focus`,
// `typing`, `refresh`). Everything is sized by the drawing's `Canvas`, so it scales with the
// card; a device's screen is the only clip.

/// The size a drawing is made at (it is scaled into the card's room, kept in proportion).
const PIC_W: f32 = 520.0;
const PIC_H: f32 = 196.0;

/// The route as the launcher's map and openOMSI's mark draw it, and the calmer part of it the
/// bus has still to drive.
const ROUTE_FILL: Color = Color::rgba(74, 144, 255, 1.0);
const ROUTE_CASING: Color = Color::rgba(18, 58, 107, 1.0);
const ROUTE_AHEAD: Color = Color::rgba(40, 78, 142, 1.0);
const ROUTE_AHEAD_CASING: Color = Color::rgba(12, 30, 58, 1.0);
/// The light at the opening's pen: a trail, a glint on glass.
const PEN: Color = Color::rgba(222, 236, 255, 1.0);
/// The night city: its ground, its blocks (some a little lighter), its main streets on their
/// casing, its river (lighter in its middle) and shore, its park and trees, the warm light of
/// lamps and headlights, and the dark at a drawing's edges.
const NIGHT: Color = Color::rgba(13, 19, 34, 1.0);
const BLOCK: Color = Color::rgba(21, 30, 50, 1.0);
const BLOCK_LIT: Color = Color::rgba(26, 37, 61, 1.0);
const STREET: Color = Color::rgba(31, 41, 64, 1.0);
const STREET_CASING: Color = Color::rgba(8, 12, 23, 1.0);
const WATER: Color = Color::rgba(10, 30, 60, 1.0);
const WATER_LIGHT: Color = Color::rgba(15, 41, 78, 1.0);
const SHORE: Color = Color::rgba(30, 60, 100, 1.0);
const PARK: Color = Color::rgba(14, 38, 42, 1.0);
const TREE: Color = Color::rgba(20, 52, 55, 1.0);
const WARM: Color = Color::rgba(255, 220, 156, 1.0);
const SHADE: Color = Color::rgba(3, 5, 12, 1.0);
/// The bus from above: its roof (lighter in the middle, it is rounded), its rim, its glass, the
/// units on its roof, its rear lights.
const BUS_ROOF: Color = Color::rgba(241, 244, 249, 1.0);
const BUS_SIDE: Color = Color::rgba(176, 186, 205, 1.0);
const BUS_RIM: Color = Color::rgba(58, 70, 94, 1.0);
const BUS_GLASS: Color = Color::rgba(13, 21, 37, 1.0);
const BUS_UNIT: Color = Color::rgba(206, 213, 227, 1.0);
const BUS_UNIT_EDGE: Color = Color::rgba(148, 158, 180, 1.0);
const REAR_LIGHT: Color = Color::rgba(232, 64, 52, 1.0);
/// A device: its body lit from above, its glass; a key's top and its wall; a label on the map.
const BODY_TOP: Color = Color::rgba(54, 62, 82, 1.0);
const BODY_BOTTOM: Color = Color::rgba(24, 29, 41, 1.0);
const GLASS: Color = Color::rgba(5, 8, 15, 1.0);
const KEY_TOP: Color = Color::rgba(70, 81, 106, 1.0);
const KEY_LOW: Color = Color::rgba(48, 57, 77, 1.0);
const KEY_WALL: Color = Color::rgba(18, 23, 34, 1.0);
const CHIP: Color = Color::rgba(24, 31, 49, 1.0);
/// The game's dusk through the windscreen: the sky, the town against it, the ground, the road.
const SKY_TOP: Color = Color::rgba(11, 18, 37, 1.0);
const SKY_LOW: Color = Color::rgba(43, 57, 93, 1.0);
const TOWN_FAR: Color = Color::rgba(27, 36, 60, 1.0);
const TOWN_NEAR: Color = Color::rgba(15, 21, 36, 1.0);
const DUSK_GROUND: Color = Color::rgba(15, 20, 31, 1.0);
const ROAD_FAR: Color = Color::rgba(46, 54, 74, 1.0);
const ROAD_NEAR: Color = Color::rgba(26, 31, 44, 1.0);
/// The duty board's (`nav_duty`): its rail ahead of the bus and behind it, the stop it heads for.
const BOARD_RAIL: Color = Color::rgba(63, 136, 232, 1.0);
const BOARD_RAIL_DONE: Color = Color::rgba(74, 84, 98, 1.0);
const NOW: Color = Color::rgba(240, 180, 41, 1.0);
/// An IBIS's display: amber light on a dark glass.
const LED: Color = Color::rgba(255, 178, 62, 1.0);
const LED_GROUND: Color = Color::rgba(25, 17, 6, 1.0);

/// A drawing's own coordinates laid into the room it has: where its corner is, its scale, and
/// how much of it is seen (it comes in with the words).
#[derive(Clone, Copy)]
struct Canvas {
    o: Vec2,
    k: f32,
    a: f32,
}

impl Canvas {
    fn new(r: Rect, a: f32) -> Canvas {
        let k = (r.w / PIC_W).min(r.h / PIC_H).max(0.05);
        Canvas { o: r.center() - Vec2::new(PIC_W, PIC_H) * k * 0.5, k, a }
    }
    /// Units `s` times the size of these, their 0, 0 at `x`, `y` of these (the city in a screen).
    fn inner(&self, x: f32, y: f32, s: f32) -> Canvas {
        Canvas { o: self.at(x, y), k: self.k * s, a: self.a }
    }
    fn at(&self, x: f32, y: f32) -> Vec2 {
        self.o + Vec2::new(x, y) * self.k
    }
    fn pt(&self, q: Vec2) -> Vec2 {
        self.o + q * self.k
    }
    fn pts(&self, q: &[Vec2]) -> Vec<Vec2> {
        q.iter().map(|q| self.pt(*q)).collect()
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(self.o.x + x * self.k, self.o.y + y * self.k, w * self.k, h * self.k)
    }
    fn len(&self, v: f32) -> f32 {
        v * self.k
    }
    fn ink(&self, c: Color) -> Color {
        c.alpha(self.a)
    }
    fn line(&self, pts: &[(f32, f32)]) -> Vec<Vec2> {
        pts.iter().map(|(x, y)| self.at(*x, *y)).collect()
    }
    /// Type `px` units tall, on half pixels (a size of its own each frame as the card grows
    /// would fill the atlas).
    fn font(&self, px: f32) -> f32 {
        (px * self.k * 2.0).round() * 0.5
    }
}

thread_local! {
    /// The drawing on show, when it came, and when it was drawn last (see `clock`).
    static SHOWN: std::cell::Cell<Option<(Picture, f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Seconds since the card with `pic` came: its motion starts from its beginning each time a
/// card shows it.
fn clock(pic: Picture, now: f32) -> f32 {
    SHOWN.with(|s| {
        let start = match s.get() {
            Some((p, start, seen)) if p == pic && (0.0..0.25).contains(&(now - seen)) => start,
            _ => now,
        };
        s.set(Some((pic, start, now)));
        now - start
    })
}

/// A card's drawing in `r`, `a` of it seen.
fn picture(ui: &mut Ui, pic: Picture, r: Rect, a: f32) {
    let c = Canvas::new(r, a);
    let t = ui.motion.then(|| clock(pic, ui.time));
    if t.is_some() {
        ui.keep_moving();
    }
    match pic {
        Picture::Welcome => city_drawing(ui, c, r, t, false),
        Picture::Arrived => city_drawing(ui, c, r, t, true),
        Picture::Navigator => navigator_drawing(ui, c, r, t),
        Picture::CityMap => city_map_drawing(ui, c, r, t),
        Picture::Companion => companion_drawing(ui, c, r, t),
    }
}

// --- the night city ------------------------------------------------------------------------

/// Where the gaps between the city's blocks run (the drawing's units): the streets across it,
/// and the ones down it. The route's streets are among them; the city reaches past the
/// drawing so that a wider room - and a map that shows more of it - is filled.
const ACROSS: [f32; 11] = [-66.0, -26.0, 16.0, 58.0, 94.0, 122.0, 150.0, 180.0, 212.0, 246.0, 282.0];
const DOWN: [f32; 20] = [-98.0, -58.0, -18.0, 22.0, 64.0, 106.0, 146.0, 176.0, 206.0, 240.0, 280.0, 330.0, 368.0, 404.0, 440.0, 476.0, 514.0, 552.0, 590.0, 630.0];
/// A minor street's width (the gap between two blocks), a main street's and its casing's, the
/// river's.
const ALLEY: f32 = 5.2;
const STREET_W: f32 = 14.0;
const STREET_EDGE: f32 = 1.6;
const RIVER_W: f32 = 30.0;
/// The park, between four streets.
const PARK_AREA: Rect = Rect::new(24.6, 18.6, 78.8, 72.8);
/// The route's streets, turning at these corners (rounded), and its stops - their names on the
/// signs of OMSI 2's own map - the last its terminus.
const ROUTE_LINE: [(f32, f32); 6] = [(-50.0, 150.0), (106.0, 150.0), (166.0, 94.0), (360.0, 94.0), (396.0, 58.0), (476.0, 58.0)];
const CORNER_R: f32 = 22.0;
const STOP_AT: [(f32, f32); 4] = [(78.0, 150.0), (244.0, 94.0), (330.0, 94.0), (476.0, 58.0)];
const STOP_NAMES: [&str; 4] = ["Rathaus Spandau", "Altstädter Ring", "Zitadelle", "Hakenfelde"];
/// The route's band and casing, a stop's sign and the terminus's (as the mark has them, to the
/// route's width).
const ROUTE_W: f32 = 8.0;
const ROUTE_EDGE: f32 = 2.4;
const SIGN_R: f32 = 7.2;
const TERMINUS_R: f32 = 8.6;
/// Half the bus seen from above: its length and its width.
const BUS_HALF: Vec2 = Vec2::new(22.0, 8.4);

/// A little hash of two numbers: the city's blocks and its town's outline differ, the same
/// every frame.
fn hash(i: usize, j: usize) -> u32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B1) ^ (j as u32).wrapping_mul(0x85EB_CA77) ^ 0x2545_F491;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}

/// A path through `pts` with its corners rounded by `radius` (less where a leg is short): the
/// way a street turns.
fn rounded_path(pts: &[Vec2], radius: f32) -> Path {
    let mut path = Path::new(pts[0]);
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let r = radius.min(a.distance(b) * 0.5).min(b.distance(c) * 0.5);
        let (din, dout) = ((b - a).normalize_or_zero(), (c - b).normalize_or_zero());
        let (p0, p1) = (b - din * r, b + dout * r);
        path.line_to(p0);
        // (handles a little over half way to the corner: near enough a circle's arc)
        path.cubic_to(p0 + din * r * 0.55, p1 - dout * r * 0.55, p1);
    }
    path.line_to(pts[pts.len() - 1]);
    path
}

fn v2(q: (f32, f32)) -> Vec2 {
    Vec2::new(q.0, q.1)
}

/// The route in the drawing's units.
fn route_path() -> Path {
    rounded_path(&ROUTE_LINE.map(v2), CORNER_R)
}

/// The river, from the top of the city to its bottom.
fn river_path() -> Path {
    let mut p = Path::new(Vec2::new(296.0, -80.0));
    p.cubic_to(Vec2::new(272.0, 10.0), Vec2::new(306.0, 60.0), Vec2::new(290.0, 110.0)).cubic_to(Vec2::new(276.0, 150.0), Vec2::new(304.0, 200.0), Vec2::new(294.0, 300.0));
    p
}

/// The main streets: the route's (on past its terminus), one down the city and one across it.
fn main_streets() -> [Vec<Vec2>; 3] {
    let mut route = ROUTE_LINE.map(v2).to_vec();
    route[0].x = -120.0;
    route[5].x = 660.0;
    [rounded_path(&route, CORNER_R).points().to_vec(), vec![Vec2::new(206.0, -90.0), Vec2::new(206.0, 320.0)], vec![Vec2::new(-120.0, 180.0), Vec2::new(660.0, 180.0)]]
}

/// How far `q` is from the line through `pts`, and how far along it the nearest point is.
fn nearest(pts: &[Vec2], q: Vec2) -> (f32, f32) {
    let (mut best, mut along, mut run) = (f32::MAX, 0.0, 0.0);
    for w in pts.windows(2) {
        let ab = w[1] - w[0];
        let l = ab.length();
        let f = if l > 0.0 { ((q - w[0]).dot(ab) / (l * l)).clamp(0.0, 1.0) } else { 0.0 };
        let d = (w[0] + ab * f).distance(q);
        if d < best {
            (best, along) = (d, run + f * l);
        }
        run += l;
    }
    (best, along)
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// The city's blocks (the drawing's units), and whether each is one of the lighter ones: one
/// between every two streets, some cut in two by an alley, none in the river or the park.
fn blocks() -> Vec<(Rect, bool)> {
    let river = river_path();
    let wet = |r: Rect| {
        let corners = [Vec2::new(r.x, r.y), Vec2::new(r.right(), r.y), Vec2::new(r.x, r.bottom()), Vec2::new(r.right(), r.bottom()), r.center()];
        corners.iter().any(|q| nearest(river.points(), *q).0 < RIVER_W * 0.5 + 3.0)
    };
    let half = ALLEY * 0.5;
    let mut out = Vec::new();
    for i in 0..DOWN.len() - 1 {
        for j in 0..ACROSS.len() - 1 {
            let r = Rect::new(DOWN[i] + half, ACROSS[j] + half, DOWN[i + 1] - DOWN[i] - ALLEY, ACROSS[j + 1] - ACROSS[j] - ALLEY);
            if overlap(r, PARK_AREA.inset(-half)) || wet(r) {
                continue;
            }
            let h = hash(i, j);
            let lit = h % 4 == 0;
            let cut = 0.38 + ((h >> 8) % 6) as f32 * 0.05;
            if h % 3 == 0 && r.w > 30.0 {
                let w = (r.w - ALLEY * 0.7) * cut;
                out.push((Rect::new(r.x, r.y, w, r.h), lit));
                out.push((Rect::new(r.x + w + ALLEY * 0.7, r.y, r.w - w - ALLEY * 0.7, r.h), !lit && h % 5 == 0));
            } else if h % 5 == 1 && r.h > 30.0 {
                let hh = (r.h - ALLEY * 0.7) * cut;
                out.push((Rect::new(r.x, r.y, r.w, hh), lit));
                out.push((Rect::new(r.x, r.y + hh + ALLEY * 0.7, r.w, r.h - hh - ALLEY * 0.7), lit));
            } else {
                out.push((r, lit));
            }
        }
    }
    out
}

/// The park's trees: where they stand and how large their crowns are.
fn trees() -> Vec<(Vec2, f32)> {
    let mut out = Vec::new();
    for i in 0..5 {
        for j in 0..4 {
            let h = hash(i + 40, j + 40);
            if h % 5 == 0 {
                continue;
            }
            let jitter = Vec2::new(((h >> 4) % 7) as f32 - 3.0, ((h >> 9) % 7) as f32 - 3.0);
            let q = Vec2::new(PARK_AREA.x + 11.0 + i as f32 * 14.2, PARK_AREA.y + 11.0 + j as f32 * 17.0) + jitter;
            out.push((q, 5.0 + ((h >> 13) % 4) as f32 * 0.8));
        }
    }
    out
}

/// The night city under a drawing (`c` places its units): its blocks (a little raised), the
/// park, the river under its bridges, the main streets, and the city's light (`glow` how bright
/// it is as it breathes) - everything but the route.
fn city(p: &mut Painter, c: Canvas, glow: f32) {
    // (laid out once: they are the same every frame)
    static BLOCKS: std::sync::OnceLock<Vec<(Rect, bool)>> = std::sync::OnceLock::new();
    let blocks = BLOCKS.get_or_init(blocks);
    let drop = Vec2::new(0.5, 1.6);
    for (r, _) in blocks {
        p.rounded(c.rect(r.x + drop.x, r.y + drop.y, r.w, r.h), c.len(3.0), c.ink(SHADE.alpha(0.45)));
    }
    for (r, lit) in blocks {
        p.rounded(c.rect(r.x, r.y, r.w, r.h), c.len(3.0), c.ink(if *lit { BLOCK_LIT } else { BLOCK }));
    }
    let park = PARK_AREA;
    p.rounded(c.rect(park.x, park.y, park.w, park.h), c.len(9.0), c.ink(PARK));
    for (q, r) in trees() {
        p.circle(c.pt(q + drop * 0.6), c.len(r), c.ink(SHADE.alpha(0.35)));
        p.circle(c.pt(q), c.len(r), c.ink(if r > 6.0 { TREE } else { TREE.lighten(0.03) }));
    }
    let river = river_path();
    let water = c.pts(river.points());
    p.stroke_edged(&water, c.len(RIVER_W), c.len(1.3), c.ink(WATER), c.ink(SHORE));
    p.stroke(&water, c.len(RIVER_W * 0.4), c.ink(WATER_LIGHT.alpha(0.6)));
    let streets = main_streets();
    for s in streets.iter().rev() {
        p.stroke_edged(&c.pts(s), c.len(STREET_W), c.len(STREET_EDGE), c.ink(STREET), c.ink(STREET_CASING));
    }
    // the bridges' railings, where the streets across cross the river
    for y in [94.0, 180.0] {
        let x = river.points().iter().min_by(|a, b| (a.y - y).abs().total_cmp(&(b.y - y).abs())).map_or(290.0, |q| q.x);
        for side in [-1.0, 1.0] {
            let yy = y + side * (STREET_W * 0.5 + STREET_EDGE + 0.8);
            p.stroke(&[c.at(x - RIVER_W * 0.62, yy), c.at(x + RIVER_W * 0.62, yy)], c.len(1.3), c.ink(SHORE.lighten(0.12)));
        }
    }
    // the city's light, round its middle
    soft_light(p, c.at(250.0, 98.0), Vec2::new(c.len(260.0), c.len(130.0)), c.ink(accent().alpha(0.085 * glow)));
}

/// How bright the city's light is `t` seconds in: breathing slowly round its rest (1 standing
/// still).
fn breathing(t: Option<f32>) -> f32 {
    t.map_or(1.0, |t| 1.0 + 0.22 * (t * std::f32::consts::TAU / 6.5).sin())
}

// --- shapes --------------------------------------------------------------------------------

/// A soft light: an ellipse of `radii` round `centre`, `c` in its middle fading to nothing at
/// its rim (as `Painter::radial`, stretched).
fn soft_light(p: &mut Painter, centre: Vec2, radii: Vec2, c: Color) {
    const N: usize = 48;
    const RINGS: usize = 8;
    let clear = c.alpha(0.0);
    let col = |f: f32| c.mix(clear, f * f * (3.0 - 2.0 * f));
    let dir = |k: usize| {
        let a = std::f32::consts::TAU * (k % N) as f32 / N as f32;
        Vec2::new(a.cos() * radii.x, a.sin() * radii.y)
    };
    for j in 0..RINGS {
        let (f0, f1) = (j as f32 / RINGS as f32, (j + 1) as f32 / RINGS as f32);
        let (c0, c1) = (col(f0), col(f1));
        for k in 0..N {
            let (d0, d1) = (dir(k), dir(k + 1));
            if j == 0 {
                p.tri(centre, centre + d0 * f1, centre + d1 * f1, c0, c1, c1);
            } else {
                p.tri(centre + d0 * f0, centre + d0 * f1, centre + d1 * f1, c0, c1, c1);
                p.tri(centre + d0 * f0, centre + d1 * f1, centre + d1 * f0, c0, c1, c0);
            }
        }
    }
}

/// The dark round a drawing's edges, deepest in its corners.
fn vignette(p: &mut Painter, r: Rect, c: Color) {
    let clear = c.alpha(0.0);
    let (bw, bh) = (r.w * 0.16, r.h * 0.34);
    p.gradient(Rect::new(r.x, r.y, r.w, bh), c, clear);
    p.gradient(Rect::new(r.x, r.bottom() - bh, r.w, bh), clear, c);
    p.gradient_h(Rect::new(r.x, r.y, bw, r.h), c, clear);
    p.gradient_h(Rect::new(r.right() - bw, r.y, bw, r.h), clear, c);
}

/// A rounded box's outline round `centre` (`half` its half size), `CORNER` + 1 points a corner
/// whatever its size (two of them make a soft edge point by point).
fn box_at(centre: Vec2, half: Vec2, radius: f32) -> Vec<Vec2> {
    outline(Rect::new(centre.x - half.x, centre.y - half.y, 2.0 * half.x, 2.0 * half.y), radius)
}

/// Points of a thing's own (x ahead, y to its right) put at `at` heading `dir`, `k` screen
/// units to one of its own.
fn turn(pts: &[Vec2], at: Vec2, dir: Vec2, k: f32) -> Vec<Vec2> {
    let side = Vec2::new(-dir.y, dir.x);
    pts.iter().map(|q| at + (dir * q.x + side * q.y) * k).collect()
}

/// A convex shape in `c`, with a soft edge going out from it to `outer` (an outline of as many
/// points): a shadow, a glow.
fn soft_fill(p: &mut Painter, inner: &[Vec2], outer: &[Vec2], c: Color) {
    p.convex(inner, c);
    let clear = c.alpha(0.0);
    let n = inner.len().min(outer.len());
    for k in 0..n {
        let j = (k + 1) % n;
        p.tri(inner[k], outer[k], outer[j], c, clear, clear);
        p.tri(inner[k], outer[j], inner[j], c, clear, c);
    }
}

/// The part of the convex polygon `poly` inside `r`.
fn clip_poly(poly: &[Vec2], r: Rect) -> Vec<Vec2> {
    let mut out = poly.to_vec();
    // (inside an edge where the point's projection on its normal is at least its offset)
    for (n, o) in [(Vec2::X, r.x), (-Vec2::X, -r.right()), (Vec2::Y, r.y), (-Vec2::Y, -r.bottom())] {
        let input = std::mem::take(&mut out);
        for i in 0..input.len() {
            let (a, b) = (input[i], input[(i + 1) % input.len()]);
            let (da, db) = (a.dot(n) - o, b.dot(n) - o);
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(a + (b - a) * (da / (da - db)));
            }
        }
    }
    out
}

/// The light on a screen's glass: a soft band across its top left, brightest along its middle,
/// and a thinner one beside it.
fn sheen(p: &mut Painter, screen: Rect, radius: f32, a: f32) {
    let area = screen.inset(radius * 0.45);
    let along = Vec2::new(-0.55, 1.0).normalize();
    let across = Vec2::new(along.y, -along.x);
    let big = (area.w + area.h) * 2.0;
    for (at, half, strength) in [(0.14, 0.09, 0.04), (0.26, 0.025, 0.028)] {
        let mid = Vec2::new(area.x + area.w * at, area.y);
        let half = area.w.min(area.h * 2.4) * half;
        let col = |q: Vec2| PEN.alpha(a * strength * (1.0 - ((q - mid).dot(across) / half).abs()).clamp(0.0, 1.0));
        for (from, to) in [(-half, 0.0), (0.0, half)] {
            let band = [mid + across * from - along * big, mid + across * to - along * big, mid + across * to + along * big, mid + across * from + along * big];
            let poly = clip_poly(&band, area);
            for k in 1..poly.len().saturating_sub(1) {
                p.tri(poly[0], poly[k], poly[k + 1], col(poly[0]), col(poly[k]), col(poly[k + 1]));
            }
        }
    }
}

/// A four-pointed sparkle at `at`, `r` from its middle to a point.
fn sparkle(p: &mut Painter, at: Vec2, r: f32, c: Color) {
    p.circle(at, r * 0.9, c.alpha(0.12));
    for d in [Vec2::X, Vec2::Y] {
        let s = Vec2::new(-d.y, d.x) * r * 0.2;
        p.convex(&[at + d * r, at + s, at - d * r, at - s], c);
    }
}

/// Words in a drawing: as text where they can be read, as a soft bar of their length where they
/// would be too small to.
#[allow(clippy::too_many_arguments)]
fn words(ui: &mut Ui, c: Canvas, text: &str, r: Rect, px: f32, weight: Weight, ink: Color, align: Align) {
    let size = c.font(px);
    if size * ui.scale >= 6.5 {
        ui.text_in(text, r, size, weight, c.ink(ink), align);
        return;
    }
    let w = ui.width(text, size, weight).min(r.w);
    let h = (size * 0.55).max(1.0);
    let x = match align {
        Align::Left => r.x,
        Align::Center => r.center().x - w * 0.5,
        Align::Right => r.right() - w,
    };
    ui.p().rounded(Rect::new(x, r.center().y - h * 0.5, w, h), h * 0.5, c.ink(ink.alpha(0.4)));
}

/// A line's number on its yellow plate (as the duty board prints it) from `x`, centred on `cy`
/// (units), `h` tall; returns its width.
fn plate(ui: &mut Ui, c: Canvas, line: &str, x: f32, cy: f32, h: f32) -> f32 {
    let w = (h * 0.62 * line.chars().count() as f32 + h * 0.55).max(h * 1.35);
    let r = c.rect(x, cy - h * 0.5, w, h);
    ui.p().rounded(r, c.len(h * 0.26), c.ink(LINE));
    words(ui, c, line, r, h * 0.66, Weight::Black, ON_LINE, Align::Center);
    w
}

/// A device's body over `body` (units): a soft shadow under it, a dark body lit from above with
/// a light edge, and its glass. Returns its screen inside the bezel, and the screen's corners.
fn device(p: &mut Painter, c: Canvas, body: Rect, radius: f32, bezel: f32) -> (Rect, f32) {
    let b = c.rect(body.x, body.y, body.w, body.h);
    let (rad, bez) = (c.len(radius), c.len(bezel));
    p.shadow(Rect::new(b.x + c.len(5.0), b.y + c.len(14.0), b.w - c.len(10.0), b.h - c.len(6.0)), rad, c.len(30.0), c.ink(Color::BLACK.alpha(0.62)));
    p.rounded_gradient(b, rad, c.ink(BODY_TOP), c.ink(BODY_BOTTOM));
    p.rounded_border(b, rad, 1.0, c.ink(Color::WHITE.alpha(0.15)));
    let screen = b.inset(bez);
    let sr = (rad - bez * 0.7).max(c.len(2.0));
    p.rounded(screen, sr, c.ink(GLASS));
    (screen, sr)
}

/// The ground behind a card's devices: the night, a soft light behind them at `light`, darker
/// towards the edges.
fn backdrop(p: &mut Painter, c: Canvas, room: Rect, light: Vec2) {
    p.rect(room, c.ink(NIGHT));
    soft_light(p, light, Vec2::new(c.len(280.0), c.len(140.0)), c.ink(accent().alpha(0.12)));
    vignette(p, room, c.ink(SHADE.alpha(0.7)));
}

/// A key as on a keyboard: its top lit from above on its wall, its word crisp on it.
fn keycap(ui: &mut Ui, c: Canvas, x: f32, y: f32, w: f32, label: &str) {
    let whole = c.rect(x, y, w, 27.0);
    let top = c.rect(x, y, w, 23.0);
    let rad = c.len(6.0);
    let p = ui.p();
    p.shadow(Rect::new(whole.x + c.len(1.0), whole.y + c.len(4.0), whole.w - c.len(2.0), whole.h), rad, c.len(12.0), c.ink(Color::BLACK.alpha(0.55)));
    p.rounded(whole, rad, c.ink(KEY_WALL));
    p.rounded_gradient(top, rad, c.ink(KEY_TOP), c.ink(KEY_LOW));
    p.rounded_border(top, rad, 1.0, c.ink(Color::WHITE.alpha(0.12)));
    words(ui, c, label, top, 11.5, Weight::Bold, TEXT, Align::Center);
}

/// "Shift + key" from `x`, `y` on.
fn shortcut(ui: &mut Ui, c: Canvas, x: f32, y: f32, key: &str) {
    keycap(ui, c, x, y, 58.0, "Shift");
    ui.text_in("+", c.rect(x + 58.0, y, 22.0, 23.0), c.font(14.0), Weight::Bold, c.ink(TEXT_SOFT), Align::Center);
    keycap(ui, c, x + 80.0, y, 30.0, key);
}

// --- the bus -------------------------------------------------------------------------------

/// A bus seen from above at `at`, heading `dir`, `k` screen units to one of its own, `a` of it
/// seen: its shadow on the street, the white roof (rounded, lighter in the middle) on a crisp
/// rim, the windscreen wrapping round the front with a glint on it, the lit destination display
/// over it, the units on the roof, the mirrors, and its lights.
fn bus_from_above(p: &mut Painter, at: Vec2, dir: Vec2, k: f32, a: f32) {
    let put = |pts: &[Vec2]| turn(pts, at, dir, k);
    let (hl, hw) = (BUS_HALF.x, BUS_HALF.y);
    // (in shares of its half length and half width)
    let f = |x: f32, y: f32| Vec2::new(x * hl, y * hw);
    let half = |x: f32, y: f32| Vec2::new(x * hl, y * hw);
    // its shadow, cast a little down and to the right of the city's light
    let off = Vec2::new(1.0, 2.6) * k;
    let shadow_in: Vec<Vec2> = put(&box_at(Vec2::ZERO, BUS_HALF - Vec2::splat(1.0), 4.0)).into_iter().map(|q| q + off).collect();
    let shadow_out: Vec<Vec2> = put(&box_at(Vec2::ZERO, BUS_HALF + Vec2::splat(5.5), 11.0)).into_iter().map(|q| q + off).collect();
    soft_fill(p, &shadow_in, &shadow_out, Color::BLACK.alpha(0.55 * a));
    for s in [-1.0, 1.0] {
        p.convex(&put(&[f(0.79, s * 0.95), f(0.87, s * 0.95), f(0.85, s * 1.26), f(0.79, s * 1.26)]), BUS_RIM.alpha(a));
    }
    // the body on its rim, the roof lighter along its middle
    p.convex(&put(&box_at(Vec2::ZERO, BUS_HALF, hw * 0.62)), BUS_RIM.alpha(a));
    let roof_local = box_at(Vec2::ZERO, BUS_HALF - Vec2::splat(1.0), hw * 0.5);
    let roof = put(&roof_local);
    let shade = |q: Vec2| BUS_ROOF.mix(BUS_SIDE, (q.y.abs() / (hw - 1.0)).powi(2)).alpha(a);
    for i in 0..roof.len() {
        let j = (i + 1) % roof.len();
        p.tri(at, roof[i], roof[j], BUS_ROOF.alpha(a), shade(roof_local[i]), shade(roof_local[j]));
    }
    // the windscreen round the front, a glint on it, the lit destination display over it; the
    // rear window
    p.convex(&put(&[f(0.665, -0.8), f(0.885, -0.88), f(0.955, -0.7), f(0.955, 0.7), f(0.885, 0.88), f(0.665, 0.8)]), BUS_GLASS.alpha(a));
    p.convex(&put(&[f(0.71, -0.52), f(0.75, -0.64), f(0.905, 0.22), f(0.865, 0.34)]), PEN.alpha(0.32 * a));
    p.convex(&put(&[f(0.585, -0.66), f(0.625, -0.66), f(0.625, 0.66), f(0.585, 0.66)]), LINE.alpha(a));
    p.convex(&put(&[f(-0.958, -0.64), f(-0.9, -0.74), f(-0.9, 0.74), f(-0.958, 0.64)]), BUS_GLASS.alpha(0.85 * a));
    // the units on its roof: the air conditioning, a smaller one, a hatch
    for (x, len) in [(0.15, 0.27), (-0.53, 0.17)] {
        p.convex(&put(&box_at(f(x, 0.0), half(len, 0.6), 2.2)), BUS_UNIT_EDGE.alpha(a));
        p.convex(&put(&box_at(f(x, 0.0), half(len, 0.6) - Vec2::splat(0.7), 1.6)), BUS_UNIT.alpha(a));
    }
    p.convex(&put(&box_at(f(-0.19, 0.0), Vec2::new(1.7, 2.0), 0.6)), BUS_SIDE.alpha(a));
    for s in [-1.0, 1.0] {
        let lights = put(&[f(0.972, s * 0.72), f(-0.975, s * 0.72)]);
        p.circle(lights[0], 1.15 * k, WARM.alpha(a));
        p.circle(lights[1], 1.0 * k, REAR_LIGHT.alpha(a));
    }
}

/// The light the bus throws ahead of it: a soft fan from its front, fading out ahead and to the
/// sides.
fn headlights(p: &mut Painter, at: Vec2, dir: Vec2, k: f32, c: Color) {
    let side = Vec2::new(-dir.y, dir.x);
    let front = at + dir * (BUS_HALF.x - 0.8) * k;
    // rows going out ahead (how far, how wide, how bright), across each a fan feathered at its
    // sides
    let rows = [(0.0, 0.95, 1.0), (22.0, 1.7, 0.4), (58.0, 2.8, 0.0)];
    let across = [(-1.0f32, 0.0f32), (-0.5, 1.0), (0.5, 1.0), (1.0, 0.0)];
    let point = |(ahead, wide, _): (f32, f32, f32), x: f32| front + (dir * ahead + side * x * wide * BUS_HALF.y) * k;
    for r in 0..rows.len() - 1 {
        let (r0, r1) = (rows[r], rows[r + 1]);
        for i in 0..across.len() - 1 {
            let (a0, a1) = (across[i], across[i + 1]);
            let (p00, p01, p10, p11) = (point(r0, a0.0), point(r0, a1.0), point(r1, a0.0), point(r1, a1.0));
            let (c00, c01, c10, c11) = (c.alpha(r0.2 * a0.1), c.alpha(r0.2 * a1.1), c.alpha(r1.2 * a0.1), c.alpha(r1.2 * a1.1));
            p.tri(p00, p01, p11, c00, c01, c11);
            p.tri(p00, p11, p10, c00, c11, c10);
        }
    }
}

/// A soft light along `pts`, `w` wide: `c` along its middle, fading to nothing at its sides (a lit
/// route's glow on the map under it).
fn glow_along(p: &mut Painter, pts: &[Vec2], w: f32, c: Color) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let clear = c.alpha(0.0);
    let side = |i: usize| {
        let before = if i > 0 { (pts[i] - pts[i - 1]).normalize_or_zero() } else { Vec2::ZERO };
        let after = if i + 1 < n { (pts[i + 1] - pts[i]).normalize_or_zero() } else { Vec2::ZERO };
        let d = (before + after).normalize_or_zero();
        Vec2::new(-d.y, d.x) * w * 0.5
    };
    for i in 0..n - 1 {
        let (a, b, sa, sb) = (pts[i], pts[i + 1], side(i), side(i + 1));
        for s in [1.0, -1.0] {
            p.tri(a, a + sa * s, b + sb * s, c, clear, clear);
            p.tri(a, b + sb * s, b, c, clear, c);
        }
    }
}

/// A light along `pts` from its tail to its head: thin and clear at the tail, `w` wide and `c`
/// at the head.
fn trail(p: &mut Painter, pts: &[Vec2], w: f32, c: Color) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let mut run = vec![0.0f32; n];
    for i in 1..n {
        run[i] = run[i - 1] + pts[i].distance(pts[i - 1]);
    }
    let total = run[n - 1].max(1e-3);
    let mut last: Option<(Vec2, Vec2, Color)> = None;
    for i in 0..n {
        let f = run[i] / total;
        let d = if i + 1 < n { pts[i + 1] - pts[i] } else { pts[i] - pts[i - 1] }.normalize_or_zero();
        let side = Vec2::new(-d.y, d.x) * w * 0.5 * (0.2 + 0.8 * f);
        let col = c.alpha(f * f);
        let (l, r) = (pts[i] + side, pts[i] - side);
        if let Some((pl, pr, pc)) = last {
            p.tri(pl, l, r, pc, col, col);
            p.tri(pl, r, pr, pc, col, pc);
        }
        last = Some((l, r, col));
    }
}

/// A stop's name over its sign at `at` (screen), `above` over its middle: a small dark label
/// with the stop's sign in it and a point at the stop, kept inside `room`, coming in as `shown`
/// goes to 1 (it rises a little into its place).
#[allow(clippy::too_many_arguments)]
fn label(ui: &mut Ui, c: Canvas, at: Vec2, above: f32, name: &str, shown: f32, terminus: bool, room: Rect) {
    let a = shown * c.a;
    if a <= 0.01 {
        return;
    }
    // (read on a phone's small card too: never smaller than the interface's smallest type, the
    // label growing with its words)
    let px = c.font(9.5).max(7.5);
    let u = px / 9.5;
    let tw = ui.width(name, px, Weight::Bold);
    let (h, r, pad) = (20.0 * u, 4.4 * u, 6.0 * u);
    let w = pad + 2.0 * r + 5.0 * u + tw + 9.0 * u;
    let rise = (1.0 - ease_out_back(shown)) * 5.0 * u;
    let x = (at.x - w * 0.5).clamp(room.x + 6.0 * u, (room.right() - 6.0 * u - w).max(room.x + 6.0 * u));
    let y = (at.y - above - 8.0 * u - h + rise).max(room.y + 4.0 * u);
    let b = Rect::new(x, y, w, h);
    let p = ui.p();
    p.shadow(Rect::new(b.x, b.y + 3.0 * u, b.w, b.h), h * 0.5, 12.0 * u, Color::BLACK.alpha(0.5 * a));
    let tip = at.x.clamp(b.x + h * 0.6, b.right() - h * 0.6);
    p.convex(&[Vec2::new(tip - 4.5 * u, b.bottom() - 1.0), Vec2::new(tip + 4.5 * u, b.bottom() - 1.0), Vec2::new(tip, b.bottom() + 4.5 * u)], CHIP.alpha(a));
    p.rounded(b, h * 0.5, CHIP.alpha(a));
    p.rounded_border(b, h * 0.5, 1.0, (if terminus { LINE.alpha(0.45) } else { Color::WHITE.alpha(0.13) }).alpha(a));
    intro::sign(p, Vec2::new(b.x + pad + r, b.center().y), r, false, a);
    ui.text_in(name, Rect::new(b.x + pad + 2.0 * r + 5.0 * u, b.y, tw + 4.0 * u, b.h), px, Weight::Bold, TEXT.alpha(a), Align::Left);
}

/// Out past 1 and back, a little (a label settling into its place).
fn ease_out_back(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0) - 1.0;
    let s = 1.25;
    1.0 + (s + 1.0) * u * u * u + s * u * u
}

// --- the welcome's ride --------------------------------------------------------------------

/// Where the welcome's bus is on its round.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ride {
    /// How far along it is, as a share of the way from where it stands at the first stop to
    /// where it stands at the last.
    along: f32,
    /// How much of it is seen: it fades out at the end of the line and comes in at the start.
    seen: f32,
    /// Its speed, as a share of its fastest (0 standing).
    speed: f32,
    /// The stop it got to last, the seconds since, and the seconds since it left there (0 while
    /// it stands there).
    stop: usize,
    since: f32,
    left: f32,
}

// The round, in seconds: standing at the first stop (it comes in meanwhile); the drive, shared
// out by the legs' lengths but never too short for one; a stand at each stop and a longer one
// at the terminus, after which it fades. A stop's name comes as the bus gets there and goes as
// it leaves; the ring that goes out from the stop.
const FIRST_S: f32 = 1.1;
const DRIVE_S: f32 = 4.8;
const LEG_MIN_S: f32 = 1.0;
const DWELL_S: f32 = 1.5;
const LAST_S: f32 = 2.0;
const FADE_S: f32 = 0.5;
const NAME_IN_S: f32 = 0.3;
const NAME_OUT_S: f32 = 0.35;
const PULSE_S: f32 = 0.9;

impl Ride {
    /// Standing at stop `stop` of `stops`, `since` seconds after it got there.
    fn standing(stop: usize, stops: &[f32], since: f32) -> Ride {
        Ride { along: stops.get(stop).copied().unwrap_or(0.0), seen: 1.0, speed: 0.0, stop, since, left: 0.0 }
    }

    /// How much of the stop's name is shown (0 to 1).
    fn name_shown(&self) -> f32 {
        smoothstep(self.since / NAME_IN_S) * (1.0 - smoothstep(self.left / NAME_OUT_S)) * self.seen
    }

    /// How far the ring going out from the stop as the bus gets there is (0 to 1), while it is.
    fn pulse(&self) -> Option<f32> {
        (self.left == 0.0 && self.since < PULSE_S).then_some(self.since / PULSE_S)
    }
}

fn leg_s(stops: &[f32], i: usize) -> f32 {
    (DRIVE_S * (stops[i + 1] - stops[i])).max(LEG_MIN_S)
}

/// The whole round over `stops` (seconds).
fn round_s(stops: &[f32]) -> f32 {
    let n = stops.len();
    FIRST_S + (0..n.saturating_sub(1)).map(|i| leg_s(stops, i)).sum::<f32>() + DWELL_S * n.saturating_sub(2) as f32 + LAST_S + FADE_S
}

/// The speed of `ease_in_out_cubic` at `f`, as a share of its fastest (in its middle).
fn ease_speed(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    4.0 * if f < 0.5 { f * f } else { (1.0 - f) * (1.0 - f) }
}

/// The welcome's bus `t` seconds into its round over stops at `stops` (shares of the way, rising
/// from 0 to 1): it comes in at the first stop, drives from stop to stop - easing away and in -
/// and stands at each; at the terminus it stands longer, fades, and starts again.
fn ride(t: f32, stops: &[f32]) -> Ride {
    let n = stops.len();
    if n < 2 {
        return Ride::standing(0, stops, t.max(0.0));
    }
    let mut u = t.rem_euclid(round_s(stops));
    if u < FIRST_S {
        return Ride { seen: smoothstep(u / FADE_S), ..Ride::standing(0, stops, u) };
    }
    u -= FIRST_S;
    let mut stood = FIRST_S;
    for i in 0..n - 1 {
        let d = leg_s(stops, i);
        if u < d {
            let f = u / d;
            return Ride { along: stops[i] + (stops[i + 1] - stops[i]) * ease_in_out_cubic(f), seen: 1.0, speed: ease_speed(f), stop: i, since: stood + u, left: u.max(1e-4) };
        }
        u -= d;
        let last = i + 2 == n;
        stood = if last { LAST_S } else { DWELL_S };
        if last || u < stood {
            let fade = if last { smoothstep((u - LAST_S) / FADE_S) } else { 0.0 };
            return Ride { seen: 1.0 - fade, ..Ride::standing(i + 1, stops, u) };
        }
        u -= stood;
    }
    Ride::standing(n - 1, stops, 0.0)
}

/// The arrival's celebration `t` seconds in (None: standing still): the two rings going out from
/// the terminus in turn - how far out each is (0 to 1), while it is - and how bright each
/// sparkle round it is (0 to 1), twinkling slowly out of step.
fn celebration(t: Option<f32>) -> ([Option<f32>; 2], [f32; 5]) {
    const EVERY: f32 = 3.2;
    const SPREAD: f32 = 1.7;
    const FIRST: f32 = 0.35;
    let Some(t) = t else { return ([Some(0.42), None], [0.85; 5]) };
    let ring = |k: usize| {
        let from = FIRST + k as f32 * EVERY * 0.5;
        let u = (t - from).rem_euclid(EVERY);
        (t >= from && u < SPREAD).then_some(u / SPREAD)
    };
    let twinkle = |i: usize| {
        let s = 0.5 + 0.5 * (t * std::f32::consts::TAU / 2.8 + i as f32 * 2.1).sin();
        smoothstep(s) * smoothstep((t - 0.2 - 0.12 * i as f32) / 0.5)
    };
    ([ring(0), ring(1)], std::array::from_fn(twinkle))
}

/// The welcome's drawing (and a phone's goodbye's): the night city, the route through it with
/// its stops, and a bus driving it from stop to stop - its name over each stop it gets to - or
/// arrived at its terminus, the whole route lit and a quiet celebration round it.
fn city_drawing(ui: &mut Ui, c: Canvas, room: Rect, t: Option<f32>, arrived: bool) {
    let route = route_path();
    let pts = route.points();
    let last = STOP_AT.len() - 1;
    let stop_d: Vec<f32> = STOP_AT.iter().map(|q| nearest(pts, v2(*q)).1).collect();
    // where the bus stands at each: its front a little short of the sign (of its yellow ring at
    // the terminus)
    let stand: Vec<f32> = (0..STOP_AT.len()).map(|i| stop_d[i] - BUS_HALF.x - if i == last { TERMINUS_R * 1.84 + 4.0 } else { SIGN_R * 1.2 + 4.0 }).collect();
    let span = (stand[last] - stand[0]).max(1.0);
    let shares: Vec<f32> = stand.iter().map(|s| (s - stand[0]) / span).collect();
    let ride = match (arrived, t) {
        (true, t) => Ride::standing(last, &shares, t.unwrap_or(10.0)),
        (false, Some(t)) => ride(t, &shares),
        (false, None) => Ride::standing(1, &shares, 10.0),
    };
    let bus_d = stand[0] + ride.along * span;
    let p = ui.p();
    p.rect(room, c.ink(NIGHT));
    city(p, c, breathing(t));
    // the route: all of it calm, the part driven bright (it fades with the bus at the end of
    // the line, and comes back with it at the start)
    let w = c.len(ROUTE_W);
    let e = c.len(ROUTE_EDGE);
    let lit = if arrived { route.length() } else { bus_d + BUS_HALF.x * 0.5 };
    let lit_a = if arrived { 1.0 } else { ride.seen };
    let driven = c.pts(&route.part(0.0, lit));
    glow_along(p, &driven, c.len(30.0), c.ink(accent().alpha(0.2 * lit_a)));
    p.stroke_edged(&c.pts(pts), w, e, c.ink(ROUTE_AHEAD), c.ink(ROUTE_AHEAD_CASING));
    p.stroke_edged(&driven, w, e, c.ink(ROUTE_FILL.alpha(lit_a)), c.ink(ROUTE_CASING.alpha(lit_a)));
    vignette(p, room, c.ink(SHADE.alpha(0.78)));
    // the stops, a ring going out from the one the bus gets to
    for (i, q) in STOP_AT.iter().enumerate() {
        let terminus = i == last;
        intro::sign(p, c.at(q.0, q.1), c.len(if terminus { TERMINUS_R } else { SIGN_R }), terminus, c.a);
    }
    if let Some(u) = ride.pulse().filter(|_| !arrived) {
        let e = ease_out_cubic(u);
        let terminus = ride.stop == last;
        let base = if terminus { TERMINUS_R * 1.84 } else { SIGN_R * 1.25 };
        let r = c.len(base + 16.0 * e);
        let lw = c.len(2.2 - 1.1 * e);
        let col = if terminus { LINE } else { ROUTE_FILL };
        let q = STOP_AT[ride.stop];
        p.arc(c.at(q.0, q.1), r - lw * 0.5, r + lw * 0.5, 0.0, std::f32::consts::TAU, c.ink(col.alpha(0.6 * (1.0 - e) * (1.0 - e) * ride.seen)));
    }
    // the bus: its light on the street round it and ahead of it, a short trail of light behind
    // it while it drives, and the bus
    let (at, dir) = route.at(bus_d);
    let (at, seen) = (c.pt(at), ride.seen * c.a);
    p.radial(at, c.len(36.0), accent().alpha(0.2 * seen), accent().alpha(0.0));
    headlights(p, at, dir, c.k, WARM.alpha(0.17 * seen));
    if ride.speed > 0.01 {
        let tail = bus_d - BUS_HALF.x * 0.7;
        trail(p, &c.pts(&route.part(tail - 46.0 * ride.speed, tail)), c.len(5.0), PEN.alpha(0.55 * seen));
    }
    bus_from_above(p, at, dir, c.k, seen);
    // arrived: rings going out from the terminus in turn, sparkles twinkling round it
    if arrived {
        let end = c.at(STOP_AT[last].0, STOP_AT[last].1);
        let (rings, sparkles) = celebration(t);
        let p = ui.p();
        p.radial(end, c.len(44.0), c.ink(LINE.alpha(0.14)), LINE.alpha(0.0));
        for u in rings.into_iter().flatten() {
            let e = ease_out_cubic(u);
            let r = c.len(TERMINUS_R * 1.84 + 4.0 + 26.0 * e);
            let lw = c.len(2.0 - 1.0 * e);
            p.arc(end, r - lw * 0.5, r + lw * 0.5, 0.0, std::f32::consts::TAU, c.ink(LINE.alpha(0.55 * (1.0 - e) * (1.0 - e))));
        }
        for (k, ((x, y, size), s)) in [(34.0, -12.0, 5.5), (22.0, 27.0, 4.2), (-12.0, 30.0, 3.6), (50.0, 10.0, 3.4), (-36.0, 25.0, 3.0)].into_iter().zip(sparkles).enumerate() {
            let col = if k % 2 == 0 { LINE } else { PEN };
            sparkle(p, end + Vec2::new(c.len(x), c.len(y)), c.len(size * (0.55 + 0.45 * s)), c.ink(col.alpha(0.3 + 0.7 * s)));
        }
    }
    // the name of the stop the bus is at
    let i = ride.stop;
    let shown = if arrived { smoothstep(ride.since / NAME_IN_S) } else { ride.name_shown() };
    let above = if i == last { TERMINUS_R * 1.84 } else { SIGN_R * 1.2 };
    label(ui, c, c.at(STOP_AT[i].0, STOP_AT[i].1), c.len(above), STOP_NAMES[i], shown, i == last, room);
}

// --- the game's screens --------------------------------------------------------------------

/// A monitor standing in the drawing: its body, and the neck going down out of the picture.
/// Returns its screen and the screen's corners.
fn monitor(p: &mut Painter, c: Canvas) -> (Rect, f32) {
    let neck = c.line(&[(236.0, 176.0), (284.0, 176.0), (292.0, 206.0), (228.0, 206.0)]);
    p.tri(neck[0], neck[1], neck[2], c.ink(BODY_BOTTOM.darken(0.3)), c.ink(BODY_BOTTOM.darken(0.3)), c.ink(BODY_TOP));
    p.tri(neck[0], neck[2], neck[3], c.ink(BODY_BOTTOM.darken(0.3)), c.ink(BODY_TOP), c.ink(BODY_TOP));
    device(p, c, Rect::new(18.0, 6.0, 484.0, 176.0), 12.0, 6.0)
}

/// The game as the driver sees it at dusk, in `s` (a screen, drawn inside its clip): the sky over
/// the town's outline, the road running to the horizon with its lines and the lamps along it,
/// and the dashboard's edge.
fn road_scene(p: &mut Painter, s: Rect, a: f32) {
    const HORIZON: f32 = 0.5;
    const VANISH: f32 = 0.33;
    let at = |u: f32, v: f32| Vec2::new(s.x + u * s.w, s.y + v * s.h);
    let ink = |c: Color| c.alpha(a);
    p.gradient(Rect::new(s.x, s.y, s.w, s.h * HORIZON), ink(SKY_TOP), ink(SKY_LOW));
    p.gradient(Rect::new(s.x, s.y + s.h * (HORIZON - 0.16), s.w, s.h * 0.16), WARM.alpha(0.0), WARM.alpha(0.1 * a));
    // the town: a row far off and a darker one before it, a few windows lit
    for (row, colour, low) in [(0usize, TOWN_FAR, 0.1f32), (1, TOWN_NEAR, 0.05)] {
        let (mut u, mut k) = (-0.02f32, 0usize);
        while u < 1.02 {
            let h = hash(k, row + 7);
            let w = 0.03 + (h % 7) as f32 * 0.007;
            let tall = low + ((h >> 3) % 9) as f32 * 0.017;
            let top = HORIZON - tall;
            p.rect(Rect::new(s.x + u * s.w, s.y + top * s.h, w * s.w + 0.6, tall * s.h + 1.0), ink(colour));
            if row == 1 {
                for wi in 0..3 {
                    if (h >> (12 + wi * 3)) % 3 == 0 {
                        let (wu, wv) = (u + w * (0.25 + 0.25 * wi as f32), top + tall * 0.3);
                        p.rect(Rect::new(s.x + wu * s.w, s.y + wv * s.h, s.w * 0.006, s.h * 0.012), WARM.alpha(0.5 * a));
                    }
                }
            }
            u += w + 0.004;
            k += 1;
        }
    }
    p.rect(Rect::new(s.x, s.y + s.h * HORIZON, s.w, s.h * (1.0 - HORIZON)), ink(DUSK_GROUND));
    // the road to the vanishing point, its edges, the dashes down its middle - nearer together
    // and thinner far away
    let road = |side: f32, z: f32| at(VANISH + side * 0.55 * z, HORIZON + (1.0 - HORIZON) * z);
    let (hl, hr, bl, br) = (road(-1.0, 0.0), road(1.0, 0.0), road(-1.0, 1.0), road(1.0, 1.0));
    p.tri(hl, hr, br, ink(ROAD_FAR), ink(ROAD_FAR), ink(ROAD_NEAR));
    p.tri(hl, br, bl, ink(ROAD_FAR), ink(ROAD_NEAR), ink(ROAD_NEAR));
    for side in [-0.92, 0.92] {
        p.convex(&[road(side - 0.012, 0.02), road(side + 0.012, 0.02), road(side + 0.03, 1.0), road(side - 0.03, 1.0)], Color::WHITE.alpha(0.3 * a));
    }
    for i in 0..7 {
        let (z0, z1) = (1.0 / (1.0 + i as f32), 1.0 / (1.45 + i as f32));
        let half = |z: f32| 0.022 * z;
        p.convex(&[road(-half(z0), z0), road(half(z0), z0), road(half(z1), z1), road(-half(z1), z1)], Color::WHITE.alpha(0.62 * a));
    }
    // the lamps along its right side
    for d in [1.25f32, 2.1, 3.5, 6.0, 10.0] {
        let z = 1.0 / d;
        let foot = road(1.22, z);
        let top = foot - Vec2::new(0.0, s.h * 0.42 * z);
        p.convex(&[foot - Vec2::new(s.h * 0.006 * z, 0.0), foot + Vec2::new(s.h * 0.006 * z, 0.0), top + Vec2::new(s.h * 0.004 * z, 0.0), top - Vec2::new(s.h * 0.004 * z, 0.0)], ink(TOWN_NEAR));
        p.radial(top, s.h * 0.22 * z, WARM.alpha(0.22 * a), WARM.alpha(0.0));
        p.circle(top, (s.h * 0.014 * z).max(0.7), WARM.alpha(0.9 * a));
    }
    // the dashboard's edge
    let dash = [at(0.0, 1.02), at(0.0, 0.93), at(0.3, 0.9), at(0.7, 0.9), at(1.0, 0.93), at(1.0, 1.02)];
    p.convex(&dash, ink(Color::rgba(7, 10, 17, 1.0)));
    p.stroke(&dash[1..5], s.h * 0.006, Color::WHITE.alpha(0.07 * a));
}

/// The rows of the duty board in the navigator's drawing: the stop, when the bus is there.
const BOARD_ROWS: [(&str, &str); 4] = [("Rathaus Spandau", "14:02"), ("Altstädter Ring", "14:05"), ("Zitadelle", "14:08"), ("Hakenfelde", "14:12")];

/// The navigator's duty board `t` seconds in: the stop the bus heads for moves a row down as it
/// goes - standing in between - from the second row to the terminus, and the board starts again
/// (fading). Returns that row (between two while it moves) and how much of the board is seen.
fn board_focus(t: f32) -> (f32, f32) {
    const HOLD: f32 = 2.1;
    const MOVE: f32 = 0.6;
    const FADE: f32 = 0.35;
    let (first, rows) = (1usize, BOARD_ROWS.len());
    let steps = rows - first;
    let round = steps as f32 * HOLD + (steps - 1) as f32 * MOVE + 2.0 * FADE;
    let mut u = t.rem_euclid(round);
    if u < FADE {
        return (first as f32, smoothstep(u / FADE));
    }
    u -= FADE;
    for row in first..rows {
        if row + 1 == rows {
            return (row as f32, 1.0 - smoothstep((u - HOLD) / FADE));
        }
        if u < HOLD {
            return (row as f32, 1.0);
        }
        u -= HOLD;
        if u < MOVE {
            return (row as f32 + ease_in_out_cubic(u / MOVE), 1.0);
        }
        u -= MOVE;
    }
    ((rows - 1) as f32, 0.0)
}

/// Shift+N: the game on a monitor, the road at dusk through the windscreen, and in its corner the
/// navigator - the small map with the bus on its route, and the duty board under it, the stop it
/// heads for moving down as it goes.
fn navigator_drawing(ui: &mut Ui, c: Canvas, room: Rect, t: Option<f32>) {
    backdrop(ui.p(), c, room, c.at(260.0, 96.0));
    let (screen, sr) = monitor(ui.p(), c);
    let (focus, seen) = t.map_or((2.0, 1.0), board_focus);
    ui.push_clip(screen, sr);
    road_scene(ui.p(), screen, c.a);
    navigator_panel(ui, c, Rect::new(312.0, 18.0, 178.0, 152.0), focus, seen);
    ui.pop_clip();
    sheen(ui.p(), screen, sr, c.a);
    shortcut(ui, c, 40.0, 138.0, "N");
}

/// Where the navigator's bus is when it heads for row `focus` of the board (between two while
/// the board moves on): most of the way from the stop before to that one.
fn heading(focus: f32, stops: &[f32]) -> f32 {
    let at = |row: usize| {
        let row = row.clamp(1, stops.len() - 1);
        stops[row - 1] + (stops[row] - stops[row - 1]) * 0.62
    };
    let lo = focus.floor().max(0.0) as usize;
    let f = focus - lo as f32;
    at(lo) + (at(lo + 1) - at(lo)) * f
}

/// The navigator as the game shows it, in `r` (units): its map with the route and the bus, the
/// trip's line plate, terminus and punctuality, and the stops - the one the bus heads for at
/// `focus`.
fn navigator_panel(ui: &mut Ui, c: Canvas, r: Rect, focus: f32, seen: f32) {
    let panel = c.rect(r.x, r.y, r.w, r.h);
    let p = ui.p();
    p.shadow(Rect::new(panel.x, panel.y + c.len(6.0), panel.w, panel.h), c.len(9.0), c.len(20.0), c.ink(Color::BLACK.alpha(0.55)));
    p.rounded(panel, c.len(9.0), c.ink(PANEL.alpha(0.97)));
    p.rounded_border(panel, c.len(9.0), 1.0, c.ink(Color::WHITE.alpha(0.1)));
    // the small map: the night city round the route, the bus on it
    let map = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, 52.0);
    let mr = c.rect(map.x, map.y, map.w, map.h);
    ui.push_clip(mr, c.len(6.0));
    let s = 0.36;
    let m = c.inner(map.x - 36.0 * s, map.y - 30.0 * s, s);
    let route = route_path();
    let stop_d: Vec<f32> = STOP_AT.iter().map(|q| nearest(route.points(), v2(*q)).1).collect();
    let bus_d = heading(focus, &stop_d);
    let p = ui.p();
    p.rect(mr, c.ink(NIGHT));
    city(p, m, 1.0);
    p.stroke_edged(&m.pts(route.points()), m.len(ROUTE_W * 1.3), m.len(ROUTE_EDGE * 1.2), c.ink(ROUTE_AHEAD), c.ink(ROUTE_AHEAD_CASING));
    p.stroke_edged(&m.pts(&route.part(0.0, bus_d)), m.len(ROUTE_W * 1.3), m.len(ROUTE_EDGE * 1.2), c.ink(ROUTE_FILL), c.ink(ROUTE_CASING));
    for (i, q) in STOP_AT.iter().enumerate() {
        intro::sign(p, m.at(q.0, q.1), m.len(if i == 3 { TERMINUS_R } else { SIGN_R }) * 1.15, i == 3, c.a);
    }
    let (at, dir) = route.at(bus_d);
    let at = m.pt(at);
    p.radial(at, c.len(11.0), c.ink(accent().alpha(0.45)), accent().alpha(0.0));
    let arrow = |grow: f32| turn(&[Vec2::new(5.6 + grow, 0.0), Vec2::new(-4.0 - grow, -4.6 - grow), Vec2::new(-1.8, 0.0), Vec2::new(-4.0 - grow, 4.6 + grow)], at, dir, c.k);
    for (q, col) in [(arrow(1.3), ROUTE_CASING), (arrow(0.0), Color::WHITE)] {
        p.tri(q[0], q[1], q[2], c.ink(col), c.ink(col), c.ink(col));
        p.tri(q[0], q[2], q[3], c.ink(col), c.ink(col), c.ink(col));
    }
    ui.pop_clip();
    // the trip: its line, where it goes, on time
    let a = seen;
    let cy = r.y + 72.0;
    let pw = plate(ui, c, "24", r.x + 8.0, cy, 14.0);
    let chip_w = 46.0;
    let chip = c.rect(r.right() - 8.0 - chip_w, cy - 7.0, chip_w, 14.0);
    ui.p().rounded(chip, c.len(7.0), c.ink(ON_TIME));
    words(ui, c, "on time", chip, 7.5, Weight::Bold, Color::WHITE, Align::Center);
    words(ui, c, "Hakenfelde", c.rect(r.x + 14.0 + pw, cy - 8.0, r.w - 30.0 - pw - chip_w, 16.0), 9.5, Weight::Bold, TEXT, Align::Left);
    ui.p().rect(Rect::new(c.at(r.x + 8.0, 0.0).x, c.at(0.0, r.y + 84.5).y, c.len(r.w - 16.0), 1.0), c.ink(Color::WHITE.alpha(0.08)));
    // the stops: done, the one it heads for, ahead - on the rail
    let row_h = 14.0;
    let top = r.y + 88.0;
    let rail_x = r.x + 15.0;
    let hl = c.rect(r.x + 5.0, top + focus * row_h, r.w - 10.0, row_h);
    ui.p().rounded(hl, c.len(4.0), c.ink(NOW.alpha(0.1 * a)));
    for (i, (name, time)) in BOARD_ROWS.iter().enumerate() {
        let y = top + i as f32 * row_h;
        let cy = y + row_h * 0.5;
        let d = focus - i as f32;
        let (now, done, ahead) = ((1.0 - d.abs()).max(0.0), d.clamp(0.0, 1.0), (-d).clamp(0.0, 1.0));
        let p = ui.p();
        // (the rail through a row grey once the bus has passed its stop, as the board's)
        let rail = c.ink(BOARD_RAIL.mix(BOARD_RAIL_DONE, done).alpha(a));
        if i > 0 {
            p.rect(c.rect(rail_x - 1.1, y, 2.2, row_h * 0.5), rail);
        }
        if i + 1 < BOARD_ROWS.len() {
            p.rect(c.rect(rail_x - 1.1, cy, 2.2, row_h * 0.5), rail);
        }
        let pin = c.at(rail_x, cy);
        p.circle(pin, c.len(2.6), c.ink(BOARD_RAIL_DONE.alpha(done * a)));
        p.circle(pin, c.len(3.0), c.ink(BOARD_RAIL.alpha(ahead * a)));
        p.circle(pin, c.len(1.6), c.ink(PANEL.alpha(ahead * a)));
        p.circle(pin, c.len(6.0), c.ink(NOW.alpha(0.22 * now * a)));
        p.circle(pin, c.len(3.8), c.ink(NOW.alpha(now * a)));
        let name_ink = TEXT_FAINT.mix(TEXT_SOFT, ahead).mix(Color::WHITE, now).alpha(a);
        let time_ink = TEXT_FAINT.mix(TEXT_DIM, ahead).mix(NOW, now).alpha(a);
        let weight = if now > 0.5 { Weight::Bold } else { Weight::Medium };
        words(ui, c, name, c.rect(rail_x + 8.0, y, r.w - 64.0, row_h), 8.5, weight, name_ink, Align::Left);
        words(ui, c, time, c.rect(r.right() - 44.0, y, 36.0, row_h), 8.5, weight, time_ink, Align::Right);
    }
}

/// The number a driver signs on with, as keys of the keypad (1 to 9 are its first nine, 0 its
/// eleventh): 482913.
const NUMBER_KEYS: [usize; 6] = [3, 7, 1, 8, 0, 2];

/// Signing on, a moment of it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Typing {
    /// How many of the number's digits are typed.
    typed: usize,
    /// The key tapped last and the seconds since.
    tap: Option<(usize, f32)>,
    /// How far the "signed on" has come (0 to 1), and how much of it all is seen (it clears to
    /// start again).
    done: f32,
    seen: f32,
}

/// Signing on `t` seconds in: a pause, the number's digits tapped one after another, the field
/// turning to "signed on", a while so, and it clears for the next round.
fn typing(t: f32) -> Typing {
    const START: f32 = 0.7;
    const KEY: f32 = 0.42;
    const CHECK: f32 = 0.35;
    const HOLD: f32 = 1.8;
    const CLEAR: f32 = 0.4;
    let n = NUMBER_KEYS.len();
    let last = START + (n - 1) as f32 * KEY;
    let round = last + CHECK + HOLD + CLEAR + 0.2;
    let u = t.rem_euclid(round);
    let typed = if u < START { 0 } else { (((u - START) / KEY).floor() as usize + 1).min(n) };
    let tap = (typed > 0).then(|| (NUMBER_KEYS[typed - 1], u - START - (typed - 1) as f32 * KEY));
    let done = smoothstep((u - last - CHECK) / 0.3);
    let seen = 1.0 - smoothstep((u - last - CHECK - HOLD) / CLEAR);
    Typing { typed, tap, done, seen }
}

/// Shift+M: the game on a monitor showing the city map with the route, and beside it signing on
/// - the number being tapped on the keypad, the driver's pass under it.
fn city_map_drawing(ui: &mut Ui, c: Canvas, room: Rect, t: Option<f32>) {
    backdrop(ui.p(), c, room, c.at(260.0, 96.0));
    let (screen, sr) = monitor(ui.p(), c);
    ui.push_clip(screen, sr);
    // the city map: the night city, the route lit up to the bus
    let s = 0.6;
    let m = c.inner(10.0 + 12.0 * s, 12.0 + 40.0 * s, s);
    let route = route_path();
    let pts = route.points();
    let stop_d: Vec<f32> = STOP_AT.iter().map(|q| nearest(pts, v2(*q)).1).collect();
    let bus_d = stop_d[1] + (stop_d[2] - stop_d[1]) * 0.45;
    let p = ui.p();
    p.rect(screen, c.ink(NIGHT));
    city(p, m, 1.0);
    let driven = m.pts(&route.part(0.0, bus_d + 8.0));
    glow_along(p, &driven, m.len(30.0), c.ink(accent().alpha(0.18)));
    p.stroke_edged(&m.pts(pts), m.len(ROUTE_W), m.len(ROUTE_EDGE), c.ink(ROUTE_AHEAD), c.ink(ROUTE_AHEAD_CASING));
    p.stroke_edged(&driven, m.len(ROUTE_W), m.len(ROUTE_EDGE), c.ink(ROUTE_FILL), c.ink(ROUTE_CASING));
    for (i, q) in STOP_AT.iter().enumerate() {
        intro::sign(p, m.at(q.0, q.1), m.len(if i == 3 { TERMINUS_R } else { SIGN_R }), i == 3, c.a);
    }
    let (at, dir) = route.at(bus_d);
    let at = m.pt(at);
    p.radial(at, m.len(40.0), c.ink(accent().alpha(0.22)), accent().alpha(0.0));
    bus_from_above(p, at, dir, m.k, c.a);
    // signing on beside it
    let ty = t.map_or(Typing { typed: 4, tap: None, done: 0.0, seen: 1.0 }, typing);
    sign_on(ui, c, Rect::new(326.0, 12.0, 176.0, 164.0), ty);
    ui.pop_clip();
    sheen(ui.p(), screen, sr, c.a);
    shortcut(ui, c, 40.0, 138.0, "M");
}

/// Signing on, in `r` (units): the number so far, the keypad - the key tapped rippling - and the
/// driver's pass with the number and the code.
fn sign_on(ui: &mut Ui, c: Canvas, r: Rect, ty: Typing) {
    let p = ui.p();
    p.rect(c.rect(r.x, r.y, r.w, r.h), c.ink(PANEL));
    p.rect(c.rect(r.x, r.y, 0.0, r.h).pad(-0.5, 0.0), c.ink(Color::WHITE.alpha(0.1)));
    let x0 = r.x + 10.0;
    let w = 150.0;
    ui.icon("badge", c.at(x0 + 6.0, r.y + 13.0), c.len(11.0), c.ink(TEXT_SOFT));
    words(ui, c, "Personnel number", c.rect(x0 + 15.0, r.y + 6.0, w - 15.0, 14.0), 8.5, Weight::Medium, TEXT_SOFT, Align::Left);
    // the number: dots for the digits typed, rings for the rest; green once signed on
    let field = c.rect(x0, r.y + 23.0, w, 20.0);
    let p = ui.p();
    p.rounded(field, c.len(5.0), c.ink(FIELD));
    if ty.done > 0.0 {
        p.rounded_border(field, c.len(5.0), 1.2, c.ink(OK.alpha(ty.done)));
    }
    for k in 0..NUMBER_KEYS.len() {
        let at = c.at(x0 + w * 0.5 - 8.0 + (k as f32 - 2.5) * 16.0, r.y + 33.0);
        if k < ty.typed {
            p.circle(at, c.len(2.8), c.ink(TEXT.mix(OK, ty.done).alpha(ty.seen)));
        } else {
            p.arc(at, c.len(2.2), c.len(3.1), 0.0, std::f32::consts::TAU, c.ink(TEXT_FAINT));
        }
    }
    if ty.done > 0.0 {
        ui.icon("check_circle", c.at(x0 + w - 11.0, r.y + 33.0), c.len(12.0), c.ink(OK.alpha(ty.done * ty.seen)));
    }
    // the keypad
    let keys = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "C", "0", ""];
    let (kw, kh, gap) = (48.0, 17.0, 3.0);
    for (k, word) in keys.iter().enumerate() {
        let kr = c.rect(x0 + (k % 3) as f32 * (kw + gap), r.y + 49.0 + (k / 3) as f32 * (kh + gap), kw, kh);
        let tap = ty.tap.filter(|(key, since)| *key == k && *since < 0.5).map(|(_, since)| since / 0.5);
        let p = ui.p();
        p.rounded(kr, c.len(4.5), c.ink(Color::rgba(35, 43, 61, 1.0)));
        p.rounded_border(kr, c.len(4.5), 1.0, c.ink(Color::WHITE.alpha(0.05)));
        if let Some(u) = tap {
            let e = ease_out_cubic(u);
            p.rounded(kr, c.len(4.5), c.ink(accent().alpha(0.45 * (1.0 - u))));
            p.circle(kr.center(), c.len(4.0 + 20.0 * e), c.ink(Color::WHITE.alpha(0.22 * (1.0 - e) * (1.0 - e))));
        }
        if word.is_empty() {
            ui.icon("chevron_left", kr.center(), c.len(12.0), c.ink(TEXT_SOFT));
        } else {
            words(ui, c, word, kr, 9.5, Weight::Bold, TEXT, Align::Center);
        }
    }
    // the driver's pass
    let pass = c.rect(x0, r.y + 131.0, w, 24.0);
    let p = ui.p();
    p.rounded(pass, c.len(5.0), c.ink(LINE.alpha(0.13)));
    p.rounded_border(pass, c.len(5.0), 1.0, c.ink(LINE.alpha(0.5)));
    p.rounded(c.rect(x0, r.y + 131.0, 3.0, 24.0).pad(0.0, c.len(4.0)), c.len(1.5), c.ink(LINE));
    ui.icon("badge", c.at(x0 + 14.0, r.y + 143.0), c.len(12.0), c.ink(LINE));
    words(ui, c, "482913 · 5821", c.rect(x0 + 24.0, r.y + 131.0, w - 30.0, 24.0), 9.5, Weight::Bold, LINE, Align::Left);
}

/// The phone's and the tablet's screens `t` seconds in: which of their states they show (the
/// trip the duty is on), and the seconds since they last refreshed.
fn refresh(t: f32) -> (usize, f32) {
    const EVERY: f32 = 3.4;
    let t = t + 2.2;
    let k = (t / EVERY).floor().max(0.0);
    ((k as usize) % 3, t - k * EVERY)
}

/// Phone & tablet: the game on a laptop, and in the same network a tablet with the duty menu
/// and a phone in front of it showing the bus's IBIS - both refreshing as the game moves on.
fn companion_drawing(ui: &mut Ui, c: Canvas, room: Rect, t: Option<f32>) {
    backdrop(ui.p(), c, room, c.at(300.0, 100.0));
    let (state, since) = t.map_or((0, 99.0), refresh);
    // the laptop with the game
    let p = ui.p();
    soft_light(p, c.at(102.0, 151.0), Vec2::new(c.len(112.0), c.len(9.0)), c.ink(Color::BLACK.alpha(0.6)));
    let deck = c.line(&[(8.0, 142.0), (196.0, 142.0), (206.0, 151.0), (-2.0, 151.0)]);
    p.tri(deck[0], deck[1], deck[2], c.ink(BODY_TOP), c.ink(BODY_TOP), c.ink(BODY_BOTTOM));
    p.tri(deck[0], deck[2], deck[3], c.ink(BODY_TOP), c.ink(BODY_BOTTOM), c.ink(BODY_BOTTOM));
    p.rounded(c.rect(84.0, 142.0, 36.0, 2.6), c.len(1.3), c.ink(BODY_BOTTOM));
    let (screen, sr) = device(p, c, Rect::new(14.0, 34.0, 176.0, 108.0), 8.0, 5.0);
    ui.push_clip(screen, sr);
    road_scene(ui.p(), screen, c.a);
    let p = ui.p();
    p.shadow(c.rect(131.0, 45.0, 50.0, 58.0), c.len(4.0), c.len(8.0), c.ink(Color::BLACK.alpha(0.5)));
    p.rounded(c.rect(131.0, 43.0, 50.0, 58.0), c.len(4.0), c.ink(PANEL.alpha(0.96)));
    p.rounded(c.rect(134.0, 46.0, 44.0, 24.0), c.len(2.5), c.ink(NIGHT));
    for i in 0..4 {
        for j in 0..2 {
            p.rounded(c.rect(135.0 + i as f32 * 10.8, 47.0 + j as f32 * 11.5, 9.0, 10.0), c.len(1.2), c.ink(BLOCK_LIT));
        }
    }
    let mini = c.line(&[(134.5, 64.0), (149.0, 64.0), (156.0, 57.5), (177.5, 57.5)]);
    p.stroke_edged(&mini, c.len(2.4), c.len(0.8), c.ink(ROUTE_FILL), c.ink(ROUTE_CASING));
    intro::sign(p, c.at(143.0, 64.0), c.len(1.9), false, c.a);
    intro::sign(p, c.at(170.0, 57.5), c.len(2.2), true, c.a);
    p.rounded(c.rect(135.0, 74.0, 9.0, 6.0), c.len(1.5), c.ink(LINE));
    p.rounded(c.rect(147.0, 75.5, 22.0, 3.0), c.len(1.5), c.ink(TEXT_SOFT.alpha(0.75)));
    for (k, w) in [28.0f32, 22.0].iter().enumerate() {
        p.circle(c.at(137.0, 86.5 + k as f32 * 7.0), c.len(1.4), c.ink(if k == 0 { NOW } else { BOARD_RAIL }));
        p.rounded(c.rect(141.0, 85.2 + k as f32 * 7.0, *w, 2.6), c.len(1.3), c.ink(TEXT_SOFT.alpha(if k == 0 { 0.7 } else { 0.35 })));
    }
    ui.pop_clip();
    sheen(ui.p(), screen, sr, c.a);
    // the network between them, a ring going out with each refresh
    let hub = c.at(202.0, 16.0);
    let p = ui.p();
    if since < 0.9 {
        let e = ease_out_cubic(since / 0.9);
        let r = c.len(10.0 + 16.0 * e);
        p.arc(hub, r - c.len(0.8), r + c.len(0.8), 0.0, std::f32::consts::TAU, c.ink(ROUTE_FILL.alpha(0.6 * (1.0 - e) * (1.0 - e))));
    }
    p.circle(hub, c.len(10.0), c.ink(PANEL));
    p.arc(hub, c.len(9.0), c.len(10.0), 0.0, std::f32::consts::TAU, c.ink(Color::WHITE.alpha(0.12)));
    ui.icon("wifi_tethering", hub, c.len(13.0), c.ink(ROUTE_FILL));
    // the tablet: the duty menu
    let (screen, sr) = device(ui.p(), c, Rect::new(214.0, 24.0, 196.0, 140.0), 14.0, 7.0);
    ui.p().circle(c.at(217.5, 94.0), c.len(1.2), c.ink(Color::rgba(40, 48, 66, 1.0)));
    ui.push_clip(screen, sr);
    duty_menu(ui, c, Rect::new(221.0, 31.0, 182.0, 126.0), state, since);
    ui.pop_clip();
    sheen(ui.p(), screen, sr, c.a);
    // the phone in front of it: the IBIS
    let (screen, sr) = device(ui.p(), c, Rect::new(394.0, 40.0, 80.0, 150.0), 16.0, 4.5);
    ui.push_clip(screen, sr);
    ibis(ui, c, Rect::new(398.5, 44.5, 71.0, 141.0), state, since);
    ui.pop_clip();
    sheen(ui.p(), screen, sr, c.a);
}

/// The trips of the duty on the tablet: when each leaves, its line and where it goes.
const TRIPS: [(&str, &str, &str); 3] = [("14:02", "24", "Hakenfelde"), ("14:41", "24", "Rathaus Spandau"), ("15:20", "36", "Haselhorst")];

/// The duty menu on a tablet, in `r` (units): its bar, the duty with its line and tour and the
/// button that starts it, and its trips - the one under way lit, moving on with each refresh.
fn duty_menu(ui: &mut Ui, c: Canvas, r: Rect, state: usize, since: f32) {
    let p = ui.p();
    p.rect(c.rect(r.x, r.y, r.w, r.h), c.ink(Color::rgba(13, 18, 29, 1.0)));
    p.rect(c.rect(r.x, r.y, r.w, 17.0), c.ink(Color::rgba(19, 25, 39, 1.0)));
    words(ui, c, "Duty", c.rect(r.x + 9.0, r.y, 80.0, 17.0), 8.5, Weight::Bold, TEXT, Align::Left);
    ui.p().circle(c.at(r.right() - 52.0, r.y + 8.5), c.len(2.4), c.ink(OK));
    words(ui, c, "Signed on", c.rect(r.right() - 47.0, r.y, 42.0, 17.0), 7.0, Weight::Medium, TEXT_DIM, Align::Left);
    // the duty: its line, where it goes, its tour and hours
    let card = c.rect(r.x + 7.0, r.y + 23.0, r.w - 14.0, 31.0);
    ui.p().rounded(card, c.len(5.0), c.ink(FIELD));
    let pw = plate(ui, c, "24", r.x + 13.0, r.y + 38.5, 15.0);
    let button = c.rect(r.right() - 55.0, r.y + 31.0, 42.0, 15.0);
    words(ui, c, "Hakenfelde", c.rect(r.x + 19.0 + pw, r.y + 26.0, r.w - 82.0 - pw, 14.0), 9.5, Weight::Bold, TEXT, Align::Left);
    let tour = format!("{} 3  ·  14:02 – 15:40", omsi_ui::tr("Tour"));
    words(ui, c, &tour, c.rect(r.x + 19.0 + pw, r.y + 39.0, r.w - 82.0 - pw, 12.0), 7.5, Weight::Medium, TEXT_DIM, Align::Left);
    ui.p().rounded(button, c.len(4.5), c.ink(accent()));
    words(ui, c, "Start", button, 8.0, Weight::Bold, Color::WHITE, Align::Center);
    // its trips, the one under way lit (the light slides on with a refresh)
    let row_h = 18.0;
    let top = r.y + 61.0;
    let slide = ease_in_out_cubic((since / 0.45).min(1.0));
    let from = (state + TRIPS.len() - 1) % TRIPS.len();
    let lit_row = if state == 0 { state as f32 } else { from as f32 + (state as f32 - from as f32) * slide };
    let hl = c.rect(r.x + 7.0, top + lit_row * row_h, r.w - 14.0, row_h - 1.0);
    let p = ui.p();
    p.rounded(hl, c.len(4.0), c.ink(accent().alpha(0.18)));
    p.rounded(Rect::new(hl.x, hl.y + c.len(3.0), c.len(2.4), hl.h - c.len(6.0)), c.len(1.2), c.ink(ROUTE_FILL));
    for (k, (time, line, to)) in TRIPS.iter().enumerate() {
        let y = top + k as f32 * row_h;
        let lit = (1.0 - (k as f32 - lit_row).abs()).clamp(0.0, 1.0);
        words(ui, c, time, c.rect(r.x + 14.0, y, 30.0, row_h - 1.0), 8.0, Weight::Bold, TEXT_DIM.mix(TEXT, lit), Align::Left);
        let w = plate(ui, c, line, r.x + 44.0, y + (row_h - 1.0) * 0.5, 11.0);
        words(ui, c, to, c.rect(r.x + 50.0 + w, y, r.w - 64.0 - w, row_h - 1.0), 8.0, Weight::Medium, TEXT_SOFT.mix(TEXT, lit), Align::Left);
    }
}

/// The stops the IBIS says are next, a refresh at a time.
const NEXT_STOPS: [&str; 3] = ["Altstädter Ring", "Zitadelle", "Hakenfelde"];

/// The bus's IBIS on a phone, in `r` (units): its amber display - the line and tour, where it
/// goes and the next stop, which changes with a light running down the display as it refreshes
/// - and its keys under it.
fn ibis(ui: &mut Ui, c: Canvas, r: Rect, state: usize, since: f32) {
    let p = ui.p();
    p.rect(c.rect(r.x, r.y, r.w, r.h), c.ink(Color::rgba(13, 17, 28, 1.0)));
    p.rounded(c.rect(r.x + r.w * 0.5 - 7.5, r.y + 4.0, 15.0, 4.6), c.len(2.3), c.ink(Color::BLACK));
    words(ui, c, "IBIS", c.rect(r.x + 6.0, r.y + 11.0, 40.0, 12.0), 7.5, Weight::Bold, TEXT_DIM, Align::Left);
    // the display
    let d = Rect::new(r.x + 5.0, r.y + 25.0, r.w - 10.0, 46.0);
    let dr = c.rect(d.x, d.y, d.w, d.h);
    let p = ui.p();
    p.rounded(dr, c.len(3.5), c.ink(LED_GROUND));
    p.rounded_border(dr, c.len(3.5), 1.0, c.ink(LED.alpha(0.22)));
    words(ui, c, "24", c.rect(d.x + 5.0, d.y + 3.0, 26.0, 17.0), 14.0, Weight::Black, LED, Align::Left);
    let after = c.at(d.x + 5.0, d.y + 9.0) + Vec2::new(ui.width("24", c.font(14.0), Weight::Black) + c.len(3.0), 0.0);
    words(ui, c, "/ 03", Rect::new(after.x, after.y, c.len(28.0), c.len(12.0)), 8.0, Weight::Bold, LED.alpha(0.7), Align::Left);
    words(ui, c, "Hakenfelde", c.rect(d.x + 5.0, d.y + 19.0, d.w - 10.0, 12.0), 8.0, Weight::Bold, LED, Align::Left);
    // the next stop: the old one going and the new one coming as it refreshes
    let next = |k: usize| NEXT_STOPS[k % NEXT_STOPS.len()];
    let out = 1.0 - smoothstep((since - 0.1) / 0.2);
    let into = smoothstep((since - 0.28) / 0.25);
    let row = c.rect(d.x + 5.0, d.y + 31.0, d.w - 10.0, 11.0);
    if out > 0.0 {
        words(ui, c, &format!("› {}", next(state + NEXT_STOPS.len() - 1)), row, 7.5, Weight::Medium, LED.alpha(0.8 * out), Align::Left);
    }
    words(ui, c, &format!("› {}", next(state)), row, 7.5, Weight::Medium, LED.alpha(0.8 * into), Align::Left);
    if since < 0.55 {
        let u = since / 0.55;
        let band = c.len(9.0);
        let y = dr.y - band + (dr.h + band) * u;
        let b = Rect::new(dr.x + 1.0, y.max(dr.y + 1.0), dr.w - 2.0, (y + band).min(dr.bottom() - 1.0) - y.max(dr.y + 1.0));
        if b.h > 0.0 {
            ui.p().gradient(b, LED.alpha(0.0), c.ink(LED.alpha(0.2 * (1.0 - u))));
        }
    }
    // its keys
    let p = ui.p();
    for k in 0..12 {
        let kr = c.rect(r.x + 6.0 + (k % 3) as f32 * 20.5, r.y + 78.0 + (k / 3) as f32 * 13.0, 17.5, 10.0);
        let tint = match k {
            9 => Color::rgba(28, 74, 52, 1.0),
            11 => Color::rgba(84, 34, 36, 1.0),
            _ => Color::rgba(31, 38, 55, 1.0),
        };
        p.rounded(kr, c.len(2.5), c.ink(tint));
        p.rounded_border(kr, c.len(2.5), 1.0, c.ink(Color::WHITE.alpha(0.05)));
    }
    p.rounded(c.rect(r.x + r.w * 0.5 - 11.0, r.bottom() - 6.0, 22.0, 2.2), c.len(1.1), c.ink(Color::WHITE.alpha(0.35)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn everything() -> Ctx {
        Ctx { phone: false, map: true, entries: true, bus: true, showroom: false, own_line: false, server: false, company: false }
    }

    #[test]
    fn the_companys_tour_is_translated() {
        for s in COMPANY_STOPS {
            let (chapter, title, text) = s.words();
            for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
                for k in [chapter, title, text, "Off to work", "Your own bus company, %{name}"] {
                    assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
                }
            }
        }
    }

    #[test]
    fn the_companys_tour_shows_its_pages_or_the_founding() {
        // before there is a company: its welcome and the founding wizard
        assert_eq!(company_plan(&everything()), vec![CoWelcome, CoFound]);
        // with one: its welcome, its clock and tabs, every page, and the "?" at the end
        let p = company_plan(&Ctx { company: true, ..everything() });
        assert_eq!(p.len(), COMPANY_STOPS.len() - 1);
        assert!(!p.contains(&CoFound) && p.first() == Some(&CoWelcome) && p.last() == Some(&CoDone));
        // the launcher's tour has none of them, and theirs none of its
        assert!(plan(&Ctx { company: true, ..everything() }).iter().all(|s| !COMPANY_STOPS.contains(s)));
        for s in COMPANY_STOPS {
            let (chapter, title, text) = s.words();
            assert!(!chapter.is_empty() && !title.is_empty() && text.len() > 30, "{s:?}");
            let sentences = text.matches(". ").count() + 1;
            assert!((1..=3).contains(&sentences), "{s:?}: {sentences} sentences");
            match s.place() {
                Place::Company(tab) => assert!(tab <= 6, "{s:?}"),
                Place::Home => assert_eq!(s, CoDone),
                p => panic!("{s:?}: {p:?} is no place of the company's"),
            }
        }
        // (each page's stop is on its tab)
        assert_eq!((CoFleet.place(), CoPlanning.place(), CoCareer.place()), (Place::Company(1), Place::Company(5), Place::Company(6)));
    }

    #[test]
    fn with_everything_chosen_every_stop_but_the_phones_end_is_made() {
        let p = plan(&everything());
        assert_eq!(p.len(), STOPS.len() - 1);
        assert_eq!(p.first(), Some(&Welcome));
        assert_eq!(p.last(), Some(&Done));
        assert!(!p.contains(&Goodbye));
    }

    #[test]
    fn without_a_map_what_needs_one_is_left_out_and_nothing_is_chosen() {
        let p = plan(&Ctx { map: false, entries: false, ..everything() });
        for s in [StartAt, Day, ShiftLength, ShiftList, ShiftRoute, FreeStarts, FreeLine] {
            assert!(!p.contains(&s), "{s:?} needs a map");
        }
        for s in [Welcome, Modes, Record, Languages, Steps, MapViews, MapTile, MainAction, Buses, LookIn3d, Navigator, CityMap, Companion, Done] {
            assert!(p.contains(&s), "{s:?} needs no map");
        }
    }

    #[test]
    fn a_phone_gets_the_cards_alone() {
        let p = plan(&Ctx { phone: true, ..everything() });
        assert_eq!(p, vec![Welcome, Navigator, CityMap, Companion, Goodbye]);
        assert!(p.iter().all(|s| matches!(s.target(), Target::Card(_))));
    }

    #[test]
    fn the_bus_and_free_stops_follow_what_their_steps_show() {
        let p = plan(&Ctx { showroom: true, ..everything() });
        assert!(!p.contains(&Buses) && !p.contains(&LookIn3d));
        let p = plan(&Ctx { bus: false, ..everything() });
        assert!(p.contains(&Buses) && !p.contains(&LookIn3d));
        let p = plan(&Ctx { own_line: true, server: true, entries: false, ..everything() });
        assert!(!p.contains(&FreeStarts) && p.contains(&FreeLine) && !p.contains(&Day) && !p.contains(&StartAt));
    }

    #[test]
    fn every_stop_has_its_words_and_its_place() {
        for s in STOPS {
            let (chapter, title, text) = s.words();
            assert!(!chapter.is_empty() && !title.is_empty() && text.len() > 30, "{s:?}");
            // (two or three sentences at most, as Omsi-Hub's)
            let sentences = text.matches(". ").count() + 1;
            assert!((1..=3).contains(&sentences), "{s:?}: {sentences} sentences");
            match s.place() {
                Place::On(Step::Duty, m) => assert_eq!(m, Some(Mode::Shift), "{s:?}: the duty step is a shift's"),
                Place::On(Step::Start, m) => assert_eq!(m, Some(Mode::Free), "{s:?}: the start point is a free drive's"),
                Place::Keep => assert!(matches!(s.target(), Target::Card(_)), "{s:?}: only a card stays where it is"),
                _ => {}
            }
        }
        // the end puts the player back where they were
        assert_eq!(Done.place(), Place::Home);
    }

    #[test]
    fn the_keys_go_on_back_and_out() {
        assert_eq!(key_command(&[Key::Right]), Some(Command::Next));
        assert_eq!(key_command(&[Key::Enter]), Some(Command::Next));
        assert_eq!(key_command(&[Key::Left]), Some(Command::Back));
        assert_eq!(key_command(&[Key::Escape]), Some(Command::Skip));
        assert_eq!(key_command(&[Key::Up, Key::Tab]), None);
        assert_eq!(key_command(&[Key::Tab, Key::Left, Key::Right]), Some(Command::Back));
    }

    #[test]
    fn on_back_and_out_through_the_stops() {
        assert_eq!(after(Command::Next, 0, 5), Some(1));
        assert_eq!(after(Command::Next, 4, 5), None, "Next on the last stop ends the tour");
        assert_eq!(after(Command::Back, 3, 5), Some(2));
        assert_eq!(after(Command::Back, 0, 5), Some(0), "nothing before the first");
        assert_eq!(after(Command::Skip, 2, 5), None);
        // a stop left out: going forward the next one takes its place, going back the one before
        assert_eq!(without(2, 4, true), Some(2));
        assert_eq!(without(4, 4, true), None, "the last one left out going forward ends it");
        assert_eq!(without(2, 4, false), Some(1));
        assert_eq!(without(0, 4, false), Some(0));
        assert_eq!(without(0, 0, true), None);
    }

    const SCREEN: Vec2 = Vec2::new(1440.0, 900.0);
    const BUBBLE: Vec2 = Vec2::new(372.0, 260.0);

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn whole_in_window(r: Rect, screen: Vec2) -> bool {
        r.x >= MARGIN - 0.01 && r.y >= MARGIN - 0.01 && r.right() <= screen.x - MARGIN + 0.01 && r.bottom() <= screen.y - MARGIN + 0.01
    }

    #[test]
    fn the_bubble_goes_where_it_fits_beside_the_spotlight() {
        // the bar's flags, top right: under them, in the window
        let flags = hole_round(Rect::new(1200.0, 22.0, 150.0, 40.0), SCREEN);
        let (b, side) = place(flags, BUBBLE, SCREEN);
        assert_eq!(side, Side::Below);
        assert!(whole_in_window(b, SCREEN) && !overlaps(b, flags));
        // the main action, bottom right: above it
        let main = hole_round(Rect::new(1196.0, 807.0, 220.0, 62.0), SCREEN);
        let (b, side) = place(main, BUBBLE, SCREEN);
        assert_eq!(side, Side::Above);
        assert!(whole_in_window(b, SCREEN) && !overlaps(b, main));
        // a part of the sheet on the left: to its right, over the map
        let length = hole_round(Rect::new(40.0, 170.0, 360.0, 110.0), SCREEN);
        let (b, side) = place(length, BUBBLE, SCREEN);
        assert_eq!(side, Side::Right);
        assert!(whole_in_window(b, SCREEN) && !overlaps(b, length));
        // the whole sheet on the left, as tall as the window: to its right as well
        let sheet = hole_round(Rect::new(22.0, 85.0, 395.0, 785.0), SCREEN);
        assert_eq!(place(sheet, BUBBLE, SCREEN).1, Side::Right);
        // the bus tiles, filling the window: over their corner, still whole in the window
        let grid = hole_round(Rect::new(22.0, 85.0, 1396.0, 700.0), SCREEN);
        let (b, side) = place(grid, BUBBLE, SCREEN);
        assert_eq!(side, Side::Over);
        assert!(whole_in_window(b, SCREEN));
    }

    #[test]
    fn the_bubble_stays_in_a_phones_window() {
        let screen = Vec2::new(420.0, 900.0);
        let size = Vec2::new(388.0, 300.0);
        for part in [Rect::new(10.0, 10.0, 60.0, 30.0), Rect::new(300.0, 820.0, 100.0, 60.0), Rect::new(0.0, 300.0, 420.0, 200.0)] {
            let (b, _) = place(hole_round(part, screen), size, screen);
            assert!(whole_in_window(b, screen), "{part:?} -> {b:?}");
        }
    }

    #[test]
    fn the_arrow_points_from_the_bubble_at_the_spotlight() {
        let hole = Rect::new(600.0, 100.0, 200.0, 60.0);
        for (side, b) in [
            (Side::Below, Rect::new(500.0, 178.0, 372.0, 200.0)),
            (Side::Above, Rect::new(500.0, -118.0, 372.0, 200.0)),
            (Side::Right, Rect::new(818.0, 30.0, 372.0, 200.0)),
            (Side::Left, Rect::new(210.0, 30.0, 372.0, 200.0)),
        ] {
            let [a, tip, z] = arrow(b, side, hole).unwrap();
            // the base on the bubble's edge, the tip outside it, towards the spotlight
            assert!(!b.inset(1.0).contains(tip), "{side:?}: the tip is outside the bubble");
            let mid = (a + z) * 0.5;
            assert!((tip - hole.center()).length() < (mid - hole.center()).length(), "{side:?}: the tip is nearer the spotlight");
            assert!(((a - z).length() - ARROW_W).abs() < 0.01);
        }
        assert!(arrow(Rect::new(0.0, 0.0, 100.0, 100.0), Side::Over, hole).is_none());
        // a spotlight far to the side: the arrow stays clear of the bubble's corner
        let b = Rect::new(500.0, 178.0, 372.0, 200.0);
        let [_, tip, _] = arrow(b, Side::Below, Rect::new(0.0, 100.0, 50.0, 60.0)).unwrap();
        assert!(tip.x >= b.x + SHEET_RADIUS + ARROW_W * 0.5 - 0.01);
    }

    #[test]
    fn the_route_is_framed_where_the_map_shows_it() {
        let map = Rect::new(440.0, 85.0, 960.0, 700.0);
        let r = route_box(&[Vec2::new(600.0, 300.0), Vec2::new(900.0, 500.0), Vec2::new(-50.0, 40.0)], map).unwrap();
        assert!(r.x <= 600.0 && r.right() >= 900.0 && r.y <= 300.0 && r.bottom() >= 500.0);
        assert!(r.x >= map.x && r.right() <= map.right(), "the stop off the map does not count");
        // one stop: not a dot
        let one = route_box(&[Vec2::new(700.0, 400.0)], map).unwrap();
        assert!(one.w >= 160.0 && one.h >= 120.0);
        assert!(route_box(&[Vec2::new(10.0, 10.0)], map).is_none());
        assert!(route_box(&[], map).is_none());
    }

    /// Whether `at` lies in a triangle of the painter's that is not clear all over.
    fn covered(p: &Painter, at: Vec2) -> bool {
        p.verts.chunks(3).any(|t| {
            let v: Vec<Vec2> = t.iter().map(|v| Vec2::new(v.pos[0], v.pos[1])).collect();
            let alpha = t.iter().map(|v| v.color[3]).fold(0.0f32, f32::max);
            let s = |a: Vec2, b: Vec2| (b - a).perp_dot(at - a);
            let (d0, d1, d2) = (s(v[0], v[1]), s(v[1], v[2]), s(v[2], v[0]));
            let inside = (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0);
            inside && alpha > 0.0 && (d0.abs() + d1.abs() + d2.abs()) > 1e-3
        })
    }

    #[test]
    fn the_dark_covers_the_window_but_the_spotlight() {
        let screen = Rect::new(0.0, 0.0, 1440.0, 900.0);
        let hole = Rect::new(300.0, 200.0, 400.0, 160.0);
        let mut p = Painter::new();
        dim(&mut p, screen, hole, HOLE_RADIUS, DIM);
        assert!(!covered(&p, hole.center()), "the spotlight is clear");
        assert!(!covered(&p, Vec2::new(hole.x + 30.0, hole.y + 2.0)), "up to its edge");
        for at in [Vec2::new(1.0, 1.0), Vec2::new(1439.0, 899.0), Vec2::new(hole.x - 40.0, hole.center().y), Vec2::new(hole.center().x, hole.bottom() + 40.0), Vec2::new(hole.right() + 13.0, hole.y - 13.0), Vec2::new(720.0, 600.0)] {
            assert!(covered(&p, at), "{at:?} is dark");
        }
        // a spotlight shrunk to a point (a card): all dark but a soft dot
        let mut p = Painter::new();
        dim(&mut p, screen, Rect::new(720.0, 450.0, 0.0, 0.0), 0.0, DIM);
        assert!(covered(&p, Vec2::new(720.0, 470.0)) && covered(&p, Vec2::new(100.0, 100.0)));
    }

    const SHARES: [f32; 4] = [0.0, 0.42, 0.7, 1.0];

    #[test]
    fn the_welcomes_bus_comes_in_drives_from_stop_to_stop_and_comes_round_again() {
        // it comes in at the first stop, standing
        let first = ride(0.0, &SHARES);
        assert_eq!((first.along, first.speed, first.stop, first.seen), (0.0, 0.0, 0, 0.0));
        assert!(ride(FIRST_S * 0.9, &SHARES).seen == 1.0 && ride(FIRST_S * 0.9, &SHARES).speed == 0.0);
        // away gently, fastest half way, and standing at the next stop exactly
        let away = ride(FIRST_S + 0.05, &SHARES);
        assert!(away.along > 0.0 && away.along < 0.01 && away.speed < 0.1 && away.stop == 0);
        let leg = leg_s(&SHARES, 0);
        assert!(ride(FIRST_S + leg * 0.5, &SHARES).speed > 0.99);
        let at = ride(FIRST_S + leg + 0.2, &SHARES);
        assert_eq!((at.along, at.stop, at.speed, at.left), (0.42, 1, 0.0, 0.0));
        // a longer leg takes longer, a short one not too short
        assert!(leg_s(&SHARES, 0) > leg_s(&SHARES, 1) && leg_s(&SHARES, 1) >= LEG_MIN_S);
        // never backwards within a round, always on the way
        let round = round_s(&SHARES);
        let mut last = 0.0;
        for k in 0..400 {
            let r = ride(round * k as f32 / 400.0, &SHARES);
            assert!(r.along >= last - 1e-4 && (0.0..=1.0).contains(&r.along) && (0.0..=1.0).contains(&r.seen));
            last = r.along;
        }
        // it stands longest at the terminus, fades there, and comes in at the start again
        let end = ride(round - FADE_S - 0.5, &SHARES);
        assert_eq!((end.along, end.stop, end.seen), (1.0, 3, 1.0));
        assert!(ride(round - FADE_S * 0.3, &SHARES).seen < 0.5);
        assert_eq!(ride(round + 0.01, &SHARES).stop, 0);
    }

    #[test]
    fn a_stops_name_shows_while_the_bus_is_there_and_it_rings_once() {
        let leg = leg_s(&SHARES, 0);
        let arrive = FIRST_S + leg;
        assert_eq!(ride(arrive - 0.6, &SHARES).name_shown(), 0.0, "on its way: the stop before's name has gone");
        assert!(ride(arrive + NAME_IN_S + 0.05, &SHARES).name_shown() > 0.99);
        let pulse = ride(arrive + 0.1, &SHARES).pulse().unwrap();
        assert!(pulse > 0.0 && pulse < 0.2);
        assert!(ride(arrive + PULSE_S + 0.1, &SHARES).pulse().is_none());
        // leaving: the name goes soon after, no ring while it drives
        let left = ride(arrive + DWELL_S + NAME_OUT_S + 0.05, &SHARES);
        assert!(left.name_shown() < 0.01 && left.pulse().is_none() && left.stop == 1);
        // standing still (no animations): the name there, no ring
        let still = Ride::standing(1, &SHARES, 10.0);
        assert_eq!((still.name_shown(), still.pulse()), (1.0, None));
    }

    #[test]
    fn the_arrival_rings_in_turn_and_its_sparkles_twinkle_softly() {
        let (rings, sparkles) = celebration(None);
        assert!(rings[0].is_some() && sparkles.iter().all(|s| *s > 0.5), "standing still: a whole picture");
        assert_eq!(celebration(Some(0.0)).0, [None, None], "nothing at once");
        let mut seen = [false; 2];
        for k in 0..800 {
            let (rings, sparkles) = celebration(Some(k as f32 * 0.01));
            for (i, r) in rings.iter().enumerate() {
                if let Some(u) = r {
                    assert!((0.0..1.0).contains(u));
                    seen[i] = true;
                }
            }
            assert!(sparkles.iter().all(|s| (0.0..=1.0).contains(s)));
        }
        assert_eq!(seen, [true, true]);
        // a sparkle changes slowly (no flicker from one frame to the next)
        let (a, b) = (celebration(Some(3.0)).1, celebration(Some(3.0 + 1.0 / 60.0)).1);
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.05));
    }

    #[test]
    fn the_navigators_board_moves_on_a_row_at_a_time() {
        let (f, seen) = board_focus(0.0);
        assert_eq!((f, seen), (1.0, 0.0));
        let mut last = 1.0;
        let mut rows = Vec::new();
        for k in 0..1000 {
            let (f, seen) = board_focus(k as f32 * 0.01);
            assert!((1.0..=3.0).contains(&f) && (0.0..=1.0).contains(&seen));
            if f >= last {
                last = f;
            } else {
                // (only when it starts again, faded out)
                assert!(board_focus(k as f32 * 0.01 - 0.01).1 < 0.05, "back to the top only unseen");
                last = f;
            }
            if f.fract() == 0.0 && !rows.contains(&(f as usize)) {
                rows.push(f as usize);
            }
        }
        assert_eq!(rows, vec![1, 2, 3], "it stands at every row from the second to the terminus");
        // the navigator's bus is between the stop before and the one it heads for
        let stops = [0.0, 100.0, 200.0, 300.0];
        assert!((heading(1.0, &stops) - 62.0).abs() < 1e-3 && (heading(2.5, &stops) - 212.0).abs() < 1e-3 && heading(3.0, &stops) < 300.0);
    }

    #[test]
    fn signing_on_types_the_number_and_starts_again() {
        let start = typing(0.0);
        assert_eq!((start.typed, start.tap, start.done), (0, None, 0.0));
        let mut typed = 0;
        let mut taps = Vec::new();
        for k in 0..600 {
            let ty = typing(k as f32 * 0.01);
            assert!(ty.typed >= typed || ty.typed == 0, "digits are only added, until it clears");
            if ty.typed > typed {
                taps.push(ty.tap.unwrap().0);
            }
            typed = ty.typed;
            if ty.done > 0.0 {
                assert_eq!(ty.typed, NUMBER_KEYS.len(), "signed on with the whole number");
            }
        }
        assert_eq!(&taps[..NUMBER_KEYS.len()], &NUMBER_KEYS, "the keys of 482913, in turn");
        assert!((0..600).any(|k| typing(k as f32 * 0.01).done == 1.0));
        assert!((0..600).any(|k| typing(k as f32 * 0.01).seen < 0.1));
    }

    #[test]
    fn the_phone_and_the_tablet_refresh_now_and_then() {
        let mut changes = 0;
        let mut last = refresh(0.0).0;
        for k in 1..1200 {
            let (state, since) = refresh(k as f32 * 0.01);
            assert!(state < 3 && since >= 0.0 && since < 3.5);
            if state != last {
                assert!(since < 0.02, "a refresh starts its effects");
                changes += 1;
            }
            last = state;
        }
        assert!((3..=4).contains(&changes), "{changes}");
        assert!(refresh(0.0).1 > 1.0, "not at once as the card comes");
    }

    #[test]
    fn the_night_city_is_laid_out_round_its_river_park_and_route() {
        let river = river_path();
        let b = blocks();
        assert!(b.len() > 120, "{}", b.len());
        for (r, _) in &b {
            assert!(r.w > 2.0 && r.h > 2.0, "{r:?}");
            assert!(!overlap(*r, PARK_AREA), "{r:?} in the park");
            assert!(nearest(river.points(), r.center()).0 > RIVER_W * 0.5, "{r:?} in the river");
        }
        for (i, (a, _)) in b.iter().enumerate() {
            assert!(b[i + 1..].iter().all(|(o, _)| !overlap(*a, *o)), "{a:?} overlaps another block");
        }
        // the stops lie on the route, in its order, the terminus at its end; the drawing shows
        // them all, with room for a name over each
        let route = route_path();
        let along: Vec<f32> = STOP_AT.iter().map(|q| {
            let (d, s) = nearest(route.points(), v2(*q));
            assert!(d < 0.5, "{q:?} is off the route by {d}");
            s
        }).collect();
        assert!(along.windows(2).all(|w| w[1] > w[0] + 80.0));
        assert!((along[3] - route.length()).abs() < 0.5);
        assert!(STOP_AT.iter().all(|q| q.0 > 20.0 && q.0 < PIC_W - 20.0 && q.1 > 40.0 && q.1 < PIC_H - 20.0));
        // a rounded corner stays near its street and runs on smoothly
        let pts = route.points();
        assert!(pts.windows(3).all(|w| (w[1] - w[0]).normalize_or_zero().dot((w[2] - w[1]).normalize_or_zero()) > 0.9));
    }

    #[test]
    fn a_shape_is_cut_to_a_screen() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let cut = clip_poly(&[Vec2::new(-5.0, 2.0), Vec2::new(5.0, 2.0), Vec2::new(5.0, 8.0), Vec2::new(-5.0, 8.0)], r);
        let area: f32 = (1..cut.len() - 1).map(|k| (cut[k] - cut[0]).perp_dot(cut[k + 1] - cut[0]).abs() * 0.5).sum();
        assert!((area - 30.0).abs() < 1e-3, "{area}");
        assert!(cut.iter().all(|q| q.x >= -1e-4 && q.x <= 10.0001));
        assert!(clip_poly(&[Vec2::new(20.0, 20.0), Vec2::new(30.0, 20.0), Vec2::new(30.0, 30.0)], r).is_empty());
    }

    #[test]
    fn every_drawing_is_drawn_still_and_moving_at_every_size() {
        let mut ui = Ui::new();
        for pic in [Picture::Welcome, Picture::Navigator, Picture::CityMap, Picture::Companion, Picture::Arrived] {
            for room in [Rect::new(450.0, 260.0, 540.0, 196.0), Rect::new(16.0, 200.0, 368.0, 133.6), Rect::new(600.0, 300.0, 1080.0, 392.0)] {
                for motion in [true, false] {
                    for t in [0.0, 1.7, 6.3] {
                        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
                        ui.motion = motion;
                        ui.time = 100.0 + t;
                        picture(&mut ui, pic, room, 1.0);
                        let (layers, verts, _) = ui.finish();
                        assert!(verts.len() > 3000, "{pic:?} {room:?}: {}", verts.len());
                        // (a clip for a device's screen, no layer for every piece)
                        assert!(layers.len() <= 8, "{pic:?}: {} layers", layers.len());
                        assert!(verts.iter().all(|v| v.pos.iter().chain(&v.color).all(|x| x.is_finite())));
                    }
                }
            }
        }
    }

    #[test]
    fn the_first_of_a_name_drawn_counts() {
        clear_anchors();
        anchor("map-tile-any", Rect::new(10.0, 10.0, 5.0, 5.0));
        anchor("map-tile-any", Rect::new(50.0, 50.0, 5.0, 5.0));
        assert_eq!(anchored("map-tile-any"), Some(Rect::new(10.0, 10.0, 5.0, 5.0)));
        assert_eq!(anchored("map-tile"), None);
        clear_anchors();
        assert_eq!(anchored("map-tile-any"), None);
    }

    fn view(part: Option<Rect>, at: usize, total: usize) -> View {
        let (chapter, title, text) = if part.is_some() { Modes.words() } else { Navigator.words() };
        View { chapter, title: title.to_string(), text, picture: part.is_none().then_some(Picture::Navigator), part, at, total, first: at == 0, last: at + 1 == total, phone: false, open: 1.0, words: 1.0, closing: false, company: false }
    }

    fn frame(ui: &mut Ui, v: &View, g: &mut Glide) -> Option<Command> {
        ui.begin(Vec2::new(1440.0, 900.0), 1.0, 1.0 / 60.0);
        overlay(ui, v, g)
    }

    #[test]
    fn the_bubble_has_its_buttons_beside_the_spotlight_and_they_answer() {
        let mut ui = Ui::new();
        let mut g = Glide::default();
        let tiles = Rect::new(56.0, 200.0, 1328.0, 260.0);
        let v = view(Some(tiles), 3, 11);
        assert_eq!(frame(&mut ui, &v, &mut g), None);
        let next = *ui.drawn.get(&id_of("tour-next")).expect("Next is drawn");
        let back = *ui.drawn.get(&id_of("tour-back")).expect("Back is drawn");
        let skip = *ui.drawn.get(&id_of("tour-skip")).expect("Skip is drawn");
        assert!(!overlaps(next, tiles) && !overlaps(back, tiles) && !overlaps(skip, tiles), "the bubble is beside the part, not on it");
        assert!(skip.x < back.x && back.x < next.x);
        ui.input.mouse = next.center();
        ui.input.pressed = true;
        ui.input.down = true;
        assert_eq!(frame(&mut ui, &v, &mut g), None);
        (ui.input.pressed, ui.input.down, ui.input.released) = (false, false, true);
        assert_eq!(frame(&mut ui, &v, &mut g), Some(Command::Next));
        ui.input.released = false;
        ui.input.keys.push(Key::Escape);
        assert_eq!(frame(&mut ui, &v, &mut g), Some(Command::Skip));
    }

    #[test]
    fn the_first_stop_has_no_back_and_the_last_no_skip() {
        let mut ui = Ui::new();
        let mut g = Glide::default();
        frame(&mut ui, &view(None, 0, 11), &mut g);
        assert!(ui.drawn.contains_key(&id_of("tour-next")) && ui.drawn.contains_key(&id_of("tour-skip")));
        assert!(!ui.drawn.contains_key(&id_of("tour-back")));
        frame(&mut ui, &view(None, 10, 11), &mut g);
        assert!(ui.drawn.contains_key(&id_of("tour-back")) && !ui.drawn.contains_key(&id_of("tour-skip")));
        // a card lies in the middle of the window
        let next = ui.drawn[&id_of("tour-next")];
        assert!((next.right() - (720.0 + CARD_W * 0.5 - PAD)).abs() < 1.0, "{next:?}");
    }

    #[test]
    fn closing_it_takes_no_clicks() {
        let mut ui = Ui::new();
        let mut g = Glide::default();
        let mut v = view(Some(Rect::new(100.0, 100.0, 200.0, 50.0)), 2, 11);
        v.closing = true;
        v.open = 0.4;
        ui.input.keys.push(Key::Enter);
        assert_eq!(frame(&mut ui, &v, &mut g), None);
        assert!(!ui.drawn.contains_key(&id_of("tour-next")));
    }
}
