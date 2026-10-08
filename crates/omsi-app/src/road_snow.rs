//! Snow on the roads that builds up while it snows and thaws again (`RoadSnow`), and the
//! field of ruts and tyre tracks around the camera the renderer lays into it (`SnowTracks`,
//! see omsi-render's road_snow.wgsl).
//!
//! The weather's "snow on road" was a switch: the roads were white or bare from one moment
//! to the next, and the season's WinterSnowfall textures went with it. Here the carriageways
//! carry a cover of their own instead: it starts as the weather has it, grows while snow
//! falls (a heavy fall closes a road in some ten minutes), thaws when it is warm or rains,
//! and the traffic keeps the lanes' wheel tracks down to slush. The tyres of every vehicle
//! near the camera press tracks into it, which the snow falling after them fills again.

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use omsi_render::{Renderer, SNOW_TRACK_SIDE, SNOW_TRACK_TEXELS, SNOW_TRACK_TILE};
use omsi_sim::traffic::{LaneKind, Network, GRID_CELL};

use crate::puddles::{vehicle_tyres, Tyre, REMOTE_KEY};

/// Built-up snow on the roads instead of the weather's on/off "snow on road"
/// (`OMSI_ROAD_SNOW`: `off` for the old way, a number for the cover to start with).
pub(crate) fn enabled() -> bool {
    !matches!(omsi_cfg::flags::OMSI_ROAD_SNOW.var(), Some("off") | Some("0ff") | Some("false"))
}

/// The tyres on the road of the player's bus, the traffic and the other LAN players' (the
/// keys as the spray has them).
pub(crate) fn tyres(
    player: Option<&crate::player::Player>,
    traffic: Option<&crate::traffic::Traffic>,
    remotes: &crate::lan::LanGame,
) -> Vec<Tyre> {
    let mut out = Vec::new();
    if let Some(p) = player {
        vehicle_tyres(&p.vehicle, 0, &mut out);
    }
    if let Some(t) = traffic {
        for c in &t.cars {
            vehicle_tyres(&c.vehicle, c.id.wrapping_add(1), &mut out);
        }
    }
    for (id, r) in &remotes.remotes {
        vehicle_tyres(r.vehicle(), REMOTE_KEY | *id as u64, &mut out);
    }
    out
}

/// A full road cover per second of the heaviest snowfall: ten minutes to close a road.
const SETTLE_RATE: f32 = 1.0 / 600.0;
/// How deep the lanes' ruts show (the traffic of a road that has had any).
const RUTS: f32 = 0.8;

/// The snow on the roads over time.
#[derive(Default)]
pub(crate) struct RoadSnow {
    /// How far it covers the carriageways (0..1).
    pub cover: f32,
    /// The snow fallen since the start, in thousandths of a full cover (it only grows): a
    /// tyre track remembers it and fills as more falls.
    pub fallen: f64,
    started: bool,
    /// The snow on the roofs of the vehicles around the camera.
    pub roofs: Roofs,
}

impl RoadSnow {
    /// `dt` seconds of the weather: the first call takes the weather's "snow on road" (or
    /// `OMSI_ROAD_SNOW`'s number) as the roads' state to begin with.
    pub fn step(&mut self, dt: f32, w: &omsi_content::weather::Weather) {
        if !self.started {
            self.started = true;
            self.cover = omsi_cfg::flags::OMSI_ROAD_SNOW
                .parse::<f32>()
                .unwrap_or(if w.snow_on_road { 1.0 } else { 0.0 })
                .clamp(0.0, 1.0);
        }
        let (kind, rate) = crate::weather_setup::precip_of(w);
        let temp = w.temp.0;
        if kind == 2 && rate > 0.0 && temp < 2.0 {
            let fall = dt * rate * SETTLE_RATE;
            self.cover += fall;
            self.fallen += fall as f64 * 1000.0;
        } else {
            // it thaws by the warmth of the air, and faster under rain
            let warm = (temp - 0.5).max(0.0) * 0.0002;
            let washed = if kind == 1 { rate * 0.003 } else { 0.0 };
            self.cover -= dt * (warm + washed);
        }
        self.cover = self.cover.clamp(0.0, 1.0);
    }

    /// What the renderer draws the roads' snow with.
    pub fn light(&self, lighting: &mut omsi_render::Lighting, tracks: &SnowTracks) {
        lighting.road_snow = Some(self.cover);
        lighting.snow_fallen = self.fallen;
        lighting.snow_ruts = RUTS;
        lighting.snow_tracks = tracks.ready();
    }
}

/// The snow fallen on a roof that closes it (thousandths of a road cover, as `fallen`): a
/// roof is white a little before the road, which the traffic keeps down.
const ROOF_FILL: f64 = 700.0;
/// Speed (m/s) above which the airstream takes snow off a roof, and how much a metre at
/// 12 m/s above it: some 400 m at 50 km/h, 200 m at 80.
const ROOF_BLOW_FROM: f32 = 6.0;
const ROOF_BLOW: f32 = 0.004;

/// The snow on one vehicle's roof: it gathers what falls, is blown off by the airstream
/// when the vehicle goes fast, and thaws as the roads' snow does. Kept by the snow fallen
/// and the way driven, not by the clock, so a run that renders only now and then (the
/// offscreen pictures) keeps it as the window does.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoofSnow {
    /// How much lies on it (0..1).
    pub amount: f32,
    fallen: f64,
    cover: f32,
    at: DVec3,
}

impl RoofSnow {
    fn new(amount: f32, fallen: f64, cover: f32, at: DVec3) -> Self {
        RoofSnow { amount: amount.clamp(0.0, 1.0), fallen, cover, at }
    }

    fn step(&mut self, fallen: f64, cover: f32, at: DVec3, speed: f32) {
        // what has fallen since
        self.amount += ((fallen - self.fallen).max(0.0) / ROOF_FILL) as f32;
        self.fallen = fallen;
        // the airstream, over the way driven (a jump - a car put elsewhere - blows nothing)
        let way = (at - self.at).truncate().length() as f32;
        self.at = at;
        if way < 50.0 && speed.abs() > ROOF_BLOW_FROM {
            self.amount -= way * (speed.abs() - ROOF_BLOW_FROM) / 12.0 * ROOF_BLOW;
        }
        // the thaw: as the roads' snow goes, the roofs' goes a little faster
        if cover < self.cover {
            self.amount -= (self.cover - cover) * 1.3;
        }
        self.cover = cover;
        if cover <= 0.0 {
            self.amount = self.amount.min(0.0);
        }
        self.amount = self.amount.clamp(0.0, 1.0);
    }
}

/// The roofs' snow of the vehicles around the camera, by the spray's keys (0 the player's).
#[derive(Default)]
pub(crate) struct Roofs {
    map: HashMap<u64, RoofSnow>,
}

impl Roofs {
    /// One step of `vehicles` (key, vehicle): one seen for the first time starts with as
    /// much as has settled on the roads (the player's bus) or a little less (the traffic,
    /// which has been driving); one no longer there is forgotten.
    pub fn step(&mut self, vehicles: &[(u64, &omsi_sim::VehicleInstance)], cover: f32, fallen: f64) {
        let mut next = HashMap::with_capacity(vehicles.len());
        for &(key, v) in vehicles {
            let mut r = self.map.get(&key).copied().unwrap_or_else(|| {
                RoofSnow::new(if key == 0 { cover } else { cover * 0.6 }, fallen, cover, v.position)
            });
            r.step(fallen, cover, v.position, v.physics.speed);
            next.insert(key, r);
        }
        self.map = next;
    }

    /// How much snow lies on vehicle `key`'s roof.
    pub fn amount(&self, key: u64) -> f32 {
        self.map.get(&key).map_or(0.0, |r| r.amount)
    }
}

/// The vehicles whose roofs gather snow: the player's bus (key 0) and the traffic.
pub(crate) fn roof_vehicles<'a>(
    player: Option<&'a crate::player::Player>,
    traffic: Option<&'a crate::traffic::Traffic>,
) -> Vec<(u64, &'a omsi_sim::VehicleInstance)> {
    let mut out = Vec::new();
    if let Some(p) = player {
        out.push((0, &p.vehicle));
    }
    if let Some(t) = traffic {
        out.extend(t.cars.iter().map(|c| (c.id.wrapping_add(1), &c.vehicle)));
    }
    out
}

/// Show the roof's snow on every part of a vehicle's render (its coupled parts' too).
pub(crate) fn show_roof(
    r: &Renderer,
    scene: &mut omsi_render::Scene,
    body: &crate::scene::VehicleRender,
    trailers: &[crate::scene::VehicleRender],
    amount: f32,
) {
    for render in std::iter::once(body).chain(trailers) {
        for &i in &render.instances {
            r.set_roof_snow(scene, i, amount);
        }
    }
}

/// The roofs' snow of the player's bus and the traffic's cars on their renders.
pub(crate) fn show_roofs(
    r: &Renderer,
    scene: &mut omsi_render::Scene,
    roofs: &Roofs,
    player: Option<&crate::player::Player>,
    traffic: Option<&crate::traffic::Traffic>,
    view: &crate::view_sync::traffic::TrafficView,
) {
    if let Some(p) = player {
        show_roof(r, scene, &p.render, &p.trailer_renders, roofs.amount(0));
    }
    if let Some(t) = traffic {
        for c in &t.cars {
            if let Some(cr) = view.car(c.id) {
                show_roof(r, scene, &cr.body, &cr.trailers, roofs.amount(c.id.wrapping_add(1)));
            }
        }
    }
}

/// A tile of the field: 32 m a side, 256 texels.
const TILE_M: f64 = 32.0;
const TILE_PX: usize = 256;
/// Tiles a side of the field (it wraps around the world every `SNOW_TRACK_SIDE` metres).
const SLOTS: usize = 8;
const TEXEL: f64 = TILE_M / TILE_PX as f64;
/// Tiles whose ruts are drawn in a frame at most (a jump of the camera fills the field
/// over a few frames instead of in one long one).
const RUTS_PER_FRAME: usize = 6;
/// Frames the lane network must stay as it is before the ruts are drawn again from it.
const LANES_STEADY: u32 = 30;
/// Half the gauge of a lane's wheel tracks and how wide each rut is (m): between a car's
/// and a bus's, so both drive in them.
const RUT_HALF_GAUGE: f64 = 0.9;
const RUT_SIGMA: f64 = 0.24;
/// Half a tyre's width (m).
const TYRE_HALF: f64 = 0.13;
/// Tyres farther from the camera leave no track (m).
const TRACK_RANGE: f64 = 110.0;

#[derive(Default)]
struct Slot {
    /// The world tile it holds (x, y in tiles).
    tile: Option<(i64, i64)>,
    /// Its texels (RGBA, rows along world y).
    px: Vec<u8>,
    ruts_done: bool,
    /// Its ruts have been drawn since it took this tile (and may be drawn again).
    drawn: bool,
    /// The texels changed since the last upload (x0, y0, x1, y1, exclusive ends).
    dirty: Option<(usize, usize, usize, usize)>,
}

impl Slot {
    fn touch(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        self.dirty = Some(match self.dirty {
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
            None => (x0, y0, x1, y1),
        });
    }
}

/// The ruts and the tyre tracks of the roads around the camera (see road_snow.wgsl).
#[derive(Default)]
pub(crate) struct SnowTracks {
    slots: Vec<Slot>,
    /// The lanes the ruts were drawn from (the network grows as tiles load), the count
    /// last seen and the frames it has stayed so.
    lanes_drawn: usize,
    lanes_seen: usize,
    lanes_steady: u32,
    /// Each tyre's contact last frame.
    last: HashMap<u64, DVec3>,
    ready: bool,
}

impl SnowTracks {
    /// The field holds the tiles around the camera.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// The field around `eye`: the ruts of `net`'s lanes, the tracks of `tyres` pressed in
    /// while the roads carry snow (`cover`), with the snow `fallen` so far.
    pub fn update(&mut self, r: &Renderer, eye: DVec3, net: Option<&Network>, tyres: &[Tyre], cover: f32, fallen: f64) {
        debug_assert_eq!(SLOTS * TILE_PX, SNOW_TRACK_TEXELS as usize);
        debug_assert_eq!(TILE_PX, SNOW_TRACK_TILE as usize);
        debug_assert!((SLOTS as f64 * TILE_M - SNOW_TRACK_SIDE).abs() < 1e-9);
        if cover <= 0.0 && !self.ready {
            // (no snow on the roads, none before: nothing to keep)
            self.last.clear();
            return;
        }
        if self.slots.is_empty() {
            self.slots = (0..SLOTS * SLOTS).map(|_| Slot::default()).collect();
        }
        // new lanes: the ruts drawn again (the tracks stay) - once the network has stopped
        // growing for a moment: tiles placed in the background bring their lanes a few at
        // a time, and redrawing at each set kept the field from ever being whole
        let lanes = net.map_or(0, |n| n.lanes.len());
        if lanes != self.lanes_seen {
            self.lanes_seen = lanes;
            self.lanes_steady = 0;
        }
        self.lanes_steady = self.lanes_steady.saturating_add(1);
        if lanes != self.lanes_drawn && self.lanes_steady >= LANES_STEADY {
            self.lanes_drawn = lanes;
            for s in &mut self.slots {
                s.ruts_done = false;
            }
        }
        // the tiles around the camera, each in its slot of the wrapping field
        let cx = (eye.x / TILE_M).floor() as i64;
        let cy = (eye.y / TILE_M).floor() as i64;
        let half = SLOTS as i64 / 2;
        let mut ruts_left = RUTS_PER_FRAME;
        let mut all_drawn = true;
        for ty in cy - half + 1..=cy + half {
            for tx in cx - half + 1..=cx + half {
                let s = &mut self.slots[slot_of(tx, ty)];
                if s.tile != Some((tx, ty)) {
                    s.tile = Some((tx, ty));
                    s.px.clear();
                    s.px.resize(TILE_PX * TILE_PX * 4, 0);
                    s.ruts_done = false;
                    s.drawn = false;
                    s.touch(0, 0, TILE_PX, TILE_PX);
                }
                if !s.ruts_done && ruts_left > 0 {
                    ruts_left -= 1;
                    draw_ruts(s, (tx, ty), net, fallen);
                    s.ruts_done = true;
                    s.drawn = true;
                    s.touch(0, 0, TILE_PX, TILE_PX);
                }
                all_drawn &= s.drawn;
            }
        }
        // the tyres' tracks, from where each touched the road last frame
        let mut now = HashMap::with_capacity(tyres.len());
        if cover > 0.02 {
            let stamp = (fallen.rem_euclid(65536.0)) as u16;
            for t in tyres {
                if (t.contact - eye).truncate().length() > TRACK_RANGE {
                    continue;
                }
                if let Some(&prev) = self.last.get(&t.key) {
                    let d = (t.contact - prev).truncate().length();
                    if d > 0.005 && d < 4.0 {
                        self.press(prev.truncate(), t.contact.truncate(), stamp);
                    }
                }
                now.insert(t.key, t.contact);
            }
        }
        self.last = now;
        // what changed goes to the GPU: the changed rows of each tile (whole rows lie one
        // after the other in the field's buffer)
        for (i, s) in self.slots.iter_mut().enumerate() {
            let Some((_, y0, _, y1)) = s.dirty.take() else { continue };
            r.set_snow_track_rows(i as u32, y0 as u32, &s.px[y0 * TILE_PX * 4..y1 * TILE_PX * 4]);
        }
        if all_drawn && !self.ready {
            let rutted = self.slots.iter().filter(|s| s.px.chunks_exact(4).any(|p| p[0] > 64)).count();
            log::info!("road snow: track field ready around ({:.0}, {:.0}), {lanes} lanes, ruts on {rutted} of {} tiles", eye.x, eye.y, self.slots.len());
        }
        self.ready = all_drawn || self.ready;
    }

    /// A tyre's track from `a` to `b` (world), stamped with the snow fallen when it was
    /// pressed.
    fn press(&mut self, a: DVec2, b: DVec2, stamp: u16) {
        let lo = a.min(b) - DVec2::splat(TYRE_HALF + TEXEL);
        let hi = a.max(b) + DVec2::splat(TYRE_HALF + TEXEL);
        let (gx0, gy0) = ((lo.x / TEXEL).floor() as i64, (lo.y / TEXEL).floor() as i64);
        let (gx1, gy1) = ((hi.x / TEXEL).ceil() as i64, (hi.y / TEXEL).ceil() as i64);
        let ab = b - a;
        let len2 = ab.length_squared().max(1e-12);
        for gy in gy0..gy1 {
            for gx in gx0..gx1 {
                let p = DVec2::new((gx as f64 + 0.5) * TEXEL, (gy as f64 + 0.5) * TEXEL);
                let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
                // how much of the texel the tyre covers, over a texel's width at its edge:
                // pressed all or nothing, the track's edge was a staircase of texels
                let d = (p - (a + ab * t)).length();
                let cover = ((TYRE_HALF + TEXEL * 0.5 - d) / TEXEL).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let (tx, ty) = (gx.div_euclid(TILE_PX as i64), gy.div_euclid(TILE_PX as i64));
                let s = &mut self.slots[slot_of(tx, ty)];
                if s.tile != Some((tx, ty)) {
                    continue; // (a tile outside the field)
                }
                let (lx, ly) = (gx.rem_euclid(TILE_PX as i64) as usize, gy.rem_euclid(TILE_PX as i64) as usize);
                let o = (ly * TILE_PX + lx) * 4;
                let g = (cover * 255.0).round() as u8;
                // (a texel the tyre crosses mostly takes this moment's snow; its edge texels
                // keep the older moment of a fuller track under them)
                if g >= s.px[o + 1] || cover > 0.5 {
                    s.px[o + 2] = (stamp >> 8) as u8;
                    s.px[o + 3] = stamp as u8;
                }
                s.px[o + 1] = s.px[o + 1].max(g);
                s.touch(lx, ly, lx + 1, ly + 1);
            }
        }
    }
}

/// The slot of the wrapping field that holds world tile (x, y).
fn slot_of(tx: i64, ty: i64) -> usize {
    ty.rem_euclid(SLOTS as i64) as usize * SLOTS + tx.rem_euclid(SLOTS as i64) as usize
}

/// The ruts of the street lanes over a tile: two wheel tracks along each lane, a bell's
/// profile across each (`r`, the strongest where lanes overlap).
///
/// A rut fills with the snow that falls while no wheel runs in it (road_snow.wgsl): each
/// texel keeps the moment a tyre last pressed it (`b`, `a`). Where none has yet, it takes
/// a moment as long ago as the lane's traffic makes likely - a busy street's ruts are
/// fresh, a quiet lane's half filled already.
fn draw_ruts(s: &mut Slot, (tx, ty): (i64, i64), net: Option<&Network>, fallen: f64) {
    // (drawn again - the network has grown - a rut keeps the moment it had: its filling
    // goes on where it was)
    let had: Vec<bool> = s.px.chunks_exact(4).map(|p| p[0] > 0).collect();
    for p in s.px.chunks_exact_mut(4) {
        p[0] = 0;
    }
    let x0 = tx as f64 * TILE_M;
    let y0 = ty as f64 * TILE_M;
    // (OMSI_ROAD_SNOW_GRID: a line every 4 m along both axes instead of the ruts, and a
    // thicker one where world x is a multiple of 32 m - to see the field lie on the world)
    if omsi_cfg::flags::OMSI_ROAD_SNOW_GRID.is_set() {
        for py in 0..TILE_PX {
            for px in 0..TILE_PX {
                let (gx, gy) = (px % 32, py % 32);
                let v = gx < 2 || gy < 2 || (px < 6);
                s.px[(py * TILE_PX + px) * 4] = if v { 255 } else { 0 };
            }
        }
        return;
    }
    let Some(net) = net else { return };
    let reach = RUT_HALF_GAUGE + 3.0 * RUT_SIGMA;
    // the lanes of the network's cells the tile (and the ruts' reach) touches
    let (c0x, c0y) = (((x0 - reach) / GRID_CELL).floor() as i32, ((y0 - reach) / GRID_CELL).floor() as i32);
    let (c1x, c1y) = (((x0 + TILE_M + reach) / GRID_CELL).floor() as i32, ((y0 + TILE_M + reach) / GRID_CELL).floor() as i32);
    let mut lanes: Vec<usize> = Vec::new();
    for cy in c0y..=c1y {
        for cx in c0x..=c1x {
            if let Some(l) = net.grid.get(&(cx, cy)) {
                lanes.extend_from_slice(l);
            }
        }
    }
    lanes.sort_unstable();
    lanes.dedup();
    for li in lanes {
        let lane = &net.lanes[li];
        // (every street lane: a bus lane or a terminus loop the random traffic keeps out
        // of has its ruts all the same)
        if lane.kind != LaneKind::Street {
            continue;
        }
        // (each lane its own wander, so that neighbouring lanes do not wander alike)
        let seed = li as f64 * 1.618;
        // (snow fallen since a wheel last ran here, thousandths of a cover: the ruts fill
        // from 150 on and are gone at 520, see the shader - a lane of the medium traffic
        // fresh, one the random traffic keeps out of (a terminus loop, a bus lane: the
        // buses still use it) a third filled)
        let since = 280.0 / (1.0 + 3.0 * lane.density.max(0.0) as f64);
        let then = (fallen - since).rem_euclid(65536.0) as u16;
        for (k, w) in lane.points.windows(2).enumerate() {
            let (a, b) = (w[0].truncate(), w[1].truncate());
            let (d0, d1) = (
                lane.dist.get(k).copied().unwrap_or(0.0) as f64,
                lane.dist.get(k + 1).copied().unwrap_or(0.0) as f64,
            );
            let lo = a.min(b) - DVec2::splat(reach);
            let hi = a.max(b) + DVec2::splat(reach);
            if hi.x < x0 || hi.y < y0 || lo.x > x0 + TILE_M || lo.y > y0 + TILE_M {
                continue;
            }
            let ab = b - a;
            let len2 = ab.length_squared();
            if len2 < 1e-6 {
                continue;
            }
            let px0 = (((lo.x - x0) / TEXEL).floor().max(0.0)) as usize;
            let py0 = (((lo.y - y0) / TEXEL).floor().max(0.0)) as usize;
            let px1 = (((hi.x - x0) / TEXEL).ceil() as usize).min(TILE_PX);
            let py1 = (((hi.y - y0) / TEXEL).ceil() as usize).min(TILE_PX);
            for py in py0..py1 {
                for px in px0..px1 {
                    let p = DVec2::new(x0 + (px as f64 + 0.5) * TEXEL, y0 + (py as f64 + 0.5) * TEXEL);
                    let t = (p - a).dot(ab) / len2;
                    // (each piece of the lane up to its ends: the next piece goes on from there)
                    if !(-0.02..=1.02).contains(&t) {
                        continue;
                    }
                    let across = (p - (a + ab * t)).length();
                    // The wheels do not run on a rail: the tracks wander a hand's breadth
                    // and widen and narrow along the road, and are worn deeper in places.
                    let along = d0 + (d1 - d0) * t.clamp(0.0, 1.0);
                    let wander = (along / 9.0 + seed).sin() * 0.07 + (along / 3.7 + seed * 2.3).sin() * 0.03;
                    let width = RUT_SIGMA * (1.0 + 0.25 * (along / 6.3 + seed * 0.7).sin());
                    let depth = 0.75 + 0.25 * (along / 4.1 + seed * 1.9).sin();
                    let off = (across - RUT_HALF_GAUGE - wander) / width;
                    let rut = (-0.5 * off * off).exp() * depth;
                    let v = (rut * 255.0).round() as u8;
                    let o = (py * TILE_PX + px) * 4;
                    if v > s.px[o] {
                        s.px[o] = v;
                        // (a texel a tyre has pressed keeps its own moment, as one
                        // that was a rut before)
                        if s.px[o + 1] == 0 && !had[py * TILE_PX + px] {
                            s.px[o + 2] = (then >> 8) as u8;
                            s.px[o + 3] = then as u8;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_wraps_as_the_shader_reads_it() {
        // a texel's place in the GPU field is its world index modulo the field's size,
        // whichever tile of the wrap it is in
        for (gx, gy) in [(0i64, 0i64), (255, 256), (-1, -1), (2047, 2048), (-2049, 12345)] {
            let (tx, ty) = (gx.div_euclid(TILE_PX as i64), gy.div_euclid(TILE_PX as i64));
            let i = slot_of(tx, ty);
            let (sx, sy) = (i % SLOTS, i / SLOTS);
            let x = sx * TILE_PX + gx.rem_euclid(TILE_PX as i64) as usize;
            let y = sy * TILE_PX + gy.rem_euclid(TILE_PX as i64) as usize;
            let n = SNOW_TRACK_TEXELS as i64;
            assert_eq!((x as i64, y as i64), (gx.rem_euclid(n), gy.rem_euclid(n)));
        }
    }

    #[test]
    fn a_roof_gathers_snow_and_the_airstream_takes_it() {
        let mut r = RoofSnow::new(0.0, 0.0, 0.5, DVec3::ZERO);
        // standing in the snow: 700 thousandths fallen close it
        r.step(350.0, 0.5, DVec3::ZERO, 0.0);
        assert!((r.amount - 0.5).abs() < 1e-4, "{}", r.amount);
        // slow through town: it stays
        r.step(350.0, 0.5, DVec3::new(30.0, 0.0, 0.0), 5.0);
        assert!((r.amount - 0.5).abs() < 1e-4);
        // 50 km/h for 120 m: a good part of it goes
        let mut x = 30.0;
        for _ in 0..12 {
            x += 10.0;
            r.step(350.0, 0.5, DVec3::new(x, 0.0, 0.0), 13.9);
        }
        assert!(r.amount < 0.25 && r.amount > 0.0, "{}", r.amount);
        // the thaw takes the rest with the roads' snow
        r.step(350.0, 0.0, DVec3::new(x, 0.0, 0.0), 0.0);
        assert_eq!(r.amount, 0.0);
    }

    #[test]
    fn snow_settles_while_it_falls_and_thaws_in_the_warm() {
        let mut w = omsi_content::weather::Weather::default();
        w.temp.0 = -3.0;
        w.precip = vec![2.0, 255.0];
        let mut r = RoadSnow::default();
        r.step(0.0, &w);
        assert_eq!(r.cover, 0.0);
        for _ in 0..300 {
            r.step(1.0, &w);
        }
        // five minutes of the heaviest snow: half a cover, and that much fallen
        assert!((r.cover - 0.5).abs() < 0.01, "{}", r.cover);
        assert!((r.fallen - 500.0).abs() < 10.0, "{}", r.fallen);
        // rain on it: it goes again, and what fell is still counted
        w.temp.0 = 4.0;
        w.precip = vec![1.0, 255.0];
        for _ in 0..600 {
            r.step(1.0, &w);
        }
        assert_eq!(r.cover, 0.0);
        assert!((r.fallen - 500.0).abs() < 10.0);
    }
}
