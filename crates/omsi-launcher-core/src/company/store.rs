//! Companies on the disk: `~/.openomsi/companies/<id>.json`, one file each, with the file's
//! version (an older file is brought up to date when it is read). Written whole to a
//! temporary file first and then moved over the old one: a launcher closed in the middle of
//! writing leaves the company as it was.
//!
//! The live hook's file lies beside it (`<id>.live.jsonl`): the game appends what the
//! company's buses did while it ran (`append_live`), the next close takes it in.

use super::day::{self, DayReport, LiveEvent};
use super::model::{Company, VERSION};
use super::network;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn dir(data: &Path) -> PathBuf {
    data.join("companies")
}

pub fn path_of(data: &Path, id: &str) -> PathBuf {
    dir(data).join(format!("{id}.json"))
}

/// An id for a new company of that name that no file has yet.
pub fn unused_id(data: &Path, name: &str) -> String {
    let base = super::slug(name);
    let mut id = base.clone();
    let mut n = 2;
    while path_of(data, &id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A file of an older version brought up to date.
pub fn migrate(mut c: Company) -> Company {
    // (an older file ran every line: those its roster plans run on, the others wait for the
    // planning - a line runs only once it is planned)
    for k in 0..c.lines.len() {
        if c.lines[k].service_from == Some(super::model::LEGACY_SERVICE) {
            let name = c.lines[k].name.clone();
            let planned = c.planning.week.iter().any(|r| r.line.eq_ignore_ascii_case(&name) && (r.bus.is_some() || r.duties.iter().any(Option::is_some)));
            c.lines[k].service_from = planned.then_some(0);
        }
    }
    if c.contract_index <= 0.0 {
        c.contract_index = 1.0;
    }
    if c.price_index <= 0.0 {
        c.price_index = 1.0;
    }
    // (a company from before the licences: its people drive what they drove)
    if c.quals == 0 {
        super::licences::grant_fleet(&mut c);
        c.quals = 1;
    }
    c.version = VERSION;
    c
}

pub fn save(data: &Path, c: &Company) -> Result<()> {
    let d = dir(data);
    std::fs::create_dir_all(&d).with_context(|| format!("cannot make {}", d.display()))?;
    let path = path_of(data, &c.id);
    let tmp = d.join(format!("{}.json.tmp", c.id));
    std::fs::write(&tmp, serde_json::to_string_pretty(c)?).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

pub fn load(data: &Path, id: &str) -> Result<Company> {
    let path = path_of(data, id);
    let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let c: Company = serde_json::from_str(&text).with_context(|| format!("{} is not a company", path.display()))?;
    Ok(migrate(c))
}

/// The companies of a driver (all of them for an empty name), by name.
pub fn list(data: &Path, profile: &str) -> Vec<Company> {
    let Ok(rd) = std::fs::read_dir(dir(data)) else { return Vec::new() };
    let mut out: Vec<Company> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let id = n.strip_suffix(".json")?;
            load(data, id).ok()
        })
        .filter(|c| profile.trim().is_empty() || c.profile.trim().eq_ignore_ascii_case(profile.trim()))
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn delete(data: &Path, id: &str) -> Result<()> {
    let _ = std::fs::remove_file(live_file(data, id));
    std::fs::remove_file(path_of(data, id)).with_context(|| format!("cannot delete {id}"))
}

/// The live hook's file of a company.
pub fn live_file(data: &Path, id: &str) -> PathBuf {
    dir(data).join(format!("{id}.live.jsonl"))
}

/// The game reports an event of the company (a line to the live file).
pub fn append_live(data: &Path, id: &str, ev: &LiveEvent) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir(data))?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(live_file(data, id))?;
    writeln!(f, "{}", serde_json::to_string(ev)?)?;
    Ok(())
}

/// Take what the game reported into the company (and empty the file).
pub fn take_live(data: &Path, c: &mut Company) {
    let path = live_file(data, &c.id);
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    for ev in text.lines().filter_map(|l| serde_json::from_str::<LiveEvent>(l).ok()) {
        day::record_live(c, ev);
    }
    let _ = std::fs::remove_file(&path);
}

/// The company's tours of its current day, from the map's timetable of that date.
pub fn tours_today(c: &Company) -> Result<(Vec<crate::LineInfo>, Vec<network::TourOfDay>)> {
    let lines = crate::list_lines(&c.map, &c.date)?;
    let tours = network::tours_of_day(c, &lines, &c.date);
    Ok((lines, tours))
}

/// Close the company's day: the timetable of its date, the player's trip reports, what the
/// game reported live; saved afterwards.
pub fn close_day(data: &Path, c: &mut Company) -> Result<DayReport> {
    take_live(data, c);
    let (lines, tours) = tours_today(c)?;
    network::refresh_lines(c, &lines);
    let trips = crate::trips_of(data, &c.profile);
    let report = day::close_day(c, tours, &trips);
    let report = super::depot::after_close(c, report, &lines);
    save(data, c)?;
    Ok(report)
}

/// What the game reported of the company since the day's last close, without taking it in
/// (the clock tells it as it comes, the day's close books it): the events after the first
/// `seen`.
pub fn peek_live(data: &Path, id: &str, seen: usize) -> Vec<LiveEvent> {
    let Ok(text) = std::fs::read_to_string(live_file(data, id)) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str::<LiveEvent>(l).ok()).skip(seen).collect()
}

/// The company's world on the disk, for its clock: the map's timetable of a date (read once
/// per date), and the night as `close_day` runs it (saved by the caller).
pub struct Disk<'a> {
    data: &'a Path,
    read: Vec<(String, String, Vec<crate::LineInfo>)>,
}

impl<'a> Disk<'a> {
    pub fn new(data: &'a Path) -> Self {
        Disk { data, read: Vec::new() }
    }

    /// The timetable of a date read already (the pages have the company's day).
    pub fn knowing(mut self, map: &str, date: &str, lines: Vec<crate::LineInfo>) -> Self {
        self.read.push((map.to_string(), date.to_string(), lines));
        self
    }
}

impl super::clock::World for Disk<'_> {
    fn lines(&mut self, c: &Company, date: &str) -> std::result::Result<Vec<crate::LineInfo>, String> {
        if let Some(x) = self.read.iter().find(|x| x.0 == c.map && x.1 == date) {
            return Ok(x.2.clone());
        }
        let lines = crate::list_lines(&c.map, date).map_err(|e| format!("{e:#}"))?;
        // (the last few days are enough)
        if self.read.len() > 3 {
            self.read.remove(0);
        }
        self.read.push((c.map.clone(), date.to_string(), lines.clone()));
        Ok(lines)
    }

    fn close_day(&mut self, c: &mut Company, lines: &[crate::LineInfo]) -> std::result::Result<DayReport, String> {
        take_live(self.data, c);
        network::refresh_lines(c, lines);
        let tours = network::tours_of_day(c, lines, &c.date);
        let trips = crate::trips_of(self.data, &c.profile);
        let report = day::close_day(c, tours, &trips);
        Ok(super::depot::after_close(c, report, lines))
    }
}

/// Simulate the company until `to` (the clock's minutes, `clock::target`) and save it.
pub fn simulate(data: &Path, c: &mut Company, to: i64, quick: bool, w: Option<Disk>) -> Result<super::clock::Run> {
    let mut w = w.unwrap_or_else(|| Disk::new(data));
    let run = super::clock::advance(c, to, &mut w, quick).map_err(|e| anyhow::anyhow!(e))?;
    save(data, c)?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("openomsi-company-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_company_is_saved_read_and_listed() {
        let data = temp("store");
        let mut c = found(&Founding { name: "Stadtbus".into(), ..Default::default() }, "Luc");
        c.id = unused_id(&data, &c.name);
        save(&data, &c).unwrap();
        assert_eq!(load(&data, &c.id).unwrap(), c);
        // a second of the same name gets another file
        let id2 = unused_id(&data, "Stadtbus");
        assert_eq!(id2, "stadtbus-2");
        let mut d = found(&Founding { name: "Anderer".into(), ..Default::default() }, "Anna");
        d.id = id2;
        save(&data, &d).unwrap();
        assert_eq!(list(&data, "luc").iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["Stadtbus"]);
        assert_eq!(list(&data, "").len(), 2);
        // the live hook's file is taken in once
        append_live(&data, &c.id, &LiveEvent::Breakdown { vehicle: 3 }).unwrap();
        take_live(&data, &mut c);
        assert_eq!(c.live, vec![LiveEvent::Breakdown { vehicle: 3 }]);
        take_live(&data, &mut c);
        assert_eq!(c.live.len(), 1);
        delete(&data, &d.id).unwrap();
        assert_eq!(list(&data, "").len(), 1);
        // an older file without the newer fields still reads
        let old = r##"{"version":0,"id":"alt","profile":"Luc","name":"Alt","short":"A","colours":["#fff","#000"],"map":"","map_name":"","depot":"","founded":"2024-01-01","date":"2024-01-01","difficulty":"easy","cash":5,"reputation":50,"punctuality":90,"price_index":1,"ledger":[]}"##;
        std::fs::write(path_of(&data, "alt"), old).unwrap();
        let a = load(&data, "alt").unwrap();
        assert_eq!((a.version, a.contract_index, a.cash), (VERSION, 1.0, 5));
        let _ = std::fs::remove_dir_all(&data);
    }
}
