//! The depot editor: depot files (`.hof`) of the player's own (`omsi_launcher_lib::owndepot`),
//! so that the destination displays are no puzzle. A file is made new - empty, laid out as
//! OMSI's own, or as a copy of the map's depot file, of one of the chosen bus's, or of one of
//! the player's - for a map, and kept in `~/.openomsi/depots`, never in the OMSI 2 folder.
//!
//! Its parts are tables, not text: the destinations (code, name, the text for each display, an
//! "everybody out" service trip, a special trip offered for any bus), the IBIS's stops, the
//! routes with their stops, the special and service trips (with the usual ones a click away)
//! and the file itself (its name, its map, what each text is for). The chosen destination is
//! shown lit as the front matrix shows it - and, with a bus chosen on the Drive page, in that
//! bus's own display font (`displayfont`), wider than its display said so - and what is wrong
//! with the file is said plainly under it: two destinations with one code, a route to a
//! destination the file lacks, a text too long for its display.
//!
//! A line of the line editor chooses such a file under Displays: its destinations, stops and
//! routes go into it when the line is saved (shown here under File, changed there), and every
//! bus that drives the line is given it. Chosen on the bus step, it goes to any bus - for a
//! special or a service trip.

use super::theme::*;
use super::ui::{ButtonKind, Ui};
use super::Launcher;
use glam::Vec2;
use omsi_launcher_lib as core;
use omsi_launcher_lib::linehof::{Coding, Role};
use omsi_launcher_lib::owndepot::{self, Doc, Entry, Issue, Place};
use omsi_ui::paint::Align;
use omsi_ui::{tr, Color, Rect, Weight};
use std::path::{Path, PathBuf};

const TABS: [&str; 5] = ["Destinations", "IBIS stops", "Routes", "Special trips", "File"];
/// The height of a row of the tables.
const RH: f32 = 30.0;

/// The file being edited.
struct Open {
    key: String,
    path: PathBuf,
    doc: Doc,
    coding: Coding,
    blocks: Vec<(String, String)>,
    dirty: bool,
    /// Bumped on every change (the problems are worked out again).
    rev: u64,
}

/// What a new file starts from.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    Empty,
    /// The depot file the map names.
    Map,
    File(PathBuf),
}

/// The new file's form.
struct NewForm {
    name: String,
    map: usize,
    from: usize,
    sources: Vec<(String, Source)>,
    sources_for: usize,
}

/// What the chosen destination's texts are for, worked out once a change.
#[derive(Default)]
struct Layout {
    rev: u64,
    key: String,
    labels: Vec<String>,
    limits: Vec<Option<usize>>,
    roles: Vec<Role>,
    issues: Vec<Issue>,
}

#[derive(Default)]
pub struct DepotEditorView {
    entries: Vec<Entry>,
    read: bool,
    open: Option<Open>,
    tab: usize,
    sel: usize,
    sel_stop: usize,
    sel_trip: usize,
    filter: String,
    /// The chosen destination's code as typed (its place, the text).
    code_text: (usize, String),
    form: Option<NewForm>,
    delete_armed: bool,
    white: bool,
    /// The line number on the preview.
    line: String,
    layout: Layout,
    /// A file to open (the line editor's "Open in the depot editor"), and whether the line
    /// editor opened the page (its way back).
    want: Option<String>,
    from_lines: bool,
    /// The stop chosen to add to a route.
    add_stop: usize,
}

impl DepotEditorView {
    /// Open the page on the player's depot file `key` (empty: as it was).
    pub fn show(&mut self, key: &str, from_lines: bool) {
        self.read = false;
        self.from_lines = from_lines;
        if !key.trim().is_empty() {
            self.want = Some(key.trim().to_string());
        }
    }

    /// The line editor opened the page: Back goes there.
    pub fn from_lines(&self) -> bool {
        self.from_lines
    }

    /// Something of the file changed: unsaved, the problems worked out again.
    fn touched(&mut self) {
        if let Some(o) = self.open.as_mut() {
            o.dirty = true;
            o.rev += 1;
        }
    }

    /// Open the player's depot file at `path` - not over changes to another not saved yet.
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        if let Some(o) = self.open.as_ref().filter(|o| o.dirty) {
            if o.path == path {
                return Ok(());
            }
            return Err(tr("%{name} has changes that are not saved: save it first").replace("%{name}", &o.doc.hof.name));
        }
        let l = owndepot::load(path)?;
        let key = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        self.open = Some(Open { key, path: path.to_path_buf(), doc: l.doc, coding: l.coding, blocks: l.blocks, dirty: false, rev: 0 });
        (self.sel, self.sel_stop, self.sel_trip, self.delete_armed) = (0, 0, 0, false);
        self.code_text = (usize::MAX, String::new());
        self.layout = Layout::default();
        Ok(())
    }
}

fn map_folder(file: &str) -> String {
    core::lines::map_folder(file)
}

/// The friendly name of the map whose folder is `folder` ("" for none).
fn map_name(l: &Launcher, folder: &str) -> String {
    if folder.trim().is_empty() {
        return tr("any map").into_owned();
    }
    l.state.maps.iter().find(|m| map_folder(&m.file).eq_ignore_ascii_case(folder)).map(|m| if m.friendly.trim().is_empty() { m.name.clone() } else { m.friendly.clone() }).unwrap_or_else(|| folder.to_string())
}

pub fn draw(l: &mut Launcher, area: Rect) {
    if !l.pages.depots.read {
        let v = &mut l.pages.depots;
        v.entries = owndepot::list(&owndepot::dir());
        v.read = true;
        // the file asked for, else (nothing open yet) the first
        let want = v.want.take().and_then(|key| owndepot::path_of(&owndepot::dir(), &key)).or_else(|| v.open.is_none().then(|| v.entries.first().map(|e| e.path.clone())).flatten());
        if let Some(p) = want {
            if let Err(e) = v.open_path(&p) {
                l.state.set_status(e, true);
            }
        }
    }
    if area.w < 900.0 {
        l.ui.paragraph("The depot editor needs a wider window: open it on a computer.", Vec2::new(area.x, area.y), area.w, 13.0, Weight::Regular, TEXT_DIM);
        return;
    }
    let side = 280.0;
    left_panel(l, Rect::new(area.x, area.y, side, area.h));
    let r = Rect::new(area.x + side + GAP, area.y, area.w - side - GAP, area.h);
    if l.pages.depots.open.is_none() {
        l.ui.card(r);
        let inner = r.inset(20.0);
        l.ui.heading(inner, "Depot files of your own", Some("departure_board"));
        l.ui.paragraph("A depot file tells the buses' IBIS and destination displays what to show. Make one of your own: start from the map's depot file, one of the bus's or an empty one, and give its destinations, stops and routes the texts you want - each display's shown as it lights up. A line chooses it in the line editor under Displays, and every bus that drives the line is given it; chosen on the bus step, it goes to any bus, for a special or a service trip.", Vec2::new(inner.x, inner.y + 32.0), inner.w.min(720.0), 13.5, Weight::Regular, TEXT_SOFT);
        return;
    }
    right_panel(l, r);
}

// --- the list of files -----------------------------------------------------------------------

fn left_panel(l: &mut Launcher, r: Rect) {
    l.ui.card(r);
    let inner = Rect::new(r.x + 12.0, r.y + 10.0, r.w - 24.0, r.h - 20.0);
    l.ui.heading(inner, "Your depot files", None);
    let form_h = if l.pages.depots.form.is_some() { 4.0 * (18.0 + ROW + 6.0) + ROW + 12.0 } else { ROW + 8.0 };
    let list = Rect::new(inner.x - 4.0, inner.y + 26.0, inner.w + 8.0, inner.h - 26.0 - form_h);
    let entries = l.pages.depots.entries.clone();
    let names: Vec<String> = entries.iter().map(|e| map_name(l, &e.map)).collect();
    crate::mt::protect(entries.iter().map(|e| e.name.as_str()).chain(names.iter().map(String::as_str)));
    let open_key = l.pages.depots.open.as_ref().map(|o| (o.key.clone(), o.dirty));
    let mut pick = None;
    l.ui.scroll_area("de-files", list, &mut |ui, a| {
        if entries.is_empty() {
            return ui.paragraph("None yet: make one with the button below.", Vec2::new(a.x + 6.0, a.y + 4.0), a.w - 12.0, 12.5, Weight::Regular, TEXT_DIM) + 10.0;
        }
        let mut y = a.y;
        for (k, e) in entries.iter().enumerate() {
            let rr = Rect::new(a.x + 4.0, y, a.w - 8.0, 46.0);
            let chosen = open_key.as_ref().is_some_and(|o| o.0.eq_ignore_ascii_case(&e.key));
            if ui.row(&format!("de-file-{k}"), rr, chosen) {
                pick = Some(k);
            }
            let ink = if chosen { on_accent() } else { TEXT };
            let soft = if chosen { on_accent().alpha(0.8) } else { TEXT_DIM };
            ui.text_in(&e.name, Rect::new(rr.x + 10.0, rr.y + 4.0, rr.w - 30.0, 20.0), 13.0, Weight::Bold, ink, Align::Left);
            let what = tr("%{map} · %{n} destinations").replace("%{map}", &names[k]).replace("%{n}", &e.termini.len().to_string());
            ui.text_in(&what, Rect::new(rr.x + 10.0, rr.y + 23.0, rr.w - 20.0, 18.0), 11.0, Weight::Regular, soft, Align::Left);
            if chosen && open_key.as_ref().is_some_and(|o| o.1) {
                ui.p().circle(Vec2::new(rr.right() - 12.0, rr.y + 14.0), 4.0, on_accent());
            }
            y += 50.0;
        }
        y - a.y
    });
    if let Some(k) = pick {
        let p = entries[k].path.clone();
        if let Err(e) = l.pages.depots.open_path(&p) {
            l.state.set_status(e, true);
        }
    }
    let fy = list.bottom() + 8.0;
    if l.pages.depots.form.is_none() {
        if l.ui.button("de-new", Rect::new(inner.x, fy, inner.w, ROW), "New depot file", Some("add"), ButtonKind::Primary) {
            let map = l.state.maps.iter().position(|m| m.file == l.state.choice.map).unwrap_or(0);
            l.pages.depots.form = Some(NewForm { name: String::new(), map, from: if l.state.maps.get(map).is_some_and(|m| !m.hof.trim().is_empty()) { 1 } else { 0 }, sources: Vec::new(), sources_for: usize::MAX });
        }
        return;
    }
    new_form(l, Rect::new(inner.x, fy, inner.w, inner.bottom() - fy));
}

/// What a new file may start from, for the map `m` (an index into the map list).
fn sources(l: &Launcher, m: usize) -> Vec<(String, Source)> {
    let mut out = vec![(tr("Empty, laid out as OMSI's own").into_owned(), Source::Empty)];
    if let Some(map) = l.state.maps.get(m).filter(|m| !m.hof.trim().is_empty()) {
        out.push((tr("A copy of the map's: %{hof}").replace("%{hof}", map.hof.trim()), Source::Map));
    }
    if let Some(bus) = l.state.bus() {
        let path = omsi_cfg::resolve_path(Path::new(&l.state.config.root), &bus.file);
        if let Some(dir) = path.parent() {
            let name = core::display_bus_name(&bus.name);
            for f in omsi_vehicle::hof::depot_files(&omsi_vehicle::hof::depot_dir(dir)) {
                let file = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
                out.push((tr("A copy of %{bus}'s: %{file}").replace("%{bus}", &name).replace("%{file}", &file), Source::File(f)));
            }
        }
    }
    for e in &l.pages.depots.entries {
        out.push((tr("A copy of mine: %{name}").replace("%{name}", &e.name), Source::File(e.path.clone())));
    }
    out
}

fn new_form(l: &mut Launcher, r: Rect) {
    let maps: Vec<String> = l.state.maps.iter().map(|m| if m.friendly.trim().is_empty() { m.name.clone() } else { m.friendly.clone() }).collect();
    let m = l.pages.depots.form.as_ref().map(|f| f.map).unwrap_or(0);
    if l.pages.depots.form.as_ref().is_some_and(|f| f.sources_for != m) {
        let s = sources(l, m);
        if let Some(f) = l.pages.depots.form.as_mut() {
            f.from = f.from.min(s.len().saturating_sub(1));
            f.sources = s;
            f.sources_for = m;
        }
    }
    let Launcher { ui, pages, .. } = &mut *l;
    let Some(f) = pages.depots.form.as_mut() else { return };
    let (mut make, mut cancel) = (false, false);
    let mut y = r.y;
    let field = |ui: &mut Ui, y: &mut f32, label: &str| {
        ui.text_in(&tr(label).to_uppercase(), Rect::new(r.x, *y, r.w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
        *y += 18.0;
        let fr = Rect::new(r.x, *y, r.w, ROW);
        *y += ROW + 6.0;
        fr
    };
    let fr = field(ui, &mut y, "Name");
    let hint = if maps.is_empty() { String::new() } else { tr("%{map} - mine").replace("%{map}", &maps[f.map.min(maps.len() - 1)]) };
    ui.text_input("de-new-name", fr, &mut f.name, &hint, None);
    let fr = field(ui, &mut y, "For the map");
    if !maps.is_empty() {
        let mut m = f.map.min(maps.len() - 1);
        if ui.select("de-new-map", fr, &mut m, &maps) {
            f.map = m;
        }
    }
    let fr = field(ui, &mut y, "Start from");
    let labels: Vec<String> = f.sources.iter().map(|s| s.0.clone()).collect();
    if !labels.is_empty() {
        let mut k = f.from.min(labels.len() - 1);
        if ui.select("de-new-from", fr, &mut k, &labels) {
            f.from = k;
        }
    }
    let half = (r.w - GAP) * 0.5;
    if ui.button("de-new-make", Rect::new(r.x, y, half, ROW), "Make it", Some("check"), ButtonKind::Primary) {
        make = true;
    }
    if ui.button("de-new-cancel", Rect::new(r.x + half + GAP, y, half, ROW), "Cancel", None, ButtonKind::Normal) {
        cancel = true;
    }
    if cancel {
        pages.depots.form = None;
    } else if make && !maps.is_empty() {
        make_file(l, hint);
    }
}

/// The new file of the form made, and opened.
fn make_file(l: &mut Launcher, hint: String) {
    let Some(f) = l.pages.depots.form.as_ref() else { return };
    let Some(map) = l.state.maps.get(f.map).cloned() else { return };
    let name = if f.name.trim().is_empty() { hint } else { f.name.trim().to_string() };
    let folder = map_folder(&map.file);
    let source = f.sources.get(f.from).map(|s| s.1.clone()).unwrap_or(Source::Empty);
    let read = match &source {
        Source::Empty => Ok(None),
        Source::Map => {
            let found = core::depot::find_sources(map.hof.trim(), &map.file);
            match core::depot::best(&found).and_then(|i| omsi_cfg::vfs::read(&found[i].path).ok()) {
                Some(b) => Ok(Some(b)),
                None => Err(tr("The map's depot file %{hof} was not found on this computer").replace("%{hof}", map.hof.trim())),
            }
        }
        Source::File(p) => omsi_cfg::vfs::read(p).map(Some).map_err(|e| format!("{}: {e}", p.display())),
    };
    let made = read.and_then(|bytes| {
        let (mut doc, coding) = match bytes {
            Some(b) => {
                let r = owndepot::read(&b);
                (r.doc, r.coding)
            }
            None => (Doc::blank(&name, &folder), Coding::Page(omsi_cfg::codepage::CodePage::Windows1252)),
        };
        // (a copy of another of the player's keeps its special trips; of anyone else's, they
        // are marked anew)
        if !matches!(&source, Source::File(p) if p.starts_with(owndepot::dir())) {
            doc.specials.clear();
        }
        doc.hof.name = name.clone();
        doc.map = folder.clone();
        owndepot::create(&owndepot::dir(), &doc, coding)
    });
    match made {
        Ok(p) => {
            let v = &mut l.pages.depots;
            v.form = None;
            v.entries = owndepot::list(&owndepot::dir());
            l.pages.lines.own_changed();
            if let Err(e) = v.open_path(&p) {
                l.state.set_status(e, true);
                return;
            }
            crate::mt::protect([name.as_str()]);
            l.state.set_status(tr("Depot file %{name} made").replace("%{name}", &name), false);
        }
        Err(e) => l.state.set_status(e, true),
    }
}

// --- the file open ---------------------------------------------------------------------------

fn save(l: &mut Launcher) {
    let Some(o) = l.pages.depots.open.as_mut() else { return };
    o.doc.tidy();
    match owndepot::save(&o.path, &o.doc, o.coding) {
        Ok(bytes) => {
            o.dirty = false;
            let file = o.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let n = core::content_dir().map(|c| owndepot::refresh(&c, &file, &bytes)).unwrap_or(0);
            // (the bus step's tiles read the files again)
            omsi_cfg::content_changed();
            let name = o.doc.hof.name.clone();
            l.pages.depots.entries = owndepot::list(&owndepot::dir());
            l.pages.lines.own_changed();
            let text = if n > 0 { tr("%{name} saved; the copies beside %{n} bus folder(s) too").replace("%{n}", &n.to_string()) } else { tr("%{name} saved").into_owned() };
            l.state.set_status(text.replace("%{name}", &name), false);
        }
        Err(e) => l.state.set_status(format!("{}: {e}", tr("Not saved")), true),
    }
}

fn delete(l: &mut Launcher) {
    let Some(o) = l.pages.depots.open.take() else { return };
    let name = o.doc.hof.name.clone();
    match owndepot::delete(&o.path, core::content_dir().as_deref()) {
        Ok(()) => {
            l.pages.depots.entries = owndepot::list(&owndepot::dir());
            l.pages.lines.own_changed();
            l.state.set_status(tr("%{name} deleted (kept aside in the depots folder, under deleted)").replace("%{name}", &name), false);
        }
        Err(e) => {
            l.state.set_status(e, true);
            l.pages.depots.open = Some(o);
        }
    }
    l.pages.depots.delete_armed = false;
}

/// The layout and the problems of the open file, worked out again when it changed.
fn refresh_layout(l: &mut Launcher) {
    let maps_hof = {
        let Some(o) = l.pages.depots.open.as_ref() else { return };
        l.state.maps.iter().find(|m| map_folder(&m.file).eq_ignore_ascii_case(&o.doc.map)).map(|m| m.hof.clone()).unwrap_or_default()
    };
    let v = &mut l.pages.depots;
    let Some(o) = v.open.as_ref() else { return };
    if v.layout.rev == o.rev && v.layout.key == o.key && !v.layout.labels.is_empty() {
        return;
    }
    let labels = o.doc.labels();
    v.layout = Layout {
        rev: o.rev,
        key: o.key.clone(),
        limits: labels.iter().enumerate().map(|(k, (r, _))| owndepot::limit(*r, o.doc.notes.get(k).map(String::as_str).unwrap_or(""))).collect(),
        roles: labels.iter().map(|(r, _)| *r).collect(),
        labels: labels.iter().enumerate().map(|(k, (role, note))| if note.is_empty() { tr(role.label()).replace("%{n}", &(k + 1).to_string()) } else { note.chars().take(48).collect() }).collect(),
        issues: o.doc.issues(&maps_hof, o.coding),
    };
}

fn right_panel(l: &mut Launcher, r: Rect) {
    refresh_layout(l);
    l.ui.card(r);
    let inner = Rect::new(r.x + 16.0, r.y + 12.0, r.w - 32.0, r.h - 24.0);
    // the head: the name, the map, save and delete
    let maps: Vec<(String, String)> = l.state.maps.iter().map(|m| (if m.friendly.trim().is_empty() { m.name.clone() } else { m.friendly.clone() }, map_folder(&m.file))).collect();
    let (mut do_save, mut do_delete) = (false, false);
    {
        let Launcher { ui, pages, .. } = &mut *l;
        let v = &mut pages.depots;
        let armed = v.delete_armed;
        let o = v.open.as_mut().unwrap();
        let mut changed = ui.text_input("de-name", Rect::new(inner.x, inner.y, 300.0, ROW), &mut o.doc.hof.name, "The depot file's name", Some("departure_board"));
        let mut options: Vec<String> = maps.iter().map(|m| m.0.clone()).collect();
        options.push(tr("Any map").into_owned());
        let mut m = maps.iter().position(|m| m.1.eq_ignore_ascii_case(&o.doc.map)).unwrap_or(maps.len());
        if ui.select("de-map", Rect::new(inner.x + 312.0, inner.y, 230.0, ROW), &mut m, &options) {
            o.doc.map = maps.get(m).map(|x| x.1.clone()).unwrap_or_default();
            changed = true;
        }
        let bw = 120.0;
        let save_r = Rect::new(inner.right() - bw, inner.y, bw, ROW);
        if ui.button("de-save", save_r, "Save", Some("save"), if o.dirty { ButtonKind::Primary } else { ButtonKind::Normal }) {
            do_save = true;
        }
        let del_w = if armed { 170.0 } else { 110.0 };
        let del_r = Rect::new(save_r.x - GAP - del_w, inner.y, del_w, ROW);
        if ui.button("de-delete", del_r, if armed { "Really delete it?" } else { "Delete" }, Some("delete"), ButtonKind::Danger) {
            if armed {
                do_delete = true;
            } else {
                v.delete_armed = true;
            }
        }
        let state_x = inner.x + 554.0;
        let says = if o.dirty { tr("Not saved") } else { tr("Saved") };
        ui.text_in(&says, Rect::new(state_x, inner.y, (del_r.x - state_x - 8.0).max(0.0), ROW), 12.0, Weight::Medium, if o.dirty { WARN } else { TEXT_FAINT }, Align::Left);
        let file = format!("{}.hof", o.key);
        let mut tab = v.tab;
        let tw = inner.w.min(640.0);
        if ui.segmented("de-tab", Rect::new(inner.x, inner.y + ROW + 12.0, tw, ROW), &mut tab, &TABS) {
            v.tab = tab;
            v.delete_armed = false;
        }
        crate::mt::protect([file.as_str()]);
        ui.text_in(&file, Rect::new(inner.x + tw + 16.0, inner.y + ROW + 12.0, (inner.w - tw - 16.0).max(0.0), ROW), 11.5, Weight::Regular, TEXT_FAINT, Align::Right);
        if changed {
            v.touched();
        }
    }
    if do_save {
        save(l);
    }
    if do_delete {
        delete(l);
        return;
    }
    let issues_h = 104.0;
    let body = Rect::new(inner.x, inner.y + 2.0 * ROW + 26.0, inner.w, inner.h - 2.0 * ROW - 26.0 - issues_h - 10.0);
    match l.pages.depots.tab {
        0 => destinations(l, body),
        1 => stops(l, body),
        2 => routes(l, body),
        3 => specials(l, body),
        _ => file_tab(l, body),
    }
    problems(l, Rect::new(inner.x, inner.bottom() - issues_h, inner.w, issues_h));
}

/// The problems of the file under the tabs: the game's first, a click shows the one it is about.
fn problems(l: &mut Launcher, r: Rect) {
    let issues = l.pages.depots.layout.issues.clone();
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    ui.p().rect(Rect::new(r.x, r.y, r.w, 1.0), HAIRLINE);
    let serious = issues.iter().filter(|i| i.serious).count();
    let (head, c, icon) = if issues.is_empty() {
        (tr("No problems: the game reads it as it is meant").into_owned(), OK, "check_circle")
    } else if serious > 0 {
        (tr("%{n} problem(s) the game trips over, %{m} worth knowing").replace("%{n}", &serious.to_string()).replace("%{m}", &(issues.len() - serious).to_string()), DANGER, "error")
    } else {
        (tr("%{m} thing(s) worth knowing").replace("%{m}", &issues.len().to_string()), WARN, "warning")
    };
    ui.icon(icon, Vec2::new(r.x + 9.0, r.y + 20.0), 16.0, c);
    ui.text_in(&head, Rect::new(r.x + 24.0, r.y + 10.0, r.w - 24.0, 20.0), 12.5, Weight::Bold, c, Align::Left);
    let list = Rect::new(r.x - 4.0, r.y + 34.0, r.w + 8.0, r.h - 36.0);
    let mut go = None;
    ui.scroll_area("de-problems", list, &mut |ui, a| {
        let mut y = a.y;
        for (k, i) in issues.iter().enumerate() {
            let rr = Rect::new(a.x + 4.0, y, a.w - 8.0, 22.0);
            if ui.row(&format!("de-problem-{k}"), rr, false) {
                go = Some(i.at);
            }
            ui.p().circle(Vec2::new(rr.x + 9.0, rr.center().y), 3.5, if i.serious { DANGER } else { WARN });
            let text = issue_text(i);
            ui.text_in(&text, Rect::new(rr.x + 20.0, rr.y, rr.w - 24.0, rr.h), 12.0, Weight::Regular, TEXT_SOFT, Align::Left);
            y += 24.0;
        }
        y - a.y
    });
    match go {
        Some(Place::Terminus(i)) => (v.tab, v.sel, v.filter) = (0, i, String::new()),
        Some(Place::Stop(i)) => (v.tab, v.sel_stop) = (1, i),
        Some(Place::Trip(i)) => (v.tab, v.sel_trip) = (2, i),
        Some(Place::File) => v.tab = 4,
        None => {}
    }
}

/// A problem in the interface's language.
fn issue_text(i: &Issue) -> String {
    let t = i.args.iter().fold(tr(i.text).into_owned(), |t, (k, v)| t.replace(&format!("%{{{k}}}"), v));
    crate::mt::protect([t.as_str()]);
    t
}

/// The heading of a part of a tab.
fn head(ui: &mut Ui, text: &str, r: Rect) {
    ui.text_in(&tr(text).to_uppercase(), r, 10.5, Weight::Bold, TEXT_DIM, Align::Left);
}

/// The indices of the first text of each role.
fn first(roles: &[Role], role: Role) -> Option<usize> {
    roles.iter().position(|r| *r == role)
}

// --- the destinations ------------------------------------------------------------------------

fn destinations(l: &mut Launcher, body: Rect) {
    let table = Rect::new(body.x, body.y, (body.w * 0.5).floor(), body.h);
    let detail = Rect::new(table.right() + GAP + 4.0, body.y, body.w - table.w - GAP - 4.0, body.h);
    let roles = l.pages.depots.layout.roles.clone();
    let (top, bottom) = (first(&roles, Role::FrontTop), first(&roles, Role::FrontBottom));
    let ibis = first(&roles, Role::IbisDisplay).or(first(&roles, Role::ClearName)).unwrap_or(0);
    let issues = l.pages.depots.layout.issues.clone();
    {
        let Launcher { ui, pages, .. } = &mut *l;
        let v = &mut pages.depots;
        let bw = 120.0;
        ui.text_input("de-filter", Rect::new(table.x, table.y, table.w - bw - 8.0, ROW), &mut v.filter, "Find a destination or code", Some("search"));
        if ui.button("de-add", Rect::new(table.right() - bw, table.y, bw, ROW), "Add", Some("add"), ButtonKind::Normal) {
            let o = v.open.as_mut().unwrap();
            let name = tr("New destination").into_owned();
            v.sel = o.doc.add_destination(&name, false, false);
            v.filter.clear();
            v.touched();
            ui.scroll_to("de-termini", v.sel as f32 * RH, RH, table.h);
        }
        let hy = table.y + ROW + 10.0;
        let cols = [(table.x + 8.0, 52.0, "Code"), (table.x + 66.0, table.w * 0.42, "Destination"), (table.x + 74.0 + table.w * 0.42, table.w * 0.45, "Front")];
        for (x, w, t) in cols {
            head(ui, t, Rect::new(x, hy, w, 16.0));
        }
        let o = v.open.as_ref().unwrap();
        let filter = v.filter.trim().to_lowercase();
        let rows: Vec<usize> = (0..o.doc.hof.termini.len()).filter(|i| filter.is_empty() || o.doc.hof.termini[*i].texture_id.to_lowercase().contains(&filter) || o.doc.hof.termini[*i].code.to_string() == filter).collect();
        let list = Rect::new(table.x - 4.0, hy + 20.0, table.w + 8.0, table.bottom() - hy - 20.0);
        let sel = v.sel;
        let mut pick = None;
        let doc = &o.doc;
        ui.scroll_area("de-termini", list, &mut |ui, a| {
            let first_y = a.y;
            let skip = ((list.y - a.y) / RH).floor().max(0.0) as usize;
            let shown = (list.h / RH).ceil() as usize + 2;
            for (n, &i) in rows.iter().enumerate().skip(skip).take(shown) {
                let t = &doc.hof.termini[i];
                let rr = Rect::new(a.x + 4.0, first_y + n as f32 * RH, a.w - 8.0, RH - 2.0);
                if ui.row(&format!("de-t-{i}"), rr, sel == i) {
                    pick = Some(i);
                }
                let ink = if sel == i { on_accent() } else { TEXT };
                let soft = if sel == i { on_accent().alpha(0.8) } else { TEXT_DIM };
                ui.text_in(&t.code.to_string(), Rect::new(rr.x + 4.0, rr.y, 48.0, rr.h), 12.0, Weight::Bold, soft, Align::Right);
                let name = if t.texture_id.trim().is_empty() { "-".to_string() } else { t.texture_id.clone() };
                crate::mt::protect([name.as_str()]);
                ui.text_in(&name, Rect::new(rr.x + 62.0, rr.y, a.w * 0.42 - 26.0, rr.h), 12.5, Weight::Medium, ink, Align::Left);
                let front = [top, bottom].iter().flatten().filter_map(|k| t.strings.get(*k)).map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" / ");
                let front = if front.is_empty() { t.strings.get(ibis).map(|s| s.trim().to_string()).unwrap_or_default() } else { front };
                ui.text_in(&front, Rect::new(rr.x + 70.0 + a.w * 0.42, rr.y, a.w * 0.45 - 40.0, rr.h), 11.5, Weight::Regular, soft, Align::Left);
                // its marks: a special trip, a problem
                let mut mx = rr.right() - 12.0;
                if issues.iter().any(|x| x.at == Place::Terminus(i)) {
                    let c = if issues.iter().any(|x| x.at == Place::Terminus(i) && x.serious) { DANGER } else { WARN };
                    ui.icon("warning", Vec2::new(mx, rr.center().y), 13.0, if sel == i { on_accent() } else { c });
                    mx -= 18.0;
                }
                if doc.is_special(i) {
                    ui.icon("flag", Vec2::new(mx, rr.center().y), 13.0, if sel == i { on_accent() } else { accent_2() });
                }
            }
            rows.len() as f32 * RH
        });
        if let Some(i) = pick {
            v.sel = i;
            v.delete_armed = false;
        }
        if rows.is_empty() {
            let say = if doc.hof.termini.is_empty() { "No destinations yet: add one." } else { "None matches." };
            ui.text_in(say, Rect::new(list.x + 10.0, list.y + 6.0, list.w - 20.0, 20.0), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
        }
    }
    destination_detail(l, detail);
}

fn destination_detail(l: &mut Launcher, r: Rect) {
    let n = l.pages.depots.open.as_ref().map(|o| o.doc.hof.termini.len()).unwrap_or(0);
    if n == 0 {
        return;
    }
    l.pages.depots.sel = l.pages.depots.sel.min(n - 1);
    let i = l.pages.depots.sel;
    let roles = l.pages.depots.layout.roles.clone();
    let labels = l.pages.depots.layout.labels.clone();
    let limits = l.pages.depots.layout.limits.clone();
    let issues: Vec<Issue> = l.pages.depots.layout.issues.iter().filter(|x| x.at == Place::Terminus(i)).cloned().collect();
    let strings = l.pages.depots.open.as_ref().unwrap().doc.hof.termini[i].strings.clone();
    let get = |role: Role| first(&roles, role).and_then(|k| strings.get(k)).map(|s| s.trim().to_string()).unwrap_or_default();
    let (l1, l2) = match (first(&roles, Role::FrontTop), first(&roles, Role::IbisDisplay)) {
        (Some(_), _) => (get(Role::FrontTop), get(Role::FrontBottom)),
        (None, Some(_)) => (get(Role::IbisDisplay), String::new()),
        _ => (strings.first().map(|s| s.trim().to_string()).unwrap_or_default(), String::new()),
    };
    let ibis_text = { let s = get(Role::IbisDisplay); if s.is_empty() { get(Role::ClearName) } else { s } };
    let side = get(Role::Side);
    // the bus chosen on the Drive page: its own display, its own font
    let bus = l.state.bus().map(|b| (b.file.clone(), core::display_bus_name(&b.name)));
    let root = l.state.config.root.clone();
    let on_bus = bus.as_ref().and_then(|(file, _)| {
        let text = [l1.as_str(), l2.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then(|| l.state.display_fonts.preview(&root, file, &text)).flatten()
    });
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    if v.code_text.0 != i {
        v.code_text = (i, v.open.as_ref().unwrap().doc.hof.termini[i].code.to_string());
    }
    let mut changed = false;
    let mut act: Option<&str> = None;
    let mut line = v.line.clone();
    let mut white = v.white;
    let mut code_text = v.code_text.1.clone();
    let o = v.open.as_mut().unwrap();
    let doc = &mut o.doc;
    ui.scroll_area(&format!("de-detail-{}", o.key), r, &mut |ui, a| {
        let (x, w) = (a.x + 2.0, a.w - 10.0);
        let mut y = a.y + 2.0;
        // the front as it lights up, the line number beside it
        head(ui, "On the front display", Rect::new(x, y, w - 110.0, 16.0));
        ui.text_input("de-line", Rect::new(x + w - 90.0, y - 6.0, 90.0, 26.0), &mut line, "Line", None);
        y += 22.0;
        let sign = Rect::new(x + 3.0, y + 3.0, w - 6.0, 62.0);
        super::lineeditor::led_sign(ui, sign, &line, &l1, &l2, white);
        if ui.interact(super::ui::id_of("de-sign"), sign).2 {
            white = !white;
        }
        ui.tooltip(sign, "Click: amber or white");
        y += 74.0;
        // the IBIS and the side
        let half = (w - GAP) * 0.5;
        let lcd = Rect::new(x, y, half, 30.0);
        ui.p().rounded(lcd, 5.0, Color::rgba(126, 148, 96, 1.0));
        ui.p().rounded_border(lcd, 5.0, 1.0, Color::rgba(40, 48, 30, 1.0));
        ui.text_in(&ibis_text, lcd.pad(8.0, 0.0), 13.0, Weight::Bold, Color::rgba(24, 30, 18, 1.0), Align::Left);
        ui.tooltip(lcd, "The IBIS");
        let sr = Rect::new(x + half + GAP, y, half, 30.0);
        ui.p().rounded(sr, 5.0, Color::rgba(12, 12, 14, 1.0));
        let lit = if white { Color::rgba(236, 242, 255, 1.0) } else { Color::rgba(255, 172, 28, 1.0) };
        ui.text_in(&side, sr.pad(8.0, 0.0), 12.5, Weight::Bold, lit, Align::Center);
        ui.tooltip(sr, "The side display");
        y += 40.0;
        if let Some((_, name)) = &bus {
            let said = tr("On %{bus}, in its own display font").replace("%{bus}", name);
            crate::mt::protect([said.as_str()]);
            ui.text_in(&said, Rect::new(x, y, w, 16.0), 11.0, Weight::Medium, TEXT_DIM, Align::Left);
            y += 18.0;
            let big = Rect::new(x, y, w, 46.0);
            ui.p().rounded(big, 6.0, Color::rgba(10, 9, 8, 1.0));
            match on_bus.as_ref() {
                Some(p) if p.none => {
                    ui.text_in("It draws its displays as pictures of its own: no preview in a font", big, 11.5, Weight::Regular, TEXT_FAINT, Align::Center);
                    y += 50.0;
                }
                Some(p) => {
                    if let Some((tex, pw, ph)) = p.picture {
                        let room = big.inset(6.0);
                        let k = (room.h / ph as f32).min(room.w / pw as f32).min(1.0);
                        let (iw, ih) = (pw as f32 * k, ph as f32 * k);
                        ui.image(Rect::new(room.center().x - iw * 0.5, room.center().y - ih * 0.5, iw, ih), tex, 0.0);
                    } else {
                        ui.text_in("Drawing the sign…", big, 12.0, Weight::Regular, TEXT_FAINT, Align::Center);
                    }
                    y += 50.0;
                    if p.too_wide {
                        y += ui.paragraph(&tr("Wider than the display of %{bus} in its font %{font}: the end is cut off").replace("%{bus}", name).replace("%{font}", &p.font), Vec2::new(x, y), w, 11.5, Weight::Medium, WARN) + 4.0;
                    }
                }
                None => {
                    ui.text_in("Reading the bus's displays…", big, 11.5, Weight::Regular, TEXT_FAINT, Align::Center);
                    y += 50.0;
                }
            }
        }
        y += 6.0;
        // its name, code and kind
        let t = &mut doc.hof.termini[i];
        head(ui, "Destination", Rect::new(x, y, w, 16.0));
        y += 18.0;
        let cw = 90.0;
        changed |= ui.text_input(&format!("de-ident-{i}"), Rect::new(x, y, w - cw - 8.0, ROW), &mut t.texture_id, "What the timetable's trips name", None);
        if ui.text_input(&format!("de-code-{i}"), Rect::new(x + w - cw, y, cw, ROW), &mut code_text, "Code", None) {
            if let Ok(c) = code_text.trim().parse::<i32>() {
                t.code = c;
                changed = true;
            }
        }
        y += ROW + 8.0;
        let mut all_exit = t.all_exit;
        if ui.toggle(&format!("de-allexit-{i}"), Rect::new(x, y, w, 28.0), &mut all_exit, "Nobody boards: a service trip (everybody gets off)") {
            t.all_exit = all_exit;
            changed = true;
        }
        y += 32.0;
        let code = t.code;
        let mut special = doc.specials.contains(&code) || doc.hof.termini[i].all_exit;
        if ui.toggle(&format!("de-special-{i}"), Rect::new(x, y, w, 28.0), &mut special, "Offered for any bus: a special trip") {
            act = Some(if special { "special-on" } else { "special-off" });
        }
        y += 38.0;
        // its texts, one for each string of the file
        head(ui, "Texts", Rect::new(x, y, w, 16.0));
        y += 20.0;
        let t = &mut doc.hof.termini[i];
        for k in 0..t.strings.len() {
            let label = labels.get(k).cloned().unwrap_or_else(|| tr("Text %{n}").replace("%{n}", &(k + 1).to_string()));
            let count = t.strings[k].trim_end().chars().count();
            let max = limits.get(k).copied().flatten();
            ui.text_in(&label, Rect::new(x, y, w - 60.0, 16.0), 11.0, Weight::Medium, TEXT_SOFT, Align::Left);
            if let Some(m) = max {
                ui.text_in(&format!("{count}/{m}"), Rect::new(x + w - 56.0, y, 56.0, 16.0), 11.0, Weight::Bold, if count > m { WARN } else { TEXT_FAINT }, Align::Right);
            }
            y += 17.0;
            changed |= ui.text_input(&format!("de-s-{i}-{k}"), Rect::new(x, y, w, ROW - 4.0), &mut t.strings[k], "-", None);
            y += ROW + 2.0;
        }
        y += 6.0;
        for is in &issues {
            y += ui.paragraph(&issue_text(is), Vec2::new(x, y), w, 11.5, Weight::Medium, if is.serious { DANGER } else { WARN }) + 4.0;
        }
        let half = (w - GAP) * 0.5;
        if ui.button(&format!("de-dup-{i}"), Rect::new(x, y + 4.0, half, ROW), "Duplicate", Some("content_copy"), ButtonKind::Normal) {
            act = Some("dup");
        }
        if ui.button(&format!("de-del-{i}"), Rect::new(x + half + GAP, y + 4.0, half, ROW), "Delete", Some("delete"), ButtonKind::Danger) {
            act = Some("del");
        }
        y += ROW + 12.0;
        y - a.y
    });
    v.line = line;
    v.white = white;
    v.code_text.1 = code_text;
    let o = v.open.as_mut().unwrap();
    match act {
        Some("special-on") | Some("special-off") => {
            o.doc.set_special(i, act == Some("special-on"));
            changed = true;
        }
        Some("dup") => {
            let mut t = o.doc.hof.termini[i].clone();
            t.code = o.doc.free_code(t.code + 1);
            o.doc.hof.termini.insert(i + 1, t);
            v.sel = i + 1;
            changed = true;
        }
        Some("del") => {
            let code = o.doc.hof.termini.remove(i).code;
            o.doc.specials.retain(|c| *c != code);
            v.sel = i.saturating_sub(1);
            changed = true;
        }
        _ => {}
    }
    if changed {
        if let Some(o) = v.open.as_mut() {
            o.doc.tidy();
        }
        v.code_text.0 = usize::MAX;
        if act.is_none() {
            // (the code typed stays as typed)
            v.code_text.0 = i;
        }
        v.touched();
    }
}

// --- the IBIS's stops ------------------------------------------------------------------------

fn stops(l: &mut Launcher, body: Rect) {
    let issues = l.pages.depots.layout.issues.clone();
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    let table = Rect::new(body.x, body.y, (body.w * 0.5).floor(), body.h);
    let detail = Rect::new(table.right() + GAP + 4.0, body.y, body.w - table.w - GAP - 4.0, body.h);
    let mut changed = false;
    let bw = 120.0;
    ui.paragraph("The stops the IBIS names along a route: what it shows for each.", Vec2::new(table.x, table.y + 4.0), table.w - bw - 12.0, 12.0, Weight::Regular, TEXT_DIM);
    let mut add = false;
    if ui.button("de-stop-add", Rect::new(table.right() - bw, table.y, bw, ROW), "Add", Some("add"), ButtonKind::Normal) {
        add = true;
    }
    let o = v.open.as_mut().unwrap();
    if add {
        v.sel_stop = o.doc.add_stop(&tr("New stop"));
        changed = true;
    }
    let hy = table.y + ROW + 10.0;
    head(ui, "Stop", Rect::new(table.x + 8.0, hy, table.w * 0.45, 16.0));
    head(ui, "On the IBIS", Rect::new(table.x + table.w * 0.5, hy, table.w * 0.5, 16.0));
    let list = Rect::new(table.x - 4.0, hy + 20.0, table.w + 8.0, table.bottom() - hy - 20.0);
    let doc = &o.doc;
    let sel = v.sel_stop;
    let mut pick = None;
    ui.scroll_area("de-stops", list, &mut |ui, a| {
        let skip = ((list.y - a.y) / RH).floor().max(0.0) as usize;
        let shown = (list.h / RH).ceil() as usize + 2;
        for (k, b) in doc.hof.bus_stops.iter().enumerate().skip(skip).take(shown) {
            let rr = Rect::new(a.x + 4.0, a.y + k as f32 * RH, a.w - 8.0, RH - 2.0);
            if ui.row(&format!("de-b-{k}"), rr, sel == k) {
                pick = Some(k);
            }
            let ink = if sel == k { on_accent() } else { TEXT };
            ui.text_in(&b.ident, Rect::new(rr.x + 8.0, rr.y, a.w * 0.45, rr.h), 12.5, Weight::Medium, ink, Align::Left);
            ui.text_in(b.strings.first().map(String::as_str).unwrap_or(""), Rect::new(rr.x + a.w * 0.5, rr.y, a.w * 0.5 - 30.0, rr.h), 11.5, Weight::Regular, if sel == k { on_accent() } else { TEXT_DIM }, Align::Left);
            if issues.iter().any(|x| x.at == Place::Stop(k)) {
                ui.icon("warning", Vec2::new(rr.right() - 12.0, rr.center().y), 13.0, if sel == k { on_accent() } else { WARN });
            }
        }
        doc.hof.bus_stops.len() as f32 * RH
    });
    if let Some(k) = pick {
        v.sel_stop = k;
    }
    let o = v.open.as_mut().unwrap();
    let n = o.doc.hof.bus_stops.len();
    let mut del = false;
    if n > 0 {
        let k = v.sel_stop.min(n - 1);
        v.sel_stop = k;
        let notes = o.doc.stop_notes.clone();
        let b = &mut o.doc.hof.bus_stops[k];
        let (x, w) = (detail.x, detail.w);
        let mut y = detail.y;
        head(ui, "Stop", Rect::new(x, y, w, 16.0));
        y += 18.0;
        changed |= ui.text_input(&format!("de-stop-ident-{k}"), Rect::new(x, y, w, ROW), &mut b.ident, "What the routes name it", None);
        y += ROW + 10.0;
        head(ui, "Texts", Rect::new(x, y, w, 16.0));
        y += 20.0;
        for j in 0..b.strings.len() {
            let label = notes.get(j).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| if j == 0 { tr("IBIS display").into_owned() } else { tr("Text %{n}").replace("%{n}", &(j + 1).to_string()) });
            ui.text_in(&label, Rect::new(x, y, w, 16.0), 11.0, Weight::Medium, TEXT_SOFT, Align::Left);
            y += 17.0;
            changed |= ui.text_input(&format!("de-stop-s-{k}-{j}"), Rect::new(x, y, w, ROW - 4.0), &mut b.strings[j], "-", None);
            y += ROW + 2.0;
        }
        for is in issues.iter().filter(|x| x.at == Place::Stop(k)) {
            y += ui.paragraph(&issue_text(is), Vec2::new(x, y), w, 11.5, Weight::Medium, if is.serious { DANGER } else { WARN }) + 4.0;
        }
        if ui.button(&format!("de-stop-del-{k}"), Rect::new(x, y + 6.0, 160.0, ROW), "Delete", Some("delete"), ButtonKind::Danger) {
            del = true;
        }
        if del {
            o.doc.hof.bus_stops.remove(k);
            v.sel_stop = k.saturating_sub(1);
            changed = true;
        }
    }
    if changed {
        v.touched();
    }
}

// --- the routes ------------------------------------------------------------------------------

fn routes(l: &mut Launcher, body: Rect) {
    let issues = l.pages.depots.layout.issues.clone();
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    let table = Rect::new(body.x, body.y, (body.w * 0.5).floor(), body.h);
    let detail = Rect::new(table.right() + GAP + 4.0, body.y, body.w - table.w - GAP - 4.0, body.h);
    let mut changed = false;
    let bw = 120.0;
    ui.paragraph("What the IBIS types: a route's code, where it goes and its stops.", Vec2::new(table.x, table.y + 4.0), table.w - bw - 12.0, 12.0, Weight::Regular, TEXT_DIM);
    let o = v.open.as_mut().unwrap();
    if ui.button("de-trip-add", Rect::new(table.right() - bw, table.y, bw, ROW), "Add", Some("add"), ButtonKind::Normal) {
        let code = o.doc.free_route(0);
        let dest = o.doc.hof.termini.iter().find(|t| !t.all_exit).map(|t| t.code.to_string()).unwrap_or_default();
        o.doc.hof.info_trips.push(omsi_vehicle::hof::InfoTrip { code: code.to_string(), name: tr("New route").to_uppercase(), route: dest, line: (code / 100).to_string(), extra: Vec::new() });
        o.doc.tidy();
        v.sel_trip = o.doc.hof.info_trips.len() - 1;
        changed = true;
    }
    let hy = table.y + ROW + 10.0;
    for (x, w, t) in [(table.x + 8.0, 60.0, "Code"), (table.x + 76.0, 40.0, "Line"), (table.x + 124.0, table.w - 130.0, "Goes to")] {
        head(ui, t, Rect::new(x, hy, w, 16.0));
    }
    let list = Rect::new(table.x - 4.0, hy + 20.0, table.w + 8.0, table.bottom() - hy - 20.0);
    let doc = &o.doc;
    let sel = v.sel_trip;
    let mut pick = None;
    ui.scroll_area("de-trips", list, &mut |ui, a| {
        let skip = ((list.y - a.y) / RH).floor().max(0.0) as usize;
        let shown = (list.h / RH).ceil() as usize + 2;
        for (k, t) in doc.hof.info_trips.iter().enumerate().skip(skip).take(shown) {
            let rr = Rect::new(a.x + 4.0, a.y + k as f32 * RH, a.w - 8.0, RH - 2.0);
            if ui.row(&format!("de-r-{k}"), rr, sel == k) {
                pick = Some(k);
            }
            let ink = if sel == k { on_accent() } else { TEXT };
            let soft = if sel == k { on_accent().alpha(0.8) } else { TEXT_DIM };
            ui.text_in(&t.code, Rect::new(rr.x + 4.0, rr.y, 60.0, rr.h), 12.0, Weight::Bold, ink, Align::Left);
            ui.text_in(&t.line, Rect::new(rr.x + 72.0, rr.y, 44.0, rr.h), 12.0, Weight::Medium, soft, Align::Left);
            let code = omsi_cfg::parse_i32(&t.route);
            let to = doc.hof.termini.iter().find(|x| x.code == code && !t.route.trim().is_empty()).map(|x| x.texture_id.clone()).unwrap_or_else(|| format!("? {}", t.route.trim()));
            let stops = doc.hof.info_busstop_lists.get(k).map(Vec::len).unwrap_or(0);
            ui.text_in(&format!("{to} · {}", tr("%{n} stops").replace("%{n}", &stops.to_string())), Rect::new(rr.x + 120.0, rr.y, a.w - 160.0, rr.h), 11.5, Weight::Regular, soft, Align::Left);
            if issues.iter().any(|x| x.at == Place::Trip(k)) {
                let c = if issues.iter().any(|x| x.at == Place::Trip(k) && x.serious) { DANGER } else { WARN };
                ui.icon("warning", Vec2::new(rr.right() - 12.0, rr.center().y), 13.0, if sel == k { on_accent() } else { c });
            }
        }
        doc.hof.info_trips.len() as f32 * RH
    });
    if let Some(k) = pick {
        v.sel_trip = k;
    }
    let o = v.open.as_mut().unwrap();
    let n = o.doc.hof.info_trips.len();
    if n == 0 {
        if changed {
            v.touched();
        }
        return;
    }
    let k = v.sel_trip.min(n - 1);
    v.sel_trip = k;
    let (x, w) = (detail.x, detail.w);
    let mut y = detail.y;
    let termini: Vec<(i32, String)> = o.doc.hof.termini.iter().map(|t| (t.code, t.texture_id.clone())).collect();
    let stop_names: Vec<String> = o.doc.hof.bus_stops.iter().map(|b| b.ident.clone()).collect();
    let third = (w - 2.0 * GAP) / 3.0;
    for (n, label) in ["Code", "Line", "Name"].iter().enumerate() {
        head(ui, label, Rect::new(x + n as f32 * (third + GAP), y, third, 16.0));
    }
    y += 18.0;
    {
        let t = &mut o.doc.hof.info_trips[k];
        changed |= ui.text_input(&format!("de-trip-code-{k}"), Rect::new(x, y, third, ROW), &mut t.code, "Code", None);
        changed |= ui.text_input(&format!("de-trip-line-{k}"), Rect::new(x + third + GAP, y, third, ROW), &mut t.line, "Line", None);
        changed |= ui.text_input(&format!("de-trip-name-{k}"), Rect::new(x + 2.0 * (third + GAP), y, third, ROW), &mut t.name, "Name", None);
        y += ROW + 8.0;
        let options: Vec<String> = termini.iter().map(|(c, n)| format!("{c} · {n}")).collect();
        let code = omsi_cfg::parse_i32(&t.route);
        if !options.is_empty() {
            let mut s = termini.iter().position(|(c, _)| *c == code && !t.route.trim().is_empty()).unwrap_or(usize::MAX);
            let mut shown = options.clone();
            if s == usize::MAX {
                shown.push(tr("? %{code}: not in the file").replace("%{code}", t.route.trim()));
                s = shown.len() - 1;
            }
            ui.text_in(&tr("Goes to").to_uppercase(), Rect::new(x, y, w, 16.0), 10.5, Weight::Bold, TEXT_DIM, Align::Left);
            y += 18.0;
            if ui.select(&format!("de-trip-to-{k}"), Rect::new(x, y, w, ROW), &mut s, &shown) {
                if let Some((c, _)) = termini.get(s) {
                    t.route = c.to_string();
                    changed = true;
                }
            }
            y += ROW + 10.0;
        }
    }
    head(ui, "Its stops", Rect::new(x, y, w, 16.0));
    y += 20.0;
    let bottom_h = ROW + 8.0 + 40.0;
    let list = Rect::new(x - 4.0, y, w + 8.0, (detail.bottom() - y - bottom_h).max(60.0));
    let mine = o.doc.hof.info_busstop_lists[k].clone();
    let mut act: Option<(usize, i32)> = None;
    ui.scroll_area(&format!("de-trip-stops-{k}"), list, &mut |ui, a| {
        let mut yy = a.y;
        for (j, s) in mine.iter().enumerate() {
            let rr = Rect::new(a.x + 4.0, yy, a.w - 8.0, 26.0);
            ui.text_in(&format!("{}.", j + 1), Rect::new(rr.x, rr.y, 26.0, rr.h), 11.5, Weight::Bold, TEXT_FAINT, Align::Right);
            ui.text_in(s, Rect::new(rr.x + 34.0, rr.y, rr.w - 120.0, rr.h), 12.5, Weight::Medium, TEXT, Align::Left);
            let cy = rr.center().y;
            if j > 0 && ui.icon_button(&format!("de-ts-up-{k}-{j}"), Vec2::new(rr.right() - 60.0, cy), 10.0, "keyboard_arrow_up", "Earlier") {
                act = Some((j, -1));
            }
            if j + 1 < mine.len() && ui.icon_button(&format!("de-ts-down-{k}-{j}"), Vec2::new(rr.right() - 36.0, cy), 10.0, "keyboard_arrow_down", "Later") {
                act = Some((j, 1));
            }
            if ui.icon_button(&format!("de-ts-x-{k}-{j}"), Vec2::new(rr.right() - 12.0, cy), 10.0, "close", "Take it out") {
                act = Some((j, 0));
            }
            yy += 28.0;
        }
        if mine.is_empty() {
            ui.text_in("No stops yet.", Rect::new(a.x + 8.0, yy, a.w - 16.0, 22.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            yy += 24.0;
        }
        yy - a.y
    });
    let list_k = &mut o.doc.hof.info_busstop_lists[k];
    match act {
        Some((j, 0)) => {
            list_k.remove(j);
            changed = true;
        }
        Some((j, d)) => {
            list_k.swap(j, (j as i32 + d) as usize);
            changed = true;
        }
        None => {}
    }
    let by = list.bottom() + 8.0;
    if !stop_names.is_empty() {
        v.add_stop = v.add_stop.min(stop_names.len() - 1);
        let mut s = v.add_stop;
        if ui.select(&format!("de-trip-add-stop-{k}"), Rect::new(x, by, w - 128.0, ROW), &mut s, &stop_names) {
            v.add_stop = s;
        }
        if ui.button(&format!("de-trip-add-go-{k}"), Rect::new(x + w - 120.0, by, 120.0, ROW), "Add stop", Some("add"), ButtonKind::Normal) {
            o.doc.hof.info_busstop_lists[k].push(stop_names[v.add_stop].clone());
            changed = true;
        }
    } else {
        ui.text_in("Add the IBIS's stops first (IBIS stops).", Rect::new(x, by, w, ROW), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
    }
    if ui.button(&format!("de-trip-del-{k}"), Rect::new(x, by + ROW + 8.0, 160.0, 30.0), "Delete route", Some("delete"), ButtonKind::Danger) {
        o.doc.hof.info_trips.remove(k);
        o.doc.hof.info_busstop_lists.remove(k);
        v.sel_trip = k.saturating_sub(1);
        changed = true;
    }
    if changed {
        v.touched();
    }
}

// --- the special and service trips -----------------------------------------------------------

fn specials(l: &mut Launcher, body: Rect) {
    // (the usual ones are OMSI's own German names, as the depot files have them)
    crate::mt::protect(owndepot::SPECIALS.iter().map(|s| s.0));
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    let mut changed = false;
    let o = v.open.as_mut().unwrap();
    let (x, w) = (body.x, body.w.min(760.0));
    let mut y = body.y;
    y += ui.paragraph("Special and service trips are offered for any bus: choose this depot file for the bus on the Drive page (Depot file) and type the code into the IBIS. Where nobody boards, everybody gets off at the first stop.", Vec2::new(x, y), w, 12.5, Weight::Regular, TEXT_SOFT) + 12.0;
    // what the game calls a service trip
    head(ui, "The service trip", Rect::new(x, y, w, 16.0));
    y += 18.0;
    let mut options = vec![tr("None").into_owned()];
    options.extend(o.doc.hof.termini.iter().map(|t| format!("{} · {}", t.code, t.texture_id)));
    let mut s = o.doc.destination(&o.doc.hof.service_trip).map(|i| i + 1).unwrap_or(0);
    if ui.select("de-servicetrip", Rect::new(x, y, w.min(420.0), ROW), &mut s, &options) {
        o.doc.hof.service_trip = if s == 0 { String::new() } else { o.doc.hof.termini[s - 1].texture_id.clone() };
        changed = true;
    }
    ui.text_in("What a bus shows driving to and from the depot", Rect::new(x + w.min(420.0) + 12.0, y, (w - w.min(420.0) - 12.0).max(0.0), ROW), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
    y += ROW + 16.0;
    // the ones the file has, and the usual ones a click away (scrolling)
    let list: Vec<usize> = (0..o.doc.hof.termini.len()).filter(|i| o.doc.is_special(*i)).collect();
    let mut go = None;
    let mut add = None;
    let doc = &o.doc;
    ui.scroll_area("de-specials", Rect::new(x - 4.0, y, w + 8.0, body.bottom() - y), &mut |ui, a| {
        let (x, w) = (a.x + 4.0, a.w - 8.0);
        let mut y = a.y;
        head(ui, "In this file", Rect::new(x, y, w, 16.0));
        y += 20.0;
        for i in &list {
            let t = &doc.hof.termini[*i];
            let rr = Rect::new(x, y, w, 30.0);
            if ui.row(&format!("de-sp-{i}"), rr, false) {
                go = Some(*i);
            }
            ui.icon(if t.all_exit { "logout" } else { "flag" }, Vec2::new(rr.x + 14.0, rr.center().y), 15.0, accent_2());
            ui.text_in(&t.code.to_string(), Rect::new(rr.x + 30.0, rr.y, 50.0, rr.h), 12.5, Weight::Bold, TEXT, Align::Right);
            ui.text_in(&t.texture_id, Rect::new(rr.x + 92.0, rr.y, w * 0.4, rr.h), 12.5, Weight::Medium, TEXT, Align::Left);
            let kind = if t.all_exit { tr("nobody boards") } else { tr("special trip") };
            ui.text_in(&kind, Rect::new(rr.x + 100.0 + w * 0.4, rr.y, w * 0.5 - 100.0, rr.h), 11.5, Weight::Regular, TEXT_DIM, Align::Left);
            y += 32.0;
        }
        if list.is_empty() {
            ui.text_in("None yet.", Rect::new(x, y, w, 22.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
            y += 26.0;
        }
        y += 10.0;
        head(ui, "Add the usual ones", Rect::new(x, y, w, 16.0));
        y += 22.0;
        let mut bx = x;
        for (k, (name, all_exit)) in owndepot::SPECIALS.iter().enumerate() {
            if doc.destination(name).is_some() {
                continue;
            }
            let bw = ui.width(name, 13.0, Weight::Bold) + 52.0;
            if bx + bw > x + w {
                bx = x;
                y += ROW + 8.0;
            }
            if ui.button(&format!("de-sp-add-{k}"), Rect::new(bx, y, bw, ROW), name, Some("add"), ButtonKind::Normal) {
                add = Some((*name, *all_exit));
            }
            bx += bw + 8.0;
        }
        y + ROW + 8.0 - a.y
    });
    if let Some((name, all_exit)) = add {
        v.sel = o.doc.add_destination(name, all_exit, true);
        changed = true;
    }
    if let Some(i) = go {
        (v.tab, v.sel, v.filter) = (0, i, String::new());
    }
    if changed {
        v.touched();
    }
}

// --- the file itself -------------------------------------------------------------------------

fn file_tab(l: &mut Launcher, body: Rect) {
    let Launcher { ui, pages, .. } = l;
    let v = &mut pages.depots;
    let mut changed = false;
    let o = v.open.as_mut().unwrap();
    let blocks = o.blocks.clone();
    let doc = &mut o.doc;
    let mut count: Option<usize> = None;
    ui.scroll_area("de-file", body, &mut |ui, a| {
        let (x, w) = (a.x + 2.0, a.w.min(760.0) - 10.0);
        let mut y = a.y + 2.0;
        head(ui, "Texts of a destination", Rect::new(x, y, w, 16.0));
        y += 20.0;
        let n = doc.hof.string_count_terminus;
        ui.text_in(&tr("%{n} texts, one for each display: what each is for").replace("%{n}", &n.to_string()), Rect::new(x, y, w - 80.0, ROW), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
        if ui.icon_button("de-count-less", Vec2::new(x + w - 50.0, y + ROW * 0.5), 12.0, "remove", "One text less (the last of every destination goes)") && n > 1 {
            count = Some(n - 1);
        }
        if ui.icon_button("de-count-more", Vec2::new(x + w - 14.0, y + ROW * 0.5), 12.0, "add", "One text more") {
            count = Some(n + 1);
        }
        y += ROW + 4.0;
        y += ui.paragraph("A bus's scripts read the texts by their place: keep the order the map's buses expect (OMSI's own: IBIS, front line 1 and 2, side, roller blind, IBIS 2, extra sign, picture).", Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_DIM) + 8.0;
        for k in 0..doc.notes.len() {
            ui.text_in(&tr("Text %{n}").replace("%{n}", &(k + 1).to_string()), Rect::new(x, y, 70.0, ROW - 4.0), 12.0, Weight::Bold, TEXT_DIM, Align::Left);
            changed |= ui.text_input(&format!("de-note-{k}"), Rect::new(x + 76.0, y, w - 76.0, ROW - 4.0), &mut doc.notes[k], "What it is for", None);
            y += ROW;
        }
        y += 12.0;
        head(ui, "Global texts", Rect::new(x, y, w, 16.0));
        y += 20.0;
        y += ui.paragraph("Folders of the announcements, the roller blinds and the side signs, and the IBIS's special codes, as the bus's scripts read them.", Vec2::new(x, y), w, 11.5, Weight::Regular, TEXT_DIM) + 6.0;
        for k in 0..doc.hof.global_strings.len() {
            ui.text_in(&format!("{k}"), Rect::new(x, y, 24.0, ROW - 4.0), 12.0, Weight::Bold, TEXT_DIM, Align::Left);
            changed |= ui.text_input(&format!("de-global-{k}"), Rect::new(x + 30.0, y, w - 30.0, ROW - 4.0), &mut doc.hof.global_strings[k], "-", None);
            y += ROW;
        }
        if ui.button("de-global-add", Rect::new(x, y + 2.0, 170.0, 30.0), "One more", Some("add"), ButtonKind::Ghost) {
            doc.hof.global_strings.push(String::new());
            changed = true;
        }
        y += 44.0;
        head(ui, "From your lines", Rect::new(x, y, w, 16.0));
        y += 20.0;
        if blocks.is_empty() {
            y += ui.paragraph("No line has chosen this depot file yet: in the line editor, under Displays.", Vec2::new(x, y), w, 12.0, Weight::Regular, TEXT_DIM) + 6.0;
        }
        for (map, body) in &blocks {
            let h = owndepot::block_entries(doc, body);
            let routes: Vec<String> = h.info_trips.iter().map(|t| format!("{} ({})", t.code, t.line)).collect();
            let text = tr("Map %{map}: %{d} destination(s), %{s} stop(s), routes %{r} - written when a line is saved; change them in the line editor").replace("%{map}", map).replace("%{d}", &h.termini.len().to_string()).replace("%{s}", &h.bus_stops.len().to_string()).replace("%{r}", &routes.join(", "));
            crate::mt::protect([text.as_str()]);
            y += ui.paragraph(&text, Vec2::new(x, y), w, 12.0, Weight::Regular, TEXT_SOFT) + 6.0;
        }
        y - a.y + 8.0
    });
    if let Some(n) = count {
        o.doc.set_string_count(n);
        changed = true;
    }
    if changed {
        v.touched();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every text of the page is translated (`locales/hof.yml`).
    #[test]
    fn the_depot_editor_speaks_every_language() {
        let mut keys: Vec<&str> = TABS.to_vec();
        keys.extend([
            "Depot editor",
            "Depot files of your own",
            "Your depot files",
            "New depot file",
            "Make it",
            "Start from",
            "For the map",
            "Not saved",
            "Saved",
            "Really delete it?",
            "On the front display",
            "Nobody boards: a service trip (everybody gets off)",
            "Offered for any bus: a special trip",
            "No problems: the game reads it as it is meant",
            "Add the usual ones",
            "From your lines",
            "%{map} · %{n} destinations",
        ]);
        let issues = {
            let mut d = Doc::blank("", "");
            let a = d.add_destination("A", false, false);
            d.add_destination("A", false, false);
            d.hof.termini[a].code = -1;
            d.hof.termini[a].strings[0] = "x".repeat(30);
            d.add_destination("", false, false);
            d.hof.bus_stops.push(omsi_vehicle::hof::BusStop { ident: String::new(), strings: vec!["x".repeat(20)] });
            d.hof.info_trips.push(omsi_vehicle::hof::InfoTrip { code: "x".into(), route: "9".into(), ..Default::default() });
            d.hof.info_busstop_lists.push(vec![]);
            d.hof.service_trip = "Nowhere".into();
            d.issues("", Coding::Utf16)
        };
        let texts: Vec<&str> = issues.iter().map(|i| i.text).collect();
        assert!(texts.len() >= 10, "{texts:?}");
        keys.extend(texts.iter().copied());
        // and every text of the table itself, in all six
        let table = include_str!("../../locales/hof.yml");
        let own: Vec<&str> = table.lines().filter_map(|l| l.strip_prefix('"').and_then(|l| l.strip_suffix("\":"))).collect();
        assert!(own.len() > 100);
        keys.extend(own);
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in &keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
    }
}
