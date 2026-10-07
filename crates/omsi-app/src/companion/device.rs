//! A device of the bus as it really looks: a live picture taken by the game.
//!
//! Omsi-Hub could only build an IBIS or a ticket machine again from OMSI 2's files, and the
//! first tablet page did the same with the cab's clickable meshes: a ticket table came out as
//! a black box and a heap of overlapping keys. openOMSI draws the cab itself, so a device is
//! photographed instead and drawn into a texture of its own a few times a second, as a mirror
//! is (`Renderer::render_to_texture`), and read back without waiting for the GPU.
//!
//! The picture is a scan of the device's face ([`View::face`]): the camera looks straight at
//! it along its normal, the device's own up (its display's) is the picture's up, and the
//! picture is the device's face and nothing else - square, level, filling a tablet's or a
//! phone's screen. The camera stands far off with a narrow view, so that keys standing out of
//! the face are drawn where they are (as good as orthographic), and draws nothing nearer than
//! a little before the device: no wheel, door or pillar of the cab comes between.
//!
//! The same camera says where a tap goes: a tap on the picture is a ray from the camera
//! through that point, from just before the device, and the ray is clicked into the cab as the
//! mouse clicks (`Player::html_hit`, then `Player::click`) - whatever the picture shows there
//! is pressed.
//!
//! Everything here is in the bus's own frame (metres from its origin, unturned): the camera
//! moves with the bus, the picture stays still.

use glam::{DVec3, Mat4, Vec2, Vec3, Vec4Swizzles};

/// The picture's longer side (pixels).
pub(crate) const LONG_SIDE: u32 = 960;
/// Pictures a second at most while a device watches the picture.
pub(crate) const FPS: f64 = 5.0;
/// Room round the device in the picture (a share of its size on each side): a little of its
/// case shows too.
const MARGIN: f32 = 0.1;
/// The camera stands at least this far from the device's middle (m)...
const MIN_DIST: f32 = 0.22;
/// ...and this many times the device's radius, so that its depth does not stretch it; never
/// further than the driver's eye.
const DIST_PER_RADIUS: f32 = 2.6;
const NEAR: f32 = 0.015;
/// The narrowest and widest field of view (degrees, vertical).
const FOV_MIN: f32 = 3.0;
const FOV_MAX: f32 = 110.0;
/// The picture's widest and narrowest shape (width over height).
const ASPECT_MIN: f32 = 0.5;
const ASPECT_MAX: f32 = 2.4;
/// The longer side of the picture of a device seen straight on (pixels): a tablet's whole
/// screen.
pub(crate) const FACE_LONG_SIDE: u32 = 1920;
/// The camera straight before a device stands this many times the device's longer side away
/// (m, within these bounds): a narrow view, the device's depth hardly drawn smaller.
const FACE_DIST: f32 = 10.0;
const FACE_DIST_MIN: f32 = 1.0;
const FACE_DIST_MAX: f32 = 8.0;
/// It draws what lies this far before the device's face (a share of the device's longer side,
/// m within these bounds): its keys and its case, nothing of the cab before it.
const FACE_FRONT: f32 = 0.6;
const FACE_FRONT_MIN: f32 = 0.08;
const FACE_FRONT_MAX: f32 = 0.3;
/// How far off straight down (or up) the camera before a device lying flat looks (radians):
/// enough for the renderer's pitch to say which way is up.
const STRAIGHT_DOWN: f32 = 2e-3;

/// How a device is photographed: a camera in the bus's own frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub pos: Vec3,
    /// Where it looks, and the picture's up (at right angles to it).
    pub fwd: Vec3,
    pub up: Vec3,
    /// Vertical field of view (degrees), width over height, the near and the far plane (m).
    pub fov_deg: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// The picture (pixels).
    pub size: (u32, u32),
}

impl View {
    /// The camera for a device whose screen and keys have the corners `points`, seen from
    /// `eye` (the driver's), unless the screen faces away from it (`normal`, out of the screen):
    /// then from before the screen. `up` is the bus's up.
    pub(crate) fn frame(eye: Vec3, normal: Vec3, points: &[Vec3], up: Vec3) -> Option<View> {
        let (lo, hi) = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        if points.is_empty() || !lo.is_finite() || !hi.is_finite() {
            return None;
        }
        let centre = (lo + hi) * 0.5;
        let radius = points.iter().map(|p| p.distance(centre)).fold(0.0f32, f32::max).max(0.01);
        let to = centre - eye;
        let deye = to.length();
        let normal = normal.normalize_or_zero();
        // (the driver sees the screen's face: from the eye; else straight at the screen)
        let from_eye = deye > 0.05 && (normal == Vec3::ZERO || (-to / deye).dot(normal) > 0.2);
        let fwd = if from_eye { to / deye } else if normal != Vec3::ZERO { -normal } else { return None };
        let reach = if from_eye { deye } else { f32::MAX };
        let dist = (radius * DIST_PER_RADIUS).max(MIN_DIST).min(reach);
        let pos = centre - fwd * dist;
        let mut upp = (up - fwd * fwd.dot(up)).normalize_or_zero();
        if upp == Vec3::ZERO {
            upp = fwd.any_orthonormal_vector();
        }
        let right = fwd.cross(upp);
        let (mut tx, mut ty, mut zmax) = (0.0f32, 0.0f32, 0.0f32);
        for p in points {
            let d = *p - pos;
            let z = d.dot(fwd).max(NEAR * 2.0);
            tx = tx.max(d.dot(right).abs() / z);
            ty = ty.max(d.dot(upp).abs() / z);
            zmax = zmax.max(z);
        }
        let least = (FOV_MIN * 0.5).to_radians().tan();
        let (tx, ty) = ((tx * (1.0 + 2.0 * MARGIN)).max(least), (ty * (1.0 + 2.0 * MARGIN)).max(least));
        let aspect = (tx / ty).clamp(ASPECT_MIN, ASPECT_MAX);
        let half = ty.max(tx / aspect);
        let fov_deg = (2.0 * half.atan()).to_degrees().clamp(FOV_MIN, FOV_MAX);
        let even = |x: f32| ((x / 2.0).round() as u32 * 2).max(16);
        let size = if aspect >= 1.0 { (LONG_SIDE, even(LONG_SIDE as f32 / aspect)) } else { (even(LONG_SIDE as f32 * aspect), LONG_SIDE) };
        Some(View { pos, fwd, up: upp, fov_deg, aspect, near: NEAR, far: zmax + radius + 1.0, size })
    }

    /// The camera straight before a device's face (bus frame): on the normal through the
    /// middle of the rectangle with the corner `top_left`, `across` its width and `down` its
    /// height, looking at it, `down` the picture's down; the picture exactly that rectangle,
    /// at a tablet's size. A flat thing seen square on is not drawn smaller at its far side:
    /// the picture is the face as it is, and a point of it is where the face has it.
    pub(crate) fn face(top_left: Vec3, across: Vec3, down: Vec3, normal: Vec3) -> Option<View> {
        let (w, h) = (across.length(), down.length());
        let normal = normal.normalize_or_zero();
        if w < 1e-3 || h < 1e-3 || normal == Vec3::ZERO || !top_left.is_finite() || !across.is_finite() || !down.is_finite() {
            return None;
        }
        let centre = top_left + (across + down) * 0.5;
        let long = w.max(h);
        let dist = (long * FACE_DIST).clamp(FACE_DIST_MIN, FACE_DIST_MAX);
        let front = (long * FACE_FRONT).clamp(FACE_FRONT_MIN, FACE_FRONT_MAX);
        let mut fwd = -normal;
        // (a device lying flat: the renderer's camera, which has a heading and a pitch, cannot
        // look exactly straight down - a hair off towards the picture's bottom, aimed at the
        // device's middle all the same)
        let level = Vec3::new(fwd.x, fwd.y, 0.0);
        if level.length() < STRAIGHT_DOWN {
            let towards = Vec3::new(down.x, down.y, 0.0).normalize_or(Vec3::X);
            fwd = (Vec3::new(0.0, 0.0, fwd.z.signum()) * (1.0 - STRAIGHT_DOWN * STRAIGHT_DOWN).sqrt() + towards * STRAIGHT_DOWN).normalize();
        }
        let up = (-down / h - fwd * fwd.dot(-down / h)).normalize_or_zero();
        if up == Vec3::ZERO {
            return None;
        }
        let aspect = w / h;
        let fov_deg = (2.0 * (h * 0.5 / dist).atan()).to_degrees();
        let even = |x: f32| ((x / 2.0).round() as u32 * 2).max(16);
        let size = if aspect >= 1.0 { (FACE_LONG_SIDE, even(FACE_LONG_SIDE as f32 / aspect)) } else { (even(FACE_LONG_SIDE as f32 * aspect), FACE_LONG_SIDE) };
        Some(View { pos: centre - fwd * dist, fwd, up, fov_deg, aspect, near: dist - front, far: dist + front.max(0.5), size })
    }

    /// The same camera moved with the device by `m` (a device on a door that swings).
    pub(crate) fn moved(&self, m: Mat4) -> View {
        View { pos: m.transform_point3(self.pos), fwd: m.transform_vector3(self.fwd).normalize_or(self.fwd), up: m.transform_vector3(self.up).normalize_or(self.up), ..*self }
    }

    /// The camera's view and projection as the renderer makes them (reversed depth).
    fn view_proj(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_deg.to_radians(), self.aspect, self.far, self.near) * Mat4::look_to_rh(self.pos, self.fwd, self.up)
    }

    /// Where point `p` (bus frame) is in the picture: 0..1 across from the left and down from
    /// the top. None behind the camera.
    pub(crate) fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj() * p.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let ndc = c.xy() / c.w;
        Some(Vec2::new((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5))
    }

    /// The ray through point (`x`, `y`) of the picture (0..1 as [`View::project`] has them):
    /// from where the picture begins (the near plane: what lies nearer the camera is not in
    /// it), its direction (bus frame).
    pub(crate) fn ray(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj().inverse();
        // (depth 0 is the far plane with reversed depth: the longest baseline)
        let far = inv.project_point3(Vec3::new(2.0 * x - 1.0, 1.0 - 2.0 * y, 0.0));
        let dir = (far - self.pos).normalize_or(self.fwd);
        (self.pos + dir * (self.near / dir.dot(self.fwd).max(1e-3)), dir)
    }

    /// The part of the picture a box with the corners `corners` covers (x0, y0, x1, y1 within
    /// 0..1); None when it lies wholly outside or behind.
    pub(crate) fn rect_of(&self, corners: &[Vec3]) -> Option<[f32; 4]> {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in corners {
            let q = self.project(*p)?;
            r = [r[0].min(q.x), r[1].min(q.y), r[2].max(q.x), r[3].max(q.y)];
        }
        let r = [r[0].clamp(0.0, 1.0), r[1].clamp(0.0, 1.0), r[2].clamp(0.0, 1.0), r[3].clamp(0.0, 1.0)];
        (r[2] - r[0] > 1e-4 && r[3] - r[1] > 1e-4).then_some(r)
    }

    /// The half-angle (radians) of the rings of rays a tap tries round its own when it hits
    /// no switch (`Player::pick`): what four of the picture's pixels subtend.
    pub(crate) fn spread(&self) -> f32 {
        self.fov_deg.to_radians() / self.size.1.max(1) as f32 * 4.0
    }

    /// The renderer's camera for the bus turned by `rot` (`body_rotation`) at `origin`.
    pub(crate) fn camera(&self, rot: Mat4, origin: DVec3) -> omsi_render::Camera {
        let f = rot.transform_vector3(self.fwd).normalize_or(Vec3::Y);
        let u = rot.transform_vector3(self.up).normalize_or(Vec3::Z);
        let yaw = f.x.atan2(f.y).to_degrees();
        let pitch = f.z.clamp(-1.0, 1.0).asin().to_degrees();
        // (Camera::up turns the level up about the view by `roll`: the bus's lean)
        let r0 = Vec3::new(f.y, -f.x, 0.0).normalize_or_zero();
        let roll = if r0 == Vec3::ZERO { 0.0 } else { u.dot(r0).atan2(u.dot(r0.cross(f))).to_degrees() };
        omsi_render::Camera { position: origin + rot.transform_point3(self.pos).as_dvec3(), yaw, pitch, roll, fov_deg: self.fov_deg, near: self.near, far: self.far }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An IBIS 0.9 m before the driver's eye and 0.4 m lower, facing the driver: its display
    /// and a row of keys under it.
    fn ibis() -> (Vec3, Vec3, Vec<Vec3>) {
        let eye = Vec3::new(-0.5, 4.0, 1.8);
        let normal = Vec3::new(0.0, -1.0, 0.3).normalize();
        let mut pts = Vec::new();
        for x in [-0.6f32, -0.4] {
            for z in [1.35f32, 1.45] {
                pts.push(Vec3::new(x, 4.9, z));
            }
        }
        (eye, normal, pts)
    }

    #[test]
    fn a_device_fills_the_picture_with_a_margin_round_it() {
        let (eye, normal, pts) = ibis();
        let v = View::frame(eye, normal, &pts, Vec3::Z).unwrap();
        // between the eye and the device, looking at it
        assert!(v.pos.distance(eye) < eye.distance(Vec3::new(-0.5, 4.9, 1.4)));
        let rs: Vec<Vec2> = pts.iter().map(|p| v.project(*p).unwrap()).collect();
        for r in &rs {
            assert!(r.x > 0.0 && r.x < 1.0 && r.y > 0.0 && r.y < 1.0, "{r:?} outside the picture");
        }
        // touching the margin on the long side
        let (x0, x1) = rs.iter().fold((1.0f32, 0.0f32), |a, r| (a.0.min(r.x), a.1.max(r.x)));
        let (y0, y1) = rs.iter().fold((1.0f32, 0.0f32), |a, r| (a.0.min(r.y), a.1.max(r.y)));
        assert!(x1 - x0 > 0.75 || y1 - y0 > 0.75, "the device is small in its picture: {x0}..{x1} x {y0}..{y1}");
        // the picture's shape follows the device's (wider than tall), its size even
        assert!(v.aspect > 1.2, "{}", v.aspect);
        assert_eq!(v.size.0, LONG_SIDE);
        assert_eq!(v.size.1 % 2, 0);
        // the bus's up is the picture's up: the higher row is higher in the picture
        let top = v.project(Vec3::new(-0.5, 4.9, 1.45)).unwrap();
        let bottom = v.project(Vec3::new(-0.5, 4.9, 1.35)).unwrap();
        assert!(top.y < bottom.y);
    }

    #[test]
    fn a_tap_on_the_picture_is_a_ray_through_that_point() {
        let (eye, normal, pts) = ibis();
        let v = View::frame(eye, normal, &pts, Vec3::Z).unwrap();
        for p in [Vec3::new(-0.58, 4.9, 1.37), Vec3::new(-0.45, 4.9, 1.44), Vec3::new(-0.5, 4.9, 1.4)] {
            let q = v.project(p).unwrap();
            let (o, d) = v.ray(q.x, q.y);
            // the ray passes the point
            let along = (p - o).dot(d);
            assert!((o + d * along).distance(p) < 1e-4, "{p:?}");
        }
        assert!(v.spread() > 0.0 && v.spread() < 0.01);
    }

    /// An ALMEX's display: 0.2 x 0.12 m, upright at y = 4, facing the driver (-y), tilted back
    /// a little.
    fn display() -> (Vec3, Vec3, Vec3, Vec3) {
        let normal = Vec3::new(0.0, -1.0, 0.2).normalize();
        let down = Vec3::new(0.0, -0.2, -1.0).normalize() * 0.12;
        let across = Vec3::X * 0.2;
        (Vec3::new(0.0, 4.0, 1.72), across, down, normal)
    }

    #[test]
    fn a_display_seen_straight_on_fills_its_picture_exactly() {
        let (tl, across, down, normal) = display();
        let v = View::face(tl, across, down, normal).unwrap();
        // its corners are the picture's corners, its middle the picture's middle
        for (p, want) in [(tl, (0.0, 0.0)), (tl + across, (1.0, 0.0)), (tl + down, (0.0, 1.0)), (tl + across + down, (1.0, 1.0)), (tl + (across + down) * 0.5, (0.5, 0.5))] {
            let q = v.project(p).unwrap();
            assert!(q.distance(Vec2::new(want.0, want.1)) < 1e-4, "{p:?}: {q:?}");
        }
        // a tablet's size, of the display's shape
        assert_eq!(v.size.0, FACE_LONG_SIDE);
        assert!((v.size.0 as f32 / v.size.1 as f32 - 0.2 / 0.12).abs() < 0.01, "{:?}", v.size);
        assert!((v.aspect - 0.2 / 0.12).abs() < 1e-4);
        // before it, on the side it faces
        assert!((v.pos - (tl + (across + down) * 0.5)).dot(normal) > 0.1);
        assert!(View::face(tl, Vec3::ZERO, down, normal).is_none());
    }

    #[test]
    fn a_tap_on_a_displays_picture_goes_through_that_point_of_it() {
        let (tl, across, down, normal) = display();
        let v = View::face(tl, across, down, normal).unwrap();
        // a button 3/4 across and near the bottom: tapped at that place of the picture
        for (x, y) in [(0.75f32, 0.9f32), (0.1, 0.1), (0.5, 0.5)] {
            let at = tl + across * x + down * y;
            let (o, d) = v.ray(x, y);
            // the ray meets the display's plane at the button
            let t = (at - o).dot(normal) / d.dot(normal);
            assert!((o + d * t).distance(at) < 1e-4, "({x}, {y})");
        }
        // the device swings with its door: the camera goes with it and the tap still lands
        let m = Mat4::from_translation(Vec3::new(0.3, -0.1, 0.0)) * Mat4::from_rotation_z(0.6);
        let moved = v.moved(m);
        let at = m.transform_point3(tl + across * 0.75 + down * 0.9);
        let q = moved.project(at).unwrap();
        assert!(q.distance(Vec2::new(0.75, 0.9)) < 1e-4, "{q:?}");
    }

    #[test]
    fn a_screen_facing_away_from_the_driver_is_photographed_from_before_it() {
        let (eye, _, pts) = ibis();
        // (facing forward, away from the driver)
        let v = View::frame(eye, Vec3::Y, &pts, Vec3::Z).unwrap();
        assert!(v.fwd.distance(-Vec3::Y) < 1e-5);
        assert!(v.pos.y > 4.9);
        assert!(View::frame(eye, Vec3::Y, &[], Vec3::Z).is_none());
    }

    /// A real bus of the installed OMSI 2 (`OMSI_ROOT`, the bus `OMSI_DEVICE_BUS` - the MAN SL
    /// with its IBIS and its ticket table unless said otherwise; skipped without them): every
    /// device of its cab photographed headless the way the game does it (a render texture,
    /// read back), no picture empty, written as PNGs to `OMSI_DEVICE_SHOTS` when that is set;
    /// and a tap on the middle of a key in its picture is a ray that reaches that key - the
    /// cab's own pick (`player::pick_in`) finds it.
    #[test]
    fn a_real_buses_devices_are_photographed_and_their_keys_hit() {
        let Some(root) = omsi_cfg::env::var_os("OMSI_ROOT").map(std::path::PathBuf::from) else {
            eprintln!("skipped: no OMSI_ROOT");
            return;
        };
        let bus = root.join(std::env::var("OMSI_DEVICE_BUS").unwrap_or_else(|_| "Vehicles/MAN_SL_SG/MAN_SL_standard.bus".into()));
        let map = root.join("maps/Grundorf/global.cfg");
        if !bus.is_file() || !map.is_file() {
            eprintln!("skipped: no {} or {}", bus.display(), map.display());
            return;
        }
        let clock = omsi_sim::SimClock { time: 6.0 * 3600.0 + 57.0 * 60.0 + 7.0, ..Default::default() };
        let vt = std::sync::Arc::new(omsi_sim::VehicleType::load(&root, &bus).expect("the bus"));
        let mut vehicle = omsi_sim::VehicleInstance::new(vt.clone(), omsi_sim::VehicleHost::new(clock.clone()));
        let instance = crate::graphics_instance();
        let mut renderer = pollster::block_on(omsi_render::Renderer::new_with(&instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb), omsi_render::RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() })).expect("a headless renderer");
        let mut scene = renderer.new_scene();
        let world = crate::scene::World::open(&root, &map, clock.date_code()).expect("the map");
        let mut render = world.add_vehicle(&renderer, &mut scene, &vt, None);
        vehicle.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
        for _ in 0..10 {
            vehicle.update(1.0 / 30.0);
        }
        crate::player::sync_vehicle_transforms(&renderer, &mut scene, &mut vehicle, &mut render, &mut [], true);
        let lighting = crate::lights::lighting_from(&omsi_sim::Daylight::compute(&clock, None), 50000.0);
        let screens = crate::companion::screens::discover(&vehicle, &root);
        let shots = std::env::var_os("OMSI_DEVICE_SHOTS").map(std::path::PathBuf::from);
        let (mut keys, mut hit) = (0, 0);
        let mut photographed = 0;
        for s in &screens {
            let Some(view) = s.view else { continue };
            let rot = vehicle.body_rotation();
            let tex = renderer.add_readable_render_texture(&mut scene, view.size.0, view.size.1);
            renderer.render_to_texture(&mut scene, tex, &view.camera(rot, vehicle.position), &lighting, view.aspect);
            let rb = renderer.start_readback(&scene, tex).expect("a readable texture");
            let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
            let (w, h, px) = rb.take().flatten().expect("the picture came back");
            assert_eq!((w, h), view.size);
            // not one colour all over: the device is in it
            let lum: Vec<f32> = px.chunks_exact(4).map(|p| p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11).collect();
            let mean = lum.iter().sum::<f32>() / lum.len() as f32;
            let spread = (lum.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / lum.len() as f32).sqrt();
            eprintln!("{} ({}): {}x{} picture, {} keys, brightness {mean:.0} ± {spread:.0}", s.name, s.id, w, h, s.keys.len());            // (a device this bus does not have - its meshes hidden by the bus's options - is a
            // picture of the wall where it would be)
            let fitted = s.meshes.iter().any(|m| vehicle.mesh_props.get(m.0).is_some_and(|p| p.visible));
            assert!(spread > 4.0 || !fitted, "{}: the picture is all one colour", s.name);
            if let Some(dir) = shots.as_ref() {
                let _ = std::fs::create_dir_all(dir);
                image::save_buffer(dir.join(format!("{}.png", s.id)), &px, w, h, image::ColorType::Rgba8).expect("the picture written");
            }
            photographed += 1;
            // the middle of each key in the picture: the ray through it reaches that key
            for (k, spot) in s.keys.iter().zip(&s.key_spots).filter(|(_, r)| r[2] > r[0]) {
                let (o, d) = view.ray((spot[0] + spot[2]) * 0.5, (spot[1] + spot[3]) * 0.5);
                let origin = vehicle.position + rot.transform_point3(o).as_dvec3();
                keys += 1;
                if crate::player::pick_in(&vehicle, origin, rot.transform_vector3(d), view.spread()) == Some(k.mesh) {
                    hit += 1;
                }
            }
        }
        eprintln!("{photographed} device(s) photographed; {hit} of {keys} keys hit at the middle of their picture");
        assert!(photographed > 0, "no device in {}", bus.display());
        assert!(hit * 10 >= keys * 8, "only {hit} of {keys} keys are hit where the picture shows them");
    }

    /// A real bus with a device of pages (`OMSI_PANEL_BUS` of the installed OMSI 2 at
    /// `OMSI_ROOT` - the Hamburg electric bus with its ALMEX unless said otherwise; skipped
    /// without it), switched on as its script does it (`OMSI_PANEL_SET`, `name=value,...`):
    /// the device is found as one, named after its meshes, its display photographed straight
    /// on at a tablet's size (written to `OMSI_DEVICE_SHOTS` when that is set), and a tap at
    /// the middle of each touch field in the picture reaches that field.
    #[test]
    fn a_real_buses_device_of_pages_is_photographed_straight_on() {
        let Some(root) = omsi_cfg::env::var_os("OMSI_ROOT").map(std::path::PathBuf::from) else {
            eprintln!("skipped: no OMSI_ROOT");
            return;
        };
        let bus = root.join(std::env::var("OMSI_PANEL_BUS").unwrap_or_else(|_| "Vehicles/HH20_EBus2021/HHEBus2021_main.bus".into()));
        let map = root.join("maps/Grundorf/global.cfg");
        if !bus.is_file() || !map.is_file() {
            eprintln!("skipped: no {} or {}", bus.display(), map.display());
            return;
        }
        // (the ALMEX on its battery, as after the driver left it: its script keeps it going and
        // writes its clock)
        let set = std::env::var("OMSI_PANEL_SET").unwrap_or_else(|_| "almex_ein=1,almex_standby=1,almex_standby_zeit=1000000,almex_menu=0,almex_menu_req=0".into());
        let want = std::env::var("OMSI_PANEL_NAME").unwrap_or_else(|_| "ALMEX".into());
        let clock = omsi_sim::SimClock { time: 6.0 * 3600.0 + 57.0 * 60.0 + 7.0, ..Default::default() };
        let vt = std::sync::Arc::new(omsi_sim::VehicleType::load(&root, &bus).expect("the bus"));
        let mut vehicle = omsi_sim::VehicleInstance::new(vt.clone(), omsi_sim::VehicleHost::new(clock.clone()));
        let instance = crate::graphics_instance();
        let mut renderer = pollster::block_on(omsi_render::Renderer::new_with(&instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb), omsi_render::RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() })).expect("a headless renderer");
        let mut scene = renderer.new_scene();
        let world = crate::scene::World::open(&root, &map, clock.date_code()).expect("the map");
        let mut render = world.add_vehicle(&renderer, &mut scene, &vt, None);
        vehicle.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
        let set_all = |vehicle: &mut omsi_sim::VehicleInstance| {
            for kv in set.split(',').filter(|s| !s.trim().is_empty()) {
                let (k, v) = kv.split_once('=').expect("name=value");
                vehicle.set_var(k.trim(), v.trim().parse().expect("a number"));
            }
        };
        for k in 0..20 {
            if k == 10 {
                set_all(&mut vehicle);
            }
            vehicle.update(1.0 / 30.0);
        }
        set_all(&mut vehicle);
        let props = omsi_sim::vehicle::compute_mesh_props(&vehicle.ty, &|n| vehicle.var(n));
        vehicle.mesh_props = props;
        crate::player::sync_vehicle_transforms(&renderer, &mut scene, &mut vehicle, &mut render, &mut [], true);
        let mut budget = usize::MAX;
        crate::scene::sync_vehicle_textures(&renderer, &mut scene, &mut vehicle, &render, &mut budget);
        let looked = std::time::Instant::now();
        let screens = crate::companion::screens::discover(&vehicle, &root);
        eprintln!("{} screen(s) found in {:?}", screens.len(), looked.elapsed());
        for s in &screens {
            eprintln!("{} ({}): {:?}, {:.3} x {:.3} m, {} pages, {} fields, {} keys, picture {:?}", s.name, s.id, s.source, s.size[0], s.size[1], s.meshes.len(), s.fields.len(), s.keys.len(), s.view.map(|v| v.size));
        }
        let s = screens.iter().find(|s| s.source == crate::companion::screens::Source::Panel && s.name == want).unwrap_or_else(|| panic!("no device {want} in {}", bus.display()));
        assert!(s.fields.len() >= 4, "{}: {} touch fields", s.name, s.fields.len());
        let view = s.view_now(&vehicle).expect("a camera");
        assert_eq!(view.size.0.max(view.size.1), FACE_LONG_SIDE);
        let lighting = crate::lights::lighting_from(&omsi_sim::Daylight::compute(&clock, None), 50000.0);
        let rot = vehicle.body_rotation();
        let tex = renderer.add_readable_render_texture(&mut scene, view.size.0, view.size.1);
        let mut light = lighting.clone();
        light.shadows = false;
        light.min_obj_size = 0.0;
        light.ambient = light.ambient.max(Vec3::splat(0.35));
        renderer.render_to_texture(&mut scene, tex, &view.camera(rot, vehicle.position), &light, view.aspect);
        let rb = renderer.start_readback(&scene, tex).expect("a readable texture");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        let (w, h, px) = rb.take().flatten().expect("the picture came back");
        assert_eq!((w, h), view.size);
        let lum: Vec<f32> = px.chunks_exact(4).map(|p| p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11).collect();
        let mean = lum.iter().sum::<f32>() / lum.len() as f32;
        let spread = (lum.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / lum.len() as f32).sqrt();
        eprintln!("{}: {w}x{h} picture, brightness {mean:.0} ± {spread:.0}", s.name);
        assert!(spread > 4.0, "the picture is all one colour");
        if let Some(dir) = std::env::var_os("OMSI_DEVICE_SHOTS").map(std::path::PathBuf::from) {
            let _ = std::fs::create_dir_all(&dir);
            image::save_buffer(dir.join(format!("{}.png", s.id)), &px, w, h, image::ColorType::Rgba8).expect("the picture written");
        }
        // a tap at the middle of each touch field shown (in the picture) reaches it, or a field
        // of the same event over it
        let unturn = rot.inverse();
        let event = |i: usize| vehicle.ty.meshes.get(i).and_then(|m| vehicle.ty.model.meshes.get(m.def_index)).and_then(|d| d.mouse_event.clone());
        let (mut shown, mut hit) = (0, 0);
        for &f in &s.fields {
            if !vehicle.mesh_props.get(f).is_some_and(|p| p.visible) {
                continue;
            }
            let Some((c, _)) = crate::companion::screens::mesh_centre(&vehicle, f) else { continue };
            let Some(q) = view.project(unturn.transform_point3(c)) else { continue };
            if !(0.0..=1.0).contains(&q.x) || !(0.0..=1.0).contains(&q.y) {
                continue;
            }
            shown += 1;
            let (o, d) = view.ray(q.x, q.y);
            let got = crate::player::pick_in(&vehicle, vehicle.position + rot.transform_point3(o).as_dvec3(), rot.transform_vector3(d), view.spread());
            if got == Some(f) || got.is_some_and(|g| event(g) == event(f)) {
                hit += 1;
            } else {
                eprintln!("  {:?} at ({:.3}, {:.3}): the tap reached {:?}", event(f), q.x, q.y, got.and_then(event));
            }
        }
        eprintln!("{hit} of {shown} touch fields shown hit at their middle");
        assert!(shown > 0 && hit * 10 >= shown * 9, "only {hit} of {shown} touch fields are hit where the picture shows them");
    }

    /// A face scan is the picture the renderer draws, also of a ticket table lying flat (the
    /// camera looking straight down, where the renderer's camera has no up of its own), and a
    /// tap on it starts just before the device, past whatever of the cab stands between.
    #[test]
    fn a_face_scan_is_what_the_renderer_draws_even_looking_straight_down() {
        let table = (Vec3::new(0.2, 4.5, 1.0), Vec3::X * 0.3, -Vec3::Y * 0.15, Vec3::Z);
        for (tl, across, down, normal) in [table, display()] {
            let v = View::face(tl, across, down, normal).unwrap();
            let pts = [tl, tl + across, tl + down, tl + across + down, tl + (across + down) * 0.5];
            for (heading, pitch, bank) in [(0.0f32, 0.0f32, 0.0f32), (73.0, 2.0, -3.0), (-160.0, -4.0, 5.0)] {
                let rot = Mat4::from_quat(glam::Quat::from_rotation_z((-heading).to_radians()) * glam::Quat::from_rotation_x(pitch.to_radians()) * glam::Quat::from_rotation_y(bank.to_radians()));
                let origin = DVec3::new(500.0, 250.0, 30.0);
                let cam = v.camera(rot, origin);
                let vp = cam.view_proj(v.aspect, origin);
                for p in &pts {
                    let c = vp * rot.transform_point3(*p).extend(1.0);
                    let ndc = c.xy() / c.w;
                    let got = Vec2::new((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);
                    let want = v.project(*p).unwrap();
                    assert!(got.distance(want) < 2e-3, "heading {heading}, normal {normal:?}: {got:?} against {want:?}");
                }
            }
            // the picture's top is the face's top, square on: no turn, no keystone
            let (a, b) = (v.project(tl).unwrap(), v.project(tl + across).unwrap());
            assert!((a.y - b.y).abs() < 1e-3 && a.distance(Vec2::ZERO) < 1e-3, "{a:?} {b:?}");
            // a tap starts before the face, within what the picture draws
            let (o, _) = v.ray(0.5, 0.5);
            let before = (o - (tl + (across + down) * 0.5)).dot(normal);
            assert!(before > 0.05 && before < 0.35, "{before}");
        }
    }

    /// The picture the renderer draws for the camera is the picture the taps are mapped on:
    /// the renderer's own projection of the turned, moved camera puts a point where
    /// [`View::project`] does.
    #[test]
    fn the_renderers_camera_sees_what_the_view_maps() {
        let (eye, normal, pts) = ibis();
        let v = View::frame(eye, normal, &pts, Vec3::Z).unwrap();
        for (heading, pitch, bank) in [(0.0f32, 0.0f32, 0.0f32), (73.0, 2.0, -3.0), (-160.0, -4.0, 5.0)] {
            let rot = Mat4::from_quat(glam::Quat::from_rotation_z((-heading).to_radians()) * glam::Quat::from_rotation_x(pitch.to_radians()) * glam::Quat::from_rotation_y(bank.to_radians()));
            let origin = DVec3::new(1234.5, -987.25, 40.0);
            let cam = v.camera(rot, origin);
            let vp = cam.view_proj(v.aspect, origin);
            for p in &pts {
                let world = rot.transform_point3(*p);
                let c = vp * world.extend(1.0);
                let ndc = c.xy() / c.w;
                let want = v.project(*p).unwrap();
                let got = Vec2::new((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);
                assert!(got.distance(want) < 1e-3, "heading {heading}: {got:?} against {want:?}");
            }
        }
    }
}
