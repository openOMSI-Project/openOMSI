//! OMSI's text formats.
//!
//! Almost every OMSI content file (`.bus`, `.sco`, `.sli`, `.cfg`, `.hof`, `.map`, …) is a list
//! of lines in which a *keyword* line such as `[mesh]` is followed by a fixed number of parameter
//! lines.  Everything else is free text and ignored.  The rules, read off the loaders of
//! `Omsi.exe` 2.2.032 (every one compares the line it read with its keyword literals by plain
//! string equality) and checked against the stock and installed content, are:
//!
//! * a keyword is recognised only when the whole line is `[name]`: an indented keyword
//!   (the stock files' help texts, the F90 lorry's disabled second rear axle, the EN92's
//!   disabled fourth door-slam sound) is free text, and so is one followed by spaces;
//! * the name is spelled as the original spells it (`[matl_noZwrite]`, not `[matl_nozWrite]`);
//!   names the original does not know are compared without regard to case;
//! * two loaders differ: `.hof` cuts trailing tabs, spaces and quotes off every line
//!   (spreadsheet exports), `ailists.cfg` lower-cases its lines ([`KeywordRule`]);
//! * the main content loaders skip `-<DISABLED>-` … `-<ENABLED>-` between blocks
//!   ([`CfgReader::disabled_blocks`]);
//! * parameters are read verbatim, one per line, empty lines included.
//!
//! Text is read in the code page it was written in: a byte-order mark says so, otherwise
//! [`codepage::detect`] tells UTF-8, Windows-1251 (Russian mods), 1250 (Polish, Czech) and
//! the stock content's Windows-1252 apart.

use std::path::{Path, PathBuf};

pub mod codepage;
mod keywords;
pub mod number;
pub mod vfs;
pub mod install_search;
pub use install_search::find_original_install;
pub use number::{parse_f32, parse_f64, parse_i32, parse_i64};
pub use vfs::{add_content_zip, mount_zip};

/// Decode raw file bytes with OMSI's encoding rules.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let (s, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(&bytes[2..]);
        s.into_owned()
    } else if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        String::from_utf8_lossy(&bytes[3..]).into_owned()
    } else {
        codepage::decode(bytes)
    }
}

/// Encode a string the way OMSI writes it (Windows-1252, CR LF).
pub fn encode_text(s: &str) -> Vec<u8> {
    let (b, _, _) = encoding_rs::WINDOWS_1252.encode(s);
    b.into_owned()
}

/// Split into lines. Handles CR LF, LF and bare CR.
pub fn split_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum CfgError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// A loaded text file, split into lines.
#[derive(Debug, Clone)]
pub struct CfgFile {
    pub path: PathBuf,
    pub lines: Vec<String>,
}

impl CfgFile {
    pub fn read(path: impl AsRef<Path>) -> Result<Self, CfgError> {
        let path = path.as_ref();
        let bytes = vfs::read(path).map_err(|e| CfgError::Io { path: path.to_path_buf(), source: e })?;
        Ok(Self::from_bytes(path, &bytes))
    }

    pub fn from_bytes(path: impl AsRef<Path>, bytes: &[u8]) -> Self {
        Self { path: path.as_ref().to_path_buf(), lines: split_lines(&decode_text(bytes)) }
    }

    pub fn from_str(path: impl AsRef<Path>, text: &str) -> Self {
        Self { path: path.as_ref().to_path_buf(), lines: split_lines(text) }
    }

    pub fn reader(&self) -> CfgReader<'_> {
        CfgReader { file: self, pos: 0, block_line: 0, rule: KeywordRule::Exact, disabled_blocks: false }
    }

    /// Directory the file lives in (for resolving relative paths in it).
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }
}

/// How a loader of the original tells a keyword line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeywordRule {
    /// The line is exactly `[name]` with the original's spelling (almost every loader).
    #[default]
    Exact,
    /// Trailing tabs, spaces, line breaks and double quotes are cut off first (`.hof`, whose
    /// stock files are spreadsheet exports: `[infosystem_trip]` followed by twelve tabs).
    TrimEnd,
    /// The line is lower-cased first (`ailists.cfg`).
    AnyCase,
}

/// What `.hof` cuts off the end of every line.
fn hof_trailing(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' ' | '"')
}

/// The original's spelling of a keyword it knows (by the name in any case).
fn omsi_spelling(name: &str) -> Option<&'static str> {
    let lower;
    let key = if name.bytes().any(|b| b.is_ascii_uppercase()) {
        lower = name.to_ascii_lowercase();
        lower.as_str()
    } else {
        name
    };
    if let Ok(i) = keywords::MIXED_CASE.binary_search_by(|k| cmp_lower(k, key)) {
        return Some(keywords::MIXED_CASE[i]);
    }
    keywords::LOWER_CASE.binary_search(&key).ok().map(|i| keywords::LOWER_CASE[i])
}

/// `a.to_ascii_lowercase().cmp(b)` without allocating.
fn cmp_lower(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes())
}

/// Test whether a line is a keyword line and return the keyword (without brackets, as
/// written): the whole line is `[name]`, and `name` is spelled like the original spells it
/// when the original knows it.
pub fn keyword_of(line: &str) -> Option<&str> {
    keyword_with(line, KeywordRule::Exact)
}

/// [`keyword_of`] for a loader with its own rule.
pub fn keyword_with(line: &str, rule: KeywordRule) -> Option<&str> {
    let t = if rule == KeywordRule::TrimEnd { line.trim_end_matches(hof_trailing) } else { line };
    if t.len() < 2 || t.as_bytes()[0] != b'[' || !t.ends_with(']') {
        return None;
    }
    let inner = &t[1..t.len() - 1];
    // Keywords never contain a second bracket; free text like "[mesh] macht ..." is never a
    // keyword because it does not end with ']'.
    if inner.is_empty() || inner.contains('[') || inner.contains(']') {
        return None;
    }
    if rule != KeywordRule::AnyCase {
        if let Some(spelling) = omsi_spelling(inner) {
            if spelling != inner {
                return None;
            }
        }
    }
    Some(inner)
}

/// What [`CfgReader::next_entry`] stopped at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry<'t> {
    /// A keyword line: the keyword in lower case.
    Keyword(String),
    /// A line that is exactly one of the bare tokens asked for.
    Token(&'t str),
}

/// Sequential reader over a [`CfgFile`] with the original semantics.
#[derive(Clone)]
pub struct CfgReader<'a> {
    file: &'a CfgFile,
    pos: usize,
    block_line: usize,
    rule: KeywordRule,
    disabled_blocks: bool,
}

impl<'a> CfgReader<'a> {
    /// Read keyword lines by `rule` instead of [`KeywordRule::Exact`].
    pub fn with_rule(mut self, rule: KeywordRule) -> Self {
        self.rule = rule;
        self
    }

    /// Skip `-<DISABLED>-` … `-<ENABLED>-` between blocks, as the original's loaders of
    /// `.bus`/`.ovh`, `.sco`, `model.cfg`, `passengercabin.cfg`, `paths.cfg`, `.hum` and
    /// `envir.cfg` do (the marker lines must stand alone, exactly so; a marker read as a
    /// parameter is a parameter). Mods switch whole meshes off this way.
    pub fn disabled_blocks(mut self) -> Self {
        self.disabled_blocks = true;
        self
    }

    /// Whether `line` is a keyword line by this reader's rule.
    pub fn keyword(&self, line: &'a str) -> Option<&'a str> {
        keyword_with(line, self.rule)
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }

    /// Current line index (0-based) of the *next* line to be read.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// 1-based line number of the keyword that opened the current block (for messages).
    pub fn block_line(&self) -> usize {
        self.block_line
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.file.lines.len());
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.file.lines.len()
    }

    /// Advance to the next keyword line and return the keyword in lower case.
    pub fn next_keyword(&mut self) -> Option<String> {
        match self.next_entry(&[]) {
            Some(Entry::Keyword(k)) => Some(k),
            _ => None,
        }
    }

    /// Advance to the next keyword line or the next line that is exactly one of `tokens`.
    /// The original's loaders see sub-commands such as `attach_trans` or `anim_rot` the way
    /// they see keywords: as a line of their own anywhere between blocks, spelled exactly so.
    pub fn next_entry<'t>(&mut self, tokens: &[&'t str]) -> Option<Entry<'t>> {
        while self.pos < self.file.lines.len() {
            let line = self.file.lines[self.pos].as_str();
            self.pos += 1;
            if let Some(k) = keyword_with(line, self.rule) {
                self.block_line = self.pos;
                return Some(Entry::Keyword(k.to_ascii_lowercase()));
            }
            if let Some(t) = tokens.iter().find(|t| **t == line) {
                self.block_line = self.pos;
                return Some(Entry::Token(t));
            }
            if self.disabled_blocks && line == "-<DISABLED>-" {
                while self.pos < self.file.lines.len() {
                    self.pos += 1;
                    if self.file.lines[self.pos - 1] == "-<ENABLED>-" {
                        break;
                    }
                }
            }
        }
        None
    }

    /// Peek at the next keyword without consuming anything.
    pub fn peek_keyword(&self) -> Option<String> {
        let mut c = self.clone();
        c.next_keyword()
    }

    /// Read the next raw parameter line. Missing lines at EOF read as empty strings, which is
    /// what the original does with a truncated file.
    pub fn line(&mut self) -> &'a str {
        if self.pos < self.file.lines.len() {
            let l = &self.file.lines[self.pos];
            self.pos += 1;
            l.as_str()
        } else {
            self.pos = self.file.lines.len() + 1;
            ""
        }
    }

    /// Like [`line`](Self::line) but with trailing whitespace removed.
    pub fn str(&mut self) -> &'a str {
        self.line().trim_end()
    }

    /// Trimmed on both sides (numbers, identifiers).
    pub fn word(&mut self) -> &'a str {
        self.line().trim()
    }

    pub fn f64(&mut self) -> f64 {
        parse_f64(self.line())
    }

    pub fn f32(&mut self) -> f32 {
        parse_f32(self.line())
    }

    pub fn i32(&mut self) -> i32 {
        parse_i32(self.line())
    }

    /// `n` integers, then any further whole-number lines that follow at once (a blank line,
    /// a keyword or anything else ends the list): OMSI's fixed-length lists read as always,
    /// and a file may give more (openOMSI: `[illumination_interior]` with more than four
    /// lamps).
    pub fn i32_list(&mut self, n: usize) -> Vec<i32> {
        let mut v: Vec<i32> = (0..n).map(|_| self.i32()).collect();
        while self.pos < self.file.lines.len() {
            let l = self.file.lines[self.pos].trim();
            let whole = !l.is_empty() && l.strip_prefix('-').unwrap_or(l).chars().all(|c| c.is_ascii_digit());
            if !whole || keyword_with(self.file.lines[self.pos].as_str(), self.rule).is_some() {
                break;
            }
            v.push(parse_i32(l));
            self.pos += 1;
        }
        v
    }

    pub fn i64(&mut self) -> i64 {
        parse_i64(self.line())
    }

    pub fn u32(&mut self) -> u32 {
        parse_i64(self.line()).max(0) as u32
    }

    pub fn usize(&mut self) -> usize {
        parse_i64(self.line()).max(0) as usize
    }

    /// `1`/`true` → true; anything else false.
    pub fn bool(&mut self) -> bool {
        let w = self.word();
        w == "1" || w.eq_ignore_ascii_case("true")
    }

    /// Read `n` lines as `f32`.
    pub fn f32s<const N: usize>(&mut self) -> [f32; N] {
        let mut a = [0.0; N];
        for v in a.iter_mut() {
            *v = self.f32();
        }
        a
    }

    pub fn f64s<const N: usize>(&mut self) -> [f64; N] {
        let mut a = [0.0; N];
        for v in a.iter_mut() {
            *v = self.f64();
        }
        a
    }

    /// Read all lines up to (not including) a line that is `terminator` (by the reader's
    /// rule, like a keyword). Used for `[description] … [end]`.
    pub fn until(&mut self, terminator: &str) -> Vec<&'a str> {
        let mut out = Vec::new();
        while self.pos < self.file.lines.len() {
            let l = self.file.lines[self.pos].as_str();
            self.pos += 1;
            let hit = match self.rule {
                KeywordRule::Exact => l == terminator,
                KeywordRule::TrimEnd => l.trim_end_matches(hof_trailing) == terminator,
                KeywordRule::AnyCase => l.eq_ignore_ascii_case(terminator),
            };
            if hit {
                break;
            }
            out.push(l);
        }
        out
    }

    /// All remaining lines that are not keywords, until the next keyword (not consumed).
    pub fn rest_of_block(&mut self) -> Vec<&'a str> {
        let mut out = Vec::new();
        while self.pos < self.file.lines.len() {
            let l = self.file.lines[self.pos].as_str();
            if keyword_with(l, self.rule).is_some() {
                break;
            }
            self.pos += 1;
            out.push(l);
        }
        out
    }

    /// Raw access to the lines.
    pub fn lines(&self) -> &'a [String] {
        &self.file.lines
    }
}

/// Convert an OMSI content path (backslashes, case-insensitive on Windows) to a real path
/// relative to `base`, resolving the case of every component on case-sensitive file systems.
///
/// The components of a content path the way Windows reads them: `\` and `/` both separate,
/// empty and `.` components vanish, a folder name loses one trailing dot and the last name
/// all trailing dots and spaces (Win32 path normalisation: an Ahlheim mesh asks for
/// `anz-oben.jpg.`, which Windows opens as `anz-oben.jpg`). `..` is left for the caller.
pub fn windows_components(rel: &str) -> Vec<&str> {
    let parts: Vec<&str> = rel.trim().split(['/', '\\']).collect();
    let n = parts.len();
    let mut out = Vec::with_capacity(n);
    for (i, c) in parts.into_iter().enumerate() {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            out.push(c);
            continue;
        }
        // a path ending in a separator has an empty last part, so its last folder counts
        // as a folder here
        let c = if i + 1 == n { c.trim_end_matches(['.', ' ']) } else { c.strip_suffix('.').unwrap_or(c) };
        if !c.is_empty() {
            out.push(c);
        }
    }
    out
}

/// How names compare, as on Windows: without regard to case (letters beyond ASCII too -
/// `Bahnübergang` is `BAHNÜBERGANG`).
fn name_key(name: &str) -> String {
    if name.is_ascii() {
        name.to_ascii_lowercase()
    } else {
        // (composed: a Russian stop's announcement `Лесной.wav` unpacked on a Mac is often
        // stored with `й` as `и` + a combining breve, and was never found)
        use unicode_normalization::UnicodeNormalization;
        name.nfc().collect::<String>().to_lowercase()
    }
}

type Listing = std::sync::Arc<std::collections::HashMap<String, std::ffi::OsString>>;
static LISTINGS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, Listing>>> = std::sync::OnceLock::new();
/// Bumped whenever content may have come or gone (a root added or taken away, a mod
/// installed while the game runs): the caches of what is where start afresh.
static CONTENT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The content changed: directory listings are read again, and lookups that missed before
/// (see `content_generation`) are tried again.
pub fn content_changed() {
    CONTENT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(l) = LISTINGS.get() {
        l.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// How often the content changed so far (a cache of lookups keeps its misses only as
/// long as this stays the same).
pub fn content_generation() -> u64 {
    CONTENT_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// The entries of a directory by [`name_key`], listed once.
fn dir_listing(dir: &Path) -> Listing {
    let cache = LISTINGS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(l) = cache.lock().unwrap().get(dir) {
        return l.clone();
    }
    let mut map = std::collections::HashMap::new();
    let entries = vfs::list_dir(dir).unwrap_or_default();
    for (name, _) in &entries {
        map.insert(name_key(&name.to_string_lossy()), name.clone());
    }
    // the other spellings of names beyond ASCII (see `codepage::name_variants`), behind
    // every real name
    for (name, _) in &entries {
        for v in codepage::name_variants(&name.to_string_lossy()) {
            map.entry(name_key(&v)).or_insert_with(|| name.clone());
        }
    }
    let l = std::sync::Arc::new(map);
    cache.lock().unwrap().insert(dir.to_path_buf(), l.clone());
    l
}

/// The entry of `dir` that `name` means: the same name without regard to case, or one of
/// its other spellings (a name beyond ASCII read in another code page on the way).
fn find_in_dir(dir: &Path, name: &str) -> Option<std::ffi::OsString> {
    let listing = dir_listing(dir);
    if let Some(n) = listing.get(&name_key(name)) {
        return Some(n.clone());
    }
    codepage::name_variants(name)
        .iter()
        .find_map(|v| listing.get(&name_key(v)).cloned())
}

/// Content roots, highest priority first: the game's own content folder next to its
/// binary (where mods are installed) and the OMSI 2 installation. A path asked for
/// relative to a folder of one root is looked for at the same place under the others,
/// so a mod's `Vehicles\\Foo\\foo.bus` or a replacement texture is found as if it had
/// been copied into the original installation.
static CONTENT_ROOTS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Register a content root. Roots are searched in the order they were added.
pub fn add_content_root(root: PathBuf) {
    let mut r = CONTENT_ROOTS.lock().unwrap();
    if !r.iter().any(|x| x == &root) {
        r.push(root);
        drop(r);
        content_changed();
    }
}

/// Register a content root to be searched just before `before` (at the end when `before`
/// is not a root): an archive installed while the launcher runs goes in front of the
/// OMSI installation, where the game will have it on its next start.
pub fn add_content_root_before(root: PathBuf, before: &Path) {
    let mut r = CONTENT_ROOTS.lock().unwrap();
    if r.iter().any(|x| x == &root) {
        return;
    }
    match r.iter().position(|x| x == before) {
        Some(i) => r.insert(i, root),
        None => r.push(root),
    }
    drop(r);
    content_changed();
}

/// Register a content root searched before every other one (the content a LAN host sent
/// for the session: its files are the host's versions, and must win).
pub fn add_content_root_first(root: PathBuf) {
    let mut r = CONTENT_ROOTS.lock().unwrap();
    r.retain(|x| x != &root);
    r.insert(0, root);
    drop(r);
    content_changed();
}

/// Take a content root away again (a LAN session's content, when the session ends).
pub fn remove_content_root(root: &Path) {
    CONTENT_ROOTS.lock().unwrap().retain(|x| x != root);
    content_changed();
}

static SANDBOX_ROOTS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Mark a content root as a sandbox: content received from another machine. It is read as
/// data (maps, vehicles, objects) like any other root, but nothing that runs code comes from
/// it - no plugins are loaded from a sandbox.
pub fn mark_sandbox(root: PathBuf) {
    let mut r = SANDBOX_ROOTS.lock().unwrap();
    if !r.contains(&root) {
        r.push(root);
    }
}

/// Does `path` lie in a sandbox root (see [`mark_sandbox`])?
pub fn is_sandbox(path: &Path) -> bool {
    SANDBOX_ROOTS.lock().unwrap().iter().any(|r| path.starts_with(r))
}

/// The first content root (in the order they are searched) that has `rel`, and the file
/// or folder found there (case-insensitive, as Windows reads the folder).
pub fn find_in_roots(rel: &str) -> Option<(PathBuf, PathBuf)> {
    let comps = windows_components(rel);
    content_roots().into_iter().find_map(|r| resolve_existing(&r, &comps).map(|p| (r, p)))
}

pub fn content_roots() -> Vec<PathBuf> {
    CONTENT_ROOTS.lock().unwrap().clone()
}

/// Every content root's copy of the folder `dir` (which lies under one of the roots) that
/// exists, highest priority first; just `dir` when it is under no root. A mod that adds
/// files to a stock folder (a repaint's `.cti` next to the original ones) puts them into
/// the same folder under the content folder (or an archive in `<content>/Archives`), and a
/// listing must see both.
pub fn mirrored_dirs(dir: &Path) -> Vec<PathBuf> {
    let roots = content_roots();
    let Some(suffix) = owner_root(dir, &roots).and_then(|r| dir.strip_prefix(r).ok()) else { return vec![dir.to_path_buf()] };
    // a real folder: its names as they are
    let rel: Vec<String> = suffix.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let rel: Vec<&str> = rel.iter().map(|s| s.as_str()).collect();
    let mut out: Vec<PathBuf> = Vec::new();
    for r in &roots {
        let found = if rel.is_empty() { Some(r.clone()) } else { resolve_existing(r, &rel) };
        if let Some(p) = found.filter(|p| vfs::is_dir(p)) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    if out.is_empty() {
        out.push(dir.to_path_buf());
    }
    out
}

/// The content root `path` belongs to: the deepest one that contains it. An archive
/// mounted from `<content>/Archives` lies inside the content folder, and a path inside the
/// archive is the archive's, not the content folder's.
fn owner_root<'a>(path: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.components().count())
}

/// The same relative location of `base` under `root` (None if `base` belongs to `root`
/// itself, or to no root at all).
fn mirrored_base(base: &Path, root: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let owner = owner_root(base, roots)?;
    if owner == root {
        return None;
    }
    base.strip_prefix(owner).ok().map(|suffix| root.join(suffix))
}

/// Case-insensitive walk of the components `rel` from `base` using the cached directory
/// listings only; None as soon as a component does not exist.
fn resolve_existing(base: &Path, rel: &[&str]) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for comp in rel {
        if comp.is_empty() || *comp == "." {
            continue;
        }
        if *comp == ".." {
            cur.pop();
            continue;
        }
        let found = find_in_dir(&cur, comp).map(|n| cur.join(n))?;
        cur = found;
    }
    Some(cur)
}

/// The vehicle pack (`Vehicles/<folder>`, relative to its root) that `rel` from `base` lands
/// in, when it does: the files of one vehicle refer to each other relative to its folder.
fn vehicle_package(base: &Path, owner: &Path, rel: &[&str]) -> Option<String> {
    let suffix = base.strip_prefix(owner).ok()?;
    let mut parts: Vec<String> = suffix
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for comp in rel {
        if *comp == ".." {
            parts.pop();
        } else {
            parts.push(comp.to_string());
        }
    }
    if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("Vehicles") {
        Some(format!("Vehicles/{}", parts[1]))
    } else {
        None
    }
}

/// The vehicle definitions (`.bus`, `.ovh`, `.zug`) lying directly in a pack folder.
fn package_definitions(root: &Path, package: &str) -> std::collections::HashSet<String> {
    let comps: Vec<&str> = package.split('/').collect();
    let Some(dir) = resolve_existing(root, &comps) else {
        return Default::default();
    };
    vfs::list_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, is_dir)| !is_dir)
        .map(|(n, _)| name_key(&n.to_string_lossy()))
        .filter(|n| n.ends_with(".bus") || n.ends_with(".ovh") || n.ends_with(".zug"))
        .collect()
}

/// Whether the copy of vehicle pack `package` under the higher-priority root `over` patches
/// the one under `owner` (the root the vehicle was loaded from), i.e. its files replace
/// the owner's the way files copied over an installation would.
///
/// That holds for a patch (no vehicle definitions of its own, or only ones the owner's pack
/// has as well). A pack with vehicles the owner's copy does not know is another *version*
/// of the pack - a map archive bundles its own Citaro Facelift next to the one installed
/// in the content folder, with scripts, varlists and constfiles that belong together - and
/// taking single files from it broke the bus it did not come from (the Ahlheim Citaro's
/// VDV and IBIS scripts ran on the installed pack's varlists, missing half their
/// variables, and every display stayed dark). Such a copy is only asked for files the
/// owner's pack does not have.
fn overlays_package(over: &Path, owner: &Path, package: &str) -> bool {
    type Key = (PathBuf, PathBuf, String);
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<Key, bool>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (over.to_path_buf(), owner.to_path_buf(), package.to_ascii_lowercase());
    if let Some(v) = cache.lock().unwrap().get(&key) {
        return *v;
    }
    // (a root without the pack at all is only where the lookup starts: `--bus` is resolved
    // from the installation's root)
    let comps: Vec<&str> = package.split('/').collect();
    let owner_has = resolve_existing(owner, &comps).map(|d| vfs::is_dir(&d)).unwrap_or(false);
    let theirs = package_definitions(over, package);
    let ok = !owner_has || theirs.is_empty() || theirs.is_subset(&package_definitions(owner, package));
    cache.lock().unwrap().insert(key, ok);
    ok
}

/// The vehicle pack a missing file belongs to, when that whole pack is installed in no
/// content root: a mod that borrows parts from another (the Ahlheim Citaro's Faremaster
/// ticket machine and dashboard come from `Vehicles/Urbino_II`) is missing them because the
/// other pack is not installed, not because of a broken path, and the player can fix that.
pub fn missing_vehicle_pack(path: &Path) -> Option<String> {
    let roots = content_roots();
    let owner = owner_root(path, &roots)?.clone();
    let suffix = path.strip_prefix(&owner).ok()?;
    let mut comps = suffix.components().filter_map(|c| match c {
        std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
        _ => None,
    });
    let first = comps.next()?;
    let pack = comps.next()?;
    if !first.eq_ignore_ascii_case("Vehicles") || comps.next().is_none() {
        return None;
    }
    let installed = roots.iter().any(|r| {
        resolve_existing(r, &["Vehicles", pack.as_str()]).map(|d| vfs::is_dir(&d)).unwrap_or(false)
    });
    (!installed).then_some(pack)
}

/// Every content root's version of a folder given relative to a root (existing ones only),
/// highest priority first - for listing maps, vehicles, weathers across the installation
/// and the installed mods.
pub fn content_dirs(rel: &str) -> Vec<PathBuf> {
    let rel = windows_components(rel);
    content_roots().iter().filter_map(|r| resolve_existing(r, &rel)).filter(|p| vfs::is_dir(p)).collect()
}

/// Directory entries of `rel` (relative to a content root) merged over all roots; an
/// entry of a higher-priority root hides the same name lower down.
pub fn read_dir_merged(rel: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for d in content_dirs(rel) {
        for (name, _) in vfs::list_dir(&d).unwrap_or_default() {
            if seen.insert(name_key(&name.to_string_lossy())) {
                out.push(d.join(name));
            }
        }
    }
    out
}

/// The file a content file names (`rel`, relative to the folder `base`), found the way
/// Windows finds it: see [`windows_components`] for the spelling rules, names compare without
/// regard to case, and a mod's copy under a content root of higher priority comes first.
pub fn resolve_path(base: &Path, rel: &str) -> PathBuf {
    let rel = windows_components(rel);
    let rel = rel.as_slice();
    // a mod's copy first: the same place under a content root of higher priority than the
    // one `base` belongs to
    let roots = content_roots();
    let owner = if roots.len() > 1 { owner_root(base, &roots).cloned() } else { None };
    if let Some(owner) = &owner {
        let package = vehicle_package(base, owner, rel);
        for r in roots.iter().take_while(|r| *r != owner) {
            // another version of the same vehicle pack does not patch this one (see
            // `overlays_package`); it is only asked below, for what this one lacks
            if let Some(pkg) = &package {
                if !overlays_package(r, owner, pkg) {
                    continue;
                }
            }
            if let Some(mb) = mirrored_base(base, r, &roots) {
                if let Some(p) = resolve_existing(&mb, rel) {
                    return p;
                }
            }
        }
    }
    let mut cur = base.to_path_buf();
    for comp in rel {
        if *comp == ".." {
            cur.pop();
            continue;
        }
        let direct = cur.join(comp);
        if vfs::exists(&direct) {
            cur = direct;
            continue;
        }
        // case-insensitive lookup through a per-directory listing kept from the first
        // time (scanning the folder afresh for every miss made texture lookups crawl)
        let found = find_in_dir(&cur, comp).map(|n| cur.join(n));
        cur = found.unwrap_or(direct);
    }
    if let (Some(owner), false) = (&owner, vfs::exists(&cur)) {
        // not in `base`'s root either: another root may still have it - a lower-priority
        // one, or another version of the pack that was passed over above
        for r in roots.iter().filter(|r| *r != owner) {
            if let Some(mb) = mirrored_base(base, r, &roots) {
                if let Some(p) = resolve_existing(&mb, rel) {
                    return p;
                }
            }
        }
    }
    cur
}

/// What an installation of the original OMSI 2 must have for openOMSI to run on it: the
/// game itself and the stock content every player gets with it (both stock maps, the stock
/// buses, people, fonts, weather, inputs). Any copy of OMSI 2 has them, whatever version or
/// shop it came from; a folder that lacks them (openOMSI's own content folder, a mod pack,
/// a half-copied installation) is not a base to play on - everybody plays on the same
/// original content.
pub const ORIGINAL_ESSENTIALS: &[&str] = &[
    "Omsi.exe",
    "maps/Grundorf/global.cfg",
    "maps/Berlin-Spandau/global.cfg",
    "Vehicles/MAN_SD200",
    "Vehicles/MAN_SD202",
    "Vehicles/MAN_NL_NG",
    "Sceneryobjects",
    "Splines",
    "Texture",
    "Fonts",
    "Humans",
    "Weather",
    "Inputs",
    "envir.cfg",
];

/// The key assignment of the OMSI installation at `root`: its `Inputs/keyboard.cfg`, or
/// where it has none (it is the player's own, and not every copy comes with one) the
/// standard `keyboard_reset.cfg`, which OMSI loads then too.
pub fn original_keyboard_cfg(root: &Path) -> PathBuf {
    resolve_existing(root, &["Inputs", "keyboard.cfg"])
        .or_else(|| resolve_existing(root, &["Inputs", "keyboard_reset.cfg"]))
        .unwrap_or_else(|| root.join("Inputs").join("keyboard.cfg"))
}

/// The essentials (see [`ORIGINAL_ESSENTIALS`]) that `root` lacks; empty for a complete
/// original installation. openOMSI's content folder never counts as one.
pub fn missing_original_essentials(root: &Path) -> Vec<String> {
    // (a folder with Omsi.exe in it is the game's, even marked: openOMSI unpacked into the OMSI
    // folder made it its content folder once - see `content_folder_of` - and every start after
    // that said the game was not there)
    let marked = root.join(CONTENT_MARKER).exists() || root.join(LEGACY_CONTENT_MARKER).exists();
    if marked && resolve_existing(root, &["Omsi.exe"]).is_none() {
        return vec![format!("{} (this is the openOMSI content folder, not the original game)", root.display())];
    }
    ORIGINAL_ESSENTIALS
        .iter()
        .filter(|rel| {
            let comps: Vec<&str> = rel.split('/').collect();
            // case-insensitive, as Windows reads the folder
            resolve_existing(root, &comps).is_none()
        })
        .map(|s| s.to_string())
        .collect()
}

/// The folders an OMSI 2 installation has (as the original spells them). A content
/// folder of openOMSI is laid out the same way, so a mod is installed by putting its
/// folders here - and the game finds them exactly as the original would.
pub const CONTENT_FOLDERS: &[&str] = &[
    "Vehicles", "maps", "Sceneryobjects", "Splines", "Texture", "Fonts", "Plugins", "TicketPacks", "Drivers", "Weather", "Announcements", "Humans", "Money", "Scripts", "Trains", "Situations", "Inputs", "Sound",
];

/// Marker file of an openOMSI content folder (so it is never mistaken for the OMSI 2
/// installation itself).
pub const CONTENT_MARKER: &str = ".openomsi-content";
/// The marker of a content folder made before the project was called openOMSI.
pub const LEGACY_CONTENT_MARKER: &str = ".omsi-rewrite-content";

/// Move the data of a version from before the rename (`~/.omsi-rewrite`,
/// `~/.omsi-rewrite-root`) to its new place (`~/.openomsi`, `~/.openomsi-root`), once.
/// (First thing at the start of every program: an unusable `HOME` is dropped here, see
/// [`drop_unusable_home`].)
pub fn migrate_legacy_data_dir() {
    drop_unusable_home();
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from) else {
        return;
    };
    for (old, new) in [(".omsi-rewrite", ".openomsi"), (".omsi-rewrite-root", ".openomsi-root")] {
        let (old, new) = (home.join(old), home.join(new));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// On Windows a `HOME` variable some other program set for itself (a Unix-style path, a
/// network drive that is not connected) is no folder to keep openOMSI's data in: the
/// launcher's settings went nowhere, and the OMSI folder chosen under Setup was forgotten
/// as soon as it was saved - the lists stayed empty. Such a `HOME` is dropped for this
/// program (and the game it starts), which then uses `USERPROFILE` as without one.
pub fn drop_unusable_home() {
    if !cfg!(windows) {
        return;
    }
    let Some(h) = std::env::var_os("HOME") else { return };
    let p = std::path::PathBuf::from(&h);
    let usable = p.is_absolute() && p.is_dir() && std::fs::create_dir_all(p.join(".openomsi")).is_ok();
    if !usable && std::env::var_os("USERPROFILE").is_some() {
        std::env::remove_var("HOME");
    }
}

/// openOMSI's content folder for a program in `dir`: that folder - unless it is the original
/// OMSI 2 folder itself (openOMSI unpacked into it), which openOMSI never writes to: then the
/// `openOMSI` folder inside it.
pub fn content_folder_of(dir: &Path) -> PathBuf {
    if resolve_existing(dir, &["Omsi.exe"]).is_some() && resolve_existing(dir, &["maps"]).is_some() {
        dir.join("openOMSI")
    } else {
        dir.to_path_buf()
    }
}

/// Check if a directory is writable by attempting to create and remove a probe file.
pub fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".openomsi-write-test");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Create the content folder layout at `dir` (idempotent).
pub fn ensure_content_layout(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for f in CONTENT_FOLDERS {
        std::fs::create_dir_all(dir.join(f))?;
    }
    std::fs::create_dir_all(dir.join("Mods"))?;
    let marker = dir.join(CONTENT_MARKER);
    if !marker.exists() {
        std::fs::write(&marker, "This folder holds openOMSI's own content and installed mods, laid out like the original game.\nDrop a mod folder or zip into Mods/ and the launcher sorts it into place.\n")?;
    }
    let readme = dir.join("Mods").join("README.txt");
    if !readme.exists() {
        std::fs::write(&readme, "Put a mod here (a folder or a .zip) and start the launcher: it works out what the mod is\n(a bus, a map, scenery objects, splines, textures, fonts ...) and sorts it into the folders\nnext to this one. The original OMSI 2 folder is never written to.\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_check() {
        let tmp = std::env::temp_dir();
        assert!(is_writable(&tmp));
        let nonexistent = tmp.join("nonexistent_subfolder_xyz_123");
        assert!(!is_writable(&nonexistent));
    }

    #[test]
    fn keywords() {
        assert_eq!(keyword_of("[mesh]"), Some("mesh"));
        assert_eq!(keyword_of("\t[mesh]"), None);
        assert_eq!(keyword_of("        [NightMapMode]"), None);
        assert_eq!(keyword_of("[mesh] "), None);
        assert_eq!(keyword_of("[busstop] macht aus dem Objekt"), None);
        assert_eq!(keyword_of("[coupling_front] / [coupling_back]"), None);
        // the original's spelling counts for its own keywords …
        assert_eq!(keyword_of("[matl_noZwrite]"), Some("matl_noZwrite"));
        assert_eq!(keyword_of("[matl_nozWrite]"), None);
        assert_eq!(keyword_of("[NoDistanceCheck]"), None);
        assert_eq!(keyword_of("[Mesh]"), None);
        assert_eq!(keyword_of("[LOD]"), Some("LOD"));
        assert_eq!(keyword_of("[lod]"), None);
        // … not for the rest
        assert_eq!(keyword_of("[Infosystem_Busstop]"), Some("Infosystem_Busstop"));
        // .hof: spreadsheet exports
        assert_eq!(keyword_with("[global_strings]\t\t\t", KeywordRule::TrimEnd), Some("global_strings"));
        assert_eq!(keyword_with("[end]\"\t", KeywordRule::TrimEnd), Some("end"));
        assert_eq!(keyword_with("\t[end]", KeywordRule::TrimEnd), None);
        assert_eq!(keyword_of("[global_strings]\t"), None);
        // ailists.cfg
        assert_eq!(keyword_with("[AIGroup_2]", KeywordRule::AnyCase), Some("AIGroup_2"));
        assert_eq!(keyword_with(" [aigroup_2]", KeywordRule::AnyCase), None);
        for k in keywords::MIXED_CASE.iter().chain(keywords::LOWER_CASE) {
            assert_eq!(omsi_spelling(&k.to_ascii_uppercase()), Some(*k));
        }
    }

    /// The stock files' indented keywords are the original's help texts and switched-off
    /// blocks: the SD202 cabin's `[exit]` help line adds no exit.
    #[test]
    fn indented_help_text() {
        let text = "\t[entry]\t\tdefiniert Eingang\r\n\t[exit]\r\n\tnum\t\tanalog f\u{fc}r Ausgang\r\n\r\n###\r\n[exit]\r\n20\r\n";
        let f = CfgFile::from_str("passengercabin.cfg", text);
        let mut r = f.reader();
        assert_eq!(r.next_keyword().as_deref(), Some("exit"));
        assert_eq!(r.i32(), 20);
        assert_eq!(r.next_keyword(), None);
    }

    #[test]
    fn disabled_blocks() {
        let text = "[mesh]\na.o3d\n-<DISABLED>-\n[mesh]\nb.o3d\n-<ENABLED>-\n[mesh]\n-<DISABLED>-\n[mesh]\nd.o3d\n";
        let f = CfgFile::from_str("model.cfg", text);
        let mut r = f.reader().disabled_blocks();
        let mut meshes = Vec::new();
        while let Some(k) = r.next_keyword() {
            assert_eq!(k, "mesh");
            meshes.push(r.str().to_string());
        }
        // a marker read as a parameter is a parameter; an unclosed block runs to the end
        assert_eq!(meshes, vec!["a.o3d", "-<DISABLED>-", "d.o3d"]);
        // loaders without the feature see the blocks
        let mut r = f.reader();
        let mut n = 0;
        while r.next_keyword().is_some() {
            n += 1;
        }
        assert_eq!(n, 4);
        let f = CfgFile::from_str("model.cfg", "-<DISABLED>-\n[mesh]\nx\n -<ENABLED>-\n[mesh]\ny\n");
        assert_eq!(f.reader().disabled_blocks().next_keyword(), None);
    }

    #[test]
    fn bare_tokens() {
        let text = "[new_attachment]\n\ntext\n[complexity]\n2\n\nattach_trans\n1\n2\n3\n\tattach_rot_x\nAttach_rot_y\n";
        let f = CfgFile::from_str("x.sco", text);
        let mut r = f.reader();
        const T: &[&str] = &["attach_trans", "attach_rot_x", "attach_rot_y"];
        assert_eq!(r.next_entry(T), Some(Entry::Keyword("new_attachment".into())));
        assert_eq!(r.next_entry(T), Some(Entry::Keyword("complexity".into())));
        assert_eq!(r.i32(), 2);
        assert_eq!(r.next_entry(T), Some(Entry::Token("attach_trans")));
        assert_eq!(r.f32s::<3>(), [1.0, 2.0, 3.0]);
        assert_eq!(r.next_entry(T), None);
    }

    #[test]
    fn until_follows_the_rule() {
        let f = CfgFile::from_str("x.owt", "[description]\na\n[END]\n[end] \nb\n[end]\nc\n");
        let mut r = f.reader();
        r.next_keyword();
        assert_eq!(r.until("[end]"), vec!["a", "[END]", "[end] ", "b"]);
        let f = CfgFile::from_str("x.hof", "[addbusstop_list]\na\t\n[end]\t\t\nc\n");
        let mut r = f.reader().with_rule(KeywordRule::TrimEnd);
        r.next_keyword();
        assert_eq!(r.until("[end]"), vec!["a\t"]);
        assert_eq!(r.str(), "c");
    }

    #[test]
    fn windows_paths() {
        assert_eq!(windows_components("texture\\anz-oben.jpg."), vec!["texture", "anz-oben.jpg"]);
        assert_eq!(windows_components(" ..\\..\\Model.\\a.o3d . "), vec!["..", "..", "Model", "a.o3d"]);
        assert_eq!(windows_components("\\Splines//x\\.\\y.sli"), vec!["Splines", "x", "y.sli"]);
        assert_eq!(windows_components("Folder..\\x"), vec!["Folder.", "x"]);
        assert_eq!(windows_components("Folder.\\"), vec!["Folder"]);
        assert_eq!(windows_components("str_gehweg02.bmp "), vec!["str_gehweg02.bmp"]);
        assert_eq!(windows_components(""), Vec::<&str>::new());
        let dir = std::env::temp_dir().join(format!("omsi-cfg-paths-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Texture").join("Bahn\u{fc}bergang")).unwrap();
        std::fs::write(dir.join("Texture").join("Bahn\u{fc}bergang").join("anz-oben.jpg"), b"x").unwrap();
        let p = resolve_path(&dir.join("model"), "..\\TEXTURE.\\BAHN\u{dc}BERGANG/Anz-Oben.JPG. ");
        assert!(vfs::is_file(&p), "{}", p.display());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reader() {
        let f = CfgFile::from_str("x.cfg", "hello\r\n[mesh]\r\na.o3d\r\n\r\n[LOD]\r\n0.5\r\n");
        let mut r = f.reader();
        assert_eq!(r.next_keyword().as_deref(), Some("mesh"));
        assert_eq!(r.str(), "a.o3d");
        assert_eq!(r.next_keyword().as_deref(), Some("lod"));
        assert_eq!(r.f32(), 0.5);
        assert_eq!(r.next_keyword(), None);
    }

    #[test]
    fn utf16() {
        let mut b = vec![0xFF, 0xFE];
        for c in "[name]\r\nGrundorf".encode_utf16() {
            b.extend_from_slice(&c.to_le_bytes());
        }
        let f = CfgFile::from_bytes("g.cfg", &b);
        assert_eq!(f.lines, vec!["[name]", "Grundorf"]);
    }
}

/// The `OMSI_*` switches of the environment, read once: the frame asked for dozens of them
/// every time round (`getenv` takes a lock and walks the environment - 2 % of a frame's CPU
/// time in `sync_materials` alone). The environment is not changed while the game runs.
pub mod env {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::sync::{OnceLock, RwLock};

    fn cache() -> &'static RwLock<HashMap<String, Option<OsString>>> {
        static C: OnceLock<RwLock<HashMap<String, Option<OsString>>>> = OnceLock::new();
        C.get_or_init(Default::default)
    }

    pub fn var_os(name: &str) -> Option<OsString> {
        if let Some(v) = cache().read().ok().and_then(|c| c.get(name).cloned()) {
            return v;
        }
        let v = std::env::var_os(name);
        if let Ok(mut c) = cache().write() {
            c.insert(name.to_string(), v.clone());
        }
        v
    }

    pub fn var(name: &str) -> Result<String, std::env::VarError> {
        match var_os(name) {
            Some(s) => s.into_string().map_err(std::env::VarError::NotUnicode),
            None => Err(std::env::VarError::NotPresent),
        }
    }
}
