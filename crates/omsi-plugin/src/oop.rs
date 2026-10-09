//! Compiled plugins: `plugins/*.oop`, made by `oopc` of the openOMSI Development Tools.
//!
//! A `.oop` holds build output only - a stripped WebAssembly module, or Lua compiled into one
//! obfuscated chunk - encrypted and, when its author signed it, signed (the format is the
//! `oop-format` crate's). The game reads it into memory: the code never touches the disk.
//! Only the plugin's assets (pictures, sounds, data files), which its API functions read from a
//! folder, are put into a cache folder beside it.
//!
//! The permissions in its header are what it may do: a function needing another one fails
//! (a plain `.lua` file keeps every permission, as before).

use crate::api::Perm;
use crate::lua::{LuaSource, LuaSpec};
use crate::wasm_plugin::WasmSpec;
use oop_format::{Kind, Oop};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The `.oop` files of a plugins folder.
pub fn find_oop(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("oop"))).collect();
    v.sort();
    v
}

/// A `.oop`, read and checked, ready to start.
pub enum Loaded {
    Lua(LuaSpec),
    Wasm(WasmSpec),
}

/// Most a `.oop` file may weigh on disk.
const MAX_FILE: u64 = 512 << 20;

/// Read and check `path` (tag, signature, paths, sizes) and make its plugin's spec.
pub fn load(path: &Path) -> Result<Loaded, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err("larger than 512 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let oop = Oop::read(&bytes).map_err(|e| format!("not a usable .oop (damaged, changed after it was built, or made for another openOMSI): {e}"))?;
    let h = oop.header().clone();
    if h.api_abi > crate::wasm::ABI as u32 {
        return Err(format!("made for plugin API {}, this openOMSI has {} - update openOMSI", h.api_abi, crate::wasm::ABI));
    }
    match oop.signer_fingerprint() {
        Some(f) => log::info!("{}: {} {} by {}, signed by {f}", path.display(), h.name, h.version, h.authors.join(", ")),
        None => log::warn!("{}: {} {} is not signed: who built it is not known", path.display(), h.name, h.version),
    }
    let perms = permissions(&h.permissions);
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| h.id.clone());
    let dir = path.parent().unwrap_or(Path::new("."));
    let data_dir = dir.join(format!("{name}.data"));
    let entry = h.entry.clone();
    let kind = h.kind.clone();
    let files = oop.into_files();
    let folder = extract_assets(dir, &name, &files);
    match kind {
        Kind::Wasm => {
            let wasm = files.into_iter().find(|(p, _)| *p == entry).map(|(_, b)| b).ok_or_else(|| format!("its module {entry} is missing"))?;
            Ok(Loaded::Wasm(WasmSpec { name, wasm, data_dir, folder, perms: Some(perms) }))
        }
        Kind::Lua => {
            let source = ArchiveSource::new(&entry, files)?;
            let save_file = dir.join(format!("{name}.save.lua"));
            Ok(Loaded::Lua(LuaSpec { name, source: Box::new(source), save_file, data_dir, folder, perms: Some(perms) }))
        }
        other => Err(format!("a plugin of kind {} this openOMSI does not know - update openOMSI", other.as_str())),
    }
}

/// The header's permissions as the API knows them (an unknown one is logged and left out:
/// a newer `oopc`'s permission grants nothing here).
fn permissions(names: &[String]) -> HashSet<Perm> {
    names
        .iter()
        .filter_map(|n| {
            let p = Perm::parse(n);
            if p.is_none() {
                log::warn!("unknown plugin permission {n:?} left out");
            }
            p
        })
        .collect()
}

/// Whether a file of the archive is code (kept in memory) rather than an asset.
fn is_code(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".lua") || l.ends_with(".wasm")
}

/// The assets into `<plugins>/.oop-cache/<name>/` (written again only where they changed);
/// None when the plugin has none.
fn extract_assets(dir: &Path, name: &str, files: &[(String, Vec<u8>)]) -> Option<PathBuf> {
    let assets: Vec<&(String, Vec<u8>)> = files.iter().filter(|(p, _)| !is_code(p)).collect();
    if assets.is_empty() {
        return None;
    }
    let root = dir.join(".oop-cache").join(name);
    for (p, bytes) in assets {
        // (the paths were checked by `Oop::read`: relative, no `..`)
        let to = root.join(p);
        if std::fs::read(&to).is_ok_and(|old| old == *bytes) {
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&to, bytes) {
            log::warn!("{name}: could not write {}: {e}", to.display());
        }
    }
    Some(root)
}

/// A Lua plugin's code from the archive, in memory.
pub struct ArchiveSource {
    entry: String,
    lua: HashMap<String, String>,
}

impl ArchiveSource {
    pub fn new(entry: &str, files: Vec<(String, Vec<u8>)>) -> Result<ArchiveSource, String> {
        let mut lua = HashMap::new();
        for (p, b) in files {
            if p.to_ascii_lowercase().ends_with(".lua") {
                let text = String::from_utf8(b).map_err(|_| format!("{p} is not text (compiled Lua is refused: rebuild it with oopc)"))?;
                lua.insert(p, text);
            }
        }
        if !lua.contains_key(entry) {
            return Err(format!("its main file {entry} is missing"));
        }
        Ok(ArchiveSource { entry: entry.to_string(), lua })
    }
}

impl LuaSource for ArchiveSource {
    fn main(&self) -> Result<(String, String), String> {
        Ok((self.lua[&self.entry].clone(), self.entry.clone()))
    }

    fn module(&self, name: &str) -> Option<(String, String)> {
        let rel = name.replace('.', "/");
        [format!("{rel}.lua"), format!("{rel}/init.lua")].into_iter().find_map(|f| self.lua.get(&f).map(|t| (t.clone(), f)))
    }
}
