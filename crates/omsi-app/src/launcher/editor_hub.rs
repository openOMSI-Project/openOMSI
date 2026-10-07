//! The editor hub: what a player makes of a map rather than drives on it, each a tile - the
//! line editor (lines of their own, `lineeditor`), the depot editor (depot files of their own,
//! `depoteditor`), the livery editor, the timetable (the Timetable page) and the map's objects
//! (the game's own object editor, started on the map chosen here). It opens from the fourth
//! tile of the start, as Omsi-Hub's bus company opens from the tile beside its ways to drive.

use super::theme::*;
use super::ui::{id_of, ButtonKind};
use super::{Launcher, Page};
use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

#[derive(Default)]
pub struct HubView {
    /// The map the object editor starts on (an index into the map list), and whether it was
    /// set from the Drive page's choice yet.
    pub map: usize,
    picked: bool,
}

/// The livery editor. (Wired to its page by the integration; until then it says so.)
pub fn open_livery(l: &mut Launcher) {
    super::livery::open(l, None, None);
}

const TILES: [(&str, &str, &str); 5] = [
    ("Line editor", "route", "Compose a line of your own on any map: click its stops, the way between them is found over the roads. You and the timetable's buses drive it."),
    ("Depot editor", "departure_board", "A depot file of your own: the destinations and what each display shows, the IBIS's stops and routes, special and service trips. A line can choose it, and every bus that drives the line gets it."),
    ("Livery editor", "palette", "Paint a bus in colours of your own and drive it."),
    ("Timetable", "schedule", "The map's lines: their tours and when each trip leaves."),
    ("Map objects", "open_with", "The game's object editor on a map: move, turn, add and delete its objects and shape the ground. The game starts with the editor on."),
];

pub fn draw(l: &mut Launcher, area: Rect) {
    let cols: usize = if area.w >= 1100.0 { 3 } else if area.w >= 700.0 { 2 } else { 1 };
    let rows = TILES.len().div_ceil(cols);
    let gap = 16.0;
    let tw = (area.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let th = ((area.h - gap * (rows as f32 - 1.0)) / rows as f32).min(if cols == 1 { 190.0 } else { 260.0 });
    let mut open = None;
    for (k, (title, icon, text)) in TILES.iter().enumerate() {
        let base = Rect::new(area.x + (k % cols) as f32 * (tw + gap), area.y + (k / cols) as f32 * (th + gap), tw, th);
        let objects = k == TILES.len() - 1;
        // (the map objects' tile holds its own choice and button: it does not lift as a whole)
        let t = if objects { l.ui.tile_from(id_of(&format!("hub-tile-{k}")), base, SHEET_RADIUS, (false, false, false)) } else { l.ui.tile(id_of(&format!("hub-tile-{k}")), base, SHEET_RADIUS) };
        let tile = t.r;
        l.ui.tile_shadow(&t);
        l.ui.p().rounded(tile, SHEET_RADIUS, PANEL.mix(HOVER, t.hover));
        l.ui.tile_light(&t, 0.05);
        l.ui.tile_edge(&t, 1.0, EDGE);
        let badge = Rect::new(tile.x + 22.0, tile.y + 22.0, 46.0, 46.0);
        l.ui.p().rounded(badge, RADIUS, Color::WHITE.alpha(0.1).mix(accent(), 0.2 + 0.7 * t.hover));
        l.ui.icon(icon, badge.center(), 23.0, Color::WHITE.mix(on_accent(), t.hover));
        l.ui.text_in(title, Rect::new(badge.right() + 16.0, badge.y, tile.w - 110.0, badge.h), 20.0, Weight::Bold, TEXT, Align::Left);
        l.ui.paragraph(text, Vec2::new(tile.x + 22.0, badge.bottom() + 12.0), tile.w - 44.0, 13.5, Weight::Regular, TEXT_SOFT);
        if objects {
            objects_controls(l, tile);
        } else {
            l.ui.icon("chevron_right", Vec2::new(tile.right() - 30.0, tile.y + 45.0), 22.0, TEXT_DIM.mix(TEXT, t.hover));
            if t.clicked {
                open = Some(k);
            }
        }
        if k == 0 {
            super::tour::anchor("editor-lines", tile);
        }
    }
    match open {
        Some(0) => l.go(Page::Lines),
        Some(1) => {
            l.pages.depots.show("", false);
            l.go(Page::Depots);
        }
        Some(2) => open_livery(l),
        Some(3) => l.go(Page::Timetable),
        _ => {}
    }
}

/// The map objects' tile: which map, and the start (the game opens with the editor on).
fn objects_controls(l: &mut Launcher, tile: Rect) {
    let maps: Vec<(String, String)> = l.state.maps.iter().map(|m| (m.friendly.clone(), m.file.clone())).collect();
    let by = tile.bottom() - 22.0 - ROW;
    if maps.is_empty() {
        l.ui.text_in("No maps found.", Rect::new(tile.x + 22.0, by, tile.w - 44.0, ROW), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
        return;
    }
    let hub = &mut l.pages.hub;
    if !hub.picked {
        hub.picked = true;
        hub.map = maps.iter().position(|m| m.1 == l.state.choice.map).unwrap_or(0);
    }
    hub.map = hub.map.min(maps.len() - 1);
    let bw = 220.0f32.min((tile.w - 44.0) * 0.5);
    let names: Vec<String> = maps.iter().map(|m| m.0.clone()).collect();
    let mut m = hub.map;
    if l.ui.select("hub-objects-map", Rect::new(tile.x + 22.0, by, tile.w - 44.0 - bw - GAP, ROW), &mut m, &names) {
        l.pages.hub.map = m;
    }
    let busy = l.state.in_game();
    if l.ui.button("hub-objects-start", Rect::new(tile.right() - 22.0 - bw, by, bw, ROW), "Start the object editor", Some("play_arrow"), ButtonKind::Primary) && !busy {
        let file = maps[l.pages.hub.map].1.clone();
        l.state.launch_editor(&file);
    }
}
