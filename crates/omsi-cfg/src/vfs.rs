//! The files of the content roots: plain folders, and `.zip` archives mounted as folders.
//!
//! A map or scenery pack is often shipped as one archive of several gigabytes (Ahlheim V5:
//! 5.2 GB zipped, 15.6 GB and 53 827 files unpacked). Unpacking it is not needed: an archive
//! is *mounted* at its own file path, so `…/AhlheimV5.zip/maps/Ahlheim 5/global.cfg` names
//! the entry `OMSI 2/maps/Ahlheim 5/global.cfg` inside it, and every loader that reads
//! through [`read`] / [`exists`] / [`list_dir`] cannot tell it from an unpacked folder.
//!
//! * The central directory is read once into an index (ZIP64 included - archives past 4 GB
//!   need it); an entry is then one positioned read plus a stored copy or an inflate.
//! * Lookups are case-insensitive, as on Windows, and `\` separators in entry names count
//!   as `/`.
//! * The folder inside the archive that holds OMSI's content folders (`Vehicles`, `maps`,
//!   `Sceneryobjects` …) becomes the mount's root: archives usually wrap everything in an
//!   `OMSI 2/` folder next to documentation.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// One file inside an archive.
#[derive(Debug, Clone)]
struct ZipEntry {
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    /// Offset of the local file header.
    header: u64,
}

/// A mounted archive.
pub struct ZipArchive {
    /// The archive file, which is also the folder it is mounted as.
    path: PathBuf,
    file: std::fs::File,
    #[cfg(not(unix))]
    lock: std::sync::Mutex<()>,
    /// The archive's own folder that became the mount root (original spelling, `/` at the end
    /// unless empty).
    prefix: String,
    /// Entries by lower-case path relative to the mount root.
    entries: HashMap<String, ZipEntry>,
    /// Folder contents by lower-case folder path ("" = the root): (name as stored, is folder).
    dirs: HashMap<String, Vec<(String, bool)>>,
}

static MOUNTS: RwLock<Vec<Arc<ZipArchive>>> = RwLock::new(Vec::new());

/// The largest entry that is read into memory (a corrupt size field must not allocate
/// gigabytes).
const MAX_ENTRY: u64 = 1 << 31;

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7]])
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Code page 437, the encoding of entry names written without the UTF-8 flag (what the
/// Windows zip tools of the OMSI years produced for "Straße" or "Bahnübergang").
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

fn decode_name(raw: &[u8], utf8: bool) -> String {
    if utf8 || raw.is_ascii() {
        return String::from_utf8_lossy(raw).into_owned();
    }
    match std::str::from_utf8(raw) {
        // many tools write UTF-8 without setting the flag
        Ok(s) => s.to_string(),
        Err(_) => raw.iter().map(|&b| if b < 0x80 { b as char } else { CP437_HIGH[(b - 0x80) as usize] }).collect(),
    }
}

/// Lower-case, `/`-separated, without `.`/`..`/empty components.
fn normalize(rel: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in rel.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c.to_lowercase()),
        }
    }
    parts.join("/")
}

impl ZipArchive {
    /// Read the central directory of `path`.
    pub fn open(path: &Path) -> io::Result<ZipArchive> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let archive = ZipArchive { path: path.to_path_buf(), file, #[cfg(not(unix))] lock: std::sync::Mutex::new(()), prefix: String::new(), entries: HashMap::new(), dirs: HashMap::new() };
        // end of central directory: the last 22 bytes plus a comment of up to 65 535
        let tail_len = len.min(22 + 65_535);
        let tail = archive.read_at(len - tail_len, tail_len as usize)?;
        let eocd = (0..tail.len().saturating_sub(21)).rev().find(|&i| le32(&tail, i) == 0x0605_4b50).ok_or_else(|| bad(format!("{}: not a zip archive", path.display())))?;
        let e = &tail[eocd..];
        let mut count = le16(e, 10) as u64;
        let mut cd_size = le32(e, 12) as u64;
        let mut cd_offset = le32(e, 16) as u64;
        // ZIP64: a locator right before the classic record points at the 64-bit one
        if eocd >= 20 && le32(&tail, eocd - 20) == 0x0706_4b50 {
            let at = le64(&tail, eocd - 20 + 8);
            let z = archive.read_at(at, 56)?;
            if le32(&z, 0) != 0x0606_4b50 {
                return Err(bad(format!("{}: broken ZIP64 end record", path.display())));
            }
            count = le64(&z, 32);
            cd_size = le64(&z, 40);
            cd_offset = le64(&z, 48);
        }
        if cd_offset.checked_add(cd_size).map(|end| end > len).unwrap_or(true) || cd_size > 1 << 32 {
            return Err(bad(format!("{}: central directory out of range", path.display())));
        }
        let cd = archive.read_at(cd_offset, cd_size as usize)?;
        let mut raw: Vec<(String, ZipEntry, bool)> = Vec::with_capacity(count as usize);
        let mut p = 0usize;
        while p + 46 <= cd.len() && le32(&cd, p) == 0x0201_4b50 {
            let flags = le16(&cd, p + 8);
            let method = le16(&cd, p + 10);
            let crc = le32(&cd, p + 16);
            let mut compressed = le32(&cd, p + 20) as u64;
            let mut size = le32(&cd, p + 24) as u64;
            let name_len = le16(&cd, p + 28) as usize;
            let extra_len = le16(&cd, p + 30) as usize;
            let comment_len = le16(&cd, p + 32) as usize;
            let mut header = le32(&cd, p + 42) as u64;
            let end = p + 46 + name_len + extra_len + comment_len;
            if end > cd.len() {
                break;
            }
            let name_raw = &cd[p + 46..p + 46 + name_len];
            let extra = &cd[p + 46 + name_len..p + 46 + name_len + extra_len];
            let mut name = decode_name(name_raw, flags & 0x800 != 0);
            // extra fields: ZIP64 sizes/offset, and Info-ZIP's Unicode path
            let mut q = 0usize;
            while q + 4 <= extra.len() {
                let id = le16(extra, q);
                let n = le16(extra, q + 2) as usize;
                let body = &extra[(q + 4).min(extra.len())..(q + 4 + n).min(extra.len())];
                if id == 0x0001 {
                    let mut k = 0usize;
                    let mut take = |v: &mut u64| {
                        if *v == 0xFFFF_FFFF && k + 8 <= body.len() {
                            *v = le64(body, k);
                            k += 8;
                        }
                    };
                    take(&mut size);
                    take(&mut compressed);
                    take(&mut header);
                } else if id == 0x7075 && body.len() > 5 {
                    if let Ok(s) = std::str::from_utf8(&body[5..]) {
                        name = s.to_string();
                    }
                }
                q += 4 + n;
            }
            p = end;
            let name = name.replace('\\', "/");
            let is_dir = name.ends_with('/');
            raw.push((name, ZipEntry { method, flags, crc, compressed, size, header }, is_dir));
        }
        let mut archive = archive;
        archive.prefix = Self::content_prefix(raw.iter().map(|(n, _, _)| n.as_str()));
        let prefix_lower = archive.prefix.to_lowercase();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (name, entry, is_dir) in raw {
            let lower = name.to_lowercase();
            let Some(rel_lower) = lower.strip_prefix(&prefix_lower) else { continue };
            let rel_lower = normalize(rel_lower);
            if rel_lower.is_empty() {
                continue;
            }
            // the original spelling of every component, for listings
            let rel: Vec<&str> = name[archive.prefix.len()..].split('/').filter(|c| !c.is_empty() && *c != ".").collect();
            let lower_parts: Vec<&str> = rel_lower.split('/').collect();
            if rel.len() != lower_parts.len() {
                continue;
            }
            for k in 0..rel.len() {
                let parent = lower_parts[..k].join("/");
                let child_is_dir = k + 1 < rel.len() || is_dir;
                if seen.insert(lower_parts[..=k].join("/")) {
                    archive.dirs.entry(parent).or_default().push((rel[k].to_string(), child_is_dir));
                }
            }
            if is_dir {
                archive.dirs.entry(rel_lower).or_default();
            } else {
                archive.entries.insert(rel_lower, entry);
            }
        }
        Ok(archive)
    }

    /// The archive folder that holds OMSI's content folders ("" when the archive starts with
    /// them). Only a folder that does not lie inside a content folder can be the root: a
    /// bus's `Vehicles/MAN_Lion/` has `Sound` and `Texture` sub-folders as well, and so has a
    /// scenery pack's `Sceneryobjects/Pack/`. Of the candidates, the one with the most
    /// content folders among its children wins - counting first the names found only at the
    /// top of an installation (`Vehicles`, `maps` ...), then those an add-on has inside too
    /// (`Sound`, `Texture`, `Scripts`) - then the shallowest, then the first by name.
    fn content_prefix<'a>(names: impl Iterator<Item = &'a str>) -> String {
        const SHARED: [&str; 3] = ["Texture", "Sounds", "Scripts"];
        let is_content = |c: &str| crate::CONTENT_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(c));
        // folder (lower case) -> (spelling, lower-case names of its sub-folders)
        let mut children: HashMap<String, (String, Vec<String>)> = HashMap::new();
        for n in names {
            let parts: Vec<&str> = n.split('/').filter(|c| !c.is_empty()).collect();
            let folders = if n.ends_with('/') { parts.len() } else { parts.len().saturating_sub(1) };
            for k in 0..folders.min(6) {
                if k > 0 && is_content(parts[k - 1]) {
                    break; // below a content folder: an add-on's own folders
                }
                let dir: String = parts[..k].iter().map(|c| format!("{c}/")).collect();
                let e = children.entry(dir.to_lowercase()).or_insert_with(|| (dir.clone(), Vec::new()));
                let child = parts[k].to_lowercase();
                if !e.1.contains(&child) {
                    e.1.push(child);
                }
            }
        }
        children
            .iter()
            .filter_map(|(key, (dir, kids))| {
                let present: Vec<&str> = crate::CONTENT_FOLDERS.iter().copied().filter(|f| kids.iter().any(|k| k.eq_ignore_ascii_case(f))).collect();
                if present.is_empty() {
                    return None;
                }
                let shared = present.iter().filter(|f| SHARED.iter().any(|s| s.eq_ignore_ascii_case(f))).count();
                let depth = key.matches('/').count();
                Some(((present.len() - shared, shared, std::cmp::Reverse(depth), std::cmp::Reverse(key.as_str())), dir))
            })
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, dir)| dir.clone())
            .unwrap_or_default()
    }

    #[cfg(unix)]
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        use std::os::unix::fs::FileExt;
        let mut buf = vec![0u8; len];
        self.file.read_exact_at(&mut buf, offset)?;
        Ok(buf)
    }

    #[cfg(not(unix))]
    fn read_at(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        use std::io::Seek;
        let _g = self.lock.lock().unwrap();
        let mut f = &self.file;
        f.seek(io::SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// The archive file (= the mount point).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The folder inside the archive the mount starts at.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn file_count(&self) -> usize {
        self.entries.len()
    }

    /// Unpacked size of all files below the mount root.
    pub fn total_size(&self) -> u64 {
        self.entries.values().map(|e| e.size).sum()
    }

    /// Contents of the entry at `rel` (relative to the mount root, any case).
    pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        let key = normalize(rel);
        let e = self.entries.get(&key).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{}: no entry {rel}", self.path.display())))?;
        if e.flags & 1 != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported, format!("{}: {rel} is encrypted", self.path.display())));
        }
        if e.size > MAX_ENTRY || e.compressed > MAX_ENTRY {
            return Err(bad(format!("{}: {rel} is too large ({} bytes)", self.path.display(), e.size)));
        }
        let local = self.read_at(e.header, 30)?;
        if le32(&local, 0) != 0x0403_4b50 {
            return Err(bad(format!("{}: {rel}: bad local header", self.path.display())));
        }
        let data_at = e.header + 30 + le16(&local, 26) as u64 + le16(&local, 28) as u64;
        let packed = self.read_at(data_at, e.compressed as usize)?;
        let out = match e.method {
            0 => packed,
            8 => {
                let mut out = Vec::with_capacity(e.size as usize);
                flate2::read::DeflateDecoder::new(&packed[..]).take(e.size + 1).read_to_end(&mut out)?;
                out
            }
            m => return Err(io::Error::new(io::ErrorKind::Unsupported, format!("{}: {rel}: compression method {m} is not supported", self.path.display()))),
        };
        if out.len() as u64 != e.size {
            return Err(bad(format!("{}: {rel}: {} bytes unpacked, {} expected", self.path.display(), out.len(), e.size)));
        }
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        if crc.sum() != e.crc {
            return Err(bad(format!("{}: {rel}: checksum mismatch", self.path.display())));
        }
        Ok(out)
    }

    fn is_file(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    fn is_dir(&self, key: &str) -> bool {
        self.dirs.contains_key(key)
    }
}

/// Mount the archive at `path` (as the folder `path`). Mounting the same archive twice is a
/// no-op. Returns the mount point.
pub fn mount_zip(path: &Path) -> io::Result<PathBuf> {
    if let Some(m) = MOUNTS.read().unwrap().iter().find(|m| m.path == path) {
        return Ok(m.path.clone());
    }
    let t0 = std::time::Instant::now();
    let archive = ZipArchive::open(path)?;
    log::info!("zip {}: {} files ({:.1} GB unpacked) under '{}', indexed in {:.0} ms", path.display(), archive.file_count(), archive.total_size() as f64 / 1e9, archive.prefix, t0.elapsed().as_secs_f64() * 1000.0);
    if archive.prefix.is_empty() && !archive.dirs.get("").map(|l| l.iter().any(|(n, _)| crate::CONTENT_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(n)))).unwrap_or(false) {
        log::warn!("zip {}: no OMSI content folders (Vehicles, maps, Sceneryobjects ...) found; mounted as it is", path.display());
    }
    let mount = archive.path.clone();
    MOUNTS.write().unwrap().push(Arc::new(archive));
    Ok(mount)
}

/// Mount a zip archive and make it a content root (searched after the roots added before).
pub fn add_content_zip(path: &Path) -> io::Result<PathBuf> {
    let mount = mount_zip(path)?;
    crate::add_content_root(mount.clone());
    Ok(mount)
}

/// The mounted archives.
pub fn mounts() -> Vec<Arc<ZipArchive>> {
    MOUNTS.read().unwrap().clone()
}

/// The archive `path` lies in, and its lower-case path inside the mount. Names are read as
/// Windows reads them (a folder loses one trailing dot, the last name all trailing dots and
/// spaces), since the archive can only hold names Windows could write.
fn locate(path: &Path) -> Option<(Arc<ZipArchive>, String)> {
    let mounts = MOUNTS.read().unwrap();
    if mounts.is_empty() {
        return None;
    }
    for m in mounts.iter() {
        if let Ok(rest) = path.strip_prefix(&m.path) {
            let mut parts: Vec<String> = Vec::new();
            let comps: Vec<Component> = rest.components().collect();
            for (i, c) in comps.iter().enumerate() {
                match c {
                    Component::Normal(s) => {
                        let s = s.to_string_lossy();
                        let s: &str = &s;
                        let s = if i + 1 == comps.len() { s.trim_end_matches(['.', ' ']) } else { s.strip_suffix('.').unwrap_or(s) };
                        if !s.is_empty() {
                            parts.push(s.to_lowercase());
                        }
                    }
                    Component::ParentDir => {
                        parts.pop();
                    }
                    _ => {}
                }
            }
            return Some((m.clone(), parts.join("/")));
        }
    }
    None
}

/// The archive file `path` lies in (the mount point), if it lies in a mounted one.
pub fn archive_of(path: &Path) -> Option<PathBuf> {
    locate(path).map(|(m, _)| m.path.clone())
}

/// Read a whole file, from a folder or from a mounted archive.
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match locate(path) {
        Some((m, key)) => m.read(&key),
        None => std::fs::read(path),
    }
}

/// Read a text file with OMSI's encoding rules.
pub fn read_text(path: &Path) -> io::Result<String> {
    read(path).map(|b| crate::decode_text(&b))
}

pub fn exists(path: &Path) -> bool {
    match locate(path) {
        Some((m, key)) => key.is_empty() || m.is_file(&key) || m.is_dir(&key),
        None => path.exists(),
    }
}

pub fn is_file(path: &Path) -> bool {
    match locate(path) {
        Some((m, key)) => m.is_file(&key),
        None => path.is_file(),
    }
}

pub fn is_dir(path: &Path) -> bool {
    match locate(path) {
        Some((m, key)) => key.is_empty() || m.is_dir(&key),
        None => path.is_dir(),
    }
}

/// Names in a folder with a flag for sub-folders; `None` when it is not a folder.
pub fn list_dir(path: &Path) -> Option<Vec<(OsString, bool)>> {
    match locate(path) {
        Some((m, key)) => m.dirs.get(&key).map(|l| l.iter().map(|(n, d)| (OsString::from(n), *d)).collect()),
        None => {
            let rd = std::fs::read_dir(path).ok()?;
            // the entry's own type costs nothing; only a symbolic link needs a stat
            Some(
                rd.flatten()
                    .map(|e| {
                        let dir = match e.file_type() {
                            Ok(t) if t.is_symlink() => e.path().is_dir(),
                            Ok(t) => t.is_dir(),
                            Err(_) => false,
                        };
                        (e.file_name(), dir)
                    })
                    .collect(),
            )
        }
    }
}

/// Full paths of a folder's entries (like `std::fs::read_dir`), empty when it is not a folder.
pub fn read_dir_paths(path: &Path) -> Vec<PathBuf> {
    list_dir(path).map(|l| l.into_iter().map(|(n, _)| path.join(n)).collect()).unwrap_or_default()
}

/// Mount every `.zip` in `dir` (in name order) as a content root: where an installer puts
/// map and mod archives that are read in place. Returns the mount points.
pub fn mount_dir_zips(dir: &Path) -> Vec<PathBuf> {
    let mut zips: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false)).collect(),
        Err(_) => return Vec::new(),
    };
    zips.sort();
    let mut out = Vec::new();
    for p in zips {
        match add_content_zip(&p) {
            Ok(m) => out.push(m),
            Err(e) => log::warn!("content zip {}: {e}", p.display()),
        }
    }
    out
}

/// Mount every archive listed in `OMSI_CONTENT_ZIP` (separated like `PATH`) as a content
/// root. Returns the mount points.
pub fn mount_env_zips() -> Vec<PathBuf> {
    let Some(v) = std::env::var_os("OMSI_CONTENT_ZIP") else { return Vec::new() };
    let mut out = Vec::new();
    for p in std::env::split_paths(&v) {
        if p.as_os_str().is_empty() {
            continue;
        }
        match add_content_zip(&p) {
            Ok(m) => out.push(m),
            Err(e) => log::warn!("content zip {}: {e}", p.display()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A small archive with a wrapper folder, a stored and a deflated entry.
    fn build_zip(path: &Path) {
        let files: Vec<(&str, Vec<u8>, bool)> = vec![
            ("Docs/readme.txt", b"hello".to_vec(), false),
            ("OMSI 2/maps/Test Map/global.cfg", b"[name]\r\nTest\r\n".repeat(20), true),
            ("OMSI 2/Sceneryobjects/Stra\u{df}e/Schild.sco", b"[mesh]\r\nschild.o3d\r\n".to_vec(), false),
            ("OMSI 2/Vehicles/", Vec::new(), false),
        ];
        write_zip(path, &files);
    }

    /// An archive of `files`: (name, contents, deflated).
    fn write_zip(path: &Path, files: &[(&str, Vec<u8>, bool)]) {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for (name, data, deflate) in files {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let packed = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.clone()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            let head = |sig: u32, buf: &mut Vec<u8>, central: bool| {
                buf.extend_from_slice(&sig.to_le_bytes());
                if central {
                    buf.extend_from_slice(&20u16.to_le_bytes());
                }
                buf.extend_from_slice(&20u16.to_le_bytes());
                buf.extend_from_slice(&0x800u16.to_le_bytes());
                buf.extend_from_slice(&method.to_le_bytes());
                buf.extend_from_slice(&[0; 4]);
                buf.extend_from_slice(&crc.sum().to_le_bytes());
                buf.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
                buf.extend_from_slice(&0u16.to_le_bytes());
                if central {
                    // comment length, disk, internal and external attributes
                    buf.extend_from_slice(&[0; 10]);
                    buf.extend_from_slice(&offset.to_le_bytes());
                }
                buf.extend_from_slice(name.as_bytes());
            };
            head(0x0403_4b50, &mut out, false);
            out.extend_from_slice(&packed);
            head(0x0201_4b50, &mut central, true);
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn zip_mount() {
        let dir = std::env::temp_dir().join(format!("omsi-cfg-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("pack.zip");
        build_zip(&zip);
        let mount = mount_zip(&zip).unwrap();
        let a = mounts().into_iter().find(|m| m.path() == zip).unwrap();
        assert_eq!(a.prefix(), "OMSI 2/");
        let cfg = mount.join("MAPS").join("test map").join("Global.CFG");
        assert!(is_file(&cfg));
        assert_eq!(read(&cfg).unwrap(), b"[name]\r\nTest\r\n".repeat(20));
        assert!(is_dir(&mount.join("maps")));
        assert!(is_dir(&mount.join("vehicles")));
        assert!(!exists(&mount.join("Docs")));
        let sco = crate::resolve_path(&mount, "sceneryobjects\\STRASSE_DOES_NOT_EXIST\\x.sco");
        assert!(!exists(&sco));
        let sco = crate::resolve_path(&mount, "Sceneryobjects\\straße\\schild.sco");
        assert!(is_file(&sco), "{}", sco.display());
        assert_eq!(read(&sco).unwrap(), b"[mesh]\r\nschild.o3d\r\n");
        let names: Vec<String> = list_dir(&mount).unwrap().into_iter().map(|(n, _)| n.to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"maps".to_string()) && names.contains(&"Sceneryobjects".to_string()), "{names:?}");
        let f = crate::CfgFile::read(&cfg).unwrap();
        assert_eq!(f.lines[1], "Test");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mount_root_of_an_addon_archive() {
        let prefix = |names: &[&str]| ZipArchive::content_prefix(names.iter().copied());
        // a bus: its own Sound and Texture folders do not make it the root
        let bus = ["OMSI 2/Vehicles/MAN_Lion/Lion.bus", "OMSI 2/Vehicles/MAN_Lion/Model/model.cfg", "OMSI 2/Vehicles/MAN_Lion/Script/main.osc", "OMSI 2/Vehicles/MAN_Lion/Sound/sound.cfg", "OMSI 2/Vehicles/MAN_Lion/Texture/body.dds", "Readme.txt"];
        assert_eq!(prefix(&bus), "OMSI 2/");
        // two buses tie between themselves, never with the root
        let two = ["Vehicles/A/Sound/a.wav", "Vehicles/A/Texture/a.dds", "Vehicles/B/Sound/b.wav", "Vehicles/B/Texture/b.dds"];
        assert_eq!(prefix(&two), "");
        // a scenery pack
        let pack = ["Pack v2/Sceneryobjects/Pack/model/a.o3d", "Pack v2/Sceneryobjects/Pack/texture/a.bmp", "Pack v2/Sceneryobjects/Pack/sound/a.wav", "Pack v2/Sceneryobjects/Pack/a.sco"];
        assert_eq!(prefix(&pack), "Pack v2/");
        // a folder with the top-level names wins over one with only the shared ones
        let mixed = ["Extras/Bonus/Sound/x.wav", "Extras/Bonus/Texture/x.dds", "OMSI 2/maps/M/global.cfg"];
        assert_eq!(prefix(&mixed), "OMSI 2/");
        // a texture pack still has its root
        assert_eq!(prefix(&["Winter/Texture/snow.dds", "Winter/readme.txt"]), "Winter/");
        // on a tie, the shallowest, then the first by name
        assert_eq!(prefix(&["b/Vehicles/x.bus", "a/Vehicles/y.bus"]), "a/");
        assert_eq!(prefix(&["docs/readme.txt"]), "");
    }

    /// Archives in the content folder's `Archives` lie inside the content folder, yet a path in
    /// one of them falls back to the archives and roots after it, like any other archive.
    #[test]
    fn archive_inside_the_content_folder() {
        let dir = std::env::temp_dir().join(format!("omsi-cfg-roots-{}", std::process::id()));
        let content = dir.join("content");
        let install = dir.join("install");
        std::fs::create_dir_all(content.join("Archives")).unwrap();
        std::fs::create_dir_all(content.join("Vehicles").join("MyBus")).unwrap();
        std::fs::write(content.join("Vehicles").join("MyBus").join("override.txt"), b"content").unwrap();
        std::fs::create_dir_all(install.join("Vehicles").join("MAN_SD202").join("Sound")).unwrap();
        std::fs::write(install.join("Vehicles").join("MAN_SD202").join("Sound").join("x.wav"), b"stock").unwrap();
        let x = content.join("Archives").join("x.zip");
        write_zip(&x, &[("OMSI 2/Vehicles/MyBus/bus.bus", b"bus".to_vec(), false), ("OMSI 2/Vehicles/MyBus/override.txt", b"zip".to_vec(), false)]);
        let y = content.join("Archives").join("y.zip");
        write_zip(&y, &[("Vehicles/Other/y.txt", b"y".to_vec(), false)]);
        crate::add_content_root(content.clone());
        let mounts = mount_dir_zips(&content.join("Archives"));
        assert_eq!(mounts, vec![x.clone(), y.clone()]);
        crate::add_content_root(install.clone());
        let bus = x.join("Vehicles").join("MyBus");
        assert!(is_file(&crate::resolve_path(&bus, "bus.bus")));
        // the content folder has priority over the archive
        assert_eq!(read(&crate::resolve_path(&bus, "override.txt")).unwrap(), b"content");
        // the installation after it
        let wav = crate::resolve_path(&bus, "..\\MAN_SD202\\Sound\\x.wav");
        assert_eq!(wav, install.join("Vehicles").join("MAN_SD202").join("Sound").join("x.wav"));
        // and an archive after it
        let other = crate::resolve_path(&bus, "..\\other\\Y.TXT");
        assert_eq!(read(&other).unwrap(), b"y", "{}", other.display());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names() {
        assert_eq!(decode_name(&[b'S', b't', b'r', b'a', 0xE1, b'e'], false), "Straße");
        assert_eq!(normalize("a\\B/./c/../D"), "a/b/d");
    }
}
