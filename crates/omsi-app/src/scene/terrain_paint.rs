//! Terrain paint masks, vegetation sway, seasonal textures and surface maps.
use super::*;

/// A texture of a tile's own (its night light map, the roads' cut, a ground paint mask) for
/// the GPU: one level as always, compressed where the device takes blocks and the picture
/// stays close (`mask`: only the alpha channel is read).
pub(super) fn tile_texture(img: Image, mask: bool) -> TextureData {
    omsi_texture::gpu::prepare_single_level(img, mask).0
}

/// Edge (texels) a ground paint mask is brought up to before it is smoothed.
pub(super) const PAINT_MASK_MIN: usize = 512;

/// A ground paint mask made ready to be drawn: the editor's brush writes nothing but 0 and
/// 255, one texel every 0.6-3 m (2^params[0] texels a tile). Sampled as it is, the edge of
/// a car park or a field follows the texel grid, and the shader's sharpening (see
/// `fs_main`) turned that into hard steps - a staircase along every painted edge, metres
/// long where the edge runs nearly along the grid. Brought to at least
/// `PAINT_MASK_MIN` texels (bilinearly) and blurred by a little under one of its own
/// texels, the mask becomes a soft ramp whose half-way line is a smooth curve through the
/// steps' middles; the shader's sharpening then gives a crisp edge along that curve.
/// Returns the alpha as RGBA (white) and the new edge lengths.
pub(super) fn smooth_paint_mask(rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let s = (PAINT_MASK_MIN / w.max(1)).max(1).min(PAINT_MASK_MIN / h.max(1)).max(1);
    let (dw, dh) = (w * s, h * s);
    let src = |i: isize, j: isize| -> f32 {
        let i = i.clamp(0, w as isize - 1) as usize;
        let j = j.clamp(0, h as isize - 1) as usize;
        rgba[(j * w + i) * 4 + 3] as f32
    };
    // bilinear magnification, texel centres aligned as the GPU samples them
    let mut a = vec![0f32; dw * dh];
    for y in 0..dh {
        let fy = (y as f32 + 0.5) / s as f32 - 0.5;
        let (j0, ty) = (fy.floor() as isize, fy - fy.floor());
        for x in 0..dw {
            let fx = (x as f32 + 0.5) / s as f32 - 0.5;
            let (i0, tx) = (fx.floor() as isize, fx - fx.floor());
            let top = src(i0, j0) * (1.0 - tx) + src(i0 + 1, j0) * tx;
            let bottom = src(i0, j0 + 1) * (1.0 - tx) + src(i0 + 1, j0 + 1) * tx;
            a[y * dw + x] = top * (1.0 - ty) + bottom * ty;
        }
    }
    // separable Gaussian, sigma 0.85 of a source texel (edges clamped)
    let sigma = 0.85 * s as f32;
    let r = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r)
        .map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let norm: f32 = kernel.iter().sum();
    let mut tmp = vec![0f32; dw * dh];
    for y in 0..dh {
        let row = &a[y * dw..][..dw];
        for x in 0..dw {
            let mut acc = 0.0;
            for (k, wk) in kernel.iter().enumerate() {
                let xi = (x as isize + k as isize - r).clamp(0, dw as isize - 1) as usize;
                acc += row[xi] * wk;
            }
            tmp[y * dw + x] = acc / norm;
        }
    }
    let mut out = vec![255u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = 0.0;
            for (k, wk) in kernel.iter().enumerate() {
                let yi = (y as isize + k as isize - r).clamp(0, dh as isize - 1) as usize;
                acc += tmp[yi * dw + x] * wk;
            }
            out[(y * dw + x) * 4 + 3] = (acc / norm).round().clamp(0.0, 255.0) as u8;
        }
    }
    (out, dw, dh)
}

/// The alpha (0..1) of an image at (u, v) in 0..1, sampled bilinearly with the texel
/// centres where the GPU has them (edges clamped).
pub(super) fn bilinear_alpha(img: &Image, u: f32, v: f32) -> f32 {
    let (w, h) = (img.width as isize, img.height as isize);
    let fx = u * w as f32 - 0.5;
    let fy = v * h as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let a = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w - 1) as usize;
        let y = y.clamp(0, h - 1) as usize;
        img.rgba[(y * w as usize + x) * 4 + 3] as f32 / 255.0
    };
    let (x0, y0) = (x0 as isize, y0 as isize);
    let top = a(x0, y0) * (1.0 - tx) + a(x0 + 1, y0) * tx;
    let bottom = a(x0, y0 + 1) * (1.0 - tx) + a(x0 + 1, y0 + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Split the material slots of an object mesh whose texture carries `[terrainmapping]`
/// off into a mesh of their own. OMSI does not draw such a slot with its texture (the
/// stock ones are a 1x1 placeholder, TH_Wald's Gras01.dds a single green pixel): the slot
/// takes on the map's first ground texture, so that the grass on top of a rock, a
/// traffic island or a roundabout runs on seamlessly from the meadow around it. The split
/// mesh therefore gets the terrain's own uv (tile space, see `build_terrain_mesh`) for
/// the object placed at `pos`/`xf` on the tile at `origin`, and is drawn with the tile's
/// uncut base material. Returns the mesh without those slots and the split-off one.
#[cfg(test)]
pub(super) fn split_terrain_mapped(
    src: &MeshData,
    slots: &[usize],
    pos: DVec3,
    xf: Mat4,
    origin: DVec3,
) -> (MeshData, MeshData) {
    (terrain_rest(src, slots), terrain_ground(src, slots, pos, xf, origin))
}

/// The mesh without its `[terrainmapping]` slots (see `split_terrain_mapped`): the same for
/// every placement of a type, so it is made once per type.
pub(super) fn terrain_rest(src: &MeshData, slots: &[usize]) -> MeshData {
    let mut rest = src.clone();
    rest.ranges.retain(|r| !slots.contains(&(r.2 as usize)));
    rest
}

/// The `[terrainmapping]` slots of a mesh in tile space (see `split_terrain_mapped`).
pub(super) fn terrain_ground(src: &MeshData, slots: &[usize], pos: DVec3, xf: Mat4, origin: DVec3) -> MeshData {
    let mut ground = MeshData {
        one_sided: src.one_sided,
        ..MeshData::default()
    };
    let mut map: HashMap<u32, u32> = HashMap::new();
    let to_tile = (pos - origin) / tile_size();
    for &(start, count, slot) in &src.ranges {
        if !slots.contains(&(slot as usize)) {
            continue;
        }
        for &k in &src.indices[start as usize..(start + count) as usize] {
            let v = *map.entry(k).or_insert_with(|| {
                let p = src.positions[k as usize];
                let local = xf.transform_point3(p).as_dvec3() / tile_size() + to_tile;
                ground.positions.push(p);
                ground.normals.push(src.normals.get(k as usize).copied().unwrap_or(glam::Vec3::Z));
                ground.uvs.push(glam::Vec2::new(local.x as f32, local.y as f32));
                ground.positions.len() as u32 - 1
            });
            ground.indices.push(v);
        }
    }
    let n = ground.indices.len() as u32;
    if n > 0 {
        ground.ranges.push((0, n, 0));
    }
    ground
}

/// Two crossed unit quads (1 m wide, 1 m tall, centred at x=0, standing on z=0).
/// Windy trees: whether a scenery object is a plant whose leaves the wind moves - by its
/// `[groups]` (the stock "Trees LQ", "Deciduous", "Shrubbery", "Arbors", "Plants" and their
/// kin in other content), or, for an object filed in no group, by the words of its file's
/// name - and if so how much it gives to the wind. Not a `[tree]` (its cards have their
/// own) and not a backdrop (a forest painted on one wide card).
pub(super) fn vegetation_give_of(sco: &omsi_scenery::sco::SceneryObject) -> Option<f32> {
    // (not "wald": a whole DLC's objects are grouped as "Thüringer Wald", and its houses'
    // cut-out windows and railings swayed in the wind, #1775)
    const GROUP_WORDS: [&str; 17] = ["tree", "baum", "bäume", "baeume", "deciduous", "conifer", "shrub", "bush", "busch", "strauch", "hecke", "hedge", "plant", "pflanz", "arbor", "vegetation", "forest"];
    const NAME_WORDS: [&str; 15] = ["tree", "trees", "baum", "shrub", "shrubbery", "bush", "busch", "strauch", "hecke", "hedge", "arbor", "chestnut", "kastanie", "palm", "plant"];
    if sco.tree.is_some() {
        return None;
    }
    let groups: Vec<String> = sco.groups.iter().map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty()).collect();
    if groups.iter().any(|g| g.contains("backdrop")) {
        return None;
    }
    let stem = sco.path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let plant = if groups.is_empty() {
        stem.split(|c: char| !c.is_alphabetic()).any(|w| NAME_WORDS.contains(&w))
    } else {
        groups.iter().any(|g| GROUP_WORDS.iter().any(|w| g.contains(w)))
    };
    plant.then(|| vegetation_give(&[&stem, &groups.join(" ")]))
}

/// The windy trees' pivot of a plant model's leaf slot (`MaterialExtra::sway`, mesh units):
/// the crown leaves the trunk where the slot's lowest leaves hang, and at a quarter of the
/// plant's height at least (a picture of the whole tree on crossed cards, trunk and all,
/// is one slot); a plant up to 3 m tall - a shrub, a hedge - bends from the ground.
pub(super) fn foliage_sway(meshes: &[(MeshData, Vec<omsi_o3d::Material>, Vec<MaterialDef>)], mesh: &MeshData, slot: u32, give: f32) -> Option<[f32; 3]> {
    let (lo, hi) = meshes
        .iter()
        .flat_map(|(m, _, _)| m.positions.iter())
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    let leaf_lo = mesh
        .ranges
        .iter()
        .filter(|r| r.2 == slot)
        .flat_map(|r| mesh.indices.get(r.0 as usize..(r.0 + r.1) as usize).unwrap_or(&[]))
        .filter_map(|&i| mesh.positions.get(i as usize))
        .fold(f32::MAX, |lo, p| lo.min(p.z));
    let height = hi - lo;
    if !(height > 0.2) || leaf_lo == f32::MAX {
        return None;
    }
    let pivot = if height <= 3.0 { lo } else { leaf_lo.max(lo + 0.25 * height) };
    (hi - pivot > 0.1).then_some([pivot, hi, if height <= 3.0 { 0.5 * give } else { give }])
}

/// How much a tree gives to the wind by its kind (windy trees): a conifer's needles and
/// stiff whorls of branches move less than a broadleaf's crown.
pub(super) fn vegetation_give(names: &[&str]) -> f32 {
    const CONIFER: [&str; 12] = ["fir", "tanne", "fichte", "kiefer", "pine", "spruce", "conifer", "nadel", "cypress", "zypresse", "thuja", "larch"];
    let conifer = names.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        CONIFER.iter().any(|c| n.contains(c))
    });
    if conifer { 0.6 } else { 1.0 }
}

/// The windy trees' pivot of a `[tree]`'s cards (`MaterialExtra::sway`, in the card's units
/// of its height): the crown leaves the trunk at about a quarter of the picture's height; a
/// shrub (a type no taller than 3 m) bends from the ground.
pub(super) fn tree_card_sway(ot: &ObjectType, texture: &str) -> [f32; 3] {
    let shrub = ot.sco.tree.as_ref().is_some_and(|t| t.2 > 0.0 && t.2 <= 3.0);
    let path = ot.sco.path.to_string_lossy();
    [if shrub { SHRUB_CARD_PIVOT } else { TREE_CARD_PIVOT }, 1.0, vegetation_give(&[texture, &path])]
}

/// Where a `[tree]` card's crown leaves its trunk, and a shrub's (of the card's height).
pub(super) const TREE_CARD_PIVOT: f32 = 0.28;
pub(super) const SHRUB_CARD_PIVOT: f32 = 0.03;

pub(super) fn tree_quad_mesh() -> MeshData {
    // Each card is a small grid of quads rather than one: a flat grid is the same picture,
    // and it gives the windy trees (shader.wgsl `tree_sway`) vertices to bend the crown by
    // while the trunk below the pivot stays where it stands. The rows lie on the pivots
    // (the bend starts exactly there) and evenly over the crown, enough for its curve; the
    // bend is the same across the card, so two columns do (the boughs' lobes are a crown
    // wide). 12 quads a side: every tree of a forest pays for it.
    const TREE_COLS: u32 = 2;
    const ROWS: [f32; 7] = [0.0, SHRUB_CARD_PIVOT, TREE_CARD_PIVOT, 0.46, 0.64, 0.82, 1.0];
    const TREE_ROWS: u32 = ROWS.len() as u32 - 1;
    let mut m = MeshData::default();
    for (dx, dy) in [(0.5f32, 0.0f32), (0.0, 0.5)] {
        let base = m.positions.len() as u32;
        for r in 0..=TREE_ROWS {
            for c in 0..=TREE_COLS {
                let u = c as f32 / TREE_COLS as f32;
                let z = ROWS[r as usize];
                let sx = u * 2.0 - 1.0;
                m.positions.push(glam::Vec3::new(dx * sx, dy * sx, z));
                m.normals.push(glam::Vec3::Z);
                m.uvs.push(glam::Vec2::new(u, 1.0 - z));
            }
        }
        let at = |c: u32, r: u32| base + r * (TREE_COLS + 1) + c;
        for r in 0..TREE_ROWS {
            for c in 0..TREE_COLS {
                let (a, b, cc, d) = (at(c, r), at(c + 1, r), at(c + 1, r + 1), at(c, r + 1));
                // both sides
                m.indices.extend_from_slice(&[a, b, cc, a, cc, d, a, cc, b, a, d, cc]);
            }
        }
    }
    m.ranges.push((0, m.indices.len() as u32, 0));
    m
}

/// Where textures are looked up for a given content directory.
/// How far a road surface may ride above the ground and still have the ground cut away
/// under it. Anything higher is a bridge or an embankment, where cutting would open a hole.
/// `OMSI_HEIGHTPROFILE_GROUND=1`: the wheels stand on the splines' `[heightprofile]`s as
/// they did before, instead of on the drawn splines as Omsi.exe stands them (A/B runs).
pub(super) fn heightprofile_ground() -> bool {
    // (global: an environment switch read once per process, asked by the staging threads)
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| omsi_cfg::flags::OMSI_HEIGHTPROFILE_GROUND.is_set())
}

pub(super) fn surface_flush() -> f32 {
    // (global: an environment setting read once per process; also asked by `drawn_ground`
    // and the drive probe, which run without a `World`)
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        omsi_cfg::flags::OMSI_SURFACE_FLUSH.parse()
            .unwrap_or(0.12)
    })
}

/// Whether this session's weather lies as snow (`[snow]` in the `.owt`), set by the app
/// when it reads the weather and asked while the vehicles go onto the GPU.
/// (Global: `weather_setup` and `input_script` set it, before and apart from any `World`.)
pub static SNOW_WEATHER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) fn snowing() -> bool {
    SNOW_WEATHER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether a texture name resolves to the season's own copy of it (`texture\WinterSnow\…`,
/// or a folder the season falls back to):
/// a vehicle that brings its own winter picture keeps it.
pub(super) fn seasonal_texture(name: &str, dirs: &[&Path]) -> bool {
    let seasons = omsi_texture::season_folders();
    match omsi_texture::find_texture(name, dirs) {
        Some(p) if !seasons.is_empty() => p.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            seasons.iter().any(|s| c.eq_ignore_ascii_case(s))
        }),
        _ => false,
    }
}

pub fn texture_dirs(root: &Path, content_dir: &Path) -> Vec<PathBuf> {
    // (found whatever its case: `Texture` of a parked car's folder on Linux, whose file
    // system tells `texture` from `Texture`, left the car white)
    let mut dirs = vec![omsi_cfg::resolve_path(content_dir, "texture"), content_dir.to_path_buf()];
    // vehicle folders keep the model in `model\` and the textures in `texture\` next to it
    if let Some(parent) = content_dir.parent() {
        dirs.push(omsi_cfg::resolve_path(parent, "texture"));
    }
    dirs.push(omsi_cfg::resolve_path(root, "Texture"));
    dirs
}

/// The `.surf` map of a texture: a picture named after the texture as the content asks for
/// it, plus `.surf` (`str_kopfgr01.bmp.surf`, also beside a `.dds`), in the texture's folder.
/// Its red channel is the bumpiness of a road drawn with the texture ([`HeightMap`]), which
/// OMSI 2 lays under the wheels (#886). Loaded once per file.
///
/// [`HeightMap`]: omsi_geometry::HeightMap
pub fn surf_map(texture: &str, dirs: &[&Path]) -> Option<Arc<omsi_geometry::HeightMap>> {
    // (global, not a field of `World`: a memo of what the file holds, the same whatever map
    // is open, so a new `World` reads nothing twice; the tests ask without a `World`)
    static MEMO: std::sync::OnceLock<Mutex<HashMap<PathBuf, Option<Arc<omsi_geometry::HeightMap>>>>> = std::sync::OnceLock::new();
    // OMSI_NO_SURF: every road as smooth as before (A/B)
    if omsi_cfg::flags::OMSI_NO_SURF.is_set() {
        return None;
    }
    let found = omsi_texture::find_texture(texture, dirs)?;
    let dir = found.parent()?;
    let req = texture.trim().replace('\\', "/");
    let base = req.rsplit('/').next().unwrap_or(&req).to_string();
    let found_name = found.file_name()?.to_string_lossy().into_owned();
    let path = [base, found_name]
        .iter()
        .map(|n| omsi_cfg::resolve_path(dir, &format!("{n}.surf")))
        // through the VFS: a texture found in a mounted archive has its map in there too
        .find(|p| omsi_cfg::vfs::is_file(p))?;
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = memo.lock().get(&path) {
        return m.clone();
    }
    let map = match omsi_texture::decode_file(&path) {
        Ok(img) => omsi_geometry::HeightMap::from_rgba(img.width as usize, img.height as usize, &img.rgba).map(Arc::new),
        Err(e) => {
            log::warn!("{e}");
            None
        }
    };
    memo.lock().insert(path, map.clone());
    map
}

/// OMSI's `[surface]` id of a texture (0 asphalt, 1 concrete, 2 cobblestone, 3 dirt,
/// 4 grass, 5 gravel, 6 snow, 7 deep snow): what its `.cfg` says, as Omsi.exe hands it to
/// the scripts (`Axle_SurfaceID_`), and the ambience's tyres roll on. Few textures have
/// one - the stock ones of the roads and crossings mostly do, a map's ground textures and
/// most add-ons' not - so a texture without is told by its name (`gras`, `schotter`,
/// `kopfstein` …); a name that says nothing is asphalt, OMSI's own default. Memoised.
pub fn surface_id(texture: &str, dirs: &[&Path]) -> u8 {
    static MEMO: std::sync::OnceLock<Mutex<HashMap<PathBuf, u8>>> = std::sync::OnceLock::new();
    let Some(found) = omsi_texture::find_texture(texture, dirs) else {
        return surface_by_name(texture);
    };
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(id) = memo.lock().get(&found) {
        return *id;
    }
    let cfg = omsi_texture::cfg_path(texture, &found).map(|c| omsi_texture::TextureCfg::load(&c));
    let id = match cfg {
        Some(c) if c.surface_given => c.surface.clamp(0, 255) as u8,
        _ => surface_by_name(texture),
    };
    memo.lock().insert(found, id);
    id
}

/// The `[surface]` a texture's file name suggests (German and English names of the stock
/// and the common add-on textures).
pub fn surface_by_name(texture: &str) -> u8 {
    let n = texture.replace('\\', "/").rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| n.contains(k));
    if has(&["schnee", "snow"]) {
        6
    } else if has(&["kopfstein", "kopfgr", "cobble", "pflaster", "sett", "waschpfl"]) {
        2
    } else if has(&["schotter", "kies", "gravel", "splitt", "ballast", "gleisbett"]) {
        5
    } else if has(&["gras", "rasen", "wiese", "meadow", "lawn", "gruen"]) {
        4
    } else if has(&["sand", "erde", "dirt", "mud", "matsch", "feld", "acker", "field", "soil", "lehm", "waldweg", "forest"]) {
        3
    } else if has(&["beton", "concrete", "platte", "gehweg", "verbund", "pavement", "sidewalk", "wabenst", "gw_"]) {
        1
    } else {
        0
    }
}
