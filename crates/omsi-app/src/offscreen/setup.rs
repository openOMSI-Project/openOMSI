//! The offscreen run's start: the renderer, the world, the traffic, the timetable, the
//! player's bus and duty, the people, the weather and the run's settings.

use super::*;
use crate::view_sync::people::PeopleView;

impl<'a> Offscreen<'a> {
    pub(super) fn setup(
        args: &'a Args,
        out: &'a PathBuf,
        mut lan_off: Option<omsi_net::LanSession>,
        mut remotes_off: lan::LanGame,
    ) -> Result<Self> {
        let (w, h) = args
            .size
            .split_once('x')
            .map(|(a, b)| {
                (
                    a.parse::<u32>().unwrap_or(1600),
                    b.parse::<u32>().unwrap_or(900),
                )
            })
            .unwrap_or((1600, 900));
        let view_aspect = w as f32 / h.max(1) as f32;
        let settings = settings::Settings::load();
        crate::rain::set_quality(&settings.rain_quality);
        let instance = graphics_instance();
        let mut renderer = pollster::block_on(Renderer::new_with(
            &instance,
            None,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            settings.render_options(),
        ))?;
        log::info!("adapter: {}", renderer.adapter_name);
        crate::lights::load_smoke_texture(&mut renderer, &args.root);
        crate::lights::set_corona_root(&args.root);
        let mut scene = renderer.new_scene();
        let (world, camera) = lan::answering_while(&mut lan_off, args.bus.as_deref(), || load_world(args, &renderer, &mut scene))?;
        // the map's own route arrows, with OMSI 2's route arrows
        world.show_help_arrows(&renderer, &mut scene, settings.nav_arrows);
        let lan_seed = lan_off.as_ref().map(lan::population_seed);
        // (a player who joins another's game draws the host's traffic in it, whatever their own
        // count says: without it the host's cars had nowhere to go - "passengers, but no
        // traffic" on a server)
        // (and without traffic it still runs the light programs and switches the lamps)
        let mut traffic = new_traffic(args, &world, &renderer, &mut scene, lan_seed)?;
        // what the renderer shows of the traffic and the people (see `view_sync`)
        let mut sim_view = crate::view_sync::SimView::default();
        let mut schedule = if args.schedule {
            Some(schedule::Schedule::new(
                &args.root,
                &world,
                &start_clock(args),
            ))
        } else {
            None
        };
        if let Some(s) = schedule.as_mut() {
            lan::answering_while(&mut lan_off, args.bus.as_deref(), || {
                s.precache(
                    &world,
                    &renderer,
                    &mut scene,
                    traffic.as_mut(),
                    parse_time(&args.time),
                )
            });
            if let (Some(t), true) = (traffic.as_mut(), omsi_cfg::flags::OMSI_CHECK_TRIPS.is_set()) {
                s.check_routes(&world, t);
            }
        }
        let mut player = spawn_player(args, &world, &renderer, &mut scene)?;
        let spawn_z = player.as_ref().map(|p| p.vehicle.position.z).unwrap_or(0.0);
        if let Some(p) = player.as_mut() {
            p.vehicle.host.auto_clutch = if settings.auto_clutch { 1.0 } else { 0.0 };
            // OMSI_PAX_CAM=n: `--view pax` from the bus's n-th passenger camera
            if let Some(k) = omsi_cfg::flags::OMSI_PAX_CAM.parse() {
                p.cam_choice.1 = k;
            }
        }
        let center = player
            .as_ref()
            .map(|p| p.vehicle.position)
            .unwrap_or(camera.position);
        let (duty, duty_error) = player_duty(args, &world, schedule.as_mut(), player.as_mut());
        if let Some(p) = player.as_mut() {
            let active = if duty.is_some() { 1.0 } else { 0.0 };
            p.vehicle.host.schedule_active = active;
            p.vehicle.set_var("schedule_active", active);
        }
        let journey = None;
        let career = args
            .driver
            .as_deref()
            .map(|d| career::Career::load(&args.root, d))
            .unwrap_or_default();
        let humans_off = new_humans(args, &settings, &world, &renderer, &mut scene, &mut sim_view.people, schedule.as_ref(), player.as_mut(), lan_seed, center);
        let player_ref: Option<Player> = None;
        let envir = omsi_content::Envir::load(&args.root.join("envir.cfg")).ok();
        let weather = load_weather(args);
        setup_sky(args, &renderer, &mut scene, envir.as_ref(), Some(&weather));
        if let Some(p) = player.as_mut() {
            apply_weather(&mut p.vehicle, &weather, initial_wetness(&weather));
        }
        // the workshop's waiting time moves the clock on, so the sky has to follow it
        let mut service_seconds = 0.0f64;
        let daylight0 = omsi_sim::Daylight::compute(&start_clock(args), envir.as_ref());
        if let Some(p) = player.as_mut() {
            p.vehicle.set_var("Envir_Brightness", daylight0.envir_brightness(world.light_map_light_at(p.vehicle.position)));
            let mut clock = p.vehicle.host.clock.clone();
            let was = clock.time;
            let at_station = at_petrol_station(&world, &p.vehicle);
            for line in run_services(
                args,
                &mut p.vehicle,
                &mut clock,
                world.global.repair_time_min,
                at_station,
            ) {
                log::info!("{line}");
            }
            service_seconds = clock.time - was;
            p.vehicle.host.clock = clock;
        }
        // one simulation loop for everything: traffic, the player's vehicle (test profile and
        // timed triggers) and the passengers, so that they see each other every frame
        let dt = 1.0 / 30.0;
        let drive_frames = args.drive.map(|s| (s / dt) as usize).unwrap_or(0);
        let wheel_worst: Option<(DVec3, f64)> = None;
        let total_frames = drive_frames
            .max(if humans_off.is_some() { 30 } else { 0 })
            .max(if traffic.is_some() { 1 } else { 0 });
        // a dedicated server runs until it is told to stop (SIGTERM, Ctrl+C)
        let server = args.server.is_some();
        let total_frames = if server { usize::MAX } else { total_frames };
        if server {
            quit::install(|_| {});
            log::info!("server: running; Ctrl+C or SIGTERM stops it");
        }
        let timed: Vec<(String, f32)> = parse_triggers(args)
            .into_iter()
            .filter(|(_, t)| *t > 0.0)
            .collect();
        let mut snapshot_times: Vec<f32> = args
            .snapshots
            .as_deref()
            .unwrap_or("")
            .split(',')
            .filter_map(|v| v.trim().parse::<f32>().ok())
            .collect();
        snapshot_times.sort_by(|a, b| a.total_cmp(b));
        let drive_start = player
            .as_ref()
            .map(|p| p.vehicle.position)
            .unwrap_or(DVec3::ZERO);
        // test harness for crashes, kerbs and reversing: OMSI_DRIVE_PROFILE="t throttle brake
        // [steer]/ …" holds piecewise constant pedals from each t on, OMSI_DRIVE_V0 gives the
        // bus a speed (km/h) on the first frame, OMSI_DEBUG_PHYSICS=secs logs the pose
        let drive_profile: Vec<[f32; 4]> = omsi_cfg::flags::OMSI_DRIVE_PROFILE.var()
            .unwrap_or_default()
            .split(['/', ';'])
            .filter_map(|s| {
                let v: Vec<f32> = s
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                (v.len() >= 3).then(|| [v[0], v[1], v[2], v.get(3).copied().unwrap_or(0.0)])
            })
            .collect();
        let drive_v0: Option<f32> = omsi_cfg::flags::OMSI_DRIVE_V0.parse();
        let physics_log: f32 = omsi_cfg::flags::OMSI_DEBUG_PHYSICS.parse()
            .unwrap_or(0.0);
        let last_reasons: Vec<String> = Vec::new();
        if args.autostart && !args.is_resuming() {
            if let Some(p) = player.as_mut() {
                log::info!("{}", p.start_up());
                if let Some(d) = duty.as_ref() {
                    // typed into the IBIS when the start-up has the electrics on
                    let (trip, stop) = d.trip_for_ibis();
                    p.set_duty_destination(trip, stop);
                }
            }
        }
        if let Some(t) = traffic.as_mut() {
            t.day_time = parse_time(&args.time);
            let daylight = omsi_sim::Daylight::compute(
                &start_clock(args),
                omsi_content::Envir::load(&args.root.join("envir.cfg"))
                    .ok()
                    .as_ref(),
            );
            steps::set_ai_daylight(t, daylight, steps::gloomy_weather(Some(&weather)));
            t.populate(&mut sim_view.traffic, &world, &renderer, &mut scene, center);
        }
        ground_sample(&world, traffic.as_ref(), center)?;
        // LAN in an offscreen run too, so that one game's view of another can be rendered
        // (`OMSI_LAN_AUDIO=1`: with the other buses' sounds, heard at the camera - for the logs)
        let lan_audio = (lan_off.is_some() && omsi_cfg::flags::OMSI_LAN_AUDIO.is_set())
            .then(omsi_audio::AudioEngine::new);
        if let (Some(l), Some(p)) = (lan_off.as_mut(), player.as_mut()) {
            lan::settle_spawn(
                l,
                &mut remotes_off,
                p,
                args,
                &world,
                traffic.as_ref().map(|t| &t.net),
                lan::WELCOME_WAIT,
            );
        }
        let mut run_clock = start_clock(args);
        run_clock.advance(service_seconds as f32);
        // a dedicated server's administration and clock (see `admin`)
        let mut srv_admin = crate::admin::ServerAdmin::default();
        let srv_clock = 0.0f64;
        // the METAR sync of a dedicated server: the report is downloaded in the background (at
        // once, then every ten minutes) and its values are told to the players
        let srv_metar: Option<String> = if server { crate::server::SERVER_METAR.get().cloned().flatten() } else { None };
        let srv_metar_due = std::time::Instant::now();
        let srv_metar_rx: Option<std::sync::mpsc::Receiver<Option<omsi_content::weather::Weather>>> = None;
        // (the weather's name on the status page)
        let srv_weather_name = if weather.path.to_string_lossy().starts_with("metar:") {
            weather.name.clone()
        } else {
            weather.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        };
        if let (true, Some((pw, speed))) = (server, crate::server::SERVER_ADMIN.get()) {
            srv_admin.password = pw.clone();
            if let Some(l) = lan_off.as_mut() {
                l.clock_speed = *speed;
            }
        }
        let ground_gap = crate::ground_gap::GroundGap::from_env();
        if let Some(t) = traffic.as_ref() {
            crate::ground_gap::check_lanes(&world, t);
        }
        // the tyres' spray (see `puddles`), from roads as wet as the weather left them
        let spray = puddles::Spray::new();
        let wetness = initial_wetness(&weather);
        let cabin_air = crate::condensation::CabinAir::new();
        let real_time = RealTime::default();
        let recorder = record::Recorder::new(out, player.as_mut(), &args.root);
        Ok(Offscreen {
            args,
            out,
            w,
            h,
            view_aspect,
            settings,
            renderer,
            scene,
            world,
            camera,
            traffic,
            schedule,
            player,
            spawn_z,
            center,
            duty,
            duty_error,
            journey,
            career,
            humans_off,
            sim_view,
            player_ref,
            envir,
            weather,
            service_seconds,
            dt,
            drive_frames,
            wheel_worst,
            total_frames,
            server,
            timed,
            snapshot_times,
            drive_start,
            drive_profile,
            drive_v0,
            physics_log,
            last_reasons,
            lan_audio,
            lan_off,
            remotes_off,
            run_clock,
            wetness,
            cabin_air,
            srv_admin,
            srv_clock,
            srv_metar,
            srv_metar_due,
            srv_metar_rx,
            srv_weather_name,
            ground_gap,
            spray,
            real_time,
            recorder,
        })
    }
}

/// The AI traffic (with no cars asked for, the light programs and the lamps only).
fn new_traffic(
    args: &Args,
    world: &World,
    renderer: &Renderer,
    scene: &mut Scene,
    lan_seed: Option<u64>,
) -> Result<Option<traffic::Traffic>> {
    let mut t = traffic::Traffic::new(&args.root, world, args.traffic)?;
    t.lights_only = !(args.traffic > 0 || args.schedule || crate::rail_drive::args_rail(args) || args.lan_join.is_some());
    t.no_timetable_buses = args.no_timetable_buses;
    if let Some(seed) = lan_seed {
        t.set_lan_seed(seed);
    }
    if args.traffic > 0 {
        t.precache_random(world, renderer, scene);
    }
    Ok(Some(t))
}

/// The player's duty (`--line`, `--tour`, `--trip`), or why there is none.
fn player_duty(
    args: &Args,
    world: &World,
    mut schedule: Option<&mut schedule::Schedule>,
    mut player: Option<&mut Player>,
) -> (Option<schedule::PlayerDuty>, Option<String>) {
    let mut duty: Option<schedule::PlayerDuty> = None;
    // why the duty asked for cannot be driven (the picture's HUD says it too)
    let mut duty_error: Option<String> = None;
    if let (Some(sch), Some(line), Some(p)) = (schedule.as_mut(), &args.line, player.as_mut()) {
        match sch.player_duty(
            world,
            line,
            args.tour.as_deref().unwrap_or(""),
            parse_time(&args.time),
            args.trip.as_deref(),
            args.whole_tour,
        ) {
            Ok(mut d) => {
                if let Some(k) = args.duty_trip {
                    d.start_at(k, args.duty_first_stop);
                }
                if args.is_resuming() {
                    d.resume(&mut p.vehicle, parse_time(&args.time), args.situation_next_stop);
                    // (as in the window: see `App`)
                    p.duty_typed = args.autostart;
                } else {
                    d.update(&mut p.vehicle, parse_time(&args.time));
                }
                let mut fonts = world.fonts.lock();
                if let Err(e) = crate::schedule_paper::update_vehicle(
                    &mut p.vehicle,
                    &d,
                    &mut fonts,
                ) {
                    log::warn!("driver timetable paper: {e:#}");
                }
                log::info!(
                    "duty: line {} tour {} trip {} next stop {} ({}) delay {:.0} s, stops {:?}",
                    d.line,
                    d.tour,
                    d.trips[d.trip_index].name,
                    d.next_stop,
                    p.vehicle
                        .host
                        .tt_stops
                        .get(d.next_stop)
                        .map(|s| s.0.clone())
                        .unwrap_or_default(),
                    p.vehicle.host.tt_delay,
                    p.vehicle
                        .host
                        .tt_stops
                        .iter()
                        .map(|s| format!(
                            "{} {:02}:{:02}",
                            s.0,
                            (s.2 / 3600.0) as i32,
                            ((s.2 % 3600.0) / 60.0) as i32
                        ))
                        .collect::<Vec<_>>()
                );
                duty = Some(d);
            }
            Err(e) => {
                log::warn!("no player duty: {e}");
                duty_error = Some(e);
            }
        }
    }
    (duty, duty_error)
}

/// The people at the stops and in the bus (`--passengers`, and always in a LAN game).
#[allow(clippy::too_many_arguments)]
fn new_humans(
    args: &Args,
    settings: &settings::Settings,
    world: &World,
    renderer: &Renderer,
    scene: &mut Scene,
    view: &mut PeopleView,
    schedule: Option<&schedule::Schedule>,
    mut player: Option<&mut Player>,
    lan_seed: Option<u64>,
    center: DVec3,
) -> Option<humans::Humans> {
    if args.passengers || args.lan_join.is_some() {
        let mut h = humans::Humans::new(&args.root, view);
        if let Some(seed) = lan_seed {
            h.set_lan_seed(seed);
        }
        // a dedicated server plays nowhere itself: its people are the LAN players' alone (at
        // the map's camera they filled the whole pool, and the players met nobody)
        h.players_only = args.server.is_some() && player.is_none();
        h.exact_fare = settings.exact_fare;
        h.boarding = settings.boarding.clone();
        h.stand_chance = settings.standing_chance;
        h.voices = match settings.pax_voices.as_str() { "off" => 2, "tickets" => 1, _ => 0 };
        if let Some(p) = player.as_mut() {
            h.set_cabin(&mut p.vehicle);
            h.ticket_key = ticket_key_name(&args.root, &p.bindings);
            h.tickets = p.vehicle.host.tickets.clone();
            if !world.global.money_system.trim().is_empty() {
                h.money = Some(money::Money::new(&args.root, &world.global.money_system));
            }
        }
        // (with the passengers setting, as in the window)
        h.density = world
            .global
            .passenger_density((parse_time(&args.time) / 3600.0) as f32)
            * settings.pax_density;
        h.time_of_day = parse_time(&args.time);
        h.stop_targets = schedule.as_ref().map(|s| s.stop_targets());
        h.stop_names = schedule.as_ref().map(|s| s.stop_names());
        // (the trips due at the stops soon, as in the window: #1415)
        h.due_dests = schedule.as_ref().map(|s| s.due_destinations(parse_time(&args.time)));
        h.populate(view, world, renderer, scene, center);
        if let Some(p) = player.as_ref() {
            if args.riders > 0 {
                h.seed_riders(view, args.riders, &p.vehicle, world, renderer, scene);
            }
        }
        Some(h)
    } else {
        None
    }
}

    // OMSI_GROUND_SAMPLE=<csv>: what the wheels stand on every metre along the street lanes
    // within 400 m of the start (lane, s, x, y, lane z, ground z, the id of an object whose
    // collision mesh stands in the way there) - two builds compared on
    // the same map show where a change of the ground rules adds or removes a bump
fn ground_sample(world: &World, traffic: Option<&traffic::Traffic>, center: DVec3) -> Result<()> {
    if let (Some(path), Some(t)) = (omsi_cfg::flags::OMSI_GROUND_SAMPLE.var(), traffic.as_ref()) {
        use std::io::Write;
        let Ok(mut f) = std::fs::File::create(&path) else { return Err(anyhow::anyhow!("OMSI_GROUND_SAMPLE: cannot write {path}")) };
        let collision = world.collision.lock().clone();
        for (li, l) in t.net.lanes.iter().enumerate() {
            if l.kind != omsi_sim::traffic::LaneKind::Street { continue; }
            let len = l.length();
            let mut s = 0.0f32;
            while s < len {
                let (p, _) = l.at(s);
                if (p.truncate() - center.truncate()).length() < 400.0 {
                    let g = crate::scene::drive_probe(&world.terrains, &world.surfaces, p.x, p.y, p.z + 0.5);
                    // and a wall there: a 2 m box from 0.3 m to 3 m over the ground, against
                    // the objects' collision meshes (the id of the first one it touches)
                    let base = g.below.unwrap_or(p.z);
                    let mut probe = omsi_sim::collision::Obb::from_box([2.0, 2.0, 2.7, 0.0, 0.0, 0.0], DVec3::new(p.x, p.y, base), 0.0);
                    probe.z0 = base + 0.3;
                    probe.z1 = base + 3.0;
                    let wall = collision.meshes.iter().find(|m| m.parts_near(&probe, None).next().is_some()).map(|m| m.id);
                    // and beside the lane, where a bus's wheels run (1.1 m) and a lane over
                    // (2.5 m): a ground wider than the road shows there
                    let (q, _) = l.at((s + 0.5).min(len));
                    let dir = (q - p).truncate().normalize_or_zero();
                    let side: Vec<String> = [-2.5, -1.1, 1.1, 2.5]
                        .iter()
                        .map(|&d| {
                            let w = p.truncate() + glam::DVec2::new(dir.y, -dir.x) * d;
                            crate::scene::drive_probe(&world.terrains, &world.surfaces, w.x, w.y, p.z + 0.5).below.map(|z| format!("{z:.4}")).unwrap_or_default()
                        })
                        .collect();
                    let _ = writeln!(f, "{li},{s:.1},{:.2},{:.2},{:.3},{},{},{}", p.x, p.y, p.z, g.below.map(|z| format!("{z:.4}")).unwrap_or_default(), wall.map(|w| w.to_string()).unwrap_or_default(), side.join(","));
                }
                s += 1.0;
            }
        }
    }
    Ok(())
}
