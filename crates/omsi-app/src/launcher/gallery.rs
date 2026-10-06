//! The Bus gallery: every bus as a card with its picture, a search over them and the
//! favourites. A card clicked chooses the bus; the foot under the grid then drives it,
//! paints it (the Livery studio) or turns it round on the Drive page.
//!
//! The pictures are the game's own renderer's (the showroom's: model, paint, materials),
//! taken one bus after the other by a showroom of their own while the page is open, and
//! kept as PNG files in `~/.openomsi/thumbs` - a bus is pictured once, and again only when
//! its file changes. Only the cards in view ask for theirs.

use super::theme::*;
use super::ui::{id_of, ButtonKind};
use super::{showroom, Launcher, Page};
use glam::Vec2;
use omsi_launcher_lib::{display_bus_name, VehicleInfo};
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The pictures' size in pixels (a card shows it at half that or less).
pub const THUMB_W: u32 = 560;
pub const THUMB_H: u32 = 300;

#[derive(Default)]
pub struct GalleryView {
    pub filter: String,
    pub only_favourites: bool,
    /// The pictures on the GPU, by key (see `key_of`), and those read or taken but not yet
    /// uploaded.
    pub tex: HashMap<String, usize>,
    pub pending: Vec<(String, image::RgbaImage)>,
    /// When each picture came (the interface's clock): it fades in over the card.
    pub arrived: HashMap<String, f32>,
    /// The buses asked for this frame (in view, no picture yet), in the order they stand.
    wanted: Vec<(String, VehicleInfo)>,
    /// The bus being pictured now: its key, the look, and how long it has been waited for.
    current: Option<(String, showroom::Look, f32)>,
    /// Keys whose picture could not be taken (not asked again this run).
    failed: HashSet<String>,
    /// Keys whose file was looked for already.
    looked: HashSet<String>,
}

impl GalleryView {
    /// The device went: every picture is uploaded again from its file.
    pub fn drop_gpu(&mut self) {
        self.tex.clear();
        self.pending.clear();
        self.looked.clear();
        self.current = None;
    }
}

/// A bus's picture's key: its file and when that changed (another version is pictured anew).
fn key_of(root: &str, v: &VehicleInfo) -> String {
    let path = omsi_cfg::resolve_path(std::path::Path::new(root), &v.file);
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in v.file.to_lowercase().replace('\\', "/").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}-{stamp}")
}

fn thumbs_dir() -> PathBuf {
    omsi_launcher_lib::data_dir().join("thumbs")
}

/// Per frame, from the launcher with the renderer: the next bus is loaded into the gallery's
/// own showroom, a bus loaded is pictured, and files found are read.
pub fn pump(l: &mut super::Launcher, renderer: &mut omsi_render::Renderer, dt: f32) {
    let g = &mut l.gallery;
    // files already there: read (a few a frame)
    let mut read = 0;
    let wanted = std::mem::take(&mut g.wanted);
    for (key, _) in &wanted {
        if read >= 4 {
            break;
        }
        if g.tex.contains_key(key) || g.looked.contains(key) {
            continue;
        }
        g.looked.insert(key.clone());
        let file = thumbs_dir().join(format!("{key}.png"));
        if let Ok(img) = image::open(&file) {
            g.pending.push((key.clone(), img.to_rgba8()));
            read += 1;
        }
    }
    // the bus being pictured: there yet?
    if let Some((key, look, waited)) = g.current.as_mut() {
        *waited += dt;
        l.thumbs.update(renderer, dt);
        if l.thumbs.shows(look) {
            if let Some(img) = l.thumbs.snapshot(renderer, THUMB_W, THUMB_H) {
                let dir = thumbs_dir();
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = img.save(dir.join(format!("{key}.png"))) {
                    log::warn!("gallery: picture of {} not kept: {e}", look.bus);
                }
                g.pending.push((key.clone(), img));
            }
            g.current = None;
        } else if *waited > 40.0 || (*waited > 1.0 && l.thumbs.failed_on(look)) {
            log::info!("gallery: no picture of {}", look.bus);
            g.failed.insert(key.clone());
            g.current = None;
        }
        return;
    }
    // the next one in view without a picture
    let next = wanted.into_iter().find(|(k, _)| !g.tex.contains_key(k) && !g.failed.contains(k) && g.looked.contains(k) && !g.pending.iter().any(|p| &p.0 == k));
    if let Some((key, v)) = next {
        let c = &l.state.choice;
        let look = showroom::Look { root: PathBuf::from(&l.state.config.root), map: c.map.clone(), bus: v.file.clone(), paint: String::new(), weather: String::new(), time: 13 * 60, date: c.date.clone() };
        if look.map.is_empty() {
            return;
        }
        l.thumbs.set_view(242.0, 7.0, 0.92);
        l.thumbs.want(look.clone());
        l.thumbs.update(renderer, dt);
        g.current = Some((key, look, 0.0));
    }
}

/// The buses as the gallery lists them: by maker and name, the search and the stars applied.
fn listed(l: &mut Launcher) -> Vec<VehicleInfo> {
    let q = l.gallery.filter.trim().to_lowercase();
    let favs = super::drive::favourites(&mut l.drive);
    let mut v: Vec<VehicleInfo> = l
        .state
        .vehicles
        .iter()
        .filter(|v| q.is_empty() || display_bus_name(&v.name).to_lowercase().contains(&q) || v.manufacturer.to_lowercase().contains(&q) || v.file.to_lowercase().contains(&q))
        .filter(|v| !l.gallery.only_favourites || favs.contains(&v.file.replace('\\', "/").to_lowercase()))
        .cloned()
        .collect();
    v.sort_by(|a, b| a.manufacturer.to_lowercase().cmp(&b.manufacturer.to_lowercase()).then_with(|| super::drive::natural(&a.name).cmp(&super::drive::natural(&b.name))));
    v
}

pub fn draw(l: &mut Launcher, area: Rect) {
    let body = l.page_title(area, "Bus gallery", "Every bus you have, pictured by the game's own renderer. Choose one to drive it or to paint it.");
    // the search and the stars, top right
    let sw = (body.w * 0.3).clamp(220.0, 340.0);
    let search = Rect::new(area.right() - sw, area.y + 6.0, sw, ROW);
    if l.ui.text_input("gallery-filter", search, &mut l.gallery.filter, "Search buses…", Some("search")) {
        l.ui.scroll.remove(&id_of("gallery-grid"));
    }
    let fav_w = l.ui.width(&omsi_ui::tr("Favourites only"), 13.0, Weight::Regular) + 50.0;
    let mut only = l.gallery.only_favourites;
    if l.ui.toggle("gallery-favourites", Rect::new(search.x - fav_w - 16.0, search.y + 7.0, fav_w, 22.0), &mut only, "Favourites only") {
        l.gallery.only_favourites = only;
        l.ui.scroll.remove(&id_of("gallery-grid"));
    }

    let buses = listed(l);
    let foot_h = 72.0;
    let grid = Rect::new(body.x, body.y, body.w, (body.h - foot_h - 12.0).max(120.0));
    let cols = ((grid.w + 16.0) / 290.0).floor().clamp(2.0, 6.0) as usize;
    let gap = 16.0;
    let cw = (grid.w - 14.0 - gap * (cols - 1) as f32) / cols as f32;
    let ch = cw * (THUMB_H as f32 / THUMB_W as f32) + 58.0;
    let root = l.state.config.root.clone();
    let chosen = l.state.choice.bus.clone();
    let loading = l.state.loading_content;
    let tex = l.gallery.tex.clone();
    let favs = super::drive::favourites(&mut l.drive);
    let current = l.gallery.current.as_ref().map(|c| c.0.clone());
    let mut pick: Option<String> = None;
    let mut star: Option<String> = None;
    let mut wanted: Vec<(String, VehicleInfo)> = Vec::new();
    let time = l.ui.time;
    let since = l.page_t;
    let arrived = l.gallery.arrived.clone();
    l.ui.scroll_area("gallery-grid", grid, &mut |ui, view| {
        if buses.is_empty() {
            ui.text_in(if loading { "Reading the buses…" } else { "No buses found. Try another search." }, Rect::new(view.x, view.y + 20.0, view.w, 30.0), 13.0, Weight::Regular, TEXT_DIM(), Align::Center);
            return 60.0;
        }
        for (k, v) in buses.iter().enumerate() {
            let (col, row) = (k % cols, k / cols);
            let r = Rect::new(view.x + col as f32 * (cw + gap), view.y + 4.0 + row as f32 * (ch + gap), cw, ch);
            if !ui.rect_visible(r) {
                continue;
            }
            let key = key_of(&root, v);
            // (the cards come in one after the other, rising)
            let a = appear(since, 0.05 + (k as f32 * 0.035).min(0.6), 0.45);
            let r = Rect::new(r.x, r.y + 24.0 * (1.0 - a), r.w, r.h);
            let id = id_of(&format!("gallery-card-{}", v.file));
            let (h, held, clicked) = ui.interact(id, r);
            if clicked {
                pick = Some(v.file.clone());
            }
            let t = ui.anim(id, if h { 1.0 } else { 0.0 }, 0.07);
            let sel = v.file == chosen;
            let r = if held { r.inset(1.0) } else { Rect::new(r.x, r.y - 2.0 * t, r.w, r.h) };
            if t > 0.01 {
                ui.p().shadow(Rect::new(r.x, r.y + 6.0, r.w, r.h), RADIUS, 20.0, SHADOW().alpha(t));
            }
            ui.p().rounded(r, RADIUS, if sel { SELECTED() } else { PANEL().mix(HOVER(), t * 0.6) });
            let pic = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, r.h - 64.0);
            match tex.get(&key) {
                Some(&t) => {
                    ui.image(pic, t, RADIUS - 3.0);
                    let b = appear(time - arrived.get(&key).copied().unwrap_or(-10.0), 0.0, 0.5);
                    if b < 1.0 {
                        ui.p().rounded(pic, RADIUS - 3.0, FIELD().alpha(1.0 - b));
                    }
                }
                None => {
                    ui.p().rounded(pic, RADIUS - 3.0, FIELD());
                    // (a bus waiting for its picture: an outline of one, and the spinner on the
                    // one being taken)
                    let c = pic.center();
                    ui.icon("directions_bus", c, pic.h * 0.4, TEXT_FAINT().alpha(0.5));
                    if current.as_deref() == Some(key.as_str()) {
                        let a = time * 5.0;
                        ui.p().arc(Vec2::new(pic.right() - 16.0, pic.y + 16.0), 6.0, 8.0, a, a + 4.2, TEXT_SOFT());
                    }
                    wanted.push((key.clone(), v.clone()));
                }
            }
            let name = display_bus_name(&v.name);
            ui.text_in(&name, Rect::new(r.x + 12.0, pic.bottom() + 8.0, r.w - 50.0, 20.0), 13.5, Weight::Bold, TEXT(), Align::Left);
            ui.tooltip(Rect::new(r.x, pic.bottom(), r.w - 40.0, 50.0), &format!("{name}\n{}", v.file));
            let sub = format!("{} · {}", if v.manufacturer.is_empty() { omsi_ui::tr("Bus").to_string() } else { v.manufacturer.clone() }, super::drive::liveries_text(v.paints.len() + 1));
            ui.text_in(&sub, Rect::new(r.x + 12.0, pic.bottom() + 28.0, r.w - 50.0, 18.0), 11.5, Weight::Regular, TEXT_DIM(), Align::Left);
            // the star
            let fav = favs.contains(&v.file.replace('\\', "/").to_lowercase());
            let sr = Rect::new(r.right() - 36.0, pic.bottom() + 12.0, 28.0, 28.0);
            let (hs, _, cs) = ui.interact(id_of(&format!("gallery-star-{}", v.file)), sr);
            if cs {
                star = Some(v.file.clone());
                pick = None;
            }
            ui.icon("star", sr.center(), 18.0, if fav { ACCENT_2() } else if hs { TEXT_SOFT() } else { LIFT().alpha(0.18) });
            if v.installed {
                ui.badge(Vec2::new(pic.x + 8.0, pic.y + 8.0), "MOD", ACCENT());
            }
            if !v.missing_packs.is_empty() {
                ui.badge(Vec2::new(pic.x + 8.0, pic.bottom() - 26.0), "PARTS MISSING", WARN());
            }
            ui.p().rounded_border(r, RADIUS, if sel { 2.0 } else { 1.0 }, if sel { ACCENT() } else { EDGE().mix(ACCENT(), t * 0.7) });
            if a < 1.0 {
                ui.p().rounded(r.inset(-1.0), RADIUS, BACKDROP().alpha(1.0 - a));
            }
        }
        let rows = buses.len().div_ceil(cols);
        rows as f32 * (ch + gap) + 8.0
    });
    l.gallery.wanted = wanted;
    if let Some(file) = star {
        super::drive::toggle_favourite(&mut l.drive, &file);
    }
    if let Some(file) = pick {
        l.state.select_bus(&file);
    }

    // the foot: the chosen bus and where it goes from here
    let foot = Rect::new(body.x, grid.bottom() + 12.0, body.w, foot_h);
    l.ui.panel(foot);
    let count = format!("{} {}", buses.len(), omsi_ui::tr(if buses.len() == 1 { "bus" } else { "buses" }));
    match l.state.bus().cloned() {
        Some(v) => {
            l.ui.icon("directions_bus", Vec2::new(foot.x + 30.0, foot.center().y), 24.0, ACCENT());
            l.ui.text_in(&display_bus_name(&v.name), Rect::new(foot.x + 54.0, foot.y + 14.0, foot.w * 0.4, 22.0), 15.0, Weight::Bold, TEXT(), Align::Left);
            let paint = if l.state.choice.paint.is_empty() { super::drive::default_livery_label(&v).to_string() } else { l.state.choice.paint.clone() };
            l.ui.text_in(&format!("{paint} · {count}"), Rect::new(foot.x + 54.0, foot.y + 38.0, foot.w * 0.4, 18.0), 12.0, Weight::Regular, TEXT_DIM(), Align::Left);
            let bw = 170.0;
            let by = foot.center().y - 22.0;
            let go = Rect::new(foot.right() - bw - 14.0, by, bw, 44.0);
            if l.ui.button("gallery-drive", go, "Drive it", Some("play_arrow"), ButtonKind::Primary) {
                l.drive.tab = 1;
                l.go(Page::Drive);
            }
            let paint_r = Rect::new(go.x - bw - 10.0, by, bw, 44.0);
            if l.ui.button("gallery-paint", paint_r, "Paint it", Some("format_paint"), ButtonKind::Normal) {
                l.go(Page::Livery);
            }
            let look_r = Rect::new(paint_r.x - bw - 10.0, by, bw, 44.0);
            if l.ui.button("gallery-look", look_r, "Look at it", Some("3d_rotation"), ButtonKind::Ghost) {
                l.drive.tab = 0;
                l.go(Page::Drive);
            }
        }
        None => {
            l.ui.text_in(&count, Rect::new(foot.x + 20.0, foot.y, foot.w - 40.0, foot.h), 13.0, Weight::Regular, TEXT_DIM(), Align::Left);
        }
    }
}
