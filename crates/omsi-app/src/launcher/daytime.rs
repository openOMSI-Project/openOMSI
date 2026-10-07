//! The day and the weather - the setup's "Day & weather" step, for every way to drive - laid
//! out as Omsi-Hub lays out a sheet: each choice under its name in small capitals, with what
//! is chosen on the right; few options as chips, on and off as switches, many as a list. And
//! everything openOMSI sets for the day stays on it: the time and the date, the season, how
//! many cars drive, the passengers and the timetable's buses, the bus in service or the driver
//! on foot at the start, and the weather - the map's, a preset, OMSI's current weather of an
//! airport (METAR), the weather cycle, or one's own.
//!
//! The sheet scrolls as one, so its body only gets the `Ui` and the choice (`day_body`): what
//! the rest of the launcher must do about a change - read the day's timetable again, save the
//! choice - comes back in `DayOut`.

use super::drive::{custom_weather_summary, joined_server_name, nearest_airport, selected_weather_as_custom};
use super::state::Choice;
use super::theme::*;
use super::ui::{id_of, parse_date, weekday, ButtonKind, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// The seasons as the choice and `--season` name them, and as the chips say them.
const SEASONS: [&str; 5] = ["auto", "spring", "summer", "autumn", "winter"];
const SEASON_NAMES: [&str; 5] = ["By date", "Spring", "Summer", "Autumn", "Winter"];
pub(super) const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// A weather to choose: the value of `Choice::weather`, its name, what it is, its icon, and
/// whether it came new.
#[derive(Clone, Debug)]
pub struct WeatherItem {
    pub value: String,
    pub name: String,
    pub meta: String,
    pub icon: &'static str,
    pub fresh: bool,
}

/// What the sheet shows besides the choice.
#[derive(Default)]
pub struct DayCtx {
    pub items: Vec<WeatherItem>,
    /// OMSI's METAR airports (code, name), when the current weather is chosen.
    pub airports: Vec<(String, String)>,
    /// The chosen preset as one's own weather, when it can be edited so.
    pub as_custom: Option<String>,
    /// The season the date falls in (what "By date" means now).
    pub season_of_date: &'static str,
    /// A duty of the timetable: the minutes its bus stands ready before the first departure
    /// (None: a free drive, which starts when it is asked to).
    pub lead: Option<i64>,
}

/// What a frame of the sheet changed.
#[derive(Default, Debug, PartialEq)]
pub struct DayOut {
    pub touched: bool,
    /// The bus's lead before the first departure chosen (minutes).
    pub lead: Option<i64>,
    /// The date moved: the day's timetable is another.
    pub date_changed: bool,
    /// The season moved: a preset of the other season goes.
    pub season_changed: bool,
}

impl DayCtx {
    fn of(l: &Launcher) -> DayCtx {
        let c = &l.state.choice;
        let custom_now = crate::weather_setup::custom_weather(Some(&c.weather));
        let custom_meta = custom_now.as_ref().map(custom_weather_summary).unwrap_or_else(|| "Set visibility, wind, clouds, rain, temperature and road state".into());
        let mut items = vec![
            WeatherItem { value: String::new(), name: "Map default".into(), meta: "Whatever the map starts with".into(), icon: "wb_sunny", fresh: false },
            WeatherItem { value: custom_now.unwrap_or_default().encode(), name: "Custom weather".into(), meta: custom_meta, icon: "tune", fresh: false },
        ];
        // OMSI 2's current weather: an airport's METAR report, fetched when the game starts
        // (the airport nearest the map, not Berlin's for every map)
        let metar = c.weather.strip_prefix("metar:").map(str::to_string);
        let code = metar.clone().unwrap_or_else(|| nearest_airport(&l.state.config.root, &c.map));
        items.push(WeatherItem { value: format!("metar:{code}"), name: "Current weather".into(), meta: omsi_ui::tr("METAR of %{code} (fetched at the start)").replace("%{code}", &code), icon: "public", fresh: false });
        items.push(WeatherItem { value: "cycle".into(), name: "Weather cycle".into(), meta: "Changes every 25-60 minutes, as the month allows".into(), icon: "autorenew", fresh: false });
        for w in &l.state.weathers {
            if !l.state.weather_fits(w) {
                continue;
            }
            let vis = if w.fog_m >= 20000.0 { omsi_ui::tr("clear air").into_owned() } else { format!("{:.0} m", w.fog_m) };
            let clouds = w.clouds.to_lowercase();
            let icon = if w.snow || w.precip.starts_with("snow") {
                "weather_snowy"
            } else if w.precip.starts_with("rain") {
                "rainy"
            } else if w.fog_m < 1500.0 {
                "foggy"
            } else if clouds.contains("overcast") {
                "cloud"
            } else if clouds.contains("cumulus") {
                "partly_cloudy_day"
            } else {
                "wb_sunny"
            };
            items.push(WeatherItem { value: w.file.clone(), name: w.name.clone(), meta: format!("{:.0} °C · {} · {vis}", w.temp, w.precip), icon, fresh: l.state.fresh.contains_key(&w.file) });
        }
        let airports = if metar.is_some() { crate::weather_setup::metar_airports(std::path::Path::new(&l.state.config.root)) } else { Vec::new() };
        let (_, m, _) = parse_date(&c.date);
        let lead = (!c.free).then(|| omsi_launcher_lib::bus_lead(l.state.settings.get("bus_lead").and_then(|x| x.as_i64())));
        DayCtx { items, airports, as_custom: selected_weather_as_custom(&l.state.config.root, &c.weather), season_of_date: season_of_month(m), lead }
    }
}

fn season_of_month(m: u32) -> &'static str {
    match m {
        12 | 1 | 2 => "Winter",
        3..=5 => "Spring",
        6..=8 => "Summer",
        _ => "Autumn",
    }
}

/// Chips in a row, on to the next row where the width is used up, as Omsi-Hub's: the chosen
/// one filled. Returns the chip clicked and the rows' height.
pub(super) fn chips(ui: &mut Ui, name: &str, x: f32, y: f32, w: f32, labels: &[String], selected: usize) -> (Option<usize>, f32) {
    let (h, gap) = (30.0, 8.0);
    let (mut cx, mut cy) = (x, y);
    let mut clicked = None;
    for (k, label) in labels.iter().enumerate() {
        let on = k == selected;
        let cw = (ui.width(label, 12.5, Weight::Bold) + 28.0).min(w);
        if cx > x && cx + cw > x + w {
            cx = x;
            cy += h + gap;
        }
        let r = Rect::new(cx, cy, cw, h);
        let (hover, _, click) = ui.interact(id_of(&format!("{name}-{k}")), r);
        if on {
            ui.p().rounded(r, h * 0.5, accent());
        } else {
            ui.p().rounded(r, h * 0.5, if hover { HOVER } else { Color::CLEAR });
            ui.p().rounded_border(r, h * 0.5, 1.0, Color::WHITE.alpha(if hover { 0.24 } else { 0.13 }));
        }
        ui.text_in(label, r, 12.5, if on { Weight::Bold } else { Weight::Medium }, if on { on_accent() } else if hover { TEXT } else { TEXT_SOFT }, Align::Center);
        if click {
            clicked = Some(k);
        }
        cx += cw + gap;
    }
    (clicked, cy + h - y)
}

/// A section's head: its name in small capitals, and what is chosen on the right.
pub(super) fn section(ui: &mut Ui, x: f32, y: f32, w: f32, title: &str, value: &str) {
    ui.text_in(&omsi_ui::tr(title).to_uppercase(), Rect::new(x, y, w * 0.6, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
    if !value.is_empty() {
        ui.text_in(value, Rect::new(x + w * 0.35, y - 1.0, w * 0.65, 18.0), 13.0, Weight::Bold, TEXT, Align::Right);
    }
}

/// A switch on a row of its own, a hairline under it.
fn switch(ui: &mut Ui, name: &str, x: f32, y: f32, w: f32, value: &mut bool, label: &str) -> bool {
    let changed = ui.toggle(name, Rect::new(x, y, w, 36.0), value, label);
    ui.p().rect(Rect::new(x, y + 36.5, w, 1.0), HAIRLINE);
    changed
}

/// The Day & weather sheet under its head (`flow` draws the sheet, the head and the actions).
pub(super) fn day_panel(l: &mut Launcher, s: Rect, body: Rect) {
    super::tour::anchor("day-sheet", s);
    let foot = day_foot(l);
    let rest = super::flow::sheet_foot(l, Rect::new(s.x, body.y, s.w, s.bottom() - body.y), &foot);
    let area = Rect::new(body.x + 18.0, body.y, body.w - 24.0, (rest.bottom() - body.y - 8.0).max(60.0));
    if let Some(name) = joined_server_name(l) {
        server_body(l, area, &name);
        return;
    }
    let ctx = DayCtx::of(l);
    let mut out = DayOut::default();
    let choice = &mut l.state.choice;
    l.ui.scroll_area("day-sheet", area, &mut |ui, v| day_body(ui, v, choice, &ctx, &mut out));
    if out.date_changed {
        l.state.load_lines();
    }
    if out.season_changed {
        // (no summer shower in the snow: a preset of the season left goes)
        let w = l.state.choice.weather.clone();
        if l.state.weathers.iter().find(|x| x.file == w).is_some_and(|x| !l.state.weather_fits(x)) {
            l.state.choice.weather.clear();
        }
    }
    if let Some(m) = out.lead {
        l.state.settings["bus_lead"] = serde_json::json!(m);
        l.state.settings_dirty = 0.3;
    }
    if out.touched {
        l.state.touched();
    }
}

/// The leads offered for a duty's bus before its first departure (minutes).
const LEADS: [i64; 7] = [0, 2, 5, 10, 15, 20, 30];

/// The computer's clock: its date (`YYYY-MM-DD`) and the minutes since midnight - the
/// "Current time" and "Current date" of the day's sheet and the time step.
pub(super) fn clock_now() -> Option<(String, i32)> {
    omsi_launcher_lib::local_now().map(|(y, m, d, h, mi)| (format!("{y:04}-{m:02}-{d:02}"), h * 60 + mi))
}

/// The day and the weather chosen, in one line under the sheet's title.
pub(super) fn day_line(l: &Launcher) -> String {
    // (on a server: its clock and its weather)
    if joined_server_name(l).is_some() {
        return super::drive::start_line(l);
    }
    let c = &l.state.choice;
    let (y, m, d) = parse_date(&c.date);
    let weather = match c.weather.as_str() {
        "" => omsi_ui::tr("As on the map").into_owned(),
        "cycle" => omsi_ui::tr("Weather cycle").into_owned(),
        w if w.starts_with("metar:") => omsi_ui::tr("Current weather at %{code}").replace("%{code}", &w["metar:".len()..]),
        w if crate::weather_setup::custom_weather(Some(w)).is_some() => omsi_ui::tr("Custom weather").into_owned(),
        w => l.state.weathers.iter().find(|x| x.file == w).map(|x| x.name.clone()).unwrap_or_else(|| w.to_string()),
    };
    let day = omsi_ui::tr(WEEKDAYS[weekday(y, m, d).clamp(0, 6) as usize]);
    let month = omsi_ui::tr(super::ui::MONTHS_LONG[(m as usize).clamp(1, 12) - 1]);
    format!("{:02}:{:02} · {day} {d} {month} {y} · {weather}", c.time / 60, c.time % 60)
}

/// The day under the sheet: its weekday, and how many lines run on it.
fn day_foot(l: &Launcher) -> String {
    let (y, m, d) = parse_date(&l.state.choice.date);
    let day = omsi_ui::tr(WEEKDAYS[weekday(y, m, d).clamp(0, 6) as usize]).into_owned();
    let read = l.state.lines_for.1 == l.state.choice.date && !l.state.loading_lines;
    if !read {
        return format!("{day} · {}", omsi_ui::tr("Reading the timetable…"));
    }
    let n = l.state.lines.iter().filter(|x| x.tours.iter().any(|t| t.runs)).count();
    format!("{day} · {}", omsi_ui::tr(if n == 1 { "1 line runs on this day" } else { "%{n} lines run on this day" }).replace("%{n}", &n.to_string()))
}

/// On a server: its clock and its weather, nothing to choose.
fn server_body(l: &mut Launcher, r: Rect, name: &str) {
    let info = l.state.joined_server.as_ref().and_then(|a| l.state.server_info.get(a)).and_then(|x| x.1.as_ref().ok()).cloned();
    let mut y = r.y + 4.0;
    l.ui.icon("lock", Vec2::new(r.x + 7.0, y + 8.0), 14.0, TEXT_DIM);
    section(&mut l.ui, r.x + 20.0, y, r.w - 20.0, &omsi_ui::tr("Set by %{name}").replace("%{name}", name), "");
    y += 30.0;
    let time = info.as_ref().map(|i| i.time.clone()).unwrap_or_default();
    let weather = info.as_ref().map(|i| if i.weather.is_empty() { omsi_ui::tr("the map's").into_owned() } else { i.weather.clone() }).unwrap_or_default();
    for (k, v) in [("Time", time), ("Weather", weather)] {
        l.ui.text_in(k, Rect::new(r.x, y, 110.0, 30.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
        l.ui.text_in(&v, Rect::new(r.x + 110.0, y, r.w - 110.0, 30.0), 13.5, Weight::Bold, TEXT, Align::Left);
        l.ui.p().rect(Rect::new(r.x, y + 30.5, r.w, 1.0), HAIRLINE);
        y += 34.0;
    }
    l.ui.paragraph("On a server the map, the time, the date and the weather are the same for everybody: the server keeps the world's clock. You choose your bus and your duty.", Vec2::new(r.x, y + 12.0), r.w, 12.5, Weight::Regular, TEXT_DIM);
}

/// The sheet's choices from `v`'s top (its width less the scrollbar's): when, the season,
/// the traffic, the start, and the weather. Returns their height.
pub(super) fn day_body(ui: &mut Ui, v: Rect, c: &mut Choice, ctx: &DayCtx, out: &mut DayOut) -> f32 {
    let x = v.x;
    let w = v.w - 14.0;
    let mut y = v.y + 2.0;
    // when: the time and the date, the weekday on the right
    let (yy, mm, dd) = parse_date(&c.date);
    section(ui, x, y, w, "When", &omsi_ui::tr(WEEKDAYS[weekday(yy, mm, dd).clamp(0, 6) as usize]));
    y += 24.0;
    let half = (w - 10.0) * 0.5;
    let mut t = c.time;
    if ui.time_field("time", Rect::new(x, y, half, 44.0), &mut t) {
        c.time = t;
        out.touched = true;
    }
    let mut d = c.date.clone();
    if ui.date_field("date", Rect::new(x + half + 10.0, y, half, 44.0), &mut d) {
        c.date = d;
        c.season = "auto".into();
        out.date_changed = true;
        out.season_changed = true;
        out.touched = true;
    }
    y += 44.0 + 10.0;
    // (Luc: the moment it is now, at a click - the computer's time, its date)
    if ui.button("time-now", Rect::new(x, y, half, 38.0), "Current time", None, ButtonKind::Normal) {
        if let Some((_, t)) = clock_now() {
            c.time = t;
            out.touched = true;
        }
    }
    if ui.button("date-now", Rect::new(x + half + 10.0, y, half, 38.0), "Current date", None, ButtonKind::Normal) {
        if let Some((d, _)) = clock_now().filter(|(d, _)| *d != c.date) {
            c.date = d;
            c.season = "auto".into();
            out.date_changed = true;
            out.season_changed = true;
            out.touched = true;
        }
    }
    y += 38.0 + 24.0;
    // the bus ready before the first departure (a duty of the timetable: a free drive starts
    // when it is asked to)
    if let Some(lead) = ctx.lead {
        let t = c.time - lead as i32;
        let start = format!("{:02}:{:02}", t.rem_euclid(1440) / 60, t.rem_euclid(1440) % 60);
        let say = if lead == 0 { omsi_ui::tr("at the departure").into_owned() } else { omsi_ui::tr("the game starts at %{time}").replace("%{time}", &start) };
        section(ui, x, y, w, "Bus ready before departure", &say);
        y += 24.0;
        let labels: Vec<String> = LEADS.iter().map(|m| if *m == 0 { omsi_ui::tr("None").into_owned() } else { format!("{m} min") }).collect();
        let at = LEADS.iter().position(|m| *m == lead).unwrap_or(0);
        let (pick, h) = chips(ui, "day-lead", x, y, w, &labels, at);
        if let Some(k) = pick.filter(|k| *k != at) {
            out.lead = Some(LEADS[k]);
        }
        y += h + 24.0;
    }
    // the season: by the date, or one of the four (the date moves into it, so the
    // timetable is the season's too)
    let s = SEASONS.iter().position(|x| *x == c.season).unwrap_or(0);
    section(ui, x, y, w, "Season", if s == 0 { ctx.season_of_date } else { "" });
    y += 24.0;
    let labels: Vec<String> = SEASON_NAMES.iter().map(|x| x.to_string()).collect();
    let (pick, h) = chips(ui, "day-season", x, y, w, &labels, s);
    if let Some(k) = pick.filter(|k| *k != s) {
        c.season = SEASONS[k].to_string();
        if k > 0 {
            let month = ["", "04", "07", "10", "01"][k];
            let (year, day) = (c.date.get(0..4).unwrap_or("1989").to_string(), c.date.get(8..10).unwrap_or("15").to_string());
            c.date = format!("{year}-{month}-{day}");
            out.date_changed = true;
        }
        out.season_changed = true;
        out.touched = true;
    }
    y += h + 24.0;
    // the traffic: the cars, the people at the stops, the timetable's buses
    section(ui, x, y, w, "Traffic", "");
    y += 22.0;
    let mut traffic = c.traffic;
    if ui.slider("traffic", Rect::new(x, y, w, 34.0), &mut traffic, 0.0, 120.0, 1.0, "Cars around", &|v| format!("{v:.0}")) {
        c.traffic = traffic;
        out.touched = true;
    }
    y += 38.0;
    ui.p().rect(Rect::new(x, y - 1.5, w, 1.0), HAIRLINE);
    let mut b = c.passengers;
    if switch(ui, "pax", x, y, w, &mut b, "Passengers") {
        c.passengers = b;
        out.touched = true;
    }
    y += 38.0;
    let mut b = c.schedule;
    if switch(ui, "sched", x, y, w, &mut b, "Timetable buses") {
        c.schedule = b;
        out.touched = true;
    }
    y += 38.0 + 24.0;
    // the start: the bus ready to go, or the driver on foot beside it
    section(ui, x, y, w, "At the start", "");
    y += 22.0;
    let mut b = c.autostart;
    if switch(ui, "autostart", x, y, w, &mut b, "Put the bus into service on start (Shift+U)") {
        c.autostart = b;
        out.touched = true;
    }
    y += 38.0;
    let mut b = c.on_foot;
    if switch(ui, "onfoot", x, y, w, &mut b, "Start on foot (place a bus from the game menu)") {
        c.on_foot = b;
        out.touched = true;
    }
    y += 38.0 + 24.0;
    // the weather
    let chosen = ctx.items.iter().find(|i| i.value == c.weather).map(|i| omsi_ui::tr(&i.name).into_owned()).unwrap_or_default();
    section(ui, x, y, w, "Weather", &chosen);
    y += 24.0;
    if let Some(mut custom) = crate::weather_setup::custom_weather(Some(&c.weather)) {
        // one's own weather: its editor in the list's place (the value is still just
        // `choice.weather`, so it goes to the game through `--weather` as any other)
        if ui.button("weather-presets", Rect::new(x, y, w, 34.0), "Choose a weather preset", Some("arrow_back"), ButtonKind::Normal) {
            c.weather.clear();
            out.touched = true;
            return y + 44.0 - v.y;
        }
        y += 46.0;
        let mut changed = false;
        let row = |y: f32| Rect::new(x, y, w, 34.0);
        changed |= ui.slider("custom-vis", row(y), &mut custom.visibility_m, 50.0, 50_000.0, 50.0, "Visibility", &|x| if x >= 49_950.0 { "unlimited".into() } else if x >= 1000.0 { format!("{:.1} km", x / 1000.0) } else { format!("{x:.0} m") });
        y += 40.0;
        changed |= ui.slider("custom-bright", row(y), &mut custom.brightness, 0.0, 1.5, 0.05, "Brightness", &|x| format!("{:.0} %", x * 100.0));
        y += 40.0;
        changed |= ui.slider("custom-wdir", row(y), &mut custom.wind_dir, 0.0, 355.0, 5.0, "Wind direction", &|x| format!("{x:.0}°"));
        y += 40.0;
        changed |= ui.slider("custom-wspeed", row(y), &mut custom.wind_speed, 0.0, 40.0, 0.5, "Wind speed", &|x| format!("{x:.1} m/s"));
        y += 40.0;
        changed |= ui.slider("custom-temp", row(y), &mut custom.temp_c, -30.0, 45.0, 1.0, "Temperature", &|x| format!("{x:.0} °C"));
        y += 40.0;
        let temp_for_dew = custom.temp_c;
        changed |= ui.slider("custom-hum", row(y), &mut custom.humidity, 0.0, 100.0, 1.0, "Humidity", &|x| format!("{x:.0} % · {}", omsi_ui::tr("dew %{t} °C").replace("%{t}", &format!("{:.0}", crate::weather_setup::dew_point_c(temp_for_dew, x)))));
        y += 46.0;
        ui.label(Rect::new(x, y, 130.0, 32.0), "Cloud type");
        let clouds: Vec<String> = crate::weather_setup::CUSTOM_CLOUDS.iter().map(|x| x.to_string()).collect();
        let mut cloud = custom.cloud;
        if ui.select("custom-cloud", Rect::new(x + 130.0, y, w - 130.0, 32.0), &mut cloud, &clouds) {
            custom.cloud = cloud;
            changed = true;
        }
        y += 42.0;
        ui.label(Rect::new(x, y, 130.0, 32.0), "Precipitation");
        let precips: Vec<String> = crate::weather_setup::CUSTOM_PRECIP.iter().map(|x| x.to_string()).collect();
        let mut precip = custom.precip.clamp(0, 2) as usize;
        if ui.select("custom-precip", Rect::new(x + 130.0, y, w - 130.0, 32.0), &mut precip, &precips) {
            custom.precip = precip as i32;
            changed = true;
        }
        y += 42.0;
        changed |= ui.slider("custom-intensity", row(y), &mut custom.precip_intensity, 0.0, 255.0, 1.0, "Precipitation intensity", &|x| format!("{x:.0} / 255"));
        y += 40.0;
        changed |= ui.slider("custom-wet", row(y), &mut custom.road_wetness, 0.0, 1.0, 0.05, "Road wetness", &|x| format!("{:.0} %", x * 100.0));
        y += 40.0;
        changed |= switch(ui, "custom-snow", x, y, w, &mut custom.snow_cover, "Snow cover");
        y += 38.0;
        changed |= switch(ui, "custom-snow-road", x, y, w, &mut custom.snow_on_road, "Snow on road");
        y += 44.0;
        if changed {
            custom.normalize();
            c.weather = custom.encode();
            out.touched = true;
        }
        return y - v.y;
    }
    if let Some(custom) = ctx.as_custom.as_ref() {
        if ui.button("weather-edit-current", Rect::new(x, y, w, 34.0), "Edit selected weather as custom", Some("tune"), ButtonKind::Normal) {
            c.weather = custom.clone();
            out.touched = true;
        }
        y += 44.0;
    }
    // the current weather: of which airport
    if let Some(code) = c.weather.strip_prefix("metar:").map(str::to_string) {
        let mut airport: String = code.to_uppercase().chars().take(4).collect();
        ui.label(Rect::new(x, y, 80.0, ROW), "Airport");
        let list_w = 150.0;
        let input_w = w - 80.0 - list_w - 8.0;
        if ui.text_input("metar-airport", Rect::new(x + 80.0, y, input_w, ROW), &mut airport, "ICAO", None) {
            let a: String = airport.chars().filter(|c| c.is_ascii_alphabetic()).take(4).collect::<String>().to_uppercase();
            c.weather = format!("metar:{a}");
            out.touched = true;
        }
        let names: Vec<String> = ctx.airports.iter().map(|a| a.1.clone()).collect();
        let mut sel = ctx.airports.iter().position(|a| a.0.eq_ignore_ascii_case(&airport)).unwrap_or(0);
        if ui.select("metar-airport-list", Rect::new(x + w - list_w, y, list_w, ROW), &mut sel, &names) {
            if let Some(a) = ctx.airports.get(sel) {
                c.weather = format!("metar:{}", a.0);
                out.touched = true;
            }
        }
        y += ROW + 10.0;
    }
    // the weathers: a list, the chosen one filled
    const ROW_H: f32 = 50.0;
    for item in &ctx.items {
        let rr = Rect::new(x - 6.0, y, w + 12.0, ROW_H - 3.0);
        if ui.rect_visible(rr) {
            let on = item.value == c.weather;
            if ui.row(&format!("day-w-{}", item.value), rr, on) && !on {
                c.weather = item.value.clone();
                out.touched = true;
            }
            ui.icon(item.icon, Vec2::new(rr.x + 22.0, rr.center().y), 20.0, if on { on_accent() } else { TEXT_SOFT });
            let tw = ui.text_in(&item.name, Rect::new(rr.x + 46.0, rr.y + 7.0, rr.w - 56.0, 19.0), 13.5, Weight::Bold, if on { on_accent() } else { TEXT }, Align::Left);
            if item.fresh {
                ui.badge(Vec2::new(rr.x + 52.0 + tw, rr.y + 9.0), "NEW", OK);
            }
            ui.text_in(&item.meta, Rect::new(rr.x + 46.0, rr.y + 27.0, rr.w - 56.0, 16.0), 11.5, Weight::Regular, if on { on_accent().alpha(0.85) } else { TEXT_DIM }, Align::Left);
        }
        y += ROW_H;
    }
    y + 6.0 - v.y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> DayCtx {
        let item = |value: &str, name: &str| WeatherItem { value: value.into(), name: name.into(), meta: String::new(), icon: "wb_sunny", fresh: false };
        DayCtx { items: vec![item("", "Map default"), item("custom:x", "Custom weather"), item("metar:EDDB", "Current weather"), item("cycle", "Weather cycle"), item("weather/sun.owt", "Sun")], airports: vec![("EDDB".into(), "Berlin".into())], as_custom: None, season_of_date: "Spring", lead: Some(5) }
    }

    /// One frame of the sheet, tall enough that nothing is cut off.
    fn frame(ui: &mut Ui, c: &mut Choice, out: &mut DayOut) -> f32 {
        ui.begin(Vec2::new(420.0, 2400.0), 1.0, 1.0 / 60.0);
        day_body(ui, Rect::new(0.0, 0.0, 400.0, 2400.0), c, &ctx(), out)
    }

    /// Click `name`: the mouse goes down over it and comes up again.
    fn click(c: &mut Choice, name: &str) -> DayOut {
        let mut ui = Ui::new();
        let mut out = DayOut::default();
        frame(&mut ui, c, &mut out);
        let r = *ui.drawn.get(&id_of(name)).unwrap_or_else(|| panic!("{name} is not on the sheet"));
        ui.input.mouse = r.center();
        ui.input.pressed = true;
        ui.input.down = true;
        frame(&mut ui, c, &mut out);
        ui.input.pressed = false;
        ui.input.down = false;
        ui.input.released = true;
        frame(&mut ui, c, &mut out);
        out
    }

    /// Everything openOMSI sets for the day is on the sheet: the time and the date, the
    /// seasons, the traffic, the passengers, the timetable's buses, the start in service or
    /// on foot, and every weather.
    #[test]
    fn every_choice_of_the_day_is_on_the_sheet() {
        let mut ui = Ui::new();
        let mut c = Choice::default();
        frame(&mut ui, &mut c, &mut DayOut::default());
        let mut names: Vec<String> = ["date", "traffic", "pax", "sched", "autostart", "onfoot"].iter().map(|x| x.to_string()).collect();
        names.extend((0..5).map(|k| format!("day-season-{k}")));
        names.extend(ctx().items.iter().map(|i| format!("day-w-{}", i.value)));
        for name in &names {
            assert!(ui.drawn.contains_key(&id_of(name)), "{name} is not on the sheet");
        }
        assert!(ui.drawn.contains_key(&(id_of("time.0") ^ 1)), "the time's arrows are not on the sheet");
    }

    #[test]
    fn the_bus_can_stand_ready_before_the_departure() {
        let mut c = Choice { time: 14 * 60 + 15, ..Default::default() };
        let out = click(&mut c, "day-lead-3");
        assert_eq!(out.lead, Some(10));
        // (the duty starts the lead before: before midnight, the day before)
        assert_eq!(omsi_launcher_lib::lead_start("2026-10-06", 14 * 60 + 15, 10), ("2026-10-06".to_string(), 14 * 60 + 5));
        assert_eq!(omsi_launcher_lib::lead_start("2026-10-06", 3, 10), ("2026-10-05".to_string(), 24 * 60 - 7));
    }

    #[test]
    fn the_current_time_and_date_are_a_click_away() {
        let (today, now) = clock_now().expect("the computer has a clock");
        let mut c = Choice { date: "1989-05-30".into(), time: 0, season: "winter".into(), ..Default::default() };
        assert!(click(&mut c, "time-now").touched);
        // (a minute may have passed in between)
        assert!((c.time - now).abs() <= 1, "{} against {now}", c.time);
        assert_eq!(c.date, "1989-05-30");
        let out = click(&mut c, "date-now");
        assert_eq!((c.date.as_str(), c.season.as_str()), (today.as_str(), "auto"));
        assert!(out.date_changed && out.season_changed);
    }

    #[test]
    fn a_season_moves_the_date_into_it() {
        let mut c = Choice { date: "1989-05-30".into(), ..Default::default() };
        let out = click(&mut c, "day-season-4");
        assert_eq!((c.season.as_str(), c.date.as_str()), ("winter", "1989-01-30"));
        assert!(out.touched && out.date_changed && out.season_changed);
        // and by the date again: the date stays
        let out = click(&mut c, "day-season-0");
        assert_eq!((c.season.as_str(), c.date.as_str()), ("auto", "1989-01-30"));
        assert!(out.season_changed && !out.date_changed);
    }

    #[test]
    fn switches_and_weathers_change_the_choice() {
        let mut c = Choice::default();
        assert!(c.passengers);
        assert!(click(&mut c, "pax").touched);
        assert!(!c.passengers);
        click(&mut c, "day-w-cycle");
        assert_eq!(c.weather, "cycle");
        // the current weather asks for its airport
        click(&mut c, "day-w-metar:EDDB");
        let mut ui = Ui::new();
        frame(&mut ui, &mut c, &mut DayOut::default());
        assert!(ui.drawn.contains_key(&id_of("metar-airport")) && ui.drawn.contains_key(&id_of("metar-airport-list")));
    }

    /// One's own weather turns the list into its editor, and back.
    #[test]
    fn custom_weather_has_its_editor() {
        let mut c = Choice { weather: crate::weather_setup::CustomWeather::default().encode(), ..Default::default() };
        let mut ui = Ui::new();
        frame(&mut ui, &mut c, &mut DayOut::default());
        for name in ["weather-presets", "custom-vis", "custom-temp", "custom-cloud", "custom-precip", "custom-wet", "custom-snow", "custom-snow-road"] {
            assert!(ui.drawn.contains_key(&id_of(name)), "{name} is not in the editor");
        }
        assert!(!ui.drawn.contains_key(&id_of("day-w-cycle")), "the list is not shown under the editor");
        click(&mut c, "weather-presets");
        assert_eq!(c.weather, "");
    }
}
