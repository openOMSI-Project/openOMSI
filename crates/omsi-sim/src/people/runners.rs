//! Somebody late for the bus. While a bus is on its way to a stop near the player and still
//! 150 m off or more, now and then somebody is put out on the pavement of the stop's street,
//! out of the player's sight, walking along to the stop - from before it (they start to run
//! as the bus overtakes them) or from beyond it (they run once it stands at the stop). They
//! run along the pavement and only the last metres straight to the doors. A timetable bus
//! waits a few seconds for them (`holds_bus`); the player's bus does not have to. With
//! nobody put out, somebody else walking near runs - or, for a timetable bus, somebody
//! comes from behind it out of sight. Left behind, they stand a moment and walk back.

use super::*;

/// Chance per bus standing at a stop near the player (`OMSI_RUNNER_CHANCE`).
pub const RUNNER_CHANCE: f32 = 0.05;
/// How far along the pavement from the stop somebody late starts (m).
pub const RUNNER_START: (f32, f32) = (25.0, 60.0);
/// A bus at least this far off its next stop has somebody put out for it there (m).
pub const RUNNER_PLACE_FROM: f64 = 150.0;
/// The speed a bus is taken to come at, for when it gets to the stop (m/s).
pub const RUNNER_BUS_SPEED: f64 = 6.0;
/// Where somebody put out is meant to be when the bus gets to the stop: beyond it, coming
/// towards the bus, or before it, the bus having overtaken them (m from the stop).
pub const RUNNER_BEYOND_AT: (f32, f32) = (25.0, 45.0);
pub const RUNNER_BEFORE_AT: (f32, f32) = (35.0, 60.0);
/// The longest walk along the pavement somebody is put out at the start of (m).
pub const RUNNER_ROUTE_MAX: f32 = 160.0;
/// Seconds a roll is kept after its bus last looked to be coming: the bus's next stop
/// flickers, and a forgotten roll put a second person out for the same bus.
pub const RUNNER_ROLL_KEPT: f64 = 30.0;
/// Somebody walking within this of the stop can run straight to a bus standing there (m).
pub const RUNNER_NEAR: f64 = 50.0;
/// How far to either side of the pavement line at the stop the walk to it may go (m): not
/// into a yard, a building or a side street.
pub const RUNNER_STREET: f64 = 12.0;
/// Running along the pavement, this close to the stop they make for the doors (m).
pub const RUNNER_TO_DOORS: f64 = 15.0;
/// The bus overtaking somebody before the stop: within this of them they start to run (m).
pub const RUNNER_OVERTAKEN: f64 = 25.0;
/// The pace of somebody hurrying for the bus (m/s).
pub const RUNNER_SPEED: (f32, f32) = (3.0, 3.6);
/// Seconds somebody hurrying keeps a timetable bus at the stop, at most.
pub const RUNNER_HOLD_MAX: f32 = 12.0;
/// Seconds somebody left behind stands looking after the bus.
pub const RUNNER_MISSED: f32 = 2.0;

/// Logs with `OMSI_DEBUG_PAX`.
macro_rules! trace {
    ($($arg:tt)*) => {
        if debug_pax() {
            log::info!($($arg)*)
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LatePhase {
    /// Walking up to the stop at their own pace; the bus is not standing there yet.
    Approach,
    /// The bus stands at the stop: hurrying for it.
    Hurry,
    /// The bus left without them: the seconds left standing there.
    Missed(f32),
}

/// A bus's roll at a stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Roll {
    /// Somebody is to run for it; nobody put out yet.
    Due,
    /// Pedestrian `id` was put out on the stop's pavement while it was far off.
    Placed(u32),
    /// Pedestrian `id`, put out for it, runs for it along the pavement.
    Running(u32),
    /// Nobody (more) for this visit.
    Done,
}

/// A roll and what goes with it (`PeopleSim::runner_rolls`): when its bus last looked to be
/// coming (`RUNNER_ROLL_KEPT`), and why nobody could be put out yet (`OMSI_DEBUG_PAX`).
#[derive(Debug, Clone, Copy)]
pub struct RollEntry {
    pub roll: Roll,
    pub kept: f64,
    pub why: &'static str,
}

/// Where somebody late comes from: a pedestrian already walking near (by index), or a
/// place behind the bus out of sight.
#[derive(Debug, Clone, Copy)]
enum Source {
    Stroller(usize),
    Spawn(DVec3),
}

/// What makes a passenger somebody late (`Pax::late`).
#[derive(Debug, Clone, Copy)]
pub struct Late {
    pub phase: LatePhase,
    /// Their own walking pace, and the pace they hurry at.
    pub walk: f32,
    pub run: f32,
    /// Seconds spent hurrying (a timetable bus waits `RUNNER_HOLD_MAX` at most).
    pub hurried: f32,
    /// The bus has stood at the stop since they began to hurry: it leaving now leaves them
    /// behind (before, it was still pulling in).
    pub stood: bool,
}

impl Late {
    /// Whether they keep a timetable bus at the stop now.
    pub fn holds(&self) -> bool {
        self.phase == LatePhase::Hurry && self.hurried < RUNNER_HOLD_MAX
    }
}

/// The chance per bus (`OMSI_RUNNER_CHANCE`, else `RUNNER_CHANCE`), 0..1.
pub fn runner_chance() -> f32 {
    omsi_cfg::flags::OMSI_RUNNER_CHANCE.parse::<f32>().unwrap_or(RUNNER_CHANCE).clamp(0.0, 1.0)
}

/// `t` (0..1) of the way through `range`.
fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t
}

/// Bus `b` of this frame's.
fn bus_of<'a>(buses: &'a [BusNow], bus_ix: &HashMap<BusId, usize>, b: BusId) -> Option<&'a BusNow> {
    bus_ix.get(&b).map(|k| &buses[*k])
}

/// How far the pavement (lane `lane`) 10 m from the stop at `s0` lies along `fwd`, that way
/// along the lane (`sign`): which way is beyond the stop.
fn along(net: &Network, lane: usize, s0: f32, lane_len: f32, stop_pos: DVec3, fwd: DVec2, sign: f32) -> f64 {
    (net.lanes[lane].at((s0 + sign * 10.0).clamp(0.0, lane_len)).0 - stop_pos).truncate().dot(fwd)
}

/// Whether a bus listed at stop `id` takes somebody waiting there for `dest` with line record
/// `line` (as `bus_for` would choose it, `fit`).
fn bus_takes(id: i64, stop: &PaxStop, bn: &BusNow, dest: Option<&str>, line: Option<usize>) -> bool {
    if bn.cabin.entries.is_empty() {
        return false;
    }
    match line.and_then(|k| stop.lines.get(k)) {
        // no line record: the first bus listed
        None => stop.buses.first().is_some_and(|b| b.0 == bn.id),
        Some((_, termini)) => bn.terminus.as_deref().is_some_and(|t| fit(id, stop, dest, termini, t, &bn.takes).is_some()),
    }
}

/// A walk along the pavement network from `dist` m away to the stop, the stop lying on lane
/// `lane` at `s0`: out from the stop along the lane in direction `sign` and on at the
/// junctions (`PedNet::next_leg`, choosing by `pick`), then the legs the other way round.
/// Where it starts, the walking heading there, and the legs; none where the pavement ends or
/// leaves the line of the pavement at the stop (along `fwd`) by more than `RUNNER_STREET`.
#[allow(clippy::too_many_arguments)]
pub fn route_to_stop(ped: &PedNet, net: &Network, lane: usize, s0: f32, sign: f32, dist: f32, pick: u64, fwd: DVec2) -> Option<(DVec3, f64, Vec<Leg>)> {
    let stop_pos = net.lanes.get(lane)?.at(s0).0;
    let off_street = |l: &Leg| {
        let lane = &net.lanes[l.lane];
        let n = ((l.len() / 5.0).ceil() as usize).max(1);
        (0..=n).any(|k| (lane.at(l.a + (l.b - l.a) * k as f32 / n as f32).0 - stop_pos).truncate().perp_dot(fwd).abs() > RUNNER_STREET)
    };
    let len0 = net.lanes.get(lane)?.length();
    let mut leg = Leg { lane, a: s0, b: if sign > 0.0 { len0 } else { 0.0 } };
    let mut out: Vec<Leg> = Vec::new();
    let mut left = dist;
    loop {
        if leg.len() >= left {
            out.push(Leg { b: leg.dist(left), ..leg });
            break;
        }
        left -= leg.len();
        out.push(leg);
        if out.len() > 8 {
            return None;
        }
        let node = ped.end_node(net, &leg)?;
        leg = ped.next_leg(net, node, leg.lane, pick)?;
    }
    if out.iter().any(off_street) {
        return None;
    }
    let legs: Vec<Leg> = out.iter().rev().map(|l| Leg { lane: l.lane, a: l.b, b: l.a }).collect();
    let (q, h) = legs[0].at(net, 0.0);
    Some((q, h, legs))
}

impl PeopleSim {
    /// Somebody late for a bus at a stop near the player. Once the bus is on its way to the
    /// stop and still 150 m off or more it has its roll (`runner_chance`), and a pedestrian is
    /// put out out of the player's sight, walking along to the stop (`place_walker`). Once the
    /// bus stands in the stop's box they run for it (`run_along`, `spawn_runner`). A bus not
    /// known to be coming has its roll when it is listed at the stop.
    pub fn runners_tick(&mut self, world: &dyn World, net: &Network, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        if self.mirror || self.avatar_only {
            return;
        }
        // the buses on their way to a stop, how far off, and whether the stop is near the
        // player (only then is anybody put out; a roll stays while its bus heads there)
        let heading: Vec<(i64, BusId, f64, bool)> = buses
            .iter()
            .filter_map(|bn| {
                let next = bn.next_stop.as_ref()?;
                let s = self.stops.get(&next.id)?;
                Some((next.id, bn.id, (bn.pos - s.pos).truncate().length(), s.near))
            })
            .collect();
        // (a roll whose bus neither comes to the stop, nor is listed or near there, is
        // forgotten after `RUNNER_ROLL_KEPT`; whoever runs for it walks on)
        let (stops, now) = (&self.stops, self.time);
        let mut gone: Vec<u32> = Vec::new();
        self.runner_rolls.retain(|(s, b), e| {
            let keep = heading.iter().any(|h| h.0 == *s && h.1 == *b)
                || stops
                    .get(s)
                    .is_some_and(|st| st.buses.iter().any(|x| x.0 == *b) || bus_ix.get(b).is_some_and(|k| (buses[*k].pos - st.pos).length() < 120.0));
            if keep {
                e.kept = now;
            }
            let still = now - e.kept < RUNNER_ROLL_KEPT;
            if let (false, Roll::Running(pid)) = (still, e.roll) {
                gone.push(pid);
            }
            still
        });
        for pid in gone {
            self.stop_running(pid);
        }
        let chance = runner_chance();
        if chance <= 0.0 {
            return;
        }
        for &(id, bus, d, near) in &heading {
            if !near || d < RUNNER_PLACE_FROM {
                continue;
            }
            let roll = match self.runner_rolls.get(&(id, bus)) {
                Some(e) => e.roll,
                None => {
                    let roll = if (self.rand_f() as f32) < chance { Roll::Due } else { Roll::Done };
                    self.new_roll((id, bus), roll)
                }
            };
            // (somewhere in sight now, perhaps not a frame later as the bus comes on)
            let Some(bn) = bus_of(buses, bus_ix, bus).filter(|_| roll == Roll::Due) else {
                continue;
            };
            match self.place_walker(world, net, id, bn, d) {
                Ok(pid) => self.set_roll((id, bus), Roll::Placed(pid)),
                Err(why) => self.runner_rolls.get_mut(&(id, bus)).unwrap().why = why,
            }
        }
        self.run_along(world, net, buses, bus_ix);
        let mut ids: Vec<i64> = self.stops.iter().filter(|(_, s)| s.near && !s.buses.is_empty()).map(|(k, _)| *k).collect();
        ids.sort_unstable();
        for id in ids {
            for (bus, in_box) in self.stops[&id].buses.clone() {
                let Some(bn) = bus_of(buses, bus_ix, bus) else {
                    continue;
                };
                let standing = bn.speed.abs() < 0.5;
                let roll = match self.runner_rolls.get(&(id, bus)) {
                    // (those put out run along the pavement, `run_along`)
                    Some(RollEntry { roll: Roll::Done | Roll::Placed(_) | Roll::Running(_), .. }) => continue,
                    Some(e) => *e,
                    // (not known to be coming: the roll now - one already standing when the
                    // stop came near has nobody running up)
                    None => {
                        let roll = if standing || self.rand_f() as f32 >= chance { Roll::Done } else { Roll::Due };
                        self.new_roll((id, bus), roll);
                        continue;
                    }
                };
                if !(standing && in_box) {
                    continue;
                }
                self.set_roll((id, bus), Roll::Done);
                let why = if roll.why.is_empty() { "it was never 150 m off" } else { roll.why };
                trace!("t={:.1} nobody was put out for {bus:?} at stop {id}: {why}", self.time);
                self.spawn_runner(world, net, id, bus, None, false, buses, bus_ix);
            }
        }
    }

    /// A new roll, kept from now.
    fn new_roll(&mut self, key: (i64, BusId), roll: Roll) -> Roll {
        self.runner_rolls.insert(key, RollEntry { roll, kept: self.time, why: "" });
        roll
    }

    /// Where a roll stands now.
    fn set_roll(&mut self, key: (i64, BusId), roll: Roll) {
        if let Some(e) = self.runner_rolls.get_mut(&key) {
            e.roll = roll;
        }
    }

    /// The people put out: somebody before the stop starts to run as their bus overtakes
    /// them, somebody beyond it once it stands at the stop - along the pavement, at a run,
    /// until they are `RUNNER_TO_DOORS` off the stop and make for the doors as a passenger
    /// (`spawn_runner`). A bus that drives on, or leaves, before they are there: they walk on.
    fn run_along(&mut self, world: &dyn World, net: &Network, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let mut ours: Vec<((i64, BusId), Roll)> =
            self.runner_rolls.iter().filter(|(_, e)| matches!(e.roll, Roll::Placed(_) | Roll::Running(_))).map(|(k, e)| (*k, e.roll)).collect();
        ours.sort_by_key(|((s, b), _)| (*s, format!("{b:?}")));
        for ((id, bus), roll) in ours {
            let (Roll::Placed(pid) | Roll::Running(pid)) = roll else {
                continue;
            };
            let Some(j) = self.people.iter().position(|p| p.id == pid && matches!(p.state, State::Strolling(_))) else {
                // (gone, or somebody else now)
                self.set_roll((id, bus), Roll::Done);
                continue;
            };
            let Some(bn) = bus_of(buses, bus_ix, bus) else {
                continue;
            };
            let Some((stop_pos, in_box)) = self.stops.get(&id).map(|s| (s.pos, s.buses.iter().any(|x| x.0 == bus && x.1))) else {
                continue;
            };
            let standing = in_box && bn.speed.abs() < 0.5;
            let at = self.people[j].position;
            let to_stop = (at - stop_pos).truncate().length();
            match roll {
                // (there before the bus, which came slower than it was taken to: they wait
                // there as anybody, and hurry to its doors when it stands)
                Roll::Placed(_) if to_stop < RUNNER_TO_DOORS => {
                    self.set_roll((id, bus), Roll::Done);
                    trace!("t={:.1} pax {} put out for {bus:?} is at stop {id} before it", self.time, self.people[j].label());
                    if self.spawn_runner(world, net, id, bus, Some(j), false, buses, bus_ix).is_none() {
                        self.stop_running(pid);
                    }
                }
                Roll::Placed(_) => {
                    let rel = (at - bn.pos).truncate();
                    if standing || (rel.dot(bn.fwd()) < 2.0 && rel.length() < RUNNER_OVERTAKEN) {
                        self.people[j].pace = lerp(RUNNER_SPEED, self.rand_f() as f32) as f64;
                        self.set_roll((id, bus), Roll::Running(pid));
                        let why = if standing { "it stands at the stop" } else { "it overtakes them" };
                        trace!("t={:.1} pax {} runs along the pavement for {bus:?}, {to_stop:.0} m off stop {id}: {why}", self.time, self.people[j].label());
                    }
                }
                _ => {
                    // (it drove on past the stop, or left it, before they were there)
                    if !self.listed_at(id, bus) && (bn.pos - stop_pos).truncate().dot(bn.fwd()) > 10.0 {
                        self.stop_running(pid);
                        self.set_roll((id, bus), Roll::Done);
                    } else if to_stop < RUNNER_TO_DOORS || (standing && to_stop < 2.0 * RUNNER_TO_DOORS) {
                        self.set_roll((id, bus), Roll::Done);
                        let pace = self.people[j].pace as f32;
                        if self.spawn_runner(world, net, id, bus, Some(j), true, buses, bus_ix).is_none() {
                            self.stop_running(pid);
                        } else if let Some(p) = self.pax_mut(j) {
                            // (at the pace they ran along the pavement)
                            p.walk_speed = pace;
                            if let Some(l) = p.late.as_mut() {
                                l.run = pace;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Somebody who ran for a bus along the pavement and need not any more: on at a walk.
    fn stop_running(&mut self, pid: u32) {
        let pace = 1.0 + self.rand_f() * 0.2;
        if let Some(p) = self.people.iter_mut().find(|p| p.id == pid && matches!(p.state, State::Strolling(_))) {
            p.pace = pace;
        }
    }

    /// A pedestrian put out for a bus `bn` `d_bus` m off stop `id`, out of the player's sight,
    /// walking along the pavement to the stop: from beyond it, coming towards the bus, or
    /// from before it, the bus overtaking them - either, as it falls, and the other when that
    /// one does not do. Put out as far along as they walk until the bus gets there (at
    /// `RUNNER_BUS_SPEED`), to be `RUNNER_BEYOND_AT` / `RUNNER_BEFORE_AT` from the stop then.
    /// Their id, or why nobody (tried again next frame).
    fn place_walker(&mut self, world: &dyn World, net: &Network, id: i64, bn: &BusNow, d_bus: f64) -> Result<u32, &'static str> {
        let (lane, s0, stop_pos, heading) =
            self.stops.get(&id).and_then(|s| s.lane.map(|(l, s0)| (l, s0, s.pos, s.heading))).ok_or("the stop has no pavement")?;
        let lane_len = net.lanes.get(lane).ok_or("the stop has no pavement")?.length();
        // (beyond the stop by the stop's own way: the bus's heading this far off, before a
        // bend, put "beyond" behind it as often as not)
        let fwd = DVec2::new(heading.to_radians().sin(), heading.to_radians().cos());
        let probe = |sign| along(net, lane, s0, lane_len, stop_pos, fwd, sign);
        let beyond = if probe(1.0) >= probe(-1.0) { 1.0f32 } else { -1.0 };
        let pace = 1.0 + self.rand_f() * 0.2;
        let eta = (d_bus / RUNNER_BUS_SPEED) as f32;
        let r = self.rand_f() as f32;
        let pick = self.rand();
        let sides = if self.rand_f() < 0.5 { [beyond, -beyond] } else { [-beyond, beyond] };
        let mut why = "";
        for sign in sides {
            let at = lerp(if sign == beyond { RUNNER_BEYOND_AT } else { RUNNER_BEFORE_AT }, r);
            let mut dist = (at + pace as f32 * eta).min(RUNNER_ROUTE_MAX);
            if sign != beyond {
                // (ahead of the bus, which overtakes them: not behind it already)
                dist = dist.min(d_bus as f32 - 30.0);
                if dist < at + 10.0 {
                    why = "the bus is too close to start before the stop";
                    continue;
                }
            }
            // (a few ways on at the junctions: one may turn off the street, another not)
            let route = self.ped.as_ref().and_then(|ped| (0..4u64).find_map(|k| route_to_stop(ped, net, lane, s0, sign, dist, pick.wrapping_add(k), fwd)));
            let Some((q, h, legs)) = route else {
                why = "the pavement ends or leaves the street";
                continue;
            };
            // (to the pavement at the stop: the stop's object stands out at the kerb)
            why = if crosses_street(net, q.truncate(), net.lanes[lane].at(s0).0.truncate()) {
                "the walk starts across the street"
            } else if !world.has_ground(q.x, q.y) {
                "the walk starts where nothing is loaded"
            } else if self.seen(q) {
                "both ways are in sight"
            } else {
                ""
            };
            if !why.is_empty() {
                continue;
            }
            let side = 0.3 + self.rand_f() as f32 * 0.4;
            let Some(i) = self.spawn(world, q, h, State::Strolling(PedWalk::new(legs, true, side))) else {
                return Err("no room for anybody more");
            };
            self.people[i].activity = Activity::Walk;
            self.people[i].pace = pace;
            let way = if sign == beyond { "beyond" } else { "before" };
            trace!(
                "t={:.1} pax {} put out {dist:.0} m along the pavement {way} stop {id} for {:?}, {d_bus:.0} m off",
                self.time,
                self.people[i].label(),
                bn.id
            );
            return Ok(self.people[i].id);
        }
        Err(why)
    }

    /// A passenger for bus `bus` at stop `id`, running for it (`running`) or walking up to a
    /// free waiting place: the one put out for it (`placed`) - else somebody walking near -
    /// else, for a timetable bus, somebody from behind it out of sight (`start_behind`).
    /// Nobody when the stop is full, has no pavement, no destination drawn is one the bus goes
    /// to, or nobody is there to run.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_runner(
        &mut self,
        world: &dyn World,
        net: &Network,
        id: i64,
        bus: BusId,
        placed: Option<usize>,
        running: bool,
        buses: &[BusNow],
        bus_ix: &HashMap<BusId, usize>,
    ) -> Option<usize> {
        let Some((lane, s0, stop_pos, full)) = self.stops.get(&id).and_then(|s| s.lane.map(|(l, s0)| (l, s0, s.pos, s.taken.iter().all(|t| *t)))) else {
            trace!("t={:.1} nobody late for {bus:?} at stop {id}: the stop has no pavement", self.time);
            return None;
        };
        if full {
            trace!("t={:.1} nobody late for {bus:?} at stop {id}: every waiting place is taken", self.time);
            return None;
        }
        let bn = bus_of(buses, bus_ix, bus)?;
        // (a few draws: the stop's destinations go with every line calling there - and with
        // none of those for this bus, the first of the stop's line records it takes)
        let mut drawn = None;
        for _ in 0..4 {
            let (dest, line) = self.draw_dest(id);
            if bus_takes(id, &self.stops[&id], bn, dest.as_deref(), line) {
                drawn = Some((dest, line));
                break;
            }
        }
        if drawn.is_none() {
            let s = &self.stops[&id];
            drawn = (0..s.lines.len()).find(|&k| bus_takes(id, s, bn, Some(&s.lines[k].0), Some(k))).map(|k| (Some(s.lines[k].0.clone()), Some(k)));
        }
        let Some((dest, line)) = drawn else {
            trace!("t={:.1} nobody late for {bus:?} at stop {id}: no destination drawn is one this bus goes to", self.time);
            return None;
        };
        let lane_len = net.lanes.get(lane)?.length();
        let fwd = bn.fwd();
        let jitter = self.rand_f() as f32 * 5.0;
        // (distances to the pavement at the stop, see `walks_near`; anybody but the one put
        // out not right at the stop, as if they had been waiting there)
        let pave = net.lanes[lane].at(s0).0;
        let near = placed
            .filter(|&j| self.walks_near(net, j, pave))
            .or_else(|| self.stroller_near(net, pave).filter(|&j| (self.people[j].position - pave).truncate().length() > 5.0))
            .map(Source::Stroller);
        // (not for the player's bus: whoever runs for it was there to be seen before)
        let from = near
            .or_else(|| (bus != BusId::Player).then(|| self.start_behind(world, net, lane, s0, lane_len, stop_pos, fwd, jitter).map(Source::Spawn)).flatten());
        let Some(from) = from else {
            trace!(
                "t={:.1} nobody late for {bus:?} at stop {id}: nobody walking near, and every place behind is in sight, across the street or not loaded",
                self.time
            );
            return None;
        };
        let k = self.take_spot(id)?;
        let walk = 1.1 + (self.rand_f() as f32 * 2.0 - 1.0) * 0.2;
        let run = lerp(RUNNER_SPEED, self.rand_f() as f32);
        let mut pax = Pax::new(walk, self.rand_f());
        pax.stop = Some(id);
        pax.spot = Some(k);
        pax.dest = dest;
        pax.line = line;
        pax.bus = Some(bus);
        let phase = if running { LatePhase::Hurry } else { LatePhase::Approach };
        pax.late = Some(Late { phase, walk, run, hurried: 0.0, stood: false });
        if running {
            pax.walk_speed = run;
        }
        let i = match from {
            Source::Stroller(j) => {
                // (on from where they walk, the way they face)
                let p = &mut self.people[j];
                pax.pos = p.position;
                pax.yaw = p.heading.to_radians();
                p.state = State::Pax(Box::new(pax));
                j
            }
            Source::Spawn(start) => {
                let yaw = yaw_of((stop_pos - start).truncate());
                pax.pos = start;
                pax.yaw = yaw;
                let Some(i) = self.spawn(world, start, yaw.to_degrees(), State::Pax(Box::new(pax))) else {
                    self.free_spot(id, k);
                    return None;
                };
                i
            }
        };
        // (already running: on to the gather point and the doors)
        let task = if running { Task::ToBus } else { Task::WalkingToBusstop };
        self.set_task(i, task, buses, bus_ix, world);
        let how = match from {
            Source::Stroller(j) if Some(j) == placed => "put out for it, runs",
            Source::Stroller(_) => "walking near, runs",
            Source::Spawn(_) => "comes from behind",
        };
        trace!(
            "t={:.1} pax {} late for {bus:?} at stop {id}, {how}, {:.0} m away",
            self.time,
            self.people[i].label(),
            (stop_pos - self.people[i].position).truncate().length()
        );
        Some(i)
    }

    /// Whether person `j` strolls the pavement within `RUNNER_NEAR` of `pave`, the point of
    /// the pavement at a stop, on its side of the street: somebody who could run for a bus
    /// there. (Not to the stop's object: it stands out at the kerb, and the way to it from the
    /// pavement crossed the lane's middle.)
    fn walks_near(&self, net: &Network, j: usize, pave: DVec3) -> bool {
        let Some(p) = self.people.get(j) else {
            return false;
        };
        if p.puppet.is_some() || p.remote || !matches!(p.state, State::Strolling(_)) {
            return false;
        }
        (p.position - pave).truncate().length() < RUNNER_NEAR
            && (p.position.z - pave.z).abs() < 3.0
            && !crosses_street(net, p.position.truncate(), pave.truncate())
    }

    /// The nearest of the people who could run for a bus at a stop whose pavement is at
    /// `pave` (`walks_near`).
    fn stroller_near(&self, net: &Network, pave: DVec3) -> Option<usize> {
        let dist = |j: usize| (self.people[j].position - pave).truncate().length();
        (0..self.people.len()).filter(|&j| self.walks_near(net, j, pave)).min_by(|&a, &b| dist(a).total_cmp(&dist(b)))
    }

    /// A place 25 to 60 m along the stop's pavement (lane `lane` at `s0`) out of the
    /// player's sight, on the stop's side of the street: behind the bus first (against
    /// `fwd`), where the driver sees them come in the mirror.
    #[allow(clippy::too_many_arguments)]
    fn start_behind(&self, world: &dyn World, net: &Network, lane: usize, s0: f32, lane_len: f32, stop_pos: DVec3, fwd: DVec2, jitter: f32) -> Option<DVec3> {
        let probe = |sign| along(net, lane, s0, lane_len, stop_pos, fwd, sign);
        let signs = if probe(1.0) <= probe(-1.0) { [1.0f32, -1.0] } else { [-1.0, 1.0] };
        for sign in signs {
            for d in [25.0f32, 35.0, 45.0, 55.0] {
                let a = (s0 + sign * (d + jitter)).clamp(0.0, lane_len);
                // (the pavement ends sooner: too close to the stop to be late from)
                if (a - s0).abs() < RUNNER_START.0 * 0.6 {
                    break;
                }
                let (q, _) = net.lanes[lane].at(a);
                if !crosses_street(net, q.truncate(), net.lanes[lane].at(s0).0.truncate()) && world.has_ground(q.x, q.y) && !self.seen(q) {
                    return Some(q);
                }
            }
        }
        None
    }

    /// Somebody late, before their movement this frame: running once their bus stands at
    /// the stop, left behind when it goes, an ordinary passenger again once aboard or
    /// waiting at the stop.
    pub fn runner_step(&mut self, i: usize, dt: f32, world: &dyn World, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>) {
        let Some(p) = self.pax(i) else { return };
        let Some(late) = p.late else { return };
        let (task, inside, stop, bus) = (p.task, p.inside, p.stop, p.bus);
        let Some(stop) = stop.filter(|_| inside.is_none()) else {
            self.end_late(i);
            return;
        };
        let bn = bus.and_then(|b| bus_of(buses, bus_ix, b));
        let listed = bus.is_some_and(|b| self.listed_at(stop, b));
        let standing = bn.is_some_and(|b| b.speed.abs() < 3.0 && self.in_stop_box(stop, b.id));
        match late.phase {
            // (the bus drove on, or they reached the stop first: they wait as anybody)
            LatePhase::Approach if !listed || task == Task::WaitingForBus => self.end_late(i),
            LatePhase::Approach if standing => {
                let p = self.pax_mut(i).unwrap();
                p.walk_speed = late.run;
                p.late = Some(Late { phase: LatePhase::Hurry, ..late });
                trace!("t={:.1} pax {} hurries for {bus:?}", self.time, self.people[i].label());
                if task == Task::WalkingToBusstop {
                    self.set_task(i, Task::ToBus, buses, bus_ix, world);
                }
            }
            LatePhase::Approach => {}
            LatePhase::Hurry => {
                // (running since it overtook them, still pulling in: not left behind yet)
                let stood = late.stood || standing;
                if !listed || (stood && bn.is_none_or(|b| b.speed.abs() >= 3.0)) {
                    let p = self.pax_mut(i).unwrap();
                    p.walk_speed = 0.0;
                    p.late = Some(Late { phase: LatePhase::Missed(RUNNER_MISSED), ..late });
                    trace!("t={:.1} pax {} missed {bus:?}", self.time, self.people[i].label());
                } else if task == Task::WaitingForBus {
                    // (gave up at its shut doors, back at the stop)
                    self.end_late(i);
                } else {
                    self.pax_mut(i).unwrap().late = Some(Late { hurried: late.hurried + dt, stood, ..late });
                }
            }
            LatePhase::Missed(t) if t > dt => self.pax_mut(i).unwrap().late = Some(Late { phase: LatePhase::Missed(t - dt), ..late }),
            LatePhase::Missed(_) => {
                self.end_late(i);
                if !matches!(task, Task::WalkingToBusstop | Task::WaitingForBus) {
                    self.set_task(i, Task::WalkingToBusstop, buses, bus_ix, world);
                }
            }
        }
    }

    /// Whether person `id` was put out to run for a bus that has not come yet (they are not
    /// taken away for room or for being far off).
    pub fn put_out(&self, id: u32) -> bool {
        self.runner_rolls.values().any(|e| matches!(e.roll, Roll::Placed(p) | Roll::Running(p) if p == id))
    }

    /// No longer late: their own pace again.
    fn end_late(&mut self, i: usize) {
        let Some(p) = self.pax_mut(i) else { return };
        if let Some(late) = p.late.take() {
            p.walk_speed = late.walk;
        }
    }
}
