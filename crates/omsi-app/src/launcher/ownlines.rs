//! The player's own lines among the map's (Luc: "in het menu moet een optie komen om zelf
//! gemaakte lijnen te selecteren"). The line editor writes a line into the map's timetable as
//! an ordinary `.ttl` (`core::lines`), so the launcher's line lists had it among the map's
//! own, under its file name. Wherever a line is chosen now - the duty step, the free drive,
//! the phone and the classic Drive page - a switch above the list picks "Map lines" or "My
//! lines", and the player's are shown by their plate in the line's colour, their name and
//! where they go - and, a line of another kind than regular, its kind of service (school
//! transport, weekend trips, on demand). The choice is kept with the duty (`Choice::my_lines`).

use super::theme::*;
use super::ui::{id_of, Ui};
use glam::Vec2;
use omsi_launcher_lib::lines::{own_line_of, OwnLine};
use omsi_launcher_lib::service::ServiceKind;
use omsi_launcher_lib::LineInfo;
use omsi_ui::paint::Align;
use omsi_ui::{Color, Rect, Weight};

/// What a list says where "My lines" cannot be chosen yet.
pub(super) const NONE_YET: &str = "Make one in Editor → Line editor";

/// What a dropdown option with a plate in front begins with, and what parts it (see
/// `plate_option`): a character no line or stop name has.
const MARK: char = '\u{2}';

/// `#rrggbb` as a colour (the editor's blue when it is none).
pub(super) fn colour_of(hex: &str) -> Color {
    let h = hex.trim().trim_start_matches('#');
    let v = (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten().unwrap_or(0x2a75f7);
    Color::rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 1.0)
}

/// The ink a plate of colour `c` is written in: dark on a light one (a yellow line), white
/// on the others.
pub(super) fn ink_on(c: Color) -> Color {
    let [r, g, b, _] = c.0;
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.6 {
        ON_LINE
    } else {
        Color::WHITE
    }
}

/// A player's line's plate: its number on the line's colour, `h` high at `at`. Returns its
/// width.
pub(super) fn plate(ui: &mut Ui, at: Vec2, number: &str, colour: &str, h: f32) -> f32 {
    let px = h * 0.6;
    let c = colour_of(colour);
    let w = (ui.width(number, px, Weight::Black) + h * 0.7).max(h * 1.8);
    let r = Rect::new(at.x, at.y, w, h);
    ui.p().rounded(r, (h * 0.25).min(5.0), c);
    ui.text_in(number, r, px, Weight::Black, ink_on(c), Align::Center);
    w
}

/// The small "own" mark beside a line of the player's where the map's are listed with it.
/// Returns its width.
pub(super) fn badge(ui: &mut Ui, at: Vec2) -> f32 {
    ui.badge(at, &omsi_ui::tr("own").to_uppercase(), accent_2())
}

/// A dropdown option that `Ui::select` draws with the line's plate in front of `rest`.
pub(super) fn plate_option(o: &OwnLine, rest: &str) -> String {
    format!("{MARK}{}{MARK}{}{MARK}{rest}", o.colour.trim(), o.number.trim())
}

/// The colour, the number and the rest of a `plate_option` (None: a plain option).
pub(super) fn parse_plate(s: &str) -> Option<(&str, &str, &str)> {
    let mut parts = s.strip_prefix(MARK)?.splitn(3, MARK);
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The lines of `lines` as the map's and the player's (in the order they came).
pub(super) fn split<'a>(lines: impl IntoIterator<Item = &'a LineInfo>, own: &[OwnLine]) -> (Vec<&'a LineInfo>, Vec<&'a LineInfo>) {
    lines.into_iter().partition(|l| own_line_of(&l.name, own).is_none())
}

/// The list shows the player's lines: they were chosen and there are some (else the map's -
/// the choice is kept for when there are).
pub(super) fn showing_mine(chosen: bool, mine: usize) -> bool {
    chosen && mine > 0
}

/// A line of the timetable as a dropdown option: a player's by its plate, its name and where
/// it goes; one of the map's by its file name and its termini, as before.
pub(super) fn option_label(line: &LineInfo, own: &[OwnLine]) -> String {
    let termini = line.termini.join(" · ");
    match own_line_of(&line.name, own) {
        Some(o) => {
            let caption = o.caption();
            plate_option(&o, &with_kind(&o, if caption.is_empty() { termini } else { caption }))
        }
        None if termini.is_empty() => line.name.clone(),
        None => format!("{}  ·  {termini}", line.name),
    }
}

/// The name a summary gives the timetable line `name`: a player's line's number, else the
/// file name it has.
pub(super) fn shown_name(name: &str, own: &[OwnLine]) -> String {
    own_line_of(name, own).map(|o| o.number).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| name.to_string())
}

/// What a row of a player's line says under its plate: its name and where it goes, else the
/// timetable's termini.
pub(super) fn caption_of(o: &OwnLine, line: &LineInfo) -> String {
    let c = o.caption();
    with_kind(o, if c.is_empty() { line.termini.join(" · ") } else { c })
}

/// A player's line's words with its kind of service after them ("Ring · Markt – Bahnhof ·
/// School transport"); a regular line's as they are.
pub(super) fn with_kind(o: &OwnLine, text: String) -> String {
    if o.service == ServiceKind::Regular {
        return text;
    }
    let kind = omsi_ui::tr(o.service.label());
    if text.is_empty() {
        kind.into_owned()
    } else {
        format!("{text} · {kind}")
    }
}

/// Where a player's line goes ("Markt – Bahnhof"), for a row that shows its name already:
/// else the timetable's termini.
pub(super) fn destinations_of(o: &OwnLine, line: &LineInfo) -> String {
    if o.destinations.is_empty() {
        line.termini.join(" – ")
    } else {
        o.destinations.join(" – ")
    }
}

/// What the switch answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Switched {
    No,
    /// The list was switched: to the player's lines (true) or the map's.
    To(bool),
    /// "My lines" was clicked while there are none: say where they are made.
    Hint,
}

/// The switch above a line list: "Map lines (12)" and "My lines (2)". `mine` is what was
/// chosen; "My lines" cannot be chosen while there are none (its tooltip says where they are
/// made, and a click asks the caller to say so).
pub(super) fn switch(ui: &mut Ui, name: &str, r: Rect, mine: bool, counts: (usize, usize)) -> Switched {
    let labels = [omsi_ui::tr("Map lines (%{n})").replace("%{n}", &counts.0.to_string()), omsi_ui::tr("My lines (%{n})").replace("%{n}", &counts.1.to_string())];
    let refs = [labels[0].as_str(), labels[1].as_str()];
    let none = counts.1 == 0;
    let mut sel = usize::from(showing_mine(mine, counts.1));
    let off: &[usize] = if none { &[1] } else { &[] };
    let changed = ui.segmented_some(name, r, &mut sel, &refs, off);
    let mine_cell = Rect::new(r.x + r.w * 0.5, r.y, r.w * 0.5, r.h);
    if none {
        ui.tooltip(mine_cell, NONE_YET);
        let (_, _, clicked) = ui.interact(id_of(&format!("{name}-none")), mine_cell);
        if clicked {
            return Switched::Hint;
        }
    } else {
        ui.tooltip(mine_cell, "The lines you made in the line editor");
    }
    if changed {
        Switched::To(sel == 1)
    } else {
        Switched::No
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(name: &str, termini: &[&str]) -> LineInfo {
        LineInfo { name: name.into(), user_allowed: true, termini: termini.iter().map(|t| t.to_string()).collect(), tours: Vec::new() }
    }

    fn own() -> Vec<OwnLine> {
        vec![OwnLine { id: 3, file: "oo_42".into(), number: "42".into(), name: "Ring".into(), colour: "#e03c31".into(), destinations: vec!["Markt".into(), "Bahnhof".into()], ..Default::default() }]
    }

    #[test]
    fn the_players_lines_are_told_from_the_maps() {
        let lines = vec![line("Montag - Freitag", &["Oberhof"]), line("oo_42", &["Markt"]), line("Samstag", &[]), line("oo_7", &["Kirche"])];
        let (map, mine) = split(&lines, &own());
        assert_eq!(map.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["Montag - Freitag", "Samstag"]);
        // (one of the editor's whose registry is gone is the player's all the same)
        assert_eq!(mine.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["oo_42", "oo_7"]);
        // "My lines" is shown only while there are some
        assert!(showing_mine(true, 2));
        assert!(!showing_mine(true, 0));
        assert!(!showing_mine(false, 2));
    }

    #[test]
    fn an_option_of_the_players_carries_its_plate() {
        let own = own();
        let o = option_label(&line("oo_42", &["Markt"]), &own);
        assert_eq!(parse_plate(&o), Some(("#e03c31", "42", "Ring · Markt – Bahnhof")));
        // without a registry: the number from the file, the timetable's termini
        assert_eq!(parse_plate(&option_label(&line("oo_7", &["Kirche", "Markt"]), &own)), Some(("#2a75f7", "7", "Kirche · Markt")));
        // the map's lines as before
        let m = option_label(&line("Samstag", &["Oberhof", "Wangenbach"]), &own);
        assert_eq!(m, "Samstag  ·  Oberhof · Wangenbach");
        assert_eq!(parse_plate(&m), None);
        assert_eq!(option_label(&line("Samstag", &[]), &own), "Samstag");
        assert_eq!(caption_of(&OwnLine { number: "9".into(), ..Default::default() }, &line("oo_9", &["Ende"])), "Ende");
        assert_eq!(destinations_of(&own[0], &line("oo_42", &["X"])), "Markt – Bahnhof");
        assert_eq!(destinations_of(&OwnLine::default(), &line("oo_9", &["A", "B"])), "A – B");
        // a line of another kind says so
        let school = OwnLine { service: ServiceKind::School, ..own[0].clone() };
        assert_eq!(caption_of(&school, &line("oo_42", &[])), "Ring · Markt – Bahnhof · School transport");
        // a summary names the player's line by its number
        assert_eq!(shown_name("oo_42", &own), "42");
        assert_eq!(shown_name("oo_7", &own), "7");
        assert_eq!(shown_name("Samstag", &own), "Samstag");
    }

    #[test]
    fn a_plate_is_written_so_it_can_be_read() {
        assert_eq!(colour_of("#e03c31").0, Color::rgba(0xe0, 0x3c, 0x31, 1.0).0);
        assert_eq!(colour_of("nonsense").0, colour_of("#2a75f7").0);
        // dark ink on yellow and white, white on red, blue and black
        assert_eq!(ink_on(LINE).0, ON_LINE.0);
        assert_eq!(ink_on(Color::WHITE).0, ON_LINE.0);
        for c in ["#e03c31", "#2a75f7", "#000000"] {
            assert_eq!(ink_on(colour_of(c)).0, Color::WHITE.0);
        }
    }
}
