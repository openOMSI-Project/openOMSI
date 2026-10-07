//! The studio's screen, after Omsi-Hub's: the tools along the top with the name and Save, the
//! layers on the left, the bus in the middle (the mouse paints on it; right-drag turns it,
//! the middle button or Shift-drag moves it, the wheel zooms, views 1-6, F fits, O held shows
//! the livery before, E the eraser, [ and ] the brush's size; while mirroring the other side in
//! a corner), and on the right what the tool or the chosen layer offers - the quick livery, the
//! bus options, the colour picker, the stripes, texts, pictures, shapes and the brush.

use super::bake::BusGeom;
use super::model::{self, Coupling, Grip, Kind, Layer, Place, Side, StripeTemplate};
use super::{colour, paint, shapes, Launcher, Session};
use crate::launcher::busoptions;
use crate::launcher::theme::*;
use crate::launcher::ui::{ButtonKind, Key, Ui};
use glam::{DVec3, Vec2, Vec3};
use omsi_render::Camera;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Fill,
    Stripe,
    Text,
    Image,
    Shape,
    Pen,
    /// The brush, and with E the eraser.
    Brush,
}

const TOOLS: [(Tool, &str, &str); 8] = [
    (Tool::Select, "Select", "livery_select"),
    (Tool::Fill, "Fill", "livery_fill"),
    (Tool::Stripe, "Stripe", "livery_stripe"),
    (Tool::Text, "Text", "livery_text"),
    (Tool::Image, "Image", "livery_image"),
    (Tool::Shape, "Shapes", "livery_shape"),
    (Tool::Pen, "Pen", "livery_pen"),
    (Tool::Brush, "Brush", "livery_brush"),
];

/// Choosing a bus (and a livery to begin from), or a livery begun before.
#[derive(Default)]
pub struct Chooser {
    search: String,
    bus: Option<String>,
    paint: usize,
    /// How it begins (`model::STARTS`).
    start: usize,
}

/// What the colour picker colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PickFor {
    #[default]
    Main,
    Outline,
    GradientEnd,
    Quick(usize),
}

pub struct Picker {
    hsv: [f32; 3],
    hex: String,
    pub target: PickFor,
    pub eyedropper: bool,
    /// The colour new layers get.
    pub colour: [u8; 3],
}

impl Default for Picker {
    fn default() -> Picker {
        let colour = [0x1d, 0x3f, 0x8f];
        Picker { hsv: colour::hsv(colour::unit(colour)), hex: model::hex(colour), target: PickFor::Main, eyedropper: false, colour }
    }
}

/// The studio's camera: orbiting `target` at heading `yaw`, looking down by `pitch`, `dist` off;
/// eased to where it is sent.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cam {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    to: (Vec3, f32, f32, f32),
}

const FOV: f32 = 30.0;

impl Cam {
    pub fn camera(&self) -> Camera {
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let fwd = Vec3::new(sy * cp, cy * cp, -sp);
        let pos = self.target - fwd * self.dist;
        Camera { position: pos.as_dvec3(), yaw: self.yaw, pitch: -self.pitch, roll: 0.0, fov_deg: FOV, near: 0.1, far: 800.0 }
    }

    /// The view `k` (1 left, 2 right, 3 front, 4 rear, 5 roof, 6 angled), the bus fitted.
    pub fn fitting(dims: &model::BusDims, k: u32, aspect: f32) -> Cam {
        let mut c = Cam { target: (dims.min + dims.max) * 0.5, ..Default::default() };
        (c.yaw, c.pitch) = view_angles(k);
        c.dist = fit(dims, c.target, c.yaw, c.pitch, aspect);
        c.to = (c.target, c.yaw, c.pitch, c.dist);
        c
    }

    fn send(&mut self, target: Vec3, yaw: f32, pitch: f32, dist: f32) {
        // (the heading the short way round)
        let mut yaw = yaw;
        while yaw - self.yaw > 180.0 {
            yaw -= 360.0;
        }
        while yaw - self.yaw < -180.0 {
            yaw += 360.0;
        }
        self.to = (target, yaw, pitch, dist);
    }

    /// Eased one frame on; true while it moves.
    fn ease(&mut self, dt: f32) -> bool {
        let k = 1.0 - (-dt / 0.12).exp();
        let (t, y, p, d) = self.to;
        let moving = (t - self.target).length() > 1e-3 || (y - self.yaw).abs() > 0.02 || (p - self.pitch).abs() > 0.02 || (d - self.dist).abs() > 1e-3;
        self.target += (t - self.target) * k;
        self.yaw += (y - self.yaw) * k;
        self.pitch += (p - self.pitch) * k;
        self.dist += (d - self.dist) * k;
        moving
    }
}

fn view_angles(k: u32) -> (f32, f32) {
    match k {
        1 => (90.0, 2.0),
        2 => (270.0, 2.0),
        3 => (180.0, 4.0),
        4 => (0.0, 4.0),
        5 => (0.0, 89.0),
        _ => (215.0, 14.0),
    }
}

/// How far off the camera must stand for the box to fill the picture (`aspect` wide).
fn fit(dims: &model::BusDims, target: Vec3, yaw: f32, pitch: f32, aspect: f32) -> f32 {
    let corners: Vec<Vec3> = (0..8).map(|k| Vec3::new(if k & 1 == 0 { dims.min.x } else { dims.max.x }, if k & 2 == 0 { dims.min.y } else { dims.max.y }, if k & 4 == 0 { dims.min.z } else { dims.max.z })).collect();
    let fits = |d: f32| {
        let c = Cam { target, yaw, pitch, dist: d, to: (target, yaw, pitch, d) }.camera();
        let m = c.view_proj(aspect, DVec3::ZERO);
        corners.iter().all(|p| {
            let q = m * p.extend(1.0);
            q.w > 0.0 && (q.x / q.w).abs() < 0.86 && (q.y / q.w).abs() < 0.86
        })
    };
    let (mut lo, mut hi) = (1.0f32, 400.0f32);
    for _ in 0..30 {
        let mid = (lo + hi) * 0.5;
        if fits(mid) { hi = mid } else { lo = mid }
    }
    hi
}

/// What the mouse is doing on the bus.
#[derive(Clone, Debug)]
enum Drag {
    Orbit,
    Pan,
    /// Moving a decal: the point grabbed relative to its middle, on its mirrored copy.
    Move { id: String, offset: Vec3, copy: bool },
    /// A handle of a decal pulled: which, the decal and its size when it was grabbed, and where
    /// in its box.
    Scale { id: String, grip: Grip, start: Place, from: Vec2, size: (f32, f32), height_cm: f32 },
    Rotate { id: String, start: f32, from: f32 },
    Edge { id: String, upper: bool },
    /// The pen's corner being pulled into a curve.
    Curve,
    /// A stroke of the brush or the eraser into a brush layer.
    Brush { id: String },
}

/// What a click on the bus places.
#[derive(Clone, Debug)]
enum Armed {
    Picture { hash: String, aspect: f32, white: bool },
    Shape(&'static str),
}

/// The pen's corners: their place in the side's plane (metres, y up) and the curve's handle.
#[derive(Clone, Debug, Default)]
struct Pen {
    side: Option<Side>,
    origin: Vec3,
    nodes: Vec<(Vec2, Option<Vec2>)>,
}

#[derive(Default)]
pub struct State {
    pub name: String,
    pub before_after: bool,
    drag: Option<Drag>,
    armed: Option<Armed>,
    pen: Pen,
    last_mouse: Vec2,
    /// The text tool's text, font and size.
    text: String,
    font: usize,
    size_cm: f32,
    /// Where the bus's picture lies and its camera (for the mouse).
    view: Option<(Rect, Camera)>,
    /// The brush: its radius (cm), hardness, cover, whether it erases, and the layer it last
    /// painted into.
    pub brush_cm: f32,
    pub hardness: f32,
    pub brush_opacity: f32,
    pub eraser: bool,
    brush_layer: Option<String>,
    /// A layer held in the list (its id, where the mouse went down, whether it is being dragged
    /// to another place).
    layer_drag: Option<(String, Vec2, bool)>,
    /// "Remove from the game" asked once (a second click within a few seconds does it).
    remove_asked: Option<std::time::Instant>,
    /// How much of the chosen decal lies on texels both sides share, for the place it had.
    shared: Option<(String, f32)>,
    /// The other side's picture in the bus's put away by its cross (its chip brings it back).
    hide_other: bool,
}

impl State {
    pub fn named(name: String) -> State {
        State { name, brush_cm: 12.0, hardness: 0.6, brush_opacity: 1.0, ..Default::default() }
    }

    /// Where the bus's picture lay last frame.
    pub fn view(&self) -> Option<Rect> {
        self.view.map(|v| v.0)
    }
}

const TOP: f32 = 14.0;
const BAR_H: f32 = 58.0;
const LEFT_W: f32 = 256.0;
const RIGHT_W: f32 = 344.0;
const M: f32 = 14.0;

/// The swatches every picker offers.
const PALETTE: [u32; 16] = [0xffffff, 0xd9d9d9, 0x8c8c8c, 0x1f1f1f, 0xffd23f, 0xf28c28, 0xd93a30, 0x8f1d3f, 0xe85d9e, 0x6b3fa0, 0x1d3f8f, 0x2a75f7, 0x2bb3c0, 0x1a8a4f, 0x7cc24a, 0x6b4f2a];

pub fn draw(l: &mut Launcher) {
    let size = l.ui.size;
    let full = Rect::new(0.0, 0.0, size.x, size.y);
    l.ui.p().rect(full, GROUND);
    if l.livery.session.is_none() {
        chooser(l, full);
        return;
    }
    let dt = l.ui.dt;
    // the bus's picture between the panels
    let view = Rect::new(M + LEFT_W + 10.0, TOP + BAR_H + 10.0, size.x - 2.0 * M - LEFT_W - RIGHT_W - 20.0, size.y - TOP - BAR_H - 10.0 - M);
    {
        let v = &mut l.livery;
        let s = v.session.as_mut().unwrap();
        if s.cam.ease(dt) {
            l.ui.keep_moving();
        }
        let cam = s.cam.camera();
        v.showroom.set_camera(Some(cam));
        v.view_rect = Some(view);
        s.ui.view = Some((view, cam));
        l.ui.p().rounded(view, RADIUS, FIELD);
        if let (Some(tex), true) = (v.view_tex, v.showroom.has_picture()) {
            l.ui.image(view, tex, RADIUS);
        }
        // the other side while mirroring, in a corner: what the copies make there. The bus
        // under it is painted as anywhere: with the mouse over it, or painting, it fades away;
        // its cross puts it away, a chip in the corner brings it back.
        let room = s.project.mirror.on && s.ready.is_some() && view.w > 520.0;
        let other = (room && !s.ui.hide_other).then(|| {
            let ow = (view.w * 0.3).min(360.0);
            Rect::new(view.right() - ow - 12.0, view.bottom() - ow * view.h / view.w - 12.0, ow, ow * view.h / view.w)
        });
        v.other_rect = other;
        if let (Some(o), Some(tex)) = (other, v.other_tex) {
            if v.showroom.second_view().is_some() {
                let mouse = l.ui.input.mouse;
                let busy = s.ui.drag.is_some() || (l.ui.input.down && view.contains(mouse));
                let a = l.ui.anim(crate::launcher::ui::id_of("livery-other-fade"), if o.contains(mouse) || busy { 0.15 } else { 1.0 }, 0.1);
                l.ui.p().rounded(o.pad(-2.0, -2.0), RADIUS + 2.0, HAIRLINE.alpha(a));
                l.ui.image_faded(o, tex, RADIUS, a);
                // (on a dark chip: the sky behind them is light)
                let label = omsi_ui::tr("Other side");
                let lw = l.ui.width(&label, 11.5, Weight::Medium).min(o.w - 56.0);
                l.ui.p().rounded(Rect::new(o.x + 6.0, o.y + 5.0, lw + 16.0, 20.0), 10.0, Color::BLACK.alpha(0.5 * a));
                l.ui.text_in(&label, Rect::new(o.x + 14.0, o.y + 7.0, lw + 2.0, 16.0), 11.5, Weight::Medium, TEXT.alpha(a), Align::Left);
                let c = Vec2::new(o.right() - 15.0, o.y + 15.0);
                l.ui.p().circle(c, 11.0, Color::BLACK.alpha(0.5 * a));
                l.ui.solid(Rect::new(c.x - 11.0, c.y - 11.0, 22.0, 22.0));
                if l.ui.icon_button("livery-other-hide", c, 11.0, "close", "Hide the other side") {
                    s.ui.hide_other = true;
                }
            }
        } else if room {
            let chip = Rect::new(view.right() - 140.0, view.y + 12.0, 128.0, 30.0);
            l.ui.solid(chip);
            if l.ui.button("livery-other-show", chip, "Other side", Some("visibility"), ButtonKind::Normal) {
                s.ui.hide_other = false;
            }
        }
        let msg = if let Some(e) = &s.failed {
            Some(omsi_ui::tr("The bus cannot be painted: %{e}").replace("%{e}", e))
        } else if !v.showroom.has_picture() {
            Some(omsi_ui::tr("Loading the bus…").into_owned())
        } else if s.ready.is_none() {
            Some(omsi_ui::tr("Preparing the paint…").into_owned())
        } else {
            None
        };
        if let Some(m) = msg {
            let r = Rect::new(view.x + 20.0, view.center().y - 40.0, view.w - 40.0, 24.0);
            l.ui.text_in(&m, r, 14.0, Weight::Medium, if s.failed.is_some() { DANGER.lighten(0.2) } else { TEXT_DIM }, Align::Center);
            if s.failed.is_none() {
                let c = Vec2::new(view.center().x, view.center().y);
                let a = l.ui.time * 5.0;
                l.ui.p().arc(c, 9.0, 12.0, a, a + 4.2, TEXT_SOFT);
                l.ui.keep_moving();
            }
        }
    }
    top_bar(l, Rect::new(M, TOP, size.x - 2.0 * M, BAR_H));
    // (Back: the studio is left, nothing of it is drawn any more - the panels below took the
    // session that was no longer there, and the launcher went down with it)
    if l.livery.session.is_none() {
        return;
    }
    layers_panel(l, Rect::new(M, TOP + BAR_H + 10.0, LEFT_W, size.y - TOP - BAR_H - 10.0 - M));
    right_panel(l, Rect::new(size.x - M - RIGHT_W, TOP + BAR_H + 10.0, RIGHT_W, size.y - TOP - BAR_H - 10.0 - M));
    view_bar(l, view);
    overlay(l, view);
    pointer(l, view);
    keys(l);
    // the change going on ends with the mouse let go and no field typed in
    let s = l.livery.session.as_mut().unwrap();
    if !l.ui.input.down && l.ui.focus.is_none() {
        s.settle();
    }
    if let Some((t, err, at)) = s.status.clone() {
        let fade = if err { 1.0 } else { (1.0 - (at.elapsed().as_secs_f32() - 5.0) / 1.0).clamp(0.0, 1.0) };
        if fade > 0.0 {
            let w = (l.ui.width(&t, 12.5, Weight::Medium) + 32.0).min(view.w - 40.0);
            let r = Rect::new(view.center().x - w * 0.5, view.y + 14.0, w, 28.0);
            l.ui.p().rounded(r, 14.0, ON_MAP.alpha(fade));
            l.ui.text_in(&t, r.pad(14.0, 0.0), 12.5, Weight::Medium, if err { DANGER.lighten(0.25) } else { TEXT }.alpha(fade), Align::Center);
        }
    }
}

// --- choosing a bus ----------------------------------------------------------------------------

fn chooser(l: &mut Launcher, full: Rect) {
    let r = Rect::new((full.w - 980.0).max(32.0) * 0.5, 40.0, full.w.min(980.0 + 64.0) - 64.0, full.h - 80.0);
    l.ui.panel(r);
    if l.ui.button("livery-close", Rect::new(r.right() - 124.0, r.y + 20.0, 104.0, 38.0), "Back", Some("chevron_left"), ButtonKind::Normal) {
        l.go(crate::launcher::Page::Drive);
    }
    l.ui.icon("livery_fill", Vec2::new(r.x + 34.0, r.y + 40.0), 24.0, TEXT);
    l.ui.text_in("Livery studio", Rect::new(r.x + 56.0, r.y + 22.0, 400.0, 30.0), 24.0, Weight::Bold, TEXT, Align::Left);
    l.ui.text_in("Paint a bus in 3D and drive it in your own livery.", Rect::new(r.x + 56.0, r.y + 52.0, r.w - 200.0, 18.0), 13.0, Weight::Regular, TEXT_DIM, Align::Left);
    let body = Rect::new(r.x + 24.0, r.y + 92.0, r.w - 48.0, r.h - 116.0);
    let (left, right) = body.cut_left(body.w * 0.42);
    // liveries begun before
    let lh = l.ui.heading(left, "Continue a livery", None);
    let list = super::projects();
    let mut open: Option<model::Project> = None;
    if list.is_empty() {
        l.ui.text_in("None yet.", Rect::new(lh.x, lh.y + 4.0, lh.w, 20.0), 13.0, Weight::Regular, TEXT_FAINT, Align::Left);
    }
    let names: std::collections::HashMap<String, String> = l.state.vehicles.iter().map(|v| (v.file.clone(), v.name.clone())).collect();
    l.ui.scroll_area("livery-projects", Rect::new(lh.x, lh.y, lh.w - 24.0, lh.h), &mut |ui, a| {
        for (k, p) in list.iter().enumerate() {
            let row = Rect::new(a.x, a.y + k as f32 * 52.0, a.w, 46.0);
            if ui.row(&format!("livery-project-{k}"), row, false) {
                open = Some(p.clone());
            }
            let title = if p.name.is_empty() { omsi_ui::tr("Untitled livery").into_owned() } else { p.name.clone() };
            ui.text_in(&title, Rect::new(row.x + 12.0, row.y + 4.0, row.w - 24.0, 20.0), 13.5, Weight::Bold, TEXT, Align::Left);
            let bus = names.get(&p.bus).cloned().unwrap_or_else(|| p.bus.rsplit('/').next().unwrap_or("").to_string());
            let sub = format!("{bus} · {} {}", p.layers.len(), omsi_ui::tr("layers"));
            ui.text_in(&sub, Rect::new(row.x + 12.0, row.y + 24.0, row.w - 24.0, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
        }
        list.len() as f32 * 52.0
    });
    if let Some(p) = open {
        let (bus, paint) = (p.bus.clone(), p.start_paint.clone());
        super::start(l, bus, paint, Some(p));
        return;
    }
    // a new one
    let rh = l.ui.heading(Rect::new(right.x + 24.0, right.y, right.w - 24.0, right.h), "Paint a new livery", None);
    let mut search = std::mem::take(&mut l.livery.chooser.search);
    l.ui.text_input("livery-search", Rect::new(rh.x, rh.y, rh.w, 36.0), &mut search, "Search a bus", Some("search"));
    l.livery.chooser.search = search;
    let q = l.livery.chooser.search.to_lowercase();
    let buses: Vec<(String, String, Vec<String>)> = l.state.vehicles.iter().filter(|v| q.is_empty() || v.name.to_lowercase().contains(&q) || v.file.to_lowercase().contains(&q)).map(|v| (v.file.clone(), v.name.clone(), v.paints.clone())).collect();
    let chosen = l.livery.chooser.bus.clone();
    let mut pick: Option<String> = None;
    let list_r = Rect::new(rh.x, rh.y + 46.0, rh.w, rh.h - 46.0 - 100.0);
    l.ui.scroll_area("livery-buses", list_r, &mut |ui, a| {
        for (k, (file, name, _)) in buses.iter().enumerate() {
            let row = Rect::new(a.x, a.y + k as f32 * 38.0, a.w - 14.0, 34.0);
            if !ui.rect_visible(row) {
                continue;
            }
            if ui.row(&format!("livery-bus-{k}"), row, chosen.as_deref() == Some(file.as_str())) {
                pick = Some(file.clone());
            }
            let on = chosen.as_deref() == Some(file.as_str());
            ui.icon("directions_bus", Vec2::new(row.x + 18.0, row.center().y), 17.0, if on { on_accent() } else { TEXT_DIM });
            ui.text_in(name, Rect::new(row.x + 36.0, row.y, row.w - 44.0, row.h), 13.0, Weight::Medium, if on { on_accent() } else { TEXT }, Align::Left);
        }
        buses.len() as f32 * 38.0
    });
    if let Some(p) = pick {
        l.livery.chooser.bus = Some(p);
        l.livery.chooser.paint = 0;
    }
    let foot = Rect::new(rh.x, list_r.bottom() + 14.0, rh.w, 86.0);
    if let Some(bus) = l.livery.chooser.bus.clone() {
        let paints = l.state.vehicles.iter().find(|v| v.file == bus).map(|v| v.paints.clone()).unwrap_or_default();
        let mut opts = vec![omsi_ui::tr("The model's own paint").into_owned()];
        opts.extend(paints.iter().cloned());
        l.ui.label(Rect::new(foot.x, foot.y, 150.0, 34.0), "Begin from");
        let mut k = l.livery.chooser.paint.min(opts.len() - 1);
        l.ui.select("livery-start-paint", Rect::new(foot.x + 150.0, foot.y, foot.w - 150.0, 34.0), &mut k, &opts);
        l.livery.chooser.paint = k;
        // how: the quick livery, the livery as it is, its colours plain, plain (the last two on
        // the model's own paint; from the model's own the first three are the same)
        let starts: Vec<&str> = model::STARTS.iter().map(|s| s.1).collect();
        let mut how = l.livery.chooser.start.min(starts.len() - 1);
        l.ui.label(Rect::new(foot.x, foot.y + 44.0, 150.0, 34.0), "Begin with");
        if l.ui.select("livery-start-how", Rect::new(foot.x + 150.0, foot.y + 44.0, foot.w - 150.0 - 230.0, 34.0), &mut how, &starts.iter().map(|s| omsi_ui::tr(s).into_owned()).collect::<Vec<_>>()) {
            l.livery.chooser.start = how;
        }
        if l.ui.button("livery-begin", Rect::new(foot.right() - 220.0, foot.y + 44.0, 220.0, 40.0), "Start painting", Some("livery_fill"), ButtonKind::Primary) {
            let paint = (k > 0).then(|| paints[k - 1].clone());
            super::start_new(l, bus, paint, model::STARTS[how].0);
        }
    } else {
        l.ui.text_in("Choose the bus to paint.", Rect::new(foot.x, foot.y, foot.w, 34.0), 13.0, Weight::Regular, TEXT_FAINT, Align::Left);
    }
}

// --- the bar ---------------------------------------------------------------------------------

fn top_bar(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    if l.ui.button("livery-back", Rect::new(r.x + 10.0, r.y + 10.0, 92.0, 38.0), "Back", Some("chevron_left"), ButtonKind::Ghost) {
        super::leave(l);
        return;
    }
    let bus = l.livery.session.as_ref().map(|s| s.project.bus.clone()).unwrap_or_default();
    let bus_name = l.state.vehicles.iter().find(|v| v.file == bus).map(|v| v.name.clone()).unwrap_or_else(|| bus.rsplit('/').next().unwrap_or("").to_string());
    l.ui.text_in("Livery studio", Rect::new(r.x + 112.0, r.y + 9.0, 180.0, 22.0), 15.5, Weight::Bold, TEXT, Align::Left);
    l.ui.text_in(&bus_name, Rect::new(r.x + 112.0, r.y + 30.0, 180.0, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
    // the tools
    let mut x = r.x + 300.0;
    let tw = 56.0;
    for (tool, label, icon) in TOOLS {
        let b = Rect::new(x, r.y + 6.0, tw, r.h - 12.0);
        let on = l.livery.session.as_ref().is_some_and(|s| s.tool == tool);
        if tool_button(&mut l.ui, &format!("livery-tool-{label}"), b, label, icon, on) {
            let s = l.livery.session.as_mut().unwrap();
            s.tool = tool;
            s.ui.armed = None;
            s.ui.pen = Pen::default();
            if tool != Tool::Select {
                s.selected = None;
            }
        }
        x += tw + 4.0;
    }
    x += 10.0;
    l.ui.p().rect(Rect::new(x, r.y + 12.0, 1.0, r.h - 24.0), HAIRLINE);
    x += 12.0;
    let mirror = l.livery.session.as_ref().is_some_and(|s| s.project.mirror.on);
    if tool_button(&mut l.ui, "livery-mirror", Rect::new(x, r.y + 6.0, 66.0, r.h - 12.0), "Mirror", "livery_mirror", mirror) {
        let s = l.livery.session.as_mut().unwrap();
        let before = s.project.doc();
        s.project.mirror.on = !s.project.mirror.on;
        s.changed_from(before);
        s.settle();
    }
    l.ui.tooltip(Rect::new(x, r.y + 6.0, 66.0, r.h - 12.0), "Decals on one side are copied onto the other");
    x += 74.0;
    let (can_undo, can_redo) = l.livery.session.as_ref().map(|s| (s.history.can_undo() || s.pending.is_some(), s.history.can_redo())).unwrap_or_default();
    if l.ui.icon_button("livery-undo", Vec2::new(x + 18.0, r.center().y), 17.0, "livery_undo", "Undo (Ctrl+Z)") && can_undo {
        l.livery.session.as_mut().unwrap().undo();
    }
    if l.ui.icon_button("livery-redo", Vec2::new(x + 56.0, r.center().y), 17.0, "livery_redo", "Redo (Ctrl+Y)") && can_redo {
        l.livery.session.as_mut().unwrap().redo();
    }
    // the name and Save
    let save = Rect::new(r.right() - 10.0 - 170.0, r.y + 9.0, 170.0, 40.0);
    let field = Rect::new(save.x - 10.0 - 230.0, r.y + 11.0, 230.0, 36.0);
    if field.x > x + 90.0 {
        let s = l.livery.session.as_mut().unwrap();
        let mut name = std::mem::take(&mut s.ui.name);
        l.ui.text_input("livery-name", field, &mut name, "Name of the livery", None);
        s.ui.name = name;
    }
    let s = l.livery.session.as_ref().unwrap();
    if let Some((t, f)) = s.progress.clone() {
        l.ui.progress(Rect::new(save.x, save.y + 26.0, save.w, 6.0), f, true);
        l.ui.text_in(&t, Rect::new(save.x, save.y, save.w, 22.0), 12.0, Weight::Medium, TEXT_SOFT, Align::Left);
        l.ui.keep_moving();
    } else {
        let ready = s.ready.is_some();
        let label = if s.queued {
            "Saved once the game closes"
        } else if l.state.in_game() {
            "Save when the game closes"
        } else if s.project.placed.is_some() {
            "Save again"
        } else {
            "Save to game"
        };
        // (for the bus company: what saving it costs)
        let cost = super::company_cost(l);
        let label = match &cost {
            Some((_, amount, _)) if !s.queued => omsi_ui::tr("Save · %{amount}").replace("%{amount}", &super::eur(*amount)),
            _ => omsi_ui::tr(label).into_owned(),
        };
        if l.ui.button("livery-save", save, &label, Some(if s.queued { "schedule" } else { "save" }), if ready && !s.queued { ButtonKind::Primary } else { ButtonKind::Normal }) && ready {
            super::save(l);
        }
        if let Some((company, amount, cash)) = cost {
            let tip = omsi_ui::tr("For %{company}: the design and the painting cost %{amount}; it has %{cash}.").replace("%{company}", &company).replace("%{amount}", &super::eur(amount)).replace("%{cash}", &super::eur(cash));
            l.ui.tooltip(save, &tip);
        }
    }
}

fn tool_button(ui: &mut Ui, name: &str, r: Rect, label: &str, icon: &str, on: bool) -> bool {
    let id = crate::launcher::ui::id_of(name);
    let (h, _, clicked) = ui.interact(id, r);
    let t = ui.anim(id, if h { 1.0 } else { 0.0 }, 0.08);
    if on {
        ui.p().rounded(r, RADIUS, accent());
    } else if t > 0.01 {
        ui.p().rounded(r, RADIUS, HOVER.alpha(t));
    }
    let c = if on { on_accent() } else if h { TEXT } else { TEXT_SOFT };
    ui.icon(icon, Vec2::new(r.center().x, r.y + 15.0), 20.0, c);
    ui.text_in(label, Rect::new(r.x, r.y + 27.0, r.w, 16.0), 11.0, Weight::Medium, c, Align::Center);
    clicked
}

// --- the layers ------------------------------------------------------------------------------

fn layers_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = r.pad(14.0, 12.0);
    let head = l.ui.heading(inner, "Layers", None);
    let s = l.livery.session.as_mut().unwrap();
    let n = s.project.layers.len();
    let list = Rect::new(head.x - 4.0, head.y, head.w + 8.0, head.h - 92.0);
    let selected = s.selected.clone();
    let layers = s.project.layers.clone();
    let mut select: Option<String> = None;
    let mut toggle: Option<usize> = None;
    let mut lock: Option<usize> = None;
    // a layer dragged up or down the list: where it would go (a gap between the rows, from the top)
    let held = s.ui.layer_drag.clone();
    let mut pressed_on: Option<String> = None;
    let mut gap: Option<usize> = None;
    if layers.is_empty() {
        l.ui.paragraph("Nothing painted yet. Choose a tool above, or begin with the quick livery on the right.", Vec2::new(list.x + 6.0, list.y + 4.0), list.w - 12.0, 12.5, Weight::Regular, TEXT_FAINT);
    }
    l.ui.scroll_area("livery-layers", list, &mut |ui, a| {
        let m = ui.input.mouse;
        if let Some((_, _, true)) = &held {
            let g = (((m.y - a.y) / 38.0).round().max(0.0) as usize).min(n);
            gap = Some(g);
        }
        for (row_k, k) in (0..n).rev().enumerate() {
            let ly = &layers[k];
            let row = Rect::new(a.x, a.y + row_k as f32 * 38.0, a.w - 8.0, 34.0);
            let on = selected.as_deref() == Some(ly.id.as_str());
            if ui.input.pressed && Rect::new(row.x + 30.0, row.y, row.w - 60.0, row.h).contains(m) {
                pressed_on = Some(ly.id.clone());
            }
            if held.as_ref().is_some_and(|h| h.2 && h.0 == ly.id) {
                ui.p().rounded(row, RADIUS, HOVER.alpha(0.5));
            }
            if ui.row(&format!("livery-layer-{}", ly.id), Rect::new(row.x + 30.0, row.y, row.w - 60.0, row.h), on) {
                select = Some(ly.id.clone());
            }
            if on {
                ui.p().rounded(Rect::new(row.x, row.y, 30.0, row.h), RADIUS, Color::CLEAR);
            }
            let c = if on { on_accent() } else { TEXT };
            if ui.icon_button(&format!("livery-eye-{}", ly.id), Vec2::new(row.x + 15.0, row.center().y), 12.0, if ly.visible { "visibility" } else { "livery_eye_off" }, "Show or hide") {
                toggle = Some(k);
            }
            ui.icon(ly.kind.icon(), Vec2::new(row.x + 44.0, row.center().y), 16.0, if on { on_accent() } else { TEXT_DIM });
            let name = if ly.name.is_empty() { ly.kind.label().to_string() } else { ly.name.clone() };
            ui.text_in(&name, Rect::new(row.x + 58.0, row.y, row.w - 92.0, row.h), 13.0, Weight::Medium, if ly.visible { c } else { TEXT_FAINT }, Align::Left);
            if let Some(col) = ly.kind.colour().and_then(model::parse_hex) {
                let sw = Rect::new(row.right() - 52.0, row.center().y - 6.0, 12.0, 12.0);
                ui.p().rounded(sw, 3.0, Color::rgba(col[0], col[1], col[2], 1.0));
            }
            if ui.icon_button(&format!("livery-lock-{}", ly.id), Vec2::new(row.right() - 17.0, row.center().y), 11.0, if ly.locked { "lock" } else { "livery_unlock" }, "Lock") {
                lock = Some(k);
            }
        }
        if let Some(g) = gap {
            ui.p().rect(Rect::new(a.x + 4.0, a.y + g as f32 * 38.0 - 3.0, a.w - 16.0, 2.0), accent());
        }
        n as f32 * 38.0
    });
    // the drag: begun past a few pixels, the layer put into the gap on letting go
    let input = l.ui.input.clone();
    if let Some(id) = pressed_on {
        s.ui.layer_drag = Some((id, input.mouse, false));
    }
    let mut moved: Option<(usize, usize)> = None;
    if let Some((id, from, moving)) = s.ui.layer_drag.clone() {
        if input.down {
            if !moving && input.mouse.distance(from) > 6.0 {
                s.ui.layer_drag = Some((id, from, true));
            }
            if moving {
                l.ui.keep_moving();
            }
        } else {
            if let (true, Some(g), Some(i)) = (moving, gap, s.project.index_of(&id)) {
                // (the list shows the top layer first: gap g lies above the layer n - g)
                let row = n - 1 - i;
                let to_row = if g > row { g - 1 } else { g };
                let to = n - 1 - to_row.min(n - 1);
                if to != i {
                    moved = Some((i, to));
                }
            }
            s.ui.layer_drag = None;
        }
    }
    let before = s.project.doc();
    let mut changed = false;
    if let Some(id) = select {
        s.selected = Some(id);
        s.tool = Tool::Select;
    }
    if let Some(k) = toggle {
        s.project.layers[k].visible = !s.project.layers[k].visible;
        changed = true;
    }
    if let Some(k) = lock {
        s.project.layers[k].locked = !s.project.layers[k].locked;
        changed = true;
    }
    if let Some((from, to)) = moved {
        model::reorder(&mut s.project.layers, from, to);
        changed = true;
    }
    // what is done to the chosen layer
    let at = s.selected.as_ref().and_then(|id| s.project.index_of(id));
    let foot = Rect::new(inner.x, inner.bottom() - 82.0, inner.w, 36.0);
    let bw = (foot.w - 12.0) / 4.0;
    let acts = [("livery-up", "Up", "keyboard_arrow_up"), ("livery-down", "Down", "keyboard_arrow_down"), ("livery-dup", "Copy", "content_copy"), ("livery-del", "Delete", "delete")];
    for (k, (name, label, icon)) in acts.iter().enumerate() {
        let b = Rect::new(foot.x + k as f32 * (bw + 4.0), foot.y, bw, foot.h);
        if l.ui.button(name, b, "", Some(icon), ButtonKind::Normal) {
            if let Some(i) = at {
                match k {
                    0 if i + 1 < s.project.layers.len() => model::reorder(&mut s.project.layers, i, i + 1),
                    1 if i > 0 => model::reorder(&mut s.project.layers, i, i - 1),
                    2 => s.selected = model::duplicate(&mut s.project.layers, i),
                    3 if !s.project.layers[i].locked => {
                        s.project.layers.remove(i);
                        s.selected = None;
                    }
                    _ => {}
                }
                changed = true;
            }
        }
        l.ui.tooltip(b, label);
    }
    if changed {
        s.changed_from(before);
        s.settle();
    }
    let base = s.project.start_paint.clone().unwrap_or_else(|| omsi_ui::tr("the model's own").into_owned());
    l.ui.text_in(&format!("{} {base}", omsi_ui::tr("Begun from:")), Rect::new(inner.x, inner.bottom() - 34.0, inner.w, 18.0), 12.0, Weight::Regular, TEXT_DIM, Align::Left);
    let info = s.ready.as_ref().map(|r| format!("{} × {} · {:.0} {}", r.full.first().map(|f| f.0).unwrap_or(0), r.full.first().map(|f| f.1).unwrap_or(0), r.density * r.full.first().map(|f| f.0 as f32).unwrap_or(1.0) / r.bases.first().map(|b| b.width as f32).unwrap_or(1.0), omsi_ui::tr("texels/m")));
    if let Some(info) = info {
        l.ui.text_in(&info, Rect::new(inner.x, inner.bottom() - 16.0, inner.w - 22.0, 16.0), 11.5, Weight::Regular, TEXT_FAINT, Align::Left);
    }
    // every change kept at once: the mark lights up as it is written
    let c = Vec2::new(inner.right() - 8.0, inner.bottom() - 8.0);
    let lit = s.temp_at.is_some_and(|t| t.elapsed().as_secs_f32() < 0.8);
    l.ui.icon("save", c, 14.0, if lit { accent() } else { TEXT_FAINT });
    l.ui.tooltip(Rect::new(c.x - 10.0, c.y - 10.0, 20.0, 20.0), "Every change is kept at once in a temporary file: after a crash the studio opens with it");
}

// --- the right panel -------------------------------------------------------------------------

fn right_panel(l: &mut Launcher, r: Rect) {
    l.ui.panel(r);
    let inner = r.pad(16.0, 12.0);
    let s = l.livery.session.as_mut().unwrap();
    let before = s.project.doc();
    let quick_before = s.project.quick.clone();
    let shape_tex = l.livery.shape_tex.clone();
    let fonts = shapes::font_names();
    let mut actions: Vec<Action> = Vec::new();
    // what the bus offers as bus options (read on a worker the first time)
    let opts = BusOpts { cat: l.state.bus_options.catalogue(&l.state.config.root, &s.project.bus), technical_open: l.state.bus_options.technical_open };
    l.ui.scroll_area("livery-props", inner, &mut |ui, a| {
        let mut y = a.y;
        let w = a.w - 10.0;
        let sel = s.selected.clone().and_then(|id| s.project.index_of(&id));
        match sel {
            Some(i) => y = layer_props(ui, s, i, Rect::new(a.x, y, w, 0.0), &shape_tex, fonts, &mut actions),
            None => y = tool_props(ui, s, Rect::new(a.x, y, w, 0.0), &shape_tex, fonts, &opts, &mut actions),
        }
        y - a.y + 20.0
    });
    for act in actions {
        match act {
            Action::ChoosePicture => choose_picture(l),
            Action::ChooseLogo => choose_logo(l),
            Action::RemoveFromGame => super::remove_from_game(l),
            Action::BusOption(e) => {
                let s = l.livery.session.as_mut().unwrap();
                let before = s.project.doc();
                let old = s.project.options.clone();
                match e {
                    busoptions::Edit::Set(var, Some(v)) => {
                        s.project.options.insert(var, v);
                    }
                    busoptions::Edit::Set(var, None) => {
                        s.project.options.remove(&var);
                    }
                    busoptions::Edit::Reset => s.project.options.clear(),
                    busoptions::Edit::Technical => l.state.bus_options.edit(&s.project.bus, busoptions::Edit::Technical),
                }
                let s = l.livery.session.as_mut().unwrap();
                if s.project.options != old {
                    s.changed_from(before);
                    s.settle();
                }
            }
        }
    }
    let s = l.livery.session.as_mut().unwrap();
    if s.project.quick != quick_before {
        if let (Some(q), Some(d)) = (s.project.quick.clone(), s.dims()) {
            s.project.layers = model::apply_quick(&s.project.layers, &q, &d, &shapes::text_width);
        }
    }
    if s.project.doc() != before {
        s.changed_from(before);
    }
}

enum Action {
    ChoosePicture,
    ChooseLogo,
    RemoveFromGame,
    BusOption(busoptions::Edit),
}

/// The bus options the bus offers, for the panel.
struct BusOpts {
    cat: Option<std::sync::Arc<busoptions::Catalogue>>,
    technical_open: bool,
}

fn section(ui: &mut Ui, x: f32, y: f32, w: f32, title: &str) -> f32 {
    ui.heading(Rect::new(x, y, w, 26.0), title, None);
    y + 28.0
}

fn hint(ui: &mut Ui, x: f32, y: f32, w: f32, text: &str) -> f32 {
    y + ui.paragraph(text, Vec2::new(x, y), w, 12.5, Weight::Regular, TEXT_DIM) + 8.0
}

/// What a tool offers before anything is chosen.
fn tool_props(ui: &mut Ui, s: &mut Session, r: Rect, shape_tex: &std::collections::HashMap<&'static str, usize>, fonts: &[String], opts: &BusOpts, actions: &mut Vec<Action>) -> f32 {
    let (x, w) = (r.x, r.w);
    let mut y = r.y;
    match s.tool {
        Tool::Select => {
            y = section(ui, x, y, w, "Quick livery");
            y = hint(ui, x, y, w, "Three colours, a stripe, a name and a logo: every choice becomes a layer you can change afterwards.");
            let mut q = s.project.quick.clone().unwrap_or_default();
            // the three colours
            let labels = ["Body", "Stripe", "Name"];
            for k in 0..3 {
                let b = Rect::new(x + k as f32 * (w / 3.0), y, w / 3.0 - 8.0, 52.0);
                let id = crate::launcher::ui::id_of(&format!("livery-quick-{k}"));
                let (h, _, clicked) = ui.interact(id, b);
                let on = s.picker.target == PickFor::Quick(k);
                ui.p().rounded(b, RADIUS, if on { HOVER } else if h { HOVER.alpha(0.6) } else { FIELD });
                let sw = Rect::new(b.x + 8.0, b.y + 8.0, b.w - 16.0, 20.0);
                match q.colours[k].as_deref().and_then(model::parse_hex) {
                    Some(c) => ui.p().rounded(sw, 5.0, Color::rgba(c[0], c[1], c[2], 1.0)),
                    None => ui.p().rounded_border(sw, 5.0, 1.0, TEXT_FAINT),
                }
                if on {
                    ui.p().rounded_border(b, RADIUS, 1.5, accent());
                }
                ui.text_in(labels[k], Rect::new(b.x, b.y + 30.0, b.w, 18.0), 11.5, Weight::Medium, TEXT_SOFT, Align::Center);
                if clicked {
                    s.picker.target = PickFor::Quick(k);
                    if let Some(c) = q.colours[k].as_deref().and_then(model::parse_hex) {
                        s.picker.colour = c;
                    }
                }
            }
            y += 60.0;
            if let PickFor::Quick(k) = s.picker.target {
                let mut c = q.colours[k].as_deref().and_then(model::parse_hex).unwrap_or(s.picker.colour);
                let (changed, h) = picker(ui, "livery-quick-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
                if changed {
                    q.colours[k] = Some(model::hex(c));
                    remember(&mut s.recent, c);
                }
                y += h + 8.0;
            }
            y = section(ui, x, y, w, "Stripe");
            let chosen = q.stripe_chosen || q.colours[1].is_some();
            if let Some(t) = stripe_grid(ui, "livery-quick-stripe", Rect::new(x, y, w, 0.0), chosen.then_some(q.stripe), &q.colours[1].clone().unwrap_or_else(|| "#ffffff".into()), &mut y) {
                q.stripe = t;
                q.stripe_chosen = true;
            }
            y = section(ui, x, y + 6.0, w, "Name on the bus");
            let mut name = q.name.clone();
            if ui.text_input("livery-quick-name", Rect::new(x, y, w, 36.0), &mut name, "Text", None) {
                q.name = name;
            }
            y += 46.0;
            y = section(ui, x, y, w, "Logo");
            if ui.button("livery-quick-logo", Rect::new(x, y, w - 46.0, 36.0), if q.logo.is_some() { "Another logo…" } else { "Choose a logo…" }, Some("upload"), ButtonKind::Normal) {
                actions.push(Action::ChooseLogo);
            }
            if q.logo.is_some() && ui.icon_button("livery-quick-logo-off", Vec2::new(x + w - 20.0, y + 18.0), 14.0, "close", "No logo") {
                q.logo = None;
            }
            y += 46.0;
            if q != s.project.quick.clone().unwrap_or_default() {
                s.project.quick = Some(q);
            }
            y = hint(ui, x, y + 6.0, w, "Click a layer on the bus to change it, drag it to move it. Right-drag turns the bus, the wheel zooms, 1-6 are the views, F fits, O held shows the livery before.");
            // the bus options the livery sets (its mirrors, rims, displays): on the bus at once,
            // written into the livery
            if let Some(cat) = opts.cat.as_deref() {
                let livery = s.project.start_paint.clone().unwrap_or_default();
                let picks = s.project.options.clone();
                let section = busoptions::Section { cat, livery: &livery, picks: &picks, technical_open: opts.technical_open };
                let (h, e) = busoptions::section(ui, x, y + 4.0, w, &section);
                if let Some(e) = e {
                    actions.push(Action::BusOption(e));
                }
                y += h + if h > 0.0 { 12.0 } else { 0.0 };
            }
            // the buses of its family: the livery fits them as it is
            if !s.family.is_empty() {
                y = section(ui, x, y + 4.0, w, "Also on");
                y = hint(ui, x, y, w, "These buses take the same paint. Look at the livery on them; those with a folder of their own get it too when you tick them.");
                let own = s.parts_ctc();
                for kin in s.family.clone() {
                    let r = Rect::new(x, y, w, 34.0);
                    let looking = s.look.bus == kin.bus;
                    ui.text_in(&kin.name, Rect::new(r.x, r.y, r.w - 120.0, r.h), 12.5, Weight::Medium, if looking { accent() } else { TEXT }, Align::Left);
                    if ui.button(&format!("livery-kin-look-{}", kin.bus), Rect::new(r.right() - 112.0, r.y + 2.0, 72.0, 30.0), if looking { "Back" } else { "Look" }, None, ButtonKind::Normal) {
                        if looking {
                            s.look.bus = s.project.bus.clone();
                            s.look.paint = s.project.start_paint.clone().unwrap_or_default();
                        } else {
                            s.look.bus = kin.bus.clone();
                            s.look.paint = String::new();
                        }
                    }
                    if kin.ctc.is_some() && kin.ctc != own {
                        let mut on = s.project.family.contains(&kin.bus);
                        if ui.toggle(&format!("livery-kin-save-{}", kin.bus), Rect::new(r.right() - 36.0, r.y + 2.0, 36.0, 30.0), &mut on, "") {
                            if on {
                                s.project.family.push(kin.bus.clone());
                            } else {
                                s.project.family.retain(|b| *b != kin.bus);
                            }
                        }
                        ui.tooltip(Rect::new(r.right() - 36.0, r.y + 2.0, 36.0, 30.0), "Save the livery for this bus too");
                    } else {
                        ui.icon("check", Vec2::new(r.right() - 18.0, r.center().y), 15.0, TEXT_DIM);
                        ui.tooltip(Rect::new(r.right() - 36.0, r.y + 2.0, 36.0, 30.0), "It shares the bus's folder: the livery comes on it by itself");
                    }
                    y += 38.0;
                }
                y += 6.0;
            }
            // the mirror plane, moved a little off the middle for a bus that is not symmetric
            if let (true, Some(d)) = (s.project.mirror.on, s.dims()) {
                y = section(ui, x, y + 4.0, w, "Mirror");
                let mut off = (s.project.mirror.plane_x.unwrap_or(d.middle_x()) - d.middle_x()) * 100.0;
                if ui.slider("livery-mirror-plane", Rect::new(x, y, w, 30.0), &mut off, -30.0, 30.0, 0.5, "Mirror plane", &|v| format!("{v:+.1} cm")) {
                    s.project.mirror.plane_x = (off.abs() >= 0.25).then(|| d.middle_x() + off / 100.0);
                }
                ui.tooltip(Rect::new(x, y, w, 30.0), "Where the copies on the other side are mirrored, from the bus's middle");
                y += 40.0;
            }
            // out of the game again
            if s.project.placed.is_some() && s.export.is_none() {
                let asked = s.ui.remove_asked.is_some_and(|t| t.elapsed().as_secs_f32() < 4.0);
                let label = if asked { "Click again to remove it" } else { "Remove from the game" };
                if ui.button("livery-remove", Rect::new(x, y, w, 36.0), label, Some("delete"), if asked { ButtonKind::Danger } else { ButtonKind::Normal }) {
                    if asked {
                        s.ui.remove_asked = None;
                        actions.push(Action::RemoveFromGame);
                    } else {
                        s.ui.remove_asked = Some(std::time::Instant::now());
                    }
                }
                ui.tooltip(Rect::new(x, y, w, 36.0), "Takes the saved livery out of the game; the design stays here");
                y += 46.0;
            }
        }
        Tool::Fill => {
            y = section(ui, x, y, w, "Fill");
            y = hint(ui, x, y, w, "Click a colour on the bus: all of that colour takes the new one. Rubbers, lamps and windows keep theirs.");
            let mut c = s.picker.colour;
            s.picker.target = PickFor::Main;
            let (_, h) = picker(ui, "livery-fill-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
            s.picker.colour = c;
            y += h + 10.0;
            if ui.button("livery-fill-all", Rect::new(x, y, w, 38.0), "Paint the whole bus", Some("livery_fill"), ButtonKind::Normal) {
                let mut ly = model::layer("Base colour", model::base_colour(&model::hex(c)));
                ly.from_recipe = false;
                s.selected = Some(ly.id.clone());
                s.project.layers.insert(0, ly);
                remember(&mut s.recent, c);
            }
            y += 48.0;
        }
        Tool::Stripe => {
            y = section(ui, x, y, w, "Stripe");
            y = hint(ui, x, y, w, "Choose a stripe: it is laid round the bus at the height of its windows. Drag its edges on the bus afterwards.");
            let col = model::hex(s.picker.colour);
            if let Some(t) = stripe_grid(ui, "livery-stripes", Rect::new(x, y, w, 0.0), None, &col, &mut y) {
                if let Some(d) = s.dims() {
                    let ly = model::layer(t.label(), model::stripe(t, &col, &d));
                    s.selected = Some(ly.id.clone());
                    s.project.layers.push(ly);
                    s.tool = Tool::Select;
                }
            }
            y += 6.0;
            let mut c = s.picker.colour;
            let (_, h) = picker(ui, "livery-stripe-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
            s.picker.colour = c;
            y += h;
        }
        Tool::Text => {
            y = section(ui, x, y, w, "Text");
            y = hint(ui, x, y, w, "Type the text, then click on the bus where it goes.");
            if s.ui.text.is_empty() && s.ui.size_cm == 0.0 {
                s.ui.text = omsi_ui::tr("Text").into_owned();
                s.ui.size_cm = 25.0;
            }
            let mut t = std::mem::take(&mut s.ui.text);
            ui.text_input("livery-text-new", Rect::new(x, y, w, 36.0), &mut t, "Text", None);
            s.ui.text = t;
            y += 44.0;
            let names: Vec<String> = fonts.to_vec();
            ui.select("livery-text-font", Rect::new(x, y, w, 34.0), &mut s.ui.font, &names);
            y += 42.0;
            ui.slider("livery-text-size", Rect::new(x, y, w, 30.0), &mut s.ui.size_cm, 2.0, 150.0, 1.0, "Letter height", &|v| format!("{v:.0} cm"));
            y += 40.0;
            let mut c = s.picker.colour;
            let (_, h) = picker(ui, "livery-text-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
            s.picker.colour = c;
            y += h;
        }
        Tool::Image => {
            y = section(ui, x, y, w, "Picture");
            y = hint(ui, x, y, w, "A logo, a coat of arms, a photo: PNG, JPG, BMP, TGA or SVG. Choose one or drop it onto the window, then click on the bus where it goes.");
            if ui.button("livery-pick-picture", Rect::new(x, y, w, 40.0), "Choose a picture…", Some("upload"), ButtonKind::Primary) {
                actions.push(Action::ChoosePicture);
            }
            y += 50.0;
            if let Some(Armed::Picture { hash, .. }) = &s.ui.armed {
                if let Some(p) = s.pictures.get(hash) {
                    y = hint(ui, x, y, w, &omsi_ui::tr("Ready to place: %{w} × %{h} pixels. Click on the bus.").replace("%{w}", &p.w.to_string()).replace("%{h}", &p.h.to_string()));
                }
            }
        }
        Tool::Shape => {
            y = section(ui, x, y, w, "Shapes");
            y = hint(ui, x, y, w, "Choose a shape, then click on the bus where it goes. With the pen you draw shapes of your own.");
            let armed = match &s.ui.armed {
                Some(Armed::Shape(k)) => Some(*k),
                _ => None,
            };
            if let Some(k) = shape_grid(ui, "livery-shapes", Rect::new(x, y, w, 0.0), armed, shape_tex, &mut y) {
                s.ui.armed = Some(Armed::Shape(k));
            }
            y += 8.0;
            let mut c = s.picker.colour;
            let (_, h) = picker(ui, "livery-shape-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
            s.picker.colour = c;
            y += h;
        }
        Tool::Brush => {
            y = section(ui, x, y, w, if s.ui.eraser { "Eraser" } else { "Brush" });
            y = hint(ui, x, y, w, "Paint on the bus with the left button. E switches between the brush and the eraser, [ and ] make it smaller and larger. The strokes are kept as lines on the bus: they stay sharp at any size.");
            let mut k = s.ui.eraser as usize;
            if ui.segmented("livery-brush-mode", Rect::new(x, y, w, 32.0), &mut k, &["Brush", "Eraser"]) {
                s.ui.eraser = k == 1;
            }
            y += 42.0;
            ui.slider("livery-brush-size", Rect::new(x, y, w, 30.0), &mut s.ui.brush_cm, 0.5, 100.0, 0.5, "Size", &|v| format!("{v:.1} cm"));
            y += 36.0;
            let mut hd = s.ui.hardness * 100.0;
            if ui.slider("livery-brush-hardness", Rect::new(x, y, w, 30.0), &mut hd, 0.0, 100.0, 1.0, "Hardness", &|v| format!("{v:.0}%")) {
                s.ui.hardness = hd / 100.0;
            }
            y += 36.0;
            let mut op = s.ui.brush_opacity * 100.0;
            if ui.slider("livery-brush-opacity", Rect::new(x, y, w, 30.0), &mut op, 1.0, 100.0, 1.0, "Cover", &|v| format!("{v:.0}%")) {
                s.ui.brush_opacity = op / 100.0;
            }
            y += 40.0;
            if !s.ui.eraser {
                let mut c = s.picker.colour;
                let (_, h) = picker(ui, "livery-brush-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
                s.picker.colour = c;
                y += h;
            }
        }
        Tool::Pen => {
            y = section(ui, x, y, w, "Pen");
            y = hint(ui, x, y, w, "Click the corners of your shape on the bus. Hold the button and drag to bend the line into a curve. Click the first corner again or press Enter to finish; Escape lets go.");
            let n = s.ui.pen.nodes.len();
            if n > 0 {
                y = hint(ui, x, y, w, &omsi_ui::tr("%{n} corners").replace("%{n}", &n.to_string()));
                if n >= 3 && ui.button("livery-pen-done", Rect::new(x, y, w * 0.5 - 4.0, 36.0), "Finish", Some("check"), ButtonKind::Primary) {
                    finish_pen(s);
                }
                if ui.button("livery-pen-cancel", Rect::new(x + w * 0.5 + 4.0, y, w * 0.5 - 4.0, 36.0), "Cancel", Some("close"), ButtonKind::Normal) {
                    s.ui.pen = Pen::default();
                }
                y += 46.0;
            }
            let mut c = s.picker.colour;
            let (_, h) = picker(ui, "livery-pen-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &s.recent);
            s.picker.colour = c;
            y += h;
        }
    }
    y
}

/// How much (0..1) of a decal at `place`, `w` x `h` metres, lies on texels the bus's two sides
/// share (a texture used for both): there it shows on the other side too, the wrong way round.
fn shared_part(s: &Session, place: &Place, w: f32, h: f32) -> f32 {
    let Some(r) = s.ready.as_ref() else { return 0.0 };
    let (u, v, n) = place.side.axes();
    let (sn, cs) = place.rotation.to_radians().sin_cos();
    let (mut on, mut all) = (0, 0);
    for a in 0..8 {
        for b in 0..4 {
            let (x, y) = ((a as f32 + 0.5) / 8.0 - 0.5, (b as f32 + 0.5) / 4.0 - 0.5);
            let (x, y) = (x * w, y * h);
            let p = place.centre + u * (x * cs - y * sn) + v * (x * sn + y * cs);
            let Some((tri, _, _, bw)) = r.geom.hit(p + n * 0.6, -n) else { continue };
            let Some((k, i)) = texel(s, tri, bw) else { continue };
            all += 1;
            if r.canvases.get(k).is_some_and(|c| c.bake.second.get(i).is_some_and(|x| *x != u32::MAX)) {
                on += 1;
            }
        }
    }
    if all == 0 { 0.0 } else { on as f32 / all as f32 }
}

/// The nearest place along the bus (and up or down a little) where the decal lies on no shared
/// texels.
fn free_spot(s: &Session, place: &Place, w: f32, h: f32) -> Option<Place> {
    let d = s.dims()?;
    let (u, v, _) = place.side.axes();
    let mut steps: Vec<f32> = Vec::new();
    let mut k = 0.2;
    while k < d.length() {
        steps.push(k);
        steps.push(-k);
        k += 0.2;
    }
    for up in [0.0, 0.25, -0.25, 0.5, -0.5] {
        for &along in std::iter::once(&0.0).chain(&steps) {
            let mut p = place.clone();
            p.centre += u * along + v * up;
            if p.centre.y < d.min.y || p.centre.y > d.max.y || p.centre.z < d.min.z || p.centre.z > d.max.z {
                continue;
            }
            if shared_part(s, &p, w, h) <= 0.02 {
                return Some(p);
            }
        }
    }
    None
}

/// What the chosen layer offers.
fn layer_props(ui: &mut Ui, s: &mut Session, i: usize, r: Rect, shape_tex: &std::collections::HashMap<&'static str, usize>, fonts: &[String], actions: &mut Vec<Action>) -> f32 {
    let (x, w) = (r.x, r.w);
    let mut y = r.y;
    let dims = s.dims();
    let windows = s.ready.as_ref().and_then(|r| r.windows);
    let mirror_on = s.project.mirror.on;
    // a decal on texels both sides share: said, and a free spot offered
    let decal = s.project.layers.get(i).filter(|l| !l.locked).and_then(|l| Some((l.kind.place()?.clone(), size_of(s, l)?)));
    if let Some((place, (dw, dh))) = decal {
        let key = format!("{}|{:?}|{dw}|{dh}", s.project.layers[i].id, place);
        let part = match &s.ui.shared {
            Some((k, f)) if *k == key => *f,
            _ => {
                let f = shared_part(s, &place, dw, dh);
                s.ui.shared = Some((key, f));
                f
            }
        };
        if part > 0.02 {
            ui.p().rounded(Rect::new(x, y, w, 4.0), 2.0, DANGER.alpha(0.6));
            y = hint(ui, x, y + 8.0, w, "Here the bus's two sides share their paint: what stands here shows on the other side too, the wrong way round.");
            if ui.button("livery-free-spot", Rect::new(x, y, w, 34.0), "Move to a free spot", Some("open_with"), ButtonKind::Normal) {
                match free_spot(s, &place, dw, dh) {
                    Some(p) => {
                        if let Some(pl) = s.project.layers[i].kind.place_mut() {
                            *pl = p;
                        }
                    }
                    None => s.say(omsi_ui::tr("No free spot was found on this side."), false),
                }
            }
            y += 44.0;
        }
    }
    let recent = s.recent.clone();
    // (a picture's own proportions, for its size fields)
    let picture_aspect = match s.project.layers.get(i).map(|l| &l.kind) {
        Some(Kind::Image { image, aspect, .. }) => Some(paint::picture_aspect(image, *aspect, &s.pictures)),
        _ => None,
    };
    let Some(layer) = s.project.layers.get_mut(i) else { return y };
    let label = layer.kind.label();
    ui.icon(layer.kind.icon(), Vec2::new(x + 10.0, y + 14.0), 18.0, TEXT);
    ui.text_in(label, Rect::new(x + 26.0, y + 2.0, w - 60.0, 24.0), 15.0, Weight::Bold, TEXT, Align::Left);
    if ui.icon_button("livery-deselect", Vec2::new(x + w - 14.0, y + 14.0), 13.0, "close", "Done (Escape)") {
        s.selected = None;
        return y + 30.0;
    }
    y += 34.0;
    if layer.locked {
        y = hint(ui, x, y, w, "This layer is locked: unlock it in the list to change it.");
    }
    let mut name = layer.name.clone();
    if ui.text_input(&format!("livery-layer-name-{}", layer.id), Rect::new(x, y, w, 34.0), &mut name, "Layer name", None) && !layer.locked {
        layer.name = name;
    }
    y += 42.0;
    let locked = layer.locked;
    // what the kind has
    let mut kind = layer.kind.clone();
    match &mut kind {
        Kind::Fill { radius, .. } => {
            if *radius < 999.0 {
                ui.slider("livery-fill-radius", Rect::new(x, y, w, 30.0), radius, 2.0, 60.0, 1.0, "Colour range", &|v| format!("ΔE {v:.0}"));
                y += 38.0;
            }
        }
        Kind::Stripe { template, h1, h2, angle, wave, sides, .. } => {
            let opts: Vec<String> = StripeTemplate::ALL.iter().map(|t| omsi_ui::tr(t.label()).into_owned()).collect();
            let mut k = StripeTemplate::ALL.iter().position(|t| t == template).unwrap_or(0);
            if ui.select("livery-stripe-template", Rect::new(x, y, w, 34.0), &mut k, &opts) {
                if let Some(d) = dims {
                    if let Kind::Stripe { h1: a, h2: b, angle: an, wave: wv, sides: sd, .. } = model::stripe(StripeTemplate::ALL[k], "#fff", &d) {
                        (*template, *h1, *h2, *angle, *wave, *sides) = (StripeTemplate::ALL[k], a, b, an, wv, sd);
                    }
                }
            }
            y += 42.0;
            let top = dims.map(|d| d.height() + 0.2).unwrap_or(4.0);
            ui.slider("livery-stripe-h1", Rect::new(x, y, w, 30.0), h1, -0.5, top, 0.01, "Lower edge", &|v| format!("{v:.2} m"));
            y += 36.0;
            ui.slider("livery-stripe-h2", Rect::new(x, y, w, 30.0), h2, -0.5, top, 0.01, "Upper edge", &|v| format!("{v:.2} m"));
            y += 36.0;
            ui.slider("livery-stripe-angle", Rect::new(x, y, w, 30.0), angle, -20.0, 20.0, 0.5, "Slant", &|v| format!("{v:.1}°"));
            y += 36.0;
            ui.slider("livery-stripe-wave", Rect::new(x, y, w, 30.0), wave, 0.0, 0.5, 0.01, "Wave", &|v| format!("{v:.2} m"));
            y += 38.0;
            let mut k = match sides {
                model::Sides::All => 0,
                model::Sides::Sides => 1,
                model::Sides::Front => 2,
                model::Sides::Rear => 3,
            };
            if ui.segmented("livery-stripe-sides", Rect::new(x, y, w, 32.0), &mut k, &["All round", "Sides", "Front", "Rear"]) {
                *sides = [model::Sides::All, model::Sides::Sides, model::Sides::Front, model::Sides::Rear][k];
            }
            y += 42.0;
        }
        Kind::Text { text, font, height_cm, spacing, place, .. } => {
            ui.text_input("livery-text-edit", Rect::new(x, y, w, 36.0), text, "Text", None);
            y += 44.0;
            let mut k = fonts.iter().position(|f| f == font).unwrap_or(0);
            if ui.select("livery-text-font-edit", Rect::new(x, y, w, 34.0), &mut k, fonts) {
                *font = fonts[k].clone();
            }
            y += 42.0;
            ui.slider("livery-text-height", Rect::new(x, y, w, 30.0), height_cm, 2.0, 150.0, 0.5, "Letter height", &|v| format!("{v:.0} cm"));
            y += 36.0;
            ui.slider("livery-text-spacing", Rect::new(x, y, w, 30.0), spacing, -10.0, 60.0, 1.0, "Letter spacing", &|v| format!("{v:.0}%"));
            y += 40.0;
            y = place_props(ui, place, x, y, w, mirror_on, false);
        }
        Kind::Image { white_clear, place, .. } => {
            if ui.button("livery-image-other", Rect::new(x, y, w, 36.0), "Another picture…", Some("upload"), ButtonKind::Normal) {
                actions.push(Action::ChoosePicture);
            }
            y += 44.0;
            ui.toggle("livery-image-white", Rect::new(x, y, w, 30.0), white_clear, "White becomes transparent");
            y += 38.0;
            y = size_props(ui, place, picture_aspect, x, y, w);
            y = place_props(ui, place, x, y, w, mirror_on, false);
        }
        Kind::Shape { shape, place, .. } => {
            if let Some(k) = shape_grid(ui, "livery-shape-edit", Rect::new(x, y, w, 0.0), Some(shape_key(shape)), shape_tex, &mut y) {
                *shape = k.to_string();
            }
            y += 6.0;
            y = size_props(ui, place, None, x, y, w);
            y = place_props(ui, place, x, y, w, mirror_on, true);
        }
        Kind::Path { place, .. } => {
            y = size_props(ui, place, None, x, y, w);
            y = place_props(ui, place, x, y, w, mirror_on, true);
        }
        Kind::Brush { strokes, .. } => {
            let (n, e) = (strokes.iter().filter(|s| !s.erase).count(), strokes.iter().filter(|s| s.erase).count());
            y = hint(ui, x, y, w, &omsi_ui::tr("%{n} strokes of the brush, %{e} of the eraser. Choose the brush tool to paint on in this layer.").replace("%{n}", &n.to_string()).replace("%{e}", &e.to_string()));
            if !strokes.is_empty() && ui.button("livery-brush-undo-stroke", Rect::new(x, y, w, 34.0), "Take back the last stroke", Some("livery_undo"), ButtonKind::Normal) {
                strokes.pop();
            }
            y += 42.0;
        }
    }
    if !locked {
        layer.kind = kind;
    }
    // the colours: the layer's, its outline's and its gradient's end
    if layer.kind.colour().is_some() {
        y = section(ui, x, y, w, "Colour");
        let has_outline = layer.kind.outline_mut().is_some();
        let mut opts: Vec<&str> = vec!["Colour"];
        if has_outline {
            opts.push("Outline");
        }
        opts.push("Gradient");
        let mut k = match s.picker.target {
            PickFor::Outline if has_outline => 1,
            PickFor::GradientEnd => opts.len() - 1,
            _ => 0,
        };
        if ui.segmented("livery-colour-for", Rect::new(x, y, w, 30.0), &mut k, &opts) {
            s.picker.target = if k == 0 { PickFor::Main } else if has_outline && k == 1 { PickFor::Outline } else { PickFor::GradientEnd };
        }
        y += 40.0;
        let target = match s.picker.target {
            PickFor::Quick(_) => PickFor::Main,
            t => t,
        };
        match target {
            PickFor::Outline => {
                let o = layer.kind.outline_mut().unwrap();
                let mut on = o.is_some();
                if ui.toggle("livery-outline-on", Rect::new(x, y, w, 30.0), &mut on, "Outline") && !locked {
                    *o = on.then(|| model::Outline { colour: "#ffffff".into(), width_cm: 2.0 });
                }
                y += 36.0;
                if let Some(ol) = o.as_mut() {
                    ui.slider("livery-outline-width", Rect::new(x, y, w, 30.0), &mut ol.width_cm, 0.2, 10.0, 0.1, "Width", &|v| format!("{v:.1} cm"));
                    y += 38.0;
                    let mut c = model::parse_hex(&ol.colour).unwrap_or([255, 255, 255]);
                    let (changed, h) = picker(ui, "livery-outline-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &recent);
                    if changed && !locked {
                        ol.colour = model::hex(c);
                    }
                    y += h;
                }
            }
            PickFor::GradientEnd => {
                let g = layer.kind.gradient_mut().unwrap();
                let mut on = g.is_some();
                if ui.toggle("livery-gradient-on", Rect::new(x, y, w, 30.0), &mut on, "Gradient") && !locked {
                    *g = on.then(|| model::Gradient { kind: model::GradientKind::Linear, colour2: "#ffffff".into(), angle: 0.0 });
                }
                y += 36.0;
                if let Some(gr) = g.as_mut() {
                    let mut k = if gr.kind == model::GradientKind::Radial { 1 } else { 0 };
                    if ui.segmented("livery-gradient-kind", Rect::new(x, y, w, 30.0), &mut k, &["Linear", "Radial"]) {
                        gr.kind = if k == 1 { model::GradientKind::Radial } else { model::GradientKind::Linear };
                    }
                    y += 38.0;
                    if gr.kind == model::GradientKind::Linear {
                        ui.slider("livery-gradient-angle", Rect::new(x, y, w, 30.0), &mut gr.angle, -180.0, 180.0, 5.0, "Direction", &|v| format!("{v:.0}°"));
                        y += 38.0;
                    }
                    let mut c = model::parse_hex(&gr.colour2).unwrap_or([255, 255, 255]);
                    let (changed, h) = picker(ui, "livery-gradient-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &recent);
                    if changed && !locked {
                        gr.colour2 = model::hex(c);
                    }
                    y += h;
                }
            }
            _ => {
                let mut c = layer.kind.colour().and_then(model::parse_hex).unwrap_or([255, 255, 255]);
                let (changed, h) = picker(ui, "livery-layer-picker", Rect::new(x, y, w, 0.0), &mut s.picker, &mut c, &recent);
                if changed && !locked {
                    layer.kind.set_colour(model::hex(c));
                    s.picker.colour = c;
                }
                y += h;
            }
        }
    }
    // how it is painted
    let layer = &mut s.project.layers[i];
    y = section(ui, x, y + 6.0, w, "Painting");
    let mut o = layer.opacity * 100.0;
    if ui.slider("livery-opacity", Rect::new(x, y, w, 30.0), &mut o, 0.0, 100.0, 1.0, "Opacity", &|v| format!("{v:.0}%")) && !layer.locked {
        layer.opacity = o / 100.0;
    }
    y += 36.0;
    let mut d = layer.detail * 100.0;
    if ui.slider("livery-detail", Rect::new(x, y, w, 30.0), &mut d, 0.0, 100.0, 1.0, "Keep details", &|v| format!("{v:.0}%")) && !layer.locked {
        layer.detail = d / 100.0;
    }
    ui.tooltip(Rect::new(x, y, w, 30.0), "How much of the seams, dirt and shading of the paint below shows through");
    y += 38.0;
    let mut t = layer.over_trim;
    if ui.toggle("livery-over-trim", Rect::new(x, y, w, 30.0), &mut t, "Also over rubbers and lamps") && !layer.locked {
        layer.over_trim = t;
    }
    y += 36.0;
    // the windows: a print on the glass (window advertising, a wrap over the windows)
    if windows.is_some() {
        let mut g = layer.over_glass;
        if ui.toggle("livery-over-glass", Rect::new(x, y, w, 30.0), &mut g, "Also over the windows") && !layer.locked {
            layer.over_glass = g;
        }
        y += 36.0;
        if windows == Some(false) && layer.over_glass {
            y = hint(ui, x, y, w, "This bus's windows cannot carry a livery: their texture is not one a livery can change.");
        }
    }
    y + 4.0
}

fn shape_key(s: &str) -> &'static str {
    shapes::SHAPES.iter().find(|(k, _)| *k == s).map(|(k, _)| *k).unwrap_or("rechthoek")
}

/// A decal's size: its width and height, and the lock that keeps them in proportion (off: the
/// one stretches without the other). `aspect`: a picture's own height over its width.
fn size_props(ui: &mut Ui, place: &mut Place, aspect: Option<f32>, x: f32, mut y: f32, w: f32) -> f32 {
    let mut width = place.width_m;
    if ui.slider("livery-place-width", Rect::new(x, y, w, 30.0), &mut width, 0.05, 12.0, 0.01, "Width", &|v| format!("{v:.2} m")) {
        place.resize(Some(width), None, aspect);
    }
    y += 36.0;
    let mut h = place.height_m.unwrap_or(place.width_m * aspect.unwrap_or(1.0));
    if ui.slider("livery-place-height", Rect::new(x, y, w, 30.0), &mut h, 0.05, 4.0, 0.01, "Height", &|v| format!("{v:.2} m")) {
        place.resize(None, Some(h), aspect);
    }
    y += 36.0;
    ui.toggle("livery-place-ratio", Rect::new(x, y, w, 30.0), &mut place.keep_ratio, "Keep proportions");
    y + 40.0
}

/// Where a decal stands: its side, its turn, and its copy on the other side (`shape`: a mirror
/// image unless the player chose otherwise).
fn place_props(ui: &mut Ui, place: &mut Place, x: f32, mut y: f32, w: f32, mirror_on: bool, shape: bool) -> f32 {
    ui.slider("livery-place-rotation", Rect::new(x, y, w, 30.0), &mut place.rotation, -180.0, 180.0, 1.0, "Rotation", &|v| format!("{v:.0}°"));
    y += 38.0;
    let sides = [Side::L, Side::R, Side::V, Side::A, Side::D];
    let mut k = sides.iter().position(|s| *s == place.side).unwrap_or(0);
    if ui.segmented("livery-place-side", Rect::new(x, y, w, 30.0), &mut k, &["Left", "Right", "Front", "Rear", "Roof"]) {
        place.side = sides[k];
    }
    y += 40.0;
    if mirror_on && matches!(place.side, Side::L | Side::R) {
        let mut coupled = place.mirror == Coupling::Coupled;
        if ui.toggle("livery-place-mirror", Rect::new(x, y, w, 30.0), &mut coupled, "Also on the other side") {
            place.mirror = if coupled { Coupling::Coupled } else { Coupling::Single };
        }
        y += 36.0;
        if coupled {
            // (off: the copy reads the same way, as letters must; a shape's is a mirror image)
            let mut image = place.mirror_image_or(shape);
            if ui.toggle("livery-place-image", Rect::new(x, y, w, 30.0), &mut image, "Mirror image on the other side") {
                place.set_mirror_image(image);
            }
            y += 36.0;
        }
    }
    y + 4.0
}

/// The eight stripe templates as small pictures of a bus; returns the one clicked.
fn stripe_grid(ui: &mut Ui, name: &str, r: Rect, chosen: Option<StripeTemplate>, colour: &str, y: &mut f32) -> Option<StripeTemplate> {
    let cols = 4;
    let tw = (r.w - (cols - 1) as f32 * 8.0) / cols as f32;
    let th = 58.0;
    let c = model::parse_hex(colour).map(|c| Color::rgba(c[0], c[1], c[2], 1.0)).unwrap_or(Color::WHITE);
    let mut out = None;
    for (k, t) in StripeTemplate::ALL.iter().enumerate() {
        let b = Rect::new(r.x + (k % cols) as f32 * (tw + 8.0), *y + (k / cols) as f32 * (th + 8.0), tw, th);
        let id = crate::launcher::ui::id_of(&format!("{name}-{k}"));
        let (h, _, clicked) = ui.interact(id, b);
        let on = chosen == Some(*t);
        ui.p().rounded(b, 8.0, if on { HOVER } else if h { HOVER.alpha(0.7) } else { FIELD });
        if on {
            ui.p().rounded_border(b, 8.0, 1.5, accent());
        }
        // the bus from the side: its body, windows, wheels, and the stripe on it
        let body = Rect::new(b.x + 6.0, b.y + 6.0, b.w - 12.0, 26.0);
        ui.p().rounded(body, 4.0, Color::rgba(70, 80, 100, 1.0));
        let (x0, x1, y0, y1) = (body.x, body.right(), body.y, body.bottom());
        let at = |f: f32| y1 - f * (y1 - y0);
        match t {
            StripeTemplate::Skirt => ui.p().rect(Rect::new(x0, at(0.3), x1 - x0, at(0.0) - at(0.3)), c),
            StripeTemplate::WindowBand => ui.p().rect(Rect::new(x0, at(0.5), x1 - x0, 3.0), c),
            StripeTemplate::RoofBand => ui.p().rect(Rect::new(x0, y0, x1 - x0, 5.0), c),
            StripeTemplate::Slanted => ui.p().convex(&[Vec2::new(x0, at(0.15)), Vec2::new(x1, at(0.4)), Vec2::new(x1, at(0.25)), Vec2::new(x0, at(0.0))], c),
            StripeTemplate::Wave => {
                let pts: Vec<Vec2> = (0..=16).map(|i| Vec2::new(x0 + (x1 - x0) * i as f32 / 16.0, at(0.25) + 3.0 * (i as f32 * 0.8).sin())).collect();
                ui.p().stroke(&pts, 4.0, c);
            }
            StripeTemplate::TwoTone => ui.p().rect(Rect::new(x0, at(0.5), x1 - x0, at(0.0) - at(0.5)), c),
            StripeTemplate::Front => ui.p().rect(Rect::new(x1 - 6.0, y0, 6.0, y1 - y0), c),
            StripeTemplate::Rear => ui.p().rect(Rect::new(x0, y0, 6.0, y1 - y0), c),
        }
        for k in 0..4 {
            ui.p().rect(Rect::new(x0 + 6.0 + k as f32 * (body.w - 12.0) / 4.0, at(0.82), (body.w - 12.0) / 4.0 - 3.0, 6.0), Color::rgba(150, 180, 210, 0.7));
        }
        ui.p().circle(Vec2::new(x0 + 9.0, y1), 3.5, Color::rgba(20, 20, 24, 1.0));
        ui.p().circle(Vec2::new(x1 - 10.0, y1), 3.5, Color::rgba(20, 20, 24, 1.0));
        ui.text_in(t.label(), Rect::new(b.x + 2.0, b.y + 38.0, b.w - 4.0, 16.0), 10.5, Weight::Medium, if on || h { TEXT } else { TEXT_SOFT }, Align::Center);
        if clicked {
            out = Some(*t);
        }
    }
    *y += 2.0 * (th + 8.0);
    out
}

/// The built-in shapes as tiles; returns the one clicked.
fn shape_grid(ui: &mut Ui, name: &str, r: Rect, chosen: Option<&str>, tex: &std::collections::HashMap<&'static str, usize>, y: &mut f32) -> Option<&'static str> {
    let cols = 5;
    let tw = (r.w - (cols - 1) as f32 * 6.0) / cols as f32;
    let mut out = None;
    for (k, (key, label)) in shapes::SHAPES.iter().enumerate() {
        let b = Rect::new(r.x + (k % cols) as f32 * (tw + 6.0), *y + (k / cols) as f32 * (tw + 6.0), tw, tw);
        let id = crate::launcher::ui::id_of(&format!("{name}-{key}"));
        let (h, _, clicked) = ui.interact(id, b);
        let on = chosen == Some(*key);
        ui.p().rounded(b, 8.0, if on { accent() } else if h { HOVER } else { FIELD });
        if let Some(t) = tex.get(key) {
            ui.image(b.inset(9.0), *t, 0.0);
        }
        ui.tooltip(b, label);
        if clicked {
            out = Some(*key);
        }
    }
    *y += (shapes::SHAPES.len().div_ceil(cols)) as f32 * (tw + 6.0);
    out
}

fn remember(recent: &mut Vec<[u8; 3]>, c: [u8; 3]) {
    recent.retain(|x| *x != c);
    recent.insert(0, c);
    recent.truncate(8);
}

/// The colour picker: a saturation and value square over the hue's strip, the hex code, the
/// swatches and the colours used last, and the eyedropper that takes a colour from the bus.
/// Returns whether the colour changed, and the height it took.
fn picker(ui: &mut Ui, name: &str, r: Rect, p: &mut Picker, c: &mut [u8; 3], recent: &[[u8; 3]]) -> (bool, f32) {
    let w = r.w;
    let (x, y0) = (r.x, r.y);
    let mut changed = false;
    // the state follows a colour changed elsewhere
    let shown = colour::bytes(colour::rgb_of_hsv(p.hsv));
    if shown != *c {
        let hsv = colour::hsv(colour::unit(*c));
        p.hsv = if hsv[1] < 1e-3 || hsv[2] < 1e-3 { [p.hsv[0], hsv[1], hsv[2]] } else { hsv };
        p.hex = model::hex(*c);
    }
    let sq = Rect::new(x, y0, w, 120.0);
    let hue = colour::bytes(colour::rgb_of_hsv([p.hsv[0], 1.0, 1.0]));
    ui.p().rounded(sq, 6.0, Color::rgba(hue[0], hue[1], hue[2], 1.0));
    ui.p().gradient_h(sq, Color::WHITE, Color::WHITE.alpha(0.0));
    ui.p().gradient(sq, Color::BLACK.alpha(0.0), Color::BLACK);
    let id = crate::launcher::ui::id_of(&format!("{name}-sv"));
    let (_, held, _) = ui.interact(id, sq);
    if held {
        let m = ui.input.mouse;
        p.hsv[1] = ((m.x - sq.x) / sq.w).clamp(0.0, 1.0);
        p.hsv[2] = 1.0 - ((m.y - sq.y) / sq.h).clamp(0.0, 1.0);
        changed = true;
    }
    let k = Vec2::new(sq.x + p.hsv[1] * sq.w, sq.y + (1.0 - p.hsv[2]) * sq.h);
    ui.p().circle(k, 7.0, Color::WHITE);
    ui.p().circle(k, 5.0, Color::rgba(c[0], c[1], c[2], 1.0));
    // the hues
    let bar = Rect::new(x, y0 + 128.0, w, 14.0);
    for s in 0..6 {
        let a = colour::bytes(colour::rgb_of_hsv([s as f32 * 60.0, 1.0, 1.0]));
        let b = colour::bytes(colour::rgb_of_hsv([(s + 1) as f32 * 60.0, 1.0, 1.0]));
        ui.p().gradient_h(Rect::new(bar.x + bar.w * s as f32 / 6.0, bar.y, bar.w / 6.0 + 0.5, bar.h), Color::rgba(a[0], a[1], a[2], 1.0), Color::rgba(b[0], b[1], b[2], 1.0));
    }
    let id = crate::launcher::ui::id_of(&format!("{name}-hue"));
    let (_, held, _) = ui.interact(id, bar.pad(0.0, -4.0));
    if held {
        p.hsv[0] = ((ui.input.mouse.x - bar.x) / bar.w).clamp(0.0, 0.9999) * 360.0;
        changed = true;
    }
    let hx = bar.x + p.hsv[0] / 360.0 * bar.w;
    ui.p().rounded_border(Rect::new(hx - 4.0, bar.y - 3.0, 8.0, bar.h + 6.0), 3.0, 2.0, Color::WHITE);
    if changed {
        *c = colour::bytes(colour::rgb_of_hsv(p.hsv));
        p.hex = model::hex(*c);
    }
    // the code, the colour itself and the eyedropper
    let row = y0 + 152.0;
    let mut hex = p.hex.clone();
    if ui.text_input(&format!("{name}-hex"), Rect::new(x, row, 110.0, 32.0), &mut hex, "#1d3f8f", None) {
        p.hex = hex.clone();
        if let Some(v) = model::parse_hex(&hex).filter(|_| hex.trim().trim_start_matches('#').len() == 6) {
            *c = v;
            p.hsv = colour::hsv(colour::unit(v));
            changed = true;
        }
    }
    ui.p().rounded(Rect::new(x + 118.0, row, 44.0, 32.0), 8.0, Color::rgba(c[0], c[1], c[2], 1.0));
    ui.p().rounded_border(Rect::new(x + 118.0, row, 44.0, 32.0), 8.0, 1.0, EDGE);
    let eye = Rect::new(x + w - 122.0, row, 122.0, 32.0);
    if ui.button(&format!("{name}-eye"), eye, "From the bus", Some("livery_eyedrop"), if p.eyedropper { ButtonKind::Primary } else { ButtonKind::Normal }) {
        p.eyedropper = !p.eyedropper;
    }
    // the swatches, and the colours used last
    let sw = (w - 7.0 * 4.0) / 8.0;
    let mut yy = row + 42.0;
    for (k, hexv) in PALETTE.iter().enumerate() {
        let b = Rect::new(x + (k % 8) as f32 * (sw + 4.0), yy + (k / 8) as f32 * (20.0 + 4.0), sw, 20.0);
        let v = [(hexv >> 16) as u8, (hexv >> 8) as u8, *hexv as u8];
        if swatch(ui, &format!("{name}-sw{k}"), b, v, v == *c) {
            *c = v;
            changed = true;
        }
    }
    yy += 2.0 * 24.0;
    if !recent.is_empty() {
        ui.text_in("Used last", Rect::new(x, yy, w, 16.0), 11.0, Weight::Medium, TEXT_DIM, Align::Left);
        yy += 18.0;
        for (k, v) in recent.iter().take(8).enumerate() {
            let b = Rect::new(x + k as f32 * (sw + 4.0), yy, sw, 20.0);
            if swatch(ui, &format!("{name}-rc{k}"), b, *v, *v == *c) {
                *c = *v;
                changed = true;
            }
        }
        yy += 24.0;
    }
    if changed {
        p.hsv = colour::hsv(colour::unit(*c));
        p.hex = model::hex(*c);
    }
    (changed, yy - y0 + 6.0)
}

fn swatch(ui: &mut Ui, name: &str, r: Rect, c: [u8; 3], on: bool) -> bool {
    let id = crate::launcher::ui::id_of(name);
    let (h, _, clicked) = ui.interact(id, r);
    ui.p().rounded(r, 5.0, Color::rgba(c[0], c[1], c[2], 1.0));
    ui.p().rounded_border(r, 5.0, if on { 2.0 } else { 1.0 }, if on { accent() } else if h { Color::WHITE.alpha(0.5) } else { EDGE });
    clicked
}

// --- pictures ----------------------------------------------------------------------------------

fn pick_picture_file() -> Option<std::path::PathBuf> {
    #[cfg(not(target_os = "android"))]
    {
        rfd::FileDialog::new().set_title(omsi_ui::tr("Choose a picture").as_ref()).add_filter(omsi_ui::tr("Pictures").as_ref(), &["png", "jpg", "jpeg", "bmp", "tga", "svg", "dds"]).pick_file()
    }
    #[cfg(target_os = "android")]
    {
        None
    }
}

fn choose_picture(l: &mut Launcher) {
    let Some(path) = pick_picture_file() else { return };
    if let Some(s) = l.livery.session.as_mut() {
        picture_chosen(s, &path);
    }
}

/// A picture chosen or dropped: the chosen picture layer takes it, or it waits to be placed.
pub fn picture_chosen(s: &mut Session, path: &std::path::Path) {
    match s.import_picture(path) {
        Ok((hash, pic)) => {
            let aspect = pic.h as f32 / pic.w.max(1) as f32;
            let white = shapes::wants_white_clear(&pic);
            // the chosen picture layer gets the new picture
            if let Some(i) = s.selected.as_ref().and_then(|id| s.project.index_of(id)) {
                let before = s.project.doc();
                if let Kind::Image { image, aspect: a, white_clear, .. } = &mut s.project.layers[i].kind {
                    *image = hash;
                    *a = Some(aspect);
                    *white_clear = white;
                    s.changed_from(before);
                    s.settle();
                    return;
                }
            }
            s.tool = Tool::Image;
            s.selected = None;
            s.ui.armed = Some(Armed::Picture { hash, aspect, white });
            s.say(omsi_ui::tr("Click on the bus where the picture goes."), false);
        }
        Err(e) => s.say(e, true),
    }
}

fn choose_logo(l: &mut Launcher) {
    let Some(path) = pick_picture_file() else { return };
    let Some(s) = l.livery.session.as_mut() else { return };
    match s.import_picture(&path) {
        Ok((hash, pic)) => {
            let before = s.project.doc();
            let mut q = s.project.quick.clone().unwrap_or_default();
            q.logo = Some(hash);
            q.logo_aspect = Some(pic.h as f32 / pic.w.max(1) as f32);
            s.project.quick = Some(q.clone());
            if let Some(d) = s.dims() {
                s.project.layers = model::apply_quick(&s.project.layers, &q, &d, &shapes::text_width);
            }
            s.changed_from(before);
            s.settle();
        }
        Err(e) => s.say(e, true),
    }
}

// --- the bus under the mouse ---------------------------------------------------------------------

fn view_bar(l: &mut Launcher, view: Rect) {
    let labels = ["Left", "Right", "Front", "Rear", "Roof", "Angled"];
    let bw = 70.0;
    let total = labels.len() as f32 * (bw + 4.0) + 150.0;
    let bar = Rect::new(view.center().x - total * 0.5, view.bottom() - 52.0, total, 40.0);
    l.ui.solid(bar);
    l.ui.p().rounded(bar, 20.0, ON_MAP);
    for (k, label) in labels.iter().enumerate() {
        let b = Rect::new(bar.x + 6.0 + k as f32 * (bw + 4.0), bar.y + 5.0, bw, 30.0);
        if l.ui.button(&format!("livery-view-{k}"), b, label, None, ButtonKind::Ghost) {
            set_view(l, k as u32 + 1);
        }
        l.ui.tooltip(b, &format!("{}", k + 1));
    }
    let ba = Rect::new(bar.right() - 140.0, bar.y + 5.0, 132.0, 30.0);
    let id = crate::launcher::ui::id_of("livery-before");
    let (h, held, _) = l.ui.interact(id, ba);
    if held || h {
        l.ui.p().rounded(ba, 15.0, if held { accent() } else { HOVER });
    }
    l.ui.p().rounded_border(ba, 15.0, 1.0, EDGE);
    l.ui.icon("visibility", Vec2::new(ba.x + 16.0, ba.center().y), 15.0, if held || h { TEXT } else { TEXT_DIM });
    l.ui.text_in("Before / after", Rect::new(ba.x + 28.0, ba.y, ba.w - 32.0, ba.h), 12.0, Weight::Medium, if held || h { TEXT } else { TEXT_DIM }, Align::Center);
    l.ui.tooltip(ba, "Hold to see the livery before (or hold O)");
    if let Some(s) = l.livery.session.as_mut() {
        s.ui.before_after = held;
    }
}

fn set_view(l: &mut Launcher, k: u32) {
    let Some(s) = l.livery.session.as_mut() else { return };
    let Some(d) = s.dims() else { return };
    let aspect = s.ui.view.map(|(r, _)| r.w / r.h.max(1.0)).unwrap_or(1.7);
    let (yaw, pitch) = view_angles(k);
    let target = (d.min + d.max) * 0.5;
    let dist = fit(&d, target, yaw, pitch, aspect);
    s.cam.send(target, yaw, pitch, dist);
}

fn fit_view(l: &mut Launcher) {
    let Some(s) = l.livery.session.as_mut() else { return };
    let Some(d) = s.dims() else { return };
    let aspect = s.ui.view.map(|(r, _)| r.w / r.h.max(1.0)).unwrap_or(1.7);
    let target = (d.min + d.max) * 0.5;
    let (yaw, pitch) = (s.cam.yaw, s.cam.pitch);
    s.cam.send(target, yaw, pitch, fit(&d, target, yaw, pitch, aspect));
}

/// The ray under the mouse (bus frame).
fn ray(view: Rect, cam: &Camera, m: Vec2) -> (Vec3, Vec3) {
    let ndc = Vec2::new((m.x - view.x) / view.w * 2.0 - 1.0, 1.0 - (m.y - view.y) / view.h * 2.0);
    cam.ray(ndc.x, ndc.y, view.w / view.h.max(1.0), DVec3::ZERO)
}

/// Where a point of the bus lands in the view.
fn screen(view: Rect, cam: &Camera, p: Vec3) -> Option<Vec2> {
    let m = cam.view_proj(view.w / view.h.max(1.0), DVec3::ZERO);
    let q = m * p.extend(1.0);
    (q.w > 1e-4).then(|| Vec2::new(view.x + (q.x / q.w + 1.0) * 0.5 * view.w, view.y + (1.0 - q.y / q.w) * 0.5 * view.h))
}

/// Where the ray meets the plane through `o` with normal `n`.
fn on_plane(o: Vec3, d: Vec3, at: Vec3, n: Vec3) -> Option<Vec3> {
    let den = d.dot(n);
    if den.abs() < 1e-5 {
        return None;
    }
    let t = (at - o).dot(n) / den;
    (t > 0.0).then(|| o + d * t)
}

/// The decal size of a layer.
fn size_of(s: &Session, l: &Layer) -> Option<(f32, f32)> {
    paint::decal_size(&l.kind, &s.pictures)
}

/// The decal (and whether its mirrored copy) under the point `p` with normal `n`, the top one first.
fn decal_at(s: &Session, p: Vec3, n: Vec3) -> Option<(String, bool)> {
    let plane = s.mirror_plane();
    for l in s.project.layers.iter().rev() {
        if !l.visible || l.locked {
            continue;
        }
        let (Some(place), Some((w, h))) = (l.kind.place(), size_of(s, l)) else { continue };
        let mut places = vec![(place.clone(), false)];
        if let Some(pl) = plane {
            if let Some((m, _)) = model::mirror_place(place, pl, matches!(l.kind, Kind::Text { .. })) {
                places.push((m, true));
            }
        }
        for (pl, copy) in places {
            let (_, _, sn) = pl.side.axes();
            let q = pl.local(p);
            if q.x.abs() <= w * 0.5 && q.y.abs() <= h * 0.5 && (p - pl.centre).dot(sn).abs() < 0.8 && n.dot(sn) > 0.3 {
                return Some((l.id.clone(), copy));
            }
        }
    }
    None
}

/// The texel under a hit: its target and its place in the editing size's texture.
fn texel(s: &Session, tri: usize, w: Vec3) -> Option<(usize, usize)> {
    let r = s.ready.as_ref()?;
    let t = &r.geom.tris[tri];
    let k = r.targets.iter().position(|x| Some(x.index) == t.target)?;
    let base = &r.bases[k];
    let c = (t.uv[0] + t.uv[1] + t.uv[2]) / 3.0;
    let uv = t.uv[0] * w.x + t.uv[1] * w.y + t.uv[2] * w.z - c.floor();
    let x = ((uv.x * base.width as f32).floor() as i64).rem_euclid(base.width as i64) as usize;
    let y = ((uv.y * base.height as f32).floor() as i64).rem_euclid(base.height as i64) as usize;
    Some((k, y * base.width as usize + x))
}

fn pointer(l: &mut Launcher, view: Rect) {
    let input = l.ui.input.clone();
    // (the other side's picture as well: it fades away for the bus under it)
    let over = !l.ui.over_ui && view.contains(input.mouse);
    let s = l.livery.session.as_mut().unwrap();
    let middle = l.livery.middle;
    let Some((_, cam)) = s.ui.view else { return };
    let last = s.ui.last_mouse;
    s.ui.last_mouse = input.mouse;
    let d = input.mouse - last;
    // the camera: right turns, the middle (or Shift with the left) moves, the wheel zooms
    if over && input.wheel.y.abs() > 0.0 {
        let k = (1.0 - input.wheel.y * 0.1).clamp(0.7, 1.4);
        let (t, y, p, dd) = s.cam.to;
        s.cam.to = (t, y, p, (dd * k).clamp(2.0, 120.0));
    }
    if over && (input.right_pressed || (middle && s.ui.drag.is_none())) {
        s.ui.drag = Some(if input.right_pressed { Drag::Orbit } else { Drag::Pan });
    }
    match s.ui.drag {
        Some(Drag::Orbit) => {
            if !input.right_down {
                s.ui.drag = None;
            } else {
                let (t, y, p, dd) = s.cam.to;
                s.cam.to = (t, y + d.x * 0.35, (p + d.y * 0.25).clamp(-10.0, 89.0), dd);
                s.cam.yaw = s.cam.to.1;
                s.cam.pitch = s.cam.to.2;
            }
            return;
        }
        Some(Drag::Pan) => {
            if !middle && !input.down {
                s.ui.drag = None;
            } else {
                let (f, r) = (cam.forward(), cam.right());
                let up = r.cross(f).normalize_or(Vec3::Z);
                let k = s.cam.dist * (FOV.to_radians() * 0.5).tan() * 2.0 / view.h.max(1.0);
                let mv = -r * d.x * k + up * d.y * k;
                s.cam.to.0 += mv;
                s.cam.target = s.cam.to.0;
            }
            return;
        }
        _ => {}
    }
    if s.ready.is_none() {
        return;
    }
    let (o, dir) = ray(view, &cam, input.mouse);
    let hit = s.ready.as_ref().and_then(|r| r.geom.hit(o, dir));
    // a drag going on
    if let Some(drag) = s.ui.drag.clone() {
        if !input.down {
            s.ui.drag = None;
            return;
        }
        drag_on(s, drag, o, dir, hit, input.shift);
        return;
    }
    if !over {
        return;
    }
    if s.ui.armed.is_some() || matches!(s.tool, Tool::Text | Tool::Fill | Tool::Pen | Tool::Brush) || s.picker.eyedropper {
        l.ui.cursor = winit::window::CursorIcon::Crosshair;
    }
    // the brush's size on the bus under the mouse
    if s.tool == Tool::Brush {
        if let Some((_, p, n, _)) = hit {
            let side = n.cross(Vec3::Z).normalize_or(Vec3::X);
            if let (Some(a), Some(b)) = (screen(view, &cam, p), screen(view, &cam, p + side * s.ui.brush_cm / 100.0)) {
                let r = a.distance(b).max(2.0);
                let c = if s.ui.eraser { DANGER.lighten(0.3) } else { TEXT };
                l.ui.p().arc(a, r - 1.0, r + 1.0, 0.0, std::f32::consts::TAU, c.alpha(0.9));
                l.ui.p().arc(a, r + 1.0, r + 2.0, 0.0, std::f32::consts::TAU, Color::rgba(0, 0, 0, 0.5));
            }
        }
    }
    if !input.pressed {
        return;
    }
    // Shift with the left: move the view
    if input.shift && !matches!(s.tool, Tool::Pen) {
        s.ui.drag = Some(Drag::Pan);
        return;
    }
    // the eyedropper
    if s.picker.eyedropper {
        if let Some((tri, _, _, w)) = hit {
            if let (Some((k, i)), Some(r)) = (texel(s, tri, w), s.ready.as_ref()) {
                if let Some(px) = r.current.get(k).and_then(|p| p.get(i * 4..i * 4 + 3)) {
                    let c = [px[0], px[1], px[2]];
                    s.picker.eyedropper = false;
                    set_picked(s, c);
                }
            }
        }
        return;
    }
    // the handles of what is chosen
    if let Some(drag) = handle_at(s, view, &cam, input.mouse) {
        s.ui.drag = Some(drag);
        return;
    }
    let before = s.project.doc();
    let colour = model::hex(s.picker.colour);
    match (s.tool, hit) {
        (Tool::Pen, _) => pen_click(s, o, dir, hit.map(|h| (h.1, h.2)), view, &cam, input.mouse),
        (Tool::Brush, Some((_, p, n, _))) => brush_down(s, p, n),
        (_, None) => {
            if s.tool == Tool::Select {
                s.selected = None;
            }
        }
        (_, Some((tri, p, n, w))) => {
            // (a decal put onto a window is a print on the glass)
            let on_glass = s.ready.as_ref().is_some_and(|r| r.geom.tris.get(tri).is_some_and(|t| t.glass));
            if let Some(armed) = s.ui.armed.take() {
                let side = Side::of_normal(n);
                let place = Place::new(side, p, 0.6);
                let ly = match armed {
                    Armed::Picture { hash, aspect, white } => model::placed_layer("Image", Kind::Image { image: hash, white_clear: white, aspect: Some(aspect), place: Place { width_m: if aspect > 1.5 { 0.4 } else { 0.8 }, ..place } }, on_glass),
                    Armed::Shape(k) => {
                        let label = shapes::SHAPES.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or("Shape");
                        model::placed_layer(label, Kind::Shape { shape: k.to_string(), colour: colour.clone(), outline: None, gradient: None, place }, on_glass)
                    }
                };
                s.selected = Some(ly.id.clone());
                s.project.layers.push(ly);
                s.tool = Tool::Select;
            } else {
                match s.tool {
                    Tool::Text => {
                        let text = if s.ui.text.trim().is_empty() { omsi_ui::tr("Text").into_owned() } else { s.ui.text.clone() };
                        let font = shapes::font_names().get(s.ui.font).cloned().unwrap_or_else(|| shapes::DEFAULT_FONT.into());
                        let height_cm = if s.ui.size_cm > 0.0 { s.ui.size_cm } else { 25.0 };
                        let width = shapes::text_width(&text, &font) * height_cm / 100.0;
                        let ly = model::placed_layer("Text", Kind::Text { text, font, height_cm, colour: colour.clone(), outline: None, spacing: 0.0, gradient: None, place: Place::new(Side::of_normal(n), p, width) }, on_glass);
                        s.selected = Some(ly.id.clone());
                        s.project.layers.push(ly);
                        s.tool = Tool::Select;
                        remember(&mut s.recent, s.picker.colour);
                    }
                    Tool::Fill => {
                        if let Some((k, i)) = texel(s, tri, w) {
                            let r = s.ready.as_ref().unwrap();
                            let b = &r.bases[k].rgba[i * 4..i * 4 + 3];
                            let lab = colour::lab([b[0], b[1], b[2]]);
                            let radius = r.zones.get(k).map(|z| z.fill_radius(lab)).unwrap_or(1000.0);
                            let ly = model::layer("Fill", Kind::Fill { centre: lab, radius, group: None, colour: colour.clone(), gradient: None });
                            s.selected = Some(ly.id.clone());
                            s.project.layers.push(ly);
                            remember(&mut s.recent, s.picker.colour);
                        }
                    }
                    _ => {
                        // choose what lies there: a decal, else a stripe at that height
                        if let Some((id, copy)) = decal_at(s, p, n) {
                            let place = s.project.layer(&id).and_then(|l| l.kind.place().cloned()).unwrap();
                            let grab = if copy { Vec3::new(2.0 * s.mirror_plane().unwrap_or(0.0) - p.x, p.y, p.z) } else { p };
                            s.ui.drag = Some(Drag::Move { id: id.clone(), offset: grab - place.centre, copy });
                            s.selected = Some(id);
                            s.tool = Tool::Select;
                        } else {
                            let d = s.dims().unwrap();
                            let found = s.project.layers.iter().rev().find(|l| {
                                l.visible && match &l.kind {
                                    Kind::Stripe { h1, h2, angle, wave, sides, .. } => {
                                        let (a, b) = model::stripe_edges(*h1, *h2, *angle, *wave, p.y, &d);
                                        let z = p.z - d.min.z;
                                        z >= a && z <= b && model::stripe_side_weight(*sides, n) > 0.5
                                    }
                                    _ => false,
                                }
                            });
                            s.selected = found.map(|l| l.id.clone());
                        }
                    }
                }
            }
        }
    }
    if s.project.doc() != before {
        s.changed_from(before);
    }
}

/// A handle of the chosen layer under the mouse: a decal's corners (size), its turn, a stripe's
/// edges.
fn handle_at(s: &Session, view: Rect, cam: &Camera, m: Vec2) -> Option<Drag> {
    let id = s.selected.clone()?;
    let l = s.project.layer(&id)?;
    if l.locked {
        return None;
    }
    let d = s.dims()?;
    match &l.kind {
        Kind::Stripe { h1, h2, angle, wave, .. } => {
            for (upper, h) in [(false, *h1), (true, *h2)] {
                let line = stripe_line(&d, s.ready.as_ref().map(|r| &*r.geom), cam, h, *angle, *wave);
                if line.windows(2).any(|w| match (screen(view, cam, w[0]), screen(view, cam, w[1])) {
                    (Some(a), Some(b)) => dist_to_segment(m, a, b) < 7.0,
                    _ => false,
                }) {
                    return Some(Drag::Edge { id, upper });
                }
            }
            None
        }
        kind => {
            let place = kind.place()?;
            let (w, h) = size_of(s, l)?;
            let corners = place.corners(w, h);
            // (a text keeps its proportions: only its corners)
            let text = matches!(kind, Kind::Text { .. });
            for (grip, c) in Grip::handles(&corners) {
                if (text && grip != Grip::Corner) || !screen(view, cam, c).is_some_and(|q| q.distance(m) < 10.0) {
                    continue;
                }
                let from = place.local(c);
                let height_cm = match kind {
                    Kind::Text { height_cm, .. } => *height_cm,
                    _ => 0.0,
                };
                return Some(Drag::Scale { id, grip, start: place.clone(), from, size: (w, h), height_cm });
            }
            let rot = rotate_handle(place, w, h);
            if screen(view, cam, rot).is_some_and(|q| q.distance(m) < 10.0) {
                let p0 = Place { rotation: 0.0, ..place.clone() };
                let q = p0.local(rot);
                return Some(Drag::Rotate { id, start: place.rotation, from: q.y.atan2(q.x).to_degrees() });
            }
            None
        }
    }
}

fn rotate_handle(place: &Place, w: f32, h: f32) -> Vec3 {
    let c = place.corners(w, h);
    let top = (c[2] + c[3]) * 0.5;
    let up = (c[3] - c[0]).normalize_or(Vec3::Z);
    top + up * (0.12 + h * 0.15)
}

fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// A stripe's edge along the side of the bus the camera sees: at its height as the paint has it
/// (`stripe_edges`, the same as the painter's), on the body itself where it is there - on the
/// box's face, which the mirrors widen and the body curves in from, the line looked off the paint.
fn stripe_line(d: &model::BusDims, geom: Option<&BusGeom>, cam: &Camera, h: f32, angle: f32, wave: f32) -> Vec<Vec3> {
    let right = cam.position.x >= d.middle_x() as f64;
    let (x, out) = if right { (d.max.x, 1.0) } else { (d.min.x, -1.0) };
    let n = 64;
    (0..=n)
        .map(|k| {
            let y = d.min.y + d.length() * k as f32 / n as f32;
            let (e, _) = model::stripe_edges(h, h, angle, wave, y, d);
            let z = d.min.z + e;
            let from = Vec3::new(x + out, y, z);
            let on = geom.and_then(|g| g.ray(from, Vec3::new(-out, 0.0, 0.0))).map(|(_, t, _)| from.x - out * t).filter(|sx| (sx - x).abs() < 0.6);
            Vec3::new(on.unwrap_or(x), y, z)
        })
        .collect()
}

fn drag_on(s: &mut Session, drag: Drag, o: Vec3, dir: Vec3, hit: Option<(usize, Vec3, Vec3, Vec3)>, shift: bool) {
    let before = s.project.doc();
    let plane = s.mirror_plane();
    let dims = s.dims();
    match drag {
        Drag::Move { id, offset, copy } => {
            let Some((_, p, n, _)) = hit else { return };
            let p = if copy { Vec3::new(2.0 * plane.unwrap_or(0.0) - p.x, p.y, p.z) } else { p };
            let n = if copy { Vec3::new(-n.x, n.y, n.z) } else { n };
            if let Some(place) = s.project.layer_mut(&id).filter(|l| !l.locked).and_then(|l| l.kind.place_mut()) {
                place.centre = p - offset;
                let side = Side::of_normal(n);
                if side != place.side && n.dot(side.axes().2) > 0.85 {
                    place.side = side;
                }
            }
        }
        Drag::Scale { id, grip, start, from, size, height_cm } => {
            let (_, _, sn) = start.side.axes();
            let Some(q) = on_plane(o, dir, start.centre, sn) else { return };
            let now = start.local(q);
            let Some(l) = s.project.layer_mut(&id) else { return };
            match &mut l.kind {
                Kind::Text { height_cm: hc, place, .. } => {
                    let k = (now.length() / from.length().max(1e-3)).clamp(0.05, 20.0);
                    *hc = (height_cm * k).clamp(2.0, 150.0);
                    place.width_m = start.width_m * k;
                }
                // (a corner with Shift: freely; the copy on the other side has the same place)
                kind => {
                    if let Some(place) = kind.place_mut() {
                        place.pulled(&start, grip, size, from, now, shift);
                    }
                }
            }
        }
        Drag::Rotate { id, start, from } => {
            let Some(l) = s.project.layer_mut(&id) else { return };
            let Some(place) = l.kind.place_mut() else { return };
            let (_, _, sn) = place.side.axes();
            let Some(q) = on_plane(o, dir, place.centre, sn) else { return };
            let p0 = Place { rotation: 0.0, ..place.clone() };
            let v = p0.local(q);
            let mut a = start + v.y.atan2(v.x).to_degrees() - from;
            while a > 180.0 {
                a -= 360.0;
            }
            while a < -180.0 {
                a += 360.0;
            }
            // (steps of 15° with Shift)
            place.rotation = if shift { (a / 15.0).round() * 15.0 } else { a };
        }
        Drag::Edge { id, upper } => {
            let Some(d) = dims else { return };
            let Some(l) = s.project.layer_mut(&id) else { return };
            if let Kind::Stripe { h1, h2, angle, wave, .. } = &mut l.kind {
                // the side plane the camera looks at
                let (plane_at, n) = if o.x >= d.middle_x() { (Vec3::new(d.max.x, 0.0, 0.0), Vec3::X) } else { (Vec3::new(d.min.x, 0.0, 0.0), Vec3::NEG_X) };
                let Some(q) = hit.map(|h| h.1).or_else(|| on_plane(o, dir, plane_at, n)) else { return };
                let (e, _) = model::stripe_edges(0.0, 0.0, *angle, *wave, q.y, &d);
                let h = model::snap_to_window(q.z - d.min.z - e, &d);
                if upper { *h2 = h.max(*h1 + 0.02) } else { *h1 = h.min(*h2 - 0.02) }
            }
        }
        Drag::Curve => {
            let Some(side) = s.ui.pen.side else { return };
            let (_, _, sn) = side.axes();
            let Some(q) = on_plane(o, dir, s.ui.pen.origin, sn) else { return };
            let local = Place::new(side, s.ui.pen.origin, 1.0).local(q);
            if let Some(last) = s.ui.pen.nodes.last_mut() {
                last.1 = Some(local);
            }
        }
        Drag::Brush { id } => {
            let Some((_, p, n, _)) = hit else { return };
            let Some(Layer { kind: Kind::Brush { strokes, .. }, .. }) = s.project.layer_mut(&id) else { return };
            let Some(st) = strokes.last_mut() else { return };
            // (a point a third of the radius on from the last)
            let last = st.points.last().map(|q| Vec3::from(*q)).unwrap_or(p);
            if p.distance(last) >= st.radius_cm / 100.0 * 0.3 {
                st.points.push(p.into());
                st.normals.push(n.into());
            }
        }
        Drag::Orbit | Drag::Pan => {}
    }
    if s.project.doc() != before {
        s.changed_from(before);
    }
}

/// The brush (or the eraser) put down at `p` on the bus: a new stroke into the chosen brush
/// layer, the one painted last, or (the brush, in another colour or with none yet) a new one.
fn brush_down(s: &mut Session, p: Vec3, n: Vec3) {
    let colour = model::hex(s.picker.colour);
    let is_brush = |s: &Session, id: &str| s.project.layer(id).is_some_and(|l| matches!(l.kind, Kind::Brush { .. }) && !l.locked);
    let mut target = s.selected.clone().filter(|id| is_brush(s, id)).or_else(|| s.ui.brush_layer.clone().filter(|id| is_brush(s, id)));
    if s.ui.eraser {
        target = target.or_else(|| s.project.layers.iter().rev().find(|l| matches!(l.kind, Kind::Brush { .. }) && !l.locked).map(|l| l.id.clone()));
        if target.is_none() {
            s.say(omsi_ui::tr("Nothing painted with the brush yet to erase."), false);
            return;
        }
    } else if s.selected.as_ref() != target.as_ref() && target.as_ref().is_some_and(|id| s.project.layer(id).and_then(|l| l.kind.colour()).is_some_and(|c| !c.eq_ignore_ascii_case(&colour))) {
        target = None;
    }
    let id = match target {
        Some(id) => id,
        None => {
            let ly = model::layer("Brush", Kind::Brush { colour: colour.clone(), strokes: Vec::new() });
            let id = ly.id.clone();
            s.project.layers.push(ly);
            remember(&mut s.recent, s.picker.colour);
            id
        }
    };
    let stroke = model::Stroke { points: vec![p.into()], normals: vec![n.into()], radius_cm: s.ui.brush_cm, hardness: s.ui.hardness, opacity: s.ui.brush_opacity, erase: s.ui.eraser };
    if let Some(Layer { kind: Kind::Brush { strokes, .. }, .. }) = s.project.layer_mut(&id) {
        strokes.push(stroke);
    }
    s.ui.brush_layer = Some(id.clone());
    s.selected = Some(id.clone());
    s.ui.drag = Some(Drag::Brush { id });
}

fn set_picked(s: &mut Session, c: [u8; 3]) {
    let before = s.project.doc();
    s.picker.colour = c;
    remember(&mut s.recent, c);
    if let Some(i) = s.selected.as_ref().and_then(|id| s.project.index_of(id)) {
        let l = &mut s.project.layers[i];
        if l.locked {
            return;
        }
        match s.picker.target {
            PickFor::Outline => {
                if let Some(Some(o)) = l.kind.outline_mut() {
                    o.colour = model::hex(c);
                }
            }
            PickFor::GradientEnd => {
                if let Some(Some(g)) = l.kind.gradient_mut() {
                    g.colour2 = model::hex(c);
                }
            }
            _ => l.kind.set_colour(model::hex(c)),
        }
    } else if let PickFor::Quick(k) = s.picker.target {
        let mut q = s.project.quick.clone().unwrap_or_default();
        q.colours[k] = Some(model::hex(c));
        s.project.quick = Some(q.clone());
        if let Some(d) = s.dims() {
            s.project.layers = model::apply_quick(&s.project.layers, &q, &d, &shapes::text_width);
        }
    }
    s.changed_from(before);
    s.settle();
}

fn pen_click(s: &mut Session, o: Vec3, dir: Vec3, hit: Option<(Vec3, Vec3)>, view: Rect, cam: &Camera, m: Vec2) {
    if s.ui.pen.side.is_none() {
        let Some((p, n)) = hit else { return };
        s.ui.pen = Pen { side: Some(Side::of_normal(n)), origin: p, nodes: vec![(Vec2::ZERO, None)] };
        s.ui.drag = Some(Drag::Curve);
        return;
    }
    let side = s.ui.pen.side.unwrap();
    let (_, _, sn) = side.axes();
    let Some(q) = on_plane(o, dir, s.ui.pen.origin, sn) else { return };
    let frame = Place::new(side, s.ui.pen.origin, 1.0);
    // back on the first corner: done
    if s.ui.pen.nodes.len() >= 3 && screen(view, cam, s.ui.pen.origin).is_some_and(|f| f.distance(m) < 10.0) {
        finish_pen(s);
        return;
    }
    s.ui.pen.nodes.push((frame.local(q), None));
    s.ui.drag = Some(Drag::Curve);
}

/// The pen's shape as a layer of its own.
fn finish_pen(s: &mut Session) {
    let pen = std::mem::take(&mut s.ui.pen);
    let Some(side) = pen.side else { return };
    // the handle pulled out of a corner, and its twin into it
    let pts: Vec<(Vec2, Option<Vec2>, Option<Vec2>)> = pen.nodes.iter().map(|(p, h)| (*p, h.map(|h| *p * 2.0 - h), *h)).collect();
    let Some((mid, size, nodes)) = shapes::fit_nodes(&pts) else { return };
    let (u, v, _) = side.axes();
    let mut place = Place::new(side, pen.origin + u * mid.x + v * mid.y, size.x);
    place.height_m = Some(size.y);
    let before = s.project.doc();
    let ly = model::layer("Own shape", Kind::Path { nodes, colour: model::hex(s.picker.colour), outline: None, gradient: None, place });
    s.selected = Some(ly.id.clone());
    s.project.layers.push(ly);
    s.tool = Tool::Select;
    s.changed_from(before);
    s.settle();
}

/// What is drawn over the bus: the chosen layer's outline and handles, the pen's corners, the
/// window line while a stripe's edge is dragged.
fn overlay(l: &mut Launcher, view: Rect) {
    let s = l.livery.session.as_ref().unwrap();
    let Some((_, cam)) = s.ui.view else { return };
    let Some(d) = s.dims() else { return };
    l.ui.push_clip(view, RADIUS);
    let pts = |ps: &[Vec3]| -> Vec<Vec2> { ps.iter().filter_map(|p| screen(view, &cam, *p)).collect() };
    if let Some(layer) = s.selected.as_ref().and_then(|id| s.project.layer(id)) {
        match &layer.kind {
            Kind::Stripe { h1, h2, angle, wave, .. } => {
                for h in [*h1, *h2] {
                    let line = pts(&stripe_line(&d, s.ready.as_ref().map(|r| &*r.geom), &cam, h, *angle, *wave));
                    l.ui.p().stroke(&line, 2.0, accent());
                    if let Some(mid) = line.get(line.len() / 2) {
                        l.ui.p().circle(*mid, 6.0, Color::WHITE);
                        l.ui.p().circle(*mid, 4.0, accent());
                    }
                }
                if matches!(s.ui.drag, Some(Drag::Edge { .. })) {
                    let wl = pts(&stripe_line(&d, s.ready.as_ref().map(|r| &*r.geom), &cam, d.window_height(), 0.0, 0.0));
                    l.ui.p().stroke(&wl, 1.0, LINE.alpha(0.8));
                }
            }
            kind => {
                if let (Some(place), Some((w, h))) = (kind.place(), size_of(s, layer)) {
                    let c = place.corners(w, h);
                    let mut ring = pts(&c);
                    if ring.len() == 4 {
                        ring.push(ring[0]);
                        l.ui.p().stroke(&ring, 1.5, accent());
                        for q in &ring[..4] {
                            l.ui.p().circle(*q, 6.0, Color::WHITE);
                            l.ui.p().circle(*q, 4.0, accent());
                        }
                        // the handles in the middle of the sides (stretching), not on a text
                        if !matches!(kind, Kind::Text { .. }) {
                            for (_, p) in Grip::handles(&c).into_iter().skip(4) {
                                if let Some(q) = screen(view, &cam, p) {
                                    l.ui.p().rounded(Rect::new(q.x - 4.5, q.y - 4.5, 9.0, 9.0), 2.0, Color::WHITE);
                                    l.ui.p().rounded(Rect::new(q.x - 3.0, q.y - 3.0, 6.0, 6.0), 1.5, accent());
                                }
                            }
                        }
                        let top = screen(view, &cam, (c[2] + c[3]) * 0.5);
                        if let (Some(top), Some(r)) = (top, screen(view, &cam, rotate_handle(place, w, h))) {
                            l.ui.p().line(top, r, 1.5, accent());
                            l.ui.p().circle(r, 6.0, accent());
                            l.ui.p().circle(r, 3.0, Color::WHITE);
                        }
                    }
                    // its copy on the other side
                    if let Some(pl) = s.mirror_plane() {
                        if let Some((m, _)) = model::mirror_place(place, pl, matches!(kind, Kind::Text { .. })) {
                            let mut r2 = pts(&m.corners(w, h));
                            if r2.len() == 4 {
                                r2.push(r2[0]);
                                l.ui.p().stroke(&r2, 1.0, accent().alpha(0.45));
                            }
                        }
                    }
                }
            }
        }
    }
    // the pen's corners and lines
    if let Some(side) = s.ui.pen.side {
        let frame = Place::new(side, s.ui.pen.origin, 1.0);
        let (u, v, _) = side.axes();
        let at = |q: Vec2| frame.centre + u * q.x + v * q.y;
        let nodes = &s.ui.pen.nodes;
        let mut path: Vec<Vec3> = Vec::new();
        for k in 0..nodes.len() {
            let (p, hout) = nodes[k];
            if k + 1 < nodes.len() {
                let (q, qh) = nodes[k + 1];
                let c1 = hout.unwrap_or(p);
                let c2 = qh.map(|h| q * 2.0 - h).unwrap_or(q);
                for i in 0..=12 {
                    let t = i as f32 / 12.0;
                    let a = p.lerp(c1, t).lerp(c1.lerp(c2, t), t);
                    let b = c1.lerp(c2, t).lerp(c2.lerp(q, t), t);
                    path.push(at(a.lerp(b, t)));
                }
            }
        }
        let line = pts(&path);
        l.ui.p().stroke(&line, 2.0, accent());
        for (k, (p, h)) in nodes.iter().enumerate() {
            if let Some(q) = screen(view, &cam, at(*p)) {
                l.ui.p().circle(q, if k == 0 { 7.0 } else { 5.0 }, Color::WHITE);
                l.ui.p().circle(q, if k == 0 { 5.0 } else { 3.5 }, accent());
                if let (Some(h), Some(hq)) = (h, h.and_then(|h| screen(view, &cam, at(h)))) {
                    let _ = h;
                    l.ui.p().line(q, hq, 1.0, TEXT_SOFT);
                    l.ui.p().circle(hq, 3.0, TEXT_SOFT);
                }
            }
        }
    }
    l.ui.pop_clip();
}

fn keys(l: &mut Launcher) {
    if l.ui.focus.is_some() {
        return;
    }
    let input = l.ui.input.clone();
    let Some(code) = input.raw_key else {
        for k in &input.keys {
            key_named(l, *k);
        }
        return;
    };
    let ctrl = input.ctrl;
    match code {
        KeyCode::KeyZ if ctrl && input.shift => l.livery.session.as_mut().unwrap().redo(),
        KeyCode::KeyZ if ctrl => l.livery.session.as_mut().unwrap().undo(),
        KeyCode::KeyY if ctrl => l.livery.session.as_mut().unwrap().redo(),
        KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 if !ctrl => {
            let k = match code {
                KeyCode::Digit1 => 1,
                KeyCode::Digit2 => 2,
                KeyCode::Digit3 => 3,
                KeyCode::Digit4 => 4,
                KeyCode::Digit5 => 5,
                _ => 6,
            };
            set_view(l, k);
        }
        KeyCode::KeyF if !ctrl => fit_view(l),
        KeyCode::KeyE if !ctrl => {
            let s = l.livery.session.as_mut().unwrap();
            if s.tool == Tool::Brush {
                s.ui.eraser = !s.ui.eraser;
            } else {
                s.tool = Tool::Brush;
                s.ui.eraser = true;
                s.ui.armed = None;
            }
        }
        KeyCode::BracketLeft | KeyCode::BracketRight if !ctrl => {
            let s = l.livery.session.as_mut().unwrap();
            let k = if code == KeyCode::BracketRight { 1.25 } else { 0.8 };
            s.ui.brush_cm = (s.ui.brush_cm * k).clamp(0.5, 100.0);
        }
        _ => {
            for k in &input.keys {
                key_named(l, *k);
            }
        }
    }
}

fn key_named(l: &mut Launcher, k: Key) {
    let s = l.livery.session.as_mut().unwrap();
    match k {
        Key::Escape => {
            if s.ui.pen.side.is_some() {
                s.ui.pen = Pen::default();
            } else if s.ui.armed.is_some() || s.picker.eyedropper {
                s.ui.armed = None;
                s.picker.eyedropper = false;
            } else {
                s.selected = None;
            }
        }
        Key::Enter if s.ui.pen.nodes.len() >= 3 => finish_pen(s),
        Key::Delete => {
            if let Some(i) = s.selected.as_ref().and_then(|id| s.project.index_of(id)) {
                if !s.project.layers[i].locked {
                    let before = s.project.doc();
                    s.project.layers.remove(i);
                    s.selected = None;
                    s.changed_from(before);
                    s.settle();
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_views_frame_the_bus_and_the_mouse_finds_it_again() {
        let d = model::BusDims { min: Vec3::new(-1.25, -6.0, 0.3), max: Vec3::new(1.25, 6.0, 3.3), window: None };
        for k in 1..=6 {
            let view = Rect::new(0.0, 0.0, 1000.0, 600.0);
            let c = Cam::fitting(&d, k, view.w / view.h).camera();
            // the box's middle lands in the view's middle and a ray through it goes back there
            let m = screen(view, &c, (d.min + d.max) * 0.5).unwrap();
            assert!((m - view.center()).length() < 1.0, "{k}: {m}");
            let (o, dir) = ray(view, &c, m);
            let back = o + dir * (((d.min + d.max) * 0.5 - o).length());
            assert!((back - (d.min + d.max) * 0.5).length() < 0.05, "{k}: {back}");
        }
        // the left view looks at the left side
        let c = Cam::fitting(&d, 1, 1.7).camera();
        assert!(c.position.x < -5.0);
        assert!(c.forward().x > 0.99);
    }
}
