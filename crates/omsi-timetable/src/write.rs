//! Writing `TTData` files the way OMSI 2's own editor lays them out: trips (`.ttp`), tracks
//! (`.ttr`), station links and bus stops. The line (`.ttl`) has its own writer in `lib.rs`.
//!
//! `StnLinks.cfg` and `Busstops.cfg` belong to the map; what openOMSI adds to them goes in a
//! block between two comment lines (`BLOCK_BEGIN` and `BLOCK_END`, which no reader takes for
//! a keyword), so that it can be written again or taken out without touching a line of the
//! map's own.

use crate::{BusStopEntry, StnLink, Track, Trip};
use std::path::Path;

/// The comment lines around what openOMSI adds to a map's `StnLinks.cfg` or `Busstops.cfg`.
pub const BLOCK_BEGIN: &str = "--- openOMSI line editor: begin (written again on every save) ---";
pub const BLOCK_END: &str = "--- openOMSI line editor: end ---";

/// A number as OMSI writes one: three decimals, no exponent.
fn num(v: f64) -> String {
    format!("{v:.3}")
}

impl Trip {
    /// The trip as OMSI writes a `.ttp`: the `[trip]` block (its track, the terminus, the
    /// line), every station as `[station_typ2]`, then every profile with its fixed times.
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        o.push_str("-----------------------\r\nTime Table Trip File\r\n-----------------------\r\n\r\nCreated with openOMSI\r\n\r\n");
        o.push_str(&format!("[trip]\r\n{}\r\n{}\r\n{}\r\n\r\n", self.display_name, self.terminus, self.line));
        if self.train_reverse {
            o.push_str("[trainreverse]\r\n\r\n");
        }
        for (i, id) in self.stations.iter().enumerate() {
            o.push_str(&format!("   {i}:\r\n[station_typ2]\r\n{id}\r\n\r\n"));
        }
        for p in &self.profiles {
            o.push_str(&format!("[profile]\r\n{}\r\n{}\r\n\r\n", p.name, num(p.factor as f64)));
            for (i, m) in &p.man_arr_time {
                o.push_str(&format!("[profile_man_arr_time]\r\n{i}\r\n{}\r\n\r\n", num(*m as f64)));
            }
            for (i, m) in &p.man_dep_time {
                o.push_str(&format!("[profile_man_dep_time]\r\n{i}\r\n{}\r\n\r\n", num(*m as f64)));
            }
            for (i, k) in &p.other_stopping {
                o.push_str(&format!("[profile_otherstopping]\r\n{i}\r\n{k}\r\n\r\n"));
            }
        }
        o
    }
}

impl Track {
    /// The track as OMSI writes a `.ttr`: one `[track_entry]` per road piece (id, path, tile
    /// index, the tile's path number, length, 0 - and a seventh value when the entry has one).
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        o.push_str("-----------------------\r\nTime Table Track File\r\n-----------------------\r\n\r\nCreated with openOMSI\r\n\r\n");
        for e in &self.entries {
            o.push_str("[track_entry]\r\n");
            for (k, v) in e.values.iter().enumerate() {
                // (the length and anything after it with decimals, the rest as the integers
                // they are)
                if k == 4 {
                    o.push_str(&num(*v));
                } else {
                    o.push_str(&format!("{}", *v as i64));
                }
                o.push_str("\r\n");
            }
            o.push_str("\r\n");
        }
        o
    }
}

impl StnLink {
    /// The link as `StnLinks.cfg` holds it, under a comment naming the two stops.
    pub fn to_text(&self, from: &str, to: &str) -> String {
        let mut o = format!("{from} ==> {to}\r\n[StnLink]\r\n{}\r\n{}\r\n{}\r\n", num(self.length), self.from_id, self.to_id);
        for (k, v) in self.params.iter().enumerate() {
            // (the last two are entry indices)
            if k >= 4 {
                o.push_str(&format!("{}\r\n", *v as i64));
            } else {
                o.push_str(&format!("{}\r\n", num(*v)));
            }
        }
        o.push_str("\r\n");
        for e in &self.entries {
            let v = &e.values;
            o.push_str(&format!("[StnLink_entry]\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n\r\n", v[0] as i64, v[1] as i64, v[2] as i64, num(v[3]), v[4] as i64, v[5] as i64, v[6] as i64));
        }
        o
    }
}

impl BusStopEntry {
    /// The stop as `Busstops.cfg` holds it.
    pub fn to_text(&self) -> String {
        format!("[busstop]\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n\r\n", self.name, self.group, self.object_id, num(self.params[0]), self.params[1] as i64, self.params[2] as i64)
    }
}

/// `text` written to `path` in the code page of the file it replaces (Windows-1252 for a new
/// one, as OMSI's own files are).
pub fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    let page = std::fs::read(path).map(|b| omsi_cfg::codepage::detect(&b)).unwrap_or(omsi_cfg::codepage::CodePage::Windows1252);
    let (bytes, _, _) = page.encoding().encode(text);
    std::fs::write(path, bytes)
}

/// `text` without openOMSI's block (see `BLOCK_BEGIN`), and with `block` as the new one at
/// its end when it is not empty. Everything outside the block stays as it was.
pub fn replace_block(text: &str, block: &str) -> String {
    let mut out = String::with_capacity(text.len() + block.len());
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t == BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if t == BLOCK_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
        }
    }
    if !block.trim().is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        out.push_str(&format!("\r\n{BLOCK_BEGIN}\r\n\r\n{block}{BLOCK_END}\r\n"));
    }
    out
}

/// `path`'s block (see `replace_block`) written anew, in the file's own code page; a file that
/// is not there is made when there is something to put in it.
pub fn update_block(path: &Path, block: &str) -> std::io::Result<()> {
    let old = match std::fs::read(path) {
        Ok(b) => {
            let page = omsi_cfg::codepage::detect(&b);
            page.encoding().decode(&b).0.into_owned()
        }
        Err(_) if block.trim().is_empty() => return Ok(()),
        Err(_) => String::new(),
    };
    let new = replace_block(&old, block);
    if new == old {
        return Ok(());
    }
    write_text(path, &new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_busstops, parse_stnlinks, StnLinkEntry, TrackEntry, TripProfile};
    use omsi_cfg::CfgFile;

    #[test]
    fn a_trip_reads_back() {
        let t = Trip {
            name: "oo_42_a".into(),
            display_name: "oo_42_a".into(),
            terminus: "Krankenhaus".into(),
            line: "42".into(),
            stations: vec![108, 104, 129],
            profiles: vec![TripProfile { name: "standard".into(), factor: 7.0, man_dep_time: vec![(1, 3.0)], ..Default::default() }],
            ..Default::default()
        };
        let back = Trip::parse(&CfgFile::from_str("oo_42_a.ttp", &t.to_text()));
        assert_eq!((back.display_name.as_str(), back.terminus.as_str(), back.line.as_str()), ("oo_42_a", "Krankenhaus", "42"));
        assert_eq!(back.stations, t.stations);
        assert_eq!(back.profiles, t.profiles);
    }

    #[test]
    fn a_track_and_a_link_read_back() {
        let dir = std::env::temp_dir().join(format!("omsi_tt_write_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tr = Track { path: dir.join("x.ttr"), entries: vec![TrackEntry { values: vec![99.0, 0.0, 4.0, 0.0, 55.0, 0.0] }, TrackEntry { values: vec![30.0, 1.0, 4.0, 0.0, 50.25, 0.0] }] };
        std::fs::write(dir.join("x.ttr"), tr.to_text()).unwrap();
        let back = Track::load(&dir.join("x.ttr")).unwrap();
        assert_eq!(back.entries, tr.entries);
        let link = StnLink { length: 272.0, from_id: 104, to_id: 129, params: [1.65, 3.37, 6.74, 34.9, 0.0, 1.0], entries: vec![StnLinkEntry { values: [99.0, 0.0, 4.0, 55.0, -1.0, 0.0, 0.0] }, StnLinkEntry { values: [30.0, 1.0, 4.0, 50.0, -1.0, 0.0, 0.0] }] };
        let links = parse_stnlinks(&CfgFile::from_str("StnLinks.cfg", &link.to_text("Bauernhof", "Nordspitze")));
        assert_eq!(links, vec![link]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_block_is_replaced_and_the_rest_kept() {
        let own = "[busstop]\r\nNordspitze\r\n3\r\n129\r\n0.0\r\n0\r\n0\r\n";
        let a = BusStopEntry { name: "Markt".into(), group: 4, object_id: 7, params: [0.0, 0.0, 0.0] };
        let b = BusStopEntry { name: "Kirche".into(), group: 4, object_id: 8, params: [0.0, 0.0, 0.0] };
        let once = replace_block(own, &a.to_text());
        let twice = replace_block(&once, &b.to_text());
        let stops = parse_busstops(&CfgFile::from_str("Busstops.cfg", &twice));
        assert_eq!(stops.iter().map(|s| s.object_id).collect::<Vec<_>>(), vec![129, 8]);
        // an empty block takes ours out and leaves the map's as it was
        assert_eq!(replace_block(&twice, "").trim_end(), own.trim_end());
    }
}
