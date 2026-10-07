//! Devices made of pages: a ticket printer, an IBIS or an info display built the way most
//! add-on buses build them.
//!
//! Not every device of a bus draws its picture into a script texture. The Hamburg buses'
//! ALMEX is a stack of flat meshes at one place, each a menu page with a picture of its own
//! (`17_almex_screen_0.o3d` .. `_20`), one shown at a time by `[visible] almex_menu N`; small
//! `[texttexture]` meshes on it write the clock, the input and the stops, and small
//! `[mouseevent]` meshes on it are its touch fields. Such a device is found by where its
//! meshes are: the pages (flat meshes at one place switched by one variable, or meshes named
//! a screen), the text and script textures on their plane, the switches on it and round it.
//!
//! Its picture is the game's own: a camera straight before the display surface (the pages'
//! bounds) photographs exactly that surface ([`super::device::View::face`]), and a tap on the
//! picture is clicked along the camera's ray into the cab, so the touch fields work as the
//! mouse works them.

use glam::Vec3;

use super::screens::{device_keys, keys_beside, plane_of, Candidate, Key, Plane, Tri, BIG, KEY_MAX};

/// A page is flat: its corners lie this close to its plane (m)...
const FLAT: f32 = 0.006;
/// ...and it is at least this big on either side (m): a lamp or a key is no page.
const PAGE_MIN: f32 = 0.025;
/// Two pages lie at one place when their planes are this close (m) and this parallel...
const SAME_PLANE: f32 = 0.015;
const PARALLEL: f32 = 0.97;
/// ...and they cover at least this share of the smaller one.
const OVERLAP: f32 = 0.5;
/// A stack of this many pages, with nothing written on it, is a device still (a menu); two
/// are a lamp's on and off.
const MENU: usize = 3;
/// A text or script texture lies on the display this close to its plane (m): a little before
/// it, as a decal.
pub(crate) const FACE_OFF: f32 = 0.03;
/// A switch over the display (a touch field) lies this close to its plane (m).
const FIELD_OFF: f32 = 0.06;
/// Room round the display within which a part still counts as on it (m).
const EDGE: f32 = 0.01;
/// Words that name a mesh as a screen.
pub(crate) const SCREEN_WORDS: [&str; 5] = ["screen", "display", "bildschirm", "anzeige", "monitor"];

/// A mesh of the inside as the device finder sees it.
#[derive(Debug, Clone)]
pub(crate) struct Part {
    /// The model mesh (index into the bus's meshes).
    pub mesh: usize,
    /// Its file's name without the folder and the extension, lower case.
    pub name: String,
    /// Its `[visible]` variable and the value that shows it.
    pub switch: Option<(String, f32)>,
    /// It shows a text or script texture (it is written on), not a picture of its own.
    pub face: bool,
    pub event: Option<String>,
    /// Its triangles in the bus's own frame.
    pub tris: Vec<Tri>,
    /// The radius of its sphere (m).
    pub radius: f32,
}

impl Part {
    fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.tris.iter().flat_map(|t| t.p)
    }

    fn centre(&self) -> Vec3 {
        let (lo, hi) = self.points().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        (lo + hi) * 0.5
    }

    fn named_screen(&self) -> bool {
        SCREEN_WORDS.iter().any(|w| self.name.contains(w))
    }
}

/// A device found: its pages, what is written on them, its switches.
#[derive(Debug, Clone)]
pub(crate) struct Panel {
    /// Indices into the parts: the pages (the largest first), the text and script textures on
    /// them, the switches over them.
    pub pages: Vec<usize>,
    pub faces: Vec<usize>,
    pub fields: Vec<usize>,
    /// The keys round the display (not over it).
    pub keys: Vec<Key>,
    /// The display surface: its top left corner (`origin`), across its width (`du`), down its
    /// height (`dv`), out of it towards whoever looks at it (`normal`).
    pub plane: Plane,
    pub name: String,
}

impl Panel {
    /// Width and height of the display (m).
    pub(crate) fn size(&self) -> (f32, f32) {
        self.plane.scale()
    }

    /// Whether `p` (bus frame) lies on the display, within `off` before or behind it.
    pub(crate) fn holds(&self, p: Vec3, off: f32) -> bool {
        let (w, h) = self.size();
        let (eu, ev) = (EDGE / w.max(1e-4), EDGE / h.max(1e-4));
        self.plane.project(p).is_some_and(|(u, v, o)| u > -eu && u < 1.0 + eu && v > -ev && v < 1.0 + ev && o.abs() <= off)
    }
}

/// A flat surface's frame: a point of it, across it (the texture's `u`), up it (against the
/// texture's `v`) and out of it.
#[derive(Debug, Clone, Copy)]
struct Frame {
    origin: Vec3,
    right: Vec3,
    up: Vec3,
    normal: Vec3,
}

impl Frame {
    /// The frame of a part's surface, from the triangle that covers most of its texture (the
    /// way its picture stands); facing the way its vertex normals say, else towards `eye`.
    fn of(tris: &[Tri], eye: Vec3) -> Option<Frame> {
        let pts: Vec<Vec3> = tris.iter().flat_map(|t| t.p).collect();
        if pts.is_empty() {
            return None;
        }
        let origin = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let biggest = tris.iter().max_by(|a, b| area(a).total_cmp(&area(b)))?;
        let (mut normal, along) = match plane_of(tris) {
            Some(pl) => (pl.normal, -pl.dv),
            None => ((biggest.p[1] - biggest.p[0]).cross(biggest.p[2] - biggest.p[0]).normalize_or_zero(), Vec3::Z),
        };
        if normal == Vec3::ZERO {
            return None;
        }
        // (vertex normals that say nothing: the side the driver sees)
        let said: Vec3 = tris.iter().flat_map(|t| t.n).sum();
        if said.dot(normal).abs() < 1e-3 * pts.len() as f32 {
            if (eye - origin).dot(normal) < 0.0 {
                normal = -normal;
            }
        } else if said.dot(normal) < 0.0 {
            normal = -normal;
        }
        let mut up = (along - normal * along.dot(normal)).normalize_or_zero();
        if up == Vec3::ZERO {
            up = (Vec3::Z - normal * normal.z).normalize_or_zero();
        }
        if up == Vec3::ZERO {
            up = normal.any_orthonormal_vector();
        }
        Some(Frame { origin, right: up.cross(normal), up, normal })
    }

    /// Where `p` is: across, up, before the surface (m).
    fn local(&self, p: Vec3) -> Vec3 {
        let d = p - self.origin;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.normal))
    }

    /// The rectangle (left, bottom, right, top) the points cover, and how far before the
    /// surface they lie (least, most).
    fn rect(&self, pts: impl Iterator<Item = Vec3>) -> Option<([f32; 4], f32, f32)> {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in pts {
            let q = self.local(p);
            r = [r[0].min(q.x), r[1].min(q.y), r[2].max(q.x), r[3].max(q.y)];
            lo = lo.min(q.z);
            hi = hi.max(q.z);
        }
        (r[2] >= r[0] && r[3] >= r[1]).then_some((r, lo, hi))
    }
}

fn area(t: &Tri) -> f32 {
    (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5
}

fn rect_area(r: [f32; 4]) -> f32 {
    (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0)
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    rect_area([a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])])
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// A page: flat, of a picture of its own (no text or script texture), switched or named a
/// screen, neither tiny nor a whole panel. Its frame and its rectangle in it.
fn page(p: &Part, eye: Vec3) -> Option<(Frame, [f32; 4])> {
    if p.face || p.radius > BIG || p.tris.is_empty() || (p.switch.is_none() && !p.named_screen()) {
        return None;
    }
    let f = Frame::of(&p.tris, eye)?;
    let (r, lo, hi) = f.rect(p.points())?;
    (lo >= -FLAT && hi <= FLAT && r[2] - r[0] >= PAGE_MIN && r[3] - r[1] >= PAGE_MIN).then_some((f, r))
}

/// Whether page `b` lies at the place of page `a` (in `a`'s frame, `ra` its rectangle): on its
/// plane and over most of the smaller of them.
fn same_place(a: &Frame, ra: [f32; 4], b: &Part, fb: &Frame) -> bool {
    if a.normal.dot(fb.normal) < PARALLEL {
        return false;
    }
    let Some((rb, lo, hi)) = a.rect(b.points()) else { return false };
    lo >= -SAME_PLANE && hi <= SAME_PLANE && overlap(ra, rb) >= OVERLAP * rect_area(ra).min(rect_area(rb)).max(1e-8)
}

/// Union-find's root.
fn root(up: &mut [usize], mut i: usize) -> usize {
    while up[i] != i {
        up[i] = up[up[i]];
        i = up[i];
    }
    i
}

/// The devices among the inside's parts (bus frame; `eye` the driver's).
pub(crate) fn find(parts: &[Part], eye: Vec3) -> Vec<Panel> {
    let pages: Vec<(usize, Frame, [f32; 4])> = parts.iter().enumerate().filter_map(|(i, p)| page(p, eye).map(|(f, r)| (i, f, r))).collect();
    // pages at one place switched by one variable: a stack of pages, when it has two values
    let n = pages.len();
    let mut up: Vec<usize> = (0..n).collect();
    for a in 0..n {
        for b in a + 1..n {
            let (pa, pb) = (&parts[pages[a].0], &parts[pages[b].0]);
            let same_var = matches!((&pa.switch, &pb.switch), (Some(x), Some(y)) if x.0.eq_ignore_ascii_case(&y.0));
            if same_var && same_place(&pages[a].1, pages[a].2, pb, &pages[b].1) {
                let (ra, rb) = (root(&mut up, a), root(&mut up, b));
                up[ra] = rb;
            }
        }
    }
    // (the values each stack's variable takes)
    let mut values: std::collections::HashMap<usize, Vec<i64>> = Default::default();
    for a in 0..n {
        let r = root(&mut up, a);
        let list = values.entry(r).or_default();
        if let Some(s) = parts[pages[a].0].switch.as_ref() {
            if !list.contains(&(s.1.round() as i64)) {
                list.push(s.1.round() as i64);
            }
        }
    }
    // (a stack of pages, or one mesh named a screen; that one needs something on it, see
    // below)
    let seed: Vec<bool> = (0..n).map(|a| values.get(&root(&mut up, a)).is_some_and(|v| v.len() >= 2) || parts[pages[a].0].named_screen()).collect();
    // stacks at one place are one device (an ALMEX switches a few pages by a variable of
    // their own); a page of no stack that lies over one is its too
    for a in 0..n {
        for b in 0..n {
            if a == b || !seed[a] || (seed[b] && b < a) {
                continue;
            }
            let pb = &parts[pages[b].0];
            // (a key that is switched is no page of it)
            if !seed[b] && pb.event.is_some() {
                continue;
            }
            if same_place(&pages[a].1, pages[a].2, pb, &pages[b].1) {
                let (ra, rb) = (root(&mut up, a), root(&mut up, b));
                up[rb] = ra;
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut of_root: std::collections::HashMap<usize, usize> = Default::default();
    let seeded: std::collections::HashSet<usize> = (0..n).filter(|&b| seed[b]).map(|b| root(&mut up, b)).collect();
    for a in 0..n {
        let r = root(&mut up, a);
        if !seeded.contains(&r) {
            continue;
        }
        let g = *of_root.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(a);
    }
    // (the most values one variable of the device takes: a menu has many pages, a lamp's
    // pictures are two - on and off)
    let stacked = |g: &[usize]| {
        let mut values: Vec<(String, i64)> = g.iter().filter_map(|&a| parts[pages[a].0].switch.as_ref().map(|s| (s.0.to_ascii_lowercase(), s.1.round() as i64))).collect();
        values.sort();
        values.dedup();
        values.iter().map(|v| values.iter().filter(|x| x.0 == v.0).count()).max().unwrap_or(0)
    };
    let mut out: Vec<Panel> = Vec::new();
    for g in groups {
        // the frame of the largest page, the display the pages' bounds in it
        let mut g = g;
        g.sort_by(|&a, &b| rect_area(pages[b].2).total_cmp(&rect_area(pages[a].2)).then(a.cmp(&b)));
        let f = pages[g[0]].1;
        let mut rect = pages[g[0]].2;
        for &a in &g[1..] {
            if let Some((r, _, _)) = f.rect(parts[pages[a].0].points()) {
                rect = union(rect, r);
            }
        }
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        if w > 2.0 * BIG || h > 2.0 * BIG {
            continue;
        }
        let plane = Plane { origin: f.origin + f.right * rect[0] + f.up * rect[3], du: f.right * w, dv: -f.up * h, normal: f.normal };
        let on = |p: &Part, off: f32| -> bool {
            let c = f.local(p.centre());
            c.x > rect[0] - EDGE && c.x < rect[2] + EDGE && c.y > rect[1] - EDGE && c.y < rect[3] + EDGE && c.z.abs() <= off
        };
        let page_meshes: Vec<usize> = g.iter().map(|&a| pages[a].0).collect();
        let faces: Vec<usize> = (0..parts.len()).filter(|&i| parts[i].face && !page_meshes.contains(&i) && f.rect(parts[i].points()).is_some_and(|(_, lo, hi)| lo >= -FACE_OFF && hi <= FACE_OFF) && on(&parts[i], FACE_OFF)).collect();
        let small = |p: &Part| p.radius <= KEY_MAX * 1.5 || (p.radius <= BIG && f.rect(p.points()).is_some_and(|(r, _, _)| overlap(r, rect) >= 0.9 * rect_area(r)));
        let fields: Vec<usize> = (0..parts.len()).filter(|&i| parts[i].event.is_some() && small(&parts[i]) && on(&parts[i], FIELD_OFF)).collect();
        // something written on it or touch fields on it, or a menu of pages of its own
        if faces.is_empty() && fields.len() < 2 && (stacked(&g) < MENU || w < 2.0 * PAGE_MIN || h < PAGE_MIN) {
            continue;
        }
        // the keys round it (an IBIS's under its display)
        let candidates: Vec<Candidate> = parts
            .iter()
            .enumerate()
            .filter(|(i, p)| p.event.is_some() && !fields.contains(i) && p.radius <= BIG)
            .filter_map(|(_, p)| {
                let (lo, hi) = p.points().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), q| (lo.min(q), hi.max(q)));
                if lo.x > hi.x {
                    return None;
                }
                let corners = std::array::from_fn(|k| Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z }));
                Some(Candidate { mesh: p.mesh, event: p.event.clone().unwrap_or_default(), corners, radius: p.radius })
            })
            .collect();
        let keys = device_keys(w, h, keys_beside(&plane, [0.0, 0.0, 1.0, 1.0], &candidates));
        let mut words: Vec<&str> = Vec::new();
        for &i in page_meshes.iter().chain(&faces) {
            words.push(&parts[i].name);
        }
        for &i in &fields {
            words.extend(parts[i].event.as_deref());
        }
        let page_names: Vec<&str> = page_meshes.iter().map(|&i| parts[i].name.as_str()).collect();
        let name = device_name(&words).or_else(|| shared_name(&page_names)).unwrap_or_else(|| super::screens::screen_name(&parts[page_meshes[0]].name));
        out.push(Panel { pages: g.iter().map(|&a| pages[a].0).collect(), faces, fields, keys, plane, name });
    }
    // (two devices at one place: the one with more on it)
    out.sort_by(|a, b| (b.faces.len() + b.fields.len()).cmp(&(a.faces.len() + a.fields.len())));
    let mut kept: Vec<Panel> = Vec::new();
    for p in out {
        let c = p.plane.at(0.5, 0.5);
        if !kept.iter().any(|k| k.holds(c, SAME_PLANE)) {
            kept.push(p);
        }
    }
    kept
}

/// Words of mesh names that say nothing of the device.
const PLAIN_WORDS: [&str; 24] = [
    "screen", "display", "bildschirm", "anzeige", "monitor", "click", "button", "taste", "knopf", "mesh", "text", "menu", "page", "seite", "ein", "aus", "off", "oben", "unten", "links", "rechts", "bild", "neu", "new",
];

/// A device's name from its meshes' names and events: the word most of them share (`almex`
/// in `17_almex_screen_0`, `17_almex_s_uhrzeit` and `almex_clickU1` is "ALMEX").
pub(crate) fn device_name(names: &[&str]) -> Option<String> {
    let mut count: Vec<(String, usize)> = Vec::new();
    for n in names {
        let mut seen: Vec<String> = Vec::new();
        for w in n.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| w.len() >= 3) {
            let w = w.to_ascii_lowercase();
            if PLAIN_WORDS.contains(&w.as_str()) || seen.contains(&w) {
                continue;
            }
            seen.push(w.clone());
            match count.iter_mut().find(|c| c.0 == w) {
                Some(c) => c.1 += 1,
                None => count.push((w, 1)),
            }
        }
    }
    let best = count.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.len().cmp(&a.0.len())))?;
    if best.1 * 2 < names.len() {
        return None;
    }
    let w = &best.0;
    Some(if w.len() <= 5 { w.to_ascii_uppercase() } else { w[..1].to_ascii_uppercase() + &w[1..] })
}

/// A device's name from what its pages' names begin with: `21_fd_anzeige_1` .. `_8` is "FD
/// anzeige" (the number in front, the page's number after it left out).
pub(crate) fn shared_name(names: &[&str]) -> Option<String> {
    let first = names.first()?.to_ascii_lowercase();
    let mut n = first.len();
    for other in &names[1..] {
        let other = other.to_ascii_lowercase();
        n = n.min(first.bytes().zip(other.bytes()).take_while(|(a, b)| a == b).count());
    }
    // (cut back to a whole word when one of the names goes on within the last word)
    let mut prefix = &first[..n];
    let inside_word = |name: &str| name.as_bytes().get(n).is_some_and(|b| b.is_ascii_alphanumeric());
    if n > 0 && first.as_bytes()[n - 1].is_ascii_alphanumeric() && names.iter().any(|m| inside_word(&m.to_ascii_lowercase())) {
        prefix = prefix.rfind(|c: char| !c.is_ascii_alphanumeric()).map_or("", |k| &prefix[..k]);
    }
    let words: Vec<&str> = prefix.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty() && !w.bytes().all(|b| b.is_ascii_digit())).collect();
    if words.is_empty() {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for (k, w) in words.iter().enumerate() {
        out.push(if w.len() <= 3 { w.to_ascii_uppercase() } else if k == 0 { w[..1].to_ascii_uppercase() + &w[1..] } else { w.to_string() });
    }
    Some(out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    /// A flat rectangle standing upright at `y`, facing -y (a driver at y < `y`): left `x`,
    /// bottom `z`, `w` x `h` m, its texture upright on it.
    fn quad(x: f32, z: f32, w: f32, h: f32, y: f32) -> Vec<Tri> {
        let n = Vec3::new(0.0, -1.0, 0.0);
        let p = |a: f32, b: f32| Vec3::new(a, y, b);
        let (tl, tr, bl, br) = (p(x, z + h), p(x + w, z + h), p(x, z), p(x + w, z));
        let (a, b, c, d) = (Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0));
        vec![Tri { p: [tl, tr, bl], n: [n; 3], uv: [a, b, c] }, Tri { p: [tr, br, bl], n: [n; 3], uv: [b, d, c] }]
    }

    fn part(mesh: usize, name: &str, tris: Vec<Tri>) -> Part {
        let pts: Vec<Vec3> = tris.iter().flat_map(|t| t.p).collect();
        let (lo, hi) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        Part { mesh, name: name.into(), switch: None, face: false, event: None, tris, radius: (hi - lo).length() * 0.5 }
    }

    /// An ALMEX at y = 4: a 0.2 x 0.12 m display (x 0..0.2, z 1.6..1.72) of three pages switched
    /// by `almex_menu`, a page of its own variable over part of it, two text textures and four
    /// touch fields on it (a row of buttons along its bottom), a key beside it; and nearby the
    /// door (a big switch on the same plane), a lamp of the cab and the clock on another wall.
    fn almex() -> Vec<Part> {
        let mut parts = Vec::new();
        for (k, v) in [-1.0f32, 0.0, 1.0].into_iter().enumerate() {
            let mut p = part(k, &format!("17_almex_screen_{v}"), quad(0.0, 1.6, 0.2, 0.12, 4.0));
            p.switch = Some(("almex_menu".into(), v));
            parts.push(p);
        }
        // (a page of its own variable over the upper part)
        let mut p = part(3, "17_almex_screen_9_tt", quad(0.0, 1.66, 0.2, 0.06, 4.0));
        p.switch = Some(("almex_menu_9".into(), 1.0));
        parts.push(p);
        for (k, (x, z)) in [(0.15f32, 1.70f32), (0.02, 1.65)].into_iter().enumerate() {
            let mut p = part(4 + k, &format!("17_almex_s_text{k}"), quad(x, z, 0.04, 0.012, 3.998));
            p.face = true;
            p.switch = Some(("almex_ein".into(), 1.0));
            parts.push(p);
        }
        for k in 0..4 {
            let mut p = part(6 + k, &format!("17_almex_clickU{}", k + 1), quad(0.01 + k as f32 * 0.05, 1.605, 0.04, 0.02, 3.995));
            p.event = Some(format!("almex_clickU{}", k + 1));
            p.switch = Some(("almex_ein".into(), 1.0));
            parts.push(p);
        }
        // a hard key just right of the display
        let mut key = part(10, "17_almex_key", quad(0.215, 1.62, 0.02, 0.02, 3.99));
        key.event = Some("almex_papier".into());
        parts.push(key);
        // the cab door: a big switch on the same plane; a lamp of the cab switched by a
        // variable of two values but far away; the clock: a text texture on another wall
        let mut door = part(11, "21_fahrertuer", quad(-0.5, 0.8, 1.2, 1.4, 3.99));
        door.event = Some("door_driver".into());
        parts.push(door);
        for (k, v) in [0.0f32, 1.0].into_iter().enumerate() {
            let mut p = part(12 + k, "17_kontrollleuchte", quad(1.0, 2.0, 0.1, 0.1, 4.0));
            p.switch = Some(("licht".into(), v));
            parts.push(p);
        }
        let mut clock = part(14, "17_uhr", quad(0.05, 1.65, 0.05, 0.02, 3.5));
        clock.face = true;
        parts.push(clock);
        parts
    }

    #[test]
    fn a_stack_of_pages_with_its_texts_and_touch_fields_is_one_device() {
        let parts = almex();
        let eye = Vec3::new(-0.3, 3.3, 1.9);
        let found = find(&parts, eye);
        assert_eq!(found.len(), 1, "the ALMEX, not the lamp's two pictures: {found:#?}");
        let d = &found[0];
        assert_eq!(d.name, "ALMEX");
        let mut pages: Vec<usize> = d.pages.iter().map(|&i| parts[i].mesh).collect();
        pages.sort();
        assert_eq!(pages, vec![0, 1, 2, 3], "the stack and the page over it");
        assert_eq!(d.faces, vec![4, 5], "the two texts, not the clock on the other wall");
        assert_eq!(d.fields, vec![6, 7, 8, 9], "the four touch fields, not the door");
        let events: Vec<&str> = d.keys.iter().map(|k| k.event.as_str()).collect();
        assert_eq!(events, vec!["almex_papier"], "the key beside it, not the door");
        // the display: the pages' bounds, upright, facing the driver
        let (w, h) = d.size();
        assert!((w - 0.2).abs() < 1e-4 && (h - 0.12).abs() < 1e-4, "{w} x {h}");
        assert!(d.plane.origin.distance(Vec3::new(0.0, 4.0, 1.72)) < 1e-4, "top left: {:?}", d.plane.origin);
        assert!(d.plane.normal.distance(-Vec3::Y) < 1e-4);
        assert!(d.plane.du.normalize().distance(Vec3::X) < 1e-4 && d.plane.dv.normalize().distance(-Vec3::Z) < 1e-4);
        assert!(d.holds(Vec3::new(0.1, 3.99, 1.65), 0.03) && !d.holds(Vec3::new(0.3, 4.0, 1.65), 0.03));
        // a lamp of three pictures is a menu of its own (a display of the dashboard)
        let mut parts = parts;
        let mut third = part(15, "17_kontrollleuchte", quad(1.0, 2.0, 0.1, 0.1, 4.0));
        third.switch = Some(("licht".into(), 2.0));
        parts.push(third);
        let found = find(&parts, eye);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].name, "Kontrollleuchte", "named after most of its meshes");
        assert_eq!(found[1].pages.len(), 3);
    }

    #[test]
    fn single_meshes_are_no_device_unless_named_a_screen_with_something_on_it() {
        let mut parts = almex();
        // the menu's other pages gone: one page of almex_menu left
        parts.retain(|p| p.mesh != 1 && p.mesh != 2 && p.mesh != 3 && p.mesh != 12 && p.mesh != 13);
        // still a device: it is named a screen and has texts and touch fields on it
        let found = find(&parts, Vec3::new(-0.3, 3.3, 1.9));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fields.len(), 4);
        // without them it is a lone picture
        parts.retain(|p| !p.face && p.event.is_none());
        assert!(find(&parts, Vec3::new(-0.3, 3.3, 1.9)).is_empty());
        // a mesh switched by a variable but not named a screen, alone: nothing
        let mut p = part(0, "17_sonnenblende", quad(0.0, 1.6, 0.2, 0.12, 4.0));
        p.switch = Some(("blende".into(), 1.0));
        assert!(find(&[p], Vec3::ZERO).is_empty());
    }

    #[test]
    fn a_page_facing_nowhere_faces_the_driver() {
        let mut tris = quad(0.0, 1.6, 0.2, 0.12, 4.0);
        for t in &mut tris {
            t.n = [Vec3::ZERO; 3];
        }
        let f = Frame::of(&tris, Vec3::new(0.0, 3.0, 1.7)).unwrap();
        assert!(f.normal.distance(-Vec3::Y) < 1e-4);
        let f = Frame::of(&tris, Vec3::new(0.0, 5.0, 1.7)).unwrap();
        assert!(f.normal.distance(Vec3::Y) < 1e-4);
    }

    #[test]
    fn a_device_is_named_by_the_word_its_meshes_share() {
        assert_eq!(device_name(&["17_almex_screen_0", "17_almex_s_uhrzeit", "almex_clickU1"]).as_deref(), Some("ALMEX"));
        assert_eq!(device_name(&["ibis_display", "IBIS_7", "IBIS_eingabe"]).as_deref(), Some("IBIS"));
        assert_eq!(device_name(&["pvs_fahrerdisplay_oben_1", "pvs_fahrerdisplay_oben_2"]).as_deref(), Some("PVS"));
        assert_eq!(device_name(&["19_fahrgastinfo_1", "19_fahrgastinfo_2"]).as_deref(), Some("Fahrgastinfo"));
        assert_eq!(device_name(&["display_1", "screen_2"]), None);
        // else what its pages' names begin with
        assert_eq!(shared_name(&["21_fd_anzeige_1", "21_fd_anzeige_2", "21_fd_anzeige_8"]).as_deref(), Some("FD anzeige"));
        assert_eq!(shared_name(&["21_fd_anzeige_r_1", "21_fd_anzeige_r_7"]).as_deref(), Some("FD anzeige R"));
        assert_eq!(shared_name(&["17_display", "17_display_an"]).as_deref(), Some("Display"));
        assert_eq!(shared_name(&["17_display_x", "17_monitor_y"]), None);
    }
}
