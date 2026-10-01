//! Getting up from the driver's seat (the `get_up` setting, Ctrl+Shift+G) and walking about
//! as the passengers do: W A S D walk (relative to where the camera looks), Shift runs,
//! Space jumps, the right mouse button looks round; the view is out of the walker's eyes
//! (F1), F4 lets the free camera go (the walker waits), F1 comes back. G by a bus's door takes a free
//! passenger seat in it (the player's own bus, a timetable bus, another player's), G again
//! gets up and out by the nearest door; G at the own bus's front door sits back at the
//! wheel. Other buses are never driven.
//!
//! The walker is an avatar of the people's animation (`Humans::avatar`): the gait with its
//! feet on the ground, turning, sitting down on a seat and getting up are the passengers'
//! own, eased as theirs are; the camera eases into each new view instead of cutting.

use crate::humans::{AvatarCmd, BusId, Humans};
use crate::App;
use glam::{DVec2, DVec3};
use omsi_sim::collision::Obb;
use winit::keyboard::KeyCode;

/// The local player's avatar key (other players' walkers are `REMOTE_KEY + id`).
pub(crate) const AVATAR_KEY: u32 = 0;
pub(crate) const REMOTE_KEY: u32 = 1_000_000;

/// Walking and running pace (m/s), how quickly the walker gets to it (1/s), jump speed.
const WALK: f64 = 1.45;
const RUN: f64 = 4.3;
const ACCEL: f64 = 7.0;
const JUMP: f64 = 4.0;
/// How near a door (m) G gets in.
const DOOR_REACH: f64 = 3.2;
/// The walker's radius against walls and vehicles (m).
const RADIUS: f64 = 0.28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FootCam {
    /// Out of the walker's eyes (F1).
    First,
    /// The free camera (F4): the walker stands where it was.
    Free,
}

pub(crate) struct OnFoot {
    /// The feet on the ground.
    pub pos: DVec3,
    pub heading: f64,
    pub vel: DVec2,
    /// Height of the feet over the ground and its rate (a jump).
    pub lift: f64,
    pub vz: f64,
    /// Sitting on this seat of this bus.
    pub seat: Option<(BusId, usize)>,
    /// Standing or walking inside this bus, at this point of its cabin (bus frame): the
    /// walker goes with the bus, on its floor, and in and out through its open doors.
    pub inside: Option<(BusId, glam::Vec3)>,
    pub cam: FootCam,
    /// Where the camera looks (degrees).
    pub yaw: f32,
    pub pitch: f32,
    /// The camera as it eases towards where it belongs.
    pub eye: Option<DVec3>,
    pub eye_yaw: f32,
    /// How far the eyes' glide out of the cab's camera was behind the walker's head when it
    /// ended: let go of over a third of a second, so that the camera does not jump the
    /// last bit onto the head (the bus's own placement of the eyes takes over there).
    pub lag: DVec3,
    /// Just got up from the driver's seat: seconds (1 → 0) the eyes take to leave the cab's
    /// camera, slowly at first, instead of jumping to where the walker's head is.
    pub settle: f32,
    /// The view the player had in the bus (back to it at the wheel).
    pub view_before: String,
    /// The figure (a human type index, taken modulo their number).
    pub kind: u64,
    /// Just sat down: the view turns to where the seat faces.
    pub face_seat: bool,
    /// Stepping through a door or out of the cab: walked over a moment, not jumped.
    pub transit: Option<Transit>,
    /// Arrived at the end of a walk that had somewhere to go (the wheel): done next frame.
    pub arrive: Option<Then>,
}

/// What a walk leads to once walked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Then {
    Nothing,
    /// the own bus's wheel
    Wheel,
    /// the wheel of the placed vehicle with this uid
    Placed(u64),
}

/// A step the walker takes by itself: from `from` to `to` (world, feet) over `dur` seconds,
/// then inside `after` (a bus and the cabin point) or on the ground.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Transit {
    pub from: DVec3,
    pub to: DVec3,
    pub t: f32,
    pub dur: f32,
    pub after: Option<(BusId, glam::Vec3)>,
    pub then: Then,
}

impl Transit {
    /// At walking pace, but never a jump nor a crawl.
    fn walk(from: DVec3, to: DVec3, after: Option<(BusId, glam::Vec3)>) -> Transit {
        let d = (to - from).length() as f32;
        Transit { from, to, t: 0.0, dur: (d / WALK as f32).clamp(0.35, 2.0), after, then: Then::Nothing }
    }

    /// A walk in to the driver's place, then `then` (getting in is walked, not jumped).
    fn walk_in(from: DVec3, to: DVec3, then: Then) -> Transit {
        let d = (to - from).length() as f32;
        Transit { from, to, t: 0.0, dur: (d / WALK as f32).clamp(0.6, 2.5), after: None, then }
    }
}

impl OnFoot {
    /// Whether the feet are on the ground.
    fn grounded(&self) -> bool {
        self.lift <= 1e-4 && self.vz <= 0.0
    }
}

fn wrap(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// How near a door (its outside point) the walker must be to be let out through it.
const DOOR_OUT_REACH: f64 = 3.2;

/// Push `p` (plan view) out of box `o` grown by `r`.
fn push_out(p: DVec2, o: &Obb, r: f64) -> DVec2 {
    let (s, c) = o.heading.sin_cos();
    let right = DVec2::new(c, -s);
    let fwd = DVec2::new(s, c);
    let d = p - o.center;
    let (lx, ly) = (d.dot(right), d.dot(fwd));
    let (hx, hy) = (o.half.x + r, o.half.y + r);
    if lx.abs() >= hx || ly.abs() >= hy {
        return p;
    }
    // out through the nearer side
    if hx - lx.abs() < hy - ly.abs() {
        o.center + right * (hx * lx.signum()) + fwd * ly
    } else {
        o.center + right * lx + fwd * (hy * ly.signum())
    }
}

/// Where one can stand outside vehicle `v` on side `side` (+1 its right): by its door on
/// that side nearest `near` (within reach of it: out through a door, never through the
/// wall beside the seat); None where there is no such door, or a wall or another vehicle is
/// in the way.
fn outside_spot(h: &mut Humans, world: Option<&crate::scene::World>, v: &omsi_sim::VehicleInstance, others: &[Obb], side: f64, near: DVec3) -> Option<DVec3> {
    let hd = v.heading.to_radians();
    let (fwd, right) = (DVec2::new(hd.sin(), hd.cos()), DVec2::new(hd.cos(), -hd.sin()));
    let half_w = v.ty.def.bounding_box.map(|b| b[0] as f64 * 0.5).unwrap_or(1.25);
    let door = h
        .vehicle_doors(v)
        .into_iter()
        .filter(|d| (*d - v.position).truncate().dot(right) * side > 0.0)
        .min_by(|a, b| (*a - near).length().total_cmp(&(*b - near).length()));
    let door = door.filter(|d| (*d - near).truncate().length() < DOOR_OUT_REACH)?;
    let _ = (fwd, half_w);
    outside_at(world, v, others, door)
}

/// Where one stands outside vehicle `v` by its door (outside point) `door`: clear of the
/// body, on the ground; None where a wall or another vehicle is in the way.
fn outside_at(world: Option<&crate::scene::World>, v: &omsi_sim::VehicleInstance, others: &[Obb], door: DVec3) -> Option<DVec3> {
    let own = v.ty.def.bounding_box.map(|bb| Obb::from_box(bb, v.position, v.heading));
    let mut p = door.truncate();
    // clear of the own bus's body
    if let Some(o) = own.as_ref() {
        p = push_out(p, o, RADIUS + 0.15);
    }
    let z = world.and_then(|w| w.walk_height(p.x, p.y)).unwrap_or(v.position.z);
    // room: no wall, no other vehicle
    let blocked = |q: DVec2| -> bool {
        let walls = world
            .map(|w| {
                let probe = Obb { center: q, half: DVec2::splat(1.0), heading: 0.0, z0: z - 1.0, z1: z + 2.5, velocity: DVec2::ZERO, mass: 0.0, pole: None, id: -1 };
                w.collision.lock().obstacles_near(&probe)
            })
            .unwrap_or_default();
        walls.iter().filter(|o| o.z0 < z + 1.6 && o.z1 > z + 0.45).chain(others.iter()).any(|o| (push_out(q, o, RADIUS) - q).length() > 0.05)
    };
    if blocked(p) {
        return None;
    }
    // (not up a wall nor down a drop)
    if (z - v.position.z).abs() > 1.5 {
        return None;
    }
    Some(DVec3::new(p.x, p.y, z))
}

impl App {
    /// The boxes of the vehicles within `r` of `at`: the own bus, the traffic, the other
    /// players' buses.
    fn vehicle_boxes(&self, at: DVec2, r: f64) -> Vec<Obb> {
        let mut boxes = Vec::new();
        let mut add = |v: &omsi_sim::VehicleInstance| {
            if (v.position.truncate() - at).length() > r {
                return;
            }
            if let Some(bb) = v.ty.def.bounding_box {
                boxes.push(Obb::from_box(bb, v.position, v.heading));
            }
        };
        if let Some(p) = self.player.as_ref() {
            add(&p.vehicle);
        }
        // (and the vehicles one placed: walked through before)
        for q in &self.placed {
            add(&q.vehicle);
        }
        if let Some(t) = self.traffic.as_ref() {
            for c in &t.cars {
                add(&c.vehicle);
            }
        }
        for rm in self.remotes.remotes.values() {
            add(rm.vehicle());
        }
        boxes
    }

    /// Ctrl+Shift+G at the wheel: up and out of the front door.
    pub(crate) fn get_up(&mut self) {
        if self.on_foot.is_some() {
            return;
        }
        if !self.settings.get_up {
            self.service_msg = Some(("Getting up is off: turn on \"Ability to get up\" in the settings".into(), 5.0));
            return;
        }
        // the other vehicles round the own bus (room to step out)
        let others: Vec<Obb> = {
            let at = self.player.as_ref().map(|p| p.vehicle.position.truncate()).unwrap_or_default();
            let own = self.player.as_ref().and_then(|p| p.vehicle.ty.def.bounding_box.map(|bb| Obb::from_box(bb, p.vehicle.position, p.vehicle.heading)));
            self.vehicle_boxes(at, 30.0).into_iter().filter(|o| own.map(|w| (w.center - o.center).length() > 0.01).unwrap_or(true)).collect()
        };
        let Some(p) = self.player.as_mut() else { return };
        p.axes.release_all();
        // the people's animation carries the walker: without passengers, a crowd of one
        if self.humans.is_none() {
            let mut h = crate::humans::Humans::new(&self.args.root);
            h.avatar_only = true;
            h.set_cabin(&mut p.vehicle);
            self.humans = Some(h);
        }
        let v = &p.vehicle;
        // the driver's own figure walks off, not a passenger's
        let driver_ty = p.driver.as_ref().map(|d| d.human_type());
        // out by the door by the driver's seat, else beside the seat
        let doors: Vec<DVec3> = self.humans.as_mut().and_then(|h| h.vehicle_driver_door(v)).into_iter().collect();
        let h = v.heading.to_radians();
        let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
        let half = v.ty.def.bounding_box.map(|b| (b[0] as f64 * 0.5, b[1] as f64 * 0.5 + b[4] as f64)).unwrap_or((1.25, 5.5));
        let pos = doors.first().copied().unwrap_or_else(|| {
            let xy = v.position.truncate() + fwd * (half.1 - 1.8) + right * (half.0 + 0.6);
            DVec3::new(xy.x, xy.y, v.position.z)
        });
        let pos = match self.world.as_ref().and_then(|w| w.walk_height(pos.x, pos.y)) {
            Some(z) if (z - pos.z).abs() < 2.0 => DVec3::new(pos.x, pos.y, z),
            _ => pos,
        };
        // facing away from the bus
        let away = (pos.truncate() - v.position.truncate()).dot(right).signum();
        let face = (right * away).x.atan2((right * away).y).to_degrees();
        let kind = match (driver_ty, self.humans.as_mut()) {
            (Some(t), Some(h)) => h.type_index(t) as u64,
            _ => self.args.root.to_string_lossy().len() as u64 * 7 + 3,
        };
        // up where the driver looks: out of that side of the bus (by a door there, else
        // beside the cab) when there is room outside, else beside the seat inside, facing
        // that way
        let look_yaw = self.camera.as_ref().map(|c| c.yaw as f64).unwrap_or(v.heading);
        let ly = look_yaw.to_radians();
        let look = DVec2::new(ly.sin(), ly.cos());
        let stand = self.humans.as_mut().and_then(|h| h.driver_stand(v));
        let seat_w = stand.and_then(|l| self.humans.as_mut().and_then(|h| h.vehicle_cabin_world(v, l))).unwrap_or(v.position);
        let side = look.dot(right);
        // a van's cab door by the seat (the W906): out of it, as a van driver gets out -
        // there is no standing room in such a cab, the driver who got up stood with their
        // head in the roof over the windscreen
        let cab_door = self.humans.as_mut().and_then(|h| h.vehicle_cab_door(v));
        let outside = match (cab_door, side.abs() > 0.45, self.humans.as_mut()) {
            (Some(d), _, Some(_)) => outside_at(self.world.as_deref(), v, &others, d),
            (None, true, Some(h)) => outside_spot(h, self.world.as_deref(), v, &others, side.signum(), seat_w),
            _ => None,
        };
        let _ = face;
        // up from the seat where one stands beside it, and out (where there is room) on
        // foot: walked, not put down outside at once
        let (pos, face, inside, transit) = match (outside, stand) {
            (Some(o), Some(l)) => (seat_w, look_yaw, Some((BusId::Player, l)), Some(Transit::walk(seat_w, o, None))),
            (Some(o), None) => (pos, look_yaw, None, Some(Transit::walk(pos, o, None))),
            (None, Some(l)) => (seat_w, look_yaw, Some((BusId::Player, l)), None),
            (None, None) => (pos, look_yaw, None, None),
        };
        self.on_foot = Some(OnFoot {
            pos,
            heading: face,
            vel: DVec2::ZERO,
            lift: 0.0,
            vz: 0.0,
            seat: None,
            inside,
            // (out of the driver's own eyes, as the cab view was)
            cam: FootCam::First,
            yaw: face as f32,
            pitch: -8.0,
            eye: self.camera.as_ref().map(|c| c.position),
            eye_yaw: self.camera.as_ref().map(|c| c.yaw).unwrap_or(face as f32),
            lag: DVec3::ZERO,
            // (only with the bus standing: at speed the eyes have to stay with it)
            settle: if self.player.as_ref().is_some_and(|p| p.vehicle.physics.velocity_kmh().abs() < 3.0) { 1.0 } else { 0.0 },
            view_before: if self.view == "foot" { "driver".into() } else { self.view.clone() },
            kind,
            face_seat: false,
            transit,
            arrive: None,
        });
        self.view = "foot".into();
        self.service_msg = Some(("On foot: W A S D walk, Shift runs, F4 free camera / F1 back, G sits down (by the driver's place: back at the wheel), Ctrl+Shift+G steps out of the bus".into(), 7.0));
    }

    /// Ctrl+Shift+G inside a bus: out by its nearest door, open or shut (getting up now
    /// stands the driver up inside, and a shut door kept them in).
    fn step_out(&mut self) {
        let Some(f) = self.on_foot.as_ref() else { return };
        let Some((bus, _)) = f.inside else { return };
        let (pos, yaw) = (f.pos, f.yaw as f64);
        let spot = if bus == BusId::Player {
            // out of the side the walker looks at (else the side of the nearest door)
            let others: Vec<Obb> = {
                let own = self.player.as_ref().and_then(|p| p.vehicle.ty.def.bounding_box.map(|bb| Obb::from_box(bb, p.vehicle.position, p.vehicle.heading)));
                self.vehicle_boxes(pos.truncate(), 30.0).into_iter().filter(|o| own.map(|w| (w.center - o.center).length() > 0.01).unwrap_or(true)).collect()
            };
            match (self.player.as_ref(), self.humans.as_mut()) {
                (Some(p), Some(h)) => {
                    let v = &p.vehicle;
                    let hd = v.heading.to_radians();
                    let right = DVec2::new(hd.cos(), -hd.sin());
                    let ly = yaw.to_radians();
                    let look_side = DVec2::new(ly.sin(), ly.cos()).dot(right);
                    let near_side = h.vehicle_doors(v).into_iter().min_by(|a, b| (*a - pos).length().total_cmp(&(*b - pos).length())).map(|d| (d - v.position).truncate().dot(right).signum()).unwrap_or(1.0);
                    let first = if look_side.abs() > 0.3 { look_side.signum() } else { near_side };
                    outside_spot(h, self.world.as_deref(), v, &others, first, pos).or_else(|| outside_spot(h, self.world.as_deref(), v, &others, -first, pos))
                }
                _ => None,
            }
        } else {
            self.humans.as_ref().and_then(|h| h.cabin_doors(bus).into_iter().map(|d| d.1).filter(|d| (*d - pos).truncate().length() < DOOR_OUT_REACH).min_by(|a, b| (*a - pos).length().total_cmp(&(*b - pos).length())))
        };
        if let Some(d) = spot {
            let z = self.world.as_ref().and_then(|w| w.walk_height(d.x, d.y)).unwrap_or(d.z);
            let f = self.on_foot.as_mut().unwrap();
            // (walked out, not put down outside)
            f.transit = Some(Transit::walk(f.pos, DVec3::new(d.x, d.y, z), None));
            f.vel = DVec2::ZERO;
        } else {
            self.service_msg = Some(("No door within reach here (or no room outside it): walk to a door first".into(), 3.0));
        }
    }

    /// Esc → Remove this vehicle: the bus driven goes (its riders step out where they are)
    /// and the player stands beside where its driver's door was, on foot; another bus is
    /// taken by walking up to its driver's door (G), or placed from the menu.
    pub(crate) fn remove_driven_vehicle(&mut self) {
        if self.player.is_none() {
            self.service_msg = Some(("There is no vehicle to remove: you are on foot".into(), 3.0));
            return;
        }
        // where to stand: by the driver's door, else beside the cab
        let stand = {
            let p = self.player.as_ref().unwrap();
            let v = &p.vehicle;
            if self.humans.is_none() {
                let mut h = crate::humans::Humans::new(&self.args.root);
                h.avatar_only = true;
                self.humans = Some(h);
            }
            let door = self.humans.as_mut().and_then(|h| h.vehicle_driver_door(v));
            let h = v.heading.to_radians();
            let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
            let half = v.ty.def.bounding_box.map(|b| (b[0] as f64 * 0.5, b[1] as f64 * 0.5 + b[4] as f64)).unwrap_or((1.25, 5.5));
            let p = door.unwrap_or_else(|| {
                let xy = v.position.truncate() + fwd * (half.1 - 1.8) - right * (half.0 + 0.8);
                DVec3::new(xy.x, xy.y, v.position.z)
            });
            let z = self.world.as_ref().and_then(|w| w.walk_height(p.x, p.y)).unwrap_or(p.z);
            (DVec3::new(p.x, p.y, z), v.heading)
        };
        if let Some(f) = self.on_foot.take() {
            if let Some(h) = self.humans.as_mut() {
                h.avatar_remove(AVATAR_KEY);
            }
            let _ = f;
        }
        let mut p = self.player.take().unwrap();
        if let (Some(a), Some(mut ss)) = (self.audio.as_ref(), p.sounds.take()) {
            ss.stop_all(a);
        }
        if let (Some(w), Some(r), Some(scene)) = (self.world.clone(), self.renderer.as_ref(), self.scene.as_mut()) {
            if let Some(mut d) = p.driver.take() {
                d.hide(r, scene);
            }
            if let Some(h) = self.humans.as_mut() {
                h.evict(BusId::Player, &w);
            }
            w.release_vehicle(r, scene, p.render);
            for t in p.trailer_renders {
                w.release_vehicle(r, scene, t);
            }
        }
        let name = format!("{} {}", p.vehicle.ty.def.manufacturer, p.vehicle.ty.def.type_name);
        self.start_on_foot(stand.0, stand.1);
        self.service_msg = Some((format!("{} removed: you are on foot (Esc menu: Place a vehicle, or G at a bus's driver's door)", name.trim()), 6.0));
    }

    /// On foot at `pos` without a bus of one's own (the vehicle removed, or the session
    /// started as a pedestrian).
    pub(crate) fn start_on_foot(&mut self, pos: DVec3, heading: f64) {
        if self.humans.is_none() {
            let mut h = crate::humans::Humans::new(&self.args.root);
            h.avatar_only = true;
            self.humans = Some(h);
        }
        self.on_foot = Some(OnFoot {
            pos,
            heading,
            vel: DVec2::ZERO,
            lift: 0.0,
            vz: 0.0,
            seat: None,
            inside: None,
            cam: FootCam::First,
            yaw: heading as f32,
            pitch: -5.0,
            eye: None,
            eye_yaw: heading as f32,
            lag: DVec3::ZERO,
            settle: 0.0,
            view_before: "driver".into(),
            kind: self.args.root.to_string_lossy().len() as u64 * 7 + 3,
            face_seat: false,
            transit: None,
            arrive: None,
        });
        self.view = "foot".into();
    }

    /// A placed vehicle whose driver's door (or cab) is within reach of `pos`: its index.
    fn placed_cab_near(&mut self, pos: DVec3) -> Option<usize> {
        let h = self.humans.as_mut()?;
        let mut best: Option<(usize, f64)> = None;
        for (k, q) in self.placed.iter().enumerate() {
            let v = &q.vehicle;
            let door = h.vehicle_driver_door(v).or_else(|| h.driver_stand(v).and_then(|l| h.vehicle_cabin_world(v, l)));
            if let Some(d) = door {
                let dist = (d - pos).truncate().length();
                if dist < DOOR_REACH + 0.6 && best.map(|b| dist < b.1).unwrap_or(true) {
                    best = Some((k, dist));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Take the wheel of placed vehicle `k` (the one driven now, if any, stays placed).
    pub(crate) fn take_placed(&mut self, k: usize) {
        if k >= self.placed.len() {
            return;
        }
        let mut next = self.placed.remove(k);
        if let Some(a) = self.audio.as_ref() {
            next.load_sounds(a);
        }
        next.vehicle.host.auto_clutch = if self.settings.auto_clutch { 1.0 } else { 0.0 };
        if let Some(now) = self.player.take() {
            let mut now = now;
            if let (Some(a), Some(mut ss)) = (self.audio.as_ref(), now.sounds.take()) {
                ss.stop_all(a);
            }
            if let Some(h) = self.humans.as_mut() {
                h.player_bus_swapped(now.uid, next.uid, &mut next.vehicle);
            }
            self.placed.push(now);
        } else if let Some(h) = self.humans.as_mut() {
            // (no bus driven before: whoever rides in this one is the player's bus's now)
            h.player_bus_swapped(0, next.uid, &mut next.vehicle);
        }
        let name = format!("{} {}", next.vehicle.ty.def.manufacturer, next.vehicle.ty.def.type_name);
        self.player = Some(next);
        if let Some(f) = self.on_foot.take() {
            if let Some(h) = self.humans.as_mut() {
                h.avatar_remove(AVATAR_KEY);
            }
            let _ = f;
        }
        self.view = "driver".into();
        self.sync_view_look();
        self.look = (0.0, 0.0);
        if let (Some(cam), Some(p)) = (self.camera.as_ref(), self.player.as_ref()) {
            self.camera = Some(p.camera("driver", cam));
        }
        self.service_msg = Some((format!("Now driving: {}", name.trim()), 4.0));
    }

    /// Esc → Back to my bus: wherever the walker has got to, back at the wheel.
    pub(crate) fn back_to_bus(&mut self) {
        if self.on_foot.is_some() && self.player.is_some() {
            self.sit_at_the_wheel();
            self.service_msg = Some(("Back at the wheel".into(), 3.0));
        }
    }

    /// In through the cab to the own bus's driver's place, walked, then at the wheel.
    fn walk_to_wheel(&mut self) {
        let to = self.player.as_ref().and_then(|p| {
            let v = &p.vehicle;
            let h = self.humans.as_mut()?;
            h.driver_stand(v).and_then(|l| h.vehicle_cabin_world(v, l))
        });
        match to {
            Some(to) => self.walk_in(to, Then::Wheel),
            None => self.sit_at_the_wheel(),
        }
    }

    /// Walk from where the walker is to `to`, then `then`.
    fn walk_in(&mut self, to: DVec3, then: Then) {
        if let Some(f) = self.on_foot.as_mut() {
            f.transit = Some(Transit::walk_in(f.pos, to, then));
            f.seat = None;
            f.inside = None;
            f.vel = DVec2::ZERO;
        }
    }

    /// Back at the wheel of the own bus.
    fn sit_at_the_wheel(&mut self) {
        let Some(f) = self.on_foot.take() else { return };
        if let Some(h) = self.humans.as_mut() {
            h.avatar_remove(AVATAR_KEY);
        }
        self.view = if f.view_before == "outside" || f.view_before == "driver" { f.view_before } else { "driver".into() };
        // (the eyes glide from where the walker's were into the cab camera)
        if self.view == "driver" {
            self.cam_blend.entering = true;
        }
        self.sync_view_look();
        self.look = (0.0, 0.0);
        // (the walking keys' help goes with the walking: it stood over the cab view)
        if self.service_msg.as_ref().is_some_and(|m| m.0.starts_with("On foot")) {
            self.service_msg = None;
        }
    }

    /// G on foot: into a seat by the nearest door (the own bus's front door: the wheel),
    /// or up from the seat and out.
    fn use_seat(&mut self) {
        let Some(f) = self.on_foot.as_ref() else { return };
        let Some(h) = self.humans.as_ref() else { return };
        if let Some((bus, k)) = f.seat {
            // up beside the seat, inside the bus (out of it through a door, as the
            // passengers go)
            if let Some(l) = h.seat_stand(bus, k) {
                if let Some((w, hd)) = h.cabin_world(bus, l) {
                    let f = self.on_foot.as_mut().unwrap();
                    f.pos = w;
                    f.seat = None;
                    f.inside = Some((bus, l));
                    f.vel = DVec2::ZERO;
                    f.heading = hd;
                    return;
                }
            }
            // up, and out by the door nearest the seat
            let at = h.avatar_body(AVATAR_KEY).map(|b| b.0).unwrap_or(f.pos);
            let door = h.bus_doors(bus).into_iter().min_by(|a, b| (*a - at).length().total_cmp(&(*b - at).length()));
            let Some(door) = door else { return };
            let z = self.world.as_ref().and_then(|w| w.walk_height(door.x, door.y)).unwrap_or(door.z);
            // out of the door, facing away from the bus
            let away = h.bus_center(bus).map(|c| door.truncate() - c.truncate()).unwrap_or(DVec2::Y);
            let face = away.x.atan2(away.y).to_degrees();
            let f = self.on_foot.as_mut().unwrap();
            f.transit = Some(Transit::walk(at, DVec3::new(door.x, door.y, z), None));
            f.pos = at;
            f.seat = None;
            f.vel = DVec2::ZERO;
            f.heading = face;
            f.yaw = face as f32;
            return;
        }
        let pos = f.pos;
        // inside a bus: the wheel of the own bus by the driver's place, else the free seat
        // nearest by
        if let Some((bus, _)) = f.inside {
            if bus == BusId::Player {
                let stand = self.player.as_ref().and_then(|p| self.humans.as_mut().and_then(|h| h.driver_stand(&p.vehicle).and_then(|l| h.vehicle_cabin_world(&p.vehicle, l))));
                if stand.map(|s| (s - pos).truncate().length() < 2.5).unwrap_or(true) {
                    self.sit_at_the_wheel();
                    return;
                }
            }
            let Some(h) = self.humans.as_ref() else { return };
            // (any bus: the own, a timetable bus, another player's)
            match h.seat_nearest(bus, pos, 2.5) {
                Some(k) => {
                    let f = self.on_foot.as_mut().unwrap();
                    f.seat = Some((bus, k));
                    f.inside = None;
                    f.face_seat = true;
                    f.vel = DVec2::ZERO;
                }
                None => self.service_msg = Some(("No free seat near: walk up to one".into(), 3.0)),
            }
            return;
        }
        // at the cab of a vehicle one placed (or with no bus of one's own): its wheel
        if let Some(k) = self.placed_cab_near(pos) {
            let own_near = self.player.as_ref().and_then(|p| self.humans.as_mut().and_then(|h| h.vehicle_driver_door(&p.vehicle))).map(|d| (d - pos).truncate().length() < DOOR_REACH).unwrap_or(false);
            if !own_near {
                let uid = self.placed[k].uid;
                let seat = {
                    let v = &self.placed[k].vehicle;
                    self.humans.as_mut().and_then(|h| h.driver_stand(v).and_then(|l| h.vehicle_cabin_world(v, l)))
                };
                match seat {
                    Some(to) => self.walk_in(to, Then::Placed(uid)),
                    None => self.take_placed(k),
                }
                return;
            }
        }
        // (the door by the driver's seat, not the first door of the list: next to it the
        // walker was put in the seat behind the driver)
        let own_front = self.player.as_ref().and_then(|p| self.humans.as_mut().and_then(|h| h.vehicle_driver_door(&p.vehicle)));
        let Some(h) = self.humans.as_ref() else { return };
        if omsi_cfg::env::var_os("OMSI_DEBUG_FOOT").is_some() { log::info!("on foot at ({:.1}, {:.1}): G - own bus doors {:?}, a seat near: {:?}", pos.x, pos.y, h.bus_doors(BusId::Player).iter().map(|d| ((d.x * 10.0).round() / 10.0, (d.y * 10.0).round() / 10.0)).collect::<Vec<_>>(), h.seat_near(pos, DOOR_REACH, None).map(|s| (s.bus, s.seat))); }
        if let Some(d) = own_front {
            if (d - pos).truncate().length() < DOOR_REACH {
                self.walk_to_wheel();
                return;
            }
        }
        // beside the own bus's cab, on either side (a bus without a door by its driver, or
        // one got out of on the other side): back at the wheel too
        let cab = self.player.as_ref().and_then(|p| {
            let v = &p.vehicle;
            let h = self.humans.as_mut()?;
            h.driver_stand(v).and_then(|l| h.vehicle_cabin_world(v, l))
        });
        if cab.map(|c| (c - pos).truncate().length() < 3.8).unwrap_or(false) {
            self.walk_to_wheel();
            return;
        }
        // Another bus is got into through one of its open doors, on foot (G at an open door
        // steps in): never into a seat straight from the pavement, through its side.
        let Some(h) = self.humans.as_ref() else { return };
        let mut door: Option<(BusId, glam::Vec3, f64)> = None;
        for bus in h.bus_ids_near(pos, 25.0) {
            for (inside, outside, _, open) in h.cabin_doors(bus) {
                let d = (outside.truncate() - pos.truncate()).length();
                if open && d < 1.4 && door.map(|x| d < x.2).unwrap_or(true) {
                    door = Some((bus, inside, d));
                }
            }
        }
        match door.and_then(|(bus, inside, _)| h.cabin_world(bus, inside).map(|w| (bus, inside, w.0))) {
            Some((bus, inside, w)) => {
                let f = self.on_foot.as_mut().unwrap();
                f.transit = Some(Transit::walk(f.pos, w, Some((bus, inside))));
                f.vel = DVec2::ZERO;
                f.lift = 0.0;
                f.vz = 0.0;
            }
            None => {
                self.service_msg = Some(("Get in through an open door of the bus, then G by a seat sits down".into(), 4.0));
            }
        }
    }

    /// The keys on foot (and Ctrl+Shift+G at the wheel); true when the key was taken.
    pub(crate) fn foot_key(&mut self, code: KeyCode, pressed: bool, repeat: bool, ctrl: bool, shift: bool) -> bool {
        if self.on_foot.is_none() {
            if pressed && !repeat && code == KeyCode::KeyG && ctrl && shift && self.player.is_some() {
                self.get_up();
                return true;
            }
            return false;
        }
        match code {
            // the game's own keys stay (menu, pause, screenshots, chat)
            KeyCode::Escape | KeyCode::F12 | KeyCode::KeyP | KeyCode::KeyV | KeyCode::Slash => false,
            KeyCode::F1 => {
                if pressed && !repeat {
                    if let Some(f) = self.on_foot.as_mut() {
                        f.cam = FootCam::First;
                        f.eye = None;
                    }
                    self.view = "foot".into();
                }
                true
            }
            KeyCode::F4 => {
                if pressed && !repeat {
                    if let Some(f) = self.on_foot.as_mut() {
                        f.cam = FootCam::Free;
                        f.vel = DVec2::ZERO;
                    }
                    self.view = "free".into();
                    self.ego = false;
                }
                true
            }
            // (no view from outside the walker: out of the eyes, or the free camera)
            KeyCode::F2 | KeyCode::F3 => true,
            // the free camera's keys are its own
            _ if self.on_foot.as_ref().map(|f| f.cam == FootCam::Free).unwrap_or(false) => false,
            KeyCode::KeyG => {
                if pressed && !repeat {
                    if ctrl && shift && self.on_foot.as_ref().map(|f| f.inside.is_some()).unwrap_or(false) {
                        self.step_out();
                    } else {
                        self.use_seat();
                    }
                }
                true
            }
            KeyCode::Space => {
                if let (true, false, Some(f)) = (pressed, repeat, self.on_foot.as_mut()) {
                    if f.seat.is_none() && f.grounded() {
                        f.vz = JUMP;
                    }
                }
                true
            }
            // everything else is the walker's, not the bus's (no switch is worked from the
            // pavement)
            _ => true,
        }
    }

    /// Turn the walker's view (right mouse button, arrows).
    pub(crate) fn foot_look(&mut self, dx: f32, dy: f32) {
        if let Some(f) = self.on_foot.as_mut() {
            f.yaw = (f.yaw + dx).rem_euclid(360.0);
            f.pitch = (f.pitch - dy).clamp(-80.0, 80.0);
        }
    }

    /// One frame on foot: the walk, the avatar, the camera.
    pub(crate) fn tick_on_foot(&mut self, dt: f32) {
        // walked in to a driver's place: at the wheel now
        match self.on_foot.as_mut().and_then(|f| f.arrive.take()) {
            Some(Then::Wheel) if self.player.is_some() => {
                self.sit_at_the_wheel();
                self.service_msg = Some(("Back at the wheel".into(), 2.0));
                return;
            }
            Some(Then::Placed(uid)) => {
                if let Some(k) = self.placed.iter().position(|q| q.uid == uid) {
                    self.take_placed(k);
                    return;
                }
            }
            _ => {}
        }
        let Some(mut f) = self.on_foot.take() else { return };
        let dt64 = dt as f64;
        let key = |k: KeyCode| self.keys.contains(&k);
        if key(KeyCode::ArrowLeft) {
            f.yaw -= 90.0 * dt;
        }
        if key(KeyCode::ArrowRight) {
            f.yaw += 90.0 * dt;
        }
        if key(KeyCode::ArrowUp) {
            f.pitch = (f.pitch + 60.0 * dt).min(80.0);
        }
        if key(KeyCode::ArrowDown) {
            f.pitch = (f.pitch - 60.0 * dt).max(-80.0);
        }
        // the seat's bus went away (out of range): up where the seat was
        if let (Some((bus, _)), Some(h)) = (f.seat, self.humans.as_ref()) {
            if !h.bus_here(bus) {
                f.seat = None;
            }
        }
        if let (Some((bus, _)), Some(h)) = (f.inside, self.humans.as_ref()) {
            // (the own bus is there while there is one: the people's list of buses is
            // filled at their next tick, not yet in the frame the player got up)
            if !h.bus_here(bus) && !(bus == BusId::Player && self.player.is_some()) {
                f.inside = None;
            }
        }
        let free = f.cam == FootCam::Free;
        // a step taken by itself (through a door, out of the cab): walked there
        if let Some(mut tr) = f.transit.filter(|_| !self.paused) {
            tr.t += dt;
            let k = (tr.t / tr.dur).clamp(0.0, 1.0) as f64;
            f.inside = None;
            f.seat = None;
            f.lift = 0.0;
            f.vz = 0.0;
            f.pos = tr.from.lerp(tr.to, k);
            f.vel = (tr.to - tr.from).truncate() / tr.dur as f64;
            if f.vel.length() > 0.1 {
                let course = f.vel.x.atan2(f.vel.y).to_degrees();
                let d = wrap(course - f.heading);
                f.heading = wrap(f.heading + d.clamp(-220.0 * dt64, 220.0 * dt64));
            }
            if k >= 1.0 {
                f.transit = None;
                f.inside = tr.after;
                f.vel = DVec2::ZERO;
                if tr.then != Then::Nothing {
                    f.arrive = Some(tr.then);
                }
            } else {
                f.transit = Some(tr);
            }
        }
        if f.seat.is_none() && !self.paused && !free && f.transit.is_none() {
            // where the keys go, as the camera looks
            let y = (f.yaw as f64).to_radians();
            let (fwd, right) = (DVec2::new(y.sin(), y.cos()), DVec2::new(y.cos(), -y.sin()));
            let mut dir = DVec2::ZERO;
            if key(KeyCode::KeyW) {
                dir += fwd;
            }
            if key(KeyCode::KeyS) {
                dir -= fwd;
            }
            if key(KeyCode::KeyD) {
                dir += right;
            }
            if key(KeyCode::KeyA) {
                dir -= right;
            }
            let run = key(KeyCode::ShiftLeft) || key(KeyCode::ShiftRight);
            let want = dir.normalize_or_zero() * if run { RUN } else { WALK };
            // (in the air only a little steering)
            let k = 1.0 - (-dt64 * if f.grounded() { ACCEL } else { 1.0 }).exp();
            f.vel += (want - f.vel) * k;
            if f.vel.length() < 0.02 && want == DVec2::ZERO {
                f.vel = DVec2::ZERO;
            }
            // the body turns to where it goes (out of its eyes: to where they look)
            let face = Some(f.yaw as f64);
            if let Some(face) = face {
                let d = wrap(face - f.heading);
                // (a person turns at most ~220°/s: the body spun round its planted feet)
                let step = if f.vel.length() < 0.3 { 110.0 } else { 220.0 } * dt64;
                f.heading = wrap(f.heading + d.clamp(-step, step));
            }
            // inside a bus: along its cabin, with it as it drives, out through an open door
            let mut stepped_in = false;
            if let (Some((bus, local)), Some(h)) = (f.inside, self.humans.as_ref()) {
                let hd = h.cabin_world(bus, local).map(|x| x.1).unwrap_or(0.0).to_radians();
                let (bf, br) = (DVec2::new(hd.sin(), hd.cos()), DVec2::new(hd.cos(), -hd.sin()));
                let step = glam::Vec2::new((f.vel.dot(br) * dt64) as f32, (f.vel.dot(bf) * dt64) as f32);
                if let Some((l, w)) = h.cabin_walk(bus, local, step) {
                    f.pos = w;
                    f.inside = Some((bus, l));
                    f.lift = 0.0;
                    f.vz = 0.0;
                    // at an open door, heading out of it: down onto the pavement
                    let out = h.cabin_doors(bus).into_iter().find(|(inside, _, side, open)| {
                        *open && (inside.truncate() - l.truncate()).length() < 1.0 && step.x * side > 0.0005
                    });
                    if let Some((_, outside, _, _)) = out {
                        let z = self.world.as_ref().and_then(|w| w.walk_height(outside.x, outside.y)).unwrap_or(outside.z);
                        // down the step onto the pavement, walked (it was a jump of a metre)
                        f.transit = Some(Transit::walk(w, DVec3::new(outside.x, outside.y, z), None));
                    }
                }
                stepped_in = true;
            } else if let Some(h) = self.humans.as_ref() {
                // on the pavement at an open door of the own bus, walking towards it: in
                let moving = f.vel.length() > 0.3;
                if moving {
                    'buses: for bus in h.bus_ids_near(f.pos, 25.0) {
                        for (inside, outside, _, open) in h.cabin_doors(bus) {
                            // (only through a door that is open, whoever's bus it is)
                            if !open || (outside.truncate() - f.pos.truncate()).length() > 0.9 {
                                continue;
                            }
                            let Some((wi, _)) = h.cabin_world(bus, inside) else { continue };
                            if (wi.truncate() - outside.truncate()).dot(f.vel) > 0.0 {
                                // up the step into the bus, walked
                                f.transit = Some(Transit::walk(f.pos, wi, Some((bus, inside))));
                                f.lift = 0.0;
                                f.vz = 0.0;
                                stepped_in = true;
                                break 'buses;
                            }
                        }
                    }
                }
            }
            // on: the ground where the feet come, a step up to 45 cm, never through walls
            // or vehicles
            let mut next = if stepped_in { f.pos.truncate() } else { f.pos.truncate() + f.vel * dt64 };
            if stepped_in {
                // (inside: the cabin said where)
            } else if let Some(w) = self.world.as_ref() {
                let z = f.pos.z;
                let probe = Obb { center: next, half: DVec2::splat(3.0), heading: 0.0, z0: z - 1.0, z1: z + 2.5, velocity: DVec2::ZERO, mass: 0.0, pole: None, id: -1 };
                let walls = w.collision.lock().obstacles_near(&probe);
                let mut boxes: Vec<Obb> = walls.into_iter().filter(|o| o.z0 < z + 1.6 + f.lift && o.z1 > z + 0.45 + f.lift).collect();
                boxes.extend(self.vehicle_boxes(next, 20.0));
                for _ in 0..2 {
                    for o in &boxes {
                        next = push_out(next, o, RADIUS);
                    }
                }
                match w.walk_height(next.x, next.y) {
                    Some(g) if g - (z + f.lift) > 0.45 => {
                        // too high a step: stay (and lose the speed into it)
                        next = f.pos.truncate();
                        f.vel *= 0.3;
                    }
                    Some(g) => {
                        let feet = z + f.lift;
                        if g >= feet - 0.02 || f.grounded() && feet - g < 0.4 {
                            // on the ground (down a kerb the feet follow it)
                            f.pos.z = g;
                            if f.vz <= 0.0 {
                                f.lift = 0.0;
                            } else {
                                f.lift = (feet - g).max(0.0);
                            }
                        } else {
                            // stepped off something high: falling
                            f.pos.z = g;
                            f.lift = feet - g;
                        }
                    }
                    None => {}
                }
            }
            f.pos.x = next.x;
            f.pos.y = next.y;
            // the jump (not inside a bus)
            if f.inside.is_some() {
                f.vz = 0.0;
                f.lift = 0.0;
            }
            if f.vz > 0.0 || f.lift > 0.0 {
                f.vz -= 9.81 * dt64;
                f.lift += f.vz * dt64;
                if f.lift <= 0.0 {
                    f.lift = 0.0;
                    f.vz = 0.0;
                }
            }
        }
        // the avatar
        let show = free;
        if let (Some(h), Some(w), Some(r), Some(scene)) = (self.humans.as_mut(), self.world.as_ref(), self.renderer.as_ref(), self.scene.as_mut()) {
            // (stepping through a door the feet keep to the step, not to the road under it)
            let cmd = AvatarCmd { pos: f.pos, heading: f.heading, vel: f.vel, lift: f.lift, seat: f.seat, floor: f.inside.map(|_| f.pos.z).or(f.transit.map(|_| f.pos.z)), aboard: f.inside };
            h.avatar(AVATAR_KEY, w, r, scene, cmd, f.kind);
            h.avatar_show(AVATAR_KEY, show);
        }
        // seated, the walker is where the seat is
        let body = self.humans.as_ref().and_then(|h| h.avatar_body(AVATAR_KEY));
        if let (Some((feet, heading, _)), Some(_)) = (body, f.seat) {
            f.pos = feet;
            f.heading = heading;
            if f.face_seat {
                let d = wrap(heading - f.yaw as f64);
                f.yaw = (f.yaw as f64 + d * (1.0 - (-dt64 * 4.0).exp())) as f32;
                f.pitch += (-5.0 - f.pitch) * (1.0 - (-dt * 4.0).exp());
                if d.abs() < 2.0 {
                    f.face_seat = false;
                }
            }
        }
        // the camera, out of the eyes (the free camera moves by itself)
        if let (Some(cam), false) = (self.camera.as_mut(), free) {
            let eye = body.map(|b| b.2).unwrap_or(f.pos + DVec3::new(0.0, 0.0, 1.62 + f.lift));
            let y = (f.yaw as f64).to_radians();
            let (want, want_yaw, want_pitch) = (eye + DVec3::new(y.sin(), y.cos(), 0.0) * 0.08, f.yaw, f.pitch);
            // just got up: 4 per second at first (a glide out of the cab's camera), then up to
            // the walk's 18 over the second
            let settling = f.settle > 0.0;
            f.settle = (f.settle - dt).max(0.0);
            let u = (1.0 - f.settle as f64).clamp(0.0, 1.0);
            let rate = 4.0 + 14.0 * u * u * (3.0 - 2.0 * u);
            let k = 1.0 - (-dt64 * rate).exp();
            let at = match f.eye {
                Some(e) if (e - want).length() < 60.0 => e + (want - e) * k,
                _ => want,
            };
            f.eye = Some(at);
            // (what the glide is still behind by is kept while it lasts and let go of after:
            // `foot_after_humans` puts the eyes on the head from then on, and without this
            // the last bit of the way was one jump)
            if settling {
                f.lag = at - want;
            } else {
                f.lag *= (-dt64 * 9.0).exp();
                if f.lag.length() < 0.001 {
                    f.lag = DVec3::ZERO;
                }
            }
            let dy = ((want_yaw - f.eye_yaw + 540.0).rem_euclid(360.0)) - 180.0;
            f.eye_yaw = (f.eye_yaw + dy * k as f32).rem_euclid(360.0);
            cam.position = at;
            cam.yaw = f.eye_yaw;
            cam.pitch += (want_pitch - cam.pitch) * k as f32;
            // (the cab's camera leans with the bus: the lean goes out of the picture with the
            // rest of the glide)
            cam.roll += (0.0 - cam.roll) * k as f32;
        }
        if omsi_cfg::env::var_os("OMSI_DEBUG_FOOT").is_some() && (self.total_frames % 30 == 0) {
            let body = self.humans.as_ref().and_then(|h| h.avatar_body(AVATAR_KEY));
            log::info!("foot: inside {:?} eye {:?} pos ({:.2}, {:.2}, {:.2}) heading {:.0} yaw {:.0} vel ({:.2}, {:.2}) lift {:.2} seat {:?} cam {:?} cam_pos {:?} cam_yaw {:.0} body {:?}", f.inside, body.map(|b| b.2), f.pos.x, f.pos.y, f.pos.z, f.heading, f.yaw, f.vel.x, f.vel.y, f.lift, f.seat, f.cam, self.camera.as_ref().map(|c| c.position), self.camera.as_ref().map(|c| c.yaw).unwrap_or(0.0), body);
        }
        self.on_foot = Some(f);
    }

    /// After the people's tick, aboard a bus: the walker and the eyes where the bus is
    /// this frame. The walk above placed them on the bus as the people had it a frame
    /// before, and the camera eased after that: at speed the saloon trembled round the
    /// eyes and every stop jerked them forwards.
    pub(crate) fn foot_after_humans(&mut self) {
        let Some(f) = self.on_foot.as_mut() else { return };
        if f.settle > 0.0 || f.cam != FootCam::First || (f.seat.is_none() && f.inside.is_none()) {
            return;
        }
        let Some(h) = self.humans.as_ref() else { return };
        if let Some((bus, l)) = f.inside {
            if let Some((w, _)) = h.cabin_world(bus, l) {
                f.pos = w;
            }
        }
        let eye = h.avatar_body(AVATAR_KEY).map(|b| b.2).unwrap_or(f.pos + DVec3::new(0.0, 0.0, 1.62));
        let y = (f.eye_yaw as f64).to_radians();
        let at = eye + DVec3::new(y.sin(), y.cos(), 0.0) * 0.08 + f.lag;
        f.eye = Some(at);
        if let Some(cam) = self.camera.as_mut() {
            cam.position = at;
        }
    }

    /// The bus the player on foot is in (standing or sitting), if any.
    pub(crate) fn foot_bus(&self) -> Option<BusId> {
        let f = self.on_foot.as_ref()?;
        f.seat.map(|s| s.0).or(f.inside.map(|i| i.0))
    }

    /// The pose other players see of this one on foot (None at the wheel).
    pub(crate) fn walker_pose(&self) -> Option<omsi_net::Walker> {
        let f = self.on_foot.as_ref()?;
        // aboard a player's bus (ours or another's): where in it, so that the others draw
        // us in that bus as it drives
        let my_id = self.lan.as_ref().map(|l| l.my_id).filter(|i| *i != 0);
        let owner = |b: BusId| match b {
            BusId::Player => my_id,
            BusId::Ai(x) => crate::humans::remote_bus_player(x),
        };
        let aboard = match (f.seat, f.inside) {
            (Some((b, k)), _) => owner(b).map(|o| omsi_net::Aboard {
                owner: o,
                local: self.humans.as_ref().and_then(|h| h.seat_stand(b, k)).map(|l| l.to_array()).unwrap_or_default(),
                seat: Some(k as u16),
            }),
            (None, Some((b, l))) => owner(b).map(|o| omsi_net::Aboard { owner: o, local: l.to_array(), seat: None }),
            _ => None,
        };
        // (the way it goes, which is not where the body faces when it steps sideways)
        let course = if f.vel.length() > 0.05 { f.vel.x.atan2(f.vel.y).to_degrees() as f32 } else { f.heading as f32 };
        Some(omsi_net::Walker { x: f.pos.x, y: f.pos.y, z: f.pos.z + f.lift, heading: f.heading as f32, speed: f.vel.length() as f32, course, seated: f.seat.is_some(), aboard })
    }

    /// Other players on foot: their avatars.
    pub(crate) fn sync_remote_walkers(&mut self) {
        let walkers: Vec<(u32, Option<omsi_net::Walker>, String)> = self.remotes.remotes.iter().map(|(id, r)| (*id, r.last.walker, r.last.figure.clone())).collect();
        if walkers.iter().all(|w| w.1.is_none()) && self.remote_walkers.is_empty() {
            return;
        }
        if walkers.iter().any(|w| w.1.is_some()) && self.humans.is_none() {
            let mut h = Humans::new(&self.args.root);
            h.avatar_only = true;
            self.humans = Some(h);
        }
        let (Some(h), Some(w), Some(r), Some(scene)) = (self.humans.as_mut(), self.world.as_ref(), self.renderer.as_ref(), self.scene.as_mut()) else { return };
        let my_id = self.lan.as_ref().map(|l| l.my_id).unwrap_or(0);
        let mut now = Vec::new();
        for (id, wk, figure) in walkers {
            let Some(wk) = wk else { continue };
            // (the way it goes: an older game sends none, then its facing)
            let hh = (if wk.course.is_finite() { wk.course } else { wk.heading } as f64).to_radians();
            // the player's own figure, as their game draws them (a random passenger's
            // otherwise), else one fixed by their id
            let kind = omsi_net::human_path(&figure)
                .map(|rel| omsi_cfg::resolve_path(&self.args.root, &rel))
                .filter(|p| omsi_cfg::vfs::exists(p))
                .and_then(|p| crate::driver::cached_type(&p))
                .map(|t| h.type_index(t) as u64)
                .unwrap_or(id as u64 * 13 + 5);
            // aboard a player's bus (ours, or a third player's): in that bus's frame as it
            // is drawn here, on its seat or its floor
            let aboard = wk.aboard.and_then(|a| {
                let bus = if a.owner == my_id { BusId::Player } else { BusId::Ai(crate::humans::remote_bus_id(a.owner)) };
                let (at, bh) = h.cabin_world(bus, glam::Vec3::from(a.local))?;
                Some((bus, a.seat, at, bh))
            });
            let cmd = match aboard {
                Some((bus, Some(k), at, _)) => AvatarCmd { pos: at, heading: wk.heading as f64, vel: DVec2::ZERO, lift: 0.0, seat: Some((bus, k as usize)), floor: Some(at.z), aboard: None },
                Some((bus, None, at, _)) => AvatarCmd { pos: at, heading: wk.heading as f64, vel: DVec2::new(hh.sin(), hh.cos()) * wk.speed as f64, lift: 0.0, seat: None, floor: Some(at.z), aboard: wk.aboard.map(|a| (bus, glam::Vec3::from(a.local))) },
                // (sitting in a bus that is not a player's: not drawn - its seat is not known)
                None if wk.seated => continue,
                None => AvatarCmd { pos: DVec3::new(wk.x, wk.y, wk.z), heading: wk.heading as f64, vel: DVec2::new(hh.sin(), hh.cos()) * wk.speed as f64, lift: 0.0, seat: None, floor: w.walk_height(wk.x, wk.y).filter(|g| wk.z > g + 0.25).map(|_| wk.z), aboard: None },
            };
            h.avatar(REMOTE_KEY + id, w, r, scene, cmd, kind);
            if !self.remote_walkers.contains(&id) {
                log::info!("LAN: player {id} got up and walks at ({:.1}, {:.1})", wk.x, wk.y);
            }
            now.push(id);
        }
        for id in std::mem::take(&mut self.remote_walkers) {
            if !now.contains(&id) {
                h.avatar_remove(REMOTE_KEY + id);
            }
        }
        self.remote_walkers = now;
    }
}
