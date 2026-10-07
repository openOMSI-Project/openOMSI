//! The drive watch: what the Bus Company Simulator judges a tour by besides the timetable.
//! Frame by frame it follows the front of the player's bus along the traffic's lanes and
//! sees
//!
//! - **red lights**: the bus crossing a stop line - the start of a lane a traffic light lets
//!   traffic into - while that light shows red (after a moment's grace: a light that turned
//!   red as the bus was on the line is no offence). Which way the bus went over the junction
//!   is only known some metres on, so the lights of every way on are noted at the line and
//!   the one it took is judged when it is clear;
//! - **speed cameras**: fixed ones on a share of the lanes (more on fast roads, some where
//!   the limit is low), placed by the lane's identity so that every game finds them in the
//!   same places; one flashes when the bus passes it faster than the limit and the
//!   tolerance;
//! - **comfort**: hard braking, hard starts, and stops that end with a jerk (a good driver
//!   eases off the brake before the bus stands).
//!
//! An offence is said on the screen at once - a calm card at the top with the fine, a
//! camera's flash - and every event is kept with the time of the career's clock, so that a
//! trip's report takes those of its own stretch (`fill`). The rules are plain functions of
//! the lanes and the lights, tested without a window or a game.

use glam::{DVec3, Vec2};
use omsi_launcher_lib::company::career::{self, Offence, OffenceKind};
use omsi_launcher_lib::TripRun;
use omsi_render::{Renderer, Scene, TextureId};
use omsi_sim::traffic::{Aspect, Lane, LaneKind, Network, TrafficLightController};
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Weight};

use crate::nav_duty::{tr_with, Pen, EDGE, LATE_INK, NOW, SHEET, TEXT};

/// A light that has been red for less than this when the bus crosses its line is no
/// offence (s).
pub const RED_GRACE: f32 = 0.5;
/// How far over the stop line the front must be before the crossing counts (m): a bus that
/// stopped with its nose a little over the line and waits there is not running the light.
const OVER_LINE: f32 = 2.0;
/// How far into the junction the way the bus took is judged (m).
const JUDGE_AT: f32 = 8.0;
/// Seconds a crossing waits to be judged before it is let go.
const JUDGE_WITHIN: f64 = 15.0;
/// Slower than this (km/h) over the line is creeping, not running the light.
const CREEP: f32 = 4.0;

/// Hard braking and hard starts (m/s², the acceleration along smoothed over a quarter of a
/// second), and a stop's last half second braking harder than `ROUGH_STOP` on average.
pub const HARD_BRAKE: f32 = 2.8;
pub const HARD_START: f32 = 2.0;
pub const ROUGH_STOP: f32 = 1.6;
/// The same hard braking counts once: the next only after the deceleration eased below this.
const REARM: f32 = 1.2;

/// What the watch saw.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Offence(Offence),
    HardBrake,
    HardStart,
    RoughStop,
}

/// A light at the line, as it was when the bus crossed it.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    lane: usize,
    red: bool,
    red_for: f32,
}

/// A crossing to be judged: the lights of every way on from the lane before the line.
#[derive(Clone, Debug)]
struct Crossing {
    from: usize,
    at: f64,
    speed: f32,
    ways: Vec<Seen>,
}

/// Where a speed camera stands on a lane, if one does: on a share of the street lanes of the
/// map (12 % where 70 km/h and more is allowed, 8 % at 60, 6 % where it is 30 or less, 2.5 %
/// of the rest), at least 60 m long, somewhere in their middle - drawn from the lane's
/// identity in the map, so it is the same in every game. Returns the distance along it.
pub fn camera_on(lane: &Lane) -> Option<f32> {
    let key = lane.key?;
    if lane.kind != LaneKind::Street || lane.invisible || lane.no_cars || lane.length() < 60.0 {
        return None;
    }
    // (FNV-1a over the identity)
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.tile.0.to_le_bytes().into_iter().chain(key.tile.1.to_le_bytes()).chain(key.id.to_le_bytes()).chain(key.path.to_le_bytes()).chain([lane.reversed as u8, 0x5a]) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let u = (h >> 11) as f64 / (1u64 << 53) as f64;
    let v = ((h.wrapping_mul(0x9e37_79b9_7f4a_7c15)) >> 11) as f64 / (1u64 << 53) as f64;
    let limit = lane.speed_limit_kmh;
    let share = if limit >= 70.0 {
        0.12
    } else if limit >= 60.0 {
        0.08
    } else if limit <= 30.0 {
        0.06
    } else {
        0.025
    };
    (u < share).then(|| lane.length() * (0.3 + 0.4 * v as f32))
}

/// The watch.
#[derive(Default)]
pub struct DriveWatch {
    /// The lane the front is on and how far along, the frame before.
    last: Option<(usize, f32)>,
    /// The last lane without a light the bus came from (the way to a stop line).
    approach: Option<usize>,
    /// The approach a crossing was noted for already.
    crossed_from: Option<usize>,
    crossing: Option<Crossing>,
    /// Per light (controller, light): since when it shows red (career seconds).
    red_since: std::collections::HashMap<(usize, usize), f64>,
    /// The cameras that flashed lately: (lane, when).
    flashed: Vec<(usize, f64)>,
    /// The speed (m/s) the frame before, the acceleration along smoothed, and the braking
    /// of the last moments before a stop: (time, acceleration).
    speed: f32,
    accel: f32,
    braking: bool,
    starting: bool,
    slow: std::collections::VecDeque<(f64, f32)>,
    /// Everything seen this game: (career seconds, what).
    pub events: Vec<(f64, Event)>,
    shown: Option<Notice>,
    flash: f32,
    look: Option<Look>,
}

/// How many events are kept (a game of many hours keeps its last ones).
const KEPT: usize = 4000;

/// What the watch says on the screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Notice {
    pub icon: &'static str,
    pub ink: Color,
    pub title: String,
    pub detail: String,
    pub left: f32,
    pub shown: f32,
}

/// How long a notice stays (seconds of the game running) and how long the flash lasts.
const NOTICE_FOR: f32 = 5.0;
const FLASH_FOR: f32 = 0.45;

impl DriveWatch {
    /// One frame of the player's bus: `t` the career's clock (s), its front, heading and
    /// speed (m/s), the traffic's lanes and what their lights show (controller, light).
    /// Returns what it saw this frame.
    pub fn step(&mut self, t: f64, dt: f32, front: DVec3, heading: f64, speed: f32, net: Option<&Network>, light: &dyn Fn(usize, usize) -> Option<i32>) -> Vec<Event> {
        let mut out = Vec::new();
        self.comfort(t, dt, speed, &mut out);
        if let Some(net) = net {
            self.lanes(t, front, heading, speed, net, light, &mut out);
        }
        for e in &out {
            self.events.push((t, e.clone()));
        }
        if self.events.len() > KEPT {
            let extra = self.events.len() - KEPT;
            self.events.drain(..extra);
        }
        out
    }

    fn comfort(&mut self, t: f64, dt: f32, v: f32, out: &mut Vec<Event>) {
        let dt = dt.max(1e-3);
        let raw = (v - self.speed) / dt;
        let k = (4.0 * dt).min(0.5);
        self.accel += (raw - self.accel) * k;
        let a = self.accel;
        let was = self.speed;
        self.speed = v;
        // hard braking while moving, a hard start
        if a < -HARD_BRAKE && v > 2.0 && !self.braking {
            self.braking = true;
            out.push(Event::HardBrake);
        } else if a > -REARM {
            self.braking = false;
        }
        if a > HARD_START && v > 0.5 && !self.starting {
            self.starting = true;
            out.push(Event::HardStart);
        } else if a < HARD_START * 0.6 {
            self.starting = false;
        }
        // the last half second before the bus stands: how hard it braked
        if v < 2.5 && v > 0.05 {
            self.slow.push_back((t, a));
        } else if v >= 2.5 {
            self.slow.clear();
        }
        while self.slow.front().is_some_and(|x| t - x.0 > 0.5) {
            self.slow.pop_front();
        }
        if was > 0.05 && v <= 0.05 {
            if self.slow.len() >= 3 {
                let mean = self.slow.iter().map(|x| x.1).sum::<f32>() / self.slow.len() as f32;
                if mean < -ROUGH_STOP {
                    out.push(Event::RoughStop);
                }
            }
            self.slow.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lanes(&mut self, t: f64, front: DVec3, heading: f64, speed: f32, net: &Network, light: &dyn Fn(usize, usize) -> Option<i32>, out: &mut Vec<Event>) {
        let red = |c: usize, li: usize| light(c, li).map(TrafficLightController::aspect) == Some(Aspect::Red);
        let Some((lane, s, _)) = net.lane_along(front, heading, LaneKind::Street, 3.5, 45.0) else {
            self.last = None;
            return;
        };
        let l = &net.lanes[lane];
        // the lights ahead (the ways on from this lane, and this lane's own): since when red
        let mut watched: Vec<(usize, usize)> = l.next.iter().filter_map(|&n| net.lanes.get(n).and_then(|x| x.traffic_light)).collect();
        watched.extend(l.traffic_light);
        for (c, li) in watched {
            if red(c, li) {
                self.red_since.entry((c, li)).or_insert(t);
            } else {
                self.red_since.remove(&(c, li));
            }
        }
        if l.traffic_light.is_none() {
            if self.approach != Some(lane) {
                self.approach = Some(lane);
                self.crossed_from = None;
            }
        } else if let Some(a) = self.approach.filter(|a| self.crossed_from != Some(*a) && net.lanes[*a].next.contains(&lane) && s >= OVER_LINE) {
            // over the line: the lights of every way on from the approach, as they are now
            self.crossed_from = Some(a);
            let ways = net.lanes[a]
                .next
                .iter()
                .filter_map(|&n| {
                    let (c, li) = net.lanes[n].traffic_light?;
                    let is_red = red(c, li);
                    let red_for = if is_red { self.red_since.get(&(c, li)).map(|x| (t - x) as f32).unwrap_or(0.0) } else { 0.0 };
                    Some(Seen { lane: n, red: is_red, red_for })
                })
                .collect();
            self.crossing = Some(Crossing { from: a, at: t, speed: speed.abs() * 3.6, ways });
        }
        // the crossing judged once the way the bus took is clear
        let verdict = self.crossing.as_ref().map(|cr| {
            let took = cr.ways.iter().find(|w| w.lane == lane && (s >= JUDGE_AT || l.length() < JUDGE_AT + 1.0)).or_else(|| cr.ways.iter().find(|w| net.lanes[w.lane].next.contains(&lane)));
            let lost = t - cr.at > JUDGE_WITHIN || (lane != cr.from && !cr.ways.iter().any(|w| w.lane == lane) && t - cr.at > 3.0);
            match took {
                Some(w) => Some(Some(w.clone())),
                // (lost on the way: judged only when every way was red)
                None if lost => Some((!cr.ways.is_empty() && cr.ways.iter().all(|w| w.red)).then(|| cr.ways.iter().min_by(|a, b| a.red_for.total_cmp(&b.red_for)).cloned()).flatten()),
                None => None,
            }
        });
        if let Some(Some(judged)) = verdict {
            let cr = self.crossing.take().expect("a crossing");
            if let Some(w) = judged.filter(|w| w.red && w.red_for >= RED_GRACE && cr.speed >= CREEP) {
                out.push(Event::Offence(Offence { kind: OffenceKind::RedLight, at: 0.0, speed: cr.speed, limit: 0.0, red_for: w.red_for, fine: career::red_light_fine(w.red_for) }));
            }
        }
        // a camera passed on this lane
        if let (Some(cam), Some((last_lane, last_s))) = (camera_on(l), self.last) {
            self.flashed.retain(|f| t - f.1 < 30.0);
            if last_lane == lane && last_s < cam && s >= cam && !self.flashed.iter().any(|f| f.0 == lane) {
                let kmh = speed.abs() * 3.6;
                let limit = l.speed_limit_kmh;
                if let Some(over) = career::over_limit(kmh, limit) {
                    self.flashed.push((lane, t));
                    let fine = career::speeding_fine(over, limit <= 50.0);
                    out.push(Event::Offence(Offence { kind: OffenceKind::Speeding, at: 0.0, speed: kmh, limit, red_for: 0.0, fine }));
                }
            }
        }
        self.last = Some((lane, s));
    }

    /// One frame of the game: the player's bus (its outline: centre, heading, half length),
    /// the traffic, the career's clock. Offences are said on the screen.
    pub fn tick(&mut self, t: f64, dt: f32, outline: crate::traffic::PlayerBox, traffic: Option<&crate::traffic::Traffic>) {
        let (centre, heading, half_len, _, speed) = outline;
        let h = heading.to_radians();
        let front = centre + DVec3::new(h.sin(), h.cos(), 0.0) * half_len as f64;
        let events = match traffic {
            Some(tr) => self.step(t, dt, front, heading, speed, Some(&tr.net), &|c, li| tr.light_state(c, li).map(|s| s.0)),
            None => self.step(t, dt, front, heading, speed, None, &|_, _| None),
        };
        for e in events {
            if let Event::Offence(o) = e {
                if o.kind == OffenceKind::Speeding {
                    self.flash = FLASH_FOR;
                }
                log::info!("drive watch: {:?} at {:.0} km/h (limit {:.0}, red for {:.1} s): fine {:.2}", o.kind, o.speed, o.limit, o.red_for, o.fine as f64 / 100.0);
                self.shown = Some(notice(&o));
            }
        }
    }

    /// What the watch saw between `from` and `to` (career seconds): the counts, the fines
    /// and the offences (their times from `from`).
    pub fn between(&self, from: f64, to: f64) -> Tally {
        let mut t = Tally::default();
        for (at, e) in self.events.iter().filter(|(at, _)| *at >= from && *at <= to + 1e-6) {
            match e {
                Event::HardBrake => t.hard_brakes += 1,
                Event::HardStart => t.hard_starts += 1,
                Event::RoughStop => t.rough_stops += 1,
                Event::Offence(o) => {
                    match o.kind {
                        OffenceKind::RedLight => t.red_lights += 1,
                        OffenceKind::Speeding => t.speeding += 1,
                    }
                    t.fines += o.fine;
                    t.offences.push(Offence { at: (at - from).max(0.0), ..o.clone() });
                }
            }
        }
        t
    }

    /// Put what the watch saw on a trip into its report: the trip ended at `now` (career
    /// seconds) after `run.seconds`.
    pub fn fill(&self, run: &mut TripRun, now: f64) {
        let t = self.between(now - run.seconds.max(0.0), now);
        run.watched = true;
        run.red_lights = t.red_lights;
        run.speeding = t.speeding;
        run.fines = t.fines;
        run.hard_brakes = t.hard_brakes;
        run.hard_starts = t.hard_starts;
        run.rough_stops = t.rough_stops;
        run.offences = t.offences;
    }
}

/// What the watch saw over a stretch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    pub red_lights: i32,
    pub speeding: i32,
    pub fines: i64,
    pub hard_brakes: i32,
    pub hard_starts: i32,
    pub rough_stops: i32,
    pub offences: Vec<Offence>,
}

/// Whole euros as the language writes them ("€90", "€ 90", "90 €").
pub(crate) fn euros(cents: i64) -> String {
    let v = (cents as f64 / 100.0).round() as i64;
    match omsi_ui::i18n::language().as_str() {
        "" | "en" => format!("€{v}"),
        "nl" => format!("€ {v}"),
        _ => format!("{v} €"),
    }
}

/// What the card says of an offence.
pub(crate) fn notice(o: &Offence) -> Notice {
    match o.kind {
        OffenceKind::RedLight => Notice {
            icon: "traffic",
            ink: LATE_INK,
            title: omsi_ui::tr("Red light").into_owned(),
            detail: tr_with("You crossed the stop line on red · fine %{amount}", &[("amount", euros(o.fine))]),
            left: NOTICE_FOR,
            shown: 0.0,
        },
        OffenceKind::Speeding => Notice {
            icon: "photo_camera",
            ink: NOW,
            title: omsi_ui::tr("Speed camera").into_owned(),
            detail: tr_with("%{speed} km/h where %{limit} is allowed · fine %{amount}", &[("speed", format!("{:.0}", o.speed)), ("limit", format!("{:.0}", o.limit)), ("amount", euros(o.fine))]),
            left: NOTICE_FOR,
            shown: 0.0,
        },
    }
}

// --- on the screen ------------------------------------------------------------------------------

/// The notice card's size at scale 1.
pub(crate) const NOTICE_W: f32 = 400.0;
pub(crate) const NOTICE_H: f32 = 64.0;

/// Draw a notice into `r` (`NOTICE_W` by `NOTICE_H` at scale `s`), with the time it has left
/// (0 - 1) along its bottom.
pub(crate) fn draw_notice(pen: &mut Pen, n: &Notice, r: Rect, s: f32, left: f32) {
    let radius = 12.0 * s;
    pen.p.rounded(r, radius, SHEET.alpha(0.97));
    pen.p.rounded_border(r, radius, 1.0, EDGE);
    // the offence's icon in a circle of its colour
    let c = Vec2::new(r.x + 34.0 * s, r.center().y);
    pen.p.circle(c, 17.0 * s, n.ink.alpha(0.16));
    pen.p.icon(pen.atlas, n.icon, c, 19.0 * s, n.ink);
    let x = r.x + 62.0 * s;
    let w = (r.right() - 16.0 * s - x).max(0.0);
    pen.text_in(&n.title.to_uppercase(), 11.0 * s, Weight::Bold, Rect::new(x, r.y + 12.0 * s, w, 16.0 * s), Align::Left, n.ink);
    pen.text_in(&n.detail, 14.0 * s, Weight::Medium, Rect::new(x, r.y + 30.0 * s, w, 20.0 * s), Align::Left, TEXT);
    let line = Rect::new(r.x + radius, r.bottom() - 3.0 * s, (r.w - 2.0 * radius) * left.clamp(0.0, 1.0), 2.0 * s);
    if line.w > 0.5 {
        pen.p.rounded(line, 1.0 * s, n.ink.alpha(0.7));
    }
}

/// What draws the card and the flash: textures of their own, as the trip's report draws.
struct Look {
    gpu: Gpu,
    atlas: Atlas,
    fonts: Fonts,
    card: Option<(TextureId, u32, u32)>,
    flash: Option<TextureId>,
}

impl DriveWatch {
    /// Draw the notice (centred at the top of the interface's part of the window, `hud`:
    /// x, y, width, height, below `under` when the trip's card is there) and a camera's
    /// flash. `running`: the game runs (its time counts the notice down).
    pub fn frame(&mut self, renderer: &Renderer, scene: &mut Scene, hud: [f32; 4], ui_scale: f32, under: Option<f32>, dt: f32, running: bool) {
        if self.shown.is_none() && self.flash <= 0.0 {
            return;
        }
        let look = self.look.get_or_insert_with(|| {
            let atlas = Atlas::new(1024);
            Look { gpu: Gpu::new(&renderer.device, renderer.format(), crate::trip_report::samples(renderer.format()), atlas.size), atlas, fonts: Fonts::hanken(), card: None, flash: None }
        });
        // the flash: white over the whole view, gone in under half a second
        if self.flash > 0.0 {
            self.flash -= dt;
            let a = (self.flash / FLASH_FOR).clamp(0.0, 1.0).powf(1.5) * 0.85;
            let tex = *look.flash.get_or_insert_with(|| {
                let t = renderer.add_render_texture(scene, 8, 8);
                scene.premultiplied.insert(t);
                t
            });
            if let Some(view) = renderer.texture_view(scene, tex) {
                let mut p = Painter::new();
                p.rect(Rect::new(0.0, 0.0, 8.0, 8.0), Color::WHITE);
                look.gpu.upload(&renderer.device, &renderer.queue, 1, &p.verts);
                let layers = [Layer::flat([0.0, 0.0, 8.0, 8.0], 0.0, a)];
                let draws = [Draw { buffer: 1, range: 0..p.verts.len() as u32, layer: 0, texture: 0 }];
                let mut enc = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("camera flash") });
                look.gpu.render(&renderer.device, &renderer.queue, &mut enc, &view, (8, 8), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
                renderer.queue.submit([enc.finish()]);
                scene.overlays.push((tex, [hud[0], hud[1], hud[0] + hud[2], hud[1] + hud[3]]));
            }
        }
        let Some(n) = self.shown.as_mut() else { return };
        if running {
            n.left -= dt;
        }
        let target = if n.left > 0.0 { 1.0 } else { 0.0 };
        let step = dt / 0.25;
        n.shown = if n.shown < target { (n.shown + step).min(target) } else { (n.shown - step).max(target) };
        if n.left <= 0.0 && n.shown <= 0.0 {
            self.shown = None;
            return;
        }
        let s = ((hud[3] * 0.33).max(300.0) * ui_scale.max(0.1) / 360.0).min((hud[2] - 32.0).max(120.0) / NOTICE_W).max(0.4);
        let m = (12.0 * s).round();
        let (cw, ch) = ((NOTICE_W * s).round(), (NOTICE_H * s).round());
        let size = ((cw + 2.0 * m) as u32, (ch + 2.0 * m) as u32);
        let eased = 1.0 - (1.0 - n.shown).powi(3);
        let x0 = hud[0] + ((hud[2] - cw) * 0.5).round();
        let top = under.map(|u| u + 10.0 * s).unwrap_or(hud[1] + (hud[3] * 0.08).max(16.0));
        let y0 = (top - (1.0 - eased) * 12.0 * s).round();
        if look.card.is_none_or(|t| (t.1, t.2) != size) {
            if let Some((t, _, _)) = look.card.take() {
                renderer.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
            let t = renderer.add_render_texture(scene, size.0, size.1);
            scene.premultiplied.insert(t);
            look.card = Some((t, size.0, size.1));
        }
        let Some((tex, _, _)) = look.card else { return };
        let Some(view) = renderer.texture_view(scene, tex) else { return };
        look.atlas.begin_frame();
        let mut p = Painter::new();
        let r = Rect::new(m, m, cw, ch);
        p.shadow(r, 12.0 * s, 14.0 * s, Color::rgba(0, 0, 0, 0.4));
        draw_notice(&mut Pen { p: &mut p, atlas: &mut look.atlas, fonts: &look.fonts }, n, r, s, n.left / NOTICE_FOR);
        let (device, queue) = (&renderer.device, &renderer.queue);
        look.gpu.upload(device, queue, 0, &p.verts);
        look.gpu.upload_atlas(queue, &mut look.atlas);
        let layers = [Layer::flat([0.0, 0.0, size.0 as f32, size.1 as f32], 0.0, eased)];
        let draws = [Draw { buffer: 0, range: 0..p.verts.len() as u32, layer: 0, texture: 0 }];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("drive watch") });
        look.gpu.render(device, queue, &mut enc, &view, size, Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
        scene.overlays.push((tex, [x0 - m, y0 - m, x0 + cw + m, y0 + ch + m]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_sim::traffic::{LaneBuilder, LaneKey};

    /// A straight road north from (0, -200) to a stop line at (0, 0), and three ways on over
    /// the junction (straight on, left, right), each behind its own light of controller 0.
    fn junction() -> Network {
        let mut net = Network::default();
        let mut approach = LaneBuilder::polyline(vec![DVec3::new(0.0, -200.0, 0.0), DVec3::new(0.0, 0.0, 0.0)], LaneKind::Street, 3.0);
        approach.next = vec![1, 2, 3];
        let straight = LaneBuilder::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 30.0, 0.0)], LaneKind::Street, 3.0);
        let left = LaneBuilder::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(-2.0, 10.0, 0.0), DVec3::new(-20.0, 16.0, 0.0)], LaneKind::Street, 3.0);
        let right = LaneBuilder::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(2.0, 8.0, 0.0), DVec3::new(15.0, 12.0, 0.0)], LaneKind::Street, 3.0);
        let mut ways = [straight, left, right];
        for (k, w) in ways.iter_mut().enumerate() {
            w.traffic_light = Some((0, k));
            w.next = vec![4];
        }
        let beyond = LaneBuilder::polyline(vec![DVec3::new(0.0, 30.0, 0.0), DVec3::new(0.0, 300.0, 0.0)], LaneKind::Street, 3.0);
        net.lanes = vec![approach];
        net.lanes.extend(ways);
        net.lanes.push(beyond);
        net.build_grid();
        net
    }

    /// Drive north along x = 0 from y0 to y1 at `kmh`, with the lights `states` (straight,
    /// left, right) switching to `after` at time `switch`; returns the events.
    fn drive(net: &Network, y0: f64, y1: f64, kmh: f32, states: [i32; 3], switch: f64, after: [i32; 3]) -> Vec<Event> {
        let mut w = DriveWatch::default();
        let v = kmh / 3.6;
        let dt = 0.05;
        let mut t = 0.0;
        let mut y = y0;
        let mut out = Vec::new();
        // (at speed already: no hard start)
        w.speed = v;
        while y < y1 {
            let now = if t < switch { states } else { after };
            out.extend(w.step(t, dt as f32, DVec3::new(0.0, y, 0.0), 0.0, v, Some(net), &|_, li| now.get(li).copied()));
            y += v as f64 * dt;
            t += dt;
        }
        out
    }

    fn offences(ev: &[Event]) -> Vec<Offence> {
        ev.iter().filter_map(|e| if let Event::Offence(o) = e { Some(o.clone()) } else { None }).collect()
    }

    #[test]
    fn a_red_light_run_is_seen_and_a_green_one_is_not() {
        let net = junction();
        // red all the time: one offence, of two hundred euros (red long before)
        let ev = offences(&drive(&net, -60.0, 60.0, 40.0, [1, 1, 1], 1e9, [1, 1, 1]));
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].kind, ev[0].fine), (OffenceKind::RedLight, 200_00));
        assert!((ev[0].speed - 40.0).abs() < 0.5);
        // green: nothing
        assert!(offences(&drive(&net, -60.0, 60.0, 40.0, [6, 6, 6], 1e9, [6, 6, 6])).is_empty());
        // yellow: the driver's decision, no offence
        assert!(offences(&drive(&net, -60.0, 60.0, 40.0, [9, 9, 9], 1e9, [9, 9, 9])).is_empty());
        // red only a moment before the line: within the grace
        let at_line = 60.0 / (40.0 / 3.6);
        assert!(offences(&drive(&net, -60.0, 60.0, 40.0, [6, 6, 6], at_line as f64 - 0.2, [1, 1, 1])).is_empty());
        // red for most of a second: ninety euros
        let ev = offences(&drive(&net, -60.0, 60.0, 40.0, [6, 6, 6], at_line as f64 - 0.6, [1, 1, 1]));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fine, 90_00);
    }

    #[test]
    fn only_the_light_of_the_way_taken_counts() {
        let net = junction();
        // straight on is green, the turns are red: the bus goes straight on
        assert!(offences(&drive(&net, -60.0, 60.0, 40.0, [6, 1, 1], 1e9, [6, 1, 1])).is_empty());
        // straight on red, the left turn green: the bus goes straight on and is fined
        assert_eq!(offences(&drive(&net, -60.0, 60.0, 40.0, [1, 6, 6], 1e9, [1, 6, 6])).len(), 1);
    }

    #[test]
    fn a_bus_waiting_at_the_line_is_no_offence() {
        let net = junction();
        let mut w = DriveWatch::default();
        let light = |red: bool| move |_: usize, _: usize| Some(if red { 1 } else { 6 });
        let mut t = 0.0;
        // up to the line at 30 km/h, stopping with the nose half a metre over it
        let mut y = -40.0;
        while y < 0.5 {
            w.step(t, 0.05, DVec3::new(0.0, y, 0.0), 0.0, 8.0, Some(&net), &light(true));
            y += 0.4;
            t += 0.05;
        }
        for _ in 0..200 {
            w.step(t, 0.05, DVec3::new(0.0, 0.5, 0.0), 0.0, 0.0, Some(&net), &light(true));
            t += 0.05;
        }
        // green, and away
        let mut out = Vec::new();
        while y < 40.0 {
            out.extend(w.step(t, 0.05, DVec3::new(0.0, y, 0.0), 0.0, 6.0, Some(&net), &light(false)));
            y += 0.3;
            t += 0.05;
        }
        assert!(offences(&out).is_empty());
    }

    fn camera_lane(limit: f32) -> Option<(Lane, f32)> {
        for id in 0..2000 {
            let mut l = LaneBuilder::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 200.0, 0.0)], LaneKind::Street, 3.0);
            l.key = Some(LaneKey { tile: (3, 4), id, path: 0 });
            l.speed_limit_kmh = limit;
            if let Some(s) = camera_on(&l) {
                return Some((l, s));
            }
        }
        None
    }

    #[test]
    fn speed_cameras_stand_on_a_share_of_the_lanes_and_flash() {
        // the same lane always has its camera in the same place; fast roads have more
        let count = |limit: f32| {
            (0..4000)
                .filter(|id| {
                    let mut l = LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 100.0, 0.0)], LaneKind::Street, 3.0);
                    l.key = Some(LaneKey { tile: (0, 0), id: *id, path: 1 });
                    l.speed_limit_kmh = limit;
                    camera_on(&l).is_some()
                })
                .count()
        };
        let (town, fast) = (count(50.0), count(80.0));
        assert!((40..170).contains(&town), "{town}");
        assert!((380..600).contains(&fast), "{fast}");
        let (lane, s) = camera_lane(50.0).expect("a lane with a camera");
        assert_eq!(camera_on(&lane), Some(s));
        assert!((60.0..=140.0).contains(&s));
        // short lanes, footpaths and lanes without a place in the map have none
        let mut short = lane.clone();
        short.points[1].y = 40.0;
        short.refresh();
        assert_eq!(camera_on(&short), None);
        assert_eq!(camera_on(&Lane { key: None, ..lane.clone() }), None);
        let mut net = Network::default();
        net.lanes = vec![lane];
        net.build_grid();
        let pass = |kmh: f32| offences(&drive(&net, 10.0, 190.0, kmh, [6; 3], 1e9, [6; 3]));
        assert!(pass(52.0).is_empty(), "within the tolerance");
        let ev = pass(66.0);
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].kind, ev[0].limit, ev[0].fine), (OffenceKind::Speeding, 50.0, 50_00));
    }

    /// Speeds over time (one value a twentieth of a second) through the comfort watch.
    fn ride(speeds: &[f32]) -> Vec<Event> {
        let mut w = DriveWatch::default();
        w.speed = speeds.first().copied().unwrap_or(0.0);
        let mut out = Vec::new();
        for (k, v) in speeds.iter().enumerate() {
            out.extend(w.step(k as f64 * 0.05, 0.05, DVec3::ZERO, 0.0, *v, None, &|_, _| None));
        }
        out
    }

    /// From `v0` to `v1` m/s at `a` m/s², then `hold` seconds at `v1`.
    fn ramp(v0: f32, v1: f32, a: f32, hold: f32) -> Vec<f32> {
        let mut out = Vec::new();
        let mut v = v0;
        let step = a.abs() * 0.05 * if v1 >= v0 { 1.0 } else { -1.0 };
        while (v1 - v).abs() > step.abs() {
            out.push(v);
            v += step;
        }
        for _ in 0..(hold / 0.05) as usize {
            out.push(v1);
        }
        out
    }

    #[test]
    fn comfort_sees_hard_braking_hard_starts_and_rough_stops() {
        let count = |ev: &[Event], e: Event| ev.iter().filter(|x| **x == e).count();
        // a calm start and a gentle stop eased off at the end: nothing
        let mut calm = ramp(0.0, 13.0, 1.2, 5.0);
        calm.extend(ramp(13.0, 2.0, 1.1, 0.0));
        calm.extend(ramp(2.0, 0.0, 0.6, 2.0));
        assert!(ride(&calm).is_empty(), "{:?}", ride(&calm));
        // a hard start and a hard brake: once each
        let mut hard = ramp(0.0, 13.0, 3.0, 3.0);
        hard.extend(ramp(13.0, 6.0, 4.5, 3.0));
        let ev = ride(&hard);
        assert_eq!((count(&ev, Event::HardStart), count(&ev, Event::HardBrake)), (1, 1));
        // a stop braked hard to the end: a rough stop
        let mut rough = ramp(8.0, 8.0, 1.0, 1.0);
        rough.extend(ramp(8.0, 0.0, 2.4, 2.0));
        let ev = ride(&rough);
        assert_eq!(count(&ev, Event::RoughStop), 1);
    }

    #[test]
    fn a_trip_takes_the_events_of_its_stretch() {
        let mut w = DriveWatch::default();
        let o = |fine| Event::Offence(Offence { kind: OffenceKind::Speeding, fine, ..Default::default() });
        w.events = vec![(10.0, o(30_00)), (100.0, Event::HardBrake), (150.0, o(70_00)), (150.0, Event::RoughStop), (400.0, Event::HardStart)];
        let mut run = TripRun { seconds: 100.0, ..Default::default() };
        w.fill(&mut run, 200.0);
        assert!(run.watched);
        assert_eq!((run.speeding, run.fines, run.hard_brakes, run.rough_stops, run.hard_starts), (1, 70_00, 1, 1, 0));
        assert_eq!(run.offences.len(), 1);
        assert_eq!(run.offences[0].at, 50.0);
    }

    #[test]
    fn the_notices_are_translated() {
        let keys = ["Red light", "Speed camera", "You crossed the stop line on red · fine %{amount}", "%{speed} km/h where %{limit} is allowed · fine %{amount}"];
        for language in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for key in keys {
                let t = crate::_rust_i18n_try_translate(language, key);
                assert!(t.as_ref().is_some_and(|t| !t.trim().is_empty()), "{language}: {key}");
                for ph in ["%{amount}", "%{speed}", "%{limit}"] {
                    assert_eq!(key.contains(ph), t.as_ref().unwrap().contains(ph), "{language}: {key}");
                }
            }
        }
    }

    /// Pictures of the notices, in Dutch:
    /// `OMSI_TRIP_REPORT_PREVIEW=<folder> cargo test -p omsi-app --lib drive_watch -- --ignored`.
    #[test]
    #[ignore]
    fn preview_pictures() {
        let Ok(dir) = std::env::var("OMSI_TRIP_REPORT_PREVIEW") else { return };
        crate::ui_language("NLD");
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(2048);
        let s = 2.0;
        let ns = [notice(&Offence { kind: OffenceKind::RedLight, red_for: 2.0, speed: 38.0, fine: 200_00, ..Default::default() }), notice(&Offence { kind: OffenceKind::Speeding, speed: 66.0, limit: 50.0, fine: 50_00, ..Default::default() })];
        let mut img = image::RgbaImage::from_pixel((NOTICE_W * s + 60.0) as u32, ((NOTICE_H * s + 30.0) * 2.0 + 30.0) as u32, image::Rgba([70, 84, 96, 255]));
        for (k, n) in ns.iter().enumerate() {
            let mut p = Painter::new();
            let r = Rect::new(30.0, 30.0 + k as f32 * (NOTICE_H * s + 30.0), NOTICE_W * s, NOTICE_H * s);
            p.shadow(r, 12.0 * s, 14.0 * s, Color::rgba(0, 0, 0, 0.4));
            draw_notice(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, n, r, s, 0.7);
            crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e5, 1e5));
        }
        img.save(format!("{dir}/drive_watch.png")).unwrap();
    }
}
