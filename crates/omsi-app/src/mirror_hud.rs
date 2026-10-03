//! Copies of the bus's mirrors laid over the picture, in panels the driver arranges.
//!
//! The pictures the game draws for the bus's mirror glass (`reflexionN.bmp`) are render
//! textures; each panel shows one of them in a rectangle of the window. The layout is kept
//! per bus in `mirror_hud.cfg` next to the settings.
//!
//! In the cab:
//! * **Ctrl+M** shows or hides the panels;
//! * **Ctrl+Shift+M** starts and ends the *editor*. Only while it is on does the mouse
//!   work on the panels (the left button drags one, the wheel resizes it, Shift+wheel makes
//!   it wider or narrower) and a few keys (the arrows aim the mirror of the panel under the
//!   cursor, Insert adds a panel, Delete takes it away, C shows another mirror in it, Esc ends
//!   the editor). With the editor off the panels are only pictures and everything works as in
//!   the game without them.

use crate::player::Player;
use crate::scene::{MirrorGlass, World};
use glam::Vec3;
use omsi_render::{Renderer, Scene, TextureId};
use winit::keyboard::KeyCode;

/// `mirror_hud` setting: the panels a bus without a saved layout starts with.
pub(crate) const RIGHT: u8 = 1;
pub(crate) const LEFT: u8 = 2;

pub(crate) const HINT: &str = "Mirror editor: drag/wheel = move/size, arrows = aim, Alt+arrows and PgUp/PgDn = shift the mirror, -/+ = field of view, R = reset (Shift+R: all), Insert/Delete/C = add/remove/other, Ctrl+Shift+M = done";

const MIN_H: f32 = 0.08;
const MAX_H: f32 = 0.70;
const MIN_ASPECT: f32 = 0.25;
const MAX_ASPECT: f32 = 4.0;
const MARGIN: f32 = 0.012;

/// How a panel lays its picture out: the render texture is drawn for the mirror's glass, and
/// the glass's mesh puts it on with its own turn (the usual side mirror shows it left to right
/// over, one in a roof housing may have it on its side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Turn {
    /// The picture's u runs up or down the panel (and its v across).
    pub transposed: bool,
    /// The rectangle is drawn from its right edge / its bottom edge: what runs across it
    /// runs to the left, what runs down it runs up.
    pub x_reversed: bool,
    pub y_reversed: bool,
}

impl Default for Turn {
    /// The usual side mirror: u runs to the left, v down.
    fn default() -> Self {
        Turn { transposed: false, x_reversed: true, y_reversed: false }
    }
}

impl Turn {
    /// How the picture lies on a glass as seen from `eye`: the glass's `centre` and the way its
    /// position moves with u (`du`) and v (`dv`), all in the bus's frame (x right, y forward,
    /// z up). The viewer's up is the bus's up across the line of sight.
    fn of(eye: Vec3, centre: Vec3, du: Vec3, dv: Vec3) -> Turn {
        let Some(sight) = (centre - eye).try_normalize() else { return Turn::default() };
        let Some(up) = (Vec3::Z - sight * sight.z).try_normalize() else { return Turn::default() };
        let right = sight.cross(up);
        let (du, dv) = (du.try_normalize().unwrap_or(Vec3::ZERO), dv.try_normalize().unwrap_or(Vec3::ZERO));
        let (ur, uu, vr, vu) = (du.dot(right), du.dot(up), dv.dot(right), dv.dot(up));
        if ur.abs() >= uu.abs() {
            // u across, v down the picture (the top row is v 0)
            Turn { transposed: false, x_reversed: ur < 0.0, y_reversed: vu > 0.0 }
        } else {
            // u up or down, v across
            Turn { transposed: true, x_reversed: vr < 0.0, y_reversed: uu > 0.0 }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Panel {
    /// Reflection camera (the bus's mirror) this panel shows.
    pub cam: usize,
    /// Top left corner, as fractions of the window's width and height.
    pub x: f32,
    pub y: f32,
    /// Height as a fraction of the window's height, and width / height.
    pub h: f32,
    pub aspect: f32,
}

#[derive(Default)]
pub(crate) struct MirrorHud {
    pub enabled: bool,
    pub panels: Vec<Panel>,
    /// The editor is on: the mouse and a few keys work on the panels.
    edit: bool,
    /// The bus the layout belongs to (its `.bus` path, lower case); empty before one is known.
    key: String,
    /// The panel being dragged and where it was grabbed (pixels from its corner).
    drag: Option<(usize, f32, f32)>,
    /// A plain yellow pixel for the editor's frames.
    frame: Option<TextureId>,
    /// Width / height of each mirror's glass (0: unknown), from the model.
    aspects: Vec<f32>,
    /// Where each mirror's glass is and how the picture lies on it, see `World::mirror_glass`.
    glass: Vec<Option<MirrorGlass>>,
    /// How each panel lays its mirror's picture out to match its glass (see `Turn`).
    turns: Vec<Turn>,
    /// The keys held in the editor that aim and shift the mirror: left, right, up, down,
    /// Page Up (the mirror forward), Page Down (back), minus (a narrower field of view), plus
    /// (a wider one).
    arrows: [bool; 8],
}

impl MirrorHud {
    /// Mirrors are to be drawn for the panels (even those that no glass in view shows).
    pub fn active(&self) -> bool {
        self.enabled && !self.panels.is_empty()
    }

    /// The glass shapes the world has found in the bus's model.
    pub fn set_aspects(&mut self, a: Vec<f32>) {
        self.aspects = a;
    }

    /// The glasses the world has found in the bus's model.
    pub fn set_glass(&mut self, g: Vec<Option<MirrorGlass>>) {
        self.glass = g;
    }

    /// Width / height a mirror's panel starts with, and whether it sits on the right.
    fn shape(&self, p: &Player, cam: usize) -> (f32, bool) {
        let (guess, right) = shape(p, cam);
        let glass = self.aspects.get(cam).copied().filter(|a| *a > 0.0);
        (glass.unwrap_or(guess), right)
    }

    pub fn editing(&self) -> bool {
        self.edit && self.enabled
    }

    /// An arrow or Page key in the editor works the mirror under the cursor (see `turning`): the
    /// press is taken (true); what is let go is not, so that a key the game saw go down comes up
    /// there too.
    pub fn arrow(&mut self, code: KeyCode, pressed: bool) -> bool {
        let i = match code {
            KeyCode::ArrowLeft => 0,
            KeyCode::ArrowRight => 1,
            KeyCode::ArrowUp => 2,
            KeyCode::ArrowDown => 3,
            KeyCode::PageUp => 4,
            KeyCode::PageDown => 5,
            KeyCode::Minus | KeyCode::NumpadSubtract => 6,
            KeyCode::Equal | KeyCode::NumpadAdd => 7,
            _ => return false,
        };
        if !self.editing() {
            self.arrows = [false; 8];
            return false;
        }
        self.arrows[i] = pressed;
        pressed
    }

    /// The keys held (left, right, up, down, Page Up, Page Down, minus, plus), while the editor
    /// is on and one is.
    pub fn turning(&self) -> Option<[bool; 8]> {
        (self.editing() && self.arrows.iter().any(|a| *a)).then_some(self.arrows)
    }

    /// The mirror the panel under the cursor shows.
    pub fn cam_under(&self, cursor: (f32, f32), size: (f32, f32)) -> Option<usize> {
        self.hit(cursor.0, cursor.1, size.0, size.1).map(|i| self.panels[i].cam)
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Load this bus's layout (once per bus); `setting` is the `mirror_hud` setting, which
    /// gives a bus with nothing saved its first panels.
    pub fn sync(&mut self, p: &Player, setting: u8) {
        if !self.glass.is_empty() {
            let rot = p.vehicle.body_rotation().inverse();
            let eye = rot.transform_vector3((crate::camera_util::driver_eye(p) - p.vehicle.position).as_vec3());
            self.turns = self.glass.iter().map(|g| g.map(|g| Turn::of(eye, g.centre, g.du, g.dv)).unwrap_or_default()).collect();
        }
        let path = p.vehicle.ty.def.path.to_string_lossy().to_string();
        let key = path.to_ascii_lowercase();
        if self.key == key {
            return;
        }
        self.key = key;
        self.drag = None;
        self.edit = false;
        match load(&path) {
            Some((enabled, panels)) => {
                self.enabled = enabled;
                self.panels = panels;
            }
            None => {
                self.enabled = setting != 0;
                self.panels = self.default_panels(p, setting);
            }
        }
        let n = p.vehicle.ty.def.cameras_reflexion.len();
        self.panels.retain(|q| q.cam < n);
    }

    fn save(&self) {
        if self.key.is_empty() {
            return;
        }
        save(&self.key, self.enabled, &self.panels);
    }

    /// Ctrl+M.
    pub fn toggle(&mut self, p: &Player) -> String {
        if self.panels.is_empty() {
            self.add(p);
            self.enabled = true;
        } else {
            self.enabled = !self.enabled;
        }
        if !self.enabled {
            self.edit = false;
            self.drag = None;
            self.arrows = [false; 8];
        }
        self.save();
        if self.enabled { "Mirror panels on (Ctrl+M: off, Ctrl+Shift+M: edit)".into() } else { "Mirror panels off (Ctrl+M)".into() }
    }

    /// Ctrl+Shift+M: the editor on or off.
    pub fn toggle_edit(&mut self, p: &Player) -> String {
        if self.edit {
            self.edit = false;
            self.drag = None;
            self.arrows = [false; 8];
            self.save();
            return "Mirror editor off".into();
        }
        if p.vehicle.ty.def.cameras_reflexion.is_empty() {
            return "This bus has no mirrors to show".into();
        }
        self.enabled = true;
        if self.panels.is_empty() {
            self.add(p);
        }
        self.edit = true;
        HINT.into()
    }

    /// The editor's keys; `Some(message)` when the key was one of them (and is taken).
    pub fn key(&mut self, code: KeyCode, p: &Player, cursor: (f32, f32), size: (f32, f32)) -> Option<String> {
        if !self.editing() {
            return None;
        }
        match code {
            KeyCode::Insert => {
                let n = p.vehicle.ty.def.cameras_reflexion.len();
                if self.panels.len() >= n {
                    return Some(format!("All {n} mirrors of this bus have a panel already"));
                }
                self.add(p);
                self.save();
                Some(format!("Mirror panel added ({} in all)", self.panels.len()))
            }
            KeyCode::Delete | KeyCode::Backspace => {
                let i = self.hit(cursor.0, cursor.1, size.0, size.1).or(self.panels.len().checked_sub(1))?;
                self.panels.remove(i);
                self.drag = None;
                self.save();
                Some(format!("Mirror panel removed ({} left)", self.panels.len()))
            }
            KeyCode::KeyC => {
                let n = p.vehicle.ty.def.cameras_reflexion.len();
                let i = self.hit(cursor.0, cursor.1, size.0, size.1).or(self.panels.len().checked_sub(1))?;
                let used: Vec<usize> = self.panels.iter().map(|q| q.cam).collect();
                let cur = self.panels[i].cam;
                // the next mirror after this one that no other panel shows (else the next)
                let next = (1..=n).map(|d| (cur + d) % n).find(|c| !used.contains(c)).unwrap_or((cur + 1) % n);
                let (aspect, _) = self.shape(p, next);
                self.panels[i].cam = next;
                self.panels[i].aspect = aspect;
                self.save();
                Some(format!("Panel shows mirror {} of {}", next + 1, n))
            }
            KeyCode::Escape => {
                self.edit = false;
                self.drag = None;
                self.arrows = [false; 8];
                self.save();
                Some("Mirror editor off".into())
            }
            _ => None,
        }
    }

    fn add(&mut self, p: &Player) {
        let n = p.vehicle.ty.def.cameras_reflexion.len();
        if n == 0 {
            return;
        }
        // prefer the main side mirrors, then the rest in the order the bus lists them
        let mut order: Vec<usize> = [side_mirror(p, true), side_mirror(p, false)].into_iter().flatten().collect();
        let rest: Vec<usize> = (0..n).filter(|i| !order.contains(i)).collect();
        order.extend(rest);
        let Some(cam) = order.into_iter().find(|c| !self.panels.iter().any(|q| q.cam == *c)) else { return };
        let (aspect, right) = self.shape(p, cam);
        let h = 0.30;
        // stacked down the edge of the side the mirror is on, below the ones already there
        let on_side = self.panels.iter().filter(|q| (q.x > 0.5) == right).count() as f32;
        let y = (0.12 + 0.02 * on_side + on_side * h).min(1.0 - h - MARGIN);
        let x = if right { 1.0 - MARGIN - h * aspect * 0.5625 } else { MARGIN };
        self.panels.push(Panel { cam, x, y, h, aspect });
    }

    fn default_panels(&self, p: &Player, setting: u8) -> Vec<Panel> {
        let mut out = Vec::new();
        for (right, wanted) in [(true, setting & RIGHT != 0), (false, setting & LEFT != 0)] {
            if !wanted {
                continue;
            }
            let Some(cam) = side_mirror(p, right) else { continue };
            let (aspect, _) = self.shape(p, cam);
            let h = 0.30;
            let x = if right { 1.0 - MARGIN - h * aspect * 0.5625 } else { MARGIN };
            out.push(Panel { cam, x, y: 0.28, h, aspect });
        }
        out
    }

    /// Where a panel is on screen, in pixels: left, top, right, bottom.
    fn rect(q: &Panel, w: f32, h: f32) -> [f32; 4] {
        let ph = q.h * h;
        let pw = ph * q.aspect;
        let (x0, y0) = (q.x * w, q.y * h);
        [x0, y0, x0 + pw, y0 + ph]
    }

    fn hit(&self, x: f32, y: f32, w: f32, h: f32) -> Option<usize> {
        if !self.active() {
            return None;
        }
        // the one drawn last is on top
        (0..self.panels.len()).rev().find(|&i| {
            let r = Self::rect(&self.panels[i], w, h);
            x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]
        })
    }

    /// The left button over a panel starts dragging it (returns true: the click is taken);
    /// letting it go ends the drag and keeps the place. Only with the editor on.
    pub fn press(&mut self, pressed: bool, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if !pressed {
            if self.drag.take().is_some() {
                self.save();
                return true;
            }
            return false;
        }
        if !self.editing() {
            return false;
        }
        let Some(i) = self.hit(cursor.0, cursor.1, size.0, size.1) else { return false };
        let r = Self::rect(&self.panels[i], size.0, size.1);
        // on top of the others from now on
        let q = self.panels.remove(i);
        self.panels.push(q);
        self.drag = Some((self.panels.len() - 1, cursor.0 - r[0], cursor.1 - r[1]));
        true
    }

    /// The cursor moved: a panel being dragged follows.
    pub fn moved(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        let Some((i, dx, dy)) = self.drag else { return false };
        let Some(q) = self.panels.get_mut(i) else { return false };
        let ph = q.h * size.1;
        let pw = ph * q.aspect;
        q.x = ((cursor.0 - dx) / size.0).clamp(0.0, ((size.0 - pw) / size.0).max(0.0));
        q.y = ((cursor.1 - dy) / size.1).clamp(0.0, ((size.1 - ph) / size.1).max(0.0));
        true
    }

    /// The wheel over a panel makes it larger or smaller (with Shift wider or narrower);
    /// true when one was under the cursor. Only with the editor on.
    pub fn wheel(&mut self, amount: f32, shift: bool, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if !self.editing() {
            return false;
        }
        let Some(i) = self.hit(cursor.0, cursor.1, size.0, size.1) else { return false };
        let q = &mut self.panels[i];
        if shift {
            q.aspect = (q.aspect * (1.0 + 0.06 * amount)).clamp(MIN_ASPECT, MAX_ASPECT);
        } else {
            q.h = (q.h * (1.0 + 0.06 * amount)).clamp(MIN_H, MAX_H);
        }
        // stays inside the window
        let ph = q.h * size.1;
        let pw = ph * q.aspect;
        q.x = q.x.clamp(0.0, ((size.0 - pw) / size.0).max(0.0));
        q.y = q.y.clamp(0.0, ((size.1 - ph) / size.1).max(0.0));
        self.save();
        true
    }

    /// The editor's frame colour, made once.
    pub fn ensure_frame(&mut self, r: &Renderer, scene: &mut Scene) {
        if self.frame.is_none() && self.editing() {
            let image = omsi_texture::Image { width: 1, height: 1, rgba: vec![255, 200, 0, 255], has_alpha: false };
            self.frame = Some(r.add_texture(scene, &image, false));
        }
    }

    /// Add the panels' pictures to the frame's overlays (after the HUD's and the
    /// interface's, so that their indices stay put). `w` and `h` are the window's size.
    pub fn push(&self, scene: &mut Scene, world: &World, w: f32, h: f32, cursor: (f32, f32)) {
        if !self.active() {
            return;
        }
        let textures = world.mirror_textures.lock().clone();
        let under = if self.editing() { self.hit(cursor.0, cursor.1, w, h) } else { None };
        for (i, q) in self.panels.iter().enumerate() {
            let Some(tex) = textures.get(q.cam).copied().flatten() else { continue };
            let r = Self::rect(q, w, h);
            // in the editor each panel has a frame, the one under the cursor a thicker one
            if let (true, Some(f)) = (self.editing(), self.frame) {
                let t = if under == Some(i) || self.drag.is_some_and(|d| d.0 == i) { 5.0 } else { 2.0 };
                scene.overlays.push((f, [r[0] - t, r[1] - t, r[2] + t, r[3] + t]));
            }
            // (laid out the way the glass's mesh lays the picture on it - over left to right
            // for the usual side mirror - so that the panel shows what the glass shows)
            let t = self.turns.get(q.cam).copied().unwrap_or_default();
            let (x0, x1) = if t.x_reversed { (r[2], r[0]) } else { (r[0], r[2]) };
            let (y0, y1) = if t.y_reversed { (r[3], r[1]) } else { (r[1], r[3]) };
            if t.transposed {
                scene.transposed.insert(tex);
            } else {
                scene.transposed.remove(&tex);
            }
            scene.overlays.push((tex, [x0, y0, x1, y1]));
        }
    }
}

/// The reflection camera farthest out to one side of the bus (the vehicle's x grows to the
/// right): the main mirror, not the wide-angle or ramp ones inside it.
pub(crate) fn side_mirror(p: &Player, right: bool) -> Option<usize> {
    p.vehicle
        .ty
        .def
        .cameras_reflexion
        .iter()
        .enumerate()
        .filter(|(_, c)| if right { c.pos[0] > 0.3 } else { c.pos[0] < -0.3 })
        .max_by(|a, b| a.1.pos[0].abs().partial_cmp(&b.1.pos[0].abs()).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
}

/// Width / height a mirror's panel starts with, and whether it sits on the right: the main
/// side mirrors are tall, those in the middle (the inside mirror) wide.
fn shape(p: &Player, cam: usize) -> (f32, bool) {
    let x = p.vehicle.ty.def.cameras_reflexion.get(cam).map(|c| c.pos[0]).unwrap_or(0.0);
    let right = x >= 0.0;
    let aspect = if x.abs() > 0.3 { 0.55 } else { 2.2 };
    (aspect, right)
}

fn file() -> Option<std::path::PathBuf> {
    crate::settings::Settings::path().map(|p| p.with_file_name("mirror_hud.cfg"))
}

/// `<bus path>|on=1` and `<bus path>|panel=cam,x,y,h,aspect` lines.
fn load(bus: &str) -> Option<(bool, Vec<Panel>)> {
    let text = std::fs::read_to_string(file()?).ok()?;
    let key = bus.to_ascii_lowercase();
    let mut enabled = None;
    let mut panels = Vec::new();
    for line in text.lines() {
        let Some((head, value)) = line.split_once('=') else { continue };
        let Some((path, what)) = head.rsplit_once('|') else { continue };
        if path.trim().to_ascii_lowercase() != key {
            continue;
        }
        match what.trim() {
            "on" => enabled = Some(value.trim() != "0"),
            "panel" => {
                let v: Vec<f32> = value.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if v.len() == 5 {
                    panels.push(Panel {
                        cam: v[0].max(0.0) as usize,
                        x: v[1].clamp(0.0, 1.0),
                        y: v[2].clamp(0.0, 1.0),
                        h: v[3].clamp(MIN_H, MAX_H),
                        aspect: v[4].clamp(MIN_ASPECT, MAX_ASPECT),
                    });
                }
            }
            _ => {}
        }
    }
    enabled.map(|e| (e, panels))
}

fn save(key: &str, enabled: bool, panels: &[Panel]) {
    let Some(p) = file() else { return };
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| l.rsplit_once('=').and_then(|(k, _)| k.rsplit_once('|')).is_none_or(|(f, _)| f.trim().to_ascii_lowercase() != key))
        .map(str::to_string)
        .collect();
    lines.push(format!("{key}|on={}", enabled as u8));
    for q in panels {
        lines.push(format!("{key}|panel={},{:.4},{:.4},{:.4},{:.3}", q.cam, q.x, q.y, q.h, q.aspect));
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, lines.join("\n") + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editing(panels: Vec<Panel>) -> MirrorHud {
        MirrorHud { enabled: true, edit: true, panels, ..Default::default() }
    }

    #[test]
    fn a_panel_keeps_its_place_inside_the_window() {
        let mut m = editing(vec![Panel { cam: 0, x: 0.9, y: 0.9, h: 0.3, aspect: 0.5 }]);
        assert!(m.wheel(5.0, false, (0.9 * 1600.0 + 5.0, 0.9 * 900.0 + 5.0), (1600.0, 900.0)));
        let r = MirrorHud::rect(&m.panels[0], 1600.0, 900.0);
        assert!(r[2] <= 1600.5 && r[3] <= 900.5, "{r:?}");
    }

    #[test]
    fn dragging_moves_the_grabbed_corner_with_the_cursor() {
        let mut m = editing(vec![Panel { cam: 0, x: 0.1, y: 0.1, h: 0.3, aspect: 1.0 }]);
        let size = (1000.0, 1000.0);
        assert!(m.press(true, (150.0, 150.0), size));
        assert!(m.moved((350.0, 250.0), size));
        let q = m.panels[0];
        assert!((q.x - 0.3).abs() < 1e-4 && (q.y - 0.2).abs() < 1e-4, "{q:?}");
        // letting go ends the drag; the cursor then moves nothing
        assert!(m.press(false, (350.0, 250.0), size));
        assert!(!m.moved((900.0, 900.0), size));
    }

    #[test]
    fn the_mouse_does_nothing_to_the_panels_outside_the_editor() {
        let mut m = editing(vec![Panel { cam: 0, x: 0.1, y: 0.1, h: 0.3, aspect: 1.0 }]);
        m.edit = false;
        let size = (1000.0, 1000.0);
        assert!(!m.press(true, (150.0, 150.0), size), "a click is left to the cab");
        assert!(!m.wheel(1.0, false, (150.0, 150.0), size), "the wheel is left to the zoom");
        assert!(!m.dragging());
    }

    #[test]
    fn the_arrows_are_the_editors_only_while_it_is_on() {
        let mut m = editing(vec![Panel { cam: 3, x: 0.1, y: 0.1, h: 0.3, aspect: 1.0 }]);
        let size = (1000.0, 1000.0);
        assert!(m.arrow(KeyCode::ArrowLeft, true), "a press is taken");
        assert_eq!(m.turning(), Some([true, false, false, false, false, false, false, false]));
        assert_eq!(m.cam_under((150.0, 150.0), size), Some(3));
        assert_eq!(m.cam_under((900.0, 900.0), size), None);
        assert!(!m.arrow(KeyCode::ArrowLeft, false), "a release goes on to the game");
        assert_eq!(m.turning(), None);
        m.edit = false;
        assert!(!m.arrow(KeyCode::ArrowRight, true), "outside the editor the key is the game's");
        assert_eq!(m.turning(), None);
        assert!(!m.arrow(KeyCode::KeyA, true));
        m.edit = true;
        assert!(m.arrow(KeyCode::PageUp, true), "Page Up shifts the mirror forward");
        assert_eq!(m.turning(), Some([false, false, false, false, true, false, false, false]));
        assert!(m.arrow(KeyCode::Equal, true), "plus widens the field of view");
        assert_eq!(m.turning().map(|a| a[7]), Some(true));
    }

    #[test]
    fn only_a_click_on_a_panel_is_taken() {
        let mut m = editing(vec![Panel { cam: 0, x: 0.1, y: 0.1, h: 0.3, aspect: 1.0 }]);
        assert!(!m.press(true, (900.0, 900.0), (1000.0, 1000.0)));
        assert!(!m.dragging());
    }

    #[test]
    fn the_panel_picked_up_comes_to_the_top() {
        let mut m = editing(vec![
            Panel { cam: 0, x: 0.1, y: 0.1, h: 0.3, aspect: 1.0 },
            Panel { cam: 1, x: 0.5, y: 0.5, h: 0.3, aspect: 1.0 },
        ]);
        assert!(m.press(true, (150.0, 150.0), (1000.0, 1000.0)));
        assert_eq!(m.panels.last().map(|q| q.cam), Some(0));
    }

    #[test]
    fn a_picture_is_laid_out_as_its_glass_shows_it_to_the_driver() {
        let (eye, centre) = (Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0));
        let turn = |du, dv| Turn::of(eye, centre, du, dv);
        // seen from the front: right is +x, up is +z
        assert_eq!(turn(Vec3::X, -Vec3::Z), Turn { transposed: false, x_reversed: false, y_reversed: false });
        // the usual side mirror: the picture runs to the left
        assert_eq!(turn(-Vec3::X, -Vec3::Z), Turn::default());
        // v up the glass: the picture upside down
        assert_eq!(turn(Vec3::X, Vec3::Z), Turn { transposed: false, x_reversed: false, y_reversed: true });
        // on its side: u up the glass, v across it
        assert_eq!(turn(Vec3::Z, Vec3::X), Turn { transposed: true, x_reversed: false, y_reversed: true });
        assert_eq!(turn(-Vec3::Z, -Vec3::X), Turn { transposed: true, x_reversed: true, y_reversed: false });
    }
}
