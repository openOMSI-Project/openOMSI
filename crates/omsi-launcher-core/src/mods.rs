//! The mods in the content folder, one by one: what each install put there (its own folders,
//! or the archive it is read from), switched off and on, and removed.
//!
//! Every install the launcher makes is noted in `Mods/.mods.json` with the folders it made
//! (a bus's own folder, a map's) or the archive used in place. What lies in the content folder
//! without such a note - put there by hand, or installed before the notes - is listed as well,
//! one entry per bus, map, other folder and archive, so that anything can be switched off or
//! removed by itself. Switched off, a mod's folders move to `Mods/disabled/<id>` (the game
//! and the lists no longer see them) and back again when it is switched on; removed, they are
//! deleted. The original OMSI 2 folder is never touched: only the content folder holds mods.

use crate::install::{self, ARCHIVES};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The notes of the installs, in `<content>/Mods`.
pub const REGISTRY: &str = ".mods.json";
/// Where a switched-off mod's folders wait, in `<content>/Mods`.
pub const DISABLED: &str = "disabled";

/// What a mod is, by what it holds first.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Bus,
    Map,
    Archive,
    #[default]
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mod {
    /// Its name in `Mods/disabled` and in the notes (a file name).
    pub id: String,
    /// What it is called: the folder or archive it was installed from.
    pub name: String,
    /// What it owns, content-relative with '/': `Vehicles/MAN_SD202`, `Archives/x.zip`.
    pub paths: Vec<String>,
    pub enabled: bool,
    /// When it was installed (seconds since 1970; 0: not known).
    #[serde(default)]
    pub installed: u64,
    /// Bytes on the disk (worked out when listed).
    #[serde(skip_deserializing, default)]
    pub bytes: u64,
    #[serde(skip_deserializing, default)]
    pub kind: Kind,
    /// Noted by an install (else found in the content folder without a note).
    #[serde(skip_deserializing, default)]
    pub noted: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Registry {
    mods: Vec<Mod>,
}

fn registry_file(content: &Path) -> PathBuf {
    content.join("Mods").join(REGISTRY)
}

fn load(content: &Path) -> Registry {
    std::fs::read(registry_file(content)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(content: &Path, r: &Registry) -> anyhow::Result<()> {
    let f = registry_file(content);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(r)?)?;
    std::fs::rename(&tmp, &f)?;
    Ok(())
}

/// A file name of `name`, not yet taken by another mod.
fn new_id(r: &Registry, name: &str) -> String {
    let base: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect::<String>().trim().trim_matches('.').to_string();
    let base = if base.is_empty() { "mod".to_string() } else { base };
    let mut id = base.clone();
    let mut k = 2;
    while r.mods.iter().any(|m| m.id.eq_ignore_ascii_case(&id)) {
        id = format!("{base} ({k})");
        k += 1;
    }
    id
}

fn kind_of(paths: &[String]) -> Kind {
    let top = |p: &String| p.split('/').next().unwrap_or("").to_ascii_lowercase();
    if paths.iter().any(|p| top(p) == ARCHIVES.to_ascii_lowercase()) {
        Kind::Archive
    } else if paths.iter().any(|p| top(p) == "maps") {
        Kind::Map
    } else if paths.iter().any(|p| top(p) == "vehicles") {
        Kind::Bus
    } else {
        Kind::Other
    }
}

/// Note an install: `name` put `paths` (the folders it made, or its archive) into the content
/// folder. An earlier note of the same name gains them (installed again).
pub fn record(content: &Path, name: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let mut r = load(content);
    match r.mods.iter_mut().find(|m| m.name.eq_ignore_ascii_case(name)) {
        Some(m) => {
            for p in paths {
                if !m.paths.iter().any(|x| x.eq_ignore_ascii_case(p)) {
                    m.paths.push(p.clone());
                }
            }
            m.enabled = true;
            m.installed = install::now_secs();
        }
        None => {
            let id = new_id(&r, name);
            r.mods.push(Mod { id, name: name.to_string(), paths: paths.to_vec(), enabled: true, installed: install::now_secs(), bytes: 0, kind: Kind::Other, noted: true });
        }
    }
    if let Err(e) = save(content, &r) {
        crate::log_to_file(&format!("mods: the note of {name} was not written: {e:#}"));
    }
}

fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn disabled_dir(content: &Path, id: &str) -> PathBuf {
    content.join("Mods").join(DISABLED).join(id)
}

/// Every mod of the content folder: the noted ones (switched on or off), then what lies there
/// without a note, one entry per bus, map, other folder and archive.
pub fn list(content: &Path) -> Vec<Mod> {
    let mut r = load(content);
    import_records(content, &mut r);
    // (a note whose folders are gone everywhere - deleted by hand - goes too)
    let before = r.mods.len();
    r.mods.retain(|m| m.paths.iter().any(|p| content.join(p).exists() || disabled_dir(content, &m.id).join(p).exists()));
    if r.mods.len() != before {
        let _ = save(content, &r);
    }
    let mut out = Vec::new();
    for m in &r.mods {
        let mut m = m.clone();
        let at = |p: &String| if m.enabled { content.join(p) } else { disabled_dir(content, &m.id).join(p) };
        m.bytes = m.paths.iter().map(|p| size_of(&at(p))).sum();
        m.kind = kind_of(&m.paths);
        m.noted = true;
        out.push(m);
    }
    let claimed: Vec<String> = r.mods.iter().flat_map(|m| m.paths.iter().map(|p| p.to_ascii_lowercase())).collect();
    let is_claimed = |rel: &str| {
        let rel = rel.to_ascii_lowercase();
        claimed.iter().any(|c| *c == rel || rel.starts_with(&format!("{c}/")) || c.starts_with(&format!("{rel}/")))
    };
    let mut found = Vec::new();
    for folder in omsi_cfg::CONTENT_FOLDERS {
        let Ok(rd) = std::fs::read_dir(content.join(folder)) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            let rel = format!("{folder}/{name}");
            if is_claimed(&rel) {
                continue;
            }
            found.push((name, rel));
        }
    }
    if let Ok(rd) = std::fs::read_dir(content.join(ARCHIVES)) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let rel = format!("{ARCHIVES}/{name}");
            if !name.starts_with('.') && e.path().is_file() && !is_claimed(&rel) {
                found.push((name, rel));
            }
        }
    }
    for (name, rel) in found {
        let paths = vec![rel.clone()];
        out.push(Mod { id: format!("found:{rel}"), name, bytes: size_of(&content.join(&rel)), kind: kind_of(&paths), paths, enabled: true, installed: 0, noted: false });
    }
    out
}

/// The notes of the inbox installs made before `Mods/.mods.json` (`Mods/.installed-records`)
/// taken over.
fn import_records(content: &Path, r: &mut Registry) {
    let dir = content.join("Mods").join(install::RECORDS);
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let mut changed = false;
    for e in rd.flatten() {
        let Some(name) = e.file_name().to_string_lossy().strip_suffix(".txt").map(str::to_string) else { continue };
        if r.mods.iter().any(|m| m.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let paths: Vec<String> = std::fs::read_to_string(e.path()).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && content.join(l).exists()).collect();
        if paths.is_empty() {
            continue;
        }
        let id = new_id(r, &name);
        r.mods.push(Mod { id, name, paths, enabled: true, installed: 0, bytes: 0, kind: Kind::Other, noted: true });
        changed = true;
    }
    if changed {
        let _ = save(content, r);
    }
}

/// The note of `id` (a mod found without one gets one now).
fn take(content: &Path, r: &mut Registry, id: &str) -> anyhow::Result<usize> {
    if let Some(k) = r.mods.iter().position(|m| m.id == id) {
        return Ok(k);
    }
    let rel = id.strip_prefix("found:").ok_or_else(|| anyhow::anyhow!("no mod {id}"))?;
    if install::check_rel_pub(rel).is_err() || !content.join(rel).exists() {
        anyhow::bail!("{rel} is not in the content folder");
    }
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let new = new_id(r, &name);
    r.mods.push(Mod { id: new, name, paths: vec![rel.to_string()], enabled: true, installed: 0, bytes: 0, kind: Kind::Other, noted: true });
    Ok(r.mods.len() - 1)
}

/// Whether an install runs (it may be moving folders this would move too). (The tests of the
/// installer run their installs beside these, in other folders.)
fn installing() -> bool {
    !cfg!(test) && install::jobs().iter().any(|j| j.finished.is_none())
}

fn move_path(from: &Path, to: &Path) -> anyhow::Result<()> {
    if !from.exists() {
        return Ok(());
    }
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    if to.exists() {
        anyhow::bail!("{} is in the way", to.display());
    }
    std::fs::rename(from, to).map_err(|e| anyhow::anyhow!("moving {} to {}: {e}", from.display(), to.display()))
}

/// The archives among `paths` no longer read by the lists (switched off or removed).
fn unmount(content: &Path, paths: &[String]) {
    for p in paths.iter().filter(|p| p.to_ascii_lowercase().starts_with(&format!("{}/", ARCHIVES.to_ascii_lowercase()))) {
        let zip = content.join(p);
        for root in omsi_cfg::content_roots().into_iter().filter(|r| r.starts_with(&zip)) {
            omsi_cfg::remove_content_root(&root);
        }
    }
}

/// Switch the mod `id` off (its folders into `Mods/disabled/<id>`) or on (back again).
pub fn set_enabled(content: &Path, id: &str, on: bool) -> anyhow::Result<String> {
    if installing() {
        anyhow::bail!("an install is under way: wait for it to end");
    }
    let mut r = load(content);
    let k = take(content, &mut r, id)?;
    let m = r.mods[k].clone();
    if m.enabled == on {
        return Ok(m.name);
    }
    let off = disabled_dir(content, &m.id);
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    if !on {
        unmount(content, &m.paths);
    }
    for p in &m.paths {
        let (from, to) = if on { (off.join(p), content.join(p)) } else { (content.join(p), off.join(p)) };
        if let Err(e) = move_path(&from, &to) {
            // (what was moved goes back: a mod is either on or off, never half)
            for (a, b) in moved.into_iter().rev() {
                let _ = std::fs::rename(&b, &a);
            }
            return Err(e);
        }
        moved.push((from, to));
    }
    if on {
        for p in m.paths.iter().filter(|p| p.to_ascii_lowercase().starts_with(&format!("{}/", ARCHIVES.to_ascii_lowercase()))) {
            crate::mount_archive(&content.join(p));
        }
        let _ = std::fs::remove_dir_all(&off);
    }
    r.mods[k].enabled = on;
    save(content, &r)?;
    omsi_cfg::content_changed();
    Ok(m.name)
}

/// Delete the mod `id`: its folders (wherever they are, on or off), its note, and what its
/// inbox install left in `Mods/installed`.
pub fn remove(content: &Path, id: &str) -> anyhow::Result<String> {
    if installing() {
        anyhow::bail!("an install is under way: wait for it to end");
    }
    let mut r = load(content);
    let k = take(content, &mut r, id)?;
    let m = r.mods.remove(k);
    unmount(content, &m.paths);
    for p in &m.paths {
        install::check_rel_pub(p)?;
        for at in [content.join(p), disabled_dir(content, &m.id).join(p)] {
            if at.is_dir() {
                std::fs::remove_dir_all(&at).map_err(|e| anyhow::anyhow!("deleting {}: {e}", at.display()))?;
            } else if at.exists() {
                std::fs::remove_file(&at).map_err(|e| anyhow::anyhow!("deleting {}: {e}", at.display()))?;
            }
        }
    }
    let _ = std::fs::remove_dir_all(disabled_dir(content, &m.id));
    // (an inbox install's leftovers: gone with it, so it is not taken for a new install)
    let installed = content.join("Mods").join("installed").join(&m.name);
    let _ = std::fs::remove_dir_all(&installed);
    let _ = std::fs::remove_file(&installed);
    let _ = std::fs::remove_file(content.join("Mods").join(install::RECORDS).join(format!("{}.txt", m.name)));
    save(content, &r)?;
    omsi_cfg::content_changed();
    Ok(m.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("openomsi_mods_{}_{n}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(d.join("Vehicles/Big Bus")).unwrap();
        std::fs::write(d.join("Vehicles/Big Bus/big.bus"), b"[name]").unwrap();
        std::fs::create_dir_all(d.join("maps/Hill Town")).unwrap();
        std::fs::write(d.join("maps/Hill Town/global.cfg"), b"x").unwrap();
        std::fs::create_dir_all(d.join("Vehicles/Hand Made")).unwrap();
        d
    }

    #[test]
    fn an_install_is_noted_and_the_rest_is_found_one_by_one() {
        let c = content();
        record(&c, "BigBus_v2.zip", &["Vehicles/Big Bus".into()]);
        let l = list(&c);
        let noted: Vec<_> = l.iter().filter(|m| m.noted).collect();
        assert_eq!(noted.len(), 1);
        assert_eq!(noted[0].name, "BigBus_v2.zip");
        assert_eq!(noted[0].kind, Kind::Bus);
        assert!(noted[0].bytes > 0);
        let found: Vec<&str> = l.iter().filter(|m| !m.noted).map(|m| m.paths[0].as_str()).collect();
        assert!(found.contains(&"maps/Hill Town") && found.contains(&"Vehicles/Hand Made"));
        assert!(!found.contains(&"Vehicles/Big Bus"));
        std::fs::remove_dir_all(c).unwrap();
    }

    #[test]
    fn a_mod_switched_off_leaves_the_content_folder_and_comes_back() {
        let c = content();
        record(&c, "Big Bus", &["Vehicles/Big Bus".into()]);
        let id = list(&c).into_iter().find(|m| m.noted).unwrap().id;
        set_enabled(&c, &id, false).unwrap();
        assert!(!c.join("Vehicles/Big Bus").exists());
        assert!(c.join("Mods/disabled").join(&id).join("Vehicles/Big Bus/big.bus").exists());
        let m = list(&c).into_iter().find(|m| m.id == id).unwrap();
        assert!(!m.enabled && m.bytes > 0);
        set_enabled(&c, &id, true).unwrap();
        assert!(c.join("Vehicles/Big Bus/big.bus").exists());
        assert!(!c.join("Mods/disabled").join(&id).exists());
        std::fs::remove_dir_all(c).unwrap();
    }

    /// A switched-off mod waits in `Mods/disabled`: the Mods inbox, which installs what is
    /// put into `Mods`, must not take it for a new mod (it put it straight back).
    #[test]
    fn a_switched_off_mod_is_not_installed_again_from_the_inbox() {
        let c = content();
        let id = list(&c).into_iter().find(|m| m.paths == ["Vehicles/Big Bus"]).unwrap().id;
        set_enabled(&c, &id, false).unwrap();
        assert!(c.join("Mods").join(DISABLED).is_dir());
        assert!(crate::inbox_entries(&c).is_empty(), "{:?}", crate::inbox_entries(&c));
        std::fs::remove_dir_all(c).unwrap();
    }

    #[test]
    fn a_found_folder_is_switched_off_and_removed_by_itself() {
        let c = content();
        let found = list(&c).into_iter().find(|m| m.paths == ["maps/Hill Town"]).unwrap();
        set_enabled(&c, &found.id, false).unwrap();
        assert!(!c.join("maps/Hill Town").exists());
        let noted = list(&c).into_iter().find(|m| m.name == "Hill Town").unwrap();
        assert!(noted.noted && !noted.enabled);
        remove(&c, &noted.id).unwrap();
        assert!(!c.join("Mods/disabled").join(&noted.id).exists());
        assert!(list(&c).iter().all(|m| m.name != "Hill Town"));
        // the others are untouched
        assert!(c.join("Vehicles/Big Bus/big.bus").exists());
        std::fs::remove_dir_all(c).unwrap();
    }
}
