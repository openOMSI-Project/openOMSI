//! The `World`, its lights and lamps, the drive probe and the ground queries.
use super::*;

pub struct World {
    pub root: PathBuf,
    pub global: GlobalCfg,
    pub map_dir: PathBuf,
    /// Indexed parked car lists of the map, loaded when a parking space uses one.
    pub(super) parklist: Mutex<HashMap<usize, Vec<String>>>,
    /// Render textures of the player's mirrors (`reflexionN.bmp`), by camera index.
    pub mirror_textures: Mutex<Vec<Option<TextureId>>>,
    /// Width / height of the glass of the player bus mirror N (from the mesh that shows its
    /// picture), 0 when not known: the shape of the panels that copy the mirrors to the screen.
    pub mirror_aspect: Mutex<Vec<f32>>,
    /// Where the glass of the player bus mirror N is and which way the picture's u and v run
    /// over it (middle, position per u, position per v; the bus's frame), from the mesh that
    /// shows it: how the panels that copy the mirrors turn the picture.
    pub mirror_glass: Mutex<Vec<Option<MirrorGlass>>>,
    pub(super) object_types: Mutex<HashMap<String, Option<Arc<ObjectType>>>>,
    pub(super) spline_types: Mutex<HashMap<String, Option<Arc<SplineType>>>>,
    pub textures: Arc<TextureCache>,
    /// World position (with terrain height) and rotation of every loaded map object by id.
    pub object_positions: Mutex<HashMap<i64, (DVec3, [f64; 3])>>,
    /// The objects whose id is used on more than one tile, by (tile, id) (see
    /// `MapIndex::duplicates`, `World::entry_point_place`).
    pub object_dups: Mutex<HashMap<((i32, i32), i64), (DVec3, [f64; 3])>>,
    /// Loaded terrains by tile coordinate.
    pub terrains: Arc<RwLock<HashMap<(i32, i32), Arc<Terrain>>>>,
    /// Surface rasters (roads) by tile coordinate.
    pub surfaces: Arc<RwLock<HashMap<(i32, i32), Arc<TileSurface>>>>,
    /// GPU resources per vehicle type (by .bus path) and paint scheme, shared by AI vehicles.
    pub(super) vehicle_gpu: Mutex<HashMap<VehicleKey, VehicleSet>>,
    /// GPU textures of vehicles by file: one bus spawned in twenty adverts used to upload
    /// its whole texture set twenty times (200 ms a spawn, 1.5 fps on Spandau).
    /// (With the number of sets holding each.)
    pub(super) vehicle_textures: Arc<Mutex<HashMap<PathBuf, (TextureId, usize)>>>,
    /// GPU meshes of vehicles by (bus file, mesh index), shared across paint schemes.
    pub(super) vehicle_meshes: Arc<Mutex<HashMap<(PathBuf, usize), (MeshId, usize)>>>,
    /// Vehicle meshes and textures made on a worker, until their set is uploaded.
    pub(super) vehicle_ready: Arc<Mutex<PreparedVehicles>>,
    /// Textures uploaded as RGBA to spare a frame, being compressed on the workers, and the
    /// compressed ones waiting to be swapped in.
    pub(super) upgrades_pending: Mutex<hashbrown::HashSet<PathBuf>>,
    pub(super) upgrades_done: Arc<Mutex<Vec<(PathBuf, Arc<TextureData>)>>>,
    /// Roller-blind pictures (`[matl_freetex]`) uploaded as RGBA, to be compressed.
    pub(super) freetex_upgrades: Arc<Mutex<Vec<PathBuf>>>,
    /// OMSI's `[texmemlimit]`: bytes the scenery and vehicle textures may take on the GPU
    /// (0 = no limit), and when the budget was last looked at.
    pub(super) texture_limit: std::sync::atomic::AtomicU64,
    pub(super) budget_checked: Mutex<Option<std::time::Instant>>,
    /// Traffic-path lanes collected while building tiles.
    pub lanes: Mutex<Vec<omsi_sim::traffic::Lane>>,
    /// The tiles whose lanes and parked cars have been put into `lanes` and `parked_cars`.
    /// These two and `lanes` are only filled, and should only be taken, while the `lanes`
    /// lock is held: whoever takes them then has every parked car together with the lanes
    /// it stands beside, and knows which tiles those came from.
    pub lane_tiles: Mutex<Vec<(i32, i32)>>,
    /// Traffic light programs of placed crossings, and which map object owns each.
    pub traffic_lights: Mutex<Vec<TrafficLightController>>,
    pub controller_of_object: Mutex<HashMap<i64, usize>>,
    /// Placed traffic light lamps.
    pub light_objects: Mutex<Vec<LightObject>>,
    /// Placed objects with scripts / animations.
    pub scripted: Mutex<Vec<ScriptedObject>>,
    /// The clock and the departure boards the scenery scripts read.
    pub timetable_boards: Mutex<StopBoards>,
    /// The map's `Holidays.txt`, read when first asked.
    pub calendar: std::sync::OnceLock<omsi_map::Calendar>,
    /// The number plates of `registrations.txt` (the active chrono scenarios' first, the
    /// latest before, then the map's own: the original), read when first asked.
    pub registrations: std::sync::OnceLock<Vec<String>>,
    /// The clock the run starts at: a scenery object placed before the simulation's clock
    /// reaches the boards runs its `{init}` on it (it ran on 09:00 of 1989).
    pub start_clock: Mutex<omsi_sim::SimClock>,
    /// Obstacles for vehicle collisions (of the loaded tiles; replaced when they change).
    pub collision: Mutex<Arc<omsi_sim::collision::CollisionWorld>>,
    /// `[crashmode_pole]` objects of the loaded tiles by collision key: where they stand and
    /// their instances, so that a post a vehicle knocked over can be laid on the ground
    /// (see [`World::lay_down_pole`]). A tile's posts leave with it: its instances are
    /// handed to other objects.
    pub poles: Mutex<HashMap<i64, (DVec3, Mat4, Vec<usize>)>>,
    /// Posts knocked over in this run and the way they fell: a tile loaded again lays them
    /// down again.
    pub(super) fallen_poles: Mutex<HashMap<i64, DVec3>>,
    /// The parked cars of the loaded tiles by collision key, so that one can pull out into
    /// the traffic (see [`World::depart_parked`]). They leave with their tile.
    pub parked_objects: Mutex<HashMap<i64, ParkedObject>>,
    /// The instances of the route arrows the map's author put up (`[helparrow]` objects) by
    /// tile: drawn only while OMSI 2's route arrows are on (see [`World::show_help_arrows`]).
    pub(super) help_arrows: Mutex<HashMap<(i32, i32), Vec<usize>>>,
    /// Whether they are drawn now.
    pub(super) help_arrows_shown: std::sync::atomic::AtomicBool,
    /// `OMSI_CHECK_ROADS`: road points under the ground, and where (over every tile this
    /// world has cut, see [`World::cut_terrain`]).
    pub(super) over_road: std::sync::atomic::AtomicUsize,
    pub(super) over_road_at: std::sync::Mutex<Vec<(f64, f64, f32, f32)>>,
    /// Parked cars that drove off in this run: their space stays empty when the tile comes
    /// back.
    pub(super) departed: Mutex<std::collections::HashSet<i64>>,
    /// What a parked car that drove off left behind: its object and its boxes, for an AI
    /// car of the same kind that parks in the space again (`return_parked`).
    pub(super) departed_objects: Mutex<std::collections::HashMap<i64, (ParkedObject, Vec<omsi_sim::collision::Obb>, Vec<omsi_sim::collision::Obb>)>>,
    /// The loaded tiles' own objects by map id, for the object editor (`crate::editor`).
    pub edit_objects: Mutex<HashMap<i64, EditObject>>,
    /// What the object editor did this run, by map id: kept over tile reloads until saved.
    pub object_edits: Mutex<HashMap<i64, ObjectEdit>>,
    /// The ground as the editor's brush has left it, by tile (read instead of the file).
    pub terrain_edits: Mutex<HashMap<(i32, i32), Terrain>>,
    /// Placed `[busstop]` objects: (map id, world position, heading, name).
    pub bus_stops: Mutex<Vec<(i64, DVec3, f64, String)>>,
    /// Where people wait at the stops: the `[passpos]` points of placed objects with a
    /// `[passengercabin]` (the maps' `people_standing_*` markers and bus shelters) as
    /// (object id, world position, heading in degrees, seat height - 0 for a standing place).
    pub waiting_places: Mutex<Vec<(i64, DVec3, f64, f32)>>,
    /// Passenger cabins of waiting objects by file, read once.
    pub(super) waiting_cabins: Mutex<HashMap<PathBuf, Option<Arc<omsi_vehicle::PassengerCabin>>>>,
    /// Counts the changes of the loaded tiles (see [`World::refresh_tile_lists`]): whoever
    /// keeps what it derived from the stops, the waiting places or the ground looks again.
    pub tiles_generation: std::sync::atomic::AtomicU64,
    /// Parked cars placed on `[carpark_p]` spaces: world position and heading (deg), for
    /// the traffic to steer round (see `lane_tiles`).
    pub parked_cars: Mutex<Vec<(DVec3, f64)>>,
    /// The boxes of the parked cars of the loaded tiles, which people walk round.
    pub parked_boxes: Mutex<Arc<Vec<omsi_sim::collision::Obb>>>,
    /// Placed objects with particle systems (chimney smoke, the fireworks, a memorial's
    /// flame) by tile.
    pub particle_objects: Mutex<HashMap<(i32, i32), Vec<ParticleObject>>>,
    /// The boxes of the loaded `[petrolstation]` objects (the depots' fuel and wash yards):
    /// OMSI lets the pump and the wash run only while the bus's box
    /// overlaps one, and sends the workshop's team out when the bus stands in none.
    pub petrol_stations: Mutex<Vec<omsi_sim::collision::Obb>>,
    /// Parked cars standing in the loaded tiles, and the options' `[AIMaxCountParked]`
    /// (0 = every space the map fills, -1 = none): past it the spaces stay empty.
    pub parked_live: std::sync::atomic::AtomicUsize,
    pub parked_max: i64,
    /// Places that echo (`[triggerbox_new]` + `[triggerbox_setreverb]`: the railway bridges'
    /// underpasses): the box, the reverberation time (s) and the distance (m) over which it
    /// fades in at the box's sides.
    pub reverb_zones: Mutex<Vec<(omsi_sim::collision::Obb, f32, f32)>>,
    /// The loaded tiles' night light maps (`.map.LM.bmp`), for the light map atlas.
    pub light_maps: Mutex<HashMap<(i32, i32), Arc<omsi_texture::Image>>>,
    pub(super) light_maps_generation: std::sync::atomic::AtomicU64,
    /// The atlas as last filled: centre tile and the generation of `light_maps`.
    pub(super) light_map_atlas: Mutex<Option<((i32, i32), u64)>>,
    /// The map's `signalroutes.cfg` (unit `mc_fahrstrasse`): which track pieces each railway
    /// signal protects, its distant signal, the next signal and a speed limit.
    pub signal_routes: Vec<omsi_map::ailists::SignalRoute>,
    /// Chrono folders active on the sim date, in order, and the merged AI lists / date.
    /// The chrono scenarios in force on the sim date (changed at midnight: `set_date`).
    pub chrono_dirs: parking_lot::RwLock<Vec<PathBuf>>,
    pub ailists: omsi_map::AiLists,
    pub date: i32,
    /// Ticket pack (chrono folders may override the map's).
    pub ticket_pack: String,
    /// Fonts for text and script textures, shared by all vehicles.
    pub fonts: Arc<Mutex<omsi_sim::texttex::FontLibrary>>,
    /// Scenery material variants switched by `NightlightA`: (instance, slot, material on, off).
    pub night_slots: Mutex<Vec<(usize, usize, MaterialId, MaterialId)>>,
    /// Objects whose night textures follow a `[NightMapMode]` timetable (see `update_night_modes`).
    pub night_modes: Mutex<Vec<NightMode>>,
    /// Light coronas and point lights of placed scenery objects.
    pub static_coronas: Mutex<Vec<StaticCorona>>,
    pub static_lights: Mutex<Vec<omsi_render::PointLight>>,
    /// Every tile file of the map read once: splines for the spline attachment rows,
    /// objects for entry points and stops of tiles that are not loaded.
    pub(super) index: Mutex<Option<Arc<MapIndex>>>,
    /// What each loaded tile added (see [`TileState`]).
    pub tile_state: Mutex<HashMap<(i32, i32), TileState>>,
    /// Tiles that have been loaded at least once: their lanes, light programs and parked
    /// cars are in the lists for good.
    pub(super) seeded: Mutex<hashbrown::HashSet<(i32, i32)>>,
    /// The GPU side of the loaded tiles.
    pub(super) gpu: Mutex<GpuCache>,
    /// Types this installation lacks, logged once each (file, what it is).
    pub(super) missing: Mutex<hashbrown::HashMap<String, &'static str>>,
    /// Tiles read and typed that loaded tiles (or tiles on their way) depend on.
    pub(super) staged: Mutex<HashMap<(i32, i32), Arc<StagedTile>>>,
    pub(super) layout: Mutex<Option<Arc<TileLayout>>>,
    /// Scenery objects' sound configurations, read once per file.
    pub(super) sound_cfgs: Mutex<HashMap<PathBuf, Option<Arc<omsi_vehicle::SoundCfg>>>>,
}

/// A placed object's particle systems (`[smoke]`, `[particle_emitter]`).
pub struct ParticleObject {
    pub map_id: i64,
    pub pos: DVec3,
    pub rot: Mat4,
    pub set: omsi_sim::particles::ParticleSet,
}

/// A `[light_enh]`/`[light_enh_2]` of a placed scenery object.
#[derive(Debug, Clone)]
pub struct StaticCorona {
    pub corona: omsi_render::Corona,
    /// What switches it: a constant, the night flag, or an object variable (treated as on).
    pub switch: LightSwitch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LightSwitch {
    Constant(f32),
    Night,
    Variable(String),
}

impl LightSwitch {
    pub fn parse(var: &str) -> LightSwitch {
        let v = var.trim();
        if let Ok(x) = v.parse::<f32>() {
            LightSwitch::Constant(x)
        } else if v.eq_ignore_ascii_case("NightlightA") || v.is_empty() {
            LightSwitch::Night
        } else {
            LightSwitch::Variable(v.to_string())
        }
    }
}

/// Resolve the three standard traffic-lamp channels without going through the scenery VM.
///
/// Stock `.sco` files use `red`, `yellow` and `green` in both `[visible]` and
/// `[light_enh_2]`.  Keeping this mapping at the renderer boundary is important: a missing
/// or platform-specific script load must not turn an unknown variable into an always-visible
/// mesh (which makes all three bulbs appear lit).  Non-standard channels, such as `Left`, are
/// still resolved by the object's script.
pub fn standard_traffic_lamp(
    var: &str,
    red: bool,
    yellow: bool,
    green: bool,
    approach: bool,
) -> Option<f32> {
    match var.trim().to_ascii_lowercase().as_str() {
        "red" | "rot" => Some(red as i32 as f32),
        "yellow" | "gelb" | "amber" => Some(yellow as i32 as f32),
        "green" | "gruen" | "grün" => Some(green as i32 as f32),
        "trafficlightapproach" => Some(approach as i32 as f32),
        _ => None,
    }
}

/// Resolve a lamp channel after its script has run. A zero script output is meaningful
/// (not a reason to use the stock phase), notably while a pedestrian lamp is blinking.
pub(crate) fn traffic_lamp_value(var: &str, scripted: Option<f32>, standard: Option<f32>) -> f32 {
    scripted
        .or_else(|| var.trim().parse::<f32>().ok())
        .or(standard)
        .unwrap_or(0.0)
}

/// Coronas and point lights of a model in the frame of `xf` (rotation) at `pos`.
/// `value_of` resolves the light variables (vehicle state or scenery switches), with the
/// lights' brightness as their `timeconst` has let it follow the switch (`fades`: one value
/// per `[light_enh_2]` of the model in order; missing = at once).
///
/// The lights of every detail level count, not only LOD 0's: a `[light_enh]` belongs to the
/// mesh before it, and 40 stock models (the Spandau neon, sodium and gas street lamps, the
/// Sv signals, the ICE and RE160 coaches) declare theirs after the far `[LOD] 0` mesh - their
/// glow still shows up close in OMSI. No stock model repeats a light in two levels.
#[cfg(test)]
pub fn model_lights_faded(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
) -> Vec<omsi_render::Corona> {
    let mut out = Vec::new();
    model_lights_into(model, mesh_transforms, pos, value_of, fades, &mut out, None);
    out
}

/// [`model_lights_faded`] added to `out`: every vehicle's lamps are gathered every frame,
/// and a list of their own (and one of their owners) for each was only copied over.
pub fn model_lights_extend(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
    out: &mut Vec<omsi_render::Corona>,
) {
    model_lights_into(model, mesh_transforms, pos, value_of, fades, out, None);
}

/// Every light of a model in the order [`model_lights_owned`] numbers them: the mesh it
/// belongs to, its place and its direction (zero for a `[light_enh]` and an omni light).
/// Omsi.exe files each `[light_enh]`/`[light_enh_2]` with the `[mesh]` before it (the
/// model loader, 0x5f3140: the light goes into the current mesh's list, mesh +0x1b0) and
/// draws it where that mesh's animation takes it - the lamps along a level crossing's arm
/// rise with the arm.
pub fn model_light_sources(model: &Model) -> Vec<(usize, glam::Vec3, glam::Vec3)> {
    let mut out = Vec::new();
    for (i, md) in model.meshes.iter().enumerate() {
        for l in &md.light_enh {
            out.push((i, glam::Vec3::from(l.pos), glam::Vec3::ZERO));
        }
        for l in &md.light_enh_2 {
            out.push((i, glam::Vec3::from(l.pos), if l.omni { glam::Vec3::ZERO } else { glam::Vec3::from(l.dir) }));
        }
    }
    out
}

/// The sprites of one lamp, a `[light_enh]` or a `[light_enh_2]` alike (Omsi.exe 0x5a0068):
/// its `glow`, left out with effect bit 4; with effect bit 1 a star (light_effect1.bmp)
/// turned to the viewer, 2.5 times the size and growing with the glow's strength
/// (corona.wgsl, flag bit 8); without bit 2 a halo round it in fog, seen from in front
/// (sizes and strengths in `lights::collect` and corona.wgsl, like the cone's) - `size` is
/// the light's size, `halo_cone` its outer and inner half cone angles (radians).
pub(super) fn push_lamp_sprites(out: &mut Vec<omsi_render::Corona>, glow: omsi_render::Corona, effect: u8, size: f32, halo_cone: (f32, f32)) {
    if effect & 4 == 0 {
        out.push(glow);
    }
    if effect & 1 != 0 {
        out.push(omsi_render::Corona {
            size: size * 1.25,
            rotating: 2,
            flags: 8,
            texture: crate::lights::star_texture_id(),
            ..glow
        });
    }
    if effect & 2 == 0 {
        out.push(omsi_render::Corona {
            position: glow.position,
            size,
            color: glow.color,
            brightness: glow.brightness,
            direction: glow.direction,
            cone_cos: halo_cone.0,
            inner_cos: halo_cone.1,
            texture: crate::lights::glow_texture_id(),
            halo: true,
            ..Default::default()
        });
    }
}

/// [`model_lights_faded`], each sprite with the light it belongs to (the n-th light of the
/// model, `[light_enh]` and `[light_enh_2]` in file order - the order `value_of` is asked
/// in): one light gives several sprites (its glow, star, fog halo and cone).
pub fn model_lights_owned(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
) -> Vec<(omsi_render::Corona, usize)> {
    let mut out = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    model_lights_into(model, mesh_transforms, pos, value_of, fades, &mut out, Some(&mut owners));
    out.into_iter().zip(owners).collect()
}

/// The sprites of a model's lights added to `out`, and with `owners` the light each of them
/// belongs to (see [`model_lights_owned`]).
fn model_lights_into(
    model: &Model,
    mesh_transforms: &dyn Fn(usize) -> Mat4,
    pos: DVec3,
    value_of: &dyn Fn(&str) -> f32,
    fades: &[f32],
    out: &mut Vec<omsi_render::Corona>,
    mut owners: Option<&mut Vec<usize>>,
) {
    let base = out.len();
    let mut seq = 0usize;
    let mut li = 0usize;
    let model_dir = model.path.parent().unwrap_or(std::path::Path::new(""));
    for (i, md) in model.meshes.iter().enumerate() {
        // most meshes carry no light, and their transform is not free (every mesh of every
        // AI car, every frame)
        if md.light_enh.is_empty() && md.light_enh_2.is_empty() {
            continue;
        }
        let first_li = li;
        li += md.light_enh_2.len();
        let xf = mesh_transforms(i);
        for l in &md.light_enh {
            if let Some(o) = owners.as_deref_mut() {
                o.resize(out.len() - base, seq.wrapping_sub(1));
            }
            seq += 1;
            // Omsi.exe reads a `[light_enh]` into the same lamp as a `[light_enh_2]` (0x5f2bb6,
            // a TLampensetting) and draws it alike (0x5a0068): omnidirectional, turned to the
            // viewer, its four numbers the brightness factor, the z offset, the effect bits
            // and the fade time, then its own bitmap. Drawn as a bare licht.bmp glow on the
            // lamp, the stop request lamp of the MAN NL and SD202 (`D92_Haltewunsch.bmp`,
            // 5 cm to the front) showed as a ring round its dome (#1159). (Its fade time is
            // not followed: such a lamp is on or off at once.)
            let factor = l.values.first().copied().filter(|f| *f > 0.0).unwrap_or(1.0);
            let b = (value_of(&l.variable) * factor).clamp(0.0, 2.0);
            if !(b > 0.0) || !b.is_finite() {
                continue;
            }
            let p = xf.transform_point3(glam::Vec3::from(l.pos)).as_dvec3() + pos;
            let effect = l.values.get(2).map(|v| *v as i32).unwrap_or(1).clamp(0, 7) as u8;
            // (the glow is as wide as the light's size: OMSI draws its sprite half that
            // either side of the lamp)
            let glow = omsi_render::Corona {
                position: p,
                size: (l.size * 0.5).max(0.0),
                color: [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0],
                brightness: b,
                direction: glam::Vec3::ZERO,
                cone_cos: -1.0,
                rotating: 2,
                z_offset: l.values.get(1).copied().unwrap_or(0.1).max(0.0),
                flags: effect & !1,
                texture: l.texture.as_deref().map(|b| crate::lights::corona_texture_id(model_dir, b)).filter(|t| *t != 0).unwrap_or_else(crate::lights::glow_texture_id),
                ..Default::default()
            };
            push_lamp_sprites(out, glow, effect, l.size, (0.0, 0.0));
        }
        for (k, l) in md.light_enh_2.iter().enumerate() {
            if let Some(o) = owners.as_deref_mut() {
                o.resize(out.len() - base, seq.wrapping_sub(1));
            }
            seq += 1;
            // the fading variable: 0 dark, 1 normal, 2 double (times the factor), as far as
            // the lamp has come on or gone out (`timeconst`)
            let b = match fades.get(first_li + k) {
                Some(f) => *f,
                None => (value_of(&l.variable) * if l.factor > 0.0 { l.factor } else { 1.0 }).clamp(0.0, 2.0),
            };
            if !(b > 0.0) || !b.is_finite() {
                continue;
            }
            let p = xf.transform_point3(glam::Vec3::from(l.pos)).as_dvec3() + pos;
            let dir = if l.omni {
                glam::Vec3::ZERO
            } else {
                xf.transform_vector3(glam::Vec3::from(l.dir))
                    .normalize_or_zero()
            };
            let up = xf.transform_vector3(glam::Vec3::from(l.up)).normalize_or(glam::Vec3::Z);
            let half_cos = |deg: f32| (deg.max(1.0) * 0.5).min(180.0).to_radians().cos();
            let (outer, inner) = (l.cone_outer.max(l.cone_inner), l.cone_inner.min(l.cone_outer));
            let flags = l.values.first().map(|v| omsi_cfg::parse_f32(v) as i32).unwrap_or(0).clamp(0, 7) as u8;
            let color = [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0];
            // the original: the glow, the light's own bitmap (else licht.bmp) as wide as its
            // size, with its star and its halo in fog (`push_lamp_sprites`)
            let glow = omsi_render::Corona {
                position: p,
                size: (l.size * 0.5).max(0.0),
                color,
                brightness: b,
                direction: dir,
                cone_cos: half_cos(outer),
                inner_cos: if inner > 0.0 { half_cos(inner) } else { -2.0 },
                // an omnidirectional light has no face to show: it turns to the viewer
                rotating: if l.omni { 2 } else { l.rotating.clamp(0, 2) as u8 },
                up,
                z_offset: l.z_offset.max(0.0),
                flags: flags & !1,
                texture: l.bitmap.as_deref().map(|b| crate::lights::corona_texture_id(model_dir, b)).filter(|t| *t != 0).unwrap_or_else(crate::lights::glow_texture_id),
                ..Default::default()
            };
            push_lamp_sprites(out, glow, flags, l.size, ((outer * 0.5).to_radians(), (inner.max(0.0) * 0.5).to_radians()));
            // the light's cone in fog (the original: built for a directional light
            // with the cone flag whose cone angles make sense; effect bit 2 leaves it out).
            // Its size and strength follow the weather and the viewer (`lights::collect`,
            // corona.wgsl), so the raw values go along: the light's size and brightness and
            // the half cone angles in radians.
            if l.cone && !l.omni && dir.length_squared() > 0.5 && l.cone_inner >= 0.0 && l.cone_outer >= l.cone_inner && flags & 2 == 0 {
                out.push(omsi_render::Corona {
                    position: p,
                    size: l.size,
                    color: [l.color[0] / 255.0, l.color[1] / 255.0, l.color[2] / 255.0],
                    brightness: b,
                    direction: dir,
                    cone_cos: (l.cone_outer * 0.5).to_radians(),
                    inner_cos: (l.cone_inner * 0.5).to_radians(),
                    texture: crate::lights::cone_texture_id(),
                    beam: true,
                    ..Default::default()
                });
            }
        }
    }
    if let Some(o) = owners {
        o.resize(out.len() - base, seq.wrapping_sub(1));
    }
}

/// An object whose top stays this low (m over its foot) is no wall for a vehicle body; with
/// a `[collision_mesh]` it is a step the wheels climb (a traffic island).
pub const LOW_OBJECT: f32 = 0.3;

/// Faces this close over another road face are paint on it, not a step (m). Omsi.exe's
/// ground query (0x7a0814) takes the highest face whatever lies under it; this keeps only
/// the thinnest layers flat (a marking a centimetre or two over the asphalt). At 4.5 cm it
/// also took the speed cushions, manhole and plate objects, lowered kerbs and slab edges
/// away, and the bottom 4.5 cm of every speed bump's ramp - "no road bumps", and wheels
/// drawn sunk into what they drove on.
pub(super) const PAINT_LAYER: f32 = 0.02;

/// What a wheel stands on at world (x, y): the faces of the roads, crossings and surface
/// objects there, and the terrain wherever it is not cut away under them - the highest at
/// or below `top`, and the lowest above it (a kerb the tyre is up against).
pub fn drive_probe(
    terrains: &RwLock<HashMap<(i32, i32), Arc<Terrain>>>,
    surfaces: &RwLock<HashMap<(i32, i32), Arc<TileSurface>>>,
    x: f64,
    y: f64,
    top: f64,
) -> omsi_sim::rigid::GroundProbe {
    let key = tile_key(x, y);
    let surface = surfaces.read().get(&key).cloned();
    let terrain = terrains.read().get(&key).cloned();
    probe_tile(surface.as_deref(), terrain.as_deref(), key, x, y, top)
}

pub(super) fn tile_key(x: f64, y: f64) -> (i32, i32) {
    (
        (x / tile_size()).floor() as i32,
        (y / tile_size()).floor() as i32,
    )
}

/// [`drive_probe`] on the tile `key` that holds (x, y).
pub(super) fn probe_tile(
    surface: Option<&TileSurface>,
    terrain: Option<&Terrain>,
    key: (i32, i32),
    x: f64,
    y: f64,
    top: f64,
) -> omsi_sim::rigid::GroundProbe {
    let lx = (x - key.0 as f64 * tile_size()) as f32;
    let ly = (y - key.1 as f64 * tile_size()) as f32;
    let mut probe = omsi_geometry::Probe::default();
    let mut zs = [0f32; 64];
    let mut walls = None;
    let n = surface.map_or(0, |s| s.drive.heights(lx, ly, &mut zs, &mut walls));
    if let Some(s) = surface {
        let below = |lim: f32| {
            if n > zs.len() {
                s.drive.probe(lx, ly, lim)
            } else {
                zs[..n].iter().fold(omsi_geometry::Probe::default(), |p, &z| p.merge(omsi_geometry::Probe::of(z, lim)))
            }
        };
        probe = below(top as f32);
        // a painted layer is no step: road markings made as `[surface]` objects or as
        // splines with a height profile lie a centimetre or three over the asphalt, and the
        // wheels climbed every line - the bus hopped at stops and over dotted lines (Horizon).
        // Where another road face lies that little below, the wheel stands on that one.
        // (only road faces: the terrain under a road is often that close too)
        // (layer under layer: a marking over a marking over the asphalt, as the map editor
        // stacks them where lines cross or a box junction lies over a lane's arrows, was
        // still a step when only the first was looked through)
        let mut layers = 0;
        while let Some(z1) = probe.below.filter(|_| layers < 4) {
            layers += 1;
            match below(z1 - 0.0005).below {
                Some(z2) if z1 - z2 < PAINT_LAYER => probe.below = Some(z2),
                _ => break,
            }
        }
    }
    // On a road the wheel stands on the road, as in OMSI: the terrain under it or over it
    // (an embankment the road runs under, ground poking through the asphalt) is no
    // ground and no wall there. Taken with the road, a terrain face over the carriageway was
    // an invisible wall under bridges, and one through it a bump that threw the bus.
    let ground = terrain.map(|t| {
        let h = omsi_geometry::terrain_height(t, lx, ly);
        let cut = surface
            .map(|s| s.cut_at(lx, ly, h, surface_flush()))
            .unwrap_or(false);
        (h, cut)
    });
    // ... unless that face lies buried well under ground that is drawn here and is under the
    // wheel, not over it: the lower slope of an embankment spline (Marcel's `Damm1` falls
    // 20 m over 30 m on each side) reaching under a junction the terrain carries. Omsi.exe
    // takes the highest face there, the ground; taken as the road, it dropped the bus 8 m
    // through the asphalt into the slope (Cotterell, the junction by the park at 250, 427).
    let buried = matches!((probe.below, ground), (Some(z), Some((h, false))) if h <= top as f32 && h - z > BURIED_FACE);
    let on_road = probe.below.is_some() && !buried;
    if let (Some((h, cut)), false) = (ground, on_road) {
        // the ground counts where it is drawn; where it is cut away and nothing else is
        // there (a surface without a collision), it still carries rather than let the
        // vehicle drop out of the world
        if !cut || (probe.below.is_none() && h <= top as f32) {
            probe = probe.merge(omsi_geometry::Probe::of(h, top as f32));
        }
    }
    // A wall's top (a narrow height profile high on a wall spline) is never stood on: where
    // it stands a step over the ground here it is a wall the tyre meets, whatever the height
    // it is probed from; nearer the ground than that the wheel rolls on the road beside it
    // (where the wall's top met the road the wheels went up onto it and rode along it)
    if let Some(s) = surface {
        let walls = if n > zs.len() { s.drive.probe_walls(lx, ly, f32::MAX).below } else { walls };
        if let (Some(zw), Some(g)) = (walls, probe.below) {
            if zw > g + WALL_TOP_STEP {
                probe.above = Some(probe.above.map_or(zw, |a| a.min(zw)));
            }
        }
    }
    omsi_sim::rigid::GroundProbe {
        below: probe.below.map(|z| z as f64),
        above: probe.above.map(|z| z as f64),
    }
}

/// What is drawn at world (x, y) under `top`: the highest face of the splines and surface
/// objects (as lifted for drawing) and the terrain where it is not cut away - the picture's
/// ground, without any of the wheel rules of [`drive_probe`] (`OMSI_GROUND_GAP` measures the
/// tyres against it).
pub fn drawn_ground(
    terrains: &RwLock<HashMap<(i32, i32), Arc<Terrain>>>,
    surfaces: &RwLock<HashMap<(i32, i32), Arc<TileSurface>>>,
    x: f64,
    y: f64,
    top: f64,
) -> Option<f64> {
    let key = tile_key(x, y);
    let lx = (x - key.0 as f64 * tile_size()) as f32;
    let ly = (y - key.1 as f64 * tile_size()) as f32;
    let surface = surfaces.read().get(&key).cloned();
    let terrain = terrains.read().get(&key).cloned();
    let mut best: Option<f32> = None;
    if let Some(s) = surface.as_deref() {
        let a = s.drive.probe(lx, ly, top as f32).below;
        let b = s.drive.probe_walls(lx, ly, top as f32).below;
        best = a.into_iter().chain(b).reduce(f32::max);
    }
    if let Some(t) = terrain.as_deref() {
        let h = omsi_geometry::terrain_height(t, lx, ly);
        let cut = surface.as_deref().is_some_and(|s| s.cut_at(lx, ly, h, surface_flush()));
        if !cut && h <= top as f32 {
            best = Some(best.map_or(h, |b| b.max(h)));
        }
    }
    best.map(|z| z as f64)
}

/// How far a road face may lie under drawn ground before it counts as buried (m): far more
/// than the ground poking through the asphalt that the road is there to keep out.
pub(super) const BURIED_FACE: f32 = 1.0;

/// How far over the ground a wall's top must stand to be a wall to the wheels (a kerb is
/// less, and the tyre climbs it).
pub(super) const WALL_TOP_STEP: f32 = 0.3;

/// The ground the player's wheels stand on: [`drive_probe`] over the loaded tiles.
pub struct DriveGround {
    pub terrains: Arc<RwLock<HashMap<(i32, i32), Arc<Terrain>>>>,
    pub surfaces: Arc<RwLock<HashMap<(i32, i32), Arc<TileSurface>>>>,
}

pub(super) type TileRefs = ((i32, i32), Option<Arc<TileSurface>>, Option<Arc<Terrain>>);

impl omsi_sim::rigid::Ground for DriveGround {
    fn probe(&self, x: f64, y: f64, top: f64) -> omsi_sim::rigid::GroundProbe {
        drive_probe(&self.terrains, &self.surfaces, x, y, top)
    }

    /// The tiles under the vehicle are looked up once per step, not twice for every one of
    /// the few hundred points its tyres ask for (each a lock of both maps the loader works
    /// on, two lookups and two reference counts).
    fn session(&self) -> Box<dyn Fn(f64, f64, f64) -> omsi_sim::rigid::GroundProbe + '_> {
        let tiles: std::cell::RefCell<Vec<TileRefs>> =
            std::cell::RefCell::new(Vec::with_capacity(4));
        Box::new(move |x, y, top| {
            let key = tile_key(x, y);
            let mut tiles = tiles.borrow_mut();
            let i = match tiles.iter().position(|t| t.0 == key) {
                Some(i) => i,
                None => {
                    tiles.push((
                        key,
                        self.surfaces.read().get(&key).cloned(),
                        self.terrains.read().get(&key).cloned(),
                    ));
                    tiles.len() - 1
                }
            };
            let (_, surface, terrain) = &tiles[i];
            probe_tile(surface.as_deref(), terrain.as_deref(), key, x, y, top)
        })
    }
}

/// Raster resolution of the per-tile surface mask (texels per tile edge).
pub const SURFACE_RASTER: usize = 512;

impl World {
    /// Ground height (road surface where present, else terrain) at world x, y.
    /// Terrain height alone at world x, y (no road surfaces).
    pub fn ground_terrain(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let t = self.terrains.read();
        Some(t.get(&(tx, ty))?.sample(lx, ly) as f64)
    }

    /// The height a vehicle put down at (x, y) stands at: the face its wheels would stand on
    /// (a road, a deck, a floor, the ground; [`drive_probe`]) under `near` + 1.5 m and at most
    /// 3 m below it. The raster's [`World::ground_height`] takes the surface of its texel,
    /// and a bus put down beside a wall (an entry point on a pavement, London) stood on the
    /// wall's top and floated there.
    pub fn stand_height(&self, x: f64, y: f64, near: f64) -> Option<f64> {
        drive_probe(&self.terrains, &self.surfaces, x, y, near + 1.5).below.filter(|g| near - g < 3.0)
    }

    /// Where entry point `ep` stands (position, heading): its object, found on the tile
    /// the entry point names (global.cfg's `[entrypoints]` record holds the index of its
    /// tile in the `[map]` list, and the place within that tile). An object of that id on
    /// another tile (a map joined from two, whose ids repeat) is not it: the record's own
    /// place is taken then.
    pub fn entry_point_place(&self, ep: &omsi_map::global::EntryPoint) -> Option<(DVec3, [f64; 3])> {
        let s = tile_size();
        let tile = usize::try_from(ep.group).ok().and_then(|i| self.global.raw_tiles.get(i)).copied();
        if let Some(t) = tile {
            if let Some(p) = self.object_dups.lock().get(&(t, ep.object_id)) {
                return Some(*p);
            }
        }
        let found = self.object_positions.lock().get(&ep.object_id).copied();
        let recorded = tile.filter(|_| ep.pos.iter().chain(ep.quat.iter()).all(|v| v.is_finite())).map(|(tx, ty)| {
            let heading = (2.0 * ep.quat[1].atan2(ep.quat[3])).to_degrees().rem_euclid(360.0);
            (DVec3::new(tx as f64 * s + ep.pos[0], ty as f64 * s + ep.pos[1], ep.pos[2]), [heading, 0.0, 0.0])
        });
        match (found, recorded) {
            // (an object may stand a little outside its tile's square: far off only is another)
            (Some(f), Some(r)) if (f.0.truncate() - r.0.truncate()).length() > 50.0 => {
                log::info!("entry point {} \"{}\": object {} stands at ({:.0}, {:.0}), on another tile than the entry point's ({:.0}, {:.0}): the entry point's own place", ep.index, ep.name, ep.object_id, f.0.x, f.0.y, r.0.x, r.0.y);
                Some(r)
            }
            (Some(f), _) => Some(f),
            (None, r) => r,
        }
    }

    pub fn ground_height(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        if let Some(s) = self.surfaces.read().get(&(tx, ty)) {
            // the road the wheels stand on, not a bridge deck or an embankment over it
            if let Some(h) = s.sample_road(lx, ly).or_else(|| s.sample(lx, ly)) {
                return Some(h as f64);
            }
        }
        let t = self.terrains.read();
        let terrain = t.get(&(tx, ty))?;
        Some(terrain.sample(lx, ly) as f64)
    }

    /// The wetness a puddle would use at world (x, y): `wetness` where a road surface is
    /// under the point (the same `[moisture]` ground `enhanced.wgsl`'s reflective puddle
    /// patches sit on), 0 on bare terrain or where no surface is loaded there yet. Approximate
    /// on purpose - `puddles::water_at` only needs to agree with the shader's own mask
    /// closely enough that a tyre's spray starts where the reflection does, not to the texel.
    pub fn wet_road_at(&self, x: f64, y: f64, wetness: f32) -> f32 {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let on_road = self
            .surfaces
            .read()
            .get(&(tx, ty))
            .is_some_and(|s| s.sample_road(lx, ly).is_some());
        if on_road {
            wetness.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The ground under a point of the outside camera's arm: the highest face of the roads,
    /// crossings, surface objects and terrain at or below `top`. A face higher up - the
    /// roof over a petrol station's forecourt, a bridge deck - is not the ground there (the
    /// top surface of the raster is, and it put the camera on the canopy); a roof's mesh
    /// stops the camera instead.
    pub fn camera_ground(&self, x: f64, y: f64, top: f64) -> Option<f64> {
        drive_probe(&self.terrains, &self.surfaces, x, y, top).below
    }

    /// Local visible road plane under a vehicle: the exact faces where there are any (raster
    /// heights are coarser). Choose the nearby deck, never a roof above it.
    pub fn puddle_surface(&self, position: DVec3) -> Option<(f64, glam::Vec3)> {
        let height = self.camera_ground(position.x, position.y, position.z + 0.35)?;
        let key = tile_key(position.x, position.y);
        let x = (position.x - key.0 as f64 * tile_size()) as f32;
        let y = (position.y - key.1 as f64 * tile_size()) as f32;
        let normal = self.surfaces.read().get(&key)
            .and_then(|s| s.drive.surface_below(x, y, height as f32 + 0.002))
            .filter(|(z, _)| (*z as f64 - height).abs() < 0.005)
            .map(|(_, n)| n).unwrap_or(glam::Vec3::Z);
        Some((height, normal))
    }

    /// The ground painting of one tile: `texture/map/<tile>.map.<layer>.dds`, one 8-bit
    /// alpha mask per `[groundtex]` above the first that the editor's brush has touched on
    /// this tile. That is how OMSI puts asphalt under a car park, cobbles on a side street
    /// or a field into the meadow without placing a single object.
    ///
    /// The mask is stored like a picture (first row = north), the terrain mesh's v runs
    /// north with y, so the rows are turned over here.
    pub(super) fn load_ground_paint(&self, tile_path: &Path) -> Vec<(usize, Image)> {
        let Some(name) = tile_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for layer in 1..self.global.ground_textures.len() {
            let path =
                omsi_cfg::resolve_path(&self.map_dir, &format!("texture/map/{name}.{layer}.dds"));
            if !omsi_cfg::vfs::is_file(&path) {
                continue;
            }
            match omsi_texture::decode_file(&path) {
                Ok(img) => {
                    let (w, h) = (img.width as usize, img.height as usize);
                    let mut rgba = vec![0u8; w * h * 4];
                    for j in 0..h {
                        let src = &img.rgba[(h - 1 - j) * w * 4..][..w * 4];
                        rgba[j * w * 4..][..w * 4].copy_from_slice(src);
                    }
                    out.push((
                        layer,
                        Image {
                            width: img.width,
                            height: img.height,
                            rgba,
                            has_alpha: true,
                        },
                    ));
                }
                Err(e) => log::warn!("ground paint {}: {e}", path.display()),
            }
        }
        out
    }

    /// Height for somebody on foot: the top of whatever is here - a pavement, a platform,
    /// a painted yard - and the bare ground where there is nothing. [`ground_height`] is
    /// the wheels' answer instead: it picks the drivable surface, which is the road *under*
    /// the kerb, and standing people on that buried them to the ankles in the pavement.
    pub fn walk_height(&self, x: f64, y: f64) -> Option<f64> {
        let tx = (x / tile_size()).floor() as i32;
        let ty = (y / tile_size()).floor() as i32;
        let lx = (x - tx as f64 * tile_size()) as f32;
        let ly = (y - ty as f64 * tile_size()) as f32;
        let surface = self
            .surfaces
            .read()
            .get(&(tx, ty))
            .and_then(|s| s.sample(lx, ly))
            .map(|h| h as f64);
        let terrain = self
            .terrains
            .read()
            .get(&(tx, ty))
            .map(|t| t.sample(lx, ly) as f64);
        let rough = match (surface, terrain) {
            (Some(s), Some(t)) => Some(s.max(t)),
            (s, t) => s.or(t),
        }?;
        // The raster says roughly where the floor is (a texel is 0.7 m on a Berlin tile, and
        // it holds the highest surface in it): the faces themselves say exactly. Read from the
        // raster, people stood 15 cm up in the air beside a kerb or sank into it, and climbed
        // every slope in steps. The highest face a little over the raster's height is taken:
        // the kerb's top on the pavement, the carriageway beside it.
        let probe = drive_probe(&self.terrains, &self.surfaces, x, y, rough + 0.3);
        match probe.below {
            Some(b) if rough - (b as f64) < 1.0 => Some(b as f64),
            _ => Some(rough),
        }
    }

    /// The floor under somebody at height `near` at (x, y): the highest face no more than a
    /// step (0.5 m, Omsi.exe 0x630498) over them - a station's floor under its roof, a car park's level under the
    /// deck above - else [`World::walk_height`]'s highest one. (Asked for the highest, the
    /// people of an indoor station stood on its roof.)
    ///
    /// Nothing under them within 3 m: the highest face, but only up to 0.5 m over them - a
    /// pavement whose tile came after them. Omsi.exe keeps its people at the heights of
    /// their paths and waiting places; the highest face, a bus shelter's roof 2.5 m up, put
    /// the people waiting under it on top of it.
    pub fn walk_height_near(&self, x: f64, y: f64, near: f64) -> Option<f64> {
        self.walk_height_reach(x, y, near, 0.5)
    }

    /// [`World::walk_height_near`] that also sees faces up to `reach` over `near`: the walker
    /// on foot looks a metre up to stop at a face too high to step onto (a platform's edge)
    /// instead of walking under it.
    pub fn walk_height_reach(&self, x: f64, y: f64, near: f64, reach: f64) -> Option<f64> {
        let probe = drive_probe(&self.terrains, &self.surfaces, x, y, near + reach);
        match probe.below {
            Some(b) if near - b < 3.0 => Some(b),
            _ => self.walk_height(x, y).filter(|z| *z < near + reach.max(0.5)),
        }
    }

    /// The clock a scenery script starts on: the simulation's, else the run's start.
    pub fn script_clock(&self) -> omsi_sim::SimClock {
        self.timetable_boards.lock().clock.clone().unwrap_or_else(|| self.start_clock.lock().clone())
    }
}
