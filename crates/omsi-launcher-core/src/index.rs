//! The content index: what the lists of maps, buses and weathers are built from, cached
//! per folder and keyed by modification times, so that the lists can be asked for again
//! whenever something changes without reading every `.bus` and model anew.
//!
//! `content_stamp` is the cheap part the page polls: a hash over the modification times of
//! the content folders, their entries, the folders the cached entries depend on (a bus's
//! paint folders) and the Mods inbox. When it changes, the page asks for the lists again and
//! only the folders whose own stamp changed are read.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Increment when the cached catalog format or the logic that derives it changes.
const INDEX_VERSION: u32 = 3;

/// Modification time of `p`; a path inside an archive used in place has the archive's.
pub fn mtime_ns(p: &Path) -> u64 {
    let archive = omsi_cfg::vfs::archive_of(p);
    let p = archive.as_deref().unwrap_or(p);
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn hasher() -> std::collections::hash_map::DefaultHasher {
    std::collections::hash_map::DefaultHasher::new()
}

/// Stamp of folders: their own times and every direct entry's name, time and size.
pub fn folder_stamp(dirs: &[PathBuf]) -> u64 {
    let mut h = hasher();
    for d in dirs {
        d.hash(&mut h);
        mtime_ns(d).hash(&mut h);
        if let Ok(rd) = std::fs::read_dir(d) {
            let mut items: Vec<(String, u64, u64)> = rd
                .flatten()
                .map(|e| {
                    let md = e.metadata().ok();
                    let t = md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
                    (e.file_name().to_string_lossy().to_string(), t, md.map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0))
                })
                .collect();
            items.sort();
            items.hash(&mut h);
        }
    }
    h.finish()
}

/// Stamp of single files (their times and sizes).
pub fn files_stamp(files: &[PathBuf]) -> u64 {
    let mut h = hasher();
    for f in files {
        f.hash(&mut h);
        let archive = omsi_cfg::vfs::archive_of(f);
        let md = std::fs::metadata(archive.as_deref().unwrap_or(f)).ok();
        md.as_ref().and_then(|m| m.modified().ok()).hash(&mut h);
        md.map(|m| m.len()).hash(&mut h);
    }
    h.finish()
}

fn deps_stamp(deps: &[PathBuf]) -> u64 {
    let mut h = hasher();
    for d in deps {
        d.hash(&mut h);
        mtime_ns(d).hash(&mut h);
    }
    h.finish()
}

fn combine(a: u64, b: u64) -> u64 {
    let mut h = hasher();
    (a, b).hash(&mut h);
    h.finish()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Entry {
    stamp: u64,
    /// Folders (outside the entry's own) whose change makes the entry stale.
    deps: Vec<PathBuf>,
    value: serde_json::Value,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    version: u32,
    entries: HashMap<String, Entry>,
    #[serde(skip)]
    dirty: bool,
    #[serde(skip)]
    loaded: bool,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn store_path() -> PathBuf {
    crate::data_dir().join("cache").join("content-index.json")
}

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.get_or_insert_with(Store::default);
    if !s.loaded {
        let disk: Option<Store> = std::fs::read(store_path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
        *s = match disk {
            Some(d) if d.version == INDEX_VERSION => d,
            _ => Store { version: INDEX_VERSION, ..Default::default() },
        };
        s.loaded = true;
    }
    f(s)
}

/// The value for `key`, from the cache when `stamp` (and the dependencies recorded with
/// it) are unchanged, else made anew by `make`, which also names its dependencies.
pub fn cached<T: Serialize + DeserializeOwned>(key: &str, stamp: u64, make: impl FnOnce() -> (T, Vec<PathBuf>)) -> T {
    // (the files looked at and the value read outside the lock: the lists are read by
    // several threads at once)
    let entry = with_store(|s| s.entries.get(key).map(|e| (e.stamp, e.deps.clone(), e.value.clone())));
    if let Some((full, deps, value)) = entry {
        if combine(stamp, deps_stamp(&deps)) == full {
            if let Ok(v) = serde_json::from_value::<T>(value) {
                return v;
            }
        }
    }
    let (value, deps) = make();
    let json = serde_json::to_value(&value).unwrap_or(serde_json::Value::Null);
    let full = combine(stamp, deps_stamp(&deps));
    with_store(|s| {
        s.entries.insert(key.to_string(), Entry { stamp: full, deps, value: json });
        s.dirty = true;
    });
    value
}

/// Write the cache to disk if it changed; entries whose key is not in `keep` (folders that
/// are gone) are dropped first when `keep` is given for that key prefix.
pub fn save(prefix: &str, keep: Option<&[String]>) {
    with_store(|s| {
        if let Some(keep) = keep {
            let before = s.entries.len();
            s.entries.retain(|k, _| !k.starts_with(prefix) || keep.contains(k));
            if s.entries.len() != before {
                s.dirty = true;
            }
        }
        if !s.dirty {
            return;
        }
        let p = store_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        if let Ok(b) = serde_json::to_vec(&*s) {
            if std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, &p).is_ok() {
                s.dirty = false;
            }
        }
    });
}

/// The folders whose change the page should notice: for each base (content folder, OMSI
/// folder) the maps / Vehicles / Weather folders with their entries, each map's
/// `global.cfg`, the dependencies of the cached entries, and the Mods inbox.
pub fn content_stamp(bases: &[PathBuf], inbox: Option<&Path>) -> String {
    let mut h = hasher();
    // the archives used in place come and go with the content folder's `Archives`
    if let Some(c) = bases.first() {
        folder_stamp(&[c.join(crate::install::ARCHIVES)]).hash(&mut h);
    }
    for b in bases {
        for rel in ["maps", "Vehicles", "Weather"] {
            let d = b.join(rel);
            folder_stamp(std::slice::from_ref(&d)).hash(&mut h);
            if rel == "Weather" {
                continue;
            }
            if let Ok(rd) = std::fs::read_dir(&d) {
                let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                subs.sort();
                for s in subs {
                    mtime_ns(&s).hash(&mut h);
                    if rel == "maps" {
                        mtime_ns(&s.join("global.cfg")).hash(&mut h);
                    }
                }
            }
        }
    }
    with_store(|s| {
        let mut deps: Vec<&PathBuf> = s.entries.values().flat_map(|e| e.deps.iter()).collect();
        deps.sort();
        deps.dedup();
        for d in deps {
            mtime_ns(d).hash(&mut h);
        }
    });
    if let Some(i) = inbox {
        folder_stamp(&[i.to_path_buf(), i.join(crate::install::WAITING)]).hash(&mut h);
    }
    format!("{:016x}", h.finish())
}
