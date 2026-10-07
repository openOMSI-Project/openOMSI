//! Rankings: the player's company among the others of its map, and the player among the
//! drivers. In a single-player game the others are rivals made up per map - the same ones
//! every time for that map, growing as the player's company grows older and the player
//! drives more, so that there is always somebody ahead - and a multiplayer server will hand
//! in its real companies and drivers as `Entry`s of their own (`rank` takes any mix).

use super::career;
use super::dates;
use super::levels;
use super::model::Company;
use super::rng::Rng;

/// One place in a table.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// The player's own (his company, himself).
    pub you: bool,
    /// What the table is ordered by: experience points.
    pub points: i64,
    pub level: u32,
    /// Trips on time (companies) or the average evaluation (drivers), 0 - 100.
    pub quality: f64,
    /// Buses (companies) or trips driven (drivers).
    pub count: u32,
}

/// The table in its order: the most points first (equal points: the player first, then by
/// name). Returns it with the places numbered from 1.
pub fn rank(mut entries: Vec<Entry>) -> Vec<(usize, Entry)> {
    entries.sort_by(|a, b| b.points.cmp(&a.points).then(b.you.cmp(&a.you)).then(a.name.cmp(&b.name)));
    entries.into_iter().enumerate().map(|(k, e)| (k + 1, e)).collect()
}

/// The player's place in a ranked table.
pub fn place_of(table: &[(usize, Entry)]) -> Option<usize> {
    table.iter().find(|x| x.1.you).map(|x| x.0)
}

const KINDS: [&str; 8] = ["Verkehrsbetriebe", "Regiobus", "Stadtbus", "Omnibus", "Busverkehr", "Linienverkehr", "Reisedienst", "Kraftverkehr"];
const FAMILIES: [&str; 16] = ["Müller", "Schulte", "Krause", "Becker", "Hoffmann", "Wagner", "Peters", "Vogel", "Lehmann", "Brandt", "Jansen", "Kowalski", "Fischer", "Roth", "Albers", "Hahn"];
const PLACES: [&str; 10] = ["Nord", "Süd", "Land", "Kreis", "Mitte", "Ost", "West", "Tal", "Heide", "Berg"];
const FIRST: [&str; 16] = ["Anna", "Jonas", "Lea", "Felix", "Mia", "Lukas", "Emma", "Paul", "Sophie", "Ben", "Lena", "Tim", "Clara", "Max", "Nora", "Erik"];

/// A map's file as the rivals are drawn for it (either slash, any case).
fn map_key(map: &str) -> String {
    map.trim().replace('\\', "/").to_lowercase()
}

/// The map's short name from its file (`maps/Grundorf/global.cfg` Grundorf).
fn place_name(map: &str) -> String {
    let parts: Vec<&str> = map.split(['/', '\\']).filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("global.cfg") && !p.eq_ignore_ascii_case("maps")).collect();
    let name = parts.last().copied().unwrap_or("Stadt");
    let name: String = name.split(['_', '-', ' ']).next().unwrap_or(name).chars().take(16).collect();
    if name.is_empty() {
        "Stadt".to_string()
    } else {
        name
    }
}

/// The rival companies of a map, `days` after the player's company was founded (seven of
/// them: a big one of the town ahead from the start, the rest at their own pace), none of
/// them named `avoid` (the player's company).
pub fn rival_companies(map: &str, days: i64, avoid: &str) -> Vec<Entry> {
    let mut rng = Rng::of(&["rivals", &map_key(map)], 0);
    let place = place_name(map);
    let days = days.max(0) as f64;
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for k in 0..7 {
        let name = loop {
            let kind = *rng.pick(&KINDS).unwrap_or(&"Omnibus");
            let n = match rng.int(0, 2) {
                0 => format!("{kind} {place}"),
                1 => format!("{kind} {}", rng.pick(&FAMILIES).unwrap_or(&"Müller")),
                _ => format!("{kind} {place}-{}", rng.pick(&PLACES).unwrap_or(&"Land")),
            };
            if !used.contains(&n) && !n.eq_ignore_ascii_case(avoid.trim()) {
                break n;
            }
        };
        used.push(name.clone());
        // (the first is the town's big operator: far ahead; the others start small and grow
        // about as fast as a well run company does)
        let start = if k == 0 { rng.range(9_000.0, 14_000.0) } else { rng.range(0.0, 3_000.0) };
        let rate = if k == 0 { rng.range(40.0, 70.0) } else { rng.range(45.0, 150.0) };
        let points = (start + rate * days.powf(0.95)).round() as i64;
        let quality = rng.range(78.0, 97.0);
        out.push(Entry { name, you: false, points, level: levels::level_of(points), quality: (quality * 10.0).round() / 10.0, count: (4 + points / 1_500) as u32 });
    }
    out
}

/// The player's company as an entry.
pub fn company_entry(c: &Company) -> Entry {
    Entry { name: c.name.clone(), you: true, points: c.progress.xp, level: levels::level(c), quality: c.punctuality, count: c.fleet.len() as u32 }
}

/// The table of the companies of the player's company's map.
pub fn company_table(c: &Company) -> Vec<(usize, Entry)> {
    let days = dates::between(&c.founded, &c.date);
    let mut all = rival_companies(&c.map, days, &c.name);
    all.push(company_entry(c));
    rank(all)
}

/// The rival drivers of a map when the player has driven `trips` trips (eight of them, each
/// at his own pace per trip).
pub fn rival_drivers(map: &str, trips: usize) -> Vec<Entry> {
    let mut rng = Rng::of(&["drivers", &map_key(map)], 0);
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::new();
    while out.len() < 8 {
        let name = format!("{} {}", rng.pick(&FIRST).unwrap_or(&"Anna"), rng.pick(&FAMILIES).unwrap_or(&"Müller"));
        if used.contains(&name) {
            continue;
        }
        used.push(name.clone());
        let start = rng.range(0.0, 4_000.0);
        let rate = rng.range(90.0, 260.0);
        let driven = (trips as f64 * rng.range(0.6, 1.4)).round();
        let points = (start + rate * driven).round() as i64;
        let score = rng.range(62.0, 93.0);
        out.push(Entry { name, you: false, points, level: career::level_of(points), quality: (score * 10.0).round() / 10.0, count: driven as u32 });
    }
    out
}

/// The table of the drivers: the player (`you`, his experience, average score and trips)
/// among the map's rivals.
pub fn driver_table(map: &str, you: &str, summary: &career::Summary) -> Vec<(usize, Entry)> {
    let mut all = rival_drivers(map, summary.trips);
    all.push(Entry { name: you.to_string(), you: true, points: summary.progress.xp, level: summary.progress.level, quality: summary.average_score.unwrap_or(0.0), count: summary.trips as u32 });
    rank(all)
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;

    #[test]
    fn the_rivals_are_the_maps_and_grow() {
        let a = rival_companies("maps/Grundorf/global.cfg", 0, "");
        assert_eq!(a, rival_companies("maps\\Grundorf\\global.cfg", 0, ""), "the same rivals whatever the slashes");
        assert_eq!(a.len(), 7);
        assert!(a.iter().any(|e| e.name.contains("Grundorf")));
        let names: std::collections::HashSet<&str> = a.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 7);
        let later = rival_companies("maps/Grundorf/global.cfg", 200, "");
        assert!(later.iter().zip(&a).all(|(l, e)| l.points > e.points));
        assert_ne!(rival_companies("maps/Spandau/global.cfg", 0, "")[0].name, a[0].name);
        // never the player's own name
        let mine = a[0].name.clone();
        assert!(rival_companies("maps/Grundorf/global.cfg", 0, &mine.to_uppercase()).iter().all(|e| e.name != mine));
        assert_eq!(rival_drivers("maps/Grundorf/global.cfg", 10).len(), 8);
    }

    #[test]
    fn the_player_is_placed_among_them() {
        let mut c = found(&Founding { name: "Stadtbus Grundorf".into(), map: "maps/Grundorf/global.cfg".into(), date: "2024-05-06".into(), ..Default::default() }, "Luc");
        let t = company_table(&c);
        assert_eq!(t.len(), 8);
        assert_eq!(t.iter().map(|x| x.0).collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
        let start = place_of(&t).unwrap();
        // the town's big operator is ahead of a new company
        assert!(start > 1);
        c.progress.xp = 1_000_000;
        assert_eq!(place_of(&company_table(&c)), Some(1));
        // equal points: the player first
        let e = |name: &str, you: bool| Entry { name: name.into(), you, points: 10, level: 1, quality: 0.0, count: 0 };
        let r = rank(vec![e("B", false), e("A", false), e("Me", true)]);
        assert_eq!(r.iter().map(|x| x.1.name.as_str()).collect::<Vec<_>>(), ["Me", "A", "B"]);
    }
}
