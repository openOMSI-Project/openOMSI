//! Avatars: the player got up from the seat (`on_foot`), or another player walks about. The
//! game moves them; the people's animation poses them - the gait and its feet on the
//! ground, sitting down on a seat and getting up - so every change is eased, never a jump.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PuppetMode {
    /// The player on foot (or another player's walker): moved by the game, see `avatar`.
    Avatar,
}

/// A person the game moves itself (the player on foot).
#[derive(Debug, Clone, Copy)]
pub struct Puppet {
    pub mode: PuppetMode,
}

/// What the game wants of an avatar this frame.
#[derive(Debug, Clone, Copy)]
pub struct AvatarCmd {
    /// The feet (on foot), in the world.
    pub pos: DVec3,
    /// Facing (degrees, OMSI's).
    pub heading: f64,
    /// Velocity over the ground (m/s).
    pub vel: DVec2,
    /// How high the feet are over the ground (a jump).
    pub lift: f64,
    /// Sitting on this seat of this bus.
    pub seat: Option<(BusId, usize)>,
    /// Standing on a vehicle's floor at this height rather than on the ground (walking
    /// inside a bus: the feet stay on its floor, not reaching down to the road).
    pub floor: Option<f64>,
    /// Standing or walking inside this bus at this point of its cabin (bus frame): placed
    /// in the bus's frame as it is this frame, as its passengers are (a world point taken
    /// a frame earlier left the figure trembling behind the moving bus).
    pub aboard: Option<(BusId, Vec3)>,
}

/// A seat an avatar may take: which, in which bus.
#[derive(Debug, Clone, Copy)]
pub struct SeatSpot {
    pub bus: BusId,
    pub seat: usize,
}

/// How near a walker comes to a seat (the sitter's hip point) or the driver's place, m.
const SEAT_CLEARANCE: f32 = 0.38;

/// A step from `from` to `to` (cabin x, y) kept `SEAT_CLEARANCE` from every point of `solid`:
/// slid round the ones it would come too near (walking away from one that close is let be).
/// None where no such place is near (wedged between two).
/// (A step that came too near was refused whole: on a double-decker's upper deck, whose seats
/// stand 0.46 m either side of the aisle, a walker a few centimetres off its middle - as one
/// comes off the stairs - could not move along it at all.)
fn clear_of_seats(from: glam::Vec2, to: glam::Vec2, solid: &[glam::Vec2]) -> Option<glam::Vec2> {
    let mut xy = to;
    for _ in 0..3 {
        let mut pushed = false;
        for &c in solid {
            let (dn, d0) = ((xy - c).length(), (from - c).length());
            if dn < SEAT_CLEARANCE - 1e-4 && dn < d0 {
                // out to the clearance, on the side the walker comes from
                let away = if dn > 1e-4 { (xy - c) / dn } else { (from - c).normalize_or(glam::Vec2::X) };
                xy = c + away * SEAT_CLEARANCE;
                pushed = true;
            }
        }
        if !pushed {
            return Some(xy);
        }
    }
    // (still too near one after sliding off the others: stay)
    solid.iter().all(|&c| (xy - c).length() >= SEAT_CLEARANCE - 1e-3 || (xy - c).length() >= (from - c).length()).then_some(xy)
}

#[cfg(test)]
mod seat_clearance_tests {
    use super::*;
    use glam::Vec2;

    /// The SD202's upper deck: seats 0.46 m either side of the aisle. A walker 0.17 m off its
    /// middle walking back slides along the seat beside it instead of standing still.
    #[test]
    fn a_walker_slides_along_a_seat_instead_of_stopping() {
        let seats = [Vec2::new(0.46, 2.185), Vec2::new(-0.46, 1.962)];
        let mut at = Vec2::new(0.19, 2.53);
        for _ in 0..40 {
            at = clear_of_seats(at, at + Vec2::new(0.0, -0.03), &seats).expect("a way past");
        }
        assert!(at.y < 1.5, "stuck at {at}");
        for s in seats {
            assert!((at - s).length() >= SEAT_CLEARANCE - 1e-3);
        }
    }

    /// Straight into a seat: kept off it, not through it.
    #[test]
    fn a_seat_is_not_walked_through() {
        let seat = Vec2::new(0.0, 1.0);
        let mut at = Vec2::new(0.0, 0.0);
        for _ in 0..60 {
            at = clear_of_seats(at, at + Vec2::new(0.0, 0.03), &[seat]).unwrap_or(at);
        }
        assert!((at - seat).length() >= SEAT_CLEARANCE - 1e-3, "{at}");
        // and away from it again freely
        let back = clear_of_seats(at, at - Vec2::new(0.0, 0.03), &[seat]);
        assert_eq!(back, Some(at - Vec2::new(0.0, 0.03)));
    }

    /// Between two seats closer together than twice the clearance: no way through.
    #[test]
    fn too_narrow_a_gap_holds_the_walker() {
        let seats = [Vec2::new(-0.3, 1.0), Vec2::new(0.3, 1.0)];
        let mut at = Vec2::new(0.0, 0.5);
        for _ in 0..40 {
            at = clear_of_seats(at, at + Vec2::new(0.0, 0.03), &seats).unwrap_or(at);
        }
        assert!(at.y < 1.0 - 0.1, "{at}");
    }
}

impl PeopleSim {
    /// Put avatar `key` where `cmd` says (made on its first call, of figure `kind`).
    pub fn avatar(&mut self, key: u32, world: &dyn World, cmd: AvatarCmd, kind: u64) {
        let known = self.avatars.get(&key).copied().filter(|id| self.people.iter().any(|p| p.id == *id));
        if known.is_none() {
            let state = State::Idle;
            let n = self.types.len().max(1) as u64;
            let Some(i) = self.spawn_as(world, cmd.pos, cmd.heading, state, Some((kind % n) as usize)) else { return };
            self.people[i].puppet = Some(Puppet { mode: PuppetMode::Avatar });
            self.avatars.insert(key, self.people[i].id);
        }
        // a seat taken is kept from the passengers; one left is theirs again
        let before = self.avatar_cmds.get(&key).and_then(|c| c.seat);
        if before != cmd.seat {
            if let Some((b, k)) = before {
                self.free_seat(b, k);
            }
            if let Some((b, k)) = cmd.seat {
                if let Some(t) = self.seats.get_mut(&b).and_then(|v| v.get_mut(k)) {
                    *t = true;
                }
            }
        }
        self.avatar_cmds.insert(key, cmd);
    }

    /// Take avatar `key` away.
    pub fn avatar_remove(&mut self, key: u32) {
        if let Some(c) = self.avatar_cmds.remove(&key) {
            if let Some((b, k)) = c.seat {
                self.free_seat(b, k);
            }
        }
        if let Some(id) = self.avatars.remove(&key) {
            if let Some(i) = self.people.iter().position(|p| p.id == id) {
                let p = self.people.swap_remove(i);
                self.retire(&p);
            }
        }
    }

    /// Draw avatar `key` or not (the first-person view looks out of its eyes).
    pub fn avatar_show(&mut self, key: u32, show: bool) {
        if let Some(id) = self.avatars.get(&key) {
            self.avatar_hidden.insert(*id, !show);
        }
    }

    /// Where avatar `key` is drawn: its feet, facing, and its eyes.
    pub fn avatar_body(&self, key: u32) -> Option<(DVec3, f64, DVec3)> {
        let id = self.avatars.get(&key)?;
        let p = self.people.iter().find(|p| p.id == *id)?;
        let rig = &p.ty.rig;
        let eye_h = (rig.head_top - 0.11 * rig.scale) as f64;
        let eye = match (p.place, self.avatar_cmds.get(&key).and_then(|c| c.seat)) {
            (Place::Bus(b, _), Some((_, k))) => {
                let bn = self.last_buses.iter().find(|x| x.id == b)?;
                let s = bn.cabin.seats.get(k)?;
                // sitting: the eyes over the hip, a little back
                let r = s.rot.to_radians();
                bn.world(s.pos + Vec3::new(-r.sin() * 0.05, -r.cos() * 0.05, (eye_h - rig.hip[0].z as f64) as f32 + 0.04))
            }
            _ => p.position + DVec3::new(0.0, 0.0, eye_h),
        };
        Some((p.position, p.heading, eye))
    }

    /// The seat nearest `at` with a door of its bus within `reach` of it (people and
    /// the other avatars' seats taken), among the buses of the last tick; `only` limits it
    /// to one bus.
    pub fn seat_near(&self, at: DVec3, reach: f64, only: Option<BusId>) -> Option<SeatSpot> {
        let mut best: Option<(f64, SeatSpot)> = None;
        for bn in &self.last_buses {
            if only.map(|o| o != bn.id).unwrap_or(false) {
                continue;
            }
            // the nearest door (entries and exits: any door will do to get in)
            let door = bn
                .cabin
                .entries
                .iter()
                .chain(bn.cabin.exits.iter())
                .map(|d| bn.world(d.outside))
                .min_by(|a, b| (*a - at).length().total_cmp(&(*b - at).length()));
            let Some(door) = door else { continue };
            let d = (door - at).truncate().length();
            if d > reach {
                continue;
            }
            let taken = self.seats.get(&bn.id);
            let seat = bn
                .cabin
                .seats
                .iter()
                .enumerate()
                .filter(|(k, s)| s.seated && !taken.and_then(|t| t.get(*k)).copied().unwrap_or(false))
                .min_by(|a, b| (bn.world(a.1.floor) - door).length().total_cmp(&(bn.world(b.1.floor) - door).length()))
                .map(|(k, _)| k);
            let Some(seat) = seat else { continue };
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, SeatSpot { bus: bn.id, seat }));
            }
        }
        best.map(|b| b.1)
    }

    /// Where the doors of a bus are now (outside, in the world).
    pub fn bus_doors(&self, bus: BusId) -> Vec<DVec3> {
        self.last_buses
            .iter()
            .find(|b| b.id == bus)
            .map(|bn| bn.cabin.entries.iter().chain(bn.cabin.exits.iter()).map(|d| bn.world(d.outside)).collect())
            .unwrap_or_default()
    }

    /// The door of vehicle `v` nearest its driver's seat (outside, in the world): where the
    /// driver gets in and out.
    pub fn vehicle_driver_door(&mut self, v: &VehicleInstance) -> Option<DVec3> {
        // (a van's own cab door first: its driver does not climb in through the sliding door)
        if let Some(d) = self.vehicle_cab_door(v) {
            return Some(d);
        }
        let cabin = self.cabin_for(v)?;
        let seat = cabin.data.driver_positions.first().map(|d| Vec3::from(d.pos)).unwrap_or(Vec3::new(-0.8, 4.5, 1.0));
        let door = cabin.entries.iter().chain(cabin.exits.iter()).min_by(|a, b| (a.outside - seat).truncate().length().total_cmp(&(b.outside - seat).truncate().length()))?;
        let trailers = part_frames(v, &cabin);
        Some(train_point(v.position, &v.body_rotation(), &trailers, door.outside))
    }

    /// A door of the driver's own beside the driver's seat (a van's or a coach's cab door:
    /// on the driver's side, level with the seat), in the world, outside.
    pub fn vehicle_cab_door(&mut self, v: &VehicleInstance) -> Option<DVec3> {
        let cabin = self.cabin_for(v)?;
        let seat = cabin.data.driver_positions.first().map(|d| Vec3::from(d.pos))?;
        let door = cabin
            .entries
            .iter()
            .chain(cabin.exits.iter())
            .filter(|d| d.outside.x * seat.x > 0.0 && (d.outside.y - seat.y).abs() < 1.5)
            .min_by(|a, b| (a.outside - seat).truncate().length().total_cmp(&(b.outside - seat).truncate().length()))
            .map(|d| d.outside);
        // a van or minibus (the W906: its cabin knows only the sliding door, the passengers'):
        // the driver's door beside the seat, which every such vehicle has
        let door = door.or_else(|| {
            let bb = v.ty.def.bounding_box?;
            (bb[1] < 8.5 && seat.x.abs() > 0.2).then(|| Vec3::new(seat.x.signum() * (bb[0] * 0.5 + bb[3] * seat.x.signum() + 0.45), seat.y, 0.0))
        })?;
        let trailers = part_frames(v, &cabin);
        Some(train_point(v.position, &v.body_rotation(), &trailers, door))
    }

    /// Put `ty` among the figures (once) and give its index: the player's own figure.
    pub fn type_index(&mut self, ty: Arc<HumanType>) -> usize {
        if let Some(i) = self.types.iter().position(|t| Arc::ptr_eq(t, &ty) || t.def.path == ty.def.path) {
            return i;
        }
        self.types.push(ty);
        self.types.len() - 1
    }

    /// Where the doors of vehicle `v` are now (outside, in the world), entries first.
    pub fn vehicle_doors(&mut self, v: &VehicleInstance) -> Vec<DVec3> {
        let Some(cabin) = self.cabin_for(v) else { return Vec::new() };
        let trailers = part_frames(v, &cabin);
        let rot = v.body_rotation();
        cabin.entries.iter().chain(cabin.exits.iter()).map(|d| train_point(v.position, &rot, &trailers, d.outside)).collect()
    }

    /// A walker inside bus `bus` moving from cabin point `local` by `step` (bus frame,
    /// metres): kept within a corridor round the cabin's own path network (the aisles,
    /// the door areas, the space by the driver) and on its floor. Gives the new cabin point
    /// and where that is in the world now.
    pub fn cabin_walk(&self, bus: BusId, local: Vec3, step: glam::Vec2) -> Option<(Vec3, DVec3)> {
        const WIDTH: f32 = 0.3;
        // what one step can climb, m
        const STEP_UP: f32 = 0.6;
        let bn = self.last_buses.iter().find(|b| b.id == bus)?;
        let pts = &bn.cabin.graph.points;
        let want = glam::Vec2::new(local.x + step.x, local.y + step.y);
        let mut best: Option<(f32, glam::Vec2, f32)> = None;
        for &(a, b, _) in &bn.cabin.links {
            let (Some(pa), Some(pb)) = (pts.get(a.max(0) as usize), pts.get(b.max(0) as usize)) else { continue };
            let (a2, b2) = (pa.truncate(), pb.truncate());
            let ab = b2 - a2;
            let t = if ab.length_squared() > 1e-6 { ((want - a2).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            let q = a2 + ab * t;
            // (the height tells the decks of a double-decker apart, not the steps: counted
            // in full, a staircase rising from its first centimetre lost to the aisle beside
            // it and the corridor held the walker at the aisle - an invisible wall at the
            // foot of the stairs)
            let d = (want - q).length() + ((local.z - (pa.z + (pb.z - pa.z) * t)).abs() - STEP_UP).max(0.0) * 2.0;
            if best.map(|x| d < x.0).unwrap_or(true) {
                best = Some((d, q, pa.z + (pb.z - pa.z) * t));
            }
        }
        if best.is_none() {
            for pt in pts {
                let d = (want - pt.truncate()).length();
                if best.map(|x| d < x.0).unwrap_or(true) {
                    best = Some((d, pt.truncate(), pt.z));
                }
            }
        }
        let (d, q, z) = best?;
        let xy = if d > WIDTH { q + (want - q) / d * WIDTH } else { want };
        // not through the seats and the driver's place: no nearer to one than 0.38 m
        // (walking away from one that close is let be)
        // (on the walker's deck: the seats under a staircase, or the upper deck's over the
        // aisle below, stopped the walker where nothing stands)
        let from = local.truncate();
        let solid: Vec<glam::Vec2> = bn.cabin.seats.iter().filter(|s| s.seated && (s.floor.z - local.z).abs() < 0.45).map(|s| s.pos.truncate()).chain(bn.cabin.data.driver_positions.iter().filter(|d| (d.pos[2] - (local.z + 0.4)).abs() < 1.0).map(|d| glam::Vec2::new(d.pos[0], d.pos[1]))).collect();
        let Some(xy) = clear_of_seats(from, xy, &solid) else { return Some((local, bn.world(local))) };
        let l = Vec3::new(xy.x, xy.y, z);
        Some((l, bn.world(l)))
    }

    /// The doors of bus `bus`: the threshold in the cabin, where one stands outside (world),
    /// which side of the bus (+1 right) and whether it is open now.
    pub fn cabin_doors(&self, bus: BusId) -> Vec<(Vec3, DVec3, f32, bool)> {
        let Some(bn) = self.last_buses.iter().find(|b| b.id == bus) else { return Vec::new() };
        let (eo, xo) = bn.walk_open.as_ref().map(|w| (&w.0, &w.1)).unwrap_or((&bn.entry_open, &bn.exit_open));
        let entries = bn.cabin.entries.iter().enumerate().map(|(k, d)| (d, eo.get(k).copied().unwrap_or(false)));
        let exits = bn.cabin.exits.iter().enumerate().map(|(k, d)| (d, xo.get(k).copied().unwrap_or(false)));
        entries.chain(exits).map(|(d, open)| (d.inside, bn.world(d.outside), d.side, open)).collect()
    }

    /// The buses of the last tick within `r` of `at`, the own first.
    pub fn bus_ids_near(&self, at: DVec3, r: f64) -> Vec<BusId> {
        let mut v: Vec<(BusId, f64)> = self
            .last_buses
            .iter()
            .map(|b| {
                // The front origin can be more than 25 m from the last section of a
                // biarticulated bus: the nearest of its sections and its doors as they stand,
                // on curves too.
                let doors = b.cabin.entries.iter().chain(&b.cabin.exits).map(|door| b.world(door.outside));
                let distance = std::iter::once(b.pos)
                    .chain(b.trailers.iter().map(|part| part.pos))
                    .chain(doors)
                    .map(|pos| (pos - at).truncate().length())
                    .fold(f64::INFINITY, f64::min);
                (b.id, distance)
            })
            .filter(|x| x.1 < r)
            .collect();
        v.sort_by(|a, b| (a.0 != BusId::Player).cmp(&(b.0 != BusId::Player)).then(a.1.total_cmp(&b.1)));
        v.into_iter().map(|x| x.0).collect()
    }

    /// Where the cabin point `local` of bus `bus` is in the world now, and the bus's heading.
    pub fn cabin_world(&self, bus: BusId, local: Vec3) -> Option<(DVec3, f64)> {
        let bn = self.last_buses.iter().find(|b| b.id == bus)?;
        Some((bn.world(local), bn.heading))
    }

    /// The free seat of bus `bus` nearest the world point `at` (for a walker inside it).
    pub fn seat_nearest(&self, bus: BusId, at: DVec3, reach: f64) -> Option<usize> {
        let bn = self.last_buses.iter().find(|b| b.id == bus)?;
        let taken = self.seats.get(&bn.id);
        bn.cabin
            .seats
            .iter()
            .enumerate()
            .filter(|(k, s)| s.seated && !taken.and_then(|t| t.get(*k)).copied().unwrap_or(false))
            .map(|(k, s)| (k, (bn.world(s.floor) - at).truncate().length()))
            .filter(|(_, d)| *d < reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// The cabin path point nearest seat `seat` of bus `bus` (where one stands up to).
    pub fn seat_stand(&self, bus: BusId, seat: usize) -> Option<Vec3> {
        let bn = self.last_buses.iter().find(|b| b.id == bus)?;
        let s = bn.cabin.seats.get(seat)?;
        // (on the seat's own deck: in a double-decker the nearest point in plan could be
        // the one straight above or below it)
        let d = |a: &Vec3| (a.truncate() - s.floor.truncate()).length() + (a.z - s.floor.z).abs() * 3.0;
        bn.cabin.graph.points.iter().copied().min_by(|a, b| d(a).total_cmp(&d(b)))
    }

    /// Cabin point `local` of vehicle `v` in the world (before the buses' first tick).
    pub fn vehicle_cabin_world(&mut self, v: &VehicleInstance, local: Vec3) -> Option<DVec3> {
        let cabin = self.cabin_for(v)?;
        let trailers = part_frames(v, &cabin);
        Some(train_point(v.position, &v.body_rotation(), &trailers, local))
    }

    /// Where the driver stands up in vehicle `v`'s cabin: the cabin's path point nearest the
    /// driver's seat (bus frame).
    pub fn driver_stand(&mut self, v: &VehicleInstance) -> Option<Vec3> {
        let cabin = self.cabin_for(v)?;
        let seat = cabin.data.driver_positions.first().map(|d| Vec3::from(d.pos)).unwrap_or(Vec3::new(-0.8, 4.5, 1.0));
        // the driver's position is the hip, half a metre over the cab floor: a double
        // decker's upper deck lies straight over the cab and was as near in plan, and the
        // driver who got up stood in the roof over the windscreen
        let d = |a: &Vec3| (a.truncate() - seat.truncate()).length() + (a.z - (seat.z - 0.5)).abs() * 3.0;
        cabin.graph.points.iter().copied().min_by(|a, b| d(a).total_cmp(&d(b)))
    }

    /// How many people are in (or boarding, riding, leaving) bus `bus`.
    pub fn people_in(&self, bus: BusId) -> usize {
        self.people.iter().filter(|p| matches!(p.place, Place::Bus(b, _) if b == bus) || p.state.bus() == Some(bus)).count()
    }

    /// Where bus `bus` stands (its origin), as of the last tick.
    pub fn bus_center(&self, bus: BusId) -> Option<DVec3> {
        self.last_buses.iter().find(|b| b.id == bus).map(|b| b.pos)
    }

    /// Is `bus` among the buses of the last tick?
    pub fn bus_here(&self, bus: BusId) -> bool {
        self.last_buses.iter().any(|b| b.id == bus)
    }

    /// The player's (or another player's) body on foot, animated as Omsi.exe animates its
    /// people: sitting on a seat (its hip on the `[passpos]`), walking or standing.
    pub fn animate_avatar(&mut self, i: usize, dt: f32, world: &dyn World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let id = self.people[i].id;
        let Some(key) = self.avatars.iter().find(|(_, v)| **v == id).map(|(k, _)| *k) else { return };
        let Some(cmd) = self.avatar_cmds.get(&key).copied() else { return };
        let dt_ms = dt * 1000.0;
        let seated = cmd.seat.and_then(|(b, k)| {
            let bn = bus_ix.get(&b).map(|x| &buses[*x])?;
            let s = bn.cabin.seats.get(k)?.clone();
            Some((b, s, bn))
        });
        let seatheight = self.people[i].ty.def.seat_height;
        let p = &mut self.people[i];
        let input = match seated {
            Some((b, s, bn)) => {
                // on the seat, in its bus's frame (set_task(7): the feet the human's seat
                // height under the seat point, facing the way the seat does)
                let l = if s.seated { s.pos - Vec3::Z * seatheight } else { s.pos };
                p.place = Place::Bus(b, l);
                p.lheading = s.rot as f64;
                p.position = bn.world(l);
                p.tilt = bn.tilt_at(l);
                p.heading = bn.heading_at(l) + p.lheading;
                p.interior = bn.interior;
                p.vel = DVec2::ZERO;
                p.activity = if s.seated { Activity::Sit } else { Activity::Stand };
                AnimInput { kind: if s.seated { 2 } else { 0 }, seat_height: s.height, room_height: pax::OUTSIDE_ROOM, dt_ms, ..Default::default() }
            }
            None if cmd.aboard.is_some_and(|(b, _)| bus_ix.contains_key(&b)) => {
                let (b, l) = cmd.aboard.unwrap();
                let bn = &buses[bus_ix[&b]];
                let bh = bn.heading_at(l);
                p.place = Place::Bus(b, l);
                p.lheading = wrap_heading(cmd.heading - bh);
                p.position = bn.world(l);
                p.tilt = bn.tilt_at(l);
                p.heading = cmd.heading;
                p.interior = bn.interior;
                p.vel = cmd.vel;
                let v = cmd.vel.length() as f32;
                p.activity = if v > 0.05 { Activity::Walk } else { Activity::Stand };
                AnimInput { kind: (v > 0.05) as u8, speed: v, moved: v * dt, room_height: pax::OUTSIDE_ROOM, dt_ms, ..Default::default() }
            }
            None => {
                p.place = Place::Ground;
                p.tilt = Mat4::IDENTITY;
                p.interior = 0.0;
                let ground = cmd.floor.unwrap_or_else(|| world.walk_height(cmd.pos.x, cmd.pos.y).unwrap_or(cmd.pos.z));
                let origin = DVec3::new(cmd.pos.x, cmd.pos.y, if cmd.floor.is_some() { ground } else { cmd.pos.z.max(ground) } + cmd.lift.max(0.0));
                p.position = origin;
                p.heading = cmd.heading;
                p.vel = cmd.vel;
                let v = cmd.vel.length() as f32;
                p.activity = if v > 0.05 { Activity::Walk } else { Activity::Stand };
                AnimInput { kind: (v > 0.05) as u8, speed: v, moved: v * dt, room_height: pax::OUTSIDE_ROOM, dt_ms, ..Default::default() }
            }
        };
        p.anim.advance(&p.ty.omsi, &input);
    }
}
