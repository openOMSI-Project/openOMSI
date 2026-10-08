//! The steps of a frame that the window (`App::frame`) and the offscreen run
//! (`run_offscreen`) both take. Each one takes what it works on rather than the `App`, so
//! that the offscreen run can call it with its own locals. The offscreen run takes them as
//! the window does (its clock goes on, the settings count); the parameters left say what
//! only one of them has (a pause, LAN players, plugins).

use super::*;
use crate::view_sync::people::PeopleView;
use crate::view_sync::traffic::TrafficView;

/// The vehicles the AI traffic must see besides its own: the other LAN players' buses, the
/// player's own and the ones the player placed - and, unless the game is paused, the
/// traffic's step itself with the player's priority, blinker, switches and signals.
/// `player_rail`: the player's place on a rail line for the signals.
#[allow(clippy::too_many_arguments)]
pub(crate) fn traffic_tick(
    t: &mut traffic::Traffic,
    view: &mut TrafficView,
    world: &World,
    dt: f32,
    paused: bool,
    player: Option<&Player>,
    remotes: &lan::LanGame,
    placed: &[Player],
    player_rail: Option<(usize, bool)>,
) {
    t.others = lan_outlines(remotes);
    t.others.extend(own_outlines(player, placed));
    t.other_blinkers = outline_indicators(remotes, player, placed);
    if !paused {
        t.player_priority = player.and_then(|p| p.vehicle.var("TrafficPriority")).is_some_and(|v| v > 0.5);
        t.player_blinker = player.map(|p| lan::indicator(&p.vehicle)).unwrap_or(0);
        t.tick(view, dt, player.map(|p| player_outline(p)));
        world.set_switches(&t.switch_requests());
        world.set_signals(&t.signal_aspects(&world.signal_routes, player_rail));
    }
}

/// The AI vehicles round the player's bus that it collides with (none with `collide` off:
/// the options' [no_collision_vehToVeh], the bus drives through the traffic).
pub(crate) fn traffic_boxes(t: &traffic::Traffic, player: Option<&mut Player>, collide: bool) {
    if let Some(p) = player {
        p.vehicle.dynamic_boxes = if collide { t.boxes(p.vehicle.position, 80.0) } else { Vec::new() };
    }
}

/// The player's hits on moving AI vehicles in its last step, handed to those cars (their
/// recoil, their collision scripts, their dents).
pub(crate) fn deliver_player_impacts(p: &mut Player, traffic: Option<&mut traffic::Traffic>) {
    let impacts = p.vehicle.take_dynamic_impacts();
    if let (false, Some(t)) = (impacts.is_empty(), traffic) {
        t.player_impacts(impacts);
    }
}

/// Rain, snow, fog or a closed cloud cover: the AI drives with its lights on by day.
pub(crate) fn gloomy_weather(weather: Option<&omsi_content::weather::Weather>) -> bool {
    weather.map(|w| {
        let (kind, rate) = precip_of(w);
        w.fog.0 < 600.0 || (kind != 0 && rate > 0.05) || w.clouds.0.trim().to_ascii_lowercase().starts_with("overcast")
    }).unwrap_or(false)
}

/// The AI's lights (and a bus's saloon lamps, which its scripts switch with them): Omsi
/// switches them on below a light value of 0.75, before the street lamps (0.6), and off
/// after them in the morning - and by day when it is `gloomy`.
pub(crate) fn set_ai_daylight(t: &mut traffic::Traffic, daylight: omsi_sim::Daylight, gloomy: bool) {
    t.night = daylight.brightness < 0.75 || gloomy;
    t.daylight = Some(daylight);
}

/// No timetable vehicle is put into the player's bus or a LAN player's.
pub(crate) fn set_keep_clear(t: &mut traffic::Traffic, player: Option<&Player>, remotes: &lan::LanGame) {
    t.keep_clear = player
        .map(|p| traffic::vehicle_bodies(&p.vehicle))
        .unwrap_or_default();
    t.keep_clear.extend(
        remotes
            .remotes
            .values()
            .flat_map(|r| traffic::vehicle_bodies(r.vehicle())),
    );
}

/// The timetable puts out its departures: at the start the trips that left within the last
/// 20 minutes (`first`), then those of the next moments.
pub(crate) fn schedule_tick(
    s: &mut schedule::Schedule,
    world: &World,
    t: &mut traffic::Traffic,
    view: &mut TrafficView,
    r: &Renderer,
    scene: &mut Scene,
    first: bool,
) {
    let window = if first { 20.0 * 60.0 } else { 2.5 };
    s.tick(world, t, view, r, scene, t.day_time, window);
}

/// The street lamps (`lamps`: switched to the daylight's state) and the lit windows of the
/// houses by their [NightMapMode] timetable (`night_modes`).
pub(crate) fn world_lamps(
    w: &World,
    r: &Renderer,
    scene: &mut Scene,
    clock: &omsi_sim::SimClock,
    daylight: &omsi_sim::Daylight,
    lamps: bool,
    night_modes: bool,
) {
    if lamps {
        w.set_lamps(r, scene, daylight.lamps_on);
    }
    if night_modes {
        w.update_night_modes(r, scene, clock, daylight.brightness);
    }
}

/// How many people the map wants about at this time of day (`time`, seconds), with the
/// passengers setting (OMSI's `AIPassFactor`, in per cent), and how late the player's duty
/// runs (the passengers waiting at its stops grow impatient).
pub(crate) fn humans_by_hour(
    h: &mut humans::Humans,
    world: &World,
    time: f64,
    pax_density: f32,
    duty: Option<&schedule::PlayerDuty>,
) {
    h.density = world
        .global
        .passenger_density((time / 3600.0) as f32)
        * pax_density;
    h.time_of_day = time;
    h.delay = duty.map(|d| d.delay(time)).unwrap_or(0.0);
}

/// The trips due at the stops soon, the ones the people turning up there wait for: made
/// anew once a game minute, on the traffic's clock (#1415).
pub(crate) fn humans_due_trips(
    h: &mut humans::Humans,
    schedule: Option<&schedule::Schedule>,
    traffic: Option<&traffic::Traffic>,
) {
    if let (Some(s), Some(t)) = (schedule, traffic) {
        h.keep_due_dests(t.day_time, |at| s.due_destinations(at));
    }
}

/// What the bus's scripts are told of the surroundings: the light around it, the sun's
/// height and the weather (`wetness`: the roads').
pub(crate) fn tell_surroundings(
    p: &mut Player,
    world: Option<&World>,
    daylight: &omsi_sim::Daylight,
    weather: Option<&omsi_content::weather::Weather>,
    wetness: f32,
) {
    let lm = world.and_then(|w| w.light_map_light_at(p.vehicle.position));
    p.vehicle.set_var("Envir_Brightness", daylight.envir_brightness(lm));
    p.vehicle.host.sun_alt = daylight.altitude_deg;
    if let Some(w) = weather {
        apply_weather(&mut p.vehicle, w, wetness);
    }
}

/// The people's counts in the personnel file and the pedestrians the bus knocked down
/// (none with `collide` off: the options' [no_collision_pedastrians]).
pub(crate) fn people_in_career(career: &mut career::Career, h: &mut humans::Humans, p: &Player, collide: bool) -> u32 {
    career_from_humans(career, h);
    let hurt = if collide { h.run_over(&p.vehicle) } else { 0 };
    if hurt > 0 {
        career.crashes[1] += hurt as i32;
    }
    hurt
}

/// The bus's own variables of the people aboard and the duty, the personnel file's step
/// (`tick`: unless paused or the session was written) and a crash in it: its energy (J,
/// 0: none).
pub(crate) fn career_step(career: &mut career::Career, p: &mut Player, riders: usize, on_duty: bool, dt: f32, tick: bool) -> f32 {
    // the engine's own variables of the bus (see `update_engine_vars`)
    p.vehicle.host.humans_count = riders as f32;
    p.vehicle.host.schedule_active = if on_duty { 1.0 } else { 0.0 };
    let crash = std::mem::take(&mut p.vehicle.last_crash);
    if tick {
        career.tick(dt, &p.vehicle, riders);
    }
    if crash > 0.0 {
        career.crashed(crash, p.vehicle.physics.velocity_kmh() / 3.6);
    }
    crash
}

/// The cabin air of the player's bus and the condensation on its glass, `dt` on.
pub(crate) fn cabin_air_step(
    cabin: &mut crate::condensation::CabinAir,
    dt: f32,
    p: &Player,
    weather: &omsi_content::weather::Weather,
    humans: Option<&humans::Humans>,
) {
    let (riders, doors) = humans
        .map(|h| (h.riding(), crate::condensation::open_doors(&h.cabin_doors(crate::humans::BusId::Player))))
        .unwrap_or((0, 0));
    let ci = crate::condensation::inputs_for(&p.vehicle, weather, riders, doors);
    cabin.step(dt, &ci);
}

/// The [wind] at the height of the tyres' spray: direction (deg) and speed (m/s) in the lee
/// of the street.
pub(crate) fn spray_wind(weather: &omsi_content::weather::Weather) -> Vec3 {
    Vec3::new(weather.wind.0.to_radians().sin() * weather.wind.1, weather.wind.0.to_radians().cos() * weather.wind.1, 0.0)
        * puddles::GROUND_WIND
}

/// The people's step, and what it means for the buses: the validators used (the bus's
/// `ev_Stamper` sound), who wants to get off or on at the stops, the boarding the AI buses
/// wait for. Whether the player's bus took a ticket (see `Humans::tick`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_humans(
    h: &mut humans::Humans,
    view: &mut PeopleView,
    dt: f32,
    world: &World,
    mut player: Option<&mut Player>,
    traffic: Option<&mut traffic::Traffic>,
    r: &Renderer,
    scene: &mut Scene,
) -> bool {
    let mut traffic = traffic;
    let took = h.tick(
        view,
        dt,
        world,
        player.as_deref().map(|p| &p.vehicle),
        traffic.as_deref(),
        r,
        scene,
    );
    // validators used: the bus's `ev_Stamper` sound
    for bus in h.take_stamped() {
        match bus {
            None => {
                if let Some(p) = player.as_deref_mut() {
                    p.vehicle.host.fired_triggers.push("ev_Stamper".into());
                }
            }
            Some(id) => {
                if let Some(c) = traffic.as_deref_mut().and_then(|t| t.cars.iter_mut().find(|c| c.id == id)) {
                    c.vehicle.host.fired_triggers.push("ev_Stamper".into());
                }
            }
        }
    }
    if let Some(t) = traffic {
        let (alighting, waiting) = h.stop_wishes();
        t.set_stop_wishes(alighting, waiting);
        for (id, stop, secs) in h.take_holds() {
            t.hold_boarding(id, stop, secs);
        }
        for (id, doors) in h.take_ai_requests() {
            t.set_pax_requests(id, &doors);
        }
    }
    took
}

/// The people's counts in the personnel file.
pub(crate) fn career_from_humans(career: &mut career::Career, h: &humans::Humans) {
    career.tickets = (h.tickets_sold as i32, h.ticket_cash as f64);
    career.boarded = h.boarded as i32;
    career.served = h.served as i32;
    career.stepped_in = h.stepped_in as i32;
    career.content = h.content as i32;
    career.ticket_requests = h.ticket_requests as i32;
    career.ticket_points = h.ticket_points as i32;
}

/// The player's duty at `time`: a stop picked on the bus's HTML display, the stops served
/// (the personnel file, the journey's log), a change of trip typed into the IBIS, the
/// driver's timetable paper.
/// `game_clock`: the clock the journey's head line is written with (none: the bus's own).
/// `learn_loaded`: the stops of the tiles loaded are learnt first. `plugin_events`: where
/// skipped stops and the end of a trip are told to the plugins (none: a trip's end is
/// dropped, the skipped stops are kept).
#[allow(clippy::too_many_arguments)]
pub(crate) fn duty_step(
    d: &mut schedule::PlayerDuty,
    p: &mut Player,
    world: &World,
    career: &mut career::Career,
    journey: &mut Option<crate::journey::Journey>,
    root: &Path,
    time: f64,
    game_clock: Option<&omsi_sim::SimClock>,
    learn_loaded: bool,
    plugin_events: Option<&mut Vec<omsi_plugin::GameEvent>>,
) {
    if let Some(stop) = p.html_next_stop.take() {
        if d.skip_to(stop) {
            let (trip, k) = d.trip_for_ibis();
            p.ibis_to_stop(trip, k);
        }
    }
    if learn_loaded {
        d.learn_loaded(&world.object_positions.lock());
    }
    let due = (d.trip_index, d.next_stop);
    let served = d.update(&mut p.vehicle, time);
    // (also one ended between frames: the last stop skipped from the menu; a trip reopened
    // by a page counts from its start again, and its next end is told too)
    if let Some(run) = d.take_reopened() {
        career.trip_reopened(run);
    }
    let ended = d.take_finished();
    if let Some((arrival, departure)) = served {
        career.stop_served(arrival, departure);
    }
    {
        let clock = game_clock.unwrap_or(&p.vehicle.host.clock);
        let career = &*career;
        crate::journey::note(journey, d, due, served, root, || crate::journey::head(career, &world.global.name, &p.vehicle, clock));
    }
    if let Some(events) = plugin_events {
        use omsi_plugin::InfoValue::{Num, Text};
        if let Some((count, due_at, at)) = d.take_skipped() {
            crate::plugins::queue_event(events, "stops_skipped", vec![Num(count as f64), Num(due_at as f64), Num(at as f64)]);
        }
        // the trip over, rated before the next one starts its ratings afresh (none for a
        // trip the bus was never driven on: a duty taken at its last stop)
        if let Some(f) = ended {
            if let Some([driving, comfort, tickets]) = career.trip_ended(f.run) {
                crate::plugins::queue_event(events, "trip_done", vec![Num(f.index as f64 + 1.0), Text(f.how.as_str().into()), Num(driving), Num(comfort), Num(tickets)]);
            }
        }
    }
    career.trip_driven(d.trip_run());
    if d.take_trip_change() && p.duty_typed {
        let (trip, stop) = d.trip_for_ibis();
        p.set_duty_destination(trip, stop);
    }
    let mut fonts = world.fonts.lock();
    if let Err(e) = crate::schedule_paper::update_vehicle(
        &mut p.vehicle,
        d,
        &mut fonts,
    ) {
        log::warn!("driver timetable paper: {e:#}");
    }
}

/// The tyres' spray (see `puddles`): what every vehicle's tyres throw up from the water on
/// the road, `wetness` as wet as the picture draws it, the air moving with `wind`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn throw_spray(
    spray: &mut puddles::Spray,
    dt: f32,
    player: Option<&Player>,
    traffic: Option<&traffic::Traffic>,
    remotes: &lan::LanGame,
    eye: DVec3,
    wind: Vec3,
    world: &World,
    wetness: f32,
) {
    let mut vehicles: Vec<(u64, &omsi_sim::VehicleInstance)> = Vec::new();
    if let Some(p) = player {
        vehicles.push((0, &p.vehicle));
    }
    if let Some(t) = traffic {
        vehicles.extend(t.cars.iter().map(|c| (c.id.wrapping_add(1), &c.vehicle)));
    }
    vehicles.extend(remotes.remotes.iter().map(|(id, r)| (puddles::REMOTE_KEY | *id as u64, r.vehicle())));
    spray.frame(dt, &vehicles, eye, wind, &|x, y| puddles::water_at(x, y, world.wet_road_at(x, y, wetness)));
}

/// The vehicles whose own lights shine (`lights::collect`): the player's, the AI's and the
/// other LAN players'.
pub(crate) fn light_vehicles<'a>(
    player: Option<&'a Player>,
    traffic: Option<&'a traffic::Traffic>,
    remotes: &'a lan::LanGame,
) -> Vec<&'a omsi_sim::VehicleInstance> {
    let mut vehicles: Vec<&omsi_sim::VehicleInstance> = Vec::new();
    if let Some(p) = player {
        vehicles.push(&p.vehicle);
    }
    if let Some(t) = traffic {
        vehicles.extend(t.cars.iter().map(|c| &c.vehicle));
    }
    vehicles.extend(remotes.remotes.values().map(|r| r.vehicle()));
    vehicles
}

/// The lighting a picture is drawn with: the weather's (`cloud_drift`, the roads' `wetness`;
/// `OMSI_WETNESS` in its place), dressed for the bus the camera may be in (`inside`) and the
/// player's (`driven`: the wind on its glass, `condensation` on it). `animation_time`: the
/// simulation's seconds the renderer animates the scene by (the rain on the glass).
#[allow(clippy::too_many_arguments)]
pub(crate) fn picture_lighting(
    daylight: &omsi_sim::Daylight,
    weather: Option<&omsi_content::weather::Weather>,
    cloud_drift: [f32; 2],
    wetness: f32,
    world: Option<&World>,
    inside: Option<&omsi_sim::VehicleInstance>,
    driven: Option<&omsi_sim::VehicleInstance>,
    condensation: [f32; 4],
    settings: &crate::settings::Settings,
    animation_time: f32,
) -> omsi_render::Lighting {
    let mut lighting = match weather {
        Some(w) => weather_lighting(daylight, w, cloud_drift, wetness, settings.shadows),
        None => lights::lighting_from(daylight, 50000.0),
    };
    lighting.wetness = omsi_cfg::flags::OMSI_WETNESS.parse()
        .unwrap_or(wetness);
    dress_lighting(&mut lighting, world, inside, settings);
    // (the air the glass meets: the bus's own speed against the weather's wind)
    lighting.glass_wind = driven.map(crate::lights::vehicle_velocity).unwrap_or_default()
        - weather.map(crate::rain::weather_wind).unwrap_or_default();
    lighting.animation_time = Some(animation_time);
    lighting.condensation = condensation;
    // an LED panel's dots burn this much above their own colour (16 levels,
    // see `Settings::led_glow`); the panel's picture and its mask are held at
    // this mip level at most (`Settings::led_mips`)
    lighting.led_glow = settings.led_glow as f32 * 0.25;
    lighting.led_mips = settings.led_mips;
    // the enhanced clouds marched in fewer steps (`Settings::cloud_quality`)
    lighting.low_clouds = settings.cloud_quality == "low";
    lighting
}

/// What the lighting needs of the bus the camera may be in (`inside`: its box keeps the
/// weather out, its floor and its rear sections' hold no puddles) and of the settings.
pub(crate) fn dress_lighting(
    lighting: &mut omsi_render::Lighting,
    world: Option<&World>,
    inside: Option<&omsi_sim::VehicleInstance>,
    settings: &crate::settings::Settings,
) {
    lighting.inside = inside.and_then(|v| v.ty.def.bounding_box.map(|bb| (v.position, v.heading, bb)));
    let puddle_surface = lighting.inside.and_then(|(o, _, _)| world.and_then(|w| w.puddle_surface(o)));
    lighting.puddle_ground = puddle_surface.map(|(h, _)| h);
    lighting.puddle_normal = puddle_surface.map_or(glam::Vec3::Z, |(_, n)| n);
    lighting.puddle_parts = inside.into_iter().flat_map(|v| &v.trailers)
        .filter_map(|t| t.ty.def.bounding_box.map(|bb| (t.position, t.heading, bb))).take(3).collect();
    lighting.detail = settings.detail_textures;
    lighting.windy_trees = settings.windy_trees();
    lighting.night_brightness = settings.night_brightness;
}

/// The time of day and the departure displays' boards (without a timetable only the clock).
pub(crate) fn departure_boards(
    schedule: Option<&mut schedule::Schedule>,
    world: &World,
    traffic: Option<&traffic::Traffic>,
    duty: Option<&schedule::PlayerDuty>,
    player_hof: Option<&omsi_vehicle::Hof>,
    clock: &omsi_sim::SimClock,
) {
    match schedule {
        Some(s) => s.update_boards(world, traffic, duty, player_hof, clock),
        None => world.timetable_boards.lock().clock = Some(clock.clone()),
    }
}

/// The mirror panels follow the bus's mirrors (their aspects and glass from the world).
pub(crate) fn mirror_hud_sync(hud: &mut crate::mirror_hud::MirrorHud, world: &World, p: &Player, mode: u8) {
    hud.set_aspects(world.mirror_aspect.lock().clone());
    hud.set_glass(world.mirror_glass.lock().clone());
    hud.sync(p, mode);
}

/// The mirror panels over the picture, in the interface's `viewport` (see
/// `Settings::hud_viewport`); `cursor` in the viewport's pixels.
pub(crate) fn push_mirror_hud(
    hud: &crate::mirror_hud::MirrorHud,
    scene: &mut Scene,
    world: &World,
    viewport: [f32; 4],
    cursor: (f32, f32),
) {
    let start = scene.overlays.len();
    hud.push(scene, world, viewport[2], viewport[3], cursor);
    crate::ui::shift_overlays(scene, start, viewport[0]);
}
