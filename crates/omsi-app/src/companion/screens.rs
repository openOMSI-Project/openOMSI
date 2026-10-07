//! The bus's screens on a phone or tablet.
//!
//! Omsi-Hub had to build the screens of an IBIS, an ALMEX or a ticket machine again from
//! OMSI 2's files, since OMSI kept its pictures to itself. openOMSI draws every screen itself,
//! and on the CPU: a `[scripttexture]` (what the scripts draw with `ST*`), an `[htmltexture]`
//! page (drawn into its script texture) and a `[texttexture]` (a string variable in a font)
//! are pictures in memory before they go to the graphics card. So a screen is sent as it is
//! - read when a device watches it, at most [`FPS`] times a second and only when it changed,
//! cut to the part of the texture the screen's mesh shows.
//!
//! What a screen is: a texture of the bus that a material of a mesh seen from inside the bus
//! shows (`[useScriptTexture]`, `[useTextTexture]`; not a transparency map, that is a
//! destination matrix's LEDs). Its name is the mesh's file (`AFR4_Display.o3d` is "AFR4
//! Display").
//!
//! A tap on the screen goes where a click in the cab goes: on a page straight to the page
//! (`html_pointer`, the same as the mouse), else as a click along the screen's normal at the
//! place of the tap - whatever switch lies there in the cab (a touch field in front of a
//! display) is worked by the cab's own click handling (`Player::click`, then `release`).
//! The switches beside a screen (the keys of an IBIS around its display) are found by where
//! they are: `[mouseevent]` meshes near the screen's plane, those that lie round it as one
//! device ([`device_keys`]). The device - the screen, those keys and the case they sit in -
//! is scanned by the game straight on, level and edge to edge ([`device_face`],
//! [`super::device`]), and a tap on that picture works whatever it shows there.
//!
//! A device built of pages instead (the Hamburg buses' ALMEX: menu pages switched by a
//! variable, text textures and touch fields on them) is found by where its meshes lie
//! ([`super::panels`]); it has no texture of its own to send ([`Source::Panel`]), only the
//! game's picture of its display seen straight on, and the text textures on it are part of
//! that picture rather than screens of their own.

use glam::{Vec2, Vec3};

/// Most pictures a second of a watched screen.
pub(crate) const FPS: f64 = 10.0;
/// A switch beside a screen lies this close to its plane (m)...
const KEY_OFF_PLANE: f32 = 0.06;
/// (further off it, the further it lies from the screen: a keypad leaning back by up to some
/// 20 degrees from its display)
const KEY_TILT: f32 = 0.4;
/// ...and is no bigger than this (m, either side): a whole panel is no key.
pub(crate) const KEY_MAX: f32 = 0.12;
/// Most keys of one screen's device (the nearest): a ticket table has 45.
const MAX_KEYS: usize = 64;
/// A switch counts as big (a draggable panel, a door leaf) beyond this radius (m): no tap or
/// key works it (the cockpit's own hover leaves out the same, see `Player::hovered_part`).
pub(crate) const BIG: f32 = 0.45;

/// A triangle of a screen's surface in the bus's frame (metres from its origin): corners,
/// their normals and texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tri {
    pub p: [Vec3; 3],
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
}

/// The plane a screen lies in, with its texture laid over it: where texture coordinate
/// (0, 0) lies and how far one unit of `u` and of `v` reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plane {
    pub origin: Vec3,
    pub du: Vec3,
    pub dv: Vec3,
    /// Out of the screen, towards whoever looks at it (as its vertex normals say).
    pub normal: Vec3,
}

fn uv_area(t: &Tri) -> f32 {
    let (a, b) = (t.uv[1] - t.uv[0], t.uv[2] - t.uv[0]);
    (a.x * b.y - a.y * b.x).abs() * 0.5
}

/// The plane of a screen, from its triangle that covers most of the texture.
pub(crate) fn plane_of(tris: &[Tri]) -> Option<Plane> {
    let t = tris.iter().filter(|t| uv_area(t) > 1e-9).max_by(|a, b| uv_area(a).total_cmp(&uv_area(b)))?;
    let (e1, e2) = (t.p[1] - t.p[0], t.p[2] - t.p[0]);
    let (a, b) = (t.uv[1] - t.uv[0], t.uv[2] - t.uv[0]);
    let det = a.x * b.y - a.y * b.x;
    if det.abs() < 1e-12 {
        return None;
    }
    let du = (e1 * b.y - e2 * a.y) / det;
    let dv = (e2 * a.x - e1 * b.x) / det;
    let origin = t.p[0] - du * t.uv[0].x - dv * t.uv[0].y;
    let mut normal = du.cross(dv).normalize_or_zero();
    if normal == Vec3::ZERO {
        return None;
    }
    let along: Vec3 = tris.iter().flat_map(|t| t.n).sum();
    if along.dot(normal) < 0.0 {
        normal = -normal;
    }
    Some(Plane { origin, du, dv, normal })
}

impl Plane {
    pub(crate) fn at(&self, u: f32, v: f32) -> Vec3 {
        self.origin + self.du * u + self.dv * v
    }

    /// Where `p` lies over the texture (`u`, `v`) and how far before the plane (m).
    pub(crate) fn project(&self, p: Vec3) -> Option<(f32, f32, f32)> {
        let m = glam::Mat3::from_cols(self.du, self.dv, self.normal);
        if m.determinant().abs() < 1e-12 {
            return None;
        }
        let x = m.inverse() * (p - self.origin);
        Some((x.x, x.y, x.z))
    }

    /// Metres one unit of `u` and of `v` reach.
    pub(crate) fn scale(&self) -> (f32, f32) {
        (self.du.length(), self.dv.length())
    }
}

/// Where on the screen's surface texture coordinate (`u`, `v`) lies, and the surface's
/// normal there: on the triangle that shows it, else (between triangles) on its plane.
pub(crate) fn surface_at(tris: &[Tri], u: f32, v: f32) -> Option<(Vec3, Vec3)> {
    let q = Vec2::new(u, v);
    for t in tris {
        let (a, b, c) = (t.uv[0], t.uv[1], t.uv[2]);
        let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if det.abs() < 1e-12 {
            continue;
        }
        let w1 = ((q.x - a.x) * (c.y - a.y) - (c.x - a.x) * (q.y - a.y)) / det;
        let w2 = ((b.x - a.x) * (q.y - a.y) - (q.x - a.x) * (b.y - a.y)) / det;
        let w0 = 1.0 - w1 - w2;
        let eps = -1e-4;
        if w0 >= eps && w1 >= eps && w2 >= eps {
            let p = t.p[0] * w0 + t.p[1] * w1 + t.p[2] * w2;
            let geometric = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).normalize_or_zero();
            let smooth = (t.n[0] * w0 + t.n[1] * w1 + t.n[2] * w2).normalize_or_zero();
            let n = if smooth != Vec3::ZERO { smooth } else { geometric };
            return Some((p, n));
        }
    }
    let plane = plane_of(tris)?;
    Some((plane.at(u, v), plane.normal))
}

/// The part of the texture the triangles show (`u0`, `v0`, `u1`, `v1`), as their texture
/// coordinates have it: a texture repeats, and a mesh may show it whole turns away (the
/// Hamburg buses' text textures lie at `v` -0.57..-0.47, which is 0.43..0.53 of the texture).
/// At most one turn of it.
pub(crate) fn uv_bounds(tris: &[Tri]) -> Option<[f32; 4]> {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for uv in tris.iter().flat_map(|t| t.uv) {
        b = [b[0].min(uv.x), b[1].min(uv.y), b[2].max(uv.x), b[3].max(uv.y)];
    }
    if !(b[0].is_finite() && b[1].is_finite() && b[2].is_finite() && b[3].is_finite()) {
        return None;
    }
    let b = [b[0], b[1], b[2].min(b[0].floor() + 1.0), b[3].min(b[1].floor() + 1.0)];
    (b[2] - b[0] > 1e-4 && b[3] - b[1] > 1e-4).then_some(b)
}

/// A texture coordinate of the part `crop` within the texture's own 0..1 (the turn `crop`
/// starts in taken off).
pub(crate) fn in_texture(crop: [f32; 4], u: f32, v: f32) -> (f32, f32) {
    (u - crop[0].floor(), v - crop[1].floor())
}

/// The pixels of a texture (`width` x `height`) the part `crop` covers: x0, y0, x1, y1.
pub(crate) fn crop_pixels(crop: [f32; 4], width: u32, height: u32) -> [u32; 4] {
    let (u0, v0) = in_texture(crop, crop[0], crop[1]);
    let (u1, v1) = in_texture(crop, crop[2], crop[3]);
    let crop = [u0, v0, u1, v1];
    let px = |f: f32, n: u32| ((f * n as f32).round().max(0.0) as u32).min(n);
    let (x0, y0) = (px(crop[0], width), px(crop[1], height));
    [x0, y0, px(crop[2], width).max(x0 + 1).min(width), px(crop[3], height).max(y0 + 1).min(height)]
}

/// A switch the screen's device has beside the screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Key {
    /// The model mesh (index into the bus's meshes) and its `[mouseevent]`.
    pub mesh: usize,
    pub event: String,
    pub label: String,
    /// Where it is: left, top, width, height in metres from the top left corner of the part
    /// of the screen shown (left of or above the screen is negative).
    pub rect: [f32; 4],
}

/// A `[mouseevent]` mesh near a screen: its mesh, its event, the corners of its box and the
/// radius of its sphere (both in the bus's frame).
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub mesh: usize,
    pub event: String,
    pub corners: [Vec3; 8],
    pub radius: f32,
}

/// The switches beside a screen showing `crop` of its texture on `plane`: in the plane's
/// neighbourhood, small, next to the picture rather than over it (one over it is worked by a
/// tap on the picture), the nearest [`MAX_KEYS`].
pub(crate) fn keys_beside(plane: &Plane, crop: [f32; 4], candidates: &[Candidate]) -> Vec<Key> {
    let (su, sv) = plane.scale();
    if su < 1e-6 || sv < 1e-6 {
        return Vec::new();
    }
    let (w, h) = ((crop[2] - crop[0]) * su, (crop[3] - crop[1]) * sv);
    let margin = 0.12f32.max(w.max(h));
    let mut found: Vec<(f32, Key)> = Vec::new();
    for c in candidates {
        if c.radius > KEY_MAX * 1.5 {
            continue;
        }
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut off = 0.0f32;
        let mut ok = true;
        for p in c.corners {
            let Some((u, v, o)) = plane.project(p) else {
                ok = false;
                break;
            };
            let (x, y) = ((u - crop[0]) * su, (v - crop[1]) * sv);
            r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            off += o / 8.0;
        }
        if !ok || r[2] - r[0] > KEY_MAX || r[3] - r[1] > KEY_MAX {
            continue;
        }
        let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
        let over = cx > 0.0 && cx < w && cy > 0.0 && cy < h;
        let near = cx > -margin && cx < w + margin && cy > -margin && cy < h + margin;
        if over || !near {
            continue;
        }
        // (distance from the screen's rectangle; a keypad may lean away from its display's
        // plane, as a ticket table's does)
        let dx = (-cx).max(cx - w).max(0.0);
        let dy = (-cy).max(cy - h).max(0.0);
        if off.abs() > KEY_OFF_PLANE + KEY_TILT * dx.hypot(dy) {
            continue;
        }
        let rect = [r[0], r[1], (r[2] - r[0]).max(0.004), (r[3] - r[1]).max(0.004)];
        found.push((dx.hypot(dy), Key { mesh: c.mesh, event: c.event.clone(), label: String::new(), rect }));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.truncate(MAX_KEYS);
    let mut keys: Vec<Key> = found.into_iter().map(|f| f.1).collect();
    let labels = key_labels(&keys.iter().map(|k| k.event.as_str()).collect::<Vec<_>>());
    for (k, l) in keys.iter_mut().zip(labels) {
        k.label = l;
    }
    // (in reading order: rows from the top, left to right)
    keys.sort_by(|a, b| (a.rect[1] / 0.01).round().total_cmp(&(b.rect[1] / 0.01).round()).then(a.rect[0].total_cmp(&b.rect[0])));
    keys
}


/// Keys this close to the screen, or to a key that is the screen's device's, are the device's
/// too (m): a keypad laid out round a display is, the two keys of another device beside it are
/// not.
const KEY_GAP: f32 = 0.04;

/// How far apart two rectangles (left, top, width, height) are: 0 when they touch.
fn gap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let dx = (b[0] - (a[0] + a[2])).max(a[0] - (b[0] + b[2])).max(0.0);
    let dy = (b[1] - (a[1] + a[3])).max(a[1] - (b[1] + b[3])).max(0.0);
    dx.hypot(dy)
}

/// The keys of the device whose screen (`w` x `h` m) they lie round: those within [`KEY_GAP`]
/// of the screen or of one of its keys, found outwards from the screen.
pub(crate) fn device_keys(w: f32, h: f32, keys: Vec<Key>) -> Vec<Key> {
    let screen = [0.0, 0.0, w, h];
    let mut mine: Vec<bool> = keys.iter().map(|k| gap(screen, k.rect) <= KEY_GAP).collect();
    loop {
        let grew: Vec<usize> = (0..keys.len()).filter(|&i| !mine[i] && (0..keys.len()).any(|j| mine[j] && gap(keys[j].rect, keys[i].rect) <= KEY_GAP)).collect();
        if grew.is_empty() {
            break;
        }
        for i in grew {
            mine[i] = true;
        }
    }
    keys.into_iter().zip(mine).filter(|k| k.1).map(|k| k.0).collect()
}

/// Short labels for the keys of one device: the word all of them begin with (`IBIS_`) left
/// out, words that only say "button" left out, a few well-known ones as signs.
pub(crate) fn key_labels(events: &[&str]) -> Vec<String> {
    let first = |e: &str| e.split(['_', ' ']).next().unwrap_or("").to_ascii_lowercase();
    let shared = events.first().map(|e| first(e)).filter(|f| !f.is_empty() && events.len() > 1 && events.iter().filter(|e| first(e) == *f).count() * 2 > events.len());
    events
        .iter()
        .map(|e| {
            let words: Vec<&str> = e.split(['_', ' ']).filter(|w| !w.is_empty()).collect();
            let skip = usize::from(shared.as_deref().is_some_and(|s| words.len() > 1 && words[0].eq_ignore_ascii_case(s)));
            let rest: Vec<&str> = words[skip..]
                .iter()
                .copied()
                .filter(|w| !matches!(w.to_ascii_lowercase().as_str(), "set" | "setmode" | "mode" | "taste" | "button" | "btn" | "key" | "knopf" | "click"))
                .collect();
            let text = match rest.last().map(|w| w.to_ascii_lowercase()).as_deref() {
                _ if rest.is_empty() => words.last().copied().unwrap_or(e).to_string(),
                Some("eingabe" | "enter" | "ok") if rest.len() == 1 => "\u{23ce}".into(),
                Some("loeschen" | "löschen" | "clear" | "ce" | "del") if rest.len() == 1 => "C".into(),
                Some("vor" | "next" | "right" | "rechts") if rest.len() == 1 => "\u{25b6}".into(),
                Some("rueck" | "zurueck" | "back" | "prev" | "left" | "links") if rest.len() == 1 => "\u{25c0}".into(),
                Some("hoch" | "up" | "auf") if rest.len() == 1 => "\u{25b2}".into(),
                Some("runter" | "down" | "ab") if rest.len() == 1 => "\u{25bc}".into(),
                _ => rest.join(" "),
            };
            text.chars().take(10).collect()
        })
        .collect()
}

/// A screen's name from its mesh's file: `generic\AFR4\AFR4_Display.o3d` is "AFR4 Display".
pub(crate) fn screen_name(mesh_file: &str) -> String {
    let stem = std::path::Path::new(&mesh_file.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = stem.replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() { "Screen".into() } else { name }
}

/// Whether a watched screen is due for another picture.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Pacer {
    last: Option<f64>,
}

impl Pacer {
    /// `now` in seconds of a steady clock; true (and counted as taken) at most `fps` times a
    /// second.
    pub(crate) fn due(&mut self, now: f64, fps: f64) -> bool {
        if self.last.is_some_and(|t| now - t < 1.0 / fps && now >= t) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

// ---------------------------------------------------------------------------------------
// the bus

/// Where a screen's picture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// `host.script_textures[n]` (an `[htmltexture]` page draws into one too).
    Script(usize),
    /// `text_textures[n]`.
    Text(usize),
    /// None of its own: a device of pages (an ALMEX, see [`super::panels`]), only the game's
    /// picture of it.
    Panel,
}

/// A screen of the bus driven, as found when the bus came.
#[derive(Debug, Clone)]
pub(crate) struct Screen {
    pub id: String,
    pub source: Source,
    /// An `[htmltexture]` page: taps go to it.
    pub html: bool,
    pub name: String,
    /// The part of the texture shown (u0, v0, u1, v1).
    pub crop: [f32; 4],
    /// The size of that part in the bus (m).
    pub size: [f32; 2],
    /// The meshes showing it and their triangles (first index of each in the mesh's indices).
    pub meshes: Vec<(usize, Vec<usize>)>,
    pub keys: Vec<Key>,
    /// The camera that photographs the device (the screen and its keys), and where its keys
    /// are in that picture (x0, y0, x1, y1 within 0..1; one a key, empty for a key outside it).
    pub view: Option<super::device::View>,
    pub key_spots: Vec<[f32; 4]>,
    /// A device that moves (an ALMEX on the cab's door): the mesh it moves with, and the
    /// inverse of where that mesh was when the view was made (bus frame).
    pub anchor: Option<(usize, glam::Mat4)>,
    /// The switches over its display (touch fields): their meshes.
    pub fields: Vec<usize>,
    /// The device as the page draws it ([`super::form`]); None when nothing of it could be
    /// laid flat (the page shows its live picture then).
    pub form: Option<std::sync::Arc<super::form::Form>>,
    // what was last sent of it
    pub pacer: Pacer,
    /// When its form's live state was last made.
    pub live: Pacer,
    pub last: Option<std::sync::Arc<Vec<u8>>>,
    pub last_text: Option<String>,
    /// A picture of a texture its script keeps locked, waiting to be the same at the next
    /// look (then it is no half-drawn one).
    pub still: Option<std::sync::Arc<Vec<u8>>>,
    /// Since when a tap or key holds a switch of it down (let go with the release).
    pub held: Option<std::time::Instant>,
    /// A page (`[htmltexture]`) a tap on the device's picture pressed: its script texture and
    /// where (it gets the moves and the release).
    pub page_held: Option<(usize, f32, f32)>,
}

impl Screen {
    /// The camera that photographs the device, where the device is now.
    pub(crate) fn view_now(&self, v: &omsi_sim::VehicleInstance) -> Option<super::device::View> {
        let view = self.view?;
        let Some((mesh, was)) = self.anchor else { return Some(view) };
        if mesh >= v.ty.meshes.len() {
            return Some(view);
        }
        let now = v.body_rotation().inverse() * v.mesh_local_transform(mesh);
        if !now.is_finite() || now.determinant().abs() < 1e-9 {
            return Some(view);
        }
        Some(view.moved(now * was))
    }
}

/// The triangles of `meshes` as they stand now (the mesh's animation and the bus's turn
/// included), leaving out hidden meshes.
pub(crate) fn tris_now(v: &omsi_sim::VehicleInstance, meshes: &[(usize, Vec<usize>)]) -> Vec<Tri> {
    let mut out = Vec::new();
    for (i, firsts) in meshes {
        let (Some(vm), Some(props)) = (v.ty.meshes.get(*i), v.mesh_props.get(*i)) else { continue };
        if !props.visible {
            continue;
        }
        let xf = v.mesh_local_transform(*i);
        let d = &vm.data;
        for &k in firsts {
            let Some(idx) = d.indices.get(k..k + 3) else { continue };
            let at = |j: usize| -> Option<(Vec3, Vec3, Vec2)> {
                let x = idx[j] as usize;
                Some((xf.transform_point3(*d.positions.get(x)?), xf.transform_vector3(d.normals.get(x).copied().unwrap_or(Vec3::ZERO)).normalize_or_zero(), d.uvs.get(x).copied().unwrap_or(Vec2::ZERO)))
            };
            let (Some(a), Some(b), Some(c)) = (at(0), at(1), at(2)) else { continue };
            out.push(Tri { p: [a.0, b.0, c.0], n: [a.1, b.1, c.1], uv: [a.2, b.2, c.2] });
        }
    }
    out
}

/// The corners of mesh `i`'s box as the mesh stands now (bus frame).
pub(crate) fn mesh_corners(v: &omsi_sim::VehicleInstance, i: usize) -> Option<[Vec3; 8]> {
    let (lo, hi) = *v.ty.mesh_boxes.get(i)?;
    let xf = v.mesh_local_transform(i);
    let mut c = [Vec3::ZERO; 8];
    for (k, p) in c.iter_mut().enumerate() {
        let q = Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z });
        *p = xf.transform_point3(q);
    }
    Some(c)
}

/// The middle of mesh `i` as it stands now (bus frame), and its radius.
pub(crate) fn mesh_centre(v: &omsi_sim::VehicleInstance, i: usize) -> Option<(Vec3, f32)> {
    let (c, r) = *v.ty.mesh_bounds.get(i)?;
    Some((v.mesh_local_transform(i).transform_point3(c), r))
}

/// The screens of a bus: the textures its inside shows, pages first, then the devices with
/// keys, then the rest by size.
pub(crate) fn discover(v: &omsi_sim::VehicleInstance, root: &std::path::Path) -> Vec<Screen> {
    let inside = |vp: i32| vp == 0 || vp & 2 != 0;
    // the driver's eye in the bus's own frame (the seat's camera), whence the devices are
    // photographed
    let def = &v.ty.def;
    let eye = def.cameras_driver.get(def.camera_std).or(def.cameras_driver.first()).map_or(Vec3::new(0.0, 0.0, 2.0), |c| Vec3::new(c.pos[0], c.pos[1], c.pos[2]));
    let pages: Vec<usize> = v.html_textures.iter().map(|t| t.script_index).collect();
    // per texture: (lod, mesh, triangles)
    let mut shown: std::collections::BTreeMap<(u8, usize), Vec<(usize, usize, Vec<usize>)>> = Default::default();
    for (i, vm) in v.ty.meshes.iter().enumerate() {
        let Some(def) = v.ty.model.meshes.get(vm.def_index) else { continue };
        if !inside(def.viewpoint) {
            continue;
        }
        for m in &def.materials {
            let src = match (m.use_script_texture, m.use_text_texture) {
                (Some(n), _) if n >= 0 && (n as usize) < v.host.script_textures.len() => (0u8, n as usize),
                (None, Some(n)) if n >= 0 && (n as usize) < v.text_textures.len() => (1u8, n as usize),
                _ => continue,
            };
            let Some(slot) = omsi_sim::vehicle::override_slot(&vm.materials, m) else { continue };
            let firsts: Vec<usize> = vm.data.ranges.iter().filter(|r| r.2 as usize == slot).flat_map(|r| (r.0 as usize..(r.0 + r.1) as usize).step_by(3)).collect();
            if !firsts.is_empty() {
                shown.entry(src).or_default().push((def.lod, i, firsts));
            }
        }
    }
    // the switches of the inside, for the keys beside each screen
    let candidates: Vec<Candidate> = v
        .ty
        .meshes
        .iter()
        .enumerate()
        .filter_map(|(i, vm)| {
            let def = v.ty.model.meshes.get(vm.def_index)?;
            let event = def.mouse_event.as_deref().map(str::trim).filter(|e| !e.is_empty())?;
            if !inside(def.viewpoint) || def.lod > 0 {
                return None;
            }
            let (_, radius) = mesh_centre(v, i)?;
            Some(Candidate { mesh: i, event: event.to_string(), corners: mesh_corners(v, i)?, radius })
        })
        .collect();
    // the devices made of pages (an ALMEX), from the inside's meshes in the bus's own frame
    let unturn = v.body_rotation().inverse();
    let parts = panel_parts(v, &inside);
    let panels = super::panels::find(&parts, eye);
    let mut out: Vec<Screen> = Vec::new();
    for ((kind, n), mut meshes) in shown {
        // (only the finest level of detail that shows it)
        let lod = meshes.iter().map(|m| m.0).min().unwrap_or(0);
        meshes.retain(|m| m.0 == lod);
        let meshes: Vec<(usize, Vec<usize>)> = meshes.into_iter().map(|m| (m.1, m.2)).collect();
        // where the screen is: its visible meshes (a bus with two kinds of device hides the
        // one it does not have), all of them when none is (a display switched off is still a
        // screen); taps go to whichever is visible then (`tris_now`)
        let visible: Vec<&(usize, Vec<usize>)> = meshes.iter().filter(|m| v.mesh_props.get(m.0).is_some_and(|p| p.visible)).collect();
        let placed: Vec<&(usize, Vec<usize>)> = if visible.is_empty() { meshes.iter().collect() } else { visible };
        let name_file = v.ty.model.meshes[v.ty.meshes[placed[0].0].def_index].file.clone();
        // (the device moves with its display)
        let meshes_anchor = placed[0].0;
        let all: Vec<Tri> = {
            let mut t = Vec::new();
            for (i, firsts) in placed {
                let xf = v.mesh_local_transform(*i);
                let d = &v.ty.meshes[*i].data;
                for &k in firsts {
                    let Some(idx) = d.indices.get(k..k + 3) else { continue };
                    let get = |j: usize| -> Option<(Vec3, Vec3, Vec2)> {
                        let x = idx[j] as usize;
                        Some((xf.transform_point3(*d.positions.get(x)?), xf.transform_vector3(d.normals.get(x).copied().unwrap_or(Vec3::ZERO)), d.uvs.get(x).copied().unwrap_or(Vec2::ZERO)))
                    };
                    if let (Some(a), Some(b), Some(c)) = (get(0), get(1), get(2)) {
                        t.push(Tri { p: [a.0, b.0, c.0], n: [a.1, b.1, c.1], uv: [a.2, b.2, c.2] });
                    }
                }
            }
            t
        };
        let (Some(crop), Some(plane)) = (uv_bounds(&all), plane_of(&all)) else { continue };
        // (written on a device of pages: part of that device's picture)
        let middle = unturn.transform_point3(plane.at((crop[0] + crop[2]) * 0.5, (crop[1] + crop[3]) * 0.5));
        if panels.iter().any(|d| d.holds(middle, super::panels::FACE_OFF)) {
            continue;
        }
        let (su, sv) = plane.scale();
        let size = [(crop[2] - crop[0]) * su, (crop[3] - crop[1]) * sv];
        let keys = device_keys(size[0], size[1], keys_beside(&plane, crop, &candidates));
        // the device's picture: its face scanned straight on, in the bus's own frame (the
        // points above are turned with the bus); from the driver's eye should its plane be
        // no plane
        let face = Face { top_left: unturn.transform_point3(plane.at(crop[0], crop[1])), ax: unturn.transform_vector3(plane.du / su.max(1e-6)), ay: unturn.transform_vector3(plane.dv / sv.max(1e-6)), normal: unturn.transform_vector3(plane.normal) };
        let key_corners: Vec<[Vec3; 8]> = keys.iter().map(|k| mesh_corners(v, k.mesh).map(|c| c.map(|p| unturn.transform_point3(p))).unwrap_or([Vec3::ZERO; 8])).collect();
        let rect = device_face(v, &face, size[0], size[1], &keys);
        let form = rect.and_then(|r| super::form::Sheet::of(r.0, r.1, r.2, r.3)).and_then(|sheet| super::form::of_vehicle(v, root, &sheet, Some(meshes_anchor))).map(std::sync::Arc::new);
        let view = rect.and_then(|r| super::device::View::face(r.0, r.1, r.2, r.3)).or_else(|| {
            let mut points: Vec<Vec3> = all.iter().flat_map(|t| t.p).map(|p| unturn.transform_point3(p)).collect();
            points.extend(key_corners.iter().flatten().copied().filter(|p| *p != Vec3::ZERO));
            super::device::View::frame(eye, face.normal, &points, Vec3::Z)
        });
        // (one a key, in the keys' order; an empty one for a key outside the picture)
        let key_spots = view.map(|vw| key_corners.iter().map(|c| vw.rect_of(c).unwrap_or([0.0; 4])).collect()).unwrap_or_default();
        let (id, source, html) = match kind {
            0 => (format!("s{n}"), Source::Script(n), pages.contains(&n)),
            _ => (format!("t{n}"), Source::Text(n), false),
        };
        out.push(Screen { id, source, html, name: screen_name(&name_file), crop, size, meshes, keys, view, key_spots, anchor: None, fields: Vec::new(), form, pacer: Pacer::default(), live: Pacer::default(), last: None, last_text: None, still: None, held: None, page_held: None });
    }
    for d in panels {
        out.push(panel_screen(v, root, &parts, d));
    }
    // (the devices of pages first, then the pages, the devices with keys, the rest by size)
    out.sort_by(|a, b| {
        (b.source == Source::Panel)
            .cmp(&(a.source == Source::Panel))
            .then(b.html.cmp(&a.html))
            .then((!b.keys.is_empty()).cmp(&!a.keys.is_empty()))
            .then((b.size[0] * b.size[1]).total_cmp(&(a.size[0] * a.size[1])))
    });
    // (two screens of one name: numbered)
    let names: Vec<String> = out.iter().map(|s| s.name.clone()).collect();
    for (k, s) in out.iter_mut().enumerate() {
        if names.iter().filter(|n| **n == s.name).count() > 1 {
            let nth = names[..k].iter().filter(|n| **n == s.name).count() + 1;
            s.name = format!("{} {nth}", s.name);
        }
    }
    out
}

/// A device's face (bus frame): the top left corner of its display, across and down the
/// display (unit), out of it towards whoever looks at it.
pub(crate) struct Face {
    pub top_left: Vec3,
    pub ax: Vec3,
    pub ay: Vec3,
    pub normal: Vec3,
}

/// A device's case reaches at most this share of its display's and keys' size beyond them...
const CASE_REACH: f32 = 0.35;
/// ...and is made of the meshes' corners on its face (this far behind and before the
/// display's plane, m) that touch them or each other (within this, m), each mesh's no bigger
/// than this share of their size, the mesh no bigger than this radius (m).
const CASE_BEHIND: f32 = 0.03;
const CASE_BEFORE: f32 = 0.06;
const CASE_TOUCH: f32 = 0.006;
const CASE_SIZE: f32 = 1.6;
const CASE_MAX: f32 = 0.6;
/// Room round a device with keys in its picture (a share of its longer side).
const CASE_MARGIN: f32 = 0.025;

/// What of a device the camera scans and the page draws: its display (`w` x `h` m from the
/// face's corner) and, when it has keys round it, the keys and the case they sit in with a
/// little room - a display alone is its display, edge to edge.
///
/// It is the rectangle of the face: its top left corner, across its width, down its height,
/// out of it (bus frame), for [`super::device::View::face`] and [`super::form::Sheet::of`].
pub(crate) fn device_face(v: &omsi_sim::VehicleInstance, face: &Face, w: f32, h: f32, keys: &[Key]) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    if keys.is_empty() {
        return Some((face.top_left, face.ax * w, face.ay * h, face.normal));
    }
    // the device's face: the display's and the keypad's planes as one (a ticket table's keypad
    // leans back from its display), the display's up its up
    let unturn = v.body_rotation().inverse();
    let corners: Vec<Vec3> = keys.iter().filter_map(|k| mesh_corners(v, k.mesh)).flatten().map(|p| unturn.transform_point3(p)).collect();
    let centres: Vec<Vec3> = keys.iter().filter_map(|k| mesh_centre(v, k.mesh)).map(|(c, _)| unturn.transform_point3(c)).collect();
    let display = [face.top_left, face.top_left + face.ax * w, face.top_left + face.ay * h, face.top_left + face.ax * w + face.ay * h];
    let keys_area = {
        let (lo, hi) = centres.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(lo, hi), c| {
            let q = Vec2::new((*c - face.top_left).dot(face.ax), (*c - face.top_left).dot(face.ay));
            (lo.min(q), hi.max(q))
        });
        ((hi - lo).max(Vec2::ZERO) + Vec2::splat(0.02)).element_product()
    };
    let normal = match plane_normal(&centres, face.normal) {
        Some(n) if n.dot(face.normal) < 0.9986 => (face.normal * (w * h) + n * keys_area).normalize_or(face.normal),
        _ => face.normal,
    };
    let up = (-face.ay - normal * (-face.ay).dot(normal)).normalize_or_zero();
    if up == Vec3::ZERO {
        return None;
    }
    let (ax, ay) = (up.cross(normal), -up);
    let points: Vec<Vec3> = display.iter().copied().chain(corners.iter().copied()).collect();
    let origin = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let tilted = Face { top_left: origin, ax, ay, normal };
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in &points {
        let (x, y) = ((*p - origin).dot(ax), (*p - origin).dot(ay));
        r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
    }
    let r = case_round(v, &tilted, r);
    let m = CASE_MARGIN * (r[2] - r[0]).max(r[3] - r[1]);
    let r = [r[0] - m, r[1] - m, r[2] + m, r[3] + m];
    Some((origin + ax * r[0] + ay * r[1], ax * (r[2] - r[0]), ay * (r[3] - r[1]), normal))
}

/// The normal of the plane the points lie in (towards `towards`), when they span one.
pub(crate) fn plane_normal(points: &[Vec3], towards: Vec3) -> Option<Vec3> {
    if points.len() < 4 {
        return None;
    }
    let c = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let mut sum = Vec3::ZERO;
    for (k, a) in points.iter().enumerate() {
        for b in &points[k + 1..] {
            let n = (*a - c).cross(*b - c);
            sum += if n.dot(towards) < 0.0 { -n } else { n };
        }
    }
    // (points along a line span no plane)
    let spread: f32 = points.iter().map(|p| (*p - c).length_squared()).sum();
    (sum.length() > 1e-3 * spread * points.len() as f32).then(|| sum.normalize())
}

/// How far a device's case reaches round its display and keys (`r`: left, top, right, bottom
/// in metres on its face): the meshes of the inside on its face that touch them, or touch what
/// touches them - its case's front, the keys that do nothing in this bus, the labels beside
/// them - none much bigger than the display and keys together, at most [`CASE_REACH`] beyond
/// them.
fn case_round(v: &omsi_sim::VehicleInstance, face: &Face, r: [f32; 4]) -> [f32; 4] {
    let unturn = v.body_rotation().inverse();
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let centre = face.top_left + face.ax * ((r[0] + r[2]) * 0.5) + face.ay * ((r[1] + r[3]) * 0.5);
    let mut parts: Vec<[f32; 4]> = Vec::new();
    for (i, vm) in v.ty.meshes.iter().enumerate() {
        let Some(def) = v.ty.model.meshes.get(vm.def_index) else { continue };
        if def.lod > 0 || !(def.viewpoint == 0 || def.viewpoint & 2 != 0) || !v.mesh_props.get(i).is_some_and(|p| p.visible) || vm.data.positions.len() > 20_000 {
            continue;
        }
        let Some((c, rad)) = mesh_centre(v, i) else { continue };
        if rad > CASE_MAX || unturn.transform_point3(c).distance(centre) > rad + w.max(h) {
            continue;
        }
        let xf = unturn * v.mesh_local_transform(i);
        // (its corners on the face: a case's front, not its sides and the arm it hangs on)
        let mut m = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &vm.data.positions {
            let q = xf.transform_point3(*p) - face.top_left;
            if (-CASE_BEHIND..=CASE_BEFORE).contains(&q.dot(face.normal)) {
                let (x, y) = (q.dot(face.ax), q.dot(face.ay));
                m = [m[0].min(x), m[1].min(y), m[2].max(x), m[3].max(y)];
            }
        }
        if m[2] >= m[0] && m[2] - m[0] <= CASE_SIZE * w && m[3] - m[1] <= CASE_SIZE * h {
            parts.push(m);
        }
    }
    // (outwards from the display and keys, part by part)
    let gap = |a: [f32; 4], b: [f32; 4]| (b[0] - a[2]).max(a[0] - b[2]).max(0.0).hypot((b[1] - a[3]).max(a[1] - b[3]).max(0.0));
    let mut out = r;
    let mut taken = vec![false; parts.len()];
    loop {
        let mut grew = false;
        for (k, m) in parts.iter().enumerate() {
            if !taken[k] && gap(*m, out) <= CASE_TOUCH {
                taken[k] = true;
                grew = true;
                out = [out[0].min(m[0]), out[1].min(m[1]), out[2].max(m[2]), out[3].max(m[3])];
            }
        }
        if !grew {
            break;
        }
    }
    [out[0].max(r[0] - CASE_REACH * w), out[1].max(r[1] - CASE_REACH * h), out[2].min(r[2] + CASE_REACH * w), out[3].min(r[3] + CASE_REACH * h)]
}

/// The meshes of the inside a device of pages may be made of (see [`super::panels`]): those
/// switched by a variable or named a screen, those written on, the switches; in the bus's own
/// frame as they stand now.
fn panel_parts(v: &omsi_sim::VehicleInstance, inside: &dyn Fn(i32) -> bool) -> Vec<super::panels::Part> {
    let unturn = v.body_rotation().inverse();
    v.ty.meshes
        .iter()
        .enumerate()
        .filter_map(|(i, vm)| {
            let def = v.ty.model.meshes.get(vm.def_index)?;
            if !inside(def.viewpoint) || def.lod > 0 {
                return None;
            }
            let face = def.materials.iter().any(|m| m.use_script_texture.is_some_and(|n| n >= 0 && (n as usize) < v.host.script_textures.len()) || m.use_text_texture.is_some_and(|n| n >= 0 && (n as usize) < v.text_textures.len()));
            let event = def.mouse_event.as_deref().map(str::trim).filter(|e| !e.is_empty()).map(str::to_string);
            let name = std::path::Path::new(&def.file.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let named = super::panels::SCREEN_WORDS.iter().any(|w| name.contains(w));
            if !(face || event.is_some() || def.visible.is_some() || named) {
                return None;
            }
            // (a whole wall, or a mesh of thousands of triangles, is no part of a device)
            let (_, radius) = *v.ty.mesh_bounds.get(i)?;
            let d = &vm.data;
            if radius > 2.0 * BIG || d.indices.len() > 3 * 4096 {
                return None;
            }
            let xf = unturn * v.mesh_local_transform(i);
            let tris = d
                .indices
                .chunks_exact(3)
                .filter_map(|idx| {
                    let at = |j: usize| -> Option<(Vec3, Vec3, Vec2)> {
                        let x = idx[j] as usize;
                        Some((xf.transform_point3(*d.positions.get(x)?), xf.transform_vector3(d.normals.get(x).copied().unwrap_or(Vec3::ZERO)).normalize_or_zero(), d.uvs.get(x).copied().unwrap_or(Vec2::ZERO)))
                    };
                    let (a, b, c) = (at(0)?, at(1)?, at(2)?);
                    Some(Tri { p: [a.0, b.0, c.0], n: [a.1, b.1, c.1], uv: [a.2, b.2, c.2] })
                })
                .collect();
            Some(super::panels::Part { mesh: i, name, switch: def.visible.clone(), face, event, tris, radius })
        })
        .collect()
}

/// The screen of a device of pages: its picture the display (and the keys round it) seen
/// straight on, at a tablet's size.
fn panel_screen(v: &omsi_sim::VehicleInstance, root: &std::path::Path, parts: &[super::panels::Part], d: super::panels::Panel) -> Screen {
    let (w, h) = d.size();
    let unturn = v.body_rotation().inverse();
    let meshes: Vec<(usize, Vec<usize>)> = d.pages.iter().map(|&k| parts[k].mesh).map(|i| (i, (0..v.ty.meshes[i].data.indices.len() / 3 * 3).step_by(3).collect())).collect();
    // the picture: the display (and the keys round it, in their case) scanned straight on
    let face = Face { top_left: d.plane.origin, ax: d.plane.du / w.max(1e-6), ay: d.plane.dv / h.max(1e-6), normal: d.plane.normal };
    let rect = device_face(v, &face, w, h, &d.keys);
    let view = rect.and_then(|r| super::device::View::face(r.0, r.1, r.2, r.3));
    let first_page = meshes.first().map_or(0, |m| m.0);
    let form = rect.and_then(|r| super::form::Sheet::of(r.0, r.1, r.2, r.3)).and_then(|sheet| super::form::of_vehicle(v, root, &sheet, Some(first_page))).map(std::sync::Arc::new);
    let key_corners: Vec<[Vec3; 8]> = d.keys.iter().map(|k| mesh_corners(v, k.mesh).map(|c| c.map(|p| unturn.transform_point3(p))).unwrap_or([Vec3::ZERO; 8])).collect();
    let key_spots = view.map(|vw| key_corners.iter().map(|c| vw.rect_of(c).unwrap_or([0.0; 4])).collect()).unwrap_or_default();
    let first = meshes.first().map_or(0, |m| m.0);
    let was = unturn * v.mesh_local_transform(first);
    let anchor = (was.is_finite() && was.determinant().abs() > 1e-9).then(|| (first, was.inverse()));
    Screen {
        id: format!("p{first}"),
        source: Source::Panel,
        html: false,
        name: d.name,
        crop: [0.0, 0.0, 1.0, 1.0],
        size: [w, h],
        meshes,
        keys: d.keys,
        view,
        key_spots,
        anchor,
        fields: d.fields.iter().map(|&k| parts[k].mesh).collect(),
        form,
        pacer: Pacer::default(),
        live: Pacer::default(),
        last: None,
        last_text: None,
        still: None,
        held: None,
        page_held: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screen 0.2 m wide and 0.1 m high standing upright at y = 1, facing -y (towards a
    /// driver at y < 1), showing the texture part (0.25, 0.5)..(0.75, 1.0).
    fn screen() -> Vec<Tri> {
        let n = Vec3::new(0.0, -1.0, 0.0);
        let p = |x: f32, z: f32| Vec3::new(x, 1.0, z);
        let (tl, tr, bl, br) = (p(0.0, 1.1), p(0.2, 1.1), p(0.0, 1.0), p(0.2, 1.0));
        let (uv_tl, uv_tr, uv_bl, uv_br) = (Vec2::new(0.25, 0.5), Vec2::new(0.75, 0.5), Vec2::new(0.25, 1.0), Vec2::new(0.75, 1.0));
        vec![Tri { p: [tl, tr, bl], n: [n; 3], uv: [uv_tl, uv_tr, uv_bl] }, Tri { p: [tr, br, bl], n: [n; 3], uv: [uv_tr, uv_br, uv_bl] }]
    }

    fn cube(c: Vec3, half: f32) -> [Vec3; 8] {
        let mut out = [Vec3::ZERO; 8];
        for (k, p) in out.iter_mut().enumerate() {
            *p = c + Vec3::new(if k & 1 == 0 { -half } else { half }, if k & 2 == 0 { -half } else { half }, if k & 4 == 0 { -half } else { half });
        }
        out
    }

    #[test]
    fn the_plane_of_a_screen_maps_its_texture_to_the_bus() {
        let pl = plane_of(&screen()).unwrap();
        assert!(pl.at(0.25, 0.5).distance(Vec3::new(0.0, 1.0, 1.1)) < 1e-5);
        assert!(pl.at(0.75, 1.0).distance(Vec3::new(0.2, 1.0, 1.0)) < 1e-5);
        assert!(pl.normal.distance(Vec3::new(0.0, -1.0, 0.0)) < 1e-5, "towards the driver");
        let (u, v, off) = pl.project(Vec3::new(0.1, 0.95, 1.05)).unwrap();
        assert!((u - 0.5).abs() < 1e-5 && (v - 0.75).abs() < 1e-5 && (off - 0.05).abs() < 1e-5);
        let (su, sv) = pl.scale();
        assert!((su - 0.4).abs() < 1e-5 && (sv - 0.2).abs() < 1e-5);
    }

    #[test]
    fn a_tap_lands_on_the_surface_where_the_texture_shows_it() {
        let (p, n) = surface_at(&screen(), 0.5, 0.75).unwrap();
        assert!(p.distance(Vec3::new(0.1, 1.0, 1.05)) < 1e-5);
        assert!(n.distance(Vec3::new(0.0, -1.0, 0.0)) < 1e-5);
        // a place the triangles do not cover still lands on the plane
        let (p, _) = surface_at(&screen(), 0.0, 0.0).unwrap();
        assert!(p.distance(Vec3::new(-0.1, 1.0, 1.2)) < 1e-5);
        assert_eq!(uv_bounds(&screen()), Some([0.25, 0.5, 0.75, 1.0]));
        assert_eq!(crop_pixels([0.25, 0.5, 0.75, 1.0], 512, 256), [128, 128, 384, 256]);
        assert_eq!(crop_pixels([0.5, 0.5, 0.5, 0.5], 4, 4), [2, 2, 3, 3], "never empty");
        // a texture shown whole turns away is that part of it
        let mut wrapped = screen();
        for t in &mut wrapped {
            for uv in &mut t.uv {
                *uv -= Vec2::new(2.0, 1.0);
            }
        }
        let crop = uv_bounds(&wrapped).unwrap();
        assert_eq!(crop, [-1.75, -0.5, -1.25, 0.0]);
        assert_eq!(crop_pixels(crop, 512, 256), [128, 128, 384, 256]);
        assert_eq!(in_texture(crop, -1.5, -0.25), (0.5, 0.75));
    }

    #[test]
    fn keys_beside_the_screen_are_laid_out_where_they_are() {
        let key = |mesh: usize, event: &str, c: Vec3| Candidate { mesh, event: event.into(), corners: cube(c, 0.01), radius: 0.017 };
        let cands = vec![
            key(1, "IBIS_1", Vec3::new(0.02, 0.99, 0.97)),
            key(2, "IBIS_2", Vec3::new(0.06, 0.99, 0.97)),
            key(3, "IBIS_eingabe", Vec3::new(0.18, 0.99, 0.97)),
            // over the screen: a touch field, worked by tapping the picture
            key(4, "IBIS_touch", Vec3::new(0.1, 0.99, 1.05)),
            // far away, or off the plane, or a big panel: not this device's
            key(5, "door_1", Vec3::new(1.5, 0.99, 0.97)),
            key(6, "IBIS_3", Vec3::new(0.1, 0.7, 0.97)),
            Candidate { mesh: 7, event: "VDV_position".into(), corners: cube(Vec3::new(0.1, 0.99, 0.9), 0.2), radius: 0.35 },
        ];
        let pl = plane_of(&screen()).unwrap();
        let keys = keys_beside(&pl, [0.25, 0.5, 0.75, 1.0], &cands);
        let events: Vec<&str> = keys.iter().map(|k| k.event.as_str()).collect();
        assert_eq!(events, vec!["IBIS_1", "IBIS_2", "IBIS_eingabe"]);
        let labels: Vec<&str> = keys.iter().map(|k| k.label.as_str()).collect();
        assert_eq!(labels, vec!["1", "2", "\u{23ce}"]);
        // below the screen (0.1 m high), at their places across it
        let k = &keys[0];
        assert!((k.rect[0] - 0.01).abs() < 1e-4 && (k.rect[1] - 0.12).abs() < 1e-4 && (k.rect[2] - 0.02).abs() < 1e-4, "{k:?}");
    }

    #[test]
    fn a_keypad_round_the_display_is_the_devices_and_another_devices_keys_are_not() {
        let key = |event: &str, x: f32, y: f32| Key { mesh: 0, event: event.into(), label: String::new(), rect: [x, y, 0.02, 0.02] };
        // a display 0.2 x 0.06 m; a keypad in rows under it, each row touching the one above;
        // two keys of another device 0.15 m to the left
        let mut keys = Vec::new();
        for row in 0..4 {
            for col in 0..9 {
                keys.push(key(&format!("pad_{row}{col}"), col as f32 * 0.022, 0.07 + row as f32 * 0.025));
            }
        }
        keys.push(key("IBIS_a", -0.17, 0.08));
        keys.push(key("IBIS_b", -0.17, 0.11));
        let mine = device_keys(0.2, 0.06, keys);
        assert_eq!(mine.len(), 36, "the whole keypad, row after row");
        assert!(mine.iter().all(|k| k.event.starts_with("pad_")));
        assert_eq!(gap([0.0, 0.0, 1.0, 1.0], [0.5, 0.5, 1.0, 1.0]), 0.0);
        assert!((gap([0.0, 0.0, 1.0, 1.0], [4.0, 5.0, 1.0, 1.0]) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn labels_leave_out_the_devices_name_and_words_for_button() {
        assert_eq!(key_labels(&["IBIS_7", "IBIS_setmode_linie_kurs", "IBIS_loeschen", "IBIS_vor"]), vec!["7", "linie kurs", "C", "\u{25b6}"]);
        assert_eq!(key_labels(&["Taste"]), vec!["Taste"]);
        assert_eq!(key_labels(&["ticketprinter_button_ticket_1", "ticketprinter_button_enter"]), vec!["ticket 1", "\u{23ce}"]);
        assert_eq!(screen_name("generic\\AFR4\\AFR4_Display.o3d"), "AFR4 Display");
        assert_eq!(screen_name(""), "Screen");
    }

    #[test]
    fn a_keypad_leaning_back_is_found_and_its_plane_known() {
        // a keypad under the screen, its lower rows towards the driver (20 degrees): its last
        // row lies well off the screen's plane, and is its keys all the same
        let lean = 20f32.to_radians();
        let mut cands = Vec::new();
        let mut centres = Vec::new();
        for row in 0..5 {
            for col in 0..4 {
                let down = 0.02 + row as f32 * 0.04;
                let c = Vec3::new(0.02 + col as f32 * 0.05, 1.0 - down * lean.sin(), 1.0 - down * lean.cos());
                centres.push(c);
                cands.push(Candidate { mesh: row * 4 + col, event: format!("pad_{row}{col}"), corners: cube(c, 0.009), radius: 0.016 });
            }
        }
        let pl = plane_of(&screen()).unwrap();
        let keys = device_keys(0.2, 0.1, keys_beside(&pl, [0.25, 0.5, 0.75, 1.0], &cands));
        assert_eq!(keys.len(), 20, "every row");
        // the keypad's plane: leaning from the screen's (-y), facing up a little
        let n = plane_normal(&centres, -Vec3::Y).unwrap();
        assert!((n.dot(-Vec3::Y) - lean.cos()).abs() < 1e-3 && n.z > 0.0, "{n:?}");
        // a row of keys spans no plane
        assert!(plane_normal(&centres[..4], -Vec3::Y).is_none());
    }

    #[test]
    fn a_watched_screen_is_read_at_most_fps_times_a_second() {
        let mut p = Pacer::default();
        assert!(p.due(10.0, FPS));
        assert!(!p.due(10.05, FPS));
        assert!(p.due(10.11, FPS));
        // (a clock that went back starts again)
        assert!(p.due(1.0, FPS));
    }
}
