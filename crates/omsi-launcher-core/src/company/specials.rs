//! Special trips and special timetables: what of a map's timetable is not a line to run under
//! a concession (Luc: a depot run cannot be bought - it belongs to the tours that need it;
//! "and there are a few more special ones").
//!
//! The maps write them in many ways. Depot runs ("Betriebsfahrt": from the depot to a tour's
//! first stop and back) come as trips of their own line number ("X" in Bad Hügelsdorf, "B" in
//! Hohenkirchen and the Thuringian Forest, often none at all in Berlin-Spandau and
//! Rheinhausen), with the depot or "Betriebsfahrt" on the display, inside the tours that need
//! them. Empty positioning runs ("Leerfahrt", the Hamburg maps' "AW"/"EW" runs and their
//! out-of-service display "www.hochbahn.de", Vienna's "Sonderwagen") likewise; Hamburg also
//! keeps a whole timetable of them ("Leer"). Event shuttles ("(Shuttle) Halloween", "FFF
//! Shuttle") come with the chrono days of Bad Hügelsdorf; school runs ("Schulbus", Hohenkirchen's
//! line "S") run on school days; "KI-" timetables are other operators' traffic for the AI, as
//! are the trains, ships, planes, taxis and refuse lorries of some maps.
//!
//! What follows from the kind: a depot run, an empty run, a workshop run, a driving-school
//! run or a test drive carries no passengers - inside a company tour it is run with it,
//! booked as empty kilometres and driver time (`Kind::empty`); such timetables, rail
//! replacement and occasional specials are never offered as concessions nor added as lines of
//! their own (`Kind::line`); school runs are lines like the others (the school authority's
//! contract), running on school days.

use crate::LineInfo;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A line with passengers.
    Service,
    /// School runs: a line too, on school days.
    School,
    /// A depot run: to or from the depot, part of its tour.
    Depot,
    /// An empty positioning run (Leerfahrt, Dienstfahrt, Einsetzer and Aussetzer runs,
    /// Überführung): part of its tour.
    Empty,
    /// To or from the workshop: part of the workshop's jobs.
    Workshop,
    /// Driving-school runs: the training courses'.
    DrivingSchool,
    /// Test drives: the dealer's.
    TestDrive,
    /// Rail replacement (Schienenersatzverkehr): an occasional contract of the rail operator.
    RailReplacement,
    /// Event shuttles, specials, sightseeing: occasional contracts.
    Occasional,
    /// Not the company's: other operators' AI traffic, trains, ships, planes, taxis, lorries.
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Service => "Line",
            Kind::School => "School runs",
            Kind::Depot => "Depot run",
            Kind::Empty => "Empty run",
            Kind::Workshop => "Workshop run",
            Kind::DrivingSchool => "Driving school",
            Kind::TestDrive => "Test drive",
            Kind::RailReplacement => "Rail replacement",
            Kind::Occasional => "Special service",
            Kind::Other => "Other traffic",
        }
    }

    /// A trip of this kind carries no passengers: its kilometres and its driver's time are
    /// booked with its tour, without fares or the authority's payment.
    pub fn empty(self) -> bool {
        matches!(self, Kind::Depot | Kind::Empty | Kind::Workshop | Kind::DrivingSchool | Kind::TestDrive)
    }

    /// A timetable of this kind is a line of its own: offered under a concession, added on
    /// the Lines page.
    pub fn line(self) -> bool {
        matches!(self, Kind::Service | Kind::School)
    }
}

/// Lower case with the umlauts written out ("Überführung" → "ueberfuehrung").
fn fold(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().to_lowercase().chars() {
        match ch {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c => out.push(c),
        }
    }
    out
}

fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
}

/// The kind of a trip, by its line number (`line`: what its displays show), its name, its
/// destination (`terminus`: the display's text), its stops, and the map's depots (a trip to
/// one of them is a depot run).
pub fn trip_kind(line: &str, name: &str, terminus: &str, stops: usize, depots: &[&str]) -> Kind {
    let (l, n, t) = (fold(line), fold(name), fold(terminus));
    let all = format!("{n} {t}");
    let has = |k: &str| all.contains(k);
    let nw = words(&n);
    // other traffic
    if n.starts_with("ki-") || n.starts_with("ki_") || nw.first() == Some(&"ki") || has("taxi") || has("muellwagen") || words(&all).contains(&"muell") || has("postdrohne") || has("falschpark") || has("flugzeug") || has("hubschrauber") || has("schiff") || has("faehre") {
        return Kind::Other;
    }
    if has("fahrschul") {
        return Kind::DrivingSchool;
    }
    if has("werkstatt") {
        return Kind::Workshop;
    }
    if has("testfahrt") || has("probefahrt") {
        return Kind::TestDrive;
    }
    if has("schienenersatz") || has("ersatzverkehr") || words(&all).contains(&"sev") || l.starts_with("sev") {
        return Kind::RailReplacement;
    }
    // depot runs: the display says so, the line is the depot runs' code, or it goes to a depot
    let depot_word = |s: &str| s.contains("betriebsfahrt") || s.contains("betriebshof") || s.contains("betriebsg") || s.contains("depot");
    if depot_word(&t) || matches!(l.as_str(), "x" | "b" | "bf" | "0") || n.contains("betriebsfahrt") || depots.iter().any(|d| !d.trim().is_empty() && fold(d) == t) {
        return Kind::Depot;
    }
    // empty runs: the display says so, or the name
    if t.contains("leerfahrt") || t.contains("dienstfahrt") || t.contains("sonderwagen") || t.contains("nicht einsteigen") || t.starts_with("www.") || n.contains("(leer)") || nw.first() == Some(&"leer") || ["leerfahrt", "dienstfahrt", "ueberfuehrung", "einsetzer", "aussetzer"].iter().any(|k| n.contains(k)) {
        return Kind::Empty;
    }
    if has("shuttle") || has("sonderfahrt") || has("sonderverkehr") || has("stadtrund") {
        return Kind::Occasional;
    }
    if has("schulbus") || has("schuelerverkehr") || n.starts_with("sb_") || (matches!(l.as_str(), "s" | "sb") && t.contains("schul")) {
        return Kind::School;
    }
    // (a trip of under three stops carries nobody: a positioning run, the Hamburg maps'
    // "AW"/"EW" runs among them)
    if stops < 3 {
        return Kind::Empty;
    }
    Kind::Service
}

/// The kind of a timetable (a `.ttl`): by its name, whether the player may drive it, and its
/// trips (a line with any passenger trip is a line; the trips' kind otherwise).
pub fn line_kind(l: &LineInfo, depots: &[&str]) -> Kind {
    let schulbus = fold(&l.name).contains("schulbus");
    if let Some(k) = by_name(&l.name) {
        return k;
    }
    if !l.user_allowed && !schulbus {
        return Kind::Other;
    }
    by_trips(l, depots, schulbus)
}

/// A timetable that is special by its name or its trips, whether the player may drive it or
/// not (None: a line, if only for the AI) - what the Lines page leaves out.
pub fn special_line(l: &LineInfo, depots: &[&str]) -> Option<Kind> {
    let k = by_name(&l.name).unwrap_or_else(|| by_trips(l, depots, fold(&l.name).contains("schulbus")));
    (!k.line()).then_some(k)
}

fn by_name(name: &str) -> Option<Kind> {
    let nm = fold(name);
    let w = words(&nm);
    Some(if w.first() == Some(&"leer") || nm.contains("leerfahrt") {
        Kind::Empty
    } else if nm.contains("betriebsfahrt") {
        Kind::Depot
    } else if nm.contains("testfahrt") {
        Kind::TestDrive
    } else if nm.contains("fahrschul") {
        Kind::DrivingSchool
    } else if nm.contains("werkstatt") {
        Kind::Workshop
    } else if w.contains(&"sev") || nm.contains("schienenersatz") || nm.contains("ersatzverkehr") {
        Kind::RailReplacement
    } else if nm.contains("shuttle") || nm.contains("sonderverkehr") || nm.contains("sonderfahrt") || nm.contains("stadtrund") {
        Kind::Occasional
    } else if nm.starts_with("ki-") || w.first() == Some(&"ki") || other_traffic(&nm) {
        Kind::Other
    } else {
        return None;
    })
}

/// A name of traffic no bus company runs: planes, helicopters, ships and ferries, trains, taxis,
/// lorries (a timetable "Flugzeug" was offered as a line, Luc).
fn other_traffic(folded: &str) -> bool {
    let w = words(folded);
    ["flugzeug", "flieger", "hubschrauber", "helikopter", "schiff", "faehre", "taxi", "lkw"].iter().any(|k| folded.contains(k)) || ["zug", "zuege", "ice", "re", "rb"].iter().any(|k| w.first() == Some(k))
}

fn by_trips(l: &LineInfo, depots: &[&str], schulbus: bool) -> Kind {
    let mut kinds: Vec<(Kind, usize)> = Vec::new();
    for t in l.tours.iter().flat_map(|t| t.trips.iter()) {
        let k = trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), depots);
        match kinds.iter_mut().find(|x| x.0 == k) {
            Some(x) => x.1 += 1,
            None => kinds.push((k, 1)),
        }
    }
    let any = |k: Kind| kinds.iter().any(|x| x.0 == k);
    if any(Kind::Service) {
        return if schulbus { Kind::School } else { Kind::Service };
    }
    if any(Kind::School) || schulbus {
        return Kind::School;
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    kinds.first().map(|x| x.0).unwrap_or(Kind::Other)
}

/// What a line's passenger trips run between: their two most frequent destinations ("A – B";
/// not the depot runs' "Betriebsfahrt").
pub fn caption_of(l: &LineInfo, depots: &[&str]) -> String {
    let mut seen: Vec<(String, usize, usize)> = Vec::new();
    for t in l.tours.iter().flat_map(|t| t.trips.iter()).filter(|t| !trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), depots).empty()) {
        let name = destination(&t.terminus);
        if name.is_empty() {
            continue;
        }
        let n = seen.len();
        match seen.iter_mut().find(|x| x.0 == name) {
            Some(x) => x.1 += 1,
            None => seen.push((name.to_string(), 1, n)),
        }
    }
    seen.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    let mut two: Vec<&(String, usize, usize)> = seen.iter().take(2).collect();
    two.sort_by_key(|x| x.2);
    two.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(" – ")
}

/// A display's destination: its first field (Bad Hügelsdorf writes "Hauptbahnhof", a run of
/// spaces and "301" for the display's two parts).
fn destination(terminus: &str) -> &str {
    let t = terminus.trim();
    t.find("   ").map_or(t, |k| t[..k].trim_end())
}

/// The number a line shows: its first passenger trip's, else its name.
pub fn number_of(l: &LineInfo, depots: &[&str]) -> String {
    l.tours
        .iter()
        .flat_map(|t| t.trips.iter())
        .filter(|t| !trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), depots).empty())
        .map(|t| t.line.trim())
        .find(|n| !n.is_empty())
        .unwrap_or(&l.name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StopInfo, TourInfo, TripInfo};

    fn k(line: &str, name: &str, terminus: &str, stops: usize) -> Kind {
        trip_kind(line, name, terminus, stops, &["Betriebshof Grundorf"])
    }

    #[test]
    fn the_maps_special_trips_are_known_by_their_names_and_displays() {
        // Bad Hügelsdorf: line X, "(leer)", the depot on the display; its pull-outs in service
        assert_eq!(k("X", "(leer) Betriebshof VBBH - Altes Schloss", "Betriebsfahrt", 3), Kind::Depot);
        assert_eq!(k("X", "(leer) Altes Schloss - Betriebshof VBBH", "Betriebshof VBBH", 3), Kind::Depot);
        assert_eq!(k("307", "(Ausrueck_307) Betriebshof VBBH - Bismarckplatz", "Bismarckplatz", 8), Kind::Service);
        assert_eq!(k("307", "(Einrueck_307) Bismarckplatz - Betriebshof VBBH", "Malger Weg", 8), Kind::Service);
        // Berlin-Spandau and Rheinhausen: no number, "Betriebsfahrt" (even past many stops)
        assert_eq!(k("", "", "Betriebsfahrt", 2), Kind::Depot);
        assert_eq!(k("92", "", "Betriebsfahrt", 21), Kind::Depot);
        // Hohenkirchen and the Thuringian Forest: line B; the school runs of line S
        assert_eq!(k("B", "B_GSH2-HOB3", "Betriebsfahrt", 2), Kind::Depot);
        assert_eq!(k("S", "S_HIL-GSH2", "Gesamtschule Hohenkirchen", 9), Kind::School);
        assert_eq!(k("", "SB_WBG_ZOBA", "Schulbus", 3), Kind::School);
        assert_eq!(k("X49", "49_HAA-HOB2", "Hohenkirchen, Bahnhof", 18), Kind::Service);
        // Hamburg: the out-of-service display, "Leerfahrt", the short AW/EW runs
        assert_eq!(k("109", "109 P1UALHBF", "www.hochbahn.de", 2), Kind::Empty);
        assert_eq!(k("688", "688 ATH P", "PVG LEERFAHRT", 2), Kind::Empty);
        assert_eq!(k("183", "183_AW", "Schnelsen Kalvslohtwiete", 2), Kind::Empty);
        assert_eq!(k("777", "taxi1", "S Wedel", 2), Kind::Other);
        // Vienna: "Sonderwagen" and the garage
        assert_eq!(k("24A", "", "Sonderwagen", 2), Kind::Empty);
        assert_eq!(k("23A", "", "Vorgartenstr Betriebsg", 2), Kind::Depot);
        // the company's depot; the rest of the special kinds
        assert_eq!(k("5", "5 Rathaus - Depot", "Betriebshof Grundorf", 6), Kind::Depot);
        assert_eq!(k("F", "Fahrschule Runde 1", "Fahrschule", 6), Kind::DrivingSchool);
        assert_eq!(k("", "Werkstattfahrt Hof", "Werkstatt", 2), Kind::Workshop);
        assert_eq!(k("SEV", "SEV Bahnhof - Nord", "Bahnhof", 8), Kind::RailReplacement);
        assert_eq!(k("", "Überführung Nord", "Nordpark", 4), Kind::Empty);
        assert_eq!(k("", "(Shuttle) Halloween", "Altstadt", 6), Kind::Occasional);
        assert_eq!(k("", "KI-5_HFW-WEH", "Wederhof", 3), Kind::Other);
        // a stop named after a school, or an "Omnibushof", is no special
        assert_eq!(k("25", "", "Pestalozzischule", 11), Kind::Service);
        assert_eq!(k("13N", "", "Am Omnibushof", 13), Kind::Service);
        assert!(Kind::Depot.empty() && Kind::Empty.empty() && !Kind::School.empty() && !Kind::Service.empty());
        assert!(Kind::School.line() && !Kind::Depot.line() && !Kind::Occasional.line());
    }

    fn timetable(name: &str, allowed: bool, trips: &[(&str, &str, &str, usize)]) -> LineInfo {
        let trip = |(k, (line, name, term, stops)): (usize, &(&str, &str, &str, usize))| TripInfo {
            name: name.to_string(),
            index: k + 1,
            line: line.to_string(),
            from: "A".into(),
            terminus: term.to_string(),
            departure: 0.0,
            arrival: 0.0,
            stops: (0..*stops).map(|s| StopInfo { name: format!("S{s}"), id: s as i64, arr: 0.0, dep: 0.0 }).collect(),
            km: 1.0,
        };
        LineInfo {
            name: name.into(),
            user_allowed: allowed,
            termini: vec![],
            tours: vec![TourInfo { number: "1".into(), ai_group: String::new(), first: 0.0, last: 0.0, days: "daily".into(), runs: true, next_run: None, trips: trips.iter().enumerate().map(trip).collect() }],
        }
    }

    #[test]
    fn a_timetable_is_a_line_only_with_passenger_trips() {
        let day = timetable("Montag - Freitag", true, &[("X", "(leer) Betriebshof VBBH - HBF", "Betriebsfahrt", 3), ("301", "301_HBF - Kunsthalle", "Kunsthalle", 9)]);
        assert_eq!(line_kind(&day, &[]), Kind::Service);
        assert_eq!(number_of(&day, &[]), "301", "not the depot run's X");
        assert_eq!(caption_of(&day, &[]), "Kunsthalle");
        assert_eq!(line_kind(&timetable("Leer", false, &[("", "Leer_D-FAR", "", 2)]), &[]), Kind::Empty);
        assert_eq!(line_kind(&timetable("Betriebsfahrten", true, &[("", "", "Betriebsfahrt", 2)]), &[]), Kind::Depot);
        assert_eq!(line_kind(&timetable("Testfahrt", true, &[("1", "T", "Rathaus", 5)]), &[]), Kind::TestDrive);
        assert_eq!(line_kind(&timetable("S_Schulbus", true, &[("S", "", "Oberfeld-Gesamtschule", 14)]), &[]), Kind::School);
        assert_eq!(line_kind(&timetable("FFF Shuttle", true, &[("", "", "Augustaplatz", 6)]), &[]), Kind::Occasional);
        assert_eq!(line_kind(&timetable("KI-Linien", true, &[("5", "KI-5_HFW-WEH", "Wederhof", 3)]), &[]), Kind::Other);
        assert_eq!(line_kind(&timetable("RB", false, &[("RB", "RB_Gleis1", "Rastatt", 2)]), &[]), Kind::Other);
        assert_eq!(line_kind(&timetable("76", true, &[("76", "", "Bauernhof", 7)]), &[]), Kind::Service);
        // the Lines page: an AI-only line is still a line there, a timetable of depot runs not
        assert_eq!(special_line(&timetable("251", false, &[("251", "", "Bahnhof", 9)]), &[]), None);
        // (other traffic by its name: a plane is no line, an airfield's bus is)
        assert_eq!(special_line(&timetable("Flugzeug", false, &[("Flugplatz BDL", "", "Flugzeug", 5)]), &[]), Some(Kind::Other));
        assert_eq!(special_line(&timetable("Flugplatz", true, &[("F", "", "Flugplatz", 6)]), &[]), None);
        assert_eq!(special_line(&timetable("Leer", false, &[("", "Leer_D-FAR", "", 2)]), &[]), Some(Kind::Empty));
        // "Mueller-Touristik" is an operator, not a refuse lorry
        assert_eq!(k("B", "B_ZOBC_Kessler", "Mueller-Touristik", 3), Kind::Depot);
        assert_eq!(k("731", "731_TBO_WBGA", "Mueller-Touristik", 9), Kind::Service);
    }

    /// The installed maps' timetables as the market sees them (read only; run by hand:
    /// `cargo test -p omsi-launcher-core real_maps -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn real_maps() {
        let root = std::path::Path::new(r"C:\Program Files (x86)\Steam\steamapps\common\OMSI 2\maps");
        let Ok(rd) = std::fs::read_dir(root) else { return };
        for d in rd.flatten().filter(|d| d.path().join("TTData").is_dir()) {
            let Ok(lines) = crate::lines_on(&d.path(), "2024-03-04") else { continue };
            let mut out: Vec<String> = Vec::new();
            for l in &lines {
                let k = line_kind(l, &[]);
                let empty = l.tours.iter().flat_map(|t| t.trips.iter()).filter(|t| trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), &[]).empty()).count();
                let all = l.tours.iter().map(|t| t.trips.len()).sum::<usize>();
                out.push(format!("{} [{}] {:?} {empty}/{all} empty", l.name, number_of(l, &[]), k));
            }
            println!("== {}
   {}", d.file_name().to_string_lossy(), out.join("
   "));
        }
    }

    #[test]
    fn bad_huegelsdorfs_depot_run_x_is_never_a_line_of_its_own() {
        // (the map's real timetables, read only, where the game is installed)
        let dir = std::path::Path::new("C:/Program Files (x86)/Steam/steamapps/common/OMSI 2/maps/Bad_Huegelsdorf_2020");
        let Ok(lines) = crate::lines_on(dir, "2005-10-14") else { return };
        let depots = ["Bad Huegelsdorf 2020 VBBH"];
        let mut days = 0;
        for l in &lines {
            for t in l.tours.iter().flat_map(|t| t.trips.iter()).filter(|t| t.line.trim().eq_ignore_ascii_case("x")) {
                assert_eq!(trip_kind(&t.line, &t.name, &t.terminus, t.stops.len(), &depots), Kind::Depot, "{}", t.name);
            }
            if line_kind(l, &depots).line() {
                days += 1;
                // (the whole-day timetables open with a run of X: named after their passenger trips)
                assert_ne!(number_of(l, &depots), "X", "{}", l.name);
                let caption = caption_of(l, &depots);
                assert!(!caption.contains("Betriebsfahrt") && !caption.contains("   "), "{}: {caption}", l.name);
            }
        }
        if !lines.is_empty() {
            assert!(days >= 3, "the weekday, Saturday and Sunday timetables");
        }
    }
}
