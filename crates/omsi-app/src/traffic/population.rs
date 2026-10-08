//! The population around the player: where cars may appear and vanish unseen, putting
//! new ones on the road and taking far ones off.

use super::*;
use crate::scene::World;
use omsi_render::{Renderer, Scene};

/// A car that gave up and has stood this long (s) held by nothing anybody can see goes even
/// in view (see `Traffic::populate_seen`).
pub(super) const PHANTOM_WAIT: f32 = 90.0;


/// Ground height for an AI vehicle's wheels: the road surface (blended between raster
/// texels), else any surface, else the terrain.
pub(super) fn ai_ground(world: &World) -> Arc<dyn Fn(f64, f64) -> Option<f64> + Send + Sync> {
    let terrains = world.terrains.clone();
    let surfaces = world.surfaces.clone();
    Arc::new(move |x, y| {
        let tx = (x / omsi_map::tile_size()).floor() as i32;
        let ty = (y / omsi_map::tile_size()).floor() as i32;
        let lx = (x - tx as f64 * omsi_map::tile_size()) as f32;
        let ly = (y - ty as f64 * omsi_map::tile_size()) as f32;
        let surface = surfaces.read().get(&(tx, ty)).cloned();
        if let Some(h) = surface.and_then(|s| s.sample_road_smooth(lx, ly)) {
            return Some(h as f64);
        }
        let t = terrains.read();
        Some(t.get(&(tx, ty))?.sample(lx, ly) as f64)
    })
}

/// The bare ground at a point (no roads, decks or platforms on it).
pub(super) fn terrain_height(world: &World, x: f64, y: f64) -> Option<f64> {
    let tx = (x / omsi_map::tile_size()).floor() as i32;
    let ty = (y / omsi_map::tile_size()).floor() as i32;
    let t = world.terrains.read();
    let terrain = t.get(&(tx, ty))?;
    Some(terrain.sample(
        (x - tx as f64 * omsi_map::tile_size()) as f32,
        (y - ty as f64 * omsi_map::tile_size()) as f32,
    ) as f64)
}

impl Traffic {
    /// Is a vehicle of radius `r` at `p` hidden from the viewer by buildings (or a hill)?
    /// Every line of sight to it must be: to its middle, to both ends whichever way it
    /// points and over its roof. A single line to the middle let a car come and go half
    /// out from behind a corner, in plain view.
    pub(super) fn occluded(&self, world: &World, v: &Viewer, p: DVec3, r: f64) -> bool {
        let rel = (p - v.pos).truncate();
        let across = if rel.length() > 1e-3 {
            DVec3::new(-rel.y, rel.x, 0.0).normalize()
        } else {
            DVec3::X
        };
        let along = DVec3::new(rel.x, rel.y, 0.0).normalize_or_zero();
        let reach = (r * 0.8).max(1.5);
        let mut targets = vec![
            p + DVec3::new(0.0, 0.0, 1.2),
            p + across * reach + DVec3::new(0.0, 0.0, 1.0),
            p - across * reach + DVec3::new(0.0, 0.0, 1.0),
            p - along * reach + DVec3::new(0.0, 0.0, 1.0),
        ];
        // the roof of a bus or a lorry shows over a wall a car hides behind
        if r > 4.0 {
            targets.push(p + DVec3::new(0.0, 0.0, 3.2));
        }
        targets.into_iter().all(|t| self.sight_blocked(world, v, t))
    }

    /// Is the line of sight from the viewer to the point `target` blocked by a building
    /// (or a hill)?
    pub(super) fn sight_blocked(&self, world: &World, v: &Viewer, target: DVec3) -> bool {
        let p = target;
        let blocker = match &self.sim.occluders {
            Some(c) => c.ray_blocker(v.pos, target, 2.5, 3.0),
            None => world.collision.lock().ray_blocker(v.pos, target, 2.5, 3.0),
        };
        if let Some(b) = blocker {
            if self.sim.debug_population {
                log::info!("  line of sight to ({:.0}, {:.0}) blocked by a box at ({:.1}, {:.1}) {:.1} x {:.1} m, z {:.1}..{:.1}", p.x, p.y, b.center.x, b.center.y, b.half.x * 2.0, b.half.y * 2.0, b.z0, b.z1);
            }
            return true;
        }
        // the ground between: a crest or an embankment
        for k in 1..8 {
            let t = k as f64 / 8.0;
            let q = v.pos.lerp(target, t);
            if let Some(g) = terrain_height(world, q.x, q.y) {
                if g > q.z + 0.5 {
                    if self.sim.debug_population {
                        log::info!("  line of sight to ({:.0}, {:.0}) blocked by the ground at ({:.0}, {:.0}): {:.1} over {:.1}", p.x, p.y, q.x, q.y, g, q.z);
                    }
                    return true;
                }
            }
        }
        false
    }

    /// May a vehicle be put on the road at `p` without the player seeing it appear? A
    /// timetable bus asks this before it spawns mid-route.
    pub fn may_appear(&self, world: &World, p: DVec3) -> bool {
        self.sim.initial || self.hidden(world, p, 8.0)
    }

    /// The world is still being built (the first populate, the first seconds): vehicles
    /// may be put anywhere.
    pub fn loading_phase(&self) -> bool {
        self.sim.initial || self.sim.time < 2.0
    }

    /// Could the player not see a vehicle (radius `r`) at `p` appear or vanish? Never close
    /// by: mirrors, a turn of the head and the gaps between houses see what is near,
    /// whatever the collision boxes say (a bus let appear 40 m away behind a box that stood
    /// for a building with a gateway in it was seen popping up in the middle of the street).
    /// Within `NEAR_HIDE` only behind a building or the ground, wherever the camera looks -
    /// the mirrors look back, and the head turns: buses appeared 160 m behind the player
    /// in plain sight of the mirrors, and a bus waiting at the edge of the loaded route
    /// vanished beside the player's bus because the camera was looking ahead. Further off:
    /// beyond what is drawn, out of the picture, or behind something (`Viewer::hides`). A
    /// LAN host asks it of every other player's bus too (`TrafficSim::unseen`).
    pub fn hidden(&self, world: &World, p: DVec3, r: f64) -> bool {
        self.sim.unseen(p, r, |v| self.occluded(world, v, p, r))
    }

    /// Spawn cars until `target` are within `spawn_radius` of `center`; despawn far ones.
    pub fn populate(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
    ) {
        self.populate_seen(view, world, renderer, scene, center, None);
    }

    /// Keep the population around the player: cars are taken off only where nobody sees
    /// it (far away and out of view, or behind a building), and new ones appear only there.
    /// `facing` (a unit vector) stands in for the viewer when none has been set.
    pub fn populate_seen(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
        facing: Option<DVec3>,
    ) {
        if self.sim.mirror {
            return;
        }
        if self.sim.viewer.is_none() {
            if let Some(f) = facing {
                self.sim.viewer = Some(Viewer {
                    pos: center,
                    forward: f,
                    tan_x: 1.2,
                    tan_y: 0.6,
                    range: VISIBLE_RANGE,
                    min_size: 0.0,
                    max_dist: 0.0,
                    fov: 1.0,
                });
            }
        }
        let far = self.sim.spawn_radius * DESPAWN_FACTOR;
        self.advance_dormant();
        // the cars somebody stands behind
        let queued: std::collections::HashSet<u64> = self.sim.cars.iter().filter(|c| c.stopped > 5.0).filter_map(|c| c.lead_car).collect();
        let mut i = 0;
        let mut off_ground = 0usize;
        let mut asleep = 0usize;
        while i < self.sim.cars.len() {
            let c = &self.sim.cars[i];
            let p = c.vehicle.position;
            // (the nearest player: a LAN host keeps the traffic around the others too)
            let dist = self
                .lan_centers
                .iter()
                .fold((p - center).length(), |d, o| d.min((p - *o).length()));
            // every car on the road or the rails whose ground has been unloaded goes: the
            // lanes stay in the network when their tile goes, but nothing may drive over
            // ground that is not there (tiles only go well beyond the view, timetable buses
            // included; their trips come back with the tiles)
            let flying = self
                .net
                .lanes
                .get(c.state.lane)
                .map(|l| l.kind == LaneKind::Air)
                .unwrap_or(false);
            let unloaded = !flying && !world.has_ground(p.x, p.y);
            off_ground += unloaded as usize;
            let random = !c.is_bus() || c.gone;
            let r = (c.state.length as f64 * 0.5).max(2.0);
            // standing at the end of the network (the map's edge): Omsi.exe never lets a
            // random car stand there - once 0x71dc9c finds no next segment (0x612e10) its
            // segment stays -1 and 0x6fe3fc deletes it the same frame (0x703bb0); a bus
            // that gave up goes once out of sight, or too far off for the renderer to draw it
            let at_end = c.gone
                && c.stopped > if c.is_bus() { 20.0 } else { 0.5 }
                && c.state.route.is_empty()
                && self.sim.net.lanes[c.state.lane].next.is_empty();
            // (the other players of a LAN session look too)
            let from_eye = self.sim.nearest_eye(p).unwrap_or(dist);
            // a timetable bus waiting where the loaded part of its route ends
            let at_edge = c.route_open()
                && c.state.speed < 0.1
                && c.state.route.last() == Some(&c.state.lane)
                && c.state.s > self.sim.net.lanes[c.state.lane].length() - 25.0;
            // a random car still on its way that goes out of range sleeps instead (see
            // `DormantCar`); only one whose trip is over leaves the map
            let sleeps_instead = random && !c.gone && !c.is_bus() && self.sim.net.lanes.get(c.state.lane).map(|l| l.kind == LaneKind::Street).unwrap_or(false);
            let remove = if unloaded {
                true
            } else if at_edge {
                // (its trip comes back with the tiles; kept until nobody saw it, a bus stood
                // with its passengers at the far end of a straight road for good)
                self.hidden(world, p, r) || (c.stopped > 8.0 && from_eye > 180.0) || c.stopped > 150.0
            } else if !random {
                // a timetable bus standing for minutes away from its stops and lights is in a
                // jam that does not clear: it goes once nobody sees it, as one at the edge
                // (kept for good, the jam behind it never cleared)
                c.is_bus() && c.stopped > 240.0 && !c.at_station() && !c.light_hold && (from_eye > 40.0 || self.hidden(world, p, r))
            } else if at_end && (!c.is_bus() || self.hidden(world, p, r) || (c.stopped > 8.0 && from_eye > 180.0) || c.stopped > 150.0 || (c.stopped > 25.0 && queued.contains(&c.id) && from_eye > 25.0)) {
                // (and in view too once others wait behind it: a fire engine at the end of a
                // dead-end street held a queue of fourteen cars for two and a half minutes)
                // (taken at once it vanished in plain view 300 m ahead; but a car kept
                // until nobody could see it stood for good at the end of a long straight
                // road in view, and the queue behind it - timetable buses with their
                // passengers among them - never moved again)
                true
            } else if c.gone || dist > far {
                // in plain view a car stays until the renderer leaves it out anyway, and
                // close by (the mirrors, a turn of the head) it stays in any case
                // (one that gave up in a gridlock goes after four minutes even in view,
                // unless right beside the viewer: kept until nobody saw it, a jam at a
                // junction the player watched never cleared; and one held for a minute and
                // a half by nothing anybody can see - no car, bus or light in front of it,
                // nobody it gives way to - goes then: whatever held it, it stood against
                // an invisible wall with the traffic queued up behind it for as long as the
                // player looked)
                (dist > VISIBLE_RANGE * 1.3 && from_eye > NEAR_HIDE)
                    || self.hidden(world, p, r)
                    || (c.gone && c.stopped > 240.0 && from_eye > 40.0)
                    || (c.gone && c.progress.1 > PHANTOM_WAIT && matches!(c.why.0, "" | "parked" | "people") && from_eye > 25.0)
            } else {
                false
            };
            if remove && sleeps_instead && !at_end {
                let c = self.sim.cars.swap_remove(i);
                asleep += 1;
                self.sim.dormant.push(DormantCar {
                    id: c.id,
                    ty: c.vehicle.ty.clone(),
                    kind: LaneKind::Street,
                    lane: c.state.lane,
                    s: c.state.s,
                    speed: c.state.speed.max(2.0),
                    seed: c.seed,
                    scheme: c.scheme,
                    walk: c.seed ^ c.id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
                });
                self.drop_sounds(c.id);
                view.release_car(world, renderer, scene, c.id);
            } else if remove {
                let c = self.sim.cars.swap_remove(i);
                if c.is_bus() && (unloaded || at_edge) {
                    self.sim.removed_scheduled.push(c.id);
                }
                if self.sim.debug_population {
                    let v = self.sim.viewer;
                    log::info!("population t={:.1}: car {} removed at ({:.0}, {:.0}), {:.0} m from the player, in frame {}, behind a building {}, {}", self.sim.time, c.id, p.x, p.y, dist, v.map(|v| v.frames(p, r)).unwrap_or(false), v.map(|v| self.occluded(world, &v, p, r)).unwrap_or(false), if c.gone { "finished" } else { "far away" });
                }
                self.drop_sounds(c.id);
                view.release_car(world, renderer, scene, c.id);
            } else {
                i += 1;
            }
        }
        if off_ground > 0 && omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
            log::info!("traffic: {off_ground} vehicles taken away with the tiles under them");
        }
        if (asleep > 0 || !self.sim.dormant.is_empty()) && self.sim.debug_population {
            log::info!("population t={:.1}: {asleep} cars went out of range and drive on unseen; {} on the map out of range, {} in range", self.sim.time, self.sim.dormant.len(), self.sim.cars.len());
        }
        if self.sim.types.is_empty() || self.sim.net.lanes.is_empty() {
            self.sim.initial = false;
            return;
        }
        // made only for the lights: nothing new while the target is 0, but the cars of a
        // target raised and lowered again go as they do anywhere (returning before the loop
        // above, they stood at the map's edge and drove over unloaded tiles for good)
        if self.sim.lights_only && self.sim.target == 0 {
            return;
        }
        // aircraft: a few on the flight paths, independent of the street target
        let has_air = self.sim.types.iter().any(|t| t.2 == LaneKind::Air);
        // the map's traffic density by hour (and group) scales the street traffic ...
        let density = self.street_density();
        // ... and so does how much road there is around: the same number of cars looks
        // empty on a six-lane Berlin junction and crowded on a village lane, so the count
        // asked for is per a neighbourhood of about 250 lanes; and as Omsi spawns on each
        // path at a rate of its [rule] trafficdensity, paths of low density bring fewer cars
        // and those of density 0 (or kept clear of cars) none
        let near_density: Vec<f32> = self.sim.net.lanes_starting_near(center, self.sim.spawn_radius)
            .into_iter()
            .map(|i| &self.sim.net.lanes[i])
            .filter(|l| {
                l.kind == LaneKind::Street
                    && l.points
                        .first()
                        .map(|p| (*p - center).length() < self.sim.spawn_radius)
                        .unwrap_or(false)
            })
            .map(|l| if l.no_cars { 0.0 } else { l.density.clamp(0.0, 4.0) })
            .collect();
        let street_target = (self.sim.target as f32 * density * road_scale(&near_density)).round() as usize;
        // the cars that come into range again, where they have got to
        self.wake_dormant(view, world, renderer, scene, center, street_target);
        // the whole map's population: as dense as around the player, on every street the
        // map has shown so far (sleeping where the player is not)
        self.fill_map(center, street_target);
        for (kind, target) in [
            (LaneKind::Street, street_target),
            (LaneKind::Air, if has_air { 3 } else { 0 }),
        ] {
            // (a LAN host counts the cars round itself only: counted over the whole map,
            // the traffic it keeps round the other players met its own target and the
            // host drove through empty streets, #342)
            if kind == LaneKind::Street && !self.sim.lan_centers.is_empty() {
                self.sim.count_near = Some((center, self.sim.spawn_radius));
            }
            self.populate_kind(view, world, renderer, scene, center, kind, target);
        }
        self.populate_lan_centers(view, world, renderer, scene, center, street_target);
        if !self.sim.initial {
            self.pull_out_parked(view, world, renderer, scene, center);
            self.park_in(world, center);
        }
        self.sim.initial = false;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn populate_kind(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
        kind: LaneKind,
        target: usize,
    ) {
        let radius = if kind == LaneKind::Air {
            self.sim.spawn_radius * 6.0
        } else {
            self.sim.spawn_radius
        };
        let nearby = self.sim.net.lanes_starting_near(center, radius);
        // candidate lanes of this kind near the centre
        let pick = |through: bool| -> Vec<(usize, f32)> {
            nearby.iter().copied()
                .map(|i| (i, &self.sim.net.lanes[i]))
                .filter(|(_, l)| {
                    l.kind == kind
                        && l.length() > 8.0
                        && (l.start() - center).truncate().length() < radius
                })
                // lanes the map keeps clear of cars, and those whose [rule] trafficdensity is
                // zero, are not spawned on at all; a lower density makes a lane that much less
                // likely to be picked
                .filter(|(_, l)| !l.no_cars && l.density > 0.0)
                // nor, where there are others, lanes that end the network just ahead (the car
                // would only drive into the end and wait there to be taken away)
                .filter(|(i, _)| {
                    !through
                        || kind != LaneKind::Street
                        || self
                            .net
                            .reach
                            .get(*i)
                            .map(|r| *r >= omsi_sim::traffic::DEAD_END)
                            .unwrap_or(true)
                })
                // as many cars on a lane as metres of it (times its density): counted per
                // lane, the many short lanes of a junction drew the cars into the town's
                // tangles and left the long roads between them empty
                .map(|(i, l)| (i, l.length() * l.density.clamp(0.05, 4.0)))
                .collect::<Vec<(usize, f32)>>()
        };
        let mut candidates = pick(true);
        if candidates.is_empty() {
            candidates = pick(false);
        }
        if candidates.is_empty() {
            return;
        }
        let mut acc = 0.0f32;
        let cumulative: Vec<f32> = candidates.iter().map(|c| { acc += c.1; acc }).collect();
        let total_w = acc.max(1e-3);
        let mut attempts = 0;
        let counted_near = self.sim.count_near.take();
        let unscheduled = self
            .cars
            .iter()
            .filter(|c| {
                !c.is_bus()
                    && !c.gone
                    && counted_near
                        .map(|(p, r)| (c.vehicle.position - p).length() < r)
                        .unwrap_or(true)
                    && self
                        .net
                        .lanes
                        .get(c.state.lane)
                        .map(|l| l.kind == kind)
                        .unwrap_or(false)
            })
            .count();
        let mut count = unscheduled;
        while count < target && attempts < target * 12 {
            attempts += 1;
            let x = self.rand_f() as f32 * total_w;
            let lane = candidates[cumulative.partition_point(|&c| c < x).min(candidates.len() - 1)].0;
            let s = (self.rand_f() * (self.sim.net.lanes[lane].length() as f64 - 6.0)) as f32 + 3.0;
            let (p, _) = self.sim.net.lanes[lane].at(s);
            let rel = p - center;
            if rel.length() < 40.0 {
                continue; // not right next to the player
            }
            // nobody may see it appear (the first population is the world as it loads)
            if kind == LaneKind::Street && !self.sim.initial && !self.may_appear(world, p) {
                continue;
            }
            if self
                .cars
                .iter()
                .any(|c| (c.vehicle.position - p).length() < 14.0)
            {
                continue;
            }
            // only on loaded ground (a lane's tile may have gone again)
            if kind != LaneKind::Air && !world.has_ground(p.x, p.y) {
                continue;
            }
            let heading = self.sim.net.lanes[lane].at(s).1 as f64;
            // nor just in front of one driving up to that place (it would have to stop hard)
            let in_front_of_someone = self.sim.cars.iter().any(|c| {
                let rel = p - c.vehicle.position;
                let h = c.vehicle.heading.to_radians();
                let (along, across) = (
                    rel.x * h.sin() + rel.y * h.cos(),
                    (rel.x * h.cos() - rel.y * h.sin()).abs(),
                );
                along > 0.0
                    && along
                        < 20.0
                            + (c.state.speed * c.state.speed / (2.0 * c.state.decel.max(1.0)))
                                as f64
                                * 1.5
                    && across < 3.0
            });
            if in_front_of_someone {
                continue;
            }
            if self
                .parked
                .get(&lane)
                .map(|l| {
                    l.iter()
                        .any(|&(ps, lat)| (ps - s).abs() < 8.0 && lat.abs() < 1.5)
                })
                .unwrap_or(false)
            {
                continue; // not into a car parked in the lane
            }
            // (a lane may carry none of the groups that drive now: try another)
            let Some(ty) = self.pick_type(kind, Some(lane)) else {
                continue;
            };
            // (nor onto the rear section of an articulated bus, nor the player's bus)
            if kind != LaneKind::Air && !self.spawn_clear(&ty, p, heading) {
                continue;
            }
            let seed = self.rand();
            self.create_car(view, world, renderer, scene, center, kind, lane, s, ty, seed, None, None, None, None);
            count += 1;
        }
    }

    /// The cars out of range that have come near again take their bodies back - where
    /// nobody sees it happen, up to a little over the number asked for around the player.
    pub(super) fn wake_dormant(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene, center: DVec3, target: usize) {
        if self.sim.dormant.is_empty() {
            return;
        }
        let active = self.sim.cars.iter().filter(|c| !c.is_bus() && !c.gone).count();
        let mut budget = (target as f32 * 1.25).ceil() as usize;
        budget = budget.saturating_sub(active);
        let centers: Vec<DVec3> = std::iter::once(center).chain(self.sim.lan_centers.iter().copied()).collect();
        let mut i = 0;
        while i < self.sim.dormant.len() && budget > 0 {
            let (p, h, ty) = {
                let d = &self.sim.dormant[i];
                let l = &self.sim.net.lanes[d.lane];
                let (p, h) = l.at(d.s.clamp(0.0, (l.length() - 0.1).max(0.0)));
                (p, h as f64, d.ty.clone())
            };
            let near = centers.iter().any(|c| (p - *c).truncate().length() < self.sim.spawn_radius);
            // (as a new car: never close by, where the mirrors and a turn of the head see
            // it - woken out of the picture 30-60 m from the bus, a car came into being in
            // the mirror or just round the corner)
            let ok = near
                && world.has_ground(p.x, p.y)
                && self.may_appear(world, p)
                && !self.sim.cars.iter().any(|c| (c.vehicle.position - p).length() < 14.0)
                && self.spawn_clear(&ty, p, h);
            if ok {
                let d = self.sim.dormant.swap_remove(i);
                self.create_car(view, world, renderer, scene, center, d.kind, d.lane, d.s, d.ty, d.seed, Some(d.scheme), Some(d.id), Some(d.speed), None);
                budget -= 1;
            } else {
                i += 1;
            }
        }
    }

    /// Put a random car of type `ty` on `lane` at `s` metres into it and return its id:
    /// `scheme` Some = that paint scheme (a car coming back from out of range keeps its
    /// looks), `id` Some = that id, `speed` Some = at about that speed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_car(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
        kind: LaneKind,
        lane: usize,
        s: f32,
        ty: Arc<VehicleType>,
        seed: u64,
        scheme: Option<Option<usize>>,
        id: Option<u64>,
        speed: Option<f32>,
        bus: Option<BusSetup>,
    ) -> u64 {
        let mut host = omsi_sim::VehicleHost::new(omsi_sim::SimClock::default());
        host.font_lib = Some(world.fonts.clone());
        if let Some(b) = &bus {
            host.hof = b.hof.clone();
            b.timetable.install(&mut host, b.stops.first());
        }
        // random paint scheme / advert (its variables there for the scripts' {init})
        let scheme = match scheme {
            Some(s) => s,
            None if ty.paint_schemes.is_empty() => None,
            None => Some((seed >> 8) as usize % ty.paint_schemes.len().min(AI_SCHEMES)),
        };
        host.paint_scheme = Some(scheme);
        let mut vehicle = VehicleInstance::new(ty.clone(), host);
        if let Some((num, reg)) = bus.as_ref().and_then(|b| b.number.clone()) {
            if let Some(i) = ty.program.str_var("number") {
                vehicle.state.str_vars[i as usize] = num;
            }
            if let Some(i) = ty.program.str_var("ident") {
                vehicle.state.str_vars[i as usize] = reg;
            }
        } else {
            // A vehicle of the random traffic with a `[number]` list takes a number of it at
            // random and the plate beside it, else the plate its mode makes of the number
            // (TRoadVehicleInst.virtual_11 at 0x7e7b51); a free plate is one of the map's
            // registrations.txt.
            let numbers = ty.def.numbers_with_plates();
            if !numbers.is_empty() {
                let (n, plate) = &numbers[(seed.rotate_left(29) % numbers.len() as u64) as usize];
                if let Some(i) = ty.program.str_var("number") {
                    vehicle.state.str_vars[i as usize] = n.clone();
                }
                if ty.def.registration_mode != 1 {
                    if let Some(i) = ty.program.str_var("ident") {
                        vehicle.state.str_vars[i as usize] = if plate.is_empty() { ty.def.plate_of_number(n) } else { plate.clone() };
                    }
                }
            }
            if ty.def.registration_mode == 1 {
                if let (Some(i), Some(reg)) = (ty.program.str_var("ident"), world.free_registration(seed.rotate_left(17))) {
                    vehicle.state.str_vars[i as usize] = reg;
                }
            }
        }
        // aircraft keep the height of their flight path: a ground sampler would pull
        // them down onto the streets
        vehicle.ground = if kind == LaneKind::Air {
            None
        } else {
            Some(ai_ground(world))
        };
        // and what its wheels stand on, asked as the player's are (see `AiBody::settle`); a
        // coupled part (an articulated bus's rear, a lorry's trailer) asks it too, with the
        // height it is at - the plain sampler gave it the deck of a bridge over its road
        // (`OMSI_AI_WAY_ONLY=1`: on the way and the plain sampler, as before - A/B runs)
        vehicle.contact = (kind == LaneKind::Street && !omsi_cfg::flags::OMSI_AI_WAY_ONLY.is_set()).then(|| {
            std::sync::Arc::new(crate::scene::DriveGround {
                terrains: world.terrains.clone(),
                surfaces: world.surfaces.clone(),
            }) as std::sync::Arc<dyn omsi_sim::rigid::Ground>
        });
        vehicle.apply_paint_vars(scheme);
        let render = self.new_car_render(world, renderer, scene, &mut vehicle, &ty, scheme);
        if !ty.model.text_textures.is_empty() || bus.is_some() {
            vehicle.init_text_textures(&mut world.fonts.lock(), &|p| {
                omsi_texture::decode_file(p)
                    .ok()
                    .map(|i| (i.width, i.height, i.rgba))
            });
        }
        // a rear section's plates and numbers are `[texttexture]`s of its own reading the
        // leading vehicle's strings (`TrailerPart::update_text_textures`), so they need the
        // same fonts the front's do
        for t in vehicle.trailers.iter_mut() {
            t.init_text_textures(&mut world.fonts.lock(), &|p| {
                omsi_texture::decode_file(p)
                    .ok()
                    .map(|i| (i.width, i.height, i.rgba))
            });
        }
        let id = self.place_car(vehicle, kind, lane, s, ty, seed, scheme, id, speed, bus);
        if self.sim.debug_population && kind == LaneKind::Street {
            let v = self.sim.viewer;
            let pos = self.sim.cars[self.sim.cars.len() - 1].vehicle.position;
            if !self.sim.initial && v.map(|v| v.frames(pos, 2.5)).unwrap_or(false) {
                self.sim.framed_spawns.push((id, pos));
            }
            log::info!("population t={:.1}: car {id} appears at ({:.0}, {:.0}), {:.0} m from the centre, {:.0} m from the camera, in frame {}, behind a building {}{}", self.sim.time, pos.x, pos.y, (pos - center).length(), v.map(|v| (pos - v.pos).length()).unwrap_or(0.0), v.map(|v| v.frames(pos, 2.5)).unwrap_or(false), v.map(|v| self.occluded(world, &v, pos, 2.5)).unwrap_or(false), if self.sim.initial { " (initial)" } else { "" });
        }
        view.insert(id, render);
        id
    }

    /// The vehicle/paint sets the random traffic draws from.
    pub fn random_sets(&self) -> Vec<(Arc<VehicleType>, Option<usize>)> {
        let mut sets: Vec<(Arc<VehicleType>, Option<usize>)> = Vec::new();
        for (ty, ..) in &self.sim.types {
            let n = ty.paint_schemes.len().min(AI_SCHEMES);
            let schemes: Vec<Option<usize>> = if n == 0 { vec![None] } else { (0..n).map(Some).collect() };
            for scheme in schemes {
                if !sets.iter().any(|(t, s)| t.def.path == ty.def.path && *s == scheme) {
                    sets.push((ty.clone(), scheme));
                }
            }
        }
        sets
    }

    /// Put a timetable bus on the road: an AI car like any other (`create_car`), on its
    /// trip's route at `s` metres into the first lane, with its service (stops).
    /// Returns the car index. `scheme`: the paint scheme to use (Some), or a random one.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_bus(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        ty: Arc<VehicleType>,
        route: Vec<usize>,
        s: f32,
        stops: Vec<(usize, f32, f32, f64, i64, f32)>,
        number: Option<(String, String)>,
        hof: Option<Arc<omsi_vehicle::Hof>>,
        scheme: Option<Option<usize>>,
        timetable: crate::bus_service::AiTimetable,
    ) -> Option<usize> {
        let &lane = route.first()?;
        let kind = self.sim.net.lanes.get(lane)?.kind;
        // the options' [AIMaxCountScheduled]: no more timetable vehicles than that at once
        if self.sim.no_timetable_buses {
            return None;
        }
        if self.sim.max_scheduled > 0 && self.sim.cars.iter().filter(|c| c.is_bus() || !c.state.route.is_empty()).count() >= self.sim.max_scheduled as usize {
            return None;
        }
        let seed = self.rand();
        let setup = BusSetup {
            route,
            stops: stops.into_iter().map(crate::bus_service::Stop::from_tuple).collect(),
            number,
            hof,
            timetable,
        };
        let center = self.sim.viewer.map(|v| v.pos).unwrap_or_default();
        let id = self.create_car(view, world, renderer, scene, center, kind, lane, s, ty.clone(), seed, scheme, None, None, Some(setup));
        let ci = self.sim.cars.iter().rposition(|c| c.id == id)?;
        if kind == LaneKind::Air {
            let p = self.sim.cars[ci].vehicle.position;
            let ground = world
                .ground_height(p.x, p.y)
                .map(|g| format!("{:.0} m above the ground", p.z - g))
                .unwrap_or_else(|| "over unloaded ground".into());
            log::info!("aircraft {} on its flight path at ({:.0}, {:.0}), height {:.0} m, {ground}, {:.0} km/h", ty.def.path.file_name().unwrap_or_default().to_string_lossy(), p.x, p.y, p.z, self.sim.cars[ci].state.speed * 3.6);
        }
        Some(ci)
    }

    /// Street traffic around the other players of a LAN session too (host): each of them
    /// gets its own share of cars where no other player's share lies already.
    pub(super) fn populate_lan_centers(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
        target: usize,
    ) {
        let centers = self.sim.lan_centers.clone();
        let mut done = vec![center];
        for c in centers {
            if done
                .iter()
                .any(|d| (*d - c).truncate().length() < self.sim.spawn_radius)
            {
                continue;
            }
            self.sim.count_near = Some((c, self.sim.spawn_radius));
            self.populate_kind(view, world, renderer, scene, c, LaneKind::Street, target);
            self.sim.count_near = None;
            done.push(c);
        }
    }
}
