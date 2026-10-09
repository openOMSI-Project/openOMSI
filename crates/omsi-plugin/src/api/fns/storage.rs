//! A plugin's own data: a key/value storage (`storage.*`, kept as `storage.json`) and files
//! (`files.*`), both in its data folder and nowhere else.

use super::{def, o, p, rec};
use crate::api::{json, paths, ApiError, ApiFn, Ctx, PluginState, Value};
use std::path::{Path, PathBuf};

const NEW: &str = crate::api::VERSION;
/// The storage's file in the data folder.
const STORE_FILE: &str = "storage.json";
/// Largest file a plugin writes at once, and all its data together.
const MAX_FILE: usize = 16 << 20;
const MAX_TOTAL: u64 = 256 << 20;

/// The key/value storage of a plugin.
#[derive(Debug, Default)]
pub struct Store {
    values: Vec<(String, Value)>,
    dirty: bool,
}

impl Store {
    fn load(dir: &Path) -> Store {
        let values = match std::fs::read_to_string(dir.join(STORE_FILE)).ok().map(|t| json::decode(&t, false)) {
            Some(Ok(Value::Map(m))) => m,
            Some(Err(e)) => {
                log::warn!("{}: {e}; starting empty", dir.join(STORE_FILE).display());
                Vec::new()
            }
            _ => Vec::new(),
        };
        Store { values, dirty: false }
    }
}

/// The plugin's storage, read from its file on first use.
fn store(s: &mut PluginState) -> &mut Store {
    if s.storage.is_none() {
        s.storage = Some(Store::load(&s.data_dir));
    }
    s.storage.as_mut().expect("loaded above")
}

/// Write the storage when it changed (the plugin stops, `storage.save`).
pub fn flush(s: &mut PluginState) {
    let Some(st) = s.storage.as_mut().filter(|st| st.dirty) else { return };
    st.dirty = false;
    let text = json::encode_pretty(&Value::Map(st.values.clone()));
    let r = std::fs::create_dir_all(&s.data_dir).and_then(|_| {
        let tmp = s.data_dir.join(format!("{STORE_FILE}.new"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, s.data_dir.join(STORE_FILE))
    });
    if let Err(e) = r {
        log::warn!("{} storage not saved: {e}", s.tag);
    }
}

/// A value the storage keeps: no functions.
fn storable(v: &Value) -> Result<(), String> {
    match v {
        Value::Callback(_) => Err("a function cannot be stored".into()),
        Value::List(l) => l.iter().try_for_each(storable),
        Value::Map(m) => m.iter().try_for_each(|(_, v)| storable(v)),
        _ => Ok(()),
    }
}

/// A path in the plugin's data folder.
fn data_path(c: &mut Ctx<'_>, rel: &str) -> Result<PathBuf, String> {
    paths::inside(&c.state().data_dir, rel)
}

fn dir_size(d: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(d) else { return 0 };
    rd.flatten().map(|e| match e.metadata() {
        Ok(m) if m.is_dir() => dir_size(&e.path()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }).sum()
}

/// Write (or add to) a file of the data folder, within the limits.
fn write(c: &mut Ctx<'_>, rel: &str, data: &[u8], append: bool) -> Result<(), String> {
    let path = data_path(c, rel)?;
    if data.len() > MAX_FILE {
        return Err(format!("at most {} MB at once", MAX_FILE >> 20));
    }
    let dir = c.state().data_dir.clone();
    if dir_size(&dir) + data.len() as u64 > MAX_TOTAL {
        return Err(format!("a plugin's data folder holds at most {} MB", MAX_TOTAL >> 20));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if append {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
        f.write_all(data).map_err(|e| e.to_string())
    } else {
        std::fs::write(&path, data).map_err(|e| e.to_string())
    }
}

/// The text (or bytes) of `v` to write.
fn bytes(v: &Value) -> Result<Vec<u8>, ApiError> {
    match v {
        Value::Bytes(b) => Ok(b.clone()),
        v => v.to_text().map(String::into_bytes).ok_or_else(|| ApiError("the data is a string".into())),
    }
}

/// The value, or nil and the reason (several values).
fn or_reason(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => Value::List(vec![v]),
        Err(e) => Value::List(vec![Value::Nil, Value::Str(e)]),
    }
}

/// A folder's entries: `{name, dir, size}` sorted by name.
pub(crate) fn list_dir(d: &Path) -> Result<Value, String> {
    let rd = std::fs::read_dir(d).map_err(|e| e.to_string())?;
    let mut out: Vec<(String, bool, u64)> = rd.flatten().filter_map(|e| {
        let m = e.metadata().ok()?;
        Some((e.file_name().to_string_lossy().into_owned(), m.is_dir(), m.len()))
    }).collect();
    out.sort();
    Ok(Value::List(out.into_iter().map(|(n, d, s)| rec(vec![("name", n.into()), ("dir", d.into()), ("size", Value::Int(if d { 0 } else { s as i64 }))])).collect()))
}

pub static FNS: &[ApiFn] = &[
    def!("storage.get", "storage", [p("key", "string")], "any", "A value the plugin stored, or `nil`. The storage is the plugin's own and survives the session (kept as `storage.json` in its data folder).", NEW, Storage, false, |c, a| {
        let k = a.str(0)?;
        Ok::<_, ApiError>(store(c.state()).values.iter().find(|(x, _)| *x == k).map_or(Value::Nil, |(_, v)| v.clone()))
    }),
    def!("storage.set", "storage", [p("key", "string"), p("value", "any")], "nil", "Stores a value under a key: a number, text, boolean or a table of them (`nil` removes it). It is written when the game ends, the plugin is loaded again, or `storage.save()` is called.", NEW, Storage, false, |c, a| {
        let k = a.str(0)?;
        let v = a.take(1);
        storable(&v).map_err(ApiError)?;
        let st = store(c.state());
        st.dirty = true;
        match (st.values.iter().position(|(x, _)| *x == k), v.is_nil()) {
            (Some(i), true) => {
                st.values.remove(i);
            }
            (Some(i), false) => st.values[i].1 = v,
            (None, false) => st.values.push((k, v)),
            (None, true) => {}
        }
        Ok::<_, ApiError>(())
    }),
    def!("storage.delete", "storage", [p("key", "string")], "boolean", "Removes a key; `true` when it was there.", NEW, Storage, false, |c, a| {
        let k = a.str(0)?;
        let st = store(c.state());
        let n = st.values.len();
        st.values.retain(|(x, _)| *x != k);
        st.dirty |= st.values.len() != n;
        Ok::<_, ApiError>(st.values.len() != n)
    }),
    def!("storage.keys", "storage", [], "list of strings", "The keys stored, in the order they were first set.", NEW, Storage, false, |c, a| store(c.state()).values.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()),
    def!("storage.all", "storage", [], "table", "Everything stored, as one table.", NEW, Storage, false, |c, a| Value::Map(store(c.state()).values.clone())),
    def!("storage.clear", "storage", [], "nil", "Removes every key.", NEW, Storage, false, |c, a| {
        let st = store(c.state());
        st.dirty |= !st.values.is_empty();
        st.values.clear();
    }),
    def!("storage.save", "storage", [], "nil", "Writes the storage now (it is written by itself when the game ends).", NEW, Storage, false, |c, a| flush(c.state())),
    def!("files.read", "storage", [p("path", "string")], "text, or nil and the reason", "A file of the plugin's data folder (`path` relative to it; `..` and absolute paths are refused).", NEW, Storage, true, |c, a| {
        let rel = a.str(0)?;
        Ok::<_, ApiError>(or_reason(data_path(c, &rel).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())).map(|b| match String::from_utf8(b) {
            Ok(s) => Value::Str(s),
            Err(e) => Value::Bytes(e.into_bytes()),
        })))
    }),
    def!("files.write", "storage", [p("path", "string"), p("data", "string")], "true, or false and the reason", "Writes a file of the data folder (its folders are made); at most 16 MB at once and 256 MB in all.", NEW, Storage, true, |c, a| {
        let (rel, data) = (a.str(0)?, bytes(a.get(1))?);
        Ok::<_, ApiError>(super::core::ok_or_reason(write(c, &rel, &data, false)))
    }),
    def!("files.append", "storage", [p("path", "string"), p("data", "string")], "true, or false and the reason", "Adds to the end of a file of the data folder (a log, a CSV of trips).", NEW, Storage, true, |c, a| {
        let (rel, data) = (a.str(0)?, bytes(a.get(1))?);
        Ok::<_, ApiError>(super::core::ok_or_reason(write(c, &rel, &data, true)))
    }),
    def!("files.exists", "storage", [p("path", "string")], "boolean", "Whether the data folder has this file or folder.", NEW, Storage, false, |c, a| {
        let rel = a.str(0)?;
        Ok::<_, ApiError>(data_path(c, &rel).is_ok_and(|p| p.exists()))
    }),
    def!("files.list", "storage", [o("dir", "string")], "list of tables, or nil and the reason", "The entries of the data folder (or a folder in it): `{name, dir, size}`.", NEW, Storage, true, |c, a| {
        let rel = a.opt_str(0)?.unwrap_or_default();
        Ok::<_, ApiError>(or_reason(data_path(c, &rel).and_then(|p| list_dir(&p))))
    }),
    def!("files.delete", "storage", [p("path", "string")], "boolean", "Removes a file or an empty folder of the data folder.", NEW, Storage, false, |c, a| {
        let rel = a.str(0)?;
        Ok::<_, ApiError>(data_path(c, &rel).ok().filter(|_| !rel.trim_matches(['/', '\\', '.']).is_empty()).is_some_and(|p| if p.is_dir() { std::fs::remove_dir(&p).is_ok() } else { std::fs::remove_file(&p).is_ok() }))
    }),
    def!("files.mkdir", "storage", [p("path", "string")], "boolean", "Makes a folder (and those above it) in the data folder.", NEW, Storage, false, |c, a| {
        let rel = a.str(0)?;
        Ok::<_, ApiError>(data_path(c, &rel).is_ok_and(|p| std::fs::create_dir_all(p).is_ok()))
    }),
    def!("files.dir", "storage", [], "string", "Where the data folder is on this computer (to tell the player; the plugin reaches it with relative paths only).", NEW, Storage, false, |c, a| c.state().data_dir.to_string_lossy().into_owned()),
    def!("json.encode", "util", [p("value", "any"), o("pretty", "bool")], "string", "A value as JSON text (a list as an array, a table with keys as an object; `pretty`: indented).", NEW, None, false, |c, a| {
        let v = a.get(0);
        if a.flag(1, false) { json::encode_pretty(v) } else { json::encode(v) }
    }),
    def!("json.decode", "util", [p("text", "string")], "value, or nil and the reason", "Reads JSON text: objects and arrays become tables, `null` nil.", NEW, None, true, |c, a| {
        let t = a.str(0)?;
        Ok::<_, ApiError>(or_reason(json::decode(&t, false)))
    }),
];
