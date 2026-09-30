//! Where a session starts from: the clock, the depot (`.hof`), a saved situation and writing one.

use super::*;

/// The depot file of a vehicle for this map: `--hof`, else the hof named by the map's
/// `[aigroup_depot]`, else the first .hof next to the .bus file.
pub(crate) fn find_hof(
    args: &Args,
    world: &World,
    vt: &omsi_sim::VehicleType,
) -> Option<std::sync::Arc<omsi_vehicle::Hof>> {
    let dir = vt.def.dir();
    let mut names: Vec<String> = Vec::new();
    if let Some(h) = &args.hof {
        names.push(h.clone());
    }
    names.extend(world.ailists.groups.iter().filter_map(|g| g.hof.clone()));
    // every wanted name in order: the bus's own depot files first (by file name, then by
    // the [name] inside), then the map's depot as another vehicle folder has it (a mod bus
    // brings only the depot of its own map: see `omsi_vehicle::hof::depot_anywhere`)
    for n in &names {
        if let Some(h) = omsi_vehicle::hof::depot_in(dir, n) {
            log::info!("using depot file {} (name {n})", h.path.display());
            return Some(std::sync::Arc::new(h));
        }
    }
    for n in &names {
        if let Some(h) = omsi_vehicle::hof::depot_anywhere(n) {
            log::info!(
                "{} has no depot file '{n}'; using {}",
                dir.display(),
                h.path.display()
            );
            return Some(std::sync::Arc::new(h));
        }
    }
    let p = omsi_vehicle::hof::depot_files(dir).into_iter().next()?;
    log::info!("using depot file {}", p.display());
    omsi_vehicle::Hof::load(&p).ok().map(std::sync::Arc::new)
}

pub(crate) fn start_clock(args: &Args) -> omsi_sim::SimClock {
    let mut c = omsi_sim::SimClock::default();
    if let Some(d) = &args.date {
        let v: Vec<i32> = d.split('-').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() == 3 {
            c.set_date(v[0], v[1], v[2]);
        }
    }
    if let Some(doy) = args.day_of_year {
        c.day_of_year = doy.max(1);
    }
    c.time = parse_time(&args.time);
    c
}

/// Fill the arguments from a situation file (`.osn`): the original's saved game state.
pub(crate) fn apply_situation(args: &mut Args) -> Result<()> {
    let Some(rel) = args.situation.clone() else {
        return Ok(());
    };
    // (an absolute path is the file itself: a situation saved elsewhere, a test's)
    let path = if Path::new(&rel).is_absolute() { PathBuf::from(&rel) } else { omsi_cfg::resolve_path(&args.root, &rel) };
    let sit = omsi_content::Situation::load(&path)
        .map_err(|e| anyhow!("loading {}: {e}", path.display()))?;
    log::info!(
        "situation \"{}\": map {} time {:?} vehicles {}",
        sit.name,
        sit.map,
        sit.time,
        sit.vehicles.len()
    );
    apply_situation_parsed(&sit, args);
    Ok(())
}

pub(crate) fn apply_situation_parsed(sit: &omsi_content::situation::Situation, args: &mut Args) {
    args.map = sit.map.replace('\\', "/");
    // A map from an archive was saved as its whole path (`\Users\…\Archives\x.zip\Maps\
    // Novi Sad\global.cfg`), which then went after the root and the game closed at once:
    // from its `maps/` folder on, as every map is named.
    args.map = content_relative(&args.map, "maps");
    args.date = Some(format!("{}-01-01", sit.time.0));
    args.day_of_year = Some(sit.time.1);
    args.time = format!("{:02}:{:02}:{}", sit.time.2, sit.time.3, sit.time.4 as i32);
    if let Some(w) = &sit.weather {
        args.weather = Some(w.clone());
    }
    args.schedule = true;
    args.no_menu = true;
    // the player's vehicle: [ismyVehicle] or [myvehicle] index
    let mine = sit
        .vehicles
        .iter()
        .position(|v| v.is_my_vehicle)
        .or_else(|| {
            sit.vehicles
                .get(sit.my_vehicle.max(0) as usize)
                .map(|_| sit.my_vehicle as usize)
        });
    // the map's tile grid first: a position is a tile and a place in it, and a
    // `[worldcoordinates]` map's tiles are ~372 m and scaled onto the grid
    let global_path = omsi_cfg::resolve_path(&args.root, &args.map);
    match omsi_map::GlobalCfg::load(&global_path) {
        Ok(g) => omsi_map::configure_grid(&g),
        Err(e) => log::warn!("situation: map {}: {e}", global_path.display()),
    }
    if let Some(i) = mine {
        let v = &sit.vehicles[i];
        args.bus = Some(content_relative(&v.file.replace('\\', "/"), "vehicles"));
        // tile-local x, height, y, then the body's rotation as a Direct3D quaternion
        // (x, y, z, w; y is up): a bus standing level has (0, sin a/2, 0, cos a/2) with a
        // its heading. "Linie 5" (0, 0.976, 0, -0.219) is 205.3 degrees, along the road
        // there; reading the pair as sin/cos of the heading itself turned it by half.
        let (x, y) = omsi_map::tile_local_to_world(v.tile.0, v.tile.1, v.pos[0], v.pos[2]);
        let heading = situation_heading(&v.orientation);
        // (with its height: a bus saved on a bridge stays on it)
        args.spawn = Some(format!("{x},{y},{heading},{}", v.pos[1]));
        if !v.paint.trim().is_empty() {
            args.hof = Some(v.paint.clone());
        }
        if v.timetable.len() >= 2 {
            args.line = Some(v.timetable[0].clone());
            args.tour = Some(v.timetable[1].clone());
            // the third value is the trip of the tour under way (0 = the first)
            if let Some(t) = v.timetable.get(2).and_then(|t| t.trim().parse::<usize>().ok()) {
                args.trip = Some((t + 1).to_string());
            }
        }
        args.situation_vars = v.vars.iter().map(|(n, x)| (n.clone(), *x as f32)).collect();
        // the livery it was driven in: the scheme's index is the `Colorscheme` variable (the
        // bus came back in its default paint - the variable alone repaints nothing)
        if let Some((_, c)) = v.vars.iter().find(|(n, _)| n.eq_ignore_ascii_case("Colorscheme")).filter(|(_, c)| *c >= 0.0) {
            args.paint = Some(format!("{}", *c as i64));
        }
        args.situation_strvars = v.string_vars.clone();
        log::info!(
            "situation: player {} at {:?} line {:?} tour {:?} hof {:?}, {} vars",
            v.file,
            args.spawn,
            args.line,
            args.tour,
            args.hof,
            v.vars.len()
        );
    } else {
        log::warn!("situation: no player vehicle found");
    }
    // the vehicles placed besides the one driven: they stand where they were saved, in the
    // state their variables say (lights, doors, the engine), as OMSI loads them
    args.situation_others = sit
        .vehicles
        .iter()
        .enumerate()
        .filter(|(i, v)| Some(*i) != mine && v.coupled_with.is_none())
        .map(|(_, v)| {
            let (x, y) = omsi_map::tile_local_to_world(v.tile.0, v.tile.1, v.pos[0], v.pos[2]);
            let paint = v
                .vars
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("Colorscheme"))
                .filter(|(_, c)| *c >= 0.0)
                .map(|(_, c)| format!("{}", *c as i64));
            crate::cli::SituationOther {
                bus: content_relative(&v.file.replace('\\', "/"), "vehicles"),
                spawn: format!("{x},{y},{},{}", situation_heading(&v.orientation), v.pos[1]),
                hof: Some(v.paint.clone()).filter(|p| !p.trim().is_empty()),
                paint,
                vars: v.vars.iter().map(|(n, x)| (n.clone(), *x as f32)).collect(),
                strvars: v.string_vars.clone(),
            }
        })
        .collect();
    if !args.situation_others.is_empty() {
        log::info!("situation: {} more vehicle(s) placed", args.situation_others.len());
    }
}

/// Build a `.osn` situation from the running world: map, clock, weather and every vehicle
/// with its script variables (the player's marked as `[ismyVehicle]`). Like OMSI, only
/// the vehicles the player placed are written - never the AI traffic, which a loaded
/// situation brings back by itself (every stock situation holds exactly one vehicle; the
/// AI cars within 900 m written before came back in OMSI as placed, standing vehicles).
pub(crate) fn build_situation(
    args: &Args,
    world: &scene::World,
    clock: &omsi_sim::SimClock,
    weather: Option<&str>,
    player: Option<&Player>,
    placed: &[Player],
    camera: &Camera,
    duty: Option<&crate::schedule::PlayerDuty>,
    name: &str,
) -> omsi_content::situation::Situation {
    use omsi_content::situation::{Situation, SituationVehicle};
    let rel = |p: &std::path::Path| -> String {
        p.strip_prefix(&args.root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('/', "\\")
    };
    let vehicle_record = |v: &omsi_sim::VehicleInstance, id: f64, mine: bool| -> SituationVehicle {
        let (tile, (lx, ly)) = omsi_map::world_to_tile_local(v.position.x, v.position.y);
        // the body's rotation as OMSI writes it: a quaternion (x, y, z, w) about the up axis,
        // then three more numbers (zero for a standing vehicle)
        let h = v.heading.to_radians() / 2.0;
        let orientation = [0.0, h.sin(), 0.0, h.cos(), 0.0, 0.0, 0.0, 0.0, 0.0];
        let vars: Vec<(String, f64)> =
            v.ty.program
                .var_names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    (
                        n.clone(),
                        v.state.vars.get(i).copied().unwrap_or(0.0) as f64,
                    )
                })
                .collect();
        let string_vars: Vec<(String, String)> =
            v.ty.program
                .str_var_names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    (
                        n.clone(),
                        v.state.str_vars.get(i).cloned().unwrap_or_default(),
                    )
                })
                .collect();
        SituationVehicle {
            file: rel(&v.ty.def.path),
            pos: [lx, v.position.z, ly],
            orientation,
            tile,
            id,
            paint: v
                .host
                .hof
                .as_ref()
                .map(|h| h.name.clone())
                .unwrap_or_default(),
            coupled_with: None,
            is_my_vehicle: mine,
            vars,
            string_vars,
            timetable: Vec::new(),
        }
    };
    let mut vehicles = Vec::new();
    let mut my = -1;
    if let Some(p) = player {
        my = 0;
        let mut rec = vehicle_record(&p.vehicle, 1.0, true);
        // the duty: line, tour, the trip under way and its next stop (OMSI writes two more
        // numbers whose meaning is not settled; they are left at 0 and not read back)
        if let Some(d) = duty {
            rec.timetable = vec![
                d.line.clone(),
                d.tour.clone(),
                d.trip_index.to_string(),
                d.next_stop.to_string(),
                "0".into(),
                "0".into(),
            ];
        }
        vehicles.push(rec);
    }
    // and the vehicles placed besides it, standing where they are (only the driven one was
    // written: a session continued with the last one alone, #139)
    for (k, q) in placed.iter().enumerate() {
        vehicles.push(vehicle_record(&q.vehicle, 2.0 + k as f64, false));
    }
    let (cam_tile, cam_local) = omsi_map::world_to_tile_local(camera.position.x, camera.position.y);
    Situation {
        path: Default::default(),
        name: name.to_string(),
        description: format!(
            "Saved by the openOMSI at {:02}:{:02}",
            (clock.time / 3600.0) as i32,
            ((clock.time % 3600.0) / 60.0) as i32
        ),
        map: rel(&world.global.path),
        weather: weather.map(|w| w.to_string()),
        time: (
            clock.year,
            clock.day_of_year,
            (clock.time / 3600.0) as i32,
            ((clock.time % 3600.0) / 60.0) as i32,
            (clock.time % 60.0) as f64,
        ),
        // the map camera is written in the centre tile's frame
        center_tile: cam_tile,
        map_cam: vec![
            cam_local.0,
            camera.position.z,
            cam_local.1,
            camera.yaw as f64,
            camera.pitch as f64,
            25.0,
        ],
        ego_pos: vec![10.0, 0.0, 10.0, 0.0, 0.0],
        timetable_active: args.schedule,
        my_vehicle: my,
        view: 0,
        vehicles,
    }
}

/// The heading (degrees clockwise from north) of a situation vehicle's rotation quaternion
/// (x, y, z, w with y up, as OMSI writes it).
pub(crate) fn situation_heading(q: &[f64; 9]) -> f64 {
    (2.0 * q[1].atan2(q[3]).to_degrees()).rem_euclid(360.0)
}

/// The key `Inputs/keyboard.cfg` gives `ticket_give`, as the key names file spells it.
pub(crate) fn ticket_key_name(root: &Path, bindings: &[omsi_content::KeyBinding]) -> String {
    let Some(b) = bindings
        .iter()
        .find(|b| b.action.eq_ignore_ascii_case("ticket_give"))
    else {
        return "T".into();
    };
    let names =
        omsi_content::input::load_key_names(&root.join("Inputs/ENG.kyb")).unwrap_or_default();
    let key = names
        .iter()
        .find(|(c, _)| *c == b.scan_code)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| format!("key {}", b.scan_code));
    let mut out = String::new();
    for (bit, name) in [(omsi_content::input::KEY_SHIFT, "Shift+"), (omsi_content::input::KEY_CTRL, "Ctrl+"), (omsi_content::input::KEY_ALT, "Alt+")] {
        if b.modifier & bit != 0 {
            out.push_str(name);
        }
    }
    out + &key
}

/// A content path saved whole (a map or bus from an archive or the content folder:
/// `/Users/…/Archives/x.zip/Maps/Novi Sad/global.cfg`) from its `folder` (`maps`,
/// `vehicles`) on, as the game names content; went after the root, it closed the game.
fn content_relative(path: &str, folder: &str) -> String {
    let p = path.replace('\\', "/");
    if !(p.starts_with('/') || p.get(1..2) == Some(":")) {
        return p;
    }
    match p.to_ascii_lowercase().rfind(&format!("/{folder}/")) {
        Some(i) => p[i + 1..].to_string(),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn situation_others_preserve_their_colorscheme() {
        let sit = omsi_content::situation::Situation {
            map: "maps/Berlin/global.cfg".into(),
            vehicles: vec![
                omsi_content::situation::SituationVehicle {
                    file: "Vehicles/MAN_SD200/MAN_SD77.bus".into(),
                    is_my_vehicle: true,
                    vars: vec![("Colorscheme".into(), 1.0)],
                    ..Default::default()
                },
                omsi_content::situation::SituationVehicle {
                    file: "Vehicles/MAN_NL_NG/MAN_EN92.bus".into(),
                    is_my_vehicle: false,
                    vars: vec![("Colorscheme".into(), 4.0)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut args = crate::cli::Args::parse_from(["openomsi"]);
        apply_situation_parsed(&sit, &mut args);
        assert_eq!(args.paint.as_deref(), Some("1"));
        assert_eq!(args.situation_others.len(), 1);
        assert_eq!(args.situation_others[0].paint.as_deref(), Some("4"));
    }
}
