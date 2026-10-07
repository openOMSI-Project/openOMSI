//! The bus company's welcome and tutorial (Luc: "een welkomstscherm voor de busbedrijfmodus,
//! met een tutorial"): the first time a driver opens the company, its tour starts by itself -
//! a welcome card with what the mode is, then (once a company runs) its pages one by one, each
//! with its part lit (`tour::start_company`). It can be skipped; where it was left is kept per
//! driver in `~/.openomsi/company-tour.json`, and the "?" in the company's bar goes on from
//! there (or from the start, once it was seen to the end). A tour that only showed the welcome
//! and the founding goes on through the pages by itself once the company is founded.

use super::super::Launcher;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a driver has seen of the company's tour.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Seen {
    /// It was shown (the welcome at least).
    pub seen: bool,
    /// Where it was left when it was skipped (its stop, 0: the start).
    pub at: usize,
    /// Gone through to its end.
    pub done: bool,
    /// The welcome was shown before there was a company: its pages are still to come.
    pub pages_to_come: bool,
}

fn file() -> PathBuf {
    omsi_launcher_lib::data_dir().join("company-tour.json")
}

fn read_all(path: &Path) -> BTreeMap<String, Seen> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_all(path: &Path, all: &BTreeMap<String, Seen>) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(path, serde_json::to_vec_pretty(all).unwrap_or_default()) {
        log::warn!("company tour: not kept: {e}");
    }
}

/// What a driver has seen (nothing yet: the default).
pub(in crate::launcher) fn of(profile: &str) -> Seen {
    read_all(&file()).get(profile.trim()).cloned().unwrap_or_default()
}

fn keep(profile: &str, s: Seen) {
    let path = file();
    let mut all = read_all(&path);
    all.insert(profile.trim().to_string(), s);
    write_all(&path, &all);
}

/// The tour was left at its stop `at`: gone through (`completed`) or skipped.
pub(in crate::launcher) fn left(profile: &str, at: usize, completed: bool) {
    let mut s = of(profile);
    s.seen = true;
    if completed {
        // (a tour of the welcome and the founding alone is not the pages')
        if !s.pages_to_come {
            s.done = true;
        }
        s.at = 0;
    } else {
        s.at = at;
    }
    keep(profile, s);
}

/// What starting the tour now means for a driver who has seen `s`, with a company running or
/// not: (start it by itself, at which stop, the pages still to come after it).
pub fn on_open(s: &Seen, company: bool) -> Option<(usize, bool)> {
    if !s.seen {
        return Some((0, !company));
    }
    if s.pages_to_come && company {
        // (the welcome was seen: on with the pages)
        return Some((1, false));
    }
    None
}

/// Where the "?" starts the tour: where it was left, else from the start.
pub fn resume_at(s: &Seen) -> usize {
    if s.done {
        0
    } else {
        s.at
    }
}

/// Once a frame on the company's screen: the tour started by itself the first time (not on a
/// phone, not over a dialog, not while another tour runs).
pub(super) fn frame(l: &mut Launcher) {
    if super::super::mobile::mobile() || super::super::tour::active(l) || l.company.companies.is_none() || l.company.dialog.is_some() {
        return;
    }
    let profile = l.state.config.profile.clone();
    if l.company.tutorial_for.as_deref() == Some(profile.as_str()) {
        return;
    }
    l.company.tutorial_for = Some(profile.clone());
    let company = l.company.company.is_some() && l.company.wizard.is_none();
    let mut s = of(&profile);
    let Some((at, to_come)) = on_open(&s, company) else { return };
    s.seen = true;
    s.pages_to_come = to_come;
    keep(&profile, s);
    super::super::tour::start_company(l, at);
}

/// A company was founded: the tour may go on with its pages (looked at again next frame).
pub(super) fn founded(l: &mut Launcher) {
    l.company.tutorial_for = None;
}

/// The "?" in the company's bar: the tour from where it was left.
pub(super) fn ask(l: &mut Launcher) {
    let profile = l.state.config.profile.clone();
    let s = of(&profile);
    super::super::tour::start_company(l, resume_at(&s));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tour_starts_once_and_goes_on_where_it_was_left() {
        // a driver new to the company: the welcome (and the founding, without a company)
        let new = Seen::default();
        assert_eq!(on_open(&new, false), Some((0, true)));
        assert_eq!(on_open(&new, true), Some((0, false)));
        // seen, the company founded since: on with the pages, past the welcome
        let welcomed = Seen { seen: true, pages_to_come: true, ..Default::default() };
        assert_eq!(on_open(&welcomed, false), None);
        assert_eq!(on_open(&welcomed, true), Some((1, false)));
        // seen and skipped: not by itself again; the "?" goes on where it was left
        let skipped = Seen { seen: true, at: 5, ..Default::default() };
        assert_eq!(on_open(&skipped, true), None);
        assert_eq!(resume_at(&skipped), 5);
        // seen to the end: the "?" from the start
        assert_eq!(resume_at(&Seen { seen: true, at: 5, done: true, ..Default::default() }), 0);
    }

    #[test]
    fn what_was_seen_is_kept_per_driver() {
        let dir = std::env::temp_dir().join(format!("omsi_company_tour_{}", std::process::id()));
        let path = dir.join("company-tour.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut all = read_all(&path);
        assert!(all.is_empty());
        all.insert("Luc".into(), Seen { seen: true, at: 3, ..Default::default() });
        write_all(&path, &all);
        let back = read_all(&path);
        assert_eq!(back.get("Luc").map(|s| s.at), Some(3));
        assert!(!back.contains_key("Anna"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
