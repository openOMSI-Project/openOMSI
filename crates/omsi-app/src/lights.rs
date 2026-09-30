//! Per-frame light sources: scenery coronas/map lights by night, vehicle lights from
//! their script variables (`[light_enh_2]`, `[spotlight]` + `Spot_Select`, `[interiorlight]`).

use crate::scene::{LightSwitch, World};
use glam::{DVec3, Vec3};
use omsi_render::{Corona, LightMode, Lighting, PointLight, Scene};

use omsi_sim::{Daylight, VehicleInstance};

/// A headlight's strength for the enhanced renderer along its axis, in units of a
/// `[maplight]` core (`PointLight::intensity`): some 1 100 cd. The stock spots point 17°
/// down, so the axis meets the road a few metres ahead of the bumper; a real low beam's
/// 20 000 cd there would be a floodlight. (At 20 the road ahead at night came out barely
/// brighter with the lights on than off - 11 → 19 of 255 on Novi Sad's asphalt - where
/// OMSI's buses throw a clear pool.)
const HEADLIGHT_INTENSITY: f32 = 45.0;

/// A headlight's strength in the classic picture, as a map lamp's (`PointLight::intensity`).
/// Measured against OMSI 2 from above, the stock NL202 at night in Spandau: its pool is
/// some 18 of 255 over the unlit cobbles 1.5 - 2.5 m ahead, 12 at 4 - 5 m and nearly gone
/// at 7 m; this gives 20, 12 and 5 there. (At 1 it was 60, 40 and 15.)
const VANILLA_HEADLIGHT_INTENSITY: f32 = 0.2;

/// Lighting parameters for the renderer from the daylight model.
pub fn lighting_from(d: &Daylight, fog_range: f32) -> Lighting {
    // fog density from the weather's visibility range (an object at `range` is ~90% fogged)
    let density = (2.3 / fog_range.max(50.0)).max(0.00005);
    Lighting {
        sun_dir: d.sun_dir,
        sun_intensity: 1.0,
        sun_color: d.sun_color,
        secondary: d.secondary,
        ambient: d.ambient,
        fog_color: d.sky,
        fog_density: density,
        sky_color: d.sky,
        night: d.night,
        night_maps: Some(if d.lamps_on || d.night >= 0.5 { 1.0 } else { 0.0 }),
        sun_azimuth: d.azimuth_rad,
        sky_weights: d.sky_weights,
        envir_tint: d.envir_tint,
        ..Default::default()
    }
}

/// What the weather does to the light: an overcast sky takes the sun away and turns
/// everything grey and flat, rain darkens it further, fog and snow take their colour
/// from the sky. `cloud_density` 0..1 (1 = closed cover), `precip` 0..1, `snow` 0..1.
pub fn apply_weather(
    l: &mut Lighting,
    cloud_density: f32,
    precip_kind: i32,
    precip: f32,
    snow: f32,
) {
    let o = cloud_density.clamp(0.0, 1.0);
    let overcast = (o - 0.45).max(0.0) / 0.55; // cumulus cover keeps the sun; a closed sky loses it
    let grey = |c: Vec3, k: f32| -> Vec3 {
        let lum = c.dot(Vec3::new(0.3, 0.59, 0.11));
        c.lerp(Vec3::splat(lum), k)
    };
    // the sun: dimmed and whitened under cloud, gone under overcast
    l.sun_intensity *= 1.0 - 0.85 * overcast;
    l.sun_color = grey(l.sun_color, 0.6 * o);
    // the sky light is what is left, greyer and a little darker
    let sky_lum = l.sky_color.dot(Vec3::new(0.3, 0.59, 0.11));
    let cloud_sky = Vec3::splat(sky_lum * 0.82)
        .lerp(Vec3::new(0.62, 0.65, 0.70) * sky_lum.max(0.25) * 1.3, 0.5);
    l.sky_color = l.sky_color.lerp(cloud_sky, overcast * 0.9);
    l.secondary = grey(l.secondary, o * 0.7) * (1.0 - 0.15 * overcast);
    l.ambient = grey(l.ambient, o * 0.7) * (1.0 + 0.25 * overcast);
    l.fog_color = l.fog_color.lerp(l.sky_color, overcast);
    // rain and snow take some light and thicken the air
    // heavy rain by day is dark: the sun is gone, the sky light drops by a third and
    // everything, the bus included, sits in the same grey. The gloom also brings the
    // night factor up a little, so interior lights and nightmaps start to show - by
    // day in a downpour you do want the saloon lights on.
    let rain = if precip_kind != 0 {
        precip.clamp(0.0, 1.0)
    } else {
        0.0
    };
    l.sun_intensity *= 1.0 - 0.75 * rain;
    l.ambient *= 1.0 - 0.28 * rain;
    l.secondary *= 1.0 - 0.32 * rain;
    l.sky_color *= 1.0 - 0.22 * rain;
    l.fog_color *= 1.0 - 0.15 * rain;
    if rain > 0.0 {
        l.fog_density = l.fog_density.max(2.3 / (2500.0 - 1800.0 * rain));
    }
    // the enhanced renderer builds its own sky from these
    l.overcast = overcast;
    l.rain = rain;
    let gloom = (overcast * 0.5 + rain * 0.5).clamp(0.0, 1.0);
    l.night = l.night.max(0.45 * gloom);
    // (night maps on with the street lamps, or where the weather makes it that dark: a
    // clear dusk showed the lit windows and signs at a fraction, a rainy one at nearly
    // full, #276)
    if l.night >= 0.5 {
        l.night_maps = Some(1.0);
    }
    if snow > 0.0 {
        // snow on the ground throws light back up
        l.ambient *= 1.0 + 0.35 * snow;
        l.secondary *= 1.0 + 0.2 * snow;
        // (the snowy grey only as bright as the daylight: a fixed 0.86 made a winter night
        // in fog a pale grey dusk in the classic picture and in every mirror)
        let day = 1.0 - 0.93 * l.night.clamp(0.0, 1.0);
        l.fog_color = l.fog_color.lerp(Vec3::new(0.86, 0.88, 0.92) * day, 0.4 * snow);
    }
    l.snow = snow;
}

/// Coronas and point lights of one vehicle in its current state.
pub fn vehicle_lights(
    v: &VehicleInstance,
    coronas: &mut Vec<Corona>,
    lights: &mut Vec<PointLight>,
    night: f32,
) {
    let ty = &v.ty;
    // def index → loaded mesh index (animation transform)
    let value_of = |name: &str| -> f32 {
        let t = name.trim();
        if let Ok(x) = t.parse::<f32>() {
            return x;
        }
        v.var(t).unwrap_or(0.0)
    };
    let mesh_xf = |def_index: usize| -> glam::Mat4 {
        match ty.meshes.iter().position(|m| m.def_index == def_index) {
            Some(i) => v.mesh_local_transform(i),
            None => v.body_rotation(),
        }
    };
    coronas.extend(crate::scene::model_lights_faded(&ty.model, &mesh_xf, v.position, &value_of, &v.light_fade));
    // An articulated vehicle is one visual bus, but its rear section has its own
    // `[light_enh_2]`/corona declarations and animated meshes.  The old collector only
    // visited the leading section, which made rear lamps, destination lights and section-
    // local headlights appear dead even though the coupled part was rendered correctly.
    for t in &v.trailers {
        let part_mesh_xf = |def_index: usize| -> glam::Mat4 {
            match t.ty.meshes.iter().position(|m| m.def_index == def_index) {
                Some(i) => t.mesh_local_transform(i),
                None => t.body_rotation(),
            }
        };
        coronas.extend(crate::scene::model_lights_faded(
            &t.ty.model,
            &part_mesh_xf,
            t.position,
            &value_of,
            &t.light_fade,
        ));
    }
    let body = v.body_rotation();
    // headlights: the spotlight selected by Spot_Select
    // (OMSI_SPOT_SELECT=n: that spotlight on, for checking the headlights in a picture)
    let forced = omsi_cfg::env::var("OMSI_SPOT_SELECT").ok().and_then(|s| s.trim().parse::<f32>().ok());
    if let Some(sel) = forced.or_else(|| v.var("Spot_Select")) {
        if sel >= 0.0 {
            if let Some(sp) = ty.model.spotlights.get(sel as usize) {
                let vals = sp.values;
                let d = body
                    .transform_vector3(Vec3::new(vals[3], vals[4], vals[5]))
                    .normalize_or_zero();
                let color = [vals[6] / 255.0, vals[7] / 255.0, vals[8] / 255.0];
                // D3D's spot is only a direction: the stock NL202 puts it 3.8 m behind its
                // nose, where the real one lit the dashboard and the windscreen from inside.
                // It shines from the vehicle's front (or rear) face at its own height. The face
                // is where the model's lamps ([light_enh], [light_enh_2]) reach furthest: the
                // AA-FR Agora's spot sits 4 m behind its headlamps, and moved at most 1.5 m it
                // still shone from inside the bus onto its own front. (Within the bounding box;
                // without lamps, the box alone, by at most 1.5 m: a box grown by mirrors, a
                // coupling or a mod's odd bounds threw the light well ahead of the bus.)
                let mut apex = Vec3::new(vals[0], vals[1], vals[2]);
                let dl = Vec3::new(vals[3], vals[4], vals[5]).normalize_or_zero();
                let lamps: Vec<[f32; 3]> = ty
                    .model
                    .meshes
                    .iter()
                    .flat_map(|m| m.light_enh.iter().map(|l| l.pos).chain(m.light_enh_2.iter().map(|l| l.pos)))
                    .collect();
                let nose = lamps.iter().map(|l| l[1]).reduce(f32::max);
                let tail = lamps.iter().map(|l| l[1]).reduce(f32::min);
                let bb = ty.def.bounding_box.map(|bb| (bb[4] + bb[1] * 0.5, bb[4] - bb[1] * 0.5));
                if dl.y > 0.3 {
                    let face = match (nose.filter(|n| *n > apex.y), bb) {
                        (Some(n), Some((front, _))) => Some(n.min(front)),
                        (Some(n), None) => Some(n),
                        (None, Some((front, _))) => Some(front.min(apex.y + 1.5)),
                        (None, None) => None,
                    };
                    if let Some(face) = face.filter(|f| *f > apex.y) {
                        apex.y = face + 0.05;
                    }
                } else if dl.y < -0.3 {
                    let face = match (tail.filter(|t| *t < apex.y), bb) {
                        (Some(t), Some((_, rear))) => Some(t.max(rear)),
                        (Some(t), None) => Some(t),
                        (None, Some((_, rear))) => Some(rear.max(apex.y - 1.5)),
                        (None, None) => None,
                    };
                    if let Some(face) = face.filter(|f| *f < apex.y) {
                        apex.y = face - 0.05;
                    }
                }
                // One spot on the vehicle's axis threw one narrow beam: it is split into one
                // per headlamp on that face (within 35 cm of it), as far apart as the mean of
                // their distances from the axis (the outermost are indicators and position
                // lamps). The Agora's headlamps are 1.8 to 2.4 m apart.
                let half_width = ty.def.bounding_box.map_or(1.25, |bb| (bb[0] * 0.5).min(1.25));
                let on_face: Vec<f32> = lamps
                    .iter()
                    .filter(|l| (dl.y > 0.3 || dl.y < -0.3) && (l[1] - apex.y).abs() < 0.35)
                    .map(|l| (l[0] - apex.x).abs())
                    .collect();
                let spread = (on_face.iter().sum::<f32>() / on_face.len().max(1) as f32).min(half_width);
                let right = body.transform_vector3(Vec3::X).normalize_or_zero();
                let apex = body.transform_point3(apex);
                // inner and outer cone as full angles (values 10 and 11)
                let (inner, outer) = (
                    vals.get(10).copied().unwrap_or(30.0),
                    vals.get(11).copied().unwrap_or(70.0),
                );
                let half = |deg: f32| (deg.clamp(1.0, 179.0) * 0.5).to_radians().cos();
                let cone = [half(inner.min(outer)), half(outer)];
                let sides: &[f32] = if spread > 0.1 { &[-1.0, 1.0] } else { &[0.0] };
                for side in sides {
                    let at = v.position + (apex + right * spread * side).as_dvec3();
                    // vanilla: the spot, lit as the classic picture lights a lamp, only inside
                    // its cone (three point lights along its axis stood in for it before: they
                    // shone every way, on the bus's own body and saloon)
                    lights.push(PointLight {
                        position: at,
                        radius: vals[9].clamp(10.0, 45.0),
                        color,
                        intensity: VANILLA_HEADLIGHT_INTENSITY / sides.len() as f32 * (0.3 + 0.7 * night),
                        direction: d,
                        cone,
                        mode: LightMode::Vanilla,
                        ..Default::default()
                    });
                    // enhanced: falling off with the square of the distance from a one-metre core
                    lights.push(PointLight {
                        position: at,
                        radius: vals[9].clamp(10.0, 60.0),
                        color,
                        intensity: HEADLIGHT_INTENSITY / sides.len() as f32,
                        direction: d,
                        cone,
                        core: 1.0,
                        // (a plain spot, as Direct3D lights OMSI's road: the low-beam profile's
                        // bright band under a hard cut-off has nothing like it in the original)
                        beam: 0.0,
                        mode: LightMode::Enhanced,
                    });
                }
            }
        }
    }
    // [interiorlight]s light only the meshes listing them and the passengers (per-instance
    // term, see MeshProps::interior); they do not shine on the outside world.
    let _ = &ty.model.interior_lights;
}

/// Map lights and scenery coronas are only drawn this close to the camera.
const MAP_LIGHT_RANGE: f64 = 600.0;
const CORONA_RANGE: f64 = 1500.0;
/// How far the camera may move before the nearby static lights are gathered again.
const NEAR_MARGIN: f64 = 100.0;

/// The map's lights and coronas near the camera, switched for the lamp state. Going
/// through every corona of a city map (traffic lights, street lamps, signs) each frame
/// cost a millisecond; the near ones only change when the camera has moved on, the lamps
/// switch or tiles have loaded or gone (the streamer rebuilds the world's lists then).
#[derive(Default)]
struct NearLights {
    world: usize,
    generation: u64,
    lamps_on: bool,
    centre: DVec3,
    counts: (usize, usize),
    lights: Vec<PointLight>,
    coronas: Vec<Corona>,
}

static NEAR_LIGHTS: std::sync::Mutex<Option<NearLights>> = std::sync::Mutex::new(None);

/// Fill the scene's lights for this frame.
pub fn collect(
    world: &World,
    scene: &mut Scene,
    daylight: &Daylight,
    camera_pos: DVec3,
    vehicles: &[&VehicleInstance],
) {
    scene.lights.clear();
    scene.coronas.clear();
    scene.smoke.clear();
    omsi_sim::particles::set_eye(camera_pos);
    let night = daylight.night;
    {
        let mut guard = NEAR_LIGHTS.lock().unwrap_or_else(|e| e.into_inner());
        let near = guard.get_or_insert_with(NearLights::default);
        let static_lights = world.static_lights.lock();
        let static_coronas = world.static_coronas.lock();
        let world_id = world as *const World as usize;
        let generation = world
            .tiles_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        let stale = near.world != world_id
            || near.generation != generation
            || near.lamps_on != daylight.lamps_on
            || near.counts != (static_lights.len(), static_coronas.len())
            || (near.centre - camera_pos).length() > NEAR_MARGIN;
        if stale {
            *near = NearLights {
                world: world_id,
                generation,
                lamps_on: daylight.lamps_on,
                centre: camera_pos,
                counts: (static_lights.len(), static_coronas.len()),
                lights: Vec::new(),
                coronas: Vec::new(),
            };
            if daylight.lamps_on {
                near.lights.extend(static_lights.iter().filter(|l| {
                    (l.position - camera_pos).length() < MAP_LIGHT_RANGE + NEAR_MARGIN
                }));
            }
            for c in static_coronas.iter() {
                let on = match &c.switch {
                    LightSwitch::Constant(x) => *x,
                    LightSwitch::Night => daylight.lamps_on as i32 as f32,
                    LightSwitch::Variable(_) => daylight.lamps_on as i32 as f32,
                };
                if on <= 0.0
                    || (c.corona.position - camera_pos).length() > CORONA_RANGE + NEAR_MARGIN
                {
                    continue;
                }
                let mut corona = c.corona;
                corona.brightness *= on.min(1.0);
                near.coronas.push(corona);
            }
        }
        scene.lights.extend(
            near.lights
                .iter()
                .filter(|l| (l.position - camera_pos).length() < MAP_LIGHT_RANGE),
        );
        scene.coronas.extend(
            near.coronas
                .iter()
                .filter(|c| (c.position - camera_pos).length() <= CORONA_RANGE),
        );
    }
    // traffic lamps glow with what they show, by day as well
    for lamp in world.light_objects.lock().iter() {
        if (lamp.pos - camera_pos).length() > 1500.0 {
            continue;
        }
        for ((c, _), lit) in lamp.coronas.iter().zip(&lamp.lit) {
            if *lit <= 0.0 {
                continue;
            }
            let mut corona = *c;
            corona.brightness *= lit.min(1.0);
            scene.coronas.push(corona);
        }
    }
    for list in world.particle_objects.lock().values() {
        for po in list {
            if (po.pos - camera_pos).length() < 1500.0 {
                particle_sprites(&po.set, &mut scene.smoke, &mut scene.coronas);
            }
        }
    }
    if omsi_cfg::env::var_os("OMSI_DEBUG_PARTICLES").is_some() {
        if let Some(p) = scene.smoke.first() {
            log::info!("smoke: {} particles from objects, first at ({:.1}, {:.1}, {:.1}) size {:.2} alpha {:.2}", scene.smoke.len(), p.position.x, p.position.y, p.position.z, p.size, p.alpha);
        }
    }
    for v in vehicles {
        vehicle_lights(v, &mut scene.coronas, &mut scene.lights, night);
        particle_sprites(&v.particles, &mut scene.smoke, &mut scene.coronas);
        for t in &v.trailers {
            particle_sprites(&t.particles, &mut scene.smoke, &mut scene.coronas);
        }
    }
    // the lamps' cones in fog: drawn only while the visibility is
    // under 2 km, the fan's radius 2 m times 3 sqrt(100 / visibility) times the glow's
    // strength ((1 - ambient)^2 + 0.8) 0.6 brightness and the light's size; its colour the
    // light's times 0.3 (the viewing angle and the distance take their share in the shader,
    // which is told the visibility in `beam_width`)
    let (vis, night) = cone_weather();
    // The halo round a light in fog is half the cone's radius across either way, at 0.2.
    scene.coronas.retain_mut(|c| {
        if !c.beam && !c.halo {
            return true;
        }
        if vis >= 2000.0 {
            return false;
        }
        let glow = (night * night + 0.8) * 0.6 * c.brightness;
        let reach = 3.0 * (100.0 / vis.max(1.0)).sqrt() * glow * c.size;
        c.size = if c.beam { 2.0 * reach } else { reach };
        c.brightness = if c.beam { 0.3 } else { 0.2 };
        c.beam_width = vis.max(1.0);
        c.size > 0.05
    });
    if omsi_cfg::env::var_os("OMSI_DEBUG_CONES").is_some() {
        log::info!("cones: visibility {vis:.0} m, dark {night:.2}, {} cones of {} coronas", scene.coronas.iter().filter(|c| c.beam).count(), scene.coronas.len());
        for c in scene.coronas.iter().filter(|c| c.beam).take(4) {
            log::info!("  cone at ({:.1}, {:.1}, {:.1}) dir {:?} radius {:.2} half angles {:.0}/{:.0} deg tex {}", c.position.x, c.position.y, c.position.z, c.direction, c.size, c.inner_cos.to_degrees(), c.cone_cos.to_degrees(), c.texture);
        }
    }
    if omsi_cfg::env::var_os("OMSI_DEBUG_LIGHT").is_some() {
        scene.lights.push(PointLight {
            position: camera_pos + DVec3::new(0.0, 15.0, -2.0),
            radius: 40.0,
            color: [1.0, 0.9, 0.7],
            intensity: 2.0,
            ..Default::default()
        });
        scene.coronas.push(Corona {
            position: camera_pos + DVec3::new(0.0, 15.0, 0.0),
            size: 1.0,
            color: [1.0, 0.9, 0.7],
            brightness: 1.0,
            direction: Vec3::ZERO,
            cone_cos: -1.0,
            ..Default::default()
        });
        log::info!(
            "static lights: {:?}",
            world
                .static_lights
                .lock()
                .iter()
                .take(3)
                .collect::<Vec<_>>()
        );
        log::info!(
            "static coronas: {:?}",
            world
                .static_coronas
                .lock()
                .iter()
                .take(3)
                .collect::<Vec<_>>()
        );
    }
    // nearest lights first: the grid cells hold a limited number
    // (total_cmp: a light at a NaN position must not end the game)
    scene.lights.sort_by(|a, b| (a.position - camera_pos).length_squared().total_cmp(&(b.position - camera_pos).length_squared()));
}

/// The particles of a particle set as the renderer draws them: smoke blended over the scene,
/// and the glowing ones (`--PS_emissive--`: sparks, rockets, a flame) as coronas.
pub fn particle_sprites(set: &omsi_sim::particles::ParticleSet, smoke: &mut Vec<omsi_render::SmokeParticle>, coronas: &mut Vec<Corona>) {
    for (p, def) in set.particles() {
        let alpha = p.alpha();
        if alpha <= 0.002 {
            continue;
        }
        if def.emissive {
            coronas.push(Corona {
                position: p.pos,
                size: (p.size() * 0.5).max(0.02),
                color: p.color,
                brightness: alpha,
                direction: Vec3::ZERO,
                cone_cos: -1.0,
                z_offset: 0.0,
                ..Default::default()
            });
        } else {
            smoke.push(omsi_render::SmokeParticle { position: p.pos, size: p.size() * 0.5, color: p.color, alpha });
        }
    }
}

/// OMSI's picture for every `[smoke]` particle: `Texture\rauch.tga` of the game folder.
pub fn load_smoke_texture(renderer: &mut omsi_render::Renderer, root: &std::path::Path) {
    let path = omsi_cfg::resolve_path(root, "Texture/rauch.tga");
    match omsi_texture::decode_file(&path) {
        Ok(img) => renderer.set_smoke_texture(&img),
        Err(e) => log::warn!("smoke texture {}: {e}", path.display()),
    }
}

/// The pictures of the coronas besides the standard glow: a light's own `bitmap` and the
/// fog cone's `Texture/light_cone.bmp`, numbered as they are first asked for and uploaded
/// before the next frame is drawn (`upload_corona_textures`).
struct CoronaTextures {
    ids: std::collections::HashMap<std::path::PathBuf, u16>,
    pending: Vec<(u16, std::path::PathBuf)>,
    root: Option<std::path::PathBuf>,
}

static CORONA_TEXTURES: std::sync::Mutex<Option<CoronaTextures>> = std::sync::Mutex::new(None);

/// The game folder the standard pictures (`Texture\…`) are found in.
pub fn set_corona_root(root: &std::path::Path) {
    let mut g = CORONA_TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let t = g.get_or_insert_with(|| CoronaTextures { ids: Default::default(), pending: Vec::new(), root: None });
    t.root = Some(root.to_path_buf());
}

fn texture_id_of(path: std::path::PathBuf) -> u16 {
    let mut g = CORONA_TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
    let t = g.get_or_insert_with(|| CoronaTextures { ids: Default::default(), pending: Vec::new(), root: None });
    if let Some(id) = t.ids.get(&path) {
        return *id;
    }
    let id = (t.ids.len() + 1).min(u16::MAX as usize) as u16;
    t.ids.insert(path.clone(), id);
    t.pending.push((id, path));
    id
}

/// The picture id of a light's `bitmap`, looked for as the model's own textures are (its
/// folder's `texture\`, the folder, the vehicle's `texture\`, the game's `Texture\`); 0 (the
/// standard glow) when there is no such file.
pub fn corona_texture_id(model_dir: &std::path::Path, name: &str) -> u16 {
    // (every lit lamp of every vehicle asks every frame: looked up once)
    static KNOWN: std::sync::Mutex<Option<std::collections::HashMap<(std::path::PathBuf, String), u16>>> = std::sync::Mutex::new(None);
    let key = (model_dir.to_path_buf(), name.to_string());
    if let Some(&id) = KNOWN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key)) {
        return id;
    }
    let root = CORONA_TEXTURES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|t| t.root.clone()).unwrap_or_default();
    let mut id = 0;
    for d in crate::scene::texture_dirs(&root, model_dir) {
        let p = omsi_cfg::resolve_path(&d, name);
        if omsi_cfg::vfs::is_file(&p) {
            id = texture_id_of(p);
            break;
        }
    }
    KNOWN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key, id);
    id
}

/// One of the game's own light pictures in `Texture\` (0, the built-in glow, if missing).
fn stock_texture_id(name: &str) -> u16 {
    // (asked for every light of every vehicle every frame: the file is looked up once per
    // game folder, not each time - the lookups were a fifth of a frame's CPU time)
    static KNOWN: std::sync::Mutex<Option<std::collections::HashMap<(std::path::PathBuf, String), u16>>> = std::sync::Mutex::new(None);
    let root = CORONA_TEXTURES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|t| t.root.clone()).unwrap_or_default();
    let key = (root, name.to_string());
    if let Some(&id) = KNOWN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key)) {
        return id;
    }
    let p = omsi_cfg::resolve_path(&key.0, &format!("Texture/{name}"));
    let id = if omsi_cfg::vfs::is_file(&p) { texture_id_of(p) } else { 0 };
    KNOWN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key, id);
    id
}

/// The fog cone's picture (`Texture\light_cone.bmp`).
pub fn cone_texture_id() -> u16 {
    stock_texture_id("light_cone.bmp")
}

/// A light's glow when it names no bitmap of its own, and the halo round it in fog
/// (`Texture\licht.bmp`, the original).
pub fn glow_texture_id() -> u16 {
    stock_texture_id("licht.bmp")
}

/// The star of a light with effect bit 1 (`Texture\light_effect1.bmp`).
pub fn star_texture_id() -> u16 {
    stock_texture_id("light_effect1.bmp")
}

/// Upload the pictures asked for since the last frame.
pub fn upload_corona_textures(renderer: &mut omsi_render::Renderer) {
    let pending = {
        let mut g = CORONA_TEXTURES.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_mut() {
            Some(t) => std::mem::take(&mut t.pending),
            None => return,
        }
    };
    for (id, path) in pending {
        match omsi_texture::decode_file(&path) {
            Ok(img) => renderer.set_corona_texture(id, &img),
            Err(e) => log::warn!("corona picture {}: {e}", path.display()),
        }
    }
}

/// How strongly the lamps' cones show (0..1): fog, falling rain or snow, and the dark.
static CONE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

static CONE_NIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The weather the lamps' cones are drawn in: the fog's visibility (m) and how dark it is
/// (0 day … 1 night). Falling rain or snow counts only as far as it lowers the visibility.
pub fn set_cone_strength(fog_visibility_m: f32, _precip: f32, night: f32) {
    CONE.store(fog_visibility_m.to_bits(), std::sync::atomic::Ordering::Relaxed);
    CONE_NIGHT.store(night.clamp(0.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

fn cone_weather() -> (f32, f32) {
    let vis = f32::from_bits(CONE.load(std::sync::atomic::Ordering::Relaxed));
    let night = f32::from_bits(CONE_NIGHT.load(std::sync::atomic::Ordering::Relaxed));
    (if vis > 0.0 { vis } else { 1.0e6 }, night)
}

/// A vehicle's velocity (m/s, world) from its heading and speed: the airstream its glass
/// meets (`Lighting::glass_wind`).
pub fn vehicle_velocity(v: &omsi_sim::VehicleInstance) -> glam::Vec3 {
    let h = v.heading.to_radians();
    glam::Vec3::new(h.sin() as f32, h.cos() as f32, 0.0) * v.physics.speed
}
