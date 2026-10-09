//! The command line: `Args` and the small parsers of its values.

use super::*;

/// The window and picture size when `--size` is not given.
pub(crate) const DEFAULT_SIZE: &str = "1600x900";

#[derive(Parser, Debug, Clone)]
#[command(name = "openomsi", version = crate::startup::VERSION, about = "openOMSI")]
pub(crate) struct Args {
    /// Write a support package (a ZIP for a GitHub issue: the system, the graphics device and
    /// settings, no paths, names or log text) to this file and end, without starting the game.
    #[arg(long)]
    pub(crate) export_diagnostics: Option<PathBuf>,
    /// OMSI 2 installation root (the folder that contains `maps`, `Vehicles`, …).
    /// Found by itself when left out: $OMSI_ROOT, the folder remembered from last time,
    /// a folder next to this program, or the usual Steam locations.
    #[arg(long, default_value = ".")]
    pub(crate) root: PathBuf,
    /// Map to load, relative to root (e.g. maps/Grundorf/global.cfg).
    #[arg(long, default_value = "maps/Grundorf/global.cfg")]
    pub(crate) map: String,
    /// Render one frame to this PNG instead of opening a window.
    #[arg(long)]
    pub(crate) offscreen: Option<PathBuf>,
    /// Offscreen image size.
    #[arg(long, default_value = DEFAULT_SIZE)]
    pub(crate) size: String,
    /// Camera: x,y,z,yaw,pitch (world metres / degrees). Default: the map's editor camera.
    #[arg(long)]
    pub(crate) cam: Option<String>,
    /// Only load tiles within this many tiles of the camera/spawn tile, all at once
    /// (default: 2 for offscreen renders; the window streams the tiles around the camera).
    #[arg(long)]
    pub(crate) radius: Option<i32>,
    /// How far around the camera the window keeps tiles loaded (m); `view_distance` in the
    /// settings file, 1200 m by default.
    #[arg(long = "view-distance")]
    pub(crate) view_distance: Option<f64>,
    /// Load every tile of the map.
    #[arg(long)]
    pub(crate) all: bool,
    /// Vehicle to spawn (relative to root, e.g. Vehicles/MAN_SD200/MAN_SD80.bus).
    #[arg(long)]
    pub(crate) bus: Option<String>,
    /// Entry point index (from global.cfg) where the vehicle is placed.
    #[arg(long, default_value_t = 0)]
    pub(crate) entry: usize,
    /// View: driver, pax, outside, or free; mirror<n> shows what mirror n's camera sees (a check).
    #[arg(long, default_value = "driver")]
    pub(crate) view: String,
    /// Put the bus into service at the start of the run (the Shift+U auto-start). With
    /// `--situation` the bus is not started up again (it keeps its saved state and IBIS);
    /// the flag then only has the duty's next trips typed into the IBIS as they come.
    #[arg(long)]
    pub(crate) autostart: bool,
    /// Start as a pedestrian where the bus would stand (the bus left out): one is placed
    /// from the game menu, or a placed one taken over at its driver's door (G).
    #[arg(long)]
    pub(crate) on_foot: bool,
    /// Control preset: `simple` (W/S/A/D and Up/Down drive, Left/Right switch the interior
    /// camera as in OMSI; the default), `wasd`,
    /// `arrows` (leaves W, S and D to the jobs `Inputs/keyboard.cfg` gives them - wipers,
    /// viewpoint and the D of the automatic gearbox) or `omsi` (only the original layout,
    /// Shift + numpad). With WASD driving, hold shift for the OMSI meaning of a key.
    #[arg(long, default_value = "simple")]
    pub(crate) drive_keys: String,
    /// LAN play: host a session on this UDP port (27015 when the value is 0).
    #[arg(long)]
    pub(crate) lan_host: Option<u16>,
    /// LAN play: join the host at ip[:port], or `auto` to find one on the local network.
    #[arg(long)]
    pub(crate) lan_join: Option<String>,
    /// Dedicated server: host a session from this server.cfg (written with the defaults when
    /// missing), with no window, no sound and no graphics card.
    #[arg(long)]
    pub(crate) server: Option<PathBuf>,
    /// Your name as the other players see it.
    #[arg(long, default_value = "Driver")]
    pub(crate) lan_name: String,
    /// Season: spring, summer, autumn or winter, with its phase `-early`, `-mid` (the
    /// default) or `-late` (`autumn-late`), else the date decides, as in OMSI. The date
    /// moves into the phase's month (its typical day; a --date already in that month stays,
    /// the year and the time of day stay), half a year later south of the equator; the
    /// plants are mixed between the two texture seasons the phase lies between.
    #[arg(long)]
    pub(crate) season: Option<String>,
    /// Fire script triggers after spawning: name[@seconds],… (times apply during --drive).
    #[arg(long)]
    pub(crate) triggers: Option<String>,
    /// Set script variables after spawning: name=value[,name=value…].
    #[arg(long)]
    pub(crate) setvar: Option<String>,
    /// Offscreen: simulate this many seconds with full throttle before rendering.
    #[arg(long)]
    pub(crate) drive: Option<f32>,
    /// Number of AI traffic vehicles to keep around the camera (0 = none).
    #[arg(long, default_value_t = 0)]
    pub(crate) traffic: usize,
    /// Paint scheme / advert of the player vehicle: item name or index from its .cti files.
    #[arg(long)]
    pub(crate) paint: Option<String>,
    /// Number plate (registration) of the player vehicle, e.g. `--plate "B-AB 1234"`: it goes
    /// into the bus's `ident` string variable instead of the plate its `[number]` list, its
    /// `[registration_*]` mode or the map's `registrations.txt` gives it.
    #[arg(long)]
    pub(crate) plate: Option<String>,
    /// Fleet number of the player vehicle from its `[number]` list, e.g. `--number 4711`
    /// (the first entry when absent or not in the list); it goes into the `number` string
    /// variable and, unless the plate is free, gives the plate that list pairs with it.
    #[arg(long)]
    pub(crate) number: Option<String>,
    /// Time of day at start, HH:MM (default 09:00).
    #[arg(long, default_value = "09:00")]
    pub(crate) time: String,
    /// Run the timetable: scheduled AI buses from the map's TTData (needs --traffic > 0 or this).
    #[arg(long)]
    pub(crate) schedule: bool,
    /// With --schedule: the timetable for the player's own duty only, no timetable AI
    /// buses (the launcher's "Disable timetable buses" with a line chosen).
    #[arg(long)]
    pub(crate) no_timetable_buses: bool,
    /// Date at start, YYYY-MM-DD (default 1989-05-30).
    #[arg(long)]
    pub(crate) date: Option<String>,
    /// Window mode: quit after this many seconds (automated runs).
    #[arg(long)]
    pub(crate) exit_after: Option<f32>,
    /// Depot (.hof) name for the player vehicle (default: the map's depot group).
    #[arg(long)]
    pub(crate) hof: Option<String>,
    /// Set script string variables after spawning: name=value[,name=value…].
    #[arg(long)]
    pub(crate) setstr: Option<String>,
    /// Weather file (relative to root), e.g. Weather/Bodennebel.owt; `natural` (or none) is
    /// the physical weather model, `cycle` the weather cycle, `tomorrow` map-group live weather.
    #[arg(long)]
    pub(crate) weather: Option<String>,
    /// Passengers at bus stops (window and offscreen).
    #[arg(long)]
    pub(crate) passengers: bool,
    /// Start with this many passengers already seated in the player's bus.
    #[arg(long, default_value_t = 0)]
    pub(crate) riders: usize,
    /// Personnel file of the driver (e.g. Drivers/OMSI-Fan.odr): the run is added to it.
    #[arg(long)]
    pub(crate) driver: Option<String>,
    /// Offscreen: click the cockpit at this pixel (x,y of the rendered image) and report
    /// which switch was hit.
    #[arg(long)]
    pub(crate) click: Option<String>,
    /// Offscreen: turn the head (or swing the outside camera) by yaw,pitch degrees.
    #[arg(long)]
    pub(crate) look: Option<String>,
    /// How dirty the bus starts (0..1); it keeps collecting dirt as it drives.
    #[arg(long)]
    pub(crate) dirt: Option<f32>,
    /// Fill the tank before the run starts (the depot's fuel pump).
    #[arg(long)]
    pub(crate) refuel: bool,
    /// Run the bus through the wash before the run starts.
    #[arg(long)]
    pub(crate) wash: bool,
    /// Have the workshop repair the bus before the run starts.
    #[arg(long)]
    pub(crate) repair: bool,
    /// Spawn the player vehicle here instead of the entry point: x,y,heading[,z] (z: the
    /// road's height, when it lies above the ground).
    #[arg(long)]
    pub(crate) spawn: Option<String>,
    /// Start without the menu (the command line defines everything).
    #[arg(long)]
    pub(crate) no_menu: bool,
    /// Situation file (.osn): map, date/time, weather, the player's vehicle and its tour.
    #[arg(long)]
    pub(crate) situation: Option<String>,
    /// Write the state after --drive to this .osn file (offscreen), or bind F5 in the window.
    #[arg(long)]
    pub(crate) save_situation: Option<PathBuf>,
    /// Offscreen: put the camera behind AI car with this id (from OMSI_DEBUG_TRAFFIC logs),
    /// or "auto" = the first car that overtakes (then --snapshots are seconds after that).
    #[arg(long)]
    pub(crate) follow: Option<String>,
    /// Offscreen: also save images at these seconds of the --drive run (out_<t>.png).
    #[arg(long)]
    pub(crate) snapshots: Option<String>,
    /// Vehicle physics: rigid (default) or simple (kinematic).
    #[arg(long, default_value = "rigid")]
    pub(crate) physics: String,
    /// Day of year override (set by --situation).
    #[arg(long, hide = true)]
    pub(crate) day_of_year: Option<i32>,
    /// Vehicle state snapshot applied after spawning (set by --situation): name=value pairs.
    #[arg(skip)]
    pub(crate) situation_vars: Vec<(String, f32)>,
    #[arg(skip)]
    pub(crate) situation_strvars: Vec<(String, String)>,
    /// The saved bus odometer, applied after spawning so initialization cannot replace it.
    #[arg(skip)]
    pub(crate) situation_odometer_km: Option<f64>,
    /// Saved ordinal in the current timetable trip; absent in older situations.
    #[arg(skip)]
    pub(crate) situation_next_stop: Option<usize>,
    /// The situation's further vehicles (besides the one driven): each stands where it was
    /// saved with its variables.
    #[arg(skip)]
    pub(crate) situation_others: Vec<SituationOther>,
    /// The trip of the duty the bus was placed at (`duty_start::place_on_duty`).
    #[arg(skip)]
    pub(crate) duty_trip: Option<usize>,
    /// The stop of that trip the duty starts at (a first stop no road leads to is skipped).
    #[arg(skip)]
    pub(crate) duty_first_stop: usize,
    /// The duty's first trip was hours away at the start time: the clock was moved on to
    /// shortly before the bus has to set off (what the HUD says at the start).
    #[arg(skip)]
    pub(crate) clock_moved: Option<String>,
    /// With a duty: start at the entry point nearest to its first stop by road (the
    /// launcher's "Automatic") instead of `--entry`.
    #[arg(long)]
    pub(crate) auto_entry: bool,
    /// With a duty: the clock stays at `--time` however late its first trip leaves (the
    /// launcher's start at the real time).
    #[arg(long)]
    pub(crate) keep_time: bool,
    /// OMSI's tutorial 1..4 (its situation, and its pages beside the picture).
    #[arg(long)]
    pub(crate) tutorial: Option<usize>,
    /// Player's line and tour from the timetable (needs --schedule), e.g. --line 76 --tour 1.
    #[arg(long)]
    pub(crate) line: Option<String>,
    #[arg(long)]
    pub(crate) tour: Option<String>,
    /// The trip of the tour to start with, as OMSI's timetable dialog picks it: its
    /// departure time (HH:MM) or its place in the tour (1 = the first). Without it the duty
    /// starts with the trip under way at --time, else the next to leave.
    #[arg(long)]
    pub(crate) trip: Option<String>,
    /// With --trip: drive on with the rest of the tour after that trip instead of ending the
    /// duty at its terminus.
    #[arg(long)]
    pub(crate) whole_tour: bool,
    /// Write the player's vehicle (with --bus and --paint) as a binary glTF file for the
    /// launcher's 3D preview, and quit.
    #[arg(long)]
    pub(crate) export_glb: Option<PathBuf>,
    /// Enhanced graphics, as the settings' `enhanced=1`: the physically based renderer
    /// (high-range lighting with GGX reflections, a computed sky, soft sun shadows,
    /// automatic exposure, a glow around real highlights and the PBR Neutral tone curve).
    #[arg(long)]
    pub(crate) enhanced: bool,
    /// Enhanced+ graphics, as the settings' `graphics=enhanced_plus`: Enhanced with
    /// ray-traced sun shadows, ambient occlusion and reflections (where the graphics card
    /// can trace rays).
    #[arg(long)]
    pub(crate) enhanced_plus: bool,
    /// Open the launcher (the default when the program is started without arguments).
    #[arg(long)]
    pub(crate) launcher: bool,
    /// Skip the launcher and show the in-game start menu instead.
    #[arg(long)]
    pub(crate) menu: bool,
    /// A mod or map archive (.zip) read in place as a content root, without unpacking it;
    /// may be given several times. `OMSI_CONTENT_ZIP` (separated like PATH) does the same.
    #[arg(long = "content-zip")]
    pub(crate) content_zip: Vec<PathBuf>,
}

pub(crate) fn parse_time(s: &str) -> f64 {
    let mut it = s.split(':');
    let h: f64 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(9.0);
    let m: f64 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
    let sec: f64 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + sec
}

pub(crate) fn parse_cam(s: &str) -> Result<Camera> {
    let v: Vec<f32> = s
        .split(',')
        .map(|x| x.trim().parse::<f32>())
        .collect::<Result<_, _>>()?;
    // (a sixth number is the field of view)
    if v.len() != 5 && v.len() != 6 {
        return Err(anyhow!("--cam needs x,y,z,yaw,pitch[,fov]"));
    }
    Ok(Camera {
        position: DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64),
        yaw: v[3],
        pitch: v[4],
        roll: 0.0,
        fov_deg: v.get(5).copied().unwrap_or(60.0),
        near: 0.1,
        far: 6000.0,
    })
}

/// `--triggers a,b@2.5,c@4`: trigger names with an optional time in seconds.
pub(crate) fn parse_triggers(args: &Args) -> Vec<(String, f32)> {
    args.triggers
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| match s.split_once('@') {
            Some((n, t)) => (n.trim().to_string(), t.trim().parse().unwrap_or(0.0)),
            None => (s.trim().to_string(), 0.0),
        })
        .collect()
}

impl Args {
    pub(crate) fn is_resuming(&self) -> bool {
        self.situation.is_some()
            || self.situation_odometer_km.is_some()
            || !self.situation_vars.is_empty()
            || !self.situation_strvars.is_empty()
    }
}

/// A vehicle of a situation that is not the one driven.
#[derive(Debug, Clone, Default)]
pub(crate) struct SituationOther {
    pub bus: String,
    /// x,y,heading,z as `--spawn` takes it
    pub spawn: String,
    pub hof: Option<String>,
    pub paint: Option<String>,
    pub vars: Vec<(String, f32)>,
    pub strvars: Vec<(String, String)>,
    pub odometer_km: Option<f64>,
}
