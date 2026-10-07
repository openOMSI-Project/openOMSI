//! The trip's report: when the player's bus reaches the last stop of a trip - of the duty, or
//! of a free drive along a line - a card in the navigator's style says how the trip went, as
//! Omsi-Hub's end-of-trip popup did: the line and where it went, the stops served and how many
//! of them early, on time and late as OMSI 2 counts them, a bar of the three, the stop
//! furthest off the timetable and the average, and the distance, the jolts, the collisions and
//! the passengers on the way, and how the trip is judged (`career::evaluate`: punctuality,
//! comfort and safety from the drive watch, a grade, the experience it earned and the fines
//! it cost). It stays some seconds of the game running (Enter or a click puts
//! it away sooner), and the trip is written into the driver's record as it ends
//! (`~/.openomsi/trips/<driver>.jsonl`, `Career::write_trip`), where the launcher's service
//! record shows it.
//!
//! What a trip came to is worked out from what the duty says frame by frame by pure functions
//! ([`TripRecorder`]), and the card is laid out from the result ([`card`], [`draw_card`]), so
//! both are tested without a window or a game.

use glam::Vec2;
use omsi_launcher_lib::TripRun;
use omsi_render::{Renderer, Scene, TextureId};
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Weight};

use crate::career::{Career, EARLY_DEPARTURE, LATE_ARRIVAL};
use omsi_launcher_lib::company::career as judge;
use crate::nav_duty::{accent_ink, hhmm, offset, punctuality, tr_with, Pen, Punctuality, EARLY, EDGE, FIELD, HAIRLINE, LATE, LATE_INK, LINE, NOW, ON_TIME, ON_TIME_INK, SHEET, TEXT, TEXT_DIM, TEXT_FAINT, TEXT_SOFT};
use crate::schedule::PlayerDuty;

/// How long the card stays (seconds of the game running: not while it is paused), and how
/// long it takes to come and to go.
const SHOW_FOR: f32 = 10.0;
const FADE: f32 = 0.25;

// --- what a trip came to ----------------------------------------------------------------------

/// A stop of the trip as it was served: how late the bus arrived there (s, negative early) and
/// how late it left (None at the last stop, where the trip ends with the arrival).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Served {
    pub arrival: f64,
    pub departure: Option<f64>,
}

impl Served {
    /// How OMSI 2 counts the stop (`Career::stop_served`): early when the bus left it more
    /// than two minutes before its time, late when it arrived more than three after, both to
    /// the second. (One that was both - in late, out early at a long stop - counts early here,
    /// leaving early being what passengers miss their bus by; OMSI counts it twice.)
    pub fn judged(self) -> Punctuality {
        if self.departure.is_some_and(|d| d.round() < EARLY_DEPARTURE) {
            Punctuality::Early
        } else if self.arrival.round() > LATE_ARRIVAL {
            Punctuality::Late
        } else {
            Punctuality::OnTime
        }
    }

    /// How far off the timetable the stop was served: as the bus left it, at the last stop as
    /// it came.
    pub fn delay(self) -> f64 {
        self.departure.unwrap_or(self.arrival)
    }
}

/// What the stops come to: how many early, on time and late, the stop furthest off the
/// timetable (either way) and the average delay (s, negative early).
pub fn tally(stops: &[Served]) -> ([i32; 3], Option<f64>, Option<f64>) {
    let mut counts = [0; 3];
    for s in stops {
        counts[match s.judged() {
            Punctuality::Early => 0,
            Punctuality::OnTime => 1,
            Punctuality::Late => 2,
        }] += 1;
    }
    let delays: Vec<f64> = stops.iter().map(|s| s.delay()).filter(|d| d.is_finite()).collect();
    let worst = delays.iter().copied().max_by(|a, b| a.abs().total_cmp(&b.abs()));
    let average = (!delays.is_empty()).then(|| delays.iter().sum::<f64>() / delays.len() as f64);
    (counts, worst, average)
}

/// The career's running counters a trip's figures are the difference of.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Counters {
    pub seconds: f64,
    pub metres: f64,
    pub jolts: i32,
    pub crashes: i32,
    pub boarded: i32,
}

impl Counters {
    pub fn of(c: &Career) -> Counters {
        Counters { seconds: c.seconds, metres: c.metres, jolts: c.harsh, crashes: c.crashes[0], boarded: c.boarded }
    }
}

/// The trip under way at one moment, as much of it as its report needs (`TripView::of`).
#[derive(Debug, Clone, PartialEq)]
pub struct TripView {
    /// Which trip it is: its place in the duty, its name and when it leaves (s).
    pub key: (usize, String, i64),
    pub line: String,
    pub tour: String,
    pub terminus: String,
    /// Its place in the duty (0 = the first) and the duty's trips.
    pub index: usize,
    pub count: usize,
    /// When it is to leave its first stop and reach its last (s of the day).
    pub departure: f64,
    pub end: f64,
    /// The stops it serves (a depot run passes its stations), and its last stop's number.
    pub planned: usize,
    pub last: usize,
    /// The stop the bus is due at next, and whether it stands there.
    pub next_stop: usize,
    pub at_stop: bool,
    /// The bus has reached the last stop.
    pub done: bool,
    /// A free drive along a line: its stops are followed, not judged.
    pub free: bool,
    /// How late the bus came to the last stop, once there (and when the trip stops there).
    pub terminus_arrival: Option<f64>,
}

impl TripView {
    pub fn of(d: &PlayerDuty) -> Option<TripView> {
        let trip = d.trips.get(d.trip_index)?;
        let line = trip.line.trim().to_string();
        // (as the end-of-trip note writes it: "5 Rathaus" is the line's terminus Rathaus)
        let terminus = trip.terminus.trim();
        let terminus = if line.is_empty() { terminus } else { terminus.strip_prefix(&format!("{line} ")).unwrap_or(terminus) };
        Some(TripView {
            key: (d.trip_index, trip.name.clone(), trip.departure.round() as i64),
            line,
            tour: d.tours.get(d.trip_index).map(|t| t.1.clone()).unwrap_or_else(|| d.tour.clone()),
            terminus: terminus.trim().to_string(),
            index: d.trip_index,
            count: d.trips.len(),
            departure: trip.departure,
            end: trip.end,
            planned: trip.stops.iter().filter(|s| s.stops).count(),
            last: trip.stops.len().saturating_sub(1),
            next_stop: d.next_stop,
            at_stop: d.at_stop(),
            done: d.trip_done(),
            free: d.free,
            terminus_arrival: if d.trip_done() && trip.stops.last().is_some_and(|s| s.stops) { d.arrived_late() } else { None },
        })
    }

    /// A trip with a report: one on a line, or one that serves stops (an empty run to or from
    /// the depot has nothing to say).
    fn reportable(&self) -> bool {
        !self.line.is_empty() || self.planned > 0
    }
}

/// Follows the trip under way frame by frame and says, the frame it ends, what it came to.
#[derive(Default)]
pub struct TripRecorder {
    log: Option<TripLog>,
}

/// What is known of the trip under way.
struct TripLog {
    /// How it was the last frame.
    view: TripView,
    served: Vec<Served>,
    /// Stops the bus came to (a free drive's are counted, not judged).
    reached: usize,
    /// The counters when the trip began: followed until the bus leaves its first stop, so
    /// that the way there from the depot or the break at the terminus is not the trip's.
    start: Counters,
    started: bool,
    /// Its report was given - or it was over, or had nothing to report, when it was first seen
    /// (a situation resumed at a trip's last stop).
    reported: bool,
}

impl TripLog {
    fn new(view: TripView, now: Counters) -> TripLog {
        TripLog { started: view.next_stop > 0, reported: view.done || !view.reportable(), served: Vec::new(), reached: view.at_stop as usize, start: now, view }
    }

    fn report(&self, now: Counters, completed: bool) -> TripRun {
        let v = &self.view;
        let (counts, worst, average) = if v.free { ([0; 3], None, None) } else { tally(&self.served) };
        TripRun {
            line: v.line.clone(),
            tour: v.tour.trim().to_string(),
            terminus: v.terminus.clone(),
            trip: v.index + 1,
            trips: v.count,
            departure: v.departure,
            arrival: v.end,
            free: v.free,
            completed,
            planned: v.planned as i32,
            stops: if v.free { self.reached as i32 } else { self.served.len() as i32 },
            early: counts[0],
            late: counts[2],
            worst,
            average,
            seconds: (now.seconds - self.start.seconds).max(0.0),
            metres: (now.metres - self.start.metres).max(0.0),
            jolts: (now.jolts - self.start.jolts).max(0),
            crashes: (now.crashes - self.start.crashes).max(0),
            passengers: (now.boarded - self.start.boarded).max(0),
            ..Default::default()
        }
    }
}

impl TripRecorder {
    /// One frame of the duty: the trip under way (`view`), the stop the bus has just left with
    /// how late it arrived and left there (`PlayerDuty::update`'s answer), and the career's
    /// counters. Returns the report of a trip that ended this frame.
    pub fn observe(&mut self, view: TripView, served: Option<(f64, f64)>, now: Counters) -> Option<TripRun> {
        let mut out = None;
        // another trip (the duty went on, or a new duty began), or the same one again from an
        // earlier stop (a new duty of the same trip, a page that took it back): counted anew
        let (moved_on, again) = match self.log.as_ref() {
            Some(l) => (l.view.key != view.key, l.view.key == view.key && view.next_stop < l.view.next_stop),
            None => (false, false),
        };
        if moved_on || again {
            let old = self.log.take().expect("a trip under way");
            // (the duty goes on from a trip's last leg when the bus stands at the next trip's
            // first stop - a terminus whose stop lies away from where the buses stand: that
            // trip was driven to its end. One given up half way, or left for another duty,
            // was not)
            if moved_on && !old.reported && old.view.next_stop >= old.view.last && (old.reached > 0 || !old.served.is_empty()) {
                out = Some(old.report(now, false));
            }
        }
        let log = self.log.get_or_insert_with(|| TripLog::new(view.clone(), now));
        if !log.reported {
            if let Some((arrival, departure)) = served {
                log.served.push(Served { arrival, departure: Some(departure) });
            }
            if view.at_stop && !log.view.at_stop {
                log.reached += 1;
            }
            let left = (log.view.at_stop && !view.at_stop) || served.is_some();
            if !log.started {
                if left || view.next_stop > 0 {
                    log.started = true;
                } else {
                    log.start = now;
                }
            }
            if view.done {
                if let (false, Some(arrival)) = (view.free, view.terminus_arrival) {
                    log.served.push(Served { arrival, departure: None });
                }
                log.reported = true;
                log.view = view;
                return out.or_else(|| Some(log.report(now, true)));
            }
        }
        log.view = view;
        out
    }

    /// No duty: the trip under way is forgotten.
    pub fn forget(&mut self) {
        self.log = None;
    }
}

// --- the card -------------------------------------------------------------------------------

/// What the card says, worked out from the trip's report.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Card {
    /// The trip reached its last stop (else the duty went on from its last leg).
    pub completed: bool,
    /// "trip 2 of 5", or that it was a free drive.
    pub place: String,
    /// The line's plate (None: none), where it went and when it was to leave and arrive.
    pub line: Option<String>,
    pub terminus: String,
    pub span: String,
    /// How punctual it was; None on a free drive (or when no stop was served), which gets a
    /// note instead: its title and text.
    pub verdict: Option<Verdict>,
    pub note: (&'static str, &'static str),
    /// Three figures in tiles: the value, its colour and what it is.
    pub tiles: Vec<(String, Color, &'static str)>,
    /// A row of smaller ones: an icon, the value and what it is.
    pub facts: Vec<(&'static str, String, &'static str)>,
    /// How the trip is judged.
    pub judged: Judged,
}

/// The trip's evaluation as the card shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Judged {
    pub score: u32,
    pub grade: &'static str,
    pub stars: u32,
    pub ink: Color,
    pub xp: i64,
    /// Punctuality (None on a free drive), comfort, safety: 0 - 100.
    pub parts: [Option<u32>; 3],
    /// The fines and what they were for ("1 red light, 1 speed camera"); 0: none.
    pub fines: i64,
    pub offences: String,
}

/// The colour of a score.
fn score_ink(score: u32) -> Color {
    if score >= 75 {
        ON_TIME_INK
    } else if score >= 50 {
        NOW
    } else {
        LATE_INK
    }
}

fn judged(run: &TripRun) -> Judged {
    let e = judge::evaluate(run);
    let mut what = Vec::new();
    if run.red_lights > 0 {
        what.push(tr_with(if run.red_lights == 1 { "%{n} red light" } else { "%{n} red lights" }, &[("n", run.red_lights.to_string())]));
    }
    if run.speeding > 0 {
        what.push(tr_with(if run.speeding == 1 { "%{n} speed camera" } else { "%{n} speed cameras" }, &[("n", run.speeding.to_string())]));
    }
    Judged { score: e.score, grade: e.grade.label(), stars: e.grade.stars(), ink: score_ink(e.score), xp: e.xp, parts: [e.punctuality, Some(e.comfort), Some(e.safety)], fines: run.fines.max(0), offences: what.join(", ") }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Verdict {
    /// Per cent of the stops served on time, and the colour it is written in.
    pub percent: i64,
    pub ink: Color,
    /// The stops early, on time and late, and all of them.
    pub counts: [i32; 3],
    pub stops: i32,
    /// None early or late on a trip of three stops or more.
    pub spotless: bool,
}

/// A driving time: "34 min", over an hour "1h 05m".
fn duration(seconds: f64) -> String {
    let m = (seconds / 60.0).round().max(0.0) as i64;
    if m < 60 {
        tr_with("%{m} min", &[("m", m.to_string())])
    } else {
        tr_with("%{h}h %{m}m", &[("h", (m / 60).to_string()), ("m", format!("{:02}", m % 60))])
    }
}

/// The card of a trip.
pub(crate) fn card(run: &TripRun) -> Card {
    let timed = run.timed();
    let verdict = timed.then(|| {
        let on = run.on_time();
        let percent = (on as f64 * 100.0 / run.stops as f64).round() as i64;
        // (green as most stops on time, amber as half, red under that)
        let ink = if percent >= 80 { ON_TIME_INK } else if percent >= 50 { NOW } else { LATE_INK };
        Verdict { percent, ink, counts: [run.early, on, run.late], stops: run.stops, spotless: run.stops >= 3 && on == run.stops }
    });
    let delay = |d: Option<f64>| match d {
        Some(d) => (offset(d), punctuality(d).ink()),
        None => ("–".to_string(), TEXT_DIM),
    };
    let km = format!("{:.1} km", run.metres / 1000.0);
    let (jolts, crashes, people) = (("vibration", run.jolts.to_string(), "Jolts"), ("warning", run.crashes.to_string(), "Collisions"), ("group", run.passengers.to_string(), "Passengers"));
    let (tiles, facts) = if timed {
        let (worst, worst_ink) = delay(run.worst);
        let (average, average_ink) = delay(run.average);
        // (the kilometres say what they are: the row has room for four only so)
        (vec![(worst, worst_ink, "Worst delay"), (average, average_ink, "Average delay"), (duration(run.seconds), TEXT, "Driving time")], vec![("route", km, ""), jolts, crashes, people])
    } else {
        (vec![(run.stops.to_string(), TEXT, "Stops served"), (duration(run.seconds), TEXT, "Driving time"), (km, TEXT, "Distance")], vec![jolts, crashes, people])
    };
    Card {
        completed: run.completed,
        place: if run.free { omsi_ui::tr("Free drive").into_owned() } else { tr_with("trip %{k} of %{n}", &[("k", run.trip.to_string()), ("n", run.trips.max(run.trip).to_string())]) },
        line: Some(run.line.trim()).filter(|l| !l.is_empty()).map(str::to_string),
        terminus: run.terminus.clone(),
        span: format!("{} – {}", hhmm(run.departure), hhmm(run.arrival)),
        verdict,
        note: if run.free { ("Free drive", "No timetable: nothing to be early or late for") } else { ("Punctuality", "No stop of this trip was served") },
        tiles,
        facts,
        judged: judged(run),
    }
}

/// The card's width, and its parts' heights (at scale 1): the head (what trip, where to), the
/// punctuality (or the note in its place), the tiles, the row of facts and the foot.
pub(crate) const CARD_W: f32 = 380.0;
const HEAD: f32 = 94.0;
const VERDICT: f32 = 96.0;
const NOTE: f32 = 62.0;
const TILES: f32 = 64.0;
const FACTS: f32 = 30.0;
const JUDGED: f32 = 76.0;
/// The line under it saying what a fined trip's fines were for.
const FINED: f32 = 20.0;
const FOOT: f32 = 44.0;

/// How tall the card is (at scale 1).
pub(crate) fn card_height(c: &Card) -> f32 {
    HEAD + if c.verdict.is_some() { VERDICT } else { NOTE } + TILES + FACTS + JUDGED + if c.judged.fines > 0 { FINED } else { 0.0 } + FOOT
}

/// The card's scale: the navigator's (a third of the window's height, see `Navigator::frame`)
/// with the navigator's size setting, as far as the window has room for it.
pub(crate) fn card_scale(screen: (f32, f32), ui_scale: f32, follow_window: bool, nav_scale: f32, height: f32) -> f32 {
    let base = (screen.1 * 0.33).max(300.0);
    let base = if follow_window { base } else { base.min(480.0) };
    let s = base * ui_scale.max(0.1) * nav_scale.clamp(0.6, 2.0) / 360.0;
    s.min((screen.0 - 32.0).max(120.0) / CARD_W).min((screen.1 - 32.0).max(120.0) / height).max(0.4)
}

/// Draw the card into `r` (`CARD_W` by `card_height` at scale `s`), with the time it has left
/// to stay (0 - 1) along its bottom; `saved`: the trip is in the driver's record.
pub(crate) fn draw_card(pen: &mut Pen, c: &Card, r: Rect, s: f32, left: f32, saved: bool) {
    let radius = 12.0 * s;
    pen.p.rounded(r, radius, SHEET.alpha(0.97));
    pen.p.rounded_border(r, radius, 1.0, EDGE);
    let pad = 18.0 * s;
    let (x0, x1) = (r.x + pad, r.right() - pad);
    let w = x1 - x0;
    let ink = c.verdict.as_ref().map_or(accent_ink(), |v| v.ink);
    // the head: that the trip is over and which of the duty it was; the line, where it went,
    // its times
    let cy = r.y + 22.0 * s;
    pen.p.icon(pen.atlas, "sports_score", Vec2::new(x0 + 7.0 * s, cy), 15.0 * s, ink);
    let eyebrow = omsi_ui::tr(if c.completed { "Trip completed" } else { "Trip ended" }).to_uppercase();
    let pw = pen.width(&c.place, 11.5 * s, Weight::Medium).min(w * 0.45);
    pen.text_in(&c.place, 11.5 * s, Weight::Medium, Rect::new(x1 - pw - 2.0 * s, cy - 8.0 * s, pw + 2.0 * s, 16.0 * s), Align::Right, TEXT_FAINT);
    pen.text_in(&eyebrow, 10.5 * s, Weight::Bold, Rect::new(x0 + 20.0 * s, cy - 8.0 * s, (w - 28.0 * s - pw).max(0.0), 16.0 * s), Align::Left, TEXT_DIM);
    let cy = r.y + 52.0 * s;
    // (a trip without a line has no plate: "empty run" is the duty board's word for a run
    // that serves no stops, and this one did)
    let tx = match c.line.as_deref() {
        Some(l) => x0 + pen.plate(Some(l), x0, cy, s * 1.1) + 10.0 * s,
        None => x0,
    };
    pen.text_in(&c.terminus, 17.0 * s, Weight::Bold, Rect::new(tx, cy - 12.0 * s, (x1 - tx).max(0.0), 24.0 * s), Align::Left, TEXT);
    pen.text_in(&c.span, 12.0 * s, Weight::Medium, Rect::new(x0, r.y + 66.0 * s, w, 18.0 * s), Align::Left, TEXT_DIM);
    pen.p.rect(Rect::new(x0, r.y + (HEAD - 1.0) * s, w, 1.0), HAIRLINE);
    let mut y = r.y + HEAD * s;
    match &c.verdict {
        // how punctual: the share on time, large, in its colour; the bar of early, on time and
        // late under it, and how many each
        Some(v) => {
            let top = y + 10.0 * s;
            let big = format!("{} %", v.percent);
            let bw = pen.text_in(&big, 32.0 * s, Weight::Bold, Rect::new(x0, top, w * 0.45, 40.0 * s), Align::Left, v.ink);
            let lx = x0 + bw + 12.0 * s;
            let room = (x1 - lx - if v.spotless { 34.0 * s } else { 0.0 }).max(0.0);
            pen.text_in(&omsi_ui::tr("Punctuality").to_uppercase(), 10.0 * s, Weight::Bold, Rect::new(lx, top + 3.0 * s, room, 15.0 * s), Align::Left, TEXT_DIM);
            let line = tr_with("%{good} of %{all} stops on time", &[("good", v.counts[1].to_string()), ("all", v.stops.to_string())]);
            pen.text_in(&line, 13.0 * s, Weight::Medium, Rect::new(lx, top + 19.0 * s, room, 18.0 * s), Align::Left, TEXT_SOFT);
            if v.spotless {
                pen.p.circle(Vec2::new(x1 - 14.0 * s, top + 20.0 * s), 14.0 * s, LINE.alpha(0.16));
                pen.p.icon(pen.atlas, "emoji_events", Vec2::new(x1 - 14.0 * s, top + 20.0 * s), 18.0 * s, LINE);
            }
            // (each part of the bar a pill of its own, as long as its share)
            let bar = Rect::new(x0, top + 50.0 * s, w, 8.0 * s);
            let parts = v.counts.iter().filter(|n| **n > 0).count();
            let gap = 3.0 * s;
            let room = bar.w - gap * parts.saturating_sub(1) as f32;
            let total = v.counts.iter().sum::<i32>().max(1) as f32;
            let mut x = bar.x;
            for (k, n) in v.counts.iter().enumerate() {
                if *n <= 0 {
                    continue;
                }
                let pw = (room * *n as f32 / total).max(bar.h);
                pen.p.rounded(Rect::new(x, bar.y, pw.min(bar.right() - x), bar.h), bar.h * 0.5, [EARLY, ON_TIME, LATE][k]);
                x += pw + gap;
            }
            // the legend: early, on time, late and how many
            let mut lx = x0;
            let ly = bar.bottom() + 8.0 * s;
            for (k, word) in ["Early", "On time", "Late"].iter().enumerate() {
                pen.p.circle(Vec2::new(lx + 4.0 * s, ly + 8.0 * s), 4.0 * s, [EARLY, ON_TIME, LATE][k]);
                let ww = pen.text_in(word, 12.0 * s, Weight::Regular, Rect::new(lx + 12.0 * s, ly, (x1 - lx - 12.0 * s).max(0.0), 16.0 * s), Align::Left, TEXT_DIM);
                let n = v.counts[k].to_string();
                let nw = pen.text_in(&n, 12.0 * s, Weight::Bold, Rect::new(lx + 17.0 * s + ww, ly, (x1 - lx - 17.0 * s - ww).max(0.0), 16.0 * s), Align::Left, TEXT);
                lx += 17.0 * s + ww + nw + 18.0 * s;
            }
            y += VERDICT * s;
        }
        // a free drive (or a trip without a stop served): a note in the punctuality's place
        None => {
            let b = Rect::new(x0, y + 10.0 * s, w, 44.0 * s);
            pen.advice(b, accent_ink(), s);
            pen.p.icon(pen.atlas, if c.note.0 == "Free drive" { "alt_route" } else { "info" }, Vec2::new(b.x + 18.0 * s, b.center().y), 16.0 * s, accent_ink());
            pen.text_in(c.note.0, 12.5 * s, Weight::Bold, Rect::new(b.x + 34.0 * s, b.y + 5.0 * s, b.w - 42.0 * s, 17.0 * s), Align::Left, TEXT);
            pen.text_in(c.note.1, 11.5 * s, Weight::Medium, Rect::new(b.x + 34.0 * s, b.y + 22.0 * s, b.w - 42.0 * s, 16.0 * s), Align::Left, TEXT_DIM);
            y += NOTE * s;
        }
    }
    // the tiles: the worst and the average delay and the driving time (a free drive: the
    // stops, the time and the distance)
    let gap = 8.0 * s;
    let n = c.tiles.len().max(1) as f32;
    let tw = (w - gap * (n - 1.0)) / n;
    for (k, (value, colour, label)) in c.tiles.iter().enumerate() {
        let t = Rect::new(x0 + k as f32 * (tw + gap), y + 2.0 * s, tw, 52.0 * s);
        pen.p.rounded(t, 8.0 * s, FIELD.alpha(0.7));
        pen.text_in(value, 17.0 * s, Weight::Bold, Rect::new(t.x + 11.0 * s, t.y + 7.0 * s, t.w - 18.0 * s, 22.0 * s), Align::Left, *colour);
        pen.text_in(&omsi_ui::tr(label).to_uppercase(), 9.5 * s, Weight::Medium, Rect::new(t.x + 11.0 * s, t.y + 31.0 * s, t.w - 18.0 * s, 14.0 * s), Align::Left, TEXT_DIM);
    }
    y += TILES * s;
    // the facts in a row: an icon, the figure, what it is
    let n = c.facts.len().max(1) as f32;
    let fw = w / n;
    for (k, (icon, value, label)) in c.facts.iter().enumerate() {
        let fx = x0 + k as f32 * fw;
        let cy = y + 10.0 * s;
        pen.p.icon(pen.atlas, icon, Vec2::new(fx + 7.0 * s, cy), 14.0 * s, TEXT_DIM);
        let vx = fx + 18.0 * s;
        let vw = pen.text_in(value, 12.5 * s, Weight::Bold, Rect::new(vx, cy - 8.0 * s, (fw - 22.0 * s).max(0.0), 16.0 * s), Align::Left, TEXT);
        let (lx, room) = (vx + vw + 4.0 * s, fx + fw - 6.0 * s - (vx + vw + 4.0 * s));
        if room > 14.0 * s && !label.is_empty() {
            pen.text_in(label, 11.0 * s, Weight::Regular, Rect::new(lx, cy - 8.0 * s, room, 16.0 * s), Align::Left, TEXT_DIM);
        }
    }
    y += FACTS * s;
    draw_judged(pen, &c.judged, Rect::new(x0, y, w, JUDGED * s), s);
    y += (JUDGED + if c.judged.fines > 0 { FINED } else { 0.0 }) * s;
    // the foot: that the trip is in the record, how to put the card away, and the time it has
    // left as a line along its bottom
    pen.p.rect(Rect::new(x0, y, w, 1.0), HAIRLINE);
    let cy = y + 19.0 * s;
    let (icon, colour, said) = if saved { ("check_circle", ON_TIME_INK, "Saved to your service record") } else { ("error", LATE_INK, "The trip could not be saved") };
    pen.p.icon(pen.atlas, icon, Vec2::new(x0 + 6.0 * s, cy), 13.0 * s, colour);
    let hint = "Enter or click to close";
    let hw = pen.width(hint, 11.0 * s, Weight::Regular).min(w * 0.45);
    pen.text_in(hint, 11.0 * s, Weight::Regular, Rect::new(x1 - hw - 2.0 * s, cy - 8.0 * s, hw + 2.0 * s, 16.0 * s), Align::Right, TEXT_FAINT);
    pen.text_in(said, 11.5 * s, Weight::Medium, Rect::new(x0 + 17.0 * s, cy - 8.0 * s, (w - 17.0 * s - hw - 10.0 * s).max(0.0), 16.0 * s), Align::Left, TEXT_DIM);
    let line = Rect::new(r.x + radius, r.bottom() - 3.0 * s, (r.w - 2.0 * radius) * left.clamp(0.0, 1.0), 2.0 * s);
    if line.w > 0.5 {
        pen.p.rounded(line, 1.0 * s, ink.alpha(0.85));
    }
}

/// The evaluation's band: the score in a ring with the grade, its stars, the experience and
/// the fines on the left; the three parts as bars on the right.
fn draw_judged(pen: &mut Pen, j: &Judged, r: Rect, s: f32) {
    pen.p.rect(Rect::new(r.x, r.y, r.w, 1.0), HAIRLINE);
    let top = r.y + 10.0 * s;
    // the score
    let c = Vec2::new(r.x + 27.0 * s, top + 29.0 * s);
    pen.p.circle(c, 27.0 * s, j.ink.alpha(0.16));
    pen.p.circle(c, 22.5 * s, SHEET);
    pen.text_in(&j.score.to_string(), 19.0 * s, Weight::Bold, Rect::new(c.x - 24.0 * s, c.y - 12.0 * s, 48.0 * s, 24.0 * s), Align::Center, j.ink);
    let lx = r.x + 66.0 * s;
    let left_w = r.w * 0.58 - 76.0 * s;
    pen.text_in(&omsi_ui::tr(j.grade).to_uppercase(), 11.0 * s, Weight::Bold, Rect::new(lx, top + 2.0 * s, left_w, 15.0 * s), Align::Left, j.ink);
    for k in 0..5 {
        let on = (k as u32) < j.stars;
        pen.p.icon(pen.atlas, "star", Vec2::new(lx + 5.0 * s + k as f32 * 11.0 * s, top + 29.0 * s), 10.5 * s, if on { LINE } else { TEXT_FAINT.alpha(0.6) });
    }
    let xx = lx + 62.0 * s;
    pen.text_in(&tr_with("+%{n} XP", &[("n", j.xp.to_string())]), 13.0 * s, Weight::Bold, Rect::new(xx, top + 20.0 * s, (lx + left_w - xx).max(0.0), 18.0 * s), Align::Left, accent_ink());
    let (fine, ink) = if j.fines > 0 { (tr_with("Fines %{amount}", &[("amount", crate::drive_watch::euros(j.fines))]), LATE_INK) } else { (omsi_ui::tr("No fines").into_owned(), TEXT_DIM) };
    pen.text_in(&fine, 11.5 * s, Weight::Medium, Rect::new(lx, top + 41.0 * s, left_w, 16.0 * s), Align::Left, ink);
    // what the fines were for, across the card
    if j.fines > 0 && !j.offences.is_empty() {
        pen.p.icon(pen.atlas, "warning", Vec2::new(r.x + 7.0 * s, r.y + (JUDGED + 6.0) * s), 12.0 * s, LATE_INK);
        pen.text_in(&j.offences, 11.5 * s, Weight::Medium, Rect::new(r.x + 18.0 * s, r.y + (JUDGED - 2.0) * s, r.w - 18.0 * s, 16.0 * s), Align::Left, TEXT_SOFT);
    }
    // the parts
    let bx = r.x + r.w * 0.58;
    let bw = r.right() - bx;
    for (k, (label, v)) in ["Punctuality", "Comfort", "Safety"].iter().zip(j.parts).enumerate() {
        let y = top + k as f32 * 20.0 * s;
        let lw = bw * 0.45;
        pen.text_in(&omsi_ui::tr(label).to_uppercase(), 9.5 * s, Weight::Bold, Rect::new(bx, y, lw, 14.0 * s), Align::Left, TEXT_DIM);
        let value = v.map_or("–".to_string(), |v| v.to_string());
        pen.text_in(&value, 11.5 * s, Weight::Bold, Rect::new(r.right() - 28.0 * s, y - 1.0 * s, 28.0 * s, 15.0 * s), Align::Right, TEXT);
        let bar = Rect::new(bx + lw, y + 5.0 * s, (bw - lw - 34.0 * s).max(0.0), 4.0 * s);
        pen.p.rounded(bar, 2.0 * s, FIELD);
        if let Some(v) = v {
            let fw = (bar.w * v.min(100) as f32 / 100.0).max(bar.h);
            pen.p.rounded(Rect::new(bar.x, bar.y, fw.min(bar.w), bar.h), 2.0 * s, score_ink(v));
        }
    }
}

// --- on the screen ------------------------------------------------------------------------------

/// The report on the screen: what follows the trips, the card shown and what draws it.
#[derive(Default)]
pub struct TripReport {
    recorder: TripRecorder,
    shown: Option<Shown>,
    look: Option<Look>,
}

/// The card on the screen.
struct Shown {
    card: Card,
    saved: bool,
    /// Seconds left of the game running, and how far it has come in (0 - 1).
    left: f32,
    shown: f32,
    closing: bool,
    /// Where it is on the window (x0, y0, x1, y1).
    rect: [f32; 4],
}

/// What draws the card: into a texture of its own, as the navigator draws.
struct Look {
    gpu: Gpu,
    atlas: Atlas,
    fonts: Fonts,
    target: Option<(TextureId, u32, u32)>,
}

/// The trip judged last in this game, numbered from 1: what the phone's page shows as its own
/// end-of-trip card (`companion`).
static LATEST: std::sync::Mutex<Option<(u64, TripRun)>> = std::sync::Mutex::new(None);

/// The trip judged last in this game and its number, for the phone's page.
#[allow(dead_code)]
pub fn latest() -> Option<(u64, TripRun)> {
    LATEST.lock().ok()?.clone()
}

fn publish(run: &TripRun) {
    if let Ok(mut l) = LATEST.lock() {
        let n = l.as_ref().map_or(0, |x| x.0) + 1;
        *l = Some((n, run.clone()));
    }
}

impl TripReport {
    /// One frame of the duty, after `PlayerDuty::update` (whose answer `served` is): a trip
    /// that ended is written into the driver's record and its card shown.
    pub fn observe(&mut self, d: &PlayerDuty, served: Option<(f64, f64)>, career: &Career, watch: &crate::drive_watch::DriveWatch, map: &str, bus: &str) {
        let Some(view) = TripView::of(d) else { return };
        let Some(mut run) = self.recorder.observe(view, served, Counters::of(career)) else { return };
        // (what the drive watch saw on the trip's stretch of the career's clock)
        watch.fill(&mut run, career.seconds);
        let saved = match career.write_trip(&run, map, bus) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("the trip could not be written into the driver's record: {e}");
                false
            }
        };
        publish(&run);
        self.show(run, saved);
    }

    /// No duty: the trip under way is forgotten (the card shown stays).
    pub fn forget(&mut self) {
        self.recorder.forget();
    }

    /// Show the card of `run`.
    pub fn show(&mut self, run: TripRun, saved: bool) {
        self.shown = Some(Shown { card: card(&run), saved, left: SHOW_FOR, shown: 0.0, closing: false, rect: [0.0; 4] });
    }

    /// Where the card's bottom is on the window while it is shown (the drive watch's notice
    /// goes under it).
    pub fn bottom(&self) -> Option<f32> {
        self.shown.as_ref().filter(|s| s.rect[3] > 0.0).map(|s| s.rect[3])
    }

    /// Put the card away (Enter): whether there was one.
    pub fn dismiss(&mut self) -> bool {
        match self.shown.as_mut().filter(|s| !s.closing) {
            Some(s) => {
                s.closing = true;
                true
            }
            None => false,
        }
    }

    /// A click at (`x`, `y`) on the window: on the card it puts the card away (and is the
    /// card's).
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let over = self.shown.as_ref().is_some_and(|s| !s.closing && x >= s.rect[0] && x <= s.rect[2] && y >= s.rect[1] && y <= s.rect[3]);
        over && self.dismiss()
    }

    /// Draw the card for this frame, centred at the top of the interface's part of the window
    /// (`hud`: x, y, width, height), over the navigator and under the menus. `running`: the
    /// game runs (its time counts down the card's).
    pub fn frame(&mut self, renderer: &Renderer, scene: &mut Scene, hud: [f32; 4], settings: &crate::settings::Settings, dt: f32, running: bool) {
        let Some(sh) = self.shown.as_mut() else { return };
        if running {
            sh.left -= dt;
        }
        if sh.left <= 0.0 {
            sh.closing = true;
        }
        let target = if sh.closing { 0.0 } else { 1.0 };
        let step = dt / FADE;
        sh.shown = if sh.shown < target { (sh.shown + step).min(target) } else { (sh.shown - step).max(target) };
        if sh.closing && sh.shown <= 0.0 {
            self.shown = None;
            return;
        }
        let ch = card_height(&sh.card);
        let s = card_scale((hud[2], hud[3]), settings.ui_scale, settings.ui_scale_window, settings.nav_scale, ch);
        // (room round the card for its shadow)
        let m = (14.0 * s).round();
        let (cw, chp) = ((CARD_W * s).round(), (ch * s).round());
        let size = ((cw + 2.0 * m) as u32, (chp + 2.0 * m) as u32);
        // centred at the top, coming down a little as it comes in
        let eased = 1.0 - (1.0 - sh.shown).powi(3);
        let x0 = hud[0] + ((hud[2] - cw) * 0.5).round();
        let y0 = hud[1] + ((hud[3] * 0.08).max(16.0) - (1.0 - eased) * 14.0 * s).round();
        sh.rect = [x0, y0, x0 + cw, y0 + chp];
        let look = self.look.get_or_insert_with(|| {
            let atlas = Atlas::new(1024);
            Look { gpu: Gpu::new(&renderer.device, renderer.format(), samples(renderer.format()), atlas.size), atlas, fonts: Fonts::hanken(), target: None }
        });
        if look.target.is_none_or(|t| (t.1, t.2) != size) {
            if let Some((t, _, _)) = look.target.take() {
                renderer.free_texture(scene, t);
                scene.premultiplied.remove(&t);
            }
            let t = renderer.add_render_texture(scene, size.0, size.1);
            scene.premultiplied.insert(t);
            look.target = Some((t, size.0, size.1));
        }
        let Some((tex, _, _)) = look.target else { return };
        let Some(view) = renderer.texture_view(scene, tex) else { return };
        look.atlas.begin_frame();
        let mut p = Painter::new();
        let r = Rect::new(m, m, cw, chp);
        p.shadow(r, 12.0 * s, 18.0 * s, Color::rgba(0, 0, 0, 0.45));
        draw_card(&mut Pen { p: &mut p, atlas: &mut look.atlas, fonts: &look.fonts }, &sh.card, r, s, sh.left / SHOW_FOR, sh.saved);
        let (device, queue) = (&renderer.device, &renderer.queue);
        look.gpu.upload(device, queue, 0, &p.verts);
        look.gpu.upload_atlas(queue, &mut look.atlas);
        let layers = [Layer::flat([0.0, 0.0, size.0 as f32, size.1 as f32], 0.0, eased)];
        let draws = [Draw { buffer: 0, range: 0..p.verts.len() as u32, layer: 0, texture: 0 }];
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("trip report") });
        look.gpu.render(device, queue, &mut enc, &view, size, Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        queue.submit([enc.finish()]);
        scene.overlays.push((tex, [x0 - m, y0 - m, x0 + cw + m, y0 + chp + m]));
    }
}

/// Multisampling for the card's texture, as the navigator's (`navigator::map_samples`).
pub(crate) fn samples(format: wgpu::TextureFormat) -> u32 {
    if format.guaranteed_format_features(wgpu::Features::empty()).flags.sample_count_supported(4) {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trip of `n` stops as the recorder sees it, at stop `next` (`at`: standing there).
    fn view(n: usize, next: usize, at: bool, done: bool) -> TripView {
        TripView { key: (0, "5-1".into(), 8 * 3600), line: "5".into(), tour: "2".into(), terminus: "Rathaus".into(), index: 0, count: 3, departure: 8.0 * 3600.0, end: 8.5 * 3600.0, planned: n, last: n - 1, next_stop: next, at_stop: at, done, free: false, terminus_arrival: None }
    }

    fn counters(seconds: f64, metres: f64, jolts: i32) -> Counters {
        Counters { seconds, metres, jolts, crashes: 0, boarded: 10 * jolts }
    }

    /// Drives a trip of four stops from its first: (arrived, left) at each of the first three,
    /// then the last; returns what the recorder said at each frame.
    fn drive(times: [(f64, f64); 3], last: f64, free: bool) -> Vec<Option<TripRun>> {
        let mut r = TripRecorder::default();
        let v = |next, at, done| TripView { free, terminus_arrival: if done { Some(last) } else { None }, ..view(4, next, at, done) };
        let mut out = vec![
            // at the first stop, the break before the trip: the counters wait
            r.observe(v(0, true, false), None, counters(100.0, 5000.0, 3)),
            r.observe(v(0, true, false), None, counters(160.0, 5000.0, 4)),
        ];
        let mut t = 160.0;
        for (k, (arrived, left)) in times.iter().enumerate() {
            t += 120.0;
            // (a free drive's stops are not judged: the duty gives none)
            out.push(r.observe(v(k + 1, false, false), (!free).then_some((*arrived, *left)), counters(t, 5000.0 + 400.0 * (k + 1) as f64, 5 + k as i32)));
            t += 60.0;
            out.push(r.observe(v(k + 1, true, false), None, counters(t, 5400.0 + 400.0 * (k + 1) as f64, 5 + k as i32)));
        }
        out.push(r.observe(v(3, true, true), None, counters(t + 30.0, 6800.0, 9)));
        // the bus leaves the terminus: no second report
        out.push(r.observe(v(3, false, true), (!free).then_some((last, 600.0)), counters(t + 90.0, 6900.0, 9)));
        out
    }

    #[test]
    fn a_trip_reports_its_stops_as_omsi_counts_them() {
        let out = drive([(-30.0, 0.0), (200.0, 210.0), (-60.0, -150.0)], 100.0, false);
        let reports: Vec<&TripRun> = out.iter().flatten().collect();
        assert_eq!(reports.len(), 1, "one report, as the bus reaches the last stop");
        assert!(out[out.len() - 2].is_some());
        let r = reports[0];
        // late in at the second, out early at the third, the last on time
        assert_eq!((r.stops, r.early, r.on_time(), r.late, r.planned), (4, 1, 2, 1, 4));
        assert_eq!(r.worst, Some(210.0));
        assert_eq!(r.average, Some((0.0 + 210.0 - 150.0 + 100.0) / 4.0));
        assert!(r.completed && !r.free);
        assert_eq!((r.line.as_str(), r.tour.as_str(), r.terminus.as_str(), r.trip, r.trips), ("5", "2", "Rathaus", 1, 3));
        // from the moment it left its first stop: not the break before
        assert_eq!(r.seconds, 160.0 + 3.0 * 180.0 + 30.0 - 160.0);
        assert_eq!(r.metres, 1800.0);
        assert_eq!(r.jolts, 5);
        assert_eq!(r.passengers, 50);
    }

    #[test]
    fn a_free_drive_counts_its_stops_and_judges_none() {
        let out = drive([(0.0, 0.0); 3], 900.0, true);
        let r = out.iter().flatten().next().expect("a report").clone();
        assert!(r.free && r.completed);
        assert_eq!((r.stops, r.early, r.late, r.worst, r.average), (4, 0, 0, None, None));
        assert!(!r.timed());
    }

    #[test]
    fn stops_are_judged_to_the_second() {
        let s = |arrival: f64, departure: Option<f64>| Served { arrival, departure }.judged();
        assert_eq!(s(180.4, Some(0.0)), Punctuality::OnTime);
        assert_eq!(s(180.6, Some(0.0)), Punctuality::Late);
        assert_eq!(s(0.0, Some(-120.4)), Punctuality::OnTime);
        assert_eq!(s(0.0, Some(-120.6)), Punctuality::Early);
        // in late, out early: early (passengers miss the bus by it)
        assert_eq!(s(200.0, Some(-130.0)), Punctuality::Early);
        // at the last stop only the arrival counts: early is no fault there
        assert_eq!(s(-500.0, None), Punctuality::OnTime);
        let (counts, worst, average) = tally(&[]);
        assert_eq!((counts, worst, average), ([0; 3], None, None));
    }

    /// The duty goes on to the next trip from the last leg (the terminus lies away from where
    /// the buses stand): that trip was driven and is reported, not completed. One given up
    /// half way, or a duty left for another, is not.
    #[test]
    fn a_trip_left_on_its_last_leg_is_reported_and_one_given_up_is_not() {
        let mut r = TripRecorder::default();
        let c = counters(0.0, 0.0, 0);
        assert!(r.observe(view(3, 0, true, false), None, c).is_none());
        assert!(r.observe(view(3, 1, false, false), Some((10.0, 20.0)), c).is_none());
        assert!(r.observe(view(3, 1, true, false), None, c).is_none());
        assert!(r.observe(view(3, 2, false, false), Some((30.0, 40.0)), c).is_none());
        let next = TripView { key: (1, "5-2".into(), 9 * 3600), index: 1, ..view(3, 0, true, false) };
        let rep = r.observe(next.clone(), None, c).expect("the trip before is over");
        assert!(!rep.completed);
        assert_eq!((rep.stops, rep.planned), (2, 3));
        // the next trip given up after its first stop: the one after it reports nothing
        assert!(r.observe(TripView { next_stop: 1, at_stop: false, ..next }, Some((0.0, 0.0)), c).is_none());
        let third = TripView { key: (2, "5-3".into(), 10 * 3600), index: 2, ..view(3, 0, false, false) };
        assert!(r.observe(third, None, c).is_none());
    }

    /// A situation resumed at a trip's last stop has no trip to report; an empty run neither.
    /// A trip driven again (a new duty of the same trip) is reported again.
    #[test]
    fn a_trip_over_before_it_was_seen_says_nothing() {
        let mut r = TripRecorder::default();
        let c = counters(0.0, 0.0, 0);
        assert!(r.observe(TripView { terminus_arrival: Some(0.0), ..view(3, 2, true, true) }, None, c).is_none());
        assert!(r.observe(view(3, 2, false, true), Some((0.0, 0.0)), c).is_none());
        // the same trip anew, from its first stop
        assert!(r.observe(view(3, 0, true, false), None, c).is_none());
        assert!(r.observe(view(3, 1, false, false), Some((0.0, 0.0)), c).is_none());
        assert!(r.observe(view(3, 1, true, false), None, c).is_none());
        assert!(r.observe(view(3, 2, false, false), Some((0.0, 0.0)), c).is_none());
        assert!(r.observe(TripView { terminus_arrival: Some(0.0), ..view(3, 2, true, true) }, None, c).is_some());
        // given up half way and begun again from the first stop: the stops of the first go
        // are not counted twice
        let mut r = TripRecorder::default();
        assert!(r.observe(view(3, 0, true, false), None, c).is_none());
        assert!(r.observe(view(3, 1, false, false), Some((500.0, 500.0)), c).is_none());
        assert!(r.observe(view(3, 0, true, false), None, c).is_none());
        assert!(r.observe(view(3, 1, false, false), Some((0.0, 0.0)), c).is_none());
        assert!(r.observe(view(3, 1, true, false), None, c).is_none());
        assert!(r.observe(view(3, 2, false, false), Some((0.0, 0.0)), c).is_none());
        let rep = r.observe(TripView { terminus_arrival: Some(0.0), ..view(3, 2, true, true) }, None, c).expect("the second go");
        assert_eq!((rep.stops, rep.late), (3, 0));
        let mut r = TripRecorder::default();
        let empty = TripView { line: String::new(), planned: 0, ..view(3, 0, true, false) };
        assert!(r.observe(empty.clone(), None, c).is_none());
        assert!(r.observe(TripView { next_stop: 2, done: true, ..empty }, None, c).is_none());
    }

    fn run() -> TripRun {
        TripRun { line: "307".into(), tour: "16".into(), terminus: "Markgraf-Berthold-Platz".into(), trip: 2, trips: 5, departure: 19.0 * 3600.0 + 5.0 * 60.0, arrival: 19.0 * 3600.0 + 29.0 * 60.0, completed: true, planned: 14, stops: 12, early: 1, late: 2, worst: Some(252.0), average: Some(65.0), seconds: 1500.0, metres: 11_240.0, jolts: 2, crashes: 0, passengers: 37, watched: true, hard_brakes: 1, ..Default::default() }
    }

    #[test]
    fn the_card_says_how_the_trip_went() {
        let c = card(&run());
        assert_eq!(c.line.as_deref(), Some("307"));
        assert_eq!(c.span, "19:05 – 19:29");
        let v = c.verdict.as_ref().unwrap();
        // nine of twelve on time: amber
        assert_eq!((v.percent, v.counts, v.ink, v.spotless), (75, [1, 9, 2], NOW, false));
        assert_eq!(c.tiles.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["+4:12", "+1:05", "25 min"]);
        // (more than three minutes is late, in its colour)
        assert_eq!(c.tiles[0].1, LATE_INK);
        assert_eq!(c.facts.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(), ["11.2 km", "2", "0", "37"]);
        // judged: 75 % punctual, two jolts and a hard brake, safe
        assert_eq!(c.judged.parts, [Some(75), Some(80), Some(100)]);
        assert_eq!((c.judged.score, c.judged.grade, c.judged.fines), (84, "Good", 0));
        let fined = card(&TripRun { red_lights: 1, speeding: 2, fines: 290_00, ..run() });
        assert_eq!(fined.judged.fines, 290_00);
        // ("1 red light, 2 speed cameras" in the language of the moment: other tests change it)
        assert_eq!(fined.judged.offences.matches(", ").count(), 1);
        assert!(fined.judged.score < c.judged.score);
        let all = card(&TripRun { early: 0, late: 0, worst: Some(-20.0), seconds: 3900.0, ..run() });
        assert!(all.verdict.as_ref().unwrap().spotless);
        assert_eq!(all.verdict.as_ref().unwrap().ink, ON_TIME_INK);
        assert_eq!(all.tiles[2].0, "1h 05m");
        let free = card(&TripRun { free: true, stops: 8, ..run() });
        assert!(free.verdict.is_none());
        assert_eq!(free.note.0, "Free drive");
        assert_eq!(free.tiles[0].0, "8");
        assert!(card_height(&free) < card_height(&c));
    }

    /// The card draws inside its rect at every size, done or not, timed or free.
    #[test]
    fn the_card_draws_within_its_rect() {
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(1024);
        for run in [run(), TripRun { free: true, completed: false, line: String::new(), ..run() }, TripRun { early: 0, late: 0, ..run() }] {
            let c = card(&run);
            for s in [0.7, 1.0, 1.8] {
                let mut p = Painter::new();
                let r = Rect::new(40.0, 30.0, CARD_W * s, card_height(&c) * s);
                draw_card(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &c, r, s, 0.6, true);
                assert!(!p.verts.is_empty());
                for v in &p.verts {
                    let (x, y) = (v.pos[0] + v.ext[0] * v.width[0], v.pos[1] + v.ext[1] * v.width[0]);
                    assert!(x >= r.x - 0.5 && x <= r.right() + 0.5 && y >= r.y - 0.5 && y <= r.bottom() + 0.5, "({x}, {y}) outside {r:?} at {s}");
                }
            }
        }
    }

    /// The card fits the window: the navigator's size on a large one, smaller on a small one.
    #[test]
    fn the_card_fits_the_window() {
        let h = card_height(&card(&run()));
        let s = card_scale((1920.0, 1080.0), 1.0, true, 1.0, h);
        assert!((s - 0.99).abs() < 0.01, "{s}");
        assert!(card_scale((1920.0, 1080.0), 1.0, true, 2.0, h) > s);
        let small = card_scale((800.0, 420.0), 1.0, true, 2.0, h);
        assert!(h * small <= 420.0 - 32.0 + 0.01 && CARD_W * small <= 800.0);
    }

    /// The card's texts and the service record's for the trips are in the tables for the
    /// languages that matter most.
    #[test]
    fn the_reports_are_translated() {
        let keys = ["Trip completed", "Trip ended", "Worst delay", "Average delay", "Driving time", "Distance", "Jolts", "%{m} min", "Saved to your service record", "The trip could not be saved", "Enter or click to close", "No timetable: nothing to be early or late for", "No stop of this trip was served", "Trip reports", "Average", "Worst", "Free drive", "Punctuality", "%{good} of %{all} stops on time", "Stops served", "trip %{k} of %{n}", "Comfort", "Safety", "+%{n} XP", "Fines %{amount}", "No fines", "%{n} red light", "%{n} red lights", "%{n} speed camera", "%{n} speed cameras", "Excellent", "Good", "Fair", "Poor", "Bad"];
        for language in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for key in keys {
                let t = crate::_rust_i18n_try_translate(language, key);
                assert!(t.as_ref().is_some_and(|t| !t.trim().is_empty()), "{language}: {key}");
                assert_eq!(key.contains("%{m}"), t.unwrap().contains("%{m}"), "{language}: {key}");
            }
        }
    }

    /// Pictures of the card (a late trip, one all on time, a free drive), in Dutch:
    /// `OMSI_TRIP_REPORT_PREVIEW=<folder> cargo test -p omsi-app --lib trip_report -- --ignored`.
    #[test]
    #[ignore]
    fn preview_pictures() {
        let Ok(dir) = std::env::var("OMSI_TRIP_REPORT_PREVIEW") else { return };
        crate::ui_language("NLD");
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(2048);
        let s = 2.0;
        let runs = [TripRun { red_lights: 1, speeding: 1, fines: 250_00, ..run() }, TripRun { early: 0, late: 0, worst: Some(-40.0), average: Some(12.0), hard_brakes: 0, jolts: 0, ..run() }, TripRun { free: true, stops: 9, completed: true, ..run() }];
        let cards: Vec<Card> = runs.iter().map(card).collect();
        let tall = cards.iter().map(|c| card_height(c) * s).fold(0.0, f32::max);
        let mut img = image::RgbaImage::from_pixel(((CARD_W * s + 60.0) * 3.0) as u32, (tall + 60.0) as u32, image::Rgba([70, 84, 96, 255]));
        for (k, c) in cards.iter().enumerate() {
            let mut p = Painter::new();
            let r = Rect::new(30.0 + k as f32 * (CARD_W * s + 60.0), 30.0, CARD_W * s, card_height(c) * s);
            p.shadow(r, 12.0 * s, 18.0 * s, Color::rgba(0, 0, 0, 0.45));
            draw_card(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, c, r, s, 0.62, k != 2);
            crate::nav_duty::tests::raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e5, 1e5));
        }
        img.save(format!("{dir}/trip_report.png")).unwrap();
    }
}
