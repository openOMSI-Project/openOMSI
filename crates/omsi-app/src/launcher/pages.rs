//! The launcher's other pages: the driver's profile, the settings, the key bindings, the
//! running games, the mods and where things are.

use super::state::{fmt_bytes, hhmm, short_map};
use super::theme::*;
use super::ui::{id_of, matches, ButtonKind, Key, Ui};
use super::{Launcher, Page};
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use serde_json::{json, Value};

#[derive(Default)]
pub struct PagesView {
    /// The name of a driver being made, and whether its field is open (on the drivers step
    /// and on the service record).
    pub new_driver: String,
    pub new_driver_open: bool,
    pub confirm_delete: Option<std::time::Instant>,
    /// The driver the drivers step asks about deleting (its dialog is open while set).
    pub delete_driver: Option<String>,
    /// The "reset every setting" dialog is open.
    pub confirm_reset: bool,
    pub kb_filter: [String; 2],
    /// The OMSI-style "Add event..." browser is open for a keyboard section.
    pub kb_events: [bool; 2],
    /// (section, index) of the binding waiting for a key.
    pub capturing: Option<(usize, usize)>,
    pub drop_hover: bool,
    pub setup_root: Option<String>,
    pub setup_game: Option<String>,
    /// The Controls page's tab: 0 the keyboard, 1 the game controllers.
    pub controls_tab: usize,
    /// On a phone, which list of keys the Controls page shows (0 the bus's, 1 the game's).
    pub kb_section: usize,
    /// The Settings page's tab (see `SETTINGS_TABS`).
    pub settings_tab: usize,
    pub pads: PadsView,
    pub tt: super::timetable::TimetableView,
    /// The editor hub's map for the map objects, and the line editor.
    pub hub: super::editor_hub::HubView,
    pub lines: super::lineeditor::LineEditorView,
    pub depots: super::depoteditor::DepotEditorView,
}

/// The game controllers tab: the devices `gamectrler.cfg` sets up, the ones connected now,
/// the one shown, a button being waited for.
#[derive(Default)]
pub struct PadsView {
    pub io: Option<crate::controllers::Devices>,
    /// The set-up assistant, while it runs.
    pub wizard: Option<Wizard>,
    feedback_test: bool,
    pub devices: Option<Vec<crate::controllers::DeviceCfg>>,
    pub selected: usize,
    /// Waiting for a button of the shown device to be pressed (to add its binding).
    pub capturing: bool,
    /// A button found through "Add a button", kept visible even past the highlight.
    pub revealed_button: Option<usize>,
    pub dirty: bool,
    /// The button last pressed on the shown device and when: its line is lit, so that one
    /// sees which it is and what it does, and can give it an action there.
    pub last_pressed: Option<(usize, std::time::Instant)>,
    /// "Remove this device" clicked once, and when: a second click removes it.
    confirm_remove: Option<std::time::Instant>,
}

/// The set-up assistant of a device: the player lets go of everything, then turns the wheel
/// to the left and presses each pedal in turn; what moved most each time is that control
/// (and which way it runs), as OMSI's options dialog has the player choose by hand.
pub struct Wizard {
    pub step: usize,
    /// Where each axis rests, and where it stood at each step (left, throttle, brake,
    /// clutch).
    pub rest: [Option<f32>; 8],
    pub at: Vec<[Option<f32>; 8]>,
    pub error: Option<String>,
    calibration: Option<(std::time::Instant, crate::ffb_calibration::Calibration)>,
    ff_choice: Option<bool>,
    test_strength: f32,
}

impl PadsView {
    pub(super) fn cancel_feedback_test(&mut self) {
        release_feedback(&mut self.io, &mut self.feedback_test);
        if let Some((_, test)) = self.wizard.as_mut().and_then(|w| w.calibration.as_mut()) {
            if test.result.is_none() {
                test.fail("The test was interrupted. Please try again.");
            }
        }
    }

    /// Give every controller handle up before handing the hardware to the game. The normal
    /// controller list uses non-exclusive DirectInput too, not only the force-feedback test.
    pub(super) fn release_io(&mut self) {
        self.cancel_feedback_test();
        self.io = None;
    }
}

fn release_feedback(io: &mut Option<crate::controllers::Devices>, active: &mut bool) {
    if *active {
        *io = None;
        *active = false;
    }
}

// --- the pages' heads -----------------------------------------------------------------------

/// A page's icon and what it is for: the sheet's head on a desktop, the line under the bar on
/// a phone (the page itself draws neither: its name was drawn twice, in the bar and on it).
pub fn about(page: Page, controls_tab: usize) -> (&'static str, &'static str) {
    match page {
        Page::Profile => ("badge", "Everything the logbook kept, from the first duty to the last."),
        Page::Settings => ("settings", "Every change is saved at once; the game reads them when it starts."),
        Page::Controls if controls_tab == 0 => ("keyboard", "Click a key and press the new one (hold Shift, Ctrl or Alt for a combination); Escape leaves it as it is."),
        Page::Controls => ("sports_esports", "What each axis and button of a wheel, pedals or joystick does - OMSI 2's gamectrler.cfg, kept in the content folder."),
        Page::Sessions => ("sports_esports", "The games you started, and who drives with you."),
        Page::Mods => ("extension", "A bus, a map, scenery, a whole OMSI folder - as a folder or a .zip, .7z or .rar. The original OMSI 2 folder is never written to."),
        Page::Multiplayer => ("groups", "Drive with friends by a code, or on servers that are always on."),
        Page::Timetable => ("schedule", "A map's lines: their tours and when each trip leaves. Saved as the line's .ttl (in the content folder; OMSI 2's own files stay as they are)."),
        Page::Tutorials => ("help", "OMSI 2's own lessons: each opens its situation, with its pages beside the picture (Enter turns the page)."),
        Page::Setup => ("folder_open", "Where the original game and this one are."),
        Page::Editor => ("construction", "Make the map your own: lines of your own, liveries, the timetable and the map's objects."),
        Page::Lines => ("route", "Click the stops in order: the way between them is found over the roads. Saved, the line is driven by you and by the timetable's buses."),
        Page::Depots => ("departure_board", "Your own depot files: the destinations and what every display shows, the IBIS's stops and routes, special and service trips. A line chooses one, and every bus that drives it gets it."),
        Page::Livery => ("livery_fill", "Paint a bus in 3D: colours, stripes, texts, pictures and shapes, saved as a livery the game offers."),
        Page::Company => ("garage", "Your own transport company: buses, people and lines, in a time of its own."),
        Page::Drive => ("directions_bus", ""),
    }
}

/// What a page keeps in its sheet's head, right-aligned in `r`: the drivers on the service
/// record. Returns where it begins (`r`'s right edge when there is nothing).
pub fn head_tools(l: &mut Launcher, page: Page, r: Rect) -> f32 {
    match page {
        Page::Profile => driver_tools(l, r),
        Page::Company => super::company::head_tools(l, r),
        _ => r.right(),
    }
}

// --- the drivers ----------------------------------------------------------------------------

/// The new driver's name field (one at a time: on the drivers step or on the service record).
const NEW_NAME: &str = "driver-new-name";

/// Start making a driver: the field empty, the typing in it.
pub fn open_new_driver(l: &mut Launcher) {
    l.pages.new_driver_open = true;
    l.pages.new_driver.clear();
    l.ui.focus = Some(id_of(NEW_NAME));
}

/// Make the driver whose name was typed (OMSI's own personnel file, as the game makes one)
/// and choose them.
pub fn create_driver(l: &mut Launcher) {
    let name = l.pages.new_driver.trim().to_string();
    if name.is_empty() {
        return;
    }
    match core::create_profile(&name, "M") {
        Ok(_) => {
            l.state.config.profile = name.clone();
            let _ = core::save_config(&l.state.config);
            l.pages.new_driver.clear();
            l.pages.new_driver_open = false;
            l.state.load_profiles();
            l.state.touched();
            l.state.set_status(omsi_ui::tr("Driver %{name} created.").replace("%{name}", &name), false);
        }
        Err(e) => l.state.set_status(format!("{e:#}"), true),
    }
}

/// The tile of a driver being made, after the others on the drivers step: the name typed into
/// it, Create (or Enter) makes them, the cross (or Escape) leaves it. Some(true) to make them,
/// Some(false) to give up.
pub fn new_driver_tile(ui: &mut Ui, tile: Rect, name: &mut String) -> Option<bool> {
    let focused = ui.focus == Some(id_of(NEW_NAME));
    let enter = focused && ui.input.keys.contains(&Key::Enter);
    let escape = focused && ui.input.keys.contains(&Key::Escape);
    ui.solid(tile);
    ui.p().rounded(tile, SHEET_RADIUS, PANEL);
    ui.p().rounded_border(tile, SHEET_RADIUS, 2.0, accent());
    let mono = Rect::new(tile.x + 17.0, tile.y + 17.0, 44.0, 44.0);
    ui.p().rounded(mono, RADIUS, accent().alpha(0.3));
    match name.trim().chars().next() {
        Some(c) => {
            ui.text_in(&c.to_uppercase().to_string(), mono, 18.0, Weight::Bold, TEXT, Align::Center);
        }
        None => ui.icon("add", mono.center(), 22.0, TEXT),
    }
    ui.text_in("New driver", Rect::new(mono.right() + 12.0, mono.y, tile.right() - mono.right() - 56.0, mono.h), 15.0, Weight::Bold, TEXT, Align::Left);
    let close = Rect::new(tile.right() - 42.0, tile.y + 14.0, 28.0, 28.0);
    let (hc, _, cancel) = ui.interact(id_of("driver-new-cancel"), close);
    ui.icon("close", close.center(), 18.0, if hc { TEXT } else { TEXT_DIM });
    ui.tooltip(close, "Cancel");
    ui.text_input(NEW_NAME, Rect::new(tile.x + 17.0, tile.y + 74.0, tile.w - 34.0, 34.0), name, "The new driver's name", None);
    let create = ui.button("driver-new-create", Rect::new(tile.x + 17.0, tile.y + 116.0, tile.w - 34.0, 30.0), "Create", Some("add"), ButtonKind::Primary);
    if (create || enter) && !name.trim().is_empty() {
        Some(true)
    } else if cancel || escape {
        Some(false)
    } else {
        None
    }
}

/// The drivers on this computer, right-aligned in the row `r`: whose record it is, a new one
/// (typed in place), and deleting the chosen one (pressed twice). Returns where they begin.
pub fn driver_tools(l: &mut Launcher, r: Rect) -> f32 {
    let h = r.h;
    if l.pages.new_driver_open {
        let cancel = Rect::new(r.right() - h, r.y, h, h);
        let cw = l.ui.width("Create", 13.0, Weight::Bold) + 60.0;
        let create = Rect::new(cancel.x - 8.0 - cw, r.y, cw, h);
        let fw = (create.x - 8.0 - r.x).clamp(0.0, 260.0);
        let field = Rect::new(create.x - 8.0 - fw, r.y, fw, h);
        new_driver_row(l, field, create, cancel);
        return field.x;
    }
    let dw = l.ui.width(delete_label(l), 13.0, Weight::Bold) + 56.0;
    let del = Rect::new(r.right() - dw, r.y, dw, h);
    let nw = l.ui.width("New driver", 13.0, Weight::Bold) + 56.0;
    let new = Rect::new(del.x - 8.0 - nw, r.y, nw, h);
    let sw = (new.x - 8.0 - r.x).clamp(0.0, 220.0);
    let pick = Rect::new(new.x - 8.0 - sw, r.y, sw, h);
    driver_buttons(l, (sw > 60.0).then_some(pick), new, Some("New driver"), del);
    pick.x
}

/// The same on a phone, across the page's width from `r`'s top: the driver and a new one in
/// a row, deleting under them. Returns how high it came out.
pub fn driver_tools_narrow(l: &mut Launcher, r: Rect) -> f32 {
    let h = ROW;
    if l.pages.new_driver_open {
        let cancel = Rect::new(r.right() - h, r.y, h, h);
        let create = Rect::new(cancel.x - 8.0 - h, r.y, h, h);
        let field = Rect::new(r.x, r.y, (create.x - 8.0 - r.x).max(0.0), h);
        new_driver_row(l, field, create, cancel);
        return h;
    }
    let new = Rect::new(r.right() - h, r.y, h, h);
    let pick = Rect::new(r.x, r.y, (new.x - 8.0 - r.x).max(0.0), h);
    let del = Rect::new(r.x, r.y + h + 8.0, r.w, h);
    driver_buttons(l, Some(pick), new, None, del);
    2.0 * h + 8.0
}

/// Delete the driver `name` (their personnel file): when it was the chosen one, the first
/// driver left is chosen instead (none left: the drivers step opens a new one's tile).
pub fn delete_driver(l: &mut Launcher, name: &str) {
    match core::delete_profile(name) {
        Ok(()) => {
            l.state.set_status(omsi_ui::tr("Personnel file of %{name} deleted.").replace("%{name}", name), false);
            l.state.profiles.retain(|n| n != name);
            if l.state.config.profile == name {
                l.state.profile = None;
                l.state.config.profile = l.state.profiles.first().cloned().unwrap_or_default();
                let _ = core::save_config(&l.state.config);
            }
            l.state.load_profiles();
        }
        Err(e) => l.state.set_status(format!("{e:#}"), true),
    }
}

/// "Delete this driver", and once pressed, what a second press does.
fn delete_label(l: &Launcher) -> &'static str {
    if l.pages.confirm_delete.is_some_and(|t| t.elapsed().as_secs() < 4) {
        "Click again to delete"
    } else {
        "Delete this driver"
    }
}

/// The new driver's name, Create and the cross, where they were laid out.
fn new_driver_row(l: &mut Launcher, field: Rect, create: Rect, cancel: Rect) {
    let focused = l.ui.focus == Some(id_of(NEW_NAME));
    let enter = focused && l.ui.input.keys.contains(&Key::Enter);
    let escape = focused && l.ui.input.keys.contains(&Key::Escape);
    if l.ui.button("record-new-cancel", cancel, "", Some("close"), ButtonKind::Normal) || escape {
        l.pages.new_driver_open = false;
    }
    l.ui.tooltip(cancel, "Cancel");
    l.ui.text_input(NEW_NAME, field, &mut l.pages.new_driver, "The new driver's name", Some("person"));
    // (a narrow Create is its icon alone)
    let label = if create.w > 2.0 * create.h { "Create" } else { "" };
    if l.ui.button("record-new-create", create, label, Some("add"), ButtonKind::Primary) || enter {
        create_driver(l);
    }
    l.ui.tooltip(create, "Create");
}

/// Choosing the driver (`pick`), making a new one (`new`, its word or only its icon) and
/// deleting the chosen one, where they were laid out.
fn driver_buttons(l: &mut Launcher, pick: Option<Rect>, new: Rect, new_label: Option<&str>, del: Rect) {
    let armed = l.pages.confirm_delete.is_some_and(|t| t.elapsed().as_secs() < 4);
    if l.ui.button("profile-delete", del, delete_label(l), Some("delete"), ButtonKind::Danger) {
        if armed {
            let name = l.state.config.profile.clone();
            delete_driver(l, &name);
            l.pages.confirm_delete = None;
        } else {
            l.pages.confirm_delete = Some(std::time::Instant::now());
        }
    }
    if l.ui.button("profile-new", new, new_label.unwrap_or(""), Some("add"), ButtonKind::Normal) {
        open_new_driver(l);
    }
    l.ui.tooltip(new, "New driver");
    let names = l.state.profiles.clone();
    let mut sel = names.iter().position(|n| *n == l.state.config.profile).unwrap_or(0);
    if let Some(pick) = pick.filter(|_| !names.is_empty()) {
        if l.ui.select("profile", pick, &mut sel, &names) {
            l.state.config.profile = names[sel].clone();
            let _ = core::save_config(&l.state.config);
            l.state.load_profile();
            l.state.touched();
        }
    }
}

/// How far a driver has come from their level towards the next (0 to 1): a level begins at
/// 250 points times the square of the one before (`omsi_launcher_lib::level_of`).
pub fn level_progress(xp: i64, level: i64, next: i64) -> f32 {
    let prev = (level - 1).max(0).pow(2) * 250;
    ((xp - prev) as f32 / (next - prev).max(1) as f32).clamp(0.0, 1.0)
}

// --- the service record ---------------------------------------------------------------------

/// A big figure: its value, unit and name, and a line under it.
struct Tile {
    value: String,
    unit: &'static str,
    label: &'static str,
    note: String,
}

/// A line of a list: what, how much, where (a record's map), and a bar (a rating).
struct Fact {
    label: &'static str,
    value: String,
    note: String,
    bar: Option<f32>,
}

/// A run in the list of the last ones.
struct Run {
    when: String,
    map: String,
    line: String,
    length: String,
    km: String,
    /// (stops on time, stops)
    on_time: Option<(i32, i32)>,
}

/// What the service record shows, worked out from the personnel file and the runs.
struct Figures {
    since: String,
    tiles: Vec<Tile>,
    /// Stops served early, on time and late.
    stops: [i32; 3],
    style: Vec<Fact>,
    records: Vec<Fact>,
    maps: Vec<(String, usize)>,
    buses: Vec<(String, usize)>,
    hours: [usize; 24],
    runs: Vec<Run>,
    /// The trips the game judged at their ends (none yet: None).
    trips: Option<Trips>,
}

/// The record's trips: what they come to (value, its colour, what it is) and the last ones.
struct Trips {
    figures: Vec<(String, Color, &'static str)>,
    last: Vec<TripLine>,
}

/// A trip in the list of the last ones.
struct TripLine {
    when: String,
    line: Option<String>,
    terminus: String,
    /// (stops on time, stops) of a trip to a timetable; None on a free drive.
    on_time: Option<(i32, i32)>,
    free: bool,
    /// The average and the worst delay, as the trip's card writes them ("+1:05").
    average: Option<String>,
    worst: Option<(String, Color)>,
}

/// How a delay is written on the record (as on the trip's card) and its colour there.
fn delay_text(d: f64) -> (String, Color) {
    let c = match crate::nav_duty::punctuality(d) {
        crate::nav_duty::Punctuality::Late => LATE.lighten(0.2),
        crate::nav_duty::Punctuality::Early => EARLY_SOFT,
        crate::nav_duty::Punctuality::OnTime => ON_TIME.lighten(0.25),
    };
    (crate::nav_duty::offset(d), c)
}

/// Stops on time out of all as a colour: green all of them, plain most, red too few.
fn on_time_colour(good: i32, all: i32) -> Color {
    if good == all {
        ON_TIME.lighten(0.25)
    } else if good * 10 >= all * 8 {
        TEXT
    } else {
        LATE.lighten(0.2)
    }
}

/// The record's trips from the profile's (None without any).
fn trips_part(p: &core::Profile, offset: i64) -> Option<Trips> {
    let t = &p.trip_totals;
    if t.trips == 0 {
        return None;
    }
    let on_time = (t.stops - t.early - t.late).max(0);
    let mut figures = vec![(t.trips.to_string(), TEXT, "Trips")];
    if t.stops > 0 {
        figures.push((format!("{:.0} %", on_time as f64 * 100.0 / t.stops as f64), on_time_colour(on_time, t.stops), "On time"));
    }
    if let Some(a) = t.average {
        let (text, c) = delay_text(a);
        figures.push((text, c, "Average delay"));
    }
    if let Some(w) = t.worst {
        let (text, c) = delay_text(w);
        figures.push((text, c, "Worst delay"));
    }
    let last = p
        .trips
        .iter()
        .take(6)
        .map(|r| TripLine {
            when: date_text(r.time, offset, false),
            line: Some(r.line.trim()).filter(|l| !l.is_empty()).map(str::to_string),
            terminus: r.terminus.trim().to_string(),
            on_time: r.timed().then(|| (r.on_time(), r.stops)),
            free: r.free,
            average: r.average.filter(|_| r.timed()).map(|a| delay_text(a).0),
            worst: r.worst.filter(|_| r.timed()).map(delay_text),
        })
        .collect();
    Some(Trips { figures, last })
}

/// "2h 05m", in the interface's language.
pub(crate) fn duration_text(hours: f64) -> String {
    let m = (hours * 60.0).round().max(0.0) as i64;
    omsi_ui::tr("%{h}h %{m}m").replace("%{h}", &(m / 60).to_string()).replace("%{m}", &format!("{:02}", m % 60))
}

/// Unix time `t` as the local (year, month, day, hour, minute), `offset` seconds from UTC.
fn local_time(t: u64, offset: i64) -> (i64, u32, u32, u32, u32) {
    let s = t as i64 + offset;
    let (days, secs) = (s.div_euclid(86400), s.rem_euclid(86400));
    // the civil date of a day number (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32)
}

/// "3 October 2026" (`long`) or "3 Oct 2026, 21:40".
fn date_text(t: u64, offset: i64, long: bool) -> String {
    let (y, m, d, h, min) = local_time(t, offset);
    let k = (m as usize).clamp(1, 12) - 1;
    if long {
        format!("{d} {} {y}", omsi_ui::tr(super::ui::MONTHS_LONG[k]))
    } else {
        format!("{d} {} {y}, {h:02}:{min:02}", omsi_ui::tr(super::ui::MONTHS[k]))
    }
}

fn figures(p: &core::Profile, map_name: &dyn Fn(&str) -> String, bus_name: &dyn Fn(&str) -> String, offset: i64) -> Figures {
    let r = &p.record;
    let tr = |s: &str| omsi_ui::tr(s).into_owned();
    let tile = |value: String, unit: &'static str, label: &'static str, note: String| Tile { value, unit, label, note };
    let tiles = vec![
        tile(r.runs.to_string(), "", "Duties", String::new()),
        tile(duration_text(p.hours), "", "At the wheel", String::new()),
        tile(format!("{:.0}", p.km), "km", "Driven", String::new()),
        tile(p.stops.to_string(), "", "Stops served", String::new()),
        // (a comparison only once there is something to compare: under a hundred tickets
        // "enough for one bus" is a rounding that looks bigger than it is)
        tile(format!("{:.0}", p.tickets), "", "Tickets sold", if p.tickets >= 100.0 { tr("enough to fill %{count} buses").replace("%{count}", &format!("{:.0}", (p.tickets / 100.0).round())) } else { String::new() }),
        tile(format!("{:.2}", p.cash), "", "Takings", if p.hours > 0.0 && p.cash > 0.0 { tr("%{amount} per hour").replace("%{amount}", &format!("{:.2}", p.cash / p.hours)) } else { String::new() }),
    ];
    let fact = |label: &'static str, value: String, note: String, bar: Option<f32>| Fact { label, value, note, bar };
    let rating = |v: f64| Some((v / 100.0).clamp(0.0, 1.0) as f32);
    // (jolts per hundred kilometres, not per run: a run of half an hour and one of eight are
    // not to be compared otherwise)
    let jolts = if r.km > 0.0 { format!("{:.1}", r.jolts as f64 / r.km * 100.0) } else { "-".into() };
    let style = vec![
        fact("Collisions", p.crashes.to_string(), String::new(), None),
        fact("Pedestrians hurt", p.hurt.to_string(), String::new(), None),
        fact("Jolts per 100 km", jolts, String::new(), None),
        fact("Driving style", format!("{:.0} %", p.rating_driving), String::new(), rating(p.rating_driving)),
        fact("Comfort", format!("{:.0} %", p.rating_comfort), String::new(), rating(p.rating_comfort)),
        fact("Ticket selling", format!("{:.0} %", p.rating_tickets), String::new(), rating(p.rating_tickets)),
    ];
    let mut records = Vec::new();
    if let Some(s) = &r.longest {
        records.push(fact("Longest duty", duration_text(s.seconds / 3600.0), map_name(&s.map), None));
    }
    if let Some(s) = &r.furthest {
        records.push(fact("Furthest", format!("{:.1} km", s.metres / 1000.0), map_name(&s.map), None));
    }
    if let Some(s) = &r.busiest {
        records.push(fact("Busiest duty", tr("%{n} tickets").replace("%{n}", &s.tickets.to_string()), map_name(&s.map), None));
    }
    if let Some(s) = &r.most_punctual {
        let good = (s.stops - s.early - s.late).max(0);
        records.push(fact("Closest to the timetable", tr("%{good} of %{all} stops on time").replace("%{good}", &good.to_string()).replace("%{all}", &s.stops.to_string()), map_name(&s.map), None));
    }
    let runs = p.sessions.iter().take(8).map(|s| Run {
        when: date_text(s.time, offset, false),
        map: map_name(&s.map),
        line: s.line.clone().map(|l| s.tour.as_ref().map(|t| format!("{l} / {t}")).unwrap_or(l)).unwrap_or_else(|| tr("Free drive")),
        length: duration_text(s.seconds / 3600.0),
        km: format!("{:.1} km", s.metres / 1000.0),
        on_time: (s.stops > 0).then(|| ((s.stops - s.early - s.late).max(0), s.stops)),
    }).collect();
    Figures {
        since: if r.first > 0 { tr("Driving since %{date}").replace("%{date}", &date_text(r.first, offset, true)) } else { String::new() },
        tiles,
        stops: [p.early.max(0), (p.stops - p.early - p.late).max(0), p.late.max(0)],
        style,
        records,
        maps: r.maps.iter().map(|(m, n)| (map_name(m), *n)).collect(),
        buses: r.buses.iter().map(|(b, n)| (bus_name(b), *n)).collect(),
        hours: r.hours,
        runs,
        trips: trips_part(p, offset),
    }
}

/// The service record, Omsi-Hub's "staat van dienst": the driver and their level at the head,
/// then everything the personnel file and the runs hold - in big figures, bars and short
/// lists - scrolling as a whole. Only what was measured: a figure the runs do not give is
/// left out, not made up.
pub fn profile(l: &mut Launcher, area: Rect) {
    let mut area = area;
    if super::mobile::mobile() {
        // (a phone has no sheet head: the drivers are the page's first rows)
        let used = driver_tools_narrow(l, area) + 16.0;
        area = Rect::new(area.x, area.y + used, area.w, (area.h - used).max(0.0));
    }
    let Some(p) = l.state.profile.as_ref() else {
        l.ui.paragraph("Create a driver to start a personnel file.", Vec2::new(area.x, area.y + 8.0), area.w, 14.0, Weight::Medium, TEXT_DIM);
        return;
    };
    let maps = &l.state.maps;
    let vehicles = &l.state.vehicles;
    let map_name = |m: &str| maps.iter().find(|x| x.file == m).map(|x| if x.friendly.is_empty() { x.name.clone() } else { x.friendly.clone() }).unwrap_or_else(|| short_map(m));
    let bus_name = |b: &str| vehicles.iter().find(|v| v.file == b).map(|v| v.name.clone()).unwrap_or_else(|| b.rsplit('/').next().unwrap_or(b).trim_end_matches(".bus").to_string());
    let f = figures(p, &map_name, &bus_name, core::utc_offset());
    let (name, exists, level, frac, xp, next) = (p.name.clone(), p.exists, p.level, level_progress(p.xp, p.level, p.next_level_xp), p.xp, p.next_level_xp);
    // the head: who, since when; their level on the right, in the line's yellow (a phone:
    // the way to the next level under the name, across the page)
    let narrow = area.w < 560.0;
    let head = Rect::new(area.x, area.y, area.w, if narrow { 92.0 } else { 62.0 });
    let lw = if narrow { 120.0 } else { 220.0f32.min(area.w * 0.4) };
    let word = omsi_ui::tr("Level %{n}").replace("%{n}", &level.to_string()).to_uppercase();
    l.ui.text_in(&name, Rect::new(head.x, head.y + 2.0, head.w - lw - 20.0, 32.0), 26.0, Weight::Bold, TEXT, Align::Left);
    let under = if exists { f.since.clone() } else { omsi_ui::tr("No personnel file yet: the game writes it after the first run.").into_owned() };
    let uw = if narrow { head.w } else { head.w - lw - 20.0 };
    l.ui.text_in(&under, Rect::new(head.x, head.y + 36.0, uw, 18.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    let lv = Rect::new(head.right() - lw, head.y + 2.0, lw, 62.0);
    l.ui.text_in(&word, Rect::new(lv.x, lv.y, lv.w, 20.0), 15.0, Weight::Bold, LINE, Align::Right);
    let shown = l.ui.anim(id_of("record-level"), frac, 0.4);
    let bar = if narrow { Rect::new(head.x, head.y + 64.0, head.w, 5.0) } else { Rect::new(lv.x, lv.y + 28.0, lv.w, 5.0) };
    l.ui.p().rounded(bar, 2.5, HAIRLINE);
    l.ui.p().rounded(Rect::new(bar.x, bar.y, (bar.w * shown).max(5.0), bar.h), 2.5, LINE);
    let to_next = omsi_ui::tr("%{xp} of %{next} points to level %{n}").replace("%{xp}", &xp.to_string()).replace("%{next}", &next.to_string()).replace("%{n}", &(level + 1).to_string());
    let at = if narrow { Rect::new(head.x, bar.bottom() + 4.0, head.w, 18.0) } else { Rect::new(lv.x - 60.0, lv.y + 38.0, lv.w + 60.0, 18.0) };
    l.ui.text_in(&to_next, at, 12.0, Weight::Regular, TEXT_DIM, Align::Right);
    l.ui.p().rect(Rect::new(head.x, head.bottom() + 8.0, head.w, 1.0), HAIRLINE);
    // (the list begins `FIGURE_ROOM` higher than its first figures: room for one risen under
    // the mouse)
    let rest = Rect::new(area.x - 6.0, head.bottom() + 22.0 - FIGURE_ROOM, area.w + 12.0, (area.bottom() - head.bottom() - 22.0 + FIGURE_ROOM).max(0.0));
    l.ui.scroll_area("record", rest, &mut |ui, v| record_body(ui, Rect::new(v.x + 6.0, v.y, v.w - 20.0, v.h), &f));
}

/// A section of the record: its hairline box and its name; returns the room inside.
fn section(ui: &mut Ui, r: Rect, title: &str) -> Rect {
    ui.card(r);
    ui.text_in(&omsi_ui::tr(title).to_uppercase(), Rect::new(r.x + 16.0, r.y + 14.0, r.w - 32.0, 14.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    Rect::new(r.x + 16.0, r.y + 40.0, r.w - 32.0, (r.h - 54.0).max(0.0))
}

/// Room above the record's first figures for a figure risen under the mouse (`Ui::card_tile`).
const FIGURE_ROOM: f32 = 8.0;

/// Everything under the record's head, from `r`'s top down; returns how high it came out.
fn record_body(ui: &mut Ui, r: Rect, f: &Figures) -> f32 {
    let gap = 12.0;
    let mut y = r.y + FIGURE_ROOM;
    // the big figures, as many in a row as fit at 150 points
    // (two to a row on a phone: one each was a screen of six boxes)
    let min_w = if r.w < 400.0 { 110.0 } else { 150.0 };
    let cols = (((r.w + gap) / (min_w + gap)).floor() as usize).clamp(1, f.tiles.len().max(1));
    let tw = (r.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let th = if tw < 150.0 { 80.0 } else { 92.0 };
    let value_px = if tw < 150.0 { 22.0 } else { 28.0 };
    for (k, t) in f.tiles.iter().enumerate() {
        let base = Rect::new(r.x + (k % cols) as f32 * (tw + gap), y + (k / cols) as f32 * (th + gap), tw, th);
        if !ui.rect_visible(base) {
            continue;
        }
        // (a figure is no button, but it moves under the mouse as the tiles do: it rises, the
        // light follows the mouse and its edge lights up - `Ui::card_tile`; under its shadow
        // it takes the sheet's colour, having none of its own at rest)
        ui.solid(base);
        let m = ui.card_tile(id_of(&format!("record-figure-{k}")), base, RADIUS);
        let c = m.r;
        ui.tile_shadow(&m);
        if m.hover > 0.005 {
            ui.p().rounded(c, RADIUS, PANEL.mix(HOVER, 0.5 * m.hover));
        }
        ui.tile_light(&m, 0.04);
        ui.tile_edge(&m, 1.0, HAIRLINE);
        let pad = if tw < 150.0 { 12.0 } else { 16.0 };
        let vw = ui.text_in(&t.value, Rect::new(c.x + pad, c.y + 12.0, base.w - 2.0 * pad, 34.0), value_px, Weight::Bold, TEXT, Align::Left);
        if !t.unit.is_empty() {
            ui.text_in(t.unit, Rect::new(c.x + pad + 4.0 + vw, c.y + 20.0, 40.0, 24.0), 14.0, Weight::Bold, TEXT_DIM, Align::Left);
        }
        let ly = c.bottom() - 42.0;
        ui.text_in(&omsi_ui::tr(t.label).to_uppercase(), Rect::new(c.x + pad, ly, base.w - 2.0 * pad, 14.0), 10.5, Weight::Medium, TEXT_DIM, Align::Left);
        if !t.note.is_empty() {
            ui.text_in(&t.note, Rect::new(c.x + pad, ly + 18.0, base.w - 2.0 * pad, 16.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
    }
    y += f.tiles.len().div_ceil(cols) as f32 * (th + gap) + 6.0;
    // punctuality has the most room: it is what the trade is about
    let all = f.stops.iter().sum::<i32>();
    if all > 0 {
        let s = Rect::new(r.x, y, r.w, 132.0);
        let inner = section(ui, s, "Punctuality");
        let bar = Rect::new(inner.x, inner.y, inner.w, 12.0);
        ui.p().rounded(bar, 6.0, HAIRLINE);
        let colours = [EARLY, ON_TIME, LATE];
        let mut x = bar.x;
        for (k, n) in f.stops.iter().enumerate() {
            let w = bar.w * *n as f32 / all as f32;
            if w > 0.5 {
                ui.p().rect(Rect::new(x, bar.y, w, bar.h), colours[k]);
            }
            x += w;
        }
        let mut lx = inner.x;
        for (k, word) in ["Early", "On time", "Late"].iter().enumerate() {
            ui.p().circle(Vec2::new(lx + 4.5, bar.bottom() + 22.0), 4.5, colours[k]);
            let w = ui.text_in(word, Rect::new(lx + 14.0, bar.bottom() + 13.0, 200.0, 18.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            let n = f.stops[k].to_string();
            let nw = ui.text_in(&n, Rect::new(lx + 20.0 + w, bar.bottom() + 13.0, 80.0, 18.0), 13.0, Weight::Bold, TEXT, Align::Left);
            lx += 20.0 + w + nw + 22.0;
        }
        let pct = f.stops[1] as f32 / all as f32 * 100.0;
        let note = omsi_ui::tr("%{pct} % of %{n} stops on time").replace("%{pct}", &format!("{pct:.0}")).replace("%{n}", &all.to_string());
        ui.text_in(&note, Rect::new(inner.x, bar.bottom() + 40.0, inner.w, 18.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
        y = s.bottom() + gap;
    }
    // the trips the game judged as they ended: what they come to, and the last of them
    if let Some(t) = &f.trips {
        y += trips_section(ui, Rect::new(r.x, y, r.w, 0.0), t) + gap;
    }
    // how they drive and their records, side by side where there is room
    let two = r.w >= 640.0;
    let cw = if two { (r.w - gap) * 0.5 } else { r.w };
    let fact_h = |facts: &[Fact]| 54.0 + facts.iter().map(|x| if x.note.is_empty() { 30.0 } else { 46.0 }).sum::<f32>();
    let (h1, h2) = (fact_h(&f.style), if f.records.is_empty() { 0.0 } else { fact_h(&f.records) });
    let h = if two { h1.max(h2) } else { h1 };
    facts(ui, Rect::new(r.x, y, cw, h), "How you drive", &f.style);
    if !f.records.is_empty() {
        let at = if two { Rect::new(r.x + cw + gap, y, cw, h) } else { Rect::new(r.x, y + h1 + gap, cw, h2) };
        facts(ui, at, "Records", &f.records);
    }
    y += if two || f.records.is_empty() { h } else { h1 + gap + h2 } + gap;
    // where and with what, the most driven first, each bar as long as its share of the first
    if !f.maps.is_empty() {
        let n = f.maps.len().max(f.buses.len()) as f32;
        let h = 54.0 + n * 28.0;
        top(ui, Rect::new(r.x, y, cw, h), "Where you drive", &f.maps);
        let at = if two { Rect::new(r.x + cw + gap, y, cw, h) } else { Rect::new(r.x, y + h + gap, cw, h) };
        top(ui, at, "What you drive", &f.buses);
        y += if two { h } else { 2.0 * h + gap } + gap;
    }
    // when the runs end: the hour at the computer, not the game's
    if f.hours.iter().any(|n| *n > 0) {
        let s = Rect::new(r.x, y, r.w, 168.0);
        let inner = section(ui, s, "When you finish a duty");
        let most = *f.hours.iter().max().unwrap_or(&1) as f32;
        let cw = inner.w / 24.0;
        let base = inner.y + 90.0;
        for (hour, n) in f.hours.iter().enumerate() {
            let bh = (if most > 0.0 { *n as f32 / most * 90.0 } else { 0.0 }).max(2.0);
            let b = Rect::new(inner.x + hour as f32 * cw + 1.5, base - bh, cw - 3.0, bh);
            ui.p().rounded(b, 2.0, if *n > 0 { accent() } else { HAIRLINE });
            if hour % 6 == 0 {
                ui.text_in(&format!("{hour:02}"), Rect::new(b.x, base + 5.0, 30.0, 14.0), 10.0, Weight::Regular, TEXT_DIM, Align::Left);
            }
        }
        y = s.bottom() + gap;
    }
    // the last runs
    if f.runs.is_empty() {
        y += ui.paragraph("Nothing driven yet. Complete a duty and it appears here: hours, kilometres, punctuality, and how you drove.", Vec2::new(r.x, y + 4.0), r.w, 14.0, Weight::Regular, TEXT_DIM) + 8.0;
    } else {
        let rh = 34.0;
        let s = Rect::new(r.x, y, r.w, 54.0 + f.runs.len() as f32 * rh);
        let inner = section(ui, s, "Last duties");
        // (a narrow window: the date and the line make room)
        let wide = inner.w >= 620.0;
        for (k, run) in f.runs.iter().enumerate() {
            let row = Rect::new(inner.x, inner.y + k as f32 * rh, inner.w, rh);
            if k > 0 {
                ui.p().rect(Rect::new(row.x, row.y, row.w, 1.0), HAIRLINE);
            }
            let mut x = row.x;
            if wide {
                ui.text_in(&run.when, Rect::new(x, row.y, row.w * 0.24 - 12.0, rh), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
                x += row.w * 0.24;
            }
            let mw = if wide { row.w * 0.28 } else { row.w - 220.0 };
            ui.text_in(&run.map, Rect::new(x, row.y, mw - 12.0, rh), 13.0, Weight::Medium, TEXT, Align::Left);
            x += mw;
            if wide {
                ui.text_in(&run.line, Rect::new(x, row.y, row.w * 0.16 - 12.0, rh), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            }
            let right = row.right();
            let (ow, kw, dw) = (64.0, 80.0, 76.0);
            if let Some((good, all)) = run.on_time {
                // (green all on time, blue some early, red some late - colour is punctuality here)
                let c = if good == all { ON_TIME.lighten(0.25) } else if good * 10 >= all * 8 { TEXT } else { LATE.lighten(0.2) };
                ui.text_in(&format!("{good}/{all}"), Rect::new(right - ow, row.y, ow, rh), 13.0, Weight::Bold, c, Align::Right);
            }
            ui.text_in(&run.km, Rect::new(right - ow - kw, row.y, kw - 8.0, rh), 13.0, Weight::Regular, TEXT_SOFT, Align::Right);
            ui.text_in(&run.length, Rect::new(right - ow - kw - dw, row.y, dw - 8.0, rh), 13.0, Weight::Regular, TEXT_SOFT, Align::Right);
        }
        y = s.bottom() + gap;
    }
    y - r.y
}

/// The record's trips in a section from `r`'s top: what they come to in a row of figures (two
/// to a row on a phone), then the last ones - the line, where to, and on the right the average
/// and the worst delay and the stops on time; returns how high it came out.
fn trips_section(ui: &mut Ui, r: Rect, t: &Trips) -> f32 {
    let (rh, head) = (34.0, 22.0);
    let n = t.figures.len().max(1);
    let cols = if r.w - 32.0 < 110.0 * n as f32 { 2.min(n) } else { n };
    let fig_h = n.div_ceil(cols) as f32 * 50.0;
    let h = 54.0 + fig_h + 12.0 + if t.last.is_empty() { 0.0 } else { head + t.last.len() as f32 * rh };
    let s = Rect::new(r.x, r.y, r.w, h);
    let inner = section(ui, s, "Trip reports");
    let fw = inner.w / cols as f32;
    for (k, (value, colour, label)) in t.figures.iter().enumerate() {
        let (x, y) = (inner.x + (k % cols) as f32 * fw, inner.y + (k / cols) as f32 * 50.0);
        ui.text_in(value, Rect::new(x, y - 2.0, fw - 12.0, 26.0), 22.0, Weight::Bold, *colour, Align::Left);
        ui.text_in(&omsi_ui::tr(label).to_uppercase(), Rect::new(x, y + 26.0, fw - 12.0, 14.0), 10.5, Weight::Medium, TEXT_DIM, Align::Left);
    }
    if t.last.is_empty() {
        return h;
    }
    // the last trips, under the names of their columns
    let wide = inner.w >= 620.0;
    let (ow, dw) = (64.0, 72.0);
    let top = inner.y + fig_h + 12.0;
    let columns: &[(&str, f32)] = if wide { &[("On time", 0.0), ("Worst", ow), ("Average", ow + dw)] } else { &[("On time", 0.0), ("Worst", ow)] };
    for (word, at) in columns {
        let cell = if *at == 0.0 { Rect::new(inner.right() - ow, top, ow, head - 6.0) } else { Rect::new(inner.right() - at - dw, top, dw - 8.0, head - 6.0) };
        ui.text_in(&omsi_ui::tr(word).to_uppercase(), cell, 10.0, Weight::Medium, TEXT_FAINT, Align::Right);
    }
    for (k, line) in t.last.iter().enumerate() {
        let row = Rect::new(inner.x, top + head + k as f32 * rh, inner.w, rh);
        ui.p().rect(Rect::new(row.x, row.y, row.w, 1.0), HAIRLINE);
        let mut x = row.x;
        if wide {
            ui.text_in(&line.when, Rect::new(x, row.y, row.w * 0.22 - 12.0, rh), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
            x += row.w * 0.22;
        }
        // (the line's plate, as on the duty cards)
        if let Some(l) = &line.line {
            let pw = (ui.width(l, 12.0, Weight::Black) + 12.0).clamp(26.0, 70.0);
            let plate = Rect::new(x, row.center().y - 9.5, pw, 19.0);
            ui.p().rounded(plate, 5.0, LINE);
            ui.text_in(l, plate, 12.0, Weight::Black, ON_LINE, Align::Center);
            x += pw + 10.0;
        }
        let mut right = row.right();
        // (a free drive keeps no time: it says so where the figures would be)
        if line.free {
            ui.text_in("Free drive", Rect::new(right - ow - dw, row.y, ow + dw, rh), 12.5, Weight::Regular, TEXT_FAINT, Align::Right);
        }
        if let Some((good, all)) = line.on_time {
            ui.text_in(&format!("{good}/{all}"), Rect::new(right - ow, row.y, ow, rh), 13.0, Weight::Bold, on_time_colour(good, all), Align::Right);
        }
        right -= ow;
        if let Some((w, c)) = &line.worst {
            ui.text_in(w, Rect::new(right - dw, row.y, dw - 8.0, rh), 13.0, Weight::Medium, *c, Align::Right);
        }
        right -= dw;
        if wide {
            if let Some(a) = &line.average {
                ui.text_in(a, Rect::new(right - dw, row.y, dw - 8.0, rh), 13.0, Weight::Regular, TEXT_SOFT, Align::Right);
            }
            right -= dw;
        }
        ui.text_in(&line.terminus, Rect::new(x, row.y, (right - x - 12.0).max(0.0), rh), 13.0, Weight::Medium, TEXT, Align::Left);
    }
    h
}

/// A list of facts in a section: the name on the left, the value on the right, a bar between
/// for a rating, and where a record was set under it.
fn facts(ui: &mut Ui, r: Rect, title: &str, list: &[Fact]) {
    let inner = section(ui, r, title);
    let mut y = inner.y;
    for f in list {
        let row = Rect::new(inner.x, y, inner.w, 26.0);
        // (the value as wide as it is - "41 of 42 stops on time" - the name in what is left)
        let vw = (ui.width(&f.value, 16.0, Weight::Bold) + 4.0).min(row.w * 0.6);
        let lw = ui.text_in(f.label, Rect::new(row.x, row.y, (row.w - vw - 12.0).max(0.0), row.h), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
        ui.text_in(&f.value, Rect::new(row.right() - vw, row.y, vw, row.h), 16.0, Weight::Bold, TEXT, Align::Right);
        if let Some(t) = f.bar {
            let x0 = row.x + lw.max(row.w * 0.3) + 16.0;
            let bar = Rect::new(x0, row.center().y - 3.0, (row.right() - 70.0 - x0).max(0.0), 6.0);
            ui.p().rounded(bar, 3.0, HAIRLINE);
            ui.p().rounded(Rect::new(bar.x, bar.y, (bar.w * t).max(6.0), bar.h), 3.0, accent());
        }
        y += 30.0;
        if !f.note.is_empty() {
            ui.text_in(&f.note, Rect::new(row.x, y - 6.0, row.w, 16.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            y += 16.0;
        }
    }
}

/// The three most driven of something, a bar each as long as its share of the first.
fn top(ui: &mut Ui, r: Rect, title: &str, rows: &[(String, usize)]) {
    let inner = section(ui, r, title);
    let most = rows.first().map(|x| x.1).unwrap_or(0).max(1) as f32;
    for (k, (name, n)) in rows.iter().enumerate() {
        let row = Rect::new(inner.x, inner.y + k as f32 * 28.0, inner.w, 22.0);
        let bw = 90.0f32.min(row.w * 0.3);
        ui.text_in(name, Rect::new(row.x, row.y, row.w - bw - 52.0, row.h), 13.0, Weight::Regular, TEXT, Align::Left);
        let bar = Rect::new(row.right() - bw - 38.0, row.center().y - 3.0, bw, 6.0);
        ui.p().rounded(bar, 3.0, HAIRLINE);
        ui.p().rounded(Rect::new(bar.x, bar.y, (bar.w * *n as f32 / most).max(6.0), bar.h), 3.0, accent());
        ui.text_in(&n.to_string(), Rect::new(row.right() - 30.0, row.y, 30.0, row.h), 13.0, Weight::Bold, TEXT, Align::Right);
    }
}

// --- settings ---------------------------------------------------------------------------------

/// A value of the settings as the pages show it.
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

/// The window sizes the settings offer (`resolution`).
pub(crate) const RESOLUTIONS: &[(&str, &str)] = &[("auto", "Automatic"), ("1280x720", "1280 x 720"), ("1280x800", "1280 x 800 (Steam Deck)"), ("1366x768", "1366 x 768"), ("1600x900", "1600 x 900"), ("1920x1080", "1920 x 1080"), ("1920x1200", "1920 x 1200"), ("2560x1440", "2560 x 1440"), ("3840x2160", "3840 x 2160")];

fn sel_setting(ui: &mut Ui, s: &mut Value, dirty: &mut f32, name: &str, r: Rect, label: &str, key: &str, options: &[(&str, &str)]) {
    ui.label(Rect::new(r.x, r.y, r.w * 0.45, r.h), label);
    let cur = match get(s, key) {
        Value::String(x) => x.clone(),
        Value::Bool(b) => (*b as u8).to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let mut labels: Vec<String> = options.iter().map(|o| o.1.to_string()).collect();
    let mut values: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
    let mut sel = values.iter().position(|v| *v == cur || v.parse::<f64>().ok().zip(cur.parse::<f64>().ok()).map(|(a, b)| (a - b).abs() < 1e-6).unwrap_or(false));
    if sel.is_none() && !cur.is_empty() {
        // a value written by hand gets an entry of its own
        labels.push(cur.clone());
        values.push(cur.clone());
        sel = Some(values.len() - 1);
    }
    let mut sel = sel.unwrap_or(0);
    if ui.select(name, Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), &mut sel, &labels) {
        let v = &values[sel];
        s[key] = match get(s, key) {
            Value::Bool(_) => json!(v == "1"),
            Value::Number(_) => v.parse::<f64>().map(|f| if f.fract() == 0.0 { json!(f as i64) } else { json!(f) }).unwrap_or(json!(v)),
            _ => json!(v),
        };
        *dirty = 0.3;
    }
}

fn toggle_setting(ui: &mut Ui, s: &mut Value, dirty: &mut f32, r: Rect, label: &str, key: &str) {
    let mut v = get(s, key).as_bool().unwrap_or(false);
    if ui.toggle(&format!("set-{key}"), r, &mut v, label) {
        s[key] = json!(v);
        *dirty = 0.3;
    }
}

/// The settings page's tabs: what one has come to change.
pub const SETTINGS_TABS: [&str; 6] = ["Graphics", "Driving", "Camera", "Sound", "Gameplay", "General"];

/// What the tabs show of the launcher and ask of it (they see only the settings): the
/// updater's state, "Check now", "Reset all settings", the Controls page at one of its tabs.
struct Outside {
    update: crate::updater::Status,
    check_updates: bool,
    reset: bool,
    controls: Option<usize>,
    /// "Show the welcome screen again" or "Start the tour" was pressed.
    again: Option<Again>,
}

/// What the General tab shows again of a first start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Again {
    Welcome,
    Tour,
}

thread_local! {
    /// How high each tab's two columns came out (the stacked layout's heights, and how far
    /// the page scrolls).
    static SETTINGS_COL_H: std::cell::Cell<[[f32; 2]; SETTINGS_TABS.len()]> = const { std::cell::Cell::new([[0.0; 2]; SETTINGS_TABS.len()]) };
}

pub fn settings(l: &mut Launcher, area: Rect) {
    let body = area;
    // (one tab at a time: the whole page in three columns was taller than two screens, and a
    // setting was found by reading all of it; the tabs as Omsi-Hub's chips)
    let ch = l.ui.chips_height(body.w, 34.0, &SETTINGS_TABS);
    let mut tab = l.pages.settings_tab;
    if l.ui.chips("settings-tab", Rect::new(body.x, body.y, body.w, 34.0), &mut tab, &SETTINGS_TABS) {
        l.pages.settings_tab = tab;
    }
    let top = body.y + ch + 16.0;
    let body = Rect::new(body.x, top, body.w, (body.bottom() - top).max(0.0));
    let s = &mut l.state.settings;
    let dirty = &mut l.state.settings_dirty;
    let mut out = Outside { update: l.update.status(), check_updates: false, reset: false, controls: None, again: None };
    // (two columns side by side; where they would be too narrow to read - a phone - one
    // under the other, each as high as it was the frame before)
    let stacked = body.w < 900.0;
    // (each tab scrolled where it was left, not where another one was)
    l.ui.scroll_area(&format!("settings-page-{tab}"), body, &mut |ui, v| {
        let hs = SETTINGS_COL_H.with(|c| c.get())[tab];
        let w = v.w - 8.0;
        let (cols, h) = if stacked {
            ([Rect::new(v.x, v.y, w, hs[0]), Rect::new(v.x, v.y + hs[0] + GAP * 2.0, w, hs[1])], hs[0] + hs[1] + GAP * 2.0)
        } else {
            let cw = (w - GAP * 2.0) / 2.0;
            let h = v.h.max(hs[0]).max(hs[1]);
            ([Rect::new(v.x, v.y, cw, h), Rect::new(v.x + cw + GAP * 2.0, v.y, cw, h)], h)
        };
        let used = settings_tab(ui, tab, s, dirty, &mut out, cols);
        SETTINGS_COL_H.with(|c| {
            let mut all = c.get();
            all[tab] = used;
            c.set(all);
        });
        h
    });
    if out.check_updates {
        l.update.check();
    }
    if out.reset {
        l.pages.confirm_reset = true;
    }
    if let Some(t) = out.controls {
        l.pages.controls_tab = t;
        l.go(Page::Controls);
    }
    match out.again {
        Some(Again::Welcome) => super::welcome::show_again(l),
        // (the tour begins at the start)
        Some(Again::Tour) => {
            l.drive.step = super::flow::Step::Mode;
            l.go(Page::Drive);
            super::tour::start(l);
        }
        None => {}
    }
}

/// The dialog that asks before every setting goes back to how it came.
pub fn reset_dialog(l: &mut Launcher) {
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.solid(full);
    l.ui.p().rect(full, omsi_ui::Color::rgba(0, 0, 0, 0.62));
    let w = (size.x - 48.0).min(520.0);
    let h = 190.0;
    let r = Rect::new((size.x - w) * 0.5, (size.y - h) * 0.5, w, h);
    l.ui.panel(r);
    let inner = Rect::new(r.x + 24.0, r.y + 20.0, r.w - 48.0, r.h - 40.0);
    l.ui.icon("restart_alt", Vec2::new(inner.x + 14.0, inner.y + 14.0), 26.0, DANGER);
    l.ui.text_in("Reset every setting?", Rect::new(inner.x + 38.0, inner.y, inner.w - 38.0, 28.0), 18.0, Weight::Bold, TEXT, Align::Left);
    l.ui.paragraph("Graphics, sound, controllers and game settings go back to how they came. The language, the drivers, the key bindings and the game folder stay.", Vec2::new(inner.x, inner.y + 40.0), inner.w, 13.0, Weight::Regular, TEXT_DIM);
    let by = inner.bottom() - 38.0;
    if l.ui.button("reset-no", Rect::new(inner.right() - 250.0, by, 110.0, 38.0), "Cancel", None, ButtonKind::Normal) {
        l.pages.confirm_reset = false;
    }
    if l.ui.button("reset-yes", Rect::new(inner.right() - 130.0, by, 130.0, 38.0), "Reset", Some("restart_alt"), ButtonKind::Danger) {
        // (the language, the welcome having been seen and the launcher chosen stay: a reset
        // that opened the welcome again, or another launcher, was not asked for)
        let kept: Vec<(&str, serde_json::Value)> = ["language", "welcome_done", "launcher_ui"].iter().filter_map(|k| l.state.settings.get(*k).cloned().map(|v| (*k, v))).collect();
        l.state.settings = core::settings_from_text(None);
        for (k, v) in kept {
            l.state.settings[k] = v;
        }
        l.state.settings_dirty = 0.3;
        l.pages.confirm_reset = false;
        l.state.set_status("Every setting is back to how it came.", false);
    }
}

/// A column of a settings tab: its panel, and the rows and sections laid down it.
struct Col {
    top: f32,
    inner: Rect,
    y: f32,
}

impl Col {
    /// The column in `r`, under the title of its first section.
    fn new(ui: &mut Ui, r: Rect, title: &str) -> Col {
        ui.card(r);
        let inner = ui.heading(Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, r.h - 28.0), title, None);
        Col { top: r.y, inner, y: inner.y }
    }

    /// The next row.
    fn row(&mut self) -> Rect {
        let r = Rect::new(self.inner.x, self.y, self.inner.w, ROW - 2.0);
        self.y += ROW + 4.0;
        r
    }

    /// Another section, under its title.
    fn section(&mut self, ui: &mut Ui, title: &str) {
        self.y += 6.0;
        ui.heading(Rect::new(self.inner.x, self.y, self.inner.w, 28.0), title, None);
        self.y += 32.0;
    }

    /// How high the column came out.
    fn used(&self) -> f32 {
        self.y - self.top + 14.0
    }
}

/// Tab `tab` of the settings in its two columns. Returns the height each column needed (0
/// for one the tab leaves empty).
fn settings_tab(ui: &mut Ui, tab: usize, s: &mut Value, dirty: &mut f32, out: &mut Outside, cols: [Rect; 2]) -> [f32; 2] {
    match tab {
        0 => graphics_tab(ui, s, dirty, cols),
        1 => driving_tab(ui, s, dirty, out, cols),
        2 => camera_tab(ui, s, dirty, out, cols),
        3 => sound_tab(ui, s, dirty, cols),
        4 => gameplay_tab(ui, s, dirty, cols),
        _ => general_tab(ui, s, dirty, out, cols),
    }
}

/// The saved graphics profiles' part of the Graphics tab: the list, the name being typed.
#[derive(Default)]
struct GfxProfileUi {
    name: String,
    sel: usize,
    list: Option<Vec<String>>,
    msg: String,
}

thread_local! {
    static GFX_PROFILES: std::cell::RefCell<GfxProfileUi> = std::cell::RefCell::new(GfxProfileUi::default());
}

/// Save, load and delete the graphics settings as named profiles.
fn graphics_profiles_block(ui: &mut Ui, s: &mut Value, dirty: &mut f32, c: &mut Col) {
    GFX_PROFILES.with(|g| {
        let mut g = g.borrow_mut();
        let g = &mut *g;
        let names: Vec<String> = g.list.get_or_insert_with(|| core::graphics_profiles().into_keys().collect()).clone();
        g.sel = g.sel.min(names.len().saturating_sub(1));
        let labels: Vec<String> = if names.is_empty() { vec![omsi_ui::tr("No saved profiles").into_owned()] } else { names.clone() };
        let r = c.row();
        ui.label(Rect::new(r.x, r.y, r.w * 0.45, r.h), "Saved profile");
        if ui.select("s-gp-sel", Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), &mut g.sel, &labels) && !names.is_empty() {
            g.name = names[g.sel].clone();
        }
        let r = c.row();
        let half = (r.w - GAP) * 0.5;
        if ui.button("s-gp-load", Rect::new(r.x, r.y, half, r.h), "Load", Some("download"), ButtonKind::Normal) && !names.is_empty() {
            let name = names[g.sel].clone();
            match core::graphics_profiles().get(&name) {
                Some(p) => {
                    core::apply_graphics_profile(p, s);
                    *dirty = 0.3;
                    g.msg = omsi_ui::tr("Loaded \"%{name}\".").replace("%{name}", &name);
                }
                None => g.msg = omsi_ui::tr("\"%{name}\" is gone.").replace("%{name}", &name),
            }
        }
        if ui.button("s-gp-del", Rect::new(r.x + half + GAP, r.y, half, r.h), "Delete", Some("delete"), ButtonKind::Danger) && !names.is_empty() {
            let name = names[g.sel].clone();
            g.msg = match core::delete_graphics_profile(&name) {
                Ok(()) => omsi_ui::tr("Deleted \"%{name}\".").replace("%{name}", &name),
                Err(e) => format!("{e:#}"),
            };
            g.list = None;
        }
        let r = c.row();
        ui.text_input("s-gp-name", r, &mut g.name, "Profile name", None);
        let r = c.row();
        if ui.button("s-gp-save", r, "Save current graphics as profile", Some("save"), ButtonKind::Primary) {
            g.msg = match core::save_graphics_profile(&g.name, s) {
                Ok(name) => {
                    g.name = name.clone();
                    g.list = None;
                    if let Some(i) = core::graphics_profiles().keys().position(|k| *k == name) {
                        g.sel = i;
                    }
                    omsi_ui::tr("Saved \"%{name}\".").replace("%{name}", &name)
                }
                Err(e) => format!("{e:#}"),
            };
        }
        if !g.msg.is_empty() {
            c.y += ui.paragraph(&g.msg, Vec2::new(c.inner.x, c.y), c.inner.w, 12.5, Weight::Regular, TEXT_DIM) + 8.0;
        }
    });
}

/// How the game looks and how fast it runs.
fn graphics_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Graphics");
    // Quality presets, first: they set most of what follows. (OMSI's own
    // option_presets/*.oop are named after the PCs of their day - "PC 2006", "X10 high",
    // "Chicago Recommended" - which read as random words here.)
    let presets: [(&str, serde_json::Value); 4] = [
        ("Low", json!({"msaa": 1, "anisotropy": 2, "shadow_size": 1024, "ssao": false, "shadows": false, "detail_textures": false, "clouds": false, "view_distance": "600", "min_obj_size": 0.03, "max_obj_dist": "500", "mirror_size": 128, "mirror_refresh": "eco", "render_scale": "0.75", "texture_memory": 800})),
        ("Medium", json!({"msaa": 2, "anisotropy": 4, "shadow_size": 2048, "ssao": false, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "900", "min_obj_size": 0.02, "max_obj_dist": "750", "mirror_size": 256, "mirror_refresh": "eco", "render_scale": "auto", "texture_memory": 1200})),
        ("High", json!({"msaa": 4, "anisotropy": 8, "shadow_size": 2048, "ssao": true, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "auto", "min_obj_size": 0.013, "max_obj_dist": "auto", "mirror_size": 256, "mirror_refresh": "full", "render_scale": "auto", "texture_memory": 0})),
        ("Ultra", json!({"msaa": 4, "anisotropy": 8, "shadow_size": 4096, "ssao": true, "shadows": true, "detail_textures": true, "clouds": true, "view_distance": "2000", "min_obj_size": 0.005, "max_obj_dist": "1500", "mirror_size": 512, "mirror_refresh": "full", "render_scale": "auto", "texture_memory": 0})),
    ];
    {
        // the preset the settings match now, else "Custom"
        let matches = |p: &serde_json::Value| p.as_object().map(|o| o.iter().all(|(k, v)| {
            let cur = get(s, k);
            cur == v || cur.as_f64().zip(v.as_f64()).map(|(a, b)| (a - b).abs() < 1e-6).unwrap_or(false) || cur.as_str().zip(v.as_f64()).map(|(a, b)| a.parse::<f64>().map(|a| (a - b).abs() < 1e-6).unwrap_or(false)).unwrap_or(false) || cur.as_f64().zip(v.as_str()).map(|(a, b)| b.parse::<f64>().map(|b| (a - b).abs() < 1e-6).unwrap_or(false)).unwrap_or(false)
        })).unwrap_or(false);
        let mut labels: Vec<String> = presets.iter().map(|p| p.0.to_string()).collect();
        labels.push("Custom".to_string());
        let mut sel = presets.iter().position(|p| matches(&p.1)).unwrap_or(presets.len());
        let r = c.row();
        ui.label(Rect::new(r.x, r.y, r.w * 0.45, r.h), "Quality preset");
        if ui.select("s-preset", Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.55, r.h), &mut sel, &labels) && sel < presets.len() {
            if let Some(obj) = presets[sel].1.as_object() {
                for (k, v) in obj {
                    s[k.as_str()] = v.clone();
                }
                *dirty = 0.3;
            }
        }
    }
    sel_setting(ui, s, dirty, "s-graphics", c.row(), "Graphics", "graphics", &[("vanilla", "Vanilla (as OMSI 2)"), ("vanilla_plus", "Vanilla+"), ("enhanced", "Enhanced"), ("enhanced_plus", "Enhanced+")]);
    // Vanilla draws what OMSI 2 draws: no sun shadows, ambient occlusion or detail grain
    let classic = get(s, "graphics").as_str() == Some("vanilla");
    let traced = get(s, "graphics").as_str() == Some("enhanced_plus");
    sel_setting(ui, s, dirty, "s-msaa", c.row(), "Anti-aliasing", "msaa", &[("1", "Off"), ("2", "2x MSAA"), ("4", "4x MSAA"), ("8", "8x MSAA")]);
    sel_setting(ui, s, dirty, "s-scale", c.row(), "Render scale", "render_scale", &[("auto", "Auto"), ("1", "100%"), ("0.85", "85%"), ("0.75", "75%"), ("0.67", "67%"), ("0.5", "50%")]);
    sel_setting(ui, s, dirty, "s-af", c.row(), "Anisotropic", "anisotropy", &[("1", "Off"), ("2", "2x"), ("4", "4x"), ("8", "8x"), ("16", "16x")]);
    if !classic {
        sel_setting(ui, s, dirty, "s-shadow", c.row(), "Shadow map", "shadow_size", &[("1024", "1024"), ("2048", "2048"), ("4096", "4096")]);
        // (Enhanced+ traces its shadows, occlusion and reflections: always on there)
        if !traced {
            toggle_setting(ui, s, dirty, c.row(), "Ambient occlusion", "ssao");
            toggle_setting(ui, s, dirty, c.row(), "Sun shadows", "shadows");
        }
        sel_setting(ui, s, dirty, "s-casters", c.row(), "Shadows cast by", "shadow_casters", &[("all", "Every solid mesh"), ("omsi", "[shadow] meshes, as OMSI")]);
        toggle_setting(ui, s, dirty, c.row(), "Detail texturing up close", "detail_textures");
        // (an LED panel's dots are its own light: how bright they burn, and how much of the
        // mip chain the panel's picture and its mask are held at - 0 point-samples them,
        // the sharpest dots and the worst shimmer; higher holds them at the level the
        // screen footprint asks for at most)
        let mut led = get(s, "led_glow").as_i64().unwrap_or(6) as f32;
        if ui.slider("s-led", c.row(), &mut led, 0.0, 15.0, 1.0, "LED glow", &|v| if v < 0.5 { "Off".to_string() } else { format!("{}", v as i64) }) {
            s["led_glow"] = json!(led.round() as i64);
            *dirty = 0.3;
        }
        let mut mip = get(s, "led_mips").as_f64().unwrap_or(1.3) as f32;
        if ui.slider("s-led-mip", c.row(), &mut mip, 0.0, 4.0, 0.05, "LED mip strength", &|v| if v < 0.005 { "Off".to_string() } else { format!("{v:.2}") }) {
            s["led_mips"] = json!((mip / 0.05).round() * 0.05);
            *dirty = 0.3;
        }
    }
    // (the models' `[isshadow]` blob is what OMSI draws under a vehicle in every graphics
    // mode, the vanilla one included, so its switch is not part of the extras above)
    toggle_setting(ui, s, dirty, c.row(), "OMSI's shadow meshes (under vehicles)", "shadow_blobs");
    if !traced {
        toggle_setting(ui, s, dirty, c.row(), "Reflection maps (paint, chrome, glass)", "reflections");
    }
    toggle_setting(ui, s, dirty, c.row(), "Clouds", "clouds");
    toggle_setting(ui, s, dirty, c.row(), "Windy trees", "windy_trees");
    let left = c.used();
    let mut c = Col::new(ui, cols[1], "Display");
    toggle_setting(ui, s, dirty, c.row(), "Fullscreen", "fullscreen");
    // (the window's own size in pixels; a Steam Deck's Gaming Mode and other odd screens,
    // #904 - "Automatic" fits the screen, and fills it under gamescope)
    sel_setting(ui, s, dirty, "s-res", c.row(), "Window size", "resolution", RESOLUTIONS);
    toggle_setting(ui, s, dirty, c.row(), "V-sync", "vsync");
    sel_setting(ui, s, dirty, "s-fps", c.row(), "Frame limit", "max_fps", &[("0", "Screen refresh rate"), ("30", "30 fps"), ("45", "45 fps"), ("60", "60 fps"), ("120", "120 fps"), ("144", "144 fps"), ("1000", "Unlimited")]);
    // (a Mac has Metal only; elsewhere a driver's Vulkan that misbehaves, or a card without
    // it, is got round here)
    if cfg!(windows) {
        sel_setting(ui, s, dirty, "s-api", c.row(), "Graphics API", "graphics_api", &[("auto", "Automatic"), ("vulkan", "Vulkan"), ("dx12", "DirectX 12"), ("gl", "OpenGL")]);
    } else if !cfg!(target_os = "macos") {
        sel_setting(ui, s, dirty, "s-api", c.row(), "Graphics API", "graphics_api", &[("auto", "Automatic"), ("vulkan", "Vulkan"), ("gl", "OpenGL")]);
    }
    c.section(ui, "World & memory");
    sel_setting(ui, s, dirty, "s-view", c.row(), "View distance", "view_distance", &[("auto", "Default (1200 m)"), ("600", "600 m - fastest"), ("900", "900 m"), ("1200", "1200 m"), ("1500", "1500 m"), ("2000", "2000 m"), ("2500", "2500 m")]);
    sel_setting(ui, s, dirty, "s-maxobj", c.row(), "Object distance", "max_obj_dist", &[("auto", "Automatic"), ("500", "500 m"), ("750", "750 m"), ("900", "900 m"), ("1500", "1500 m"), ("3000", "3000 m")]);
    sel_setting(ui, s, dirty, "s-minobj", c.row(), "Small objects", "min_obj_size", &[("0.005", "All"), ("0.013", "Normal"), ("0.02", "Fewer (faster)"), ("0.03", "Few (fastest)")]);
    sel_setting(ui, s, dirty, "s-mirror", c.row(), "Mirrors", "mirror_size", &[("0", "Off"), ("128", "Low (128)"), ("256", "Normal (256)"), ("512", "High (512)"), ("1024", "Very high (1024)")]);
    sel_setting(ui, s, dirty, "s-mirror-refresh", c.row(), "Real-time reflections", "mirror_refresh", &[("off", "None (frozen picture)"), ("eco", "Economical"), ("full", "Full")]);
    // (the game takes the smaller of an eighth of the memory and what the graphics
    // adapter is taken to hold, see `memory::texture_budget`)
    let adapter_mb = omsi_render::ADAPTER_TEXTURE_MB.load(std::sync::atomic::Ordering::Relaxed) as i64;
    let auto_mb = match (get(s, "texture_memory_auto").as_i64().unwrap_or(0), adapter_mb) {
        (m, 0) => m,
        (0, a) => a,
        (m, a) => m.min(a),
    };
    let auto_label = if auto_mb > 0 { omsi_ui::tr("Automatic (%{size} here)").replace("%{size}", &mb(auto_mb)) } else { "Automatic".to_string() };
    let opts: Vec<(&str, &str)> = vec![("0", auto_label.as_str()), ("500", "500 MB"), ("1000", "1 GB"), ("1500", "1.5 GB"), ("2000", "2 GB"), ("3000", "3 GB"), ("4000", "4 GB"), ("6000", "6 GB")];
    sel_setting(ui, s, dirty, "s-texmem", c.row(), "Texture memory", "texture_memory", &opts);
    toggle_setting(ui, s, dirty, c.row(), "Compress textures on loading", "texture_compression");
    c.section(ui, "Profiles");
    graphics_profiles_block(ui, s, dirty, &mut c);
    [left, c.used()]
}

/// How the bus answers the keys, the mouse, a wheel and pedals (which key does what: the
/// Controls page).
fn driving_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, out: &mut Outside, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Keyboard & mouse");
    sel_setting(ui, s, dirty, "s-keys", c.row(), "Driving keys", "drive_keys", &[("omsi", "Custom controls (Controls page)"), ("simple", "W A S D + arrows"), ("wasd", "W A S D only"), ("arrows", "Arrow keys only")]);
    toggle_setting(ui, s, dirty, c.row(), "Steering linearity (keys at OMSI's steady pace)", "steering_linear");
    toggle_setting(ui, s, dirty, c.row(), "Old Steering (the wheel stays, turn it back yourself)", "old_steering");
    toggle_setting(ui, s, dirty, c.row(), "Dynamic steering (slower keys at speed, OMSI's redSteerSpd)", "red_steer_spd");
    let mut ms = get(s, "mouse_sens").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-mouse", c.row(), &mut ms, 0.1, 3.0, 0.05, "Mouse steering sensitivity (O)", &|v| if (v - 1.0).abs() < 0.01 { "OMSI".to_string() } else { format!("{:.0}%", v * 100.0) }) {
        s["mouse_sens"] = json!((ms * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Smooth mouse steering (off: the wheel follows the cursor at once, as in OMSI)", "mouse_smooth");
    toggle_setting(ui, s, dirty, c.row(), "A right click ends the mouse steering (as in OMSI)", "mouse_right_off");
    toggle_setting(ui, s, dirty, c.row(), "Indicators cancel themselves (as the bus's script does)", "blinker_cancel");
    toggle_setting(ui, s, dirty, c.row(), "The keyboard brake stays on until the throttle (as in OMSI)", "brake_hold");
    toggle_setting(ui, s, dirty, c.row(), "Automatic clutch (manual gearboxes)", "auto_clutch");
    toggle_setting(ui, s, dirty, c.row(), "Hold manual gear buttons (release returns to neutral)", "momentary_gears");
    // (on a duty: the line, route and destination typed into the IBIS for the driver)
    toggle_setting(ui, s, dirty, c.row(), "Fill in the IBIS automatically", "ibis_auto");
    if ui.button("s-go-keys", c.row(), "Change the keys", Some("keyboard"), ButtonKind::Normal) {
        out.controls = Some(0);
    }
    let left = c.used();
    let mut c = Col::new(ui, cols[1], "Game controllers");
    toggle_setting(ui, s, dirty, c.row(), "H-pattern shifter: return to neutral when the gear is released", "momentary_gears");
    // steering wheels: the wheel's own rotation and how much of it is the bus's full lock
    // (a real bus: about two and a half turns), force feedback the other way round
    let mut range = get(s, "wheel_range").as_f64().unwrap_or(900.0) as f32;
    if ui.slider("s-wrange", c.row(), &mut range, 180.0, 1800.0, 30.0, "Wheel rotation", &|v| format!("{v:.0}°")) {
        s["wheel_range"] = json!(range.round());
        *dirty = 0.3;
    }
    let mut lock = get(s, "wheel_lock").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-wlock", c.row(), &mut lock, 0.0, 1800.0, 30.0, "Full lock at", &|v| if v < 45.0 { "OMSI".to_string() } else { format!("{v:.0}°") }) {
        s["wheel_lock"] = json!(if lock < 45.0 { 0.0 } else { lock.round() });
        *dirty = 0.3;
    }
    // a gamepad stick's steering, eased onto where the stick points (0: as it reads)
    let mut pad_smooth = get(s, "pad_steer_smooth").as_f64().unwrap_or(120.0) as f32;
    if ui.slider("s-pad-steer-smooth", c.row(), &mut pad_smooth, 0.0, 300.0, 10.0, "Stick steering smoothing", &|v| if v <= 0.0 { "Off".to_string() } else { format!("{v:.0} ms") }) {
        s["pad_steer_smooth"] = json!(pad_smooth.round());
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Stick steers like a wheel (a wheel seen as a gamepad)", "pad_steer_linear");
    toggle_setting(ui, s, dirty, c.row(), "Arrow keys switch the cameras with a wheel too (no glance)", "arrows_switch_cams");
    // the pedals' response: softer (below 1) or stronger (above 1) than the pedal reads
    for (key, label, id) in [("pedal_throttle", "Throttle pedal strength", "s-pedt"), ("pedal_brake", "Brake pedal strength", "s-pedb")] {
        let mut v = get(s, key).as_f64().unwrap_or(1.0) as f32;
        if ui.slider(id, c.row(), &mut v, 0.5, 2.0, 0.05, label, &|v| if (v - 1.0).abs() < 0.01 { "Normal".to_string() } else if v < 1.0 { omsi_ui::tr("Softer x%{k}").replace("%{k}", &format!("{v:.2}")) } else { omsi_ui::tr("Stronger x%{k}").replace("%{k}", &format!("{v:.2}")) }) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    toggle_setting(ui, s, dirty, c.row(), "Force feedback and vibration", "ff_enabled");
    toggle_setting(ui, s, dirty, c.row(), "Invert force feedback by default", "ff_invert");
    c.y += ui.paragraph("Wheels with a saved direction use their own setting under Controls → Game controllers.", Vec2::new(c.inner.x, c.y), c.inner.w, 12.5, Weight::Regular, TEXT_DIM) + 8.0;
    toggle_setting(ui, s, dirty, c.row(), "Invert force feedback", "ff_invert");
    // what the wheel feels all the time while the bus runs: the road's grain and the
    // engine's buzz, and how long a jolt eases away once it is over
    for (key, label, id) in [("ff_road_vib", "Road texture vibration", "s-ffroad"), ("ff_engine_vib", "Engine vibration", "s-ffeng")] {
        let mut v = get(s, key).as_f64().unwrap_or(1.0) as f32;
        if ui.slider(id, c.row(), &mut v, 0.0, 4.0, 0.05, label, &|v| if v < 0.01 { "Off".to_string() } else if (v - 1.0).abs() < 0.01 { "Normal".to_string() } else { format!("{:.0}%", v * 100.0) }) {
            s[key] = json!((v * 20.0).round() / 20.0);
            *dirty = 0.3;
        }
    }
    let mut fade = get(s, "ff_fade").as_f64().unwrap_or(0.28) as f32;
    if ui.slider("s-fffade", c.row(), &mut fade, 0.0, 1.5, 0.05, "Vibration fade-out", &|v| if v < 0.01 { "Off".to_string() } else { format!("{:.0} ms", (v * 1000.0).round() as i32) }) {
        s["ff_fade"] = json!((fade * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    if ui.button("s-wreset", c.row(), "Reset wheel settings", Some("restart_alt"), ButtonKind::Normal) {
        s["wheel_range"] = json!(900.0);
        s["wheel_lock"] = json!(0.0);
        s["ff_invert"] = json!(false);
        s["ff_enabled"] = json!(true);
        s["ff_road_vib"] = json!(1.0);
        s["ff_engine_vib"] = json!(1.0);
        s["ff_fade"] = json!(0.28);
        *dirty = 0.3;
    }
    if ui.button("s-go-pads", c.row(), "Set up a wheel or pedals", Some("sports_esports"), ButtonKind::Normal) {
        out.controls = Some(1);
    }
    [left, c.used()]
}

/// What the driver sees: the seat, the views from it and around the bus, head tracking, VR.
fn camera_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, out: &mut Outside, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Driver's view");
    // the driver's eye, moved from the bus's own camera
    c.section(ui, "Seat position");
    for (key, label, id) in [("seat_y", "Seat forward / back", "s-seaty"), ("seat_z", "Seat up / down", "s-seatz"), ("seat_x", "Seat right / left", "s-seatx")] {
        let mut v = get(s, key).as_f64().unwrap_or(0.0) as f32;
        if ui.slider(id, c.row(), &mut v, -0.6, 0.6, 0.01, label, &|v| format!("{:+.0} cm", v * 100.0)) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    let mut seat_pitch = get(s, "seat_pitch_deg").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-seat-pitch", c.row(), &mut seat_pitch, -45.0, 45.0, 1.0, "Head pitch", &|v| format!("{v:+.0}°")) {
        s["seat_pitch_deg"] = json!(seat_pitch.round());
        *dirty = 0.3;
    }
    if ui.button("s-seatreset", c.row(), "Reset the seat position", Some("restart_alt"), ButtonKind::Normal) {
        for k in ["seat_x", "seat_y", "seat_z"] {
            s[k] = json!(0.0);
        }
        s["seat_pitch_deg"] = json!(0.0);
        *dirty = 0.3;
    }
    let fov_key = if get(s, "triple_screen").as_bool().unwrap_or(false)
        && !get(s, "vr").as_bool().unwrap_or(false)
    {
        "triple_fov_deg"
    } else {
        "fov"
    };
    let mut fov = get(s, fov_key).as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-fov", c.row(), &mut fov, 0.0, 120.0, 1.0, "Field of view", &|v| if v < 20.0 { "Default".to_string() } else { format!("{v:.0}°") }) {
        s[fov_key] = json!(if fov < 20.0 { 0.0 } else { fov.round() });
        *dirty = 0.3;
    }
    let mut look = get(s, "look_sens").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-look-sens", c.row(), &mut look, 0.1, 2.0, 0.05, "Mouse look sensitivity", &|v| if (v - 1.0).abs() < 0.01 { "OMSI".to_string() } else { format!("{:.0}%", v * 100.0) }) {
        s["look_sens"] = json!((look * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Right stick turns the view", "right_stick_look");
    let mut smooth = get(s, "look_smoothing_ms").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-look-smoothing", c.row(), &mut smooth, 0.0, 200.0, 10.0, "Smooth the mouse look", &|v| if v <= 0.0 { "Off".to_string() } else { format!("{v:.0} ms") }) {
        s["look_smoothing_ms"] = json!(smooth.round());
        *dirty = 0.3;
    }
    c.section(ui, "A head at rest");
    let mut idle = get(s, "head_idle").as_f64().unwrap_or(0.0) as f32;
    if ui.slider("s-head-idle", c.row(), &mut idle, 0.0, 1.0, 0.05, "Head sway at a standstill", &|v| if v <= 0.0 { "Off".to_string() } else { format!("{:.0}%", v * 100.0) }) {
        s["head_idle"] = json!((idle * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    let mut pace = get(s, "head_idle_pace").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-head-idle-pace", c.row(), &mut pace, 0.5, 2.0, 0.05, "Sway pace", &|v| format!("{:.0}%", v * 100.0)) {
        s["head_idle_pace"] = json!((pace * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Driver's view turns with the steering", "steer_look");
    let mut angle = get(s, "steer_look_angle").as_f64().unwrap_or(30.0) as f32;
    if ui.slider("s-steer-look-angle", c.row(), &mut angle, 0.0, 60.0, 1.0, "Steering view angle", &|v| format!("{v:.0}°")) {
        s["steer_look_angle"] = json!(angle);
        *dirty = 0.3;
    }
    let mut response = get(s, "steer_look_response").as_f64().unwrap_or(0.25) as f32;
    if ui.slider("s-steer-look-response", c.row(), &mut response, 0.05, 1.0, 0.05, "Steering view response", &|v| format!("{:.0} ms", v * 1000.0)) {
        s["steer_look_response"] = json!(response);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Head moves with the bus", "head_movement");
    toggle_setting(ui, s, dirty, c.row(), "Camera glides between viewpoints", "driverview_smooth");
    toggle_setting(ui, s, dirty, c.row(), "Driver's hands in the cab view", "hands_in_cab");
    toggle_setting(ui, s, dirty, c.row(), "Right mouse button turns the view, Shift+right zooms (off: right zooms as in OMSI, the wheel button turns)", "alt_view");
    toggle_setting(ui, s, dirty, c.row(), "Precision mouse zoom (FOV curve instead of the linear way)", "precision_zoom");
    let left = c.used();
    let mut c = Col::new(ui, cols[1], "Outside views");
    toggle_setting(ui, s, dirty, c.row(), "Camera collisions (outside view)", "camera_collision");
    toggle_setting(ui, s, dirty, c.row(), "Driver at the wheel (outside views)", "driver");
    c.section(ui, "Head tracking");
    toggle_setting(ui, s, dirty, c.row(), "Head tracking (TrackIR and others through opentrack, UDP 4242)", "head_tracking");
    c.section(ui, "Triple screen");
    toggle_setting(
        ui,
        s,
        dirty,
        c.row(),
        "Three screen projections",
        "triple_screen",
    );
    toggle_setting(
        ui,
        s,
        dirty,
        c.row(),
        "Span three monitors at startup",
        "triple_span",
    );
    toggle_setting(
        ui,
        s,
        dirty,
        c.row(),
        "HUD on centre screen",
        "triple_hud_center",
    );
    if get(s, "triple_screen").as_bool().unwrap_or(false) {
        ui.label(
            c.row(),
            "Three equal screens in a horizontal row. OpenXR takes priority.",
        );
        let mut value = get(s, "triple_width_mm").as_f64().unwrap_or(600.0) as f32;
        if ui.slider(
            "s-triple-width_mm",
            c.row(),
            &mut value,
            200.0,
            2000.0,
            10.0,
            "Visible width of one panel",
            &|v| format!("{v:.0} mm"),
        ) {
            s["triple_width_mm"] = json!(value);
            *dirty = 0.3;
        }
        let mut value = get(s, "triple_distance_mm").as_f64().unwrap_or(650.0) as f32;
        if ui.slider(
            "s-triple-distance_mm",
            c.row(),
            &mut value,
            200.0,
            3000.0,
            10.0,
            "Eye to centre screen",
            &|v| format!("{v:.0} mm"),
        ) {
            s["triple_distance_mm"] = json!(value);
            s["triple_fov_deg"] = json!(0.0);
            *dirty = 0.3;
        }
        let mut value = get(s, "triple_bezel_mm").as_f64().unwrap_or(0.0) as f32;
        if ui.slider(
            "s-triple-bezel_mm",
            c.row(),
            &mut value,
            0.0,
            100.0,
            1.0,
            "Both frames at each join",
            &|v| format!("{v:.0} mm"),
        ) {
            s["triple_bezel_mm"] = json!(value);
            *dirty = 0.3;
        }
        let mut value = get(s, "triple_left_angle_deg").as_f64().unwrap_or(45.0) as f32;
        if ui.slider(
            "s-triple-left_angle_deg",
            c.row(),
            &mut value,
            0.0,
            90.0,
            1.0,
            "Left screen inward angle",
            &|v| format!("{v:.0}°"),
        ) {
            s["triple_left_angle_deg"] = json!(value);
            *dirty = 0.3;
        }
        let mut value = get(s, "triple_right_angle_deg").as_f64().unwrap_or(45.0) as f32;
        if ui.slider(
            "s-triple-right_angle_deg",
            c.row(),
            &mut value,
            0.0,
            90.0,
            1.0,
            "Right screen inward angle",
            &|v| format!("{v:.0}°"),
        ) {
            s["triple_right_angle_deg"] = json!(value);
            *dirty = 0.3;
        }
        let mut value = get(s, "triple_eye_height_mm").as_f64().unwrap_or(0.0) as f32;
        if ui.slider(
            "s-triple-eye_height_mm",
            c.row(),
            &mut value,
            -500.0,
            500.0,
            1.0,
            "Eye above screen centre",
            &|v| format!("{v:.0} mm"),
        ) {
            s["triple_eye_height_mm"] = json!(value);
            *dirty = 0.3;
        }
    }
    if cfg!(windows) {
        c.section(ui, "Virtual reality");
        toggle_setting(ui, s, dirty, c.row(), "Use OpenXR headset", "vr");
        if get(s, "vr").as_bool().unwrap_or(false) {
            sel_setting(ui, s, dirty, "s-vr-scale", c.row(), "Eye resolution", "vr_scale", &[("0.5", "50%"), ("0.65", "65%"), ("0.8", "80%"), ("1", "100%")]);
            sel_setting(ui, s, dirty, "s-vr-head-smoothing", c.row(), "Head tracking smoothing", "vr_head_smoothing_ms", &[("0", "Off"), ("5", "5 ms"), ("10", "10 ms"), ("20", "20 ms"), ("30", "30 ms")]);
            sel_setting(ui, s, dirty, "s-vr-mirror-rate", c.row(), "Bus mirror refresh", "vr_mirror_rate", &[("0", "Off"), ("8", "8/s"), ("16", "16/s"), ("24", "24/s"), ("32", "32/s"), ("48", "48/s"), ("60", "60/s"), ("90", "90/s"), ("120", "120/s"), ("180", "180/s"), ("240", "240/s"), ("360", "360/s"), ("-1", "Every frame")]);
            c.y += ui.paragraph("The rate is shared by all bus mirrors. Higher rates can reduce game FPS.", Vec2::new(c.inner.x, c.y), c.inner.w, 12.0, Weight::Regular, TEXT_DIM) + 8.0;
            toggle_setting(ui, s, dirty, c.row(), "Show headset picture on monitor", "vr_desktop_mirror");
            // (the VR keys head the Controls page's game list)
            if ui.button("s-go-vr-keys", c.row(), "Change the VR keys", Some("keyboard"), ButtonKind::Normal) {
                out.controls = Some(0);
            }
        }
    }
    [left, c.used()]
}

/// How loud the bus, the traffic and the surroundings are, and what passengers say.
fn sound_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Volume");
    let mut vol = get(s, "volume").as_f64().unwrap_or(0.6) as f32;
    if ui.slider("s-vol", c.row(), &mut vol, 0.0, 1.0, 0.05, "Volume", &|v| format!("{:.0}%", v * 100.0)) {
        s["volume"] = json!((vol * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    for (key, label, id) in [("vol_ai", "Traffic", "s-volai"), ("vol_scenery", "Surroundings", "s-volsc")] {
        let mut v = get(s, key).as_f64().unwrap_or(1.0) as f32;
        if ui.slider(id, c.row(), &mut v, 0.0, 1.0, 0.05, label, &|v| format!("{:.0}%", v * 100.0)) {
            s[key] = json!((v * 100.0).round() / 100.0);
            *dirty = 0.3;
        }
    }
    toggle_setting(ui, s, dirty, c.row(), "Doppler effect", "doppler");
    sel_setting(ui, s, dirty, "s-voices", c.row(), "Passenger voices", "pax_voices", &[("all", "Greetings and tickets"), ("tickets", "Only the ticket asked for"), ("off", "Silent")]);
    [c.used(), radio_stations(ui, cols[1])]
}

thread_local! {
    /// The radio stations as the Sound settings edit them (`radio.cfg`, read the first time).
    static RADIO: std::cell::RefCell<Option<Vec<(String, String)>>> = const { std::cell::RefCell::new(None) };
}

/// The bus radios' internet stations (`radio.cfg`, see `radio`): each with its name and
/// address, removed or added here, saved at once (#857). Returns the column's height.
fn radio_stations(ui: &mut Ui, r: Rect) -> f32 {
    let mut c = Col::new(ui, r, "Radio stations");
    c.y += ui.paragraph("A radio's station button n plays the n-th station, a cassette player the first; Shift+R steps through them. An address is an MP3, AAC or Ogg stream or an .m3u/.pls playlist.", Vec2::new(c.inner.x, c.y), c.inner.w, 12.5, Weight::Regular, TEXT_DIM) + 8.0;
    RADIO.with(|cell| {
        let mut cell = cell.borrow_mut();
        let list = cell.get_or_insert_with(crate::radio::own_stations);
        let mut changed = false;
        let mut remove = None;
        for (k, (name, address)) in list.iter_mut().enumerate() {
            let row = c.row();
            let nw = (row.w * 0.3).round();
            changed |= ui.text_input(&format!("radio-name-{k}"), Rect::new(row.x, row.y, nw, row.h), name, "Name", None);
            changed |= ui.text_input(&format!("radio-url-{k}"), Rect::new(row.x + nw + 8.0, row.y, row.w - nw - 8.0 - 36.0, row.h), address, "https://…", None);
            if ui.icon_button(&format!("radio-del-{k}"), Vec2::new(row.right() - 16.0, row.center().y), 14.0, "delete", "Remove this station") {
                remove = Some(k);
            }
        }
        if let Some(k) = remove {
            list.remove(k);
            changed = true;
        }
        if ui.button("radio-add", c.row(), "Add a station", Some("add"), ButtonKind::Normal) {
            list.push((String::new(), String::new()));
        }
        if changed {
            if let Err(e) = crate::radio::save_stations(list) {
                log::warn!("radio.cfg: {e}");
            }
        }
    });
    c.used()
}

/// How the world behaves: passengers, traffic, collisions, wear, the clock.
fn gameplay_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Passengers");
    sel_setting(ui, s, dirty, "s-board", c.row(), "Boarding", "boarding", &[("auto", "Pay and take the ticket"), ("pay", "The driver sells the ticket"), ("walk", "Just walk in")]);
    toggle_setting(ui, s, dirty, c.row(), "Passengers pay the exact fare", "exact_fare");
    let mut pd = get(s, "pax_density").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-pax", c.row(), &mut pd, 0.0, 2.0, 0.1, "How many passengers", &|v| format!("{:.0}%", v * 100.0)) {
        s["pax_density"] = json!((pd * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Ability to get up (Ctrl+Shift+G)", "get_up");
    c.section(ui, "Traffic");
    sel_setting(ui, s, dirty, "s-unsched", c.row(), "Random traffic", "ai_unsched_factor", &[("25", "25%"), ("50", "50%"), ("75", "75%"), ("100", "100%"), ("150", "150%"), ("200", "200%")]);
    sel_setting(ui, s, dirty, "s-maxsched", c.row(), "Timetable vehicles", "ai_max_scheduled", &[("0", "All"), ("10", "At most 10"), ("25", "At most 25"), ("50", "At most 50")]);
    sel_setting(ui, s, dirty, "s-maxpark", c.row(), "Parked cars", "ai_max_parked", &[("-1", "None"), ("0", "Every space"), ("35", "At most 35"), ("100", "At most 100"), ("250", "At most 250")]);
    let left = c.used();
    // OMSI's own options (options.cfg)
    let mut c = Col::new(ui, cols[1], "Simulation");
    sel_setting(ui, s, dirty, "s-maint", c.row(), "Maintenance", "maintenance", &[("0", "Infinite (no wear)"), ("1", "Very bad"), ("2", "Bad"), ("3", "Normal"), ("4", "Good")]);
    toggle_setting(ui, s, dirty, c.row(), "Collisions with vehicles", "collision_vehicles");
    toggle_setting(ui, s, dirty, c.row(), "Collisions with objects (walls, poles)", "collision_objects");
    toggle_setting(ui, s, dirty, c.row(), "Collisions with people", "collision_pedestrians");
    toggle_setting(ui, s, dirty, c.row(), "Start at the real time", "use_real_time");
    toggle_setting(ui, s, dirty, c.row(), "Start on today's date", "use_real_date");
    // the game's clock follows this device's (the host's in multiplayer); the time cannot be set
    toggle_setting(ui, s, dirty, c.row(), "Sync the clock with the real time (locks the time)", "time_sync");
    // the weather follows the METAR report of the airport nearest the map; it cannot be changed then
    toggle_setting(ui, s, dirty, c.row(), "Sync the weather with METAR (locks the weather)", "metar_sync");
    // (in multiplayer the host's or the server's speed counts)
    sel_setting(ui, s, dirty, "s-timespeed", c.row(), "Time speed (not in multiplayer or with the real-time sync)", "time_speed", &[("1", "Real time"), ("2", "x2"), ("4", "x4"), ("8", "x8"), ("15", "x15"), ("30", "x30")]);
    [left, c.used()]
}

/// The language, what the screen shows besides the bus, online, the navigator, updates.
fn general_tab(ui: &mut Ui, s: &mut Value, dirty: &mut f32, out: &mut Outside, cols: [Rect; 2]) -> [f32; 2] {
    let mut c = Col::new(ui, cols[0], "Interface & online");
    {
        let langs: Vec<(&str, &str)> = core::LANGUAGES.iter().map(|l| (l.0, l.1)).collect();
        sel_setting(ui, s, dirty, "s-lang", c.row(), "Language", "language", &langs);
    }
    // (the launcher speaks the chosen language at once)
    crate::ui_language(get(s, "language").as_str().unwrap_or("ENG"));
    // (texts nobody has translated: translated on this machine, see `mt`)
    let was = get(s, "machine_translation").as_bool().unwrap_or(false);
    toggle_setting(ui, s, dirty, c.row(), "Translate the remaining texts automatically (offline, downloads 620 MB once)", "machine_translation");
    let now = get(s, "machine_translation").as_bool().unwrap_or(false);
    if now != was {
        crate::mt::enable(now);
    }
    let st = crate::mt::status();
    if now && !st.is_empty() && st != "Ready" {
        ui.text_in(&st, Rect::new(c.inner.x + 12.0, c.y - 6.0, c.inner.w - 24.0, 16.0), 11.5, omsi_ui::Weight::Regular, TEXT_FAINT, omsi_ui::paint::Align::Left);
        c.y += 14.0;
    }
    toggle_setting(ui, s, dirty, c.row(), "The launcher rests while a game runs (gives the graphics card to the game)", "launcher_rest");
    toggle_setting(ui, s, dirty, c.row(), "Discord Rich Presence", "discord_status");
    let help_height = ui.paragraph(
        "Shows the launcher or your map, bus, line and multiplayer status in Discord.",
        Vec2::new(c.inner.x + 12.0, c.y - 5.0),
        c.inner.w - 24.0,
        11.5,
        omsi_ui::Weight::Regular,
        TEXT_FAINT,
    );
    c.y += help_height + 3.0;
    toggle_setting(ui, s, dirty, c.row(), "Voice chat through GreenTeaSpeak (multiplayer)", "voice_chat");
    let help_height = ui.paragraph(
        "Players near you are heard from where they stand, when GreenTeaSpeak runs with the openOMSI plugin and the server names a voice server.",
        Vec2::new(c.inner.x + 12.0, c.y - 5.0),
        c.inner.w - 24.0,
        11.5,
        omsi_ui::Weight::Regular,
        TEXT_FAINT,
    );
    c.y += help_height + 3.0;
    // the duty's companion on a phone or a tablet (see `crate::companion`): off by default
    let r = c.row();
    toggle_setting(ui, s, dirty, r, "Phone & tablet", "companion");
    let about = "Sign on, the duty menu and the bus's screens on a phone or tablet in your network. The game shows the address and the pairing code when you drive.";
    ui.tooltip(r, about);
    c.y += ui.paragraph(about, Vec2::new(c.inner.x + 12.0, c.y - 5.0), c.inner.w - 24.0, 11.5, omsi_ui::Weight::Regular, TEXT_FAINT) + 3.0;
    if get(s, "companion").as_bool().unwrap_or(false) {
        // (covered on the screen unless asked otherwise: a stream shows the screen to everyone)
        let r = c.row();
        toggle_setting(ui, s, dirty, r, "Hide address and pairing code (streaming)", "companion_hide");
        ui.tooltip(r, "The navigator and the game menu cover the address, the pairing code and the QR code until you press Show.");
        // the Cloudflare tunnel: off unless asked for, and cloudflared fetched only on a press
        let r = c.row();
        toggle_setting(ui, s, dirty, r, "Connect through Cloudflare (also outside your network)", "companion_tunnel");
        ui.tooltip(r, "A free Cloudflare quick tunnel makes the page reachable at an https address from anywhere, e.g. over mobile data. Devices still need the pairing code.");
        if get(s, "companion_tunnel").as_bool().unwrap_or(false) {
            let (line, fetch) = match crate::companion::cloudflared() {
                crate::companion::Cloudflared::Installed(_) => ("cloudflared is installed.", false),
                crate::companion::Cloudflared::Missing => ("The tunnel needs cloudflared, Cloudflare's own program (about 40 MB).", true),
                crate::companion::Cloudflared::Downloading => ("Downloading cloudflared…", false),
                crate::companion::Cloudflared::Failed => ("cloudflared could not be downloaded. Check the internet connection and try again.", true),
            };
            c.y += ui.paragraph(line, Vec2::new(c.inner.x + 12.0, c.y - 5.0), c.inner.w - 24.0, 11.5, omsi_ui::Weight::Regular, TEXT_FAINT) + 3.0;
            if fetch && ui.button("s-cloudflared", c.row(), "Download cloudflared", Some("download"), ButtonKind::Normal) {
                crate::companion::download_cloudflared();
            }
        }
    }
    // (the launcher's own size, on top of the system's: Ctrl+wheel and Ctrl +/- set it too)
    let mut own = core::launcher_scale(get(s, "launcher_scale").as_f64()) as f32;
    // (given out as the slider is let go: dragged, the window scaled under the mouse)
    if ui.slider_on_release("s-launcher-scale", c.row(), &mut own, 0.6, 2.0, 0.05, "Launcher size", &|v| format!("{:.0}%", v * 100.0)) {
        s["launcher_scale"] = json!(core::launcher_scale(Some(own as f64)));
        *dirty = 0.3;
    }
    // (the opening, the bus between pages, the tiles following the mouse: off for less motion)
    toggle_setting(ui, s, dirty, c.row(), "Animations", "animations");
    // (the bus across the screen between pages: off by default, too much on every page)
    toggle_setting(ui, s, dirty, c.row(), "Bus across the screen between pages", "page_bus");
    // (the interface's accent colour: the presets and a picker for any other, at once)
    ui.label(c.row(), "Accent colour");
    c.y += super::accent_pick::chooser(ui, "s-accent", c.inner.x, c.y - 6.0, c.inner.w, s, dirty) + 4.0;
    // (the first start's welcome and the tour through the launcher, once more; side by side,
    // one under the other where the column is narrow)
    let r = c.row();
    let (a, b) = if r.w >= 520.0 {
        let half = (r.w - 8.0) * 0.5;
        (Rect::new(r.x, r.y, half, r.h), Rect::new(r.x + half + 8.0, r.y, half, r.h))
    } else {
        (r, c.row())
    };
    if ui.button("s-welcome-again", a, "Show the welcome screen again", Some("restart_alt"), ButtonKind::Normal) {
        out.again = Some(Again::Welcome);
    }
    if ui.button("s-tour-start", b, "Start the tour", Some("route"), ButtonKind::Normal) {
        out.again = Some(Again::Tour);
    }
    // (the texts over the picture, the menu, the timetable and the navigator: larger for
    // those who find them hard to read, smaller for more of the picture; on a window taller
    // than 1080p they grow with it as well, and the launcher grows with its window anyway)
    let mut size = get(s, "ui_scale").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-uiscale", c.row(), &mut size, 0.5, 2.0, 0.05, "Game interface size", &|v| format!("{:.0}%", v * 100.0)) {
        s["ui_scale"] = json!((size * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Interface grows with the window", "ui_scale_window");
    // (the backgrounds of the whole interface - the navigator, the menu, the timetable, the
    // notes' plates - the texts staying solid; 85 % as designed)
    let mut op = get(s, "ui_opacity").as_f64().unwrap_or(0.85) as f32;
    if ui.slider("s-uiop", c.row(), &mut op, 0.2, 1.0, 0.05, "Interface opacity", &|v| format!("{:.0}%", v * 100.0)) {
        s["ui_opacity"] = json!((op * 100.0).round() / 100.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Name of the button under the mouse", "tooltips");
    toggle_setting(ui, s, dirty, c.row(), "Frame rate in the corner", "show_fps");
    toggle_setting(ui, s, dirty, c.row(), "Notes in the top-left corner", "notes");
    toggle_setting(ui, s, dirty, c.row(), "Chat in online games", "chat");
    // (the chat's own size on top of the interface's; Ctrl + the wheel over it in the game)
    let mut chat = get(s, "chat_size").as_f64().unwrap_or(1.0) as f32;
    if ui.slider("s-chatsize", c.row(), &mut chat, 0.5, 3.0, 0.1, "Chat size", &|v| format!("{:.0}%", v * 100.0)) {
        s["chat_size"] = json!((chat * 10.0).round() / 10.0);
        *dirty = 0.3;
    }
    toggle_setting(ui, s, dirty, c.row(), "Other players' names above their buses", "name_tags");
    c.section(ui, "Navigator");
    toggle_setting(ui, s, dirty, c.row(), "Navigator (Shift+N: map, schedule, off)", "navigator");
    toggle_setting(ui, s, dirty, c.row(), "Route arrows (as in OMSI 2)", "nav_arrows");
    toggle_setting(ui, s, dirty, c.row(), "AI vehicles on the map", "nav_ai");
    // (the trip, its stops and what comes next under the map - its handle on the navigator
    // switches it in the game too)
    toggle_setting(ui, s, dirty, c.row(), "Duty board under the navigator", "nav_board");
    let mut nav = core::nav_scale(get(s, "nav_scale").as_f64()) as f32;
    if ui.slider("s-nav-scale", c.row(), &mut nav, 0.6, 2.0, 0.05, "Navigator size", &|v| format!("{:.0}%", v * 100.0)) {
        s["nav_scale"] = json!(core::nav_scale(Some(nav as f64)));
        *dirty = 0.3;
    }
    sel_setting(ui, s, dirty, "s-stop-style", c.row(), "Bus stop signs", "stop_style", &[("de", "German (H)"), ("uk", "British"), ("fr", "French")]);
    // (signing on as at a depot: off, the duty is there at once)
    toggle_setting(ui, s, dirty, c.row(), "Sign on with personnel number and code", "nav_signon");
    c.y += ui.paragraph("Before a duty the navigator asks for your number and code and for the duty order to be accepted. Off, the map and the duty show at once.", Vec2::new(c.inner.x + 12.0, c.y - 5.0), c.inner.w - 24.0, 11.5, omsi_ui::Weight::Regular, TEXT_FAINT) + 3.0;

    // the corner: a little screen with four corners to click
    let r = Rect::new(c.inner.x, c.y, c.inner.w, 70.0);
    ui.label(Rect::new(r.x, r.y, r.w * 0.45, 24.0), "Corner");
    let screen = Rect::new(r.x + r.w * 0.45, r.y, 110.0, 64.0);
    ui.p().rounded(screen, 6.0, Color::WHITE.alpha(0.05));
    ui.p().rounded_border(screen, 6.0, 1.0, Color::WHITE.alpha(0.12));
    let cur = get(s, "navigator_corner").as_str().unwrap_or("bottom-left").to_string();
    for (name, x, yy) in [("top-left", 0.0, 0.0), ("top-right", 1.0, 0.0), ("bottom-left", 0.0, 1.0), ("bottom-right", 1.0, 1.0)] {
        let cell = Rect::new(screen.x + 5.0 + x * (screen.w * 0.5), screen.y + 5.0 + yy * (screen.h * 0.5), screen.w * 0.5 - 10.0, screen.h * 0.5 - 10.0);
        let (h, _, clicked) = ui.interact(id_of(&format!("corner-{name}")), cell);
        let (dragged, size) = core::nav_rect_parts(get(s, "nav_rect").as_str().unwrap_or(""));
        if clicked {
            // (a corner chosen: the place it was dragged to in the game is forgotten, its size
            // stays)
            s["navigator_corner"] = json!(name);
            s["nav_rect"] = json!(core::nav_rect_text(None, size));
            *dirty = 0.3;
        }
        let on = cur == name && dragged.is_none();
        ui.p().rounded(cell, 3.0, if on { accent() } else { Color::WHITE.alpha(if h { 0.2 } else { 0.08 }) });
    }
    // (dragged and sized in the game: that place and shape on the little screen, until a
    // corner is chosen)
    let (dragged, size) = core::nav_rect_parts(get(s, "nav_rect").as_str().unwrap_or(""));
    if let Some(a) = dragged {
        let inner = screen.inset(5.0);
        // (its size in shares of the window's height, on the little screen - as wide as a
        // 16:9 window)
        let (cw, ch) = size.map(|z| ((z[0] as f32 * inner.h).min(inner.w), (z[1] as f32 * inner.h).min(inner.h))).unwrap_or((screen.w * 0.5 - 10.0, screen.h * 0.5 - 10.0));
        let (cw, ch) = (cw.max(6.0), ch.max(5.0));
        let cell = Rect::new(inner.x + a[0] as f32 * (inner.w - cw), inner.y + a[1] as f32 * (inner.h - ch), cw, ch);
        ui.p().rounded(cell, 3.0, accent());
    }
    c.y += 74.0;
    // (back to its corner and its own size, the board under the map, 100 %)
    if ui.button("s-nav-reset", c.row(), "Reset navigator", Some("restart_alt"), ButtonKind::Normal) {
        s["navigator_corner"] = json!("bottom-left");
        s["nav_rect"] = json!("");
        s["nav_scale"] = json!(1.0);
        s["nav_board"] = json!(true);
        *dirty = 0.3;
    }
    let left = c.used();
    // updates from the GitHub releases (see `crate::updater`)
    let mut c = Col::new(ui, cols[1], "Updates");
    toggle_setting(ui, s, dirty, c.row(), "Look for updates when the launcher starts", "update_check");
    toggle_setting(ui, s, dirty, c.row(), "Install updates without asking", "update_auto");
    toggle_setting(ui, s, dirty, c.row(), "Tell me about a new version during a session", "update_notify");
    toggle_setting(ui, s, dirty, c.row(), "Count me in the website's \"playing now\" (anonymous)", "presence");
    {
        use crate::updater::Status;
        let r = c.row();
        let busy = matches!(out.update, Status::Checking | Status::Downloading { .. } | Status::Installing(_) | Status::WaitingForInstaller(_) | Status::Restarting(_));
        if ui.button("s-upd-check", Rect::new(r.x, r.y, 150.0, r.h), if busy { "Checking…" } else { "Check now" }, Some("refresh"), ButtonKind::Normal) && !busy {
            out.check_updates = true;
        }
        let text = match &out.update {
            Status::UpToDate if crate::updater::is_test_build(crate::updater::current_version()) => format!("{} is a test build: it is not updated", crate::updater::current_version()),
            Status::UpToDate => omsi_ui::tr("%{version} is the latest version").replace("%{version}", &crate::updater::current_version()),
            Status::Available(rel) => omsi_ui::tr("%{version} is available").replace("%{version}", &rel.version),
            Status::Failed(_) => "The last check failed".to_string(),
            _ => omsi_ui::tr("This is openOMSI %{version}").replace("%{version}", &crate::updater::current_version()),
        };
        ui.text_in(&text, Rect::new(r.x + 162.0, r.y, r.w - 162.0, r.h), 12.5, omsi_ui::Weight::Regular, TEXT_DIM, omsi_ui::paint::Align::Left);
    }
    if ui.button("s-upd-github", c.row(), "github.com/Luc-nbr/openOMSI", Some("open_in_new"), ButtonKind::Ghost) {
        crate::updater::open_url(crate::updater::REPO_URL);
    }
    // every setting at once: here at the end, not first on the page where it was the
    // control one saw before any other
    c.section(ui, "Reset");
    if ui.button("s-reset", c.row(), "Reset all settings...", Some("restart_alt"), ButtonKind::Danger) {
        out.reset = true;
    }
    [left, c.used()]
}

fn mb(v: i64) -> String {
    if v >= 1000 {
        format!("{:.1} GB", v as f64 / 1000.0)
    } else {
        format!("{v} MB")
    }
}

// --- controls ---------------------------------------------------------------------------------

/// The game's own actions a controller's button can be given, besides the bus's: the doors
/// and gears of any bus, looking round while held, the bus radio while held, the cameras
/// and the views - both of OMSI's view resets, the one view's (C) and every view's (Space),
/// which a controller could not bring back to the first camera (#1167).
const PAD_GAME_ACTIONS: [&str; 26] = ["doors_all", "door_4", "door_3", "door_2", "door_1", "gear_up", "gear_down", "view_look_left", "view_look_right", "view_look_up", "view_look_down", "view_reset_direction", "view_reset_all_directions", "view_interiorcam_plus", "view_interiorcam_minus", "view_toggle_viewpoint", "view_toggle_interior", "view_set_driver", "view_set_passenger", "view_set_outside", "sim_pause", "screenshot", "quicksave", "toggel_mouse_ctrl", "toggel_ctrler", "voice_radio"];

fn action_text(names: &crate::describe::ControlNames, a: &str) -> String {
    known_action(a).unwrap_or_else(|| names.control(a))
}

fn control_names(l: &Launcher) -> &'static crate::describe::ControlNames {
    crate::describe::names(std::path::Path::new(&l.state.config.root), l.state.settings.get("language").and_then(|x| x.as_str()).unwrap_or("ENG"))
}

fn known_action(a: &str) -> Option<String> {
    if let Some(gear) = a.strip_prefix("kw_s_").and_then(|s| s.strip_suffix("_fest")) {
        return Some(omsi_ui::tr("Gear %{gear} (H-pattern)").replace("%{gear}", gear));
    }
    let known: &[(&str, &str)] = &[
        ("throttle", "Throttle"),
        ("brake", "Brake"),
        ("throttle_amplify", "Throttle (full, kickdown)"),
        ("clutch", "Clutch"),
        ("steering_left", "Steer left"),
        ("steering_right", "Steer right"),
        ("steering_neutral", "Steering to centre"),
        ("parking_brake_toggle", "Parking brake"),
        ("blinker_left_set", "Indicator left"),
        ("blinker_right_set", "Indicator right"),
        ("blinker_left_toggle", "Indicator left (toggle)"),
        ("blinker_right_toggle", "Indicator right (toggle)"),
        ("blinker_off", "Indicators off"),
        ("blinker_warn_toggle", "Hazard lights"),
        ("gear_up", "Gear up (manual gearbox)"),
        ("gear_down", "Gear down (manual gearbox)"),
        ("horn", "Horn"),
        ("kw_scheinwerfer_toggle", "Headlights"),
        ("kw_standlicht_toggle", "Sidelights"),
        ("kw_fernlicht_toggle", "High beam"),
        ("kw_m_enginestart", "Starter"),
        ("kw_wipermode_up", "Wipers (next mode)"),
        ("cp_batterietrennschalter_toggle", "Battery / ignition"),
        ("automatic_D", "Gear D"),
        ("automatic_N", "Gear N"),
        ("automatic_R", "Gear R"),
        ("bus_doorfront0", "Front door (leaf 1)"),
        ("bus_doorfront1", "Front door (leaf 2)"),
        ("bus_dooraft", "Release rear doors"),
        ("door_1", "Door 1 (front), any bus"),
        ("door_2", "Door 2, any bus"),
        ("door_3", "Door 3, any bus"),
        ("door_4", "Door 4, any bus"),
        ("doors_all", "All doors, any bus"),
        ("ticket_give", "Sell the requested ticket"),
        ("view_set_driver", "Driver's view"),
        ("view_set_passenger", "Passenger view"),
        ("view_set_outside", "Outside view"),
        ("view_toggle_viewpoint", "Next view"),
        ("view_toggle_interior", "Cabin and outside, one key"),
        ("vr_recenter", "VR: Reset view"),
        ("vr_toggle_desktop_mirror", "VR: Monitor preview"),
        ("vr_toggle_mode", "VR: Switch VR / desktop"),
        ("vr_toggle_navigator", "VR: Toggle navigator"),
        ("vr_position_navigator", "VR: Position navigator"),
        ("exit", "Quit"),
        ("chat_open", "Multiplayer: write in the chat"),
        ("chat_toggle", "Multiplayer: show / hide the chat"),
        ("voice_radio", "Multiplayer: bus radio (hold)"),
        ("sim_pause", "Pause"),
        ("screenshot", "Screenshot"),
        ("quicksave", "Quicksave"),
        ("toggel_mouse_ctrl", "Toggle mouse steering"),
        ("toggel_ctrler", "Toggle game controllers"),
    ];
    known.iter().find(|k| k.0 == a).map(|k| k.1.to_string())
}

pub fn controls(l: &mut Launcher, area: Rect) {
    let mut tab = l.pages.controls_tab;
    if l.ui.chips("controls-tab", Rect::new(area.x, area.y, area.w, 34.0), &mut tab, &["Keyboard", "Game controllers"]) {
        l.pages.controls_tab = tab;
    }
    let body = Rect::new(area.x, area.y + 50.0, area.w, (area.h - 50.0).max(0.0));
    if l.pages.controls_tab == 1 {
        game_controllers(l, body);
        return;
    }
    l.pages.pads.cancel_feedback_test();
    // a key pressed while one binding waits for it
    if let (Some((sec, idx)), Some(code)) = (l.pages.capturing, l.ui.input.raw_key) {
        use winit::keyboard::KeyCode as K;
        if code == K::Escape {
            l.pages.capturing = None;
        } else if !matches!(code, K::ShiftLeft | K::ShiftRight | K::ControlLeft | K::ControlRight | K::AltLeft | K::AltRight | K::SuperLeft | K::SuperRight) {
            match crate::keys::dik_code(code) {
                Some(scan) => {
                    let m = omsi_content::input::chord(l.ui.input.shift, l.ui.input.ctrl, l.ui.input.alt) as i64;
                    let section = ["vehicles", "game"][sec];
                    let vr_binding = l.state.keybindings.get(section).and_then(|a| a.as_array())
                        .and_then(|a| a.get(idx)).and_then(|b| b.get("action"))
                        .and_then(|a| a.as_str()).is_some_and(|a| a.starts_with("vr_"));
                    if let Some(b) = l.state.keybindings.get_mut(section).and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(idx)) {
                        // (the entry's "held" bit is the action's, not the key's: it stays)
                        let hold = b.get("modifier").and_then(|x| x.as_i64()).unwrap_or(0) & omsi_content::input::KEY_HOLD as i64;
                        b["scan_code"] = json!(scan);
                        b["modifier"] = json!(m | hold);
                    }
                    save_keys(l, vr_binding);
                }
                None => l.state.set_status(omsi_ui::tr("%{key} has no DirectInput scan code the game understands.").replace("%{key}", &format!("{code:?}")), true),
            }
            l.pages.capturing = None;
        }
        l.ui.input.raw_key = None;
    }
    // The keys below are the ones the game uses only with "Custom controls" (Settings →
    // Driving keys); the ready-made layouts keep W A S D / the arrows for driving. Say so,
    // with the switch right here - and changing a key switches by itself (see `save_keys`).
    let preset = l.state.settings.get("drive_keys").and_then(|v| v.as_str()).unwrap_or("simple").to_string();
    let body = if preset != "omsi" {
        let name = omsi_ui::tr(match preset.as_str() {
            "wasd" => "W A S D only",
            "arrows" => "Arrow keys only",
            _ => "W A S D + arrows",
        });
        let bw = if body.w < 700.0 { 150.0 } else { 200.0 };
        let text = omsi_ui::tr("Driving keys: %{name} (Settings). Those keys drive the bus and win over the list below. Change any key here and your own layout (Custom controls) is used from then on.").replace("%{name}", &name);
        // (a phone: the button under the words, not beside them in a column one word wide)
        let narrow = body.w < 520.0;
        let tw = if narrow { body.w - 56.0 } else { body.w - bw - 70.0 };
        let th = l.ui.paragraph_height(&text, tw, 12.5, Weight::Medium);
        let bar = Rect::new(body.x, body.y, body.w, if narrow { th + 22.0 + 46.0 } else { (th + 22.0).max(54.0) });
        l.ui.p().rounded(bar, 8.0, accent().alpha(0.1));
        l.ui.p().rounded_border(bar, 8.0, 1.0, accent().alpha(0.45));
        let ty = if narrow { bar.y + 11.0 } else { bar.center().y - th * 0.5 };
        l.ui.icon("info", Vec2::new(bar.x + 22.0, if narrow { ty + 9.0 } else { bar.center().y }), 20.0, accent());
        l.ui.paragraph(&text, Vec2::new(bar.x + 42.0, ty), tw, 12.5, Weight::Medium, TEXT_SOFT);
        let button = if narrow { Rect::new(bar.x + 42.0, bar.bottom() - 46.0, bar.w - 52.0, 36.0) } else { Rect::new(bar.right() - bw - 10.0, bar.center().y - 18.0, bw, 36.0) };
        if l.ui.button("kb-use-custom", button, "Use these keys", Some("keyboard"), ButtonKind::Primary) {
            use_custom_keys(l);
        }
        let used = bar.h + 12.0;
        Rect::new(body.x, body.y + used, body.w, body.h - used)
    } else {
        body
    };
    let names = control_names(l);
    let sections = [("Driving & the bus", "The bus's own keys", "vehicles"), ("The game", "Menus, views, pausing", "game")];
    // (a phone: one list at a time, the chips choosing which, each row in two lines - side by
    // side the lists were too narrow to show what a key does)
    let narrow = body.w < 640.0;
    let (body, half) = if narrow {
        let mut k = l.pages.kb_section;
        if l.ui.chips("kb-section", Rect::new(body.x, body.y, body.w, 34.0), &mut k, &[sections[0].0, sections[1].0]) {
            l.pages.kb_section = k;
        }
        (Rect::new(body.x, body.y + 48.0, body.w, (body.h - 48.0).max(0.0)), body.w)
    } else {
        (body, (body.w - GAP * 2.0) * 0.5)
    };
    for (sec, (title, sub, key)) in sections.iter().enumerate() {
        if narrow && sec != l.pages.kb_section {
            continue;
        }
        let r = if narrow { body } else { Rect::new(body.x + sec as f32 * (half + GAP * 2.0), body.y, half, body.h) };
        l.ui.card(r);
        let inner = l.ui.heading(Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, r.h - 28.0), title, Some(if sec == 0 { "directions_bus" } else { "sports_esports" }));
        l.ui.text_in(sub, Rect::new(inner.x, inner.y - 6.0, inner.w, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        let mut filter = std::mem::take(&mut l.pages.kb_filter[sec]);
        let event_w = if sec == 0 { 138.0 } else { 0.0 };
        let filter_w = if sec == 0 { (inner.w - event_w - GAP).max(120.0) } else { inner.w };
        l.ui.text_input(&format!("kb-filter-{sec}"), Rect::new(inner.x, inner.y + 18.0, filter_w, 34.0), &mut filter, if l.pages.kb_events[sec] { "Filter events…" } else { "Filter…" }, Some("search"));
        if sec == 0 && l.ui.button(
            "kb-events",
            Rect::new(inner.x + filter_w + GAP, inner.y + 18.0, event_w, 34.0),
            if l.pages.kb_events[sec] { "Back to keys" } else { "Add event…" },
            Some(if l.pages.kb_events[sec] { "arrow_back" } else { "add" }),
            ButtonKind::Normal,
        ) {
            l.pages.kb_events[sec] = !l.pages.kb_events[sec];
            filter.clear();
        }
        l.pages.kb_filter[sec] = filter.clone();
        let q = filter.to_lowercase();

        // OMSI's Add event dialog: all KY_ events from the language files, including
        // event tables supplied by installed mods. Picking one adds an unbound vehicle
        // entry and immediately waits for its key.
        if sec == 0 && l.pages.kb_events[sec] {
            let mut events = names.events();
            if !q.is_empty() {
                events.retain(|(action, label)| action.to_lowercase().contains(&q) || matches(label, &q));
            }
            let mut picked: Option<String> = None;
            l.ui.scroll_area(&format!("kb-events-{sec}"), Rect::new(inner.x - 6.0, inner.y + 62.0, inner.w + 12.0, inner.bottom() - (inner.y + 62.0)), &mut |ui, v| {
                let rh = 40.0;
                for (row, (action, label)) in events.iter().enumerate() {
                    let rr = Rect::new(v.x + 6.0, v.y + row as f32 * rh, v.w - 16.0, rh - 4.0);
                    let shown = format!("{label}  ·  KY_{action}");
                    if ui.button(&format!("kb-event-{row}"), rr, &shown, Some("add"), ButtonKind::Ghost) {
                        picked = Some(action.clone());
                    }
                }
                events.len() as f32 * rh
            });
            if let Some(action) = picked {
                if let Some(a) = l.state.keybindings.get_mut("vehicles").and_then(|a| a.as_array_mut()) {
                    a.push(json!({ "action": action.clone(), "scan_code": 0, "modifier": 0 }));
                    l.pages.capturing = Some((0, a.len() - 1));
                    l.pages.kb_events[0] = false;
                    l.pages.kb_filter[0] = action;
                    l.state.set_status("Event added. Press the key you want to use (Escape cancels).", false);
                }
            }
            continue;
        }

        let list: Vec<(usize, String, i64, i64)> = l.state.keybindings.get(*key).and_then(|a| a.as_array()).map(|a| a.iter().enumerate().map(|(i, b)| (i, b.get("action").and_then(|x| x.as_str()).unwrap_or("").to_string(), b.get("scan_code").and_then(|x| x.as_i64()).unwrap_or(0), b.get("modifier").and_then(|x| x.as_i64()).unwrap_or(0))).collect()).unwrap_or_default();
        let mut shown: Vec<(usize, String, String, bool)> = list
            .iter()
            .filter(|(_, a, s, m)| q.is_empty() || matches(&action_text(names, a), &q) || a.to_lowercase().contains(&q) || crate::keys::key_name(*s, *m).to_lowercase().contains(&q))
            .map(|(i, a, s, m)| {
                let clash = *s != 0 && list.iter().any(|(j, _, s2, m2)| j != i && s2 == s && m2 == m);
                (*i, action_text(names, a), crate::keys::key_name(*s, *m), clash)
            })
            .collect();
        if sec == 1 {
            shown.sort_by_key(|(_, label, _, _)| !label.starts_with("VR:"));
        }
        let capturing = l.pages.capturing;
        // what the row's buttons asked: (entry, cleared) a key cleared or to be pressed,
        // `more` another key for an entry's action (#854)
        let mut clicked: Option<(usize, bool)> = None;
        let mut more: Option<usize> = None;
        let time = l.ui.time;
        // a name the list does not have (a bus's own trigger a mod's readme gives a key, the
        // Urbanway's `ASS_toggle`): added to the list as a key of its own, as an [entry]
        // added to OMSI's keyboard.cfg by hand is (#854)
        let new_action = filter.trim();
        let mut list_top = inner.y + 62.0;
        if shown.is_empty() && new_action.len() > 1 && new_action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            if l.ui.button(&format!("kb-add-{sec}"), Rect::new(inner.x, list_top, inner.w, 36.0), &omsi_ui::tr("Add \"%{action}\" and give it a key").replace("%{action}", new_action), Some("add"), ButtonKind::Normal) {
                if let Some(a) = l.state.keybindings.get_mut(*key).and_then(|a| a.as_array_mut()) {
                    a.push(json!({ "action": new_action, "scan_code": 0, "modifier": 0 }));
                    l.pages.capturing = Some((sec, a.len() - 1));
                }
                // (the filter stays: the new row is the one it shows, waiting for its key)
            }
            list_top += 44.0;
        }
        l.ui.scroll_area(&format!("kb-{sec}"), Rect::new(inner.x - 6.0, list_top, inner.w + 12.0, inner.bottom() - list_top), &mut |ui, v| {
            let rh = if narrow { 74.0 } else { 40.0 };
            for (row, (i, label, keyn, clash)) in shown.iter().enumerate() {
                let rr = Rect::new(v.x + 6.0, v.y + row as f32 * rh, v.w - 16.0, rh - 4.0);
                if rr.bottom() < v.y - 50.0 {
                    continue;
                }
                ui.p().rounded(rr, 8.0, Color::WHITE.alpha(0.03));
                // (the key's row: on a phone the line under the name)
                let keys = if narrow { Rect::new(rr.x, rr.y + 34.0, rr.w, 36.0) } else { Rect::new(rr.x, rr.y, rr.w, rr.h) };
                let label_r = if narrow { Rect::new(rr.x + 12.0, rr.y + 2.0, rr.w - 24.0, 32.0) } else { Rect::new(rr.x + 12.0, rr.y, rr.w - 240.0, rr.h) };
                ui.text_in(label, label_r, 13.0, Weight::Medium, TEXT_SOFT, Align::Left);
                let kw = if narrow { (keys.w - 104.0).max(60.0) } else { 150.0 };
                // another key for the same action (OMSI's file may give one action
                // several [entry]s; several actions on one key need nothing more than the
                // same key pressed for each)
                let pr = Rect::new(keys.right() - 72.0 - kw, keys.y + 5.0, 26.0, keys.h - 10.0);
                let (hp, _, cp) = ui.interact(id_of(&format!("kb-{sec}-{i}-more")), pr);
                ui.icon("add", pr.center(), 16.0, if hp { accent() } else { TEXT_FAINT });
                if cp {
                    more = Some(*i);
                }
                let kr = Rect::new(keys.right() - 40.0 - kw, keys.y + 5.0, kw, keys.h - 10.0);
                let waiting = capturing == Some((sec, *i));
                let id = id_of(&format!("kb-{sec}-{i}"));
                let (h, _, c) = ui.interact(id, kr);
                if c {
                    clicked = Some((*i, false));
                }
                let base = if waiting { accent().alpha(0.25 + 0.15 * (time * 6.0).sin().abs()) } else if *clash { DANGER.alpha(0.22) } else { Color::WHITE.alpha(if h { 0.12 } else { 0.07 }) };
                ui.p().rounded(kr, 6.0, base);
                ui.p().rounded_border(kr, 6.0, 1.0, if waiting { accent() } else if *clash { DANGER } else { Color::WHITE.alpha(0.1) });
                ui.text_in(if waiting { "press a key…" } else { keyn }, kr, 12.0, Weight::Bold, if *clash { DANGER.lighten(0.3) } else { TEXT }, Align::Center);
                let xr = Rect::new(keys.right() - 32.0, keys.y + 5.0, 26.0, keys.h - 10.0);
                let (hx, _, cx) = ui.interact(id ^ 1, xr);
                ui.icon("close", xr.center(), 16.0, if hx { DANGER } else { TEXT_FAINT });
                if cx {
                    clicked = Some((*i, true));
                }
            }
            shown.len() as f32 * rh
        });
        match clicked {
            Some((i, true)) => {
                let vr_binding = l.state.keybindings.get(*key).and_then(|a| a.as_array())
                    .and_then(|a| a.get(i)).and_then(|b| b.get("action"))
                    .and_then(|a| a.as_str()).is_some_and(|a| a.starts_with("vr_"));
                if let Some(b) = l.state.keybindings.get_mut(*key).and_then(|a| a.as_array_mut()).and_then(|a| a.get_mut(i)) {
                    b["scan_code"] = json!(0);
                    b["modifier"] = json!(0);
                }
                save_keys(l, vr_binding);
            }
            Some((i, false)) => l.pages.capturing = Some((sec, i)),
            None => {}
        }
        if let Some(i) = more {
            if let Some(a) = l.state.keybindings.get_mut(*key).and_then(|a| a.as_array_mut()) {
                if let Some(b) = a.get(i).cloned() {
                    // (the held bit is the action's: it goes with it)
                    let hold = b.get("modifier").and_then(|x| x.as_i64()).unwrap_or(0) & omsi_content::input::KEY_HOLD as i64;
                    a.insert(i + 1, json!({ "action": b.get("action").cloned().unwrap_or(json!("")), "scan_code": 0, "modifier": hold }));
                    l.pages.capturing = Some((sec, i + 1));
                }
            }
        }
    }
    if !l.state.keybindings_error.is_empty() {
        let e = l.state.keybindings_error.clone();
        l.ui.text_in(&e, Rect::new(body.x, body.bottom() + 4.0, body.w, 18.0), 12.0, Weight::Medium, DANGER, Align::Left);
    }
}

/// Hide empty slots beyond the physical buttons without changing the saved controller file.
fn shown_button_count(buttons: &[(String, String)], physical: usize, revealed: Option<usize>) -> usize {
    physical
        .max(buttons.iter().rposition(|(action, _)| !action.trim().is_empty()).map(|i| i + 1).unwrap_or(0))
        .max(revealed.map(|i| i + 1).unwrap_or(0))
}

/// The game controllers tab (see `PadsView`).
fn game_controllers(l: &mut Launcher, body: Rect) {
    use crate::controllers::{DeviceCfg, Func};
    let hwnd = l.window.as_deref().and_then(crate::controllers::window_handle);
    let names = control_names(l);
    let pv = &mut l.pages.pads;
    if pv.io.is_none() {
        pv.io = Some(crate::controllers::Devices::new(hwnd, false));
    }
    if pv.devices.is_none() {
        let root = std::path::PathBuf::from(&l.state.config.root);
        pv.devices = Some(crate::controllers::read_cfg(&root));
    }
    // what the devices do now (and a button pressed while one is awaited)
    let mut pressed: Vec<(String, usize)> = Vec::new();
    let mut connected: Vec<crate::controllers::Connected> = Vec::new();
    if let Some(io) = pv.io.as_mut() {
        for (name, n, down) in io.poll() {
            if down {
                pressed.push((name, n));
            }
        }
        connected = io.connected();
    }
    let devices = pv.devices.get_or_insert_with(Vec::new);
    let mut body = body;
    if !l.state.settings.get("momentary_gears").and_then(|v| v.as_bool()).unwrap_or(false) && crate::hpattern::has_held_bindings(devices) {
        let height = l.ui.paragraph("H-pattern gears are assigned. Enable return to neutral under Settings → Driving → Game controllers if your shifter has no neutral button.", Vec2::new(body.x + 12.0, body.y + 8.0), body.w - 24.0, 12.5, Weight::Regular, TEXT_DIM);
        body.y += height + 20.0;
        body.h -= height + 20.0;
    }
    // devices connected but not set up yet can be added
    let list_w = (body.w * 0.32).min(360.0);
    let left = Rect::new(body.x, body.y, list_w, body.h);
    let right = Rect::new(body.x + list_w + GAP * 2.0, body.y, body.w - list_w - GAP * 2.0, body.h);
    l.ui.card(left);
    let inner = l.ui.heading(Rect::new(left.x + 18.0, left.y + 14.0, left.w - 36.0, left.h - 28.0), "Devices", Some("sports_esports"));
    let mut add: Option<String> = None;
    let mut sel = pv.selected;
    let list_r = Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h - 108.0);
    let offs: Vec<String> = l.state.settings.get("ctrl_off").and_then(|v| v.as_str()).unwrap_or("").split('|').map(str::to_string).filter(|s| !s.is_empty()).collect();
    {
        let ui = &mut l.ui;
        let devices = &*devices;
        let connected = &connected;
        ui.scroll_area("pad-list", list_r, &mut |ui, v| {
            let mut y = v.y;
            for (i, d) in devices.iter().enumerate() {
                let on = connected.iter().any(|c| crate::controllers::names_match(&d.name, &c.name));
                let switched_off = offs.iter().any(|o| o.eq_ignore_ascii_case(&d.name));
                let r = Rect::new(v.x + 6.0, y, v.w - 12.0, 44.0);
                if ui.row(&format!("pad-{i}"), r, sel == i) {
                    sel = i;
                }
                ui.text_in(&d.name, Rect::new(r.x + 12.0, r.y, r.w - 40.0, r.h), 13.0, Weight::Medium, if on { TEXT } else { TEXT_DIM }, Align::Left);
                if switched_off {
                    ui.text_in("off", Rect::new(r.right() - 40.0, r.y, 30.0, r.h), 11.5, Weight::Bold, TEXT_FAINT, Align::Right);
                } else {
                    ui.icon(if on { "check_circle" } else { "remove" }, Vec2::new(r.right() - 18.0, r.center().y), 16.0, if on { OK } else { TEXT_FAINT });
                }
                y += 48.0;
            }
            for c in connected.iter().filter(|c| !devices.iter().any(|d| crate::controllers::names_match(&d.name, &c.name))) {
                let r = Rect::new(v.x + 6.0, y, v.w - 12.0, 38.0);
                if ui.button(&format!("pad-add-{}", c.name), r, &omsi_ui::tr("Set up %{device}").replace("%{device}", &c.name), Some("add"), ButtonKind::Primary) {
                    add = Some(c.name.clone());
                }
                y += 44.0;
            }
            if devices.is_empty() && connected.is_empty() {
                y += 4.0 + ui.paragraph("No game controller is connected, and none is set up. Connect a wheel, pedals or a joystick; a gamepad works without setting up (left stick steers, the triggers are the pedals).", Vec2::new(v.x + 6.0, y + 4.0), v.w - 12.0, 13.0, Weight::Regular, TEXT_DIM);
            }
            y - v.y
        });
    }
    if sel != pv.selected {
        release_feedback(&mut pv.io, &mut pv.feedback_test);
        pv.selected = sel;
        pv.capturing = false;
        pv.revealed_button = None;
        pv.wizard = None;
        pv.confirm_remove = None;
    }
    if let Some(name) = add {
        // (a device of buttons only - a gear shifter, a button box - has no axes for the
        // assistant: its buttons are given their keys on its page)
        let axes = !pv.io.as_ref().is_some_and(|io| io.buttons_only(&name));
        devices.push(DeviceCfg { name, second: "0".into(), ..Default::default() });
        pv.selected = devices.len() - 1;
        pv.revealed_button = None;
        pv.dirty = true;
        // a new device starts with the assistant
        if axes {
            pv.wizard = Some(Wizard { step: 0, rest: [None; 8], at: Vec::new(), error: None, calibration: None, ff_choice: None, test_strength: crate::ffb_calibration::PULSE_FORCE });
        }
    }
    // the dead zone (a setting of the game's)
    let dz_r = Rect::new(inner.x, inner.bottom() - 98.0, inner.w, 34.0);
    let mut dz = l.state.settings.get("ctrl_deadzone").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    if l.ui.slider("pad-dz", dz_r, &mut dz, 0.0, 0.3, 0.01, "Dead zone", &|v| format!("{:.0} %", v * 100.0)) {
        l.state.settings["ctrl_deadzone"] = json!(dz);
        l.state.settings_dirty = 0.3;
    }
    let save_r = Rect::new(inner.x, inner.bottom() - 48.0, inner.w, 40.0);
    if l.ui.button("pad-save", save_r, if pv.dirty { "Save" } else { "Saved" }, Some("save"), if pv.dirty { ButtonKind::Primary } else { ButtonKind::Normal }) && pv.dirty {
        match save_gamectrler(devices) {
            Ok(p) => {
                pv.dirty = false;
                l.state.set_status(omsi_ui::tr("Game controllers saved to %{path}").replace("%{path}", &p.display().to_string()), false);
            }
            Err(e) => l.state.set_status(omsi_ui::tr("Not saved: %{error}").replace("%{error}", &e.to_string()), true),
        }
    }
    // the device shown
    l.ui.card(right);
    let Some(d) = devices.get_mut(pv.selected) else { return };
    let inner = l.ui.heading(Rect::new(right.x + 18.0, right.y + 14.0, right.w - 36.0, right.h - 28.0), &d.name.clone(), Some("tune"));
    let live_dev = crate::controllers::find_connected(&connected, &d.name);
    let live: Vec<(usize, f32)> = live_dev.map(|c| c.axes.clone()).unwrap_or_default();
    // every button the device has gets its line (DirectInput says how many)
    if let Some(n) = live_dev.map(|c| c.buttons).filter(|n| *n > d.buttons.len()) {
        d.buttons.resize(n, (String::new(), "0".into()));
    }
    // the assistant, over the device's page
    if let Some(w) = pv.wizard.as_mut() {
        let done = if w.step == WIZARD_STEPS.len() {
            feedback_setup(&mut l.ui, inner, w, d, &live, live_dev, &mut pv.io, &mut pv.feedback_test, hwnd,
                           l.state.settings.get("ff_invert").and_then(|v| v.as_bool()).unwrap_or(false))
        } else {
            wizard(&mut l.ui, inner, w, d, &live, live_dev.is_some(), live_dev.is_some_and(|c| c.ff_capable && !c.gamepad))
        };
        match done {
            Some(true) => {
                release_feedback(&mut pv.io, &mut pv.feedback_test);
                pv.wizard = None;
                pv.dirty = true;
                l.state.set_status("Set up: press Save to keep it (the buttons can be given their keys below).", false);
            }
            Some(false) => {
                release_feedback(&mut pv.io, &mut pv.feedback_test);
                pv.wizard = None;
            }
            None => {}
        }
        return;
    }
    // this device on or off (a second listing of the same wheel, a device not to be used)
    {
        let offs: Vec<String> = l.state.settings.get("ctrl_off").and_then(|v| v.as_str()).unwrap_or("").split('|').map(str::to_string).filter(|s| !s.is_empty()).collect();
        let mut on = !offs.iter().any(|o| o.eq_ignore_ascii_case(&d.name));
        if l.ui.toggle("pad-on", Rect::new(inner.right() - 400.0, inner.y - 36.0, 170.0, 30.0), &mut on, "Use this device") {
            let mut offs: Vec<String> = offs.into_iter().filter(|o| !o.eq_ignore_ascii_case(&d.name)).collect();
            if !on {
                offs.push(d.name.clone());
            }
            l.state.settings["ctrl_off"] = json!(offs.join("|"));
            l.state.settings_dirty = 0.3;
        }
    }
    let buttons_only = pv.io.as_ref().is_some_and(|io| io.buttons_only(&d.name));
    if !buttons_only && l.ui.button("pad-wizard", Rect::new(inner.right() - 220.0, inner.y - 36.0, 220.0, 30.0), "Set up step by step", Some("touch_app"), ButtonKind::Normal) {
        pv.wizard = Some(Wizard { step: 0, rest: [None; 8], at: Vec::new(), error: None, calibration: None, ff_choice: None, test_strength: crate::ffb_calibration::PULSE_FORCE });
    }
    const AXES: [&str; 8] = ["X axis", "Y axis", "Z axis", "X rotation", "Y rotation", "Z rotation", "Slider 1", "Slider 2"];
    let funcs: Vec<String> = Func::LABELS.iter().map(|s| s.to_string()).collect();
    let mut actions: Vec<String> = vec!["<none>".into()];
    actions.extend(l.state.keybindings.get("vehicles").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|b| b.get("action").and_then(|x| x.as_str()).map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
    // H-pattern shifters use OMSI's "_fest" actions: pressing the gate selects the gear,
    // releasing it fires "_fest_off", which lets the bus script return to neutral.
    for a in ["kw_s_R_fest", "kw_s_1_fest", "kw_s_2_fest", "kw_s_3_fest", "kw_s_4_fest", "kw_s_5_fest", "kw_s_6_fest", "kw_s_7_fest", "kw_s_8_fest", "kw_s_9_fest", "kw_s_10_fest"] {
        if !actions.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            actions.push(a.to_string());
        }
    }
    // the game's own view actions (looking around while held, the cameras, the views)
    for a in PAD_GAME_ACTIONS {
        if !actions.iter().any(|x| x == a) {
            actions.insert(1, a.to_string());
        }
    }
    actions.dedup();
    let labels: Vec<String> = actions.iter().enumerate().map(|(i, a)| if i == 0 { a.clone() } else { action_text(names, a) }).collect();
    let mut dirty = false;
    let lit = pv.last_pressed.filter(|(_, t)| t.elapsed().as_secs_f32() < 4.0).map(|(b, _)| b);
    // Some OMSI configs contain hundreds of empty trailing slots (the G920 report had
    // 131 entries for 18 physical buttons). Keep them on disk, but do not fill the UI with them.
    let shown_buttons = shown_button_count(&d.buttons, live_dev.map(|c| c.buttons).unwrap_or(0), pv.revealed_button);
    // (the axes, then every button of the device: the list scrolls - it stopped at the ten
    // buttons that fitted)
    let list = Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h - 50.0);
    let mut buttons_start_y = 0.0;
    l.ui.scroll_area("pad-detail", list, &mut |ui, v| {
        let x0 = v.x + 6.0;
        let w = v.w - 16.0;
        let mut y = v.y;
        if live_dev.is_some_and(|c| c.ff_capable) || d.ff_invert.is_some() {
            let (mut steering_force, mut vibration) = d.ff_scale.unwrap_or((1.0, 1.0));
            if ui.slider("pad-ff-steering", Rect::new(x0, y, w, ROW), &mut steering_force, 0.0, 2.0, 0.05, "Steering force", &|v| format!("{:.0}%", v * 100.0)) {
                d.ff_scale = Some((steering_force, vibration));
                dirty = true;
            }
            y += ROW + 6.0;
            if ui.slider("pad-ff-vibration", Rect::new(x0, y, w, ROW), &mut vibration, 0.0, 2.0, 0.05, "Vibration", &|v| format!("{:.0}%", v * 100.0)) {
                d.ff_scale = Some((steering_force, vibration));
                dirty = true;
            }
            y += ROW + 20.0;
            if !live_dev.is_some_and(|c| c.gamepad) {
                let mut invert = d.ff_invert.unwrap_or_else(|| l.state.settings.get("ff_invert").and_then(|v| v.as_bool()).unwrap_or(false));
                if ui.toggle("pad-ff-invert", Rect::new(x0, y, w, ROW), &mut invert, "Invert force feedback") {
                    d.ff_invert = Some(invert);
                    dirty = true;
                }
                y += ROW + 20.0;
            }
        }
        let lab_w = if w < 520.0 { 84.0 } else { 110.0 };
        let inv_w = 110.0;
        let shp_w = 140.0;
        let sel_w = (w - lab_w - inv_w - shp_w - 60.0 - 4.0 * GAP).clamp(120.0, 200.0);
        let bar_w = (w - lab_w - sel_w - inv_w - shp_w - 4.0 * GAP).max(30.0);
        let shapes: Vec<String> = crate::controllers::AXIS_SHAPES.iter().map(|s| s.0.to_string()).collect();
        // (a device of buttons only has no axes to give a function)
        for a in (0..8).filter(|_| !buttons_only) {
            let r = Rect::new(x0, y, w, ROW);
            ui.label(Rect::new(r.x, r.y, lab_w, r.h), AXES[a]);
            let bar = Rect::new(r.x + lab_w + GAP, r.y + 12.0, bar_w, r.h - 24.0);
            ui.p().rounded(bar, 4.0, Color::WHITE.alpha(0.06));
            if let Some((_, v)) = live.iter().find(|(k, _)| *k == a) {
                let x = bar.x + (v.clamp(-1.0, 1.0) + 1.0) * 0.5 * bar.w;
                ui.p().rounded(Rect::new(x - 2.0, bar.y - 3.0, 4.0, bar.h + 6.0), 2.0, accent());
            }
            let mut sel = (Func::code(d.axes[a].map(|x| x.0)) + 1) as usize;
            if ui.select(&format!("pad-axis-{a}"), Rect::new(bar.right() + GAP, r.y, sel_w, r.h), &mut sel, &funcs) {
                let inv = d.axes[a].map(|x| x.1).unwrap_or(false);
                d.axes[a] = Func::from_code(sel as i32 - 1).map(|f| (f, inv));
                dirty = true;
            }
            let mut inv = d.axes[a].map(|x| x.1).unwrap_or(false);
            if d.axes[a].is_some() && ui.toggle(&format!("pad-inv-{a}"), Rect::new(bar.right() + GAP + sel_w + GAP, r.y, inv_w, r.h), &mut inv, "Reversed") {
                if let Some(x) = d.axes[a].as_mut() {
                    x.1 = inv;
                }
                dirty = true;
            }
            if d.axes[a].is_some() {
                // the characteristic: the curve bits of the flags, the range extension kept
                let curve = d.axis_flags[a] & (4 | 8 | 0x10);
                let mut shp = crate::controllers::AXIS_SHAPES.iter().position(|s| s.1 == curve).unwrap_or(0);
                if ui.select(&format!("pad-shape-{a}"), Rect::new(bar.right() + GAP + sel_w + GAP + inv_w + GAP, r.y, shp_w, r.h), &mut shp, &shapes) {
                    d.axis_flags[a] = (d.axis_flags[a] & !(4 | 8 | 0x10)) | crate::controllers::AXIS_SHAPES[shp].1;
                    dirty = true;
                }
            }
            y += ROW + 6.0;
        }
        // buttons: their key actions
        y += 10.0;
        ui.text_in("Buttons", Rect::new(x0, y, w, 20.0), 14.0, Weight::Bold, TEXT, Align::Left);
        y += 26.0;
        buttons_start_y = y - v.y;
        let cols = if w < 560.0 { 1usize } else { 2 };
        let cw = (w - GAP * (cols - 1) as f32) / cols as f32;
        let per_row = ROW + 4.0;
        let rows = shown_buttons.div_ceil(cols);
        let latching = &mut d.latching;
        for (b, (act, _)) in d.buttons.iter_mut().take(shown_buttons).enumerate() {
            let (col, row) = (b / rows.max(1), b % rows.max(1));
            let r = Rect::new(x0 + col as f32 * (cw + GAP), y + row as f32 * per_row, cw, ROW);
            let label = button_label(b);
            if lit == Some(b) {
                ui.p().rounded(Rect::new(r.x - 4.0, r.y - 2.0, r.w + 8.0, r.h + 4.0), 6.0, accent().alpha(0.28));
            }
            ui.label(Rect::new(r.x, r.y, 90.0, r.h), &label);
            // (a latching switch - a turn signal lever, a lit hazard button - also switches
            // when it comes out)
            let latch_w = 104.0;
            let mut sel = actions.iter().position(|a| a.eq_ignore_ascii_case(act)).unwrap_or(0);
            if ui.select(&format!("pad-btn-{b}"), Rect::new(r.x + 90.0, r.y, r.w - 90.0 - latch_w - GAP, r.h), &mut sel, &labels) {
                *act = if sel == 0 { String::new() } else { actions[sel].clone() };
                dirty = true;
            }
            let mut latched = latching.contains(&b);
            if ui.toggle(&format!("pad-latch-{b}"), Rect::new(r.right() - latch_w, r.y, latch_w, r.h), &mut latched, "Latching") {
                latching.retain(|x| *x != b);
                if latched {
                    latching.push(b);
                    latching.sort_unstable();
                }
                dirty = true;
            }
        }
        y += rows as f32 * per_row;
        y - v.y + 8.0
    });
    if dirty {
        pv.dirty = true;
    }
    // a button pressed on the device: its line (added up to it)
    if let Some((name, n)) = pressed.into_iter().find(|(name, _)| crate::controllers::names_match(&d.name, name)) {
        if crate::controllers::names_match(&d.name, &name) && n < crate::controllers::HAT_BUTTONS + 16 {
            while d.buttons.len() <= n {
                d.buttons.push((String::new(), "0".into()));
                pv.dirty = true;
            }
            pv.capturing = false;
            pv.revealed_button = Some(n);
            let cols = if list.w - 16.0 < 560.0 { 1usize } else { 2 };
            let rows = shown_buttons.max(n + 1).div_ceil(cols).max(1);
            let row = n % rows;
            l.ui.scroll_to("pad-detail", buttons_start_y + row as f32 * (ROW + 4.0), ROW, list.h);
            pv.last_pressed = Some((n, std::time::Instant::now()));
            let now = d.buttons.get(n).map(|b| b.0.clone()).filter(|a| !a.is_empty());
            let label = button_label(n);
            let text = match now {
                Some(a) => omsi_ui::tr("%{device}: %{button} - %{action} (lit in the list: choose another there)").replace("%{action}", &omsi_ui::tr(&action_text(names, &a))),
                None => omsi_ui::tr("%{device}: %{button} - nothing yet (lit in the list: choose what it does)").into_owned(),
            };
            l.state.set_status(text.replace("%{device}", &name).replace("%{button}", &label), false);
        }
    }
    let add_r = Rect::new(inner.x, inner.bottom() - 40.0, 260.0, 36.0);
    if l.ui.button("pad-add-button", add_r, if pv.capturing { "Press a button on the device…" } else { "Add a button" }, Some("add"), ButtonKind::Normal) {
        pv.capturing = !pv.capturing;
    }
    // a device no longer used (a wheel sold, one that came along in OMSI's own file) leaves
    // the list on a second click; Save keeps it so, and a connected one can be set up again
    // from the list (#636)
    let armed = pv.confirm_remove.is_some_and(|t| t.elapsed().as_secs() < 4);
    let remove_r = Rect::new(inner.right() - 260.0, inner.bottom() - 40.0, 260.0, 36.0);
    if l.ui.button("pad-remove", remove_r, if armed { "Click again to remove" } else { "Remove this device" }, Some("delete"), ButtonKind::Danger) {
        if armed {
            release_feedback(&mut pv.io, &mut pv.feedback_test);
            let name = remove_device(devices, &mut pv.selected);
            pv.capturing = false;
            pv.revealed_button = None;
            pv.last_pressed = None;
            pv.confirm_remove = None;
            pv.dirty = true;
            l.state.set_status(format!("{name} removed: press Save to keep it so."), false);
        } else {
            pv.confirm_remove = Some(std::time::Instant::now());
        }
    }
}

/// Take the device shown (`selected`) out of the list; the one below it (or the last) is
/// shown next. Its name.
fn remove_device(devices: &mut Vec<crate::controllers::DeviceCfg>, selected: &mut usize) -> String {
    let name = devices.remove(*selected).name;
    *selected = (*selected).min(devices.len().saturating_sub(1));
    name
}

/// A device's button `b` as its line names it: a hat's direction, or the button's number.
fn button_label(b: usize) -> String {
    match b.checked_sub(crate::controllers::HAT_BUTTONS) {
        Some(h) => omsi_ui::tr(["Hat %{n} up", "Hat %{n} right", "Hat %{n} down", "Hat %{n} left"][h % 4]).replace("%{n}", &(h / 4 + 1).to_string()),
        None => omsi_ui::tr("Button %{n}").replace("%{n}", &(b + 1).to_string()),
    }
}

/// The steps of the set-up assistant (see `Wizard`): what the player is asked each time.
const WIZARD_STEPS: [(&str, &str); 5] = [
    ("Let go of everything", "Take your hands off the wheel and your feet off the pedals (the wheel in the middle), then press Next."),
    ("Steering", "Turn the wheel (or move the stick) all the way to the LEFT and hold it there, then press Next."),
    ("Throttle", "Press the throttle pedal all the way down and hold it, then press Next. No pedals: Skip."),
    ("Brake", "Press the brake pedal all the way down and hold it, then press Next. No brake pedal: Skip."),
    ("Clutch", "Press the clutch pedal all the way down and hold it, then press Next. No clutch: Skip."),
];

/// One frame of the assistant in `r`; Some(true) when it has set the device up, Some(false)
/// when the player gave up.
fn wizard(ui: &mut Ui, r: Rect, w: &mut Wizard, d: &mut crate::controllers::DeviceCfg, live: &[(usize, f32)], connected: bool, feedback: bool) -> Option<bool> {
    let (title, text) = WIZARD_STEPS[w.step];
    ui.text_in(&omsi_ui::tr("Step %{n} of %{total}: %{title}").replace("%{n}", &(w.step + 1).to_string()).replace("%{total}", &WIZARD_STEPS.len().to_string()).replace("%{title}", &omsi_ui::tr(title)), Rect::new(r.x, r.y, r.w, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
    let mut y = r.y + 34.0;
    y += ui.paragraph(text, Vec2::new(r.x, y), r.w, 13.5, Weight::Regular, TEXT_SOFT) + 10.0;
    if !connected {
        y += ui.paragraph("The device is not connected: plug it in (the list on the left marks it green).", Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, DANGER);
    }
    if let Some(e) = &w.error {
        y += ui.paragraph(e, Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, DANGER) + 6.0;
    }
    // the axes as they stand, so that the player sees the device answer
    for (k, v) in live {
        let bar = Rect::new(r.x + 90.0, y + 8.0, (r.w - 100.0).max(40.0), 8.0);
        ui.text_in(["X", "Y", "Z", "Rx", "Ry", "Rz", "Slider 1", "Slider 2"][*k], Rect::new(r.x, y, 84.0, 24.0), 12.0, Weight::Medium, TEXT_DIM, Align::Left);
        ui.p().rounded(bar, 4.0, Color::WHITE.alpha(0.06));
        let x = bar.x + (v.clamp(-1.0, 1.0) + 1.0) * 0.5 * bar.w;
        ui.p().rounded(Rect::new(x - 2.0, bar.y - 4.0, 4.0, bar.h + 8.0), 2.0, accent());
        y += 26.0;
    }
    let now = |live: &[(usize, f32)]| {
        let mut a = [None; 8];
        for (k, v) in live {
            a[*k] = Some(*v);
        }
        a
    };
    let by = r.bottom() - 40.0;
    if ui.button("wiz-cancel", Rect::new(r.x, by, 120.0, 36.0), "Cancel", None, ButtonKind::Ghost) {
        return Some(false);
    }
    let skip = w.step >= 2 && ui.button("wiz-skip", Rect::new(r.right() - 260.0, by, 110.0, 36.0), "Skip", None, ButtonKind::Normal);
    let next = ui.button("wiz-next", Rect::new(r.right() - 140.0, by, 140.0, 36.0), if w.step + 1 == WIZARD_STEPS.len() && !feedback { "Finish" } else { "Next" }, Some("chevron_right"), ButtonKind::Primary);
    if !(next || skip) {
        return None;
    }
    let cur = now(live);
    w.error = None;
    if w.step == 0 {
        if live.is_empty() {
            w.error = Some("The device shows no axis yet: move the wheel and the pedals a little, let go, and press Next again.".into());
            return None;
        }
        w.rest = cur;
    } else if skip {
        w.at.push([None; 8]);
    } else {
        // the axis that moved most since everything was let go (one taken before is not
        // taken again - but the throttle's axis may turn out to be the brake's too)
        let used: Vec<usize> = w.at.iter().filter_map(|a| moved_most(&w.rest, a, &[]).map(|m| m.0)).collect();
        let exclude: Vec<usize> = if w.step == 3 { used.iter().copied().take(1).collect() } else { used.clone() };
        match moved_most(&w.rest, &cur, &exclude) {
            Some(_) => w.at.push(cur),
            None => {
                w.error = Some("Nothing moved far enough. Hold it all the way, then press Next (or Skip).".into());
                return None;
            }
        }
    }
    w.step += 1;
    if w.step < WIZARD_STEPS.len() {
        return None;
    }
    let axes = wizard_result(&w.rest, &w.at);
    if feedback && axes.iter().any(|a| matches!(a, Some((crate::controllers::Func::Steering, _)))) {
        return None;
    }
    d.axes = axes;
    Some(true)
}

fn feedback_setup(
    ui: &mut Ui, r: Rect, w: &mut Wizard, d: &mut crate::controllers::DeviceCfg,
    live: &[(usize, f32)], device: Option<&crate::controllers::Connected>,
    io: &mut Option<crate::controllers::Devices>, active: &mut bool, hwnd: Option<isize>, global_invert: bool,
) -> Option<bool> {
    let axes = wizard_result(&w.rest, &w.at);
    let axis = axes.iter().position(|a| matches!(a, Some((crate::controllers::Func::Steering, _))));
    ui.text_in("Force feedback direction", Rect::new(r.x, r.y, r.w, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
    let warning = "INJURY RISK: TAKE YOUR HANDS OFF THE WHEEL. Keep hands and fingers clear before starting and throughout the test.";
    let warning_height = ui.paragraph_height(warning, r.w - 58.0, 14.5, Weight::Bold) + 20.0;
    let warning_rect = Rect::new(r.x, r.y + 34.0, r.w, warning_height);
    ui.p().rounded(warning_rect, 6.0, DANGER.alpha(0.15));
    ui.p().rounded_border(warning_rect, 6.0, 1.5, DANGER);
    ui.icon("warning", Vec2::new(warning_rect.x + 22.0, warning_rect.center().y), 26.0, DANGER);
    ui.paragraph(warning, Vec2::new(warning_rect.x + 44.0, warning_rect.y + 10.0), warning_rect.w - 58.0, 14.5, Weight::Bold, DANGER);
    let body_y = warning_rect.bottom() + 12.0;
    ui.scroll_area("wiz-ff-body", Rect::new(r.x, body_y, r.w, r.bottom() - 56.0 - body_y), &mut |ui, r| {
        let mut y = r.y;
        y += ui.paragraph("The test applies two short forces in opposite directions. Finish and press Save to keep the detected direction for this wheel.", Vec2::new(r.x, y), r.w, 13.5, Weight::Regular, TEXT_SOFT) + 16.0;
        if let Some((started, test)) = w.calibration.as_mut() {
            if *active {
                let position = axis.and_then(|a| live.iter().find(|(k, _)| *k == a).map(|(_, x)| *x));
                if let Some(force) = test.update(started.elapsed().as_secs_f32(), position) {
                    if !device.zip(axis).zip(io.as_mut()).is_some_and(|((device, axis), io)| io.calibration_pulse(&device.name, axis, force)) {
                        test.fail("Force feedback is unavailable. Choose the direction manually.");
                    }
                }
                if let Some(result) = test.result {
                    release_feedback(io, active);
                    if let Ok(invert) = result {
                        w.ff_choice = Some(invert);
                    }
                }
            }
            let (message, color) = match test.result {
                Some(Ok(false)) => ("Direction detected: normal", OK),
                Some(Ok(true)) => ("Direction detected: inverted", OK),
                Some(Err(message)) => (message, DANGER),
                None => ("Testing: keep your hands off the wheel…", TEXT_SOFT),
            };
            y += ui.paragraph(message, Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, color) + 12.0;
        }
        if !*active {
            ui.slider("wiz-ff-strength", Rect::new(r.x, y, r.w, ROW), &mut w.test_strength,
                      crate::ffb_calibration::PULSE_FORCE, crate::ffb_calibration::MAX_PULSE_FORCE, 0.01,
                      "Test strength", &|v| format!("{:.0}%", v * 100.0));
            y += ROW + 8.0;
            y += ui.paragraph("If the wheel barely moves, increase Test strength and retry. Keep your hands clear.", Vec2::new(r.x, y), r.w, 13.0, Weight::Regular, TEXT_DIM) + 10.0;
            if ui.button("wiz-ff-test", Rect::new(r.x, y, 180.0, 36.0), "Start test", Some("play_arrow"), ButtonKind::Primary) {
                if device.is_none() || axis.is_none() {
                    w.error = Some("The wheel is unavailable. Reconnect it and try again.".into());
                } else {
                    *io = None;
                    *io = Some(crate::controllers::Devices::new(hwnd, true));
                    *active = true;
                    w.error = None;
                    log::info!("FFB calibration: device {}, raw steering axis {:?}, test strength {:.0}%", device.unwrap().name, axis, w.test_strength * 100.0);
                    w.calibration = Some((std::time::Instant::now(), crate::ffb_calibration::Calibration::new(w.test_strength)));
                }
            }
            y += 48.0;
            let mut invert = w.ff_choice.or(d.ff_invert).unwrap_or(global_invert);
            if ui.toggle("wiz-ff-manual", Rect::new(r.x, y, r.w, ROW), &mut invert, "Invert force feedback") {
                w.ff_choice = Some(invert);
            }
            y += ROW + 8.0;
            y += ui.paragraph("If detection is inconclusive, retry or choose the direction manually. You can change it later on this device's page.", Vec2::new(r.x, y), r.w, 13.0, Weight::Regular, TEXT_DIM) + 8.0;
        }
        if let Some(error) = &w.error {
            y += ui.paragraph(error, Vec2::new(r.x, y), r.w, 13.0, Weight::Medium, DANGER) + 8.0;
        }
        y - r.y
    });
    let by = r.bottom() - 40.0;
    if ui.button("wiz-cancel", Rect::new(r.x, by, 120.0, 36.0), "Cancel", None, ButtonKind::Ghost) {
        return Some(false);
    }
    if !*active && ui.button("wiz-ff-finish", Rect::new(r.right() - 140.0, by, 140.0, 36.0), "Finish", Some("check"), ButtonKind::Primary) {
        d.axes = axes;
        d.ff_invert = Some(w.ff_choice.or(d.ff_invert).unwrap_or(global_invert));
        return Some(true);
    }
    None
}

/// The axes the assistant found: `rest` where everything rested, `at` where the axes stood
/// with the wheel turned left, the throttle, the brake and the clutch pressed (all None: that
/// step skipped).
fn wizard_result(rest: &[Option<f32>; 8], at: &[[Option<f32>; 8]]) -> [Option<(crate::controllers::Func, bool)>; 8] {
    use crate::controllers::Func;
    let mut axes: [Option<(Func, bool)>; 8] = [None; 8];
    let steer = at.first().and_then(|a| moved_most(rest, a, &[]));
    if let Some((k, delta)) = steer {
        // turned left the value falls: else the axis runs the other way
        axes[k] = Some((Func::Steering, delta > 0.0));
    }
    let taken: Vec<usize> = steer.map(|s| vec![s.0]).unwrap_or_default();
    let pedal = |i: usize, ex: &[usize]| at.get(i).and_then(|a| moved_most(rest, a, ex));
    let throttle = pedal(1, &taken);
    let brake = pedal(2, &taken);
    match (throttle, brake) {
        // one axis for both (pedals on a single axis): the throttle towards the raw maximum
        // (as in Omsi.exe), else the axis is reversed
        (Some((kt, dt)), Some((kb, db))) if kt == kb && dt * db < 0.0 => axes[kt] = Some((Func::ThrottleBrake, dt < 0.0)),
        _ => {
            // a pedal pressed goes towards 1
            if let Some((k, dl)) = throttle {
                axes[k] = Some((Func::Throttle, dl < 0.0));
            }
            if let Some((k, dl)) = brake.filter(|b| Some(b.0) != throttle.map(|t| t.0)) {
                axes[k] = Some((Func::Brake, dl < 0.0));
            }
        }
    }
    let mut ex = taken.clone();
    ex.extend(throttle.map(|t| t.0));
    ex.extend(brake.map(|t| t.0));
    if let Some((k, dl)) = pedal(3, &ex) {
        axes[k] = Some((Func::Clutch, dl < 0.0));
    }
    axes
}

/// The axis that moved most from `rest` to `now` (at least a sixth of its travel), not one of
/// `exclude`: (slot, how far, signed).
fn moved_most(rest: &[Option<f32>; 8], now: &[Option<f32>; 8], exclude: &[usize]) -> Option<(usize, f32)> {
    (0..8)
        .filter(|k| !exclude.contains(k))
        .filter_map(|k| Some((k, now[k]? - rest[k].unwrap_or(0.0))))
        .filter(|(_, d)| d.abs() > 0.33)
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
}

/// Write the devices to the content folder's `Inputs/gamectrler.cfg` (OMSI 2's own is only
/// read; the game takes the content folder's first).
fn save_gamectrler(devices: &[crate::controllers::DeviceCfg]) -> Result<std::path::PathBuf, String> {
    let candidate = core::content_dir().unwrap_or_else(core::data_dir).join("Inputs");
    let dir = if (candidate.exists() || std::fs::create_dir_all(&candidate).is_ok()) && omsi_cfg::is_writable(&candidate) {
        candidate
    } else {
        core::data_dir().join("Inputs")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("gamectrler.cfg");
    std::fs::write(&p, crate::controllers::cfg_text(devices)).map_err(|e| e.to_string())?;
    omsi_cfg::content_changed();
    Ok(p)
}

/// Settings → Driving keys: "Custom controls", the keys of the Controls page.
fn use_custom_keys(l: &mut Launcher) -> bool {
    if l.state.settings.get("drive_keys").and_then(|v| v.as_str()) == Some("omsi") {
        return false;
    }
    l.state.settings["drive_keys"] = json!("omsi");
    l.state.settings_dirty = 0.3;
    l.state.set_status("Driving keys: Custom controls - the game uses the keys of this page.", false);
    true
}

fn save_keys(l: &mut Launcher, vr_binding: bool) {
    match core::save_keybindings(&l.state.keybindings) {
        Ok(()) => {
            l.state.keybindings_error.clear();
            if let Ok(k) = core::get_keybindings() {
                l.state.keybindings = k;
            }
            // a key changed is a key the player wants to use: with a ready-made layout it
            // would be ignored wherever that layout has a key of its own
            if !vr_binding && use_custom_keys(l) {
                l.state.set_status("Key bindings saved; Driving keys switched to Custom controls so the game uses them.", false);
            } else {
                l.state.set_status("Key bindings saved.", false);
            }
        }
        Err(e) => {
            l.state.keybindings_error = format!("{e:#}");
            l.state.set_status(format!("{e:#}"), true);
        }
    }
}

// --- sessions ---------------------------------------------------------------------------------

fn ago(t: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(t);
    let s = now.saturating_sub(t);
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s / 60) % 60)
    }
}

pub fn sessions(l: &mut Launcher, area: Rect) {
    let body = area;
    let list = l.state.instances.clone();
    if list.is_empty() {
        let r = Rect::new(body.x, body.y, body.w.min(720.0), 120.0);
        l.ui.card(r);
        l.ui.icon("sports_esports", Vec2::new(r.x + 40.0, r.center().y), 36.0, TEXT_FAINT);
        l.ui.paragraph("No game is running. Start a duty on the Drive page; to drive with friends, turn on hosting on the Multiplayer page and give them the code shown here.", Vec2::new(r.x + 76.0, r.y + 30.0), r.w - 100.0, 13.5, Weight::Regular, TEXT_DIM);
        return;
    }
    let names: std::collections::HashMap<String, String> = l.state.vehicles.iter().map(|v| (v.file.clone(), v.name.clone())).collect();
    let short_bus = |b: &str| names.get(b).cloned().unwrap_or_else(|| b.rsplit('/').next().unwrap_or("").trim_end_matches(".bus").to_string());
    let mut y = body.y - l.ui.scroll.get(&id_of("sessions")).copied().unwrap_or(0.0);
    // (the page's whole width, as the other pages: at 900 a wide window had the cards in its
    // left half and the buttons in the middle of nowhere)
    let view = Rect::new(body.x, body.y, body.w, body.h);
    let mut actions: Vec<(u32, &str)> = Vec::new();
    let mut copy: Option<String> = None;
    l.ui.push_clip(view, 0.0);
    for i in &list {
        let lan = i.lan_status.clone().unwrap_or(Value::Null);
        let role = lan.get("role").and_then(|x| x.as_str()).unwrap_or("");
        let players: Vec<Value> = lan.get("players").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        let chat: Vec<String> = lan.get("chat").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        let warnings: Vec<String> = lan.get("warnings").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
        let log_open = l.state.open_logs.contains(&i.pid);
        let log_lines = l.state.logs.get(&i.pid).cloned().unwrap_or_default();
        let mut h = 96.0;
        if role == "host" {
            h += 96.0;
        } else if !role.is_empty() {
            h += 24.0;
        }
        h += if players.is_empty() { 0.0 } else { 26.0 + players.len() as f32 * 20.0 };
        h += if chat.is_empty() { 0.0 } else { 26.0 + chat.len().min(6) as f32 * 18.0 };
        h += warnings.len() as f32 * 20.0;
        if log_open {
            h += 220.0;
        }
        let r = Rect::new(view.x, y, view.w, h);
        l.ui.card(r);
        let running = i.running;
        let c = Vec2::new(r.x + 24.0, r.y + 28.0);
        if running {
            let pulse = (l.ui.time * 3.0).sin() * 0.5 + 0.5;
            l.ui.p().circle(c, 6.0 + 3.0 * pulse, OK.alpha(0.25));
        }
        l.ui.p().circle(c, 6.0, if running { OK } else { TEXT_FAINT });
        let duty = i.line.as_ref().map(|ln| match &i.tour {
            Some(t) => format!(" · {}", omsi_ui::tr("line %{line} / %{tour}").replace("%{line}", ln).replace("%{tour}", t)),
            None => format!(" · {}", omsi_ui::tr("line %{line}").replace("%{line}", ln)),
        }).unwrap_or_default();
        l.ui.text_in(&format!("{} · {}{duty}", short_map(&i.map), short_bus(&i.bus)), Rect::new(r.x + 42.0, r.y + 16.0, r.w - 260.0, 24.0), 16.0, Weight::Black, TEXT, Align::Left);
        let status = if running {
            if l.state.stopping.contains(&i.pid) || i.stopping.is_some() { omsi_ui::tr("stopping - saving the run…").into_owned() } else { omsi_ui::tr("running for %{time}").replace("%{time}", &ago(i.started)) }
        } else if i.exit_code == Some(0) {
            omsi_ui::tr("ended").into_owned()
        } else if i.killed {
            omsi_ui::tr("ended (killed - it did not end by itself, the run is not saved)").into_owned()
        } else {
            match i.exit_code {
                Some(c) => omsi_ui::tr("ended (exit code %{code})").replace("%{code}", &c.to_string()),
                None => omsi_ui::tr("ended").into_owned(),
            }
        };
        l.ui.text_in(&format!("{status} · {}", omsi_ui::tr("driver %{name}").replace("%{name}", &i.profile)), Rect::new(r.x + 42.0, r.y + 42.0, r.w - 60.0, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        l.ui.text_in(&i.last_line, Rect::new(r.x + 42.0, r.y + 62.0, r.w - 60.0, 18.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
        // buttons
        let bw = 110.0;
        if running {
            let stopping = l.state.stopping.contains(&i.pid);
            if l.ui.button(&format!("stop-{}", i.pid), Rect::new(r.right() - 18.0 - bw, r.y + 14.0, bw, 34.0), if stopping { "Stopping…" } else { "Stop" }, Some("close"), ButtonKind::Danger) && !stopping {
                actions.push((i.pid, "stop"));
            }
        }
        if l.ui.button(&format!("log-{}", i.pid), Rect::new(r.right() - 18.0 - bw - if running { bw + 8.0 } else { 0.0 }, r.y + 14.0, bw, 34.0), if log_open { "Hide log" } else { "Show log" }, Some("receipt_long"), ButtonKind::Normal) {
            actions.push((i.pid, "log"));
        }
        let mut yy = r.y + 86.0;
        if role == "host" {
            let code = lan.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let box_r = Rect::new(r.x + 20.0, yy, r.w - 40.0, 84.0);
            l.ui.p().rounded(box_r, 10.0, accent().alpha(0.10));
            l.ui.p().rounded_border(box_r, 10.0, 1.0, accent().alpha(0.5));
            l.ui.text_in("SESSION CODE", Rect::new(box_r.x + 16.0, box_r.y + 8.0, 200.0, 16.0), 10.5, Weight::Black, accent(), Align::Left);
            l.ui.text_in(&code, Rect::new(box_r.x + 16.0, box_r.y + 26.0, box_r.w - 170.0, 30.0), 20.0, Weight::Condensed, TEXT, Align::Left);
            l.ui.text_in("Your friends paste it into Multiplayer → Connect by Code.", Rect::new(box_r.x + 16.0, box_r.y + 58.0, box_r.w - 170.0, 18.0), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            if l.ui.button(&format!("copy-{}", i.pid), Rect::new(box_r.right() - 146.0, box_r.y + 24.0, 130.0, 36.0), "Copy code", Some("content_copy"), ButtonKind::Primary) {
                copy = Some(code.clone());
            }
            yy += 96.0;
        } else if role == "client" {
            let connected = lan.get("connected").and_then(|x| x.as_bool()).unwrap_or(false);
            let text = if let Some(rej) = lan.get("rejected").and_then(|x| x.as_str()) {
                omsi_ui::tr("not connected: %{reason}").replace("%{reason}", rej)
            } else if connected {
                omsi_ui::tr("connected to %{host}").replace("%{host}", lan.get("host_name").and_then(|x| x.as_str()).unwrap_or(""))
            } else {
                omsi_ui::tr("connecting…").into_owned()
            };
            l.ui.text_in(&omsi_ui::tr("Multiplayer: %{state}").replace("%{state}", &text), Rect::new(r.x + 42.0, yy, r.w - 60.0, 20.0), 12.5, Weight::Medium, if connected { OK } else { WARN }, Align::Left);
            yy += 24.0;
        }
        if !players.is_empty() {
            l.ui.text_in("PLAYERS", Rect::new(r.x + 42.0, yy, 200.0, 18.0), 10.5, Weight::Black, TEXT_FAINT, Align::Left);
            yy += 22.0;
            for p in &players {
                let s = |k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let pax = p.get("passengers").and_then(|x| x.as_i64()).unwrap_or(0);
                let dest = if s("destination").is_empty() { String::new() } else { format!(" · {} → {}", s("line"), s("destination")) };
                l.ui.text_in(&format!("{} · {}{dest}{} · {}", s("name"), short_bus(&s("bus")), if pax > 0 { format!(" · {}", omsi_ui::tr("%{n} passengers").replace("%{n}", &pax.to_string())) } else { String::new() }, s("where")), Rect::new(r.x + 42.0, yy, r.w - 60.0, 18.0), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
                yy += 20.0;
            }
        }
        if !chat.is_empty() {
            l.ui.text_in("CHAT  (V in the game to write)", Rect::new(r.x + 42.0, yy + 4.0, 300.0, 18.0), 10.5, Weight::Black, TEXT_FAINT, Align::Left);
            yy += 26.0;
            for c in chat.iter().rev().take(6).rev() {
                l.ui.text_in(c, Rect::new(r.x + 42.0, yy, r.w - 60.0, 18.0), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
                yy += 18.0;
            }
        }
        for w in &warnings {
            l.ui.icon("warning", Vec2::new(r.x + 50.0, yy + 9.0), 15.0, WARN);
            l.ui.text_in(w, Rect::new(r.x + 64.0, yy, r.w - 80.0, 18.0), 12.0, Weight::Medium, WARN, Align::Left);
            yy += 20.0;
        }
        if log_open {
            let lr = Rect::new(r.x + 20.0, yy + 6.0, r.w - 40.0, 204.0);
            l.ui.p().rounded(lr, 8.0, Color::rgba(6, 8, 10, 0.9));
            let text: Vec<String> = log_lines.iter().rev().take(11).rev().cloned().collect();
            for (k, line) in text.iter().enumerate() {
                l.ui.text_in(line, Rect::new(lr.x + 10.0, lr.y + 6.0 + k as f32 * 18.0, lr.w - 20.0, 18.0), 11.0, Weight::Regular, if line.contains("ERROR") { DANGER } else if line.contains("WARN") { WARN } else { TEXT_DIM }, Align::Left);
            }
        }
        y += h + GAP;
    }
    l.ui.pop_clip();
    // scrolling the list
    let content = y + l.ui.scroll.get(&id_of("sessions")).copied().unwrap_or(0.0) - body.y;
    if l.ui.hover(view) && l.ui.input.wheel.y.abs() > 0.0 {
        let s = l.ui.scroll.entry(id_of("sessions")).or_insert(0.0);
        *s = (*s - l.ui.input.wheel.y * 40.0).clamp(0.0, (content - view.h).max(0.0));
    }
    for (pid, what) in actions {
        match what {
            "stop" => l.state.stop(pid),
            _ => {
                if !l.state.open_logs.remove(&pid) {
                    l.state.open_logs.insert(pid);
                    l.state.log_tail(pid);
                }
            }
        }
    }
    if let Some(c) = copy {
        l.ui.clipboard_out = Some(c);
        l.state.set_status("Session code copied.", false);
    }
    let _ = hhmm;
}

// --- mods ----------------------------------------------------------------------------------------

pub fn mods(l: &mut Launcher, area: Rect) {
    if !l.state.mods_asked {
        l.state.load_mods();
    }
    let body = area;
    let cols = 3;
    let cw = (body.w - GAP * 2.0 * (cols as f32 - 1.0)) / cols as f32;
    let colr = |k: usize| Rect::new(body.x + k as f32 * (cw + GAP * 2.0), body.y, cw, body.h);
    // install
    let c0 = colr(0);
    l.ui.card(c0);
    let inner = l.ui.heading(Rect::new(c0.x + 18.0, c0.y + 14.0, c0.w - 36.0, c0.h - 28.0), "Install a mod", Some("download"));
    let mut y = inner.y;
    let half = (inner.w - GAP) * 0.5;
    if l.ui.button("mod-folder", Rect::new(inner.x, y, half, 40.0), "Choose a folder", Some("folder_open"), ButtonKind::Primary) {
        if super::mobile::mobile() {
            l.browse(super::mobile::Purpose::ModFolder, "");
        } else if let Some(p) = core::pick_mod(false) {
            l.state.install(p.to_string_lossy().to_string());
        }
    }
    if l.ui.button("mod-zip", Rect::new(inner.x + half + GAP, y, half, 40.0), "Choose archive", Some("inventory_2"), ButtonKind::Normal) {
        if super::mobile::mobile() {
            l.browse(super::mobile::Purpose::ModZip, "");
        } else if let Some(p) = core::pick_mod(true) {
            l.state.install(p.to_string_lossy().to_string());
        }
    }
    y += 52.0;
    l.ui.label(Rect::new(inner.x, y, inner.w, 20.0), "Archive install mode");
    y += 22.0;
    let mut m = l.state.mod_mode;
    if l.ui.segmented("mod-mode", Rect::new(inner.x, y, inner.w, 34.0), &mut m, &["Auto", "Unpacked", "Used in place"]) {
        l.state.mod_mode = m;
    }
    y += 44.0;
    let drop = Rect::new(inner.x, y, inner.w, 110.0);
    let hot = l.pages.drop_hover;
    let t = l.ui.anim(id_of("drop"), if hot { 1.0 } else { 0.0 }, 0.1);
    l.ui.p().rounded(drop, 12.0, accent().alpha(0.05 + 0.12 * t));
    // a dashed edge
    let per = 2.0 * (drop.w + drop.h);
    let n = (per / 14.0) as usize;
    for k in 0..n {
        let s = k as f32 * per / n as f32;
        let p = if s < drop.w {
            Vec2::new(drop.x + s, drop.y)
        } else if s < drop.w + drop.h {
            Vec2::new(drop.right(), drop.y + s - drop.w)
        } else if s < 2.0 * drop.w + drop.h {
            Vec2::new(drop.right() - (s - drop.w - drop.h), drop.bottom())
        } else {
            Vec2::new(drop.x, drop.bottom() - (s - 2.0 * drop.w - drop.h))
        };
        l.ui.p().circle(p, 1.3, accent().alpha(0.35 + 0.5 * t));
    }
    l.ui.icon("upload", Vec2::new(drop.center().x, drop.y + 38.0), 30.0, accent().alpha(0.6 + 0.4 * t));
    l.ui.text_in("…or drop a mod folder or .zip, .7z or .rar onto this window", Rect::new(drop.x, drop.y + 62.0, drop.w, 30.0), 12.5, Weight::Medium, TEXT_SOFT, Align::Center);
    y += 122.0;
    if !l.state.mod_path.is_empty() {
        let p = l.state.mod_path.clone();
        y += l.ui.paragraph(&p, Vec2::new(inner.x, y), inner.w, 11.5, Weight::Regular, TEXT_FAINT);
        match l.state.mod_info.clone() {
            Some(Ok(i)) if i.is_archive => {
                let fit = if i.fits { omsi_ui::tr("fits (%{free} free)").replace("%{free}", &fmt_bytes(i.free_bytes)) } else { omsi_ui::tr("does not fit: needs %{needed}, %{free} free").replace("%{needed}", &fmt_bytes(i.needed_bytes)).replace("%{free}", &fmt_bytes(i.free_bytes)) };
                let place = if i.in_place_ok { omsi_ui::tr("can be used in place").into_owned() } else { i.in_place.clone() };
                let what = omsi_ui::tr("%{size} archive, %{files} files, %{unpacked} unpacked").replace("%{size}", &fmt_bytes(i.archive_bytes)).replace("%{files}", &i.files.to_string()).replace("%{unpacked}", &fmt_bytes(i.unpacked_bytes));
                y += l.ui.paragraph(&format!("{what} - {fit}; {place}"), Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, if i.fits { TEXT_DIM } else { WARN });
            }
            Some(Err(e)) => {
                y += l.ui.paragraph(&e, Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, DANGER);
            }
            _ => {}
        }
    }
    y += 10.0;
    if let Some(m) = l.state.mods.clone() {
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "The Mods folder", None);
        y += 30.0;
        // (the folder on a line of its own, cut to the column: a path has no spaces to break
        // at and ran out of the card)
        y += l.ui.paragraph("Anything put into this folder is installed by itself once it has finished copying.", Vec2::new(inner.x, y), inner.w, 12.0, Weight::Regular, TEXT_DIM);
        let at = Rect::new(inner.x, y + 2.0, inner.w, 18.0);
        l.ui.text_in(&m.inbox, at, 12.0, Weight::Medium, TEXT_SOFT, Align::Left);
        l.ui.tooltip(at, &m.inbox);
        y += 24.0;
        if !m.inbox_items.is_empty() {
            l.ui.paragraph(&omsi_ui::tr("In it now: %{items}").replace("%{items}", &m.inbox_items.join(", ")), Vec2::new(inner.x, y + 4.0), inner.w, 12.0, Weight::Regular, TEXT_SOFT);
        }
    }
    // installs
    let c1 = colr(1);
    l.ui.card(c1);
    let inner = l.ui.heading(Rect::new(c1.x + 18.0, c1.y + 14.0, c1.w - 36.0, c1.h - 28.0), "Installs", Some("inventory_2"));
    if l.ui.button("jobs-clear", Rect::new(c1.right() - 18.0 - 130.0, c1.y + 12.0, 130.0, 30.0), "Clear finished", None, ButtonKind::Ghost) {
        core::install::clear_finished();
        l.state.poll_now();
    }
    let jobs = l.state.jobs.clone();
    let mut cancel = None;
    l.ui.scroll_area("jobs", Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, inner.h), &mut |ui, v| {
        if jobs.is_empty() {
            ui.paragraph("Nothing installed since the launcher started. Big archives are checked against the free disk space before anything is unpacked; a cancelled or failed install leaves nothing behind.", Vec2::new(v.x + 6.0, v.y), v.w - 12.0, 12.5, Weight::Regular, TEXT_DIM);
            return 60.0;
        }
        let mut y = v.y;
        for j in &jobs {
            let running = j.finished.is_none();
            let msg_h = ui.paragraph_height(&j.message, v.w - 40.0, 12.0, Weight::Regular);
            let h = 50.0 + msg_h + if running { 44.0 } else { 0.0 } + j.warnings.len() as f32 * 18.0;
            let r = Rect::new(v.x + 6.0, y, v.w - 16.0, h);
            ui.p().rounded(r, 10.0, Color::WHITE.alpha(0.04));
            ui.text_in(&j.name, Rect::new(r.x + 12.0, r.y + 8.0, r.w - 120.0, 20.0), 13.5, Weight::Bold, TEXT, Align::Left);
            let sc = match j.state.as_str() {
                "done" => OK,
                "failed" => DANGER,
                "cancelled" => TEXT_DIM,
                _ => accent(),
            };
            ui.badge(Vec2::new(r.right() - 90.0, r.y + 10.0), &omsi_ui::tr(&j.state).to_uppercase(), sc);
            let mut yy = r.y + 34.0;
            if running {
                let frac = if j.bytes_total > 0 { j.bytes_done as f32 / j.bytes_total as f32 } else if j.files_total > 0 { j.files_done as f32 / j.files_total as f32 } else { 0.0 };
                ui.progress(Rect::new(r.x + 12.0, yy, r.w - 24.0, 8.0), frac, true);
                ui.text_in(&format!("{} · {} / {}", omsi_ui::tr("%{done} / %{total} files").replace("%{done}", &j.files_done.to_string()).replace("%{total}", &j.files_total.to_string()), fmt_bytes(j.bytes_done), fmt_bytes(j.bytes_total)), Rect::new(r.x + 12.0, yy + 10.0, r.w - 24.0, 16.0), 11.0, Weight::Regular, TEXT_DIM, Align::Left);
                yy += 30.0;
            }
            yy += ui.paragraph(&j.message, Vec2::new(r.x + 12.0, yy), r.w - 24.0, 12.0, Weight::Regular, if j.state == "failed" { DANGER } else { TEXT_SOFT });
            for w in &j.warnings {
                ui.text_in(&format!("⚠ {w}"), Rect::new(r.x + 12.0, yy, r.w - 24.0, 16.0), 11.0, Weight::Regular, WARN, Align::Left);
                yy += 18.0;
            }
            if running && ui.button(&format!("cancel-{}", j.id), Rect::new(r.x + 12.0, r.bottom() - 34.0, 100.0, 28.0), "Cancel", None, ButtonKind::Danger) {
                cancel = Some(j.id);
            }
            y += h + 10.0;
        }
        y - v.y
    });
    if let Some(id) = cancel {
        core::install::cancel(id);
        l.state.poll_now();
    }
    // content folder
    let c2 = colr(2);
    l.ui.card(c2);
    let inner = l.ui.heading(Rect::new(c2.x + 18.0, c2.y + 14.0, c2.w - 36.0, c2.h - 28.0), "Content folder", Some("folder_open"));
    let Some(m) = l.state.mods.clone() else {
        l.ui.text_in("Reading…", Rect::new(inner.x, inner.y, inner.w, 20.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
        return;
    };
    let mut y = inner.y;
    let at = Rect::new(inner.x, y, inner.w, 18.0);
    l.ui.text_in(&m.content_dir, at, 12.0, Weight::Medium, TEXT_SOFT, Align::Left);
    l.ui.tooltip(at, &m.content_dir);
    y += 24.0;
    l.ui.text_in(&omsi_ui::tr("%{size} free on this disk").replace("%{size}", &fmt_bytes(m.free_bytes)), Rect::new(inner.x, y, inner.w, 20.0), 13.0, Weight::Bold, accent(), Align::Left);
    y += 30.0;
    for (f, n) in &m.folders {
        l.ui.icon("folder_open", Vec2::new(inner.x + 9.0, y + 10.0), 16.0, TEXT_DIM);
        l.ui.text_in(f, Rect::new(inner.x + 26.0, y, inner.w * 0.6, 20.0), 12.5, Weight::Medium, TEXT, Align::Left);
        l.ui.text_in(&if *n == 1 { omsi_ui::tr("one entry").into_owned() } else { omsi_ui::tr("%{n} entries").replace("%{n}", &n.to_string()) }, Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_DIM, Align::Right);
        y += 24.0;
    }
    if !m.archives.is_empty() {
        y += 8.0;
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Archives used in place", None);
        y += 30.0;
        for (n, b) in &m.archives {
            l.ui.text_in(&format!("{n}  ({})", fmt_bytes(*b)), Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 22.0;
        }
    }
    if !m.waiting.is_empty() {
        y += 8.0;
        l.ui.heading(Rect::new(inner.x, y, inner.w, 28.0), "Waiting for their bus", None);
        y += 30.0;
        for w in &m.waiting {
            l.ui.text_in(w, Rect::new(inner.x, y, inner.w, 20.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            y += 22.0;
        }
    }
}

// --- setup -----------------------------------------------------------------------------------------

pub fn setup(l: &mut Launcher, area: Rect) {
    let body = area;
    let r = Rect::new(body.x, body.y, body.w.min(820.0), 300.0);
    l.ui.card(r);
    let inner = l.ui.heading(Rect::new(r.x + 20.0, r.y + 16.0, r.w - 40.0, r.h - 32.0), "Folders", Some("folder_open"));
    let mut root = l.pages.setup_root.clone().unwrap_or_else(|| l.state.config.root.clone());
    let mut game = l.pages.setup_game.clone().unwrap_or_else(|| l.state.config.game.clone());
    let mut y = inner.y;
    l.ui.label(Rect::new(inner.x, y, 150.0, ROW), "OMSI 2 folder");
    if l.ui.text_input("cfg-root", Rect::new(inner.x + 150.0, y, inner.w - 150.0 - 110.0, ROW), &mut root, "/path/to/OMSI 2", Some("folder_open")) {
        l.pages.setup_root = Some(root.clone());
    }
    if l.ui.button("browse-root", Rect::new(inner.right() - 100.0, y, 100.0, ROW), "Browse", None, ButtonKind::Normal) {
        if super::mobile::mobile() {
            let start = root.clone();
            l.browse(super::mobile::Purpose::Root, &start);
        } else if let Some(p) = core::pick_folder("The OMSI 2 folder (with maps and Vehicles in it)") {
            l.pages.setup_root = Some(p.to_string_lossy().to_string());
        }
    }
    y += ROW + 10.0;
    if core::IN_PROCESS_GAMES {
        // (a phone: the game is this app itself)
        y += l.ui.paragraph("Copy the whole OMSI 2 folder (with maps and Vehicles in it) onto the phone - by cable, from a PC or a USB stick - for example as openOMSI/OMSI 2 in the internal storage, then choose it here with Browse. Mods go into openOMSI/Mods or are installed from the Mods page.", Vec2::new(inner.x, y), inner.w, 12.5, Weight::Regular, TEXT_DIM);
    } else {
        l.ui.label(Rect::new(inner.x, y, 150.0, ROW), "Game binary");
        if l.ui.text_input("cfg-game", Rect::new(inner.x + 150.0, y, inner.w - 150.0 - 110.0, ROW), &mut game, "openomsi", Some("terminal")) {
            l.pages.setup_game = Some(game.clone());
        }
        if l.ui.button("browse-game", Rect::new(inner.right() - 100.0, y, 100.0, ROW), "Browse", None, ButtonKind::Normal) {
            if let Some(p) = core::pick_file("The openomsi program") {
                l.pages.setup_game = Some(p.to_string_lossy().to_string());
            }
        }
        y += ROW + 16.0;
        y += l.ui.paragraph("The OMSI 2 folder is the one with maps and Vehicles in it (any complete installation). The game binary is the openomsi program; it is found by itself when it sits next to the launcher.", Vec2::new(inner.x, y), inner.w, 12.5, Weight::Regular, TEXT_DIM);
    }
    y += 12.0;
    if l.ui.button("cfg-save", Rect::new(inner.x, y, 180.0, 42.0), "Save", Some("save"), ButtonKind::Primary) {
        let chosen = root.trim().to_string();
        // A folder that is no complete installation is said so, with what it lacks, and
        // nothing is saved: it was saved, then quietly replaced by the installation found
        // before, while the page said "Saved" (#1656).
        let missing = if chosen.is_empty() { Vec::new() } else { omsi_cfg::missing_original_essentials(std::path::Path::new(&chosen)) };
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().map(String::as_str).take(6).collect();
            let more = if missing.len() > shown.len() { format!(" and {} more", missing.len() - shown.len()) } else { String::new() };
            l.state.set_status(format!("Not saved: {chosen} is not a complete OMSI 2 installation - it lacks {}{more}", shown.join(", ")), true);
        } else {
            l.state.config.root = chosen.clone();
            l.state.config.game = game.trim().to_string();
            match core::save_config(&l.state.config) {
                Ok(()) => {
                    // (what was read of the folders before is forgotten: a folder copied or
                    // changed while the launcher ran is read as it is now)
                    omsi_cfg::content_changed();
                    l.state.config = core::load_config();
                    l.pages.setup_root = None;
                    l.pages.setup_game = None;
                    let same = chosen.is_empty() || std::path::Path::new(&chosen) == std::path::Path::new(&l.state.config.root);
                    if same {
                        l.state.set_status("Saved. Reading the content again…", false);
                    } else {
                        l.state.set_status(format!("Saved, but the OMSI 2 folder used is {}", l.state.config.root), true);
                    }
                    // (a phone: the gallery is kept out of the folder chosen at once, before
                    // its media scanner lists its textures as photos, #1632)
                    #[cfg(target_os = "android")]
                    crate::android::hide_content_from_gallery();
                    l.state.load_content();
                }
                Err(e) => l.state.set_status(format!("{e:#}"), true),
            }
        }
    }
}


// --- tutorials --------------------------------------------------------------------------------

/// OMSI's four tutorials: what each teaches, and a button to start it.
pub fn tutorials(l: &mut Launcher, area: Rect) {
    let body = area;
    static LIST: std::sync::OnceLock<Vec<(usize, String, String)>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(omsi_launcher_lib::tutorials);
    if list.is_empty() {
        l.ui.paragraph("No tutorials were found in the OMSI 2 folder (Tutorials).", Vec2::new(body.x, body.y), body.w, 13.0, Weight::Regular, TEXT_DIM);
        return;
    }
    let cols = 2;
    let cw = (body.w - GAP * 2.0) / cols as f32;
    let ch = ((body.h - GAP * 2.0) / 2.0).min(330.0);
    let mut start = None;
    for (k, (n, title, text)) in list.iter().enumerate() {
        let r = Rect::new(body.x + (k % cols) as f32 * (cw + GAP * 2.0), body.y + (k / cols) as f32 * (ch + GAP * 2.0), cw, ch);
        l.ui.card(r);
        l.ui.text_in(title, Rect::new(r.x + 18.0, r.y + 14.0, r.w - 36.0, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
        l.ui.push_clip(Rect::new(r.x + 18.0, r.y + 46.0, r.w - 36.0, r.h - 110.0), 0.0);
        l.ui.paragraph(text, Vec2::new(r.x + 18.0, r.y + 46.0), r.w - 36.0, 12.5, Weight::Regular, TEXT_DIM);
        l.ui.pop_clip();
        if l.ui.button(&format!("tut-{n}"), Rect::new(r.x + 18.0, r.bottom() - 58.0, 200.0, 40.0), "Start the lesson", Some("play_arrow"), ButtonKind::Primary) {
            start = Some(*n);
        }
    }
    if let Some(n) = start {
        l.state.launch_tutorial(n);
    }
}

#[cfg(test)]
mod wizard_tests {
    fn feedback_wizard() -> super::Wizard {
        super::Wizard {
            step: super::WIZARD_STEPS.len(), rest: [Some(0.0); 8],
            at: vec![[Some(-1.0), None, None, None, None, None, None, None], [None; 8], [None; 8], [None; 8]],
            error: None, calibration: None, ff_choice: None, test_strength: crate::ffb_calibration::PULSE_FORCE,
        }
    }

    fn click_feedback(name: &str, w: &mut super::Wizard, d: &mut crate::controllers::DeviceCfg) -> Option<bool> {
        use super::*;
        let mut ui = Ui::new();
        let size = Vec2::new(900.0, 700.0);
        let area = Rect::new(20.0, 20.0, 700.0, 600.0);
        let mut io = None;
        let mut active = false;
        ui.begin(size, 1.0, 0.016);
        feedback_setup(&mut ui, area, w, d, &[], None, &mut io, &mut active, None, false);
        let rect = ui.drawn[&id_of(name)];
        ui.input.mouse = rect.center();
        ui.input.pressed = true;
        ui.input.down = true;
        ui.begin(size, 1.0, 0.016);
        feedback_setup(&mut ui, area, w, d, &[], None, &mut io, &mut active, None, false);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        ui.begin(size, 1.0, 0.016);
        let done = feedback_setup(&mut ui, area, w, d, &[], None, &mut io, &mut active, None, false);
        assert!(!active);
        assert!(io.is_none());
        done
    }

    #[test]
    fn manual_direction_is_only_applied_on_finish_and_cancel_preserves_the_device() {
        let original = crate::controllers::DeviceCfg { ff_invert: Some(true), ..Default::default() };
        let mut device = original.clone();
        let mut w = feedback_wizard();
        assert_eq!(click_feedback("wiz-ff-manual", &mut w, &mut device), None);
        assert_eq!(w.ff_choice, Some(false));
        assert_eq!(device, original);
        assert_eq!(click_feedback("wiz-cancel", &mut w, &mut device), Some(false));
        assert_eq!(device, original);
        assert_eq!(click_feedback("wiz-ff-finish", &mut w, &mut device), Some(true));
        assert_eq!(device.ff_invert, Some(false));
        assert_eq!(device.axes[0], Some((Func::Steering, false)));
    }

    #[test]
    fn disconnected_wheel_cannot_start_a_hardware_test() {
        let mut device = crate::controllers::DeviceCfg::default();
        let mut w = feedback_wizard();
        assert_eq!(click_feedback("wiz-ff-test", &mut w, &mut device), None);
        assert!(w.error.is_some());
        assert!(w.calibration.is_none());
    }

    #[test]
    fn cancelling_feedback_releases_io_and_invalidates_the_test() {
        let mut pads = super::PadsView::default();
        pads.feedback_test = true;
        pads.wizard = Some(super::Wizard {
            step: super::WIZARD_STEPS.len(), rest: [None; 8], at: Vec::new(), error: None,
            calibration: Some((std::time::Instant::now(), crate::ffb_calibration::Calibration::new(crate::ffb_calibration::PULSE_FORCE))), ff_choice: None, test_strength: crate::ffb_calibration::PULSE_FORCE,
        });
        pads.cancel_feedback_test();
        assert!(!pads.feedback_test);
        assert!(pads.io.is_none());
        let test = &pads.wizard.as_ref().unwrap().calibration.as_ref().unwrap().1;
        assert!(test.result.unwrap().is_err());
        assert_eq!(pads.wizard.as_ref().unwrap().ff_choice, None);
    }

    use crate::controllers::Func;

    #[test]
    fn h_pattern_gears_have_a_clear_name() {
        assert_eq!(super::known_action("kw_s_1_fest").as_deref(), Some("Gear 1 (H-pattern)"));
        assert_eq!(super::known_action("kw_s_R_fest").as_deref(), Some("Gear R (H-pattern)"));
    }

    #[test]
    fn empty_saved_button_slots_do_not_fill_the_controller_list() {
        let mut buttons = vec![(String::new(), "0".to_string()); 131];
        buttons[10].0 = "horn".to_string();
        assert_eq!(super::shown_button_count(&buttons, 18, None), 18);
        assert_eq!(super::shown_button_count(&buttons, 18, Some(128)), 129);
    }

    #[test]
    fn a_wheel_with_three_pedals() {
        // X the wheel; Y throttle, Z brake, Rz clutch - pedals reading 1 up, -1 down (as the
        // G25's run, "reversed")
        let rest = [Some(0.0), Some(1.0), Some(1.0), None, None, Some(1.0), None, None];
        let mut left = rest;
        left[0] = Some(-1.0);
        let mut thr = rest;
        thr[1] = Some(-1.0);
        let mut brk = rest;
        brk[2] = Some(-1.0);
        let mut clu = rest;
        clu[5] = Some(-1.0);
        let a = super::wizard_result(&rest, &[left, thr, brk, clu]);
        assert_eq!(a[0], Some((Func::Steering, false)));
        assert_eq!(a[1], Some((Func::Throttle, true)));
        assert_eq!(a[2], Some((Func::Brake, true)));
        assert_eq!(a[5], Some((Func::Clutch, true)));
    }

    #[test]
    fn pedals_on_one_axis_and_a_wheel_the_other_way() {
        let rest = [Some(0.0), Some(0.0), None, None, None, None, None, None];
        let a = super::wizard_result(&rest, &[[Some(0.9), Some(0.0), None, None, None, None, None, None], [Some(0.0), Some(-1.0), None, None, None, None, None, None], [Some(0.0), Some(1.0), None, None, None, None, None, None], [None; 8]]);
        assert_eq!(a[0], Some((Func::Steering, true)));
        assert_eq!(a[1], Some((Func::ThrottleBrake, true)));
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;

    fn session(time: u64, map: &str, minutes: f64, km: f64, stops: i32, late: i32, tickets: i32) -> core::Session {
        core::Session { time, driver: "Luc".into(), map: map.into(), bus: "Vehicles/SD200/SD200.bus".into(), line: Some("13".into()), tour: Some("2".into()), seconds: minutes * 60.0, metres: km * 1000.0, stops, late, tickets, ..Default::default() }
    }

    fn profile(sessions: Vec<core::Session>) -> core::Profile {
        let record = core::record_of(&sessions, 0);
        core::Profile { name: "Luc".into(), file: "Drivers/Luc.odr".into(), hours: 2.5, km: 52.0, xp: 300, level: 2, next_level_xp: 1000, stops: 40, early: 2, late: 6, tickets: 140.0, cash: 210.0, crashes: 1, hurt: 0, rating_driving: 87.0, rating_comfort: 64.0, rating_tickets: 100.0, sessions, exists: true, record, trips: Vec::new(), trip_totals: Default::default() }
    }

    fn trip(time: u64, line: &str, stops: i32, early: i32, late: i32, average: f64, worst: f64) -> core::TripRun {
        core::TripRun { time, driver: "Luc".into(), line: line.into(), terminus: "Rathaus Spandau".into(), planned: stops, stops, early, late, average: Some(average), worst: Some(worst), completed: true, ..Default::default() }
    }

    fn with_trips(trips: Vec<core::TripRun>) -> core::Profile {
        core::Profile { trip_totals: core::trip_totals(&trips), trips, ..profile(Vec::new()) }
    }

    /// The trips: what they come to (the share of their stops on time, the average and the
    /// worst delay) and the last ones with theirs; a free drive is listed without them.
    #[test]
    fn the_record_shows_the_trips() {
        assert!(figures(&profile(Vec::new()), &|m| m.to_string(), &|b| b.to_string(), 0).trips.is_none(), "no trips, no section");
        let p = with_trips(vec![trip(1_000_000_300, "5", 10, 1, 1, 70.0, 250.0), trip(1_000_000_200, "5", 10, 0, 0, -10.0, -60.0), core::TripRun { time: 1_000_000_100, free: true, stops: 6, line: "13".into(), ..Default::default() }]);
        let t = figures(&p, &|m| m.to_string(), &|b| b.to_string(), 0).trips.expect("trips");
        assert_eq!(t.figures.iter().map(|x| (x.0.as_str(), x.2)).collect::<Vec<_>>(), [("3", "Trips"), ("90 %", "On time"), ("+0:30", "Average delay"), ("+4:10", "Worst delay")]);
        assert_eq!(t.last.len(), 3);
        assert_eq!((t.last[0].line.as_deref(), t.last[0].on_time, t.last[0].average.as_deref()), (Some("5"), Some((8, 10)), Some("+1:10")));
        assert_eq!(t.last[0].worst.as_ref().unwrap().0, "+4:10");
        assert_eq!(t.last[0].when, "9 Sep 2001, 01:51");
        // (the free drive: no punctuality)
        assert_eq!((t.last[2].free, t.last[2].on_time, t.last[2].worst.is_none()), (true, None, true));
        // drawn on a phone and a wide window
        let f = figures(&p, &|m| m.to_string(), &|b| b.to_string(), 0);
        let mut ui = Ui::new();
        ui.begin(Vec2::new(1400.0, 900.0), 1.0, 1.0 / 60.0);
        let without = record_body(&mut ui, Rect::new(0.0, 0.0, 1300.0, 700.0), &figures(&profile(Vec::new()), &|m| m.to_string(), &|b| b.to_string(), 0));
        let wide = record_body(&mut ui, Rect::new(0.0, 0.0, 1300.0, 700.0), &f);
        let narrow = trips_section(&mut ui, Rect::new(0.0, 0.0, 360.0, 0.0), f.trips.as_ref().unwrap());
        assert!(wide > without + 150.0, "{wide} against {without}");
        // (two figures to a row on a phone)
        assert!(narrow > trips_section(&mut ui, Rect::new(0.0, 0.0, 1300.0, 0.0), f.trips.as_ref().unwrap()));
    }

    #[test]
    fn the_record_says_what_the_file_and_the_runs_hold() {
        let p = profile(vec![session(1_000_000_000, "maps/Spandau/global.cfg", 90.0, 40.0, 30, 3, 120), session(1_000_100_000, "maps/Grundorf/global.cfg", 60.0, 12.0, 10, 0, 20)]);
        let f = figures(&p, &|m| short_map(m), &|b| b.rsplit('/').next().unwrap_or(b).to_string(), 0);
        assert_eq!(f.tiles.len(), 6);
        assert_eq!(f.tiles[0].value, "2");
        assert_eq!(f.tiles[1].value, "2h 30m");
        // a hundred and forty tickets fill a bus and a half ("enough to fill" is not said
        // under a hundred)
        assert!(f.tiles[4].note.contains('1'));
        assert_eq!(f.tiles[5].note, "84.00 per hour");
        assert_eq!(f.stops, [2, 32, 6]);
        assert_eq!(f.records.iter().map(|x| x.label).collect::<Vec<_>>(), ["Longest duty", "Furthest", "Busiest duty", "Closest to the timetable"]);
        assert_eq!(f.records[0].note, "Spandau");
        assert_eq!(f.records[3].value, "10 of 10 stops on time");
        assert_eq!(f.maps[0], ("Spandau".to_string(), 1));
        assert_eq!(f.buses[0], ("SD200.bus".to_string(), 2));
        assert_eq!(f.runs.len(), 2);
        assert_eq!(f.runs[0].line, "13 / 2");
        assert_eq!(f.runs[0].on_time, Some((27, 30)));
        assert_eq!(f.since, "Driving since 9 September 2001");
        // the ratings are bars, the counts are not
        assert_eq!(f.style.iter().filter(|x| x.bar.is_some()).count(), 3);
    }

    #[test]
    fn no_runs_no_records() {
        let p = profile(Vec::new());
        let f = figures(&p, &|m| m.to_string(), &|b| b.to_string(), 0);
        assert!(f.records.is_empty() && f.runs.is_empty() && f.maps.is_empty());
        assert_eq!(f.since, "");
        // drawn, the page says there is nothing yet instead of showing empty boxes
        let mut ui = Ui::new();
        ui.begin(Vec2::new(1200.0, 900.0), 1.0, 1.0 / 60.0);
        let h = record_body(&mut ui, Rect::new(0.0, 0.0, 1000.0, 800.0), &f);
        assert!(h > 100.0);
    }

    #[test]
    fn the_record_fits_a_phone_and_a_wide_window() {
        let p = profile((0..12).map(|k| session(1_000_000_000 + k * 3600, "maps/Spandau/global.cfg", 30.0, 10.0, 8, 1, 5)).collect());
        let f = figures(&p, &|m| short_map(m), &|b| b.to_string(), 3600);
        let mut ui = Ui::new();
        ui.begin(Vec2::new(1400.0, 900.0), 1.0, 1.0 / 60.0);
        let wide = record_body(&mut ui, Rect::new(0.0, 0.0, 1300.0, 700.0), &f);
        let narrow = record_body(&mut ui, Rect::new(0.0, 0.0, 360.0, 700.0), &f);
        // (one column on a phone: taller, nothing laid beside)
        assert!(narrow > wide, "{narrow} against {wide}");
        assert_eq!(f.runs.len(), 8, "the last eight runs, not all");
    }

    #[test]
    fn times_and_levels() {
        assert_eq!(local_time(0, 0), (1970, 1, 1, 0, 0));
        assert_eq!(local_time(1_000_000_000, 7200), (2001, 9, 9, 3, 46));
        assert_eq!(local_time(951_782_400, 0), (2000, 2, 29, 0, 0));
        assert_eq!(duration_text(129.0 + 2.0 / 60.0), "129h 02m");
        assert_eq!(level_progress(250, 2, 1000), 0.0);
        assert_eq!(level_progress(625, 2, 1000), 0.5);
        assert_eq!(level_progress(0, 1, 250), 0.0);
    }

    /// The new driver's tile: Enter makes them once a name is typed, Escape gives up.
    #[test]
    fn a_new_driver_is_made_with_enter_and_left_with_escape() {
        let tile = Rect::new(100.0, 100.0, 260.0, 160.0);
        let mut ui = Ui::new();
        let mut name = String::new();
        ui.focus = Some(id_of(NEW_NAME));
        ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
        ui.input.keys.push(Key::Enter);
        assert_eq!(new_driver_tile(&mut ui, tile, &mut name), None, "no name, nothing made");
        ui.finish();
        ui.focus = Some(id_of(NEW_NAME));
        ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
        ui.input.text.push_str("Moffel");
        assert_eq!(new_driver_tile(&mut ui, tile, &mut name), None);
        assert_eq!(name, "Moffel");
        ui.finish();
        ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
        ui.input.keys.push(Key::Enter);
        assert_eq!(new_driver_tile(&mut ui, tile, &mut name), Some(true));
        ui.finish();
        ui.focus = Some(id_of(NEW_NAME));
        ui.begin(Vec2::new(800.0, 600.0), 1.0, 1.0 / 60.0);
        ui.input.keys.push(Key::Escape);
        assert_eq!(new_driver_tile(&mut ui, tile, &mut name), Some(false));
    }

    #[test]
    fn every_page_says_what_it_is_for() {
        for (page, _, _) in super::super::PAGES {
            let (icon, sub) = about(page, 0);
            assert!(!icon.is_empty());
            assert!(page == Page::Drive || !sub.is_empty(), "{page:?}");
        }
        assert_ne!(about(Page::Controls, 0).1, about(Page::Controls, 1).1);
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::updater::Status;

    /// Every clickable thing of the settings page by the tab it is on (switches are named
    /// `set-<key>`). Taken from the page as it was before the tabs: nothing may go missing.
    fn by_tab() -> Vec<Vec<&'static str>> {
        let mut graphics = vec![
            "s-gp-sel", "s-gp-load", "s-gp-del", "s-gp-name", "s-gp-save",
            "s-preset", "s-graphics", "s-msaa", "s-scale", "s-af", "s-shadow", "set-ssao", "set-shadows", "s-casters", "set-detail_textures", "s-led", "s-led-mip", "set-shadow_blobs", "set-reflections", "set-clouds", "set-windy_trees",
            "set-fullscreen", "s-res", "set-vsync", "s-fps", "s-view", "s-maxobj", "s-minobj", "s-mirror", "s-mirror-refresh", "s-texmem", "set-texture_compression",
        ];
        if !cfg!(target_os = "macos") {
            graphics.push("s-api");
        }
        let driving = vec![
            "s-keys", "set-steering_linear", "set-old_steering", "set-red_steer_spd", "s-mouse", "set-mouse_smooth", "set-mouse_right_off", "set-blinker_cancel", "set-brake_hold", "set-auto_clutch", "set-momentary_gears", "set-ibis_auto", "s-go-keys",
            "s-wrange", "s-wlock", "s-pad-steer-smooth", "set-pad_steer_linear", "set-arrows_switch_cams", "s-pedt", "s-pedb", "set-ff_enabled", "set-ff_invert", "s-ffroad", "s-ffeng", "s-fffade", "s-wreset", "s-go-pads",
        ];
        let mut camera = vec![
            "s-seaty",
            "s-seatz",
            "s-seatx",
            "s-seat-pitch",
            "s-seatreset",
            "s-fov",
            "s-look-sens",
            "set-right_stick_look",
            "s-look-smoothing",
            "s-head-idle",
            "s-head-idle-pace",
            "set-steer_look",
            "s-steer-look-angle",
            "s-steer-look-response",
            "set-head_movement",
            "set-driverview_smooth",
            "set-hands_in_cab",
            "set-alt_view",
            "set-precision_zoom",
            "set-camera_collision",
            "set-driver",
            "set-head_tracking",
            "set-triple_screen",
            "set-triple_span",
            "set-triple_hud_center",
            "s-triple-width_mm",
            "s-triple-distance_mm",
            "s-triple-bezel_mm",
            "s-triple-left_angle_deg",
            "s-triple-right_angle_deg",
            "s-triple-eye_height_mm",
        ];
        if cfg!(windows) {
            camera.extend(["set-vr", "s-vr-scale", "s-vr-head-smoothing", "s-vr-mirror-rate", "set-vr_desktop_mirror", "s-go-vr-keys"]);
        }
        // (the radio stations: one, see `frame`)
        let sound = vec!["s-vol", "s-volai", "s-volsc", "set-doppler", "s-voices", "radio-name-0", "radio-url-0", "radio-del-0", "radio-add"];
        let gameplay = vec![
            "s-board", "set-exact_fare", "s-pax", "set-get_up", "s-unsched", "s-maxsched", "s-maxpark",
            "s-maint", "set-collision_vehicles", "set-collision_objects", "set-collision_pedestrians", "set-use_real_time", "set-use_real_date", "set-time_sync", "set-metar_sync", "s-timespeed",
        ];
        let general = vec![
            "s-lang", "set-machine_translation", "set-launcher_rest", "set-discord_status", "set-voice_chat", "set-companion", "s-launcher-scale", "set-animations", "set-page_bus", "s-accent-sw0", "s-accent-sw1", "s-accent-sw2", "s-accent-sw3", "s-accent-sw4", "s-accent-sw5", "s-accent-sw6", "s-accent-sw7", "s-accent-custom", "s-welcome-again", "s-tour-start", "s-uiscale", "set-ui_scale_window", "s-uiop", "set-tooltips", "set-show_fps", "set-notes", "set-chat", "s-chatsize", "set-name_tags",
            "set-navigator", "set-nav_arrows", "set-nav_ai", "set-nav_board", "s-nav-scale", "s-stop-style", "set-nav_signon", "corner-top-left", "corner-top-right", "corner-bottom-left", "corner-bottom-right", "s-nav-reset",
            "set-update_check", "set-update_auto", "set-update_notify", "set-presence", "s-upd-check", "s-upd-github", "s-reset",
        ];
        vec![graphics, driving, camera, sound, gameplay, general]
    }

    /// Settings that show every row: Enhanced (Vanilla hides the shadows and effects), VR on.
    fn all_rows() -> Value {
        let mut s = core::settings_from_text(None);
        s["triple_screen"] = json!(true);
        s["graphics"] = json!("enhanced");
        s["vr"] = json!(true);
        s
    }

    fn outside() -> Outside {
        Outside { update: Status::Idle, check_updates: false, reset: false, controls: None, again: None }
    }

    /// One frame of tab `tab`, its two columns tall enough that nothing is cut off. The Sound
    /// tab lists one radio station (not the radio.cfg of whoever runs the tests).
    fn frame(ui: &mut Ui, tab: usize, s: &mut Value, out: &mut Outside) {
        RADIO.with(|r| {
            r.borrow_mut().get_or_insert_with(|| vec![("One".into(), "https://example.org/one.mp3".into())]);
        });
        ui.begin(Vec2::new(1200.0, 2000.0), 1.0, 1.0 / 60.0);
        let mut dirty = 0.0;
        settings_tab(ui, tab, s, &mut dirty, out, [Rect::new(0.0, 0.0, 580.0, 2000.0), Rect::new(620.0, 0.0, 580.0, 2000.0)]);
    }

    /// Click the widget `name` on tab `tab`: the mouse goes down over it and comes up again.
    fn click(tab: usize, name: &str, s: &mut Value) -> Outside {
        let mut ui = Ui::new();
        let mut out = outside();
        frame(&mut ui, tab, s, &mut out);
        let r = *ui.drawn.get(&id_of(name)).unwrap_or_else(|| panic!("{name} is not on the {} tab", SETTINGS_TABS[tab]));
        ui.input.mouse = r.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, tab, s, &mut out);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, tab, s, &mut out);
        out
    }

    #[test]
    fn every_setting_is_on_exactly_one_tab() {
        let tabs = by_tab();
        assert_eq!(tabs.len(), SETTINGS_TABS.len());
        let mut seen = std::collections::HashSet::new();
        for name in tabs.iter().flatten() {
            assert!(seen.insert(*name), "{name} is listed on two tabs");
        }
        for (tab, names) in tabs.iter().enumerate() {
            let mut ui = Ui::new();
            frame(&mut ui, tab, &mut all_rows(), &mut outside());
            for name in names {
                assert!(ui.drawn.contains_key(&id_of(name)), "{name} is not on the {} tab", SETTINGS_TABS[tab]);
            }
            assert_eq!(ui.drawn.len(), names.len(), "the {} tab has a clickable thing more than the list names", SETTINGS_TABS[tab]);
        }
    }

    #[test]
    fn right_stick_look_switch_toggles_and_saves_from_the_camera_tab() {
        let mut s = all_rows();
        assert_eq!(s["right_stick_look"], json!(true));

        click(2, "set-right_stick_look", &mut s);
        assert_eq!(s["right_stick_look"], json!(false));
        let saved = core::settings_to_text(&s, None);
        assert_eq!(
            core::settings_from_text(Some(&saved))["right_stick_look"],
            json!(false)
        );

        click(2, "set-right_stick_look", &mut s);
        assert_eq!(s["right_stick_look"], json!(true));
    }

    #[test]
    fn the_driving_tab_leads_to_the_keys_and_the_controllers() {
        let mut s = all_rows();
        assert_eq!(click(1, "s-go-keys", &mut s).controls, Some(0));
        assert_eq!(click(1, "s-go-pads", &mut s).controls, Some(1));
    }

    /// The mouse steering's smoothing is on unless switched off, and the switch is kept (#1092).
    #[test]
    fn smooth_mouse_steering_switches_off_and_is_saved() {
        let mut s = all_rows();
        assert_eq!(s["mouse_smooth"], json!(true));
        click(1, "set-mouse_smooth", &mut s);
        assert_eq!(s["mouse_smooth"], json!(false));
        let saved = core::settings_to_text(&s, None);
        assert!(saved.contains("mouse_smooth=0\n"), "{saved}");
        assert_eq!(core::settings_from_text(Some(&saved))["mouse_smooth"], json!(false));
        assert!(!crate::settings::Settings::from_text(&saved).mouse_smooth);
        assert!(crate::settings::Settings::from_text("").mouse_smooth);
    }

    #[test]
    fn the_general_tab_shows_the_welcome_and_the_tour_again() {
        let mut s = all_rows();
        let before = s.clone();
        assert_eq!(click(5, "s-welcome-again", &mut s).again, Some(Again::Welcome));
        assert_eq!(click(5, "s-tour-start", &mut s).again, Some(Again::Tour));
        assert_eq!(s, before, "the page writes welcome_done itself, once it shows the welcome");
    }

    #[test]
    fn reset_asks_first_and_changes_nothing() {
        let mut s = all_rows();
        let before = s.clone();
        let out = click(5, "s-reset", &mut s);
        assert!(out.reset);
        assert_eq!(s, before);
    }
}

#[cfg(test)]
mod pad_action_tests {
    /// Every game action a button can be given is one the game carries out from a
    /// controller (app_events: `view_look_*` / `voice_radio` while held, the gears by name,
    /// the rest through `is_game_action`, the doors through `Player::action`) - Space's
    /// reset of every view among them (#1167).
    #[test]
    fn a_button_can_reset_every_view() {
        assert!(super::PAD_GAME_ACTIONS.contains(&"view_reset_all_directions"));
        assert!(super::PAD_GAME_ACTIONS.contains(&"voice_radio"));
        assert_eq!(super::known_action("voice_radio").as_deref(), Some("Multiplayer: bus radio (hold)"));
        for a in super::PAD_GAME_ACTIONS {
            let held = a.starts_with("view_look_") || a == "voice_radio";
            let handled = held
                || crate::input_script::is_game_action(a)
                || a.starts_with("gear_")
                || crate::player::door_action(a).is_some();
            assert!(handled, "{a}");
        }
        // held pad actions are game actions so they never reach the bus script
        assert!(crate::input_script::is_game_action("voice_radio"));
        assert!(crate::input_script::is_game_action("view_look_left"));
    }
}

#[cfg(test)]
mod pad_remove_tests {
    use crate::controllers::{cfg_text, parse_cfg};

    /// A device taken out of the list is gone from the file Save writes, and the next one is
    /// shown - the one below it, or above it when it was the last (#636).
    #[test]
    fn a_removed_device_leaves_the_file() {
        let mut devices = parse_cfg("[ctrl]\r\nSideWinder Joystick\r\n0\r\n\r\n[ctrl]\r\nLogitech G25 Racing Wheel USB\r\n1\r\n\r\n[ctrl]\r\nMOZA R3 Base\r\n0\r\n");
        let mut sel = 1;
        assert_eq!(super::remove_device(&mut devices, &mut sel), "Logitech G25 Racing Wheel USB");
        assert_eq!(sel, 1);
        let names = |text: &str| parse_cfg(text).into_iter().map(|d| d.name).collect::<Vec<_>>();
        assert_eq!(names(&cfg_text(&devices)), ["SideWinder Joystick", "MOZA R3 Base"]);
        assert_eq!(super::remove_device(&mut devices, &mut sel), "MOZA R3 Base");
        assert_eq!(sel, 0);
        assert_eq!(super::remove_device(&mut devices, &mut sel), "SideWinder Joystick");
        assert_eq!(sel, 0);
        assert!(names(&cfg_text(&devices)).is_empty());
    }
}
