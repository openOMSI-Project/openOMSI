//! Where the launcher writes a map's timetable. The game reads one `TTData` folder per map -
//! the content folder's when it has one, else the map's own (`omsi_cfg::resolve_path`) - so a
//! map of the original installation gets its `TTData` copied into the content folder whole
//! before anything is written there, and the original stays as it is. A map that lies
//! unpacked in the content folder is written in place, every file it had kept as
//! `<file>.orig` the first time it is written over.
//!
//! The Timetable page saves lines here; the line editor writes its lines here too
//! (`lines::export_to_map`), and the reset puts the map's own timetable back.

use std::path::{Path, PathBuf};

/// Marks a `TTData` folder the launcher copied into the content folder: the reset deletes
/// such a copy, and only such a one.
pub const COPY_MARK: &str = ".openomsi-ttdata-copy";

/// The content folder's copy of a map's `TTData`.
pub fn copy_dir(content: &Path, map_folder: &str) -> PathBuf {
    content.join("maps").join(map_folder).join("TTData")
}

/// The map lies unpacked in the content folder (a map inside a mod archive does not: its
/// path is no folder that can be written).
fn in_content(content: &Path, path: &Path) -> bool {
    path.starts_with(content) && omsi_cfg::vfs::archive_of(path).is_none()
}

/// The content folder's copy of `original_ttdata`, made whole when it is not there yet.
fn make_copy(content: &Path, map_folder: &str, original_ttdata: &Path) -> Result<PathBuf, String> {
    let dir = copy_dir(content, map_folder);
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for (name, is_dir) in omsi_cfg::vfs::list_dir(original_ttdata).ok_or_else(|| format!("{} cannot be read", original_ttdata.display()))? {
            if !is_dir {
                let bytes = omsi_cfg::vfs::read(&original_ttdata.join(&name)).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(dir.join(COPY_MARK), b"TTData copied by the openOMSI launcher's timetable editor; its reset deletes this folder\n").map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// `<file>.orig` beside a file of a map in the content folder, the first time it is written
/// over (nothing in a copy the launcher made: the reset deletes that whole).
pub fn keep_original(path: &Path) -> Result<(), String> {
    let orig = PathBuf::from(format!("{}.orig", path.display()));
    let copied = path.parent().is_some_and(|d| d.join(COPY_MARK).is_file());
    if !copied && path.is_file() && !orig.exists() {
        std::fs::copy(path, &orig).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Where a map's line is written (`line_path` where it was read, `file` its file name): in
/// place when the map lies unpacked in the content folder (backed up as `<file>.orig` the
/// first time), else in the content folder's copy of its `TTData`.
pub fn save_target(content: &Path, line_path: &Path, file: &str, map_folder: &str, original_ttdata: &Path) -> Result<PathBuf, String> {
    if in_content(content, line_path) {
        keep_original(line_path)?;
        return Ok(line_path.to_path_buf());
    }
    Ok(make_copy(content, map_folder, original_ttdata)?.join(file))
}

/// The `TTData` folder the launcher writes a map's files into: the map's own when it lies
/// unpacked in the content folder, else the content folder's copy (made whole first).
pub fn ttdata_dir(content: &Path, map_dir: &Path, map_folder: &str) -> Result<PathBuf, String> {
    if in_content(content, map_dir) {
        let own = map_dir.join("TTData");
        std::fs::create_dir_all(&own).map_err(|e| e.to_string())?;
        return Ok(own);
    }
    let copy = copy_dir(content, map_folder);
    if copy.is_dir() {
        return Ok(copy);
    }
    make_copy(content, map_folder, &omsi_cfg::resolve_path(map_dir, "TTData"))
}

/// The map's own timetable back: the launcher's copy of its `TTData` deleted, or (a map
/// unpacked in the content folder) every file saved over put back from its `.orig`.
/// Returns what was done.
pub fn reset_timetable(content: &Path, map_dir: &Path, map_folder: &str) -> Result<String, String> {
    let copy = copy_dir(content, map_folder);
    if copy.join(COPY_MARK).is_file() {
        std::fs::remove_dir_all(&copy).map_err(|e| e.to_string())?;
        return Ok(format!("The timetable of {map_folder} is the map's own again (the edited copy was removed)"));
    }
    let own = map_dir.join("TTData");
    let mut restored = 0;
    if own.is_dir() {
        for e in std::fs::read_dir(&own).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if let Some(orig) = p.to_str().and_then(|s| s.strip_suffix(".orig")) {
                std::fs::rename(&p, orig).map_err(|e| e.to_string())?;
                restored += 1;
            }
        }
    }
    if restored > 0 {
        Ok(format!("{restored} line(s) of {map_folder} put back as they were"))
    } else {
        Err(format!("The timetable of {map_folder} has not been changed here"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_of_the_installation_is_copied_whole_and_reset() {
        let base = std::env::temp_dir().join(format!("omsi_ttstore_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (content, omsi) = (base.join("content"), base.join("omsi"));
        let map = omsi.join("maps").join("Dorf");
        std::fs::create_dir_all(map.join("TTData")).unwrap();
        std::fs::write(map.join("TTData").join("1.ttl"), "[priority]\r\n1\r\n").unwrap();
        std::fs::write(map.join("TTData").join("1_a.ttp"), "[trip]\r\n\r\nX\r\n1\r\n").unwrap();
        let dir = ttdata_dir(&content, &map, "Dorf").unwrap();
        assert_eq!(dir, copy_dir(&content, "Dorf"));
        assert!(dir.join("1.ttl").is_file() && dir.join("1_a.ttp").is_file() && dir.join(COPY_MARK).is_file());
        // a line goes into that copy
        assert_eq!(save_target(&content, &map.join("TTData").join("1.ttl"), "1.ttl", "Dorf", &map.join("TTData")).unwrap(), dir.join("1.ttl"));
        assert!(reset_timetable(&content, &map, "Dorf").is_ok());
        assert!(!dir.exists());
        // the original was never touched
        assert!(map.join("TTData").join("1.ttl").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_map_in_the_content_folder_is_written_in_place() {
        let base = std::env::temp_dir().join(format!("omsi_ttstore_in_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let content = base.join("content");
        let map = content.join("maps").join("Mod");
        std::fs::create_dir_all(map.join("TTData")).unwrap();
        let line = map.join("TTData").join("5.ttl");
        std::fs::write(&line, "own").unwrap();
        assert_eq!(ttdata_dir(&content, &map, "Mod").unwrap(), map.join("TTData"));
        assert_eq!(save_target(&content, &line, "5.ttl", "Mod", &map.join("TTData")).unwrap(), line);
        std::fs::write(&line, "changed").unwrap();
        assert!(reset_timetable(&content, &map, "Mod").is_ok());
        assert_eq!(std::fs::read_to_string(&line).unwrap(), "own");
        let _ = std::fs::remove_dir_all(&base);
    }
}
