//! PBR texture sets: maps that lie beside a diffuse texture and share its name with a
//! suffix, as the common PBR tools export them. For `Texture/bus.dds`:
//!
//! - `bus_nn`, `bus_normal`, `bus_nrm` (`_gl` after it - `bus_nn_gl` - for an OpenGL-style
//!   map, green up; Direct3D's green-down is taken otherwise): a tangent-space normal map;
//! - `bus_rr`, `bus_rough`, `bus_roughness` (or `bus_gg`, `bus_gloss`, `bus_glossiness`:
//!   inverted);
//! - `bus_mm`, `bus_metal`, `bus_metallic`, `bus_metalness`;
//! - `bus_aa`, `bus_ao`, `bus_occlusion`;
//!
//! The short suffixes are doubled letters: OMSI's mods name their night maps `_n` (the
//! Procity's `bord_bwc_n.tga`) and use `_r` and `_m` for all sorts of things.
//! - `bus_orm`, `bus_arm` (occlusion, roughness, metalness in red, green, blue), `bus_mra`
//!   (metalness, roughness, occlusion);
//!
//! each as `.png`, `.tga`, `.dds`, `.bmp` or `.jpg`. The single maps are packed into one
//! occlusion/roughness/metalness picture for the GPU. OMSI itself knows none of this: a map
//! without such files looks as it always did.

mod height;

use crate::Image;
use std::path::{Path, PathBuf};

const EXTS: [&str; 6] = ["png", "tga", "dds", "bmp", "jpg", "jpeg"];

/// The files of a PBR set found beside a diffuse texture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PbrFiles {
    /// The first normal map candidate (see `normals`).
    pub normal: Option<PathBuf>,
    /// The normal map is OpenGL-style (green up).
    pub normal_gl: bool,
    /// Every candidate, in order, with its style (OpenGL: true): the first one that is a
    /// normal map is taken (`bord_bwc_n` is the Procity's night map, `bord_bwc_nm` might not be).
    pub normals: Vec<(PathBuf, bool)>,
    /// An authored linear height map. Only used with explicit physical dimensions below,
    /// and only when there is no usable normal map.
    pub height: Option<PathBuf>,
    pub height_config: Option<PathBuf>,
    pub rough: Option<PathBuf>,
    /// `rough` is a gloss map (1 - roughness).
    pub gloss: bool,
    pub metal: Option<PathBuf>,
    pub ao: Option<PathBuf>,
    /// A packed map and its channel order: occlusion, roughness, metalness as (r, g, b)
    /// indices.
    pub packed: Option<(PathBuf, [usize; 3])>,
}

impl PbrFiles {
    pub fn is_empty(&self) -> bool {
        self.normals.is_empty()
            && self.height.is_none()
            && self.rough.is_none()
            && self.metal.is_none()
            && self.ao.is_none()
            && self.packed.is_none()
    }
}

/// The PBR maps as they go to the GPU.
pub struct PbrImages {
    pub normal: Option<Image>,
    /// Occlusion, roughness, metalness in red, green, blue.
    pub orm: Option<Image>,
    /// Normal kind (1: DirectX, 2: OpenGL, 3: physical height slopes), then
    /// occlusion, roughness and metalness present.
    pub flags: [f32; 4],
}

/// Largest side a PBR map is kept at on the GPU (they are uncompressed: a normal map
/// does not survive DXT1).
const MAX_SIDE: u32 = 4096;

/// The PBR files beside `diffuse` (none: an empty set).
pub fn find(diffuse: &Path) -> PbrFiles {
    let mut out = PbrFiles::default();
    let (Some(dir), Some(stem)) = (
        diffuse.parent(),
        diffuse.file_stem().map(|s| s.to_string_lossy().to_string()),
    ) else {
        return out;
    };
    // (the folder's pictures listed once, archives too, and compared case-insensitively:
    // mods mix `_N` and `_n`; and the same folder in every content root, highest priority
    // first - maps put into the content folder beside a texture of the OMSI install were
    // never found)
    let stem_l = stem.to_ascii_lowercase();
    let mut names: Vec<(String, PathBuf)> = Vec::new();
    for d in omsi_cfg::mirrored_dirs(dir) {
        for (s, name) in pictures_in(&d).iter() {
            if let Some(rest) = s.strip_prefix(&stem_l) {
                if let Some(suffix) = rest.strip_prefix('_').or_else(|| rest.strip_prefix('-')) {
                    if !names.iter().any(|(k, _)| k == suffix) {
                        names.push((suffix.to_string(), d.join(name)));
                    }
                }
            }
        }
    }
    let pick = |keys: &[&str]| -> Option<PathBuf> {
        keys.iter()
            .find_map(|k| names.iter().find(|(s, _)| s == k).map(|(_, p)| p.clone()))
    };
    for (keys, gl) in [
        (
            &["nn_gl", "normal_gl", "nrm_gl", "normalgl", "normal_opengl"][..],
            true,
        ),
        (
            &["nn", "normal", "nrm", "normal_dx", "nn_dx", "normalmap"][..],
            false,
        ),
    ] {
        for k in keys {
            if let Some((_, p)) = names.iter().find(|(s, _)| s == k) {
                out.normals.push((p.clone(), gl));
            }
        }
    }
    if let Some((p, gl)) = out.normals.first() {
        out.normal = Some(p.clone());
        out.normal_gl = *gl;
    }
    out.height = pick(&["height"]);
    if out.height.is_some() {
        let config = omsi_cfg::resolve_path(dir, &format!("{stem}.pbr.cfg"));
        if omsi_cfg::vfs::is_file(&config) {
            out.height_config = Some(config);
        }
    }
    // (no single letters: the Procity's `lumini_r.tga` is a night map, grey enough to pass)
    if let Some(p) = pick(&["rr", "rough", "roughness", "rgh"]) {
        out.rough = Some(p);
    } else if let Some(p) = pick(&["gg", "gloss", "glossiness"]) {
        out.rough = Some(p);
        out.gloss = true;
    }
    out.metal = pick(&["mm", "metal", "metallic", "metalness", "mtl"]);
    out.ao = pick(&[
        "aa",
        "ao",
        "occlusion",
        "ambientocclusion",
        "ambient_occlusion",
    ]);
    if let Some(p) = pick(&["orm", "arm"]) {
        out.packed = Some((p, [0, 1, 2]));
    } else if let Some(p) = pick(&["mra"]) {
        out.packed = Some((p, [2, 1, 0]));
    } else if let Some(p) = pick(&["rma"]) {
        out.packed = Some((p, [2, 0, 1]));
    }
    out
}

type Listing = std::sync::Arc<Vec<(String, std::ffi::OsString)>>;

/// The pictures of a folder whose names could be a PBR map's (they have a `_` or `-`):
/// (lower-case stem, file name). Kept per folder: a scenery folder of thousands of files is
/// asked once for every texture in it.
fn pictures_in(dir: &Path) -> Listing {
    static DIRS: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, Listing>>> =
        std::sync::Mutex::new(None);
    let mut g = DIRS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(Default::default);
    if let Some(l) = map.get(dir) {
        return l.clone();
    }
    let mut v = Vec::new();
    for (name, is_dir) in omsi_cfg::vfs::list_dir(dir).unwrap_or_default() {
        if is_dir {
            continue;
        }
        let p = Path::new(&name);
        let (Some(s), Some(x)) = (p.file_stem(), p.extension()) else {
            continue;
        };
        let x = x.to_string_lossy().to_ascii_lowercase();
        let s = s.to_string_lossy().to_ascii_lowercase();
        if EXTS.contains(&x.as_str()) && (s.contains('_') || s.contains('-')) {
            v.push((s, name));
        }
    }
    let l = std::sync::Arc::new(v);
    map.insert(dir.to_path_buf(), l.clone());
    l
}

fn load(p: &Path) -> Option<Image> {
    crate::decode_file(p)
        .map_err(|e| log::warn!("PBR map {}: {e}", p.display()))
        .ok()
}

/// An image no larger than `max` a side (halved as often as needed).
fn capped(mut img: Image, max: u32) -> Image {
    while img.width > max || img.height > max {
        let (rgba, w, h) = crate::bc::downsample(&img.rgba, img.width, img.height);
        img = Image {
            width: w,
            height: h,
            rgba,
            has_alpha: img.has_alpha,
        };
    }
    img
}

/// A channel of `img` at `w` x `h` (resized when it has another size).
fn channel(img: &Image, c: usize, w: u32, h: u32) -> Vec<u8> {
    let src = if img.width == w && img.height == h {
        img.rgba.clone()
    } else {
        crate::bc::resize(&img.rgba, img.width, img.height, w, h)
    };
    src.chunks_exact(4).map(|px| px[c]).collect()
}

/// Whether a picture is a tangent-space normal map: bluish throughout, its red and green
/// about the middle. OMSI's mods name their night maps `_n` as well (the Procity's
/// `bord_bwc_n.tga`, `[matl_nightmap]`): mostly black with the lit parts, they are not.
pub fn looks_like_normal_map(img: &Image) -> bool {
    let n = (img.width as usize * img.height as usize).max(1);
    // (every 7th texel is plenty)
    let (mut r, mut g, mut b, mut blue, mut k) = (0u64, 0u64, 0u64, 0usize, 0usize);
    for px in img.rgba.chunks_exact(4).step_by(7) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        if px[2] >= 110 && px[2] >= px[0].max(px[1]).saturating_sub(20) {
            blue += 1;
        }
        k += 1;
    }
    let _ = n;
    if k == 0 {
        return false;
    }
    let (r, g, b) = (
        r as f64 / k as f64,
        g as f64 / k as f64,
        b as f64 / k as f64,
    );
    b > 170.0
        && (r - 128.0).abs() < 45.0
        && (g - 128.0).abs() < 45.0
        && blue as f64 >= 0.9 * k as f64
}

/// A grey picture: red, green and blue within a few steps of each other nearly everywhere.
fn is_grey(img: &Image) -> bool {
    let (mut k, mut grey) = (0usize, 0usize);
    for px in img.rgba.chunks_exact(4).step_by(7) {
        let (lo, hi) = (px[0].min(px[1]).min(px[2]), px[0].max(px[1]).max(px[2]));
        if hi - lo <= 12 {
            grey += 1;
        }
        k += 1;
    }
    k > 0 && grey as f64 >= 0.95 * k as f64
}

/// Read and pack the maps of `files` for the GPU (None: nothing usable).
pub fn load_set(files: &PbrFiles) -> Option<PbrImages> {
    if files.is_empty() {
        return None;
    }
    // the first candidate that is a normal map
    let mut normal_kind = 0.0;
    let normal = files
        .normals
        .iter()
        .find_map(|(p, gl)| {
            let i = load(p)?;
            if looks_like_normal_map(&i) {
                normal_kind = if *gl { 2.0 } else { 1.0 };
                Some(capped(i, MAX_SIDE))
            } else {
                log::info!(
                    "{}: not a normal map (a night map by OMSI's `_n` naming?): left alone",
                    p.display()
                );
                None
            }
        })
        .or_else(|| {
            let path = files.height.as_deref()?;
            let Some(config) = files.height_config.as_deref() else {
                log::warn!(
                    "PBR height {}: missing .pbr.cfg physical dimensions; left alone",
                    path.display()
                );
                return None;
            };
            height::load(path, config)
                .map(|image| {
                    normal_kind = 3.0;
                    image
                })
                .map_err(|e| log::warn!("PBR height {}: {e}; left alone", path.display()))
                .ok()
        });
    // single maps are grey pictures (a `_r` or `_m` that is coloured is some other texture:
    // a seat's right half, a mask)
    let grey = |p: &Option<PathBuf>| -> Option<Image> {
        let i = load(p.as_deref()?)?;
        if is_grey(&i) {
            Some(i)
        } else {
            log::info!(
                "{}: not a grey PBR map: left alone",
                p.as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            None
        }
    };
    let rough = grey(&files.rough);
    let metal = grey(&files.metal);
    let ao = grey(&files.ao);
    let packed = files
        .packed
        .as_ref()
        .and_then(|(p, order)| load(p).map(|i| (i, *order)));
    let mut flags = [normal_kind, 0.0, 0.0, 0.0];
    let orm = if let Some((img, order)) = packed {
        let img = capped(img, MAX_SIDE);
        let (w, h) = (img.width, img.height);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for (i, px) in img.rgba.chunks_exact(4).enumerate() {
            rgba[i * 4] = px[order[0]];
            rgba[i * 4 + 1] = px[order[1]];
            rgba[i * 4 + 2] = px[order[2]];
        }
        flags[1] = 1.0;
        flags[2] = 1.0;
        flags[3] = 1.0;
        Some(Image {
            width: w,
            height: h,
            rgba,
            has_alpha: false,
        })
    } else if rough.is_some() || metal.is_some() || ao.is_some() {
        // the largest of the single maps sets the size
        let (w, h) = [&rough, &metal, &ao]
            .iter()
            .filter_map(|m| m.as_ref())
            .map(|m| (m.width, m.height))
            .max_by_key(|s| s.0 as u64 * s.1 as u64)
            .unwrap_or((1, 1));
        let (mut w, mut h) = (w, h);
        while w > MAX_SIDE || h > MAX_SIDE {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        let n = (w * h) as usize;
        let mut rgba = vec![255u8; n * 4];
        if let Some(a) = &ao {
            for (i, v) in channel(a, 0, w, h).into_iter().enumerate() {
                rgba[i * 4] = v;
            }
            flags[1] = 1.0;
        }
        if let Some(r) = &rough {
            for (i, v) in channel(r, 0, w, h).into_iter().enumerate() {
                rgba[i * 4 + 1] = if files.gloss { 255 - v } else { v };
            }
            flags[2] = 1.0;
        }
        if let Some(m) = &metal {
            for (i, v) in channel(m, 0, w, h).into_iter().enumerate() {
                rgba[i * 4 + 2] = v;
            }
            flags[3] = 1.0;
        }
        Some(Image {
            width: w,
            height: h,
            rgba,
            has_alpha: false,
        })
    } else {
        None
    };
    if normal.is_none() && orm.is_none() {
        return None;
    }
    Some(PbrImages { normal, orm, flags })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HeightFixture(PathBuf);

    impl HeightFixture {
        fn new(config: Option<&str>, explicit_normal: bool) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("omsi_pbr_height_{}_{id}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            image::RgbaImage::from_fn(8, 8, |x, _| {
                let h = (x * 30) as u8;
                image::Rgba([h, h, h, 255])
            })
            .save(dir.join("Road_HEIGHT.png"))
            .unwrap();
            if let Some(config) = config {
                std::fs::write(dir.join("Road.pbr.cfg"), config).unwrap();
            }
            if explicit_normal {
                image::RgbaImage::from_pixel(8, 8, image::Rgba([128, 128, 255, 255]))
                    .save(dir.join("Road_normal_gl.png"))
                    .unwrap();
            }
            Self(dir)
        }

        fn files(&self) -> PbrFiles {
            find(&self.0.join("Road.dds"))
        }
    }

    impl Drop for HeightFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn authored_height_needs_dimensions_and_produces_a_normal_set() {
        let missing = HeightFixture::new(None, false);
        assert!(missing.files().height.is_some());
        assert!(load_set(&missing.files()).is_none());
        let invalid = HeightFixture::new(Some("[height_scale]\n0\n[texture_size]\n1\n1"), false);
        assert!(load_set(&invalid.files()).is_none());
        let valid = HeightFixture::new(Some("[height_scale]\n0.025\n[texture_size]\n2\n2"), false);
        let loaded = load_set(&valid.files()).unwrap();
        assert_eq!(loaded.flags, [3.0, 0.0, 0.0, 0.0]);
        assert!(loaded.orm.is_none());
        let normal = loaded.normal.unwrap();
        assert_eq!((normal.width, normal.height), (8, 8));
        assert!(
            normal.rgba[(3 * 8 + 3) * 4] < 128,
            "a rising U height tilts the normal toward -U"
        );
    }

    #[test]
    fn explicit_normal_precedes_height_even_with_invalid_height_metadata() {
        let fixture = HeightFixture::new(Some("[height_scale]\nNaN"), true);
        let loaded = load_set(&fixture.files()).unwrap();
        assert_eq!(loaded.flags, [2.0, 0.0, 0.0, 0.0]);
        assert!(
            loaded
                .normal
                .unwrap()
                .rgba
                .chunks_exact(4)
                .all(|p| p == [128, 128, 255, 255])
        );
    }

    #[test]
    fn a_night_map_is_no_normal_map() {
        let flat = Image {
            width: 4,
            height: 4,
            rgba: [128u8, 128, 255, 255].repeat(16),
            has_alpha: false,
        };
        assert!(looks_like_normal_map(&flat));
        let mut night = [0u8, 0, 0, 255].repeat(16);
        night[0..4].copy_from_slice(&[255, 220, 120, 255]);
        let night = Image {
            width: 4,
            height: 4,
            rgba: night,
            has_alpha: false,
        };
        assert!(!looks_like_normal_map(&night));
    }

    #[test]
    fn finds_a_set_beside_the_texture() {
        let dir = std::env::temp_dir().join(format!("omsi_pbr_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for f in [
            "Bus.dds",
            "Bus_NN.png",
            "bus_rr.tga",
            "bus_mm.png",
            "bus_aa.jpg",
            "busstop_nn.png",
            "lamp.tga",
            "lamp_r.tga",
            "lamp_n.tga",
            "tram.dds",
            "tram_normal_gl.png",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let set = find(&dir.join("Bus.dds"));
        assert_eq!(
            set.normal
                .as_deref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string()),
            Some("Bus_NN.png".into())
        );
        assert!(set.rough.is_some() && set.metal.is_some() && set.ao.is_some());
        assert!(!set.normal_gl && !set.gloss);
        // another texture's maps are not taken (busstop_nn is not bus_nn)
        let other = find(&dir.join("busstop.dds"));
        assert!(other.normal.is_some() && other.rough.is_none());
        // (single letters are no PBR maps: OMSI's mods name night maps so)
        assert!(find(&dir.join("lamp.tga")).is_empty());
        let gl = find(&dir.join("tram.dds"));
        assert!(gl.normal.is_some() && gl.normal_gl);
        assert!(find(&dir.join("ferry.dds")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
