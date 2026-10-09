# OMSI_* environment variables

Generated from `crates/omsi-cfg/src/flags.rs` - edit the list there and run
`OMSI_FLAGS_BLESS=1 cargo test -p omsi-cfg flags` to write this file again.

- **Type** - `bool`: on when set to anything (`0` and empty included); `num`: the whole value parsed as a number;
  `text`: a path, a list or a mode.
- **Kind** - `debug`: more logging, dumps and traces only; `test`: drives a test, check pass or offscreen run, or injects
  a fault; `switch`: A/B or kill switch; `tuning`: a real setting for one run; `setup`: folders, programs, URLs;
  `build`: read when compiling.
- **Read** - `once` per process; `use`: each time the code passing it runs (start-up, loading, per step);
  `frame`: every frame in `Renderer::render_inner`; `test`: tests and examples only.

Values are read once per process and cached (`omsi_cfg::env`): changing the environment of a running game has no
effect. On Android, `openOMSI/env.txt` holds `NAME=value` lines that are set before the game starts.

## Debug output

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_BENCH_FRAMES` | bool | off | use | app | With OMSI_BENCH: log every bench frame's times, not only the medians. |
| `OMSI_DEBUG_AI_WIDE` | bool | off | use | sim | Log every tenth of a second an AI car standing over 1.5 m beside its way. |
| `OMSI_DEBUG_ANIM` | text | - | once | sim | Log the animations of the meshes whose file name contains this text. |
| `OMSI_DEBUG_BOARDS` | bool | off | use | sim | Log boarding at bus stops. |
| `OMSI_DEBUG_CAMERA` | num | 1 | once | app | Camera arm log: 1 the types and what stops the arm, 2 also every frame. |
| `OMSI_DEBUG_CAR` | num | - | use | sim | Log the state of the AI car with this id every step. |
| `OMSI_DEBUG_CAREER` | bool | off | use | app | Log the jolts the career scoring counts. |
| `OMSI_DEBUG_COLLISION` | bool | off | use | app | Log the player's collision state. |
| `OMSI_DEBUG_CONES` | bool | off | use | app, render | Log the light coronas and beams prepared per frame. |
| `OMSI_DEBUG_CULL` | num | off | frame | render | Log what near and in the picture is left out of the draw list, and why. |
| `OMSI_DEBUG_DOORS` | bool | off | use | sim | Log the door variables of AI buses twice a second. |
| `OMSI_DEBUG_DRAWS` | bool | off | frame | render | Log how many instances changed per prepare. |
| `OMSI_DEBUG_DRIVER` | bool | off | use | app | Log the driver figure's settling (grips, wrists, elbows). |
| `OMSI_DEBUG_ENHANCED` | num | 0 | once | render | Enhanced main pass shows one of its terms alone (n selects it: 1 sun shadow, 2 occlusion, ...). |
| `OMSI_DEBUG_EXPOSURE` | bool | off | use | render | Read back and log the adapted exposure metering now and then. |
| `OMSI_DEBUG_FLICKER` | bool | off | frame | render | Log near instances drawn in one frame and not the next (objects blinking). |
| `OMSI_DEBUG_FLOAT` | bool | off | use | app | Log objects floating over the ground while placing them. |
| `OMSI_DEBUG_FOG_LAMPS` | bool | off | frame | render | Log the fog lamp passes per frame. |
| `OMSI_DEBUG_FOOT` | bool | off | use | app | Log where the walking avatar is drawn. |
| `OMSI_DEBUG_HUMANS` | bool | off | use | app, sim | Log the people (passengers, pedestrians) and their placement. |
| `OMSI_DEBUG_IBIS` | bool | off | use | sim | Log the IBIS typist's reasoning. |
| `OMSI_DEBUG_INTERIOR` | bool | off | use | app | Log the interior lamps of each vehicle type. |
| `OMSI_DEBUG_JUNCTION` | bool | off | use | sim | Log why AI cars wait at a junction. |
| `OMSI_DEBUG_LAMPS` | bool | off | use | app | Log traffic lights without a crossing, and the moving lamps (barriers). |
| `OMSI_DEBUG_LAN` | bool | off | use | app | LAN: every few seconds what we know of every player, counts and bytes a second. |
| `OMSI_DEBUG_LANES` | text | - | use | app | Log the lanes with these ids (comma-separated). |
| `OMSI_DEBUG_LIGHT` | bool | off | use | app | Add a test point light above the camera. |
| `OMSI_DEBUG_LIGHTS` | text | - | use | sim | all, near or controller names: log every light change of those traffic light programs. |
| `OMSI_DEBUG_LIGHT_GRID` | bool | off | use | render | Log lights left out of a full light-grid cell. |
| `OMSI_DEBUG_MESHES` | text | off | use | app | Log per material slot which part of its texture a mesh shows (display texts). |
| `OMSI_DEBUG_MIRRORS` | bool | off | use | app | Log each mirror's eye, angles and field of view. |
| `OMSI_DEBUG_MISSING` | bool | off | use | app | Log where missing textures were looked for. |
| `OMSI_DEBUG_NAN` | bool | off | once | script | Warn at every write of a NaN or infinity into a script variable. |
| `OMSI_DEBUG_NAV` | bool | off | use | app | Log navigation details (e.g. stops whose tiles are not loaded). |
| `OMSI_DEBUG_OBJECTS` | bool | off | use | app | Log objects placed outside their tile. |
| `OMSI_DEBUG_OBJMAT` | text | - | use | app | Log how the material slots of the objects whose file name contains this text are made. |
| `OMSI_DEBUG_PARTICLES` | bool | off | use | app | Log the smoke particles. |
| `OMSI_DEBUG_PASS` | bool | off | use | sim | Log twice a second why a car standing behind something does not go round it. |
| `OMSI_DEBUG_PAX` | bool | off | use | sim | Log every change of state of the passengers. |
| `OMSI_DEBUG_PHYSICS` | num | off | use | app, sim | Log the player's pose, speed, wheel contacts and crashes (offscreen: =secs, the interval). |
| `OMSI_DEBUG_POPULATION` | bool | off | use | sim | Log where cars appear and vanish relative to the view. |
| `OMSI_DEBUG_PROPS` | bool | off | use | app | Log the destination-display (matrix) variables. |
| `OMSI_DEBUG_PUDDLES` | num | 0 | use | render | Puddle debug view (a number selects it). |
| `OMSI_DEBUG_RAIN` | bool | off | use | app | Log the tyres' spray and puddles. |
| `OMSI_DEBUG_RASTER` | text | - | use | app | x,y: log the ground raster at that point. |
| `OMSI_DEBUG_REPEATERS` | bool | off | use | app | Log repeater objects whose spline chain disagrees with the map. |
| `OMSI_DEBUG_REST` | bool | off | use | app | Log where the bus came to rest on the ground after spawning. |
| `OMSI_DEBUG_ROUTES` | bool | off | use | sim | Log where consecutive lanes of an AI route do not join. |
| `OMSI_DEBUG_RT` | num | off | use | render | Log the ray tracing structures every 30 frames (with a number: a debug view). |
| `OMSI_DEBUG_SEAT` | bool | off | use | sim | Log the driver seat's suspension meshes. |
| `OMSI_DEBUG_SERVICES` | bool | off | use | app | Log petrol station boxes and the bus's distance to them. |
| `OMSI_DEBUG_SHADOW` | num | off | frame | render | Shadow debug overlay (with a number: its radius, default 3). |
| `OMSI_DEBUG_SHADOW_FAR` | bool | off | frame | render | Log each redraw of the far shadow map. |
| `OMSI_DEBUG_SIGNALS` | bool | off | use | app | Log what each railway signal shows. |
| `OMSI_DEBUG_SKY` | bool | off | frame | render | Log the sky and sun values per frame. |
| `OMSI_DEBUG_SOUND` | num | 5 | once | app | Log which sounds were heard (number: the interval in seconds). |
| `OMSI_DEBUG_SPLINES` | bool | off | use | app | Log the splines and lanes built per tile. |
| `OMSI_DEBUG_STARTUP` | bool | off | use | sim | Log the start-up steps of a vehicle's systems. |
| `OMSI_DEBUG_STOPS` | bool | off | use | sim | Log the bus stops a bus missed. |
| `OMSI_DEBUG_STUCK` | bool | off | use | app, sim | Log the report of stuck AI cars (also explains junction waits). |
| `OMSI_DEBUG_SURFACES` | bool | off | use | app | Log the painted ground layers per tile. |
| `OMSI_DEBUG_SWITCHES` | bool | off | use | app | Log railway switches thrown for a train. |
| `OMSI_DEBUG_TEXT` | bool | off | once | sim | Log the scripts' string variable operations. |
| `OMSI_DEBUG_TEXTURES` | bool | off | use | app | Log all textures by size and format. |
| `OMSI_DEBUG_TRAFFIC` | bool | off | use | app, sim | Log traffic light programs, cars standing for long and hard bends. |
| `OMSI_DEBUG_TRAILER` | bool | off | use | sim | Log a trailer's rest, sag and lift on the ground. |
| `OMSI_DEBUG_TRAILERS` | bool | off | use | sim | Log coupled parts off the level of what pulls them. |
| `OMSI_DEBUG_TRIGGERS` | bool | off | use | app | Log the variables a mouse trigger changed. |
| `OMSI_DEBUG_UNLINKED` | bool | off | use | sim | Log lane ends with an untaken lane start near them. |
| `OMSI_DEBUG_UPLOAD` | bool | off | use | app | Log slow GPU uploads item by item. |
| `OMSI_DEBUG_VARIANTS` | text | - | use | app | Log material variants whose variable name contains this text. |
| `OMSI_DEBUG_VARS` | num | - | use | app | Offscreen: log these script variables of the player's bus (comma-separated). |
| `OMSI_DEBUG_VARS_EVERY` | num | - | use | app | With OMSI_DEBUG_VARS: log them every this many seconds through the drive. |
| `OMSI_DEBUG_VAR_SYNC` | text | - | use | app | LAN: log this variable, ours every two seconds and theirs as it comes. |
| `OMSI_DEBUG_VIEW_LAMPS` | bool | off | frame | render | Log the view lamps value per frame. |
| `OMSI_DEBUG_WALLS` | bool | off | use | sim | Log the walls a walking player slides along. |
| `OMSI_DEBUG_WARP` | bool | off | use | app | Log crossings whose ground was moved by more than a metre. |
| `OMSI_DEBUG_WHEELS` | bool | off | use | app | Log where the wheel meshes of every vehicle type turn. |
| `OMSI_DUMP_CUT` | text | - | use | app | Directory: write each tile's road-cut alpha masks as images. |
| `OMSI_DUMP_GROUND` | text | - | use | app | File: every loaded tile's final terrain and road cut as text. |
| `OMSI_DUMP_SCENERY_TEXT` | text | - | use | app | Directory: write the scenery objects' text textures as drawn. |
| `OMSI_DUMP_SCRIPTTEX` | text | - | use | app | Directory: write the player's bus's script (display) textures. |
| `OMSI_ENV_PHOTO` | num | 1 | use | render | 0 leaves the environment photo out of the debug views. |
| `OMSI_GPU_TIMERS` | bool | off | use | render | Measure GPU time per pass with timestamp queries. |
| `OMSI_GPU_TIMERS_RAW` | bool | off | use | render | With OMSI_GPU_TIMERS: the passes in the order the GPU finished them. |
| `OMSI_GROUND_GAP` | text | - | use | app | Offscreen CSV (or -): how far every drawn tyre stands over or sinks into the ground. |
| `OMSI_GROUND_GAP_RADIUS` | num | 150 | use | app | With OMSI_GROUND_GAP: the radius in metres. |
| `OMSI_GROUND_LANES` | bool | off | use | app | Along every street lane, every metre, how far the ground lies over or under the lane. |
| `OMSI_LAN_TRACE` | text | - | use | app | LAN CSV file: where the host's people are drawn. |
| `OMSI_LIST_ALIGNED` | bool | off | use | app | Log the splines aligned to the terrain. |
| `OMSI_PROFILE` | bool | off | use | app, render, sim | Time per stage of the frame, logged. |
| `OMSI_PROFILE_GPU` | bool | off | use | app | Profile: wait for the GPU so its time shows as a stage of its own. |
| `OMSI_PROFILE_JSON` | text | - | use | app | With OMSI_PROFILE and --exit-after: the exit summary (after the 15 s warm-up) as JSON into this file, see scripts/compare-performance.py. |
| `OMSI_SUSP_TRACE` | text | - | use | app | Offscreen CSV: body height and each wheel's travel and load every frame. |
| `OMSI_SUSP_TRACE_WINDOW` | text | - | use | app | Window CSV: each wheel's travel every frame. |
| `OMSI_TRACE_AI` | text | - | use | sim | CSV: every AI car's pose, steering and speed every frame. |
| `OMSI_TRACE_AI_BUSES` | bool | off | use | sim | With OMSI_TRACE_AI: the timetable buses only. |
| `OMSI_TRACE_FFB` | text | - | use | app | CSV: the force feedback frame by frame (Windows): the wheel's position and the force sent. |
| `OMSI_TRACE_PAX` | text | - | use | app | File: trace the passengers. |
| `OMSI_TRACE_REMOTE` | text | - | use | app | LAN CSV: where each other player's bus is drawn every frame. |
| `OMSI_TRACE_STEER` | text | - | use | app | CSV: the mouse steering frame by frame. |
| `OMSI_TRACE_VARS` | text | - | use | app | a,b,$c: the listed variables every half second of the run. |
| `OMSI_WATCH_VARS` | text | - | use | app | a,b: log every change of these variables of the player's bus. |
| `OMSI_WHEEL_TRACE` | bool | off | use | app | Log the deepest a drawn tyre goes into the road, once a second. |

## Test hooks

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_AUDIT_LINE` | num | - | test | sim | bus_audit example: the line (number and letter code) the IBIS typist enters instead of the bus's own. |
| `OMSI_AUTOPILOT` | num | - | use | app | Offscreen: the player's bus follows the lanes at this speed in km/h (finds where it falls through or leaves the road). |
| `OMSI_BACKGROUND` | bool | off | use | app | A test window that does not take the keyboard focus (OMSI_INPUT drives the handlers directly). |
| `OMSI_BATCH` | num | - | use | app | Offscreen: prepare the map tiles this many at a time, as the window's streaming does. |
| `OMSI_BENCH` | num | - | use | app | Offscreen: draw the final picture this many more times and log the median CPU and GPU-wait time. |
| `OMSI_BLEND_AB` | bool | off | use | app | Offscreen: a second picture with the blended draws in the old order, for a before/after of the draw order. |
| `OMSI_BRIDGE_ONLY` | bool | off | use | net | LAN: forget our own addresses so that only what the rendezvous bridge reports is tried. |
| `OMSI_BUDGET_FROM` | text | - | use | app | Offscreen with OMSI_TEXTURE_MEMORY: x,y[,MB] - the texture budget is met as seen from that point. |
| `OMSI_CAM_VEHICLE` | text | - | use | app | Offscreen: x,y,z,yaw,pitch[,fov] - a camera in the bus's own frame. |
| `OMSI_CHECK_ENTRIES` | bool | off | use | app | Map check: is there ground to stand on where a player is put down. |
| `OMSI_CHECK_GROUND` | bool | off | use | sim | Check: people on foot with a walkable surface above their heads, every two seconds. |
| `OMSI_CHECK_OBJECTS` | bool | off | use | app | Map check: report placed objects that float or are out of place while loading tiles. |
| `OMSI_CHECK_OBSTACLES` | bool | off | use | app | Map check: sweep a bus-sized box along every lane and list the obstacle boxes it hits. |
| `OMSI_CHECK_OVERLAP` | bool | off | use | sim | Check: every AI vehicle whose body has got into another's or into an obstacle. |
| `OMSI_CHECK_ROADS` | bool | off | use | app | Map check: lanes with no road surface drawn under them, and road points under the ground. |
| `OMSI_CHECK_SPIKES` | bool | off | use | app | Map check: road faces standing taller than profile, gradient and cant allow. |
| `OMSI_CHECK_SPLINES` | bool | off | use | app | Map check: chained splines whose ends do not meet in height. |
| `OMSI_CHECK_TPOSE` | bool | off | use | app | Check: people drawn in the rest (T) pose, wider than 1.3 m hand to hand. |
| `OMSI_CHECK_TRIPS` | text | - | use | app, sim | 1: build every trip's route on the loaded lanes and report breaks; a trip name: log every step of that trip. |
| `OMSI_CHECK_TYPES` | bool | off | use | app | Map check: list the object types per folder that loading leaves out. |
| `OMSI_CHECK_WALLS` | bool | off | use | sim | Check: everybody inside a bus who stands away from its walkways. |
| `OMSI_CHECK_WHEELS` | bool | off | use | app | Map check: probe the ground along every lane's wheel tracks (invisible walls, humps, holes). |
| `OMSI_CHURN` | text | - | use | app | Offscreen: x,y - load the tiles round that point and unload/reload the start area (streaming test). |
| `OMSI_CLICK_ALL` | bool | off | use | app | Check: aim at every clickable item of the cab and report which one would actually be operated. |
| `OMSI_CLOUDFLARED_OWN` | bool | off | use | net | LAN tunnel: only the game's own downloaded cloudflared, to test the download. |
| `OMSI_CONDENSATION` | text | - | use | app | Offscreen: minutes,people[,engine] - the cabin air and condensation on the glass after that long. |
| `OMSI_DAY_AIR` | text | - | once | render | haze,angstrom[,height,strat]: fix the day's air (twilight colours) instead of the varying one. |
| `OMSI_DRIVER_HANDS` | text | - | use | app | left,right: both driver hands held at these angles (grip close-ups). |
| `OMSI_DRIVER_SHIFTER` | text | - | use | app | Part of a variable or file name: force the shifter the driver's hand uses; off: both hands on the wheel. |
| `OMSI_DRIVE_PROFILE` | text | - | use | app | Offscreen: "t throttle brake [steer]/..." piecewise constant pedals (crash/kerb tests). |
| `OMSI_DRIVE_V0` | num | - | use | app | Offscreen: the bus's speed in km/h on the first frame. |
| `OMSI_FAKE_GPU_ERROR` | text | - | frame | render | Fault injection: open, open-panic, pipeline, lost-build, build, lost, frame - the GPU error to simulate. Only in builds with omsi-render's `test-hooks` feature (on by default). |
| `OMSI_FLEET_AHEAD` | num | FLEET_AHEAD | use | app | Minutes ahead the timetable fleet is planned (for tests). |
| `OMSI_FLEET_IDLE` | num | FLEET_IDLE | use | app | Seconds a fleet vehicle idles (shortened for tests). |
| `OMSI_FOLLOW_CAM` | text | - | use | app | right,ahead,up,yaw,pitch: a camera in the followed car's frame. |
| `OMSI_GLASS_WIND` | num | - | use | app | Offscreen: the rain on the glass as met at this speed in m/s. |
| `OMSI_GPU_LIMITS` | text | - | use | render | default or downlevel: request only the WebGPU default (or downlevel) limits. Set by the small_chip test. |
| `OMSI_GROUND_SAMPLE` | text | - | use | app | Offscreen CSV: what the wheels stand on every metre along the lanes near the start. |
| `OMSI_HIDE_MESH` | text | - | use | app | a\|b: leave out the meshes whose file names contain one of the parts. |
| `OMSI_HIDE_WINDOW` | text | - | use | app | from,to: treat the window as hidden between these seconds. |
| `OMSI_HOLE_PHOTO` | bool | off | use | app | With OMSI_ROAD_PHOTO: photograph from above down to 25 m under the lane (holes in the world). |
| `OMSI_INPUT` | text | - | use | app | Scripted keyboard, mouse and camera input for window runs. |
| `OMSI_JOINT_ANGLE` | num | - | use | app | Degrees: the rear section of an articulated bus held at that angle. |
| `OMSI_LANES_NEAR` | text | - | use | app | x,y,r: log the driving lanes passing there. |
| `OMSI_LAN_AUDIO` | bool | off | use | app | Offscreen LAN: with the other buses' sounds. |
| `OMSI_LAN_SAY` | text | - | use | app | "30=Hallo;45=Bye": LAN chat lines said at these seconds of the session. |
| `OMSI_LAUNCHER_EXIT` | num | - | use | app | Launcher: close after this many seconds. |
| `OMSI_LAUNCHER_INPUT` | text | - | use | app | Scripted launcher input ("t=2 click 400,300; ..."). Removed from the game's environment by the updater. |
| `OMSI_LAUNCHER_PAGE` | text | - | use | app | Launcher: open this page (e.g. mods, drive:N). |
| `OMSI_LAUNCHER_SHOT` | text | - | use | app | Launcher: secs:file.png - the window's picture into a file. |
| `OMSI_LAUNCHER_SIZE` | text | - | use | app | Launcher: WxH window size. |
| `OMSI_MOBILE` | bool | off | use | app | The phone layout on a computer (launcher, screen cut-outs). |
| `OMSI_MUTE` | bool | off | use | audio | Mix everything as usual but play nothing. |
| `OMSI_NAV_MAP` | bool | off | use | app | Open the navigation map. |
| `OMSI_NAV_MAP_MPP` | num | 2.5 | use | app | Navigation map zoom in metres per pixel. |
| `OMSI_NAV_PROBE` | text | - | use | app | x,y[,r]: the network lanes that start or end within r of that point. |
| `OMSI_NAV_SCHEDULE` | bool | off | use | app | Navigation map shows the schedule. |
| `OMSI_ONLY_MESH` | text | - | use | app | a\|b: draw only the meshes whose file names contain one of the parts. |
| `OMSI_ONLY_OBJECT` | text | - | use | app | Load only the objects whose file name contains this text. |
| `OMSI_ONLY_SURFACES` | bool | off | frame | render | Draw only the surfaces. |
| `OMSI_ORBIT_DIST` | num | 18 | use | app | Offscreen: the outside camera's distance from the vehicle in metres. |
| `OMSI_ORIGINAL` | text | - | test | sim | vehicle_vars example: the original OMSI folder. |
| `OMSI_PAX_CAM` | num | - | use | app | Offscreen --view pax: the n-th passenger camera. |
| `OMSI_PAX_CROSS` | text | - | use | sim | x,y: send pedestrians across the signalised crossing nearest that point. |
| `OMSI_PAX_WAITING` | num | - | use | sim | Number of passengers waiting at each stop. |
| `OMSI_POPULATION_SHOTS` | bool | off | use | app | Offscreen: pictures of the framed population spawns. |
| `OMSI_PROBE` | text | - | use | app | x0,y0,x1,y1[,n]: print terrain and road surface heights along a line. |
| `OMSI_PROBE_GRID` | text | - | use | app | x,y,half,step: the wheels' ground on a grid around a point. |
| `OMSI_RENDER_CLOCK` | num | 0 | use | render | Seconds the animation clock starts on (offscreen pictures). |
| `OMSI_RENDER_OCCLUDED` | bool | off | use | app | Draw frames into a texture while the window is hidden (macOS gives none). |
| `OMSI_ROAD_PHOTO` | bool | off | use | app | Map check: photograph the road network from above and report grass where a carriageway should be. |
| `OMSI_ROAD_PHOTO_N` | num | 400 | use | app | With OMSI_ROAD_PHOTO: number of sample points. |
| `OMSI_ROAD_PHOTO_SIDE` | num | 0 | use | app | With OMSI_ROAD_PHOTO: metres to either side of the carriageway. |
| `OMSI_ROAD_PHOTO_SLANT` | num | - | use | app | With OMSI_ROAD_PHOTO: from a driver's eye this far back instead of from above. |
| `OMSI_SEED` | num | random | use | app | Seed of the scripts' random numbers (repeat a session). |
| `OMSI_SKIP_OBJECT` | text | - | use | app | Leave out the objects whose file name contains this text. |
| `OMSI_SKIP_PIPE` | text | - | frame | render | Pipeline kinds to leave out of the main pass (comma-separated numbers). |
| `OMSI_SPOT_SELECT` | num | - | use | app | Turn on spotlight n (headlight pictures). |
| `OMSI_TEST_CONTENT` | text | - | test | app, o3d | Content root for tests that need real OMSI content. |
| `OMSI_TEST_WINE_DIR` | text | - | test | plugin | Plugin demo test: folder holding omsi-plugin-host.exe for Wine. |
| `OMSI_TOUCH` | bool | off | use | app | The on-screen touch controls on a computer. |
| `OMSI_TRIGGER_ALL` | bool | off | use | app | Check: fire every [mouseevent] of the model and compare the variables before and after. |
| `OMSI_UI_PREVIEW` | text | target/ui-preview.png | use | app | Plugin UI preview test: the picture's path. |
| `OMSI_WARM_FRAMES` | num | 0 | use | app | Offscreen: frames drawn before the picture. |
| `OMSI_WETNESS` | num | weather | use | app | Ground wetness the picture is drawn with. |

## A/B and kill switches

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_AI_MODEL_LOCK` | bool | off | use | sim | AI cars steer no further than their model's own steering lock (no 60 degree allowance for tight turns). |
| `OMSI_AI_WAY_ONLY` | bool | off | use | app | AI vehicles stand on their way with the plain ground sampler, as before (A/B). |
| `OMSI_BASIC_PIPELINES` | bool | off | once | render | Use the reduced (basic) render pipelines, as after a driver failed to build the full ones. |
| `OMSI_CROSSING_DEFORM` | bool | off | use | app | Apply the crossings' ground deformation again (A/B; Omsi.exe does not). |
| `OMSI_CUT_PLAIN` | bool | off | use | app | Road-cut textures uploaded without GPU-made mipmaps. |
| `OMSI_FIXED_SCALE` | bool | off | use | app | Keep the render scale fixed (no dynamic resolution). |
| `OMSI_FORCE_OPAQUE` | bool | off | use | app | Draw transmap-masked materials opaque. |
| `OMSI_FULL_GPU` | bool | off | use | render | Keep the requested settings on a small or shared graphics chip. |
| `OMSI_GL_TEXTURE_UNITS` | bool | off | use | render | Use the texture-unit layout of the OpenGL backend on any device. |
| `OMSI_GPU_ARRAYS` | text | - | use | render | textures or nostorage: take that texture array path on any device. |
| `OMSI_HEIGHTPROFILE_GROUND` | bool | off | once | app | The wheels stand on the splines' [heightprofile]s again (A/B). |
| `OMSI_INTEL_FULL_GPU` | bool | off | use | render | Keep the requested settings on an Intel Vulkan adapter. |
| `OMSI_KEEP_ALLOCATOR` | bool | off | use | app | Skip the restart that swaps in the faster allocator at start. |
| `OMSI_MIRROR_ENHANCED` | bool | off | frame | app, render | Draw the mirrors with the enhanced shading again. |
| `OMSI_NOZCHECK_BIAS` | bool | off | use | app | The old reading of [matl_noZcheck] (A/B). |
| `OMSI_NO_ANIMPARENT` | bool | off | use | sim | Every mesh animated on its own, without [animparent] (A/B). |
| `OMSI_NO_AO` | bool | off | frame | render | No ambient occlusion pass. |
| `OMSI_NO_ATTACH_FALLBACK` | bool | off | use | app | Drop attachments that have no fallback instead of placing them (A/B). |
| `OMSI_NO_BC` | bool | off | use | render | Upload all textures as RGBA instead of keeping BC compression. |
| `OMSI_NO_BOUNDS_CACHE` | bool | off | use | render | No cache of instance bounds. |
| `OMSI_NO_BRIDGE` | bool | off | use | app, net | LAN: leave the internet alone - no rendezvous bridge, no tunnel (tests, LAN parties). |
| `OMSI_NO_BUMP` | bool | off | use | app | No bump maps. |
| `OMSI_NO_BUNDLES` | bool | off | frame | render | Record the main pass directly instead of from render bundles. |
| `OMSI_NO_CLOUDS` | bool | off | use | app | No clouds. |
| `OMSI_NO_CORONAS` | bool | off | frame | render | No light coronas. |
| `OMSI_NO_CULL` | bool | off | once | render | Draw every mesh from both sides (A/B). |
| `OMSI_NO_ENHANCED` | bool | off | frame | render | Enhanced settings drawn with the plain path. |
| `OMSI_NO_ENVMAP` | bool | off | use | app | No environment maps. |
| `OMSI_NO_FXAA` | bool | off | frame | render | No FXAA. |
| `OMSI_NO_GLARE` | bool | off | frame | render | No sun glare. |
| `OMSI_NO_GLASS_PICTURE` | bool | off | frame | render | No picture behind the glass (rain films). |
| `OMSI_NO_GROUND_PAINT` | bool | off | use | app | No painted ground layers. |
| `OMSI_NO_GROUND_SPLINE_BATCHING` | bool | off | use | app | No batching of ground splines. |
| `OMSI_NO_INTERP` | bool | off | once | app | The old way without interpolation, for comparing. |
| `OMSI_NO_LAN_MODS` | bool | off | use | app | LAN: do not exchange mods. |
| `OMSI_NO_LIGHT_MAP` | bool | off | use | app | No night light maps on the tiles (A/B). |
| `OMSI_NO_MAIN_SPLIT` | bool | off | frame | render | Do not split the main pass's bundles in two. |
| `OMSI_NO_MATERIAL_SPLINE_BATCHING` | bool | off | use | app | No batching of splines by material. |
| `OMSI_NO_MODEL_ORDER` | bool | off | use | app | Draw the opaque parts of ordered models first again (A/B). |
| `OMSI_NO_MSAA_PREPASS` | bool | off | frame | render | No depth prepass with multisampling. |
| `OMSI_NO_PBR` | bool | off | use | app | No PBR materials. |
| `OMSI_NO_PLUGINS` | bool | off | use | app | No plugins loaded. |
| `OMSI_NO_POLL_THREAD` | bool | off | use | render | No device poll thread. |
| `OMSI_NO_PRESENCE` | bool | off | use | app | No presence ("playing now") service. |
| `OMSI_NO_PUDDLE_GLASS_DEPTH` | bool | off | use | render | Puddles without glass depth. |
| `OMSI_NO_PUDDLE_REFLECTIONS` | bool | off | frame | render | No puddle reflections. |
| `OMSI_NO_PUDDLE_VEHICLE` | bool | off | use | render | No vehicles in puddle reflections. |
| `OMSI_NO_RENDER_POOL` | bool | off | use | render | No encoding thread pool for rendering. |
| `OMSI_NO_RT` | bool | off | use | render | No ray queries (Enhanced+ draws as Enhanced). |
| `OMSI_NO_RT_FRAME` | bool | off | frame | render | Ray queries kept but nothing traced. |
| `OMSI_NO_RT_GRADE` | bool | off | frame | render | Enhanced+ without its colour grade. |
| `OMSI_NO_SHADOWS` | bool | off | use | render | No shadows. |
| `OMSI_NO_SMOKE` | bool | off | frame | render | No smoke. |
| `OMSI_NO_SNOWFALL` | bool | off | frame | render | No snowfall. |
| `OMSI_NO_SPLINE_BATCHING` | bool | off | use | app | No spline batching. |
| `OMSI_NO_SPLINE_HOLES` | bool | off | use | app | Splines without hole rims. |
| `OMSI_NO_SPRAY` | bool | off | use | app | No tyre spray. |
| `OMSI_NO_SURF` | bool | off | once | app | Every road as smooth as before (no surface texture) (A/B). |
| `OMSI_NO_TEXCOMPRESS` | bool | off | use | render | No runtime texture compression. |
| `OMSI_NO_TUNNEL` | bool | off | use | app | LAN: no tunnel. |
| `OMSI_NO_UPDATE` | bool | off | use | app | No update check. |
| `OMSI_NO_VAR_SYNC` | bool | off | use | app | LAN: no variable sync. |
| `OMSI_NO_WHEEL_SLIP` | bool | off | use | sim | Every wheel grips, as before wheels turned on their own (A/B). |
| `OMSI_OLD_WORLD_GRID` | bool | off | use | map | Maps with world coordinates take the one tile size of 371.9 m (comparison). |
| `OMSI_REPAIR_BODY_DEPTH` | bool | off | use | app | The old guess for [matl_alpha] 2 vehicle bodies (A/B). |
| `OMSI_ROAD_CUT` | bool | off | once | geometry | Take the ground away under every road surface (Omsi.exe does not). |
| `OMSI_RT_REFL_HALF` | bool | off | use | render | Trace reflections at half size. |
| `OMSI_SHADOW_FAR_EVERY_FRAME` | bool | off | frame | render | Redraw the far shadow map every frame. |
| `OMSI_SHADOW_NEAR_EVERY_FRAME` | bool | off | frame | render | Redraw the near shadow map every frame. |
| `OMSI_TERRAIN_ALIGN` | bool | off | use | app | Align the terrain to the splines again at load (A/B; Omsi.exe does not). |
| `OMSI_TEXTURE_RAIN` | bool | off | use | app | OMSI 2's own texture rain on the glass instead of the drops. |
| `OMSI_TRACKIR_NATIVE` | text | on | use | app | 0 turns the native TrackIR interface off. |
| `OMSI_TRAFFIC_ALL_GROUPS` | bool | off | use | app | Let restricted AI groups (aircraft, depot fleets) drive everywhere. |
| `OMSI_TYRE_SUSPENSION` | bool | off | once | sim | The old suspension with wheel mass, tyre and bump stops (A/B). |

## Tuning

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_ENHANCED` | bool | settings | use | app | Enhanced graphics for this run. |
| `OMSI_ENHANCED_PLUS` | bool | settings | use | app | Enhanced+ (ray traced) graphics for this run. |
| `OMSI_GRAPHICS` | text | settings | use | app | vanilla, vanilla_plus or enhanced: another renderer for one run. |
| `OMSI_IBIS_BUDGET` | num | 10 | use | sim | Seconds the IBIS typist may take per entry. |
| `OMSI_LAN_JOIN_TIMEOUT` | num | default | use | net | LAN: seconds to wait when joining a game. |
| `OMSI_MAX_FPS` | num | settings | use | app | Frame rate limit for this run. |
| `OMSI_METER` | text | - | use | render | gain,target,dark,bright,bias,night: the exposure metering. |
| `OMSI_MIRROR_HUD` | num | settings | use | app | Mirror HUD mode (number). |
| `OMSI_OPENXR` | bool | off | use | app | Start in OpenXR (VR) mode. |
| `OMSI_OPENXR_MIRROR_RATE` | num | settings | use | app | VR: mirror redraw rate (negative: every mirror each frame). |
| `OMSI_OPENXR_SCALE` | num | settings | use | app | VR: render scale of the eyes. |
| `OMSI_PARKED_PULL_OUT` | num | 0.035 | use | app | Chance per step a parked car pulls out. |
| `OMSI_PARK_IN` | num | 0.04 | use | app | Chance per step a car parks. |
| `OMSI_PUDDLE_F0` | num | 0.08 | use | render | Puddle reflectance at normal incidence (0.02 to 0.2). |
| `OMSI_PUDDLE_THICKNESS` | num | 0.12 | use | render | Puddle water film thickness. |
| `OMSI_SURFACE_BIAS` | num | -24 | use | render | Depth bias of road surfaces. |
| `OMSI_SURFACE_FLUSH` | num | 0.12 | once | app | Height in metres under which a surface counts as flush with the road. |
| `OMSI_TEXTURE_MEMORY` | num | settings | use | app | Texture budget in MB. |
| `OMSI_TONE_CONTRAST` | text | 1 | once | render | day[,night]: tone mapping contrast. |
| `OMSI_WINDY_TREES` | text | settings | use | app | 0 or 1: trees in the wind off or on (A/B renders). |

## Set-up

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_BACKEND` | text | settings | use | app | vulkan, dx12, metal, gl or angle (dx11, d3d11; Windows): the graphics API tried first (overrides the settings). Set at runtime by the launcher and on Android. |
| `OMSI_CLOUDFLARED` | text | - | use | net | Path of the cloudflared binary for the LAN tunnel (searched after the game's folder, before the PATH). |
| `OMSI_CONTENT` | text | beside the game | use | app, launcher-core, sim | The content folder (mods, archives, screenshots). Set at runtime on Android. |
| `OMSI_CONTENT_ZIP` | text | - | use | cfg | Extra content archives, separated like PATH (as --content-zip). |
| `OMSI_INSTANCE` | text | - | use | app, launcher-core | The id of a game instance started by the launcher (set for the child process). |
| `OMSI_LAN_IP` | text | - | use | net | a.b.c.d[,e.f.g.h]: LAN addresses to offer first (when the detection gets them wrong). |
| `OMSI_LAUNCHER` | text | - | use | app | Program to open as the launcher instead of the built-in one. |
| `OMSI_OFFICIAL_KEY` | text | - | use | app | File of the official server's signing key. |
| `OMSI_PLUGIN_HOST32` | text | beside the game | use | plugin | Path of omsi-plugin-host32.exe. |
| `OMSI_PRESENCE_URL` | text | built-in | use | app | Base URL of the presence ("playing now") service. |
| `OMSI_ROOT` | text | found | use | app, launcher-core, o3d, sim | The OMSI 2 installation folder (also the content root for tests that need real content). |
| `OMSI_SAFE_GPU` | num | 0 | use | app | Restarts after a lost graphics device: lighter on the card each time. Set at runtime on Android and by the restart. |
| `OMSI_UPDATE_URL` | text | built-in | use | app | Another release description (URL or file:///...json) for the update check. |
| `OMSI_WINE` | text | PATH | use | plugin | The Wine binary for Windows plugins. |

## Build

| Name | Type | Default | Read | Crates | Description |
|---|---|---|---|---|---|
| `OMSI_BUILD` | text | - | build | app | Compile-time (env!): the commit the binary was built from, set by build.rs. Not read at runtime. |

