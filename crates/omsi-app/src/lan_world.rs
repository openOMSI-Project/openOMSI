//! LAN play: one world for everybody. The host's game simulates the AI traffic, the
//! timetable buses, the people on the pavements and at the stops, and the traffic lights -
//! for the whole session, around every player (`Traffic::lan_centers`,
//! `Humans::lan_centers`, the tile streamer). A client simulates none of that: it draws
//! what the host sends (`omsi_net::world`), a little in the past so that it can glide
//! between two frames, and simulates only its own bus and the people who board it.
//!
//! Who a waiting passenger belongs to is decided by the host alone: a client whose bus
//! stands at a stop with a door open asks for the people waiting there (`CLAIM`); the host
//! hands over those still waiting (they leave its world) and turns down the others, who
//! have meanwhile walked up to another bus. Nobody is ever on two buses.
//!
//! What goes over the network per client, around where its bus is: cars within
//! `CAR_RADIUS` (ten times a second while they move, once a second while they stand),
//! people within `PERSON_RADIUS` (the same), the light programs within `LIGHT_RADIUS`
//! (once a second), and once per thing a description (`DESC`: its vehicle or human file).
//! `OMSI_DEBUG_LAN` logs the counts and the bytes a second, `OMSI_LAN_TRACE=<file.csv>`
//! every pose sent (host) and drawn (client) for comparing them.

use crate::humans::{Humans, MirrorPose};
use crate::scene::World;
use crate::traffic::Traffic;
use crate::view_sync::people::PeopleView;
use crate::view_sync::SimView;
use crate::Args;
use glam::{DVec2, DVec3, Vec3};
use hashbrown::{HashMap, HashSet};
use omsi_net::world::{
    self as nw, Activity as NetActivity, CarState, Desc, EntityRef, LightState, PersonPlace,
    PersonState, WorldFrame,
};
use omsi_net::{LanSession, Role};
use omsi_render::{Renderer, Scene};
use omsi_sim::human::Activity;
use omsi_sim::vehicle::AiFrame;
use omsi_sim::VehicleType;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Instant;

/// How far around a client its cars, people and light programs are sent (m).
pub const CAR_RADIUS: f64 = 650.0;
pub const PERSON_RADIUS: f64 = 260.0;
pub const LIGHT_RADIUS: f64 = 450.0;
/// How far in the past a client draws the host's world (ms): two frames of moving things
/// to glide between, and room for a late datagram.
const INTERP_DELAY: f64 = 160.0;
/// How long a client guesses on past the last frame of something moving (ms).
const MAX_EXTRAPOLATE: f64 = 300.0;
/// A description asked for is asked again after this long (s).
const WANT_AGAIN: f32 = 0.8;

type Key = (bool, u32);

/// What the host has sent one client.
#[derive(Default)]
struct View {
    acc: f32,
    lights_acc: f32,
    /// Per thing: the quantised pose sent last and the seconds since.
    sent: HashMap<Key, ([i64; 4], f32)>,
    /// Per thing: the hash of the description sent.
    described: HashMap<Key, u64>,
    /// Things that went out of sight, named for a second more.
    gone: Vec<(Key, f32)>,
    bytes: u64,
    /// What was in the last frame (for the log: counts and a checksum of the ids).
    last_counts: (usize, usize, usize),
    ids_hash: u64,
}

/// One pose of a thing as a client got it, on the host's clock.
#[derive(Clone, Copy)]
struct Sample<T> {
    ms: f64,
    v: T,
}

struct Track<T> {
    samples: Vec<Sample<T>>,
    heard: Instant,
}

impl<T: Copy> Track<T> {
    fn push(&mut self, ms: f64, v: T) {
        if self.samples.last().map(|s| ms <= s.ms).unwrap_or(false) {
            return;
        }
        self.samples.push(Sample { ms, v });
        if self.samples.len() > 12 {
            self.samples.remove(0);
        }
        self.heard = Instant::now();
    }

    /// The samples around `ms` and how far between them it lies (0..1, more when guessing
    /// on past the last).
    fn around(&self, ms: f64) -> Option<(&T, &T, f64)> {
        let s = &self.samples;
        let last = s.last()?;
        if s.len() == 1 || ms >= last.ms {
            let prev = if s.len() > 1 { &s[s.len() - 2] } else { last };
            let span = (last.ms - prev.ms).max(1.0);
            let t = if s.len() > 1 {
                1.0 + ((ms - last.ms).min(MAX_EXTRAPOLATE)).max(0.0) / span
            } else {
                1.0
            };
            return Some((&prev.v, &last.v, t));
        }
        if ms <= s[0].ms {
            return Some((&s[0].v, &s[0].v, 0.0));
        }
        let k = s.iter().position(|x| x.ms > ms)?;
        let (a, b) = (&s[k - 1], &s[k]);
        Some((&a.v, &b.v, (ms - a.ms) / (b.ms - a.ms).max(1.0)))
    }
}

/// The moment the others' things are drawn at, on their clock: it runs on with ours and is
/// pulled towards where it should be (now, less the delay, on their clock) by at most a
/// few per cent of its pace. Set anew each frame from the offset of the clocks and the
/// delay, it jumped with them - a datagram that came quicker than the others, a delay
/// grown after a slow frame of theirs - and everything drawn went back or on a bit at
/// once: the micro-teleports.
#[derive(Default, Clone, Copy)]
pub(crate) struct PlayClock {
    /// (the moment drawn, our clock then)
    at: Option<(f64, f64)>,
}

impl PlayClock {
    /// The moment to draw at `now` (our clock) when it should be `want`; more than `snap`
    /// off (a new session, a pause), it is taken at once.
    pub(crate) fn step(&mut self, now: f64, want: f64, snap: f64) -> f64 {
        let t = match self.at {
            Some((t, then)) => {
                let d = (now - then).max(0.0);
                let run = t + d;
                let err = want - run;
                if err.abs() > snap {
                    want
                } else {
                    run + err.clamp(-0.06 * d, 0.06 * d)
                }
            }
            None => want,
        };
        self.at = Some((t, now));
        t
    }
}

fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    let d = (b - a + 540.0).rem_euclid(360.0) - 180.0;
    (a + d * t).rem_euclid(360.0)
}

/// A client's copy of the host's world.
#[derive(Default)]
struct Mirror {
    on: bool,
    cars: HashMap<u32, Track<CarState>>,
    people: HashMap<u32, Track<PersonState>>,
    descs: HashMap<Key, Desc>,
    /// Descriptions asked for and when.
    wanted: HashMap<Key, Instant>,
    /// Vehicle types by file: loaded, being loaded, or not to be had.
    types: HashMap<PathBuf, Option<Arc<VehicleType>>>,
    loading: HashSet<PathBuf>,
    loaded_rx: Option<mpsc::Receiver<(PathBuf, Option<Arc<VehicleType>>)>>,
    loaded_tx: Option<mpsc::Sender<(PathBuf, Option<Arc<VehicleType>>)>>,
    /// Depot files by vehicle file (the destination displays of the timetable buses).
    hofs: HashMap<PathBuf, Option<Arc<omsi_vehicle::Hof>>>,
    /// The line and destination each car shows.
    shown: HashMap<u32, (String, String)>,
    /// Cars and people drawn (created here).
    drawn_cars: HashSet<u32>,
    drawn_people: HashSet<u32>,
    /// Distance each car has rolled (its wheels).
    odometer: HashMap<u32, f32>,
    /// The host's clock minus ours (ms), from the frames that came fastest.
    offset: Option<f64>,
    play: PlayClock,
    epoch: Option<Instant>,
    bytes_at: u64,
    granted: u32,
    denied: u32,
}

impl Mirror {
    fn now_ms(&mut self) -> f64 {
        let e = *self.epoch.get_or_insert_with(Instant::now);
        e.elapsed().as_secs_f64() * 1000.0
    }
}

/// The people of a client's own on foot, as the host draws them (host).
#[derive(Default)]
struct Upstream {
    tracks: HashMap<u32, Track<PersonState>>,
    files: HashMap<u32, String>,
    /// Their ids here, by the client's.
    ids: HashMap<u32, u32>,
    offset: Option<f64>,
    play: PlayClock,
}

/// What a client sends the host of its own people.
#[derive(Default)]
struct UpView {
    acc: f32,
    sent: HashMap<u32, ([i64; 4], f32)>,
    described: HashSet<u32>,
    gone: Vec<(u32, f32)>,
}

#[derive(Default)]
pub struct LanWorld {
    views: HashMap<u32, View>,
    mirror: Mirror,
    ups: HashMap<u32, Upstream>,
    up_view: UpView,
    /// The next id a client's person gets here (far above our own people's).
    next_up_id: u32,
    epoch: Option<Instant>,
    log_t: f32,
    trace: Option<std::io::BufWriter<std::fs::File>>,
    trace_t: f32,
    trace_opened: bool,
    /// Host: the parking spaces whose cars have driven off, as the world has them now.
    departed: Vec<i64>,
}

fn quant(x: f64, y: f64, z: f64, h: f64) -> [i64; 4] {
    [
        (x * 20.0) as i64,
        (y * 20.0) as i64,
        (z * 20.0) as i64,
        (h * 4.0) as i64,
    ]
}

fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A file path as it is written for the other games: relative to its content root.
fn relative_file(path: &Path, root: &Path) -> String {
    let mut roots = omsi_cfg::content_roots();
    roots.push(root.to_path_buf());
    relative_to_roots(path, &roots).unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
}

/// `path` relative to the root of `roots` it lies in, the deepest one: an OMSI 2 folder
/// inside the content folder (`openOMSI/OMSI 2`, as a server is often laid out) names its
/// files `Vehicles/...` as the other games find them, not `OMSI 2/Vehicles/...`, which no
/// other game has (a Linux server's AI cars were left out on every client, #1097).
pub(crate) fn relative_to_roots(path: &Path, roots: &[PathBuf]) -> Option<String> {
    let r = roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.components().count())?;
    path.strip_prefix(r).ok().map(|rel| rel.to_string_lossy().replace('\\', "/"))
}

/// A content-relative file from the host as a file here, when it exists here.
fn local_file(args: &Args, rel: &str) -> Option<PathBuf> {
    let path = omsi_cfg::resolve_path(&args.root, rel);
    if omsi_cfg::vfs::is_file(&path) {
        return Some(path);
    }
    // (a host that named it under a folder of its own, "OMSI 2/Vehicles/...": the same
    // vehicle under any content root here, as a remote player's bus is looked for)
    let k = rel.to_ascii_lowercase().replace('\\', "/").find("vehicles/")?;
    omsi_cfg::find_in_roots(&rel.replace('\\', "/")[k..]).map(|(_, p)| p).filter(|p| omsi_cfg::vfs::is_file(p))
}

fn net_activity(a: Activity) -> NetActivity {
    match a {
        Activity::Walk => NetActivity::Walk,
        Activity::Sit => NetActivity::Sit,
        _ => NetActivity::Stand,
    }
}

fn sim_activity(a: NetActivity) -> Activity {
    match a {
        NetActivity::Walk | NetActivity::Run => Activity::Walk,
        NetActivity::Sit => Activity::Sit,
        NetActivity::Stand => Activity::Stand,
    }
}

/// The line a timetable bus shows and the depot terminus it is set to, `#<index>` (host;
/// `#-1` for none, so that a client still knows it is a timetable bus).
fn car_display(v: &omsi_sim::VehicleInstance) -> (String, String) {
    let line = v
        .ty
        .program
        .str_var("SetLineTo")
        .and_then(|i| v.state.str_vars.get(i as usize))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let target = v
        .var("AI_target_index")
        .filter(|t| *t >= 0.0)
        .map(|t| t as i32)
        .unwrap_or(-1);
    (line, format!("#{target}"))
}

/// A host's timetable bus on our copy of it: line `line` and destination `destination`,
/// the row of the depot file [`car_display`] names (`#<row>`) - that row itself. Looked up
/// again by the row's sign text, it was the first row with that text anywhere on its sign:
/// Spandau's buses to U Ruhleben (282) showed Machandelweg (194), whose second line reads
/// RUHLEBEN.
fn show_car_destination(v: &mut omsi_sim::VehicleInstance, hof: Option<&omsi_vehicle::Hof>, line: &str, destination: &str) {
    let row = destination.strip_prefix('#').and_then(|n| n.parse::<usize>().ok());
    if let (Some(ti), Some(hof)) = (row, hof) {
        crate::schedule::set_ai_destination_at(v, hof, line, ti, &[]);
    }
}

impl LanWorld {
    /// Once a frame, after `LanSession::tick`.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        lan: &mut LanSession,
        dt: f32,
        args: &Args,
        world: Option<&World>,
        renderer: Option<&Renderer>,
        scene: Option<&mut Scene>,
        traffic: Option<&mut Traffic>,
        humans: Option<&mut Humans>,
        view: &mut SimView,
        me: Option<DVec3>,
    ) {
        if !self.trace_opened {
            self.trace_opened = true;
            self.trace = omsi_cfg::flags::OMSI_LAN_TRACE.var().and_then(|p| {
                std::fs::File::create(p)
                    .ok()
                    .map(std::io::BufWriter::new)
            });
        }
        match lan.role {
            Role::Host => {
                let mut humans = humans;
                self.departed = world.map(|w| w.departed_keys()).unwrap_or_default();
                self.host(lan, dt, args, traffic, humans.as_deref_mut(), me);
                if let (Some(w), Some(r), Some(sc), Some(h)) = (world, renderer, scene, humans) {
                    self.people_from_clients(lan, w, r, sc, h, &mut view.people);
                }
            }
            Role::Client => {
                let mut humans = humans;
                self.client(lan, dt, args, world, renderer, scene, traffic, humans.as_deref_mut(), view);
                if let (Some(h), Some(me)) = (humans, me) {
                    self.people_to_host(lan, dt, h, me);
                }
            }
        }
    }

    /// The people of our own on foot near our bus, to the host (client): those who walk up
    /// to it and those who got off, so that the others see them too.
    fn people_to_host(&mut self, lan: &mut LanSession, dt: f32, h: &Humans, me: DVec3) {
        if !self.mirror.on {
            return;
        }
        let v = &mut self.up_view;
        v.acc += dt;
        for s in v.sent.values_mut() {
            s.1 += dt;
        }
        if v.acc + 1.0e-4 < nw::MOVING_EVERY {
            return;
        }
        v.acc = (v.acc - nw::MOVING_EVERY).clamp(0.0, nw::MOVING_EVERY);
        // (their ids on the wire: those the host handed over keep the host's, the others
        // are numbered from 2^23 up)
        let wire = |id: u32| {
            if id >= 1 << 30 {
                ((id - (1 << 30)) & 0x7F_FFFF) | 0x80_0000
            } else {
                id & 0x7F_FFFF
            }
        };
        let mut frame = WorldFrame::default();
        let mut seen: HashSet<u32> = HashSet::new();
        for p in h.lan_people(me, PERSON_RADIUS) {
            if p.aboard.is_some() {
                continue;
            }
            let id = wire(p.id);
            seen.insert(id);
            if v.described.insert(id) {
                lan.send_desc_up(&Desc::Person {
                    id,
                    file: Humans::type_file(&p.ty),
                });
            }
            let q = quant(p.pos.x, p.pos.y, p.pos.z, p.heading);
            let q = [q[0], q[1], q[2], q[3] ^ ((p.activity as i64) << 40)];
            let due = match v.sent.get(&id) {
                None => true,
                Some((last, age)) => *last != q || *age >= nw::STANDING_EVERY,
            };
            if !due {
                continue;
            }
            v.sent.insert(id, (q, 0.0));
            frame.people.push(PersonState {
                id,
                activity: net_activity(p.activity),
                place: PersonPlace::Foot {
                    x: p.pos.x,
                    y: p.pos.y,
                    z: p.pos.z,
                    heading: p.heading as f32,
                    speed: p.speed as f32,
                    waiting: None,
                },
            });
        }
        // and the riders of our bus, where they sit or stand in it
        for p in h.lan_riders() {
            let Some((_, l, lh, seat)) = p.aboard else { continue };
            let id = wire(p.id);
            seen.insert(id);
            if v.described.insert(id) {
                lan.send_desc_up(&Desc::Person {
                    id,
                    file: Humans::type_file(&p.ty),
                });
            }
            let place = PersonPlace::Aboard {
                bus: nw::PLAYER_BUS | lan.my_id,
                x: l.x,
                y: l.y,
                z: l.z,
                heading: lh as f32,
                seat: seat.map(|s| s.min(254) as u8),
            };
            let st = PersonState { id, activity: net_activity(p.activity), place };
            let q = person_q(&st);
            let due = match v.sent.get(&id) {
                None => true,
                Some((last, age)) => *last != q || *age >= nw::STANDING_EVERY,
            };
            if !due {
                continue;
            }
            v.sent.insert(id, (q, 0.0));
            frame.people.push(st);
        }
        let gone: Vec<u32> = v.sent.keys().filter(|k| !seen.contains(*k)).copied().collect();
        for k in gone {
            v.sent.remove(&k);
            v.gone.push((k, 1.0));
        }
        for g in v.gone.iter_mut() {
            g.1 -= nw::MOVING_EVERY;
        }
        v.gone.retain(|g| g.1 > 0.0 && !seen.contains(&g.0));
        frame.gone = v.gone.iter().map(|(k, _)| (true, *k)).collect();
        if !frame.people.is_empty() || !frame.gone.is_empty() {
            lan.send_world_up(&frame);
        }
    }

    /// The clients' own people on foot, drawn here (host).
    fn people_from_clients(
        &mut self,
        lan: &mut LanSession,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        h: &mut Humans,
        view: &mut PeopleView,
    ) {
        // (claims of our bus for somebody a client sent: not ours to give)
        let _ = h.take_claims();
        let epoch = *self.epoch.get_or_insert_with(Instant::now);
        for (id, d) in lan.take_descs_up() {
            if let Desc::Person { id: pid, file } = d {
                self.ups.entry(id).or_default().files.insert(pid, file);
            }
        }
        for (id, at, f) in lan.take_world_up() {
            let up = self.ups.entry(id).or_default();
            let arrived = at
                .checked_duration_since(epoch)
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            let off = f.host_ms as f64 - arrived;
            up.offset = Some(match up.offset {
                // (drifting down 2 ms a second at their 8 frames a second: 20 ms a frame
                // made it follow every frame's way over the network)
                Some(o) if (off - o).abs() < 1000.0 => off.max(o - 0.25),
                _ => off,
            });
            for p in f.people {
                up.tracks
                    .entry(p.id)
                    .or_insert_with(|| Track {
                        samples: Vec::new(),
                        heard: Instant::now(),
                    })
                    .push(f.host_ms as f64, p);
            }
            for (_, pid) in f.gone {
                up.tracks.remove(&pid);
            }
        }
        // players that left: their people go with them
        let here: HashSet<u32> = lan.peers().map(|p| p.pose.id).collect();
        let forget = std::time::Duration::from_secs_f32(nw::FORGET_AFTER);
        let now = epoch.elapsed().as_secs_f64() * 1000.0;
        for (peer, up) in self.ups.iter_mut() {
            if !here.contains(peer) {
                up.tracks.clear();
            }
            up.tracks.retain(|_, t| t.heard.elapsed() < forget);
            let gone: Vec<u32> = up
                .ids
                .keys()
                .copied()
                .filter(|k| !up.tracks.contains_key(k))
                .collect();
            for k in gone {
                if let Some(local) = up.ids.remove(&k) {
                    h.mirror_remove(local);
                }
            }
            let render_ms = up.play.step(now, now + up.offset.unwrap_or(0.0) - INTERP_DELAY, 500.0);
            for (pid, track) in &up.tracks {
                let Some((a, b, k)) = track.around(render_ms) else {
                    continue;
                };
                let pose = person_pose(a, b, k);
                if let Some(local) = up.ids.get(pid).copied() {
                    if h.has_mirror(local) {
                        h.mirror_set(local, &pose);
                        continue;
                    }
                    up.ids.remove(pid);
                }
                if pose.aboard.map(|a| !h.knows_bus(a.0)).unwrap_or(false) {
                    continue;
                }
                let Some(ty) = up.files.get(pid).and_then(|f| h.type_by_file(f)) else {
                    continue;
                };
                if self.next_up_id < 1 << 29 {
                    self.next_up_id = 1 << 29;
                }
                let local = self.next_up_id;
                self.next_up_id += 1;
                if h.mirror_add(view, world, renderer, scene, local, ty, &pose) {
                    up.ids.insert(*pid, local);
                }
            }
        }
        self.ups.retain(|peer, up| here.contains(peer) || !up.ids.is_empty());
    }

    // -----------------------------------------------------------------------------------
    // host

    fn host(
        &mut self,
        lan: &mut LanSession,
        dt: f32,
        args: &Args,
        mut traffic: Option<&mut Traffic>,
        mut humans: Option<&mut Humans>,
        me: Option<DVec3>,
    ) {
        // where the players are: the world is kept around them too
        let players: Vec<(u32, DVec3)> = lan
            .peers()
            .filter(|p| p.has_pose && p.pose.has_vehicle())
            .map(|p| (p.pose.id, DVec3::new(p.pose.x, p.pose.y, p.pose.z)))
            .collect();
        let centers: Vec<DVec3> = players.iter().map(|p| p.1).collect();
        if let Some(t) = traffic.as_deref_mut() {
            t.lan_centers = centers.clone();
            // and what they see: no car may come or go there in front of them either
            t.lan_eyes = lan
                .peers()
                .filter(|p| p.has_pose && p.pose.has_vehicle())
                .map(|p| {
                    let at = DVec3::new(p.pose.x, p.pose.y, p.pose.z);
                    crate::traffic::Viewer::lan_player(at, p.pose.heading as f64)
                })
                .collect();
        }
        if let Some(h) = humans.as_deref_mut() {
            h.lan_centers = centers;
        }
        // waiting people a client's bus takes
        for (id, ids) in lan.take_claims() {
            let granted = humans
                .as_deref_mut()
                .map(|h| h.hand_over(id, &ids))
                .unwrap_or_default();
            let denied: Vec<u32> = ids.iter().copied().filter(|i| !granted.contains(i)).collect();
            if !granted.is_empty() || !denied.is_empty() {
                log::info!(
                    "LAN: player {id}'s bus takes {} waiting passenger(s) {:?}{}",
                    granted.len(),
                    granted,
                    if denied.is_empty() {
                        String::new()
                    } else {
                        format!("; {denied:?} have boarded another bus")
                    }
                );
            }
            lan.answer_claim(id, &granted, &denied);
            if let Some(v) = self.views.get_mut(&id) {
                for g in &granted {
                    v.sent.remove(&(true, *g));
                }
            }
        }
        // descriptions asked for again (lost)
        let wants = lan.take_wants();
        let t_ref = traffic.as_deref();
        let h_ref = humans.as_deref();
        for (id, refs) in wants {
            for r in refs {
                if let Some(d) = describe(args, t_ref, h_ref, r) {
                    lan.send_desc(id, &d);
                }
            }
        }
        self.views.retain(|id, _| players.iter().any(|p| p.0 == *id));
        let players_at = players.clone();
        let lan_me_pos = me;
        for (id, at) in players {
            let view = self.views.entry(id).or_default();
            view.acc += dt;
            view.lights_acc += dt;
            for s in view.sent.values_mut() {
                s.1 += dt;
            }
            if view.acc + 1.0e-4 < nw::MOVING_EVERY {
                continue;
            }
            view.acc = (view.acc - nw::MOVING_EVERY).clamp(0.0, nw::MOVING_EVERY);
            let mut frame = WorldFrame::default();
            let mut seen: HashSet<Key> = HashSet::new();
            let mut descs: Vec<Desc> = Vec::new();
            let mut ids_hash = 0u64;
            if let Some(t) = t_ref {
                for c in &t.cars {
                    let p = c.vehicle.position;
                    if (p - at).truncate().length() > CAR_RADIUS || c.id > nw::MAX_ID as u64 {
                        continue;
                    }
                    let key = (false, c.id as u32);
                    seen.insert(key);
                    ids_hash ^= fnv(&c.id.to_le_bytes());
                    let d = describe_car(args, c);
                    let h = fnv(d.encode().as_bytes());
                    if view.described.get(&key) != Some(&h) {
                        view.described.insert(key, h);
                        descs.push(d);
                    }
                    let q = quant(p.x, p.y, p.z, c.vehicle.heading);
                    let speed = c.state.speed;
                    let due = match view.sent.get(&key) {
                        None => true,
                        Some((last, age)) => {
                            *last != q || speed.abs() > 0.05 || *age >= nw::STANDING_EVERY
                        }
                    };
                    if !due {
                        continue;
                    }
                    view.sent.insert(key, (q, 0.0));
                    frame.cars.push(CarState {
                        id: c.id as u32,
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        heading: c.vehicle.heading as f32,
                        pitch: c.vehicle.pitch as f32,
                        bank: c.vehicle.bank as f32,
                        speed,
                        steer: c.body.steer,
                        blinker: c.state.blinker.clamp(0, 3) as u8,
                        brake: c.state.braking,
                        lights: t.night,
                        at_station: if c.at_station() {
                            1
                        } else if !c.vehicle.station_released() {
                            -1
                        } else {
                            0
                        },
                    });
                }
            }
            if let Some(h) = h_ref {
                for p in h.lan_people(at, PERSON_RADIUS) {
                    if p.id > nw::MAX_ID {
                        continue;
                    }
                    let key = (true, p.id);
                    seen.insert(key);
                    ids_hash ^= fnv(&(p.id as u64 | 1 << 40).to_le_bytes());
                    if !view.described.contains_key(&key) {
                        view.described.insert(key, 0);
                        descs.push(Desc::Person {
                            id: p.id,
                            file: Humans::type_file(&p.ty),
                        });
                    }
                    let (place, q) = match p.aboard {
                        Some((bus, l, lh, seat)) => (
                            PersonPlace::Aboard {
                                bus: bus.min(nw::MAX_ID as u64) as u32,
                                x: l.x,
                                y: l.y,
                                z: l.z,
                                heading: lh as f32,
                                seat: seat.map(|s| s.min(254) as u8),
                            },
                            quant(l.x as f64, l.y as f64, l.z as f64, lh + bus as f64 * 1000.0),
                        ),
                        None => (
                            PersonPlace::Foot {
                                x: p.pos.x,
                                y: p.pos.y,
                                z: p.pos.z,
                                heading: p.heading as f32,
                                speed: p.speed as f32,
                                waiting: p.waiting.map(|(s, k)| (s, k.min(255) as u8)),
                            },
                            quant(p.pos.x, p.pos.y, p.pos.z, p.heading),
                        ),
                    };
                    let q = [q[0], q[1], q[2], q[3] ^ ((p.activity as i64) << 40) ^ (p.waiting.is_some() as i64) << 44];
                    let due = match view.sent.get(&key) {
                        None => true,
                        Some((last, age)) => *last != q || *age >= nw::STANDING_EVERY,
                    };
                    if !due {
                        continue;
                    }
                    view.sent.insert(key, (q, 0.0));
                    frame.people.push(PersonState {
                        id: p.id,
                        activity: net_activity(p.activity),
                        place,
                    });
                }
            }
            // the riders of our own bus, and the other players' people and riders: everybody
            // sees everybody's passengers
            let mut others: Vec<(PersonState, String)> = Vec::new();
            if let Some(h) = h_ref {
                for p in h.lan_riders() {
                    let Some((_, l, lh, seat)) = p.aboard else { continue };
                    if p.id > nw::MAX_ID || (lan_me_pos.map(|m| (m - at).truncate().length() > CAR_RADIUS).unwrap_or(true)) {
                        continue;
                    }
                    others.push((
                        PersonState {
                            id: p.id,
                            activity: net_activity(p.activity),
                            place: PersonPlace::Aboard {
                                bus: nw::PLAYER_BUS | lan.my_id,
                                x: l.x,
                                y: l.y,
                                z: l.z,
                                heading: lh as f32,
                                seat: seat.map(|s| s.min(254) as u8),
                            },
                        },
                        Humans::type_file(&p.ty),
                    ));
                }
            }
            for (peer, up) in &self.ups {
                if *peer == id {
                    continue;
                }
                let bus_near = players_at.iter().any(|(pp, pos)| pp == peer && (*pos - at).truncate().length() < CAR_RADIUS);
                for (pid, track) in &up.tracks {
                    let (Some(local), Some(last), Some(file)) = (up.ids.get(pid), track.samples.last(), up.files.get(pid)) else {
                        continue;
                    };
                    let near = match last.v.place {
                        PersonPlace::Foot { x, y, .. } => (DVec2::new(x, y) - at.truncate()).length() < PERSON_RADIUS,
                        PersonPlace::Aboard { .. } => bus_near,
                    };
                    if !near {
                        continue;
                    }
                    let mut st = last.v;
                    st.id = RELAYED | ((local - (1 << 29)) & 0x3F_FFFF);
                    if let PersonPlace::Foot { ref mut waiting, .. } = st.place {
                        *waiting = None;
                    }
                    others.push((st, file.clone()));
                }
            }
            for (st, file) in others {
                let key = (true, st.id);
                seen.insert(key);
                ids_hash ^= fnv(&(st.id as u64 | 1 << 40).to_le_bytes());
                if !view.described.contains_key(&key) {
                    view.described.insert(key, 0);
                    descs.push(Desc::Person { id: st.id, file });
                }
                let q = person_q(&st);
                let due = match view.sent.get(&key) {
                    None => true,
                    Some((last, age)) => *last != q || *age >= nw::STANDING_EVERY,
                };
                if !due {
                    continue;
                }
                view.sent.insert(key, (q, 0.0));
                frame.people.push(st);
            }
            if view.lights_acc >= nw::LIGHTS_EVERY {
                view.lights_acc = 0.0;
                let keys: Vec<u32> = self.departed.iter().filter_map(|k| u32::try_from(*k).ok()).collect();
                frame.parked = Some((keys.len() == self.departed.len(), keys));
                if let Some(t) = t_ref {
                    frame.lights = t
                        .light_states(at, LIGHT_RADIUS)
                        .into_iter()
                        .filter(|(o, _, _)| (0..=u32::MAX as i64).contains(o))
                        .map(|(object, time, held)| LightState { object, time, held })
                        .collect();
                }
            }
            // what went out of sight (or away) since the last frame
            let gone: Vec<Key> = view.sent.keys().filter(|k| !seen.contains(*k)).copied().collect();
            for k in gone {
                view.sent.remove(&k);
                view.gone.push((k, 1.0));
            }
            for g in view.gone.iter_mut() {
                g.1 -= nw::MOVING_EVERY;
            }
            view.gone.retain(|g| g.1 > 0.0 && !seen.contains(&g.0));
            frame.gone = view.gone.iter().map(|(k, _)| *k).collect();
            view.last_counts = (
                seen.iter().filter(|k| !k.0).count(),
                seen.iter().filter(|k| k.0).count(),
                frame.lights.len(),
            );
            view.ids_hash = ids_hash;
            for d in &descs {
                lan.send_desc(id, d);
            }
            if let Some(f) = self.trace.as_mut() {
                let ms = lan.stamp_ms();
                for c in &frame.cars {
                    let _ = writeln!(f, "host,{ms},{id},c,{},{:.3},{:.3},{:.3},{:.2}", c.id, c.x, c.y, c.z, c.heading);
                }
                for p in &frame.people {
                    if let PersonPlace::Foot { x, y, z, heading, .. } = p.place {
                        let _ = writeln!(f, "host,{ms},{id},p,{},{x:.3},{y:.3},{z:.3},{heading:.2}", p.id);
                    }
                }
            }
            let n = lan.send_world(id, &frame);
            view.bytes += n as u64;
        }
        self.log_t -= dt;
        if self.log_t <= 0.0 && omsi_cfg::flags::OMSI_DEBUG_LAN.is_set() {
            self.log_t = 4.0;
            for (peer, up) in &self.ups {
                let aboard = up.tracks.values().filter(|t| t.samples.last().map(|s| matches!(s.v.place, PersonPlace::Aboard { .. })).unwrap_or(false)).count();
                log::info!("LAN people of player {peer}: {} heard ({aboard} riding its bus), {} drawn here", up.tracks.len(), up.ids.len());
            }
            for (id, v) in &self.views {
                log::info!(
                    "LAN world → player {id}: {} cars, {} people, {} light programs in range; ids {:016X}; {:.1} KB/s ({} KB in all)",
                    v.last_counts.0,
                    v.last_counts.1,
                    v.last_counts.2,
                    v.ids_hash,
                    0.0f32.max(v.bytes as f32 / 1024.0 / lan_seconds(lan)),
                    v.bytes / 1024
                );
            }
        }
        if let Some(f) = self.trace.as_mut() {
            let _ = f.flush();
        }
    }

    // -----------------------------------------------------------------------------------
    // client

    #[allow(clippy::too_many_arguments)]
    fn client(
        &mut self,
        lan: &mut LanSession,
        dt: f32,
        args: &Args,
        world: Option<&World>,
        renderer: Option<&Renderer>,
        scene: Option<&mut Scene>,
        mut traffic: Option<&mut Traffic>,
        mut humans: Option<&mut Humans>,
        view: &mut SimView,
    ) {
        let (Some(world), Some(renderer), Some(scene)) = (world, renderer, scene) else {
            return;
        };
        let m = &mut self.mirror;
        // the host's world while we are in its session; our own again once it is over
        let want_on = lan.welcome.is_some() && lan.rejected.is_none();
        if want_on != m.on {
            m.on = want_on;
            if let Some(t) = traffic.as_deref_mut() {
                t.set_mirror(&mut view.traffic, world, renderer, scene, want_on);
            }
            if let Some(h) = humans.as_deref_mut() {
                h.set_mirror(want_on);
            }
            m.cars.clear();
            m.people.clear();
            m.drawn_cars.clear();
            m.drawn_people.clear();
            m.shown.clear();
            m.odometer.clear();
            log::info!(
                "LAN: {}",
                if want_on {
                    "drawing the host's traffic and people"
                } else {
                    "the session is over: simulating our own traffic and people again"
                }
            );
        }
        if !m.on {
            return;
        }
        let now = m.now_ms();
        for d in lan.take_descs() {
            let key = match &d {
                Desc::Car { id, .. } => (false, *id),
                Desc::Person { id, .. } => (true, *id),
            };
            m.wanted.remove(&key);
            m.descs.insert(key, d);
        }
        for (at, f) in lan.take_world() {
            // the host's clock against ours: the frame that took the shortest way sets it
            let arrived = at
                .checked_duration_since(*m.epoch.get_or_insert(at))
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            let off = f.host_ms as f64 - arrived;
            m.offset = Some(match m.offset {
                // (and drifts down slowly, so that a clock that runs a little slower is
                // followed; a jump of more than a second is taken at once)
                Some(o) if (off - o).abs() < 1000.0 => off.max(o - dt as f64 * 2.0),
                _ => off,
            });
            let ms = f.host_ms as f64;
            for c in f.cars {
                m.cars
                    .entry(c.id)
                    .or_insert_with(|| Track {
                        samples: Vec::new(),
                        heard: Instant::now(),
                    })
                    .push(ms, c);
            }
            for p in f.people {
                m.people
                    .entry(p.id)
                    .or_insert_with(|| Track {
                        samples: Vec::new(),
                        heard: Instant::now(),
                    })
                    .push(ms, p);
            }
            if let Some(t) = traffic.as_deref_mut() {
                for l in &f.lights {
                    // (the light programs ran on at the host while the datagram travelled)
                    let late = ((m.offset.unwrap_or(off) + arrived) - ms).max(0.0) / 1000.0;
                    t.set_light_state(l.object, l.time + if l.held { 0.0 } else { late }, l.held);
                }
            }
            if let Some((complete, keys)) = &f.parked {
                let keys: Vec<i64> = keys.iter().map(|k| *k as i64).collect();
                world.mirror_departed(renderer, scene, &keys, *complete);
            }
            for (person, id) in f.gone {
                if person {
                    m.people.remove(&id);
                } else {
                    m.cars.remove(&id);
                }
            }
        }
        let render_ms = m.play.step(now, now + m.offset.unwrap_or(0.0) - INTERP_DELAY, 500.0);
        // descriptions still missing: asked for (again)
        let mut want: Vec<EntityRef> = Vec::new();
        for (person, ids) in [
            (false, m.cars.keys().copied().collect::<Vec<_>>()),
            (true, m.people.keys().copied().collect::<Vec<_>>()),
        ] {
            for id in ids {
                let key = (person, id);
                if m.descs.contains_key(&key) {
                    continue;
                }
                let again = m
                    .wanted
                    .get(&key)
                    .map(|t| t.elapsed().as_secs_f32() > WANT_AGAIN)
                    .unwrap_or(true);
                // (the host sends a description with the first frame: ask only when that
                // was lost)
                let first_heard = if person {
                    m.people.get(&id).map(|t| t.samples.len())
                } else {
                    m.cars.get(&id).map(|t| t.samples.len())
                };
                if again && first_heard.unwrap_or(0) > 2 {
                    m.wanted.insert(key, Instant::now());
                    want.push(EntityRef { person, id });
                }
            }
        }
        if !want.is_empty() {
            lan.want(&want);
        }
        // vehicle types loaded in the background
        if m.loaded_tx.is_none() {
            let (tx, rx) = mpsc::channel();
            m.loaded_tx = Some(tx);
            m.loaded_rx = Some(rx);
        }
        if let Some(rx) = m.loaded_rx.as_ref() {
            while let Ok((path, ty)) = rx.try_recv() {
                m.loading.remove(&path);
                if ty.is_none() {
                    log::warn!("LAN: the host's vehicle {} cannot be loaded here", path.display());
                }
                m.types.insert(path, ty);
            }
        }
        // cars: forgotten, gone, new
        let forget = std::time::Duration::from_secs_f32(nw::FORGET_AFTER);
        m.cars.retain(|_, t| t.heard.elapsed() < forget);
        if let Some(t) = traffic.as_deref_mut() {
            let gone: Vec<u32> = m
                .drawn_cars
                .iter()
                .copied()
                .filter(|id| !m.cars.contains_key(id))
                .collect();
            for id in gone {
                t.remove_car(&mut view.traffic, world, renderer, scene, id as u64);
                m.drawn_cars.remove(&id);
                m.shown.remove(&id);
                m.odometer.remove(&id);
            }
            let new: Vec<u32> = m
                .cars
                .keys()
                .copied()
                .filter(|id| !m.drawn_cars.contains(id))
                .collect();
            for id in new {
                let Some(Desc::Car {
                    file,
                    scheme,
                    line,
                    destination,
                    ..
                }) = m.descs.get(&(false, id)).cloned()
                else {
                    continue;
                };
                let scheduled = !line.is_empty() || !destination.is_empty();
                let Some(path) = local_file(args, &file) else {
                    if m.types.insert(PathBuf::from(&file), None).is_none() {
                        log::warn!("LAN: the host's vehicle {file} is not installed here; it is left out");
                    }
                    continue;
                };
                let ty = match m.types.get(&path) {
                    Some(Some(ty)) => ty.clone(),
                    Some(None) => continue,
                    None => match t.loaded_type(&path) {
                        Some(ty) => {
                            m.types.insert(path.clone(), Some(ty.clone()));
                            ty
                        }
                        None => {
                            if m.loading.insert(path.clone()) {
                                let tx = m.loaded_tx.clone().unwrap();
                                let root = args.root.clone();
                                std::thread::spawn(move || {
                                    let ty = VehicleType::load_ai(&root, &path).ok().map(Arc::new);
                                    let _ = tx.send((path, ty));
                                });
                            }
                            continue;
                        }
                    },
                };
                let Some(first) = m.cars[&id].samples.last().map(|s| s.v) else {
                    continue;
                };
                t.add_mirror_car(
                    &mut view.traffic,
                    world,
                    renderer,
                    scene,
                    id as u64,
                    ty,
                    scheme.map(|s| s as usize),
                    scheduled,
                    DVec3::new(first.x, first.y, first.z),
                    first.heading as f64,
                );
                m.drawn_cars.insert(id);
            }
            // where each is now, and what its scripts make of it
            let index: HashMap<u64, usize> =
                t.cars.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
            let mut work: Vec<(usize, CarState, f64)> = Vec::new();
            for (id, track) in &m.cars {
                let Some(&i) = index.get(&(*id as u64)) else {
                    continue;
                };
                let Some((a, b, k)) = track.around(render_ms) else {
                    continue;
                };
                let mut c = *b;
                // (something that has stopped is not guessed on: it stands where it stopped)
                let k = if b.speed.abs() < 0.1 { k.min(1.0) } else { k };
                let kk = k.min(1.0);
                // (a jump of more than 30 m is a teleport, not a drive: no gliding)
                let far = (DVec2::new(b.x - a.x, b.y - a.y)).length() > 30.0;
                if !far {
                    c.x = a.x + (b.x - a.x) * k;
                    c.y = a.y + (b.y - a.y) * k;
                    c.z = a.z + (b.z - a.z) * kk;
                    c.heading = lerp_angle(a.heading as f64, b.heading as f64, k) as f32;
                    c.pitch = a.pitch + (b.pitch - a.pitch) * kk as f32;
                    c.bank = a.bank + (b.bank - a.bank) * kk as f32;
                    c.speed = a.speed + (b.speed - a.speed) * kk as f32;
                    c.steer = a.steer + (b.steer - a.steer) * kk as f32;
                }
                // the flags as the later frame had them once we are past its middle
                if k < 0.5 {
                    c.blinker = a.blinker;
                    c.brake = a.brake;
                    c.at_station = a.at_station;
                }
                work.push((i, c, far as u8 as f64));
            }
            let mut shown: Vec<(usize, u32)> = Vec::new();
            for (i, c, _) in &work {
                let car = &mut t.cars[*i];
                let before = car.vehicle.position;
                let now_p = DVec3::new(c.x, c.y, c.z);
                let moved = (now_p - before).truncate().length() as f32;
                let odo = m.odometer.entry(c.id).or_insert(0.0);
                if moved < 30.0 {
                    *odo += moved * c.speed.signum();
                }
                car.vehicle.position = now_p;
                car.vehicle.heading = c.heading as f64;
                car.vehicle.pitch = c.pitch;
                car.vehicle.bank = c.bank;
                car.state.speed = c.speed;
                car.state.blinker = c.blinker as i32;
                car.state.braking = c.brake;
                if let Some(b) = car.bus.as_mut() {
                    b.phase = if c.at_station == 1 {
                        crate::bus_service::Phase::Boarding
                    } else {
                        crate::bus_service::Phase::Running
                    };
                }
                car.body.steer = c.steer;
                shown.push((*i, c.id));
            }
            // the scripts (lamps, indicators, doors, wheels) run in parallel, as the
            // traffic's own do
            {
                use rayon::prelude::*;
                let odometer = &m.odometer;
                // (read off the buses before the parallel block takes `t.cars` mutably:
                // which side each stands at its stop is the host's own timetable state)
                let sides: HashMap<usize, f32> =
                    work.iter().map(|(i, _, _)| (*i, t.cars[*i].at_station_side())).collect();
                let frames: HashMap<usize, AiFrame> = work
                    .iter()
                    .map(|(i, c, _)| {
                        (
                            *i,
                            AiFrame {
                                speed: c.speed,
                                odometer: odometer.get(&c.id).copied().unwrap_or(0.0),
                                steer_deg: c.steer,
                                blinker: c.blinker as i32,
                                brake: c.brake,
                                lights: c.lights,
                                at_station: c.at_station as i32,
                                at_station_side: sides.get(i).copied().unwrap_or(0.0),
                                priority_warning: false,
                                // (the host's buses' engines are not on the wire: running)
                                engine_off: false,
                            },
                        )
                    })
                    .collect();
                let mut run: Vec<(&mut omsi_sim::VehicleInstance, &AiFrame)> = t
                    .cars
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(i, car)| frames.get(&i).map(|f| (&mut car.vehicle, f)))
                    .collect();
                run.par_iter_mut().for_each(|(v, f)| v.update_ai(dt, f));
            }
            // the timetable buses' displays
            for (i, id) in shown {
                let Some(Desc::Car {
                    line, destination, ..
                }) = m.descs.get(&(false, id))
                else {
                    continue;
                };
                if line.is_empty() || !destination.starts_with('#') || destination == "#-1" {
                    continue;
                }
                let want = (line.clone(), destination.clone());
                if m.shown.get(&id) == Some(&want) {
                    continue;
                }
                let car = &mut t.cars[i];
                let path = car.vehicle.ty.def.path.clone();
                let hof = m
                    .hofs
                    .entry(path)
                    .or_insert_with(|| crate::find_hof(args, world, &car.vehicle.ty))
                    .clone();
                if let Some(k) = car.vehicle.ty.program.str_var("Linie") {
                    car.vehicle.state.str_vars[k as usize] = line.clone();
                }
                show_car_destination(&mut car.vehicle, hof.as_deref(), line, destination);
                m.shown.insert(id, want);
            }
        }
        // people
        m.people.retain(|_, t| t.heard.elapsed() < forget);
        if let Some(h) = humans.as_deref_mut() {
            // the host's answers to our claims
            for (ids, granted) in lan.take_grants() {
                for id in ids {
                    if granted {
                        if h.grant(id) {
                            m.granted += 1;
                            log::info!("LAN: the host hands waiting passenger {id} over to our bus");
                        } else {
                            // the host has let them go (they are ours now, and the host
                            // says no more of them): a copy kept here would stand at the
                            // stop for good, one more after every stop
                            h.mirror_remove(id);
                        }
                        m.people.remove(&id);
                        m.drawn_people.remove(&id);
                    } else {
                        m.denied += 1;
                    }
                }
            }
            let gone: Vec<u32> = m
                .drawn_people
                .iter()
                .copied()
                .filter(|id| !m.people.contains_key(id))
                .collect();
            for id in gone {
                h.mirror_remove(id);
                m.drawn_people.remove(&id);
            }
            for (id, track) in &m.people {
                let Some((a, b, k)) = track.around(render_ms) else {
                    continue;
                };
                let pose = person_pose(a, b, k);
                if m.drawn_people.contains(id) {
                    if h.has_mirror(*id) {
                        h.mirror_set(*id, &pose);
                        continue;
                    }
                    // (taken away here with a bus that went: drawn again once it is back)
                    m.drawn_people.remove(id);
                }
                // a rider only once the bus is here to sit in
                if pose.aboard.map(|a| !h.knows_bus(a.0)).unwrap_or(false) {
                    continue;
                }
                let Some(Desc::Person { file, .. }) = m.descs.get(&(true, *id)) else {
                    continue;
                };
                let Some(ty) = h.type_by_file(file) else {
                    continue;
                };
                if h.mirror_add(&mut view.people, world, renderer, scene, *id, ty, &pose) {
                    m.drawn_people.insert(*id);
                }
            }
            let claims = h.take_claims();
            if !claims.is_empty() {
                log::info!("LAN: our bus stands at a stop: asking the host for waiting passengers {claims:?}");
                lan.claim(&claims);
            }
        }
        if let Some(f) = self.trace.as_mut() {
            self.trace_t -= dt;
            if self.trace_t <= 0.0 {
                self.trace_t = 0.1;
                if let Some(t) = traffic.as_deref() {
                    for c in t.cars.iter().filter(|c| m.drawn_cars.contains(&(c.id as u32))) {
                        let p = c.vehicle.position;
                        let _ = writeln!(f, "client,{render_ms:.0},{},c,{},{:.3},{:.3},{:.3},{:.2}", lan.my_id, c.id, p.x, p.y, p.z, c.vehicle.heading);
                    }
                }
                if let Some(h) = humans.as_deref() {
                    for (id, pos) in h.mirror_positions() {
                        let _ = writeln!(f, "client,{render_ms:.0},{},p,{id},{:.3},{:.3},{:.3},0", lan.my_id, pos.x, pos.y, pos.z);
                    }
                }
                let _ = f.flush();
            }
        }
        self.log_t -= dt;
        if self.log_t <= 0.0 && omsi_cfg::flags::OMSI_DEBUG_LAN.is_set() {
            self.log_t = 4.0;
            let m = &mut self.mirror;
            let ids_hash = m
                .cars
                .keys()
                .map(|id| fnv(&(*id as u64).to_le_bytes()))
                .chain(m.people.keys().map(|id| fnv(&(*id as u64 | 1 << 40).to_le_bytes())))
                .fold(0u64, |a, b| a ^ b);
            let rate = (lan.world_received - m.bytes_at) as f32 / 4.0 / 1024.0;
            m.bytes_at = lan.world_received;
            log::info!(
                "LAN world from the host: {} cars ({} drawn), {} people ({} drawn), ids {:016X}; {:.1} KB/s; drawn {:.0} ms behind the host; {} passengers handed to us, {} not",
                m.cars.len(),
                m.drawn_cars.len(),
                m.people.len(),
                m.drawn_people.len(),
                ids_hash,
                rate,
                INTERP_DELAY,
                m.granted,
                m.denied
            );
        }
    }
}

/// A person of one client passed on to the others has this bit in its id there (the host's
/// own people are numbered far below it).
const RELAYED: u32 = 0xC0_0000;

/// What decides whether a person's pose is sent again (see `quant`).
fn person_q(p: &PersonState) -> [i64; 4] {
    let q = match p.place {
        PersonPlace::Foot { x, y, z, heading, .. } => quant(x, y, z, heading as f64),
        PersonPlace::Aboard { bus, x, y, z, heading, .. } => {
            quant(x as f64, y as f64, z as f64, heading as f64 + (bus & 0xFFFF) as f64 * 1000.0)
        }
    };
    let act = match p.activity {
        NetActivity::Stand => 0,
        NetActivity::Walk => 1,
        NetActivity::Sit => 2,
        NetActivity::Run => 3,
    };
    [q[0], q[1], q[2], q[3] ^ (act << 40)]
}

fn lan_seconds(lan: &LanSession) -> f32 {
    (lan.world_ms() as f32 / 1000.0).max(1.0)
}

/// Where a person is at `k` between two of the host's poses.
fn person_pose(a: &PersonState, b: &PersonState, k: f64) -> MirrorPose {
    match (a.place, b.place) {
        (
            PersonPlace::Foot {
                x: ax,
                y: ay,
                z: az,
                heading: ah,
                ..
            },
            PersonPlace::Foot {
                x,
                y,
                z,
                heading,
                speed,
                waiting,
            },
        ) => {
            let jump = DVec2::new(x - ax, y - ay).length() > 8.0;
            let k = if jump { 1.0 } else if speed < 0.05 { k.min(1.0) } else { k };
            let pos = DVec3::new(ax + (x - ax) * k, ay + (y - ay) * k, az + (z - az) * k.min(1.0));
            let h = lerp_angle(ah as f64, heading as f64, k.min(1.0));
            let dir = DVec2::new(x - ax, y - ay);
            let vel = if speed > 0.05 && dir.length() > 1.0e-3 {
                dir.normalize() * speed as f64
            } else {
                DVec2::ZERO
            };
            MirrorPose {
                pos,
                heading: h,
                vel,
                activity: sim_activity(b.activity),
                aboard: None,
                waiting: waiting.map(|(s, k)| (s, k as usize)),
            }
        }
        (
            _,
            PersonPlace::Aboard {
                bus,
                x,
                y,
                z,
                heading,
                seat,
            },
        ) => MirrorPose {
            pos: DVec3::ZERO,
            heading: 0.0,
            vel: DVec2::ZERO,
            activity: sim_activity(b.activity),
            aboard: Some((
                if bus & nw::PLAYER_BUS != 0 {
                    crate::humans::remote_bus_id(bus & nw::MAX_ID)
                } else {
                    bus as u64
                },
                Vec3::new(x, y, z),
                heading as f64,
                seat.map(|s| s as usize),
            )),
            waiting: None,
        },
        (_, PersonPlace::Foot { .. }) => person_pose(b, b, 1.0),
    }
}

fn describe_car(args: &Args, c: &crate::traffic::AiCar) -> Desc {
    let (line, destination) = if c.is_bus() {
        car_display(&c.vehicle)
    } else {
        (String::new(), String::new())
    };
    Desc::Car {
        id: c.id as u32,
        file: relative_file(&c.vehicle.ty.def.path, &args.root),
        // (the paint scheme it is drawn with: a host's cars are all its own, never a LAN
        // mirror's, whose `scheme` is unset)
        scheme: c.scheme.map(|s| s.min(255) as u8),
        line,
        destination,
    }
}

fn describe(
    args: &Args,
    traffic: Option<&Traffic>,
    humans: Option<&Humans>,
    r: EntityRef,
) -> Option<Desc> {
    if r.person {
        let p = humans?.lan_people_by_id(r.id)?;
        Some(Desc::Person {
            id: r.id,
            file: Humans::type_file(&p),
        })
    } else {
        let c = traffic?.cars.iter().find(|c| c.id == r.id as u64)?;
        Some(describe_car(args, c))
    }
}

#[cfg(test)]
mod tests {
    use super::{relative_to_roots, PlayClock};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_file_is_named_relative_to_the_deepest_root_holding_it() {
        // the content folder first, the OMSI 2 folder inside it after (a server's layout)
        let roots = [PathBuf::from("/srv/openOMSI"), PathBuf::from("/srv/openOMSI/OMSI 2")];
        let golf = Path::new("/srv/openOMSI/OMSI 2/Vehicles/VW_Golf_2/ai_vw_golf_2.bus");
        assert_eq!(relative_to_roots(golf, &roots).as_deref(), Some("Vehicles/VW_Golf_2/ai_vw_golf_2.bus"));
        // a mod in the content folder itself stays relative to that
        let m = Path::new("/srv/openOMSI/Vehicles/Mod/mod.bus");
        assert_eq!(relative_to_roots(m, &roots).as_deref(), Some("Vehicles/Mod/mod.bus"));
        assert_eq!(relative_to_roots(Path::new("/elsewhere/x.bus"), &roots), None);
    }

    /// The host's bus to U Ruhleben (row 2) shows U Ruhleben on ours too, not Machandelweg
    /// (row 1), whose sign has RUHLEBEN on its second line.
    #[test]
    fn a_hosts_timetable_bus_shows_the_row_it_was_given() {
        let t = |code: i32, id: &str, s: &[&str]| omsi_vehicle::hof::Terminus { code, texture_id: id.into(), strings: s.iter().map(|x| x.to_string()).collect(), ..Default::default() };
        let hof = omsi_vehicle::Hof { termini: vec![t(0, "Empty", &[""]), t(194, "Machandelweg", &["MACHANDELWEG", "RUHLEBEN"]), t(282, "U Ruhleben", &["RUHLEBEN", "U-BAHNHOF"])], ..Default::default() };
        let mut v = crate::schedule::tests::script_test_vehicle("{frame}\n{end}\n", "AI_target_index\nIBIS_TerminusIndex\nIBIS_TerminusCode\n", "SetLineTo\n");
        super::show_car_destination(&mut v, Some(&hof), "5", "#2");
        assert_eq!((v.var("AI_target_index"), v.var("IBIS_TerminusCode")), (Some(2.0), Some(282.0)));
        assert_eq!(v.str_var("SetLineTo"), "5");
        // no such row, or no depot file: nothing changes
        super::show_car_destination(&mut v, Some(&hof), "5", "#7");
        super::show_car_destination(&mut v, None, "5", "#1");
        assert_eq!(v.var("AI_target_index"), Some(2.0));
    }

    #[test]
    fn the_moment_drawn_follows_a_changed_delay_smoothly() {
        let mut c = PlayClock::default();
        let mut now = 0.0;
        let mut t = c.step(now, now - 120.0, 500.0);
        // a slow frame of theirs: the delay grows by 70 ms at once
        let mut steps = Vec::new();
        for _ in 0..100 {
            now += 16.0;
            let n = c.step(now, now - 190.0, 500.0);
            steps.push(n - t);
            t = n;
        }
        // never back, never on more than a few per cent faster or slower than the clock
        assert!(steps.iter().all(|d| *d > 16.0 * 0.93 && *d < 16.0 * 1.07), "{steps:?}");
        assert!((t - (now - 190.0)).abs() < 1e-6, "caught up: {}", t - (now - 190.0));
        // a new session: taken at once
        now += 16.0;
        assert_eq!(c.step(now, now + 5000.0, 500.0), now + 5000.0);
    }
}
