//! Textures.
//!
//! OMSI looks a texture up by *file name* in a search order: the object's `texture` folder,
//! then the global `Texture` folder, trying a same-stem DDS first, the exact name, then other supported
//! extensions (`.dds`, `.bmp`, `.tga`, `.jpg`, `.png`), plus seasonal (`Texture\Spring` …) and
//! `_LOW` variants. A `<texture>.cfg` sidecar carries per-texture flags.

use hashbrown::HashMap;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod bc;
pub mod dds;
pub mod gpu;
pub mod tga;

#[cfg(test)]
mod format_tests;

/// The widest and tallest texture any decoder accepts (what Direct3D 9 cards held).
pub const MAX_DIMENSION: usize = 16384;

pub mod pbr;
pub use gpu::{gpu_options, set_gpu_options, GpuOptions, PixelFormat, TextureData};

#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top-left origin.
    pub rgba: Vec<u8>,
    /// True when the source had an alpha channel (bmp/jpg have none).
    pub has_alpha: bool,
}

impl Image {
    pub fn solid(rgba: [u8; 4]) -> Image {
        Image { width: 1, height: 1, rgba: rgba.to_vec(), has_alpha: rgba[3] != 255 }
    }

    /// A `[matl_bumpmap]` texture as the renderer samples it: every stock and mod bump map
    /// is a grey height map (the original turns it into the du/dv map of Direct3D's
    /// bump-mapped environment stage), so the height goes into the alpha channel, which
    /// stays linear in an sRGB texture; the colour is left white.
    pub fn bump_height_map(&self) -> Image {
        let rgba = self.rgba.chunks_exact(4).flat_map(|p| [255, 255, 255, ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8]).collect();
        Image { width: self.width, height: self.height, rgba, has_alpha: true }
    }
}

/// `<texture>.cfg` sidecar flags.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextureCfg {
    /// `[terrainmapping]`: texture is mapped in world coordinates (terrain overlays).
    pub terrain_mapping: bool,
    pub terrain_mapping_alpha: bool,
    pub puddles: bool,
    pub moisture: bool,
    /// `[surface]`: 0 asphalt, 1 concrete, 2 cobblestone, 3 dirt, 4 grass, 5 gravel,
    /// 6 snow, 7 deep snow. This is the id the scripts see as `Axle_SurfaceID_`.
    pub surface: i32,
}

impl TextureCfg {
    pub fn load(path: &Path) -> TextureCfg {
        let mut c = TextureCfg::default();
        if let Ok(f) = omsi_cfg::CfgFile::read(path) {
            let mut r = f.reader();
            while let Some(k) = r.next_keyword() {
                match k.as_str() {
                    "terrainmapping" => c.terrain_mapping = true,
                    "terrainmapping_alpha" => c.terrain_mapping_alpha = true,
                    "puddles" => c.puddles = true,
                    "moisture" => c.moisture = true,
                    "surface" => c.surface = r.i32(),
                    _ => {}
                }
            }
        }
        c
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    #[error("texture not found: {0}")]
    NotFound(String),
    #[error("{0}: {1}")]
    Decode(PathBuf, String),
}

pub const EXTENSIONS: [&str; 5] = ["dds", "bmp", "tga", "jpg", "png"];

/// Decode an image file into RGBA8.
pub fn decode_file(path: &Path) -> Result<Image, TextureError> {
    let bytes = omsi_cfg::vfs::read(path).map_err(|e| TextureError::Decode(path.to_path_buf(), e.to_string()))?;
    decode_bytes(&bytes, path)
}

pub fn decode_bytes(bytes: &[u8], path: &Path) -> Result<Image, TextureError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    // The original loads by content (D3DX), so files are often misnamed: a `.dds` that is a
    // BMP is common. Sniff the signatures first and fall back to the extension (TGA has none).
    if bytes.starts_with(b"DDS ") {
        return dds::decode(bytes).map_err(|e| TextureError::Decode(path.to_path_buf(), e));
    }
    let is_tga_ext = matches!(ext.as_str(), "tga");
    let looks_tga = bytes.len() > 18 && (1..=3).contains(&(bytes[2] & 7)) && bytes[2] & 0xF0 == 0 && matches!(bytes[16], 8 | 15 | 16 | 24 | 32);
    if is_tga_ext && looks_tga && !bytes.starts_with(b"BM") && !bytes.starts_with(&[0xFF, 0xD8]) {
        return tga::decode(bytes).map_err(|e| TextureError::Decode(path.to_path_buf(), e));
    }
    let format = if bytes.starts_with(b"BM") {
        image::ImageFormat::Bmp
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        image::ImageFormat::Jpeg
    } else if bytes.starts_with(b"\x89PNG") {
        image::ImageFormat::Png
    } else if looks_tga {
        // a TGA under another name (NEOMAN's `W_Bader_KR498_disp.png`): D3DX reads it by content
        return tga::decode(bytes).map_err(|e| TextureError::Decode(path.to_path_buf(), e));
    } else {
        match ext.as_str() {
            "dds" => image::ImageFormat::Dds,
            "bmp" => image::ImageFormat::Bmp,
            "tga" => return tga::decode(bytes).map_err(|e| TextureError::Decode(path.to_path_buf(), e)),
            "jpg" | "jpeg" => image::ImageFormat::Jpeg,
            "png" => image::ImageFormat::Png,
            _ => image::guess_format(bytes).map_err(|e| TextureError::Decode(path.to_path_buf(), e.to_string()))?,
        }
    };
    let img = match image::load_from_memory_with_format(bytes, format) {
        Ok(img) => img,
        // D3DX reads what GDI would: a palette bitmap that counts more colours than its bit
        // depth holds (the A21's and the Urbino's 4-bit `LCD-Innenanzeige.bmp` says 17) is
        // read with the colours it can use, and a 24-bit one that says BI_BITFIELDS (sky
        // packs' `Texture\skybox\night01.bmp`) as the plain 24-bit bitmap it is
        Err(e) if format == image::ImageFormat::Bmp => match bmp_clamped_palette(bytes).or_else(|| bmp24_bitfields(bytes)) {
            Some(fixed) => image::load_from_memory_with_format(&fixed, format).map_err(|e| TextureError::Decode(path.to_path_buf(), e.to_string()))?,
            None => return Err(TextureError::Decode(path.to_path_buf(), e.to_string())),
        },
        Err(e) => return Err(TextureError::Decode(path.to_path_buf(), e.to_string())),
    };
    let mut has_alpha = img.color().has_alpha();
    let rgba = img.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut rgba = rgba.into_raw();
    if format == image::ImageFormat::Bmp && !has_alpha {
        has_alpha = bmp32_alpha(bytes, width, height, &mut rgba);
    }
    Ok(Image { width, height, rgba, has_alpha })
}

/// A copy of a palette bitmap whose colour count (`biClrUsed`, `biClrImportant`) is cut to
/// what its bit depth can index; None when there is nothing to cut.
fn bmp_clamped_palette(bytes: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |o: usize| bytes.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let bits = u16_at(28)? as u32;
    if !(1..=8).contains(&bits) {
        return None;
    }
    let max = 1u32 << bits;
    let (used, important) = (u32_at(46)?, u32_at(50)?);
    if used <= max && important <= max {
        return None;
    }
    let mut out = bytes.to_vec();
    out[46..50].copy_from_slice(&used.min(max).to_le_bytes());
    out[50..54].copy_from_slice(&important.min(max).to_le_bytes());
    Some(out)
}

/// A copy of a 24-bit bitmap that says `BI_BITFIELDS` (3) with its compression set to
/// `BI_RGB`: bit fields mean nothing at 24 bits, and D3DX reads the pixels as B8G8R8 where
/// the `image` crate refuses the file. The masks after a 40-byte header stay where they are
/// (the pixel offset already points past them). None when the bitmap is not one of those.
fn bmp24_bitfields(bytes: &[u8]) -> Option<Vec<u8>> {
    let bits = bytes.get(28..30).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    let compression = bytes.get(30..34).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
    if bits != 24 || compression != 3 {
        return None;
    }
    let mut out = bytes.to_vec();
    out[30..34].copy_from_slice(&0u32.to_le_bytes());
    Some(out)
}

/// The fourth byte of a 32-bit `BI_RGB` bitmap. GDI (and the `image` crate) call it
/// reserved and drop it; D3DX, which loads the original's textures, reads the bitmap as
/// A8R8G8B8 as soon as one of those bytes is not zero and as X8R8G8B8 otherwise. The
/// repaint tool writes its liveries this way (the SD200/SD202 adverts and the NL202's HVL
/// scheme are 32-bit bitmaps named `.dds`), and their fourth byte is the reflection mask
/// of the paint. Returns whether the alpha was taken over.
fn bmp32_alpha(bytes: &[u8], width: u32, height: u32, rgba: &mut [u8]) -> bool {
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (Some(offset), Some(raw_h), Some(compression)) = (u32_at(10), u32_at(22), u32_at(30)) else { return false };
    let bpp = bytes.get(28..30).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
    if bpp != 32 || compression != 0 {
        return false;
    }
    let (w, h) = (width as usize, height as usize);
    let offset = offset as usize;
    let Some(pixels) = bytes.get(offset..offset + w * h * 4) else { return false };
    if !pixels.chunks_exact(4).any(|p| p[3] != 0) {
        return false;
    }
    // rows are stored bottom-up unless the height is negative
    let bottom_up = (raw_h as i32) > 0;
    for (y, row) in pixels.chunks_exact(w * 4).enumerate() {
        let dst_y = if bottom_up { h - 1 - y } else { y };
        let dst = &mut rgba[dst_y * w * 4..(dst_y + 1) * w * 4];
        for (d, s) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
            d[3] = s[3];
        }
    }
    true
}

/// Find a texture file by OMSI's rules. `name` is the name written in the content file
/// (may carry a path and any extension); `dirs` are searched in order.
static SEASON: Mutex<Option<String>> = Mutex::new(None);

/// Season texture subfolder (`Spring`, `Fall`, `Winter`, `WinterSnow`, `WinterSnowfall`,
/// `SummerDry`): textures found in `<texture dir>/<season>/` take precedence, like OMSI's
/// `[addseason]` - or in the folders after it that [`season_folders`] lists.
pub fn set_season_folder(folder: Option<String>) {
    *SEASON.lock() = folder;
}

pub fn season_folder() -> Option<String> {
    SEASON.lock().clone()
}

/// The folders a texture's variant is looked for in under the season `head`, best first,
/// as Omsi.exe picks it (0x7f910c): in snow the snowy roads of `WinterSnowfall` when the
/// weather has snow on the road, else `WinterSnow`, and a texture with no snow picture
/// takes its `Winter` one, else its `Fall` one; in winter `Winter`, else `Fall`. (With the
/// one folder alone, a road whose snowy picture is in `WinterSnowfall` - the stock asphalt
/// - stayed bare under the heaviest snowfall, and a texture with only a winter picture
/// stayed green in the snow.)
pub fn season_chain(head: &str) -> Vec<String> {
    let chain: &[&str] = match head.to_ascii_lowercase().as_str() {
        "wintersnowfall" => &["WinterSnowfall", "WinterSnow", "Winter", "Fall"],
        "wintersnow" => &["WinterSnow", "Winter", "Fall"],
        "winter" => &["Winter", "Fall"],
        _ => return vec![head.to_string()],
    };
    chain.iter().map(|f| f.to_string()).collect()
}

/// The folders of the current season (see [`season_chain`]), none in summer.
pub fn season_folders() -> Vec<String> {
    season_folder().map(|f| season_chain(&f)).unwrap_or_default()
}

pub fn find_texture(name: &str, dirs: &[&Path]) -> Option<PathBuf> {
    // Memoised: a lookup that misses probes five extensions in every folder, each with a
    // case-insensitive directory scan - done afresh for every material of every bus it
    // cost three quarters of a minute to put the Spandau fleet on the GPU.
    // (a miss is kept only while the content stays as it was: a paint installed while the
    // game runs is found)
    static MEMO: std::sync::OnceLock<Mutex<HashMap<String, (u64, Option<PathBuf>)>>> = std::sync::OnceLock::new();
    let generation = omsi_cfg::content_generation();
    let key = {
        let mut k = String::with_capacity(256);
        k.push_str(name.trim());
        k.push('|');
        if let Some(f) = season_folder() {
            k.push_str(&f);
        }
        for d in dirs {
            k.push('|');
            k.push_str(&d.to_string_lossy());
        }
        k
    };
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((g, hit)) = memo.lock().get(&key) {
        if hit.is_some() || *g == generation {
            return hit.clone();
        }
    }
    let found = find_texture_uncached(name, dirs);
    memo.lock().insert(key, (generation, found.clone()));
    found
}

fn find_texture_uncached(name: &str, dirs: &[&Path]) -> Option<PathBuf> {
    find_texture_in_season(name, dirs, season_folder().as_deref()).or_else(|| find_texture_elsewhere(name, dirs))
}

/// A spline's or object's texture missing from its folders: the nearest same-named file under `Splines`/`Sceneryobjects`.
fn find_texture_elsewhere(name: &str, dirs: &[&Path]) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let first = dirs.first()?;
    let top = first.ancestors().find_map(|a| {
        a.file_name().and_then(|f| f.to_str()).filter(|f| f.eq_ignore_ascii_case("Splines") || f.eq_ignore_ascii_case("Sceneryobjects")).map(|f| f.to_ascii_lowercase())
    })?;
    static INDEX: std::sync::OnceLock<Mutex<HashMap<String, (u64, Arc<HashMap<String, Vec<PathBuf>>>)>>> = std::sync::OnceLock::new();
    let generation = omsi_cfg::content_generation();
    let index = {
        let mut all = INDEX.get_or_init(|| Mutex::new(HashMap::new())).lock();
        match all.get(&top) {
            Some((g, i)) if *g == generation => i.clone(),
            _ => {
                let i = Arc::new(texture_index(&top));
                all.insert(top.clone(), (generation, i.clone()));
                i
            }
        }
    };
    let key = |p: &Path| p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase());
    let want = Path::new(name);
    if !want.extension().and_then(|x| x.to_str()).is_some_and(|x| ["dds", "bmp", "jpg", "jpeg", "png", "tga"].iter().any(|t| x.eq_ignore_ascii_case(t))) {
        return None;
    }
    let candidates = index.get(&key(want)?)?;
    let shared = |p: &Path| p.components().zip(first.components()).take_while(|(a, b)| a == b).count();
    let in_texture = |p: &Path| p.parent().and_then(|d| d.file_name()).and_then(|f| f.to_str()).is_some_and(|f| f.eq_ignore_ascii_case("texture"));
    let same_ext = |p: &Path| p.extension().zip(want.extension()).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
    let found = candidates.iter().max_by_key(|p| (in_texture(p), shared(p), same_ext(p)))?.clone();
    static SAID: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    if SAID.get_or_init(Default::default).lock().insert(name.to_ascii_lowercase()) {
        log::info!("texture {name} is not in its folders ({}); taken from {}", first.display(), found.display());
    }
    Some(found)
}

/// Image files under `top` (`splines`/`sceneryobjects`) of every content root and mounted archive, by stem.
fn texture_index(top: &str) -> HashMap<String, Vec<PathBuf>> {
    let mut out: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut stack: Vec<PathBuf> = omsi_cfg::content_roots()
        .into_iter()
        .filter_map(|r| {
            let (name, _) = omsi_cfg::vfs::list_dir(&r)?.into_iter().find(|(n, d)| *d && n.to_str().is_some_and(|n| n.eq_ignore_ascii_case(top)))?;
            Some(r.join(name))
        })
        .collect();
    while let Some(dir) = stack.pop() {
        let Some(entries) = omsi_cfg::vfs::list_dir(&dir) else { continue };
        for (name, is_dir) in entries {
            let p = dir.join(name);
            if is_dir {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ["dds", "bmp", "jpg", "png", "tga"].iter().any(|t| x.eq_ignore_ascii_case(t))) {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.entry(stem.to_ascii_lowercase()).or_default().push(p);
                }
            }
        }
    }
    out
}

fn find_texture_in_season(name: &str, dirs: &[&Path], season: Option<&str>) -> Option<PathBuf> {
    // a file named in full (a paint scheme's picture, resolved in its scheme's folder)
    let full = Path::new(name.trim());
    if full.is_absolute() {
        if let (Some(parent), Some(file)) = (full.parent(), full.file_name().and_then(|f| f.to_str())) {
            if let Some(found) = find_texture_in_dir(parent, file) {
                return Some(found);
            }
        }
    }
    let name = name.trim().replace('\\', "/");
    if name.is_empty() {
        return None;
    }
    let stem_path = Path::new(&name);
    // A seasonal texture lives in a subfolder of the folder the texture itself is in:
    // `Texture\WinterSnow\gras.bmp` for `Texture\gras.bmp`. The name often carries that
    // folder with it, so the season goes in front of the file name, not in front of the
    // whole path; both spellings are tried.
    let mut names: Vec<String> = Vec::new();
    for f in season.map(season_chain).unwrap_or_default() {
        match (stem_path.parent(), stem_path.file_name()) {
            (Some(par), Some(file)) if !par.as_os_str().is_empty() => names.push(format!("{}/{}/{}", par.display(), f, file.to_string_lossy())),
            (_, Some(file)) => names.push(format!("{}/{}", f, file.to_string_lossy())),
            _ => {}
        }
    }
    names.push(name.clone());
    // A name with folders in it that is not below any of the texture folders is taken from
    // the main folder, as OMSI does (`Splines\BS_ADDON_CreativeStreets\Gehwege\texture\
    // BS_Gehweg_Allgemein1.bmp` in a spline of another folder of that add-on).
    let from_root: Vec<PathBuf> = if name.contains('/') { omsi_cfg::content_roots().into_iter().take(1).collect() } else { Vec::new() };
    let dirs: Vec<&Path> = dirs.iter().copied().chain(from_root.iter().map(|p| p.as_path())).collect();
    // Keep the pack's directory priority, then prefer its seasonal variant.
    for dir in &dirs {
        for cand_name in &names {
            if let Some(found) = find_texture_in_dir(dir, cand_name) {
                return Some(found);
            }
        }
    }
    // A path of the author's machine (`D:\OMSI 2\Vehicles\Sprinter_work\Texture\extras.jpg`
    // in the Sprinter 412D): the same file under the installation's content folders, else
    // the bare file name in the texture folders.
    let absolute = name.as_bytes().get(1) == Some(&b':') || name.starts_with('/');
    if absolute {
        let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
        if let Some(i) = parts.iter().position(|p| omsi_cfg::CONTENT_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(p))) {
            let rel = parts[i..].join("/");
            for root in omsi_cfg::content_roots() {
                if let Some(found) = find_texture_in_dir(&root, &rel) {
                    return Some(found);
                }
            }
        }
        if let Some(file) = parts.last() {
            if let Some(p) = find_texture_in_season(file, &dirs, season) {
                return Some(p);
            }
        }
    }
    None
}

/// Prefer the authored DDS replacement within one search location. Folder/season
/// precedence stays outside this function so a global DDS cannot override a local PNG.
fn find_texture_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let stem = Path::new(name).with_extension("");
    let dds = format!("{}.dds", stem.display());
    let p = omsi_cfg::resolve_path(dir, &dds);
    if omsi_cfg::vfs::is_file(&p) {
        return Some(p);
    }
    let p = omsi_cfg::resolve_path(dir, name);
    if omsi_cfg::vfs::is_file(&p) {
        return Some(p);
    }
    for ext in EXTENSIONS.into_iter().filter(|e| *e != "dds") {
        let p = omsi_cfg::resolve_path(dir, &format!("{}.{}", stem.display(), ext));
        if omsi_cfg::vfs::is_file(&p) {
            return Some(p);
        }
    }
    None
}

/// Shared, thread-safe texture cache keyed by resolved path.
///
/// It holds decoded pictures until whoever puts them on the GPU lets them go
/// ([`TextureCache::release`]); what it keeps for good is whether a file has an alpha
/// channel, which material setup keeps asking.
#[derive(Default)]
pub struct TextureCache {
    images: Mutex<HashMap<PathBuf, Arc<Image>>>,
    /// Pictures prepared for the GPU (blocks where the device takes them), until uploaded.
    gpu: Mutex<HashMap<PathBuf, Arc<TextureData>>>,
    misses: Mutex<HashMap<String, ()>>,
    alpha: Mutex<HashMap<PathBuf, bool>>,
}

/// Where the `.cfg` sidecar of a texture is. OMSI names it after the texture as the model
/// asks for it (`str_asphdrk.bmp.cfg`), whatever file is then loaded in its place: most stock
/// road textures ship as `.dds` next to a `<name>.bmp.cfg`, and looking for `<name>.dds.cfg`
/// left every such road dry in the rain. A seasonal copy (`WinterSnow/x.dds`) uses the
/// sidecar of the texture it stands in for when it has none of its own; a few sidecars drop
/// the extension (`<name>.cfg`).
pub fn cfg_path(requested: &str, found: &Path) -> Option<PathBuf> {
    let req = requested.trim().replace('\\', "/");
    let base = req.rsplit('/').next().unwrap_or(&req).to_string();
    let req_stem = Path::new(&base).with_extension("");
    let found_name = found.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let found_stem = found.with_extension("");
    let found_stem = found_stem.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dirs: Vec<&Path> = Vec::new();
    if let Some(d) = found.parent() {
        dirs.push(d);
        if let Some(pp) = d.parent() {
            dirs.push(pp);
        }
    }
    for (i, d) in dirs.iter().enumerate() {
        let mut names = vec![format!("{base}.cfg"), format!("{found_name}.cfg"), format!("{}.cfg", req_stem.to_string_lossy()), format!("{found_stem}.cfg")];
        if i > 0 {
            // the parent folder is only for a seasonal subfolder's texture
            names.truncate(1);
        }
        for n in names {
            let c = d.join(&n);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

impl TextureCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The `<texture>.cfg` sidecar of a texture, if it has one: `[moisture]` marks a
    /// surface that darkens in the wet, `[puddles]` one that collects puddles.
    pub fn cfg(&self, name: &str, dirs: &[&Path]) -> TextureCfg {
        match find_texture(name, dirs) {
            Some(p) => match cfg_path(name, &p) {
                Some(c) => TextureCfg::load(&c),
                None => TextureCfg::default(),
            },
            None => TextureCfg::default(),
        }
    }

    /// Load (or fetch from cache) a texture found through `find_texture`.
    pub fn get(&self, name: &str, dirs: &[&Path]) -> Option<Arc<Image>> {
        let path = match find_texture(name, dirs) {
            Some(p) => p,
            None => {
                let mut m = self.misses.lock();
                if m.insert(name.to_string(), ()).is_none() {
                    log::warn!("Did not find texture file \"{name}\"!");
                }
                return None;
            }
        };
        if let Some(i) = self.images.lock().get(&path) {
            return Some(i.clone());
        }
        match decode_file(&path) {
            Ok(img) => {
                let a = Arc::new(img);
                self.alpha.lock().insert(path.clone(), a.has_alpha);
                self.images.lock().insert(path, a.clone());
                Some(a)
            }
            Err(e) => {
                log::warn!("{e}");
                None
            }
        }
    }

    /// Load (or fetch from cache) a texture prepared for the GPU (see [`gpu::load_gpu`]):
    /// the file it was found in and its data.
    pub fn get_gpu(&self, name: &str, dirs: &[&Path]) -> Option<(PathBuf, Arc<TextureData>)> {
        let Some(path) = find_texture(name, dirs) else {
            let mut m = self.misses.lock();
            if m.insert(name.to_string(), ()).is_none() {
                log::warn!("Did not find texture file \"{name}\"!");
            }
            return None;
        };
        self.get_gpu_path(&path).map(|t| (path, t))
    }

    /// [`TextureCache::get_gpu`] for a file already found.
    pub fn get_gpu_path(&self, path: &Path) -> Option<Arc<TextureData>> {
        if let Some(t) = self.gpu.lock().get(path) {
            return Some(t.clone());
        }
        match gpu::load_gpu(path) {
            Ok((t, _)) => {
                let t = Arc::new(t);
                self.alpha.lock().insert(path.to_path_buf(), t.has_alpha);
                self.gpu.lock().insert(path.to_path_buf(), t.clone());
                Some(t)
            }
            Err(e) => {
                let mut m = self.misses.lock();
                if m.insert(path.to_string_lossy().into_owned(), ()).is_none() {
                    log::warn!("{e}");
                }
                None
            }
        }
    }

    /// A texture for the GPU without the work of compressing it (see
    /// [`gpu::load_gpu_fast`]): what was read ahead if there is something, else read now.
    /// The flag says compressing it on a worker is worth it.
    pub fn get_gpu_fast(&self, path: &Path) -> Option<(Arc<TextureData>, bool)> {
        if let Some(t) = self.gpu.lock().get(path) {
            return Some((t.clone(), false));
        }
        match gpu::load_gpu_fast(path) {
            Ok((t, worth)) => {
                self.alpha.lock().insert(path.to_path_buf(), t.has_alpha);
                Some((Arc::new(t), worth))
            }
            Err(e) => {
                let mut m = self.misses.lock();
                if m.insert(path.to_string_lossy().into_owned(), ()).is_none() {
                    log::warn!("{e}");
                }
                None
            }
        }
    }

    /// Whether the texture has an alpha channel (decoding it only the first time).
    pub fn has_alpha(&self, name: &str, dirs: &[&Path]) -> Option<bool> {
        let path = find_texture(name, dirs)?;
        if let Some(a) = self.alpha.lock().get(&path) {
            return Some(*a);
        }
        self.get(name, dirs).map(|i| i.has_alpha)
    }

    /// The decoded picture of `path` is not needed any more (it is on the GPU): the memory
    /// goes back. A later `get` decodes it again.
    pub fn release(&self, path: &Path) {
        self.images.lock().remove(path);
        self.gpu.lock().remove(path);
    }

    /// Let go of every decoded picture (what a batch decoded ahead and nobody uploaded).
    pub fn release_all(&self) {
        self.images.lock().clear();
        self.gpu.lock().clear();
    }

    /// Bytes of the pictures held (decoded and prepared ones).
    pub fn held_bytes(&self) -> usize {
        self.images.lock().values().map(|i| i.rgba.len()).sum::<usize>() + self.gpu.lock().values().map(|t| t.cpu_bytes()).sum::<usize>()
    }

    pub fn len(&self) -> usize {
        self.images.lock().len() + self.gpu.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasonal_textures_keep_pack_priority_and_terrain_mapping() {
        let dir = std::env::temp_dir().join(format!(
            "omsi-texture-season-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let local = dir.join("pack/texture");
        let global = dir.join("Texture");
        for folder in [&local, &global] {
            std::fs::create_dir_all(folder.join("Fall")).unwrap();
        }
        for path in [
            local.join("mapped.dds"),
            global.join("Fall/mapped.bmp"),
            local.join("seasonal.bmp"),
            local.join("Fall/seasonal.dds"),
            global.join("Fall/seasonal.bmp"),
            global.join("fallback.bmp"),
            global.join("Fall/fallback.dds"),
        ] {
            std::fs::write(path, b"lookup-only fixture").unwrap();
        }
        for name in ["mapped.bmp.cfg", "seasonal.bmp.cfg"] {
            std::fs::write(local.join(name), "[terrainmapping]\n").unwrap();
        }
        let dirs = [local.as_path(), global.as_path()];
        // A local placeholder may use a different extension from the authored name.
        // The map's unrelated autumn grass must not hide its terrain-mapping flag.
        let mapped = find_texture_in_season("mapped.bmp", &dirs, Some("Fall")).unwrap();
        assert_eq!(mapped, local.join("mapped.dds"));
        assert!(TextureCfg::load(&cfg_path("mapped.bmp", &mapped).unwrap()).terrain_mapping);
        // The pack's own seasonal variant still wins and inherits its base sidecar.
        let seasonal = find_texture_in_season("seasonal.bmp", &dirs, Some("Fall")).unwrap();
        assert_eq!(seasonal, local.join("Fall/seasonal.dds"));
        assert!(TextureCfg::load(&cfg_path("seasonal.bmp", &seasonal).unwrap()).terrain_mapping);
        // A texture absent from the pack keeps the global seasonal fallback.
        assert_eq!(
            find_texture_in_season("fallback.bmp", &dirs, Some("Fall")),
            Some(global.join("Fall/fallback.dds")),
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// In snow a texture takes the variant Omsi.exe takes: the stock asphalt's snowy
    /// picture (`WinterSnowfall`) only with snow on the road, a texture without a snow
    /// picture its winter one, one without that its autumn one, else itself.
    #[test]
    fn a_texture_without_a_snow_picture_falls_back_as_omsi_does() {
        let dir = std::env::temp_dir().join(format!("omsi-texture-snow-{}", std::process::id()));
        let tex = dir.join("texture");
        for f in ["WinterSnowfall", "WinterSnow", "Winter", "Fall"] {
            std::fs::create_dir_all(tex.join(f)).unwrap();
        }
        for path in ["road.bmp", "WinterSnowfall/road.bmp", "walk.bmp", "WinterSnow/walk.bmp", "WinterSnowfall/walk.bmp", "hedge.bmp", "Winter/hedge.bmp", "Fall/hedge.bmp", "tree.bmp", "Fall/tree.bmp", "plain.bmp"] {
            std::fs::write(tex.join(path), b"lookup-only fixture").unwrap();
        }
        let dirs = [tex.as_path()];
        let find = |name: &str, season: &str| find_texture_in_season(name, &dirs, Some(season)).unwrap();
        // snow on the road
        assert_eq!(find("road.bmp", "WinterSnowfall"), tex.join("WinterSnowfall/road.bmp"));
        assert_eq!(find("walk.bmp", "WinterSnowfall"), tex.join("WinterSnowfall/walk.bmp"));
        assert_eq!(find("hedge.bmp", "WinterSnowfall"), tex.join("Winter/hedge.bmp"));
        // snow, the roads clear
        assert_eq!(find("road.bmp", "WinterSnow"), tex.join("road.bmp"));
        assert_eq!(find("walk.bmp", "WinterSnow"), tex.join("WinterSnow/walk.bmp"));
        assert_eq!(find("hedge.bmp", "WinterSnow"), tex.join("Winter/hedge.bmp"));
        assert_eq!(find("tree.bmp", "WinterSnow"), tex.join("Fall/tree.bmp"));
        assert_eq!(find("plain.bmp", "WinterSnow"), tex.join("plain.bmp"));
        // winter: its own picture, else the autumn one
        assert_eq!(find("hedge.bmp", "Winter"), tex.join("Winter/hedge.bmp"));
        assert_eq!(find("tree.bmp", "Winter"), tex.join("Fall/tree.bmp"));
        assert_eq!(find("walk.bmp", "Winter"), tex.join("walk.bmp"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dds_precedes_exact_names_but_preserves_folder_priority() {
        let dir = std::env::temp_dir().join(format!("omsi-dds-priority-{}", std::process::id()));
        let local = dir.join("local");
        let global = dir.join("global");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&global).unwrap();
        for ext in ["png", "jpg", "tga", "bmp"] {
            let stem = format!("sign_{ext}");
            std::fs::write(local.join(format!("{stem}.{ext}")), b"x").unwrap();
            std::fs::write(local.join(format!("{stem}.DDS")), b"x").unwrap();
        }
        std::fs::write(local.join("local_only.png"), b"x").unwrap();
        std::fs::write(global.join("local_only.dds"), b"x").unwrap();
        std::fs::write(local.join("local_only.bmp"), b"x").unwrap();
        // The case-insensitive directory listing is cached on Linux. Populate the
        // fixture before the first lookup, as content loaded at game start is.
        for ext in ["png", "jpg", "tga", "bmp"] {
            let name = format!("sign_{ext}.{ext}");
            let found = find_texture_uncached(&name, &[&local]).unwrap();
            assert_eq!(found.extension().unwrap().to_string_lossy().to_ascii_lowercase(), "dds");
            assert_eq!(find_texture_uncached(local.join(&name).to_str().unwrap(), &[]), Some(found));
        }
        assert_eq!(find_texture_uncached("local_only.png", &[&local, &global]), Some(local.join("local_only.png")));
        // Without DDS, the requested format wins over the other fallback formats.
        assert_eq!(find_texture_uncached("local_only.png", &[&local]), Some(local.join("local_only.png")));
        assert_eq!(find_texture_uncached("missing.png", &[&local]), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_spline_texture_missing_from_its_folders_comes_from_another_spline_folder() {
        let dir = std::env::temp_dir().join(format!("omsi-elsewhere-{}", std::process::id()));
        let (own, other) = (dir.join("Splines/Pack/Roads/texture"), dir.join("Splines/Pack/Paths/texture"));
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("gehweg.bmp"), b"x").unwrap();
        std::fs::write(other.join("0.png"), b"x").unwrap();
        omsi_cfg::add_content_root(dir.clone());
        assert_eq!(find_texture_uncached("gehweg.bmp", &[&own]), Some(other.join("gehweg.bmp")));
        assert_eq!(find_texture_uncached("0", &[&own]), None);
        // content installed while the game runs is indexed again
        std::fs::write(other.join("late.bmp"), b"x").unwrap();
        omsi_cfg::content_changed();
        assert_eq!(find_texture_uncached("late.bmp", &[&own]), Some(other.join("late.bmp")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A mesh's texture name with Windows' quirks (Ahlheim's `anz-oben.jpg.`) finds the file.
    #[test]
    fn texture_names_as_windows_reads_them() {
        let dir = std::env::temp_dir().join(format!("omsi-texture-names-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("texture")).unwrap();
        std::fs::write(dir.join("texture").join("anz-oben.jpg"), b"x").unwrap();
        let tex = dir.join("texture");
        assert_eq!(find_texture("anz-oben.jpg.", &[tex.as_path()]), Some(tex.join("anz-oben.jpg")));
        let upper = find_texture("ANZ-OBEN.JPG .", &[tex.as_path()]).map(|p| p.to_string_lossy().to_lowercase());
        assert_eq!(upper, Some(tex.join("anz-oben.jpg").to_string_lossy().to_lowercase()));
        assert_eq!(find_texture("texture.\\anz-oben.bmp", &[dir.as_path()]), Some(tex.join("anz-oben.jpg")));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A `[matl_freetex]` name of a bus stop sign (`\New Territories East\Freetex_Lolipop\x.bmp`)
    /// is rooted at `texture\`, however its leading backslash reads as a path root.
    #[test]
    fn a_leading_backslash_names_a_texture_below_the_folder() {
        let dir = std::env::temp_dir().join(format!("omsi-freetex-root-{}", std::process::id()));
        let tex = dir.join("texture");
        let sub = tex.join("New Territories East/Freetex_Lolipop");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("x.bmp"), b"x").unwrap();
        assert_eq!(
            find_texture("\\New Territories East\\Freetex_Lolipop\\x.bmp", &[tex.as_path()]),
            Some(sub.join("x.bmp"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A 2x2 32-bit BI_RGB bitmap, stored bottom-up, with these BGRA pixels in file order.
    fn bmp32(pixels: [[u8; 4]; 4]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(54u32 + 16).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for p in pixels {
            b.extend_from_slice(&p);
        }
        b
    }

    #[test]
    fn bmp32_alpha_like_d3dx() {
        // file rows are bottom-up: the first two pixels are the lower row of the image
        let img = decode_bytes(&bmp32([[1, 2, 3, 10], [4, 5, 6, 20], [7, 8, 9, 30], [10, 11, 12, 40]]), Path::new("x.dds")).unwrap();
        assert!(img.has_alpha);
        let alpha: Vec<u8> = img.rgba.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(alpha, vec![30, 40, 10, 20]);
        assert_eq!(&img.rgba[..3], &[9, 8, 7]);
        // all fourth bytes zero: an X8R8G8B8 bitmap, opaque
        let img = decode_bytes(&bmp32([[1, 2, 3, 0]; 4]), Path::new("x.bmp")).unwrap();
        assert!(!img.has_alpha);
        assert!(img.rgba.chunks_exact(4).all(|p| p[3] == 255));
    }

    /// A 24-bit bitmap that says BI_BITFIELDS, with its three masks after the header, reads
    /// as a plain 24-bit one (a sky pack's `night01.bmp`).
    #[test]
    fn bmp24_with_bitfields() {
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(66u32 + 16).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&66u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[0; 20]);
        for m in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff] {
            b.extend_from_slice(&m.to_le_bytes());
        }
        // two rows of two BGR pixels, each padded to four bytes, bottom-up
        b.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0]);
        let img = decode_bytes(&b, Path::new("night01.bmp")).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(&img.rgba[..8], &[9, 8, 7, 255, 12, 11, 10, 255]);
    }

    /// A TGA named `.png` (NEOMAN's `W_Bader_KR498_disp.png`, a 24-bit RLE TGA) decodes as TGA.
    #[test]
    fn misnamed_tga_by_content() {
        let mut b = vec![0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 0];
        b.extend_from_slice(&[0x83, 0x30, 0x20, 0x10]);
        let img = decode_bytes(&b, Path::new("x.png")).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(&img.rgba[..4], &[0x10, 0x20, 0x30, 255]);
    }

    #[test]
    fn bump_height_in_alpha() {
        let grey = Image { width: 3, height: 1, rgba: vec![0, 0, 0, 255, 127, 127, 127, 255, 255, 255, 255, 7], has_alpha: false };
        let h = grey.bump_height_map();
        assert_eq!(h.rgba, vec![255, 255, 255, 0, 255, 255, 255, 127, 255, 255, 255, 255]);
        assert!(h.has_alpha);
        assert_eq!((h.width, h.height), (3, 1));
    }
}
