//! Saving a livery into the game: every painted texture at its full size (a band of rows at a
//! time), its mip chain, BC1 or BC3 (BC3 when the base has any alpha under 255), as a DDS with a
//! hash in its name; then the `.cti` beside the bus's own (its name begins with `~`, so the
//! numbers of the liveries there keep). Everything goes into openOMSI's content folder - its copy
//! of the bus's `[CTC]` folder - never into the OMSI 2 installation.

use super::bake::{BusGeom, Outside, Zones};
use super::model::{self, Layer};
use super::paint::{self, Canvas, Context, RasterCache};
use super::shapes::Raster;
use sha2::Digest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A painted texture to write.
#[derive(Clone)]
pub struct Target {
    pub index: u8,
    /// The model's file name for it (the stem of ours).
    pub default: String,
    /// The `[CTCTexture]` slots that take it, per part of the bus (an index into `Job::parts`).
    pub slots: Vec<(usize, String)>,
    pub base: PathBuf,
    /// The bus maker's template for it (MA, MU).
    pub template: Option<[PathBuf; 2]>,
}

/// A part of the bus (the front, an articulated bus's rear section): the content folder's copy
/// of its `[CTC]` folder, where its `.cti` goes, and the variables its livery sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub ctc_dir: PathBuf,
    pub setvars: Vec<(String, f32)>,
}

pub struct Job {
    pub name: String,
    pub nnnn: u32,
    pub date: String,
    /// The content folder, and the bus's parts (None: a part without a `[CTC]` folder).
    pub content: PathBuf,
    pub parts: Vec<Option<Part>>,
    pub targets: Vec<Target>,
    pub geom: Arc<BusGeom>,
    pub outside: Arc<Outside>,
    /// The colour zones of each target.
    pub zones: Arc<Vec<Zones>>,
    pub layers: Vec<Layer>,
    pub pictures: Arc<HashMap<String, Arc<Raster>>>,
    pub mirror: Option<f32>,
    /// What the livery's last save wrote (relative to the content folder): replaced.
    pub old: Vec<String>,
}

pub enum Msg {
    Progress(String, f32),
    Done(Vec<String>),
    Failed(String),
}

/// Rows painted at a time at the full size.
const BAND: u32 = 256;

/// Whether `p` lies in the content folder (and nowhere else may be written).
pub fn inside(p: &Path, content: &Path) -> bool {
    let norm = |p: &Path| p.components().filter(|c| !matches!(c, std::path::Component::CurDir)).collect::<PathBuf>();
    !p.components().any(|c| matches!(c, std::path::Component::ParentDir)) && norm(p).starts_with(norm(content))
}

pub fn run(job: Job, tx: std::sync::mpsc::Sender<Msg>) {
    match write(&job, &tx) {
        Ok(files) => {
            let _ = tx.send(Msg::Done(files));
        }
        Err(e) => {
            log::warn!("livery studio: saving '{}' failed: {e}", job.name);
            let _ = tx.send(Msg::Failed(e));
        }
    }
}

/// The `[CTC]` folders of the bus's parts, each once (an articulated bus's parts often share
/// theirs): the folder, its variables (the first part's), and per part which of them it is.
pub fn ctc_folders(parts: &[Option<Part>]) -> (Vec<(PathBuf, Vec<(String, f32)>)>, Vec<Option<usize>>) {
    let mut dirs: Vec<(PathBuf, Vec<(String, f32)>)> = Vec::new();
    let mut of_part = Vec::new();
    for p in parts {
        of_part.push(p.as_ref().map(|p| match dirs.iter().position(|d| d.0 == p.ctc_dir) {
            Some(k) => k,
            None => {
                dirs.push((p.ctc_dir.clone(), p.setvars.clone()));
                dirs.len() - 1
            }
        }));
    }
    (dirs, of_part)
}

fn write(job: &Job, tx: &std::sync::mpsc::Sender<Msg>) -> Result<Vec<String>, String> {
    let (dirs, of_part) = ctc_folders(&job.parts);
    if dirs.is_empty() {
        return Err("the bus has no [CTC] folder".into());
    }
    for (d, _) in &dirs {
        if !inside(d, &job.content) {
            return Err(format!("{} is not in the content folder", d.display()));
        }
    }
    let slug = model::slug(&job.name);
    let folder = model::texture_folder(job.nnnn, &slug);
    // per folder: the .cti's items
    let mut items: Vec<Vec<(String, String)>> = vec![Vec::new(); dirs.len()];
    let mut written = Vec::new();
    let n = job.targets.len().max(1) as f32;
    for (k, t) in job.targets.iter().enumerate() {
        let _ = tx.send(Msg::Progress(omsi_ui::tr("Painting the textures…").into_owned(), k as f32 / n));
        let from = Painting { geom: &job.geom, outside: &job.outside, zones: &job.zones, layers: &job.layers, pictures: &job.pictures, mirror: job.mirror };
        let said = |f: f32| {
            let _ = tx.send(Msg::Progress(omsi_ui::tr("Painting the textures…").into_owned(), (k as f32 + f * 0.8) / n));
        };
        let Some((out, w, h, alpha)) = paint_full(&from, k, t.index, &t.base, t.template.as_ref(), &|| false, &said)? else { continue };
        let _ = tx.send(Msg::Progress(omsi_ui::tr("Compressing…").into_owned(), (k as f32 + 0.85) / n));
        let format = if alpha { omsi_texture::bc::Bc::Bc3 } else { omsi_texture::bc::Bc::Bc1 { punch: false } };
        let dds = omsi_texture::dds::encode(&out, w, h, format);
        let hash = sha2::Sha256::digest(&dds);
        let stem = Path::new(&t.default).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "paint".into());
        let file = format!("{stem}_{:02x}{:02x}{:02x}{:02x}.dds", hash[0], hash[1], hash[2], hash[3]);
        // into the folder of every part that takes it, its slots into that folder's .cti
        for (d, (dir, _)) in dirs.iter().enumerate() {
            let slots: Vec<&String> = t.slots.iter().filter(|(p, _)| of_part.get(*p).copied().flatten() == Some(d)).map(|(_, s)| s).collect();
            if slots.is_empty() {
                continue;
            }
            let tex_dir = dir.join(folder.replace('\\', std::path::MAIN_SEPARATOR_STR));
            std::fs::create_dir_all(&tex_dir).map_err(|e| format!("{}: {e}", tex_dir.display()))?;
            let path = tex_dir.join(&file);
            std::fs::write(&path, &dds).map_err(|e| format!("{}: {e}", path.display()))?;
            let r = rel(&path, &job.content);
            if !written.contains(&r) {
                written.push(r);
            }
            for slot in slots {
                if !items[d].iter().any(|(s, _)| s.eq_ignore_ascii_case(slot)) {
                    items[d].push((slot.clone(), format!("{folder}\\{file}")));
                }
            }
        }
    }
    // the .cti files last: until they are there, the game knows nothing of the textures. Every
    // part's under the same name, which pairs them in the game.
    for (d, (dir, setvars)) in dirs.iter().enumerate() {
        if items[d].is_empty() {
            continue;
        }
        let text = model::cti_text(&job.name, job.nnnn, &job.date, &items[d], setvars)?;
        let cti = dir.join(model::cti_name(job.nnnn, &slug));
        std::fs::write(&cti, model::to_cp1252(&text)).map_err(|e| format!("{}: {e}", cti.display()))?;
        written.push(rel(&cti, &job.content));
    }
    // what the save before wrote and this one did not
    for old in &job.old {
        if written.contains(old) || !old.contains(model::PREFIX) {
            continue;
        }
        let p = job.content.join(old);
        if inside(&p, &job.content) {
            let _ = std::fs::remove_file(&p);
            if let Some(parent) = p.parent().filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().contains('_'))) {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    // the folder listings this process keeps are read again: the launcher had listed the
    // content folder before the livery was there (often before its bus's folder was), and
    // its `.cti` stayed unseen - the bus step then found no paint of that name and showed
    // the bus in the model's own white base
    omsi_cfg::content_changed();
    log::info!("livery studio: '{}' saved as {} files in {}", job.name, written.len(), dirs.iter().map(|d| d.0.display().to_string()).collect::<Vec<_>>().join(", "));
    Ok(written)
}

fn rel(p: &Path, content: &Path) -> String {
    p.strip_prefix(content).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Texels a metre of target `index` at `w` x `h`.
/// What the textures are painted from: the bus's shape, its colour zones per texture, the
/// layers, their pictures and the mirror plane.
pub struct Painting<'a> {
    pub geom: &'a BusGeom,
    pub outside: &'a Outside,
    pub zones: &'a [Zones],
    pub layers: &'a [Layer],
    pub pictures: &'a Arc<HashMap<String, Arc<Raster>>>,
    pub mirror: Option<f32>,
}

/// Texture `k` (its `index` on the bus, its `base` file and maker's template) painted at its full
/// size, a band of rows at a time - as the save writes it, and as the studio shows it once the
/// paint rests: its picture, size and whether its base has alpha. `progress` hears how far it
/// is (0..1); None when `stop` said so between two bands.
pub fn paint_full(from: &Painting, k: usize, index: u8, base: &Path, template: Option<&[PathBuf; 2]>, stop: &dyn Fn() -> bool, progress: &dyn Fn(f32)) -> Result<Option<(Vec<u8>, u32, u32, bool)>, String> {
    let img = omsi_texture::decode_file(base).map_err(|e| format!("{}: {e}", base.display()))?;
    let (w, h) = model::output_size(img.width, img.height);
    let base = if (w, h) != (img.width, img.height) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, w, h) } else { img.rgba };
    let alpha = base.chunks_exact(4).any(|p| p[3] < 255);
    let mut out = vec![0u8; (w * h * 4) as usize];
    let mut cache = RasterCache::default();
    let mut ops: Option<Vec<paint::Prepared>> = None;
    let template = template.and_then(|tp| super::template_pictures(tp, w, h));
    let none = Zones::default();
    let mut y = 0;
    while y < h {
        if stop() {
            return Ok(None);
        }
        let y1 = (y + BAND).min(h);
        let bake = super::bake::Bake::build(&from.geom.tris, index, w, h, y, y1, from.outside);
        let cv = Canvas::with(bake, &base, from.zones.get(k).unwrap_or(&none)).with_template(template.as_ref().map(|[ma, mu]| paint::Template::rows(ma, mu, w, y, y1)));
        if ops.is_none() {
            // (the decals drawn at the full texture's density once, for every band)
            let density = density_of(from.geom, index, w, h);
            let cx = Context { pictures: from.pictures.clone(), mirror: from.mirror, density, max_px: 4096 };
            ops = Some(paint::prepare(from.layers, &cx, &mut cache));
        }
        let (rgba, _) = paint::composite(&cv, ops.as_deref().unwrap_or(&[]), &from.geom.dims, None, None);
        out[(y * w * 4) as usize..(y1 * w * 4) as usize].copy_from_slice(&rgba);
        y = y1;
        progress(y as f32 / h as f32);
    }
    Ok(Some((out, w, h, alpha)))
}

pub fn density_of(geom: &BusGeom, index: u8, w: u32, h: u32) -> f32 {
    let mut d: Vec<f32> = geom
        .tris
        .iter()
        .filter(|t| t.target == Some(index))
        .take(4096)
        .filter_map(|t| {
            let uv = (t.uv[1] - t.uv[0]).perp_dot(t.uv[2] - t.uv[0]).abs() * (w * h) as f32;
            let world = (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length();
            (world > 1e-8 && uv > 0.0).then(|| (uv / world).sqrt())
        })
        .collect();
    d.sort_by(|a, b| a.total_cmp(b));
    d.get(d.len() / 2).copied().unwrap_or(100.0)
}

/// The next free number among the studio's liveries in `dir`.
pub fn next_number(dir: &Path) -> u32 {
    let mut max = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(rest) = n.strip_prefix(&format!("~{}_", model::PREFIX)) {
                if let Ok(v) = rest.get(..4).unwrap_or("").parse::<u32>() {
                    max = max.max(v);
                }
            }
        }
    }
    max + 1
}

#[cfg(test)]
mod tests {
    use super::super::bake::tests::box_bus;
    use super::*;

    #[test]
    fn a_livery_is_written_into_the_content_folder_and_read_by_the_game() {
        let tmp = std::env::temp_dir().join(format!("livery-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let content = tmp.join("content");
        let ctc = content.join("Vehicles").join("Box").join("Texture");
        std::fs::create_dir_all(&ctc).unwrap();
        // a base of 64 x 32 with a reflection mask
        let base = tmp.join("base.png");
        image::RgbaImage::from_pixel(66, 32, image::Rgba([30, 60, 160, 128])).save(&base).unwrap();
        let geom = Arc::new(box_bus());
        let outside = Arc::new(Outside::build(&geom.tris));
        let mut layers = vec![model::layer("Base", model::base_colour("#ff0000"))];
        layers[0].detail = 0.0;
        // an articulated bus: the rear section's [CTC] folder of its own, its slot of its own
        let trail = content.join("Vehicles").join("Box").join("TextureTrail");
        std::fs::create_dir_all(&trail).unwrap();
        let job = Job {
            name: "Lucstad".into(),
            nnnn: next_number(&ctc),
            date: "04-10-2026".into(),
            content: content.clone(),
            parts: vec![Some(Part { ctc_dir: ctc.clone(), setvars: vec![("vis_x".into(), 1.0)] }), Some(Part { ctc_dir: trail.clone(), setvars: Vec::new() })],
            targets: vec![Target { index: 0, default: "box.tga".into(), slots: vec![(0, "farbschema".into()), (1, "farbschema_trail".into())], base: base.clone(), template: None }],
            geom,
            outside,
            zones: Arc::new(vec![Zones::of(&[[30, 60, 160]])]),
            layers,
            pictures: Arc::new(HashMap::new()),
            mirror: Some(0.0),
            old: vec!["Vehicles/Box/Texture/LiveryStudio/0001_old/x.dds".into()],
        };
        let (tx, rx) = std::sync::mpsc::channel();
        run(job, tx);
        let files = loop {
            match rx.recv().unwrap() {
                Msg::Done(f) => break f,
                Msg::Failed(e) => panic!("{e}"),
                Msg::Progress(..) => {}
            }
        };
        assert_eq!(files.len(), 4, "{files:?}");
        assert!(files[2].ends_with("Texture/~LiveryStudio_0001_lucstad.cti"), "{files:?}");
        assert!(files[3].ends_with("TextureTrail/~LiveryStudio_0001_lucstad.cti"), "{files:?}");
        // the rear section's own folder has the livery under the same name, for its own slot
        let rear = omsi_sim::vehicle::load_paint_schemes(&trail);
        let r = rear.iter().find(|s| s.name == "Lucstad").expect("the rear section's livery");
        assert_eq!(r.textures.len(), 1);
        assert_eq!(r.textures[0].0, "farbschema_trail");
        assert!(omsi_cfg::resolve_path(&r.dir, &r.textures[0].1).is_file());
        // the game's own reader finds it, its texture in its folder
        let schemes = omsi_sim::vehicle::load_paint_schemes(&ctc);
        let s = schemes.iter().find(|s| s.name == "Lucstad").expect("the livery");
        assert_eq!(s.textures[0].0, "farbschema");
        assert_eq!(s.set_vars, vec![("vis_x".to_string(), 1.0)]);
        let dds = omsi_cfg::resolve_path(&s.dir, &s.textures[0].1);
        assert!(dds.is_file(), "{}", dds.display());
        let bytes = std::fs::read(&dds).unwrap();
        assert_eq!(&bytes[84..88], b"DXT5", "the base's alpha kept: BC3");
        let img = omsi_texture::dds::decode(&bytes).unwrap();
        assert_eq!((img.width, img.height), (64, 32));
        let p = &img.rgba[(16 * 64 + 48) * 4..(16 * 64 + 48) * 4 + 4];
        assert!(p[0] > 240 && p[1] < 20 && (p[3] as i32 - 128).abs() <= 3, "{p:?}");
        assert_eq!(next_number(&ctc), 2);
        assert!(!inside(&tmp.join("elsewhere"), &content));
        assert!(!inside(&content.join("..").join("x"), &content));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Luc's white bus: the launcher had listed the content folder before the save (its bus
    /// step looked at the bus), kept those listings, and after the save found no paint of the
    /// livery's name - the bus showed in the model's own white base. Read back the way the
    /// showroom reads it (the bus type, the paint by name, its texture swaps and the texture
    /// search with the paint's folder first, the GPU loader), in the same process.
    #[test]
    fn a_saved_livery_is_shown_at_once_by_the_launcher_that_saved_it() {
        let tmp = std::env::temp_dir().join(format!("livery-shown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (content, install) = (tmp.join("content"), tmp.join("OMSI 2"));
        let bus_dir = install.join("Vehicles").join("Box");
        std::fs::create_dir_all(bus_dir.join("model")).unwrap();
        std::fs::create_dir_all(bus_dir.join("Texture")).unwrap();
        std::fs::create_dir_all(content.join("Vehicles")).unwrap();
        std::fs::write(bus_dir.join("box.bus"), "[model]\r\nmodel\\model.cfg\r\n").unwrap();
        std::fs::write(bus_dir.join("model").join("model.cfg"), "[CTC]\r\nColorscheme\r\nTexture\r\n0\r\n\r\n[CTCTexture]\r\nfarbschema\r\nbox.dds\r\n").unwrap();
        std::fs::write(bus_dir.join("Texture").join("stock.cti"), "[item]\r\nStock\r\nfarbschema\r\nbox.dds\r\n").unwrap();
        // the model's own texture: a white base
        let base = bus_dir.join("Texture").join("box.dds");
        let white = vec![255u8; 64 * 32 * 4];
        std::fs::write(&base, omsi_texture::dds::encode(&white, 64, 32, omsi_texture::bc::Bc::Bc1 { punch: false })).unwrap();
        omsi_cfg::add_content_root(content.clone());
        omsi_cfg::add_content_root(install.clone());
        let bus = bus_dir.join("box.bus");
        // the bus step looks at the bus before the livery is there
        let before = omsi_sim::VehicleType::load(&install, &bus).unwrap();
        assert_eq!(crate::spawn::paint_scheme(&before, Some("Lucstad")), None);
        // the save, into the content folder's copy of the [CTC] folder (as `save` makes it)
        let ctc = content.join(bus_dir.join("Texture").strip_prefix(&install).unwrap());
        std::fs::create_dir_all(&ctc).unwrap();
        let geom = Arc::new(box_bus());
        let outside = Arc::new(Outside::build(&geom.tris));
        let mut layers = vec![model::layer("Base", model::base_colour("#ff0000"))];
        layers[0].detail = 0.0;
        let job = Job {
            name: "Lucstad".into(),
            nnnn: next_number(&ctc),
            date: "05-10-2026".into(),
            content: content.clone(),
            parts: vec![Some(Part { ctc_dir: ctc.clone(), setvars: Vec::new() })],
            targets: vec![Target { index: 0, default: "box.dds".into(), slots: vec![(0, "farbschema".into())], base: base.clone(), template: None }],
            geom,
            outside,
            zones: Arc::new(vec![Zones::of(&[[255, 255, 255]])]),
            layers,
            pictures: Arc::new(HashMap::new()),
            mirror: Some(0.0),
            old: Vec::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        run(job, tx);
        while let Ok(m) = rx.recv() {
            match m {
                Msg::Done(_) => break,
                Msg::Failed(e) => panic!("{e}"),
                Msg::Progress(..) => {}
            }
        }
        // the same launcher picks it: the paint is there, its body texture the one written
        let vt = omsi_sim::VehicleType::load(&install, &bus).unwrap();
        let scheme = crate::spawn::paint_scheme(&vt, Some("Lucstad"));
        let names: Vec<&str> = vt.paint_schemes.iter().map(|s| s.name.as_str()).collect();
        assert!(scheme.is_some(), "the saved livery among the bus's paints: {names:?}");
        let (subst, scheme_dir) = vt.scheme_substitutions(scheme.unwrap());
        let name = subst.get("box.dds").expect("the body texture swapped").clone();
        let mut dirs = vt.texture_dirs(&install);
        dirs.insert(0, scheme_dir.unwrap());
        let dirs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
        let found = omsi_texture::find_texture(&name, &dirs).expect("the livery's texture found");
        assert!(found.starts_with(&content) && found.to_string_lossy().contains("LiveryStudio"), "{}", found.display());
        let bytes = std::fs::read(&found).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 1, "depth 1, as OMSI's own textures");
        let (t, _) = omsi_texture::gpu::load_gpu_bytes(&bytes, &found, omsi_texture::gpu::GpuOptions { bc: true, compress: false }).unwrap();
        assert_eq!((t.width, t.height, t.levels.len()), (64, 32, 7), "the whole chain as blocks");
        let img = omsi_texture::decode_file(&found).unwrap();
        let p = &img.rgba[(16 * 64 + 48) * 4..(16 * 64 + 48) * 4 + 4];
        assert!(p[0] > 240 && p[1] < 20 && p[2] < 20, "red, not the white base: {p:?}");
        omsi_cfg::remove_content_root(&content);
        omsi_cfg::remove_content_root(&install);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parts_sharing_a_ctc_folder_write_one_cti() {
        let a = PathBuf::from("c/Vehicles/Bus/Texture/Werbung");
        let parts = vec![Some(Part { ctc_dir: a.clone(), setvars: vec![("x".into(), 1.0)] }), Some(Part { ctc_dir: a.clone(), setvars: Vec::new() }), None];
        let (dirs, of) = ctc_folders(&parts);
        assert_eq!(dirs, vec![(a, vec![("x".to_string(), 1.0)])]);
        assert_eq!(of, vec![Some(0), Some(0), None]);
    }
}
