//! The offscreen run: a scripted session rendered to pictures (`--offscreen`). Its start is
//! in `setup`, its steps in `step` (with the window's own where the two take the same, see
//! `app_events::steps`), what it says about the run afterwards in `report` and `checks`, and
//! the picture in `picture`.

use super::*;
use crate::app_events::steps;

mod checks;
mod picture;
mod record;
mod report;
mod setup;
mod step;

/// What an offscreen run keeps from its start to its picture (the names are the run's
/// locals when it was one function).
struct Offscreen<'a> {
    args: &'a Args,
    out: &'a PathBuf,
    w: u32,
    h: u32,
    view_aspect: f32,
    settings: settings::Settings,
    renderer: Renderer,
    scene: Scene,
    world: World,
    camera: Camera,
    traffic: Option<traffic::Traffic>,
    schedule: Option<schedule::Schedule>,
    player: Option<Player>,
    spawn_z: f64,
    center: DVec3,
    duty: Option<schedule::PlayerDuty>,
    /// why the duty asked for cannot be driven (the picture's HUD says it too)
    duty_error: Option<String>,
    journey: Option<crate::journey::Journey>,
    career: career::Career,
    humans_off: Option<humans::Humans>,
    /// what the renderer shows of the traffic and the people (see `view_sync`)
    sim_view: crate::view_sync::SimView,
    player_ref: Option<Player>,
    envir: Option<omsi_content::Envir>,
    weather: omsi_content::weather::Weather,
    /// the workshop's waiting time moves the clock on, so the sky has to follow it
    service_seconds: f64,
    dt: f32,
    drive_frames: usize,
    wheel_worst: Option<(DVec3, f64)>,
    total_frames: usize,
    /// a dedicated server runs until it is told to stop (SIGTERM, Ctrl+C)
    server: bool,
    timed: Vec<(String, f32)>,
    snapshot_times: Vec<f32>,
    drive_start: DVec3,
    drive_profile: Vec<[f32; 4]>,
    drive_v0: Option<f32>,
    physics_log: f32,
    last_reasons: Vec<String>,
    lan_audio: Option<omsi_audio::AudioEngine>,
    lan_off: Option<omsi_net::LanSession>,
    remotes_off: lan::LanGame,
    /// the clock of the moment the run has reached (the start, the workshop's wait and the
    /// seconds run; a dedicated server's own), as the window's goes on
    run_clock: omsi_sim::SimClock,
    /// how wet the roads are, rain wetting them and dry weather drying them as in the window
    wetness: f32,
    /// the cabin air of the player's bus and the condensation on its glass
    cabin_air: crate::condensation::CabinAir,
    /// a dedicated server's administration and clock (see `admin`)
    srv_admin: crate::admin::ServerAdmin,
    srv_clock: f64,
    srv_metar: Option<String>,
    srv_metar_due: std::time::Instant,
    srv_metar_rx: Option<std::sync::mpsc::Receiver<Option<omsi_content::weather::Weather>>>,
    /// the weather's name on the status page
    srv_weather_name: String,
    ground_gap: Option<crate::ground_gap::GroundGap>,
    spray: puddles::Spray,
    real_time: RealTime,
    /// `OMSI_RECORD`: the film being written (see `record`).
    recorder: Option<record::Recorder>,
}

pub(crate) fn run_offscreen(
    args: &Args,
    out: &PathBuf,
    lan_off: Option<omsi_net::LanSession>,
    remotes_off: lan::LanGame,
) -> Result<()> {
    let mut run = Offscreen::setup(args, out, lan_off, remotes_off)?;
    for i in 0..run.total_frames {
        if !run.step(i)? {
            break;
        }
    }
    run.finish_recording()?;
    run.finish()
}

impl Offscreen<'_> {
    /// After the steps: the run's reports, the checks asked for, the picture.
    fn finish(mut self) -> Result<()> {
        let args = self.args;
        let Self { ref mut camera, ref traffic, .. } = self;
        if let Some(id) = follow_id(args, traffic.as_ref()) {
            match follow_camera(traffic.as_ref(), id) {
                Some(c) => {
                    log::info!(
                        "following car {id} at ({:.1}, {:.1}) heading {:.0}",
                        c.position.x,
                        c.position.y,
                        c.yaw
                    );
                    *camera = c;
                }
                None => log::warn!("--follow: car {id} not found"),
            }
        }
        if let Some(g) = self.ground_gap.take() {
            g.report();
        }
        self.traffic_report();
        if self.server {
            if let Some(l) = self.lan_off.take() {
                l.leave();
            }
            return Ok(());
        }
        self.player_report();
        self.humans_report();
        checks::run_checks(&self.world, self.traffic.as_ref());
        let clock = self.save_run();
        self.picture(clock)
    }
}

/// The camera the run's eye is (for the traffic, the people and the spray): a followed
/// car's, else the player's view, else the run's camera.
fn eye_camera(args: &Args, traffic: Option<&traffic::Traffic>, player: Option<&Player>, camera: &Camera) -> Camera {
    match traffic.and_then(|t| follow_id(args, Some(t)).and_then(|id| follow_camera(Some(t), id))) {
        Some(c) => c,
        None => match player {
            Some(p) if args.cam.is_none() && args.view != "free" && args.follow.is_none() => {
                p.camera(&args.view, camera)
            }
            _ => *camera,
        },
    }
}

/// The player's view from `base` with the head turned as --look says (the outside view
/// kept clear of the world).
fn player_view(args: &Args, settings: &settings::Settings, p: &mut Player, base: &Camera, world: &World) -> Camera {
    let look = crate::player::driver_head_look(
        look_of(args),
        &args.view,
        settings.seat_pitch_deg,
        false,
    );
    let cam = p.camera_look(&args.view, base, look, offscreen_orbit());
    if args.view == "outside" {
        p.camera_clipped(cam, world, offscreen_orbit(), 0.0)
    } else {
        cam
    }
}

/// The player's bus and its driver into the scene, as a picture of them is taken.
fn pose_player(p: &mut Player, renderer: &Renderer, scene: &mut Scene, args: &Args, settings: &settings::Settings) {
    p.sync_transforms(
        renderer,
        scene,
        matches!(args.view.as_str(), "driver" | "pax"),
    );
    crate::scene::sync_vehicle_damage(renderer, scene, &mut p.vehicle, &mut p.render);
    p.sync_driver(renderer, scene, 1.0 / 30.0, settings.driver, args.view == "driver");
}

/// The lights of a picture: the lamps' cones in this weather, the corona pictures, the
/// light maps round `eye` and every light that shines there.
fn world_lights(
    renderer: &mut Renderer,
    world: &World,
    scene: &mut Scene,
    weather: &omsi_content::weather::Weather,
    daylight: &omsi_sim::Daylight,
    eye: DVec3,
    vehicles: &[&omsi_sim::VehicleInstance],
) {
    lights::set_cone_strength(weather.fog.0, precip_of(weather).1, daylight.night);
    lights::upload_corona_textures(renderer);
    world.update_light_map_atlas(renderer, eye);
    lights::collect(world, scene, daylight, eye, vehicles);
}

/// The offscreen loop's fixed steps kept to the clock when other games take part (a LAN
/// session, the dedicated server): a step of `dt` every `dt` on the wall, the step's own work
/// included. A whole step slept after each frame's work made a dedicated server's frames
/// 41 ms long on Gladbeck: its world ran at 80 % of the clock while its world frames were
/// stamped with the clock, so the players saw its cars and people a fifth too slow, mostly
/// guessed on past the last frame they had, and standing still for a moment again and again
/// (#660); its clock and timetable fell behind the day as well.
#[derive(Default)]
pub(crate) struct RealTime {
    /// When the last step was to end (its wait ran until then).
    end: Option<Instant>,
}

impl RealTime {
    /// Further behind than this (a stall: a long load, the machine busy), the steps go on
    /// from now rather than run back to back until they have caught up - the same quarter of
    /// a second after which the session's stamps take the wall clock again
    /// (`LanSession::tick`).
    const BEHIND_MAX: std::time::Duration = std::time::Duration::from_millis(250);

    /// How long to wait at `now`, at the end of a step of `dt` seconds.
    pub(crate) fn wait(&mut self, now: Instant, dt: f32) -> std::time::Duration {
        let dt = std::time::Duration::from_secs_f32(dt);
        let due = self.end.map(|e| e + dt).unwrap_or(now + dt);
        let due = if now > due + Self::BEHIND_MAX {
            now
        } else {
            due
        };
        self.end = Some(due);
        due.saturating_duration_since(now)
    }
}

/// A dedicated server's traffic kept on the server's clock, `speed` times as fast as the real
/// time and moved by `jump` s in this step (an admin's `time` or `clock`, the real-time sync).
/// The timetable's buses, the street traffic's density by the hour and the trips due at the
/// stops run on the traffic's clock (`day_time`), which goes on at its own `time_scale`: the
/// window sets that every frame and moves the clock with its own (`frame_weather`,
/// `shift_clock`), a server did neither. Its timetable ran on the real time from the start,
/// whatever the clock the players were told.
fn traffic_on_server_clock(t: &mut omsi_sim::ai_traffic::TrafficSim, speed: f64, jump: f64) {
    t.time_scale = speed;
    t.day_time += jump;
}

/// The tyres as they are drawn: the lowest point of each wheel mesh and how far it is over
/// the road under it (negative: in the asphalt).
fn tyre_lows(v: &omsi_sim::VehicleInstance, world: &World) -> Vec<(DVec3, f64)> {
    let mut out = Vec::new();
    for (i, vm) in v.ty.meshes.iter().enumerate() {
        let def = &v.ty.model.meshes[vm.def_index];
        if !v.mesh_props[i].visible || !def.animations.iter().any(|an| an.variable.to_ascii_lowercase().starts_with("wheel_rotation_")) {
            continue;
        }
        let xf = v.mesh_local_transform(i);
        let Some(q) = vm.data.positions.iter().map(|q| xf.transform_point3(*q)).min_by(|a, b| a.z.total_cmp(&b.z)) else { continue };
        let w = v.position + q.as_dvec3();
        if let Some(g) = world.ground_height(w.x, w.y) {
            out.push((w, w.z - g));
        }
    }
    out
}

/// The worst pitch and bank of the drive, the origin's highest and lowest over the ground,
/// and where and when the worst pitch was.
static DRIVE_EXTREMES: parking_lot::Mutex<(f32, f32, f64, f64, (DVec3, f32))> = parking_lot::Mutex::new((0.0, 0.0, f64::MIN, f64::MAX, (DVec3::ZERO, 0.0)));

/// `OMSI_CAM_VEHICLE=x,y,z,yaw,pitch[,fov]`: a camera in the bus's own frame (x right,
/// y forward, z up; yaw relative to the bus) for close-ups of displays and switches - the
/// final picture and every `--snapshots` one.
/// With a triple screen, the frustum around its three panels (see `App::sight_extent`).
fn triple_extent(settings: &crate::settings::Settings, cam: &Camera, w: u32, h: u32) -> Option<(f64, f64)> {
    (settings.triple.enabled && !settings.vr_requested()).then(|| {
        let (x, y) = settings.triple.view_extent(cam, w, h);
        (x as f64, y as f64)
    })
}

fn vehicle_camera(player: &Player, camera: &mut Camera) {
    let Some(spec) = omsi_cfg::flags::OMSI_CAM_VEHICLE.var() else { return };
    let v: Vec<f32> = spec
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if v.len() >= 5 {
        camera.position = player.vehicle.position
            + player
            .vehicle
            .body_rotation()
            .transform_point3(Vec3::new(v[0], v[1], v[2]))
            .as_dvec3();
        camera.yaw = player.vehicle.heading as f32 + v[3];
        camera.pitch = v[4];
        camera.near = 0.02;
        if let Some(f) = v.get(5) {
            camera.fov_deg = *f;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{traffic_on_server_clock, RealTime};
    use std::time::{Duration, Instant};

    /// The waits `RealTime` asks for after steps of 1/30 s that take `work` each, and the
    /// wall time they all took.
    fn run(work: &[u64]) -> (Vec<Duration>, f64) {
        let t0 = Instant::now();
        let (mut now, mut pace, mut waits) = (t0, RealTime::default(), Vec::new());
        for w in work {
            now += Duration::from_millis(*w);
            let wait = pace.wait(now, 1.0 / 30.0);
            waits.push(wait);
            now += wait;
        }
        (waits, (now - t0).as_secs_f64())
    }

    #[test]
    fn a_server_keeps_to_the_clock_whatever_its_steps_take() {
        // a dedicated server on Gladbeck: 8 ms of work a step; 300 steps (10 s of its world)
        // took 12.4 s with a whole step slept after each
        let (_, wall) = run(&[8; 300]);
        assert!((wall - 10.0).abs() < 0.04, "300 steps in {wall} s");
        // uneven steps, some longer than a step: made up by the next ones
        let uneven: Vec<u64> = (0..300).map(|i| [2, 45, 9, 60, 20][i % 5]).collect();
        let (_, wall) = run(&uneven);
        assert!((wall - 10.0).abs() < 0.07, "300 uneven steps in {wall} s");
        // a stall of a second is not made up by thirty steps at once: on from there at the
        // steps' pace
        let mut stalled = vec![5; 100];
        stalled[50] = 1000;
        let (waits, _) = run(&stalled);
        let steady = |w: &Duration| (w.as_secs_f64() - 0.0283).abs() < 0.001;
        assert!(waits[51..].iter().all(steady), "{:?}", &waits[50..60]);
    }

    #[test]
    fn a_servers_timetable_runs_on_the_servers_clock() {
        use omsi_sim::ai_traffic::{setup::RandomTypes, TrafficSim};
        // (no lanes: only the traffic's clock is looked at)
        let random = RandomTypes {
            types: Vec::new(),
            groups: Vec::new(),
            group_curves: false,
            group_uvg: Vec::new(),
            uvg_defaults: Vec::new(),
        };
        let mut t = TrafficSim::assemble(
            std::path::Path::new("."),
            Default::default(),
            random,
            Vec::new(),
            Default::default(),
            (Vec::new(), Vec::new()),
            Vec::new(),
            (1.0, 0),
            0,
        );
        // a server started at 05:20 (its clock as `server_step` keeps it: the start, the
        // seconds run at its speed, the admins' shift); x30 from its 10th second, an admin's
        // `clock 08:19` in its 20th, `time -3600` in its 25th
        let start = 5.0 * 3600.0 + 20.0 * 60.0;
        t.day_time = start;
        let (dt, mut run, mut shift) = (1.0f32 / 30.0, 0.0f64, 0.0f64);
        for i in 0..900 {
            let speed = if i < 300 { 1.0 } else { 30.0 };
            run += dt as f64 * speed;
            let was = shift;
            if i == 600 {
                shift += 8.0 * 3600.0 + 19.0 * 60.0 - (start + run + shift);
            }
            if i == 750 {
                shift -= 3600.0;
            }
            traffic_on_server_clock(&mut t, speed, shift - was);
            t.tick(dt, None);
            let clock = start + run + shift;
            assert!(
                (t.day_time - clock).abs() < 1e-6,
                "step {i}: the traffic's clock {:.1} s, the server's {clock:.1} s",
                t.day_time
            );
        }
        // 08:19, an hour back, and 299 steps of a game second each: 07:23:59 (the traffic's own
        // clock ran 30 s of the real time: 05:20:30)
        assert!(
            (t.day_time - (7.0 * 3600.0 + 23.0 * 60.0 + 59.0)).abs() < 0.01,
            "{}",
            t.day_time
        );
    }
}
