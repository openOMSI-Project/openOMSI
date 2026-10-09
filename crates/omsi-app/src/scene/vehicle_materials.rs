//! Vehicle material and texture sync, and the D3D material mapping.
use super::*;

/// Give the `[smoothskin]` meshes of a vehicle instance copies of their own: their vertices
/// are rewritten as the joint turns, which a mesh shared between every instance of the type
/// (the AI pool, and the player's own set before this ran) cannot be. Called for the
/// player's vehicle and for every AI copy alike (`render.set` says which).
pub(super) fn own_skinned_meshes(
    renderer: &Renderer,
    scene: &mut Scene,
    vt: &omsi_sim::VehicleType,
    render: &mut VehicleRender,
) {
    for (i, vm) in vt.meshes.iter().enumerate() {
        if vm.skin.is_empty() || vm.data.positions.is_empty() {
            continue;
        }
        let Some(&inst) = render.instances.get(i) else {
            continue;
        };
        // (a mesh OMSI_ONLY_MESH / OMSI_HIDE_MESH left out stays out)
        if scene
            .meshes
            .get(scene.instances[inst].mesh)
            .map(|m| m.ranges.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        let id = renderer.add_mesh(scene, &vm.data);
        renderer.set_instance_mesh(scene, inst, id);
        render.skinned.push((i, id, Vec::new()));
    }
    if !render.skinned.is_empty() {
        let names = render
            .skinned
            .iter()
            .map(|(i, _, _)| vt.model.meshes[vt.meshes[*i].def_index].file.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let name = vt
            .def
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        // the player's own vehicle says so once; an AI copy (the timetable's articulated
        // buses spawn dozens of these) only on demand
        match render.set {
            None => log::info!("{name}: {} skinned meshes ({names})", render.skinned.len()),
            Some(_) => log::debug!(
                "{name} (AI): {} skinned meshes ({names})",
                render.skinned.len()
            ),
        }
    }
}

/// Reshape the skinned meshes of a vehicle and its coupled parts whose bones moved (the
/// player's own, or an AI copy's - see `own_skinned_meshes`).
pub fn sync_skinned(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &mut omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
    parts: &mut [VehicleRender],
) {
    for (i, id, last) in render.skinned.iter_mut() {
        let key = vehicle.skin_key(*i);
        if key == *last {
            continue;
        }
        if let Some((pos, nrm)) = vehicle.skinned(*i) {
            renderer.update_mesh(scene, *id, &pos, &nrm, &vehicle.ty.meshes[*i].data.uvs);
        }
        *last = key;
    }
    let n_vars = vehicle.state.vars.len();
    for (t, r) in vehicle.trailers.iter_mut().zip(parts.iter_mut()) {
        for (i, id, last) in r.skinned.iter_mut() {
            let key = t.skin_key(*i);
            if key == *last {
                continue;
            }
            if let Some((pos, nrm)) = t.skinned(*i, n_vars) {
                renderer.update_mesh(scene, *id, &pos, &nrm, &t.ty.meshes[*i].data.uvs);
            }
            *last = key;
        }
    }
}

/// Upload the text and script textures of a vehicle that changed since the last frame.
/// Switch the material of every slot a variable controls: `[matl_freetex]` loads the file a
/// string variable names, `[texchanges]` picks an entry of its master, `[matl_change]` picks
/// between the plain material and the `[matl_item]` variant.
pub fn sync_vehicle_materials(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
) {
    sync_materials(renderer, scene, vehicle, render);
}

/// A coupled part's switched materials and text textures, driven by the variables of the
/// vehicle it is coupled to (its scripts are shared with it).
pub fn sync_vehicle_part(
    renderer: &Renderer,
    scene: &mut Scene,
    main: &omsi_sim::VehicleInstance,
    part: &mut omsi_sim::vehicle::TrailerPart,
    render: &mut VehicleRender,
) {
    sync_interior_lamps(
        renderer,
        scene,
        &part.ty,
        part.position,
        part.body_rotation(),
        |n| main.var(n),
        render,
    );
    sync_materials(renderer, scene, main, render);
    for i in part.update_text_textures(main) {
        if let (Some(Some(tex)), Some(img)) = (
            render.text_textures.get(i),
            part.text_textures[i].pending.take(),
        ) {
            let d = &part.text_textures[i].def;
            renderer.update_texture_mips(
                scene,
                *tex,
                &Image {
                    width: d.width.max(1) as u32,
                    height: d.height.max(1) as u32,
                    rgba: img,
                    has_alpha: true,
                },
            );
        }
    }
}

/// Resolve a vehicle `[matl_freetex]` name. OMSI add-ons often write paths such as
/// `..\\Texture\\mb_pmon\\warning.bmp`: if the normal lookup misses, retry the part
/// below the `Texture` component against the vehicle's texture search directories.
pub(super) fn find_vehicle_freetex(name: &str, dirs: &[&Path]) -> Option<PathBuf> {
    if let Some(path) = omsi_texture::find_texture(name, dirs) {
        return Some(path);
    }
    let normalized = name.trim().replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    let texture = parts.iter().position(|p| p.eq_ignore_ascii_case("Texture"))?;
    let rel = parts.get(texture + 1..)?.join("/");
    if rel.is_empty() {
        return None;
    }
    omsi_texture::find_texture(&rel, dirs)
}

pub(super) fn sync_materials(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &omsi_sim::VehicleInstance,
    render: &mut VehicleRender,
) {
    for v in &mut render.variants {
        let item_has_freetex = v.free.iter().any(|f| f.item_only);
        // the light maps switched on now (several `[matl_lightmap]`s, see `MultiLight`)
        let mask = v.lights.as_ref().map(|l| {
            let mut mask = 0u32;
            for (k, (_, var)) in l.maps.iter().enumerate() {
                let x = var.trim().parse::<f32>().ok().or_else(|| vehicle.var(var)).unwrap_or(0.0);
                // (on at 0.5, as each map's texture stage is, 0x7fe51f: a variable a script
                // dims through 0.1 lit the map at full)
                if x >= 0.5 {
                    mask |= 1 << k;
                }
            }
            mask
        });
        // A free texture under several light maps (an on-board unit's display dimmed by
        // two of them, #1650) is made with the maps switched on: the light maps' own
        // materials knew nothing of the free texture and put the slot's default texture
        // over the display, and a new picture came with the last map whatever its variable.
        let lights = &mut v.lights;
        for f in &mut v.free {
            let name = vehicle.str_var(&f.var);
            let name = name.trim().to_string();
            let key = match mask {
                Some(m) => format!("{}#{m}", name.to_ascii_lowercase()),
                None => name.to_ascii_lowercase(),
            };
            if f.current.as_deref() != Some(key.as_str()) {
                f.current = Some(key.clone());
                let pair = match f.cache.get(&key) {
                    Some(p) => *p,
                    None => {
                        let dirs: Vec<&Path> = f.dirs.iter().map(|p| p.as_path()).collect();
                        let found = if name.is_empty() {
                            None
                        } else {
                            let resolved = find_vehicle_freetex(&name, &dirs);
                            if resolved.is_none() {
                                log::warn!(
                                    "vehicle [matl_freetex] '{}' = {:?}: texture not found",
                                    f.var,
                                    name
                                );
                            }
                            resolved.and_then(|path| {
                                let mut shared = f.shared.lock();
                                if let Some(e) = shared.get_mut(&path) {
                                    e.1 += 1;
                                    f.held.push(path);
                                    return Some(e.0);
                                }
                                let (img, worth) = f.textures.get_gpu_fast(&path)?;
                                let id = renderer.add_texture_data(scene, &img);
                                if worth {
                                    f.wants_upgrade.lock().push(path.clone());
                                }
                                attach_pbr(renderer, scene, &path, id);
                                shared.insert(path.clone(), (id, 1));
                                f.held.push(path);
                                Some(id)
                            })
                        };
                        // An empty string or a file not found leaves the slot its own
                        // texture from the mesh (with its addressing): a roller blind's idle
                        // "next" band then stays out of sight in its transparent border
                        // instead of covering the display as an untextured white plane.
                        let mut spec = match found {
                            Some(tex) => v.spec.with_freetex(f.key, tex, f.diffuse, f.item_only),
                            None => v.spec.clone(),
                        };
                        if let (Some(m), Some(l)) = (mask, lights.as_mut()) {
                            let tex = if m == 0 { None } else { l.composite(renderer, scene, m) };
                            spec.set_lightmap(tex);
                        }
                        let p = spec.build(renderer, scene, v.base_tex);
                        f.cache.insert(key, p);
                        p
                    }
                };
                if !f.item_only {
                    v.base = pair.0;
                }
                if f.item_only || !item_has_freetex {
                    v.item = pair.1;
                }
            }
        }
        if let (Some(l), Some(mask)) = (&mut v.lights, mask) {
            // (with a free texture, the free texture's materials above carry the maps)
            if mask != l.current && v.free.is_empty() {
                l.current = mask;
                let pair = if let Some(p) = l.cache.get(&mask) {
                    *p
                } else {
                    // none on: no light map at all (the set's own materials have the last
                    // map, lit whatever its variable said)
                    let tex = if mask == 0 { None } else { l.composite(renderer, scene, mask) };
                    let mut spec = v.spec.clone();
                    spec.set_lightmap(tex);
                    let p = spec.build(renderer, scene, v.base_tex);
                    l.cache.insert(mask, p);
                    p
                };
                v.base = pair.0;
                v.item = pair.1;
            }
        }
        if let Some(inst) = render.instances.get(v.mesh) {
            let m = v.material(|n| vehicle.var(n));
            if omsi_cfg::flags::OMSI_DEBUG_VARIANTS.var().is_some_and(|f| !f.is_empty() && v.var.to_ascii_lowercase().contains(&f.to_ascii_lowercase())) {
                log::info!("variant mesh {} slot {} var {} = {:?}: material {m} (base {}, item {})", v.mesh, v.slot, v.var, vehicle.var(&v.var), v.base, v.item);
            }
            renderer.set_material(scene, *inst, v.slot, m);
        }
    }
}

/// A vehicle's `[interiorlight]`s (`variable range r g b x y z`) as lamps for this frame:
/// points of light at their place in the vehicle, as strong as their variable (0..1) times
/// `range` (1 for a saloon lamp, 0.4 for a door lamp, 2 for the LiAZ's saloon rows - a door
/// lamp 0.4 m across could not reach the step 2 m below it, so it is no distance), each
/// lighting only the meshes that list it in
/// their `[illumination_interior]` - OMSI's four per mesh, or as many as a model lists
/// (up to `omsi_render::MAX_LAMPS_PER_MESH`), as OMSI switches those lights on
/// for just that mesh. Every set of lamps some mesh names gets a run of slots of its own
/// (the LiAZ 5292 has 32 lamps; only the first eight were drawn, and its saloon stayed dark).
/// The seats' sets (`PassPos::illumination`, the lamps that light a person sitting there)
/// get theirs too: see [`VehicleRender::seat_lamps`].
pub(super) fn sync_interior_lamps(
    renderer: &Renderer,
    scene: &mut Scene,
    ty: &omsi_sim::VehicleType,
    position: DVec3,
    rotation: Mat4,
    var: impl Fn(&str) -> Option<f32>,
    render: &VehicleRender,
) {
    let n = ty.model.interior_lights.len();
    if n == 0 || render.instances.is_empty() {
        return;
    }
    let blocks = render.interior_blocks.get_or_init(|| {
        let mut blocks: Vec<(u32, Vec<usize>)> = Vec::new();
        let mut sets: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, _) in render.instances.iter().enumerate() {
            let Some(vm) = ty.meshes.get(i) else { continue };
            let set = lamp_set(&ty.model.meshes[vm.def_index].illumination_interior, n);
            if !set.is_empty() {
                sets.push((i, set));
            }
        }
        let mut distinct: Vec<Vec<usize>> = Vec::new();
        for (_, set) in &sets {
            if !distinct.contains(set) {
                distinct.push(set.clone());
            }
        }
        if let Some(cabin) = crate::driver::cabin_of(&ty.def) {
            for seat in cabin.driver_positions.iter().chain(&cabin.pass_positions) {
                let set = lamp_set(&seat.illumination, n);
                if !set.is_empty() && !distinct.contains(&set) {
                    distinct.push(set);
                }
            }
        }
        let total: u32 = distinct.iter().map(|d| d.len() as u32).sum();
        if omsi_cfg::flags::OMSI_DEBUG_INTERIOR.is_set() {
            log::info!("interior lamps of {}: {} lamps, {} instances, {} meshes lit by sets {:?}", ty.def.path.display(), n, render.instances.len(), sets.len(), distinct);
        }
        if total == 0 {
            return blocks;
        }
        let first = renderer.alloc_interior_lights(scene, total);
        render.interior_lamps.set(Some((first, total)));
        let mut at = first;
        for d in distinct {
            blocks.push((at, d.clone()));
            at += d.len() as u32;
        }
        for (i, set) in &sets {
            if let Some(b) = blocks.iter().find(|b| &b.1 == set) {
                renderer.set_interior_lamps(scene, render.instances[*i], b.0, set.len() as u32);
            }
        }
        blocks
    });
    for (first, set) in blocks {
        for (k, &li) in set.iter().enumerate() {
            let il = &ty.model.interior_lights[li];
            // on or off: OMSI enables the lamp when its variable is 0.5 or more
            //, there is no dimming
            let on = il
                .variable
                .trim()
                .parse::<f32>()
                .ok()
                .or_else(|| var(&il.variable))
                .unwrap_or(0.0)
                >= 0.5;
            let at = rotation.transform_vector3(glam::Vec3::from(il.pos));
            renderer.set_interior_light(
                scene,
                first + k as u32,
                omsi_render::PointLight {
                    position: position + at.as_dvec3(),
                    // OMSI's Direct3D light: a point light of the
                    // colour / 255, Range 100 m, attenuation 1 / (d² / range²) - full
                    // light at `range` metres, stronger closer in, a quarter at twice
                    radius: 100.0,
                    core: il.range.max(0.01),
                    color: [il.color[0] / 255.0, il.color[1] / 255.0, il.color[2] / 255.0],
                    intensity: if on { 1.0 } else { 0.0 },
                    ..Default::default()
                },
            );
        }
    }
}

/// The lamps (indices into the model's `[interiorlight]`s) a mesh's or a seat's
/// `[illumination_interior]` names: those that exist, each once, as many as a mesh may have.
pub(super) fn lamp_set(indices: &[i32], n: usize) -> Vec<usize> {
    let mut set: Vec<usize> = Vec::new();
    for &k in indices {
        if k >= 0 && (k as usize) < n && !set.contains(&(k as usize)) && set.len() < omsi_render::MAX_LAMPS_PER_MESH as usize {
            set.push(k as usize);
        }
    }
    set
}

impl VehicleRender {
    /// The lamp slots (first, count) for `set_interior_lamps` that light a person on a seat
    /// with these four lamps (`PassPos::illumination`) in a vehicle of `n` lamps; None
    /// before the vehicle's lamps are first synced, or when none of them exists.
    pub fn seat_lamps(&self, n: usize, lamps: &[i32; 4]) -> Option<(u32, u32)> {
        let set = lamp_set(lamps, n);
        let blocks = self.interior_blocks.get()?;
        blocks.iter().find(|b| b.1 == set).map(|b| (b.0, set.len() as u32))
    }
}

pub fn sync_vehicle_textures(
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &mut omsi_sim::VehicleInstance,
    render: &VehicleRender,
    budget: &mut usize,
) {
    vehicle.update_html_textures();
    sync_interior_lamps(
        renderer,
        scene,
        &vehicle.ty,
        vehicle.position,
        vehicle.body_rotation(),
        |n| vehicle.var(n),
        render,
    );
    for i in vehicle.update_text_textures() {
        if let (Some(Some(tex)), Some(img)) = (
            render.text_textures.get(i),
            vehicle.text_textures[i].pending.take(),
        ) {
            let d = &vehicle.text_textures[i].def;
            renderer.update_texture_mips(
                scene,
                *tex,
                &Image {
                    width: d.width.max(1) as u32,
                    height: d.height.max(1) as u32,
                    rgba: img,
                    has_alpha: true,
                },
            );
        }
    }
    let mut rebound = Vec::new();
    for (i, st) in vehicle.host.script_textures.iter_mut().enumerate() {
        // (far away what the scripts redraw goes up every half second: `displays_far`)
        if !render.displays_far {
            if let Some(Some(tex)) = render.script_textures.get(i) {
                if *budget == 0 {
                    continue;
                }
                let Some(rgba) = st.take_upload() else { continue };
                *budget = budget.saturating_sub(rgba.len());
                let img = Image {
                    width: st.width,
                    height: st.height,
                    rgba,
                    has_alpha: true,
                };
                if st.mipmaps {
                    if renderer.update_texture_mips(scene, *tex, &img) {
                        rebound.push(*tex);
                    }
                } else {
                    renderer.update_texture(scene, *tex, &img);
                }
            }
        }
    }
    renderer.rebind_textures(scene, &rebound);
}

/// Distance (m) up to which the `[htmltexture]` pages of scenery objects are kept running.
pub const HTML_OBJECT_NEAR: f64 = 60.0;

/// Distance (m) beyond which what a vehicle's scripts redraw is uploaded only every half
/// second (the picture itself stays: see `Traffic::sync`).
pub const DISPLAYS_FAR: f64 = 50.0;

/// A texture name that stands for "no texture": exporters write `null.bmp` into slots
/// that have none (the SD202's IBIS key click spots). The slot shows its material colour;
/// nothing is looked up, and nothing is reported missing.
pub(crate) fn is_null_texture(name: &str) -> bool {
    let n = name.trim();
    n.is_empty()
        || Path::new(&n.replace('\\', "/"))
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("null"))
}

pub(super) fn scenery_texture_key(name: &str) -> String {
    name.trim().replace('\\', "/").to_ascii_lowercase()
}

pub(super) fn scenery_texture_selection(
    ot: &ObjectType,
    inst: &omsi_sim::scenery::SceneryInstance,
) -> Vec<usize> {
    ot.dynamic_textures
        .iter()
        .map(|group| {
            let Some(value) = inst.var(&group.variable) else {
                return usize::MAX;
            };
            if !value.is_finite() || value < 0.0 {
                return usize::MAX;
            }
            let index = value.trunc() as usize;
            if index < group.choices.len() {
                index
            } else {
                usize::MAX
            }
        })
        .collect()
}

/// The Direct3D material of a slot as OMSI sets it: a `[matl_allcolor]` (diffuse rgba,
/// ambient rgb, specular rgb, emissive rgb, power) replaces the o3d file's material. Returns (diffuse colour, emissive colour, specular colour and power).
/// The diffuse colour modulates the texture; its alpha only counts where there is no
/// texture (with one, the texture's alpha is used alone, as D3D's default stage does).
/// The emissive colour lights the texture by itself (the NL202's interior display, the
/// lamps of a traffic light); the specular term is the sun's highlight.
pub(super) fn d3d_material(
    m: &omsi_o3d::Material,
    allcolor: Option<[f32; 14]>,
    textured: bool,
) -> ([f32; 4], [f32; 3], [f32; 4], [f32; 3]) {
    // (the ambient colour: Omsi.exe gives every o3d slot a white one, 0x7c62f8, and a
    // textured .x slot too, 0x7c6d2d; a [matl_allcolor] sets its own)
    let (diffuse, emissive, specular, power, ambient) = match allcolor {
        Some(v) => (
            [v[0], v[1], v[2], v[3]],
            [v[10], v[11], v[12]],
            [v[7], v[8], v[9]],
            v[13],
            [v[4], v[5], v[6]],
        ),
        None => (m.diffuse, m.emissive, m.specular, m.specular_power, [1.0; 3]),
    };
    let clamp01 = |x: f32| {
        if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let color = [
        clamp01(diffuse[0]),
        clamp01(diffuse[1]),
        clamp01(diffuse[2]),
        if textured { 1.0 } else { clamp01(diffuse[3]) },
    ];
    let emissive = emissive.map(clamp01);
    let specular = specular.map(clamp01);
    // D3D ignores the specular colour without a power to raise the highlight to
    let power = if power.is_finite() && power >= 1.0 && specular.iter().any(|c| *c > 0.004) {
        power.min(256.0)
    } else {
        0.0
    };
    (
        color,
        emissive,
        [specular[0], specular[1], specular[2], power],
        ambient.map(clamp01),
    )
}

/// The material manager's depth and reflection settings of a slot's `[matl]` commands
/// (`bump`: the loaded `[matl_bumpmap]` height map and its factor).
pub(super) fn material_extra(
    ov: &[&MaterialDef],
    env_mask: Option<TextureId>,
    bump: Option<(TextureId, f32)>,
    specular: [f32; 4],
) -> MaterialExtra {
    MaterialExtra {
        env_mask,
        no_z_write: ov.iter().any(|o| o.no_z_write),
        writes_depth: false,
        // `[matl_noZcheck]` leaves Omsi.exe's depth test on: its draw of the slot (0x7fd6c4)
        // never reads the flag, which only adds a colourless stencil pass marking the panes
        // for the raindrops (0x7c32c4 -> 0x7fc58c, ZENABLE 1, blend ZERO/ONE). Taken as "no
        // depth test", the Sprinter's inner window glass (flagged so) was drawn over the
        // body skin round every opening. OMSI_NOZCHECK_BIAS=1: the old reading.
        no_z_check: ov.iter().any(|o| o.no_z_check) && omsi_cfg::flags::OMSI_NOZCHECK_BIAS.is_set(),
        z_bias: ov.iter().map(|o| o.z_bias).find(|b| *b != 0).unwrap_or(0),
        ambient: None,
        specular,
        bump: bump.filter(|b| b.1.is_finite() && b.1 != 0.0),
        glass: false,
        night_switched: false,
        rain_film: false,
        water: false,
        display: false,
        screen: false,
        led: false,
        led_sign: false,
        no_map_lights: false,
        tree: false,
        sway: None,
        moisture: 0.0,
        transmap_declared: ov.iter().any(|o| o.transmap.is_some()),
        // (the last addressing command of the slot decides; the colour is given in bytes)
        border: ov
            .iter()
            .rev()
            .find(|o| o.tex_address != omsi_model::TexAddress::Wrap)
            .filter(|o| o.tex_address == omsi_model::TexAddress::Border)
            .map(|o| o.border_color.map(|c| (c / 255.0).clamp(0.0, 1.0))),
        metal_ok: false,
        glow: glow_strength(ov),
    }
}

/// `[matl_glow] <texture> <value>` (an openOMSI extension, see `MaterialExtra::glow`): the
/// mask a slot's commands bind in the light map's slot. A `[matl_lightmap]` of the slot keeps
/// that slot, and the glow is left out; the last `[matl_glow]` of the slot counts, as the
/// other `[matl_*]` commands do.
pub(super) fn glow_mask<'a>(ov: &[&'a MaterialDef]) -> Option<&'a str> {
    if ov.iter().any(|o| o.lightmap.is_some()) {
        return None;
    }
    ov.iter().rev().find_map(|o| o.glow.as_ref()).map(|g| g.0.as_str())
}

/// The strength of `glow_mask`'s glow in the shader's terms: the .cfg value x0.25, the
/// `Led glow` setting's own levels (6 its default); 0 without one, and for a negative value
/// (a light that would be taken away).
fn glow_strength(ov: &[&MaterialDef]) -> f32 {
    if glow_mask(ov).is_none() {
        return 0.0;
    }
    ov.iter().rev().find_map(|o| o.glow.as_ref()).map_or(0.0, |g| (g.1 * 0.25).max(0.0))
}

/// Whether a `[matl_item]`'s commands give it a light map's slot of its own (a light map or
/// a glow); without one it keeps its base material's.
pub(super) fn own_light_slot(ov: &[&MaterialDef]) -> bool {
    ov.iter().any(|o| o.lightmap.is_some() || o.glow.is_some())
}

/// The addressing of a slot's textures: its last `[matl_texadress_*]` command decides (the
/// border mode is clamped, its colour comes with `material_extra`).
pub(super) fn tex_addressing<'a>(ov: impl DoubleEndedIterator<Item = &'a MaterialDef>) -> omsi_render::TexAddressing {
    use omsi_model::TexAddress as A;
    use omsi_render::TexAddressing as R;
    match ov.rev().map(|o| o.tex_address).find(|a| *a != A::Wrap) {
        None | Some(A::Wrap) => R::Wrap,
        Some(A::Mirror) => R::Mirror,
        Some(A::Clamp | A::Border) => R::Clamp,
        Some(A::MirrorOnce) => R::MirrorOnce,
    }
}

/// The key a `[matl_bumpmap]` height map of `path` is kept under (the same file may be a
/// colour texture as well).
pub(super) fn bump_key(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}#bump", path.display()))
}

/// Whether a texture file is a season's snow picture: it lies in a `WinterSnow` folder
/// (`Texture\WinterSnow\gras.bmp`, any case), where the snow weather finds the map's
/// snowy textures, or in a `WinterSnowfall` one, its snowy roads.
pub(super) fn is_snow_picture(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().to_str().is_some_and(|s| s.eq_ignore_ascii_case("WinterSnow") || s.eq_ignore_ascii_case("WinterSnowfall")))
}

/// A PBR set beside the diffuse texture `path` (`foo_n.png` and the rest, see
/// `omsi_texture::pbr`), put up and tied to texture `id` for the materials made with it.
pub(crate) fn attach_pbr(renderer: &Renderer, scene: &mut Scene, path: &Path, id: TextureId) {
    // (and a season's snow picture is known as one: it gets no snow laid over it, #879)
    if is_snow_picture(path) {
        scene.snow_textures.insert(id);
    }
    if omsi_cfg::flags::OMSI_NO_PBR.is_set() {
        return;
    }
    let files = omsi_texture::pbr::find(path);
    if files.is_empty() {
        return;
    }
    if let Some(set) = omsi_texture::pbr::load_set(&files) {
        log::info!("PBR maps for {}: normal {:?}, occlusion/roughness/metal {:?}", path.display(), set.normal.as_ref().map(|i| (i.width, i.height)), set.flags);
        renderer.add_pbr_maps(scene, id, &set);
    }
}

/// The texture data for a key of the texture maps: a file, or a file's bump height map.
pub(super) fn load_texture_key(key: &Path, compress: bool) -> Option<TextureData> {
    let k = key.to_string_lossy();
    match k.strip_suffix("#bump") {
        Some(file) => omsi_texture::decode_file(Path::new(file))
            .map_err(|e| log::warn!("{e}"))
            .ok()
            .map(|img| omsi_texture::gpu::prepare_bump(&img, compress)),
        None => {
            if compress {
                omsi_texture::gpu::load_gpu(key).ok().map(|t| t.0)
            } else {
                omsi_texture::gpu::load_gpu_fast(key).ok().map(|t| t.0)
            }
        }
    }
}

/// Decide the alpha mode of an o3d material from the model.cfg `[matl]` overrides.
pub(crate) fn material_alpha(
    materials: &[omsi_o3d::Material],
    slot: usize,
    overrides: &[MaterialDef],
) -> AlphaMode {
    // the plain [matl] overrides of this slot decide; without one: opaque. A
    // `[matl_change]` record only opens the variants (`[matl_item]`) and says nothing of the
    // slot's own look: a `[matl]` of the same slot after it does. (The LED matrices of
    // churaPixel/Krüger++ open a change first and give the slot `[matl_alpha] 2` and the
    // script texture as its mask in a `[matl]` after it: taken as opaque from the change,
    // the mask cut nothing and the whole panel was lit.)
    // Several plain [matl] of one slot are one material in OMSI: each selects it again and
    // the commands after it modify it, so the last `[matl_alpha]` among them counts. (Taken
    // from the first block alone, an alpha-tested texture whose `[matl_alpha]` sits in a
    // second [matl] was drawn opaque, its transparent parts as solid areas.) omsi-model
    // already joins blocks spelt the same; this covers those that reach the slot otherwise
    // (an index of -1 selects the first one, as 0 does).
    let mine: Vec<&MaterialDef> = overrides.iter().filter(|o| !o.item && omsi_sim::vehicle::override_slot(materials, o) == Some(slot)).collect();
    let plain = || mine.iter().filter(|o| o.change.is_none());
    plain()
        .rev()
        .find(|o| o.alpha_set)
        .or_else(|| plain().next())
        .or(mine.first())
        .map(|o| alpha_mode(o.alpha))
        .unwrap_or(AlphaMode::Opaque)
}

pub(super) fn alpha_mode(a: i32) -> AlphaMode {
    match a {
        0 => AlphaMode::Opaque,
        1 => AlphaMode::Test,
        _ => AlphaMode::Blend,
    }
}

/// Whether a vehicle's `[useTextTexture]` slot is a display (`MaterialExtra::display`): one
/// that has a light of its own, a light map or a night map (a destination matrix, a
/// counter lit with the dashboard), not lettering on the body.
pub(super) fn text_is_display(lightmap: bool, night: bool) -> bool {
    lightmap || night
}

/// How a `[texttexture]` shows on its slot: alpha tested where the slot's `[matl_alpha]` is 1
/// (the stock route helpers, `routearrows_busstop.sco`: blended, the empty part of the text
/// wrote depth and cut away whatever was drawn behind it later - a bus beside the stop lost
/// half its roof), blended otherwise.
pub(super) fn text_alpha(materials: &[omsi_o3d::Material], slot: usize, overrides: &[MaterialDef]) -> AlphaMode {
    match material_alpha(materials, slot, overrides) {
        AlphaMode::Test => AlphaMode::Test,
        _ => AlphaMode::Blend,
    }
}

/// Placement is part of the picture: otherwise a centred sign can lend its cached
/// texture to a left-aligned one showing the same words.
/// A text texture's material, addressed as its slot is: `[matl_texadress_border]` (and
/// clamp, mirror) on the slot carry over to the text drawn into it. Made with the plain
/// wrap, Road-Hog123's bus stop flags - a second mesh whose UVs run past the text texture
/// into its border - showed the stop name repeated across the whole flag (#1645).
pub(super) fn text_material(renderer: &Renderer, scene: &mut Scene, tex: TextureId, alpha: AlphaMode, slot_ov: &[&MaterialDef]) -> MaterialId {
    let extra = material_extra(slot_ov, None, None, [0.0; 4]);
    let plain = MaterialExtra::default();
    let border_only = MaterialExtra { border: extra.border, ..plain.clone() };
    renderer.address_next.set(tex_addressing(slot_ov.iter().copied()));
    // (lit like the rest of the object: Omsi.exe only swaps the slot's texture, a sign
    // does not shine at night)
    renderer.add_material_extra(scene, Some(tex), alpha, [1.0; 4], false, None, None, None, None, [0.0; 3], border_only)
}

/// The key of a text texture material: the text's own key and how its slot addresses it.
pub(super) fn text_material_key(base: String, slot_ov: &[&MaterialDef]) -> String {
    let border = material_extra(slot_ov, None, None, [0.0; 4]).border;
    format!("{base}|{:?}|{border:?}", tex_addressing(slot_ov.iter().copied()))
}

pub(super) fn scenery_text_key(tt: &omsi_model::TextTexture, text: &str, alpha: AlphaMode) -> String {
    format!(
        "{}|{}|{}x{}|{}|{:?}|{:?}|{}|{}",
        tt.font.to_ascii_lowercase(),
        text,
        tt.width.max(1),
        tt.height.max(1),
        tt.full_color,
        tt.color,
        alpha,
        tt.orientation,
        tt.grid,
    )
}

pub(super) fn scenery_text_image(
    tt: &omsi_model::TextTexture,
    atlas: Option<Arc<omsi_content::font::FontAtlas>>,
    text: &str,
) -> Image {
    // Static and scripted text textures use the placement from their definition.
    let state = omsi_sim::texttex::TextTextureState::new(tt.clone(), atlas);
    Image {
        width: tt.width.max(1) as u32,
        height: tt.height.max(1) as u32,
        rgba: state.image(text),
        has_alpha: true,
    }
}

/// The text of one of the game's own helper objects (the route arrows' street and stop
/// names) that its `.oft` font cannot draw: the stock arrows ask for the font "test"
/// (`Fonts/test1.oft`), which has the Latin letters and German umlauts only, so a street
/// or a stop named in Cyrillic (or Greek, Chinese ...) came out as an empty arrow - at
/// most a stray `Ä` where a code page variant of `Д` happened to be in the font. Such a
/// text is drawn with the interface font (Roboto, then the system's fonts for the
/// scripts it lacks) in the texture's colour, the height of the `.oft` font's letters,
/// centred, and narrowed to the texture's width. None when the font draws every letter:
/// that text keeps OMSI's own look.
pub(super) fn helper_text_image(tt: &omsi_model::TextTexture, atlas: Option<&omsi_content::font::FontAtlas>, text: &str) -> Option<Image> {
    let drawable = |c: char| c == '@' || c.is_whitespace() || atlas.is_some_and(|a| a.font.glyph(c).is_some());
    if text.trim().is_empty() || text.chars().all(drawable) {
        return None;
    }
    // (global: the interface fonts are the same for every map; built once, on first use)
    static FONTS: std::sync::OnceLock<omsi_ui::Fonts> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(omsi_ui::Fonts::new);
    let (w, h) = (tt.width.max(1) as u32, tt.height.max(1) as u32);
    // `@` starts a new line, as in OMSI's own text textures (#1553, #1482): drawn as a
    // letter, a Polish stop's "622@Sosnowiec@Urząd@Miasta" stood in one line with its
    // at signs, and a Chinese one was squeezed into a sliver. (A text starting with `@`
    // keeps its empty first line.)
    let text = text.trim_end();
    let lines: Vec<&str> = text.split('@').map(str::trim).collect();
    let n = lines.len().max(1) as f32;
    // (the .oft's line height holds its capitals and the gap below them; Roboto's capitals
    // are 0.7 of its size, so nearly the line height gives letters of the same height)
    let line = atlas.map(|a| a.font.height.max(8) as f32).unwrap_or(h as f32 * 0.2);
    // the lines share the texture's height when they would not fit at the font's own
    let mut pitch = line.min(h as f32 / n);
    // A line too long for the texture is made smaller first, letters and all, down to two
    // thirds of the height, and only what is still too wide is narrowed: narrowed alone, a
    // long Polish street name came out in thin sticks nobody could read (#1696, #1365).
    let widest = lines.iter().filter(|l| !l.is_empty()).map(|l| fonts.render(l, pitch * 0.95, omsi_ui::Weight::Medium).w).max().unwrap_or(0);
    if widest > w {
        pitch *= (w as f32 / widest as f32).max(2.0 / 3.0);
    }
    let px = pitch * 0.95;
    let top = (h as f32 - pitch * n) * 0.5;
    let rgb = if tt.full_color { [255u8; 3] } else { [tt.color[0] as u8, tt.color[1] as u8, tt.color[2] as u8] };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (k, l) in lines.iter().enumerate() {
        if l.is_empty() {
            continue;
        }
        let bmp = fonts.render(l, px, omsi_ui::Weight::Medium);
        if bmp.w == 0 || bmp.h == 0 {
            continue;
        }
        // too long for the texture: narrowed to fit (columns sampled), the height kept
        let scale = (w as f32 / bmp.w as f32).min(1.0);
        let out_w = ((bmp.w as f32 * scale).floor() as u32).max(1);
        let x0 = (w - out_w.min(w)) / 2;
        let y0 = (top + pitch * k as f32 + (pitch - bmp.h as f32) * 0.5).round() as i32;
        for y in 0..bmp.h as i32 {
            let dy = y0 + y;
            if dy < 0 || dy >= h as i32 {
                continue;
            }
            for x in 0..out_w.min(w) {
                let sx = ((x as f32 + 0.5) / scale) as u32;
                let a = bmp.alpha[(y as u32 * bmp.w + sx.min(bmp.w - 1)) as usize];
                if a == 0 {
                    continue;
                }
                let i = ((dy as u32 * w + x0 + x) * 4) as usize;
                rgba[i..i + 3].copy_from_slice(&rgb);
                rgba[i + 3] = rgba[i + 3].max(a);
            }
        }
    }
    Some(Image { width: w, height: h, rgba, has_alpha: true })
}

/// Identify a solid vehicle body material that should participate in the depth buffer.
/// Some bus packs put either `[matl_alpha] 2` or `[matl_noZcheck]` on a complete body mesh.
/// The decision must not depend on one creator's language or on a particular bus name:
/// use the model metadata and the material's actual mesh volume, while keeping thin glass
/// and explicit overlay/transparency materials on their authored paths.
/// Words that name a pane of glass in a mesh or texture file, in the languages OMSI's
/// add-ons are made in. The body-depth repair must not turn one of these opaque when the
/// model.cfg declares it blended: a Czech bus's `okna.o3d` (windows) on the shared
/// `body.png` was drawn as a black wall, where OMSI shows the tinted glass.
pub(super) const GLASS_WORDS: [&str; 20] = [
    "window", "fenster", "glas", "scheibe", "windshield", "windscreen", // en, de ("glas" is also German: `Leuchtmelderglas.tga`)
    "okn", "sklo", // cs, sk (okna, okno, sklo)
    "szyb", "okien", // pl
    "ablak", // hu
    "steklo", // ru (transliterated)
    "vitre", "fenetre", // fr
    "vetro", "finestr", // it
    "raam", "ruit", // nl
    "ventan", "cristal", // es
];

pub(super) fn is_vehicle_body_material(
    mesh_file: &str,
    texture: &str,
    has_texture: bool,
    has_transmap: bool,
    no_z_write: bool,
    body_hint: bool,
) -> bool {
    if !has_texture || has_transmap || no_z_write || !body_hint {
        return false;
    }
    let name = format!("{} {}", mesh_file, texture).to_ascii_lowercase();
    let overlay = ["regen", "dirt", "dreck", "wiper", "matrix", "display", "shadow"];
    if GLASS_WORDS.iter().chain(overlay.iter()).any(|part| name.contains(part)) {
        return false;
    }
    true
}

/// The texture whose alpha is a blended slot's coverage: its `[matl_transmap]` file where
/// it has one (the diffuse alpha is then the reflection mask), else the diffuse texture. A
/// script texture as the map (`\S:n`) is not known at load time: the diffuse texture then.
pub(super) fn coverage_texture<'a>(transmap: Option<&'a str>, diffuse: &'a str) -> &'a str {
    match transmap.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if !t.starts_with("\\S:") => t,
        _ => diffuse,
    }
}

/// Side of the square a texture's alpha is kept at for [`slot_is_see_through`].
pub(super) const ALPHA_MASK: usize = 256;

/// A texture's alpha channel, thinned out to [`ALPHA_MASK`] squared (a body texture is
/// 4096 squared, and every blended slot of a bus asks). None for a file that cannot be
/// read or has no alpha.
pub(super) fn alpha_mask(path: &Path) -> Option<Arc<Vec<u8>>> {
    // (global, not a field of `World`: a memo of what the file holds, the same whatever map
    // is open, so a new `World` reads nothing twice)
    static MASKS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Vec<u8>>>>>> = std::sync::OnceLock::new();
    let masks = MASKS.get_or_init(Default::default);
    if let Some(m) = masks.lock().get(path) {
        return m.clone();
    }
    let mask = omsi_texture::decode_file(path).ok().filter(|img| img.has_alpha && img.width > 0 && img.height > 0).map(|img| {
        let (w, h) = (img.width as usize, img.height as usize);
        let mut out = vec![255u8; ALPHA_MASK * ALPHA_MASK];
        for y in 0..ALPHA_MASK {
            for x in 0..ALPHA_MASK {
                let (sx, sy) = ((x * w / ALPHA_MASK).min(w - 1), (y * h / ALPHA_MASK).min(h - 1));
                out[y * ALPHA_MASK + x] = img.rgba[(sy * w + sx) * 4 + 3];
            }
        }
        Arc::new(out)
    });
    masks.lock().insert(path.to_path_buf(), mask.clone());
    mask
}

/// Whether the triangles of material `slot` lie on a see-through part of their texture
/// (`mask`, see [`alpha_mask`]): two in three of them with an alpha under 0.9 at their
/// middle. A pane does - the SOR NB12's glass is 62 of 255 on its body texture; a door or
/// a body panel blended by `[matl_alpha] 2` has its paint at 255 and does not.
pub(super) fn slot_is_see_through(mesh: &MeshData, slot: usize, mask: &[u8]) -> bool {
    let (mut clear, mut all) = (0usize, 0usize);
    for &(first, count, material) in &mesh.ranges {
        if material as usize != slot {
            continue;
        }
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        for tri in mesh.indices.get(start..end).unwrap_or_default().chunks_exact(3) {
            let Some(uv) = tri.iter().map(|&i| mesh.uvs.get(i as usize).copied()).sum::<Option<glam::Vec2>>() else { continue };
            let uv = uv / 3.0;
            let at = |t: f32| ((t.rem_euclid(1.0) * ALPHA_MASK as f32) as usize).min(ALPHA_MASK - 1);
            all += 1;
            if mask[at(uv.y) * ALPHA_MASK + at(uv.x)] < 230 {
                clear += 1;
            }
        }
    }
    // (two thirds: a layer over the windows that painters draw on - the glass's own unwrap
    // in the paint scheme, its adverts and tint - is covered where the picture is, and is
    // a pane all the same)
    all > 0 && clear * 3 >= all * 2
}

/// Whether a texture (`mask`, see [`alpha_mask`]) is clear all over: no texel above 8 of
/// 255. A slot blended by `[matl_alpha] 2` with such a texture is drawn for its depth alone
/// - an "invisible cover" bus makers put in front of a roller blind or a window to hide it
/// (a 16x16 clear `.tga`, #1113, #576): Omsi.exe writes its depth in model order, and what
/// the model lists after it behind it is gone.
pub(super) fn texture_is_clear(mask: &[u8]) -> bool {
    !mask.is_empty() && mask.iter().all(|&a| a <= 8)
}

/// Return whether the triangles of one material occupy a volumetric part of the vehicle.
/// Windows and rain films are normally very thin sheets; this lets unnamed/modded body
/// meshes be repaired without maintaining a language-specific list of mesh names.
pub(super) fn material_has_vehicle_volume(mesh: &MeshData, slot: usize) -> bool {
    let mut lo = glam::Vec3::splat(f32::MAX);
    let mut hi = glam::Vec3::splat(f32::MIN);
    let mut points = 0usize;
    for &(first, count, material) in &mesh.ranges {
        if material as usize != slot {
            continue;
        }
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        for &index in mesh.indices.get(start..end).unwrap_or_default() {
            if let Some(p) = mesh.positions.get(index as usize) {
                lo = lo.min(*p);
                hi = hi.max(*p);
                points += 1;
            }
        }
    }
    if points < 8 || !lo.x.is_finite() || !hi.x.is_finite() {
        return false;
    }
    let extent = hi - lo;
    let mut sides = [extent.x.abs(), extent.y.abs(), extent.z.abs()];
    sides.sort_by(f32::total_cmp);
    // A solid shell has meaningful thickness compared with both of its other dimensions.
    // A windshield or side pane may be wide and tall, but remains a sheet in its thin axis.
    sides[2] > 3.0
        && sides[1] > 1.0
        && sides[0] > 0.5
        && sides[0] / sides[1] > 0.25
        && sides[0] / sides[2] > 0.02
}

/// Whether the triangles of material `slot` lie on the faces of another slot of the same
/// mesh: a layer modelled as a copy of the surface under it with a material of its own (a
/// baked ambient-occlusion or shading film over the floor, `[matl_alpha] 2`), which OMSI
/// blends over the surface as declared.
pub(super) fn slot_overlays_another(mesh: &MeshData, slot: usize) -> bool {
    let key = |p: &glam::Vec3| ((p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32);
    let mut own = std::collections::HashSet::new();
    let mut others = std::collections::HashSet::new();
    for &(first, count, material) in &mesh.ranges {
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(mesh.indices.len());
        let set = if material as usize == slot { &mut own } else { &mut others };
        for &index in mesh.indices.get(start..end).unwrap_or_default() {
            if let Some(p) = mesh.positions.get(index as usize) {
                set.insert(key(p));
            }
        }
    }
    own.len() >= 3 && own.iter().filter(|k| others.contains(*k)).count() * 10 >= own.len() * 9
}
