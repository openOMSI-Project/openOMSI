//! The Timetable page: a map's lines, their tours and the trips each tour runs - the part of
//! OMSI 2's timetable editor a driver uses to change when buses go. A line is saved as its
//! `.ttl`; a map of the original installation gets its `TTData` copied into the content
//! folder first (the game reads that copy before the original's, which stays untouched).
//! Trips (`.ttp`) and their tracks (`.ttr`) are the map maker's and are only chosen here.

use super::theme::*;
use super::ui::{ButtonKind, Ui};
use super::Launcher;
use omsi_launcher_lib as core;
use omsi_timetable::{Line, TimetableData, Tour, TourTrip};
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct TimetableView {
    pub map: usize,
    /// The map the data was read for (its `global.cfg`).
    loaded: Option<String>,
    data: Option<TimetableData>,
    map_dir: PathBuf,
    line: usize,
    tour: usize,
    /// The departures being typed, one per trip of the shown tour.
    times: Vec<String>,
    times_for: Option<(usize, usize)>,
    /// Minutes a copied tour runs after the one it copies (and the repeat's interval).
    offset: String,
    /// The repeat's last departure ("h:mm").
    until: String,
    /// The name typed for a new line.
    new_line: String,
    /// The lines changed and not saved yet (by name): kept while the player moves between
    /// lines - the page used to drop a line's changes when another was clicked, and a day's
    /// timetable took a save after every line.
    dirty: std::collections::BTreeSet<String>,
    /// The reset button was pressed once: the next press puts the map's own timetable back.
    reset_armed: bool,
}

/// Marks a `TTData` folder the launcher copied into the content folder (see `save_target`):
/// the reset deletes such a copy, and only such a one.
const COPY_MARK: &str = ".openomsi-ttdata-copy";

/// "h:mm" (or "h:mm:ss") from minutes after midnight.
pub fn fmt_time(min: f32) -> String {
    let s = (min as f64 * 60.0).round() as i64;
    if s % 60 == 0 {
        format!("{}:{:02}", s / 3600, s / 60 % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    }
}

/// Minutes after midnight from "h:mm", "h:mm:ss" or "h.mm" (after midnight as 24:10 and on).
pub fn parse_time(t: &str) -> Option<f32> {
    let parts: Vec<&str> = t.trim().split([':', '.']).collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let h: u32 = parts[0].trim().parse().ok()?;
    let m: u32 = parts[1].trim().parse().ok()?;
    let s: u32 = parts.get(2).map(|x| x.trim().parse().ok()).unwrap_or(Some(0))?;
    (h < 48 && m < 60 && s < 60).then(|| h as f32 * 60.0 + m as f32 + s as f32 / 60.0)
}

/// Where a map's line is written: in place when the map lies unpacked in the content folder
/// (its file backed up as `<file>.orig` the first time, for the reset), else in the content
/// folder's copy of its `TTData` (made whole first - the game reads the one folder, not a mix
/// of both). A map inside a mod archive (`.zip`) counts as not in the content folder: its
/// file was "saved in place" into the archive's path, which is no folder, and never saved.
fn save_target(line: &Line, map_folder: &str, original_ttdata: &Path) -> Result<PathBuf, String> {
    let content = core::content_dir().ok_or("no content folder")?;
    let file = line.path.file_name().map(|f| f.to_owned()).unwrap_or_else(|| format!("{}.ttl", line.name).into());
    if line.path.starts_with(&content) && omsi_cfg::vfs::archive_of(&line.path).is_none() {
        let orig = PathBuf::from(format!("{}.orig", line.path.display()));
        let copied = line.path.parent().is_some_and(|d| d.join(COPY_MARK).is_file());
        if !copied && line.path.is_file() && !orig.exists() {
            std::fs::copy(&line.path, &orig).map_err(|e| e.to_string())?;
        }
        return Ok(line.path.clone());
    }
    let dir = content.join("maps").join(map_folder).join("TTData");
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, is_dir) in omsi_cfg::vfs::list_dir(original_ttdata).ok_or_else(|| format!("{} cannot be read", original_ttdata.display()))? {
            if !is_dir {
                let bytes = omsi_cfg::vfs::read(&original_ttdata.join(&name)).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(dir.join(COPY_MARK), b"TTData copied by the openOMSI launcher's timetable editor; its reset deletes this folder\n").map_err(|e| e.to_string())?;
    }
    Ok(dir.join(file))
}

/// The map's own timetable back: the launcher's copy of its `TTData` deleted, or (a map
/// unpacked in the content folder) every line saved over put back from its `.orig`.
/// Returns what was done.
fn reset_timetable(map_dir: &Path, map_folder: &str) -> Result<String, String> {
    let content = core::content_dir().ok_or("no content folder")?;
    let copy = content.join("maps").join(map_folder).join("TTData");
    if copy.join(COPY_MARK).is_file() {
        std::fs::remove_dir_all(&copy).map_err(|e| e.to_string())?;
        return Ok(format!("The timetable of {map_folder} is the map's own again (the edited copy was removed)"));
    }
    let own = map_dir.join("TTData");
    let mut restored = 0;
    if own.is_dir() {
        for e in std::fs::read_dir(&own).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if let Some(orig) = p.to_str().and_then(|s| s.strip_suffix(".orig")) {
                std::fs::rename(&p, orig).map_err(|e| e.to_string())?;
                restored += 1;
            }
        }
    }
    if restored > 0 {
        Ok(format!("{restored} line(s) of {map_folder} put back as they were"))
    } else {
        Err(format!("The timetable of {map_folder} has not been changed here"))
    }
}

/// Save every changed line of the map (see `save_target`); how many were saved, and the
/// first error.
fn save_all(tv: &mut TimetableView) -> (usize, Option<String>) {
    let Some(data) = tv.data.as_mut() else { return (0, None) };
    let folder = tv.map_dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let original = omsi_cfg::resolve_path(&tv.map_dir, "TTData");
    let mut saved = 0;
    let mut err = None;
    for line in data.lines.iter_mut().filter(|l| tv.dirty.contains(&l.name)) {
        for t in &mut line.tours {
            t.trips.sort_by(|a, b| a.departure.total_cmp(&b.departure));
        }
        match save_target(line, &folder, &original).and_then(|p| line.save(&p).map(|_| p).map_err(|e| e.to_string())) {
            Ok(p) => {
                line.path = p;
                saved += 1;
            }
            Err(e) => {
                err.get_or_insert(format!("line {}: {e}", line.name));
            }
        }
    }
    if err.is_none() {
        tv.dirty.clear();
    }
    if saved > 0 {
        omsi_cfg::content_changed();
    }
    (saved, err)
}

/// Copies of `base` every `every` minutes after it, as long as the copy's first departure is
/// not after `until` (minutes of the day); numbered on from the highest. Returns how many.
fn repeat_tour(tours: &mut Vec<Tour>, base: &Tour, every: f32, until: f32) -> usize {
    let first = base.trips.first().map(|t| t.departure).unwrap_or(0.0);
    let mut made = 0;
    let mut k = 1.0;
    while first + every * k <= until + 1e-3 && made < 500 {
        let mut t = base.clone();
        for x in &mut t.trips {
            x.departure += every * k;
        }
        t.number = next_number(tours);
        tours.push(t);
        made += 1;
        k += 1.0;
    }
    made
}

/// A new tour's number: one past the highest that is a number.
fn next_number(tours: &[Tour]) -> String {
    (tours.iter().filter_map(|t| t.number.trim().parse::<i64>().ok()).max().unwrap_or(0) + 1).to_string()
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Timetable", "A map's lines: their tours and when each trip leaves. Saved as the line's .ttl (in the content folder; OMSI 2's own files stay as they are).");
    let maps: Vec<(String, String)> = l.state.maps.iter().map(|m| (m.friendly.clone(), m.file.clone())).collect();
    if maps.is_empty() {
        l.ui.paragraph("No maps found.", glam::Vec2::new(body.x, body.y), body.w, 14.0, Weight::Regular, TEXT_DIM());
        return;
    }
    let tv = &mut l.pages.tt;
    // (first shown: the map chosen on the Drive page, not the first of the list)
    if tv.loaded.is_none() {
        if let Some(k) = maps.iter().position(|m| m.1 == l.state.choice.map) {
            tv.map = k;
        }
    }
    tv.map = tv.map.min(maps.len() - 1);
    // read the chosen map's timetable
    if tv.loaded.as_deref() != Some(maps[tv.map].1.as_str()) {
        let file = &maps[tv.map].1;
        let root = PathBuf::from(&l.state.config.root);
        let global = core::content_dir().map(|c| c.join(file)).filter(|p| p.is_file()).unwrap_or_else(|| omsi_cfg::resolve_path(&root, file));
        tv.map_dir = global.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut d = TimetableData::load(&tv.map_dir);
        d.lines.sort_by_key(|x| x.name.to_lowercase());
        tv.data = Some(d);
        tv.loaded = Some(file.clone());
        tv.line = 0;
        tv.tour = 0;
        tv.times_for = None;
        tv.dirty.clear();
    }
    if tv.offset.is_empty() {
        tv.offset = "20".into();
    }
    if tv.until.is_empty() {
        tv.until = "22:00".into();
    }
    let col1 = (body.w * 0.24).min(300.0);
    let col2 = (body.w * 0.22).min(260.0);
    let left = Rect::new(body.x, body.y, col1, body.h);
    let mid = Rect::new(left.right() + GAP * 2.0, body.y, col2, body.h);
    let right = Rect::new(mid.right() + GAP * 2.0, body.y, body.right() - mid.right() - GAP * 2.0, body.h);
    let mut status: Option<(String, bool)> = None;

    // --- the map and its lines
    let ui: &mut Ui = &mut l.ui;
    ui.panel(left);
    let inner = ui.heading(Rect::new(left.x + 18.0, left.y + 14.0, left.w - 36.0, left.h - 28.0), "Lines", Some("route"));
    let names: Vec<String> = maps.iter().map(|m| m.0.clone()).collect();
    let mut m = tv.map;
    if ui.select("tt-map", Rect::new(inner.x, inner.y, inner.w, ROW), &mut m, &names) {
        // (another map: this one's changes are saved first, not lost)
        if !tv.dirty.is_empty() {
            let (saved, err) = save_all(tv);
            match err {
                Some(e) => l.state.set_status(format!("Not saved: {e}"), true),
                None => l.state.set_status(format!("{saved} line(s) saved"), false),
            }
        }
        tv.map = m;
        tv.loaded = None;
        return;
    }
    // the map's own timetable back (below its lines; two presses)
    let reset_r = Rect::new(inner.x, inner.bottom() - ROW, inner.w, ROW);
    // a new line: its name, then Add (the file is `<name>.ttl` in the map's TTData)
    let new_r = Rect::new(inner.x, reset_r.y - ROW - 8.0, inner.w - 90.0, ROW);
    ui.text_input("tt-new-line", new_r, &mut tv.new_line, "New line (name)", Some("add"));
    let add_line = ui.button("tt-add-line", Rect::new(new_r.right() + 8.0, new_r.y, 82.0, ROW), "Add", None, ButtonKind::Normal);
    if ui.button("tt-reset", reset_r, if tv.reset_armed { "Press again to reset" } else { "Reset timetable" }, Some("restart_alt"), ButtonKind::Normal) {
        if tv.reset_armed {
            tv.reset_armed = false;
            let folder = tv.map_dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            match reset_timetable(&tv.map_dir, &folder) {
                Ok(m) => {
                    omsi_cfg::content_changed();
                    // read again from where the game reads it
                    tv.loaded = None;
                    l.state.set_status(m, false);
                    return;
                }
                Err(e) => status = Some((e, true)),
            }
        } else {
            tv.reset_armed = true;
            status = Some(("Press \"Reset timetable\" again to put the map's own timetable back (the changes made here are lost)".into(), false));
        }
    }
    let Some(data) = tv.data.as_mut() else { return };
    if add_line {
        let name = tv.new_line.trim().to_string();
        if name.is_empty() || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
            status = Some(("Type the new line's name (it becomes the file name, so no / \\ : * ? \" < > |)".into(), true));
        } else if data.lines.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
            status = Some((format!("Line {name} is there already"), true));
        } else {
            let path = omsi_cfg::resolve_path(&tv.map_dir, "TTData").join(format!("{name}.ttl"));
            data.lines.push(Line { path, name: name.clone(), user_allowed: true, priority: 0, tours: Vec::new() });
            data.lines.sort_by_key(|x| x.name.to_lowercase());
            tv.line = data.lines.iter().position(|x| x.name == name).unwrap_or(0);
            tv.tour = 0;
            tv.times_for = None;
            tv.dirty.insert(name.clone());
            tv.new_line.clear();
            status = Some((format!("Line {name} added: give it tours, then save"), false));
        }
    }
    let mut pick_line = None;
    let lines: Vec<(String, usize)> = data.lines.iter().map(|x| (x.name.clone(), x.tours.len())).collect();
    let (sel_line, dirty) = (tv.line, &tv.dirty);
    ui.scroll_area("tt-lines", Rect::new(inner.x - 6.0, inner.y + ROW + 12.0, inner.w + 12.0, inner.h - 3.0 * ROW - 32.0), &mut |ui, v| {
        for (i, (name, tours)) in lines.iter().enumerate() {
            let r = Rect::new(v.x + 6.0, v.y + i as f32 * 42.0, v.w - 16.0, 38.0);
            if ui.row(&format!("tt-line-{i}"), r, i == sel_line) {
                pick_line = Some(i);
            }
            let star = if dirty.contains(name) { " •" } else { "" };
            ui.text_in(&format!("{name}{star}"), Rect::new(r.x + 12.0, r.y, r.w - 90.0, r.h), 13.0, Weight::Medium, TEXT(), Align::Left);
            ui.text_in(&format!("{tours} tours"), Rect::new(r.right() - 90.0, r.y, 80.0, r.h), 11.5, Weight::Regular, TEXT_DIM(), Align::Right);
        }
        lines.len() as f32 * 42.0
    });
    if lines.is_empty() {
        ui.paragraph("This map has no timetable (no TTData lines).", glam::Vec2::new(inner.x, inner.y + ROW + 20.0), inner.w, 13.0, Weight::Regular, TEXT_DIM());
        return;
    }
    if let Some(i) = pick_line {
        // (the line left keeps its changes: they are saved with the others)
        tv.line = i;
        tv.tour = 0;
        tv.times_for = None;
    }
    tv.line = tv.line.min(data.lines.len() - 1);
    let line_name = data.lines[tv.line].name.clone();
    // the trips this line can run: those naming it, else those it already runs, else all
    let mut trips: Vec<String> = data.trips.iter().filter(|t| t.line.trim().eq_ignore_ascii_case(line_name.trim())).map(|t| t.name.clone()).collect();
    for t in data.lines[tv.line].tours.iter().flat_map(|t| t.trips.iter()) {
        if !trips.iter().any(|x| x.eq_ignore_ascii_case(&t.trip)) {
            trips.push(t.trip.clone());
        }
    }
    if trips.is_empty() {
        trips = data.trips.iter().map(|t| t.name.clone()).collect();
    }
    trips.sort_by_key(|t| t.to_lowercase());
    let profiles_of = |name: &str| -> Vec<String> {
        data.trip(name).map(|t| t.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).filter(|p| !p.is_empty()).unwrap_or_else(|| vec!["0".into()])
    };
    let terminus_of = |name: &str| data.trip(name).map(|t| t.terminus.clone()).unwrap_or_default();
    let profiles: Vec<(String, Vec<String>, String)> = trips.iter().map(|t| (t.clone(), profiles_of(t), terminus_of(t))).collect();
    let line = &mut data.lines[tv.line];

    // --- its tours
    ui.panel(mid);
    let inner = ui.heading(Rect::new(mid.x + 18.0, mid.y + 14.0, mid.w - 36.0, mid.h - 28.0), &format!("Line {line_name}"), Some("directions_bus"));
    let mut pick_tour = None;
    let tours: Vec<(String, String, usize)> = line.tours.iter().map(|t| (t.number.clone(), t.trips.first().map(|x| fmt_time(x.departure)).unwrap_or_default(), t.trips.len())).collect();
    let sel_tour = tv.tour;
    let list_h = inner.h - 4.0 * (ROW + 8.0) - 8.0;
    ui.scroll_area(&format!("tt-tours-{line_name}"), Rect::new(inner.x - 6.0, inner.y, inner.w + 12.0, list_h), &mut |ui, v| {
        for (i, (num, first, n)) in tours.iter().enumerate() {
            let r = Rect::new(v.x + 6.0, v.y + i as f32 * 42.0, v.w - 16.0, 38.0);
            if ui.row(&format!("tt-tour-{i}"), r, i == sel_tour) {
                pick_tour = Some(i);
            }
            ui.text_in(&format!("Tour {num}"), Rect::new(r.x + 12.0, r.y, r.w - 100.0, r.h), 13.0, Weight::Medium, TEXT(), Align::Left);
            ui.text_in(&format!("{first} · {n}"), Rect::new(r.right() - 100.0, r.y, 90.0, r.h), 11.5, Weight::Regular, TEXT_DIM(), Align::Right);
        }
        tours.len() as f32 * 42.0
    });
    if let Some(i) = pick_tour {
        tv.tour = i;
    }
    let mut y = inner.y + list_h + 8.0;
    let half = (inner.w - GAP) * 0.5;
    if ui.button("tt-new-tour", Rect::new(inner.x, y, inner.w, ROW), "New tour", Some("add"), ButtonKind::Normal) {
        let ai_group = line.tours.last().map(|t| t.ai_group.clone()).unwrap_or_else(|| "Busses".into());
        let extra = line.tours.last().map(|t| t.extra.clone()).unwrap_or_default();
        let first = trips.first().cloned().unwrap_or_default();
        line.tours.push(Tour { number: next_number(&line.tours), ai_group, extra, trips: vec![TourTrip { trip: first, profile: 0, departure: 6.0 * 60.0 }] });
        tv.tour = line.tours.len() - 1;
        tv.dirty.insert(line_name.clone());
    }
    y += ROW + 8.0;
    ui.text_input("tt-offset", Rect::new(inner.x, y, half, ROW), &mut tv.offset, "min", Some("schedule"));
    let off = tv.offset.trim().parse::<f32>().ok();
    if ui.button("tt-copy-tour", Rect::new(inner.x + half + GAP, y, half, ROW), "Copy +min", Some("content_copy"), ButtonKind::Normal) {
        match (off, line.tours.get(tv.tour).cloned()) {
            (Some(off), Some(mut t)) => {
                for x in &mut t.trips {
                    x.departure += off;
                }
                t.number = next_number(&line.tours);
                line.tours.push(t);
                tv.tour = line.tours.len() - 1;
                tv.dirty.insert(line_name.clone());
            }
            (None, _) => status = Some(("Type the minutes the copy runs later (negative for earlier).".into(), true)),
            _ => {}
        }
    }
    y += ROW + 8.0;
    // the whole day at once: copies of the tour every <min> minutes up to a last departure
    ui.text_input("tt-until", Rect::new(inner.x, y, half, ROW), &mut tv.until, "until h:mm", Some("schedule"));
    if ui.button("tt-repeat", Rect::new(inner.x + half + GAP, y, half, ROW), "Repeat", Some("autorenew"), ButtonKind::Normal) {
        match (off.filter(|o| *o >= 1.0), parse_time(&tv.until), line.tours.get(tv.tour).cloned()) {
            (Some(every), Some(until), Some(base)) => {
                let made = repeat_tour(&mut line.tours, &base, every, until);
                if made > 0 {
                    tv.tour = line.tours.len() - 1;
                    tv.dirty.insert(line_name.clone());
                }
                status = Some((format!("{made} tour(s) made, every {every} min up to {}", fmt_time(until)), false));
            }
            (None, _, _) => status = Some(("Type the minutes between the tours (1 or more) in the field above".into(), true)),
            (_, None, _) => status = Some(("Type the last departure as h:mm".into(), true)),
            _ => {}
        }
    }
    y += ROW + 8.0;
    if ui.button("tt-del-tour", Rect::new(inner.x, y, inner.w, ROW), "Delete tour", Some("delete"), ButtonKind::Normal) && tv.tour < line.tours.len() {
        line.tours.remove(tv.tour);
        tv.tour = tv.tour.saturating_sub(1);
        tv.dirty.insert(line_name.clone());
    }
    tv.tour = tv.tour.min(line.tours.len().saturating_sub(1));

    // --- the tour's trips
    ui.panel(right);
    let Some(tour) = line.tours.get_mut(tv.tour) else {
        if let Some(s) = status {
            l.state.set_status(s.0, s.1);
        }
        return;
    };
    let inner = ui.heading(Rect::new(right.x + 18.0, right.y + 14.0, right.w - 36.0, right.h - 28.0), &format!("Tour {} - {}", tour.number, tour.ai_group), Some("schedule"));
    if tv.times_for != Some((tv.line, tv.tour)) || tv.times.len() != tour.trips.len() {
        tv.times = tour.trips.iter().map(|t| fmt_time(t.departure)).collect();
        tv.times_for = Some((tv.line, tv.tour));
    }
    let trip_names: Vec<String> = profiles.iter().map(|p| p.0.clone()).collect();
    let (tw, pw) = (110.0, 150.0);
    let dest_w = 170.0;
    let trip_w = (inner.w - tw - pw - dest_w - 40.0 - 4.0 * GAP).max(120.0);
    let head = |ui: &mut Ui, x: f32, w: f32, t: &str| ui.text_in(t, Rect::new(x, inner.y, w, 18.0), 11.5, Weight::Bold, TEXT_DIM(), Align::Left);
    head(ui, inner.x, tw, "Departs");
    head(ui, inner.x + tw + GAP, trip_w, "Trip");
    head(ui, inner.x + tw + trip_w + 2.0 * GAP, pw, "Profile");
    head(ui, inner.x + tw + trip_w + pw + 3.0 * GAP, dest_w, "To");
    let mut remove = None;
    let mut changed = false;
    let rows = Rect::new(inner.x - 6.0, inner.y + 24.0, inner.w + 12.0, inner.h - 24.0 - ROW - 14.0);
    let times = &mut tv.times;
    let key = format!("{line_name}-{}", tv.tour);
    ui.scroll_area(&format!("tt-trips-{key}"), rows, &mut |ui, v| {
        let rh = ROW + 6.0;
        for (i, t) in tour.trips.iter_mut().enumerate() {
            let y = v.y + i as f32 * rh;
            let x = v.x + 6.0;
            let bad = parse_time(&times[i]).is_none();
            if ui.text_input(&format!("tt-dep-{key}-{i}"), Rect::new(x, y, tw, ROW), &mut times[i], "h:mm", None) {
                if let Some(m) = parse_time(&times[i]) {
                    t.departure = m;
                    changed = true;
                }
            }
            if bad {
                ui.p().rounded_border(Rect::new(x, y, tw, ROW), 6.0, 1.5, DANGER());
            }
            let mut ti = trip_names.iter().position(|n| n.eq_ignore_ascii_case(&t.trip)).unwrap_or(0);
            if ui.select(&format!("tt-trip-{key}-{i}"), Rect::new(x + tw + GAP, y, trip_w, ROW), &mut ti, &trip_names) {
                t.trip = trip_names[ti].clone();
                t.profile = 0;
                changed = true;
            }
            let (_, profs, dest) = profiles.iter().find(|p| p.0.eq_ignore_ascii_case(&t.trip)).cloned().unwrap_or_default();
            let profs = if profs.is_empty() { vec![t.profile.to_string()] } else { profs };
            let mut pi = (t.profile.max(0) as usize).min(profs.len() - 1);
            if ui.select(&format!("tt-prof-{key}-{i}"), Rect::new(x + tw + trip_w + 2.0 * GAP, y, pw, ROW), &mut pi, &profs) {
                t.profile = pi as i32;
                changed = true;
            }
            ui.text_in(&dest, Rect::new(x + tw + trip_w + pw + 3.0 * GAP, y, dest_w, ROW), 12.5, Weight::Regular, TEXT_SOFT(), Align::Left);
            if ui.icon_button(&format!("tt-x-{key}-{i}"), glam::Vec2::new(v.right() - 26.0, y + ROW * 0.5), 14.0, "close", "Remove this trip") {
                remove = Some(i);
            }
        }
        tour.trips.len() as f32 * rh
    });
    if let Some(i) = remove {
        tour.trips.remove(i);
        tv.times.remove(i);
        changed = true;
    }
    // the buttons under the list
    let by = inner.bottom() - ROW;
    let bw = 150.0;
    if ui.button("tt-add-trip", Rect::new(inner.x, by, bw, ROW), "Add trip", Some("add"), ButtonKind::Normal) {
        // the next of the alternation (there and back), as far after as the last gap
        let n = tour.trips.len();
        let t = match n {
            0 => TourTrip { trip: trips.first().cloned().unwrap_or_default(), profile: 0, departure: 6.0 * 60.0 },
            1 => TourTrip { departure: tour.trips[0].departure + 30.0, ..tour.trips[0].clone() },
            _ => TourTrip { departure: tour.trips[n - 1].departure + (tour.trips[n - 1].departure - tour.trips[n - 2].departure).max(1.0), ..tour.trips[n - 2].clone() },
        };
        tv.times.push(fmt_time(t.departure));
        tour.trips.push(t);
        changed = true;
    }
    for (k, (label, d)) in [("−1 min", -1.0f32), ("+1 min", 1.0)].iter().enumerate() {
        if ui.button(&format!("tt-shift-{k}"), Rect::new(inner.x + bw + GAP + k as f32 * (100.0 + GAP), by, 100.0, ROW), label, None, ButtonKind::Normal) {
            for t in &mut tour.trips {
                t.departure += d;
            }
            tv.times_for = None;
            changed = true;
        }
    }
    if changed {
        tv.dirty.insert(line_name.clone());
    }
    let n = tv.dirty.len();
    let save_r = Rect::new(inner.right() - 170.0, by, 170.0, ROW);
    let label = match n {
        0 => "Saved".to_string(),
        1 => "Save".to_string(),
        n => format!("Save all ({n} lines)"),
    };
    if ui.button("tt-save", save_r, &label, Some("save"), if n > 0 { ButtonKind::Primary } else { ButtonKind::Normal }) && n > 0 {
        if tv.times.iter().any(|t| parse_time(t).is_none()) {
            status = Some(("A departure is not a time (h:mm).".into(), true));
        } else {
            let (saved, err) = save_all(tv);
            tv.times_for = None;
            status = Some(match err {
                Some(e) => (format!("Not saved: {e}"), true),
                None => (format!("{saved} line(s) saved"), false),
            });
        }
    }
    if let Some((s, err)) = status {
        l.state.set_status(s, err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("4:07"), Some(247.0));
        assert_eq!(parse_time("24:10"), Some(1450.0));
        assert_eq!(parse_time("4:7:30"), Some(247.5));
        assert_eq!(parse_time("4"), None);
        assert_eq!(parse_time("4:61"), None);
        assert_eq!(fmt_time(247.0), "4:07");
        assert_eq!(fmt_time(247.5), "4:07:30");
    }

    #[test]
    fn numbers() {
        let t = |n: &str| Tour { number: n.into(), ..Default::default() };
        assert_eq!(next_number(&[t("1"), t("7"), t("x")]), "8");
        assert_eq!(next_number(&[]), "1");
    }
}

#[cfg(test)]
mod repeat_tests {
    use super::*;

    #[test]
    fn a_tour_every_twenty_minutes_until_eight() {
        let base = Tour { number: "1".into(), ai_group: "Busses".into(), extra: String::new(), trips: vec![TourTrip { trip: "a".into(), profile: 0, departure: 6.0 * 60.0 }, TourTrip { trip: "b".into(), profile: 0, departure: 6.5 * 60.0 }] };
        let mut tours = vec![base.clone()];
        let made = repeat_tour(&mut tours, &base, 20.0, 8.0 * 60.0);
        // 6:20, 6:40 … 8:00
        assert_eq!(made, 6);
        assert_eq!(tours.last().unwrap().trips[0].departure, 8.0 * 60.0);
        assert_eq!(tours.last().unwrap().trips[1].departure, 8.5 * 60.0);
        assert_eq!(tours.last().unwrap().number, "7");
    }
}
