//! People on foot: passengers and pedestrians as agents with a goal.
//!
//! A passenger comes along the pavement (or already stands at the stop when the map
//! starts), waits at a free waiting place of the stop - the `[passpos]` points of the
//! map's `people_standing_*` markers and shelters, else places spread along the back of
//! the platform - and when a bus opens its doors there, queues at the nearest open
//! `[entry]` (a passenger who still has to buy a ticket only at one with a cash desk),
//! steps in when the doorway is free, pays or shows a pass at the desk, walks the cabin's
//! `paths.cfg` network to a free `[passpos]` (a standing place once the seats are gone),
//! rides, presses the stop button before their stop, walks to the nearest `[exit]` when
//! the bus stands there, steps out and walks away along the pavement - or waits at the
//! stop for another bus. Timetable (AI) buses carry their passengers the same way.
//! Nobody is taken away while the player can see them.
//!
//! An articulated bus is one cabin: the sections' path networks, seats and exits are put
//! together in the front section's frame with the sections straight behind each other, and
//! the front section's `[linkToPrevVeh]` point is joined to the rear section's
//! `[linkToNextVeh]` point, so people walk through the bellows to the seats and exits at the
//! back. Entries and exits are numbered front section first, which is how the stock door
//! scripts count them (the GN92's rear door is `PAX_Exit2`/`PAX_Exit3`). A point behind a
//! joint is carried by its own section, whatever the angle of the bend.
//!
//! Movement is a crowd: everybody on the same floor (the ground, or one bus) avoids
//! everybody else with the anticipatory model of `omsi_sim::crowd`, does not push into
//! somebody standing in front, speeds up, slows down and turns at a human pace, and keeps
//! to the aisle inside a bus. Doorways and the cash desk are taken one at a time, people
//! getting off go first, and somebody pressed against another for seconds slips past.
//! Every waiting state has a way out, and `OMSI_DEBUG_PAX=1` logs every change of state
//! and why somebody stands still.
//!
//! Pedestrians walk the map's pavement paths as one network (path ends that meet are
//! joined whatever their heading), wait at the kerb for a pedestrian light's green - and
//! only start across when it lasts long enough - and for approaching cars where there is
//! no light; nobody stops in the middle of the road.
//!
//! The map streams: stops, waiting places and pavements come with their tiles. The
//! pavement network grows as the traffic network does, a stop is set up again when its
//! neighbourhood changed and nobody uses it, a stop whose tile went takes its people with
//! it, and nobody stands or walks where the ground is not loaded.

use crate::ambience;
use crate::scene::World;
use crate::traffic::Traffic;
use glam::{DVec2, DVec3, Mat4, Vec2, Vec3};
use hashbrown::{HashMap, HashSet};
use omsi_render::{AlphaMode, Camera, MaterialId, MeshId, Renderer, Scene};
use omsi_sim::crowd::{self, Block, CrowdParams, PathGraph, Walker};
use omsi_sim::human::{skin, Activity, HumanType, Pose, PoseInput};
use omsi_sim::traffic::{LaneKind, Network};
use omsi_sim::VehicleInstance;
use omsi_vehicle::PassengerCabin;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Walking pace inside a bus (m/s): people are careful on a bus floor.
/// The "stop" of a player's bus standing with a door open that everybody leaves: its
/// driver has got up, or it is not in service. No waiting place belongs to it.
/// The map's traffic keeps left (its stops are on the left): see the doors of `Cabin`.
pub(crate) static LEFT_HAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const ALL_OUT_STOP: i64 = -7;
const PACE_IN: f64 = 0.9;
/// Gap between two people in a queue (m).
const QUEUE_GAP: f64 = 0.62;
/// How far outside the bus side somebody stands at a door (m).
const DOOR_OUT: f32 = 0.5;
/// Longest a passenger waits at the cash desk for the driver in `pay` boarding (s): then
/// they show a pass and walk on.
const PAY_PATIENCE: f32 = 18.0;
/// Body radius for the crowd inside a bus (m): aisles are narrow, people brush past.
const BODY: f64 = 0.23;
/// Body radius for the crowd outside (m): shoulders and swinging arms. With the cabin's
/// radius people on the pavement came within 0.46 m, and two walking past each other or a
/// group crossing the road merged into one another in the picture.
const BODY_OUTSIDE: f64 = 0.28;
/// How far away a waiting place still belongs to a stop (m).
const STOP_REACH: f64 = 18.0;
/// What somebody boarding who lets a passenger getting off pass in the aisle is doing.
const YIELDING: &str = "lets somebody pass in the aisle";
/// Pedestrians stroll within this distance of the player (m).
const STROLL_RADIUS: f64 = 200.0;
/// How far in front of a seat's hip point somebody stands to sit down - where the feet
/// stay while seated (m).
const SEAT_FRONT: f32 = 0.34;
/// Over this distance on either side of a joint (m) a point of an articulated bus's cabin
/// moves from the frame of the section in front to the one behind.
const JOINT_BLEND: f32 = 0.5;
/// Somebody within this height (m) of an exit's floor stands on that floor: a step or a
/// sloping aisle is less, a staircase step or the other deck of a double-decker more.
const DECK_STEP: f32 = 0.4;
/// How near the spot in front of the exit (m) somebody first in line must stand to step
/// out: from farther the straight line to the doorway crosses seats and panels.
const EXIT_REACH: f64 = 0.6;
/// Seconds a bus may stand at a stop with every door still shut before a waiting passenger
/// gives up on it coming to serve them: the driver's own door buttons take a moment, and
/// the timetable buses' door scripts open a beat after they roll to a stop.
const DOOR_GRACE: f64 = 4.0;
/// How long after a door of a standing bus was last open the people at it wait on (s).
const DOOR_SHUT_PATIENCE: f64 = 25.0;

fn debug_pax() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        omsi_cfg::env::var_os("OMSI_DEBUG_PAX").is_some()
            || omsi_cfg::env::var_os("OMSI_DEBUG_HUMANS").is_some()
    })
}

/// `OMSI_DEBUG_POSE=1`: log the people near the eye every two seconds; `=<id>`: that
/// person every frame.
fn debug_pose() -> Option<Option<u32>> {
    static ON: std::sync::OnceLock<Option<Option<u32>>> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        omsi_cfg::env::var("OMSI_DEBUG_POSE")
            .ok()
            .map(|v| v.trim().parse::<u32>().ok().filter(|id| *id > 1))
    })
}

/// Where the player looks from, for "nobody appears or vanishes in sight".
#[derive(Debug, Clone, Copy)]
pub struct Eye {
    pub pos: DVec3,
    pub fwd: DVec3,
    /// Cosine of half the diagonal field of view, with a margin.
    pub cos_half: f64,
}

impl Eye {
    pub fn of(cam: &Camera, aspect: f32) -> Eye {
        let half_v = (cam.fov_deg as f64 * 0.5).to_radians();
        let half_diag = (half_v.tan() * (1.0 + (aspect as f64).powi(2)).sqrt()).atan();
        Eye {
            pos: cam.position,
            fwd: cam.forward().as_dvec3().normalize_or_zero(),
            cos_half: (half_diag + 10f64.to_radians())
                .min(89f64.to_radians())
                .cos(),
        }
    }
}

/// A bus as the passengers know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusId {
    Player,
    Ai(u64),
}

impl BusId {
    /// Crowd space of the bus's floor (0 is the ground).
    fn space(self) -> u64 {
        match self {
            BusId::Player => 1,
            BusId::Ai(id) => 2 + id,
        }
    }
}

/// An `[entry]` or `[exit]` of a cabin, in the bus frame.
#[derive(Debug, Clone)]
struct Door {
    /// The door's path point (the threshold).
    inside: Vec3,
    /// Where somebody stands just outside, at ground level.
    outside: Vec3,
    /// +1 on the right side of the bus, -1 on the left.
    side: f32,
    /// Direction along the bus (+1 forwards) in which the queue at this door runs.
    queue_dir: f32,
    /// A passenger who still has to buy a ticket may board here (no `{noticketsale}`).
    sells: bool,
    /// Where people getting off wait for the door to open: the path point next to it.
    wait: Vec3,
    /// The aisle beyond `wait`, where the others getting off line up.
    aisle: Vec3,
}

impl Door {
    /// Whether height `z` (bus frame) is on the floor people waiting at this exit stand on.
    fn on_floor(&self, z: f32) -> bool {
        (z - self.aisle.z).abs() < DECK_STEP || (z - self.wait.z).abs() < DECK_STEP
    }

    /// Where the `k`-th person waiting at this door stands: at `wait` first, then back
    /// along the aisle towards where they came from (`from_y`).
    fn queue_place(&self, k: usize, from_y: f32) -> Vec3 {
        if k == 0 {
            return self.wait;
        }
        let in_aisle = (self.wait - self.aisle).truncate().length() < 0.05;
        let along = if from_y >= self.aisle.y { 1.0 } else { -1.0 };
        let steps = if in_aisle { k } else { k - 1 };
        self.aisle + Vec3::new(0.0, along * 0.6 * steps as f32, 0.0)
    }
}

#[derive(Debug, Clone)]
struct Seat {
    /// The `[passpos]` point: a seated passenger's hip, a standing one's feet.
    pos: Vec3,
    /// The floor in front of it, where the feet go (and where a seated passenger stands
    /// before sitting down and after getting up).
    floor: Vec3,
    rot: f32,
    seated: bool,
}

/// What passengers need to know about one vehicle type's cabin.
struct Cabin {
    data: PassengerCabin,
    graph: PathGraph,
    links: Vec<(i32, i32, bool)>,
    entries: Vec<Door>,
    exits: Vec<Door>,
    /// Where a passenger stands at the cash desk, its path point, and the heading (bus
    /// frame) they face: between the desk top, where the money goes, and the driver.
    desk: Option<(Vec3, Option<usize>, f64)>,
    /// Where a passenger stands to stamp a ticket at each `[stamper]` (a validator), and
    /// the heading (bus frame) they face.
    stampers: Vec<(Vec3, f64)>,
    seats: Vec<Seat>,
    /// The sections (one for a rigid bus), front first; everything above is in the
    /// unfolded frame of the front section.
    parts: Vec<CabinPart>,
}

/// A section of an articulated bus in its cabin's unfolded frame.
#[derive(Debug, Clone, Copy)]
struct CabinPart {
    /// Where the section's own origin lies.
    offset: Vec3,
    /// The unfolded y of the joint in front of it (the front section: none, +inf).
    joint_y: f32,
}

/// One vehicle of a coupled train as a cabin is put together from it: its definition, its
/// origin in the front vehicle's unfolded frame, and the unfolded y of its front joint.
type TrainPart<'a> = (&'a omsi_vehicle::Vehicle, Vec3, f32);

/// The sections of `v` passengers can walk through, front first: the vehicle and every
/// coupled part straight behind it (a part coupled the wrong way round and all behind it
/// are left out).
fn train_parts(v: &VehicleInstance) -> Vec<TrainPart<'_>> {
    let mut out: Vec<TrainPart<'_>> = vec![(&v.ty.def, Vec3::ZERO, f32::INFINITY)];
    let mut offset = Vec3::ZERO;
    for t in &v.trailers {
        if t.reversed {
            break;
        }
        let (back, front) = t.couplings();
        let joint_y = offset.y + back.y;
        offset += back - front;
        out.push((&t.ty.def, offset, joint_y));
    }
    out
}

impl Cabin {
    /// The cabin of a train of vehicles (see [`train_parts`]): the front one's, with the
    /// sections behind joined on as far as they have a cabin and a path network.
    fn load_train(parts: &[TrainPart<'_>]) -> Option<Cabin> {
        let (lead, _, _) = parts.first()?;
        let load_cabin = |def: &omsi_vehicle::Vehicle| -> Option<PassengerCabin> {
            let rel = def.passenger_cabin.as_ref()?;
            PassengerCabin::load(&omsi_cfg::resolve_path(def.dir(), rel))
                .map_err(|e| log::warn!("{e}"))
                .ok()
        };
        let load_paths = |def: &omsi_vehicle::Vehicle| {
            def.paths.as_ref().and_then(|rel| {
                omsi_vehicle::VehiclePaths::load(&omsi_cfg::resolve_path(def.dir(), rel))
                    .map_err(|e| log::warn!("{e}"))
                    .ok()
            })
        };
        let data = load_cabin(lead)?;
        let mut points: Vec<Vec3> = Vec::new();
        let mut links: Vec<(i32, i32, bool)> = Vec::new();
        // (merged path point or -1, sells tickets, half width of the section)
        let mut entry_points: Vec<(i32, bool, f32)> = Vec::new();
        let mut exit_points: Vec<(i32, f32)> = Vec::new();
        let mut places: Vec<(omsi_vehicle::cabin::PassPos, Vec3)> = Vec::new();
        let mut cabin_parts: Vec<CabinPart> = Vec::new();
        // the point of the section in front that leads on to the next one
        let mut rear_link: Option<usize> = None;
        for (k, (def, offset, joint_y)) in parts.iter().enumerate() {
            let cab = if k == 0 {
                Some(data.clone())
            } else {
                load_cabin(def)
            };
            let Some(cab) = cab else { break };
            let (own, own_links): (Vec<Vec3>, Vec<(i32, i32, bool)>) = match load_paths(def) {
                Some(p) => (
                    p.points
                        .iter()
                        .map(|q| Vec3::from(q.pos) + *offset)
                        .collect(),
                    p.links,
                ),
                None => (Vec::new(), Vec::new()),
            };
            let base = points.len();
            let valid = |i: i32| (i >= 0 && (i as usize) < own.len()).then_some(base + i as usize);
            let end = |front: bool| {
                (0..own.len())
                    .filter(|i| own[*i].x.abs() < 0.6)
                    .max_by(|a, b| {
                        if front {
                            own[*a].y.total_cmp(&own[*b].y)
                        } else {
                            own[*b].y.total_cmp(&own[*a].y)
                        }
                    })
                    .map(|i| base + i)
            };
            if k > 0 {
                // through the joint: from the front section's [linkToPrevVeh] point to this
                // one's [linkToNextVeh] point (the frontmost aisle point when it has none)
                let front = cab.link_to_next_veh.and_then(valid).or_else(|| end(true));
                match (rear_link, front) {
                    (Some(a), Some(b)) => links.push((a as i32, b as i32, false)),
                    // no way through: the section stays empty
                    _ => break,
                }
            }
            points.extend(own.iter().copied());
            links.extend(
                own_links
                    .iter()
                    .map(|(a, b, o)| (a + base as i32, b + base as i32, *o)),
            );
            rear_link = cab.link_to_prev_veh.and_then(valid).or_else(|| end(false));
            let half = def
                .bounding_box
                .map(|b| b[0] * 0.5)
                .unwrap_or_else(|| own.iter().map(|p| p.x.abs()).fold(1.2, f32::max));
            let shift = |i: i32| valid(i).map(|m| m as i32).unwrap_or(-1);
            entry_points.extend(
                cab.entries
                    .iter()
                    .map(|e| (shift(e.path_point), !e.no_ticket_sale, half)),
            );
            exit_points.extend(cab.exits.iter().map(|e| (shift(*e), half)));
            places.extend(cab.pass_positions.iter().map(|p| (p.clone(), *offset)));
            cabin_parts.push(CabinPart {
                offset: *offset,
                joint_y: *joint_y,
            });
        }
        let graph = PathGraph::new(points.clone(), &links);
        // (the side of the road the stops are on: where a door's own point does not tell)
        let kerb = if LEFT_HAND.load(std::sync::atomic::Ordering::Relaxed) { -1.0f32 } else { 1.0 };
        let door = |pp: i32, sells: bool, half_width: f32| -> Door {
            let point = (pp >= 0 && (pp as usize) < points.len()).then_some(pp as usize);
            let inside = point
                .map(|i| points[i])
                .unwrap_or(Vec3::new(kerb * (half_width - 0.1), 4.0, 0.4));
            // A door's side is the side of its entry point; one in the middle of the aisle
            // (or none) is taken to open to the kerb - on the left where the traffic keeps
            // left. (Always the right: a UK bus whose entry point lies on the aisle had the
            // people come to its door from the road side, round the bus.)
            let side = if inside.x.abs() < 0.6 { kerb } else if inside.x >= 0.0 { 1.0 } else { -1.0 };
            let outside = Vec3::new(side * (half_width + DOOR_OUT), inside.y, 0.0);
            // the aisle point next to the door: its neighbour nearest the middle
            let wait_point = point
                .and_then(|i| {
                    graph
                        .neighbours(i)
                        .into_iter()
                        .min_by(|a, b| points[*a].x.abs().total_cmp(&points[*b].x.abs()))
                })
                .filter(|&w| (points[w].x - inside.x).abs() > 0.3);
            // (no aisle point linked beside the door - the W906's door steps lead straight on
            // along it: the nearest path point off the door's line, else a step inwards)
            let wait = wait_point.map(|w| points[w]).unwrap_or_else(|| {
                points
                    .iter()
                    .filter(|p| (p.x - inside.x).abs() > 0.3 && (p.truncate() - inside.truncate()).length() < 1.2 && (p.z - inside.z).abs() < 0.6)
                    .min_by(|a, b| (a.truncate() - inside.truncate()).length().total_cmp(&(b.truncate() - inside.truncate()).length()))
                    .copied()
                    .unwrap_or(Vec3::new(inside.x - side * 0.7, inside.y, inside.z))
            });
            // already in the aisle, or the aisle point beyond it
            let aisle = match wait_point {
                Some(w) if points[w].x.abs() > 0.3 => graph
                    .neighbours(w)
                    .into_iter()
                    .filter(|&n| Some(n) != point)
                    .map(|n| points[n])
                    .min_by(|a, b| a.x.abs().total_cmp(&b.x.abs()))
                    .unwrap_or(wait),
                _ => wait,
            };
            Door {
                inside,
                outside,
                side,
                queue_dir: -1.0,
                sells,
                wait,
                aisle,
            }
        };
        let mut entries: Vec<Door> = entry_points
            .iter()
            .map(|(pp, sells, half)| door(*pp, *sells, *half))
            .collect();
        let exits: Vec<Door> = exit_points
            .iter()
            .map(|(pp, half)| door(*pp, false, *half))
            .collect();
        // two leaves of one door: the queue of the front leaf runs forwards, the other's back,
        // so that the two lines do not stand in each other
        for i in 0..entries.len() {
            let partner = (0..entries.len()).find(|&j| {
                j != i
                    && entries[j].side == entries[i].side
                    && (entries[j].inside.y - entries[i].inside.y).abs() < 1.4
            });
            entries[i].queue_dir = match partner {
                Some(j) if entries[j].inside.y < entries[i].inside.y => 1.0,
                _ => -1.0,
            };
        }
        let desk = data.ticket_sales.first().map(|ts| {
            let top = Vec3::from(ts.pos);
            let by_point = usize::try_from(ts.path_point)
                .ok()
                .and_then(|i| points.get(i).map(|p| (i, *p)))
                .filter(|(_, p)| (top.truncate() - p.truncate()).length() < 3.0);
            let (stand, pi) = match by_point {
                Some((i, p)) => (p, Some(i)),
                None => {
                    // no usable path point: the nearest one on the entry floor, else the floor by the desk
                    let floor = entries.first().map(|e| e.inside.z).unwrap_or(0.4);
                    let near = points
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| (p.z - floor).abs() < 0.6)
                        .min_by(|a, b| {
                            (a.1.truncate() - top.truncate())
                                .length()
                                .total_cmp(&(b.1.truncate() - top.truncate()).length())
                        });
                    match near {
                        Some((i, p)) => (*p, Some(i)),
                        None => (Vec3::new(top.x + 0.4, top.y, floor), None),
                    }
                }
            };
            let target = match data.driver_positions.first() {
                Some(d) => (top + Vec3::from(d.pos)) * 0.5,
                None => top,
            };
            let d = target - stand;
            let face = if d.truncate().length() > 0.05 {
                (d.x as f64).atan2(d.y as f64).to_degrees()
            } else {
                -90.0
            };
            (stand, pi, face)
        });
        // the validators: their path point when it lies near, else the nearest point of the
        // path network on a floor within reach of the device
        let stampers = data
            .stampers
            .iter()
            .map(|st| {
                let dev = Vec3::from(st.pos);
                let by_point = usize::try_from(st.path_point)
                    .ok()
                    .and_then(|i| points.get(i).copied())
                    .filter(|p| (dev.truncate() - p.truncate()).length() < 3.0);
                let stand = by_point.unwrap_or_else(|| {
                    points
                        .iter()
                        .filter(|p| dev.z - p.z > -0.3 && dev.z - p.z < 2.0)
                        .min_by(|a, b| (a.truncate() - dev.truncate()).length().total_cmp(&(b.truncate() - dev.truncate()).length()))
                        .copied()
                        .unwrap_or(Vec3::new(dev.x, dev.y, dev.z - 1.0))
                });
                let d = dev - stand;
                let face = if d.truncate().length() > 0.05 { (d.x as f64).atan2(d.y as f64).to_degrees() } else { 0.0 };
                (stand, face)
            })
            .collect();
        let seats = places
            .iter()
            .map(|(p, offset)| {
                let pos = Vec3::from(p.pos) + *offset;
                let seated = p.height > 0.01;
                let floor = if seated {
                    let r = p.rot.to_radians();
                    Vec3::new(
                        pos.x + r.sin() * SEAT_FRONT,
                        pos.y + r.cos() * SEAT_FRONT,
                        pos.z - p.height,
                    )
                } else {
                    pos
                };
                Seat {
                    pos,
                    floor,
                    rot: p.rot,
                    seated,
                }
            })
            .collect();
        Some(Cabin {
            data,
            graph,
            links,
            entries,
            exits,
            desk,
            stampers,
            seats,
            parts: cabin_parts,
        })
    }

    /// The point of the walkways (the path links) nearest `p` (bus frame; height weighs
    /// three times), and how far away it is.
    fn on_walkways(&self, p: Vec3) -> Option<(Vec3, f32)> {
        let pts = &self.graph.points;
        let mut best: Option<(f32, Vec3)> = None;
        for &(a, b, _) in &self.links {
            let (Some(pa), Some(pb)) = (pts.get(a.max(0) as usize), pts.get(b.max(0) as usize)) else { continue };
            let ab = *pb - *pa;
            let t = if ab.length_squared() > 1e-6 { ((p - *pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            let q = *pa + ab * t;
            let v = q - p;
            let d = (v.x * v.x + v.y * v.y + 9.0 * v.z * v.z).sqrt();
            if best.map(|x| d < x.0).unwrap_or(true) {
                best = Some((d, q));
            }
        }
        best.map(|(d, q)| (q, d))
    }

    /// Place `k` of the queue at exit door `x` for somebody coming from `from_y`: one
    /// behind the other along the aisle, on the walkways. The line ran straight along the
    /// bus whatever was there: in the W906 the queue of its sliding door ran forwards over
    /// the bonnet, where the people stood in the air. A line that would leave the walkways
    /// one way runs the other, and every place is put on the walkways.
    fn exit_queue_place(&self, x: usize, k: usize, from_y: f32) -> Vec3 {
        let door = &self.exits[x];
        let first = door.queue_place(k, from_y);
        if k == 0 || self.links.is_empty() {
            return first;
        }
        let fits = |q: Vec3| self.on_walkways(q).map(|(_, d)| d < 0.3).unwrap_or(true);
        let other = door.queue_place(k, if from_y >= door.aisle.y { door.aisle.y - 1.0 } else { door.aisle.y + 1.0 });
        let pick = if fits(first) || !fits(other) { first } else { other };
        self.on_walkways(pick).map(|(q, _)| q).unwrap_or(pick)
    }

    /// Walk inside from `from` to `to` (bus frame) along the path network.
    fn route(&self, from: Vec3, to: Vec3) -> Vec<Vec3> {
        if self.graph.is_empty() {
            return vec![to];
        }
        self.graph.route(from, to)
    }

    /// Whether a walk from door `entry` to `to` passes the cash desk.
    fn passes_desk(&self, entry: usize, to: Vec3) -> bool {
        let Some((stand, _, _)) = self.desk else {
            return false;
        };
        let Some(door) = self.entries.get(entry) else {
            return false;
        };
        let route = self.route(door.inside, to);
        let mut last = door.inside;
        for p in route {
            let (_, t) = crowd::project_on_segment(
                stand.truncate().as_dvec2(),
                last.truncate().as_dvec2(),
                p.truncate().as_dvec2(),
            );
            let q = last + (p - last) * t as f32;
            if (q.truncate() - stand.truncate()).length() < 0.4 {
                return true;
            }
            last = p;
        }
        false
    }

    /// A place for a boarding passenger: a seat (preferably not next to somebody) near the
    /// entry, a standing place once the seats are taken.
    fn choose_seat(
        &self,
        taken: &[bool],
        near: Vec3,
        luck: impl Fn(usize) -> f32,
    ) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, s) in self.seats.iter().enumerate() {
            if taken.get(i).copied().unwrap_or(true) {
                continue;
            }
            let mut score = (s.floor - near).length() * 0.2 + luck(i) * 3.0;
            if !s.seated {
                score += 30.0;
            }
            if self.seats.iter().enumerate().any(|(j, o)| {
                j != i
                    && taken.get(j).copied().unwrap_or(false)
                    && o.seated
                    && (o.pos - s.pos).length() < 0.7
            }) {
                score += 4.0;
            }
            if best.map(|b| score < b.1).unwrap_or(true) {
                best = Some((i, score));
            }
        }
        best.map(|b| b.0)
    }

    /// Floor height at a point of the bus frame: the ground outside the body, else the
    /// height of the nearest path link where it passes (a staircase rises along its links,
    /// as the walkers on it do) or of the nearest place (the seats on a platform have their
    /// floor in front of them). Only floors within a metre of `level` count, and of two
    /// about as near the one nearer that height wins: the decks of a double-decker lie over
    /// each other, and a foot put down beside the stairs belongs on the floor the body is
    /// on. The nearest path point alone put feet a whole flight up - the first landing of
    /// the SD202's stairs is 0.7 m above the aisle beside it.
    fn floor_at(&self, p: DVec2, half_width: f64, level: f64) -> Option<f64> {
        if p.x.abs() > half_width - 0.05 {
            return Some(0.0);
        }
        let pts = &self.graph.points;
        let mut best: Option<(f64, f32)> = None;
        let mut consider = |dist: f64, z: f32| {
            let dz = (z as f64 - level).abs();
            if dz >= 1.0 {
                return;
            }
            let score = dist + 0.5 * dz;
            if best.map(|b| score < b.0).unwrap_or(true) {
                best = Some((score, z));
            }
        };
        let point = |i: i32| usize::try_from(i).ok().and_then(|i| pts.get(i));
        for &(a, b, _) in &self.links {
            let (Some(pa), Some(pb)) = (point(a), point(b)) else {
                continue;
            };
            let (q, t) =
                crowd::project_on_segment(p, pa.truncate().as_dvec2(), pb.truncate().as_dvec2());
            consider((q - p).length(), pa.z + (pb.z - pa.z) * t as f32);
        }
        for n in pts.iter().chain(self.seats.iter().map(|s| &s.floor)) {
            consider((n.truncate().as_dvec2() - p).length(), n.z);
        }
        best.map(|b| b.1 as f64)
    }

    /// Which section a point of the cabin lies in (0 = the front one).
    fn part_of(&self, p: Vec3) -> usize {
        self.parts
            .iter()
            .skip(1)
            .filter(|c| p.y < c.joint_y)
            .count()
    }

    /// For logs: " in rear section n" for a point behind the first joint.
    fn part_label(&self, p: Vec3) -> String {
        match self.part_of(p) {
            0 => String::new(),
            n => format!(" in rear section {n}"),
        }
    }

    /// The exit nearest to a point inside.
    fn nearest_exit(&self, p: Vec3) -> usize {
        (0..self.exits.len())
            .min_by(|a, b| {
                (self.exits[*a].inside - p)
                    .length()
                    .total_cmp(&(self.exits[*b].inside - p).length())
            })
            .unwrap_or(0)
    }
}

/// Whether the straight way from `a` to `b` goes over a carriageway: across the centre
/// line of a street lane (walking along the kerb on the carriageway's edge does not).
fn crosses_street(net: &Network, a: DVec2, b: DVec2) -> bool {
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for p in [a, b, (a + b) * 0.5] {
        let c = Network::grid_cell(p.extend(0.0));
        if !cells.contains(&c) {
            cells.push(c);
        }
    }
    let mut seen: Vec<usize> = Vec::new();
    for c in cells {
        for &i in net.grid.get(&c).map(|v| v.as_slice()).unwrap_or(&[]) {
            if seen.contains(&i) {
                continue;
            }
            seen.push(i);
            let l = &net.lanes[i];
            if l.kind != LaneKind::Street {
                continue;
            }
            if l
                .points
                .windows(2)
                .any(|w| segments_cross(a, b, w[0].truncate(), w[1].truncate()))
            {
                return true;
            }
        }
    }
    false
}

/// Whether a point lies on a carriageway: within half a street lane's width (and 30 cm)
/// of its centre line, at about the height of `z`.
fn on_carriageway(net: &Network, p: DVec3) -> bool {
    let q = p.truncate();
    net.grid
        .get(&Network::grid_cell(p))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|&i| &net.lanes[i])
        .filter(|l| l.kind == LaneKind::Street)
        .any(|l| {
            l.points.windows(2).any(|w| {
                let (c, _) = crowd::project_on_segment(q, w[0].truncate(), w[1].truncate());
                c.distance(q) < l.width as f64 * 0.5 + 0.3 && (w[0].z - p.z).abs() < 2.0
            })
        })
}

/// Whether the segments `a`-`b` and `c`-`d` cross.
fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let side = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// A bus as the passengers see it this frame.
#[derive(Clone)]
struct BusNow {
    id: BusId,
    cabin: Arc<Cabin>,
    pos: DVec3,
    rot: Mat4,
    heading: f64,
    /// m/s, forwards.
    speed: f64,
    entry_open: Vec<bool>,
    exit_open: Vec<bool>,
    /// The doors a walker may use (another player's bus: its doors as they are, while
    /// `entry_open` stays shut for the passengers here); None: as `entry_open`/`exit_open`.
    walk_open: Option<(Vec<bool>, Vec<bool>)>,
    /// The stop it is serving (standing at it).
    stop: Option<i64>,
    /// The stop it is pulling in to: one ahead of it within 60 m, facing its way, while it
    /// still rolls. Omsi.exe lists a vehicle at a stop from 60 m out (sub_61f238) and the
    /// people waiting there set off towards a bus of theirs while it is still faster than
    /// 2 m/s (sub_62a6a0 at 0x62baa2): the driver sees who wants to get on before stopping.
    approach: Option<i64>,
    interior: f32,
    /// The saloon's air and the light outside, for what boarding passengers say.
    air: CabinAir,
    /// Half extents across / along and the centre of its bounding box (bus frame).
    half: DVec2,
    centre: DVec2,
    /// Acceleration of the floor (bus frame: x to the right, y forwards; m/s²).
    accel: DVec2,
    /// The sections behind the front one (the cabin's parts after the first).
    trailers: Vec<PartFrame>,
    /// The terminus it shows, by name (Omsi.exe's bus +0x7bc). None: "$allexit$" - the
    /// scripts' `target_index_int` names a hof terminus added with `[addterminus_allexit]`
    /// ("Nicht einsteigen", a works trip) - or none; no timetable target has it.
    terminus: Option<String>,
}

/// What passengers feel stepping into a bus (OMSI reads the same fields: the vehicle's
/// `Cabinair_Temp` and `Cabinair_relHum`, the weather's temperature and the daylight).
#[derive(Debug, Clone, Copy, Default)]
struct CabinAir {
    /// °C, when the bus keeps its cabin air (every bus does: its script or the engine).
    temp: Option<f32>,
    /// Relative humidity, a fraction.
    rel_hum: f32,
    /// The temperature outside (°C).
    outside: f32,
    /// `Envir_Brightness`: the daylight, 0 dark .. 1.
    brightness: f32,
}

impl CabinAir {
    fn of(v: &VehicleInstance) -> CabinAir {
        CabinAir {
            temp: v.var("Cabinair_Temp").filter(|t| t.is_finite()),
            rel_hum: v.var("Cabinair_relHum").filter(|h| h.is_finite()).unwrap_or(0.0),
            outside: v.host.temperature,
            brightness: v.var("Envir_Brightness").unwrap_or(1.0),
        }
    }
}

/// Where a rear section of a bus is this frame, with its place in the cabin.
#[derive(Debug, Clone, Copy)]
struct PartFrame {
    pos: DVec3,
    rot: Mat4,
    heading: f64,
    offset: Vec3,
    joint_y: f32,
    /// Half extents across / along and the centre of its bounding box (own frame).
    half: DVec2,
    centre: DVec2,
}

/// The rear sections of `v` that are parts of `cabin`, as they stand now.
fn part_frames(v: &VehicleInstance, cabin: &Cabin) -> Vec<PartFrame> {
    cabin
        .parts
        .iter()
        .skip(1)
        .zip(&v.trailers)
        .map(|(cp, t)| {
            let bb =
                t.ty.def
                    .bounding_box
                    .unwrap_or([2.5, 7.0, 3.0, 0.0, 0.0, 1.5]);
            PartFrame {
                pos: t.position,
                rot: t.body_rotation(),
                heading: t.heading,
                offset: cp.offset,
                joint_y: cp.joint_y,
                half: DVec2::new(bb[0] as f64 * 0.5, bb[1] as f64 * 0.5),
                centre: DVec2::new(bb[3] as f64, bb[4] as f64),
            }
        })
        .collect()
}

/// How far into the frame of the section behind joint `t` a cabin point `y` lies: 0 in
/// front of the joint's blend, 1 behind it.
fn behind(t: &PartFrame, y: f32) -> f32 {
    ((JOINT_BLEND - (y - t.joint_y)) / (2.0 * JOINT_BLEND)).clamp(0.0, 1.0)
}

/// Where a point of a cabin (unfolded frame) is in the world: the front section carries
/// what lies ahead of the first joint, a rear section what lies behind its joint, and near
/// a joint the two are blended, so that somebody walking through the bellows moves on
/// smoothly however far the bus is bent.
fn train_point(pos: DVec3, rot: &Mat4, trailers: &[PartFrame], local: Vec3) -> DVec3 {
    let mut here = pos + rot.transform_point3(local).as_dvec3();
    for t in trailers {
        let w = behind(t, local.y);
        if w <= 0.0 {
            break;
        }
        let there = t.pos + t.rot.transform_point3(local - t.offset).as_dvec3();
        here = here.lerp(there, w as f64);
        if w < 1.0 {
            break;
        }
    }
    here
}

/// The heading of the floor at a point of a cabin (see [`train_point`]).
fn train_heading(heading: f64, trailers: &[PartFrame], local: Vec3) -> f64 {
    let mut here = heading;
    for t in trailers {
        let w = behind(t, local.y);
        if w <= 0.0 {
            break;
        }
        here += crowd::angle_diff(here, t.heading) * w as f64;
        if w < 1.0 {
            break;
        }
    }
    here
}

impl BusNow {
    fn world(&self, local: Vec3) -> DVec3 {
        train_point(self.pos, &self.rot, &self.trailers, local)
    }
    /// The tilt (pitch and bank, in the world's axes, no heading) of the section a point of
    /// the cabin is in.
    fn tilt_at(&self, local: Vec3) -> Mat4 {
        let mut rot = self.rot;
        let mut heading = self.heading;
        for t in &self.trailers {
            if behind(t, local.y) < 0.5 {
                break;
            }
            rot = t.rot;
            heading = t.heading;
        }
        rot * Mat4::from_rotation_z(heading.to_radians() as f32)
    }
    /// The heading of the section a point of the cabin is in.
    fn heading_at(&self, local: Vec3) -> f64 {
        train_heading(self.heading, &self.trailers, local)
    }
    fn fwd(&self) -> DVec2 {
        let h = self.heading.to_radians();
        DVec2::new(h.sin(), h.cos())
    }
    fn fwd_at(&self, local: Vec3) -> DVec2 {
        let h = self.heading_at(local).to_radians();
        DVec2::new(h.sin(), h.cos())
    }
    fn right_at(&self, local: Vec3) -> DVec2 {
        let h = self.heading_at(local).to_radians();
        DVec2::new(h.cos(), -h.sin())
    }
    fn standing(&self) -> bool {
        self.speed.abs() < 0.4
    }
    /// The bodies people on the ground walk round: the bus and its rear sections.
    fn blocks(&self) -> Vec<Block> {
        let block = |pos: DVec3, heading: f64, half: DVec2, centre: DVec2| {
            let h = heading.to_radians();
            let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
            Block {
                center: pos.truncate() + right * centre.x + fwd * centre.y,
                half,
                heading: h,
                vel: fwd * self.speed,
            }
        };
        let mut out = vec![block(self.pos, self.heading, self.half, self.centre)];
        out.extend(
            self.trailers
                .iter()
                .map(|t| block(t.pos, t.heading, t.half, t.centre)),
        );
        out
    }
}

/// One piece of a walk along a pavement path: lane `lane` from distance `a` to `b`.
#[derive(Debug, Clone, Copy)]
struct Leg {
    lane: usize,
    a: f32,
    b: f32,
}

impl Leg {
    fn len(&self) -> f32 {
        (self.b - self.a).abs()
    }
    fn dist(&self, p: f32) -> f32 {
        if self.b >= self.a {
            self.a + p
        } else {
            self.a - p
        }
    }
    /// Point and walking heading `p` metres into the leg.
    fn at(&self, net: &Network, p: f32) -> (DVec3, f64) {
        let (q, h) = net.lanes[self.lane].at(self.dist(p.clamp(0.0, self.len())));
        (
            q,
            if self.b >= self.a {
                h as f64
            } else {
                h as f64 + 180.0
            },
        )
    }
    /// How far into the leg the point nearest `pos` lies, looking around `hint`.
    fn project(&self, net: &Network, pos: DVec3, hint: f32) -> f32 {
        let (lo, hi) = ((hint - 1.5).max(0.0), (hint + 3.0).min(self.len()));
        let mut best = (hint, f64::MAX);
        let mut p = lo;
        while p <= hi + 1e-3 {
            let d = (self.at(net, p).0 - pos).truncate().length_squared();
            if d < best.1 {
                best = (p, d);
            }
            p += 0.2;
        }
        best.0
    }
    /// Whether the leg starts at an end of its lane (at a kerb or a junction).
    fn from_end(&self, net: &Network) -> bool {
        self.a < 0.05 || self.a > net.lanes[self.lane].length() - 0.05
    }
}

/// A walk along the pavement network.
#[derive(Debug, Clone)]
struct PedWalk {
    legs: Vec<Leg>,
    leg: usize,
    /// Metres walked into the current leg.
    s: f32,
    /// A stroll: goes on at random when the legs run out.
    roam: bool,
    /// Keep-right offset (m).
    side: f32,
    /// Seconds spent waiting at the kerb before the current leg.
    held: f32,
}

impl PedWalk {
    fn new(legs: Vec<Leg>, roam: bool, side: f32) -> PedWalk {
        PedWalk {
            legs,
            leg: 0,
            s: 0.0,
            roam,
            side,
            held: 0.0,
        }
    }
}

#[derive(PartialEq)]
struct Open(f32, usize);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The pavement paths as a walking network: path ends closer than a metre are one
/// junction, whatever their heading (the road network joins lane ends only when they
/// continue in the same direction, which leaves every pavement corner open).
struct PedNet {
    /// Per pavement lane: its start and end junction.
    ends: HashMap<usize, (usize, usize)>,
    /// Per junction: (lane, walked forwards) leaving it.
    out: Vec<Vec<(usize, bool)>>,
    /// Where each pavement lane crosses a carriageway (lazily), and the carriageway lanes
    /// it crosses.
    crossings: HashMap<usize, Vec<DVec2>>,
    crossed: HashMap<usize, Vec<usize>>,
    /// Pavement lanes by 50 m cell.
    grid: HashMap<(i32, i32), Vec<usize>>,
    /// The junctions and a 1.5 m grid of them, for joining the paths of tiles loaded later.
    nodes: Vec<DVec3>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    /// How many lanes of the traffic network are in (the network only grows: tiles bring
    /// their lanes and the indices stay).
    built: usize,
}

impl PedNet {
    fn build(net: &Network) -> PedNet {
        let mut p = PedNet {
            ends: HashMap::new(),
            out: Vec::new(),
            crossings: HashMap::new(),
            crossed: HashMap::new(),
            grid: HashMap::new(),
            nodes: Vec::new(),
            cells: HashMap::new(),
            built: 0,
        };
        p.extend(net);
        log::info!(
            "pavement network: {} paths, {} junctions",
            p.ends.len(),
            p.nodes.len()
        );
        p
    }

    /// Take in the lanes the network gained since the last call (tiles streamed in).
    fn extend(&mut self, net: &Network) -> usize {
        let from = self.built.min(net.lanes.len());
        let before = self.ends.len();
        for i in from..net.lanes.len() {
            let l = &net.lanes[i];
            if l.kind == LaneKind::Street && from > 0 {
                // a new carriageway may cross pavement paths that are in already
                self.crossings.clear();
            }
            if l.kind != LaneKind::Sidewalk || l.points.len() < 2 || l.length() < 0.3 {
                continue;
            }
            let a = self.node_of(l.start());
            let b = self.node_of(l.end());
            if a == b && l.length() < 3.0 {
                continue;
            }
            self.ends.insert(i, (a, b));
            self.out[a].push((i, true));
            self.out[b].push((i, false));
            let mut seen: Vec<(i32, i32)> = Vec::new();
            for p in &l.points {
                let c = ((p.x / 50.0).floor() as i32, (p.y / 50.0).floor() as i32);
                if !seen.contains(&c) {
                    seen.push(c);
                    self.grid.entry(c).or_default().push(i);
                }
            }
        }
        self.built = net.lanes.len();
        self.ends.len() - before
    }

    /// The junction at `p`, a new one when there is none within a metre.
    fn node_of(&mut self, p: DVec3) -> usize {
        let (cx, cy) = ((p.x / 1.5).floor() as i64, (p.y / 1.5).floor() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(list) = self.cells.get(&(cx + dx, cy + dy)) {
                    for &n in list {
                        if (self.nodes[n] - p).truncate().length() < 1.2
                            && (self.nodes[n].z - p.z).abs() < 2.5
                        {
                            return n;
                        }
                    }
                }
            }
        }
        self.nodes.push(p);
        self.out.push(Vec::new());
        self.cells
            .entry((cx, cy))
            .or_default()
            .push(self.nodes.len() - 1);
        self.nodes.len() - 1
    }

    /// The pavement lane nearest `p` within `reach` that can be reached without going over
    /// a carriageway: (lane, distance along it, distance to it). The plain nearest one was
    /// often the pavement across the road - a passenger off a bus then walked straight
    /// over the carriageway through the traffic to it, or joined a crossing in the middle.
    fn nearest(&self, net: &Network, p: DVec3, reach: f64) -> Option<(usize, f32, f64)> {
        let (cx, cy) = ((p.x / 50.0).floor() as i32, (p.y / 50.0).floor() as i32);
        let mut cands: Vec<(usize, f32, f64)> = Vec::new();
        let mut seen = HashSet::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in self
                    .grid
                    .get(&(cx + dx, cy + dy))
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                {
                    if !seen.insert(i) {
                        continue;
                    }
                    if let Some((s, d)) = net.lanes[i].nearest_point(p) {
                        if d < reach {
                            cands.push((i, s, d));
                        }
                    }
                }
            }
        }
        cands.sort_by(|a, b| a.2.total_cmp(&b.2));
        let first = cands.first().copied();
        cands
            .into_iter()
            .find(|&(i, s, _)| {
                let (q, _) = net.lanes[i].at(s);
                !crosses_street(net, p.truncate(), q.truncate())
                    && !self.crossings.get(&i).map(|x| !x.is_empty()).unwrap_or(false)
            })
            // on an island between carriageways: the nearest after all
            .or(first)
    }

    /// The junction a leg ends at, when it ends at one.
    fn end_node(&self, net: &Network, leg: &Leg) -> Option<usize> {
        let (a, b) = *self.ends.get(&leg.lane)?;
        let len = net.lanes[leg.lane].length();
        if leg.b < 0.05 {
            Some(a)
        } else if leg.b > len - 0.05 {
            Some(b)
        } else {
            None
        }
    }

    /// A leg leaving junction `node`, not back along `came` (unless it is a dead end).
    fn next_leg(&self, net: &Network, node: usize, came: usize, pick: u64) -> Option<Leg> {
        let back = self.ends.get(&came).copied();
        let twin = |l: usize| -> bool {
            l == came
                || matches!((self.ends.get(&l), back), (Some(&(a, b)), Some((c, d))) if a == d && b == c && (net.lanes[l].length() - net.lanes[came].length()).abs() < 1.0)
        };
        let list: Vec<(usize, bool)> = self
            .out
            .get(node)?
            .iter()
            .copied()
            .filter(|(l, _)| !twin(*l))
            .collect();
        // the way on rather than back: a path leaving the junction within 110° of the way
        // the walker came (there usually is one - a pavement goes on past a side street),
        // else any. Picked from all, a stroller would turn round at every corner and walk
        // back the way they came, which looked like a change of mind for no reason.
        let heading_in = {
            let l = &net.lanes[came];
            let (a, _) = back.unwrap_or((usize::MAX, usize::MAX));
            // arriving at `node` along `came`: forwards if its end is the node
            if a == node {
                wrap_heading(l.start_heading() as f64 + 180.0)
            } else {
                l.end_heading() as f64
            }
        };
        let leaving = |&(l, fwd): &(usize, bool)| -> f64 {
            let lane = &net.lanes[l];
            if fwd {
                lane.start_heading() as f64
            } else {
                wrap_heading(lane.end_heading() as f64 + 180.0)
            }
        };
        let onward: Vec<(usize, bool)> = list
            .iter()
            .copied()
            .filter(|o| angle_between(heading_in, leaving(o)) <= 110.0)
            .collect();
        let list = if onward.is_empty() { list } else { onward };
        let (lane, fwd) = if list.is_empty() {
            // a dead end: turn round
            let (a, _) = back?;
            (came, a == node)
        } else {
            list[(pick as usize) % list.len()]
        };
        let len = net.lanes[lane].length();
        Some(if fwd {
            Leg {
                lane,
                a: 0.0,
                b: len,
            }
        } else {
            Leg {
                lane,
                a: len,
                b: 0.0,
            }
        })
    }

    /// The shortest walk from `from` (lane, distance along it) to `to`.
    fn route(&self, net: &Network, from: (usize, f32), to: (usize, f32)) -> Option<Vec<Leg>> {
        if from.0 == to.0 {
            return Some(vec![Leg {
                lane: from.0,
                a: from.1,
                b: to.1,
            }]);
        }
        let (fa, fb) = *self.ends.get(&from.0)?;
        let (ta, tb) = *self.ends.get(&to.0)?;
        let (flen, tlen) = (net.lanes[from.0].length(), net.lanes[to.0].length());
        let mut dist: HashMap<usize, f32> = HashMap::new();
        let mut prev: HashMap<usize, (usize, usize, bool)> = HashMap::new();
        let mut heap = BinaryHeap::new();
        for (n, c) in [(fa, from.1), (fb, flen - from.1)] {
            if dist.get(&n).map(|d| c < *d).unwrap_or(true) {
                dist.insert(n, c);
                heap.push(Open(c, n));
            }
        }
        let mut best: Option<(f32, usize)> = None;
        let mut pops = 0;
        while let Some(Open(cost, node)) = heap.pop() {
            pops += 1;
            if pops > 6000 || best.map(|b| cost >= b.0).unwrap_or(false) {
                break;
            }
            if dist.get(&node).map(|d| cost > *d).unwrap_or(false) {
                continue;
            }
            for (goal, extra) in [(ta, to.1), (tb, tlen - to.1)] {
                if node == goal && best.map(|b| cost + extra < b.0).unwrap_or(true) {
                    best = Some((cost + extra, node));
                }
            }
            for &(lane, fwd) in &self.out[node] {
                if lane == from.0 || lane == to.0 {
                    continue;
                }
                let (a, b) = self.ends[&lane];
                let other = if fwd { b } else { a };
                let c = cost + net.lanes[lane].length();
                if dist.get(&other).map(|d| c < *d).unwrap_or(true) {
                    dist.insert(other, c);
                    prev.insert(other, (node, lane, fwd));
                    heap.push(Open(c, other));
                }
            }
        }
        let (_, goal) = best?;
        let mut middle = Vec::new();
        let mut n = goal;
        while let Some(&(p, lane, fwd)) = prev.get(&n) {
            let len = net.lanes[lane].length();
            middle.push(if fwd {
                Leg {
                    lane,
                    a: 0.0,
                    b: len,
                }
            } else {
                Leg {
                    lane,
                    a: len,
                    b: 0.0,
                }
            });
            n = p;
            if middle.len() > 4000 {
                return None;
            }
        }
        middle.reverse();
        let mut legs = vec![Leg {
            lane: from.0,
            a: from.1,
            b: if n == fa { 0.0 } else { flen },
        }];
        legs.extend(middle);
        legs.push(Leg {
            lane: to.0,
            a: if goal == ta { 0.0 } else { tlen },
            b: to.1,
        });
        legs.retain(|l| l.len() > 0.01);
        Some(legs)
    }

    /// Where pavement lane `lane` crosses a carriageway.
    fn crossings(&mut self, net: &Network, lane: usize) -> &[DVec2] {
        if !self.crossings.contains_key(&lane) {
            let l = &net.lanes[lane];
            let mut cand: Vec<usize> = Vec::new();
            for p in &l.points {
                if let Some(list) = net
                    .grid
                    .get(&((p.x / 50.0).floor() as i32, (p.y / 50.0).floor() as i32))
                {
                    for &i in list {
                        if net.lanes[i].kind == LaneKind::Street && !cand.contains(&i) {
                            cand.push(i);
                        }
                    }
                }
            }
            let mut out = Vec::new();
            let mut crossed = Vec::new();
            for i in cand {
                let o = &net.lanes[i];
                for w in l.points.windows(2) {
                    for v in o.points.windows(2) {
                        if (w[0].z - v[0].z).abs() > 3.0 {
                            continue;
                        }
                        if let Some(x) = seg_cross(
                            w[0].truncate(),
                            w[1].truncate(),
                            v[0].truncate(),
                            v[1].truncate(),
                        ) {
                            if !crossed.contains(&i) {
                                crossed.push(i);
                            }
                            if !out.iter().any(|q: &DVec2| (*q - x).length() < 1.5) {
                                out.push(x);
                            }
                        }
                    }
                }
            }
            self.crossings.insert(lane, out);
            self.crossed.insert(lane, crossed);
        }
        &self.crossings[&lane]
    }

    /// The carriageway lanes a pavement lane crosses.
    fn crossed_lanes(&mut self, net: &Network, lane: usize) -> &[usize] {
        self.crossings(net, lane);
        &self.crossed[&lane]
    }
}

/// Seconds a pedestrian starting across `path` now has before a vehicle may drive over it:
/// until the first light of a carriageway lane it crosses (or of a lane leading into one)
/// turns green once the pedestrian green is over. Lanes that have green now, or get it
/// while the pedestrians still have theirs, are turning traffic that gives way. Without
/// such a light, the pedestrian green `green_left` and two seconds.
fn pedestrian_window(
    ped: Option<&mut PedNet>,
    net: &Network,
    traffic: &Traffic,
    path: usize,
    green_left: f32,
) -> f32 {
    let mut window = f32::MAX;
    if let Some(ped) = ped {
        for &s in ped.crossed_lanes(net, path) {
            let feeding = net.prev.get(s).map(|p| p.as_slice()).unwrap_or(&[]);
            for &l in std::iter::once(&s).chain(feeding) {
                let Some((c, li)) = net.lanes[l].traffic_light else {
                    continue;
                };
                if let Some(g) = traffic
                    .light_until_go(c, li)
                    .filter(|g| *g > 0.0 && *g >= green_left)
                {
                    window = window.min(g);
                }
            }
        }
    }
    if window == f32::MAX {
        green_left + 2.0
    } else {
        window
    }
}

fn seg_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> Option<DVec2> {
    let r = b - a;
    let s = d - c;
    let den = r.perp_dot(s);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (c - a).perp_dot(s) / den;
    let u = (c - a).perp_dot(r) / den;
    (t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0).then(|| a + r * t)
}

/// A waiting place at a stop.
#[derive(Debug, Clone)]
struct Spot {
    /// Feet, or the hip for a seat.
    pos: DVec3,
    face: f64,
    seat: f32,
    taken: Option<u32>,
}

impl Spot {
    /// Where the feet go: the place itself, or in front of a seat.
    fn floor(&self) -> DVec3 {
        if self.seat <= 0.0 {
            return self.pos;
        }
        let r = self.face.to_radians();
        let front = SEAT_FRONT as f64;
        DVec3::new(
            self.pos.x + r.sin() * front,
            self.pos.y + r.cos() * front,
            self.pos.z - self.seat as f64,
        )
    }
}

struct StopInfo {
    name: String,
    pos: DVec3,
    spots: Vec<Spot>,
    /// Pavement lane and distance along it nearest the stop.
    lane: Option<(usize, f32)>,
    seeded: bool,
    /// Seconds until the next person arrives on foot.
    next_arrival: f32,
}

/// Inside a bus, to where.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Goal {
    /// To the cash desk, then to seat `.0`.
    Desk(usize),
    /// To validator `.1` to stamp a ticket, then to seat `.0`.
    Stamper(usize, usize),
    Seat(usize),
    /// To the aisle by exit `.0`, to wait for it to open.
    ExitWait(usize),
    /// Out through exit `.0`.
    Exit(usize),
}

#[derive(Debug, Clone)]
enum State {
    /// A pedestrian strolling the pavements.
    Strolling(PedWalk),
    /// Coming along the pavement to stop `stop`, then to waiting place `spot`.
    ToStop {
        stop: i64,
        spot: usize,
        walk: PedWalk,
    },
    /// The last metres to waiting place `spot`.
    ToSpot {
        stop: i64,
        spot: usize,
    },
    Waiting {
        stop: i64,
        spot: usize,
        patience: f32,
    },
    /// At the kerb for entry `entry` of `bus`, in the queue (by `joined`).
    Queue {
        bus: BusId,
        entry: usize,
        stop: i64,
        spot: usize,
        joined: f64,
    },
    /// Walking inside a bus: `route` in the bus frame, `seg` where the current leg began.
    Aboard {
        bus: BusId,
        route: Vec<Vec3>,
        idx: usize,
        seg: Vec3,
        goal: Goal,
    },
    /// At the cash desk of `bus`; `ticket` is what they want (None: a pass to show).
    AtDesk {
        bus: BusId,
        seat: usize,
        ticket: Option<usize>,
        done: bool,
    },
    Riding {
        bus: BusId,
        seat: usize,
    },
    /// Standing in the aisle by exit `exit`, waiting for it to open.
    AtExit {
        bus: BusId,
        exit: usize,
    },
    /// Walking away: first to `target`, then along the pavement; gone once out of sight.
    Leaving {
        target: DVec3,
        walk: Option<PedWalk>,
        walked: f32,
    },
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Strolling(_) => "strolling",
            State::ToStop { .. } => "walking to a stop",
            State::ToSpot { .. } => "walking to a waiting place",
            State::Waiting { .. } => "waiting",
            State::Queue { .. } => "queueing",
            State::Aboard {
                goal: Goal::Desk(_),
                ..
            } => "walking to the cash desk",
            State::Aboard {
                goal: Goal::Seat(_),
                ..
            } => "walking to a seat",
            State::Aboard {
                goal: Goal::Stamper(..),
                ..
            } => "walking to the validator",
            State::Aboard {
                goal: Goal::ExitWait(_),
                ..
            } => "walking to the exit",
            State::Aboard {
                goal: Goal::Exit(_),
                ..
            } => "getting off",
            State::AtDesk { .. } => "at the cash desk",
            State::Riding { .. } => "riding",
            State::AtExit { .. } => "waiting at the exit",
            State::Leaving { .. } => "leaving",
        }
    }
    fn bus(&self) -> Option<BusId> {
        match self {
            State::Queue { bus, .. }
            | State::Aboard { bus, .. }
            | State::AtDesk { bus, .. }
            | State::Riding { bus, .. }
            | State::AtExit { bus, .. } => Some(*bus),
            _ => None,
        }
    }
}

/// Where a person is: on the ground, or inside a bus at a point of its frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    Ground,
    Bus(BusId, Vec3),
}

/// Seconds after which a passenger's request at an exit lapses (see `write_pax_vars`).
const EXIT_REQ_LAPSE: f32 = 120.0;

/// How near an open door (m) a passenger holds it open (the light barrier's reach).
const DOORWAY: f64 = 1.6;

pub struct Person {
    id: u32,
    ty: Arc<HumanType>,
    /// Clothing variant (`HumanType::variant_texture`).
    variant: usize,
    meshes: Vec<(MeshId, usize)>,
    position: DVec3,
    heading: f64,
    /// Heading in the bus frame while inside one.
    lheading: f64,
    place: Place,
    /// Velocity in the plane the person walks in (ground or bus floor).
    vel: DVec2,
    pace: f64,
    activity: Activity,
    /// The animation: gait, feet, sitting, reaching, head.
    anim: Pose,
    state: State,
    /// Seconds in the current state.
    t_state: f32,
    /// Skinned positions and normals, per mesh.
    skins: Vec<(Vec<Vec3>, Vec<Vec3>)>,
    /// Interior light of the bus the person is in (0 outside).
    interior: f32,
    /// The interior light as drawn: it follows `interior` over a moment (stepping through
    /// the door, people lit up and went dark again from one frame to the next).
    lit: f32,
    /// The tilt of the floor the person stands on (a bus pitching under the brakes and
    /// leaning in a bend), without its heading: riders are drawn with it. Upright on the
    /// ground; drawn upright in a tilted bus, their feet sank through the floor on one side.
    tilt: Mat4,
    /// Stop boarded at; timetable stop to get off at (player's bus, -1 = at random);
    /// stops still to ride (timetable buses).
    from: i64,
    exit_stop: i32,
    stops_left: i32,
    /// The stop object a timetable bus's rider gets off at (None: see `stops_left`).
    exit_id: Option<i64>,
    /// The rider wants out at the stop the bus stands at.
    leaving_here: bool,
    /// A bus this person will not board (they just left it).
    avoid: Option<BusId>,
    /// Which of the stop's timetable targets they want (a fraction of the list, picked when
    /// they appear; see `goes_their_way`).
    target: f32,
    /// Wants to buy this ticket (None: has a pass, or stamps one at a validator).
    ticket: Option<usize>,
    /// Whether the ticket was decided - at the first door, for that bus, as OMSI does
    ///: a validator or a ticket, from one draw.
    ticket_decided: bool,
    /// Stamps a ticket at one of the bus's validators on the way in.
    stamps: bool,
    /// Stands still until this time (s of `Humans::time`): stamping.
    pause_until: f64,
    /// Age in years: the `.hum`'s `[age]`, else 40 as in OMSI. The
    /// ticket pack's tickets have age ranges (the reduced fare is for 6..13).
    age: f32,
    /// Seconds without getting nearer the goal while wanting to move; seconds left
    /// passing through others.
    stuck: f32,
    ghost: f32,
    /// Seconds a standing vehicle has stood in the way (see the crowd step).
    car_wait: f32,
    /// Seconds left going round something in the way off the pavement's line (a lamp post
    /// on the path): the corridor does not pull them back into it meanwhile.
    detour: f32,
    /// Which way round (+1 anticlockwise, -1 clockwise) while `detour` lasts: round a corner
    /// the sides' own choices flipped each other and people shuffled at a post.
    detour_side: f64,
    /// Seconds spent waiting behind somebody standing in front.
    blocked: f32,
    /// Why the person is standing, for `OMSI_DEBUG_PAX`.
    why: &'static str,
    why_logged: &'static str,
    /// Whether this person has ever been posed (an unposed model is the file's T-pose).
    skinned: bool,
    /// Frames since the last pose and where the person stood then (the
    /// feet of a mesh posed a frame ago stay on the floor when it is drawn there).
    since_posed: u32,
    posed_at: (DVec3, f64),
    /// Ankles of the last pose (model frame), for `OMSI_TRACE_PAX`.
    ankles: [Vec3; 2],
    /// A scripted test person (`OMSI_PAX_GALLERY`).
    puppet: Option<Puppet>,
    /// LAN play: one of the host's people, drawn where the host says (`mirror_set`).
    remote: bool,
}

impl Person {
    pub fn state_name(&self) -> String {
        format!("#{} {} ({})", self.id, self.state.name(), self.why)
    }
    pub fn position(&self) -> DVec3 {
        self.position
    }
    fn inside(&self, bus: BusId) -> bool {
        matches!(self.place, Place::Bus(b, _) if b == bus)
    }
    fn local(&self) -> Option<Vec3> {
        match self.place {
            Place::Bus(_, l) => Some(l),
            Place::Ground => None,
        }
    }
    fn label(&self) -> String {
        format!(
            "#{} {}",
            self.id,
            self.ty
                .def
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    }
}

/// What a person wants this frame.
struct Want {
    vel: DVec2,
    /// Heading to turn to when standing (world on the ground, bus frame inside).
    face: Option<f64>,
    give: f64,
    corridor: Option<(DVec2, DVec2, f64)>,
    /// What they do when not walking.
    idle: Activity,
    /// Stops behind somebody standing in front instead of pushing.
    follow: bool,
    /// Distance to where they are going (for the stuck detection), if anywhere.
    goal_dist: Option<f64>,
}

impl Want {
    fn stand(face: Option<f64>, idle: Activity) -> Want {
        Want {
            vel: DVec2::ZERO,
            face,
            give: 0.35,
            corridor: None,
            idle,
            follow: false,
            goal_dist: None,
        }
    }
}

/// Velocity towards `to`, easing into the stop over the last metre.
fn arrive(from: DVec2, to: DVec2, pace: f64) -> DVec2 {
    let d = to - from;
    let dist = d.length();
    if dist < 0.1 {
        return DVec2::ZERO;
    }
    let speed = (pace * dist.min(1.0)).max(if dist > 0.3 { 0.25 } else { 0.0 });
    d / dist * speed
}

/// Seconds after one passenger's greeting or complaint before anybody says another.
const CHAT_PAUSE: f64 = 12.0;

pub struct Humans {
    types: Vec<Arc<HumanType>>,
    pub people: Vec<Person>,
    rng: u64,
    next_id: u32,
    /// Seconds since the start.
    time: f64,
    /// Passenger cabins by vehicle files (the front vehicle and its coupled parts).
    cabins: HashMap<Vec<PathBuf>, Option<Arc<Cabin>>>,
    player_cabin: Option<Arc<Cabin>>,
    /// Which places of each bus are taken.
    seats: HashMap<BusId, Vec<bool>>,
    stops: HashMap<i64, StopInfo>,
    /// The first populate put people at the stops; later stops fill on foot when in sight.
    started: bool,
    ped: Option<PedNet>,
    hidden: Vec<usize>,
    /// GPU side of the human types, shared by everyone of a type: textures by file and the
    /// materials of every (type, mesh) - each person used to upload its own copies - and
    /// the meshes and instances of the people who have gone, taken over by the next person
    /// of the same type (the skinned vertices are rewritten anyway). Without that every
    /// passenger who ever appeared kept a mesh, its textures and materials on the GPU.
    gpu_textures: HashMap<PathBuf, Option<omsi_render::TextureId>>,
    /// Per (type, clothing variant, mesh): its materials, and the meshes and instances of
    /// people who have gone, kept for the next person dressed alike.
    gpu_materials: HashMap<(usize, usize, usize), Vec<MaterialId>>,
    spare: HashMap<(usize, usize, usize), Vec<(MeshId, usize)>>,
    /// Stop the player's bus is serving (standing at it).
    served_stop: Option<i64>,
    /// `self.time` the player's bus started serving `served_stop`: for the same grace
    /// period as the timetable buses' `ai_visits`, so a waiting passenger does not turn
    /// away the instant the driver pulls in and the doors have not opened yet.
    served_stop_since: f64,
    /// Doorway of a bus busy for this many seconds more (one person at a time).
    door_busy: HashMap<(BusId, bool, usize), f32>,
    /// Timetable buses at a stop: id → (stop, time the visit began).
    ai_visits: HashMap<u64, (i64, f64)>,
    /// When each bus last had a door open (the passengers' clock).
    last_door_open: HashMap<BusId, f64>,
    /// Timetable buses whose people aboard have been seated (`seed_ai_riders`).
    ai_seeded: HashSet<u64>,
    /// Timetable buses to keep at their stop for a few seconds more (for the traffic).
    holds: Vec<(u64, f32)>,
    /// Door requests for the timetable buses' scripts: (bus, entries, exits).
    ai_requests: Vec<(u64, Vec<bool>, Vec<bool>)>,
    pub tickets: Option<Arc<omsi_content::tickets::TicketPack>>,
    /// Current ticket request at the player's cash desk: (ticket name, value).
    pub request: Option<(String, f32)>,
    /// Payment on the desk: (paid, ticket value), and the change still owed after the ticket.
    pub paid: Option<(f32, f32)>,
    pub change_due: Option<f32>,
    pub money: Option<crate::money::Money>,
    /// A rider pressed the stop button for the next stop (the app fires `door_haltewunsch`).
    pub stop_request: bool,
    /// Somebody at the kerb pressed the outside door opener (`door_aussenoeffner`).
    pub door_request: bool,
    /// Stop whose waiting passengers have already pressed the outside opener once.
    pressed_at_stop: Option<i64>,
    /// Tickets sold at the cash desk this session and what they were worth.
    pub tickets_sold: u32,
    pub ticket_cash: f32,
    /// Passengers that reached the cash desk, and those the driver served there.
    pub boarded: u32,
    pub served: u32,
    /// OMSI's rating counters: people who stepped into the player's bus
    /// and of those who had nothing to complain about (comfort = content / stepped in);
    /// tickets asked for and the points for selling them, two for the right change, one
    /// for the wrong (ticket selling = points / 2 × asked).
    pub stepped_in: u32,
    pub content: u32,
    pub ticket_requests: u32,
    pub ticket_points: u32,
    /// `PAX_Entry<i>_Req`: somebody at the kerb wants in through entry `i`.
    pub entry_req: Vec<bool>,
    /// `PAX_Exit<i>_Req`: somebody inside wants out through exit `i`.
    pub exit_req: Vec<bool>,
    /// Seconds each passenger on the way out has asked for the door (see `EXIT_REQ_LAPSE`).
    exit_req_time: hashbrown::HashMap<u32, f32>,
    /// Timetable stop index the stop request was already made for.
    requested_for: Option<i32>,
    sync_frame: u32,
    debug_last_next: i32,
    /// Feet put down since the app last collected them (see [`Humans::take_footfalls`]).
    footfalls: Vec<ambience::Footfall>,
    /// `[trafficdensity_passenger]` factor for the current hour (set by the app).
    pub density: f32,
    /// The clock's time of day in seconds (set by the app): day tickets sell by it.
    pub time_of_day: f64,
    /// How late the player's bus is on its duty (s; set by the app): over five minutes,
    /// boarding passengers may say so.
    pub delay: f64,
    /// The game's folder (the ticket pack's voices are found from it).
    root: std::path::PathBuf,
    /// What passengers said since the app last collected it (see `take_voice_lines`).
    voice_lines: Vec<VoiceLine>,
    /// When each voice file was last said (seconds of `time`): OMSI keeps such a list
    /// and says a greeting or a complaint only when that
    /// very file has not been heard for 10 s - without it every boarding passenger said
    /// "Hallo" one after the other.
    voice_said: HashMap<std::path::PathBuf, f64>,
    /// What passengers may say (the `pax_voices` setting): 0 everything, 1 only the
    /// ticket they ask for, 2 nothing.
    pub voices: u8,
    /// When anybody last greeted or complained (seconds of `time`).
    last_chat: f64,
    /// Avatars (the player on foot, other players' walkers): key → person id, and what
    /// the game wants of each this frame.
    avatars: HashMap<u32, u32>,
    avatar_cmds: HashMap<u32, AvatarCmd>,
    /// Avatars not drawn (the first-person view), by person id.
    avatar_hidden: HashMap<u32, bool>,
    /// The buses of the last tick (for the avatars' seats and doors).
    last_buses: Vec<BusNow>,
    /// Only avatars: nobody else is put on the map (the passengers are off).
    pub avatar_only: bool,
    /// The player has got up and left the wheel: a standing bus with a door open is left
    /// by its riders as at a terminus (see `ALL_OUT_STOP`).
    pub driver_away: bool,
    /// Per bus stop, Omsi.exe's station targets (0x61cb18, `Schedule::stop_targets`): the
    /// stops the trips go on to, each with the termini of those trips. A person waiting there
    /// wants one of them and boards only a bus showing one of its termini; at a stop no trip
    /// goes on from, anybody takes the first bus (0x61c33c).
    pub stop_targets: Option<HashMap<i64, Vec<HashSet<String>>>>,
    /// Buses whose validator somebody used since the app last looked (`take_stamped`).
    stamped: Vec<BusId>,
    /// Pedestrians to keep strolling near the player (scaled by `density`).
    pub pedestrians: usize,
    stroll_timer: f32,
    /// Passengers pay the exact fare: no change is ever due.
    pub exact_fare: bool,
    /// How passengers board (`boarding` in the settings): `auto` - pay and take the
    /// ticket by themselves after a moment; `pay` - wait at the desk for the driver to
    /// sell it (and show a pass after `PAY_PATIENCE`); `walk` - no cash desk at all.
    pub boarding: String,
    /// The driver pressed the ticket key (`ticket_give`): sell the requested ticket.
    pub give_ticket: bool,
    /// The driver pressed `change_give`: all the change owed goes on the tray at once.
    pub give_change_all: bool,
    /// The key that sells a ticket, as the HUD names it.
    pub ticket_key: String,
    /// Where the player looks from (set by the app every frame).
    pub eye: Option<Eye>,
    /// Around where people are kept (the player's bus, else the camera).
    center: DVec3,
    /// A line for the HUD about something that just happened.
    message: Option<String>,
    last_hint: Option<String>,
    /// Frames ticked, total and longest tick (ms).
    tick_stats: (u32, f64, f64),
    /// Speed, heading and floor acceleration of each bus last frame (for the riders' balance).
    bus_motion: HashMap<BusId, (f64, f64, DVec2)>,
    /// The scripted test people of `OMSI_PAX_GALLERY` are there.
    gallery: bool,
    /// `types` has been cut down to the map's `humans.txt`.
    map_humans_done: bool,
    /// Simulation time of the last `sync`.
    last_sync: f64,
    /// Frames synced, people posed and skinned, the time that took and the part of it spent
    /// uploading (ms), in total.
    pose_stats: (u32, usize, f64, f64),
    /// `OMSI_TRACE_PAX=<csv>`: every person near the eye, every frame (see `sync`).
    trace: Option<std::io::BufWriter<std::fs::File>>,
    /// `World::tiles_generation` the stops were last checked against.
    tiles_seen: u64,
    /// Stops to set up again with what their tiles hold now: (id, populated before, seconds
    /// to the next arrival).
    rebuild: Vec<(i64, bool, f32)>,
    /// LAN play: this game draws the host's people instead of its own (`lan_world`).
    mirror: bool,
    /// LAN play: where the other players are (host): people are kept around them too.
    pub lan_centers: Vec<DVec3>,
    /// LAN play: the other players' buses this frame (`set_remote_buses`), for their riders
    /// to sit in. Nobody of ours boards them: their doors count as shut.
    remote_now: Vec<BusNow>,
    /// The vehicles the player placed and is not driving now (`placed_bus_id`): their
    /// riders stay in them when the player drives another.
    placed_now: Vec<BusNow>,
    /// LAN play (client): waiting people our bus could take, to ask the host for, and when
    /// each was last asked for.
    claims_out: Vec<u32>,
    claimed: HashMap<u32, f64>,
}

impl Humans {
    /// LAN uses the room id as the shared source of randomness.  This keeps the
    /// initial pedestrian selection and their generated identities identical on
    /// the host and clients; subsequent movement remains simulation-local.
    pub fn set_lan_seed(&mut self, seed: u64) {
        self.rng = (seed ^ 0xA5A5_5A5A_1F2E_3D4C) as u64 | 1;
    }

    pub fn new(root: &Path) -> Humans {
        let mut types = Vec::new();
        // `Humans/<group>/*.hum` of every content root (an installed map or mod brings its
        // own people); a file of the same group and name higher up replaces the stock one
        let mut roots = omsi_cfg::content_dirs("Humans");
        if roots.is_empty() {
            roots.push(root.join("Humans"));
        }
        // (group, file name, path), sorted by group and name as the single folder used to be
        let mut found: Vec<(std::ffi::OsString, std::ffi::OsString, std::path::PathBuf)> =
            Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for r in &roots {
            for (group, is_dir) in omsi_cfg::vfs::list_dir(r).unwrap_or_default() {
                if !is_dir {
                    continue;
                }
                let d = r.join(&group);
                for (n, _) in omsi_cfg::vfs::list_dir(&d).unwrap_or_default() {
                    let lower = n.to_string_lossy().to_ascii_lowercase();
                    if !lower.ends_with(".hum") || lower.contains("driver") {
                        continue;
                    }
                    if seen.insert(format!(
                        "{}/{lower}",
                        group.to_string_lossy().to_ascii_lowercase()
                    )) {
                        found.push((group.clone(), n.clone(), d.join(&n)));
                    }
                }
            }
        }
        found.sort();
        let files: Vec<std::path::PathBuf> = found.into_iter().map(|(_, _, p)| p).collect();
        for f in files {
            match HumanType::load(&f) {
                Ok(t) => types.push(Arc::new(t)),
                Err(e) => log::warn!("human {}: {e:#}", f.display()),
            }
        }
        log::info!("humans: {} types", types.len());
        if omsi_cfg::env::var_os("OMSI_DEBUG_HUMANS").is_some() {
            for t in &types {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for m in &t.meshes {
                    for v in &m.data.positions {
                        lo = lo.min(v.z);
                        hi = hi.max(v.z);
                    }
                }
                log::info!(
                    "  {} z {lo:.2}..{hi:.2}",
                    t.def.path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
        Humans {
            types,
            people: Vec::new(),
            rng: 0x1234_5678_9ABC_DEF1,
            next_id: 1,
            time: 0.0,
            cabins: HashMap::new(),
            player_cabin: None,
            seats: HashMap::new(),
            stops: HashMap::new(),
            started: false,
            ped: None,
            hidden: Vec::new(),
            gpu_textures: HashMap::new(),
            gpu_materials: HashMap::new(),
            spare: HashMap::new(),
            served_stop: None,
            served_stop_since: 0.0,
            door_busy: HashMap::new(),
            ai_visits: HashMap::new(),
            last_door_open: HashMap::new(),
            ai_seeded: HashSet::new(),
            holds: Vec::new(),
            ai_requests: Vec::new(),
            tickets: None,
            request: None,
            paid: None,
            change_due: None,
            money: None,
            stop_request: false,
            door_request: false,
            pressed_at_stop: None,
            tickets_sold: 0,
            ticket_cash: 0.0,
            boarded: 0,
            served: 0,
            stepped_in: 0,
            content: 0,
            ticket_requests: 0,
            ticket_points: 0,
            entry_req: Vec::new(),
            exit_req: Vec::new(),
            exit_req_time: Default::default(),
            requested_for: None,
            sync_frame: 0,
            debug_last_next: -99,
            footfalls: Vec::new(),
            density: 1.0,
            time_of_day: 12.0 * 3600.0,
            delay: 0.0,
            root: root.to_path_buf(),
            voice_lines: Vec::new(),
            voice_said: HashMap::new(),
            voices: 0,
            last_chat: -1e9,
            avatars: HashMap::new(),
            avatar_cmds: HashMap::new(),
            avatar_hidden: HashMap::new(),
            last_buses: Vec::new(),
            avatar_only: false,
            driver_away: false,
            stop_targets: None,
            stamped: Vec::new(),
            pedestrians: 14,
            stroll_timer: 0.0,
            exact_fare: true,
            boarding: "auto".into(),
            give_ticket: false,
            give_change_all: false,
            ticket_key: "T".into(),
            eye: None,
            center: DVec3::ZERO,
            message: None,
            last_hint: None,
            tick_stats: (0, 0.0, 0.0),
            bus_motion: HashMap::new(),
            gallery: false,
            map_humans_done: false,
            last_sync: 0.0,
            pose_stats: (0, 0, 0.0, 0.0),
            trace: omsi_cfg::env::var("OMSI_TRACE_PAX").ok().and_then(|f| std::fs::File::create(f).ok()).map(|f| {
                use std::io::Write;
                let mut w = std::io::BufWriter::new(f);
                let _ = writeln!(w, "t,id,state,ground,posed,x,y,z,heading,lx,ly,lz,rx,ry,rz,vx,vy");
                w
            }),
            tiles_seen: 0,
            rebuild: Vec::new(),
            mirror: false,
            lan_centers: Vec::new(),
            remote_now: Vec::new(),
            placed_now: Vec::new(),
            claims_out: Vec::new(),
            claimed: HashMap::new(),
        }
    }

    /// The loaded tiles changed: a stop whose tile went takes the people bound to it along
    /// (it is far out of sight: tiles go only well beyond the view), a stop nobody uses is
    /// set up again with the waiting places and ground there now, and nobody stays on ground
    /// that is gone.
    fn tiles_changed(&mut self, world: &World) {
        let present: HashMap<i64, DVec3> =
            world.bus_stops.lock().iter().map(|s| (s.0, s.1)).collect();
        let bound = |st: &State| -> Option<i64> {
            match st {
                State::ToStop { stop, .. }
                | State::ToSpot { stop, .. }
                | State::Waiting { stop, .. }
                | State::Queue { stop, .. } => Some(*stop),
                _ => None,
            }
        };
        let used: HashSet<i64> = self.people.iter().filter_map(|p| bound(&p.state)).collect();
        let gone: Vec<i64> = self
            .stops
            .keys()
            .copied()
            .filter(|id| !present.contains_key(id))
            .collect();
        let mut removed = 0usize;
        for i in (0..self.people.len()).rev() {
            let p = &self.people[i];
            let lost_stop = bound(&p.state).map(|s| gone.contains(&s)).unwrap_or(false);
            let lost_ground = p.place == Place::Ground
                && p.puppet.is_none()
                && !world.has_ground(p.position.x, p.position.y);
            if lost_stop || lost_ground {
                self.release(i);
                let p = self.people.swap_remove(i);
                if debug_pax() {
                    log::info!(
                        "t={:.1} pax {} taken away with its tile ({}){}",
                        self.time,
                        p.label(),
                        p.state.name(),
                        if self.seen(p.position) {
                            " IN SIGHT"
                        } else {
                            ""
                        }
                    );
                }
                self.retire(&p);
                removed += 1;
            }
        }
        for id in &gone {
            self.stops.remove(id);
        }
        // set up again (keeping whether it was populated) where nobody waits yet
        let mut rebuilt = 0usize;
        let idle: Vec<i64> = self
            .stops
            .keys()
            .copied()
            .filter(|id| !used.contains(id))
            .collect();
        for id in idle {
            let Some(old) = self.stops.remove(&id) else {
                continue;
            };
            self.rebuild.push((id, old.seeded, old.next_arrival));
            rebuilt += 1;
        }
        if debug_pax()
            || (omsi_cfg::env::var_os("OMSI_PROFILE").is_some() && (removed > 0 || !gone.is_empty()))
        {
            log::info!("people: tiles changed: {} stops gone, {rebuilt} set up again, {removed} people taken away", gone.len());
        }
    }

    /// The buses somebody stamped a ticket in since the last call (the app fires their
    /// `ev_Stamper` sound trigger): `None` the player's, else the AI car's id.
    pub fn take_stamped(&mut self) -> Vec<Option<u64>> {
        std::mem::take(&mut self.stamped)
            .into_iter()
            .map(|b| match b {
                BusId::Ai(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// Lines passengers said since the last call (the app plays them where they stand).
    pub fn take_voice_lines(&mut self) -> Vec<VoiceLine> {
        std::mem::take(&mut self.voice_lines)
    }

    /// Person `i` says `name`: the ticket pack's `[voicepath]` (else
    /// its own folder), the
    /// `.hum`'s `[voice]` folder in it and `<name>.wav` - `TicketPacks\Berlin_1\M4\Hello_1.wav`.
    /// Nothing is said when the pack has no voices or the file is not there.
    fn say(&mut self, i: usize, name: &str) {
        self.say_ex(i, name, true)
    }

    /// `limited`: said only when the same file has not been said for 10 s (greetings and
    /// complaints; the ticket asked for, "thanks" and the missing change always are).
    fn say_ex(&mut self, i: usize, name: &str, limited: bool) {
        // the player may have silenced them (settings), all but the ticket they ask for
        match self.voices {
            2 => return,
            1 if !name.starts_with("Ticket_") => return,
            _ => {}
        }
        // Greetings and complaints: one at a time for the whole bus. OMSI only keeps
        // the same file from being said twice within 10 s, and with a dozen people
        // boarding every other one said hello - the saloon never stopped talking, which
        // is not how the original sounds: a few words now and then.
        if limited && self.time - self.last_chat < CHAT_PAUSE && self.time >= self.last_chat {
            return;
        }
        // (without a `[voicepath]` the pack's own folder: Berlin_1 and Berlin_86 carry the
        // voices themselves and name no path; the later packs point at theirs)
        let Some(base) = self.tickets.as_ref().and_then(|t| match &t.voice_path {
            Some(vp) if !vp.trim().is_empty() => Some(omsi_cfg::resolve_path(&self.root, vp.trim())),
            _ => t.path.parent().map(|p| p.to_path_buf()),
        }) else {
            return;
        };
        let voice = self.people[i].ty.def.voice.trim().to_string();
        if voice.is_empty() {
            return;
        }
        let dir = omsi_cfg::resolve_path(&base, &voice);
        let path = omsi_cfg::resolve_path(&dir, &format!("{name}.wav"));
        if !omsi_cfg::vfs::is_file(&path) {
            return;
        }
        if limited {
            if let Some(&t) = self.voice_said.get(&path) {
                if self.time - t < 10.0 && self.time >= t {
                    return;
                }
            }
        }
        self.voice_said.insert(path.clone(), self.time);
        if limited {
            self.last_chat = self.time;
        }
        if debug_pax() {
            log::info!("t={:.1} pax {} says {name}", self.time, self.people[i].label());
        }
        self.voice_lines.push(VoiceLine { position: self.people[i].position + DVec3::new(0.0, 0.0, 1.6), path });
    }

    /// Stepping into the player's bus (the original; people boarding other buses
    /// say nothing): a complaint when there is something to complain about, each with the
    /// pack's `whinge_prop` and the first that comes out winning -
    /// * too dark: the saloon light off (under half) while the daylight is under 0.2..0.5;
    /// * too hot: the cabin over 25..34 °C and 3..7 °C over the air outside, or between
    ///   half the outside temperature plus 20..29 °C and 25 °C - "too wet" instead when the
    ///   air in it is over 90..100 % humid;
    /// * too cold: the cabin under 8..17 °C and under the outside temperature plus 5..9 °C,
    ///   or 10..19 °C under the air outside;
    /// * too late: the bus more than five minutes behind its timetable -
    /// otherwise, with its `chattiness`, a greeting: "Hello", in the morning and the evening
    /// now and then "Good morning" / "Good evening".
    fn greet_or_complain(&mut self, i: usize, bus: BusId, interior: f32, air: CabinAir) {
        if bus != BusId::Player {
            return;
        }
        let Some((whinge, chat)) = self.tickets.as_ref().map(|t| (t.whinge_prop, t.chattiness)) else { return };
        let mut code = 0;
        if interior < 0.5 && ((self.rand_f() * 0.3 + 0.2) as f32) > air.brightness && (self.rand_f() as f32) < whinge {
            code = 1;
        }
        if let Some(t) = air.temp {
            let out = air.outside;
            let r10 = |h: &mut Self| (h.rand() % 10) as f32;
            let r5 = |h: &mut Self| (h.rand() % 5) as f32;
            let mut hot = false;
            if t > 25.0 + r10(self) && t > r5(self) + 3.0 + out {
                hot = true;
            }
            if !hot && t > r10(self) + 0.5 * out + 20.0 && t < 25.0 {
                hot = true;
            }
            if hot && code == 0 && (self.rand_f() as f32) < whinge {
                code = if ((self.rand_f() * 0.1 + 0.9) as f32) < air.rel_hum { 5 } else { 3 };
            }
            let cold = (t < r10(self) + 8.0 && t < r5(self) + out + 5.0) || t < out - r10(self) - 10.0;
            if cold && code == 0 && (self.rand_f() as f32) < whinge {
                code = 4;
            }
        }
        if code == 0 && self.delay > 300.0 && (self.rand_f() as f32) < whinge {
            code = 2;
        }
        self.stepped_in += 1;
        if code == 0 {
            self.content += 1;
        }
        let k = 1 + self.rand() % 2;
        match code {
            1 => return self.say(i, &format!("TooDark_{k}")),
            2 => return self.say(i, &format!("TooLate_{k}")),
            3 => return self.say(i, &format!("TooHot_{k}")),
            4 => return self.say(i, &format!("TooCold_{k}")),
            5 => return self.say(i, "TooWet_1"),
            _ => {}
        }
        if (self.rand_f() as f32) >= chat {
            return;
        }
        let h = self.time_of_day.rem_euclid(86_400.0) / 3600.0;
        let daypart = if (4.0..10.0).contains(&h) { 1 } else if h >= 18.0 { 2 } else { 0 };
        let k = if daypart == 0 { self.rand() % 2 } else { self.rand() % 3 };
        match (k, daypart) {
            (2, 1) => self.say(i, "GoodMorning_1"),
            (2, _) => self.say(i, "GoodEvening_1"),
            _ => self.say(i, &format!("Hello_{}", k + 1)),
        }
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn rand_f(&mut self) -> f64 {
        (self.rand() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Whether the player could see somebody standing at `p`.
    fn seen(&self, p: DVec3) -> bool {
        match self.eye {
            None => (p - self.center).length() < 150.0,
            Some(e) => {
                let d = p + DVec3::Z * 0.9 - e.pos;
                let dist = d.length();
                if dist > 230.0 {
                    return false;
                }
                dist < 3.0 || d.dot(e.fwd) / dist > e.cos_half
            }
        }
    }

    /// The cabin of a vehicle with the parts coupled behind it.
    fn cabin_for(&mut self, v: &VehicleInstance) -> Option<Arc<Cabin>> {
        let parts = train_parts(v);
        let key: Vec<PathBuf> = parts.iter().map(|p| p.0.path.clone()).collect();
        if let Some(c) = self.cabins.get(&key) {
            return c.clone();
        }
        let cabin = Cabin::load_train(&parts).map(Arc::new);
        if let Some(c) = cabin.as_ref().filter(|c| c.parts.len() > 1) {
            log::info!("passenger cabin of {}: {} sections joined ({} places, {} entries, {} exits, {} path points)", v.ty.def.path.file_name().unwrap_or_default().to_string_lossy(), c.parts.len(), c.seats.len(), c.entries.len(), c.exits.len(), c.graph.points.len());
        }
        self.cabins.insert(key, cabin.clone());
        cabin
    }

    /// The footsteps taken since the last call, for the environment sounds. They pile up
    /// only between two frames; a run without audio never looks at them, so the list is
    /// dropped once it grows past a crowd's worth of steps.
    /// `OMSI_TRACE_PAX` is writing a trace.
    pub fn tracing(&self) -> bool {
        self.trace.is_some()
    }

    pub fn take_footfalls(&mut self) -> Vec<ambience::Footfall> {
        if self.footfalls.len() > 256 {
            self.footfalls.clear();
        }
        std::mem::take(&mut self.footfalls)
    }

    /// People currently in the player's bus.
    pub fn riding(&self) -> usize {
        self.people
            .iter()
            .filter(|p| p.inside(BusId::Player))
            .count()
    }

    /// People walking the footpaths of the traffic network: (lane, distance along it). The
    /// traffic gives way to them at crossings and presses the pedestrian lights' buttons
    /// for them.
    /// Everybody on foot on the ground, for the traffic to stop for: position, velocity
    /// and whether they wait at a stop (a bus pulls up right beside those).
    pub fn on_foot(&self) -> Vec<(DVec2, DVec2, bool)> {
        self.people
            .iter()
            .filter(|p| p.place == Place::Ground)
            .map(|p| {
                let waiting = matches!(
                    p.state,
                    State::Waiting { .. } | State::Queue { .. } | State::ToSpot { .. }
                );
                (p.position.truncate(), p.vel, waiting)
            })
            .collect()
    }

    pub fn strollers(&self) -> Vec<(usize, f32)> {
        self.people
            .iter()
            .filter_map(|p| match &p.state {
                State::Strolling(walk)
                | State::ToStop { walk, .. }
                | State::Leaving {
                    walk: Some(walk), ..
                } => walk
                    .legs
                    .get(walk.leg)
                    .map(|leg| (leg.lane, leg.dist(walk.s))),
                _ => None,
            })
            .collect()
    }

    /// Seats of the player's bus from its `[passengercabin]`, and the engine's side of the
    /// ticket printer: `GivenTicket` is -1 until the driver hands a ticket over (the stock
    /// `Ticketprinter.osc` never sets it, OMSI starts it at -1 - left at 0 the first
    /// passenger took ticket 0 without the driver doing anything).
    pub fn set_cabin(&mut self, vehicle: &mut VehicleInstance) {
        vehicle.set_engine_var("GivenTicket", -1.0);
        match self.cabin_for(vehicle) {
            Some(c) => {
                log::info!("passenger cabin: {} places ({} seats), {} entries, {} exits, {} path points, desk {:?}", c.seats.len(), c.seats.iter().filter(|s| s.seated).count(), c.entries.len(), c.exits.len(), c.graph.points.len(), c.desk.map(|d| d.0));
                if debug_pax() {
                    for (i, e) in c.entries.iter().enumerate() {
                        log::info!(
                            "  entry {i}: inside {:?} wait {:?} sells {}",
                            e.inside,
                            e.wait,
                            e.sells
                        );
                    }
                    for (i, e) in c.exits.iter().enumerate() {
                        log::info!("  exit {i}: inside {:?} wait {:?}", e.inside, e.wait);
                    }
                    for (i, s) in c.seats.iter().enumerate() {
                        log::info!(
                            "  seat {i}: pos {:?} floor {:?} rot {:.0} seated {}",
                            s.pos,
                            s.floor,
                            s.rot,
                            s.seated
                        );
                    }
                }
                self.seats.insert(BusId::Player, vec![false; c.seats.len()]);
                self.entry_req = vec![false; c.entries.len().max(1)];
                self.exit_req = vec![false; c.exits.len().max(1)];
                self.player_cabin = Some(c);
            }
            None => log::info!("{}: no passenger cabin", vehicle.ty.def.path.display()),
        }
    }

    /// Timetable stop where a boarding rider will get off: one to four stops ahead (-1
    /// without a timetable: decided at random at each stop).
    fn choose_exit(&mut self, bus: Option<&VehicleInstance>, world: &World) -> i32 {
        let Some(b) = bus else { return -1 };
        let n = b.host.tt_stops.len() as i32;
        if n == 0 {
            return -1;
        }
        let next = b.host.tt_busstop_index;
        // the stops after the next one, weighed as Omsi.exe weighs them (see `draw_exit`);
        // with nothing to weigh, the end of the trip
        let from = (next + 1).clamp(0, n - 1);
        let ids: Vec<i64> = (from..n).map(|k| b.host.tt_stop_ids.get(k as usize).copied().unwrap_or(0)).collect();
        match self.draw_exit(&ids, world) {
            Some(k) => from + k as i32,
            None => n - 1,
        }
    }

    /// Where a boarding passenger gets off among the stops `ahead` (map objects, in order):
    /// Omsi.exe draws it at random, each stop as likely as its passengers-alighting number
    /// says (0x61baa8: Random x the total, then down the list until it is used up; see
    /// `tiles::stop_exit_weight`). None when there is nothing to weigh.
    fn draw_exit(&mut self, ahead: &[i64], world: &World) -> Option<usize> {
        let w: Vec<f32> = ahead.iter().map(|&id| if id == 0 { 0.5 } else { world.stop_exit_weight(id) }).collect();
        let total: f32 = w.iter().sum();
        if !(total > 0.0) {
            return None;
        }
        let mut r = self.rand_f() as f32 * total;
        for (k, wk) in w.iter().enumerate() {
            if r < *wk {
                return Some(k);
            }
            r -= wk;
        }
        Some(w.len() - 1)
    }

    /// Seat `n` passengers in the player's bus straight away, each with the stop they
    /// will get off at. Used to start a run mid-route (and by the passenger tests).
    pub fn seed_riders(
        &mut self,
        n: usize,
        bus: &VehicleInstance,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
    ) {
        let Some(cabin) = self.player_cabin.clone() else {
            return;
        };
        let frames = part_frames(bus, &cabin);
        let rot = bus.body_rotation();
        // OMSI_SEED_SEATS=i,j,k: seat exactly those places (debugging a specific bench),
        // instead of the usual n nearest-the-door places, and never have them leave.
        if let Ok(list) = omsi_cfg::env::var("OMSI_SEED_SEATS") {
            for tok in list.split(',').filter(|s| !s.trim().is_empty()) {
                let Ok(seat) = tok.trim().parse::<usize>() else {
                    continue;
                };
                if cabin.seats.get(seat).is_none()
                    || self
                        .seats
                        .get(&BusId::Player)
                        .and_then(|v| v.get(seat))
                        .copied()
                        .unwrap_or(true)
                {
                    continue;
                }
                self.seats.get_mut(&BusId::Player).unwrap()[seat] = true;
                let s = &cabin.seats[seat];
                let pos = train_point(bus.position, &rot, &frames, s.floor);
                let heading = train_heading(bus.heading, &frames, s.floor);
                if let Some(i) = self.spawn(
                    world,
                    renderer,
                    scene,
                    pos,
                    heading + s.rot as f64,
                    State::Riding {
                        bus: BusId::Player,
                        seat,
                    },
                ) {
                    let p = &mut self.people[i];
                    p.place = Place::Bus(BusId::Player, s.floor);
                    p.lheading = s.rot as f64;
                    p.exit_stop = i32::MAX;
                    p.activity = if s.seated {
                        Activity::Sit
                    } else {
                        Activity::Stand
                    };
                }
            }
            log::info!(
                "{} passengers seated at OMSI_SEED_SEATS ({} riding)",
                list.split(',').count(),
                self.riding()
            );
            return;
        }
        for _ in 0..n {
            let taken = self.seats.get(&BusId::Player).cloned().unwrap_or_default();
            let luck: Vec<f32> = (0..taken.len()).map(|_| self.rand_f() as f32).collect();
            let near = cabin
                .entries
                .first()
                .map(|e| e.inside)
                .unwrap_or(Vec3::ZERO);
            let Some(seat) = cabin.choose_seat(&taken, near, |i| luck[i]) else {
                break;
            };
            self.seats.get_mut(&BusId::Player).unwrap()[seat] = true;
            // riders already aboard may want the very stop the bus is at
            let n = bus.host.tt_stops.len() as i32;
            let exit = if n == 0 {
                -1
            } else {
                // (drawn as a boarding passenger's: `draw_exit`)
                let from = bus.host.tt_busstop_index.clamp(0, n - 1);
                let ids: Vec<i64> = (from..n).map(|k| bus.host.tt_stop_ids.get(k as usize).copied().unwrap_or(0)).collect();
                self.draw_exit(&ids, world).map(|k| from + k as i32).unwrap_or(n - 1)
            };
            let s = &cabin.seats[seat];
            let pos = train_point(bus.position, &rot, &frames, s.floor);
            let heading = train_heading(bus.heading, &frames, s.floor);
            if let Some(i) = self.spawn(
                world,
                renderer,
                scene,
                pos,
                heading + s.rot as f64,
                State::Riding {
                    bus: BusId::Player,
                    seat,
                },
            ) {
                let exit_id = usize::try_from(exit).ok().and_then(|k| bus.host.tt_stop_ids.get(k)).copied().filter(|&x| x != 0);
                let p = &mut self.people[i];
                p.place = Place::Bus(BusId::Player, s.floor);
                p.lheading = s.rot as f64;
                p.exit_stop = exit;
                p.exit_id = exit_id;
                p.activity = if s.seated {
                    Activity::Sit
                } else {
                    Activity::Stand
                };
            }
        }
        log::info!("{n} passengers already aboard ({} riding)", self.riding());
    }

    /// A timetable bus that comes near for the first time has its people aboard already
    /// (`BusService::riders`, by the hour): seated where a rider would sit, or standing
    /// once the seats are taken, each getting off a few stops on. OMSI's AI buses drive
    /// with people in them; empty ones looked like buses put out for a test.
    fn seed_ai_riders(
        &mut self,
        buses: &[BusNow],
        traffic: &Traffic,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
    ) {
        let alive: HashSet<u64> = traffic.cars.iter().map(|c| c.id).collect();
        self.ai_seeded.retain(|id| alive.contains(id));
        for bn in buses {
            let BusId::Ai(id) = bn.id else { continue };
            if self.ai_seeded.contains(&id) {
                continue;
            }
            // (seated only once someone could see into it: it comes into the list 400 m off)
            let from_eye = self.eye.map(|e| (bn.pos - e.pos).length()).unwrap_or(f64::MAX);
            if (bn.pos - self.center).length().min(from_eye) > 220.0 {
                continue;
            }
            self.ai_seeded.insert(id);
            let Some(car) = traffic.cars.iter().find(|c| c.id == id) else { continue };
            let n = car.bus.as_ref().map(|b| b.riders as usize).unwrap_or(0);
            let cabin = bn.cabin.clone();
            let near = cabin.entries.first().map(|e| e.inside).unwrap_or(Vec3::ZERO);
            let mut seated = 0;
            for _ in 0..n {
                let taken = self.seats.get(&bn.id).cloned().unwrap_or_default();
                if taken.len() != cabin.seats.len() {
                    break;
                }
                let luck: Vec<f32> = (0..taken.len()).map(|_| self.rand_f() as f32).collect();
                let Some(seat) = cabin.choose_seat(&taken, near, |i| luck[i]) else {
                    break;
                };
                self.seats.get_mut(&bn.id).unwrap()[seat] = true;
                let st = &cabin.seats[seat];
                let pos = train_point(bn.pos, &bn.rot, &bn.trailers, st.floor);
                let heading = train_heading(bn.heading, &bn.trailers, st.floor);
                // (where they get off: as a boarding passenger draws it)
                let ahead: Vec<i64> = match bn.id {
                    BusId::Ai(id) => traffic
                        .cars
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| c.bus.as_ref())
                        .map(|b| b.stops.iter().map(|st| st.id).filter(|&x| x != 0).collect())
                        .unwrap_or_default(),
                    BusId::Player => Vec::new(),
                };
                let exit_id = self.draw_exit(&ahead, world).map(|k| ahead[k]);
                if let Some(i) = self.spawn(
                    world,
                    renderer,
                    scene,
                    pos,
                    heading + st.rot as f64,
                    State::Riding { bus: bn.id, seat },
                ) {
                    let p = &mut self.people[i];
                    p.place = Place::Bus(bn.id, st.floor);
                    p.lheading = st.rot as f64;
                    p.from = -1;
                    p.stops_left = i32::MAX;
                    p.exit_id = exit_id;
                    p.exit_stop = -1;
                    p.activity = if st.seated { Activity::Sit } else { Activity::Stand };
                    seated += 1;
                } else {
                    self.seats.get_mut(&bn.id).unwrap()[seat] = false;
                    break;
                }
            }
            if debug_pax() && n > 0 {
                log::info!("t={:.1} timetable bus {id}: {seated} of {n} people aboard seated", self.time);
            }
        }
    }

    fn spawn(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        position: DVec3,
        heading: f64,
        state: State,
    ) -> Option<usize> {
        self.spawn_as(world, renderer, scene, position, heading, state, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_as(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        position: DVec3,
        heading: f64,
        state: State,
        kind: Option<usize>,
    ) -> Option<usize> {
        self.use_map_humans(world);
        if self.types.is_empty() {
            return None;
        }
        // on the surface they will walk on, not on the bare terrain under a pavement
        // (they stood in the asphalt and climbed out of it when they started walking)
        let mut position = position;
        if let Some(z) = world.walk_height_near(position.x, position.y, position.z) {
            if (z - position.z).abs() < 3.0 {
                position.z = z;
            }
        }
        // Not a twin of somebody standing near: two of the same figure in the same clothes
        // side by side at a stop was the first thing one noticed. A few tries for a figure
        // nobody near wears (then at least other clothes); with few figures installed some
        // repeat anyway.
        let near: Vec<(usize, usize)> = self
            .people
            .iter()
            .filter(|q| (q.position - position).truncate().length() < 30.0)
            .map(|q| (Arc::as_ptr(&q.ty) as usize, q.variant))
            .collect();
        let mut choice: Option<(usize, usize)> = None;
        for attempt in 0..10 {
            let pick = (self.rand() % self.types.len() as u64) as usize;
            let idx = kind.map(|k| k % self.types.len()).unwrap_or(pick);
            let t = &self.types[idx];
            let tk = Arc::as_ptr(t) as usize;
            // the default clothes or one of the `.cti` variants, alike likely
            let n_var = t.variants.len() as u64 + 1;
            let v0 = (self.rand() % n_var) as usize;
            // a clothing variant nobody near wears in this figure
            let var = (0..n_var as usize).map(|k| (v0 + k) % n_var as usize).find(|v| !near.contains(&(tk, *v)));
            let figure_free = !near.iter().any(|n| n.0 == tk);
            match var {
                Some(v) if figure_free || attempt >= 6 || kind.is_some() => {
                    choice = Some((idx, v));
                    break;
                }
                Some(v) if choice.is_none() => choice = Some((idx, v)),
                None if choice.is_none() && attempt == 9 => choice = Some((idx, v0)),
                _ => {}
            }
        }
        let (idx, variant) = choice.unwrap_or((0, 0));
        let ty = self.types[idx].clone();
        let tkey = Arc::as_ptr(&ty) as usize;
        let mut meshes = Vec::new();
        for (mi, hm) in ty.meshes.iter().enumerate() {
            let key = (tkey, variant, mi);
            // somebody of this type has gone: their mesh and instance
            if let Some((id, inst)) = self.spare.get_mut(&key).and_then(|v| v.pop()) {
                self.hidden.retain(|h| *h != inst);
                renderer.set_transform(scene, inst, position, Mat4::IDENTITY);
                renderer.set_params(scene, inst, &[], true, &[]);
                meshes.push((id, inst));
                continue;
            }
            if !self.gpu_materials.contains_key(&key) {
                let dirs = ty.texture_dirs(&world.root);
                let mut mats = Vec::new();
                for (k, m) in hm.materials.iter().enumerate() {
                    // the variant's texture from its own folder first, else the default
                    let (name, first) = ty.variant_texture(&m.texture, variant);
                    let mut look: Vec<&Path> = first.into_iter().collect();
                    look.extend(dirs.iter().map(|p| p.as_path()));
                    let found = omsi_texture::find_texture(name, &look)
                        .or_else(|| omsi_texture::find_texture(&m.texture, &look));
                    if found.is_none() && !m.texture.trim().is_empty() {
                        log::warn!(
                            "human {}: texture {} not found",
                            ty.def.path.display(),
                            m.texture
                        );
                    }
                    let tex = match found {
                        Some(path) => match self.gpu_textures.get(&path) {
                            Some(t) => *t,
                            None => {
                                let t = world
                                    .textures
                                    .get_gpu_fast(&path)
                                    .map(|(img, _)| renderer.add_texture_data(scene, &img));
                                world.textures.release(&path);
                                self.gpu_textures.insert(path, t);
                                t
                            }
                        },
                        None => None,
                    };
                    let alpha = match hm.alpha.get(k).copied().unwrap_or(0) {
                        1 => AlphaMode::Test,
                        2 => AlphaMode::Blend,
                        _ => AlphaMode::Opaque,
                    };
                    mats.push(renderer.add_material(scene, tex, alpha, [1.0; 4], false));
                }
                self.gpu_materials.insert(key, mats);
            }
            let mats = self.gpu_materials[&key].clone();
            let id = renderer.add_mesh(scene, &hm.data);
            let inst = renderer.add_instance(scene, id, position, Mat4::IDENTITY, mats);
            meshes.push((id, inst));
        }
        // walking pace from the human's `[walk_param]` (1.4 m/s by default), a little varied
        let pace = (ty.def.walk_param[0] as f64).clamp(0.9, 1.8) * (0.85 + self.rand_f() * 0.25);
        let age = ty.def.age.map(|a| a as f32).unwrap_or(40.0);
        let target = self.rand_f() as f32;
        let id = self.next_id;
        self.next_id += 1;
        if debug_pax() {
            log::info!(
                "pax #{id} ({}) appears at ({:.1}, {:.1}, {:.1}): {}{}",
                ty.def
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                position.x,
                position.y,
                position.z,
                state.name(),
                if self.seen(position) { " IN SIGHT" } else { "" }
            );
        }
        self.people.push(Person {
            id,
            ty,
            variant,
            meshes,
            position,
            heading,
            lheading: 0.0,
            place: Place::Ground,
            vel: DVec2::ZERO,
            pace,
            activity: Activity::Stand,
            anim: Pose::new(id),
            state,
            t_state: 0.0,
            skins: Vec::new(),
            interior: 0.0,
            lit: 0.0,
            tilt: Mat4::IDENTITY,
            from: -1,
            exit_stop: -1,
            stops_left: 1,
            exit_id: None,
            leaving_here: false,
            avoid: None,
            target,
            ticket: None,
            ticket_decided: false,
            stamps: false,
            pause_until: 0.0,
            age,
            stuck: 0.0,
            ghost: 0.0,
            car_wait: 0.0,
            detour: 0.0,
            detour_side: 0.0,
            blocked: 0.0,
            why: "",
            why_logged: "",
            skinned: false,
            since_posed: 0,
            posed_at: (position, heading),
            ankles: [Vec3::ZERO; 2],
            puppet: None,
            remote: false,
        });
        Some(self.people.len() - 1)
    }

    /// Someone has gone: hidden, and their meshes kept for the next person of the type.
    fn retire(&mut self, p: &Person) {
        let tkey = Arc::as_ptr(&p.ty) as usize;
        for (mi, m) in p.meshes.iter().enumerate() {
            self.hidden.push(m.1);
            self.spare.entry((tkey, p.variant, mi)).or_default().push(*m);
        }
    }

    /// What a passenger does at the door of a bus: one draw from
    /// 0..1 - below the pack's `stamper_prop` they stamp at a validator (when the bus has
    /// one), else, with that part taken off, below `ticketbuy_prop` they buy a ticket (when
    /// the bus sells them); otherwise they show a pass. The pack's buying share was raised
    /// to at least 0.3 before, and nobody ever stamped.
    fn decide_ticket(&mut self, i: usize, has_stampers: bool, sells: bool) {
        self.people[i].ticket_decided = true;
        self.people[i].ticket = None;
        self.people[i].stamps = false;
        let Some((stamp, buy)) = self.tickets.as_ref().map(|t| (t.stamper_prop, t.ticketbuy_prop)) else { return };
        let mut r = self.rand_f() as f32;
        // OMSI_PAX_PAY=1: everybody buys a ticket (a cash desk test)
        let force = omsi_cfg::env::var_os("OMSI_PAX_PAY").is_some();
        if has_stampers && !force {
            if r < stamp {
                self.people[i].stamps = true;
                return;
            }
            r -= stamp;
        }
        if sells && (force || r < buy) {
            let age = self.people[i].age;
            self.people[i].ticket = self.pick_ticket(age);
        }
    }

    /// A ticket of the pack for a passenger of `age`: those whose age
    /// range holds it, weighted by their probability - a day ticket's by the time of day
    /// as well (`day_ticket_factor`). `max_stations` plays no part in the choice.
    fn pick_ticket(&mut self, age: f32) -> Option<usize> {
        let r = self.rand_f() as f32;
        let day = day_ticket_factor(self.time_of_day);
        let t = self.tickets.as_ref()?;
        let weight = |tk: &omsi_content::tickets::Ticket| {
            if (tk.age_min as f32) > age || (tk.age_max as f32) < age {
                0.0
            } else if tk.day_ticket {
                tk.probability.max(0.0) * day
            } else {
                tk.probability.max(0.0)
            }
        };
        let total: f32 = t.tickets.iter().map(weight).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = r * total;
        for (i, tk) in t.tickets.iter().enumerate() {
            let w = weight(tk);
            if w > 0.0 && x < w {
                return Some(i);
            }
            x -= w;
        }
        None
    }

    fn set_state(&mut self, i: usize, s: State) {
        let p = &mut self.people[i];
        if debug_pax() && p.state.name() != s.name() {
            log::info!(
                "t={:.1} pax {}: {} -> {}",
                self.time,
                p.label(),
                p.state.name(),
                s.name()
            );
        }
        p.state = s;
        p.t_state = 0.0;
        p.stuck = 0.0;
        p.why = "";
    }

    fn free_spot(&mut self, stop: i64, spot: usize, id: u32) {
        if let Some(s) = self
            .stops
            .get_mut(&stop)
            .and_then(|s| s.spots.get_mut(spot))
        {
            if s.taken == Some(id) {
                s.taken = None;
            }
        }
    }

    fn free_seat(&mut self, bus: BusId, seat: usize) {
        if let Some(t) = self.seats.get_mut(&bus).and_then(|v| v.get_mut(seat)) {
            *t = false;
        }
    }

    /// Waiting places of a stop: the `[passpos]` of the map's waiting objects nearest to
    /// it (none: nobody waits there, as in OMSI; OMSI_PAX_INVENT=1 makes places along the
    /// back of the platform, off the carriageway and clear of walls).
    fn build_stop(
        &mut self,
        world: &World,
        net: Option<&Network>,
        id: i64,
        pos: DVec3,
        heading: f64,
        name: &str,
    ) -> StopInfo {
        let all_stops: Vec<(i64, DVec3)> =
            world.bus_stops.lock().iter().map(|s| (s.0, s.1)).collect();
        let mut spots: Vec<Spot> = Vec::new();
        for (_, p, face, seat) in world.waiting_places.lock().iter() {
            let d = (*p - pos).truncate().length();
            if d > STOP_REACH || (p.z - pos.z).abs() > 3.0 {
                continue;
            }
            // a waiting place belongs to its nearest stop
            if all_stops
                .iter()
                .any(|(o, q)| *o != id && (*q - *p).truncate().length() < d - 0.5)
            {
                continue;
            }
            spots.push(Spot {
                pos: *p,
                face: *face,
                seat: *seat,
                taken: None,
            });
        }
        let from_map = spots.len();
        // OMSI puts waiting people only where the map's author placed waiting objects (the
        // `people_standing_*` markers, the shelters): a stop without them has none. Places
        // made up along the platform filled the timetable's depot stops ("Am Omnibushof"
        // inside the Spandau depot, no pole, no kerb) with sixteen people each. The made-up
        // places stay for OMSI_PAX_INVENT=1 (maps whose stops have no markers at all).
        if spots.len() < 6 && omsi_cfg::env::var_os("OMSI_PAX_INVENT").is_some() {
            let h = heading.to_radians();
            let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
            let base_z = world.walk_height_near(pos.x, pos.y, pos.z).unwrap_or(pos.z);
            // the carriageway near the stop, once: (segment start, end, half width)
            let mut road: Vec<(DVec2, DVec2, f64)> = Vec::new();
            if let Some(net) = net {
                let (cx, cy) = ((pos.x / 50.0).floor() as i32, (pos.y / 50.0).floor() as i32);
                let mut seen = HashSet::new();
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for &l in net
                            .grid
                            .get(&(cx + dx, cy + dy))
                            .map(|v| v.as_slice())
                            .unwrap_or(&[])
                        {
                            let lane = &net.lanes[l];
                            if lane.kind != LaneKind::Street || !seen.insert(l) {
                                continue;
                            }
                            for w in lane.points.windows(2) {
                                let (a, b) = (w[0].truncate(), w[1].truncate());
                                if crowd::project_on_segment(pos.truncate(), a, b)
                                    .0
                                    .distance(pos.truncate())
                                    < 20.0
                                    && (w[0].z - pos.z).abs() < 4.0
                                {
                                    road.push((a, b, lane.width as f64 * 0.5));
                                }
                            }
                        }
                    }
                }
            }
            let collision = world.collision.lock();
            for row in [1.1, 1.9, 2.7] {
                for k in 0..9 {
                    if spots.len() >= 16 {
                        break;
                    }
                    let along = -1.2 + k as f64 * 0.8 + (row - 1.1) * 0.25;
                    let xy = pos.truncate() + right * row + fwd * along;
                    let Some(z) = world.walk_height_near(xy.x, xy.y, base_z) else {
                        continue;
                    };
                    if (z - base_z).abs() > 0.45 {
                        continue;
                    }
                    let p = DVec3::new(xy.x, xy.y, z);
                    if spots.iter().any(|s| (s.pos - p).truncate().length() < 0.75) {
                        continue;
                    }
                    if road.iter().any(|(a, b, half)| {
                        crowd::project_on_segment(xy, *a, *b).0.distance(xy) < half + 0.5
                    }) {
                        continue;
                    }
                    let probe = omsi_sim::collision::Obb::from_box(
                        [0.5, 0.5, 1.4, 0.0, 0.0, 0.9],
                        p,
                        heading,
                    );
                    if collision.hit(&probe).is_some() {
                        continue;
                    }
                    // facing the road, looking back up it for the bus
                    let face = heading - 90.0 - 35.0
                        + (((k * 7 + (row * 10.0) as usize) % 5) as f64 - 2.0) * 8.0;
                    spots.push(Spot {
                        pos: p,
                        face,
                        seat: 0.0,
                        taken: None,
                    });
                }
            }
        }
        let lane = net
            .and_then(|n| self.ped.as_ref().and_then(|pn| pn.nearest(n, pos, 12.0)))
            .map(|(l, s, _)| (l, s));
        if debug_pax() {
            log::info!("stop {id} '{name}' at ({:.1}, {:.1}) heading {heading:.0}: {} waiting places ({from_map} from the map), pavement {:?}", pos.x, pos.y, spots.len(), lane);
        }
        let next_arrival = 5.0 + (self.rand_f() * 30.0) as f32;
        StopInfo {
            name: name.to_string(),
            pos,
            spots,
            lane,
            seeded: false,
            next_arrival,
        }
    }

    /// Put people at the bus stops near `center`: at the start everywhere, later only at
    /// stops out of sight (the others fill with people walking up).
    pub fn populate(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
    ) {
        if self.avatar_only {
            return;
        }
        // whoever stands under the surface there (its tile's pavements and roads came
        // after them), or anyone standing still off it: onto it
        for p in self.people.iter_mut() {
            if matches!(p.place, Place::Ground) {
                if let Some(z) = world.walk_height_near(p.position.x, p.position.y, p.position.z) {
                    let d = z - p.position.z;
                    let still = p.vel.length() < 0.05;
                    if d.abs() < 3.0 && (d > 0.02 || (still && d.abs() > 0.02)) {
                        p.position.z = z;
                    }
                }
            }
        }
        self.populate_with(world, None, renderer, scene, center);
    }

    fn populate_with(
        &mut self,
        world: &World,
        net: Option<&Network>,
        renderer: &Renderer,
        scene: &mut Scene,
        center: DVec3,
    ) {
        self.center = center;
        let list: Vec<(i64, DVec3, f64, String)> = world
            .bus_stops
            .lock()
            .iter()
            .filter(|s| (s.1 - center).length() < 600.0)
            .map(|s| (s.0, s.1, s.2, s.3.clone()))
            .collect();
        let initial = !self.started;
        self.started = true;
        for (id, pos, rot, name) in list {
            // only once the ground under the stop is there
            if world.walk_height(pos.x, pos.y).is_none() {
                continue;
            }
            if !self.stops.contains_key(&id) {
                let mut info = self.build_stop(world, net, id, pos, rot, &name);
                if let Some(k) = self.rebuild.iter().position(|r| r.0 == id) {
                    let (_, seeded, next) = self.rebuild.swap_remove(k);
                    info.seeded = seeded;
                    info.next_arrival = next;
                }
                self.stops.insert(id, info);
            }
            // (a LAN client's stops fill with the host's people)
            if self.mirror {
                self.stops.get_mut(&id).unwrap().seeded = true;
            }
            let seeded = self.stops[&id].seeded;
            if seeded {
                continue;
            }
            if !initial && self.seen(pos) && (pos - center).length() < 350.0 {
                continue;
            }
            self.stops.get_mut(&id).unwrap().seeded = true;
            let n_spots = self.stops[&id].spots.len();
            let mut want = ((1 + (self.rand() % 5) as usize) as f32 * self.density.clamp(0.0, 3.0))
                .round() as usize;
            // OMSI_PAX_WAITING=n: exactly n people at every stop, all taking the next bus (a crowd test)
            let forced = omsi_cfg::env::var("OMSI_PAX_WAITING")
                .ok()
                .and_then(|v| v.parse::<usize>().ok());
            if let Some(n) = forced {
                want = n;
            }
            for _ in 0..want.min(n_spots.saturating_sub(1)) {
                let free: Vec<usize> = self.stops[&id]
                    .spots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.taken.is_none())
                    .map(|(k, _)| k)
                    .collect();
                if free.is_empty() {
                    break;
                }
                let k = free[(self.rand() as usize) % free.len()];
                let spot = self.stops[&id].spots[k].clone();
                let patience = 240.0 + self.rand_f() as f32 * 600.0;
                if let Some(i) = self.spawn(
                    world,
                    renderer,
                    scene,
                    spot.floor(),
                    spot.face,
                    State::Waiting {
                        stop: id,
                        spot: k,
                        patience,
                    },
                ) {
                    let pid = self.people[i].id;
                    if debug_pax() {
                        let pz = self.people[i].position;
                        log::info!(
                            "pax {pid} waits at stop {id} spot {k}: spot {:?} seat {:.2}, feet {:?}, surface there {:?}",
                            spot.pos, spot.seat, pz, world.walk_height(pz.x, pz.y)
                        );
                    }
                    self.stops.get_mut(&id).unwrap().spots[k].taken = Some(pid);
                    self.people[i].activity = if spot.seat > 0.0 {
                        Activity::Sit
                    } else {
                        Activity::Stand
                    };
                }
            }
        }
    }

    /// Keep strollers on the pavements near the player, and people walking up to the stops.
    fn populate_on_foot(
        &mut self,
        world: &World,
        net: &Network,
        renderer: &Renderer,
        scene: &mut Scene,
        dt: f32,
    ) {
        let Some(ped) = self.ped.take() else { return };
        self.populate_on_foot_with(&ped, world, net, renderer, scene, dt);
        self.ped = Some(ped);
    }

    fn populate_on_foot_with(
        &mut self,
        ped: &PedNet,
        world: &World,
        net: &Network,
        renderer: &Renderer,
        scene: &mut Scene,
        dt: f32,
    ) {
        let center = self.center;
        // strollers: as many as the pavement around carries
        let lanes: Vec<usize> = ped
            .ends
            .keys()
            .copied()
            .filter(|&l| {
                (net.lanes[l].start() - center).truncate().length() < STROLL_RADIUS * 0.9
                    && net.lanes[l].length() > 4.0
            })
            .collect();
        let crowd = (lanes.len() as f32 / 120.0).clamp(0.6, 3.0);
        let target =
            (self.pedestrians as f32 * crowd * self.density.clamp(0.0, 3.0)).round() as usize;
        let have = self
            .people
            .iter()
            .filter(|p| matches!(p.state, State::Strolling(_)))
            // (around this player only, when a LAN host keeps people around several)
            .filter(|p| {
                self.lan_centers.is_empty() || (p.position - center).length() < STROLL_RADIUS
            })
            .count();
        if have < target && !lanes.is_empty() {
            for _ in 0..(target - have).min(4) {
                let lane = lanes[(self.rand() as usize) % lanes.len()];
                let len = net.lanes[lane].length();
                let s = (self.rand_f() as f32 * (len - 1.0)).max(0.5);
                let (p, h) = net.lanes[lane].at(s);
                if self.seen(p)
                    || (p - center).length() < 20.0
                    || (p - center).length() > STROLL_RADIUS * 0.9
                    || !world.has_ground(p.x, p.y)
                {
                    continue;
                }
                let fwd = self.rand_f() < 0.5;
                let leg = if fwd {
                    Leg { lane, a: s, b: len }
                } else {
                    Leg { lane, a: s, b: 0.0 }
                };
                let side = 0.3 + self.rand_f() as f32 * 0.4;
                let heading = if fwd { h as f64 } else { h as f64 + 180.0 };
                if let Some(i) = self.spawn(
                    world,
                    renderer,
                    scene,
                    p,
                    heading,
                    State::Strolling(PedWalk::new(vec![leg], true, side)),
                ) {
                    self.people[i].activity = Activity::Walk;
                }
            }
        }
        // OMSI_PAX_CROSS=x,y: a few pedestrians sent across the signalised crossing nearest that point
        if let Some((x, y)) = omsi_cfg::env::var("OMSI_PAX_CROSS").ok().and_then(|v| {
            let mut it = v.split(',').filter_map(|t| t.trim().parse::<f64>().ok());
            Some((it.next()?, it.next()?))
        }) {
            let want = DVec3::new(x, y, center.z);
            let placed = self
                .people
                .iter()
                .filter(|p| matches!(p.state, State::Strolling(ref w) if !w.roam || w.side < 0.0))
                .count();
            let lane = ped
                .ends
                .keys()
                .copied()
                .filter(|&l| net.lanes[l].traffic_light.is_some())
                .min_by(|a, b| {
                    (net.lanes[*a].start() - want)
                        .truncate()
                        .length()
                        .total_cmp(&(net.lanes[*b].start() - want).truncate().length())
                });
            if let (Some(cross), 0) = (lane, placed) {
                let (start_node, _) = ped.ends[&cross];
                let feeders: Vec<(usize, bool)> = ped.out[start_node]
                    .iter()
                    .copied()
                    .filter(|(l, _)| *l != cross)
                    .collect();
                log::info!(
                    "OMSI_PAX_CROSS: crossing path {cross} light {:?}, {} paths lead to it",
                    net.lanes[cross].traffic_light,
                    feeders.len()
                );
                for k in 0..6 {
                    let Some(&(lane, fwd)) = feeders.get(k % feeders.len().max(1)) else {
                        break;
                    };
                    let len = net.lanes[lane].length();
                    let back = (3.0 + k as f32 * 1.6).min(len);
                    let first = if fwd {
                        Leg {
                            lane,
                            a: back,
                            b: 0.0,
                        }
                    } else {
                        Leg {
                            lane,
                            a: len - back,
                            b: len,
                        }
                    };
                    let over = Leg {
                        lane: cross,
                        a: 0.0,
                        b: net.lanes[cross].length(),
                    };
                    let (p, h) = first.at(net, 0.0);
                    let mut walk = PedWalk::new(vec![first, over], true, 0.4);
                    // marked so that the knob spawns them once
                    walk.side = -0.4;
                    if let Some(i) =
                        self.spawn(world, renderer, scene, p, h, State::Strolling(walk))
                    {
                        self.people[i].activity = Activity::Walk;
                    }
                }
            }
        }
        // people walking up to the stops near the player
        let ids: Vec<i64> = self
            .stops
            .iter()
            .filter(|(_, s)| (s.pos - center).length() < 260.0 && s.lane.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let (waiting, free_spots, lane, spos) = {
                let s = &self.stops[&id];
                (
                    s.spots.iter().filter(|x| x.taken.is_some()).count(),
                    s.spots.iter().filter(|x| x.taken.is_none()).count(),
                    s.lane.unwrap(),
                    s.pos,
                )
            };
            let st = self.stops.get_mut(&id).unwrap();
            st.next_arrival -= dt;
            if st.next_arrival > 0.0 {
                continue;
            }
            let dens = self.density.clamp(0.05, 3.0);
            st.next_arrival = (25.0 + 50.0 * (1.0 / dens)) * 0.5;
            let st_rand = self.rand_f() as f32;
            self.stops.get_mut(&id).unwrap().next_arrival *= 0.6 + st_rand;
            if free_spots <= 1 || waiting >= 7 {
                continue;
            }
            // somewhere 40-90 m away along the pavement, out of sight
            let mut start: Option<(usize, f32, DVec3)> = None;
            for _ in 0..6 {
                let mut leg = if self.rand_f() < 0.5 {
                    Leg {
                        lane: lane.0,
                        a: lane.1,
                        b: 0.0,
                    }
                } else {
                    Leg {
                        lane: lane.0,
                        a: lane.1,
                        b: net.lanes[lane.0].length(),
                    }
                };
                let mut left = 40.0 + self.rand_f() as f32 * 50.0;
                let mut ok = true;
                while left > leg.len() {
                    left -= leg.len();
                    let pick = self.rand();
                    match ped
                        .end_node(net, &leg)
                        .and_then(|n| ped.next_leg(net, n, leg.lane, pick))
                    {
                        Some(l) => leg = l,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let d = leg.dist(left);
                let p = net.lanes[leg.lane].at(d).0;
                if !self.seen(p) && (p - spos).length() > 25.0 && world.has_ground(p.x, p.y) {
                    start = Some((leg.lane, d, p));
                    break;
                }
            }
            let Some((sl, sd, p)) = start else { continue };
            let Some(legs) = ped.route(net, (sl, sd), lane) else {
                continue;
            };
            let free: Vec<usize> = self.stops[&id]
                .spots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.taken.is_none())
                .map(|(k, _)| k)
                .collect();
            let k = free[(self.rand() as usize) % free.len()];
            let side = 0.3 + self.rand_f() as f32 * 0.3;
            let h = legs.first().map(|l| l.at(net, 0.0).1).unwrap_or(0.0);
            if let Some(i) = self.spawn(
                world,
                renderer,
                scene,
                p,
                h,
                State::ToStop {
                    stop: id,
                    spot: k,
                    walk: PedWalk::new(legs, false, side),
                },
            ) {
                let pid = self.people[i].id;
                self.stops.get_mut(&id).unwrap().spots[k].taken = Some(pid);
                self.people[i].activity = Activity::Walk;
            }
        }
    }

    /// `PAX_Entry<i>_Open` / `PAX_Exit<i>_Open` as the bus script reports them. A bus whose
    /// script never sets them (they are not in every mod) falls back to its `door_<i>`.
    fn doors_open(v: &VehicleInstance, n_entry: usize, n_exit: usize) -> (Vec<bool>, Vec<bool>) {
        let mut entry: Vec<bool> = (0..n_entry)
            .map(|i| v.var(&format!("PAX_Entry{i}_Open")).unwrap_or(0.0) > 0.5)
            .collect();
        let mut exit: Vec<bool> = (0..n_exit)
            .map(|i| v.var(&format!("PAX_Exit{i}_Open")).unwrap_or(0.0) > 0.5)
            .collect();
        if v.var("PAX_Entry0_Open").is_none() && v.var("PAX_Exit0_Open").is_none() {
            let doors: Vec<bool> = (0..8)
                .map(|i| v.var(&format!("door_{i}")).unwrap_or(0.0) > 0.9)
                .collect();
            if doors.iter().any(|o| *o) {
                for (i, e) in entry.iter_mut().enumerate() {
                    *e = doors[i.min(7)];
                }
                // the exits follow the entries in the door_<i> numbering (door_0/1 the
                // front leaves, door_2.. the others): a bus with three or more doors and
                // no PAX_Exit vars of its own must still report its middle and rear doors
                // separately, not the front leaf's state for every one of them
                for (i, e) in exit.iter_mut().enumerate() {
                    *e = doors[(n_entry + i).min(7)];
                }
            }
        }
        (entry, exit)
    }

    /// Whether `bn` is worth walking up to: it has a door open already, or it only just
    /// pulled in and the driver (or the AI door script) has not had time to open one yet.
    /// A bus merely parked near a stop, or standing there with every door shut for a while
    /// (a break, a defect, a driver who has not noticed), must not draw a crowd: OMSI only
    /// sends people to a bus that is actually serving them.
    fn doors_open_or_arriving(&self, bn: &BusNow) -> bool {
        if bn.entry_open.iter().any(|o| *o) {
            return true;
        }
        let since = match bn.id {
            BusId::Player => (self.served_stop == bn.stop).then_some(self.served_stop_since),
            BusId::Ai(id) => self.ai_visits.get(&id).map(|v| v.1),
        };
        // (a door shut for a moment - by mistake, or to let the heat in - is no reason to
        // leave: the people turned away at once and came back when it opened again)
        let recently = self.last_door_open.get(&bn.id).is_some_and(|t| self.time - t < DOOR_SHUT_PATIENCE) && bn.speed.abs() < 0.5;
        recently || since.map(|t| self.time - t < DOOR_GRACE).unwrap_or(false)
    }

    /// The buses passengers deal with this frame.
    fn gather_buses(
        &mut self,
        world: &World,
        bus: Option<&VehicleInstance>,
        traffic: Option<&Traffic>,
    ) -> Vec<BusNow> {
        let mut out = Vec::new();
        let stops: Vec<(i64, DVec3, f64)> =
            world.bus_stops.lock().iter().map(|s| (s.0, s.1, s.2)).collect();
        // The stop a bus serves: the nearest in reach - but one facing the way the bus goes
        // before one facing the other way. The two stops of a street often lie within
        // reach of each other, and the people of the stop across the road then walked over
        // the carriageway, through the traffic, to a bus that was not theirs.
        let serving = |pos: DVec3, heading: f64, reach: f64| -> Option<i64> {
            stops
                .iter()
                .filter(|s| (s.1 - pos).length() < reach)
                .min_by(|a, c| {
                    let back = |s: &(i64, DVec3, f64)| crowd::angle_diff(heading, s.2).abs() > 100.0;
                    back(a)
                        .cmp(&back(c))
                        .then((a.1 - pos).length().total_cmp(&(c.1 - pos).length()))
                })
                .map(|s| s.0)
        };
        let approaching = |pos: DVec3, heading: f64, speed: f64| -> Option<i64> {
            if speed.abs() < 0.3 {
                return None;
            }
            stops
                .iter()
                .filter(|s| {
                    let d = (s.1 - pos).truncate();
                    let bearing = d.x.atan2(d.y).to_degrees();
                    d.length() < 60.0 && crowd::angle_diff(heading, bearing).abs() < 80.0 && crowd::angle_diff(heading, s.2).abs() <= 100.0
                })
                .min_by(|a, c| (a.1 - pos).length().total_cmp(&(c.1 - pos).length()))
                .map(|s| s.0)
        };
        let bb_of = |v: &VehicleInstance| {
            let bb =
                v.ty.def
                    .bounding_box
                    .unwrap_or([2.5, 11.0, 3.0, 0.0, 0.0, 1.5]);
            (
                DVec2::new(bb[0] as f64 * 0.5, bb[1] as f64 * 0.5),
                DVec2::new(bb[3] as f64, bb[4] as f64),
            )
        };
        if let (Some(b), Some(cabin)) = (bus, self.player_cabin.clone()) {
            // A bus that is already serving a stop keeps serving it until it really pulls
            // away: a frame-time spike must not "leave" and re-enter the stop.
            let speed = b.physics.velocity_kmh() as f64 / 3.6;
            let limit = if self.served_stop.is_some() { 4.0 } else { 0.5 };
            let (entry_open, exit_open) =
                Self::doors_open(b, cabin.entries.len(), cabin.exits.len());
            let all_exit_here = match (b.var("target_index_int"), b.host.hof.as_ref()) {
                (Some(i), Some(hof)) if i.is_finite() && i >= 0.0 => hof.termini.get(i.round() as usize).is_some_and(|t| t.all_exit),
                _ => false,
            };
            let stop = if speed.abs() * 3.6 > limit {
                None
            } else {
                serving(b.position, b.heading, 22.0).or_else(|| {
                    // the driver gone, or the bus not in service ("$allexit$"): standing
                    // with a door open, everybody gets off wherever it stands (they stayed
                    // in for good, the doors open, nobody at the wheel)
                    ((self.driver_away || all_exit_here) && exit_open.iter().chain(entry_open.iter()).any(|o| *o)).then_some(ALL_OUT_STOP)
                })
            };
            let (half, centre) = bb_of(b);
            let trailers = part_frames(b, &cabin);
            let all_exit = match (b.var("target_index_int"), b.host.hof.as_ref()) {
                (Some(i), Some(hof)) if i.is_finite() && i >= 0.0 => hof.termini.get(i.round() as usize).is_some_and(|t| t.all_exit),
                _ => false,
            };
            let terminus = match (b.var("target_index_int"), b.host.hof.as_ref()) {
                (Some(i), Some(hof)) if i.is_finite() && i >= 0.0 => hof
                    .termini
                    .get(i.round() as usize)
                    .filter(|t| !t.all_exit)
                    .map(|t| t.texture_id.trim().to_string()),
                _ => None,
            };
            out.push(BusNow {
                terminus,
                id: BusId::Player,
                walk_open: None,
                cabin,
                pos: b.position,
                rot: b.body_rotation(),
                heading: b.heading,
                speed,
                entry_open,
                exit_open,
                approach: if stop.is_none() && !all_exit { approaching(b.position, b.heading, speed) } else { None },
                stop,
                interior: b.interior_light(),
                air: CabinAir::of(b),
                half,
                centre,
                accel: DVec2::ZERO,
                trailers,
            });
        }
        if let Some(t) = traffic {
            let near = self.center;
            let riding: HashSet<u64> = self
                .people
                .iter()
                .filter_map(|p| match p.state.bus() {
                    Some(BusId::Ai(id)) => Some(id),
                    _ => None,
                })
                .collect();
            let mut visits = HashMap::new();
            for c in t.cars.iter().filter(|c| c.is_bus()) {
                let from_eye = self.eye.map(|e| (c.vehicle.position - e.pos).length()).unwrap_or(f64::MAX);
                if (c.vehicle.position - near).length().min(from_eye) > 400.0 && !riding.contains(&c.id) {
                    continue;
                }
                let Some(cabin) = self.cabin_for(&c.vehicle) else {
                    continue;
                };
                let speed = c.state.speed as f64;
                let stop = if c.at_station() && speed.abs() < 0.3 {
                    serving(c.vehicle.position, c.vehicle.heading, 18.0)
                } else {
                    None
                };
                let since = match stop {
                    Some(s) => {
                        let v = match self.ai_visits.get(&c.id) {
                            Some(&(vs, t0)) if vs == s => (vs, t0),
                            _ => (s, self.time),
                        };
                        visits.insert(c.id, v);
                        self.time - v.1
                    }
                    None => 0.0,
                };
                let open = stop.is_some();
                let (mut entry_open, mut exit_open) = (
                    vec![false; cabin.entries.len()],
                    vec![false; cabin.exits.len()],
                );
                if open {
                    if c.vehicle.var("PAX_Entry0_Open").is_some() {
                        let (e, x) =
                            Self::doors_open(&c.vehicle, cabin.entries.len(), cabin.exits.len());
                        entry_open = e;
                        exit_open = x;
                    } else if since > 2.5 {
                        // the script does not say: the doors are open while the bus boards
                        entry_open
                            .iter_mut()
                            .chain(exit_open.iter_mut())
                            .for_each(|o| *o = true);
                    }
                }
                self.seats
                    .entry(BusId::Ai(c.id))
                    .or_insert_with(|| vec![false; cabin.seats.len()]);
                let (half, centre) = bb_of(&c.vehicle);
                let trailers = part_frames(&c.vehicle, &cabin);
                out.push(BusNow {
                    terminus: c.bus.as_ref().map(|b| b.terminus.trim().to_string()).filter(|t| !t.is_empty()),
                    id: BusId::Ai(c.id),
                    walk_open: None,
                    cabin,
                    pos: c.vehicle.position,
                    rot: c.vehicle.body_rotation(),
                    heading: c.vehicle.heading,
                    speed,
                    entry_open,
                    exit_open,
                    approach: if stop.is_none() { approaching(c.vehicle.position, c.vehicle.heading, speed) } else { None },
                    stop,
                    interior: c.vehicle.interior_light(),
                    air: CabinAir::of(&c.vehicle),
                    half,
                    centre,
                    accel: DVec2::ZERO,
                    trailers,
                });
            }
            self.ai_visits = visits;
            let alive: HashSet<u64> = t.cars.iter().map(|c| c.id).chain(self.remote_now.iter().chain(self.placed_now.iter()).filter_map(|b| match b.id {
                BusId::Ai(id) => Some(id),
                BusId::Player => None,
            })).collect();
            self.seats.retain(|k, _| match k {
                BusId::Ai(id) => alive.contains(id),
                BusId::Player => true,
            });
        }
        for b in self.remote_now.iter().chain(self.placed_now.iter()) {
            self.seats.entry(b.id).or_insert_with(|| vec![false; b.cabin.seats.len()]);
            out.push(b.clone());
        }
        out
    }

    /// The vehicles standing in the world that the player placed and does not drive now:
    /// (their `Player::uid`, the vehicle). Their riders stay aboard; nobody new boards them.
    pub fn set_placed_buses<'a>(&mut self, buses: impl Iterator<Item = (u64, &'a VehicleInstance)>) {
        let mut out = Vec::new();
        for (uid, v) in buses {
            if let Some(b) = self.parked_bus(BusId::Ai(placed_bus_id(uid)), v) {
                out.push(b);
            }
        }
        self.placed_now = out;
    }

    /// Whether a bus empties at `stop` and takes nobody on there (Omsi.exe 0x61f3e3, the
    /// vehicle's +0x7c5): it is not in service - its target names no valid terminus of the
    /// depot file, or an `[addterminus_allexit]` one ("$allexit$") - or it shows this very
    /// stop as its terminus.
    fn empties_at(&self, bn: &BusNow, stop: i64) -> bool {
        match &bn.terminus {
            None => true,
            Some(t) => self.stops.get(&stop).is_some_and(|s| s.name.trim() == t.trim()),
        }
    }

    /// Whether the bus goes where person `i`, waiting at `stop`, wants to go (Omsi.exe
    /// 0x61c33c): each wants one of the stop's targets (their own pick among them) and
    /// boards a bus whose terminus serves it. Where the timetable has no trip going on from
    /// the stop, the target is invalid and they take the first bus there.
    fn goes_their_way(&self, i: usize, stop: i64, bn: &BusNow) -> bool {
        // (a bus not in service, or at its own terminus, is offered to nobody: Omsi.exe
        // returns before listing it at the stop, 0x61f3e3 - people boarded a bus showing
        // "Betriebsfahrt" or nothing at all)
        if self.empties_at(bn, stop) {
            return false;
        }
        let Some(targets) = self
            .stop_targets
            .as_ref()
            .and_then(|m| m.get(&stop))
            .filter(|t| !t.is_empty())
        else {
            return true;
        };
        let pick = ((self.people[i].target * targets.len() as f32) as usize).min(targets.len() - 1);
        bn.terminus.as_ref().is_some_and(|t| targets[pick].contains(t))
    }

    /// A bus people may be in but do not board here (another player's, one the player left).
    fn parked_bus(&mut self, id: BusId, v: &VehicleInstance) -> Option<BusNow> {
        let cabin = self.cabin_for(v)?;
        let bb = v.ty.def.bounding_box.unwrap_or([2.5, 11.0, 3.0, 0.0, 0.0, 1.5]);
        let trailers = part_frames(v, &cabin);
        let walk_open = Self::doors_open(v, cabin.entries.len(), cabin.exits.len());
        Some(BusNow {
            terminus: None,
            id,
            entry_open: vec![false; cabin.entries.len()],
            exit_open: vec![false; cabin.exits.len()],
            walk_open: Some(walk_open),
            cabin,
            pos: v.position,
            rot: v.body_rotation(),
            heading: v.heading,
            speed: v.physics.velocity_kmh() as f64 / 3.6,
            stop: None,
            approach: None,
            interior: v.interior_light(),
            air: CabinAir::of(v),
            half: DVec2::new(bb[0] as f64 * 0.5, bb[1] as f64 * 0.5),
            centre: DVec2::new(bb[3] as f64, bb[4] as f64),
            accel: DVec2::ZERO,
            trailers,
        })
    }

    /// The player now drives vehicle `new_uid` and left `old_uid`: whoever rode in the one
    /// left stays in it (it is one of the placed vehicles now), whoever rode in the one
    /// taken over is the player's bus's, and the player's cabin is the new vehicle's own.
    /// (Riders followed the player into the next bus, and people boarding it took the old
    /// bus's seats - places in the air round a minibus's bonnet.)
    pub fn player_bus_swapped(&mut self, old_uid: u64, new_uid: u64, new_vehicle: &mut VehicleInstance) {
        let old = BusId::Ai(placed_bus_id(old_uid));
        let new = BusId::Ai(placed_bus_id(new_uid));
        // (through a free id: Player -> old, new -> Player)
        let tmp = BusId::Ai(u64::MAX);
        self.remap_bus(BusId::Player, tmp);
        self.remap_bus(new, BusId::Player);
        self.remap_bus(tmp, old);
        let kept = self.seats.remove(&BusId::Player);
        self.player_cabin = None;
        self.served_stop = None;
        self.pressed_at_stop = None;
        self.set_cabin(new_vehicle);
        if let (Some(k), Some(now)) = (kept, self.seats.get_mut(&BusId::Player)) {
            if k.len() == now.len() {
                *now = k;
            }
        }
    }

    /// Bus `bus` is gone (the player removed it): whoever was in it stands where they were,
    /// on the ground, and walks off.
    pub fn evict(&mut self, bus: BusId, world: &World) {
        for p in &mut self.people {
            let inside = matches!(p.place, Place::Bus(b, _) if b == bus)
                || matches!(&p.state, State::Queue { bus: b, .. } | State::Aboard { bus: b, .. } | State::AtDesk { bus: b, .. } | State::Riding { bus: b, .. } | State::AtExit { bus: b, .. } if *b == bus);
            if !inside {
                continue;
            }
            let at = p.position;
            let z = world.walk_height(at.x, at.y).unwrap_or(at.z);
            p.place = Place::Ground;
            p.position = DVec3::new(at.x, at.y, z);
            p.tilt = Mat4::IDENTITY;
            p.interior = 0.0;
            p.vel = DVec2::ZERO;
            p.state = State::Leaving { target: p.position + DVec3::new(3.0, 3.0, 0.0), walk: None, walked: 0.0 };
            p.t_state = 0.0;
        }
        self.seats.remove(&bus);
        self.bus_motion.remove(&bus);
        if bus == BusId::Player {
            self.player_cabin = None;
            self.served_stop = None;
            self.pressed_at_stop = None;
        }
    }

    /// Everyone and everything that belongs to bus `from` belongs to `to` now.
    fn remap_bus(&mut self, from: BusId, to: BusId) {
        let fix = |b: &mut BusId| {
            if *b == from {
                *b = to;
            }
        };
        for p in &mut self.people {
            if let Place::Bus(b, _) = &mut p.place {
                fix(b);
            }
            match &mut p.state {
                State::Queue { bus, .. } | State::Aboard { bus, .. } | State::AtDesk { bus, .. } | State::Riding { bus, .. } | State::AtExit { bus, .. } => fix(bus),
                _ => {}
            }
        }
        if let Some(v) = self.seats.remove(&from) {
            self.seats.insert(to, v);
        }
        if let Some(v) = self.bus_motion.remove(&from) {
            self.bus_motion.insert(to, v);
        }
    }

    /// LAN play: the other players' buses this frame, by player id (their riders are drawn
    /// in them, see `remote_bus_id`).
    pub fn set_remote_buses<'a>(&mut self, buses: impl Iterator<Item = (u32, &'a VehicleInstance)>) {
        let mut out = Vec::new();
        for (player, v) in buses {
            let Some(cabin) = self.cabin_for(v) else { continue };
            let bb = v.ty.def.bounding_box.unwrap_or([2.5, 11.0, 3.0, 0.0, 0.0, 1.5]);
            let trailers = part_frames(v, &cabin);
            // (their doors as their game has them: a walker gets in only where one is open;
            // the passengers here never board it - that bus's own game boards them)
            let walk_open = Self::doors_open(v, cabin.entries.len(), cabin.exits.len());
            out.push(BusNow {
                terminus: None,
                id: BusId::Ai(remote_bus_id(player)),
                entry_open: vec![false; cabin.entries.len()],
                exit_open: vec![false; cabin.exits.len()],
                walk_open: Some(walk_open),
                cabin,
                pos: v.position,
                rot: v.body_rotation(),
                heading: v.heading,
                speed: v.physics.velocity_kmh() as f64 / 3.6,
                stop: None,
                approach: None,
                interior: v.interior_light(),
                air: CabinAir::of(v),
                half: DVec2::new(bb[0] as f64 * 0.5, bb[1] as f64 * 0.5),
                centre: DVec2::new(bb[3] as f64, bb[4] as f64),
                accel: DVec2::ZERO,
                trailers,
            });
        }
        self.remote_now = out;
    }

    /// Is `bus` among the buses people can be in this frame (an AI bus, or another player's)?
    pub fn knows_bus(&self, bus: u64) -> bool {
        self.remote_now.iter().any(|b| b.id == BusId::Ai(bus)) || self.seats.contains_key(&BusId::Ai(bus))
    }

    /// Is person `id` (one drawn for another game) here?
    pub fn has_mirror(&self, id: u32) -> bool {
        self.people.iter().any(|p| p.id == id && p.remote)
    }

    /// The riders of our own bus, for the other LAN players to see: (id, type, place in the
    /// bus frame, heading there, seat, activity).
    pub fn lan_riders(&self) -> Vec<LanPerson> {
        self.people
            .iter()
            .filter(|p| p.puppet.is_none() && !p.remote)
            .filter_map(|p| match p.place {
                Place::Bus(BusId::Player, l) => Some(LanPerson {
                    id: p.id,
                    ty: p.ty.clone(),
                    pos: p.position,
                    heading: p.heading,
                    speed: 0.0,
                    activity: p.activity,
                    aboard: Some((
                        0,
                        l,
                        p.lheading,
                        match p.state {
                            State::Riding { seat, .. } => Some(seat),
                            _ => None,
                        },
                    )),
                    waiting: None,
                }),
                _ => None,
            })
            .collect()
    }

    /// Nobody on foot walks into a wall: the scenery's collision boxes and meshes (shelters,
    /// fences, walls, buildings with a collision mesh) between knee and head height stop a
    /// step that would enter one, keeping the part of it along the wall. Somebody already
    /// inside one (a waiting place the map put in a shelter's box) is left alone - pushed
    /// out, they jumped. People used to walk through everything but the vehicles.
    fn keep_out_of_walls(&mut self, world: &World, who: &[usize], ground: &mut [(usize, Walker)]) {
        const R: f64 = 0.22;
        const CELL: f64 = 12.0;
        let collision = world.collision.lock();
        let places: Vec<DVec2> = world.waiting_places.lock().iter().map(|w| w.1.truncate()).collect();
        let mut cells: HashMap<(i32, i32), Vec<(Block, f64, f64)>> = HashMap::new();
        for (k, w) in ground.iter_mut() {
            let i = who[*k];
            if w.fixed || self.people[i].place != Place::Ground {
                continue;
            }
            let p0 = self.people[i].position.truncate();
            if (w.pos - p0).length_squared() < 1e-8 {
                continue;
            }
            let z = self.people[i].position.z;
            let key = ((p0.x / CELL).floor() as i32, (p0.y / CELL).floor() as i32);
            let walls = cells.entry(key).or_insert_with(|| {
                let c = DVec2::new((key.0 as f64 + 0.5) * CELL, (key.1 as f64 + 0.5) * CELL);
                let probe = omsi_sim::collision::Obb {
                    center: c,
                    half: DVec2::splat(CELL * 0.5 + 2.0),
                    heading: 0.0,
                    z0: z - 1.0,
                    z1: z + 2.5,
                    velocity: DVec2::ZERO,
                    mass: 0.0,
                    pole: None,
                    id: -1,
                };
                collision
                    .obstacles_near(&probe)
                    .into_iter()
                    .filter(|o| {
                        // a shelter given as one solid box has its waiting places inside:
                        // people go in there
                        let b = Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::ZERO };
                        !places.iter().any(|q| (*q - o.center).length() < o.half.length() + 1.0 && b.near(*q, 0.3))
                    })
                    .map(|o| {
                        (
                            Block {
                                center: o.center,
                                half: o.half + DVec2::splat(R),
                                heading: o.heading,
                                vel: DVec2::ZERO,
                            },
                            o.z0,
                            o.z1,
                        )
                    })
                    .collect()
            });
            for (b, z0, z1) in walls.iter() {
                // between the knees and the head of somebody standing here
                if *z0 > z + 1.6 || *z1 < z + 0.5 {
                    continue;
                }
                if !b.near(w.pos, 0.0) || b.near(p0, -0.01) {
                    continue;
                }
                let (q, inside) = b.closest(w.pos);
                if !inside {
                    continue;
                }
                // onto the wall's face, keeping the step along it
                if omsi_cfg::env::var_os("OMSI_DEBUG_WALLS").is_some() {
                    log::info!("t={:.1} pax {} ({}) kept out of a wall ({:.1} x {:.1} m, heights {:.1}..{:.1}) at ({:.2}, {:.2}), its centre ({:.2}, {:.2}), want ({:.2}, {:.2}) vel ({:.2}, {:.2})", self.time, self.people[i].label(), self.people[i].state.name(), b.half.x * 2.0, b.half.y * 2.0, z0 - z, z1 - z, w.pos.x, w.pos.y, b.center.x, b.center.y, w.want.x, w.want.y, w.vel.x, w.vel.y);
                }
                let n = (q - w.pos).try_normalize().unwrap_or(DVec2::ZERO);
                w.pos = q + n * 0.005;
                let vn = w.vel.dot(n);
                if vn < 0.0 {
                    w.vel -= n * vn;
                }
                let fresh = self.people[i].detour <= 0.0;
                self.people[i].detour = 2.0;
                w.corridor = None;
                // walking straight at it: round it, the way that turns least from where they
                // want to go (a lamp post or a pillar stopped people dead)
                let speed = w.want.length();
                let t = DVec2::new(-n.y, n.x);
                if fresh || self.people[i].detour_side == 0.0 {
                    let along = w.want.dot(t);
                    self.people[i].detour_side = if along.abs() > 0.05 * speed {
                        along.signum()
                    } else if i % 2 == 0 {
                        1.0
                    } else {
                        -1.0
                    };
                }
                if speed > 0.2 && w.vel.dot(t) * self.people[i].detour_side < 0.4 * speed {
                    w.vel = t * self.people[i].detour_side * speed * 0.8;
                }
            }
        }
    }

    /// `OMSI_CHECK_OVERLAP=1`: measure how often somebody on the ground stands inside a
    /// vehicle - an AI car (moving or not), the player's bus or a parked car - deeper
    /// than a few centimetres. Logs every new (person, vehicle) contact and a summary
    /// every ten seconds: person-frames inside, contacts, frames and people checked.
    fn check_overlaps(&mut self, world: &World, traffic: Option<&Traffic>, player: Option<&BusNow>) {
        struct Tally {
            frames: u64,
            person_frames: u64,
            inside: u64,
            contacts: u64,
            open: HashSet<(u32, i64)>,
            last_report: f64,
            /// People within 0.3 m of a vehicle's outline (inside or out), per kind.
            close: HashMap<&'static str, u64>,
        }
        static TALLY: std::sync::Mutex<Option<Tally>> = std::sync::Mutex::new(None);
        let mut guard = TALLY.lock().unwrap();
        let tally = guard.get_or_insert_with(|| Tally {
            frames: 0,
            person_frames: 0,
            inside: 0,
            contacts: 0,
            open: HashSet::new(),
            last_report: 0.0,
            close: HashMap::new(),
        });
        // (key, box, speed, what)
        let mut bodies: Vec<(i64, Block, f64, &'static str)> = Vec::new();
        if let Some(t) = traffic {
            for c in &t.cars {
                if (c.vehicle.position - self.center).length() > 320.0 {
                    continue;
                }
                let bb = c.vehicle.ty.def.bounding_box.unwrap_or([2.0, 4.5, 1.6, 0.0, 0.0, 0.8]);
                let o = omsi_sim::collision::Obb::from_box(bb, c.vehicle.position, c.vehicle.heading);
                let what = if c.is_bus() { "timetable bus" } else { "AI car" };
                bodies.push((
                    c.id as i64,
                    Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::ZERO },
                    c.state.speed as f64,
                    what,
                ));
            }
        }
        if let Some(pb) = player {
            for b in pb.blocks() {
                bodies.push((-1, b, pb.speed, "the player's bus"));
            }
        }
        for o in world.parked_boxes.lock().iter() {
            if (o.center - self.center.truncate()).length() > 320.0 {
                continue;
            }
            bodies.push((
                -1000 - o.id as i64,
                Block { center: o.center, half: o.half, heading: o.heading, vel: DVec2::ZERO },
                0.0,
                "parked car",
            ));
        }
        tally.frames += 1;
        let mut now: HashSet<(u32, i64)> = HashSet::new();
        // `=2` counts the gallery's people too (a row of them across a road tests the cars)
        let puppets = omsi_cfg::env::var("OMSI_CHECK_OVERLAP").map(|v| v == "2").unwrap_or(false);
        for p in &self.people {
            if p.place != Place::Ground || (p.puppet.is_some() && !puppets) {
                continue;
            }
            tally.person_frames += 1;
            let at = p.position.truncate();
            for (key, b, speed, what) in &bodies {
                if b.near(at, 0.3) {
                    let kind = if *speed > 0.5 { "moving" } else { "standing" };
                    *tally.close.entry(if *what == "parked car" { what } else { kind }).or_default() += 1;
                }
                // a few centimetres of slack: a shoulder brushing a bumper is no overlap
                let shrunk = Block { half: b.half - DVec2::splat(0.1), ..*b };
                if shrunk.half.x <= 0.0 || !shrunk.closest(at).1 {
                    continue;
                }
                tally.inside += 1;
                now.insert((p.id, *key));
                if !tally.open.contains(&(p.id, *key)) {
                    tally.contacts += 1;
                    log::info!(
                        "overlap t={:.1}: {} ({}{}{}) inside {what} {key} at {speed:.1} m/s, at ({:.1}, {:.1})",
                        self.time,
                        p.label(),
                        p.state.name(),
                        if p.why.is_empty() { "" } else { ", " },
                        p.why,
                        at.x,
                        at.y
                    );
                }
            }
        }
        tally.open = now;
        if self.time - tally.last_report >= 10.0 {
            tally.last_report = self.time;
            log::info!(
                "overlap summary t={:.0}: {} contacts, {} person-frames inside a vehicle of {} person-frames on the ground, {} frames, {} vehicles near",
                self.time,
                tally.contacts,
                tally.inside,
                tally.person_frames,
                tally.frames,
                bodies.len()
            );
            log::info!("overlap summary: person-frames within 0.3 m of a vehicle: {:?}", tally.close);
        }
    }

    /// Keep only the people the map's `humans.txt` names (once). OMSI draws a map's
    /// pedestrians and passengers from that list alone: Berlin-Spandau and Grundorf name 15
    /// of the stock types - not the uniformed DBC staff, not the aXYZ man01 - and certainly
    /// not an add-on's people installed for another map (the GSPNS ones of Novi Sad, whose
    /// man02 had no texture on Spandau and whose man04 walked with crossed legs). An entry
    /// may be listed more than once to make it more common. A map without the file, or
    /// whose list names nobody installed, keeps everybody.
    fn use_map_humans(&mut self, world: &World) {
        if self.map_humans_done {
            return;
        }
        self.map_humans_done = true;
        let list = omsi_map::ailists::load_list(&world.map_dir.join("humans.txt"));
        if list.is_empty() {
            return;
        }
        // the path below `Humans/`, lower case with forward slashes
        let key = |p: &str| -> String {
            let p = p.replace('\\', "/").to_ascii_lowercase();
            match p.rfind("humans/") {
                Some(k) => p[k + 7..].to_string(),
                None => p,
            }
        };
        let mut picked: Vec<Arc<HumanType>> = Vec::new();
        let mut missing: Vec<&str> = Vec::new();
        for line in &list {
            let want = key(line.trim());
            match self
                .types
                .iter()
                .find(|t| key(&t.def.path.to_string_lossy()) == want)
            {
                Some(t) => picked.push(t.clone()),
                None => missing.push(line),
            }
        }
        if !missing.is_empty() {
            log::warn!("humans.txt of the map names people not installed: {missing:?}");
        }
        if picked.is_empty() {
            return;
        }
        log::info!(
            "humans: {} of {} types from the map's humans.txt",
            picked.len(),
            self.types.len()
        );
        self.types = picked;
    }

    /// People the moving bus has just knocked down. OMSI counts them in the driver's
    /// personnel file; they are only counted once and then walk away.
    pub fn run_over(&mut self, bus: &VehicleInstance) -> u32 {
        if bus.physics.velocity_kmh().abs() < 5.0 {
            return 0;
        }
        let Some(bb) = bus.ty.def.bounding_box else {
            return 0;
        };
        let (half_x, half_y) = ((bb[0] - bb[3]).abs() / 2.0, (bb[1] - bb[4]).abs() / 2.0);
        let inv = bus.body_rotation().transpose();
        let mut knocked = Vec::new();
        for (i, p) in self.people.iter().enumerate() {
            if p.place != Place::Ground || matches!(p.state, State::Leaving { .. }) {
                continue;
            }
            let local = inv.transform_vector3((p.position - bus.position).as_vec3());
            if local.x.abs() < half_x + 0.2 && local.y.abs() < half_y + 0.2 && local.z.abs() < 3.0 {
                knocked.push(i);
            }
        }
        for &i in &knocked {
            let away = self.people[i].position
                + (self.people[i].position - bus.position).normalize_or_zero() * 6.0;
            self.release(i);
            self.set_state(
                i,
                State::Leaving {
                    target: away,
                    walk: None,
                    walked: 0.0,
                },
            );
        }
        knocked.len() as u32
    }

    /// Give back what a person holds (a waiting place, a seat) before they change plans.
    fn release(&mut self, i: usize) {
        let id = self.people[i].id;
        match self.people[i].state.clone() {
            State::ToStop { stop, spot, .. }
            | State::ToSpot { stop, spot }
            | State::Waiting { stop, spot, .. }
            | State::Queue { stop, spot, .. } => self.free_spot(stop, spot, id),
            State::Riding { bus, seat } | State::AtDesk { bus, seat, .. } => {
                self.free_seat(bus, seat)
            }
            State::Aboard {
                bus,
                goal: Goal::Seat(seat) | Goal::Desk(seat) | Goal::Stamper(seat, _),
                ..
            } => self.free_seat(bus, seat),
            _ => {}
        }
    }

    /// OMSI_CHECK_WALLS: everybody inside a bus who stands away from its walkways (more
    /// than 0.45 m from every path link, not on a seat): through a seat back or a wall.
    fn check_walls(&self) {
        for p in &self.people {
            let Place::Bus(bus, local) = p.place else { continue };
            if matches!(p.state, State::Riding { .. } | State::AtDesk { .. }) {
                continue;
            }
            let Some(bn) = self.last_buses.iter().find(|b| b.id == bus) else { continue };
            let pts = &bn.cabin.graph.points;
            let mut best = f32::INFINITY;
            for &(a, b, _) in &bn.cabin.links {
                let (Some(pa), Some(pb)) = (pts.get(a.max(0) as usize), pts.get(b.max(0) as usize)) else { continue };
                let ab = *pb - *pa;
                let t = if ab.length_squared() > 1e-6 { ((local - *pa).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let q = *pa + ab * t;
                best = best.min((q.truncate() - local.truncate()).length() + (q.z - local.z).abs());
            }
            let near_seat = bn.cabin.seats.iter().any(|s| (s.floor - local).truncate().length() < 0.35 || (s.pos - local).truncate().length() < 0.35);
            if best > 0.45 && !near_seat && !bn.cabin.links.is_empty() {
                log::warn!("t={:.1} person {} in bus {:?} off the walkways by {best:.2} m at ({:.2}, {:.2}, {:.2}), state {}", self.time, p.id, bus, local.x, local.y, local.z, p.state.name());
            }
        }
    }

    /// Report the waiting and alighting passengers to the bus script, the way OMSI does.
    pub fn write_pax_vars(&self, b: &mut VehicleInstance) {
        for (i, r) in self.entry_req.iter().enumerate() {
            b.set_var(&format!("PAX_Entry{i}_Req"), if *r { 1.0 } else { 0.0 });
        }
        for (i, r) in self.exit_req.iter().enumerate() {
            b.set_var(&format!("PAX_Exit{i}_Req"), if *r { 1.0 } else { 0.0 });
        }
    }

    /// Timetable buses to hold at their stop, for the traffic.
    pub fn take_holds(&mut self) -> Vec<(u64, f32)> {
        std::mem::take(&mut self.holds)
    }

    /// Door requests for the timetable buses, for the traffic to hand to their scripts.
    pub fn take_ai_requests(&mut self) -> Vec<(u64, Vec<bool>, Vec<bool>)> {
        std::mem::take(&mut self.ai_requests)
    }

    /// A line for the HUD about something that just happened.
    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    /// What the driver should do now, for the HUD: a passenger waiting at the cash desk
    /// for the ticket (only when the driver has to sell it).
    pub fn hint(&self) -> Option<String> {
        if !self.boarding.eq_ignore_ascii_case("pay") {
            return None;
        }
        let p = self.people.iter().find(|p| {
            matches!(
                p.state,
                State::AtDesk {
                    bus: BusId::Player,
                    ticket: Some(_),
                    done: false,
                    ..
                }
            )
        })?;
        let (name, value) = self.request.clone()?;
        let left = (PAY_PATIENCE - p.t_state).max(0.0);
        Some(format!(
            "Passenger waiting for a ticket: {name} {value:.2} - press {} ({left:.0} s)",
            self.ticket_key
        ))
    }

    /// People sitting on each `[passpos]` of the player's bus, for `GetHumanCountOnSeat`
    /// (the BVG Citaro folds its tip-up seats down when somebody sits on them).
    pub fn seat_counts(&self) -> Vec<u32> {
        let Some(cabin) = self.player_cabin.as_ref() else {
            return Vec::new();
        };
        let mut out = vec![0u32; cabin.seats.len()];
        for p in &self.people {
            if let State::Riding {
                bus: BusId::Player,
                seat,
            } = &p.state
            {
                if let Some(c) = out.get_mut(*seat) {
                    *c += 1;
                }
            }
        }
        out
    }

    /// How many people stand on each `paths.cfg` link inside the player's bus, for the
    /// scripts' `GetHumanCountOnPathLink` (the NL/NG uses it for the fare gate).
    pub fn path_link_counts(&self) -> Vec<u32> {
        let Some(cabin) = self.player_cabin.as_ref() else {
            return Vec::new();
        };
        let mut out = vec![0u32; cabin.links.len()];
        for p in &self.people {
            let local = match (p.place, &p.state) {
                (
                    Place::Bus(BusId::Player, l),
                    State::Aboard { .. } | State::AtDesk { .. } | State::AtExit { .. },
                ) => l,
                _ => continue,
            };
            let mut best: Option<(usize, f32)> = None;
            for (li, (a, b, _)) in cabin.links.iter().enumerate() {
                let (Some(pa), Some(pb)) = (
                    cabin.graph.points.get(*a as usize),
                    cabin.graph.points.get(*b as usize),
                ) else {
                    continue;
                };
                let ab = *pb - *pa;
                let t = ((local - *pa).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let d = (*pa + ab * t - local).length();
                if best.map(|b| d < b.1).unwrap_or(true) {
                    best = Some((li, d));
                }
            }
            if let Some((li, d)) = best {
                if d < 1.0 {
                    out[li] += 1;
                }
            }
        }
        out
    }

    /// Where everybody is, for logs.
    pub fn positions(&self) -> Vec<(String, DVec3)> {
        self.people
            .iter()
            .map(|p| (p.state.name().to_string(), p.position))
            .collect()
    }

    /// Count of people per state, for logs.
    pub fn summary(&self) -> String {
        let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
        for p in &self.people {
            *counts.entry(p.state.name()).or_default() += 1;
        }
        let mut out = counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect::<Vec<_>>()
            .join(", ");
        let (n, total, worst) = self.tick_stats;
        if n > 0 {
            out.push_str(&format!(
                "; {:.2} ms a frame, longest {worst:.1} ms",
                total / n as f64
            ));
        }
        let (frames, posed, ms, up) = self.pose_stats;
        if frames > 0 {
            out.push_str(&format!(
                "; posing {:.2} ms a frame ({:.1} people, {:.2} ms of it uploading and placing)",
                ms / frames as f64,
                posed as f64 / frames as f64,
                up / frames as f64
            ));
        }
        out
    }

    /// Advance everybody. `bus`: the player's vehicle; `traffic`: the timetable buses,
    /// the traffic lights and the cars pedestrians wait for. Returns true when a passenger
    /// took the printed ticket (the caller resets `GivenTicket`).
    pub fn tick(
        &mut self,
        dt: f32,
        world: &World,
        bus: Option<&VehicleInstance>,
        traffic: Option<&Traffic>,
        renderer: &Renderer,
        scene: &mut Scene,
    ) -> bool {
        let started = std::time::Instant::now();
        let took = self.tick_inner(dt, world, bus, traffic, renderer, scene);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        if debug_pax() && ms > 30.0 {
            log::info!(
                "t={:.1} slow people tick: {ms:.1} ms ({} people)",
                self.time,
                self.people.len()
            );
        }
        if omsi_cfg::env::var_os("OMSI_CHECK_WALLS").is_some() {
            self.check_walls();
        }
        self.tick_stats.0 += 1;
        self.tick_stats.1 += ms;
        self.tick_stats.2 = self.tick_stats.2.max(ms);
        took
    }

    fn tick_inner(
        &mut self,
        dt: f32,
        world: &World,
        bus: Option<&VehicleInstance>,
        traffic: Option<&Traffic>,
        renderer: &Renderer,
        scene: &mut Scene,
    ) -> bool {
        self.use_map_humans(world);
        self.time += dt as f64;
        let net = traffic.map(|t| &t.net);
        if let Some(b) = bus {
            self.center = b.position;
        } else if let Some(e) = self.eye {
            self.center = e.pos;
        }
        let generation = world
            .tiles_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        if generation != self.tiles_seen {
            self.tiles_seen = generation;
            self.tiles_changed(world);
        }
        // tiles brought lanes: their pavements join the network, and stops without one look again
        if let (Some(pn), Some(n)) = (self.ped.as_mut(), net) {
            if pn.built < n.lanes.len() {
                let added = pn.extend(n);
                if added > 0 {
                    let ids: Vec<(i64, DVec3)> = self
                        .stops
                        .iter()
                        .filter(|(_, s)| s.lane.is_none())
                        .map(|(k, s)| (*k, s.pos))
                        .collect();
                    for (id, pos) in ids {
                        let lane = self
                            .ped
                            .as_ref()
                            .and_then(|pn| pn.nearest(n, pos, 12.0))
                            .map(|(l, s, _)| (l, s));
                        self.stops.get_mut(&id).unwrap().lane = lane;
                    }
                    if debug_pax() {
                        log::info!(
                            "t={:.1} pavement network: {added} paths added ({} in all)",
                            self.time,
                            self.ped.as_ref().map(|p| p.ends.len()).unwrap_or(0)
                        );
                    }
                }
            }
        }
        if self.ped.is_none() {
            if let Some(n) = net {
                self.ped = Some(PedNet::build(n));
                if debug_pax() {
                    // the signalised pavement crossings nearest the player, for checking them
                    let c = self.center;
                    let mut lit: Vec<(f64, usize)> = self
                        .ped
                        .as_ref()
                        .unwrap()
                        .ends
                        .keys()
                        .filter(|&&l| n.lanes[l].traffic_light.is_some())
                        .map(|&l| {
                            (
                                ((n.lanes[l].start() + n.lanes[l].end()) * 0.5 - c).length(),
                                l,
                            )
                        })
                        .collect();
                    lit.sort_by(|a, b| a.0.total_cmp(&b.0));
                    log::info!("pavement: {} paths with a pedestrian light", lit.len());
                    for (d, l) in lit.iter().take(6) {
                        let (a, b) = (n.lanes[*l].start(), n.lanes[*l].end());
                        log::info!("  light {:?} on path {l}, {d:.0} m away: ({:.1}, {:.1}, {:.1}) -> ({:.1}, {:.1})", n.lanes[*l].traffic_light, a.x, a.y, a.z, b.x, b.y);
                    }
                }
                // stops met before the pavement network was known look for their pavement now
                let ids: Vec<(i64, DVec3)> = self.stops.iter().map(|(k, s)| (*k, s.pos)).collect();
                for (id, pos) in ids {
                    let lane = self
                        .ped
                        .as_ref()
                        .and_then(|pn| pn.nearest(n, pos, 12.0))
                        .map(|(l, s, _)| (l, s));
                    self.stops.get_mut(&id).unwrap().lane = lane;
                }
            }
        }
        let t0 = std::time::Instant::now();
        if let Some(n) = net {
            self.stroll_timer -= dt;
            if self.stroll_timer <= 0.0 {
                self.stroll_timer = 1.0;
                let c = self.center;
                self.populate_with(world, Some(n), renderer, scene, c);
                let t1 = t0.elapsed().as_secs_f64() * 1000.0;
                if !self.mirror {
                    self.populate_on_foot(world, n, renderer, scene, 1.0);
                    self.populate_lan_centers(world, n, renderer, scene);
                }
                let t2 = t0.elapsed().as_secs_f64() * 1000.0;
                if debug_pax() && t2 > 20.0 {
                    log::info!(
                        "t={:.1} slow populate: stops {t1:.1} ms, on foot {:.1} ms ({} people)",
                        self.time,
                        t2 - t1,
                        self.people.len()
                    );
                }
            }
        }
        if !self.gallery {
            self.gallery = true;
            self.make_gallery(world, renderer, scene);
        }
        let mut buses = self.gather_buses(world, bus, traffic);
        for b in &buses {
            if b.entry_open.iter().chain(b.exit_open.iter()).any(|o| *o) {
                self.last_door_open.insert(b.id, self.time);
            }
        }
        // how the floor of each bus accelerates: braking and pulling away, and round bends
        if dt > 1e-4 {
            let mut motion = HashMap::new();
            for bn in buses.iter_mut() {
                let accel = match self.bus_motion.get(&bn.id) {
                    Some(&(v0, h0, a0)) => {
                        let yaw_rate = crowd::angle_diff(h0, bn.heading).to_radians() / dt as f64;
                        let raw = DVec2::new(bn.speed * yaw_rate, (bn.speed - v0) / dt as f64)
                            .clamp(DVec2::splat(-6.0), DVec2::splat(6.0));
                        a0 + (raw - a0) * (1.0 - (-(dt as f64) / 0.2).exp())
                    }
                    None => DVec2::ZERO,
                };
                bn.accel = accel;
                motion.insert(bn.id, (bn.speed, bn.heading, accel));
            }
            self.bus_motion = motion;
        }
        let buses = buses;
        self.last_buses = buses.clone();
        if let Some(t) = traffic.filter(|_| !self.avatar_only) {
            self.seed_ai_riders(&buses, t, world, renderer, scene);
        }
        let bus_ix: HashMap<BusId, usize> =
            buses.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        for v in self.door_busy.values_mut() {
            *v -= dt;
        }
        self.door_busy.retain(|_, v| *v > 0.0);
        // the player's bus: stops, requests
        let player = bus_ix.get(&BusId::Player).map(|&i| &buses[i]);
        let at_stop = player.and_then(|b| b.stop);
        if at_stop != self.served_stop {
            if debug_pax() {
                log::info!("t={:.1} player bus serving stop {:?} (was {:?}); waiting there: {}", self.time, at_stop, self.served_stop, self.people.iter().filter(|p| matches!(p.state, State::Waiting { stop, .. } if Some(stop) == at_stop)).count());
            }
            self.served_stop = at_stop;
            self.served_stop_since = self.time;
            // (the player's bus's riders only: a timetable bus's riders decide once per
            // stop of their own bus, and lost it here whenever the player's bus came to a
            // stop somewhere - they stood at the open door until their bus drove on, #317)
            for i in 0..self.people.len() {
                if self.people[i].state.bus() == Some(BusId::Player) {
                    self.people[i].leaving_here = false;
                }
            }
            if let (Some(stop), Some(b)) = (at_stop, bus) {
                let here = b.host.tt_busstop_index;
                // Omsi.exe (0x61f3e3): a bus not in service ("$allexit$": no valid target) or
                // standing at its own terminus - the terminus it shows is this stop's name -
                // empties; every rider goes (0x62d129). Riders whose stop it is go by the
                // stop itself, not only by the timetable's index: that is reset to 0 when the
                // next trip is taken up, and at the end of a late trip nobody got off (#226
                // report: "passengers do not get out at the last stop").
                let everybody = player.is_some_and(|pb| self.empties_at(pb, stop));
                let here_id = b.host.tt_stop_ids.get(usize::try_from(here).unwrap_or(usize::MAX)).copied();
                for i in 0..self.people.len() {
                    let (exit, from) = (self.people[i].exit_stop, self.people[i].from);
                    if matches!(
                        self.people[i].state,
                        State::Riding {
                            bus: BusId::Player,
                            ..
                        } | State::AtExit {
                            bus: BusId::Player,
                            ..
                        } | State::Aboard {
                            bus: BusId::Player,
                            goal: Goal::ExitWait(_) | Goal::Exit(_),
                            ..
                        }
                    ) {
                        // (those already on their way to the door are asked too: they stood up
                        // as the bus pulled in - see `mine_ahead` - and, left out here, came to
                        // the door as if riding on and never got off, #336)
                        let exit_id = self.people[i].exit_id;
                        let leaves = if stop == ALL_OUT_STOP || self.driver_away || everybody || exit_id == Some(stop) {
                            true
                        } else if exit >= 0 {
                            exit <= here && (exit_id.is_none() || here_id == Some(stop))
                        } else {
                            from != stop && self.rand_f() < 0.35
                        };
                        self.people[i].leaving_here = leaves;
                    }
                }
            }
        }
        if let Some(b) = bus {
            // stop request: a rider whose stop comes next presses the button while driving
            let next = b.host.tt_busstop_index;
            let moving = b.physics.velocity_kmh() > 3.0;
            if debug_pax() && self.debug_last_next != next {
                self.debug_last_next = next;
                log::info!(
                    "t={:.1} timetable next stop is now {next}; riders' stops: {:?}",
                    self.time,
                    self.people
                        .iter()
                        .filter(|p| p.inside(BusId::Player))
                        .map(|p| p.exit_stop)
                        .collect::<Vec<_>>()
                );
            }
            if moving
                && self.requested_for != Some(next)
                && self
                    .people
                    .iter()
                    .any(|p| p.inside(BusId::Player) && p.exit_stop >= 0 && p.exit_stop <= next)
            {
                self.requested_for = Some(next);
                self.stop_request = true;
                if debug_pax() {
                    log::info!("t={:.1} stop request for timetable stop {next}", self.time);
                }
            }
        }
        // timetable buses: a new stop visit decides who gets off
        for bn in buses.iter().filter(|b| matches!(b.id, BusId::Ai(_))) {
            let BusId::Ai(id) = bn.id else { continue };
            let Some(stop) = bn.stop else { continue };
            let fresh = self
                .ai_visits
                .get(&id)
                .map(|v| (self.time - v.1) < dt as f64 * 1.5)
                .unwrap_or(false);
            if fresh {
                let everybody = self.empties_at(bn, stop);
                for i in 0..self.people.len() {
                    match self.people[i].state {
                        State::Riding { bus, .. }
                            if bus == bn.id && self.people[i].from != stop =>
                        {
                            self.people[i].stops_left = self.people[i].stops_left.saturating_sub(1);
                            if everybody || self.people[i].stops_left <= 0 || self.people[i].exit_id == Some(stop) {
                                self.people[i].leaving_here = true;
                            }
                        }
                        State::AtExit { bus, .. } if bus == bn.id => {
                            self.people[i].leaving_here = true
                        }
                        _ => {}
                    }
                }
            }
        }
        // how long each passenger on the way out has been asking for the door
        {
            let asking: Vec<u32> = self
                .people
                .iter()
                .filter(|p| match p.state {
                    State::AtExit { .. } | State::Aboard { goal: Goal::ExitWait(_), .. } => p.leaving_here,
                    State::Aboard { goal: Goal::Exit(_), .. } => true,
                    // (and at the door from outside: one who cannot get in - the bus full,
                    // the way blocked - pressed the request button for ever, and the door
                    // the driver shut opened again)
                    State::Queue { bus: BusId::Player, .. } => true,
                    _ => false,
                })
                .map(|p| p.id)
                .collect();
            self.exit_req_time.retain(|id, _| asking.contains(id));
            for id in asking {
                *self.exit_req_time.entry(id).or_insert(0.0) += dt;
            }
        }
        // who asks for which door of the player's bus
        for r in self.entry_req.iter_mut().chain(self.exit_req.iter_mut()) {
            *r = false;
        }
        // A request opens a shut door. Those getting off ask the whole way to the exit, as
        // Omsi.exe's riders do from the moment they stand up (state 5, 0x62d6f8): asked
        // only from the doorway, the SD202's automatic rear door shut three seconds after
        // the one in front, on the rider still walking up, and opened again when they got
        // there - the door "did not know" whether people were getting off. Those boarding
        // hold an open door from the doorway, as they wait within 0.7 m of it in OMSI.
        let in_doorway = |p: &Person, entry: Option<usize>, exit: Option<usize>| -> bool {
            let Some(pb) = player else { return true };
            let (door, open) = match (entry, exit) {
                (Some(i), _) => (pb.cabin.entries.get(i), pb.entry_open.get(i).copied().unwrap_or(false)),
                (_, Some(i)) => (pb.cabin.exits.get(i), pb.exit_open.get(i).copied().unwrap_or(false)),
                _ => (None, false),
            };
            let Some(door) = door.filter(|_| open) else { return true };
            // (inside, the bus frame: a rider's world position is not kept up)
            let near = |q: Vec3| match p.place {
                Place::Bus(_, local) => (q - local).truncate().length() < DOORWAY as f32,
                Place::Ground => (pb.world(q) - p.position).truncate().length() < DOORWAY,
            };
            near(door.inside) || near(door.outside) || near(door.wait)
        };
        for p in &self.people {
            match p.state {
                State::Queue {
                    bus: BusId::Player,
                    entry,
                    ..
                } if self.exit_req_time.get(&p.id).is_none_or(|t| *t < EXIT_REQ_LAPSE) && in_doorway(p, Some(entry), None) => {
                    if let Some(r) = self.entry_req.get_mut(entry) {
                        *r = true;
                    }
                }
                // (a request lapses when the person has been at it far longer than stepping
                // out takes - one held up somewhere kept the SD200's and the EN92's automatic
                // rear door open for good: `haltewunsch` never went off)
                State::AtExit {
                    bus: BusId::Player,
                    exit,
                }
                | State::Aboard {
                    bus: BusId::Player,
                    goal: Goal::ExitWait(exit),
                    ..
                } if p.leaving_here && self.exit_req_time.get(&p.id).is_none_or(|t| *t < EXIT_REQ_LAPSE) => {
                    if let Some(r) = self.exit_req.get_mut(exit) {
                        *r = true;
                    }
                }
                // and while they step through it: the rear door of the SD200 shuts a few
                // seconds after the last request, on the people still in the doorway (as
                // the timetable buses' requests already do)
                State::Aboard {
                    bus: BusId::Player,
                    goal: Goal::Exit(exit),
                    ..
                } if self.exit_req_time.get(&p.id).is_none_or(|t| *t < EXIT_REQ_LAPSE) => {
                    if let Some(r) = self.exit_req.get_mut(exit) {
                        *r = true;
                    }
                }
                _ => {}
            }
        }
        // the outside door opener: somebody queueing at a shut door presses it, once per stop
        if let (Some(stop), Some(pb)) = (at_stop, player) {
            let shut = self.people.iter().any(|p| matches!(p.state, State::Queue { bus: BusId::Player, entry, .. } if !pb.entry_open.get(entry).copied().unwrap_or(false)));
            if shut && self.pressed_at_stop != Some(stop) && pb.standing() {
                self.pressed_at_stop = Some(stop);
                self.door_request = true;
                if debug_pax() {
                    log::info!(
                        "t={:.1} a passenger presses the outside door opener at stop {stop}",
                        self.time
                    );
                }
            }
        }
        if let (Some(stop), Some(b)) = (self.pressed_at_stop, bus) {
            let gone = world
                .bus_stops
                .lock()
                .iter()
                .find(|s| s.0 == stop)
                .map(|s| (s.1 - b.position).length() > 40.0)
                .unwrap_or(true);
            if gone {
                self.pressed_at_stop = None;
            }
        }
        // cars pedestrians look out for: (position, velocity, half length)
        let mut cars: Vec<(DVec2, DVec2, f64)> = Vec::new();
        let mut blocks: Vec<Block> = Vec::new();
        if let Some(t) = traffic {
            for c in &t.cars {
                if (c.vehicle.position - self.center).length() > 320.0 {
                    continue;
                }
                let h = c.vehicle.heading.to_radians();
                let fwd = DVec2::new(h.sin(), h.cos());
                let bb = c
                    .vehicle
                    .ty
                    .def
                    .bounding_box
                    .unwrap_or([2.0, 4.5, 1.6, 0.0, 0.0, 0.8]);
                cars.push((
                    c.vehicle.position.truncate(),
                    fwd * c.state.speed as f64,
                    bb[1] as f64 * 0.5,
                ));
                if !matches!(c.vehicle.ty.def.kind, omsi_vehicle::VehicleKind::Other(3)) {
                    let o = omsi_sim::collision::Obb::from_box(
                        bb,
                        c.vehicle.position,
                        c.vehicle.heading,
                    );
                    blocks.push(Block {
                        center: o.center,
                        half: o.half,
                        heading: o.heading,
                        vel: fwd * c.state.speed as f64,
                    });
                    // and the rear sections of an articulated bus
                    for t in &c.vehicle.trailers {
                        let tb =
                            t.ty.def
                                .bounding_box
                                .unwrap_or([2.5, 7.0, 3.0, 0.0, 0.0, 1.5]);
                        let o = omsi_sim::collision::Obb::from_box(tb, t.position, t.heading);
                        let th = t.heading.to_radians();
                        blocks.push(Block {
                            center: o.center,
                            half: o.half,
                            heading: o.heading,
                            vel: DVec2::new(th.sin(), th.cos()) * c.state.speed as f64,
                        });
                    }
                }
            }
        }
        // the parked cars: people walked through them as if they were not there
        for o in world.parked_boxes.lock().iter() {
            if (o.center - self.center.truncate()).length() < 320.0 {
                blocks.push(Block {
                    center: o.center,
                    half: o.half,
                    heading: o.heading,
                    vel: DVec2::ZERO,
                });
            }
        }
        if let Some(pb) = player {
            cars.push((pb.pos.truncate(), pb.fwd() * pb.speed, pb.half.y));
            for t in &pb.trailers {
                let h = t.heading.to_radians();
                cars.push((
                    t.pos.truncate(),
                    DVec2::new(h.sin(), h.cos()) * pb.speed,
                    t.half.y,
                ));
            }
            blocks.extend(pb.blocks());
        }
        // queues: the order at each door
        let mut queues: HashMap<(BusId, usize), Vec<(f64, usize)>> = HashMap::new();
        for (i, p) in self.people.iter().enumerate() {
            if let State::Queue {
                bus, entry, joined, ..
            } = p.state
            {
                queues.entry((bus, entry)).or_default().push((joined, i));
            }
        }
        for q in queues.values_mut() {
            q.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        }
        let slot_of: HashMap<usize, usize> = queues
            .values()
            .flat_map(|q| q.iter().enumerate().map(|(k, (_, i))| (*i, k)))
            .collect();
        // who is where, for "do not push into somebody standing in front"
        let snapshot: Vec<(u64, DVec2, f64)> = self
            .people
            .iter()
            .map(|p| match p.place {
                Place::Ground => (0, p.position.truncate(), p.vel.length()),
                Place::Bus(b, l) => (b.space(), l.truncate().as_dvec2(), p.vel.length()),
            })
            .collect();
        let mut taken_ticket = false;
        let mut remove: Vec<usize> = Vec::new();
        let mut wants: Vec<Want> = Vec::with_capacity(self.people.len());
        for i in 0..self.people.len() {
            self.people[i].t_state += dt;
            let mut w = if self.people[i].puppet.is_some() {
                Want::stand(None, Activity::Stand)
            } else if self.people[i].remote {
                self.mirror_want(i, &buses)
            } else {
                self.decide(
                    i,
                    dt,
                    world,
                    bus,
                    net,
                    traffic,
                    &buses,
                    &bus_ix,
                    &slot_of,
                    &cars,
                    renderer,
                    scene,
                    &mut taken_ticket,
                    &mut remove,
                )
            };
            // nobody walks off while still sitting down or getting up
            let a = &self.people[i].anim;
            if a.sit_amount() > 0.02 && (a.settling() || self.people[i].activity != Activity::Sit) {
                w.vel = DVec2::ZERO;
            }
            wants.push(w);
        }
        // do not push into somebody standing (or queueing) just in front - but pass
        // somebody coming the other way, and never wait for ever behind anybody
        let wanted: Vec<DVec2> = wants.iter().map(|w| w.vel).collect();
        for (i, w) in wants.iter_mut().enumerate() {
            if !w.follow
                || w.vel.length() < 0.05
                || self.people[i].ghost > 0.0
                || w.goal_dist.map(|g| g < 0.5).unwrap_or(false)
            {
                self.people[i].blocked = 0.0;
                continue;
            }
            let (space, me, _) = snapshot[i];
            let dir = w.vel.normalize();
            let blocker = snapshot
                .iter()
                .enumerate()
                .position(|(j, (sp, pos, speed))| {
                    if j == i
                        || *sp != space
                        || *speed > 0.35
                        || (wanted[j].length() > 0.2 && wanted[j].dot(dir) <= 0.0)
                    {
                        return false;
                    }
                    // seated people are out of the way, somebody giving way is passed
                    if (matches!(self.people[j].state, State::Riding { .. })
                        && self.people[j].activity == Activity::Sit)
                        || self.people[j].why == YIELDING
                    {
                        return false;
                    }
                    let d = *pos - me;
                    let dist = d.length();
                    dist < 0.68 && dist > 1e-3 && d.dot(dir) / dist > 0.75
                });
            match blocker {
                Some(j) if self.people[i].blocked < 5.0 => {
                    self.people[i].blocked += dt;
                    w.vel = DVec2::ZERO;
                    if self.people[i].why != "in the queue" {
                        self.people[i].why = "queueing behind somebody";
                    }
                    if debug_pax()
                        && self.people[i].blocked >= 3.0
                        && self.people[i].blocked - dt < 3.0
                    {
                        log::info!(
                            "t={:.1} pax {} ({}) has waited 3 s behind pax {} ({}, {})",
                            self.time,
                            self.people[i].label(),
                            self.people[i].state.name(),
                            self.people[j].label(),
                            self.people[j].state.name(),
                            self.people[j].why
                        );
                    }
                }
                Some(_) => {}
                None => self.people[i].blocked = 0.0,
            }
        }
        // where everybody stood against where the vehicles are now, before the crowd step
        // shoves anybody out of a vehicle's box
        if omsi_cfg::env::var_os("OMSI_CHECK_OVERLAP").is_some() {
            self.check_overlaps(world, traffic, player);
        }
        // Somebody on foot whose way a standing vehicle blocks - a car that has pulled up on
        // the crossing, a bus in the yard - waits for it instead of walking into its side
        // (they pressed against it, slid along it and were drawn back by their path into
        // it again, over and over); after a while they go round it.
        for i in 0..self.people.len() {
            let p = &self.people[i];
            if remove.contains(&i) || p.puppet.is_some() || p.remote || !matches!(p.place, Place::Ground) {
                continue;
            }
            let want = wants[i].vel;
            let speed = want.length();
            if speed < 0.2 {
                self.people[i].car_wait = 0.0;
                continue;
            }
            let ahead = p.position.truncate() + want / speed * 0.9;
            let in_way = blocks.iter().any(|b| b.vel.length() < 0.5 && b.near(ahead, BODY_OUTSIDE + 0.15) && {
                let (q, inside) = b.closest(ahead);
                inside || (ahead - q).length() < BODY_OUTSIDE + 0.15
            });
            if !in_way {
                self.people[i].car_wait = 0.0;
                continue;
            }
            self.people[i].car_wait += dt;
            if self.people[i].car_wait > 8.0 {
                // round it: off the path's corridor for a few seconds, the vehicle's box
                // steering them past its end
                self.people[i].detour = self.people[i].detour.max(4.0);
                self.people[i].car_wait = 0.0;
            } else if self.people[i].detour <= 0.0 {
                wants[i].vel = DVec2::ZERO;
            }
        }
        // the crowd
        let mut walkers: Vec<Walker> = Vec::with_capacity(self.people.len());
        let mut who: Vec<usize> = Vec::with_capacity(self.people.len());
        for (i, p) in self.people.iter().enumerate() {
            if remove.contains(&i) || p.puppet.is_some() || p.remote {
                continue;
            }
            let (space, pos, fixed) = match (p.place, &p.state) {
                (Place::Bus(_, _), State::Riding { .. }) if p.activity == Activity::Sit => continue,
                (Place::Bus(b, l), State::Riding { .. }) => {
                    (b.space(), l.truncate().as_dvec2(), false)
                }
                (Place::Bus(b, l), _) => (b.space(), l.truncate().as_dvec2(), false),
                (Place::Ground, State::Waiting { .. }) if p.activity == Activity::Sit => {
                    (0, p.position.truncate(), true)
                }
                (Place::Ground, _) => (0, p.position.truncate(), false),
            };
            let w = &wants[i];
            let radius = if space == 0 { BODY_OUTSIDE } else { BODY };
            walkers.push(Walker {
                pos,
                vel: p.vel,
                radius,
                want: w.vel,
                give: w.give,
                space,
                fixed,
                ghost: p.ghost > 0.0,
                corridor: if p.detour > 0.0 { None } else { w.corridor },
            });
            who.push(i);
        }
        let near_blocks: Vec<Block> = blocks
            .into_iter()
            .filter(|b| walkers.iter().any(|w| w.space == 0 && b.near(w.pos, 25.0)))
            .collect();
        let ground_params = CrowdParams::default();
        let cabin_params = CrowdParams::cabin();
        // one step per kind of floor: the ground and the buses behave differently
        let (mut ground, mut cabin): (Vec<(usize, Walker)>, Vec<(usize, Walker)>) = walkers
            .into_iter()
            .enumerate()
            .partition(|(_, w)| w.space == 0);
        let mut g: Vec<Walker> = ground.iter().map(|x| x.1).collect();
        crowd::step(&mut g, &near_blocks, &ground_params, dt as f64);
        for (k, w) in g.into_iter().enumerate() {
            ground[k].1 = w;
        }
        let mut c: Vec<Walker> = cabin.iter().map(|x| x.1).collect();
        crowd::step(&mut c, &[], &cabin_params, dt as f64);
        for (k, w) in c.into_iter().enumerate() {
            cabin[k].1 = w;
        }
        let mut moved = vec![false; self.people.len()];
        self.keep_out_of_walls(world, &who, &mut ground);
        for (k, w) in ground.into_iter().chain(cabin) {
            let i = who[k];
            moved[i] = true;
            self.apply(i, &w, &wants[i], dt, world, net, &buses, &bus_ix);
        }
        for i in 0..self.people.len() {
            if !moved[i] {
                self.carry(i, dt, &buses, &bus_ix, wants[i].face);
            }
        }
        self.animate(dt, world, &buses, &bus_ix);
        // the timetable buses' door requests, as OMSI reports them to every bus script
        self.ai_requests.clear();
        for bn in &buses {
            let BusId::Ai(id) = bn.id else { continue };
            let mut entry = vec![false; bn.cabin.entries.len()];
            let mut exit = vec![false; bn.cabin.exits.len()];
            for p in &self.people {
                match p.state {
                    State::Queue { bus, entry: e, .. } if bus == bn.id => {
                        if let Some(r) = entry.get_mut(e) {
                            *r = true;
                        }
                    }
                    // everyone on the way out asks for the door: walking to it, waiting at
                    // it, and stepping through it (the door script drops the stop request
                    // as soon as the door is open and keeps the door open only while a
                    // request stands, so the second person off found it shut)
                    State::AtExit { bus, exit: x }
                    | State::Aboard {
                        bus,
                        goal: Goal::ExitWait(x),
                        ..
                    } if bus == bn.id && p.leaving_here && self.exit_req_time.get(&p.id).is_none_or(|t| *t < EXIT_REQ_LAPSE) => {
                        if let Some(r) = exit.get_mut(x) {
                            *r = true;
                        }
                    }
                    State::Aboard {
                        bus,
                        goal: Goal::Exit(x),
                        ..
                    } if bus == bn.id && self.exit_req_time.get(&p.id).is_none_or(|t| *t < EXIT_REQ_LAPSE) => {
                        if let Some(r) = exit.get_mut(x) {
                            *r = true;
                        }
                    }
                    _ => {}
                }
            }
            self.ai_requests.push((id, entry, exit));
        }
        // requests to hold timetable buses while people still get on
        for bn in &buses {
            let BusId::Ai(id) = bn.id else { continue };
            if bn.stop.is_none() {
                continue;
            }
            let age = self
                .ai_visits
                .get(&id)
                .map(|v| self.time - v.1)
                .unwrap_or(0.0);
            // getting on: queueing, or just through the door
            let boarding = self.people.iter().any(|p| match &p.state {
                State::Queue { bus, .. } => *bus == bn.id,
                State::Aboard {
                    goal: Goal::Exit(_) | Goal::ExitWait(_),
                    ..
                } => false,
                State::Aboard { bus, idx, .. } => *bus == bn.id && *idx == 0,
                _ => false,
            });
            // getting off: everyone whose stop this is, from standing up to stepping out -
            // a long walk to the door (17 s through a full bus) must not see the bus
            // finish its stop and the doors close in front of them
            let alighting = self.people.iter().any(|p| match &p.state {
                State::Aboard {
                    bus,
                    goal: Goal::Exit(_),
                    ..
                } => *bus == bn.id,
                State::Aboard {
                    bus,
                    goal: Goal::ExitWait(_),
                    ..
                }
                | State::AtExit { bus, .. }
                | State::Riding { bus, .. } => *bus == bn.id && p.leaving_here,
                _ => false,
            });
            // (the walks inside give up after 90 s, see `Aboard`)
            if (boarding && age < 50.0) || (alighting && age < 100.0) {
                self.holds.push((id, 2.5));
            }
        }
        remove.sort_unstable();
        remove.dedup();
        for i in remove.into_iter().rev() {
            self.release(i);
            let p = self.people.swap_remove(i);
            if debug_pax() {
                log::info!(
                    "t={:.1} pax {} taken away ({}){}",
                    self.time,
                    p.label(),
                    p.state.name(),
                    if self.seen(p.position) {
                        " IN SIGHT"
                    } else {
                        ""
                    }
                );
            }
            self.retire(&p);
        }
        for p in self.people.iter_mut() {
            if debug_pax() && p.why != p.why_logged {
                if !p.why.is_empty() {
                    log::info!(
                        "t={:.1} pax {} ({}) waits: {}",
                        self.time,
                        p.label(),
                        p.state.name(),
                        p.why
                    );
                }
                p.why_logged = p.why;
            }
        }
        if debug_pax() {
            let hint = self.hint();
            if hint
                .as_ref()
                .map(|h| h.split(" (").next().unwrap_or("").to_string())
                != self.last_hint
            {
                log::info!("t={:.1} HUD hint: {:?}", self.time, hint);
                self.last_hint = hint.map(|h| h.split(" (").next().unwrap_or("").to_string());
            }
        }
        self.give_ticket = false;
        taken_ticket
    }

    /// What person `i` wants this frame, and the changes of plan that follow.
    #[allow(clippy::too_many_arguments)]
    fn decide(
        &mut self,
        i: usize,
        dt: f32,
        world: &World,
        player_bus: Option<&VehicleInstance>,
        net: Option<&Network>,
        traffic: Option<&Traffic>,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        slot_of: &HashMap<usize, usize>,
        cars: &[(DVec2, DVec2, f64)],
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
        remove: &mut Vec<usize>,
    ) -> Want {
        let state = self.people[i].state.clone();
        let pos2 = self.people[i].position.truncate();
        let pace = self.people[i].pace;
        let id = self.people[i].id;
        // people inside a bus that is gone (a timetable bus left the map) go with it
        if let Place::Bus(b, _) = self.people[i].place {
            if !bus_ix.contains_key(&b) {
                remove.push(i);
                return Want::stand(None, Activity::Stand);
            }
        }
        // somebody walking on towards ground that is not loaded (far from everybody's eyes:
        // tiles go only well beyond the view) goes
        if self.people[i].place == Place::Ground
            && self.people[i].vel.length_squared() > 1e-4
            && !world.has_ground(pos2.x, pos2.y)
        {
            remove.push(i);
            return Want::stand(None, Activity::Stand);
        }
        match state {
            State::Strolling(mut walk) => {
                let seen = self.seen(self.people[i].position);
                // (a stroller goes only once well out of everybody's range and out of sight:
                // at the populating radius itself people vanished just behind the camera)
                let far = self.far_from_players(self.people[i].position, STROLL_RADIUS * 2.0);
                let Some(net) = net else {
                    remove.push(i);
                    return Want::stand(None, Activity::Stand);
                };
                if far && !seen {
                    remove.push(i);
                    return Want::stand(None, Activity::Stand);
                }
                let w = self.walk_want(i, &mut walk, net, traffic, cars, dt);
                self.people[i].state = State::Strolling(walk);
                w
            }
            State::ToStop {
                stop,
                spot,
                mut walk,
            } => {
                let Some(net) = net else {
                    self.set_state(i, State::ToSpot { stop, spot });
                    return Want::stand(None, Activity::Stand);
                };
                let done = walk.leg >= walk.legs.len();
                if done || self.people[i].t_state > 240.0 {
                    self.set_state(i, State::ToSpot { stop, spot });
                    return Want::stand(None, Activity::Stand);
                }
                // close to the waiting place already: straight there
                if let Some(sp) = self.stops.get(&stop).and_then(|s| s.spots.get(spot)) {
                    if walk.leg + 1 >= walk.legs.len()
                        && (sp.floor().truncate() - pos2).length() < 4.0
                    {
                        self.set_state(i, State::ToSpot { stop, spot });
                        return Want::stand(None, Activity::Stand);
                    }
                }
                let w = self.walk_want(i, &mut walk, net, traffic, cars, dt);
                self.people[i].state = State::ToStop { stop, spot, walk };
                w
            }
            State::ToSpot { stop, spot } => {
                let Some(sp) = self
                    .stops
                    .get(&stop)
                    .and_then(|s| s.spots.get(spot))
                    .cloned()
                else {
                    self.set_state(
                        i,
                        State::Leaving {
                            target: self.people[i].position,
                            walk: None,
                            walked: 0.0,
                        },
                    );
                    return Want::stand(None, Activity::Stand);
                };
                let to = sp.floor().truncate();
                let d = (to - pos2).length();
                if d < 0.25 || self.people[i].t_state > 60.0 {
                    let patience = 240.0 + self.rand_f() as f32 * 600.0;
                    self.set_state(
                        i,
                        State::Waiting {
                            stop,
                            spot,
                            patience,
                        },
                    );
                }
                Want {
                    vel: arrive(pos2, to, pace),
                    face: Some(sp.face),
                    give: 1.0,
                    corridor: None,
                    idle: Activity::Stand,
                    follow: false,
                    goal_dist: Some(d),
                }
            }
            State::Waiting {
                stop,
                spot,
                patience,
            } => {
                let Some(sp) = self
                    .stops
                    .get(&stop)
                    .and_then(|s| s.spots.get(spot))
                    .cloned()
                else {
                    self.set_state(
                        i,
                        State::Leaving {
                            target: self.people[i].position,
                            walk: None,
                            walked: 0.0,
                        },
                    );
                    return Want::stand(None, Activity::Stand);
                };
                // sits down within 0.3 m of the seat and stays seated as long as they are
                // within 0.8 m: nudged a few centimetres off it by somebody passing, a
                // person used to stand up, step back and sit down again, over and over
                let off_seat = (sp.floor().truncate() - pos2).length();
                let seated_now = self.people[i].activity == Activity::Sit;
                let idle = if sp.seat > 0.0 && (off_seat < 0.3 || (seated_now && off_seat < 0.8)) {
                    Activity::Sit
                } else {
                    Activity::Stand
                };
                let stand = Want {
                    vel: if idle == Activity::Sit {
                        DVec2::ZERO
                    } else {
                        arrive(pos2, sp.floor().truncate(), pace * 0.6)
                    },
                    face: Some(sp.face),
                    give: 0.3,
                    corridor: None,
                    idle,
                    follow: false,
                    goal_dist: None,
                };
                // a bus here: board it?
                let avoid = self.people[i].avoid;
                let t_state = self.people[i].t_state;
                let mirror = self.mirror;
                for bn in buses
                    .iter()
                    .filter(|b| b.stop == Some(stop) && b.standing())
                    // (a LAN client's people board only its own bus: the host's boards its)
                    .filter(|b| !mirror || b.id == BusId::Player)
                {
                    if avoid == Some(bn.id) {
                        self.people[i].why = "waits for another bus (just got off this one)";
                        continue;
                    }
                    if !self.goes_their_way(i, stop, bn) {
                        self.people[i].why = "waits for another line";
                        continue;
                    }
                    if !self.doors_open_or_arriving(bn) {
                        // parked there with every door shut (a break, a defect, a driver
                        // who has not opened up): stay at the waiting place instead of
                        // walking up to it
                        self.people[i].why = "the bus here has no door open";
                        continue;
                    }
                    // Each notices the bus in their own time (0.2 to 2.4 s after it stopped,
                    // the ones reading their paper later): all of the stop stepping forward
                    // in the same frame, after standing unmoved, looked like a switch.
                    let since = match bn.id {
                        BusId::Player => (self.served_stop == bn.stop).then_some(self.served_stop_since),
                        BusId::Ai(id) => self.ai_visits.get(&id).map(|v| v.1),
                    };
                    let react = 0.2 + ((self.people[i].id.wrapping_mul(2654435761) >> 7) % 1000) as f64 / 1000.0 * 2.2;
                    if since.map(|t| self.time - t < react && self.time >= t).unwrap_or(false) {
                        self.people[i].why = "has not noticed the bus yet";
                        continue;
                    }
                    let free = self
                        .seats
                        .get(&bn.id)
                        .map(|v| v.iter().any(|t| !t))
                        .unwrap_or(false);
                    if !free {
                        self.people[i].why = "the bus is full";
                        continue;
                    }
                    // the ticket for this bus first: which door and whether the cash desk
                    // are wanted depend on it
                    if !self.people[i].ticket_decided {
                        let walk_in = bn.id == BusId::Player && self.boarding.eq_ignore_ascii_case("walk");
                        self.decide_ticket(i, !bn.cabin.data.stampers.is_empty(), bn.cabin.desk.is_some() && !walk_in);
                    }
                    let Some(entry) = self.choose_entry(i, bn) else {
                        self.people[i].why = "no entry this passenger may use";
                        continue;
                    };
                    self.set_state(
                        i,
                        State::Queue {
                            bus: bn.id,
                            entry,
                            stop,
                            spot,
                            joined: self.time
                                + (bn.world(bn.cabin.entries[entry].outside).truncate() - pos2)
                                    .length()
                                    * 0.3,
                        },
                    );
                    return stand;
                }
                // a bus of theirs pulling in: up and a step or two towards the kerb to meet it,
                // facing it (see `BusNow::approach`); they board once it stands, as above
                let coming = buses
                    .iter()
                    .filter(|b| b.approach == Some(stop) && avoid != Some(b.id))
                    .filter(|b| !mirror || b.id == BusId::Player)
                    .filter(|b| self.goes_their_way(i, stop, b))
                    .min_by(|a, c| (a.pos.truncate() - pos2).length().total_cmp(&(c.pos.truncate() - pos2).length()));
                if let Some(bn) = coming {
                    let h = bn.heading.to_radians();
                    let dir = DVec2::new(h.sin(), h.cos());
                    let rel = pos2 - bn.pos.truncate();
                    let side = rel - dir * rel.dot(dir);
                    let lat = side.length();
                    let to_bus = bn.pos.truncate() - pos2;
                    let face = to_bus.x.atan2(to_bus.y).to_degrees();
                    // a metre clear of its side, and a step or two from the waiting place at
                    // most: OMSI's people keep to the walkways of the stop (its path links),
                    // and allowed 4 m they stood out on the road before the bus had stopped
                    // (a bus pulling in along the far lane, #123)
                    let clear = bn.half.x + 1.0;
                    let home = sp.floor().truncate();
                    let target = home + (pos2 - side / lat.max(1e-6) * (lat - clear) - home).clamp_length_max(1.2);
                    // (never off the pavement: a waiting place at the kerb's edge had them
                    // step out onto the carriageway in front of the bus, #123)
                    let off_kerb = net.is_some_and(|n| {
                        on_carriageway(n, target.extend(sp.floor().z)) || crosses_street(n, home, target)
                    });
                    if lat > clear + 0.3 && !off_kerb {
                        let d = (target - pos2).length();
                        self.people[i].why = "steps forward to meet the bus";
                        return Want {
                            vel: arrive(pos2, target, pace * 0.7),
                            face: Some(face),
                            give: 0.5,
                            corridor: None,
                            idle: Activity::Stand,
                            follow: false,
                            goal_dist: Some(d),
                        };
                    }
                    self.people[i].why = "waits at the kerb for the bus";
                    return Want::stand(Some(face), Activity::Stand);
                }
                if t_state > patience && !buses.iter().any(|b| b.stop == Some(stop)) {
                    // waited long enough: walks off (and somebody else will come)
                    if debug_pax() {
                        log::info!(
                            "t={:.1} pax {} gives up waiting at stop {stop} '{}' after {:.0} s",
                            self.time,
                            self.people[i].label(),
                            self.stops[&stop].name,
                            t_state
                        );
                    }
                    self.free_spot(stop, spot, id);
                    let away = self.people[i].position
                        + DVec3::new(
                            (self.rand_f() - 0.5) * 6.0,
                            (self.rand_f() - 0.5) * 6.0,
                            0.0,
                        );
                    self.set_state(
                        i,
                        State::Leaving {
                            target: away,
                            walk: None,
                            walked: 0.0,
                        },
                    );
                }
                // the "wants another line" passengers of the last bus take the next one
                if t_state > 120.0 {
                    self.people[i].avoid = None;
                }
                stand
            }
            State::Queue {
                bus,
                entry,
                stop,
                spot,
                joined,
            } => {
                let bi = bus_ix.get(&bus).copied();
                let back = |h: &mut Humans, why: &'static str| {
                    if debug_pax() {
                        log::info!(
                            "t={:.1} pax {} leaves the queue: {why}",
                            h.time,
                            h.people[i].label()
                        );
                    }
                    let patience = 120.0 + h.rand_f() as f32 * 300.0;
                    h.set_state(
                        i,
                        State::Waiting {
                            stop,
                            spot,
                            patience,
                        },
                    );
                };
                let Some(bi) = bi else {
                    back(self, "the bus is gone");
                    return Want::stand(None, Activity::Stand);
                };
                let bn = &buses[bi];
                if bn.stop != Some(stop) || !bn.standing() {
                    back(self, "the bus pulls away");
                    self.people[i].avoid = Some(bus);
                    return Want::stand(None, Activity::Stand);
                }
                if self.people[i].t_state > 120.0 {
                    back(self, "the door never opened");
                    self.people[i].avoid = Some(bus);
                    return Want::stand(None, Activity::Stand);
                }
                let door = &bn.cabin.entries[entry];
                let slot = slot_of.get(&i).copied().unwrap_or(0);
                let side = bn.right_at(door.inside) * door.side as f64;
                let base_w = bn.world(door.outside);
                let base = base_w.truncate();
                // the queue leaves the door at an angle, out from the bus and along the kerb:
                // laid along the side, 0.6 m from it, the people stood beside the body turned
                // towards it (staring at the wall), and the one at the front came to the
                // door along the side, through the folded leaf
                let q_dir = (side * 0.55 + bn.fwd_at(door.inside) * (door.queue_dir as f64 * 0.83)).normalize_or_zero();
                // (a queue running forwards stops short of the bus's front and turns out from
                // it: at a door just behind the windscreen the line went on round the nose,
                // and the people stood across the road in front of the bus, facing it)
                let q_len = if door.queue_dir > 0.0 {
                    let room = bn.centre.y + bn.half.y - door.outside.y as f64 - 0.8;
                    (room.max(0.0) / q_dir.dot(bn.fwd_at(door.inside)).max(0.1)).max(0.0)
                } else {
                    f64::INFINITY
                };
                // people getting off through this door: the front of the queue stands aside
                // for them - on the door's outside point it stood where they step down to,
                // and each waited for the other until someone gave up (#253: "passengers
                // stand at the door a long time before going in")
                let alighting = self.people.iter().any(|p| matches!(p.state, State::Aboard { bus: b, goal: Goal::Exit(x), .. } if b == bus && (bn.cabin.exits[x].inside - door.inside).length() < 2.0));
                let aside = if alighting { 0.8 } else { 0.0 };
                let place = |k: usize| {
                    let d = QUEUE_GAP * k as f64 + aside;
                    if d <= q_len {
                        base + q_dir * d
                    } else {
                        base + q_dir * q_len + side * (d - q_len)
                    }
                };
                let spot_pos = place(slot);
                let d = (spot_pos - pos2).length();
                // facing the door at the front, else the one ahead in the line
                let face_door = {
                    let v = if slot == 0 { bn.world(door.inside).truncate() - pos2 } else { place(slot - 1) - pos2 };
                    let v = if v.length() < 0.2 { -q_dir } else { v };
                    v.x.atan2(v.y).to_degrees()
                };
                let mut w = Want {
                    vel: arrive(pos2, spot_pos, pace),
                    face: Some(face_door),
                    give: 0.6,
                    corridor: None,
                    idle: Activity::Stand,
                    follow: slot > 0,
                    goal_dist: Some(d),
                };
                if slot > 0 {
                    self.people[i].why = "in the queue";
                    // A pass holder behind people who pay goes to the other leaf when it is
                    // open, has a shorter queue and does not lead past the cash desk: in
                    // `pay` boarding they stood 18 s for every payer ahead of them.
                    // (only on the way: somebody already standing at their door stays there
                    // - walking over to the other door at the last moment looked like a
                    // change of mind for no reason)
                    if let Some(other) = self
                        .pass_holder_leaf(i, bn, bus, entry, slot, slot_of)
                        .filter(|_| d > 2.5)
                    {
                        if debug_pax() {
                            log::info!("t={:.1} pax {} shows a pass: from the queue at entry {entry} (place {slot}, payers ahead) to entry {other}", self.time, self.people[i].label());
                        }
                        self.set_state(
                            i,
                            State::Queue {
                                bus,
                                entry: other,
                                stop,
                                spot,
                                joined: self.time,
                            },
                        );
                    }
                    return w;
                }
                if !bn.entry_open.get(entry).copied().unwrap_or(false) {
                    // another entry of the bus is open: go there - on the way, or when this
                    // door has stayed shut a while (not the moment it closes behind the one
                    // before)
                    if let Some(other) = self
                        .choose_entry(i, bn)
                        .filter(|e| *e != entry && bn.entry_open.get(*e).copied().unwrap_or(false))
                    {
                        self.set_state(
                            i,
                            State::Queue {
                                bus,
                                entry: other,
                                stop,
                                spot,
                                joined,
                            },
                        );
                        return w;
                    }
                    // every door has been shut long enough that none is about to open
                    // either (the driver parked here, or gave up on this stop): back to
                    // the waiting place rather than standing at a door that never opens
                    if !self.doors_open_or_arriving(bn) {
                        back(self, "every door is shut and none looks about to open");
                        return Want::stand(None, Activity::Stand);
                    }
                    self.people[i].why = "the door is shut";
                    return w;
                }
                if d > 0.6 {
                    // held off the door by something of the map in the way (a railing, a
                    // pole, a shelter's wall: people are kept out of its collision boxes)
                    // - as close as they get is close enough. They stood a metre from the
                    // open door until the bus left without them.
                    let held = d < 2.0 && self.people[i].stuck > 1.0;
                    if !held {
                        self.people[i].why = "";
                        return w;
                    }
                    if debug_pax() {
                        log::info!("t={:.1} pax {} cannot get closer to entry {entry} than {d:.1} m: boards from there", self.time, self.people[i].label());
                    }
                }
                if self.door_busy.contains_key(&(bus, false, entry)) {
                    self.people[i].why = "the doorway is busy";
                    return w;
                }
                // people getting off first
                if alighting {
                    self.people[i].why = "lets people off first";
                    return w;
                }
                let cabin = bn.cabin.clone();
                let taken = self.seats.get(&bus).cloned().unwrap_or_default();
                let luck: Vec<f32> = (0..taken.len()).map(|_| self.rand_f() as f32).collect();
                let Some(seat) = cabin.choose_seat(&taken, door.inside, |k| luck[k]) else {
                    back(self, "the bus is full");
                    self.people[i].avoid = Some(bus);
                    return w;
                };
                let walk_in = bus == BusId::Player && self.boarding.eq_ignore_ascii_case("walk");
                if !self.people[i].ticket_decided {
                    self.decide_ticket(i, !cabin.data.stampers.is_empty(), cabin.desk.is_some() && !walk_in);
                }
                // (one who stamps a ticket shows nothing at the desk: the validator is theirs)
                let stamps = self.people[i].stamps && !cabin.stampers.is_empty();
                let needs_desk = !walk_in
                    && cabin.desk.is_some()
                    && (self.people[i].ticket.is_some()
                        || (!stamps && cabin.passes_desk(entry, cabin.seats[seat].floor)));
                if needs_desk {
                    let desk_busy = self.people.iter().any(|p| matches!(p.state, State::AtDesk { bus: b, .. } | State::Aboard { bus: b, goal: Goal::Desk(_), .. } if b == bus));
                    if desk_busy {
                        // the other leaf leads past the desk? then a pass holder takes it
                        self.people[i].why = "the cash desk is busy";
                        if self.people[i].ticket.is_none() && self.people[i].t_state > 2.0 && d > 1.5 {
                            if let Some(other) = (0..cabin.entries.len()).find(|&e| {
                                e != entry
                                    && bn.entry_open.get(e).copied().unwrap_or(false)
                                    && !cabin.passes_desk(e, cabin.seats[seat].floor)
                            }) {
                                self.set_state(
                                    i,
                                    State::Queue {
                                        bus,
                                        entry: other,
                                        stop,
                                        spot,
                                        joined,
                                    },
                                );
                            }
                        }
                        return w;
                    }
                }
                // in
                self.seats.get_mut(&bus).unwrap()[seat] = true;
                self.free_spot(stop, spot, id);
                self.door_busy.insert((bus, false, entry), 0.8);
                // the kerb's height in the frame of the section the door is in
                let ground = world.walk_height(base.x, base.y).unwrap_or(base_w.z);
                // from where they stand (up to 0.6 m from the door's outside point: put
                // on that point they jumped there in one frame), carried into the frame
                // of the door's section with their walking speed
                let (r_door, f_door) = (bn.right_at(door.outside), bn.fwd_at(door.outside));
                let rel = pos2 - base;
                let start = Vec3::new(
                    door.outside.x + rel.dot(r_door) as f32,
                    door.outside.y + rel.dot(f_door) as f32,
                    (ground - base_w.z) as f32,
                );
                let vel_world = self.people[i].vel;
                let vel_local = DVec2::new(vel_world.dot(r_door), vel_world.dot(f_door));
                // one who stamps goes to the validator nearest the door first
                let stamper = (!needs_desk && self.people[i].stamps)
                    .then(|| {
                        (0..cabin.stampers.len()).min_by(|&a, &b| {
                            (cabin.stampers[a].0 - door.inside).length().total_cmp(&(cabin.stampers[b].0 - door.inside).length())
                        })
                    })
                    .flatten();
                let goal_pos = if needs_desk {
                    cabin.desk.unwrap().0
                } else if let Some(k) = stamper {
                    cabin.stampers[k].0
                } else {
                    cabin.seats[seat].floor
                };
                let mut route = vec![door.inside];
                route.extend(cabin.route(door.inside, goal_pos));
                let goal = if needs_desk {
                    Goal::Desk(seat)
                } else if let Some(k) = stamper {
                    Goal::Stamper(seat, k)
                } else {
                    Goal::Seat(seat)
                };
                self.greet_or_complain(i, bus, bn.interior, bn.air);
                if debug_pax() {
                    log::info!("t={:.1} pax {} boards {:?} through entry {entry} to place {seat} ({}{}), {}", self.time, self.people[i].label(), bus, if cabin.seats[seat].seated { "seat" } else { "standing" }, cabin.part_label(cabin.seats[seat].floor), match (needs_desk, self.people[i].ticket) {
                        (true, Some(t)) => format!("buys ticket {t}"),
                        (true, None) => "shows a pass".to_string(),
                        _ => "straight in".to_string(),
                    });
                }
                let p = &mut self.people[i];
                p.place = Place::Bus(bus, start);
                p.vel = vel_local;
                p.lheading = p.heading - bn.heading_at(start);
                p.from = stop;
                p.leaving_here = false;
                p.avoid = None;
                // OMSI_PAX_STOPS=n: everybody gets off a timetable bus after n stops (a test)
                let test_stops: Option<i32> = omsi_cfg::env::var("OMSI_PAX_STOPS").ok().and_then(|v| v.parse().ok());
                p.stops_left = test_stops.unwrap_or(i32::MAX);
                p.exit_id = None;
                let exit = if bus == BusId::Player {
                    self.choose_exit(player_bus, world)
                } else {
                    -1
                };
                // a timetable bus: where to get off among the stops it still serves, drawn
                // as Omsi.exe draws it (`draw_exit`); nothing to draw from: to its end
                if let (BusId::Ai(id), None) = (bus, test_stops) {
                    let ahead: Vec<i64> = traffic
                        .and_then(|t| t.cars.iter().find(|c| c.id == id))
                        .and_then(|c| c.bus.as_ref())
                        .map(|b| b.stops.iter().map(|st| st.id).filter(|&x| x != stop && x != 0).collect())
                        .unwrap_or_default();
                    let k = self.draw_exit(&ahead, world);
                    self.people[i].exit_id = k.map(|k| ahead[k]);
                }
                self.people[i].exit_stop = exit;
                if bus == BusId::Player {
                    // (the stop itself as well: the timetable's index starts again at 0 with
                    // the next trip, see where riders decide to get off)
                    self.people[i].exit_id = usize::try_from(exit).ok().and_then(|k| player_bus.and_then(|b| b.host.tt_stop_ids.get(k))).copied().filter(|&x| x != 0);
                }
                self.set_state(
                    i,
                    State::Aboard {
                        bus,
                        route,
                        idx: 0,
                        seg: start,
                        goal,
                    },
                );
                // on towards the doorway without stopping first
                let to_in = (door.inside - start).truncate().as_dvec2();
                w.vel = if to_in.length() > 0.05 {
                    to_in.normalize() * vel_local.length().max(0.5).min(pace)
                } else {
                    DVec2::ZERO
                };
                w
            }
            State::Aboard {
                bus,
                route,
                idx,
                seg,
                goal,
            } => {
                let Some(&bi) = bus_ix.get(&bus) else {
                    return Want::stand(None, Activity::Stand);
                };
                let bn = &buses[bi];
                let Place::Bus(_, local) = self.people[i].place else {
                    return Want::stand(None, Activity::Stand);
                };
                // stamping: a moment at the validator
                if self.people[i].pause_until > self.time {
                    return Want::stand(None, Activity::Stand);
                }
                let (mut idx, mut seg) = (idx, seg);
                let here = local.truncate().as_dvec2();
                // waypoints reached (or passed)
                while idx < route.len() {
                    let last = idx + 1 == route.len();
                    let tgt = route[idx].truncate().as_dvec2();
                    let dist = (tgt - here).length();
                    let passed = if !last {
                        let next = route[idx + 1].truncate().as_dvec2();
                        (next - here).length() < (next - tgt).length() - 0.05 && dist < 0.8
                    } else if let (Goal::Exit(_), true) = (goal, idx > 0) {
                        // the step off the bus: out is out - somebody beside the spot (the
                        // one before them may still stand on it) or past it is on the ground.
                        // Waiting to stand exactly on it, the people getting off crowded onto
                        // one point and held each other up for half a minute.
                        let from = route[idx - 1].truncate().as_dvec2();
                        let along = (tgt - from).normalize_or_zero();
                        dist < 0.45 || (here - from).dot(along) >= (tgt - from).length()
                    } else {
                        false
                    };
                    if dist < if last { 0.1 } else { 0.25 } || passed {
                        seg = route[idx];
                        idx += 1;
                    } else {
                        break;
                    }
                }
                if let (Goal::ExitWait(x), true) = (goal, idx < route.len()) {
                    // others already wait at that exit: the end of their line will do - once
                    // on the exit's own floor. The upper deck of a double-decker lies right
                    // over that line: riders coming along it stopped up there, over the rear
                    // door, and later stepped out through the stairs and the panel.
                    let queued = self.people.iter().filter(|p| matches!(p.state, State::AtExit { bus: b, exit } if b == bus && exit == x)).count();
                    let door = &bn.cabin.exits[x];
                    if queued > 0
                        && door.on_floor(local.z)
                        && door.on_floor(seg.z)
                        && door.on_floor(route[idx].z)
                    {
                        let end = bn.cabin.exit_queue_place(x, queued, local.y).truncate().as_dvec2();
                        if (end - here).length() < 0.5 {
                            idx = route.len();
                        }
                    }
                }
                if idx < route.len() && self.people[i].t_state > 90.0 {
                    // a walk that should take seconds has taken minutes: put them where they were going
                    log::warn!("pax {} could not walk to {:?} in {:?} in 90 s (at {:?}, next {:?}); placed there", self.people[i].label(), goal, bus, local, route[idx]);
                    let end = *route.last().unwrap();
                    self.people[i].place = Place::Bus(bus, end);
                    idx = route.len();
                }
                if idx >= route.len() {
                    self.arrived(i, bus, bn, goal, &route, world);
                    return Want::stand(None, Activity::Stand);
                }
                // the doors closed before somebody getting off was through them: back to
                // wait (on the last leg only while still in the doorway)
                if let Goal::Exit(x) = goal {
                    let inside = idx + 1 < route.len()
                        || (idx > 0 && {
                            let (from, to) = (
                                route[idx - 1].truncate().as_dvec2(),
                                route[idx].truncate().as_dvec2(),
                            );
                            (here - from).dot(to - from) < 0.35 * (to - from).length_squared()
                        });
                    if !bn.exit_open.get(x).copied().unwrap_or(false) && inside {
                        self.set_state(
                            i,
                            State::Aboard {
                                bus,
                                route: vec![bn.cabin.exits[x].wait],
                                idx: 0,
                                seg: local,
                                goal: Goal::ExitWait(x),
                            },
                        );
                        return Want::stand(None, Activity::Stand);
                    }
                }
                let tgt = route[idx].truncate().as_dvec2();
                let last = idx + 1 == route.len();
                let d = (tgt - here).length();
                let speed = PACE_IN.min(pace);
                let vel = if last {
                    arrive(here, tgt, speed)
                } else {
                    (tgt - here).normalize_or_zero() * speed
                };
                let dev = if idx == 0 {
                    0.22
                } else if last {
                    0.45
                } else {
                    0.3
                };
                let corridor = Some((seg.truncate().as_dvec2(), tgt, dev));
                // people getting off go first: somebody boarding who meets one in the aisle
                // waits, and the one getting off squeezes past
                let mut vel = vel;
                let dir = vel.normalize_or_zero();
                let alighting = matches!(goal, Goal::Exit(_) | Goal::ExitWait(_));
                let ahead = |p: &Person, cone: f64, reach: f64| -> bool {
                    match p.place {
                        Place::Bus(pb, l) if pb == bus => {
                            let rel = l.truncate().as_dvec2() - here;
                            let dist = rel.length();
                            dist < reach && dist > 1e-3 && rel.dot(dir) / dist > cone
                        }
                        _ => false,
                    }
                };
                if !alighting && idx > 0 {
                    let oncoming = self.people.iter().any(|p| {
                        matches!(
                            p.state,
                            State::Aboard {
                                goal: Goal::Exit(_) | Goal::ExitWait(_),
                                ..
                            }
                        ) && ahead(p, 0.6, 1.4)
                    });
                    if oncoming {
                        vel = DVec2::ZERO;
                        self.people[i].why = YIELDING;
                    }
                } else if alighting
                    && self
                        .people
                        .iter()
                        .any(|p| p.why == YIELDING && ahead(p, 0.3, 1.2))
                {
                    self.people[i].ghost = self.people[i].ghost.max(0.4);
                }
                let give = if matches!(goal, Goal::Exit(_) | Goal::ExitWait(_)) {
                    0.5
                } else {
                    1.0
                };
                self.people[i].state = State::Aboard {
                    bus,
                    route,
                    idx,
                    seg,
                    goal,
                };
                Want {
                    vel,
                    face: None,
                    give,
                    corridor,
                    idle: Activity::Stand,
                    follow: true,
                    goal_dist: Some(d + idx as f64),
                }
            }
            State::AtDesk {
                bus,
                seat,
                ticket,
                done,
            } => {
                let Some(&bi) = bus_ix.get(&bus) else {
                    return Want::stand(None, Activity::Stand);
                };
                let bn = &buses[bi];
                let Some((stand, _, face)) = bn.cabin.desk else {
                    self.leave_desk(i, bus, bn, seat);
                    return Want::stand(None, Activity::Stand);
                };
                let here = self.people[i]
                    .local()
                    .unwrap_or(stand)
                    .truncate()
                    .as_dvec2();
                // (money in hand only at the desk: held up while still stuck at the door
                // behind somebody, it looked like paying the air)
                let at_desk = (here - stand.truncate().as_dvec2()).length() < 0.7;
                let w = Want {
                    vel: arrive(here, stand.truncate().as_dvec2(), 0.5),
                    face: Some(face),
                    give: 0.2,
                    corridor: None,
                    idle: if at_desk { Activity::Pay } else { Activity::Stand },
                    follow: false,
                    goal_dist: None,
                };
                let t = self.people[i].t_state;
                if bus != BusId::Player {
                    if t > 1.2 {
                        self.leave_desk(i, bus, bn, seat);
                    }
                    return w;
                }
                let finished = self.desk_player(
                    i,
                    ticket,
                    done,
                    t,
                    player_bus,
                    world,
                    renderer,
                    scene,
                    taken_ticket,
                );
                if finished {
                    self.leave_desk(i, bus, bn, seat);
                }
                w
            }
            State::Riding { bus, seat } => {
                let Some(&bi) = bus_ix.get(&bus) else {
                    return Want::stand(None, Activity::Stand);
                };
                let bn = &buses[bi];
                let s = bn.cabin.seats[seat].clone();
                let idle = if s.seated {
                    Activity::Sit
                } else {
                    Activity::Stand
                };
                let here = self.people[i]
                    .local()
                    .unwrap_or(s.pos)
                    .truncate()
                    .as_dvec2();
                let w = Want {
                    vel: if s.seated {
                        DVec2::ZERO
                    } else {
                        arrive(here, s.pos.truncate().as_dvec2(), 0.5)
                    },
                    face: Some(s.rot as f64),
                    give: 0.6,
                    corridor: None,
                    idle,
                    follow: false,
                    goal_dist: None,
                };
                // Omsi.exe's riders get up as the bus heads into their stop (state 7, up to 60 m
                // out, while it still rolls: 0x62d129), not once it stands - then they held
                // the doors up at every stop while they came from the back
                let mine_ahead = bn.approach.is_some_and(|a| {
                    let p = &self.people[i];
                    p.exit_id == Some(a)
                        || (bus == BusId::Player && p.exit_id.is_none() && p.exit_stop >= 0 && player_bus.is_some_and(|b| b.host.tt_stop_ids.get(p.exit_stop as usize) == Some(&a)))
                        || self.empties_at(bn, a)
                });
                if mine_ahead {
                    self.people[i].leaving_here = true;
                }
                if self.people[i].leaving_here && ((bn.standing() && bn.stop.is_some()) || mine_ahead) {
                    if bn.cabin.exits.is_empty() {
                        // a cabin without an exit (a trailer section): rides on
                        self.people[i].leaving_here = false;
                        self.people[i].why = "no exit in this part of the bus";
                        return w;
                    }
                    // their stop: stand up and go to the nearest exit
                    let exit = bn.cabin.nearest_exit(s.floor);
                    if debug_pax() {
                        log::info!(
                            "t={:.1} pax {} gets up from place {seat}{} for exit {exit}{}",
                            self.time,
                            self.people[i].label(),
                            bn.cabin.part_label(s.floor),
                            bn.cabin.part_label(bn.cabin.exits[exit].inside)
                        );
                    }
                    self.free_seat(bus, seat);
                    let door = &bn.cabin.exits[exit];
                    let mut route = vec![s.floor];
                    route.extend(bn.cabin.route(s.floor, door.wait));
                    if let Some(l) = self.people[i].local() {
                        if s.seated {
                            // up from the seat onto the floor first
                            self.people[i].place = Place::Bus(bus, Vec3::new(l.x, l.y, s.floor.z));
                        }
                    }
                    let start = self.people[i].local().unwrap_or(s.floor);
                    self.set_state(
                        i,
                        State::Aboard {
                            bus,
                            route,
                            idx: 0,
                            seg: start,
                            goal: Goal::ExitWait(exit),
                        },
                    );
                    self.people[i].activity = Activity::Stand;
                }
                w
            }
            State::AtExit { bus, exit } => {
                let Some(&bi) = bus_ix.get(&bus) else {
                    return Want::stand(None, Activity::Stand);
                };
                let bn = &buses[bi];
                let door = bn.cabin.exits[exit].clone();
                let here = self.people[i]
                    .local()
                    .unwrap_or(door.wait)
                    .truncate()
                    .as_dvec2();
                let face = {
                    let v = door.inside.truncate().as_dvec2() - here;
                    v.x.atan2(v.y).to_degrees()
                };
                // one behind the other along the aisle, the first come nearest the door
                let t_me = self.people[i].t_state;
                let ahead = self.people.iter().enumerate().filter(|(j, p)| *j != i && matches!(p.state, State::AtExit { bus: b, exit: x } if b == bus && x == exit) && (p.t_state > t_me || (p.t_state == t_me && *j < i))).count();
                let spot = bn.cabin.exit_queue_place(exit, ahead, here.y as f32).truncate().as_dvec2();
                let w = Want {
                    vel: arrive(here, spot, 0.5),
                    face: Some(face),
                    give: 0.4,
                    corridor: None,
                    idle: Activity::Stand,
                    follow: false,
                    goal_dist: Some((spot - here).length()),
                };
                if !self.people[i].leaving_here {
                    self.people[i].why = "rides on to the next stop";
                    // the driver did not stop: next stop then (timetable) or a later one
                    if bn.stop.is_none() && !bn.standing() {
                        if bus == BusId::Player {
                            if let Some(b) = player_bus {
                                self.people[i].exit_stop = b.host.tt_busstop_index;
                                self.people[i].exit_id = None;
                            }
                        } else {
                            self.people[i].stops_left = 1;
                        }
                    }
                    return w;
                }
                if !bn.exit_open.get(exit).copied().unwrap_or(false) {
                    // another exit is open: go there
                    if let Some(other) = (0..bn.cabin.exits.len()).find(|&x| bn.exit_open[x]) {
                        let o = &bn.cabin.exits[other];
                        let mut route = bn.cabin.route(door.wait, o.wait);
                        route.retain(|p| (p.truncate() - door.wait.truncate()).length() > 0.05);
                        if route.is_empty() {
                            route.push(o.wait);
                        }
                        let start = self.people[i].local().unwrap_or(door.wait);
                        self.set_state(
                            i,
                            State::Aboard {
                                bus,
                                route,
                                idx: 0,
                                seg: start,
                                goal: Goal::ExitWait(other),
                            },
                        );
                        return w;
                    }
                    self.people[i].why = "the exit door is shut";
                    let t = self.people[i].t_state;
                    if bus == BusId::Player
                        && bn.stop.is_some()
                        && t > 45.0
                        && (t - dt) % 45.0 > t % 45.0
                    {
                        // pressed again, the driver seems to have missed it
                        self.stop_request = true;
                        if debug_pax() {
                            log::info!(
                                "t={:.1} pax {} presses the stop button again",
                                self.time,
                                self.people[i].label()
                            );
                        }
                    }
                    return w;
                }
                // one at a time through the doorway: the next goes when the one before has
                // reached it (released by the clock alone, three or four ended up in the
                // doorway together and blocked each other)
                let in_doorway = self.people.iter().enumerate().any(|(j, p)| j != i && matches!(p.state, State::Aboard { bus: b, goal: Goal::Exit(x), idx: 0, .. } if b == bus && x == exit));
                if self.door_busy.contains_key(&(bus, true, exit)) || in_doorway {
                    self.people[i].why = "the doorway is busy";
                    return w;
                }
                if ahead > 0 {
                    self.people[i].why = "waits for the others to get off";
                    return w;
                }
                // out, from the spot in front of the exit: the step off is a straight line to
                // the doorway. Somebody not on the exit's floor (still on the stairs or the
                // upper deck of a double-decker) walks down to that spot along the path
                // network first, somebody pushed aside steps back onto it.
                let start = self.people[i].local().unwrap_or(door.wait);
                if !door.on_floor(start.z) {
                    if debug_pax() {
                        log::info!("t={:.1} pax {} waited for exit {exit} off its floor (at {:.2}, {:.2}, {:.2}): walks down to it first", self.time, self.people[i].label(), start.x, start.y, start.z);
                    }
                    let route = bn.cabin.route(start, door.wait);
                    self.set_state(
                        i,
                        State::Aboard {
                            bus,
                            route,
                            idx: 0,
                            seg: start,
                            goal: Goal::ExitWait(exit),
                        },
                    );
                    return w;
                }
                // (held off the spot - a pole, a seat back, somebody's bag - as close as they
                // get is close enough, as for boarding: they waited there for good)
                let off = (spot - here).length();
                if off > EXIT_REACH && !(off < 1.5 && self.people[i].stuck > 1.0) {
                    self.people[i].why = "steps up to the exit";
                    return w;
                }
                self.door_busy.insert((bus, true, exit), 1.0);
                let outside_world = bn.world(door.outside);
                let ground = world
                    .walk_height(outside_world.x, outside_world.y)
                    .unwrap_or(outside_world.z);
                let out = Vec3::new(
                    door.outside.x,
                    door.outside.y,
                    (ground - outside_world.z) as f32,
                );
                self.set_state(
                    i,
                    State::Aboard {
                        bus,
                        route: vec![door.inside, out],
                        idx: 0,
                        seg: start,
                        goal: Goal::Exit(exit),
                    },
                );
                w
            }
            State::Leaving {
                target,
                walk,
                walked,
            } => {
                let seen = self.seen(self.people[i].position);
                let t = self.people[i].t_state;
                let from_eye = self
                    .eye
                    .map(|e| (self.people[i].position - e.pos).length())
                    .unwrap_or(0.0);
                // somebody who got off walks their way before leaving the map, out of sight
                // (20 m on, they vanished round the first corner of the stop)
                if !seen && (walked > 120.0 || t > 90.0 || from_eye > 250.0) {
                    remove.push(i);
                    return Want::stand(None, Activity::Stand);
                }
                match walk {
                    Some(mut walk) => {
                        let Some(net) = net else {
                            remove.push(i);
                            return Want::stand(None, Activity::Stand);
                        };
                        let w = self.walk_want(i, &mut walk, net, traffic, cars, dt);
                        let walked = walked + (self.people[i].vel.length() * dt as f64) as f32;
                        self.people[i].state = State::Leaving {
                            target,
                            walk: Some(walk),
                            walked,
                        };
                        w
                    }
                    None => {
                        let d = (target.truncate() - pos2).length();
                        let walked = walked + (self.people[i].vel.length() * dt as f64) as f32;
                        if d < 0.4 || t > 15.0 {
                            // on the pavement: walk off along it, or wait for another bus here
                            let transfer = self.rand_f() < 0.25;
                            let near_stop = self
                                .stops
                                .iter()
                                .filter(|(_, s)| {
                                    (s.pos.truncate() - pos2).length() < 30.0
                                        && s.spots.iter().any(|x| x.taken.is_none())
                                        // not the stop across the road
                                        && !net
                                            .map(|n| crosses_street(n, pos2, s.pos.truncate()))
                                            .unwrap_or(false)
                                })
                                .map(|(k, _)| *k)
                                .next();
                            if let (true, Some(stop)) =
                                (transfer && self.people[i].from >= 0, near_stop)
                            {
                                let free: Vec<usize> = self.stops[&stop]
                                    .spots
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, s)| s.taken.is_none())
                                    .map(|(k, _)| k)
                                    .collect();
                                let k = free[(self.rand() as usize) % free.len()];
                                self.stops.get_mut(&stop).unwrap().spots[k].taken = Some(id);
                                if debug_pax() {
                                    log::info!(
                                        "t={:.1} pax {} changes buses: waits at stop {stop}",
                                        self.time,
                                        self.people[i].label()
                                    );
                                }
                                self.set_state(i, State::ToSpot { stop, spot: k });
                                return Want::stand(None, Activity::Stand);
                            }
                            let lane = net.and_then(|net| {
                                self.ped
                                    .as_ref()
                                    .and_then(|pn| pn.nearest(net, self.people[i].position, 15.0))
                                    .map(|x| (net, x))
                            });
                            match lane {
                                Some((net, (l, s, _))) => {
                                    // along the pavement, the way the person was already heading
                                    let (_, h) = net.lanes[l].at(s);
                                    let hr = (h as f64).to_radians();
                                    let dir = DVec2::new(hr.sin(), hr.cos());
                                    let head = self.people[i].heading.to_radians();
                                    let fwd = dir.dot(DVec2::new(head.sin(), head.cos())) >= 0.0;
                                    let len = net.lanes[l].length();
                                    let leg = if fwd {
                                        Leg {
                                            lane: l,
                                            a: s,
                                            b: len,
                                        }
                                    } else {
                                        Leg {
                                            lane: l,
                                            a: s,
                                            b: 0.0,
                                        }
                                    };
                                    let side = 0.3 + self.rand_f() as f32 * 0.3;
                                    self.people[i].state = State::Leaving {
                                        target,
                                        walk: Some(PedWalk::new(vec![leg], true, side)),
                                        walked,
                                    };
                                }
                                None => {
                                    // no pavement: on in the same direction, unless that
                                    // is over the road - then away from it
                                    let head = self.people[i].heading.to_radians();
                                    let mut dir = DVec2::new(head.sin(), head.cos());
                                    let here = self.people[i].position;
                                    if let Some(n) = net {
                                        if crosses_street(n, pos2, pos2 + dir * 30.0) {
                                            if let Some((l, s, _)) =
                                                n.nearest_lane_near(here, LaneKind::Street)
                                            {
                                                let (q, _) = n.lanes[l].at(s);
                                                dir = (pos2 - q.truncate())
                                                    .try_normalize()
                                                    .unwrap_or(-dir);
                                            }
                                        }
                                    }
                                    let next = here + dir.extend(0.0) * 30.0;
                                    self.people[i].state = State::Leaving {
                                        target: next,
                                        walk: None,
                                        walked,
                                    };
                                }
                            }
                            return Want::stand(None, Activity::Stand);
                        }
                        self.people[i].state = State::Leaving {
                            target,
                            walk: None,
                            walked,
                        };
                        Want {
                            vel: arrive(pos2, target.truncate(), pace).normalize_or_zero() * pace,
                            face: None,
                            give: 1.0,
                            corridor: None,
                            idle: Activity::Stand,
                            follow: false,
                            goal_dist: Some(d),
                        }
                    }
                }
            }
        }
    }

    /// The entry of `bus` person `i` walks to: the nearest open one they may use (one with
    /// a cash desk for somebody who still has to buy a ticket), else the nearest allowed.
    fn choose_entry(&self, i: usize, bn: &BusNow) -> Option<usize> {
        let p = &self.people[i];
        let pays = p.ticket.is_some()
            && !(bn.id == BusId::Player && self.boarding.eq_ignore_ascii_case("walk"));
        let pos = p.position.truncate();
        let allowed: Vec<usize> = (0..bn.cabin.entries.len())
            .filter(|&e| !pays || bn.cabin.entries[e].sells)
            .collect();
        let allowed = if allowed.is_empty() {
            (0..bn.cabin.entries.len()).collect()
        } else {
            allowed
        };
        let dist = |e: usize| (bn.world(bn.cabin.entries[e].outside).truncate() - pos).length();
        // A door still shut counts as some metres farther, the more the longer one has
        // waited at it: an open door not much farther is taken, a far one only once the
        // near door stays shut. (Only the doors open at the moment counted: a bus whose
        // rear doors opened a moment before its front one sent the people waiting at the
        // front to the back.)
        let waited = if matches!(p.state, State::Queue { .. }) { p.t_state.max(0.0) as f64 } else { 0.0 };
        let shut = |e: usize| {
            if bn.entry_open.get(e).copied().unwrap_or(false) {
                0.0
            } else {
                6.0 + 2.0 * waited
            }
        };
        // with both leaves open, spread out: the shorter queue wins at similar distance
        allowed.iter().copied().min_by(|a, b| {
            let qa = self.people.iter().filter(|q| matches!(q.state, State::Queue { bus, entry, .. } if bus == bn.id && entry == *a)).count() as f64;
            let qb = self.people.iter().filter(|q| matches!(q.state, State::Queue { bus, entry, .. } if bus == bn.id && entry == *b)).count() as f64;
            (dist(*a) + qa * 0.8 + shut(*a)).total_cmp(&(dist(*b) + qb * 0.8 + shut(*b)))
        })
    }

    /// The other door leaf a pass holder in place `slot` of the queue at `entry` should move
    /// to: somebody ahead pays at the desk, and the other leaf is open, leads to a place
    /// without passing the desk and has fewer people waiting than are ahead.
    #[allow(clippy::too_many_arguments)]
    fn pass_holder_leaf(
        &self,
        i: usize,
        bn: &BusNow,
        bus: BusId,
        entry: usize,
        slot: usize,
        slot_of: &HashMap<usize, usize>,
    ) -> Option<usize> {
        let p = &self.people[i];
        let walk_in = bus == BusId::Player && self.boarding.eq_ignore_ascii_case("walk");
        if walk_in || p.ticket.is_some() || p.t_state < 1.0 || bn.cabin.desk.is_none() {
            return None;
        }
        let in_queue = |k: usize, e: usize| matches!(self.people[k].state, State::Queue { bus: b, entry: x, .. } if b == bus && x == e);
        let payers_ahead = (0..self.people.len()).any(|k| {
            k != i
                && in_queue(k, entry)
                && self.people[k].ticket.is_some()
                && slot_of.get(&k).map(|s| *s < slot).unwrap_or(false)
        });
        if !payers_ahead {
            return None;
        }
        let taken = self.seats.get(&bus).cloned().unwrap_or_default();
        (0..bn.cabin.entries.len())
            .filter(|&e| e != entry && bn.entry_open.get(e).copied().unwrap_or(false))
            .filter(|&e| {
                let near = bn.cabin.entries[e].inside;
                bn.cabin
                    .choose_seat(&taken, near, |_| 0.5)
                    .map(|seat| !bn.cabin.passes_desk(e, bn.cabin.seats[seat].floor))
                    .unwrap_or(false)
            })
            // (a leaf where people pay as well would only be the same wait)
            .filter(|&e| {
                !(0..self.people.len()).any(|k| in_queue(k, e) && self.people[k].ticket.is_some())
            })
            .map(|e| {
                (
                    e,
                    (0..self.people.len()).filter(|&k| in_queue(k, e)).count(),
                )
            })
            .filter(|&(_, n)| n < slot)
            .min_by_key(|&(_, n)| n)
            .map(|(e, _)| e)
    }

    /// Somebody walking inside a bus got where they were going.
    fn arrived(
        &mut self,
        i: usize,
        bus: BusId,
        bn: &BusNow,
        goal: Goal,
        route: &[Vec3],
        world: &World,
    ) {
        match goal {
            Goal::Stamper(seat, _) => {
                // the ticket into the validator (its sound, `ev_Stamper`, is the bus's), a
                // moment standing there, then on to the seat
                self.stamped.push(bus);
                self.people[i].pause_until = self.time + 1.5;
                let here = route.last().copied().unwrap_or(bn.cabin.seats[seat].floor);
                let mut to_seat = vec![here];
                to_seat.extend(bn.cabin.route(here, bn.cabin.seats[seat].floor));
                if debug_pax() {
                    log::info!("t={:.1} pax {} stamps a ticket in {:?}", self.time, self.people[i].label(), bus);
                }
                self.set_state(i, State::Aboard { bus, route: to_seat, idx: 0, seg: here, goal: Goal::Seat(seat) });
            }
            Goal::Desk(seat) => {
                let ticket = if bus == BusId::Player {
                    self.people[i].ticket
                } else {
                    None
                };
                if debug_pax() && bus == BusId::Player {
                    log::info!(
                        "t={:.1} pax {} at the cash desk wants {:?}",
                        self.time,
                        self.people[i].label(),
                        ticket.and_then(|t| self
                            .tickets
                            .as_ref()
                            .and_then(|p| p.tickets.get(t))
                            .map(|x| x.name.clone()))
                    );
                }
                self.set_state(
                    i,
                    State::AtDesk {
                        bus,
                        seat,
                        ticket,
                        done: false,
                    },
                );
            }
            Goal::Seat(seat) => {
                // a seated passenger stays on the floor in front of the seat: the
                // animation sits them down onto it
                let s = &bn.cabin.seats[seat];
                if debug_pax() {
                    log::info!(
                        "t={:.1} pax {} {} in {:?}{} (gets off at {})",
                        self.time,
                        self.people[i].label(),
                        if s.seated { "sits down" } else { "stands" },
                        bus,
                        bn.cabin.part_label(s.floor),
                        if bus == BusId::Player {
                            format!("timetable stop {}", self.people[i].exit_stop)
                        } else if let Some(x) = self.people[i].exit_id {
                            format!("stop object {x}")
                        } else {
                            format!("{} stops", self.people[i].stops_left)
                        }
                    );
                }
                self.people[i].activity = if s.seated {
                    Activity::Sit
                } else {
                    Activity::Stand
                };
                self.set_state(i, State::Riding { bus, seat });
            }
            Goal::ExitWait(exit) => {
                self.set_state(i, State::AtExit { bus, exit });
            }
            Goal::Exit(exit) => {
                // out on the ground, where they are (the step counts from a little way off)
                let last = route.last().copied().unwrap_or(Vec3::ZERO);
                let local = self.people[i]
                    .local()
                    .filter(|l| (l.truncate() - last.truncate()).length() < 1.0)
                    .map(|l| Vec3::new(l.x, l.y, last.z))
                    .unwrap_or(last);
                let w = bn.world(local);
                // (the height they are at: the ground under them is reached stepping down
                // - put on it at once, they dropped off the step in a frame)
                let z = w.z.max(world.walk_height(w.x, w.y).unwrap_or(w.z));
                let away = bn.right_at(local) * local.x.signum() as f64;
                let along = (self.rand_f() - 0.5) * 3.0;
                let target =
                    DVec3::new(w.x, w.y, z) + (away * 3.0 + bn.fwd_at(local) * along).extend(0.0);
                let p = &mut self.people[i];
                p.place = Place::Ground;
                p.position = DVec3::new(w.x, w.y, z);
                p.heading = bn.heading_at(local) + p.lheading;
                p.vel = away * 0.6;
                p.leaving_here = false;
                p.avoid = Some(bus);
                if debug_pax() {
                    log::info!("t={:.1} pax {} got off {:?} through exit {exit}{} at stop {:?} ({:.1}, {:.1})", self.time, self.people[i].label(), bus, bn.cabin.part_label(local), bn.stop, w.x, w.y);
                }
                self.set_state(
                    i,
                    State::Leaving {
                        target,
                        walk: None,
                        walked: 0.0,
                    },
                );
            }
        }
    }

    /// From the desk to the seat.
    fn leave_desk(&mut self, i: usize, bus: BusId, bn: &BusNow, seat: usize) {
        let from = self.people[i].local().unwrap_or(Vec3::ZERO);
        let to = bn.cabin.seats[seat].floor;
        let route = bn.cabin.route(from, to);
        self.set_state(
            i,
            State::Aboard {
                bus,
                route,
                idx: 0,
                seg: from,
                goal: Goal::Seat(seat),
            },
        );
    }

    /// The player's cash desk: payment, the ticket, the change. True when the passenger
    /// is done and walks on.
    #[allow(clippy::too_many_arguments)]
    fn desk_player(
        &mut self,
        i: usize,
        ticket: Option<usize>,
        done: bool,
        waited: f32,
        player_bus: Option<&VehicleInstance>,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        taken_ticket: &mut bool,
    ) -> bool {
        let Some(t) = ticket else {
            // a pass holder shows it and walks on
            self.people[i].why = "shows a pass";
            if waited > 0.9 {
                self.boarded += 1;
                self.served += 1;
                return true;
            }
            return false;
        };
        let Some(b) = player_bus else { return true };
        let (name, value) = self
            .tickets
            .as_ref()
            .and_then(|p| p.tickets.get(t))
            .map(|tk| (tk.name.clone(), tk.value))
            .unwrap_or_default();
        let auto = self.boarding.eq_ignore_ascii_case("auto");
        if !done {
            if self.request.is_none() {
                self.ticket_requests += 1;
                // "Einmal ..., bitte": the ticket asked for (`Ticket_<n>_<1|2>`, n from 1)
                let k = 1 + self.rand() % 2;
                self.say_ex(i, &format!("Ticket_{}_{k}", t + 1), false);
            }
            self.request = Some((name.clone(), value));
            // the money goes on the desk as the passenger steps up
            if self.paid.is_none() {
                let point = self
                    .player_cabin
                    .as_ref()
                    .and_then(|c| c.data.money_points.first().cloned());
                let mut paid_value = value;
                if let Some(m) = self.money.as_mut() {
                    let coins = if self.exact_fare || auto {
                        m.exact_coins_for(value)
                    } else {
                        m.coins_for(value)
                    };
                    paid_value = m.value_of(&coins);
                    if let Some(pt) = point {
                        m.place(
                            world,
                            renderer,
                            scene,
                            &coins,
                            Vec3::from(pt.pos),
                            pt.var,
                            false,
                        );
                    }
                }
                self.paid = Some((paid_value, value));
            }
            // the ticket key sells the requested ticket (on a bus without a printer); in
            // `auto` boarding the passenger serves themself after a moment
            let mut given = b.var("GivenTicket").unwrap_or(-1.0);
            if self.give_ticket || (auto && waited > 1.8) {
                given = t as f32;
            }
            if given >= 0.0 {
                // the ticket is taken, the driver pockets the money, change may be owed
                let owed = if auto {
                    0.0
                } else {
                    self.paid.map(|(p, v)| (p - v).max(0.0)).unwrap_or(0.0)
                };
                if let Some(m) = self.money.as_mut() {
                    m.clear(false);
                }
                self.paid = None;
                self.change_due = Some(owed);
                self.tickets_sold += 1;
                self.ticket_cash += value;
                *taken_ticket = true;
                if debug_pax() {
                    log::info!("t={:.1} pax {} got ticket {name} ({value:.2}) after {waited:.1} s, change due {owed:.2}", self.time, self.people[i].label());
                }
                if let State::AtDesk { done, .. } = &mut self.people[i].state {
                    *done = true;
                }
                self.people[i].t_state = 0.0;
                return false;
            }
            if waited > PAY_PATIENCE {
                // nobody sold a ticket: the passenger shows a pass after all and walks on
                self.boarded += 1;
                if let Some(m) = self.money.as_mut() {
                    m.clear(false);
                }
                self.paid = None;
                self.request = None;
                self.message = Some(format!("The passenger waited {PAY_PATIENCE:.0} s for a ticket ({}), showed a pass and walked on", self.ticket_key));
                if debug_pax() {
                    log::info!(
                        "t={:.1} pax {} gave up waiting for a ticket, shows a pass",
                        self.time,
                        self.people[i].label()
                    );
                }
                return true;
            }
            self.people[i].why = if auto {
                "pays"
            } else {
                "waits for the driver to sell the ticket"
            };
            return false;
        }
        // ticket in hand: waiting for the change
        let owed = self.change_due.unwrap_or(0.0);
        if std::mem::take(&mut self.give_change_all) {
            let given = self.money.as_ref().map(|m| m.change_value()).unwrap_or(0.0);
            if owed - given > 0.001 {
                let point = self.player_cabin.as_ref().and_then(|c| c.data.change_points.first().or(c.data.money_points.first()).cloned());
                if let (Some(m), Some(pt)) = (self.money.as_mut(), point) {
                    let coins = m.exact_coins_for(owed - given);
                    m.place(world, renderer, scene, &coins, Vec3::from(pt.pos), pt.var, true);
                }
            }
        }
        let given_change = self.money.as_ref().map(|m| m.change_value()).unwrap_or(0.0);
        if owed <= 0.001 || given_change >= owed - 0.001 || waited > 20.0 {
            // no change after all: "Und mein Wechselgeld?"; else thanks now and then
            if owed > 0.001 && given_change < owed - 0.001 {
                self.say_ex(i, "BadChange_1", false);
                self.ticket_points += 1;
            } else {
                self.ticket_points += 2;
                let r = self.rand_f() as f32;
                if self.tickets.as_ref().map(|t| r < t.chattiness).unwrap_or(false) {
                    self.say_ex(i, "Thanks_1", false);
                }
            }
            if let Some(m) = self.money.as_mut() {
                m.clear(true);
            }
            self.change_due = None;
            self.request = None;
            self.boarded += 1;
            self.served += 1;
            return true;
        }
        self.people[i].why = "waits for the change";
        false
    }

    /// Where a walk along the pavement takes somebody next.
    fn walk_want(
        &mut self,
        i: usize,
        walk: &mut PedWalk,
        net: &Network,
        traffic: Option<&Traffic>,
        cars: &[(DVec2, DVec2, f64)],
        dt: f32,
    ) -> Want {
        let mut ped = self.ped.take();
        let w = self.walk_want_with(ped.as_mut(), i, walk, net, traffic, cars, dt);
        self.ped = ped;
        w
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_want_with(
        &mut self,
        mut ped: Option<&mut PedNet>,
        i: usize,
        walk: &mut PedWalk,
        net: &Network,
        traffic: Option<&Traffic>,
        cars: &[(DVec2, DVec2, f64)],
        dt: f32,
    ) -> Want {
        let pos2 = self.people[i].position.truncate();
        let pace = self.people[i].pace;
        if walk.leg >= walk.legs.len() {
            return Want::stand(None, Activity::Stand);
        }
        let leg = walk.legs[walk.leg];
        if walk.s >= leg.len() - 0.35 || walk.held > 0.0 {
            // at the end of the leg: which way on
            if walk.leg + 1 >= walk.legs.len() {
                if !walk.roam {
                    walk.leg += 1;
                    return Want::stand(None, Activity::Stand);
                }
                let pick = self.rand();
                let next = ped
                    .as_ref()
                    .and_then(|p| {
                        p.end_node(net, &leg)
                            .and_then(|n| p.next_leg(net, n, leg.lane, pick))
                    })
                    .unwrap_or(Leg {
                        lane: leg.lane,
                        a: leg.b,
                        b: leg.a,
                    });
                walk.legs.push(next);
                if walk.leg > 6 {
                    walk.legs.drain(..walk.leg);
                    walk.leg = 0;
                }
            }
            let next = walk.legs[walk.leg + 1];
            match self.may_cross(
                ped.as_deref_mut(),
                net,
                &next,
                traffic,
                cars,
                pace,
                walk.held,
            ) {
                Ok(()) => {
                    if walk.held > 0.0 && debug_pax() {
                        let light = net.lanes[next.lane].traffic_light.and_then(|(c, li)| {
                            traffic
                                .and_then(|t| t.light_state(c, li))
                                .map(|(st, left)| {
                                    format!(", light {c}.{li} state {st} for {left:.1} s more")
                                })
                        });
                        log::info!(
                            "t={:.1} pax {} crosses path {} after waiting {:.0} s{}",
                            self.time,
                            self.people[i].label(),
                            next.lane,
                            walk.held,
                            light.unwrap_or_default()
                        );
                    }
                    walk.s = (walk.s - leg.len()).max(0.0);
                    walk.leg += 1;
                    walk.held = 0.0;
                }
                Err(why) => {
                    walk.held += dt;
                    self.people[i].why = why;
                    // a light that stays red (nobody crosses on red any more): a stroller
                    // gives up after three minutes and walks back the way they came
                    if walk.roam && why == "red light" && walk.held > 180.0 {
                        if debug_pax() {
                            log::info!(
                                "t={:.1} pax {} gives up waiting at the red light and turns back",
                                self.time,
                                self.people[i].label()
                            );
                        }
                        walk.legs.truncate(walk.leg + 1);
                        walk.legs.push(Leg {
                            lane: leg.lane,
                            a: leg.b,
                            b: leg.a,
                        });
                        walk.held = 0.0;
                        return Want::stand(None, Activity::Stand);
                    }
                    // at the kerb, facing the way across, spread along it and a step back
                    let (end, _) = leg.at(net, leg.len());
                    let (_, h) = next.at(net, 0.3);
                    let hr = h.to_radians();
                    let (fwd, right) = (
                        DVec2::new(hr.sin(), hr.cos()),
                        DVec2::new(hr.cos(), -hr.sin()),
                    );
                    let id = self.people[i].id;
                    let spread = ((id % 5) as f64 - 2.0) * 0.45;
                    let back = 0.25 + (id % 3) as f64 * 0.45;
                    let spot = end.truncate() + right * spread - fwd * back;
                    return Want {
                        vel: arrive(pos2, spot, pace * 0.6),
                        face: Some(h),
                        give: 0.5,
                        corridor: None,
                        idle: Activity::Stand,
                        follow: false,
                        goal_dist: None,
                    };
                }
            }
        }
        let leg = walk.legs[walk.leg];
        let len = leg.len();
        let (p, h) = leg.at(net, (walk.s + 1.3).min(len));
        let lane = &net.lanes[leg.lane];
        let width = (lane.width as f64).max(1.0);
        let crossing = lane.traffic_light.is_some()
            || ped
                .as_ref()
                .map(|p| {
                    p.crossings
                        .get(&leg.lane)
                        .map(|x| !x.is_empty())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        // keep to the right of the pavement (less so on a crossing) - the left where the
        // traffic drives on the left
        let side = if crossing {
            (walk.side.abs() as f64).min(0.3)
        } else {
            (walk.side.abs() as f64).min(width * 0.5 - 0.3).max(0.0)
        };
        let side = if net.left_hand { -side } else { side };
        let hr = h.to_radians();
        let right = DVec2::new(hr.cos(), -hr.sin());
        let target = p.truncate() + right * side;
        let vel = (target - pos2).normalize_or_zero() * pace;
        // stay on the path: the lane locally, as wide as it is
        let (a, _) = leg.at(net, (walk.s - 2.0).max(0.0));
        let (b, _) = leg.at(net, (walk.s + 2.5).min(len));
        let (m, _) = leg.at(net, (walk.s + 0.25).min(len));
        let bow = crowd::project_on_segment(m.truncate(), a.truncate(), b.truncate())
            .0
            .distance(m.truncate());
        let corridor = ((b - a).truncate().length() > 0.5).then(|| {
            (
                a.truncate() + right * side,
                b.truncate() + right * side,
                (width * 0.5 - side).max(0.35) + bow,
            )
        });
        if self.people[i].why != "queueing behind somebody" {
            self.people[i].why = "";
        }
        let left: f32 = (len - walk.s)
            + walk.legs[walk.leg + 1..]
                .iter()
                .map(|l| l.len())
                .sum::<f32>();
        Want {
            vel,
            face: None,
            give: 1.0,
            corridor,
            idle: Activity::Stand,
            follow: false,
            goal_dist: Some(left as f64),
        }
    }

    /// May a pedestrian at the kerb start along `next`? A pedestrian light must show green,
    /// and the time left to get across - the green and then the clearance until a light of
    /// the carriageway turns green - must do; without a light no car may be about to pass
    /// the crossing. Somebody who has waited very long takes any green (never a red).
    #[allow(clippy::too_many_arguments)]
    fn may_cross(
        &self,
        mut ped: Option<&mut PedNet>,
        net: &Network,
        next: &Leg,
        traffic: Option<&Traffic>,
        cars: &[(DVec2, DVec2, f64)],
        pace: f64,
        held: f32,
    ) -> Result<(), &'static str> {
        if !next.from_end(net) {
            return Ok(());
        }
        let lane = &net.lanes[next.lane];
        let t_cross = next.len() as f64 / pace.max(0.5) + 1.0;
        if let (Some((c, li)), Some(t)) = (lane.traffic_light, traffic) {
            if let Some((state, left)) = t.light_state(c, li) {
                if !omsi_sim::traffic::TrafficLightController::allows_go(state) {
                    return Err("red light");
                }
                if held > 150.0 {
                    return Ok(());
                }
                // A pedestrian green is short (8 s at Grundorf for an 11.6 m crossing that
                // takes 10.7 s): who starts on green crosses in the clearance time after
                // it, until the cars get their green. Only the green alone was counted, so
                // nobody ever started on green and everybody went across on red after
                // 150 s, in front of moving cars.
                let window = pedestrian_window(ped.as_deref_mut(), net, t, next.lane, left);
                if (window as f64) < t_cross {
                    return Err("the green ends before they would be across");
                }
                return Ok(());
            }
        }
        let Some(ped) = ped else { return Ok(()) };
        // somebody who has waited long accepts a shorter gap (down to the time the crossing
        // takes, never less): a steady stream does not hold them for ever, but nobody walks
        // out in front of a car that is about to be there (after 45 s they used to ignore
        // the cars altogether)
        let margin = if held > 45.0 { 0.0 } else if held > 20.0 { 1.0 } else { 2.5 };
        for x in ped.crossings(net, next.lane) {
            for (p, v, half) in cars {
                let rel = *x - *p;
                let dist = rel.length();
                if dist > 90.0 {
                    continue;
                }
                if dist < half + 1.5 {
                    return Err("a vehicle stands on the crossing");
                }
                let speed = v.length();
                if speed < 0.5 {
                    continue;
                }
                let dir = *v / speed;
                let along = rel.dot(dir);
                let lateral = rel.perp_dot(dir).abs();
                if along > -half && lateral < 3.5 && along / speed < t_cross + margin {
                    return Err("waits for a car to pass");
                }
            }
        }
        Ok(())
    }

    /// Take over where the crowd moved person `i`.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        i: usize,
        w: &Walker,
        want: &Want,
        dt: f32,
        world: &World,
        net: Option<&Network>,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
    ) {
        let dt64 = dt as f64;
        let time = self.time;
        let p = &mut self.people[i];
        let speed = w.vel.length();
        // somebody pressed against somebody else for seconds slips past them
        if want.vel.length() > 0.2 && speed < 0.08 {
            p.stuck += dt;
        } else if speed > 0.2 {
            p.stuck = 0.0;
        }
        p.detour = (p.detour - dt).max(0.0);
        if p.ghost > 0.0 {
            p.ghost -= dt;
        } else if p.stuck > 2.5 {
            p.ghost = 1.5;
            p.stuck = 0.0;
            if debug_pax() {
                log::info!(
                    "t={time:.1} pax {} ({}) is stuck and slips past",
                    p.label(),
                    p.state.name()
                );
            }
        }
        p.vel = w.vel;
        match p.place {
            Place::Ground => {
                p.position.x = w.pos.x;
                p.position.y = w.pos.y;
                if let Some(z) = world.walk_height_near(p.position.x, p.position.y, p.position.z) {
                    // up a kerb quickly, down it smoothly (the feet find the kerb themselves);
                    // more than a kerb below the surface is no step but a wrong height (the
                    // pavement's tile came after them): straight onto it
                    // (and whoever stands still simply stands on it: waiting people sank
                    // into a pavement that came after them and rose only when they walked)
                    p.position.z = if z - p.position.z > 0.35 || speed < 0.05 {
                        z
                    } else if z > p.position.z {
                        z.min(p.position.z + 1.5 * dt64)
                    } else {
                        z.max(p.position.z - 2.0 * dt64)
                    };
                }
            }
            Place::Bus(b, l) => {
                let here = Vec3::new(w.pos.x as f32, w.pos.y as f32, l.z);
                let z = match &p.state {
                    State::Aboard {
                        route, idx, seg, ..
                    } if *idx < route.len() => {
                        let tgt = route[*idx];
                        let (_, t) = crowd::project_on_segment(
                            here.truncate().as_dvec2(),
                            seg.truncate().as_dvec2(),
                            tgt.truncate().as_dvec2(),
                        );
                        let want_z = seg.z + (tgt.z - seg.z) * t as f32;
                        // a step is climbed, not floated up
                        l.z + (want_z - l.z).clamp(-1.2 * dt, 1.2 * dt)
                    }
                    // waiting in the line at an exit: onto the floor there (the line may
                    // start while they are still stepping down the last stair)
                    State::AtExit { .. } => {
                        match bus_ix.get(&b).map(|k| &buses[*k]).and_then(|bn| {
                            bn.cabin
                                .floor_at(here.truncate().as_dvec2(), bn.half.x, l.z as f64)
                        }) {
                            Some(f) => l.z + (f as f32 - l.z).clamp(-1.2 * dt, 1.2 * dt),
                            None => l.z,
                        }
                    }
                    _ => l.z,
                };
                let local = Vec3::new(here.x, here.y, z);
                p.place = Place::Bus(b, local);
                if let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) {
                    p.position = bn.world(local);
                    p.interior = bn.interior;
                    p.tilt = bn.tilt_at(local);
                }
            }
        }
        // progress along the pavement
        if let Some(net) = net {
            let pos = p.position;
            match &mut p.state {
                State::Strolling(walk)
                | State::ToStop { walk, .. }
                | State::Leaving {
                    walk: Some(walk), ..
                } => {
                    if let Some(leg) = walk.legs.get(walk.leg) {
                        walk.s = leg.project(net, pos, walk.s).max(walk.s - 0.3);
                    }
                }
                _ => {}
            }
        }
        let walking = if p.activity == Activity::Walk {
            speed > 0.12
        } else {
            speed > 0.3
        };
        let activity = if walking { Activity::Walk } else { want.idle };
        let inside = matches!(p.place, Place::Bus(..));
        let bus_heading = match p.place {
            Place::Bus(b, l) => bus_ix
                .get(&b)
                .map(|k| buses[*k].heading_at(l))
                .unwrap_or(0.0),
            Place::Ground => 0.0,
        };
        let current = if inside { p.lheading } else { p.heading };
        let target = if speed > 0.25 {
            Some(crowd::heading_of(w.vel))
        } else {
            want.face
        };
        // turning eases in and out (a constant rate started and stopped with a jerk): the
        // rate follows the angle still to go, up to the most a walker or a stander turns
        let turned = match target {
            Some(t) => {
                let left = crowd::angle_diff(current, t);
                let max_rate = if walking { 260.0 } else { 140.0 };
                let rate = (left.abs() * 5.0).min(max_rate).max(12.0);
                crowd::turn_towards(current, t, rate, dt64)
            }
            None => current,
        };
        if inside {
            p.lheading = turned;
            p.heading = bus_heading + turned;
        } else {
            p.heading = turned;
        }
        p.activity = activity;
    }

    /// People carried by a bus in their seat.
    fn carry(
        &mut self,
        i: usize,
        dt: f32,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
        face: Option<f64>,
    ) {
        let p = &mut self.people[i];
        let Place::Bus(b, l) = p.place else { return };
        let Some(bn) = bus_ix.get(&b).map(|k| &buses[*k]) else {
            return;
        };
        p.position = bn.world(l);
        p.tilt = bn.tilt_at(l);
        p.vel = DVec2::ZERO;
        if let Some(f) = face {
            p.lheading = crowd::turn_towards(p.lheading, f, 150.0, dt as f64);
        }
        p.heading = bn.heading_at(l) + p.lheading;
        p.interior = bn.interior;
    }

    /// OMSI's `change_take`: the driver takes back the coins lying on the change tray.
    pub fn take_change_tray(&mut self) {
        if let Some(m) = self.money.as_mut() {
            m.clear(true);
        }
    }

    /// Coins the driver handed out (from the host's GiveChangeCoin list) onto the change point.
    pub fn give_change(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        coins: &[usize],
    ) {
        if coins.is_empty() {
            return;
        }
        let point = self.player_cabin.as_ref().and_then(|c| {
            c.data
                .change_points
                .first()
                .or(c.data.money_points.first())
                .cloned()
        });
        if let (Some(m), Some(pt)) = (self.money.as_mut(), point) {
            m.place(
                world,
                renderer,
                scene,
                coins,
                Vec3::from(pt.pos),
                pt.var,
                true,
            );
        }
    }

    pub fn sync_money(&mut self, renderer: &Renderer, scene: &mut Scene, bus: &VehicleInstance) {
        if let Some(m) = self.money.as_mut() {
            m.sync(renderer, scene, bus);
        }
    }

    /// Advance everybody's animation: where they stand and look, whether they sit, pay or
    /// hold on, and how the floor under them moves.
    fn animate(
        &mut self,
        dt: f32,
        world: &World,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
    ) {
        for i in 0..self.people.len() {
            if let Some(pp) = self.people[i].puppet {
                if pp.mode == PuppetMode::Avatar {
                    self.animate_avatar(i, dt, world, buses, bus_ix);
                } else {
                    self.animate_puppet(i, dt, world);
                }
                continue;
            }
            let p = &self.people[i];
            let bn = match p.place {
                Place::Bus(b, _) => bus_ix.get(&b).map(|k| &buses[*k]),
                Place::Ground => None,
            };
            let (frame, origin, heading) = match (p.place, bn) {
                (Place::Bus(b, l), Some(_)) => (b.space(), l.as_dvec3(), p.lheading),
                _ => (0, p.position, p.heading),
            };
            let to_model = |q: DVec3| model_point(origin, heading, q);
            let rig = &p.ty.rig;
            let mut activity = p.activity;
            let mut seat: Option<Vec3> = None;
            let mut look: Option<Vec3> = None;
            let mut reach: Option<Vec3> = None;
            let mut hold = 0.0;
            let mut facing: Option<f64> = None;
            match (&p.state, bn) {
                (State::Riding { seat: k, .. }, Some(bn)) => {
                    if let Some(s) = bn.cabin.seats.get(*k) {
                        if s.seated {
                            seat = Some(to_model(s.pos.as_dvec3()));
                            facing = Some(s.rot as f64);
                        } else if bn.speed.abs() > 0.4 || bn.accel.length() > 0.4 {
                            hold = 1.0;
                        }
                    }
                }
                (State::AtExit { exit, .. }, Some(bn)) => {
                    if let Some(d) = bn.cabin.exits.get(*exit) {
                        look = Some(to_model(d.inside.as_dvec3() + DVec3::Z * 1.1));
                    }
                    if bn.speed.abs() > 0.4 || bn.accel.length() > 0.4 {
                        hold = 1.0;
                    }
                }
                (State::AtDesk { ticket, done, .. }, Some(bn)) => {
                    let driver = bn
                        .cabin
                        .data
                        .driver_positions
                        .first()
                        .map(|d| Vec3::from(d.pos) + Vec3::Z * 0.65);
                    if let Some(d) = driver {
                        look = Some(to_model(d.as_dvec3()));
                    }
                    let top = bn
                        .cabin
                        .data
                        .ticket_sales
                        .first()
                        .map(|t| to_model(Vec3::from(t.pos).as_dvec3()));
                    let t = p.t_state;
                    // the hand waits in front of the body between putting the money down
                    // and taking the ticket
                    let hover = |desk: Vec3| {
                        Vec3::new(
                            rig.shoulder[1].x * 0.9,
                            0.3 * rig.scale,
                            rig.shoulder[1].z - 0.4 * rig.scale,
                        )
                        .lerp(desk, 0.3)
                    };
                    let near_desk = match (p.place, bn.cabin.desk) {
                        (Place::Bus(_, l), Some((stand, _, _))) => (l.truncate() - stand.truncate()).length() < 0.8,
                        _ => false,
                    };
                    reach = match (bn.id, ticket, *done, top) {
                        _ if !near_desk => None,
                        (_, _, _, None) => None,
                        (BusId::Ai(_), _, _, Some(d)) => (0.1..1.1).contains(&t).then_some(d),
                        (BusId::Player, None, _, _) => match driver {
                            // a pass is held up towards the driver
                            Some(dr) if (0.1..1.0).contains(&t) => Some(
                                Vec3::new(0.1, 0.45 * rig.scale, rig.shoulder[1].z - 0.12)
                                    .lerp(to_model(dr.as_dvec3()), 0.25),
                            ),
                            _ => None,
                        },
                        (BusId::Player, Some(_), false, Some(d)) => {
                            Some(if (0.25..1.9).contains(&t) {
                                d
                            } else {
                                hover(d)
                            })
                        }
                        (BusId::Player, Some(_), true, Some(d)) => {
                            Some(if t < 1.1 { d } else { hover(d) })
                        }
                    };
                }
                (State::Queue { bus, entry, .. }, _) => {
                    if let Some(bq) = bus_ix.get(bus).map(|k| &buses[*k]) {
                        if let Some(d) = bq.cabin.entries.get(*entry) {
                            look = Some(to_model(bq.world(d.inside) + DVec3::Z * 1.2));
                        }
                    }
                }
                (State::Waiting { stop, spot, .. }, _) => {
                    if let Some(sp) = self.stops.get(stop).and_then(|s| s.spots.get(*spot)) {
                        if sp.seat > 0.0 {
                            // the hip never below the pavement under the feet (a bench
                            // standing on the bare terrain under a raised pavement had
                            // them sit in the ground)
                            let hip = DVec3::new(
                                sp.pos.x,
                                sp.pos.y,
                                sp.pos.z.max(origin.z + sp.seat as f64 * 0.8),
                            );
                            seat = Some(to_model(hip));
                            facing = Some(sp.face);
                        }
                    }
                    // A bus coming in is watched, and one standing there by the people it
                    // takes - each on their own: some never look up, the rest turn to it
                    // after a moment of their own. (Everybody at the stop looked at the
                    // front door of whatever bus came near and kept looking: the whole stop
                    // stared at the driver at once, and at a bus that was not theirs.)
                    let h = (p.id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    let watcher = h % 5 != 0;
                    let near = buses
                        .iter()
                        .filter(|b| b.speed.abs() < 14.0)
                        .map(|b| (b, (b.pos - p.position).length()))
                        .filter(|(_, d)| *d < 45.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    if let (Some((b, d)), true) = (near, watcher) {
                        let coming = b.speed.abs() > 0.5 && d > 8.0 + ((h >> 8) % 12) as f64;
                        let theirs = b.stop == Some(*stop) && self.goes_their_way(i, *stop, b);
                        if coming || theirs {
                            // (where on the bus: its door, or somewhere along its front half)
                            let aim = b
                                .cabin
                                .entries
                                .first()
                                .map(|e| b.world(e.inside))
                                .unwrap_or(b.pos)
                                + DVec3::new(((h >> 16) % 100) as f64 / 50.0 - 1.0, ((h >> 24) % 100) as f64 / 50.0 - 1.0, 0.0);
                            look = Some(to_model(aim + DVec3::Z * 1.3));
                        }
                    }
                }
                _ => {}
            }
            if activity == Activity::Sit {
                // turn round first, then sit down
                let aligned = facing
                    .map(|f| crowd::angle_diff(heading, f).abs() < 30.0)
                    .unwrap_or(true);
                if seat.is_none() || (!aligned && p.anim.sit_amount() < 0.3) {
                    activity = Activity::Stand;
                    seat = None;
                }
            }
            let sway = match bn {
                Some(b) => model_point(DVec3::ZERO, heading, b.accel.extend(0.0)),
                None => Vec3::ZERO,
            };
            let level = origin.z;
            // (beside the bus - a foot still on the pavement at the door - the floor is the
            // ground there: taken as the bus frame's z = 0, the road under the bus, a foot
            // on the kerb sank 10-15 cm into the paving stones while stepping in or out)
            let bus_floor = bn.map(|b| {
                move |at: DVec2| {
                    if at.x.abs() > b.half.x - 0.05 {
                        let base = b.world(Vec3::new(at.x as f32, at.y as f32, 0.0));
                        return match world.walk_height(base.x, base.y) {
                            Some(g) => Some(g - base.z),
                            None => Some(0.0),
                        };
                    }
                    b.cabin.floor_at(at, b.half.x, level)
                }
            });
            // (the floor at the feet' own height: a shelter's roof over them is no floor)
            let ground_floor = move |at: DVec2| world.walk_height_near(at.x, at.y, level);
            let floor: &dyn Fn(DVec2) -> Option<f64> = match &bus_floor {
                Some(f) => f,
                None => &ground_floor,
            };
            let input = PoseInput {
                activity,
                origin,
                heading,
                frame,
                velocity: p.vel,
                seat,
                look,
                reach,
                grips: None,
                grip_frames: None,
                grip_lean: 0.0,
                hold,
                sway: sway.truncate(),
                floor: Some(floor),
            };
            let world_of = |l: Vec3| bn.map(|b| b.world(l));
            let p = &mut self.people[i];
            p.anim.advance(&p.ty.rig, &input, dt);
            // a foot went down: one footstep sound where the person stands (the gait only
            // plants a foot while walking, so people standing at a stop stay quiet)
            let step = if p.anim.landed() && p.vel.length() > 0.3 {
                match p.place {
                    Place::Ground => Some((p.position, false, false)),
                    Place::Bus(b, l) => world_of(l).map(|w| (w, true, b == BusId::Player)),
                }
            } else {
                None
            };
            if let Some((position, inside, own_bus)) = step {
                self.footfalls.push(ambience::Footfall { position, inside, own_bus });
            }
            let log_it = match debug_pose() {
                Some(Some(id)) => id == p.id,
                Some(None) => {
                    (self.time % 2.0) < dt as f64
                        && self
                            .eye
                            .map(|e| (p.position - e.pos).length() < 15.0)
                            .unwrap_or(false)
                }
                None => false,
            };
            if log_it {
                log::info!(
                    "t={:.1} anim {} {} {:?} seat {:?}: {}",
                    self.time,
                    p.label(),
                    p.state.name(),
                    activity,
                    seat.map(|s| (s * 100.0).round() / 100.0),
                    p.anim.describe()
                );
            }
        }
    }

    /// Skin the people due for a new pose and push transforms to the renderer. Near people
    /// are posed every frame, far ones every few frames and people out of view rarely; the
    /// posing and skinning run in parallel.
    pub fn sync(&mut self, renderer: &Renderer, scene: &mut Scene, camera: DVec3) {
        for inst in self.hidden.drain(..) {
            renderer.set_params(scene, inst, &[], false, &[]);
        }
        let started = std::time::Instant::now();
        self.sync_frame = self.sync_frame.wrapping_add(1);
        let eye = self.eye;
        let from = eye.map(|e| e.pos).unwrap_or(camera);
        // synced only now and then (offscreen snapshots): everybody is posed afresh
        let all = self.time - self.last_sync > 0.12;
        let sdt = (self.time - self.last_sync).clamp(0.0, 0.5) as f32;
        self.last_sync = self.time;
        let mut due: Vec<bool> = Vec::with_capacity(self.people.len());
        for (k, p) in self.people.iter_mut().enumerate() {
            p.since_posed = p.since_posed.saturating_add(1);
            let d = p.position + DVec3::Z * 0.9 - from;
            let dist = d.length();
            let visible = match eye {
                Some(e) => dist < 4.0 || d.dot(e.fwd) / dist.max(1e-3) > e.cos_half - 0.15,
                None => true,
            };
            // everybody the eye can make out is posed every frame: a pose every other
            // frame at 12-30 m moved walkers in steps and made planted feet shiver
            // (within 30 m everybody, seen or not: the mirrors show the people behind the
            // bus, who were posed every twelfth frame and moved in jerks there)
            let every = if dist < 30.0 {
                1
            } else if !visible {
                12
            } else if dist < 45.0 {
                1
            } else if dist < 90.0 {
                2
            } else if dist < 160.0 {
                3
            } else {
                6
            };
            let every = if p.anim.calm() && dist > 20.0 { every * 2 } else { every };
            // spread the far ones over the frames
            let turn = (self.sync_frame + k as u32) % every == 0;
            due.push(
                !p.skinned
                    || all
                    || (p.since_posed >= every && (turn || p.since_posed >= 2 * every)),
            );
        }
        let n_due = due.iter().filter(|d| **d).count();
        let pose_one = |p: &mut Person| {
            let Person {
                anim, ty, skins, ankles, ..
            } = p;
            let posed = anim.bones(&ty.rig);
            if !posed.ok && !skins.is_empty() {
                // keep the last good mesh (the rest pose would be the file's T-pose)
                return;
            }
            *ankles = posed.ankle;
            skins.resize_with(ty.meshes.len(), Default::default);
            for (k, m) in ty.meshes.iter().enumerate() {
                let (pos, nrm) = &mut skins[k];
                skin(m, &posed.bones, pos, nrm);
            }
        };
        // a handful is quicker on this thread than handed to the pool
        if n_due >= 8 {
            self.people
                .par_iter_mut()
                .zip(due.par_iter())
                .with_min_len(2)
                .filter(|(_, go)| **go)
                .for_each(|(p, _)| pose_one(p));
        } else {
            self.people
                .iter_mut()
                .zip(&due)
                .filter(|(_, go)| **go)
                .for_each(|(p, _)| pose_one(p));
        }
        let upload = std::time::Instant::now();
        for (p, &go) in self.people.iter_mut().zip(&due) {
            if go {
                for (k, (id, _)) in p.meshes.iter().enumerate() {
                    if let Some((pos, nrm)) = p.skins.get(k) {
                        renderer.update_mesh(scene, *id, pos, nrm, &p.ty.meshes[k].data.uvs);
                    }
                }
                p.skinned = true;
                p.since_posed = 0;
                p.posed_at = (p.position, p.heading);
            }
            // riders go with their bus; on the ground a mesh not posed this frame goes on
            // with the body too (left where it was posed, a far walker moved in jerks -
            // its feet slide a few centimetres instead, which nobody sees at that distance)
            let (at, heading) = match (p.puppet, p.place) {
                (Some(pp), _) if pp.mode == PuppetMode::Treadmill => (pp.base, pp.heading),
                (_, Place::Ground) if go => p.posed_at,
                _ => (p.position, p.heading),
            };
            // (riders with the tilt of their floor)
            let tilt = if matches!(p.place, Place::Bus(..)) { p.tilt } else { Mat4::IDENTITY };
            let xf = tilt * Mat4::from_rotation_z((-heading).to_radians() as f32);
            let lit_to = if matches!(p.place, Place::Bus(..)) { p.interior } else { 0.0 };
            p.lit += (lit_to - p.lit) * (sdt / 0.4).min(1.0);
            for (_, inst) in &p.meshes {
                renderer.set_transform(scene, *inst, at, xf);
                renderer.set_interior(scene, *inst, p.lit * 0.5);
            }
            if self.avatar_hidden.contains_key(&p.id) && omsi_cfg::env::var_os("OMSI_DEBUG_FOOT").is_some() && self.sync_frame % 30 == 0 {
                log::info!("avatar drawn at ({:.2}, {:.2}, {:.2}) heading {:.0} place {:?} go {}", at.x, at.y, at.z, heading, matches!(p.place, Place::Ground), go);
            }
            if let Some(hide) = self.avatar_hidden.get_mut(&p.id) {
                // (the first-person view: the avatar's own body out of the picture; set
                // every frame, the posing would show it again)
                for (_, inst) in &p.meshes {
                    renderer.set_params(scene, *inst, &[], !*hide, &[]);
                }
            }
            if let Some(t) = self.trace.as_mut() {
                // OMSI_TRACE_PAX: where the mesh is drawn and where its ankles are, per frame
                if (at - from).length() < 40.0 {
                    use std::io::Write;
                    let a = |k: usize| at + (xf.transform_vector3(p.ankles[k])).as_dvec3();
                    let (l, r) = (a(0), a(1));
                    let _ = writeln!(
                        t,
                        "{:.4},{},{},{},{},{:.4},{:.4},{:.4},{:.2},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.3}",
                        self.time,
                        p.id,
                        p.state.name(),
                        matches!(p.place, Place::Ground) as u8,
                        go as u8,
                        at.x,
                        at.y,
                        at.z,
                        heading,
                        l.x,
                        l.y,
                        l.z,
                        r.x,
                        r.y,
                        r.z,
                        p.vel.x,
                        p.vel.y
                    );
                }
            }
        }
        self.pose_stats.0 += 1;
        self.pose_stats.1 += n_due;
        self.pose_stats.2 += started.elapsed().as_secs_f64() * 1000.0;
        self.pose_stats.3 += upload.elapsed().as_secs_f64() * 1000.0;
    }

    /// `OMSI_PAX_GALLERY=x,y[,heading]`: one scripted person of every human type in a row
    /// to the right of that point, each doing what `OMSI_PAX_GALLERY_MODE` says (a list,
    /// taken in turn): `treadmill` (walks on the spot), `walk`, `sit`, `pay`, `hold`,
    /// `look`, `step`, `turn` or `idle`. For checking the animation up close.
    fn make_gallery(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene) {
        let Some(spec) = omsi_cfg::env::var("OMSI_PAX_GALLERY").ok() else {
            return;
        };
        let v: Vec<f64> = spec
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        if v.len() < 2 {
            return;
        }
        let heading = v.get(2).copied().unwrap_or(0.0);
        let modes: Vec<PuppetMode> = omsi_cfg::env::var("OMSI_PAX_GALLERY_MODE")
            .unwrap_or_else(|_| "treadmill".into())
            .split(',')
            .filter_map(PuppetMode::parse)
            .collect();
        let modes = if modes.is_empty() {
            vec![PuppetMode::Treadmill]
        } else {
            modes
        };
        // OMSI_PAX_GALLERY_TYPES: how many (the first n), or which ("3,7,12")
        let kinds: Vec<usize> = match omsi_cfg::env::var("OMSI_PAX_GALLERY_TYPES") {
            Ok(x) if x.contains(',') => {
                x.split(',').filter_map(|k| k.trim().parse().ok()).collect()
            }
            Ok(x) => (0..x
                .trim()
                .parse::<usize>()
                .unwrap_or(self.types.len())
                .min(self.types.len()))
                .collect(),
            Err(_) => (0..self.types.len()).collect(),
        };
        let h = heading.to_radians();
        let right = DVec2::new(h.cos(), -h.sin());
        for (k, &kind) in kinds.iter().enumerate() {
            let xy = DVec2::new(v[0], v[1]) + right * (k as f64 * 1.1);
            let z = world.walk_height(xy.x, xy.y).unwrap_or(0.0);
            let base = DVec3::new(xy.x, xy.y, z);
            let state = State::Leaving {
                target: base,
                walk: None,
                walked: 0.0,
            };
            if let Some(i) = self.spawn_as(world, renderer, scene, base, heading, state, Some(kind))
            {
                let mode = modes[k % modes.len()];
                self.people[i].puppet = Some(Puppet {
                    mode,
                    base,
                    heading,
                    virt: base,
                    t: 0.0,
                });
                log::info!(
                    "gallery: {} {:?} at ({:.2}, {:.2}, {:.2})",
                    self.people[i].label(),
                    mode,
                    base.x,
                    base.y,
                    base.z
                );
            }
        }
    }

    fn animate_puppet(&mut self, i: usize, dt: f32, world: &World) {
        let Some(mut pp) = self.people[i].puppet else {
            return;
        };
        pp.t += dt;
        let t = pp.t;
        let rig = self.people[i].ty.rig.clone();
        let pace = rig.walk_speed.min(1.4) as f64;
        let ramp = |a: f32, b: f32| ((t - a) / (b - a)).clamp(0.0, 1.0) as f64;
        let mut activity = Activity::Stand;
        let (mut seat, mut look, mut reach, mut hold, mut sway) =
            (None, None, None, 0.0, Vec2::ZERO);
        let mut step_at: Option<f64> = None;
        let speed = match pp.mode {
            PuppetMode::Treadmill => pace * (ramp(1.0, 2.0) - ramp(9.0, 10.0)),
            PuppetMode::Walk => {
                if t > 8.0 && t < 9.5 {
                    pp.heading += 120.0 * dt as f64;
                }
                pace * (ramp(1.0, 2.0) - ramp(6.5, 7.5) + ramp(9.5, 10.5) - ramp(15.0, 16.0))
            }
            PuppetMode::Step => {
                step_at = Some(1.6);
                0.9 * (ramp(1.0, 1.6) - ramp(6.0, 6.6))
            }
            PuppetMode::Sit => {
                if (1.0..6.0).contains(&t) || t > 9.0 {
                    activity = Activity::Sit;
                    seat = Some(Vec3::new(0.0, -SEAT_FRONT, 0.45));
                }
                0.0
            }
            PuppetMode::Pay => {
                if (1.0..6.0).contains(&t) {
                    activity = Activity::Pay;
                    reach = Some(if (1.0..3.0).contains(&t) {
                        Vec3::new(0.02, 0.5 * rig.scale, rig.shoulder[1].z - 0.35)
                    } else {
                        Vec3::new(0.16, 0.3, rig.shoulder[1].z - 0.45)
                    });
                    look = Some(Vec3::new(-0.9, 1.0, 1.45));
                }
                0.0
            }
            PuppetMode::Hold => {
                hold = if t > 1.0 { 1.0 } else { 0.0 };
                sway = if (3.0..4.0).contains(&t) {
                    Vec2::new(0.0, -2.5)
                } else if (6.0..7.0).contains(&t) {
                    Vec2::new(0.0, 1.8)
                } else if (9.0..10.0).contains(&t) {
                    Vec2::new(1.5, 0.0)
                } else {
                    Vec2::ZERO
                };
                0.0
            }
            PuppetMode::Look => {
                let a = (t * 40.0).to_radians();
                look = Some(Vec3::new(
                    2.0 * a.sin(),
                    2.0 * a.cos(),
                    1.5 + 0.8 * (t * 0.7).sin(),
                ));
                0.0
            }
            PuppetMode::Turn => {
                if (t % 4.0) > 2.0 && (t % 4.0) < 2.75 {
                    pp.heading += 120.0 * dt as f64;
                }
                0.0
            }
            PuppetMode::Idle | PuppetMode::Avatar => 0.0,
        };
        let h = pp.heading.to_radians();
        let vel = DVec2::new(h.sin(), h.cos()) * speed;
        pp.virt += (vel * dt as f64).extend(0.0);
        let base = pp.base;
        let ground = move |at: DVec2| -> Option<f64> {
            match step_at {
                // a 0.3 m step up this far ahead of the start
                Some(d) => Some(
                    base.z
                        + if (at - base.truncate()).dot(DVec2::new(h.sin(), h.cos())) > d {
                            0.3
                        } else {
                            0.0
                        },
                ),
                None => world.walk_height(at.x, at.y),
            }
        };
        let mut origin = pp.virt;
        match (pp.mode, step_at) {
            (_, Some(d)) => {
                let along = (origin - base).truncate().length();
                let up = if along > d + 0.1 {
                    0.3
                } else if along > d - 0.2 {
                    0.3 * ((along - (d - 0.2)) / 0.3)
                } else {
                    0.0
                };
                origin.z = base.z + up;
            }
            (PuppetMode::Treadmill, _) => origin.z = base.z,
            _ => origin.z = world.walk_height(origin.x, origin.y).unwrap_or(base.z),
        }
        let input = PoseInput {
            activity,
            origin,
            heading: pp.heading,
            frame: 0,
            velocity: vel,
            seat,
            look,
            reach,
            grips: None,
            grip_frames: None,
            grip_lean: 0.0,
            hold,
            sway,
            floor: Some(&ground),
        };
        let p = &mut self.people[i];
        p.anim.advance(&rig, &input, dt);
        p.activity = activity;
        if pp.mode != PuppetMode::Treadmill {
            p.position = origin;
        }
        p.heading = pp.heading;
        p.vel = vel;
        p.puppet = Some(pp);
    }
}

/// A point of a floor frame in the model frame of somebody standing at `origin` facing
/// `heading` (degrees).
fn model_point(origin: DVec3, heading: f64, q: DVec3) -> Vec3 {
    let d = q - origin;
    let h = heading.to_radians();
    let (s, c) = (h.sin(), h.cos());
    Vec3::new(
        (d.x * c - d.y * s) as f32,
        (d.x * s + d.y * c) as f32,
        d.z as f32,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PuppetMode {
    /// The player on foot (or another player's walker): moved by the game, see `avatar`.
    Avatar,
    Treadmill,
    Walk,
    Sit,
    Pay,
    Hold,
    Look,
    Step,
    Turn,
    Idle,
}

impl PuppetMode {
    fn parse(s: &str) -> Option<PuppetMode> {
        Some(match s.trim() {
            "treadmill" => PuppetMode::Treadmill,
            "walk" => PuppetMode::Walk,
            "sit" => PuppetMode::Sit,
            "pay" => PuppetMode::Pay,
            "hold" => PuppetMode::Hold,
            "look" => PuppetMode::Look,
            "step" => PuppetMode::Step,
            "turn" => PuppetMode::Turn,
            "idle" => PuppetMode::Idle,
            _ => return None,
        })
    }
}

/// A scripted test person: where it started, which way it faces, where it would be on a
/// treadmill, and the seconds of its script.
#[derive(Debug, Clone, Copy)]
struct Puppet {
    mode: PuppetMode,
    base: DVec3,
    heading: f64,
    virt: DVec3,
    t: f32,
}

// ---------------------------------------------------------------------------------------
// Avatars: the player got up from the seat (`on_foot`), or another player walks about. The
// game moves them; the people's animation poses them - the gait and its feet on the
// ground, sitting down on a seat and getting up - so every change is eased, never a jump.

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

impl Humans {
    /// Put avatar `key` where `cmd` says (made on its first call, of figure `kind`).
    pub fn avatar(&mut self, key: u32, world: &World, renderer: &Renderer, scene: &mut Scene, cmd: AvatarCmd, kind: u64) {
        let known = self.avatars.get(&key).copied().filter(|id| self.people.iter().any(|p| p.id == *id));
        if known.is_none() {
            let state = State::Leaving { target: cmd.pos, walk: None, walked: 0.0 };
            let n = self.types.len().max(1) as u64;
            let Some(i) = self.spawn_as(world, renderer, scene, cmd.pos, cmd.heading, state, Some((kind % n) as usize)) else { return };
            self.people[i].puppet = Some(Puppet { mode: PuppetMode::Avatar, base: cmd.pos, heading: cmd.heading, virt: cmd.pos, t: 0.0 });
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
            let d = (want - q).length();
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
        let from = local.truncate();
        let solid = bn.cabin.seats.iter().filter(|s| s.seated).map(|s| s.pos.truncate()).chain(bn.cabin.data.driver_positions.iter().map(|d| glam::Vec2::new(d.pos[0], d.pos[1])));
        for c in solid {
            let (dn, d0) = ((xy - c).length(), (from - c).length());
            if dn < 0.38 && dn < d0 {
                return Some((local, bn.world(local)));
            }
        }
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
        let mut v: Vec<(BusId, f64)> = self.last_buses.iter().map(|b| (b.id, (b.pos - at).truncate().length())).filter(|x| x.1 < r).collect();
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

    fn animate_avatar(&mut self, i: usize, dt: f32, world: &World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let id = self.people[i].id;
        let Some(key) = self.avatars.iter().find(|(_, v)| **v == id).map(|(k, _)| *k) else { return };
        let Some(cmd) = self.avatar_cmds.get(&key).copied() else { return };
        let rig = self.people[i].ty.rig.clone();
        let seated = cmd.seat.and_then(|(b, k)| {
            let bn = bus_ix.get(&b).map(|x| &buses[*x])?;
            let s = bn.cabin.seats.get(k)?.clone();
            Some((b, s, bn))
        });
        match seated {
            Some((b, s, bn)) => {
                // in the seat's bus, in its frame: sitting down there (turned to the seat
                // first, then down), carried by the bus as a passenger is
                let p = &mut self.people[i];
                let l = s.floor;
                if !matches!(p.place, Place::Bus(pb, _) if pb == b) {
                    p.lheading = wrap_heading(p.heading - bn.heading_at(l));
                }
                p.place = Place::Bus(b, l);
                p.lheading = crowd::turn_towards(p.lheading, s.rot as f64, 220.0, dt as f64);
                p.position = bn.world(l);
                p.tilt = bn.tilt_at(l);
                p.heading = bn.heading_at(l) + p.lheading;
                p.interior = bn.interior;
                p.vel = DVec2::ZERO;
                let origin = l.as_dvec3();
                let heading = p.lheading;
                let aligned = crowd::angle_diff(heading, s.rot as f64).abs() < 30.0;
                let sitting = aligned || p.anim.sit_amount() > 0.3;
                let to_model = |q: DVec3| model_point(origin, heading, q);
                let level = origin.z;
                let floor = move |at: DVec2| bn.cabin.floor_at(at, bn.half.x, level);
                let sway = model_point(DVec3::ZERO, heading, bn.accel.extend(0.0));
                let input = PoseInput {
                    activity: if sitting { Activity::Sit } else { Activity::Stand },
                    origin,
                    heading,
                    frame: b.space(),
                    velocity: DVec2::ZERO,
                    seat: sitting.then(|| to_model(s.pos.as_dvec3())),
                    look: None,
                    reach: None,
                    grips: None,
                    grip_frames: None,
                    grip_lean: 0.0,
                    hold: 0.0,
                    sway: sway.truncate(),
                    floor: Some(&floor),
                };
                p.anim.advance(&rig, &input, dt);
                p.activity = input.activity;
            }
            None if cmd.aboard.is_some_and(|(b, _)| bus_ix.contains_key(&b)) => {
                let (b, l) = cmd.aboard.unwrap();
                let bn = &buses[bus_ix[&b]];
                let p = &mut self.people[i];
                let bh = bn.heading_at(l);
                p.place = Place::Bus(b, l);
                p.lheading = wrap_heading(cmd.heading - bh);
                p.position = bn.world(l);
                p.tilt = bn.tilt_at(l);
                p.heading = cmd.heading;
                p.interior = bn.interior;
                // the walk in the bus frame (x to the right, y forwards)
                let r = bh.to_radians();
                let vel = DVec2::new(cmd.vel.dot(DVec2::new(r.cos(), -r.sin())), cmd.vel.dot(DVec2::new(r.sin(), r.cos())));
                p.vel = cmd.vel;
                let origin = l.as_dvec3();
                let level = origin.z;
                let floor = move |at: DVec2| bn.cabin.floor_at(at, bn.half.x, level);
                let sway = model_point(DVec3::ZERO, p.lheading, bn.accel.extend(0.0));
                let input = PoseInput {
                    activity: if vel.length() > 0.05 { Activity::Walk } else { Activity::Stand },
                    origin,
                    heading: p.lheading,
                    frame: b.space(),
                    velocity: vel,
                    seat: None,
                    look: None,
                    reach: None,
                    grips: None,
                    grip_frames: None,
                    grip_lean: 0.0,
                    hold: 0.0,
                    sway: sway.truncate(),
                    floor: Some(&floor),
                };
                p.anim.advance(&rig, &input, dt);
                p.activity = input.activity;
            }
            None => {
                let p = &mut self.people[i];
                p.place = Place::Ground;
                p.tilt = Mat4::IDENTITY;
                p.interior = 0.0;
                let ground = cmd.floor.unwrap_or_else(|| world.walk_height(cmd.pos.x, cmd.pos.y).unwrap_or(cmd.pos.z));
                let origin = DVec3::new(cmd.pos.x, cmd.pos.y, if cmd.floor.is_some() { ground } else { cmd.pos.z.max(ground) } + cmd.lift.max(0.0));
                let airborne = cmd.lift > 0.03 || cmd.floor.is_some();
                // (in the air the feet go with the body: the floor under them is where they are)
                let air_floor = origin.z;
                let floor_air = move |_: DVec2| Some(air_floor);
                let level = origin.z;
                let floor_ground = move |at: DVec2| world.walk_height_near(at.x, at.y, level);
                let floor: &dyn Fn(DVec2) -> Option<f64> = if airborne { &floor_air } else { &floor_ground };
                let speed = cmd.vel.length();
                let input = PoseInput {
                    activity: if speed > 0.05 { Activity::Walk } else { Activity::Stand },
                    origin,
                    heading: cmd.heading,
                    frame: 0,
                    velocity: cmd.vel,
                    seat: None,
                    look: None,
                    reach: None,
                    grips: None,
                    grip_frames: None,
                    grip_lean: 0.0,
                    hold: 0.0,
                    sway: Vec2::ZERO,
                    floor: Some(floor),
                };
                p.anim.advance(&rig, &input, dt);
                p.activity = input.activity;
                p.position = origin;
                p.heading = cmd.heading;
                p.vel = cmd.vel;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// LAN play (see `lan_world`): a host keeps people around every player, tells the clients
// where they are and hands the waiting ones over to a client's bus; a client draws the
// host's people instead of its own and simulates only those who board its bus.

/// Where one of the host's people is this frame, as a client draws them.
#[derive(Debug, Clone, Copy)]
pub struct MirrorPose {
    pub pos: DVec3,
    pub heading: f64,
    pub vel: DVec2,
    pub activity: Activity,
    /// Aboard a timetable bus: (its id, the point of its frame, heading in its frame, the
    /// seat or standing place, if known).
    pub aboard: Option<(u64, Vec3, f64, Option<usize>)>,
    /// Waiting at a stop: (the stop object, the waiting place).
    pub waiting: Option<(i64, usize)>,
}

/// The bus id (`BusId::Ai`) another LAN player's bus has among the buses here: far above
/// the traffic's car ids.
pub fn remote_bus_id(player: u32) -> u64 {
    (1 << 40) | player as u64
}

/// The bus id (`BusId::Ai`) of a vehicle the player placed (`Player::uid`).
pub fn placed_bus_id(uid: u64) -> u64 {
    (2 << 40) | uid
}

/// The player whose bus `remote_bus_id` gave this id (None for a traffic bus).
pub fn remote_bus_player(bus: u64) -> Option<u32> {
    (bus >> 40 == 1).then_some((bus & 0xFFFF_FFFF) as u32)
}

/// One of the host's people as it tells the clients.
pub struct LanPerson {
    pub id: u32,
    pub ty: Arc<HumanType>,
    pub pos: DVec3,
    pub heading: f64,
    pub speed: f64,
    pub activity: Activity,
    pub aboard: Option<(u64, Vec3, f64, Option<usize>)>,
    pub waiting: Option<(i64, usize)>,
}

impl Humans {
    /// Is `p` further than `r` from us and from every other LAN player?
    fn far_from_players(&self, p: DVec3, r: f64) -> bool {
        (p - self.center).length() > r && self.lan_centers.iter().all(|c| (p - *c).length() > r)
    }

    /// The stops and pavements around the other players of a LAN session (host).
    fn populate_lan_centers(
        &mut self,
        world: &World,
        net: &Network,
        renderer: &Renderer,
        scene: &mut Scene,
    ) {
        if self.lan_centers.is_empty() {
            return;
        }
        let mine = self.center;
        for c in self.lan_centers.clone() {
            if (c - mine).length() < 150.0 {
                continue;
            }
            self.populate_with(world, Some(net), renderer, scene, c);
            self.populate_on_foot(world, net, renderer, scene, 1.0);
        }
        self.center = mine;
    }

    /// Everybody within `radius` of `near` the clients may see (host): on foot, waiting at
    /// a stop, or aboard a timetable bus - not the riders of our own bus, which the others
    /// see from outside only.
    pub fn lan_people(&self, near: DVec3, radius: f64) -> Vec<LanPerson> {
        let r2 = radius * radius;
        self.people
            .iter()
            .filter(|p| p.puppet.is_none() && !p.remote)
            .filter(|p| (p.position - near).length_squared() < r2)
            .filter_map(|p| {
                let aboard = match p.place {
                    Place::Bus(BusId::Player, _) => return None,
                    Place::Bus(BusId::Ai(bus), l) => Some((
                        bus,
                        l,
                        p.lheading,
                        match p.state {
                            State::Riding { seat, .. } => Some(seat),
                            _ => None,
                        },
                    )),
                    Place::Ground => None,
                };
                let waiting = match p.state {
                    State::Waiting { stop, spot, .. } if aboard.is_none() => Some((stop, spot)),
                    _ => None,
                };
                Some(LanPerson {
                    id: p.id,
                    ty: p.ty.clone(),
                    pos: p.position,
                    heading: p.heading,
                    speed: if aboard.is_some() { 0.0 } else { p.vel.length() },
                    activity: p.activity,
                    aboard,
                    waiting,
                })
            })
            .collect()
    }

    /// A client's bus takes these waiting people (host): those still waiting leave our
    /// world (they are the client's now); returns them. Somebody who has meanwhile walked
    /// up to another bus stays ours.
    pub fn hand_over(&mut self, ids: &[u32]) -> Vec<u32> {
        let mut out = Vec::new();
        for id in ids {
            let Some(i) = self.people.iter().position(|p| p.id == *id) else {
                continue;
            };
            if !matches!(self.people[i].state, State::Waiting { .. }) || self.people[i].remote {
                continue;
            }
            self.release(i);
            let p = self.people.swap_remove(i);
            self.retire(&p);
            out.push(*id);
        }
        out
    }

    /// Draw the host's people from now on (`on`), or simulate our own again. Everybody who
    /// is not getting on, riding or getting off our bus goes (the host's come instead; the
    /// host's copies cannot walk on by themselves).
    pub fn set_mirror(&mut self, on: bool) {
        if self.mirror == on {
            return;
        }
        self.mirror = on;
        let keep = |p: &Person| !p.remote && p.state.bus() == Some(BusId::Player);
        let mut i = 0;
        while i < self.people.len() {
            if keep(&self.people[i]) || self.people[i].puppet.is_some() {
                i += 1;
                continue;
            }
            self.release(i);
            let p = self.people.swap_remove(i);
            self.retire(&p);
        }
        // our own people from now on are numbered far above the host's (those riding with
        // us already too)
        if on {
            self.next_id = self.next_id.max(1 << 30);
            for p in self.people.iter_mut().filter(|p| p.puppet.is_none()) {
                p.id = self.next_id;
                self.next_id += 1;
            }
        }
        for s in self.stops.values_mut() {
            s.seeded = on;
            for sp in s.spots.iter_mut() {
                sp.taken = None;
            }
        }
        self.claims_out.clear();
        self.claimed.clear();
    }

    /// The human type of a file relative to a content root (`Humans/…/x.hum`).
    pub fn type_by_file(&self, file: &str) -> Option<usize> {
        let want = file.replace('\\', "/").to_ascii_lowercase();
        self.types.iter().position(|t| {
            t.def
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with(&want)
        })
    }

    /// The file of a human type relative to its content root (`Humans/…/x.hum`).
    pub fn type_file(ty: &HumanType) -> String {
        let p = ty.def.path.to_string_lossy().replace('\\', "/");
        match p.to_ascii_lowercase().rfind("/humans/") {
            Some(k) => p[k + 1..].to_string(),
            None => p,
        }
    }

    /// One of the host's people appears here (client).
    pub fn mirror_add(
        &mut self,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        id: u32,
        ty: usize,
        pose: &MirrorPose,
    ) -> bool {
        if self.people.iter().any(|p| p.id == id) {
            return false;
        }
        let state = State::Leaving {
            target: pose.pos,
            walk: None,
            walked: 0.0,
        };
        let Some(i) =
            self.spawn_as(world, renderer, scene, pose.pos, pose.heading, state, Some(ty))
        else {
            return false;
        };
        self.next_id -= 1;
        let p = &mut self.people[i];
        p.id = id;
        p.anim = Pose::new(id);
        p.remote = true;
        self.mirror_set(id, pose);
        true
    }

    /// Where one of the host's people is this frame (client).
    pub fn mirror_set(&mut self, id: u32, pose: &MirrorPose) {
        let Some(p) = self.people.iter_mut().find(|p| p.id == id && p.remote) else {
            return;
        };
        p.position = pose.pos;
        p.heading = pose.heading;
        p.vel = pose.vel;
        p.activity = pose.activity;
        let state = match (pose.aboard, pose.waiting) {
            (Some((bus, local, lheading, seat)), _) => {
                p.place = Place::Bus(BusId::Ai(bus), local);
                p.lheading = lheading;
                p.vel = DVec2::ZERO;
                State::Riding {
                    bus: BusId::Ai(bus),
                    seat: seat.unwrap_or(usize::MAX),
                }
            }
            (None, Some((stop, spot))) => {
                p.place = Place::Ground;
                State::Waiting {
                    stop,
                    spot,
                    patience: f32::MAX,
                }
            }
            (None, None) => {
                p.place = Place::Ground;
                State::Leaving {
                    target: pose.pos,
                    walk: None,
                    walked: 0.0,
                }
            }
        };
        if std::mem::discriminant(&p.state) != std::mem::discriminant(&state)
            || p.state.bus() != state.bus()
            || matches!((&p.state, &state), (State::Waiting { stop: a, spot: b, .. }, State::Waiting { stop: c, spot: d, .. }) if (a, b) != (c, d))
            || matches!((&p.state, &state), (State::Riding { seat: a, .. }, State::Riding { seat: b, .. }) if a != b)
        {
            p.state = state;
            p.t_state = 0.0;
        }
    }

    /// One of the host's people has gone (client).
    pub fn mirror_remove(&mut self, id: u32) {
        if let Some(i) = self.people.iter().position(|p| p.id == id && p.remote) {
            let p = self.people.swap_remove(i);
            self.retire(&p);
        }
        self.claimed.remove(&id);
    }

    /// A remote person this frame: they stand where the host put them (the crowd leaves
    /// them alone), and one waiting at the stop our bus serves, with a door open or about
    /// to open and a place free, is asked for (`take_claims`).
    fn mirror_want(&mut self, i: usize, buses: &[BusNow]) -> Want {
        let p = &self.people[i];
        let want = Want::stand(None, p.activity);
        let State::Waiting { stop, .. } = p.state else {
            return want;
        };
        let id = p.id;
        let Some(bn) = buses
            .iter()
            .find(|b| b.id == BusId::Player && b.stop == Some(stop) && b.standing())
        else {
            return want;
        };
        if self.empties_at(bn, stop) {
            return want;
        }
        let free = self
            .seats
            .get(&BusId::Player)
            .map(|v| v.iter().any(|t| !t))
            .unwrap_or(false);
        if !free || !self.doors_open_or_arriving(bn) || self.choose_entry(i, bn).is_none() {
            return want;
        }
        // (asked again after a while: the answer may have been lost)
        let t = self.time;
        if self.claimed.get(&id).map(|at| t - at > 3.0).unwrap_or(true) {
            self.claimed.insert(id, t);
            self.claims_out.push(id);
        }
        want
    }

    /// The type of one of our people (host).
    pub fn lan_people_by_id(&self, id: u32) -> Option<Arc<HumanType>> {
        self.people
            .iter()
            .find(|p| p.id == id && !p.remote)
            .map(|p| p.ty.clone())
    }

    /// Where the host's people are drawn (client; `OMSI_LAN_TRACE`).
    pub fn mirror_positions(&self) -> Vec<(u32, DVec3)> {
        self.people
            .iter()
            .filter(|p| p.remote && p.place == Place::Ground)
            .map(|p| (p.id, p.position))
            .collect()
    }

    /// Waiting people to ask the host for (client).
    pub fn take_claims(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.claims_out)
    }

    /// The host handed this waiting person over to our bus (client): from now on they
    /// are ours, and board as anybody waiting here would.
    pub fn grant(&mut self, id: u32) -> bool {
        self.claimed.remove(&id);
        let Some(i) = self.people.iter().position(|p| p.id == id && p.remote) else {
            return false;
        };
        let State::Waiting { stop, spot, .. } = self.people[i].state else {
            return false;
        };
        let p = &mut self.people[i];
        p.remote = false;
        p.avoid = None;
        p.vel = DVec2::ZERO;
        self.set_state(
            i,
            State::Waiting {
                stop,
                spot,
                patience: 90.0,
            },
        );
        if let Some(sp) = self.stops.get_mut(&stop).and_then(|s| s.spots.get_mut(spot)) {
            sp.taken = Some(id);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Berlin 1991's pack: full fare, short haul, day ticket (adults), and two reduced
    /// fares for 6..13.
    fn berlin_91() -> omsi_content::tickets::TicketPack {
        let t = |name: &str, age: (i32, i32), day: bool, p: f32| omsi_content::tickets::Ticket {
            name: name.into(),
            age_min: age.0,
            age_max: age.1,
            day_ticket: day,
            probability: p,
            ..Default::default()
        };
        omsi_content::tickets::TicketPack {
            stamper_prop: 0.3,
            ticketbuy_prop: 0.2,
            tickets: vec![
                t("Fahrschein", (14, 200), false, 1.0),
                t("Kurzstrecke", (14, 200), false, 0.4),
                t("Tageskarte", (14, 200), true, 0.2),
                t("Ermaessigt", (6, 13), false, 1.0),
                t("Kurzstrecke Erm", (6, 13), false, 0.4),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn tickets_by_age_and_time() {
        let mut h = Humans::new(Path::new("/nonexistent"));
        h.tickets = Some(Arc::new(berlin_91()));
        let count = |h: &mut Humans, age: f32| {
            let mut n = [0usize; 5];
            for _ in 0..4000 {
                n[h.pick_ticket(age).unwrap()] += 1;
            }
            n
        };
        // an adult (OMSI's default age of 40) never gets a reduced fare, a child only those
        h.time_of_day = 9.0 * 3600.0;
        let adult = count(&mut h, 40.0);
        assert_eq!(adult[3] + adult[4], 0);
        assert!(adult[2] > 300, "{adult:?}");
        let child = count(&mut h, 10.0);
        assert_eq!(child[0] + child[1] + child[2], 0);
        // day tickets sell best at 9:00, little early in the morning and late at night
        h.time_of_day = 1.0 * 3600.0;
        let early = count(&mut h, 40.0);
        assert!(early[2] * 4 < adult[2], "{early:?} vs {adult:?}");
        assert!(day_ticket_factor(9.0 * 3600.0) > 0.99);
        assert!(day_ticket_factor(0.0) < 0.01);
        assert!((day_ticket_factor(20.0 * 3600.0) - (1.0 - 39_600.0 / 56_376.0) as f32).abs() < 1e-3);
        // nobody in the age range: no ticket
        assert_eq!(h.pick_ticket(3.0), None);
    }

    fn lane(points: Vec<DVec3>, kind: LaneKind) -> omsi_sim::traffic::Lane {
        omsi_sim::traffic::LaneBuilder::polyline(points, kind, 2.5)
    }

    #[test]
    fn pavement_corners_are_joined_and_routed() {
        // an L of pavement: the two paths meet at a right angle, which the road network's
        // heading rule leaves unlinked
        let mut net = Network::default();
        net.lanes.push(lane(
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 20.0, 0.0)],
            LaneKind::Sidewalk,
        ));
        net.lanes.push(lane(
            vec![DVec3::new(0.5, 20.3, 0.0), DVec3::new(30.0, 20.3, 0.0)],
            LaneKind::Sidewalk,
        ));
        net.lanes.push(lane(
            vec![DVec3::new(-5.0, 10.0, 0.0), DVec3::new(5.0, 10.0, 0.0)],
            LaneKind::Street,
        ));
        net.link(1.5);
        assert!(
            net.lanes[0].next.is_empty(),
            "the road rule does not join the corner"
        );
        let ped = PedNet::build(&net);
        let legs = ped
            .route(&net, (0, 5.0), (1, 12.0))
            .expect("a route round the corner");
        assert_eq!(legs.len(), 2);
        assert_eq!((legs[0].lane, legs[0].a, legs[0].b), (0, 5.0, 20.0));
        assert_eq!(legs[1].lane, 1);
        assert!((legs[1].b - 12.0).abs() < 1e-4);
        // walking back the other way
        let back = ped.route(&net, (1, 12.0), (0, 5.0)).unwrap();
        assert!(
            (back[0].b - 0.0).abs() < 1e-4 && (back[1].a - 20.0).abs() < 1e-4,
            "{back:?}"
        );
        // a dead end turns round
        let n = ped
            .end_node(
                &net,
                &Leg {
                    lane: 1,
                    a: 0.0,
                    b: net.lanes[1].length(),
                },
            )
            .unwrap();
        let turn = ped.next_leg(&net, n, 1, 7).unwrap();
        assert_eq!(turn.lane, 1);
        assert!(turn.a > turn.b);
    }

    #[test]
    fn crossings_of_a_pavement_path_are_found() {
        let mut net = Network::default();
        net.lanes.push(lane(
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 8.0, 0.0)],
            LaneKind::Sidewalk,
        ));
        net.lanes.push(lane(
            vec![DVec3::new(-30.0, 4.0, 0.0), DVec3::new(30.0, 4.0, 0.0)],
            LaneKind::Street,
        ));
        net.link(1.5);
        let mut ped = PedNet::build(&net);
        let x = ped.crossings(&net, 0).to_vec();
        assert_eq!(x.len(), 1);
        assert!((x[0] - DVec2::new(0.0, 4.0)).length() < 1e-6);
    }

    /// An articulated bus: the front section's cabin and the rear section's (which only has
    /// exits and a seat) become one network through the joint, numbered front first, and a
    /// walk through the bent joint moves on without a jump.
    #[test]
    fn articulated_cabins_are_joined_through_the_bellows() {
        let dir = std::env::temp_dir().join(format!("omsi-humans-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        // front: door 0 at the front right, exit 1 in the middle, link to the rear at 3
        write("paths_a.cfg", "[pathpnt]\n1.2\n4\n0.4\n[pathpnt]\n0\n4\n0.5\n[pathpnt]\n0\n0\n0.5\n[pathpnt]\n0\n-4.2\n0.6\n[pathpnt]\n1.2\n0\n0.4\n[pathlink]\n0\n1\n[pathlink]\n1\n2\n[pathlink]\n2\n3\n[pathlink]\n2\n4\n");
        write(
            "cabin_a.cfg",
            "[entry]\n0\n[exit]\n4\n[linkToPrevVeh]\n3\n[passpos]\n-0.5\n2\n1.0\n0.45\n0\n",
        );
        // rear: only exits, a seat and the link to the front at point 0
        write("paths_b.cfg", "[pathpnt]\n0\n3.6\n0.6\n[pathpnt]\n0\n0\n0.6\n[pathpnt]\n1.2\n0\n0.4\n[pathpnt]\n0\n-2\n0.6\n[pathlink]\n0\n1\n[pathlink]\n1\n2\n[pathlink]\n1\n3\n");
        write(
            "cabin_b.cfg",
            "[exit]\n2\n[linkToNextVeh]\n0\n[passpos]\n-0.5\n-2\n1.1\n0.45\n0\n",
        );
        let def = |cabin: &str, paths: &str| omsi_vehicle::Vehicle {
            path: dir.join("bus.bus"),
            passenger_cabin: Some(cabin.into()),
            paths: Some(paths.into()),
            bounding_box: Some([2.5, 9.0, 3.0, 0.0, 0.0, 1.5]),
            ..Default::default()
        };
        let (front, rear) = (
            def("cabin_a.cfg", "paths_a.cfg"),
            def("cabin_b.cfg", "paths_b.cfg"),
        );
        // couplings: the front's at y -4.3, the rear's own at y 4.0
        let (back, own) = (Vec3::new(0.0, -4.3, 0.3), Vec3::new(0.0, 4.0, 0.3));
        let offset = back - own;
        let cabin =
            Cabin::load_train(&[(&front, Vec3::ZERO, f32::INFINITY), (&rear, offset, back.y)])
                .expect("cabin");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(cabin.parts.len(), 2);
        assert_eq!(cabin.graph.points.len(), 9);
        assert_eq!(
            (cabin.entries.len(), cabin.exits.len(), cabin.seats.len()),
            (1, 2, 2)
        );
        // exit 1 is the rear section's door, where the rear file puts it
        assert!(
            (cabin.exits[1].inside - Vec3::new(1.2, -8.3, 0.4)).length() < 1e-4,
            "{:?}",
            cabin.exits[1].inside
        );
        assert_eq!(cabin.part_of(cabin.exits[1].inside), 1);
        assert_eq!(cabin.part_of(cabin.exits[0].inside), 0);
        // the seat in the rear is reached from the front door through the joint
        let seat = &cabin.seats[1];
        assert!((seat.pos.y + 10.3).abs() < 1e-4 && cabin.part_of(seat.floor) == 1);
        let route = cabin.route(cabin.entries[0].inside, seat.floor);
        assert!(
            route.iter().any(|p| (p.y + 4.2).abs() < 1e-4)
                && route.iter().any(|p| (p.y + 4.7).abs() < 1e-4),
            "{route:?}"
        );
        // and the nearest exit from there is the rear one
        assert_eq!(cabin.nearest_exit(seat.floor), 1);
        // the rear section bent 30 degrees about the coupling: walking down the aisle moves on
        // smoothly, and the frames agree with the sections away from the joint
        let lead_rot = Mat4::IDENTITY;
        let bent = 30.0f64;
        let rot = Mat4::from_rotation_z((-bent).to_radians() as f32);
        let pos = back.as_dvec3() - rot.transform_point3(own).as_dvec3();
        let frames = [PartFrame {
            pos,
            rot,
            heading: bent,
            offset,
            joint_y: back.y,
            half: DVec2::new(1.25, 4.5),
            centre: DVec2::ZERO,
        }];
        // beside the aisle the two frames disagree by 0.31 m at the joint itself
        let at_joint = Vec3::new(0.6, back.y, 0.5);
        let rear_frame = pos + rot.transform_point3(at_joint - offset).as_dvec3();
        assert!((rear_frame - at_joint.as_dvec3()).length() > 0.3);
        let mut last = train_point(DVec3::ZERO, &lead_rot, &frames, Vec3::new(0.6, 0.0, 0.5));
        for k in 1..=100 {
            let y = -(k as f32) * 0.1;
            let p = train_point(DVec3::ZERO, &lead_rot, &frames, Vec3::new(0.6, y, 0.5));
            assert!(
                (p - last).length() < 0.14,
                "a jump of {:.3} m at y {y}",
                (p - last).length()
            );
            last = p;
        }
        let ahead = train_point(DVec3::ZERO, &lead_rot, &frames, Vec3::new(1.0, -1.0, 0.5));
        assert!((ahead - DVec3::new(1.0, -1.0, 0.5)).length() < 1e-4);
        let behind_joint = Vec3::new(1.0, -9.0, 0.5);
        let p = train_point(DVec3::ZERO, &lead_rot, &frames, behind_joint);
        assert!(
            (p - (pos + rot.transform_point3(behind_joint - offset).as_dvec3())).length() < 1e-4
        );
        assert!((train_heading(0.0, &frames, behind_joint) - bent).abs() < 1e-9);
        assert!(
            (train_heading(0.0, &frames, Vec3::new(0.0, back.y, 0.5)) - bent * 0.5).abs() < 1e-9
        );
    }

    /// The SD202's cabin: the stairs down from the upper deck end beside the rear exits.
    /// Only the lower deck is those exits' floor (the upper deck lies right over their
    /// waiting line), the walk from up there to an exit goes down the stairs, and a foot
    /// put down on the stairs finds the height the flight has there.
    #[test]
    fn double_decker_exits_are_reached_down_the_stairs() {
        let root = omsi_cfg::env::var_os("OMSI_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("../../../OMSI 2 Original"));
        let bus = root.join("Vehicles/MAN_SD202/MAN_D92.bus");
        if !bus.exists() {
            eprintln!("skipped: no {}", bus.display());
            return;
        }
        let def = omsi_vehicle::Vehicle::load(&bus).expect("SD202");
        let cabin = Cabin::load_train(&[(&def, Vec3::ZERO, f32::INFINITY)]).expect("cabin");
        assert_eq!(cabin.exits.len(), 2);
        let exit = &cabin.exits[1];
        assert!(
            (exit.wait - Vec3::new(0.806, -1.26, 0.505)).length() < 1e-3,
            "{:?}",
            exit.wait
        );
        assert!(exit.on_floor(0.505) && exit.on_floor(0.47) && exit.on_floor(0.64));
        assert!(
            !exit.on_floor(2.46) && !exit.on_floor(1.205),
            "the upper deck and the landing are not the exit's floor"
        );
        // the second in line waits behind the aisle point, right under the upper deck's aisle
        let queue = exit.queue_place(2, -3.0);
        let upstairs = Vec3::new(queue.x, queue.y, 2.46);
        let route = cabin.route(upstairs, exit.wait);
        assert!(
            route.iter().any(|p| (p.z - 1.82).abs() < 0.01)
                && route.iter().any(|p| (p.z - 1.205).abs() < 0.01),
            "{route:?}"
        );
        assert!((route.last().unwrap().z - exit.wait.z).abs() < 1e-4);
        // halfway down the last flight (path points 36 → 12)
        let half = DVec2::new(-0.425, -1.3075);
        let z = cabin.floor_at(half, 1.25, 0.85).unwrap();
        assert!((z - 0.855).abs() < 0.05, "{z}");
        // the decks over each other: the body's own floor
        assert!((cabin.floor_at(DVec2::new(0.0, -1.8), 1.25, 2.46).unwrap() - 2.46).abs() < 0.05);
        assert!((cabin.floor_at(DVec2::new(0.0, -1.8), 1.25, 0.55).unwrap() - 0.55).abs() < 0.05);
        // beside the stairs on the aisle, the aisle
        assert!((cabin.floor_at(DVec2::new(0.1, -1.2), 1.25, 0.505).unwrap() - 0.505).abs() < 0.02);
    }

    #[test]
    fn legs_run_both_ways() {
        let mut net = Network::default();
        net.lanes.push(lane(
            vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 10.0, 0.0)],
            LaneKind::Sidewalk,
        ));
        let back = Leg {
            lane: 0,
            a: 8.0,
            b: 2.0,
        };
        let (p, h) = back.at(&net, 1.0);
        assert!((p.y - 7.0).abs() < 1e-6);
        assert!((h - 180.0).abs() < 1e-6);
        assert!((back.project(&net, DVec3::new(0.3, 5.0, 0.0), 2.5) - 3.0).abs() < 0.11);
    }
}

/// A heading in 0..360 degrees.
/// A line a passenger says: the sample and where they stand.
pub struct VoiceLine {
    pub position: DVec3,
    pub path: std::path::PathBuf,
}

/// How much of its probability a day ticket keeps at a time of day (seconds): rising from
/// nothing at midnight to all of it at 9:00, as the ticket packs describe it, then falling
/// on OMSI's line.
fn day_ticket_factor(t: f64) -> f32 {
    let t = t.rem_euclid(86_400.0);
    let rise = t / 32_400.0;
    let fall = 1.0 - (t - 32_400.0) / (88_776.0 - 32_400.0);
    rise.min(fall).clamp(0.0, 1.0) as f32
}

fn wrap_heading(h: f64) -> f64 {
    h.rem_euclid(360.0)
}

/// The angle between two headings (degrees, 0..180).
fn angle_between(a: f64, b: f64) -> f64 {
    ((b - a + 540.0).rem_euclid(360.0) - 180.0).abs()
}
