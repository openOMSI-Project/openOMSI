//! The launcher on a phone or a tablet: fingers instead of a mouse, a keyboard that comes
//! up on the screen, a narrow rail of icons, pages that scroll as a whole, and a browser of
//! the device's storage where the desktop opens Finder or Explorer (a phone has no file
//! dialog that gives a program a path).
//!
//! `OMSI_MOBILE=1` gives the desktop launcher the same layout (with `OMSI_LAUNCHER_SIZE`
//! the size of a phone), so that it can be looked at without a phone.

use super::theme::*;
use super::ui::ButtonKind;
use super::{Launcher, Page};
use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Rect, Weight};
use std::path::{Path, PathBuf};
use winit::event::{Touch, TouchPhase};

/// Width of the rail of icons on a phone.
pub const RAIL_W_MOBILE: f32 = 64.0;
/// The height the pages are laid out for on a phone (they scroll within the screen).
pub const PAGE_H: f32 = 700.0;
/// How far a finger moves (interface points) before a touch is a drag, not a tap.
const SLOP: f32 = 9.0;

/// Whether the launcher is laid out for fingers.
pub fn mobile() -> bool {
    crate::platform::MOBILE || omsi_cfg::env::var_os("OMSI_MOBILE").is_some()
}

/// The fingers on the launcher.
#[derive(Default)]
pub struct Fingers {
    /// The finger that works the interface: its id, where it went down, and whether it has
    /// become a drag (a scroll, or turning the bus).
    main: Option<(u64, Vec2, bool)>,
    /// Where each finger is (interface points), for the pinch over the bus.
    at: Vec<(u64, Vec2)>,
    pinch: Option<f32>,
    /// The finger came up last frame: the pointer leaves the screen this frame (no hover
    /// stays behind where it was).
    lift: bool,
}

/// What the storage browser chooses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// The OMSI 2 folder (Setup).
    Root,
    /// A mod as a folder.
    ModFolder,
    /// A mod archive (.zip, .7z or .rar).
    ModZip,
}

/// The browser of the device's storage.
pub struct Browser {
    pub purpose: Purpose,
    pub dir: PathBuf,
    /// (name, is a folder, bytes)
    entries: Vec<(String, bool, u64)>,
    /// The folder is a complete OMSI 2 (Root only).
    is_root: bool,
    error: Option<String>,
}

/// The places a phone keeps files: the shared storage and any card or stick.
pub fn storage_roots() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    let shared = PathBuf::from("/storage/emulated/0");
    if shared.is_dir() {
        v.push(("Internal storage".to_string(), shared));
    }
    if let Ok(rd) = std::fs::read_dir("/storage") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n == "emulated" || n == "self" {
                continue;
            }
            if e.path().is_dir() {
                v.push((format!("Card {n}"), e.path()));
            }
        }
    }
    if v.is_empty() {
        if let Some(h) = std::env::var_os("HOME") {
            v.push(("Home".to_string(), PathBuf::from(h)));
        }
    }
    v
}

impl Browser {
    pub fn new(purpose: Purpose, start: &str) -> Browser {
        let start = PathBuf::from(start.trim());
        let dir = if !start.as_os_str().is_empty() && start.is_dir() {
            start
        } else {
            storage_roots().first().map(|r| r.1.clone()).unwrap_or_else(|| PathBuf::from("/"))
        };
        let mut b = Browser { purpose, dir: PathBuf::new(), entries: Vec::new(), is_root: false, error: None };
        b.open(dir);
        b
    }

    fn open(&mut self, dir: PathBuf) {
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let Ok(m) = e.metadata() else { continue };
                    let archive = [".zip", ".7z", ".rar"].iter().any(|ext| name.to_ascii_lowercase().ends_with(ext));
                    if m.is_dir() || (archive && self.purpose == Purpose::ModZip) {
                        self.entries.push((name, m.is_dir(), m.len()));
                    }
                }
                self.entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
            }
            Err(e) => {
                self.error = Some(if e.kind() == std::io::ErrorKind::PermissionDenied {
                    "This folder cannot be read. Allow openOMSI access to all files (Android settings → Apps → openOMSI → Permissions → Files).".to_string()
                } else {
                    format!("{e}")
                });
            }
        }
        self.is_root = self.purpose == Purpose::Root && omsi_cfg::missing_original_essentials(&dir).is_empty();
        self.dir = dir;
    }

    fn title(&self) -> &'static str {
        match self.purpose {
            Purpose::Root => "Choose the OMSI 2 folder",
            Purpose::ModFolder => "Choose the mod folder",
            Purpose::ModZip => "Choose a mod archive (.zip, .7z, .rar)",
        }
    }
}

impl Launcher {
    /// A finger on the screen, as the mouse the interface knows: a tap is a click, a drag
    /// scrolls (the list under it, else the page), a drag over the bus turns it and two
    /// fingers over it zoom.
    pub(super) fn touch(&mut self, t: Touch, scale: f32) {
        let p = Vec2::new(t.location.x as f32, t.location.y as f32) / scale;
        self.last_input = std::time::Instant::now();
        self.ui.input.touch = true;
        match t.phase {
            TouchPhase::Started => {
                self.fingers.at.retain(|(id, _)| *id != t.id);
                self.fingers.at.push((t.id, p));
                if self.fingers.at.len() == 2 {
                    // a second finger: a pinch over the bus, never a click
                    let (a, b) = (self.fingers.at[0].1, self.fingers.at[1].1);
                    self.fingers.pinch = Some(a.distance(b).max(1.0));
                    if let Some((_, _, drag)) = self.fingers.main.as_mut() {
                        *drag = true;
                    }
                    return;
                }
                if self.fingers.main.is_some() {
                    return;
                }
                self.fingers.main = Some((t.id, p, false));
                self.fingers.lift = false;
                self.ui.input.mouse = p;
                self.ui.input.pressed = true;
                self.ui.input.down = true;
                if self.preview_rect.map(|r| r.contains(p)).unwrap_or(false) {
                    self.dragging = Some(p);
                }
            }
            TouchPhase::Moved => {
                let prev = self.fingers.at.iter().find(|(id, _)| *id == t.id).map(|f| f.1);
                if let Some(f) = self.fingers.at.iter_mut().find(|(id, _)| *id == t.id) {
                    f.1 = p;
                }
                if let (Some(d0), 2) = (self.fingers.pinch, self.fingers.at.len()) {
                    let d = self.fingers.at[0].1.distance(self.fingers.at[1].1).max(1.0);
                    self.showroom.zoom_by((d0 / d).clamp(0.8, 1.25));
                    self.fingers.pinch = Some(d);
                    return;
                }
                let Some((id, start, drag)) = self.fingers.main else { return };
                if id != t.id {
                    return;
                }
                let delta = prev.map(|q| p - q).unwrap_or(Vec2::ZERO);
                if !drag && p.distance(start) > SLOP {
                    self.fingers.main = Some((id, start, true));
                }
                if let Some(last) = self.dragging {
                    let d = p - last;
                    self.showroom.orbit(d.x, d.y);
                    self.dragging = Some(p);
                    self.ui.input.mouse = p;
                    return;
                }
                // a slider or a scroll bar held keeps the finger; anything else scrolls
                let held_slider = self.ui.active.is_some() && self.ui.input.down && !self.fingers.main.map(|m| m.2).unwrap_or(false);
                self.ui.input.mouse = p;
                if self.fingers.main.map(|m| m.2).unwrap_or(false) && !held_slider {
                    self.ui.input.wheel.y += delta.y / 42.0;
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.fingers.at.retain(|(id, _)| *id != t.id);
                if self.fingers.at.len() < 2 {
                    self.fingers.pinch = None;
                }
                let Some((id, _, drag)) = self.fingers.main else { return };
                if id != t.id {
                    return;
                }
                self.fingers.main = None;
                self.dragging = None;
                if drag || t.phase == TouchPhase::Cancelled {
                    // a scroll is not a click on what the finger came up over
                    self.ui.input.mouse = Vec2::new(-1e4, -1e4);
                } else {
                    self.ui.input.mouse = p;
                }
                self.ui.input.released = true;
                self.ui.input.down = false;
                self.fingers.lift = true;
            }
        }
    }

    /// Once a frame after the interface was drawn: a lifted finger leaves no hover behind.
    pub(super) fn touch_frame(&mut self) {
        if self.fingers.lift && !self.ui.input.released {
            self.fingers.lift = false;
            self.ui.input.mouse = Vec2::new(-1e4, -1e4);
        }
    }

    /// The narrow rail: an icon for each page.
    /// Open the storage browser (a phone's "Browse").
    pub fn browse(&mut self, purpose: Purpose, start: &str) {
        self.browser = Some(Browser::new(purpose, start));
    }

    /// The storage browser over the page, when it is open.
    pub(super) fn draw_browser(&mut self) {
        let Some(mut b) = self.browser.take() else { return };
        let size = self.ui.size;
        let full = Rect::new(0.0, 0.0, size.x, size.y);
        self.ui.solid(full);
        self.ui.p().rect(full, omsi_ui::Color::rgba(0, 0, 0, 0.72));
        let r = Rect::new(24.0, 14.0, size.x - 48.0, size.y - 28.0);
        self.ui.panel(r);
        let inner = Rect::new(r.x + 16.0, r.y + 12.0, r.w - 32.0, r.h - 24.0);
        self.ui.text_in(b.title(), Rect::new(inner.x, inner.y, inner.w - 130.0, 26.0), 17.0, Weight::Bold, TEXT, Align::Left);
        let mut close = self.ui.button("browse-cancel", Rect::new(inner.right() - 120.0, inner.y - 2.0, 120.0, 34.0), "Cancel", Some("close"), ButtonKind::Ghost);
        // the places, then the folder we are in
        let mut x = inner.x;
        let y = inner.y + 34.0;
        let mut go_to: Option<PathBuf> = None;
        if self.ui.button("browse-up", Rect::new(x, y, 44.0, 34.0), "", Some("drive_folder_upload"), ButtonKind::Normal) {
            if let Some(p) = b.dir.parent() {
                go_to = Some(p.to_path_buf());
            }
        }
        x += 52.0;
        for (k, (name, path)) in storage_roots().into_iter().enumerate() {
            let w = self.ui.width(&name, 13.0, Weight::Medium) + 48.0;
            if x + w > inner.right() {
                break;
            }
            if self.ui.button(&format!("browse-root-{k}"), Rect::new(x, y, w, 34.0), &name, Some("sd_card"), ButtonKind::Normal) {
                go_to = Some(path);
            }
            x += w + 8.0;
        }
        let path_y = y + 42.0;
        self.ui.icon("folder_open", Vec2::new(inner.x + 10.0, path_y + 11.0), 16.0, TEXT_DIM);
        let shown = self.ui.fonts.fit(&b.dir.to_string_lossy(), 12.5, Weight::Medium, inner.w - 30.0);
        self.ui.text_in(&shown, Rect::new(inner.x + 26.0, path_y, inner.w - 26.0, 22.0), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
        // what to do with this folder
        let foot_h = 46.0;
        let foot = Rect::new(inner.x, inner.bottom() - foot_h + 6.0, inner.w, foot_h - 6.0);
        let mut chosen: Option<PathBuf> = None;
        match b.purpose {
            Purpose::Root => {
                let (text, c) = if b.is_root { ("A complete OMSI 2 installation", OK) } else { ("Not an OMSI 2 folder (it needs Omsi.exe, maps and Vehicles)", TEXT_DIM) };
                self.ui.text_in(text, Rect::new(foot.x, foot.y, foot.w - 230.0, foot.h), 12.5, Weight::Medium, c, Align::Left);
                if self.ui.button("browse-use", Rect::new(foot.right() - 220.0, foot.y, 220.0, foot.h), "Use this folder", Some("check"), if b.is_root { ButtonKind::Primary } else { ButtonKind::Normal }) {
                    chosen = Some(b.dir.clone());
                }
            }
            Purpose::ModFolder => {
                if self.ui.button("browse-use", Rect::new(foot.right() - 220.0, foot.y, 220.0, foot.h), "Install this folder", Some("download"), ButtonKind::Primary) {
                    chosen = Some(b.dir.clone());
                }
            }
            Purpose::ModZip => {
                self.ui.text_in("Tap a .zip, .7z or .rar to install it", Rect::new(foot.x, foot.y, foot.w, foot.h), 12.5, Weight::Regular, TEXT_DIM, Align::Left);
            }
        }
        // the folder's contents
        let list = Rect::new(inner.x, path_y + 28.0, inner.w, foot.y - path_y - 36.0);
        self.ui.p().rounded(list, 6.0, FIELD);
        if let Some(e) = b.error.clone() {
            self.ui.paragraph(&e, Vec2::new(list.x + 12.0, list.y + 12.0), list.w - 24.0, 13.0, Weight::Regular, DANGER);
        } else if b.entries.is_empty() {
            self.ui.text_in("Nothing here", list, 13.0, Weight::Regular, TEXT_FAINT, Align::Center);
        }
        let entries = b.entries.clone();
        let mut picked: Option<(String, bool)> = None;
        let key = format!("browse-list-{}", b.dir.display());
        self.ui.scroll_area(&key, list, &mut |ui, area| {
            let row_h = 40.0;
            for (k, (name, dir, bytes)) in entries.iter().enumerate() {
                let rr = Rect::new(area.x + 4.0, area.y + 4.0 + k as f32 * row_h, area.w - 12.0, row_h - 2.0);
                if rr.bottom() < list.y || rr.y > list.bottom() {
                    continue;
                }
                if ui.row(&format!("{key}-{k}"), rr, false) {
                    picked = Some((name.clone(), *dir));
                }
                ui.icon(if *dir { "folder" } else { "inventory_2" }, Vec2::new(rr.x + 18.0, rr.center().y), 20.0, if *dir { accent() } else { TEXT_SOFT });
                ui.text_in(name, Rect::new(rr.x + 40.0, rr.y, rr.w - 140.0, rr.h), 13.5, Weight::Medium, TEXT, Align::Left);
                if !*dir {
                    ui.text_in(&super::state::fmt_bytes(*bytes), Rect::new(rr.x, rr.y, rr.w - 12.0, rr.h), 12.0, Weight::Regular, TEXT_DIM, Align::Right);
                }
            }
            8.0 + entries.len() as f32 * row_h
        });
        if let Some((name, dir)) = picked {
            let p = b.dir.join(&name);
            if dir {
                go_to = Some(p);
            } else {
                chosen = Some(p);
            }
        }
        if let Some(d) = go_to {
            b.open(d);
        }
        if self.ui.input.keys.contains(&super::ui::Key::Escape) {
            close = true;
        }
        if let Some(p) = chosen {
            self.browser_chose(b.purpose, &p);
            close = true;
        }
        if !close {
            self.browser = Some(b);
        }
    }

    fn browser_chose(&mut self, purpose: Purpose, p: &Path) {
        let s = p.to_string_lossy().to_string();
        match purpose {
            Purpose::Root => {
                self.pages.setup_root = Some(s);
                self.state.set_status("Folder chosen: press Save.", false);
            }
            Purpose::ModFolder | Purpose::ModZip => {
                self.page = Page::Mods;
                self.state.install(s);
            }
        }
    }
}
