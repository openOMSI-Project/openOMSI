//! Every `OMSI_*` environment variable the code reads, in one typed list.
//!
//! The switches grew one call site at a time (`omsi_cfg::env::var_os("OMSI_X").is_some()`),
//! hundreds of them, documented only where they are read. Here each one has a name, a type,
//! a default, a kind (debug log, test hook, A/B switch, tuning, set-up) and a line of
//! documentation; `docs/DEBUG_FLAGS.md` is written from this list, and the tests below fail
//! when a name in the source is missing here or the document is out of date
//! (`OMSI_FLAGS_BLESS=1 cargo test -p omsi-cfg flags` writes it again).
//!
//! Reading a flag costs an atomic load after the first time: the value is taken once from
//! [`crate::env`] (which caches the environment for the whole process, so a flag and a
//! plain `env::var_os` of the same name always agree, and a `set_var` after the first read
//! of a name is not seen - as before) and kept in the flag. [`Flag::live_os`] and
//! [`Flag::live_var`] read the environment itself every time, for the call sites that
//! did so (a path read once at start, or a variable set at runtime: `OMSI_CONTENT` on
//! Android, `OMSI_BACKEND` by the launcher).
//!
//! Call sites name a flag with its module, `omsi_cfg::flags::OMSI_X.is_set()` (or
//! `flags::OMSI_X` after `use omsi_cfg::flags`), so that the test finds it.
//!
//! A `Bool` flag is on when the variable is set at all - to anything, `0` and the empty
//! string included.

use std::ffi::{OsStr, OsString};
use std::sync::OnceLock;

/// How the call sites take the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// On when set, whatever the value.
    Bool,
    /// A number (`str::parse` of the whole value; unparsable is as unset).
    Num,
    /// A string: a path, a list, a name, a mode.
    Text,
}

/// What a flag is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// More logging, dumps and traces; changes nothing else.
    Debug,
    /// Drives a test, a check pass or an offscreen run, or injects a fault.
    Test,
    /// A/B or kill switch: an older or plainer path, or a feature off.
    Switch,
    /// A real setting for one run (overrides the settings file, or a tuning value).
    Tuning,
    /// Where things are: folders, programs, URLs, the instance and back-end handed down.
    Setup,
    /// Read when compiling (`env!`), not at runtime.
    Build,
}

/// How often it is read (by the call sites as they stand).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// Once per process (the call site keeps it in a static).
    Once,
    /// Each time the code passing it runs: start-up, loading, or per step / per vehicle.
    Use,
    /// Every frame, in `Renderer::render_inner`.
    Frame,
    /// Only by tests and examples.
    Test,
    /// Only when compiling.
    Build,
}

/// One `OMSI_*` variable.
pub struct Flag {
    pub name: &'static str,
    pub ty: Ty,
    pub kind: Kind,
    pub read: Read,
    /// What unset means, as the docs say it (`off`, a number, `settings`, `-` for nothing).
    pub default: &'static str,
    pub doc: &'static str,
    value: OnceLock<Option<OsString>>,
}

impl Flag {
    const fn new(name: &'static str, ty: Ty, kind: Kind, read: Read, default: &'static str, doc: &'static str) -> Flag {
        Flag { name, ty, kind, read, default, doc, value: OnceLock::new() }
    }

    /// The value, as `omsi_cfg::env::var_os` gives it, without a copy.
    pub fn os(&self) -> Option<&OsStr> {
        self.value.get_or_init(|| crate::env::var_os(self.name)).as_deref()
    }

    /// Set at all (`env::var_os(..).is_some()`).
    pub fn is_set(&self) -> bool {
        self.os().is_some()
    }

    /// The value as text: `None` when unset or not Unicode (`env::var(..).ok()`).
    pub fn var(&self) -> Option<&str> {
        self.os().and_then(|v| v.to_str())
    }

    /// The whole value parsed (`env::var(..).ok().and_then(|v| v.parse().ok())`).
    pub fn parse<T: std::str::FromStr>(&self) -> Option<T> {
        self.var().and_then(|v| v.parse().ok())
    }

    /// The environment as it is now (`std::env::var_os`), not cached.
    pub fn live_os(&self) -> Option<OsString> {
        std::env::var_os(self.name)
    }

    /// The environment as it is now (`std::env::var`), not cached.
    pub fn live_var(&self) -> Result<String, std::env::VarError> {
        std::env::var(self.name)
    }
}

impl std::fmt::Debug for Flag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flag").field("name", &self.name).field("ty", &self.ty).field("kind", &self.kind).finish()
    }
}

/// The registered flag of this name.
pub fn find(name: &str) -> Option<&'static Flag> {
    ALL.binary_search_by(|f| f.name.cmp(name)).ok().map(|i| ALL[i])
}

macro_rules! flags {
    ($($name:ident: $ty:ident, $kind:ident, $read:ident, $default:literal, $doc:literal;)*) => {
        $(
            #[doc = $doc]
            pub static $name: Flag = Flag::new(stringify!($name), Ty::$ty, Kind::$kind, Read::$read, $default, $doc);
        )*
        /// Every flag, sorted by name.
        pub static ALL: &[&Flag] = &[$(&$name),*];
    };
}

// name: type, kind, read, default, doc - sorted by name
flags! {
    OMSI_AI_MODEL_LOCK: Bool, Switch, Use, "off", "AI cars steer no further than their model's own steering lock (no 60 degree allowance for tight turns).";
    OMSI_AI_WAY_ONLY: Bool, Switch, Use, "off", "AI vehicles stand on their way with the plain ground sampler, as before (A/B).";
    OMSI_AUDIT_LINE: Num, Test, Test, "-", "bus_audit example: the line (number and letter code) the IBIS typist enters instead of the bus's own.";
    OMSI_AUTOPILOT: Num, Test, Use, "-", "Offscreen: the player's bus follows the lanes at this speed in km/h (finds where it falls through or leaves the road).";
    OMSI_BACKEND: Text, Setup, Use, "settings", "vulkan, dx12, metal, gl or angle (dx11, d3d11; Windows): the graphics API tried first (overrides the settings). Set at runtime by the launcher and on Android.";
    OMSI_BACKGROUND: Bool, Test, Use, "off", "A test window that does not take the keyboard focus (OMSI_INPUT drives the handlers directly).";
    OMSI_BASIC_PIPELINES: Bool, Switch, Once, "off", "Use the reduced (basic) render pipelines, as after a driver failed to build the full ones.";
    OMSI_BATCH: Num, Test, Use, "-", "Offscreen: prepare the map tiles this many at a time, as the window's streaming does.";
    OMSI_BENCH: Num, Test, Use, "-", "Offscreen: draw the final picture this many more times and log the median CPU and GPU-wait time.";
    OMSI_BENCH_FRAMES: Bool, Debug, Use, "off", "With OMSI_BENCH: log every bench frame's times, not only the medians.";
    OMSI_BLEND_AB: Bool, Test, Use, "off", "Offscreen: a second picture with the blended draws in the old order, for a before/after of the draw order.";
    OMSI_BRIDGE_ONLY: Bool, Test, Use, "off", "LAN: forget our own addresses so that only what the rendezvous bridge reports is tried.";
    OMSI_BUDGET_FROM: Text, Test, Use, "-", "Offscreen with OMSI_TEXTURE_MEMORY: x,y[,MB] - the texture budget is met as seen from that point.";
    OMSI_BUILD: Text, Build, Build, "-", "Compile-time (env!): the commit the binary was built from, set by build.rs. Not read at runtime.";
    OMSI_CAM_VEHICLE: Text, Test, Use, "-", "Offscreen: x,y,z,yaw,pitch[,fov] - a camera in the bus's own frame.";
    OMSI_CHECK_ENTRIES: Bool, Test, Use, "off", "Map check: is there ground to stand on where a player is put down.";
    OMSI_CHECK_GROUND: Bool, Test, Use, "off", "Check: people on foot with a walkable surface above their heads, every two seconds.";
    OMSI_CHECK_OBJECTS: Bool, Test, Use, "off", "Map check: report placed objects that float or are out of place while loading tiles.";
    OMSI_CHECK_OBSTACLES: Bool, Test, Use, "off", "Map check: sweep a bus-sized box along every lane and list the obstacle boxes it hits.";
    OMSI_CHECK_OVERLAP: Bool, Test, Use, "off", "Check: every AI vehicle whose body has got into another's or into an obstacle.";
    OMSI_CHECK_ROADS: Bool, Test, Use, "off", "Map check: lanes with no road surface drawn under them, and road points under the ground.";
    OMSI_CHECK_SPIKES: Bool, Test, Use, "off", "Map check: road faces standing taller than profile, gradient and cant allow.";
    OMSI_CHECK_SPLINES: Bool, Test, Use, "off", "Map check: chained splines whose ends do not meet in height.";
    OMSI_CHECK_TPOSE: Bool, Test, Use, "off", "Check: people drawn in the rest (T) pose, wider than 1.3 m hand to hand.";
    OMSI_CHECK_TRIPS: Text, Test, Use, "-", "1: build every trip's route on the loaded lanes and report breaks; a trip name: log every step of that trip.";
    OMSI_CHECK_TYPES: Bool, Test, Use, "off", "Map check: list the object types per folder that loading leaves out.";
    OMSI_CHECK_WALLS: Bool, Test, Use, "off", "Check: everybody inside a bus who stands away from its walkways.";
    OMSI_CHECK_WHEELS: Bool, Test, Use, "off", "Map check: probe the ground along every lane's wheel tracks (invisible walls, humps, holes).";
    OMSI_CHURN: Text, Test, Use, "-", "Offscreen: x,y - load the tiles round that point and unload/reload the start area (streaming test).";
    OMSI_CLICK_ALL: Bool, Test, Use, "off", "Check: aim at every clickable item of the cab and report which one would actually be operated.";
    OMSI_CLOUDFLARED: Text, Setup, Use, "-", "Path of the cloudflared binary for the LAN tunnel (searched after the game's folder, before the PATH).";
    OMSI_CLOUDFLARED_OWN: Bool, Test, Use, "off", "LAN tunnel: only the game's own downloaded cloudflared, to test the download.";
    OMSI_CONDENSATION: Text, Test, Use, "-", "Offscreen: minutes,people[,engine] - the cabin air and condensation on the glass after that long.";
    OMSI_CONTENT: Text, Setup, Use, "beside the game", "The content folder (mods, archives, screenshots). Set at runtime on Android.";
    OMSI_CONTENT_ZIP: Text, Setup, Use, "-", "Extra content archives, separated like PATH (as --content-zip).";
    OMSI_CROSSING_DEFORM: Bool, Switch, Use, "off", "Apply the crossings' ground deformation again (A/B; Omsi.exe does not).";
    OMSI_CUT_PLAIN: Bool, Switch, Use, "off", "Road-cut textures uploaded without GPU-made mipmaps.";
    OMSI_DAY_AIR: Text, Test, Once, "-", "haze,angstrom[,height,strat]: fix the day's air (twilight colours) instead of the varying one.";
    OMSI_DEBUG_AI_WIDE: Bool, Debug, Use, "off", "Log every tenth of a second an AI car standing over 1.5 m beside its way.";
    OMSI_DEBUG_ANIM: Text, Debug, Once, "-", "Log the animations of the meshes whose file name contains this text.";
    OMSI_DEBUG_BOARDS: Bool, Debug, Use, "off", "Log boarding at bus stops.";
    OMSI_DEBUG_CAMERA: Num, Debug, Once, "1", "Camera arm log: 1 the types and what stops the arm, 2 also every frame.";
    OMSI_DEBUG_CAR: Num, Debug, Use, "-", "Log the state of the AI car with this id every step.";
    OMSI_DEBUG_CAREER: Bool, Debug, Use, "off", "Log the jolts the career scoring counts.";
    OMSI_DEBUG_COLLISION: Bool, Debug, Use, "off", "Log the player's collision state.";
    OMSI_DEBUG_CONES: Bool, Debug, Use, "off", "Log the light coronas and beams prepared per frame.";
    OMSI_DEBUG_CULL: Num, Debug, Frame, "off", "Log what near and in the picture is left out of the draw list, and why.";
    OMSI_DEBUG_DOORS: Bool, Debug, Use, "off", "Log the door variables of AI buses twice a second.";
    OMSI_DEBUG_DRAWS: Bool, Debug, Frame, "off", "Log how many instances changed per prepare.";
    OMSI_DEBUG_DRIVER: Bool, Debug, Use, "off", "Log the driver figure's settling (grips, wrists, elbows).";
    OMSI_DEBUG_ENHANCED: Num, Debug, Once, "0", "Enhanced main pass shows one of its terms alone (n selects it: 1 sun shadow, 2 occlusion, ...).";
    OMSI_DEBUG_EXPOSURE: Bool, Debug, Use, "off", "Read back and log the adapted exposure metering now and then.";
    OMSI_DEBUG_FLICKER: Bool, Debug, Frame, "off", "Log near instances drawn in one frame and not the next (objects blinking).";
    OMSI_DEBUG_FLOAT: Bool, Debug, Use, "off", "Log objects floating over the ground while placing them.";
    OMSI_DEBUG_FOG_LAMPS: Bool, Debug, Frame, "off", "Log the fog lamp passes per frame.";
    OMSI_DEBUG_FOOT: Bool, Debug, Use, "off", "Log where the walking avatar is drawn.";
    OMSI_DEBUG_HUMANS: Bool, Debug, Use, "off", "Log the people (passengers, pedestrians) and their placement.";
    OMSI_DEBUG_IBIS: Bool, Debug, Use, "off", "Log the IBIS typist's reasoning.";
    OMSI_DEBUG_INTERIOR: Bool, Debug, Use, "off", "Log the interior lamps of each vehicle type.";
    OMSI_DEBUG_JUNCTION: Bool, Debug, Use, "off", "Log why AI cars wait at a junction.";
    OMSI_DEBUG_LAMPS: Bool, Debug, Use, "off", "Log traffic lights without a crossing, and the moving lamps (barriers).";
    OMSI_DEBUG_LAN: Bool, Debug, Use, "off", "LAN: every few seconds what we know of every player, counts and bytes a second.";
    OMSI_DEBUG_LANES: Text, Debug, Use, "-", "Log the lanes with these ids (comma-separated).";
    OMSI_DEBUG_LIGHT: Bool, Debug, Use, "off", "Add a test point light above the camera.";
    OMSI_DEBUG_LIGHTS: Text, Debug, Use, "-", "all, near or controller names: log every light change of those traffic light programs.";
    OMSI_DEBUG_LIGHT_GRID: Bool, Debug, Use, "off", "Log lights left out of a full light-grid cell.";
    OMSI_DEBUG_MESHES: Text, Debug, Use, "off", "Log per material slot which part of its texture a mesh shows (display texts).";
    OMSI_DEBUG_MIRRORS: Bool, Debug, Use, "off", "Log each mirror's eye, angles and field of view.";
    OMSI_DEBUG_MISSING: Bool, Debug, Use, "off", "Log where missing textures were looked for.";
    OMSI_DEBUG_NAN: Bool, Debug, Once, "off", "Warn at every write of a NaN or infinity into a script variable.";
    OMSI_DEBUG_NAV: Bool, Debug, Use, "off", "Log navigation details (e.g. stops whose tiles are not loaded).";
    OMSI_DEBUG_OBJECTS: Bool, Debug, Use, "off", "Log objects placed outside their tile.";
    OMSI_DEBUG_OBJMAT: Text, Debug, Use, "-", "Log how the material slots of the objects whose file name contains this text are made.";
    OMSI_DEBUG_PARTICLES: Bool, Debug, Use, "off", "Log the smoke particles.";
    OMSI_DEBUG_PASS: Bool, Debug, Use, "off", "Log twice a second why a car standing behind something does not go round it.";
    OMSI_DEBUG_PAX: Bool, Debug, Use, "off", "Log every change of state of the passengers.";
    OMSI_DEBUG_PHYSICS: Num, Debug, Use, "off", "Log the player's pose, speed, wheel contacts and crashes (offscreen: =secs, the interval).";
    OMSI_DEBUG_POPULATION: Bool, Debug, Use, "off", "Log where cars appear and vanish relative to the view.";
    OMSI_DEBUG_PROPS: Bool, Debug, Use, "off", "Log the destination-display (matrix) variables.";
    OMSI_DEBUG_PUDDLES: Num, Debug, Use, "0", "Puddle debug view (a number selects it).";
    OMSI_DEBUG_RAIN: Bool, Debug, Use, "off", "Log the tyres' spray and puddles.";
    OMSI_DEBUG_RASTER: Text, Debug, Use, "-", "x,y: log the ground raster at that point.";
    OMSI_DEBUG_REPEATERS: Bool, Debug, Use, "off", "Log repeater objects whose spline chain disagrees with the map.";
    OMSI_DEBUG_REST: Bool, Debug, Use, "off", "Log where the bus came to rest on the ground after spawning.";
    OMSI_DEBUG_ROUTES: Bool, Debug, Use, "off", "Log where consecutive lanes of an AI route do not join.";
    OMSI_DEBUG_RT: Num, Debug, Use, "off", "Log the ray tracing structures every 30 frames (with a number: a debug view).";
    OMSI_DEBUG_SEAT: Bool, Debug, Use, "off", "Log the driver seat's suspension meshes.";
    OMSI_DEBUG_SERVICES: Bool, Debug, Use, "off", "Log petrol station boxes and the bus's distance to them.";
    OMSI_DEBUG_SHADOW: Num, Debug, Frame, "off", "Shadow debug overlay (with a number: its radius, default 3).";
    OMSI_DEBUG_SHADOW_FAR: Bool, Debug, Frame, "off", "Log each redraw of the far shadow map.";
    OMSI_DEBUG_SIGNALS: Bool, Debug, Use, "off", "Log what each railway signal shows.";
    OMSI_DEBUG_SKY: Bool, Debug, Frame, "off", "Log the sky and sun values per frame.";
    OMSI_DEBUG_SOUND: Num, Debug, Once, "5", "Log which sounds were heard (number: the interval in seconds).";
    OMSI_DEBUG_SPLINES: Bool, Debug, Use, "off", "Log the splines and lanes built per tile.";
    OMSI_DEBUG_STARTUP: Bool, Debug, Use, "off", "Log the start-up steps of a vehicle's systems.";
    OMSI_DEBUG_STOPS: Bool, Debug, Use, "off", "Log the bus stops a bus missed.";
    OMSI_DEBUG_STUCK: Bool, Debug, Use, "off", "Log the report of stuck AI cars (also explains junction waits).";
    OMSI_DEBUG_SURFACES: Bool, Debug, Use, "off", "Log the painted ground layers per tile.";
    OMSI_DEBUG_SWITCHES: Bool, Debug, Use, "off", "Log railway switches thrown for a train.";
    OMSI_DEBUG_TEXT: Bool, Debug, Once, "off", "Log the scripts' string variable operations.";
    OMSI_DEBUG_TEXTURES: Bool, Debug, Use, "off", "Log all textures by size and format.";
    OMSI_DEBUG_TRAFFIC: Bool, Debug, Use, "off", "Log traffic light programs, cars standing for long and hard bends.";
    OMSI_DEBUG_TRAILER: Bool, Debug, Use, "off", "Log a trailer's rest, sag and lift on the ground.";
    OMSI_DEBUG_TRAILERS: Bool, Debug, Use, "off", "Log coupled parts off the level of what pulls them.";
    OMSI_DEBUG_TRIGGERS: Bool, Debug, Use, "off", "Log the variables a mouse trigger changed.";
    OMSI_DEBUG_UNLINKED: Bool, Debug, Use, "off", "Log lane ends with an untaken lane start near them.";
    OMSI_DEBUG_UPLOAD: Bool, Debug, Use, "off", "Log slow GPU uploads item by item.";
    OMSI_DEBUG_VARIANTS: Text, Debug, Use, "-", "Log material variants whose variable name contains this text.";
    OMSI_DEBUG_VARS: Num, Debug, Use, "-", "Offscreen: log these script variables of the player's bus (comma-separated).";
    OMSI_DEBUG_VARS_EVERY: Num, Debug, Use, "-", "With OMSI_DEBUG_VARS: log them every this many seconds through the drive.";
    OMSI_DEBUG_VAR_SYNC: Text, Debug, Use, "-", "LAN: log this variable, ours every two seconds and theirs as it comes.";
    OMSI_DEBUG_VIEW_LAMPS: Bool, Debug, Frame, "off", "Log the view lamps value per frame.";
    OMSI_DEBUG_WALLS: Bool, Debug, Use, "off", "Log the walls a walking player slides along.";
    OMSI_DEBUG_WARP: Bool, Debug, Use, "off", "Log crossings whose ground was moved by more than a metre.";
    OMSI_DEBUG_WHEELS: Bool, Debug, Use, "off", "Log where the wheel meshes of every vehicle type turn.";
    OMSI_DRIVER_HANDS: Text, Test, Use, "-", "left,right: both driver hands held at these angles (grip close-ups).";
    OMSI_DRIVER_SHIFTER: Text, Test, Use, "-", "Part of a variable or file name: force the shifter the driver's hand uses; off: both hands on the wheel.";
    OMSI_DRIVE_PROFILE: Text, Test, Use, "-", "Offscreen: \"t throttle brake [steer]/...\" piecewise constant pedals (crash/kerb tests).";
    OMSI_DRIVE_V0: Num, Test, Use, "-", "Offscreen: the bus's speed in km/h on the first frame.";
    OMSI_DUMP_CUT: Text, Debug, Use, "-", "Directory: write each tile's road-cut alpha masks as images.";
    OMSI_DUMP_GROUND: Text, Debug, Use, "-", "File: every loaded tile's final terrain and road cut as text.";
    OMSI_DUMP_SCENERY_TEXT: Text, Debug, Use, "-", "Directory: write the scenery objects' text textures as drawn.";
    OMSI_DUMP_SCRIPTTEX: Text, Debug, Use, "-", "Directory: write the player's bus's script (display) textures.";
    OMSI_ENHANCED: Bool, Tuning, Use, "settings", "Enhanced graphics for this run.";
    OMSI_ENHANCED_PLUS: Bool, Tuning, Use, "settings", "Enhanced+ (ray traced) graphics for this run.";
    OMSI_ENV_PHOTO: Num, Debug, Use, "1", "0 leaves the environment photo out of the debug views.";
    OMSI_FAKE_GPU_ERROR: Text, Test, Frame, "-", "Fault injection: open, open-panic, pipeline, lost-build, build, lost, frame - the GPU error to simulate. Only in builds with omsi-render's `test-hooks` feature (on by default).";
    OMSI_FIXED_SCALE: Bool, Switch, Use, "off", "Keep the render scale fixed (no dynamic resolution).";
    OMSI_FLEET_AHEAD: Num, Test, Use, "FLEET_AHEAD", "Minutes ahead the timetable fleet is planned (for tests).";
    OMSI_FLEET_IDLE: Num, Test, Use, "FLEET_IDLE", "Seconds a fleet vehicle idles (shortened for tests).";
    OMSI_FOLLOW_CAM: Text, Test, Use, "-", "right,ahead,up,yaw,pitch: a camera in the followed car's frame.";
    OMSI_FORCE_OPAQUE: Bool, Switch, Use, "off", "Draw transmap-masked materials opaque.";
    OMSI_FULL_GPU: Bool, Switch, Use, "off", "Keep the requested settings on a small or shared graphics chip.";
    OMSI_GLASS_WIND: Num, Test, Use, "-", "Offscreen: the rain on the glass as met at this speed in m/s.";
    OMSI_GL_TEXTURE_UNITS: Bool, Switch, Use, "off", "Use the texture-unit layout of the OpenGL backend on any device.";
    OMSI_GPU_ARRAYS: Text, Switch, Use, "-", "textures or nostorage: take that texture array path on any device.";
    OMSI_GPU_LIMITS: Text, Test, Use, "-", "default or downlevel: request only the WebGPU default (or downlevel) limits. Set by the small_chip test.";
    OMSI_GPU_TIMERS: Bool, Debug, Use, "off", "Measure GPU time per pass with timestamp queries.";
    OMSI_GPU_TIMERS_RAW: Bool, Debug, Use, "off", "With OMSI_GPU_TIMERS: the passes in the order the GPU finished them.";
    OMSI_GRAPHICS: Text, Tuning, Use, "settings", "vanilla, vanilla_plus or enhanced: another renderer for one run.";
    OMSI_GROUND_GAP: Text, Debug, Use, "-", "Offscreen CSV (or -): how far every drawn tyre stands over or sinks into the ground.";
    OMSI_GROUND_GAP_RADIUS: Num, Debug, Use, "150", "With OMSI_GROUND_GAP: the radius in metres.";
    OMSI_GROUND_LANES: Bool, Debug, Use, "off", "Along every street lane, every metre, how far the ground lies over or under the lane.";
    OMSI_GROUND_SAMPLE: Text, Test, Use, "-", "Offscreen CSV: what the wheels stand on every metre along the lanes near the start.";
    OMSI_HEIGHTPROFILE_GROUND: Bool, Switch, Once, "off", "The wheels stand on the splines' [heightprofile]s again (A/B).";
    OMSI_HIDE_MESH: Text, Test, Use, "-", "a|b: leave out the meshes whose file names contain one of the parts.";
    OMSI_HIDE_WINDOW: Text, Test, Use, "-", "from,to: treat the window as hidden between these seconds.";
    OMSI_HOLE_PHOTO: Bool, Test, Use, "off", "With OMSI_ROAD_PHOTO: photograph from above down to 25 m under the lane (holes in the world).";
    OMSI_IBIS_BUDGET: Num, Tuning, Use, "10", "Seconds the IBIS typist may take per entry.";
    OMSI_INPUT: Text, Test, Use, "-", "Scripted keyboard, mouse and camera input for window runs.";
    OMSI_INSTANCE: Text, Setup, Use, "-", "The id of a game instance started by the launcher (set for the child process).";
    OMSI_INTEL_FULL_GPU: Bool, Switch, Use, "off", "Keep the requested settings on an Intel Vulkan adapter.";
    OMSI_JOINT_ANGLE: Num, Test, Use, "-", "Degrees: the rear section of an articulated bus held at that angle.";
    OMSI_KEEP_ALLOCATOR: Bool, Switch, Use, "off", "Skip the restart that swaps in the faster allocator at start.";
    OMSI_LANES_NEAR: Text, Test, Use, "-", "x,y,r: log the driving lanes passing there.";
    OMSI_LAN_AUDIO: Bool, Test, Use, "off", "Offscreen LAN: with the other buses' sounds.";
    OMSI_LAN_IP: Text, Setup, Use, "-", "a.b.c.d[,e.f.g.h]: LAN addresses to offer first (when the detection gets them wrong).";
    OMSI_LAN_JOIN_TIMEOUT: Num, Tuning, Use, "default", "LAN: seconds to wait when joining a game.";
    OMSI_LAN_SAY: Text, Test, Use, "-", "\"30=Hallo;45=Bye\": LAN chat lines said at these seconds of the session.";
    OMSI_LAN_TRACE: Text, Debug, Use, "-", "LAN CSV file: where the host's people are drawn.";
    OMSI_LAUNCHER: Text, Setup, Use, "-", "Program to open as the launcher instead of the built-in one.";
    OMSI_LAUNCHER_EXIT: Num, Test, Use, "-", "Launcher: close after this many seconds.";
    OMSI_LAUNCHER_INPUT: Text, Test, Use, "-", "Scripted launcher input (\"t=2 click 400,300; ...\"). Removed from the game's environment by the updater.";
    OMSI_LAUNCHER_PAGE: Text, Test, Use, "-", "Launcher: open this page (e.g. mods, drive:N).";
    OMSI_LAUNCHER_SHOT: Text, Test, Use, "-", "Launcher: secs:file.png - the window's picture into a file.";
    OMSI_LAUNCHER_SIZE: Text, Test, Use, "-", "Launcher: WxH window size.";
    OMSI_LIST_ALIGNED: Bool, Debug, Use, "off", "Log the splines aligned to the terrain.";
    OMSI_MAX_FPS: Num, Tuning, Use, "settings", "Frame rate limit for this run.";
    OMSI_METER: Text, Tuning, Use, "-", "gain,target,dark,bright,bias,night: the exposure metering.";
    OMSI_MIRROR_ENHANCED: Bool, Switch, Frame, "off", "Draw the mirrors with the enhanced shading again.";
    OMSI_MIRROR_HUD: Num, Tuning, Use, "settings", "Mirror HUD mode (number).";
    OMSI_MOBILE: Bool, Test, Use, "off", "The phone layout on a computer (launcher, screen cut-outs).";
    OMSI_MUTE: Bool, Test, Use, "off", "Mix everything as usual but play nothing.";
    OMSI_NAV_MAP: Bool, Test, Use, "off", "Open the navigation map.";
    OMSI_NAV_MAP_MPP: Num, Test, Use, "2.5", "Navigation map zoom in metres per pixel.";
    OMSI_NAV_PROBE: Text, Test, Use, "-", "x,y[,r]: the network lanes that start or end within r of that point.";
    OMSI_NAV_SCHEDULE: Bool, Test, Use, "off", "Navigation map shows the schedule.";
    OMSI_NOZCHECK_BIAS: Bool, Switch, Use, "off", "The old reading of [matl_noZcheck] (A/B).";
    OMSI_NO_ANIMPARENT: Bool, Switch, Use, "off", "Every mesh animated on its own, without [animparent] (A/B).";
    OMSI_NO_AO: Bool, Switch, Frame, "off", "No ambient occlusion pass.";
    OMSI_NO_ATTACH_FALLBACK: Bool, Switch, Use, "off", "Drop attachments that have no fallback instead of placing them (A/B).";
    OMSI_NO_BC: Bool, Switch, Use, "off", "Upload all textures as RGBA instead of keeping BC compression.";
    OMSI_NO_BOUNDS_CACHE: Bool, Switch, Use, "off", "No cache of instance bounds.";
    OMSI_NO_BRIDGE: Bool, Switch, Use, "off", "LAN: leave the internet alone - no rendezvous bridge, no tunnel (tests, LAN parties).";
    OMSI_NO_BUMP: Bool, Switch, Use, "off", "No bump maps.";
    OMSI_NO_BUNDLES: Bool, Switch, Frame, "off", "Record the main pass directly instead of from render bundles.";
    OMSI_NO_CLOUDS: Bool, Switch, Use, "off", "No clouds.";
    OMSI_NO_CORONAS: Bool, Switch, Frame, "off", "No light coronas.";
    OMSI_NO_CULL: Bool, Switch, Once, "off", "Draw every mesh from both sides (A/B).";
    OMSI_NO_ENHANCED: Bool, Switch, Frame, "off", "Enhanced settings drawn with the plain path.";
    OMSI_NO_ENVMAP: Bool, Switch, Use, "off", "No environment maps.";
    OMSI_NO_FXAA: Bool, Switch, Frame, "off", "No FXAA.";
    OMSI_NO_GLARE: Bool, Switch, Frame, "off", "No sun glare.";
    OMSI_NO_GLASS_PICTURE: Bool, Switch, Frame, "off", "No picture behind the glass (rain films).";
    OMSI_NO_GROUND_PAINT: Bool, Switch, Use, "off", "No painted ground layers.";
    OMSI_NO_GROUND_SPLINE_BATCHING: Bool, Switch, Use, "off", "No batching of ground splines.";
    OMSI_NO_INTERP: Bool, Switch, Once, "off", "The old way without interpolation, for comparing.";
    OMSI_NO_LAN_MODS: Bool, Switch, Use, "off", "LAN: do not exchange mods.";
    OMSI_NO_LIGHT_MAP: Bool, Switch, Use, "off", "No night light maps on the tiles (A/B).";
    OMSI_NO_MAIN_SPLIT: Bool, Switch, Frame, "off", "Do not split the main pass's bundles in two.";
    OMSI_NO_MATERIAL_SPLINE_BATCHING: Bool, Switch, Use, "off", "No batching of splines by material.";
    OMSI_NO_MODEL_ORDER: Bool, Switch, Use, "off", "Draw the opaque parts of ordered models first again (A/B).";
    OMSI_NO_MSAA_PREPASS: Bool, Switch, Frame, "off", "No depth prepass with multisampling.";
    OMSI_NO_PBR: Bool, Switch, Use, "off", "No PBR materials.";
    OMSI_NO_PLUGINS: Bool, Switch, Use, "off", "No plugins loaded.";
    OMSI_NO_POLL_THREAD: Bool, Switch, Use, "off", "No device poll thread.";
    OMSI_NO_PRESENCE: Bool, Switch, Use, "off", "No presence (\"playing now\") service.";
    OMSI_NO_PUDDLE_GLASS_DEPTH: Bool, Switch, Use, "off", "Puddles without glass depth.";
    OMSI_NO_PUDDLE_REFLECTIONS: Bool, Switch, Frame, "off", "No puddle reflections.";
    OMSI_NO_PUDDLE_VEHICLE: Bool, Switch, Use, "off", "No vehicles in puddle reflections.";
    OMSI_NO_RENDER_POOL: Bool, Switch, Use, "off", "No encoding thread pool for rendering.";
    OMSI_NO_RT: Bool, Switch, Use, "off", "No ray queries (Enhanced+ draws as Enhanced).";
    OMSI_NO_RT_FRAME: Bool, Switch, Frame, "off", "Ray queries kept but nothing traced.";
    OMSI_NO_RT_GRADE: Bool, Switch, Frame, "off", "Enhanced+ without its colour grade.";
    OMSI_NO_SHADOWS: Bool, Switch, Use, "off", "No shadows.";
    OMSI_NO_SMOKE: Bool, Switch, Frame, "off", "No smoke.";
    OMSI_NO_SNOWFALL: Bool, Switch, Frame, "off", "No snowfall.";
    OMSI_NO_SPLINE_BATCHING: Bool, Switch, Use, "off", "No spline batching.";
    OMSI_NO_SPLINE_HOLES: Bool, Switch, Use, "off", "Splines without hole rims.";
    OMSI_NO_SPRAY: Bool, Switch, Use, "off", "No tyre spray.";
    OMSI_NO_SURF: Bool, Switch, Once, "off", "Every road as smooth as before (no surface texture) (A/B).";
    OMSI_NO_TEXCOMPRESS: Bool, Switch, Use, "off", "No runtime texture compression.";
    OMSI_NO_TUNNEL: Bool, Switch, Use, "off", "LAN: no tunnel.";
    OMSI_NO_UPDATE: Bool, Switch, Use, "off", "No update check.";
    OMSI_NO_VAR_SYNC: Bool, Switch, Use, "off", "LAN: no variable sync.";
    OMSI_NO_WHEEL_SLIP: Bool, Switch, Use, "off", "Every wheel grips, as before wheels turned on their own (A/B).";
    OMSI_OFFICIAL_KEY: Text, Setup, Use, "-", "File of the official server's signing key.";
    OMSI_OLD_WORLD_GRID: Bool, Switch, Use, "off", "Maps with world coordinates take the one tile size of 371.9 m (comparison).";
    OMSI_ONLY_MESH: Text, Test, Use, "-", "a|b: draw only the meshes whose file names contain one of the parts.";
    OMSI_ONLY_OBJECT: Text, Test, Use, "-", "Load only the objects whose file name contains this text.";
    OMSI_ONLY_SURFACES: Bool, Test, Frame, "off", "Draw only the surfaces.";
    OMSI_OPENXR: Bool, Tuning, Use, "off", "Start in OpenXR (VR) mode.";
    OMSI_OPENXR_MIRROR_RATE: Num, Tuning, Use, "settings", "VR: mirror redraw rate (negative: every mirror each frame).";
    OMSI_OPENXR_SCALE: Num, Tuning, Use, "settings", "VR: render scale of the eyes.";
    OMSI_ORBIT_DIST: Num, Test, Use, "18", "Offscreen: the outside camera's distance from the vehicle in metres.";
    OMSI_ORIGINAL: Text, Test, Test, "-", "vehicle_vars example: the original OMSI folder.";
    OMSI_PARKED_PULL_OUT: Num, Tuning, Use, "0.035", "Chance per step a parked car pulls out.";
    OMSI_PARK_IN: Num, Tuning, Use, "0.04", "Chance per step a car parks.";
    OMSI_PAX_CAM: Num, Test, Use, "-", "Offscreen --view pax: the n-th passenger camera.";
    OMSI_PAX_CROSS: Text, Test, Use, "-", "x,y: send pedestrians across the signalised crossing nearest that point.";
    OMSI_PAX_WAITING: Num, Test, Use, "-", "Number of passengers waiting at each stop.";
    OMSI_PLUGIN_HOST32: Text, Setup, Use, "beside the game", "Path of omsi-plugin-host32.exe.";
    OMSI_POPULATION_SHOTS: Bool, Test, Use, "off", "Offscreen: pictures of the framed population spawns.";
    OMSI_PRESENCE_URL: Text, Setup, Use, "built-in", "Base URL of the presence (\"playing now\") service.";
    OMSI_PROBE: Text, Test, Use, "-", "x0,y0,x1,y1[,n]: print terrain and road surface heights along a line.";
    OMSI_PROBE_GRID: Text, Test, Use, "-", "x,y,half,step: the wheels' ground on a grid around a point.";
    OMSI_PROFILE: Bool, Debug, Use, "off", "Time per stage of the frame, logged.";
    OMSI_PROFILE_GPU: Bool, Debug, Use, "off", "Profile: wait for the GPU so its time shows as a stage of its own.";
    OMSI_PROFILE_JSON: Text, Debug, Use, "-", "With OMSI_PROFILE and --exit-after: the exit summary (after the 15 s warm-up) as JSON into this file, see scripts/compare-performance.py.";
    OMSI_PUDDLE_F0: Num, Tuning, Use, "0.08", "Puddle reflectance at normal incidence (0.02 to 0.2).";
    OMSI_PUDDLE_THICKNESS: Num, Tuning, Use, "0.12", "Puddle water film thickness.";
    OMSI_RENDER_CLOCK: Num, Test, Use, "0", "Seconds the animation clock starts on (offscreen pictures).";
    OMSI_RENDER_OCCLUDED: Bool, Test, Use, "off", "Draw frames into a texture while the window is hidden (macOS gives none).";
    OMSI_REPAIR_BODY_DEPTH: Bool, Switch, Use, "off", "The old guess for [matl_alpha] 2 vehicle bodies (A/B).";
    OMSI_ROAD_CUT: Bool, Switch, Once, "off", "Take the ground away under every road surface (Omsi.exe does not).";
    OMSI_ROAD_PHOTO: Bool, Test, Use, "off", "Map check: photograph the road network from above and report grass where a carriageway should be.";
    OMSI_ROAD_PHOTO_N: Num, Test, Use, "400", "With OMSI_ROAD_PHOTO: number of sample points.";
    OMSI_ROAD_PHOTO_SIDE: Num, Test, Use, "0", "With OMSI_ROAD_PHOTO: metres to either side of the carriageway.";
    OMSI_ROAD_PHOTO_SLANT: Num, Test, Use, "-", "With OMSI_ROAD_PHOTO: from a driver's eye this far back instead of from above.";
    OMSI_ROOT: Text, Setup, Use, "found", "The OMSI 2 installation folder (also the content root for tests that need real content).";
    OMSI_RT_REFL_HALF: Bool, Switch, Use, "off", "Trace reflections at half size.";
    OMSI_SAFE_GPU: Num, Setup, Use, "0", "Restarts after a lost graphics device: lighter on the card each time. Set at runtime on Android and by the restart.";
    OMSI_SEED: Num, Test, Use, "random", "Seed of the scripts' random numbers (repeat a session).";
    OMSI_SHADOW_FAR_EVERY_FRAME: Bool, Switch, Frame, "off", "Redraw the far shadow map every frame.";
    OMSI_SHADOW_NEAR_EVERY_FRAME: Bool, Switch, Frame, "off", "Redraw the near shadow map every frame.";
    OMSI_SKIP_OBJECT: Text, Test, Use, "-", "Leave out the objects whose file name contains this text.";
    OMSI_SKIP_PIPE: Text, Test, Frame, "-", "Pipeline kinds to leave out of the main pass (comma-separated numbers).";
    OMSI_SPOT_SELECT: Num, Test, Use, "-", "Turn on spotlight n (headlight pictures).";
    OMSI_SURFACE_BIAS: Num, Tuning, Use, "-24", "Depth bias of road surfaces.";
    OMSI_SURFACE_FLUSH: Num, Tuning, Once, "0.12", "Height in metres under which a surface counts as flush with the road.";
    OMSI_SUSP_TRACE: Text, Debug, Use, "-", "Offscreen CSV: body height and each wheel's travel and load every frame.";
    OMSI_SUSP_TRACE_WINDOW: Text, Debug, Use, "-", "Window CSV: each wheel's travel every frame.";
    OMSI_TERRAIN_ALIGN: Bool, Switch, Use, "off", "Align the terrain to the splines again at load (A/B; Omsi.exe does not).";
    OMSI_TEST_CONTENT: Text, Test, Test, "-", "Content root for tests that need real OMSI content.";
    OMSI_TEST_WINE_DIR: Text, Test, Test, "-", "Plugin demo test: folder holding omsi-plugin-host.exe for Wine.";
    OMSI_TEXTURE_MEMORY: Num, Tuning, Use, "settings", "Texture budget in MB.";
    OMSI_TEXTURE_RAIN: Bool, Switch, Use, "off", "OMSI 2's own texture rain on the glass instead of the drops.";
    OMSI_TONE_CONTRAST: Text, Tuning, Once, "1", "day[,night]: tone mapping contrast.";
    OMSI_TOUCH: Bool, Test, Use, "off", "The on-screen touch controls on a computer.";
    OMSI_TRACE_AI: Text, Debug, Use, "-", "CSV: every AI car's pose, steering and speed every frame.";
    OMSI_TRACE_AI_BUSES: Bool, Debug, Use, "off", "With OMSI_TRACE_AI: the timetable buses only.";
    OMSI_TRACE_FFB: Text, Debug, Use, "-", "CSV: the force feedback frame by frame (Windows): the wheel's position and the force sent.";
    OMSI_TRACE_PAX: Text, Debug, Use, "-", "File: trace the passengers.";
    OMSI_TRACE_REMOTE: Text, Debug, Use, "-", "LAN CSV: where each other player's bus is drawn every frame.";
    OMSI_TRACE_STEER: Text, Debug, Use, "-", "CSV: the mouse steering frame by frame.";
    OMSI_TRACE_VARS: Text, Debug, Use, "-", "a,b,$c: the listed variables every half second of the run.";
    OMSI_TRACKIR_NATIVE: Text, Switch, Use, "on", "0 turns the native TrackIR interface off.";
    OMSI_TRAFFIC_ALL_GROUPS: Bool, Switch, Use, "off", "Let restricted AI groups (aircraft, depot fleets) drive everywhere.";
    OMSI_TRIGGER_ALL: Bool, Test, Use, "off", "Check: fire every [mouseevent] of the model and compare the variables before and after.";
    OMSI_TYRE_SUSPENSION: Bool, Switch, Once, "off", "The old suspension with wheel mass, tyre and bump stops (A/B).";
    OMSI_UI_PREVIEW: Text, Test, Use, "target/ui-preview.png", "Plugin UI preview test: the picture's path.";
    OMSI_UPDATE_URL: Text, Setup, Use, "built-in", "Another release description (URL or file:///...json) for the update check.";
    OMSI_WARM_FRAMES: Num, Test, Use, "0", "Offscreen: frames drawn before the picture.";
    OMSI_WATCH_VARS: Text, Debug, Use, "-", "a,b: log every change of these variables of the player's bus.";
    OMSI_WETNESS: Num, Test, Use, "weather", "Ground wetness the picture is drawn with.";
    OMSI_WHEEL_TRACE: Bool, Debug, Use, "off", "Log the deepest a drawn tyre goes into the road, once a second.";
    OMSI_WINDY_TREES: Text, Tuning, Use, "settings", "0 or 1: trees in the wind off or on (A/B renders).";
    OMSI_WINE: Text, Setup, Use, "PATH", "The Wine binary for Windows plugins.";
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "target") {
                    rust_files(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// Every `OMSI_*` name the workspace's Rust code names outside comments - a string
    /// literal `"OMSI_X"` or a flag `flags::OMSI_X` - with the crates it is in. This file is
    /// left out: it is the list.
    fn names_in_source() -> BTreeMap<String, BTreeSet<String>> {
        let mut files = Vec::new();
        let crates = repo().join("crates");
        rust_files(&crates, &mut files);
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for f in files {
            let rel = f.strip_prefix(&crates).unwrap_or(&f);
            if rel == Path::new("omsi-cfg/src/flags.rs") {
                continue;
            }
            let krate = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
            for line in std::fs::read_to_string(&f).unwrap_or_default().lines() {
                let code = code_part(line);
                for (pat, close) in [("\"OMSI_", true), ("flags::OMSI_", false)] {
                    let mut rest = code;
                    while let Some(i) = rest.find(pat) {
                        let name = &rest[i + pat.len() - 5..];
                        let end = name.find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')).unwrap_or(name.len());
                        if end > 5 && (!close || name[end..].starts_with('"')) {
                            out.entry(name[..end].to_string()).or_default().insert(krate.clone());
                        }
                        rest = &name[end..];
                    }
                }
            }
        }
        out
    }

    /// A line without its `//` comment (a `//` inside a string literal stays).
    fn code_part(line: &str) -> &str {
        let mut in_str = false;
        let mut prev = ' ';
        for (i, c) in line.char_indices() {
            match c {
                '"' if prev != '\\' => in_str = !in_str,
                '/' if !in_str && prev == '/' => return &line[..i - 1],
                _ => {}
            }
            prev = if prev == '\\' && c == '\\' { ' ' } else { c };
        }
        line
    }

    fn markdown(crates: &BTreeMap<String, BTreeSet<String>>) -> String {
        let mut s = String::from(
            "# OMSI_* environment variables\n\n\
             Generated from `crates/omsi-cfg/src/flags.rs` - edit the list there and run\n\
             `OMSI_FLAGS_BLESS=1 cargo test -p omsi-cfg flags` to write this file again.\n\n\
             - **Type** - `bool`: on when set to anything (`0` and empty included); `num`: the whole value parsed as a number;\n  \
             `text`: a path, a list or a mode.\n\
             - **Kind** - `debug`: more logging, dumps and traces only; `test`: drives a test, check pass or offscreen run, or injects\n  \
             a fault; `switch`: A/B or kill switch; `tuning`: a real setting for one run; `setup`: folders, programs, URLs;\n  \
             `build`: read when compiling.\n\
             - **Read** - `once` per process; `use`: each time the code passing it runs (start-up, loading, per step);\n  \
             `frame`: every frame in `Renderer::render_inner`; `test`: tests and examples only.\n\n\
             Values are read once per process and cached (`omsi_cfg::env`): changing the environment of a running game has no\n\
             effect. On Android, `openOMSI/env.txt` holds `NAME=value` lines that are set before the game starts.\n\n",
        );
        let kinds = [(Kind::Debug, "Debug output"), (Kind::Test, "Test hooks"), (Kind::Switch, "A/B and kill switches"), (Kind::Tuning, "Tuning"), (Kind::Setup, "Set-up"), (Kind::Build, "Build")];
        for (kind, title) in kinds {
            s += &format!("## {title}\n\n| Name | Type | Default | Read | Crates | Description |\n|---|---|---|---|---|---|\n");
            for f in ALL.iter().filter(|f| f.kind == kind) {
                let c = crates.get(f.name).map(|c| c.iter().map(|k| k.trim_start_matches("omsi-")).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let ty = format!("{:?}", f.ty).to_lowercase();
                let read = format!("{:?}", f.read).to_lowercase();
                s += &format!("| `{}` | {ty} | {} | {read} | {c} | {} |\n", f.name, f.default.replace('|', "\\|"), f.doc.replace('|', "\\|"));
            }
            s += "\n";
        }
        s
    }

    #[test]
    fn flags_sorted_and_unique() {
        for w in ALL.windows(2) {
            assert!(w[0].name < w[1].name, "{} before {}", w[0].name, w[1].name);
        }
        assert!(find("OMSI_MUTE").is_some_and(|f| f.name == "OMSI_MUTE"));
        assert!(find("OMSI_NOT_A_FLAG").is_none());
    }

    #[test]
    fn flags_match_source() {
        let src = names_in_source();
        let missing: Vec<&String> = src.keys().filter(|n| find(n).is_none()).collect();
        assert!(missing.is_empty(), "OMSI_* names in the code but not in omsi_cfg::flags (add them there): {missing:?}");
        let unused: Vec<&str> = ALL.iter().map(|f| f.name).filter(|n| !src.contains_key(*n)).collect();
        assert!(unused.is_empty(), "flags no code names any more (remove them from omsi_cfg::flags): {unused:?}");
    }

    #[test]
    fn flags_doc_up_to_date() {
        let want = markdown(&names_in_source());
        let path = repo().join("docs/DEBUG_FLAGS.md");
        if std::env::var_os("OMSI_FLAGS_BLESS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(have == want, "docs/DEBUG_FLAGS.md is out of date: OMSI_FLAGS_BLESS=1 cargo test -p omsi-cfg flags");
    }

    #[test]
    fn cached_as_env() {
        // the flag reads through `env`'s cache: what `env` saw first is what the flag sees
        std::env::set_var("OMSI_FLAGS_TEST_PROBE", "2.5");
        let f = Flag::new("OMSI_FLAGS_TEST_PROBE", Ty::Num, Kind::Test, Read::Test, "-", "");
        let f: &'static Flag = Box::leak(Box::new(f));
        assert_eq!(crate::env::var("OMSI_FLAGS_TEST_PROBE").as_deref(), Ok("2.5"));
        std::env::set_var("OMSI_FLAGS_TEST_PROBE", "7");
        assert!(f.is_set());
        assert_eq!(f.var(), Some("2.5"));
        assert_eq!(f.parse::<f32>(), Some(2.5));
        assert_eq!(f.parse::<u32>(), None);
        assert_eq!(f.live_var().as_deref(), Ok("7"));
        std::env::remove_var("OMSI_FLAGS_TEST_PROBE");
    }
}
