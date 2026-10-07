//! The duty in the navigator: Omsi-Hub's duty overlay - the live duty it laid over OMSI 2 in a
//! window of its own - built into openOMSI's navigator, so that the driver has one thing to
//! look at. The navigation (the map, the route, the turns, the way back) stays the
//! navigator's; this is what it says about the duty, in Omsi-Hub's dark style (blue is the
//! trip under way, yellow the line number's plate, red, green and blue punctuality):
//!
//! * under the small map (Shift+N's second step) a board: the trip under way with its line
//!   plate, where it goes and how it stands against the timetable; the stop just behind the
//!   bus and the ones ahead with their planned times (and, off the timetable, when the bus
//!   will be there); then what comes after this trip - the break at the terminus, and the
//!   line and tour the duty goes on with when they change. During the break the board looks
//!   ahead to the next trip.
//! * beside the city map a sheet with the whole duty: every trip with its times, the breaks
//!   between them and the changes of line, the trip under way opened up with all its stops.
//!
//! Everything is worked out from the game's own schedule state ([`PlayerDuty`]: the player's
//! plan, a composed duty's legs included) by pure functions into rows, and only the rows are
//! drawn - so what the panel says is tested without a window.
//!
//! Punctuality is OMSI 2's: more than three minutes after the timetable is late, more than
//! two before it early (Omsi-Hub made the same choice).

use glam::Vec2;
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Fonts, Painter, Rect, Weight};

use crate::schedule::{PlannedTrip, PlayerDuty};

// --- Omsi-Hub's palette (the launcher's `theme.rs`) ------------------------------------

/// The accent (the player's colour, `crate::accent`; Omsi-Hub's route blue was it).
pub(crate) fn accent() -> Color {
    crate::accent::base()
}
/// The accent as letters, icons and marks on the dark sheet (lighter): a link, a code, a note.
pub(crate) fn accent_ink() -> Color {
    crate::accent::shades().soft
}
/// The line number's plate, and its ink.
pub(crate) const LINE: Color = Color::rgba(255, 210, 63, 1.0);
const ON_LINE: Color = Color::rgba(26, 20, 0, 1.0);
/// Punctuality as fills (a chip): late, on time, early.
pub(crate) const LATE: Color = Color::rgba(217, 58, 48, 1.0);
pub(crate) const ON_TIME: Color = Color::rgba(26, 138, 79, 1.0);
// (blue whatever the accent: it is a meaning)
pub(crate) const EARLY: Color = Color::rgba(42, 117, 247, 1.0);
/// The same as letters on the dark panel (the fills are too dark to read as text there;
/// Omsi-Hub's overlay wrote its delay in these).
pub(crate) const LATE_INK: Color = Color::hex(0xFF5F57);
pub(crate) const ON_TIME_INK: Color = Color::hex(0x37D67A);
pub(crate) const EARLY_INK: Color = Color::hex(0x4DA3FF);
/// The stop the bus heads for, and the advice about what comes next.
pub(crate) const NOW: Color = Color::hex(0xF0B429);
/// The stops' rail ahead of the bus (the route: the accent) and behind it.
fn rail_ahead() -> Color {
    crate::accent::shades().hover
}
const RAIL_DONE: Color = Color::hex(0x4A5462);
/// The sheet (Omsi-Hub's `--vel-overlay`), a field in it, its edge and the hairlines.
pub(crate) const SHEET: Color = Color::rgba(20, 26, 38, 1.0);
pub(crate) const FIELD: Color = Color::rgba(36, 44, 62, 1.0);
pub(crate) const EDGE: Color = Color::rgba(255, 255, 255, 0.10);
pub(crate) const HAIRLINE: Color = Color::rgba(255, 255, 255, 0.07);
pub(crate) const TEXT: Color = Color::rgba(232, 235, 242, 1.0);
pub(crate) const TEXT_SOFT: Color = Color::rgba(196, 202, 214, 1.0);
pub(crate) const TEXT_DIM: Color = Color::rgba(149, 157, 176, 1.0);
pub(crate) const TEXT_FAINT: Color = Color::rgba(96, 104, 122, 1.0);
/// The advice's text (Omsi-Hub's "next trip" note: warm on the amber tint).
const ADVICE: Color = Color::rgba(245, 230, 200, 1.0);

// --- punctuality --------------------------------------------------------------------------

/// Later than this after the timetable is late, and more than this before it early (s).
pub(crate) const LATE_AFTER: f64 = 180.0;
pub(crate) const EARLY_BEFORE: f64 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Punctuality {
    Early,
    OnTime,
    Late,
}

/// How a bus `delay` seconds off the timetable (negative early) stands.
pub(crate) fn punctuality(delay: f64) -> Punctuality {
    if delay > LATE_AFTER {
        Punctuality::Late
    } else if delay < -EARLY_BEFORE {
        Punctuality::Early
    } else {
        Punctuality::OnTime
    }
}

impl Punctuality {
    /// Its colour as letters on the dark panel.
    pub(crate) fn ink(self) -> Color {
        match self {
            Punctuality::Early => EARLY_INK,
            Punctuality::OnTime => ON_TIME_INK,
            Punctuality::Late => LATE_INK,
        }
    }

    fn fill(self) -> Color {
        match self {
            Punctuality::Early => EARLY,
            Punctuality::OnTime => ON_TIME,
            Punctuality::Late => LATE,
        }
    }
}

/// A time of day (s) as `hh:mm` (past midnight it goes on from 00:00).
pub(crate) fn hhmm(t: f64) -> String {
    let m = (t / 60.0).floor() as i64;
    format!("{:02}:{:02}", m.div_euclid(60).rem_euclid(24), m.rem_euclid(60))
}

/// How far off the timetable, as Omsi-Hub writes it: `+3:12`, `−2:30`.
pub(crate) fn offset(delay: f64) -> String {
    let s = delay.abs().round() as i64;
    format!("{}{}:{:02}", if delay < 0.0 { '\u{2212}' } else { '+' }, s / 60, s % 60)
}

/// Whole minutes, rounded up (a break with 20 s left is not over).
fn minutes_up(s: f64) -> i64 {
    (s / 60.0).ceil().max(0.0) as i64
}

/// `key` in the interface's language with its `%{name}` placeholders filled in.
pub(crate) fn tr_with(key: &str, vars: &[(&str, String)]) -> String {
    let mut t = omsi_ui::tr(key).into_owned();
    for (k, v) in vars {
        t = t.replace(&format!("%{{{k}}}"), v);
    }
    t
}

// --- the duty as the panel sees it ----------------------------------------------------------

/// The player's duty at one moment, as much of it as the panel needs.
#[derive(Debug, Clone, Copy)]
pub struct DutyState<'a> {
    pub trips: &'a [PlannedTrip],
    /// The line and tour of each trip (a duty of several tours, `Schedule::player_plan`);
    /// empty: all are `line` and `tour`.
    pub tours: &'a [(String, String, usize)],
    pub line: &'a str,
    pub tour: &'a str,
    /// The trip under way, and the stop of it the bus is due at next.
    pub trip: usize,
    pub next_stop: usize,
    /// The bus stands at that stop.
    pub at_stop: bool,
    /// The bus has reached the trip's last stop.
    pub done: bool,
    /// How late the bus is (s, negative early), as the IBIS has it: at the end of a trip
    /// against the next trip's departure (`PlayerDuty::delay`).
    pub delay: f64,
}

impl<'a> DutyState<'a> {
    /// The duty at the time of day `now` (s); None for a duty without trips.
    pub fn of(d: &'a PlayerDuty, now: f64) -> Option<DutyState<'a>> {
        d.trips.get(d.trip_index)?;
        Some(DutyState { trips: &d.trips, tours: &d.tours, line: &d.line, tour: &d.tour, trip: d.trip_index, next_stop: d.next_stop, at_stop: d.at_stop(), done: d.trip_done(), delay: d.delay(now) })
    }

    /// The line and tour trip `k` belongs to.
    fn leg(&self, k: usize) -> (&str, &str) {
        match self.tours.get(k) {
            Some((l, t, _)) => (l.as_str(), t.as_str()),
            None => (self.line, self.tour),
        }
    }

    /// The line trip `k` shows; None for an empty run (a trip to or from the depot has no
    /// line of its own and carries nobody).
    fn shown_line(&self, k: usize) -> Option<String> {
        Some(self.trips.get(k)?.line.trim()).filter(|l| !l.is_empty()).map(str::to_string)
    }

    /// The line and tour trip `k` goes on with, when they are not those of the trip before.
    fn change(&self, k: usize) -> Option<(String, String)> {
        let (l0, t0) = self.leg(k.checked_sub(1)?);
        let (l1, t1) = self.leg(k);
        let same = l0.trim().eq_ignore_ascii_case(l1.trim()) && t0.trim().eq_ignore_ascii_case(t1.trim());
        (!same).then(|| (l1.trim().to_string(), t1.trim().to_string()))
    }

    /// The break before trip `k` at the terminus of the trip before it (whole minutes).
    fn pause_before(&self, k: usize) -> i64 {
        match (k.checked_sub(1).and_then(|j| self.trips.get(j)), self.trips.get(k)) {
            (Some(a), Some(b)) => ((b.departure - a.end) / 60.0).round().max(0.0) as i64,
            _ => 0,
        }
    }

    /// How the duty stands.
    pub fn status(&self) -> Status {
        if self.done {
            return if self.trip + 1 < self.trips.len() { Status::Break(-self.delay) } else { Status::Finished };
        }
        // (before a trip begins the bus waits for its departure: that is no "early")
        if self.next_stop == 0 && self.delay < 0.0 {
            return Status::DepartsIn(-self.delay);
        }
        Status::Running(self.delay)
    }

    /// The trip the panel is about: the one under way, or during the break at its terminus
    /// the next one (that is where the driver goes now).
    pub fn focus(&self) -> usize {
        if self.done && self.trip + 1 < self.trips.len() {
            self.trip + 1
        } else {
            self.trip
        }
    }

    /// The stop the bus heads for on the focus trip (an index into its stops).
    fn at(&self) -> usize {
        if self.focus() == self.trip {
            self.next_stop
        } else {
            0
        }
    }
}

/// How the duty stands: what the chip at the top says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    /// The trip has not begun; it leaves in this many seconds.
    DepartsIn(f64),
    /// Under way, this far off the timetable (s, negative early).
    Running(f64),
    /// At the terminus between two trips: this long until the next leaves (negative: it
    /// should have left).
    Break(f64),
    /// The last trip is driven.
    Finished,
}

impl Status {
    /// The chip's text.
    pub fn label(self) -> String {
        match self {
            Status::DepartsIn(s) => tr_with("leaves in %{m} min", &[("m", minutes_up(s).max(1).to_string())]),
            Status::Running(d) => match punctuality(d) {
                Punctuality::OnTime => omsi_ui::tr("on time").into_owned(),
                _ => offset(d),
            },
            Status::Break(left) if left >= 0.0 => tr_with("break, %{m} min left", &[("m", minutes_up(left).to_string())]),
            Status::Break(left) => tr_with("%{m} min over", &[("m", minutes_up(-left).max(1).to_string())]),
            Status::Finished => omsi_ui::tr("Duty finished").into_owned(),
        }
    }

    /// The chip's fill and ink.
    fn colours(self) -> (Color, Color) {
        match self {
            Status::Running(d) => (punctuality(d).fill(), Color::WHITE),
            Status::Break(left) if left < 0.0 => (LATE, Color::WHITE),
            Status::Finished => (ON_TIME, Color::WHITE),
            _ => (FIELD, TEXT),
        }
    }
}

/// Where a stop is for the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopState {
    Done,
    /// The one it heads for (or stands at).
    Now,
    Ahead,
}

/// The served stops of `trip` (a depot run passes stations it does not stop at) with their
/// index among all its stops, and which of them is the one the bus heads for when it is due
/// at stop `at` (the number of them when it is past them all).
fn served(trip: &PlannedTrip, at: usize) -> (Vec<(usize, &crate::schedule::PlannedStop)>, usize) {
    let v: Vec<_> = trip.stops.iter().enumerate().filter(|(_, s)| s.stops).collect();
    let now = v.iter().position(|(i, _)| *i >= at).unwrap_or(v.len());
    (v, now)
}

fn stop_state(j: usize, now: usize) -> StopState {
    match j.cmp(&now) {
        std::cmp::Ordering::Less => StopState::Done,
        std::cmp::Ordering::Equal => StopState::Now,
        std::cmp::Ordering::Greater => StopState::Ahead,
    }
}

// --- the board under the small map ----------------------------------------------------------

/// A row of the board under the small map.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// The trip: its line (None: an empty run), where it goes, which of how many, when it
    /// gets there (during the break: when it leaves), and how the duty stands.
    Head { line: Option<String>, terminus: String, index: usize, count: usize, time: f64, status: Status },
    /// A stop: its planned time, when the bus will be there when that is off the timetable,
    /// its name, where it is for the bus, and whether it is the trip's last.
    Stop { planned: f64, expected: Option<f64>, name: String, state: StopState, last: bool },
    /// The stops of the trip not shown, and where they go.
    More { count: usize, terminus: String },
    /// What comes after this trip: when it leaves, its line and terminus, the break before
    /// it (minutes), and the line and tour the duty goes on with when they change.
    Next { departure: f64, line: Option<String>, terminus: String, pause: i64, change: Option<(String, String)> },
    /// The duty goes on with another line or tour (said during the break before it).
    Change { line: String, tour: String },
    /// The duty is driven to its end.
    Finished,
    /// No duty: a free drive.
    NoDuty,
}

/// The board under the small map: the trip (during the break at its terminus, the next
/// one), the stop behind the bus and up to `ahead` from the one it heads for, how many more
/// there are, and what comes after this trip. A free drive gets one row saying so.
pub fn board(state: Option<&DutyState>, ahead: usize) -> Vec<Row> {
    let Some(d) = state else { return vec![Row::NoDuty] };
    let status = d.status();
    let k = d.focus();
    let Some(trip) = d.trips.get(k) else { return vec![Row::NoDuty] };
    let on_break = matches!(status, Status::Break(_));
    let terminus = trip.terminus.trim().to_string();
    let mut rows = vec![Row::Head { line: d.shown_line(k), terminus: terminus.clone(), index: k + 1, count: d.trips.len(), time: if on_break { trip.departure } else { trip.end }, status }];
    if status == Status::Finished {
        rows.push(Row::Finished);
        return rows;
    }
    if on_break {
        if let Some((line, tour)) = d.change(k) {
            rows.push(Row::Change { line, tour });
        }
    }
    // when the bus will be at the stops ahead, when it is off the timetable (at the stop it
    // stands at it is there now)
    let late = match status {
        Status::Running(late) if punctuality(late) != Punctuality::OnTime => Some(late),
        _ => None,
    };
    let (stops, now) = served(trip, d.at());
    let now = now.min(stops.len().saturating_sub(1));
    let from = now.saturating_sub(1);
    let to = (now + ahead.max(1)).min(stops.len());
    for (j, (_, st)) in stops.iter().enumerate().take(to).skip(from) {
        let state = stop_state(j, now);
        let expected = late.filter(|_| state == StopState::Ahead || (state == StopState::Now && !d.at_stop)).map(|l| st.arr + l);
        rows.push(Row::Stop { planned: st.arr, expected, name: st.name.trim().to_string(), state, last: j + 1 == stops.len() });
    }
    if to < stops.len() {
        rows.push(Row::More { count: stops.len() - to, terminus });
    }
    if !on_break {
        if let Some(next) = d.trips.get(k + 1) {
            rows.push(Row::Next { departure: next.departure, line: d.shown_line(k + 1), terminus: next.terminus.trim().to_string(), pause: d.pause_before(k + 1), change: d.change(k + 1) });
        }
    }
    rows
}

/// The board with as many stops ahead as fit in `room` points at scale `s` (four at most,
/// two at least), and its height in points.
pub fn board_fitting(state: Option<&DutyState>, s: f32, room: f32) -> (Vec<Row>, f32) {
    let mut out = (Vec::new(), 0.0);
    for ahead in (2..=4).rev() {
        let rows = board(state, ahead);
        let h = (board_height(&rows) * s).round();
        out = (rows, h);
        if h <= room {
            break;
        }
    }
    out
}

/// The board within `room` points at scale `s` whatever it costs, for a navigator the player
/// sized (its height is the player's): as many stops ahead as fit, four at most; when not even
/// one does, what matters least goes first - how many more there are, the change of line, what
/// comes after the trip, the stop behind the bus, the stops after the next one - and with no
/// room for the trip's head at all, no board. Returns the rows and their height in points.
pub fn board_within(state: Option<&DutyState>, s: f32, room: f32) -> (Vec<Row>, f32) {
    let height = |rows: &[Row]| (board_height(rows) * s).round();
    for ahead in (1..=4).rev() {
        let rows = board(state, ahead);
        if height(&rows) <= room {
            let h = height(&rows);
            return (rows, h);
        }
    }
    let mut rows = board(state, 1);
    let drops: [fn(&Row) -> bool; 4] = [
        |r| matches!(r, Row::More { .. }),
        |r| matches!(r, Row::Change { .. }),
        |r| matches!(r, Row::Next { .. }),
        |r| matches!(r, Row::Stop { state: StopState::Done, .. }),
    ];
    for drop in drops {
        if height(&rows) <= room {
            break;
        }
        rows.retain(|r| !drop(r));
    }
    // (the stops after the one the bus heads for, from the last)
    while height(&rows) > room && rows.iter().filter(|r| matches!(r, Row::Stop { .. })).count() > 1 {
        let Some(k) = rows.iter().rposition(|r| matches!(r, Row::Stop { .. })) else { break };
        rows.remove(k);
    }
    while height(&rows) > room && rows.len() > 1 {
        rows.pop();
    }
    if height(&rows) > room {
        return (Vec::new(), 0.0);
    }
    let h = height(&rows);
    (rows, h)
}

/// How tall a row of the board is (at scale 1).
pub(crate) fn row_height(row: &Row) -> f32 {
    match row {
        Row::Head { .. } => 48.0,
        Row::Stop { .. } => 22.0,
        Row::More { .. } => 19.0,
        Row::Next { pause, change, .. } => if *pause > 0 || change.is_some() { 54.0 } else { 36.0 },
        Row::Change { .. } | Row::Finished => 36.0,
        Row::NoDuty => 30.0,
    }
}

/// Space above the first row and below the last (at scale 1).
const BOARD_TOP: f32 = 4.0;
const BOARD_BOTTOM: f32 = 8.0;

/// How tall the board is (at scale 1).
pub fn board_height(rows: &[Row]) -> f32 {
    BOARD_TOP + rows.iter().map(row_height).sum::<f32>() + BOARD_BOTTOM
}

/// Where the panel draws: the navigator's painter, atlas and fonts.
pub(crate) struct Pen<'a> {
    pub p: &'a mut Painter,
    pub atlas: &'a mut Atlas,
    pub fonts: &'a Fonts,
}

impl Pen<'_> {
    pub(crate) fn text_in(&mut self, text: &str, px: f32, weight: Weight, r: Rect, align: Align, c: Color) -> f32 {
        self.p.text_in(self.atlas, self.fonts, text, px, weight, r, align, c)
    }

    pub(crate) fn width(&self, text: &str, px: f32, weight: Weight) -> f32 {
        self.fonts.width(text, px, weight)
    }

    /// The line number's plate, as printed on Omsi-Hub's duty cards (an empty run gets an
    /// outlined tag saying so), from `x` and centred on `cy`; returns its width.
    pub(crate) fn plate(&mut self, line: Option<&str>, x: f32, cy: f32, s: f32) -> f32 {
        let h = 19.0 * s;
        match line {
            Some(l) => {
                let px = 12.0 * s;
                let l = self.fonts.fit(l, px, Weight::Black, 70.0 * s);
                let w = (self.width(&l, px, Weight::Black) + 12.0 * s).max(26.0 * s);
                let r = Rect::new(x, cy - h * 0.5, w, h);
                self.p.rounded(r, 5.0 * s, LINE);
                self.text_in(&l, px, Weight::Black, r, Align::Center, ON_LINE);
                w
            }
            None => {
                let px = 10.5 * s;
                let w = self.width("Empty run", px, Weight::Bold) + 12.0 * s;
                let r = Rect::new(x, cy - h * 0.5, w, h);
                self.p.rounded_border(r, 5.0 * s, 1.0, Color::WHITE.alpha(0.22));
                self.text_in("Empty run", px, Weight::Bold, r, Align::Center, TEXT_DIM);
                w
            }
        }
    }

    /// The status chip, its right edge at `right` and centred on `cy`; returns its width.
    fn chip(&mut self, status: Status, right: f32, cy: f32, s: f32) -> f32 {
        let (fill, ink) = status.colours();
        let text = status.label();
        let px = 12.0 * s;
        let w = self.width(&text, px, Weight::Bold) + 18.0 * s;
        let r = Rect::new(right - w, cy - 10.5 * s, w, 21.0 * s);
        self.p.rounded(r, 10.5 * s, fill);
        self.text_in(&text, px, Weight::Bold, r, Align::Center, ink);
        w
    }

    /// A stop on the rail: its pin at `x` with the rail through it (from the row above when
    /// there is a stop there, on down when there is one below; grey where the bus has been),
    /// its name from `name_x`, and on the right its planned time when given (and before
    /// that when the bus will be there) and a tag on the trip's last stop.
    #[allow(clippy::too_many_arguments)]
    fn stop(&mut self, r: Rect, x: f32, name_x: f32, name: &str, planned: Option<f64>, expected: Option<f64>, state: StopState, last: bool, above: bool, below: bool, s: f32) {
        let cy = r.center().y;
        let rail = if state == StopState::Done { RAIL_DONE } else { rail_ahead() };
        if above {
            self.p.rect(Rect::new(x - 1.5 * s, r.y, 3.0 * s, r.h * 0.5), rail);
        }
        if below {
            self.p.rect(Rect::new(x - 1.5 * s, cy, 3.0 * s, r.h * 0.5), rail);
        }
        match state {
            StopState::Done => self.p.circle(Vec2::new(x, cy), 4.0 * s, RAIL_DONE),
            StopState::Now => {
                self.p.circle(Vec2::new(x, cy), 9.0 * s, NOW.alpha(0.22));
                self.p.circle(Vec2::new(x, cy), 6.0 * s, NOW);
            }
            StopState::Ahead => {
                self.p.circle(Vec2::new(x, cy), 4.5 * s, rail_ahead());
                self.p.circle(Vec2::new(x, cy), 2.5 * s, SHEET);
            }
        }
        // the times on the right: the planned one, and before it when the bus will be there
        let tpx = 12.0 * s;
        let mut right = r.right();
        if let Some(planned) = planned {
            let text = hhmm(planned);
            let (weight, c) = time_style(state);
            let tw = self.width(&text, tpx, weight);
            self.text_in(&text, tpx, weight, Rect::new(right - tw - 2.0 * s, r.y, tw + 2.0 * s, r.h), Align::Right, c);
            right -= tw + 8.0 * s;
            if let Some(e) = expected {
                let et = hhmm(e);
                let ew = self.width(&et, tpx, Weight::Bold);
                let ink = punctuality(e - planned).ink();
                self.text_in(&et, tpx, Weight::Bold, Rect::new(right - ew - 2.0 * s, r.y, ew + 2.0 * s, r.h), Align::Right, ink);
                right -= ew + 8.0 * s;
            }
        }
        if last {
            let px = 10.0 * s;
            let w = self.width("terminus", px, Weight::Medium) + 10.0 * s;
            let t = Rect::new(right - w, cy - 8.0 * s, w, 16.0 * s);
            self.p.rounded_border(t, 4.0 * s, 1.0, Color::WHITE.alpha(0.16));
            self.text_in("terminus", px, Weight::Medium, t, Align::Center, TEXT_DIM);
            right -= w + 8.0 * s;
        }
        let (px, weight, c) = match state {
            StopState::Done => (12.5 * s, Weight::Regular, TEXT_FAINT),
            StopState::Now => (14.0 * s, Weight::Bold, Color::WHITE),
            StopState::Ahead => (13.0 * s, Weight::Medium, TEXT_SOFT),
        };
        self.text_in(name, px, weight, Rect::new(name_x, r.y, (right - name_x).max(0.0), r.h), Align::Left, c);
    }

    /// An advice box (amber tint, a bar on the left) over `r`.
    pub(crate) fn advice(&mut self, r: Rect, tint: Color, s: f32) {
        self.p.rounded(r, 6.0 * s, tint.alpha(0.13));
        self.p.rounded(Rect::new(r.x, r.y, 3.0 * s, r.h), 1.5 * s, tint);
    }
}

/// How a stop's planned time is written: bold and amber for the one the bus heads for.
fn time_style(state: StopState) -> (Weight, Color) {
    match state {
        StopState::Done => (Weight::Medium, TEXT_FAINT),
        StopState::Now => (Weight::Bold, NOW),
        StopState::Ahead => (Weight::Medium, TEXT_DIM),
    }
}

/// Draw the board's `rows` into `area` (the panel's width, `board_height` tall at scale `s`).
pub(crate) fn draw_board(pen: &mut Pen, rows: &[Row], area: Rect, s: f32) {
    let pad = 11.0 * s;
    let (x0, x1) = (area.x + pad, area.right() - pad);
    let w = x1 - x0;
    pen.p.rect(Rect::new(x0, area.y, w, 1.0), HAIRLINE);
    let mut y = area.y + BOARD_TOP * s;
    let is_stop = |i: Option<&Row>| matches!(i, Some(Row::Stop { .. }));
    for (i, row) in rows.iter().enumerate() {
        let h = row_height(row) * s;
        let r = Rect::new(x0, y, w, h);
        match row {
            Row::Head { line, terminus, index, count, time, status } => {
                let cy = r.y + 15.0 * s;
                let pw = pen.plate(line.as_deref(), x0, cy, s);
                let cw = pen.chip(*status, x1, cy, s);
                let tx = x0 + pw + 8.0 * s;
                pen.text_in(terminus, 14.0 * s, Weight::Bold, Rect::new(tx, cy - 10.0 * s, (x1 - cw - 8.0 * s - tx).max(0.0), 20.0 * s), Align::Left, TEXT);
                let when = if matches!(status, Status::Break(_)) { "leaves %{time}" } else { "arrives %{time}" };
                let sub = format!("{}  ·  {}", tr_with("trip %{k} of %{n}", &[("k", index.to_string()), ("n", count.to_string())]), tr_with(when, &[("time", hhmm(*time))]));
                pen.text_in(&sub, 11.5 * s, Weight::Medium, Rect::new(x0, r.y + 28.0 * s, w, 16.0 * s), Align::Left, TEXT_DIM);
            }
            Row::Stop { planned, expected, name, state, last } => {
                pen.stop(r, x0 + 6.0 * s, x0 + 19.0 * s, name, Some(*planned), *expected, *state, *last, is_stop(i.checked_sub(1).and_then(|j| rows.get(j))), is_stop(rows.get(i + 1)), s);
            }
            Row::More { count, terminus } => {
                let t = tr_with("and %{n} more to %{terminus}", &[("n", count.to_string()), ("terminus", terminus.clone())]);
                pen.text_in(&t, 11.5 * s, Weight::Medium, Rect::new(x0 + 19.0 * s, r.y, w - 19.0 * s, r.h), Align::Left, TEXT_FAINT);
            }
            Row::Next { departure, line, terminus, pause, change } => {
                let b = Rect::new(x0, r.y + 5.0 * s, w, h - 9.0 * s);
                pen.advice(b, NOW, s);
                let cy = b.y + 13.0 * s;
                let label = tr_with("Next %{time}", &[("time", hhmm(*departure))]);
                let lx = b.x + 12.0 * s;
                let lw = pen.text_in(&label, 12.5 * s, Weight::Bold, Rect::new(lx, cy - 9.0 * s, w * 0.4, 18.0 * s), Align::Left, ADVICE);
                let px = lx + lw + 8.0 * s;
                let plw = pen.plate(line.as_deref(), px, cy, s * 0.9);
                let tx = px + plw + 7.0 * s;
                pen.text_in(terminus, 12.5 * s, Weight::Medium, Rect::new(tx, cy - 9.0 * s, (b.right() - 8.0 * s - tx).max(0.0), 18.0 * s), Align::Left, ADVICE);
                let mut parts = Vec::new();
                if *pause > 0 {
                    parts.push(tr_with("%{m} min break", &[("m", pause.to_string())]));
                }
                if let Some((l, t)) = change {
                    parts.push(tr_with("Continue on line %{line}, tour %{tour}", &[("line", l.clone()), ("tour", t.clone())]));
                }
                if !parts.is_empty() {
                    pen.text_in(&parts.join("  ·  "), 11.5 * s, Weight::Medium, Rect::new(lx, b.y + 23.0 * s, b.right() - 8.0 * s - lx, 18.0 * s), Align::Left, ADVICE.alpha(0.85));
                }
            }
            Row::Change { line, tour } => {
                let b = Rect::new(x0, r.y + 4.0 * s, w, h - 8.0 * s);
                pen.advice(b, NOW, s);
                pen.p.icon(pen.atlas, "sync_alt", Vec2::new(b.x + 18.0 * s, b.center().y), 15.0 * s, NOW);
                let t = tr_with("Continue on line %{line}, tour %{tour}", &[("line", line.clone()), ("tour", tour.clone())]);
                pen.text_in(&t, 12.5 * s, Weight::Bold, Rect::new(b.x + 32.0 * s, b.y, b.w - 40.0 * s, b.h), Align::Left, ADVICE);
            }
            Row::Finished => {
                let b = Rect::new(x0, r.y + 4.0 * s, w, h - 8.0 * s);
                pen.advice(b, ON_TIME_INK, s);
                pen.p.icon(pen.atlas, "check_circle", Vec2::new(b.x + 18.0 * s, b.center().y), 15.0 * s, ON_TIME_INK);
                pen.text_in("Duty finished", 12.5 * s, Weight::Bold, Rect::new(b.x + 32.0 * s, b.y, b.w - 40.0 * s, b.h), Align::Left, TEXT);
            }
            Row::NoDuty => {
                pen.text_in("No duty running", 12.5 * s, Weight::Medium, r, Align::Left, TEXT_DIM);
            }
        }
        y += h;
    }
}

// --- the sheet beside the city map ----------------------------------------------------------

/// Where a trip is in the duty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TripState {
    Done,
    Now,
    Ahead,
}

/// A row of the duty sheet beside the city map.
#[derive(Debug, Clone, PartialEq)]
pub enum SheetRow {
    /// A trip: its number, when it leaves and arrives, its line (None: an empty run), where
    /// it goes, how many stops it serves, and where it is in the duty.
    Trip { index: usize, departure: f64, arrival: f64, line: Option<String>, terminus: String, stops: usize, state: TripState },
    /// A stop of the trip the duty is on.
    Stop { planned: f64, name: String, state: StopState, last: bool },
    /// The break at a terminus before the next trip (minutes).
    Pause { minutes: i64 },
    /// The duty goes on with another line or tour.
    Change { line: String, tour: String },
}

/// The whole duty for the sheet: every trip, the breaks and changes of line between them,
/// and the stops of the trip it is on (during a break, the next one).
pub fn sheet(d: &DutyState) -> Vec<SheetRow> {
    let finished = d.status() == Status::Finished;
    let focus = d.focus();
    let mut rows = Vec::new();
    for (k, trip) in d.trips.iter().enumerate() {
        if k > 0 {
            let minutes = d.pause_before(k);
            if minutes > 0 {
                rows.push(SheetRow::Pause { minutes });
            }
            if let Some((line, tour)) = d.change(k) {
                rows.push(SheetRow::Change { line, tour });
            }
        }
        let (stops, now) = served(trip, if k == focus { d.at() } else { 0 });
        let state = match k.cmp(&focus) {
            std::cmp::Ordering::Less => TripState::Done,
            std::cmp::Ordering::Equal => TripState::Now,
            std::cmp::Ordering::Greater => TripState::Ahead,
        };
        rows.push(SheetRow::Trip { index: k + 1, departure: trip.departure, arrival: trip.end, line: d.shown_line(k), terminus: trip.terminus.trim().to_string(), stops: stops.len(), state });
        if k == focus {
            let n = stops.len();
            for (j, (_, st)) in stops.into_iter().enumerate() {
                let state = if finished { StopState::Done } else { stop_state(j, now) };
                rows.push(SheetRow::Stop { planned: st.arr, name: st.name.trim().to_string(), state, last: j + 1 == n });
            }
        }
    }
    rows
}

/// How tall a row of the sheet is (at scale 1).
fn sheet_row_height(row: &SheetRow) -> f32 {
    match row {
        SheetRow::Trip { .. } => 44.0,
        SheetRow::Stop { .. } => 23.0,
        SheetRow::Pause { .. } | SheetRow::Change { .. } => 24.0,
    }
}

/// How tall the sheet's list is (at scale 1).
pub fn sheet_height(rows: &[SheetRow]) -> f32 {
    rows.iter().map(sheet_row_height).sum()
}

/// Where in the list (at scale 1) the stop the bus heads for is, else the trip it is on:
/// the list keeps that in view.
pub fn sheet_focus(rows: &[SheetRow]) -> f32 {
    let mut y = 0.0;
    let mut trip = None;
    for row in rows {
        match row {
            SheetRow::Stop { state: StopState::Now, .. } => return y,
            SheetRow::Trip { state: TripState::Now, .. } => trip = Some(y),
            _ => {}
        }
        y += sheet_row_height(row);
    }
    trip.unwrap_or(0.0)
}

/// The sheet's top part above the list (at scale 1): the title and the tiles.
pub const SHEET_HEAD: f32 = 128.0;

/// Draw the sheet's frame and head into `r`: the title with the duty's span, the status
/// chip, and tiles for the time in the game, the delay and the trip. Returns the rect the
/// list goes in.
pub(crate) fn draw_sheet_head(pen: &mut Pen, d: &DutyState, r: Rect, time: f64, s: f32) -> Rect {
    pen.p.shadow(r, 12.0 * s, 18.0 * s, Color::rgba(0, 0, 0, 0.45));
    pen.p.rounded(r, 12.0 * s, SHEET.alpha(0.96));
    pen.p.rounded_border(r, 12.0 * s, 1.0, EDGE);
    let pad = 16.0 * s;
    let (x0, x1) = (r.x + pad, r.right() - pad);
    // the title, and the duty's span below it
    pen.p.icon(pen.atlas, "schedule", Vec2::new(x0 + 10.0 * s, r.y + 27.0 * s), 20.0 * s, TEXT);
    let status = d.status();
    let cw = pen.chip(status, x1, r.y + 27.0 * s, s);
    let tx = x0 + 28.0 * s;
    pen.text_in("Duty", 18.0 * s, Weight::Bold, Rect::new(tx, r.y + 15.0 * s, (x1 - cw - 8.0 * s - tx).max(0.0), 24.0 * s), Align::Left, TEXT);
    let span = match (d.trips.first(), d.trips.last()) {
        (Some(a), Some(b)) => format!("{}  ·  {} – {}", tr_with(if d.trips.len() == 1 { "%{n} trip" } else { "%{n} trips" }, &[("n", d.trips.len().to_string())]), hhmm(a.departure), hhmm(b.end)),
        _ => String::new(),
    };
    pen.text_in(&span, 12.0 * s, Weight::Medium, Rect::new(tx, r.y + 38.0 * s, x1 - tx, 18.0 * s), Align::Left, TEXT_DIM);
    // the tiles: the time in the game, the delay (in its colour; none before a trip and
    // during a break), the trip
    let (delay, ink) = match status {
        Status::Running(late) => (offset(late), punctuality(late).ink()),
        _ => ("–".to_string(), TEXT_DIM),
    };
    let tiles = [(hhmm(time), TEXT, "In the game"), (delay, ink, "Delay"), (format!("{} / {}", d.focus() + 1, d.trips.len()), TEXT, "Trip")];
    let gap = 8.0 * s;
    let tw = (x1 - x0 - gap * 2.0) / 3.0;
    for (k, (value, c, label)) in tiles.iter().enumerate() {
        let t = Rect::new(x0 + k as f32 * (tw + gap), r.y + 64.0 * s, tw, 50.0 * s);
        pen.p.rounded(t, 8.0 * s, FIELD.alpha(0.7));
        pen.text_in(value, 17.0 * s, Weight::Bold, Rect::new(t.x + 10.0 * s, t.y + 6.0 * s, t.w - 20.0 * s, 22.0 * s), Align::Left, *c);
        let caps = omsi_ui::tr(label).to_uppercase();
        pen.text_in(&caps, 9.5 * s, Weight::Bold, Rect::new(t.x + 10.0 * s, t.y + 30.0 * s, t.w - 20.0 * s, 14.0 * s), Align::Left, TEXT_DIM);
    }
    pen.p.rect(Rect::new(r.x, r.y + SHEET_HEAD * s - 1.0, r.w, 1.0), HAIRLINE);
    Rect::new(r.x, r.y + SHEET_HEAD * s, r.w, (r.h - SHEET_HEAD * s - 8.0 * s).max(0.0))
}

/// Draw the sheet's list into `view` (in the list's own layer, clipped to it), scrolled
/// `scroll` points (at scale `s`) down; rows out of view are left out.
pub(crate) fn draw_sheet_list(pen: &mut Pen, rows: &[SheetRow], view: Rect, scroll: f32, s: f32) {
    let pad = 16.0 * s;
    let (x0, x1) = (view.x + pad, view.right() - pad);
    let w = x1 - x0;
    let mut y = view.y + 6.0 * s - scroll;
    let is_stop = |r: Option<&SheetRow>| matches!(r, Some(SheetRow::Stop { .. }));
    for (i, row) in rows.iter().enumerate() {
        let h = sheet_row_height(row) * s;
        let r = Rect::new(x0, y, w, h);
        y += h;
        if r.bottom() < view.y || r.y > view.bottom() {
            continue;
        }
        match row {
            SheetRow::Trip { index: _, departure, arrival, line, terminus, stops, state } => {
                let now = *state == TripState::Now;
                if now {
                    let b = Rect::new(view.x + 8.0 * s, r.y + 3.0 * s, view.w - 16.0 * s, r.h - 6.0 * s);
                    pen.p.rounded(b, 8.0 * s, accent().alpha(0.18));
                    pen.p.rounded(Rect::new(b.x, b.y + 6.0 * s, 3.0 * s, b.h - 12.0 * s), 1.5 * s, accent());
                }
                let (ink, soft) = if *state == TripState::Done { (TEXT_FAINT, TEXT_FAINT) } else { (TEXT, TEXT_DIM) };
                let cy = r.y + 15.0 * s;
                let tw = 44.0 * s;
                pen.text_in(&hhmm(*departure), 13.5 * s, Weight::Bold, Rect::new(x0, cy - 9.0 * s, tw, 18.0 * s), Align::Left, ink);
                let at = hhmm(*arrival);
                let aw = pen.width(&at, 13.0 * s, Weight::Medium) + 2.0 * s;
                pen.text_in(&at, 13.0 * s, Weight::Medium, Rect::new(x1 - aw, cy - 9.0 * s, aw, 18.0 * s), Align::Right, soft);
                let px = x0 + tw + 4.0 * s;
                let plw = if *state == TripState::Done {
                    // (a trip driven: its line as text, the plates are for what is to come)
                    pen.text_in(line.as_deref().unwrap_or(""), 12.5 * s, Weight::Bold, Rect::new(px, cy - 9.0 * s, 60.0 * s, 18.0 * s), Align::Left, TEXT_FAINT)
                } else {
                    pen.plate(line.as_deref(), px, cy, s * 0.92)
                };
                let nx = px + plw + 8.0 * s;
                pen.text_in(terminus, 13.5 * s, Weight::Bold, Rect::new(nx, cy - 9.0 * s, (x1 - aw - 8.0 * s - nx).max(0.0), 18.0 * s), Align::Left, ink);
                let sub = match line {
                    Some(l) => tr_with("line %{line}", &[("line", l.clone())]),
                    None => omsi_ui::tr("Empty run").into_owned(),
                };
                let count = if *stops == 1 { omsi_ui::tr("1 stop").into_owned() } else { tr_with("%{n} stops", &[("n", stops.to_string())]) };
                pen.text_in(&format!("{sub}  ·  {count}"), 11.0 * s, Weight::Medium, Rect::new(nx, r.y + 26.0 * s, (x1 - nx).max(0.0), 14.0 * s), Align::Left, if *state == TripState::Done { TEXT_FAINT } else { TEXT_DIM });
            }
            SheetRow::Stop { planned, name, state, last } => {
                // (as Omsi-Hub's live duty: the pin on the rail, the time, the name)
                let (weight, c) = time_style(*state);
                pen.text_in(&hhmm(*planned), 12.0 * s, weight, Rect::new(x0 + 22.0 * s, r.y, 44.0 * s, r.h), Align::Left, c);
                pen.stop(r, x0 + 8.0 * s, x0 + 66.0 * s, name, None, None, *state, *last, is_stop(i.checked_sub(1).and_then(|j| rows.get(j))), is_stop(rows.get(i + 1)), s);
            }
            SheetRow::Pause { minutes } => {
                pen.p.icon(pen.atlas, "pause", Vec2::new(x0 + 54.0 * s, r.center().y), 13.0 * s, TEXT_DIM);
                let t = tr_with("%{m} min break", &[("m", minutes.to_string())]);
                pen.text_in(&t, 11.5 * s, Weight::Medium, Rect::new(x0 + 66.0 * s, r.y, w - 66.0 * s, r.h), Align::Left, TEXT_DIM);
            }
            SheetRow::Change { line, tour } => {
                pen.p.icon(pen.atlas, "sync_alt", Vec2::new(x0 + 54.0 * s, r.center().y), 13.0 * s, NOW);
                let t = tr_with("Continue on line %{line}, tour %{tour}", &[("line", line.clone()), ("tour", tour.clone())]);
                pen.text_in(&t, 11.5 * s, Weight::Bold, Rect::new(x0 + 66.0 * s, r.y, w - 66.0 * s, r.h), Align::Left, NOW);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::schedule::{PlannedStop, StopDir};

    /// A trip leaving at `dep` (minutes of the day) through `stops` (name, minutes after the
    /// departure, whether the bus stops there).
    pub(crate) fn trip(line: &str, terminus: &str, dep: f64, stops: &[(&str, f64, bool)]) -> PlannedTrip {
        let stops: Vec<PlannedStop> = stops.iter().enumerate().map(|(k, (name, at, serves))| PlannedStop { object_id: k as i64, name: name.to_string(), arr: (dep + at) * 60.0, dep: (dep + at) * 60.0, position: None, dir: StopDir::default(), stops: *serves }).collect();
        let end = stops.last().map(|s| s.arr).unwrap_or(dep * 60.0);
        PlannedTrip { name: format!("{line}-{dep}"), line: line.into(), terminus: terminus.into(), departure: dep * 60.0, end, stops }
    }

    /// A composed duty: line 35 tour 1 out and back, then line 25 tour 3 (the second part),
    /// the first trip leaving the depot past a station it does not stop at.
    pub(crate) fn duty() -> (Vec<PlannedTrip>, Vec<(String, String, usize)>) {
        let a = trip("35", "Diakonissenkrankenhaus", 11.0 * 60.0 + 26.0, &[("Hauptbahnhof", 0.0, true), ("Depot gate", 1.0, false), ("Markt", 3.0, true), ("Kirchweg", 6.0, true), ("Rathaus", 9.0, true), ("Schule", 12.0, true), ("Park", 15.0, true), ("Diakonissenkrankenhaus", 20.0, true)]);
        let b = trip("35", "Hauptbahnhof", 12.0 * 60.0 + 1.0, &[("Diakonissenkrankenhaus", 0.0, true), ("Park", 5.0, true), ("Hauptbahnhof", 18.0, true)]);
        let c = trip("25", "Eichstedt", 12.0 * 60.0 + 33.0, &[("Hauptbahnhof", 0.0, true), ("Eichstedt", 28.0, true)]);
        let tours = vec![("35".into(), "1".into(), 0), ("35".into(), "1".into(), 1), ("25".into(), "3".into(), 4)];
        (vec![a, b, c], tours)
    }

    pub(crate) fn state<'a>(trips: &'a [PlannedTrip], tours: &'a [(String, String, usize)], trip: usize, next_stop: usize, done: bool, delay: f64) -> DutyState<'a> {
        DutyState { trips, tours, line: "35", tour: "1", trip, next_stop, at_stop: false, done, delay }
    }

    fn names(rows: &[Row]) -> Vec<(String, StopState)> {
        rows.iter().filter_map(|r| match r { Row::Stop { name, state, .. } => Some((name.clone(), *state)), _ => None }).collect()
    }

    #[test]
    fn punctuality_is_omsis() {
        assert_eq!(punctuality(180.0), Punctuality::OnTime);
        assert_eq!(punctuality(181.0), Punctuality::Late);
        assert_eq!(punctuality(-120.0), Punctuality::OnTime);
        assert_eq!(punctuality(-121.0), Punctuality::Early);
        assert_eq!(offset(192.0), "+3:12");
        assert_eq!(offset(-150.0), "\u{2212}2:30");
        assert_eq!(hhmm(11.0 * 3600.0 + 26.0 * 60.0 + 59.0), "11:26");
        // (a duty across midnight)
        assert_eq!(hhmm(24.0 * 3600.0 + 5.0 * 60.0), "00:05");
        assert_eq!(hhmm(-60.0), "23:59");
    }

    /// Under way: the trip with its plate and terminus, the stop behind the bus, the one it
    /// heads for and the ones after it, how many more, and the trip after this one.
    #[test]
    fn the_board_shows_the_trip_under_way() {
        let (trips, tours) = duty();
        // due at Kirchweg (index 3; the station at 1 is passed, not shown)
        let d = state(&trips, &tours, 0, 3, false, 40.0);
        let rows = board(Some(&d), 3);
        match &rows[0] {
            Row::Head { line, terminus, index, count, time, status } => {
                assert_eq!(line.as_deref(), Some("35"));
                assert_eq!(terminus, "Diakonissenkrankenhaus");
                assert_eq!((*index, *count), (1, 3));
                assert_eq!(hhmm(*time), "11:46");
                assert_eq!(*status, Status::Running(40.0));
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(names(&rows), [("Markt".to_string(), StopState::Done), ("Kirchweg".into(), StopState::Now), ("Rathaus".into(), StopState::Ahead), ("Schule".into(), StopState::Ahead)]);
        assert!(rows.contains(&Row::More { count: 2, terminus: "Diakonissenkrankenhaus".into() }));
        // on time: no second time beside the planned one
        assert!(rows.iter().all(|r| !matches!(r, Row::Stop { expected: Some(_), .. })));
        // the next trip of the same tour: its break, no change of line
        assert_eq!(rows.last(), Some(&Row::Next { departure: (12.0 * 60.0 + 1.0) * 60.0, line: Some("35".into()), terminus: "Hauptbahnhof".into(), pause: 15, change: None }));
        assert_eq!(Status::Running(40.0).label(), "on time");
    }

    /// Late: the stops ahead say when the bus will be there, the one behind it does not.
    #[test]
    fn a_late_bus_gets_its_expected_times() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 0, 3, false, 250.0);
        let rows = board(Some(&d), 2);
        let expected: Vec<Option<String>> = rows.iter().filter_map(|r| match r { Row::Stop { expected, .. } => Some(expected.map(hhmm)), _ => None }).collect();
        assert_eq!(expected, [None, Some("11:36".to_string()), Some("11:39".to_string())]);
        assert_eq!(Status::Running(250.0).label(), "+4:10");
        assert_eq!(Status::Running(250.0).colours().0, LATE);
        // standing at the stop it is due at: it is there now
        let d = DutyState { at_stop: true, ..d };
        assert!(matches!(board(Some(&d), 2)[2], Row::Stop { state: StopState::Now, expected: None, .. }));
    }

    /// At the terminus between two trips the board looks ahead: the next trip, the break
    /// left, and the line and tour the duty goes on with.
    #[test]
    fn during_the_break_the_board_shows_the_next_trip() {
        let (trips, tours) = duty();
        // trip 2 done; the delay is against trip 3's departure (12:33), eight minutes away
        let d = state(&trips, &tours, 1, 2, true, -480.0);
        assert_eq!(d.status(), Status::Break(480.0));
        assert_eq!(d.status().label(), "break, 8 min left");
        let rows = board(Some(&d), 4);
        assert!(matches!(&rows[0], Row::Head { line: Some(l), terminus, index: 3, .. } if l == "25" && terminus == "Eichstedt"));
        assert_eq!(rows[1], Row::Change { line: "25".into(), tour: "3".into() });
        assert_eq!(names(&rows), [("Hauptbahnhof".to_string(), StopState::Now), ("Eichstedt".into(), StopState::Ahead)]);
        assert!(rows.iter().all(|r| !matches!(r, Row::Next { .. } | Row::More { .. })));
        assert!(matches!(rows.last(), Some(Row::Stop { last: true, .. })));
        // the break overrun
        assert_eq!(Status::Break(-61.0).label(), "2 min over");
        assert_eq!(Status::Break(-61.0).colours().0, LATE);
    }

    /// Before the change, the trip before it says where the duty goes on.
    #[test]
    fn the_change_of_line_is_announced_on_the_trip_before() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 1, 1, false, 0.0);
        let rows = board(Some(&d), 4);
        assert_eq!(rows.last(), Some(&Row::Next { departure: (12.0 * 60.0 + 33.0) * 60.0, line: Some("25".into()), terminus: "Eichstedt".into(), pause: 14, change: Some(("25".into(), "3".into())) }));
        // a duty of one tour (no legs): the same line and tour throughout, never a change
        let one = DutyState { tours: &[], ..d };
        assert!(matches!(board(Some(&one), 4).last(), Some(Row::Next { change: None, .. })));
    }

    #[test]
    fn before_the_first_stop_the_trip_leaves_in_so_long() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 0, 0, false, -299.0);
        assert_eq!(d.status(), Status::DepartsIn(299.0));
        assert_eq!(d.status().label(), "leaves in 5 min");
        let rows = board(Some(&d), 4);
        // no stop behind the bus yet; the first served stop is the one it heads for
        assert_eq!(names(&rows)[0], ("Hauptbahnhof".to_string(), StopState::Now));
        // late for the departure: that is late
        assert_eq!(state(&trips, &tours, 0, 0, false, 200.0).status(), Status::Running(200.0));
    }

    #[test]
    fn the_last_trip_driven_finishes_the_duty() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 2, 1, true, 30.0);
        assert_eq!(d.status(), Status::Finished);
        let rows = board(Some(&d), 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], Row::Finished);
        assert_eq!(board(None, 4), [Row::NoDuty]);
    }

    /// An empty run (a trip to or from the depot) has no line of its own.
    #[test]
    fn an_empty_run_has_no_plate() {
        let (mut trips, tours) = duty();
        trips[0].line = " ".into();
        let d = state(&trips, &tours, 0, 3, false, 0.0);
        assert!(matches!(&board(Some(&d), 2)[0], Row::Head { line: None, .. }));
    }

    /// The sheet: every trip, the breaks and the change of line between them, and the trip
    /// under way opened up with its served stops; the list keeps the stop it heads for in
    /// view.
    #[test]
    fn the_sheet_lists_the_whole_duty() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 1, 1, false, 0.0);
        let rows = sheet(&d);
        let kinds: Vec<String> = rows
            .iter()
            .map(|r| match r {
                SheetRow::Trip { index, state, .. } => format!("trip {index} {state:?}"),
                SheetRow::Stop { name, state, .. } => format!("{name} {state:?}"),
                SheetRow::Pause { minutes } => format!("break {minutes}"),
                SheetRow::Change { line, tour } => format!("change {line}/{tour}"),
            })
            .collect();
        assert_eq!(kinds, ["trip 1 Done", "break 15", "trip 2 Now", "Diakonissenkrankenhaus Done", "Park Now", "Hauptbahnhof Ahead", "break 14", "change 25/3", "trip 3 Ahead"]);
        assert!(matches!(&rows[0], SheetRow::Trip { stops: 7, .. }));
        assert_eq!(sheet_focus(&rows), 44.0 + 24.0 + 44.0 + 23.0);
        assert_eq!(sheet_height(&rows), 3.0 * 44.0 + 3.0 * 24.0 + 3.0 * 23.0);
        // finished: everything behind the bus
        let done = state(&trips, &tours, 2, 1, true, 0.0);
        assert!(sheet(&done).iter().all(|r| !matches!(r, SheetRow::Stop { state: StopState::Now | StopState::Ahead, .. })));
    }

    /// A window too low for the whole board gets fewer stops ahead, never fewer than two.
    #[test]
    fn the_board_fits_the_window() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 0, 3, false, 0.0);
        let (rows, h) = board_fitting(Some(&d), 1.0, 1000.0);
        assert_eq!(names(&rows).len(), 5);
        let (fewer, h2) = board_fitting(Some(&d), 1.0, h - 1.0);
        assert_eq!(names(&fewer).len(), 4);
        assert!(h2 < h);
        let (least, _) = board_fitting(Some(&d), 1.0, 0.0);
        assert_eq!(names(&least).len(), 3);
        assert_eq!(board_fitting(None, 1.0, 0.0).0, [Row::NoDuty]);
    }

    /// A navigator the player sized keeps its board inside its room: fewer stops ahead, then
    /// the rows that matter least, and no board where not even the trip's head fits.
    #[test]
    fn the_board_keeps_within_a_sized_navigator() {
        let (trips, tours) = duty();
        let d = state(&trips, &tours, 0, 3, false, 0.0);
        let (all, h) = board_within(Some(&d), 1.0, 1000.0);
        assert_eq!(all, board(Some(&d), 4));
        for room in [h - 1.0, 180.0, 140.0, 100.0, 70.0, 60.0] {
            let (rows, rh) = board_within(Some(&d), 1.0, room);
            assert!(rh <= room && rh == (board_height(&rows) * 1.0).round(), "{room}: {rh}");
            assert!(matches!(rows.first(), Some(Row::Head { .. })), "{room}: the trip's head first");
            assert!(rows.iter().any(|r| matches!(r, Row::Stop { state: StopState::Now, .. })) || room < 90.0, "{room}: the next stop stays while it fits");
        }
        assert_eq!(board_within(Some(&d), 1.0, 59.0), (Vec::new(), 0.0), "not even the trip's head: no board");
        // (at a larger scale it needs room in proportion)
        let (big, bh) = board_within(Some(&d), 2.0, 2.0 * 140.0);
        assert!(bh <= 280.0 && big.len() == board_within(Some(&d), 1.0, 140.0).0.len());
    }

    /// The board draws inside the room its rows ask for, at any interface size.
    #[test]
    fn the_board_draws_within_its_height() {
        let (trips, tours) = duty();
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(1024);
        for (d, ahead) in [(state(&trips, &tours, 0, 3, false, 250.0), 4), (state(&trips, &tours, 1, 2, true, -480.0), 4), (state(&trips, &tours, 2, 1, true, 0.0), 2)] {
            let rows = board(Some(&d), ahead);
            for s in [0.85, 1.0, 1.6] {
                let mut p = Painter::new();
                let area = Rect::new(0.0, 300.0, 360.0 * s, board_height(&rows) * s);
                draw_board(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, &rows, area, s);
                assert!(!p.verts.is_empty());
                for v in &p.verts {
                    let (x, y) = (v.pos[0] + v.ext[0] * v.width[0], v.pos[1] + v.ext[1] * v.width[0]);
                    assert!(x >= area.x - 0.5 && x <= area.right() + 0.5 && y >= area.y - 0.5 && y <= area.bottom() + 0.5, "({x}, {y}) outside {area:?} at {s}");
                }
            }
        }
    }

    /// Every text of the panel is in the tables for the languages that matter most.
    #[test]
    fn the_panel_is_translated() {
        let keys = ["No duty running", "Empty run", "trip %{k} of %{n}", "arrives %{time}", "leaves %{time}", "leaves in %{m} min", "break, %{m} min left", "%{m} min over", "Duty finished", "and %{n} more to %{terminus}", "terminus", "Next %{time}", "%{m} min break", "Continue on line %{line}, tour %{tour}", "In the game", "Delay", "line %{line}", "1 stop", "%{n} stops", "Duty", "Trip", "on time", "%{n} trip", "%{n} trips"];
        for language in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for key in keys {
                let t = crate::_rust_i18n_try_translate(language, key);
                assert!(t.as_ref().is_some_and(|t| !t.trim().is_empty()), "{language}: {key}");
                // (the placeholders survive the translation)
                for var in ["%{k}", "%{n}", "%{m}", "%{time}", "%{terminus}", "%{line}", "%{tour}"] {
                    assert_eq!(key.contains(var), t.as_ref().unwrap().contains(var), "{language}: {key}");
                }
            }
        }
        assert_eq!(crate::_rust_i18n_try_translate("nl", "Continue on line %{line}, tour %{tour}").as_deref(), Some("Verder met lijn %{line}, omloop %{tour}"));
    }

    /// The painter's triangles (shapes and atlas sprites) drawn on the CPU over `img`, those
    /// outside `clip` left out - a picture of the panel without a GPU or a game.
    pub(crate) fn raster(verts: &[omsi_ui::Vertex], atlas: &Atlas, img: &mut image::RgbaImage, clip: Rect) {
        let edge = |a: Vec2, b: Vec2, c: Vec2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        for t in verts.chunks_exact(3) {
            let p: Vec<Vec2> = t.iter().map(|v| Vec2::new(v.pos[0] + v.ext[0] * v.width[0], v.pos[1] + v.ext[1] * v.width[0])).collect();
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 1e-6 {
                continue;
            }
            let lo = p[0].min(p[1]).min(p[2]).max(Vec2::new(clip.x, clip.y)).max(Vec2::ZERO);
            let hi = p[0].max(p[1]).max(p[2]).min(Vec2::new(clip.right(), clip.bottom())).min(Vec2::new(img.width() as f32, img.height() as f32));
            for y in lo.y.floor() as u32..hi.y.ceil().max(0.0) as u32 {
                for x in lo.x.floor() as u32..hi.x.ceil().max(0.0) as u32 {
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let (w0, w1) = (edge(p[1], p[2], q) / area, edge(p[2], p[0], q) / area);
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let mix = |k: usize| t[0].color[k] * w0 + t[1].color[k] * w1 + t[2].color[k] * w2;
                    let mut a = mix(3);
                    if t[0].mode[1] > 0.5 {
                        let u = t[0].uv[0] * w0 + t[1].uv[0] * w1 + t[2].uv[0] * w2;
                        let v = t[0].uv[1] * w0 + t[1].uv[1] * w1 + t[2].uv[1] * w2;
                        let n = atlas.size;
                        let (tx, ty) = (((u * n as f32) as u32).min(n - 1), ((v * n as f32) as u32).min(n - 1));
                        a *= atlas.rgba[((ty * n + tx) * 4 + 3) as usize] as f32 / 255.0;
                    }
                    let px = img.get_pixel_mut(x, y);
                    for k in 0..3 {
                        px.0[k] = (mix(k) * 255.0 * a + px.0[k] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }

    /// Pictures of the board (late, at the break before a change of line, before the first
    /// stop) under a stand-in for the small map, and of the sheet, in Dutch:
    /// `OMSI_NAV_DUTY_PREVIEW=<folder> cargo test -p omsi-app --lib nav_duty -- --ignored`.
    #[test]
    #[ignore]
    fn preview_pictures() {
        let Ok(dir) = std::env::var("OMSI_NAV_DUTY_PREVIEW") else { return };
        crate::ui_language("NLD");
        let (trips, tours) = duty();
        let fonts = Fonts::hanken();
        let mut atlas = Atlas::new(2048);
        let s = 2.0;
        let states = [state(&trips, &tours, 0, 3, false, 250.0), state(&trips, &tours, 1, 2, true, -480.0), DutyState { at_stop: true, ..state(&trips, &tours, 0, 0, false, -299.0) }];
        let (pw, map_h, bars) = (360.0 * s, 223.0 * s, 80.0 * s);
        let boards: Vec<Vec<Row>> = states.iter().map(|d| board(Some(d), 4)).collect();
        let tall = boards.iter().map(|b| board_height(b) * s).fold(0.0, f32::max) + map_h + bars;
        let mut img = image::RgbaImage::from_pixel(((pw + 40.0) * 3.0) as u32, (tall + 40.0) as u32, image::Rgba([70, 84, 96, 255]));
        for (k, rows) in boards.iter().enumerate() {
            let x = 20.0 + k as f32 * (pw + 40.0);
            let mut p = Painter::new();
            let h = map_h + bars + board_height(rows) * s;
            p.rounded(Rect::new(x, 20.0, pw, h), 7.0 * s, Color::rgba(20, 26, 38, 0.92));
            p.rect(Rect::new(x, 20.0, pw, 34.0 * s), Color::rgba(6, 9, 16, 0.8));
            p.rect(Rect::new(x, 20.0 + 34.0 * s, pw, map_h), Color::rgba(40, 48, 60, 1.0));
            p.rect(Rect::new(x, 20.0 + 34.0 * s + map_h, pw, 46.0 * s), Color::rgba(6, 9, 16, 0.8));
            p.text_in(&mut atlas, &fonts, "Kirchweg", 13.5 * s, Weight::Bold, Rect::new(x + 11.0 * s, 20.0 + 38.0 * s + map_h, pw, 20.0 * s), Align::Left, TEXT);
            let area = Rect::new(x, 20.0 + map_h + bars, pw, board_height(rows) * s);
            draw_board(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, rows, area, s);
            raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e5, 1e5));
        }
        img.save(format!("{dir}/board.png")).unwrap();
        // the sheet, on the city map's ground
        let (sw, sh) = (340.0 * s, 760.0 * s);
        let mut img = image::RgbaImage::from_pixel((sw + 40.0) as u32 * 2, (sh + 40.0) as u32, image::Rgba([12, 16, 26, 255]));
        for (k, d) in [state(&trips, &tours, 1, 1, false, 250.0), state(&trips, &tours, 1, 2, true, -480.0)].iter().enumerate() {
            let r = Rect::new(20.0 + k as f32 * (sw + 40.0), 20.0, sw, sh);
            let mut p = Painter::new();
            let view = draw_sheet_head(&mut Pen { p: &mut p, atlas: &mut atlas, fonts: &fonts }, d, r, 12.0 * 3600.0 + 4.0 * 60.0, s);
            raster(&p.verts, &atlas, &mut img, Rect::new(0.0, 0.0, 1e5, 1e5));
            let rows = sheet(d);
            let mut list = Painter::new();
            draw_sheet_list(&mut Pen { p: &mut list, atlas: &mut atlas, fonts: &fonts }, &rows, view, 0.0, s);
            raster(&list.verts, &atlas, &mut img, view);
        }
        img.save(format!("{dir}/sheet.png")).unwrap();
    }
}
