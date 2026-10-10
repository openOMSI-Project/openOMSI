//! Destinations, roller blinds and the IBIS codes of a trip, for the AI buses and the player's.

use super::*;

/// Put an AI bus's IBIS onto the line/terminus of its trip: the depot file gives the
/// terminus code (by ident) and the info trip (route index) for the line - the one its
/// stops (`stops`, the trip's station names) follow, [`pick_route`] - and the IBIS
/// variables the bus scripts render are set as if the driver had typed them.
pub fn set_ai_destination(
    v: &mut crate::VehicleInstance,
    hof: Option<&omsi_vehicle::Hof>,
    line: &str,
    terminus: &str,
    stops: &[&str],
) {
    set_destination(v, hof, line, terminus, stops, false)
}

/// The same with terminus number `ti` of the depot file itself (its `AI_target_index`).
pub fn set_ai_destination_at(
    v: &mut crate::VehicleInstance,
    hof: &omsi_vehicle::Hof,
    line: &str,
    ti: usize,
    stops: &[&str],
) {
    set_destination_at(v, hof, line, ti, stops, false)
}

/// The same for the player's bus, done the driver's way: a typing job
/// (`crate::ibis::Typist`) that works the bus's own IBIS keys - or its ticket machine's
/// - as a driver would, so that the IBIS script itself sets the displays, the stop list,
/// the announcements and the ticket printer. `stop` is the stop of the trip the bus is at.
/// None when the depot file has no such destination. The electrics must be on; when the
/// typing fails the IBIS variables are written directly ([`set_player_destination_directly`]).
/// The `ai_scheduled_settarget` trigger is only used for a hand-cranked roller blind, which
/// no IBIS drives, and only with the main switch already on: fired before the start-up it
/// switched the main switch on, and the start-up's toggle then switched the NL202's
/// electrics off again.
#[allow(clippy::too_many_arguments)]
pub fn player_ibis(
    v: &mut crate::VehicleInstance,
    hof: Option<&omsi_vehicle::Hof>,
    line: &str,
    terminus: &str,
    stops: &[&str],
    stop: Option<(usize, &str)>,
    operable: &dyn Fn(&str) -> bool,
    background: bool,
) -> Option<crate::ibis::Typist> {
    let h = hof?;
    let Some(target) = ibis_target(h, line, terminus, stops, stop) else {
        log::info!(
            "IBIS: terminus '{}' is not in depot file {}",
            terminus.trim(),
            h.name
        );
        return None;
    };
    // a roller blind is cranked by hand; the AI trigger turns it to the trip. Known by the
    // blind's own keys, not by its variables: the NL202 declares the roller blind's
    // variables without having one, and the AI trigger then gave its matrix a blank line
    // number that pushed the destination aside.
    if has_roller_blind(v)
        && v.var("elec_busbar_main_sw")
        .map(|x| x > 0.5)
        .unwrap_or(false)
    {
        set_line_to(v, line);
        v.set_var("AI_target_index", target.terminus_index as f32);
        v.trigger("ai_scheduled_settarget");
    }
    log::info!(
        "IBIS: typing line '{}' to '{}' (line {} route {:?} destination {:?}, stop {})",
        line.trim(),
        terminus.trim(),
        target.line,
        target.route,
        target.terminus_code,
        target.stop
    );
    Some(crate::ibis::Typist::new(v, target, operable, background))
}

/// The player's IBIS set without typing: the IBIS variables written as the IBIS script
/// would leave them.
pub fn set_player_destination_directly(
    v: &mut crate::VehicleInstance,
    hof: Option<&omsi_vehicle::Hof>,
    line: &str,
    terminus: &str,
    stops: &[&str],
) {
    set_destination(v, hof, line, terminus, stops, true)
}

/// The same with terminus number `ti` of the depot file, for a destination picked from
/// the list of them: termini often share a name (four "ul. Xutorskaya" of codes 92, 120,
/// 123 and 124, one per route), and looked up by its name the pick always gave the first
/// of them (#738). On a bus with a hand-cranked roller blind, what the blind is to be
/// turned to ([`turn_roller_blind`]).
pub fn set_player_destination_at(
    v: &mut crate::VehicleInstance,
    hof: &omsi_vehicle::Hof,
    line: &str,
    ti: usize,
    stops: &[&str],
) -> Option<BlindPick> {
    set_destination_at(v, hof, line, ti, stops, true);
    (has_roller_blind(v) && ti < hof.termini.len()).then(|| BlindPick { row: ti, line: line.trim().to_string() })
}

/// A destination picked by hand for a hand-cranked roller blind: its row of the depot file
/// and the route number for the number rollers (`SetLineTo`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlindPick {
    pub row: usize,
    pub line: String,
}

/// Turn a hand-cranked roller blind to the destination picked for it (`pick`, taken once
/// done), as Omsi.exe's line and destination dialog sets the driven bus
/// (Tform_setline.Button1Click: TRoadVehicleInst.virtual_10, the AI's way - `SetLineTo`,
/// `AI_target_index`, the `ai_scheduled_settarget` trigger, from which the stock blinds take
/// their place): written into the IBIS alone, a pick never reached an SD77's blind, and the
/// passengers go by the blind. Not before the main switch is on and the start-up
/// (`starting`) is done, as for a duty ([`player_ibis`]): the stock trigger switches the
/// main switch on itself, and the start-up's toggle then switched the electrics off again.
/// A bus picked for cold keeps the pick until it is switched on.
pub fn turn_roller_blind(v: &mut crate::VehicleInstance, pick: &mut Option<BlindPick>, starting: bool) {
    if pick.is_none() || starting || !v.var("elec_busbar_main_sw").is_some_and(|x| x > 0.5) {
        return;
    }
    let Some(p) = pick.take() else { return };
    set_line_to(v, &p.line);
    v.set_var("AI_target_index", p.row as f32);
    v.trigger("ai_scheduled_settarget");
    log::info!("roller blind turned to destination {} on route '{}'", p.row, p.line);
}

/// The row of the depot file a hand-cranked roller blind shows, as its script gives
/// `target_index_int` (the stock rollband.osc's rollband_refreshIntIndex): its plug-in sign's
/// (`rlbnd_steckschild_Termindex`, put up for a code above 1000) where it names a row with a
/// sign text, else the row the blind is turned to (`rlbnd_ziel_target`). None for a bus
/// without one or without the blind's variable.
pub(crate) fn roller_blind_row(v: &crate::VehicleInstance, hof: &omsi_vehicle::Hof) -> Option<usize> {
    if !has_roller_blind(v) {
        return None;
    }
    let row = |name: &str| v.var(name).filter(|i| *i >= 0.0).map(|i| i.round() as usize).filter(|&i| i < hof.termini.len());
    let plugged = row("rlbnd_steckschild_Termindex")
        .filter(|&i| hof.termini[i].strings.first().is_some_and(|s| !s.is_empty()));
    plugged.or_else(|| row("rlbnd_ziel_target"))
}

/// The row of the depot file whose destination the bus shows, which a route number set by
/// hand keeps: a roller blind's - the one picked for it and not turned to yet (`pick`), else
/// the one it shows ([`roller_blind_row`]; cranked by hand, the IBIS knows nothing of it, and
/// a route pick turned the blind back to the IBIS's empty row). A queued electronic
/// selection is kept too, so changing the line does not overwrite it with the old row.
/// Otherwise the IBIS's, by its place
/// in the depot file (by name it was the first of that name), else the first of the IBIS's
/// code, else the first with a name.
pub fn shown_destination(
    v: &crate::VehicleInstance,
    hof: &omsi_vehicle::Hof,
    pick: Option<&BlindPick>,
) -> Option<usize> {
    let code = v.var("IBIS_TerminusCode").unwrap_or(-1.0) as i32;
    let index = v.var("IBIS_TerminusIndex").filter(|i| *i >= 0.0).map(|i| i.round() as usize);
    pick.map(|p| p.row)
        .filter(|&i| i < hof.termini.len())
        .or_else(|| v.pending_destination.as_ref()
            .filter(|p| p.hof.as_ref() == hof)
            .map(|p| p.ti))
        .or_else(|| roller_blind_row(v, hof))
        .or_else(|| index.filter(|&i| hof.termini.get(i).is_some_and(|t| t.code == code)))
        .or_else(|| hof.termini.iter().position(|t| t.code == code))
        .or_else(|| hof.termini.iter().position(|t| t.strings.first().is_some_and(|s| !s.trim().is_empty())))
}

/// What the IBIS shows once a driver has typed a trip's codes, standing at the timetable's
/// stop `stop` (index and name).
pub fn ibis_target(
    hof: &omsi_vehicle::Hof,
    line: &str,
    terminus: &str,
    stops: &[&str],
    stop: Option<(usize, &str)>,
) -> Option<crate::ibis::Target> {
    let (codes, ti) = ibis_codes(hof, line, terminus, stops)?;
    let code = hof.termini[ti].code;
    // the IBIS looks the codes up itself: the first route of the typed code, the first
    // destination of the route's code
    let terminus_index = hof
        .termini
        .iter()
        .position(|t| t.code == code)
        .unwrap_or(ti) as i32;
    let line_number = codes.line.unwrap_or(0) / 100;
    // the last two digits of the depot code are the line's letter suffix ("5E" = line 5,
    // suffix code for E), typed as its own field on the IBIS (`ls = line*100 + suffix`,
    // ibis.rs) - dropped here it left every lettered line's suffix untyped, so a bus that
    // reads it (a destination matrix testing `IBIS_Linie_Suffix` against its own reserved
    // codes, say) found 0 instead and could take the wrong branch.
    let suffix = match codes.line.unwrap_or(0) % 100 {
        0 => line_suffix_from_text(line),
        suffix => suffix,
    };
    let route_index = codes
        .route
        .and_then(|r| {
            hof.info_trips
                .iter()
                .position(|t| omsi_cfg::parse_f32(&t.code) == (line_number * 100 + r) as f32)
        })
        .map(|i| i as i32);
    // the IBIS counts the stops of its own route list, which need not be the timetable's
    // (Grundorf's timetable has two Bauernhof stations, the route one): the stop of that
    // name nearest the timetable's place in the trip - at the trip's first stop as well,
    // for a route that begins before it
    let ibis_stop = match (route_index, stop) {
        (Some(r), Some((k, name))) => ibis_stop_index(hof, r as usize, name, k).unwrap_or(0),
        _ => 0,
    };
    Some(crate::ibis::Target {
        line: line_number,
        suffix,
        route: codes.route,
        terminus_code: codes.terminus,
        route_index,
        terminus_index,
        stop: ibis_stop,
    })
}

/// A hand-cranked roller blind (SD79 or SD83 type), known by its own keys.
pub fn has_roller_blind(v: &crate::VehicleInstance) -> bool {
    ["rollband_sync", "rlbnd_ziel_start"]
        .iter()
        .any(|t| v.ty.program.trigger(t).is_some())
}

/// `SetLineTo` for the AI trigger. A roller blind turns one roller per character -
/// hundreds, tens, units, with the letter suffixes only on the later rollers - so its
/// line is right-aligned to three places ("  5", " 5E"); the matrix scripts take the line
/// as it is.
pub(crate) fn set_line_to(v: &mut crate::VehicleInstance, line: &str) {
    let digits: String = line.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    let text = if has_roller_blind(v) {
        format!("{:>3}", line.trim())
    } else if v.ty.program.str_var("Matrix_Nmr").is_some() && !digits.is_empty() && digits.len() <= 3 {
        // the LiAZ's 4-character matrix shows three digits and a letter (see
        // omsi_script::compat): its AI path takes SetLineTo as it is
        format!("{:0>3}{}", digits, &line.trim()[digits.len()..])
    } else {
        line.trim().to_string()
    };
    if let Some(i) = v.ty.program.str_var("SetLineTo") {
        v.state.str_vars[i as usize] = text;
    }
}

pub(crate) fn complex_line_text(line: &str, line_num: f32) -> String {
    let line = line.trim();
    if line.chars().all(|c| c.is_ascii_digit()) {
        format!("{:03}  ", line_num as i32)
    } else {
        format!("{line:>5}")
    }
}

/// The letter and digits of a line named letter first ("X10", "M41"), else None.
pub(crate) fn line_prefix(line: &str) -> Option<(char, &str)> {
    let line = line.trim();
    let first = line.chars().next().filter(|c| c.is_ascii_alphabetic())?;
    let digits = &line[1..];
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then_some((first.to_ascii_uppercase(), digits))
}

pub(crate) fn line_suffix_from_text(line: &str) -> u32 {
    // the stock MAN matrices' and X10 Berlin's IBIS's "letter then number" codes
    if let Some((letter, _)) = line_prefix(line) {
        return match letter {
            'E' => 1,
            'S' => 5,
            'A' => 6,
            'D' => 11,
            'C' => 12,
            'B' => 13,
            'U' => 25,
            'M' => 28,
            'N' => 35,
            'X' => 36,
            _ => 0,
        };
    }
    match line.trim().chars().last().map(|c| c.to_ascii_uppercase()) {
        // The stock Matrix scripts use two different E codes: 1 renders E5,
        // while 10 renders 5E. Timetable line names put the letter after the
        // number, so use the latter representation here.
        Some('E') => 10,
        // These are the corresponding "number then letter" branches in the
        // stock MAN matrix scripts. Codes 1/2/3 are not generic suffixes:
        // they render prefixes/special test text and must not be guessed from
        // the letter's alphabetic position.
        Some('U') => 31,
        Some('N') => 4,
        Some('S') => 23,
        Some('M') => 32,
        _ => 0,
    }
}

/// The numeric part of a HOF line code is the line; its route suffix is a
/// separate route selector, not automatically a display-letter code. For a
/// timetable line such as `5E`, the display suffix must therefore come from
/// the text (`10` in the stock matrix scripts), while a plain `5` stays `500`.
pub(crate) fn line_code_from_text(line: &str, route_code: Option<u32>) -> Option<u32> {
    // a lettered line's IBIS number is the depot file's (X10 Berlin types X10 as 510)
    if let (Some(_), Some(code)) = (line_prefix(line), route_code) {
        return Some(code / 100 * 100 + line_suffix_from_text(line));
    }
    // (four and five digit lines too: the IBIS takes line x 100 + suffix whatever the
    // line's length, and a São Paulo 7110 fell back to its route code, whose last two
    // digits - the route, not a suffix - came out on the display as a letter, #459)
    match line_number_digits(line)
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0 && *n < 100_000)
    {
        Some(number) => Some(number * 100 + line_suffix_from_text(line)),
        None => route_code,
    }
}

/// The line's number: its leading digits, or the digits after a prefix letter ("X10" → 10).
pub(crate) fn line_number_digits(line: &str) -> String {
    match line_prefix(line) {
        Some((_, digits)) => digits.to_string(),
        None => line
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect(),
    }
}

/// The key presses a driver makes on the IBIS for a trip.
#[derive(Debug, Clone, PartialEq)]
pub struct IbisCodes {
    /// Complete depot line code: numeric line × 100 plus the letter suffix
    /// (the IBIS reads the last two digits as a suffix; 0 = none).
    pub line: Option<u32>,
    /// Route entry (the last two digits of the depot file's route code).
    pub route: Option<u32>,
    /// Destination entry, when there is no route to type.
    pub terminus: Option<u32>,
}

/// A timetable's terminus is a stop name, while a HOF often uses a shorter
/// destination/texture name for the same stop (Berlin 5E: `Spektefeld
/// Schulzentrum` vs. `Spektefeld`).  Exact matches remain preferred; the
/// boundary-aware prefix fallback handles those stock abbreviations without
/// making unrelated destinations match. The name a row goes by (its ident, its stop, the
/// first line of its sign) counts before another of its lines that reads the same: the
/// district line of Spandau's Machandelweg reads RUHLEBEN, the sign of U Ruhleben, and
/// U Ruhleben by its sign text (what a LAN player's bus says it shows) was Machandelweg.
pub(crate) fn terminus_match_score(t: &omsi_vehicle::hof::Terminus, wanted: &str) -> u8 {
    let wanted = wanted.split_whitespace().collect::<Vec<_>>().join(" ");
    if wanted.is_empty() {
        return 0;
    }
    let shown = t.strings.iter().position(|s| !s.trim().is_empty());
    let mut score = 0;
    for (k, candidate) in std::iter::once(t.texture_id.as_str())
        .chain(std::iter::once(t.terminus_stop.as_deref().unwrap_or("")))
        .chain(t.strings.iter().map(String::as_str))
        .enumerate()
    {
        let candidate = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
        let candidate_lower = candidate.to_lowercase();
        let wanted_lower = wanted.to_lowercase();
        if candidate_lower == wanted_lower {
            score = score.max(if k < 2 || Some(k - 2) == shown { 3 } else { 2 });
        } else if wanted_lower.starts_with(&(candidate_lower.clone() + " ")) {
            score = score.max(1);
        } else if candidate_lower.starts_with(&(wanted_lower + " ")) {
            score = score.max(1);
        }
    }
    score
}

/// The depot file's terminus a trip's destination names. OMSI takes the first whose ident
/// is the name (Omsi.exe TRoadVehicleInst.virtual_10: that row is `AI_target_index`); else
/// the best of the looser matches - the first of equals, not the last (a depot file whose
/// codes are not in row order put the AI bus's matrix on another terminus's picture, #110).
pub(crate) fn find_terminus(hof: &omsi_vehicle::Hof, wanted: &str) -> Option<usize> {
    termini_named(hof, wanted).first().copied()
}

/// Every row of the depot file a trip's destination names, in file order: those whose
/// ident is the name, else those of the best looser match. A depot file may give one
/// destination a row per route, each with a code of its own (four "ul. Xutorskaya" of
/// codes 92, 120, 123 and 124, #738).
pub(crate) fn termini_named(hof: &omsi_vehicle::Hof, wanted: &str) -> Vec<usize> {
    let exact = wanted.trim();
    let rows: Vec<usize> = (0..hof.termini.len()).filter(|&i| hof.termini[i].texture_id == exact).collect();
    if !rows.is_empty() {
        return rows;
    }
    let scores: Vec<u8> = hof.termini.iter().map(|t| terminus_match_score(t, wanted)).collect();
    let best = scores.iter().copied().max().unwrap_or(0);
    (0..scores.len()).filter(|&i| best > 0 && scores[i] == best).collect()
}

/// The depot file's terminus a trip of `line` to `terminus` through `stops` ends at, and
/// the route it takes ([`pick_route`]): of the rows of that name (see [`termini_named`]) the
/// one the line's route through the trip's stops leads to. Taking the first of them, the
/// IBIS got the first's code, a route of the line to it or none at all, whichever route
/// the trip drove (line 39 to "ul. Xutorskaya" typed destination 92, where its route ends
/// at 120). The first row when no route of the line goes to any of them.
pub(crate) fn trip_terminus(
    hof: &omsi_vehicle::Hof,
    line: &str,
    terminus: &str,
    stops: &[&str],
) -> Option<(usize, Option<usize>)> {
    let rows = termini_named(hof, terminus);
    let first = *rows.first()?;
    let mut codes: Vec<i32> = Vec::new();
    for &i in &rows {
        if !codes.contains(&hof.termini[i].code) {
            codes.push(hof.termini[i].code);
        }
    }
    let route = pick_route(hof, &routes_to(hof, line, &codes), stops);
    let ti = route
        .map(|r| omsi_cfg::parse_i32(&hof.info_trips[r].route))
        .and_then(|code| rows.iter().copied().find(|&i| hof.termini[i].code == code))
        .unwrap_or(first);
    Some((ti, route))
}

/// The IBIS codes of a trip from the depot file, and the terminus index they lead to.
/// A line has one route per direction and variant to the same terminus; the one whose
/// stop list follows the trip's stops (`stops`, the timetable's names) best is taken
/// ([`pick_route`]), else the first.
pub fn ibis_codes(
    hof: &omsi_vehicle::Hof,
    line: &str,
    terminus: &str,
    stops: &[&str],
) -> Option<(IbisCodes, usize)> {
    let terminus = terminus.trim();
    if terminus.is_empty() {
        return None;
    }
    let (ti, route) = trip_terminus(hof, line, terminus, stops)?;
    let code = hof.termini[ti].code;
    let route = route.and_then(|i| hof.info_trips[i].code.trim().parse::<u32>().ok());
    let codes = match route {
        Some(r) => IbisCodes {
            line: line_code_from_text(line, Some(r)),
            route: Some(r % 100),
            terminus: None,
        },
        None => IbisCodes {
            line: line_code_from_text(line, None),
            route: None,
            terminus: u32::try_from(code).ok().filter(|c| *c < 1000),
        },
    };
    Some((codes, ti))
}

/// The depot file's routes of `line` to a terminus with one of `codes`, in file order. The
/// route code is the line's number and two digits: a driver types those, whatever the
/// route's line string says (Grundorf's 7601 to Krankenhaus has "TML").
pub(crate) fn routes_to(hof: &omsi_vehicle::Hof, line: &str, codes: &[i32]) -> Vec<usize> {
    let line_digits: String = line
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let line_number = line_digits.parse::<u32>().ok();
    hof.info_trips
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            codes.contains(&omsi_cfg::parse_i32(&t.route))
                && (t.line.trim().eq_ignore_ascii_case(line.trim())
                || (!line_digits.is_empty() && t.line.trim() == line_digits)
                || (line_number.is_some()
                && t.code.trim().parse::<u32>().ok().map(|c| c / 100) == line_number))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Which of `routes` a trip through `stops` (the timetable's station names) runs: one
/// starting at the trip's first stop (the IBIS starts its count there), of those the one
/// that has most of the trip's stops in the trip's order, then the one as long as the trip
/// (a short working's route rather than the long one it is part of). Nothing in the
/// timetable ties a trip to a route of the depot file - a driver picks it by its stops -
/// and a line often has several routes to one terminus; taking the first whose first stop
/// was spelt as the timetable spells it typed the wrong one whenever the spellings
/// differed. None without routes; the first route when no stop matches any.
pub(crate) fn pick_route(hof: &omsi_vehicle::Hof, routes: &[usize], stops: &[&str]) -> Option<usize> {
    let trip: Vec<(String, Vec<String>)> = stops
        .iter()
        .map(|s| (s.trim().to_lowercase(), stop_words(s)))
        .filter(|(s, _)| !s.is_empty())
        .collect();
    let mut best: Option<(usize, (bool, usize, std::cmp::Reverse<usize>))> = None;
    for &r in routes {
        let list = hof
            .info_busstop_lists
            .get(r)
            .map(|l| l.as_slice())
            .unwrap_or(&[]);
        let names: Vec<Vec<(String, Vec<String>)>> =
            list.iter().map(|id| ident_names(hof, id)).collect();
        // the longest common subsequence: a stop missing on either side costs nothing
        // but itself, and a stop the route lists twice is not
        // matched past the rest of the trip
        let mut row = vec![0usize; names.len() + 1];
        for t in &trip {
            let mut diag = 0;
            for (j, n) in names.iter().enumerate() {
                let up = row[j + 1];
                row[j + 1] = if same_stop(n, t) { diag + 1 } else { up.max(row[j]) };
                diag = up;
            }
        }
        let found = row[names.len()];
        let starts = match (trip.first(), names.first()) {
            (Some(t), Some(n)) => same_stop(n, t),
            _ => false,
        };
        let score = (
            starts,
            found,
            std::cmp::Reverse(names.len().abs_diff(stops.len())),
        );
        if best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((r, score));
        }
    }
    match best {
        Some((r, (_, found, _))) if found > 0 => Some(r),
        _ => routes.first().copied(),
    }
}

/// The names a stop of a route's list goes by: its ident (before a `#`) and the strings
/// the depot file's `[addbusstop]` of that ident gives it, each lowercased and as its
/// [`stop_words`].
pub(crate) fn ident_names(hof: &omsi_vehicle::Hof, ident: &str) -> Vec<(String, Vec<String>)> {
    let ident = ident.split('#').next().unwrap_or("").trim();
    let mut names = vec![ident.to_string()];
    for b in &hof.bus_stops {
        if b.ident.trim().eq_ignore_ascii_case(ident) {
            names.extend(b.strings.iter().map(|s| s.trim().to_string()));
        }
    }
    names
        .into_iter()
        .filter(|n| !n.is_empty())
        .map(|n| (n.to_lowercase(), stop_words(&n)))
        .collect()
}

/// One stop of a route (its [`ident_names`]) and a timetable stop (lowercased, and its
/// [`stop_words`]) are the same: a name equal, one the start of the other, or the same
/// words.
pub(crate) fn same_stop(names: &[(String, Vec<String>)], stop: &(String, Vec<String>)) -> bool {
    names.iter().any(|(raw, words)| {
        !raw.is_empty()
            && (*raw == stop.0
            || raw.starts_with(&stop.0)
            || stop.0.starts_with(raw.as_str())
            || (!words.is_empty() && *words == stop.1))
    })
}

/// A stop name as the set of its words, so that the map's and the depot file's spellings
/// of one stop meet: in any order ("Nordstadt Bhf", "Bhf Nordstadt"), with any punctuation
/// ("Bhf. Nordstadt") and without one-letter prefixes ("F_Kirchweg", "Kirchweg").
pub(crate) fn stop_words(name: &str) -> Vec<String> {
    let mut w: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(|w| w.to_lowercase())
        .collect();
    w.sort();
    w
}

pub(crate) fn set_destination(
    v: &mut crate::VehicleInstance,
    hof: Option<&omsi_vehicle::Hof>,
    line: &str,
    terminus: &str,
    stops: &[&str],
    player: bool,
) {
    let terminus = terminus.trim();
    // Omsi.exe's `TRoadVehicleInst.virtual_10` runs for every trip a bus starts: the line
    // goes into `SetLineTo` and the script's `ai_scheduled_settarget` trigger runs even when
    // the depot file has no such terminus (`AI_target_index` then stays as it was) and even
    // for a bus whose group names no depot file at all. The mods' roller blinds and matrices
    // switch their destination picture on in that trigger, so an AI bus that skipped it
    // drove with a blank display (see `pool_depot`).
    let Some(hof) = hof.filter(|_| !terminus.is_empty()) else {
        if !player {
            set_line_to(v, line);
            v.trigger("ai_scheduled_settarget");
        }
        return;
    };
    // (an AI bus shows the first row of the name, as Omsi.exe gives it; the player's IBIS
    // the row its route leads to, as the typing does)
    let term_index = if player {
        trip_terminus(hof, line, terminus, stops).map(|(ti, _)| ti)
    } else {
        find_terminus(hof, terminus)
    };
    let Some(ti) = term_index else {
        log::debug!(
            "AI bus: terminus '{terminus}' not in depot file {} ({} termini)",
            hof.name,
            hof.termini.len()
        );
        if !player {
            set_line_to(v, line);
            v.trigger("ai_scheduled_settarget");
        }
        return;
    };
    log::debug!(
        "AI bus: line {line} terminus '{terminus}' → depot terminus {ti} code {}",
        hof.termini[ti].code
    );
    set_destination_at(v, hof, line, ti, stops, player)
}

/// The newest destination waiting for an electronic display's current transition.
pub(crate) struct PendingDestination {
    hof: std::sync::Arc<omsi_vehicle::Hof>,
    line: String,
    ti: usize,
    stops: Vec<String>,
    player: bool,
}

fn destination_transition_active(v: &crate::VehicleInstance) -> bool {
    v.ty.program.trigger("ai_scheduled_settarget").is_some()
        && !has_roller_blind(v)
        && ["LW_req_ziel_change", "Matrix_ziel_animationtimer"]
            .iter()
            .any(|name| v.var(name).is_some_and(|timer| timer > 0.0))
}

/// Called before a script frame, so the previous transition gets its final frame
/// before the newest request starts a fresh, complete transition.
pub(crate) fn apply_pending_destination(v: &mut crate::VehicleInstance) {
    if v.pending_destination.is_none() || destination_transition_active(v) {
        return;
    }
    let pending = v.pending_destination.take().unwrap();
    // A depot change makes the old row meaningless to the vehicle's scripts.
    if v.host.hof.as_deref() != Some(pending.hof.as_ref()) {
        return;
    }
    let stops: Vec<&str> = pending.stops.iter().map(String::as_str).collect();
    set_destination_at(v, &pending.hof, &pending.line, pending.ti, &stops, pending.player);
}

/// [`set_destination`] with the depot file's terminus `ti` itself.
pub(crate) fn set_destination_at(
    v: &mut crate::VehicleInstance,
    hof: &omsi_vehicle::Hof,
    line: &str,
    ti: usize,
    stops: &[&str],
    player: bool,
) {
    let Some(term) = hof.termini.get(ti) else { return };
    if destination_transition_active(v) {
        // Keep script inputs unchanged until the current animation finishes. Replacing
        // this single slot coalesces rapid selections instead of animating every one.
        let hof = v.host.hof.as_ref()
            .filter(|h| h.as_ref() == hof)
            .cloned()
            .unwrap_or_else(|| std::sync::Arc::new(hof.clone()));
        v.pending_destination = Some(PendingDestination {
            hof,
            line: line.to_string(),
            ti,
            stops: stops.iter().map(|s| s.to_string()).collect(),
            player,
        });
        return;
    }
    // An immediate selection supersedes an older request that has not been flushed yet.
    v.pending_destination = None;
    let code = term.code;
    let route_index = pick_route(hof, &routes_to(hof, line, &[code]), stops);
    let line_num = line_number_digits(line).parse::<f32>().unwrap_or(0.0);
    // The route's last two digits select its stop list; they must not replace
    // a display suffix. Otherwise an ordinary route code such as 505 becomes
    // suffix 5 and the stock matrix renders S5 instead of 5E.
    let route_code = route_index
        .and_then(|i| hof.info_trips.get(i))
        .and_then(|t| t.code.trim().parse::<u32>().ok());
    let line_code =
        line_code_from_text(line, route_code).unwrap_or_else(|| line_num.max(0.0) as u32 * 100);
    let line_suffix = (line_code % 100) as f32;
    let line_num = if line_prefix(line).is_some() {
        (line_code / 100) as f32
    } else {
        line_num
    };
    // the original's way: SetLineTo + AI_target_index, then the ai_scheduled_settarget trigger
    set_line_to(v, line);
    v.set_var("AI_target_index", ti as f32);
    let pending_blind = player.then(|| v.var("rlbnd_ziel_target")).flatten();
    let target_triggered = v.trigger("ai_scheduled_settarget");
    // The trigger also feeds destination displays, but a hand-cranked blind must remain
    // pending until `turn_roller_blind` applies the driver's selection.
    if player {
        if let Some(row) = pending_blind {
            v.set_var("rlbnd_ziel_target", row);
        }
    }
    if !player && target_triggered {
        v.set_var(
            "IBIS_RouteIndex",
            route_index.map(|r| r as f32).unwrap_or(-1.0),
        );
        return;
    }
    v.set_var("IBIS_LinieKurs", line_num);
    v.set_var("IBIS_Linie_Complex", line_code as f32);
    v.set_var("IBIS_Linie_Suffix", line_suffix);
    v.set_var("IBIS_TerminusIndex", ti as f32);
    v.set_var("IBIS_TerminusCode", code as f32);
    v.set_var(
        "IBIS_RouteIndex",
        route_index.map(|r| r as f32).unwrap_or(-1.0),
    );
    v.set_var("IBIS_mode", 0.0);
    let set_str = |v: &mut crate::VehicleInstance, name: &str, val: String| {
        if let Some(i) = v.ty.program.str_var(name) {
            v.state.str_vars[i as usize] = val;
        }
    };
    set_str(
        v,
        "IBIS_terminus_name",
        hof.termini[ti].display_name(),
    );
    let complex = if line_num > 0.0 {
        complex_line_text(line, line_num)
    } else {
        "     ".into()
    };
    set_str(v, "IBIS_Complex_Line", complex);
    // terminus texture change ident for roller blinds / matrix textures
    set_str(
        v,
        "IBIS_terminus_texture",
        hof.termini[ti].texture_id.clone(),
    );
}
