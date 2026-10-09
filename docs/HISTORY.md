# History and roadmap

The round-by-round record of how openOMSI was built, moved here verbatim from
[ARCHITECTURE.md](ARCHITECTURE.md), which now describes only the architecture. It is kept as it
was written; the release notes are in [CHANGELOG.md](../CHANGELOG.md).

## Roadmap

1. Player vehicle: model animations driven by scripts, cameras, input, sound. **(done, first pass)**
2. AI traffic on `[path]` networks: lanes from splines and objects, linked by proximity;
   kinematic AI cars from `ailists.cfg` with car following, speed limits, blinkers, and
   `{frame_ai}` scripts (`omsi-sim::traffic`, `omsi-app::traffic`, `--traffic N`). **(first pass done)**
   Traffic lights (crossing phase programs, lamp objects), CTC paint schemes, scheduled AI
   buses driving timetable tracks with stops (`omsi-app::schedule`, `--schedule --time`).
   Simple collisions: oriented obstacle boxes from placed objects ([boundingbox] or mesh
   extents) stop the player vehicle and feed coll_* sysvars. **(done)** As in Omsi.exe
   (0x7af0a4) only `[fixed]` and `[crashmode_pole]` objects are solid (a collision mesh if
   they have one, else their `[boundingbox]`), and none with `collision_objects=0` (OMSI's
   `no_collision`).
   Passengers, first pass: skinned `.hum` models posed procedurally from the `[links]`
   joints (stand / walk / sit), waiting at `[busstop]` objects, boarding the player's bus
   through the open entry door into `[passpos]` seats, leaving at later stops
   (`--passengers`, `omsi-sim::human`, `omsi-app::humans`). **(done)**
   Also done: passengers on AI buses, ticket selling at the cash desk (ticket pack,
   `GivenTicket`), AI trains from `.zug` files on rail tracks (reversed cars), crossing
   right of way, body dynamics, HUD and start menu.
   Also: rigid-body physics (`omsi-sim::rigid`), `[matl_change]/[matl_item]` variants,
   envmap reflections, lightmaps, precipitation, cash desk money, chrono events,
   situations, LODs, HUD and menu.
   Also: AI lane changes (overtaking, keeping right), passengers walking the cabin path
   network, `[matl_allcolor]` items, blob shadows, real-time mirrors, parked cars,
   aircraft on flight paths, station announcements. **(done)**
   Also: stop requests, seasons, hourly traffic/passenger density, scenery sounds,
   `[worldcoordinates]` maps (371.9 m tiles). **(done)**
   Also: interior lights per `[illumination_interior]`, AI buses pulling into stop bays,
   pedestrians, clouds, terrain night light maps, AI collisions. **(done)**
   Also: sun shadow map (2048², 160 m around the camera, PCF; off in mirrors and at night,
   `OMSI_NO_SHADOWS`). **(done)**
   Also: AI yields to the player at crossings and sees its whole outline, AI buses hold at
   stops until their timetable departure (delay tracked), obstacle boxes exclude overhead
   beams. **(done)**
   Also: two shadow cascades (140 m sharp + 700 m coarse, texel-snapped, the far one
   redrawn every few frames). **(done)**
   Also: saving situations (.osn), passenger queues at the door, turn lanes / lane keeping
   at junctions, terrain cut only under flush surfaces. **(done)**
   Also: passengers standing at the `[ticket_sale]` path point turned to the driver,
   boarding modes (pay / auto / walk), the ticket key, control presets, the navigator
   corner, weather kept out of the player's cab, AI cars blocked only by a bus in their
   own lane and pulling out round standing obstacles, a content folder with mod
   installation (launcher Mods page, `Mods/` inbox), LAN play (`omsi-net`), enhanced
   graphics rework (neutral tone curve, cloud bodies, soft shadows, aerial perspective).
   **(done)**
   Also: per-draw buffer writes coalesced into runs and appended for new instances (the
   9 000 single writes a frame were the 14 fps), fog culling, parallel culling, AI sounds
   attenuated from the vehicle (aircraft), keyboard steering at a human pace, adaptive arm
   hang per .hum rest pose, passenger separation, parked cars as lane obstacles with a
   swerve, `--season`, season-filtered weather and a native folder picker in the launcher,
   hemispheric sky light in the enhanced path. **(done)**
   Also: the passenger side of the bus script (`PAX_Entry<i>_Req/_Open`,
   `PAX_Exit<i>_Req/_Open`, `door_aussenoeffner`), and the depot services `veh_tank`,
   `veh_wash`, `malfunction_gettime` / `malfunction_reset` (the workshop's minutes come
   from the bus script, the travel time from the map's `[repair_time_min]`). **(done)**
   Also: the driver's personnel file (.odr) - stops served and how many were left early or
   late, hectometres, crashes and pedestrians knocked down, tickets and takings, and the
   three ratings OMSI shows (`--driver`, F9). **(done)**
   Also: water surfaces from `.map.water`, wet roads from the `[moisture]` texture sidecar,
   crossings warped onto the ground with `[crossing_heightdeformation]`, and `[maplight]`
   reading as the brightness it declares (a petrol station's red sign used to wash a whole
   street red at night). **(done)**
   Also: the painted ground - every `[groundtex]` above the first is blended over the tile
   through the alpha mask the editor's brush writes to `texture/map/tile_x_y.map.<n>.dds`,
   with the ground texture and its detail texture repeating as often as `global.cfg` says;
   water from the map's own `texture/water.tga`. **(done)**
   Also: Wheel_Rotation / Axle_Steering in radians and with OMSI's sign, keyboard steering
   that returns only while rolling, the arrow keys driving so that OMSI's own W/S/D keep
   the wipers, the viewpoint and the gearbox; the cockpit switch under the cursor wins the
   click, and the mouse wheel turns it. **(done)**
   Next: bus stop shelters with waiting people inside, wear over a duty (bulb lifetimes,
   battery age), and the depot chooser that tells the workshop whether the bus is standing
   in a depot. **(current)**
   Also: `[newanim]` blocks composed the way the original does (see docs/FORMATS.md) - the
   doors fold to the sides of the doorway again and every other two-stage part (gear
   selector, parking brake, sun blind, ignition key) sits where it belongs; people on foot
   stand on the top surface rather than on the road under the kerb; Shift+U puts the bus
   into service by itself. **(done)**
   Also: `[matl_noZwrite]` (blended glass no longer writes depth and punches holes into
   everything blended behind it), glass reflections weighted by the viewing angle, WASD
   driving with shift for the three vehicle keys it covers, and a build stamp in the log and
   the HUD. **(done)**
   Also: `[spline_terrain_align]` (the spline's hole outline, as Omsi.exe cuts it) and
   `[terrainhole]` applied, and the ground only cut
   where a surface really crosses it; `OMSI_ROAD_PHOTO` photographs the carriageway network
   from above and reports where the picture shows ground instead of road (0 of ~400 points
   on both stock maps). **(done)**
   Known gaps: a click reaches a switch through whatever is drawn in front of it (the
   steering wheel rim) - an occlusion test was tried and dropped because it made a third of
   the switches unreachable from the default seat.
   Also (Sept 2026, the "problems" pass): keyboard steering swings back like a damped
   spring whose pull grows with speed; the rigid body holds a braked bus still (static
   friction below 0.15 m/s, sleep under 3 cm/s) - it used to creep for ever;
   `Axle_Suspension_*` is minus the compression (positive = wheel down), so the wheels
   stay in their arches; Z/X/C indicators with the `blinker_*` → `kw_blinker_*` alias
   table; the outside camera at 10 m, clipped against the ground and the obstacle boxes;
   the cursor turns into a hand over a switch; passengers are placed after the player's
   physics step (they trembled a seat-width behind the bus); SSAO with an ordered 4x4
   pattern and a 6x6 depth-aware blur, 5x5 PCF shadows; weather changes the light
   (overcast, rain, fog over the whole sky, snow textures and cover on any map);
   `~/.openomsi/settings.cfg` (MSAA, anisotropy, SSAO, shadows, navigator, enhanced,
   fullscreen); the ETS2-style navigator (`omsi-app::navigator`, N); AI cars spawn out of
   sight; pedestrians walk both ways along the pavements (`Network::prev`); a procedural
   gait/stance/sitting pose; Shift+U also sets the IBIS to the duty (without the AI
   trigger, which switched the NL202's electrics off); stop announcements verified through
   the IBIS-2 'next stop' key; `--export-glb` writes the bus as glTF; session summaries in
   `~/.openomsi/sessions/*.json`; the Enhanced graphics path (first an HDR filter with light shafts,
   ACES, grading and vignette; since f9770c1 its own physically based renderer without them);
   and the Tauri launcher in `launcher/` (profile with hours/XP/level, bus with 3D
   preview and liveries, depot, line, tour, roadbook with the IBIS codes, time, date,
   weather, settings). **(done)**
   The round of work after that - the report of about thirty problems - has a section of
   its own below.
3. Day/night: sun from date/time, envir.cfg light colours A/B/C, nightmaps, `[maplight]`
   point lights in a screen-independent light grid, `[light_enh]` coronas, vehicle
   spotlights/interior lights. **(first pass done)** Weather, seasons, reflections, shadows.
4. HOF/IBIS callbacks, text and script textures (matrix displays), coupled vehicles.
   **(done)** Humans/passengers, ticket selling, timetable following for the player.
5. GUI (menus, dialogs): the start menu and the launcher. Situations and chrono events.
   **(done)** Plugin API bridge. **(done)** Lua plugins.

## The big round (Sept 2026): the report of about thirty problems

Everything between `5f87297` and the tip of `integration` answers one user report of about
thirty problems. It was developed on parallel tracks (maps and streaming, physics, people,
traffic, the renderer, the cockpit, mods, the launcher and LAN) and merged back one at a
time, each merge saying which side of a conflict was kept and why. By area:

* **Content, archives and mods.** A `.zip` laid out like OMSI 2 is *mounted* instead of
  unpacked (`omsi-cfg::vfs`, ZIP64, case-insensitive, `\` as `/`), so every loader reads
  through the same calls whether a file is in a folder or in an archive; the content roots
  keep their order inside archives too. Keyword lines are matched the way OMSI
  matches them - the whole line, spelled as the original spells it, with the `.hof` and
  `ailists.cfg` exceptions - `-<DISABLED>-` blocks are skipped, `[newanim]` and
  `[new_attachment]` sub-commands are whole lines anywhere after their block, and file
  names resolve as Windows resolves them (see FORMATS.md). The script VM follows the
  original where mods depend on it (`{if}` keeps its condition, `$length` keeps its
  string, `$StrToFloat` gives -1, ten registers, an unknown `$` word compiles to nothing,
  a curve no constfile defines behaves like an empty one). Mod buses find their start-up
  and their IBIS log-in in their own scripts, borrow the map's depot file from another
  folder, fall back to another weight of a missing font, and rear sections are never
  offered or spawned alone. A mod map with missing content loads as far as it can and says
  what is not installed.
* **Streaming.** Tiles are loaded and unloaded around the camera and the player's bus with
  staged neighbours (`omsi-app::tiles::Streamer`), and the result matches a whole-map load:
  `[attachObj]` objects, spline attachment rows and their repeaters, chrono changes by ID,
  `[spline_h]` heights, objects standing on the final ground, the terrain cut only under
  flush surfaces. Everything a tile owns comes and goes with it - collision meshes and
  height profiles, poles, lanes and their light programs (whose clocks keep running),
  waiting places and the pavement network, parked cars matched to lanes as the lanes
  arrive - and a timetable bus waits for the part of its route it is on. Freed GPU slots
  are recycled; `OMSI_CHURN=x,y` is the offscreen check that a picture drawn from recycled
  slots is the right one.
* **Memory.** See the *Memory* section above: textures compressed on the GPU, the
  timetable fleet read ahead and trimmed, OMSI's `[texmemlimit]` as a budget, 16×16-texel
  surface raster blocks, sound clips let go, the allocator restarted with
  `MallocLargeCache=0`. Ahlheim V5 main station 8.75 → 1.93 GB peak.
* **The vanilla picture.** Content meshes are drawn one-sided as Direct3D draws them (the
  black square over the SD200's headlight was an inside-out shell), `[matl_envmap]`
  reflects by factor × mask with `[matl_envmap_mask]` and the diffuse alpha read as D3DX
  reads it, `[matl_bumpmap]` bends the reflection, `null.bmp` means no texture, the o3d
  material's own colours apply, `[matl_noZcheck]` and `[matl_Zbias]` draw with the surface
  depth bias, and a switched material slot showing the vehicle's own text or script
  texture keeps it. `[isshadow]` blobs lie on the plane the wheels stand on and are left
  out only while the sun shadow map is drawn. Anti-aliasing no longer aborts on a count
  the device cannot do, and the user's Spandau scenario doubled its frame rate (instanced
  draw batches, render bundles and command buffers finished on helper threads, merged
  per-draw uploads, a render scale with a Catmull-Rom/CAS upscale).
* **The enhanced renderer** (`enhanced.wgsl`, `atmosphere.rs`, `post.wgsl`) is now a
  physically based renderer of its own rather than a filter over the vanilla picture:
  energy-conserving diffuse and GGX specular, a computed sky (Rayleigh/Mie with ozone and
  a multiple-scattering estimate) that also lights the scene through order-2 spherical
  harmonics, contact-hardening sun shadows, lights in physical units, aerial perspective
  and height fog, automatic exposure metered as an incident meter and corrected only
  gently, a glow only real highlights produce, PBR Neutral and FXAA. OMSI's
  `[matl_envmap]` photo is a *tint* on the reflection, not a picture (clamped, in full
  only on glass, never on the player's own windscreen seen from the cab), sampled at the
  mip level its footprint needs with a blur that grows with roughness, and a surface
  reflects no sharper than its normals turn across a pixel. The vanilla picture stays
  pixel-identical; `OMSI_DEBUG_EXPOSURE`, `OMSI_METER`, `OMSI_DEBUG_ENHANCED`,
  `OMSI_DEBUG_SKY`, `OMSI_ENV_PHOTO=0` and the `sky_report` test check it.
* **AI traffic: motion.** A road vehicle is a bicycle model steered by pure pursuit along
  smoothed lanes, with Ackermann steering, per-wheel rolling and a body on its springs
  over the blended road surface (`omsi-sim::ai_motion`); trains ride on their outer axles
  and aircraft fly their paths.
* **AI traffic: rules.** Light programs run on a cycle clock with the stock state codes,
  stop and jump points and requests; following is IDM with gentle stop-line braking; right
  of way comes from `[rule]` priority and right-before-left over meeting places, with gap
  acceptance, patience, junctions kept clear and deadlocks broken. A vehicle standing in
  the lane (the player's bus, a bus at its stop, a parked car, a LAN player's bus) is
  passed on the oncoming lane: everyone stops as far behind it as its own steering needs
  (`pull_out_room`), a pull-out starts only when the body driven along the S-curve with
  its real lock and steering rate clears the obstacle's corner by 0.25 m
  (`AiBody::sweep_clearance`), and the oncoming traffic is looked for by time back through
  the junctions before it (`Network::upstream`). The population is kept out of sight, and
  runs repeat because the lanes are sorted by map identity. Spandau, the user's scenario,
  six runs: mean AI speed 21.1 → 25.0 km/h, standing share 28 → 20 %, buses standing over
  a minute at stops 51 → 12, vehicles stuck elsewhere 6 → 0; standing-bus runs: cars stuck
  over a minute 31 → 0, braking over 4 m/s² 15 → 0.
* **Physics.** Rigid-body vehicles with impulse collisions, wheels on the splines'
  `[heightprofile]` and the surface objects' collision meshes, a tyre envelope that climbs
  kerbs and stops at walls, ride height from each vehicle's own springs, and body-level
  static brakes. The tyre reads the ground from samples anchored to the ground (every
  grade over 6.4 % used to be a run of kerbs), a step over 0.8 of the radius is a wall,
  and a wheel strike is a crash. Pusher articulated buses drive: the rear sections act on
  the front at the coupling with their driven axle's share of `M_Wheel`, their brake and
  rolling forces and their weight on a grade, and a coupled part eases its height from the
  last frame's instead of from the coupling (the GN92's rear wheels stood 6 cm high).
* **People.** Passengers and pedestrians are agents: waiting places from the map's
  markers, ordered queues at every open entry, one person per doorway, riders walking the
  cabin path network with crowd avoidance (anticipatory time-to-collision,
  `omsi-sim::crowd`), exits chosen on their own floor and reached down the stairs of a
  double-decker, and the doors held while somebody is still stepping through. Poses are
  procedural
  (`omsi-sim::human`): a gait with planted feet whose stride matches the ground speed,
  two-bone IK legs that find kerbs and steps, arms that hang and swing, paying, holding
  on, sitting down, balance against a braking bus, head look-at. Pedestrians cross only
  when the green plus the clearance lets them across. `[smoothskin]` bellows follow the
  joint dummies (CPU skinning for the player's bus).
* **Timetable and cockpit.** The player's duty starts with the trip that fits the start
  time, is never also driven by an AI bus, and is typed into whatever IBIS the bus has
  (`omsi-sim::ibis` learns the keys from the compiled scripts and tries a plan out on
  copies of the bus's state). Stop times come from the trip profiles; the stock departure
  displays get their next arrivals; a duty on a line the date's chrono takes off says so
  in the log and on the HUD. `omsi-app::describe` gives the HUD a readable name for the
  switch under the cursor, from `Languages/<LANG>_key_veh_gen*.olf` plus a cockpit
  vocabulary for what the language files do not know.
* **LAN play.** Protocol 3 (4 since: the host's traffic, people and light programs as
  well, `omsi-net::world`): bit-packed vehicle states (20 Hz while anything changes, 5 Hz
  idle, about 60 bytes a bus) plus text messages, a host that owns the world (date, time,
  weather, season, clock) and checks every field it takes in, per-player rate limits, and
  remote buses that run their own AI scripts with the sender's pedals, lights and doors
  while a per-type sync table pins their lamp, `[visible]`, sound and moving-part
  variables. They are obstacles for the AI like the player's bus. A session is found by
  its code (`OMSI-7Q4K-…`: the scrambled session id first, the address hidden under a mask
  drawn from it), by ip, by port or by searching; **V** opens the chat line.
* **The launcher and leaving the game.** Mods install as background jobs (plan from the
  archive's table of contents, free-space check, staging on the content volume, atomic
  move, progress and cancel; nothing may be written outside the staging folder), archives
  can be used in place, the lists update without a restart, and any number of games can be
  started, watched and stopped from the Sessions page (LAN status with each player's
  passengers and the last chat lines). The settings page covers view distance, the
  cockpit-name language, texture memory and compression. Stopping asks with SIGTERM and
  waits: `omsi-app::quit` turns SIGTERM/SIGINT/SIGHUP (and Cmd+Q) into the ordinary
  shutdown, so the session summary and the personnel file are written and the LAN peers
  get their BYE.
* **Open.** Scenery scripts in parallel, synchronised timetables and passengers over LAN,
  real reflections in the driver's windows, culling beyond the frustum, parked cars that
  set off, AI buses in the random traffic pool (they come from the timetable only), and
  the wheel/arch fit per bus. Three reported problems could not be reproduced: a "random
  part of the bus without textures", "one model seen through another" (no texture load
  failures in the log) and a blank display after Shift+U without a duty (the matrix is
  blank without a duty and right with one).

### Mod bus compatibility round (Sept 22 2026)

Reported: the LiAZ 5292 dead and its name garbled, the PAZ 32051 dying after the key,
the Procity's displays dark and its air hissing for ever, the Lion's City A21's doors
flapping, the C2's BVG screen and air hiss, the Citaro Facelift's dark IBIS and hiss, the
Sprinter 412D without a body, the Sprinter 312D that would not move off, the Scania's
windows. Each was traced to a general cause, most of them read off OMSI:

* **Script VM** - division by zero gives 0 (the exe's `divide` stores 0; clearing the
  stack flattened the LiAZ's battery every frame); `random` is `Random(Abs(Round(x)))`.
* **Sounds** (`TSound` update) - a triggered entry plays once per trigger and ignores its
  conditions; without a trigger it loops while its conditions hold, and `[noloop]` makes it
  play once on the rising edge (the endless ECAS/parking-brake hiss); loopsound pitch from
  the sample rate with DirectSound's 100 Hz floor; the rear section of an articulated bus
  plays its own sound config (the C2 G's engine).
* **Code pages** - text is read in the code page it was written in (UTF-8, 1251, 1250,
  1252 detected per file, `omsi-cfg::codepage`); misread file names are found by their
  other spellings; font glyphs match across code pages.
* **Pack versions** - a vehicle pack installed in two versions (content folder and a map
  archive) no longer mixes files; a bus that borrows meshes from a pack that is not
  installed says so (the Ahlheim Citaro needs Urbino_II).
* **Meshes and textures** - o3d skips unknown bytes like the exe (protected v7 meshes);
  `.x` data objects may be named; 4-bit bitmaps with 17 palette entries; DDS with
  D3DFORMAT fourccs; absolute texture paths of the author's machine.
* **Engine inputs** - `AutoClutch` is 1 as in the exe's default options (manual gearboxes
  kept throwing their gear out); `Weather_Temperature` is known before `{init}` (engines
  were made at 0 °C). Shift+digit door keys send `_off` on release like a keyboard.cfg
  key. Shift+U waits for the engine to catch, checks it keeps running and presses a
  display's own power switch.
* **Checks** - `examples/bus_audit` loads and starts every vehicle of every content root
  (445 here: all buses start; the failures left are AI types and the Urbino_II-less
  hybrid, which starts its engine by itself once the air is up); a scripted drive test of
  68 mod buses in the game moved all of them (the N4021 and the Citaro 21M only after
  their compressors had filled the tanks, as the scripts want).
* **Open** - the Scania Citywide's "raised, grey windows" did not reproduce in any view
  (its door-window texture `верх.png` was missing before the code-page fix); the C2's
  ATRON works once booted (~30 s after the electrics) and the card is put in.

### Round 6 (Sept 23 2026): shadows, online interface, persistent traffic, clouds, rain

* **Shadows** - a close cascade (32 m around the camera, 3 cm texels) in the right half of
  the near map's atlas: the near cascade's 14 cm texels made a driving bus's shadow step
  from texel to texel. The player bus's outer skin is outside even where it lies inside
  the bus's box (`weather_outside_n`): the box's side plane cut the leaning side wall into
  a glossy and a matt half. SSAO's radius shrinks close to the camera.
* **Online** - `ui.rs`: Roboto text with an outline; a Roblox-like chat (V shows/hides,
  `/` or a click types, rustrict filter, 200 lines, wheel scrolls), the name of the button
  under the mouse next to the cursor, name tags over the other players' buses; the HUD's
  LAN block is one line; the others see the driver profile's name. Settings `chat`,
  `tooltips`, `name_tags`, `show_fps`, `clouds`.
* **Traffic** - random cars out of range become `DormantCar`s that keep driving along the
  network and wake up where they have got to; only a finished car leaves the map; the map
  is filled by metres of road. AI buses serve a stop they queue for behind a standing bus,
  keep out of bays wider than 5.5 m (no more lawn), move their IBIS on to the next stop,
  and light up in fog, rain, snow and overcast.
* **Sky and glass** - the weather's cloud type from `Weather/clouds.cfg` (alpha of
  Cumulus_1..3), fair-weather cumulus when none is chosen, ray-marched volumetric cumulus
  in enhanced; raindrops on the windows drawn procedurally (sitting, drying and running
  drops) instead of the sliding texture.
* **Displays** - a flat, colourless `[CTCTexture]` placeholder takes the first scheme's
  texture in the model's own look (the Procity's LED matrix); put into service after dark
  switches the saloon lights on; text/script texture slots of scenery objects are not
  looked up on disk.
* **Performance** - only alpha-tested pipelines keep `discard` (`ALPHA_TEST` override):
  early-z / Apple's hidden surface removal for everything else (main pass −8 %).
  `OMSI_PROFILE` counts triangles per pass; `bus_audit --long` checks two minutes of running.
* **Open** - the Scania Citywide's windows and the LiAZ's display gap match the model data
  exactly (checked by a software rasteriser of the o3d files and textures); the remaining
  difference to the original could not be pinned without a picture of the original.
  Plugin DLLs of OMSI and its mods are native (Delphi/C++), not .NET.

### Round 7 (Sept 23 2026): routes

* **Routes** (`docs/ROUTES.md`) - chrono `[deactivate_lines]` is a count and names (the
  count took lines "1", "2", "17" off); a scenario is in force only with a date, both ends
  inclusive, found in the game's order; lines from the last scenario back, taken off only
  by later ones; tour masks: bit 8 school holidays, bit 9 school days (were swapped). AI
  hof choice and `SetLineTo`/`AI_target_index` documented.
* **Glass** - a slot named like glass without `[matl_alpha]` is opaque as in OMSI (the
  LiAZ's display surround and window frames showed the sky).
* **Clouds** - one seamless cloud field texture per weather (made on the CPU, equalised
  shape, billows, heights, the weather's picture); enhanced: flat-based cumulus with round
  tops found by march + bisection, no grain, ~0.05 ms; vanilla: a flat layer lit by the
  scene's own light, a closed deck when overcast.
* **Frame time** - shadow PCF exits early outside penumbrae (5 taps instead of 16), the
  far cascade is cached again (every 4th frame), casters at the LOD the camera shows,
  culling hysteresis (15 % size, 5 % distance) against popping. Enhanced Spandau at
  1600x900: all GPU passes 7.0-7.9 → 6.1-6.3 ms.
* **Open** - the Scania's windows still look different from the user's screenshot, which
  shows another paint (BVG 4492) than the pack's own textures; a night tour chosen before
  midnight starts on its last (past) trip instead of the first one after midnight (fixed in
  round 8).

### Round 8 (Sept 23 2026): plugins, lamps, culling, script robustness

* **Plugins** (`crates/omsi-plugin`, `docs/PLUGINS.md`) - `plugins/*.opl` and their DLLs,
  driven as OMSI drives them (the original reads, the original loads, the original
  calls every frame: system variables, vehicle variables, UTF-16 string buffers, trigger
  edges as key down/up). A library the process can load runs in-process; OMSI's 32-bit
  Windows DLLs run in `omsi-plugin-host32.exe` (MinGW build, Wine off Windows), verified
  end to end with a Delphi-style stdcall DLL under Wine.
* **Saloon lamps** - OMSI's D3D point lights: colour/255, attenuation
  1/(d²/range²), no cut-off, on/off at 0.5; every `[matl_lightmap]` of a material kept
  (the effect has `gMatlLightMapOn0..3`), the slot's map made of the circuits switched on.
  The LiAZ's saloon was lit by its second circuit's map only.
* **Culling** - one held screen size per object (±6 %) decides for all its meshes and LOD
  levels, a camera inside an object keeps its top level, the top level never drops out.
  The flicker log of a 40 s drive went from 15 events to none.
* **Scripts** - a bus keeping its own cabin air (`heizung.osc`) is left to it (the engine
  model on top ran the 312D's to infinity, NaN in the engine); `OMSI_DEBUG_NAN`;
  `bus_audit` reports non-finite variables (none left in 445 vehicles).
* **IBIS** - trials give up after 10 s (`OMSI_IBIS_BUDGET`) and the duty is set directly:
  the C2's Atron RBL kept a worker busy 150 s with the displays blank.
* **Night tours** - a tour after midnight picked in the evening starts at its first trip,
  and a duty counts the clock across midnight (the day before or after, whichever is
  nearer the trip under way): 13N picked at 23:00 began on its last, past trip.
* **Debugging** - `OMSI_WATCH_VARS=a,b` logs changes of the player's bus variables in the
  window; `crates/omsi-o3d/examples/{bounds,matinfo,panels,sides}`.

### Round 9 (Sept 24 2026): navigator, launcher, passengers, the LiAZ matrix

* **Navigator** (`omsi-app::navigator`) - written anew after ETS2's Route Advisor: a
  tilted 3D map drawn by `omsi-ui` into a texture of its own (4x MSAA, premultiplied
  overlay). Roads are built per area into a GPU buffer whose ribbons are extruded in the
  vertex shader to the larger of their width in metres and a minimum in pixels, so the
  speed zoom rebuilds nothing (Spandau: 151k vertices in 1.8 ms). Route progress follows
  the bus lane by lane; leaving it for two seconds starts a Dijkstra (6 km bound, U-turns
  penalised) from the lane under the bus to the first route lane ahead; without a route yet
  (tiles loading) a way to the next stop. AI traffic, per-lane congestion (EMA of car speed
  over the lane's speed, scaled by queue length), signals on the route, the speed limit, a
  header with speed, line, passengers, time, next stop, distance and punctuality in
  ENG/DEU/FRA/RUS. Shift+N; `OMSI_DEBUG_NAV`.
  Later the same day: the route is drawn in runs by traffic level (blue/green/yellow/red/
  dark red; a lane with cars is light traffic, fuller is busier, a row crawling or standing
  heavy or jammed; rebuilt when a level on the route changes) with chevrons every 28 m in a
  contrasting colour; ribbons get round joints past 20° (the clamped mitre notched
  hairpins and loops). The way from the depot is searched at once, targets only the route
  up to the next stop (it used to join wherever the route was nearest - often mid-line),
  and falls back to a road past the stop when the route's own lanes before it are cut off
  (line 31's first lane at Maulbeerallee has nothing linked into it). Street names: OMSI
  maps have none, but their street name signs (`StreetSign_*`, 670 on Spandau) carry the
  name as the object's text string; a plate runs along the street it names (sign heading
  = road heading + 90°, settled from pairs of same-name signs 150-700 m apart: 1281 of
  1429 within 15°), and the name is carried along straight continuations for 1.5 km each
  way (3876 of Spandau's 17545 lanes). The duty starts with the first trip whose first stop
  is reachable in time (straight line × 1.35 at 25 km/h plus a minute); stops beyond the
  loaded tiles learn their place from the navigator's map.
* **`omsi-ui`** - Roboto in five weights of its variable font (with substitutes for the
  symbols it lacks), Material Symbols (Rounded, filled; `assets/icons/material`,
  rasterised by resvg), a shelf-packed RGBA atlas uploaded by region, a painter of
  anti-aliased shapes (rounded boxes, soft shadows, arcs, gradients, text, icons; world
  ribbons, discs and shapes) and the wgpu pipeline for them (layers with their own camera,
  viewport and rounded clip). `examples/headless` checks it without the game.
* **Launcher** (`omsi-app::launcher` + `crates/omsi-launcher-core`) - the Tauri launcher
  replaced by the game's own window: `openomsi` without arguments opens it. The showroom is a
  scene of the game renderer (the bus placed by `World::add_vehicle` after a worker read it
  and put its textures on the GPU ahead; the game's sky and lighting of the chosen time and
  weather; `player::sync_vehicle_transforms` shared with the player's bus); the interface
  is an immediate-mode toolkit on `omsi-ui` rendered into a premultiplied overlay texture.
  The data side kept its functions and `--cli`. Drawn at full rate with focus, ten times a
  second without, not at all while hidden. `OMSI_LAUNCHER_PAGE/SHOT/EXIT/INPUT`.
* **Passengers** - the trace (`OMSI_TRACE_PAX=<csv>`) showed people posed every 2nd-15th
  frame beyond 12 m (the body moved in steps, planted feet shivered), a boarding jump of up
  to 0.6 m (placed on the door's outside point), a drop off the step when getting off, turns
  at a constant 320°/s and a swinging foot snapping 30-40 cm onto a step when its target
  crossed the step's edge. Now: posed every frame within 45 m, the mesh never left behind
  the body, boarding from where they stand at their walking speed, getting off at their own
  height, turns eased by the angle left, the landing height eased (1.8 m/s) with the landing
  waiting for it and the rise/drop curves blended.
* **LiAZ 5292 line matrix** - its `Matrix_D.osc` pads a one-digit line to four characters
  and then keeps the right three (`$SetLengthR` keeps the right end - re-read in OMSI:
  op 0x25 at 0x5d584a is `Copy(s, len-n+1, n)`), so 1-9 were blank and 5E showed `   E`.
  `omsi_script::compat` rewrites that script's number as `"03" $IntToStrEnh` → `005E`,
  recognised by its text; the AI's `SetLineTo` is zero-padded to match.

Round 12 (Sept 25 2026, user bug list): sound `[volcurve] -1/-2` (TSound: time since active /
directional factor) - the LiAZ 5292's engine was silent under way; LiAZ letter lines 052D;
fonts fall back to the other case / unaccented letter; I = all saloon circuits; L also side
lights where they have a switch of their own; terrain cells split (i,j)-(i+1,j+1) as in
`.map.terrain_0.rdy`; wheels and feet on a `[surface]` object's drawn faces (depot yard) and
people on the surfaces' faces, not the raster; `[crossing_heightdeformation]` = height field
raising the plate (Juliusturm wall); traffic never appears/vanishes within 150 m; player asks
lights (depot gate); navigator in the bus's lane; clouds in world-mod-70 km coordinates and
volumetric (32-step march); driver figure at the wheel (player and timetable buses,
`driver.rs`, setting driver); people: eased corridors (no micro jumps),
seat and door hysteresis, onward paths at junctions; saloon lamps shaded by AO. **(done)**

Round 12b (Sept 25 2026, follow-up): LiAZ line numbers through `$__DigitsFirst` (052E whatever
the letter code); I key reports what it did on the HUD; Enhanced clouds a Nubis-style volume
drawn into a 512² sky cube one face a frame; drivers from the map's `drivers.txt` (Spandau:
aXYZ man01, not in its humans.txt), AI steering-wheel rim read from the file. **Timetable buses
rewritten as traffic** (`bus_service.rs`): an `AiCar` from the same `create_car` as every car,
carrying a `BusService` (stops, bay, doors while boarding, layover boarding in the last 45 s,
early wait, blinker while the doors close, pull-out, trip end); riders aboard from the start by
the hour (`riders_at`, `OMSI_AI_RIDERS`). People keep out of the scenery's collision boxes and
mesh faces (`keep_out_of_walls`, `OMSI_DEBUG_WALLS`), going round posts one way.
`OMSI_ROAD_PHOTO_SLANT=<m>` photographs the carriageway from a driver's eye. **(done)**

Round 13 (Sept 25 2026, user bug list after 3cedaf3): a plain `[texttexture]` is centred over
its letters without the trailing gap, halved downwards (LiAZ line cells at 13/51/89 px of 166
and the letter cell at 283.6 px of 512: "092" instead of "D92"). **Enhanced clouds** rebuilt the
Horizon Zero Dawn / bevy-volumetric-clouds way (`clouds.rs` noise: Perlin-Worley shape map,
Worley detail volume; curved-Earth shell 1400-2800 m, six-step light march, Hillaire multiple
scattering, two-lobe HG, Frostbite integration) into a 1024² sky cube with eight averaged jitter
rounds; the probe reads the cube. **AI bodies**: no guessed bus bays (stay on the path),
wheels on the way (surface only within 4 cm), 0.8 critical damping. **People**: late lift-off
starts the swing at its start and keeps the push-off pitch (15-22 cm foot snaps), everybody
within 30 m posed every frame (mirrors), queues leave the door at an angle facing the one ahead.
**Driver**: fingers curled round the rim (`curl_hands`), hands laid on the rim
(`PoseInput::grip_frames`). `OMSI_INPUT orbit <m>`. **(done)**

Round 14 (Sept 25 2026, the audit's bug list; branch `r3-fixes`): **situations** - the tile grid
is set from the map before a situation's place is turned into metres (Spandau's "Linie 5" put
the bus kilometres off the map), the orientation is a quaternion (heading 2·atan2(qy, qw);
it was read as half the angle), saving writes `[actuWeather]`, `[TT_active]`, the duty and only
the player's vehicle; **timetable** - past midnight the traffic clock runs on, departures are
compared on it (`day_base`) and the date's tours chosen anew (no bus came after 24:00);
`car_use/*.ocu` applied as OMSI does, fleet numbers unique; **ticket packs** -
stamper and buying share from one draw, tickets by age (default 40) and day tickets by the
time of day, the passengers' voices (greeting, TooLate, the ticket, thanks, BadChange);
**`[lht]`** mirrored (priority, passing, keeping to the side, navigator, pavements); **audio** -
no gap where a loop turns, at most 200 voices mixed; **NaN** - non-finite numbers read as 0,
float sorts with `total_cmp`, a panic hook; quicksave and screenshots into the content folder;
the personnel file's distance carried over; the smallest note for the fare. **(done)**
Round 14b: the complaints of a dark, hot, cold or muggy saloon,
greetings and complaints only in the player's bus, `Cabinair_relHum` a fraction as in the exe;
stamping at the validators (`[stamper]`, the bus's `ev_Stamper` sound); voice parameters queued
for the mixer's next block instead of waiting on the voice list's lock. A bus driven off the
edge of a map falls (the "615 km/h" of a 90 s offscreen drive on Grundorf was the fall).
**(done)** A situation's further vehicles are placed since round 15.

Round 15 (Sept 25 2026, the diagnostic's list): **keys** - F5-F8 are the destination sign keys
again (they also quicksaved, refuelled, washed and repaired - F8 moved the clock on), the
services are in the game menu and follow OMSI (the original: pump and wash only in a
`[petrolstation]` box, the workshop's travel time only away from one), F4 is the map camera;
**sound** - a `[loopsound]`'s fifth line is its volume, `[next_random]` keeps an entry by the
fleet number's characteristic, `Snd_OutsideVol` lets the
outside in through open doors, underpasses echo (`[triggerbox_setreverb]`, a Schroeder reverb);
**sun** from `timezone.txt` (zone, place, summer time); **grip** from `StreetCond` and the
temperature (black ice); **nothing written into the original** (personnel files, key bindings);
**particles** - `[smoke]` and `[particle_emitter]` (TRauch: exhaust, coolant, spray, chimneys,
the fireworks) drawn with `Texture/rauch.tga` (how Omsi.exe moves and draws them, and where a
puff meets the road: FORMATS.md, Model); **coronas** - z offset, rotating 0/1/2, inner
cone, star, double brightness; `[nomaplighting]`; **railways** - trains throw the switches
(`[switchdir]`), signals from `signalroutes.cfg` (Signal/NextSignal), `train_*coupling`;
`[blockpath]` conflicts; **ticket desk** - `[view_ticketselling]`/`[view_schedule]` cameras,
`change_give`/`change_take`; **options** - maintenance (`wearlifespan` as the original maps it,
AI never wears), random traffic share, timetable and parked car limits, real clock/date,
collision switches, head movement; **game menu** - weather, clock, load the quicksave, drive
the next vehicle; a situation's further vehicles placed; force feedback rumble from
`FF_Vib_Amp`; the duty start opens the map once. **(done)** Still open: the map and timetable
editor, placing new vehicles and coupling them by hand, `[LightMapMapping]` exactly (objects are
lit by lamps tinted from the tile's light map), a corona's own bitmap, fade time and fog cone,
LAN timetable and passenger sync.

Round 15b (Sept 25 2026, the rest of that list): **light pictures** - a `[light_enh_2]`'s own
bitmap, its `timeconst` fade, and the fog as OMSI draws it (the original,
the original): the glow as wide as the light's size, ((1-ambient)²+0.8)·0.6·brightness
linear in the angle between the half cone angles and clamped to 1, `licht.bmp` by default; the
star (effect bit 1) `light_effect1.bmp` turned to the viewer at 2.5 times the size; below 2 km
visibility (and without effect bit 2) a `licht.bmp` halo of 3·√(100/vis)·glow·size seen from in
front, and the cone: a flat fan from the lamp spanning the outer half angle, turned about the
light's axis to face the viewer, twice the halo's radius, `light_cone.bmp` mapped as the fan's
vertices do, 0.3 of the colour, seen from the side. The scenery lamps had kept only one sprite
per light (sprites were zipped with the lights' switches): `model_lights_owned`. Headlights are
a plain spot; **`[LightMapMapping]`** - the tile light map as an atlas lighting the splines and
mapped objects in vanilla (it had been added as white light); **vehicles** - any vehicle placed
from the game menu, coupling and uncoupling by hand; **parked cars** pull out (the AI car of
the parked object's folder takes its place, indicating, beside a lane only); **LAN** -
protocol 5: the tour a player drives in `INFO` (the host's timetable leaves it to them), riders
of the players' buses in the world frames (`PLAYER_BUS`) and every player's people relayed to
the others; **object editor** - move, turn and delete a tile's `[object]`s in the game, saved as
tile copies in the content folder (UTF-16 kept), a copy finding its `.terrain` in the map's own
folder. **(done)** Still open: the full map and timetable editor (splines, ground, new objects,
timetables), a parked car in its lane pulling out of a row.

Round 16 (Sept 25 2026, the user's bug list): **launcher** - opens without the original game
(on Setup, which asks for it; only a session needs it) and finds it by itself in more places
(`omsi_cfg::find_original_install`: beside the program, every Steam library in
`libraryfolders.vdf`, Windows drives, Wine/CrossOver/Whisky bottles, the user's folders);
**fog** - none inside the player's bus (`fog_distance` takes the part of the view ray inside
its box off the fogged distance; the saloon and the door panes went milky in ground fog);
**driver** - drawn in the mirrors from his own seat (`Instance::mirror_only`), each hand on its
own half of the wheel locked to the rim (push-pull, lifted round the outside to regrip, the rim
sliding through the hands when the wheel is flicked), a fist round the rim, the seat slid at
most 16 cm and the rest reached by leaning; **passengers** - greetings and complaints only when
the same voice file has not been heard for 10 s (the original, the list at 0x861244;
the ticket, "thanks" and the missing change are never held back), riders tilt with the floor
(pitch, bank), their interior light eases in and out, money held out only at the desk, no twin
figures side by side, each notices an arriving bus 0.2-2.4 s after it stopped; **AI** - bends
sampled at fixed places of the road (the car-fixed 2.5 m grid made the allowed speed fall in
steps: braking hard with nothing ahead), lower speed limits ahead reached gently, timetable buses
pull away when the script says the doors are shut (`AI_Scheduled_AtStation` back to 0, at most
12 s), vehicles appear and vanish only out of sight within 350 m whatever the camera looks at
(mirrors), AI cars park in the spaces parked cars left (`Traffic::park_in`,
`World::return_parked`), `OMSI_TRACE_AI` says why a car is held (`why`, `why_gap`, `phase`),
`OMSI_DEBUG_CAR=<id>` logs a car's speed profile; **bus wheels** - a kerb under part of the tread
lifts the hub by that share (`tread_step`), tyre meshes seated on the physical hub when a mod's
suspension animation leaves them in the asphalt (the LiAZ: -3.8 cm); **LAN mods** - the host's
non-stock content in use goes to joining players for the session (`lan_mods`: list by index over
TCP on the session's port number, SHA-256, executables/plugins refused by name and by content,
a sandbox content root without plugins, removed at the end); **performance** - light picture
lookups cached, mirrors at most 75 pictures a second (plainly shaded in Enhanced: 12.7 -> 0.6 ms
GPU), one staging buffer for skinned meshes, the cockpit hover ray only when the aim changed:
Spandau centre 109 -> 156 fps (vanilla), 58 -> 76 fps (Enhanced). **(done)**

Round 17 (Sept 26 2026, the user's bug list): **glass in fog** - the inner face of the player
bus's own panes mirrors the dark cab, not the bright sky probe/sphere map (the doors, seen at a
grazing angle from the driver's seat, went a milky grey sheet in fog and snow; both renderers,
`near_player_vehicle` moved to shader.wgsl); **Urbino headlamps** - a mesh whose normals disagree
with its winding only until they are turned by the file's own matrix (a half turn about x: the
Urbino's lamps, fog lamps, day lights, VDV screens) is drawn as wound, not turned round
(`mesh_from_o3d`; `examples/turned` lists such files); **driver** - the hands hold the rim and
turn with it (angle from the wheel's own `[newanim]` variable × factor, each hand soft-clamped to
its comfortable arc, the rim sliding through beyond), no more push-pull regrips; the wheel's
centre is the middle of its rim ring on the axis, not the animation origin (the Urbino's origin
is the column foot, 12 cm under the hub); the first-person cab view draws a second mesh set
without head, trunk and upper arms (`without_head`, the whole figure stays in the mirrors);
**suspension** - each wheel has its own mass (12 % of its corner) between a stiff tyre (900 kN/m,
preloaded by the static load so the ride height is unchanged) and the strut; the ground under a
tyre rises no faster than the tyre can climb (2.4 × speed); the wheel falls against the body's
own acceleration (`accel_body.z`); `OMSI_SUSP_TRACE=<csv>`; **performance (Enhanced)** - sky
cube face every 4th frame, near shadow cascade every other frame (matrix kept in
`shadow_near_cache`, close part cleared by `shadow_clear_pipeline`), close cascade ≤ 2048
texels (`SHADOW_CLOSE_MAX`, scale in camera `post.w`), faded-out blended layers skipped, one
shadow compare on glass, MSAA depth prepass only off Apple GPUs; Spandau cab 45.7 -> 66.4 fps,
freezing-wet Urbino cab 39.5 -> 68.6, night 73.7 -> 108.6, outside 53.6 -> 69.1 (2560x1080,
4x MSAA, traffic 37, passengers, timetable). The offscreen `OMSI_BENCH` exaggerates GPU times
(the GPU clocks down between its waited frames): measure in a window with `OMSI_GPU_TIMERS=1
OMSI_PROFILE=1`. **(done)**

Round 17b (Sept 26 2026): **driver's grip and steering** - each hand's frame on the rim is built
from its own forearm (the knuckles across the rim as near the forearm's line as a diagonal grip
allows, `DIAGONAL` 40°, palm over the rim), not a fixed frame on the rim that bent the wrists up
to 80°; `curl_hands` closes the fingers together (`SQUEEZE`) and swings the thumb in under the
bar; rest at ten to two (±70°). Steering is a hand shuffle (`steer_hands`): each hand turns with
the rim within its arc (±20…150°); one turned out of it opens its fingers, lifts off and takes the
rim again 40° from the arc's other end while the other hand holds on (faster the faster the wheel
turns); the wheel held still 0.6 s brings the hands back to rest one at a time. The old soft clamp
left the hands frozen while the rim spun on through them. `OMSI_DRIVER_HANDS=<l>,<r>` holds the
hands at fixed angles for close-ups. **(done)**

Round 17c (Sept 26 2026): **driver's hands, smoother** - a fist's roll round the rim is its own
eased angle (`ROLL` -60…100°, 0.12 s) that falls back to the plain grip over the top where the
forearm runs along the rim (there the palm flipped over from frame to frame, up to 5000°/s; now
≤ 540°/s, the wheel's own turn), each hand's frame eased (0.07 s); regrips unhurried (0.38 s +
1 s per 350°, ≥ 0.32 s, smootherstep, lift and finger opening sin²) instead of 0.14 s flicks;
the rim slides up to 25° past a hand's arc meanwhile. First-person view: whole arms instead of
floating cuffs, the shoulders' tops (0.24 m round the joint) left out. `OMSI_DEBUG_DRIVER` logs
`HANDT` rows (wrist target, hand frame) per frame. **(done)**

Round 17d (Sept 26 2026): **driver, no first person; steadier hands** - the `driver_first_person`
setting is gone (game, launcher, launcher-core): in the cab view the figure is only in the
mirrors, as in OMSI, and the second mesh set without head is removed. The fists had the wrong
hand's chirality (palm = along × dir: the thumb pointed down the rim, the fingers were held in
over the top); now palm = dir × along, the roll measured from the rim's outer edge (0 = fingers
out over the outer edge, 90 = knuckles away from the driver, palm to the wheel's middle; range
-30…120°, 30° where the forearm says nothing). Micro jumps measured in a window run
(`OMSI_DEBUG_DRIVER` HANDP rows, second difference of the posed wrist): grip correction eased
(0.25 s, ≤ 2 mm a frame; it jumped up to 4 cm), regrips leave and meet the rim with its speed
(Hermite path), a hand sliding past its arc slows over SLIP instead of stopping dead, the
keyboard steering's return snaps to the middle only under 0.0003 (was 0.002 = 2°): worst jolt
69 -> 18 mm, the rest the wheel's own reversal. **(done)**

Round 19 (Sept 26 2026): **the user's list** - rain: a texture's `.cfg` is the requested name's
(`str_asphdrk.bmp.cfg` beside `.dds`, the original), junction objects get `[moisture]`,
puddles spread to the whole road as it soaks (puddles.tga against 255·(1−wetness),
the original); window drops fixed to the glass (grid snapped to eighth turns); enhanced clouds
from one sky-cube eye with parallax correction; traffic: the player's rear sections, placed
vehicles and a body in reach stop cars, cars/buses at the network's end leave, stale junction
claims hold nobody (`OMSI_CHECK_OVERLAP`, `OMSI_DEBUG_STUCK`); LAN duty placement for joining
players; entry spawn on the road surface; `[illumination_interior]` inherited by the next mesh
; NaN guards for Vulkan/D3D (fast-math hid them on Metal); LAN remote buses get
display texts, window rain and a driver; walking inside the bus (cabin corridor), drag-only
controls toggle by click, door groups close together; a running gait; **dedicated server**
(`openomsi --server server.cfg`, wgpu no-op device) with WebSocket transport, `/status`,
`/icon.png`, Cloudflare quick tunnels (also for Connect by Code); launcher Multiplayer page
(Connect by Code / Servers) and a server-locked Drive page with Leave Server. **(done)**

Round 20 (Sept 26 2026): multiplayer passengers - another player's bus is entered only
through an open door, G sits in its nearest free seat, the walker's place aboard a player's
bus goes over the network (`Walker::aboard`, trailing optional wire fields), the bus is drawn
from inside for whoever is in it (viewpoint 2 meshes, opaque slots, skinning, `lighting.inside`),
its saloon switches are worked through the owner's game (`CMD|from|to|trigger …`), people and
the camera aboard are placed in this frame's bus frame (`foot_after_humans`), remote buses are
interpolated between states by the sender's clock (`Pose::sent_ms`, ~2x less jitter);
administration for hosts and servers (`admin.rs`, `/admin <password>`, `server.cfg`
admin_password / time_speed / vehicles); time speed; joining offers only the host's buses
(`ServerInfo::vehicles`); returning players keep their number; cloudflared fetched
automatically; launcher without network jargon, correct scaling (flat UI layers drawn over
the whole target), scrolling settings; offline machine translation of untranslated texts
(`mt.rs`, CTranslate2 + NLLB-200, the `trad` engine); duties started hours early move the
clock; tour names as the map writes them; NFC file-name matching; cached OMSI_* env; frame
limit at the screen's refresh rate by default. **(done)**

Round 21 (Sept 27 2026, the diagnostic audit and its fixes): **security** - relay posts
signed with a session-derived HMAC on a hashed topic (no nonce or session id on the public
relay), returning players known by nonce only, server administration by challenge-response
with a lockout and rights ending with the player, NaN refused in admin commands, the mods
server and WebSocket gateway capped in connections with bounded request lines and streamed
files, hex-only store names, cloudflared and the translation model pinned and SHA-256
checked, downloaded plugins held in `Mods/plugins-not-enabled`, a mod file never deletes an
installed folder, plugin string buffers padded; **crashes and hangs** - the tile loader
survives a panic and lets go of tiles it cannot make, bad triangle indices dropped, damaged
.x/DDS/TGA/terrain files refused before they allocate or index, no 3 s wait on joining,
LAN status written off the frame, controllers merged per device with DirectInput slots;
**per OMSI** (see RE notes in the commits) - cant in percent within `[halfcantwidth]`,
`GetTime` as play time, `$IntToStrEnh`, (L.M.)/(S.M.) as system variables, (S.S.) writes,
STLoadTex/STTextOut/Refresh_Strings, driver ratings as the exe's counters (late > 180 s on
arrival, early < −120 s on departure, driving 100(1−P)), Brakeforce, all-exit termini,
TrafficPriority, FF_Vib_Period, tank_percent, Colorscheme, `[NightMapMode]` hours and InUse,
the legacy `[ailist]`, registrations.txt, `[shadow]` casters as an option, wheels that
slip and lock (achse_inertia_inv, default 0.002) so the brake scripts' ABS works, the sprung
seat, `laststn.osn` with a Continue button; **simulation** - long frames sliced, odometer
carry, articulated sections pitching and leaning, coupled parts along their own headings,
gridlock broken, pedestrians' gap acceptance, teleports not counted as driving; **rendering**
- size-keyed targets evicted by use, safe normals, device loss ends the session cleanly,
lookups forget misses when content changes, DX10 DDS; **game** - Options, Line and tour,
Driver and Fleet number in the game menu, Ctrl+click on the city map places the bus, the
volume setting reaches the mixer, OMSI 2's options.cfg imported on a first start, the
navigator shows what a jam costs, no compiler warnings. **(done)**
Still open: the map and timetable editor, player-driven trains and trams (track-bound
physics, overhead wire, `[realrail]` bogies), timetable and passenger sync over LAN, chrono
and season changes at midnight (the exe re-evaluates them; here they are fixed at load),
`relrange`, `Snd_Microphone` (the exe toggles the OS microphone line), cubemap DDS (first
face only), a controller axis/button editor with calibration and a steering spring,
Colorscheme for scenery objects and people, parallel scenery scripts, real reflections in
the driver's windows, one Grundorf road point that shows ground (0.4 %, older than this round).

Round 22 (Sept 27 2026, the rest of the audit's open list): **world** - the date is followed
at midnight as OMSI does (chrono scenarios that start or end have their tiles read
again, the season's textures change with the date and with snow); **driving** - rail
vehicles (`[rail_body_osc]`, `[contact_shoe]`, `[boogies]`) are bound to the track: placed on
the nearest rail lane, moved along it by their speed, taking the indicator's branch, else
the switch's setting, else the straightest, and throwing the points they take; coupled parts
follow a trail along the rails; a train whose scripts give no drive gets a plain traction
and brake (`rail_drive.rs`); **controllers** - the launcher's Controls → Game controllers
tab edits `gamectrler.cfg` (axes with function and direction, live bars, buttons by
pressing them, a dead zone, `[FFScale]`), written to the content folder; **editors** - the
object editor copies objects (C) and changes a copy's type (V), and shapes the ground
(Page Up/Down, F flattens, [ ] brush size; `.map.terrain` copies in the content folder,
which tile companions now prefer); the launcher's Timetable page edits a map's lines -
tours, departures, trips and profiles, copying a tour N minutes later - saved as `.ttl`
(a stock map's TTData copied whole into the content folder first); **rendering** - cube
map DDS files become sphere maps from all six faces, real reflections (the scene around the
bus rendered into six faces, one per frame, sampled by the vanilla and enhanced glass;
setting `real_reflections`, imported from OMSI's `performance_realreflexions`), scenery
scripts run in parallel on the worker threads, scenery objects' Colorscheme is −1 as the
exe gives them; the road photo check sees only the half metre over the lane (both stock
maps 0 %). **(done)** Corrections to the round 21 list: timetables and passengers over LAN
were already synchronised (the host simulates, clients claim riders); the overhead wire is
not an OMSI variable and was dropped. Still open: splines in the map editor (moving or
making road pieces regenerates their meshes, lanes and terrain cuts), a trip's route
(`.ttr`) and stop times in the timetable editor, `relrange` (no stock use, meaning
unclear), `Snd_Microphone` (the exe toggles the OS microphone line; deliberately not done).

Round 23 (Sept 27 2026, the player's bug list and a diagnostic of ~70 more): **lights** -
fog cones mapped as OMSI maps `light_cone.bmp` (the original: rim uv = sin a, 1 − cos a;
no "V" of light from every lamp); headlights light light-mapped roads in the classic
picture (a vehicle's point lights are flagged, `point_lights(map_k)`) and more strongly in
Enhanced; Shift+U after dark switches them on; the driver is lit by the lamps near the seat
(`VehicleInstance::interior_light_at`); **mirrors** - the envmap photo dims with the night
outside the classic picture, a mirror's own picture (params.y 0.9) is not brightened like a
display, the plain light of an enhanced session's mirror frame is taken down at night; at
least 8 redraws a second per mirror, two a frame at most, also near the bus from outside;
**weather** - nothing under any vehicle's roof gets snow or wet (`Instance::roof`), no rain
falls inside other players' and timetable buses, drops on panes fade out over 4-12 m;
**on foot** - stepping out, through doors and into buses is walked (`Transit`), a double
decker's driver stands up on the lower deck, riders leave a bus the driver walked away from
(`ALL_OUT_STOP`); **LAN** - fine doors (1/255) and the walker's course in the state's tail,
the player's own figure (`Pose::figure`, INFO field 13) for their walker and their driver,
an interpolation delay that follows the state rate, the walker extrapolated, remote
headlights from `Spot_Select`, a joining bus placed after the host's list, no switches of
another player's bus offered to a rider, cloudflared stopped at the end (and a stale one at
the start), the router's UPnP forwarding taken back; **AI** - a timetable bus that waits 40 s
for route that never joins drives on and leaves (Spandau lane 1242), a car given up in a
gridlock leaves after four minutes even in view; **timetable** - an entry point on the route
before the first stop reaches it, stop names from the trip file when the stop object is not
known; **money** - notes as well as coins, exact fare exact; **keys** - a release goes to
what the press started, F12 yields to a bus binding, the blinker keys follow the lever;
**clouds** drift on over midnight; **launcher** - decomposed accents composed (macOS file
names), the METAR airport nearest the map, the Timetable page on the chosen map, saving for
maps in archives, "Reset timetable". **(done)** Found to be content, not openOMSI: Novi
Sad's `.ttr` tracks jump between tiles (id 9277167 on tiles 441 and 167 in a row), a missing
`IK218N` script, missing fonts. Still open: the frame rate at 2560×1080 with 4× MSAA in
Enhanced on Novi Sad (~25-30 fps offscreen, GPU main pass ~12 ms), the "camera too far
forward" after getting in again and the Ctrl+Shift cab light on the EN92 (not reproduced),
a player seated in a timetable bus is still not drawn for the others.

Round 24 (Sept 27 2026, the player's second list): **vehicles** - Esc → Remove this vehicle
(`App::remove_driven_vehicle`: riders step out, the player stands by the cab), on foot with
no bus of one's own (`--on-foot`, launcher "Start on foot"), G at a placed vehicle's cab takes
its wheel (walked in: `Transit::walk_in` + `Then`), Place a vehicle follows the mouse on the
ground (`placing.rs`: wheel/Q/E turn, R round, click sets down, refused inside another
vehicle), placed vehicles run while on foot, a bus change keeps the riders in their own bus
(`Humans::player_bus_swapped`, placed buses as `placed_bus_id(uid)`), a bus that falls
through the world is put back where it last stood (`admin::guard_fall`); **vans** - a
`[boundingbox]` far larger than the model is cut to it (W906: 12 → 6.4 m, the invisible walls
at nose and tail), a van's driver gets out and in by the cab door (`vehicle_cab_door`), exits
only through a door within reach (`DOOR_OUT_REACH`); **passengers** - exit queues stay on the
walkways (`Cabin::exit_queue_place`; people stood over the W906's bonnet), `OMSI_CHECK_WALLS`;
**Shift+U off** - the engine stop (`kw_m_engineshutdown`) held until the rpm is at rest, then
the power; **pause menu** - scrolls (wheel, arrows, a scroll bar), new lines (remove the
placed vehicles, get up, back on the wheels, city map); **admin** - bring everybody, repair /
refuel / wash one or all, back on the wheels, clock presets, traffic; **object editor** -
mouse pick / drag / wheel turn, host only in LAN play, edits sent as `objedit` / `objadd`
commands (and all again every 10 s); **LAN** - doors opened by hand (clickable leaves) synced;
**light** - mirrors at night match the enhanced window (sky weights dimmed too), the cab's
SSAO at a third (the dashboard was black beside a lit cash desk), snow fog darkens at night,
MB 412D start sound clamped to 0 dB; **GPU memory** - automatic texture budget
min(RAM/8, adapter guess), textures awaiting compression up at half size (Novi Sad loading
peak 1.9 → 1.2 GB), faster trimming, presets keep the frame-rate governor on; **PBR** -
`omsi_texture::pbr`: doubled letters `_nn` (+`_gl`), `_rr`/`_gg`, `_mm`, `_aa` (or the
long names `_normal`, `_roughness`, `_metallic`, `_ao`), `_orm`/`_arm`/`_mra` beside a
diffuse texture (single letters are OMSI's night maps; a normal map must also look like one), normal mapping by a derivative tangent frame in enhanced.wgsl. Not reproduced: stray
light sources (Grundorf, Novi Sad at night), the EN92 Ctrl+Shift cab light.

Round 25 (Sept 27 2026): **Android** - the game is now a library (`openomsi_game`, `lib.rs`;
`main.rs` calls `run`), built for Android as `libopenomsi_game.so` for a NativeActivity
(`scripts/build-android.sh`, docs/ANDROID.md). One process and one window there: `android.rs`
runs the launcher and the game in turn (`omsi_launcher_lib::launch` keeps the command line
instead of starting a process, `platform::exit` ends a session back to the launcher),
surfaces are dropped on `suspended` and made again on `resumed`. The launcher lays itself out
for fingers (`launcher/mobile.rs`: rail of icons, page scroll, storage browser, on-screen
keyboard); the game has on-screen controls (`touch.rs`: wheel/tilt, pedals as analog axes,
gearbox, doors, brakes, indicators, horn, cab panel, cameras, tap/drag on cockpit switches,
pinch zoom, a stick on foot), drawn with omsi-ui. Phone defaults in `Settings::default`, the
navigator's `top-center` corner, `/proc/meminfo` for the texture budget. `OMSI_TOUCH=1` and
`OMSI_INPUT` `touch` commands check the controls on the computer, `OMSI_MOBILE=1` the launcher.

### 0.1.5 (Sept 27 2026): Lua plugins

`crates/omsi-plugin/src/lua.rs` + `prelude.lua` (mlua, vendored Lua 5.4): `plugins/*.lua` and
`plugins/<name>/main.lua`, one sandboxed state each (no io/os.execute/C modules, `require` in
the plugin folder), the `omsi` table (bus variables/strings/triggers by name, system
variables, position, on-screen messages, events, timers, watches, `omsi.data` saved to
`*.save.lua`), hot reload on save, a 1 s budget per call and switch-off after 10 errors.
Driven with the DLL plugins from `Plugins::frame`; the game's side (dt, vehicle name,
position, message) is `PluginIo`'s new default methods. Tests: `crates/omsi-plugin/tests/lua.rs`.

### 0.1.8 (Sept 28 2026): controls, controllers, multiplayer that lasts, phones, more builds

* **Keys** (`launcher/pages.rs`): the Controls page shows which `drive_keys` layout is in use;
  a changed binding switches it to `omsi` (Custom controls), since a ready-made layout's keys
  win (`input_script.rs`). Space is no longer reserved by the W A S D layout: it is OMSI's
  `view_reset_all_directions`. `App::view_looks` keeps a look direction per view
  (`sync_view_look` on every change of view); `App::view_zoom` narrows the field of view of
  the driver's and passenger views (the wheel, = / -, a pinch).
* **Mouse control** (0x6f4284..0x6f447b, `Panel1MouseMove` 0x82c5f8): the cursor is the
  panel's client position; steering = (2x/w - 1) / max(1, km/h / 10) x
  `[inv_min_turnradius]` into `inv_lenkradius` (+0x73c, read by the integrator 0x7e27c0);
  throttle max(0, 1 - 2y/h), brake max(0, 2y/h - 1) (`throttle` +0x5dc, `bremspedal` +0x5e0),
  all three eased for a second after switching on. `mouse_sens` scales ours.
* **Game controllers** (`controllers.rs`, `dinput.rs`): `Devices` joins gilrs (gamepads; every
  device on macOS and Linux) and on Windows DirectInput 8 (`DI8DEVCLASS_GAMECTRL`, a data
  format of 8 axes, 4 POVs, 128 buttons, axes -10000..10000, the device list read on a thread
  every 3 s). Button numbers come from the system's code (HID usage - 1, evdev BTN_JOYSTICK /
  TRIGGER_HAPPY, the WGI index); they used to be the rank among buttons pressed so far. Force
  feedback on a DirectInput wheel is one constant force set each frame: a gentle centring
  force that increases as the bus rolls, tyre scrub that resists turning at parking speed,
  a filtered and bounded contribution from the body's lateral acceleration, short
  front-wheel jolts from suspension travel or wheel impacts, and the scripts'
  `FF_Vib_Amp`/`FF_Vib_Period` as a sine; the first `[FFScale]` value scales steering forces,
  the second scales vibration for that device. The wheel's own autocentre is switched off
  (as OMSI does). The set-up assistant (`launcher/pages.rs`,
  `wizard_result`) records rest, left lock and each pedal and picks the axis that moved most.
  Wheels then offer a direction test: two 250 ms pulses at 20% force by default (adjustable
  up to 50% for wheels that barely move), started explicitly by
  the player, are compared with the raw steering-axis motion (`ffb_calibration.rs`).
  Ambiguous motion is rejected; losing focus, leaving the page or cancelling releases the
  effects. The confirmed polarity is stored per device in `gamectrler.cfg` as
  `[openOMSI.FFInvert]` (0 normal, 1 inverted), and can be changed on that device's page.
  Devices without this setting still use the existing global `ff_invert` value.
* **Multiplayer** (`omsi-net::bridge`, `lan.rs`): UPnP forwardings are asked for an hour and
  renewed every 20 minutes (they were asked once for 7200 s); the ntfy.sh rendezvous is polled
  every 6 s (host) / 2 s (joining), posts only on change or every 15 minutes, and backs off
  10 s .. 5 min after a refusal (the old once-a-second poll ran into the relay's limits within
  an hour or two); a cloudflared that exits is restarted and its new address posted.
* **Depot files by date** (`omsi-map::ailists::ailists_with_chrono`, `depot_hof_on`): the
  launcher's HOF follows the chrono scenarios of the chosen date, as the game's AI already did.
* **Installations** (`omsi-cfg::install_search::root_guesses`, `content_folder_of`): a given
  path is trimmed of quotes, a file means its folder, the folders above and an OMSI folder
  inside are tried; a program unpacked into the OMSI 2 folder uses `<OMSI>/openOMSI` as its
  content folder, and a folder with `Omsi.exe` counts as the game even if an older build
  marked it as a content folder.
* **Phones**: launcher scale at least the system's (`ui_scale`), settings stacked in one column
  below 900 points, scroll areas pass what they cannot use on to the page; the game menu has
  its own scroll offset (`App::menu_top`) that a finger drags, a tap picks.
* **Sound** (`omsi-audio`): `mixer::distance_gain` = min(1, ref / d), DirectSound 3D's
  inverse distance with `[3d]`'s reference as the minimum distance (was (ref/d)^1.6);
  `SoundSet::outside_gain` / `lowpass_of` muffle only foreign vehicles' sets (`exterior`),
  never the player bus's own entries; `ambience::Footfall::own_bus` dulls steps on the other
  side of the player bus's body from the listener.
* **Keys** (`startup::own_keys`): bindings of the content folder's `keyboard.cfg` that the
  original's does not have are the player's; the presets and their extras skip those keys.
* **Materials**: `material_alpha` takes a slot's alpha mode from its first plain `[matl]`,
  not from a `[matl_change]` record ahead of it (script-texture masks of LED matrices).
* **Spline-attached objects**: `MapIndex` places a `[splineAttachement]` row's first object on
  its spline for `object_positions`, and the placed first instance replaces it; entry points
  use the `global.cfg` record's height where it differs from their object's by over 1.5 m at
  the same place (`spawn::recorded_entry_pos`).
* **Builds**: `release.yml` builds Windows x64/ARM64 (MSVC, ARM64 on the x64 runner), macOS
  arm64/x86_64 (both on the Apple silicon runner; the machine translation is Apple silicon
  only), Linux x64/ARM64 (`ubuntu-22.04-arm`), Android, and packs the server for Linux and
  Windows (`scripts/server/start.cmd`).

### 0.1.7 (Sept 28 2026): the bus as its `.bus` file makes it, sharp screens, trees, steering, updates

Reverse engineered from Omsi.exe and put in place of our own guesses:

* **Driving physics** (`omsi-sim::rigid`, OMSI's integrator is sub_7e2574; ODE there only
  knocks over crash objects). Across the tyres OMSI has no slip angle: in its holding state
  (`+0x1d8`) the bus follows its steering geometry outright, and only when the bend asks more
  than mu x the load on all tyres, or a wheel spins or locks, does each axle slide with at
  most mu x its load, until the side speed at every tyre is under 0.1 m/s again. Ours had a
  tyre of 12 x load per radian: every bus turned at 70 % of what its steering asked, 0.8 s
  late, drifting 2-3° - the same for every file. Now a constraint per tyre with those limits.
  Every axle steers at atan((long - `[rot_pnt_long]`) x curvature), curvature = steering x
  `[inv_min_turnradius]` (0x7e3060; a tag axle behind the line steers the other way), with no
  rate limit of the physics' own (the inputs have theirs). Springs: travel measured at
  (maxwidth + minwidth) / 4, force and damper speed at maxwidth / 2 (0x7e47b3..0x7e4c8d),
  spring + damper capped at `achse_maxforce`, the tyre at least 15x the spring (94 % of the
  file's rate reaches the body; the fixed 900 kN/m tyre left 79 %). Pitch and roll damped by
  sin(x)/x a step, x = 1.5 sqrt(sum springs / mass) dt (0x7e4f55); no yaw damping.
  `cargo run --release -p omsi-sim --example handling -- <file.bus>...` prints yaw response,
  side slip, roll, roll frequency and settling per file.
* **Mouse steering** (0x6f4284): the window's full width is the full lock, divided by
  max(1, km/h / 10); a one-second ease-in after switching on; the mouse owns the wheel (a
  steering key's leftover no longer takes over when the cursor passes the middle). The speed
  in that divisor is smoothed over 0.4 s and the wheel eases towards its target in 60 ms: the
  raw speed made the wheel creep on by itself and come back in steps.
* **Keyboard steering and pedals** (sub_7e614c / sub_7d5124): OMSI's curvature grows by
  0.00005 per ms a key is held (`steering_linear`), `old_steering` leaves it where it is when
  the key is let go; the clutch key presses the pedal at once and it comes up at 0.7/s.
* **Graphics device.** The instance asks Vulkan, then DirectX 12 (Windows), then OpenGL
  (`graphics_api`, `OMSI_BACKEND`); a card below wgpu's default limits gets its own limits
  (the shadow map and any texture larger than the card takes are scaled down to fit). A GPU
  validation error is logged and the game goes on (wgpu's default handler ended it), and the
  launcher says when a game ended on a lost device and offers DirectX 12. The Windows
  download carries `dxcompiler.dll` and `dxil.dll` for DirectX 12's shader compiler.
* **Mirrors**: `[add_camera_reflexion]` cameras sit in the body's own matrix (pitch and roll
  included, `Camera::roll`); `dist` puts the eye behind the point; the 8th value of
  `[add_camera_reflexion_2]` is the radius of the sphere OMSI tests against the view frustum
  before it redraws a mirror (0x6f611f, 0x7f41ac) - mirrors out of view are not redrawn, the
  visible ones take turns (with at least 0.3 m, or a mirror half in view stood frozen).
* **Screens**: text and script texture slots are `MaterialExtra::screen`; the enhanced pass
  writes a screen mask (`MASK_FORMAT`, second colour target) that the glow's first level
  and FXAA read - FXAA had halved the contrast of the IBIS's letters.
* **Trees** (`[tree]` objects, OMSI's RefreshTrees 0x77e6b0 / 0x774444): the map's strings
  are texture, height and the width/height ratio; the billboard is height x ratio wide. We
  divided by the ratio: a slim fir (0.4) was six times too wide.
* **Tiles**: objects and spline attachments have pitch, bank (and a tilt flag) only from
  tile version 12, strings from version 4 (0x792ee7, 0x794892).
* **Touch wheel**: turns with the finger round its centre (120° of rim = full lock); the old
  drag across ran off the screen at about 0.4 of the lock to the left.
* **Updates** (`updater.rs`, `launcher/update.rs`): the GitHub API's latest release, the
  platform's asset by `release.yml`'s names, SHA-256 from the asset's `digest`. A computer
  unpacks into `.openomsi-update` beside the program and swaps each top-level item (old one
  renamed `*.old-update`, all undone on a failure; `.openomsi-files` lists what an update
  installed, so files a release drops go too; nothing else in the folder is touched), then
  starts the program file again and ends; the next start deletes `*.old-update`. Android: a
  PackageInstaller session (`OmsiActivity.installApk`), the status PendingIntent comes back
  to the activity (cancel → error in the launcher, success → the system starts the new app);
  "Install unknown apps" is asked for first. `openOMSI/env.txt` gives a phone `OMSI_*`
  switches. The JNI calls now go to the real NativeActivity (`AndroidApp::activity_as_ptr`):
  ndk_context's context is the Application - the buttons' vibration never reached Java.
  Checked: desktop end to end on macOS (update, not now, auto, damaged file, read-only
  folder), Android end to end on the emulator (permission, cancel, update and restart),
  Windows `cargo check`.
* **Windy trees** (`windy_trees` setting, Graphics tab; `OMSI_WINDY_TREES=0/1`): foliage of
  plants (by `[groups]` or file name; `[tree]` cards too) bends above the crown's pivot as a
  cantilever (shader.wgsl `tree_sway`): drag lean ~ v^1.5 (Vogel), gusts a frozen field
  carried at the mean wind (Taylor, turbulence intensity 0.3), the tree's own swing at
  1.6/sqrt(height) Hz, in every shadow pass too. `[tree]` cards are 2x6 quads with rows on the
  pivots. `OMSI_RENDER_CLOCK=<s>` starts the animation clock later for offscreen stills.
* **Window condensation** (`condensation.rs`, Enhanced): an energy and vapour balance of the
  player's cabin - heater on the warming coolant with a thermostat, body heat and breath (wet
  coats in rain), skin losses, air changes from fans, speed and open doors (an entry+exit door
  counted once) - and each pane's film from the dew point at its inner face (Lewis' relation).
  The defroster's patch grows as fast as its jet dries the film; a new bus or a jump starts
  afresh. The shader takes optical depths: half the scattered light is diffracted (post.wgsl
  `misted` blur), half wide-angle as a veil lit by street and cabin (some 15 % back, Briscoe
  and Galvin). `OMSI_CONDENSATION=<min>,<people>[,engine]` for offscreen checks. Checked:
  Grundorf SD202 D86 driver view in Schmuddelwetter, engine on (sides misted, windscreen
  defrosted) and off (all misted, frame and dashboard sharp); trees in an 18 m/s storm.
* **Enhanced paint**: a `[matl_envmap]` body is a clear coat (f0 0.04, its factor x mask its
  smoothness: 1 and up polished, 0.05..0.2 satin trim, unmasked parts rough); bodies painted
  through a `[matl_transmap]` (most mod buses and cars) reflect too - they were left out
  and showed matte paint beside reflecting windows. Enhanced+ traces the coat like glass.
* **Night adaptation (Enhanced)**: the lamp meter (`view_lamp_light`) takes the ground where
  a grid of rays through the picture meets the street (the player's vehicle's height), not
  points 1.6 m under the camera (in the air from any higher camera: a moonless night was
  metered over lamp-lit lots and every lamp's light went white on modded maps), and adds
  the lamps' glare in view (Stiles-Holladay veil, `glare_veil`). Checked on Derevenka
  (mod map) against 0.1.1740 and on Grundorf.
* **0.2.3 follow-up**: night adaptation never goes below the lit district's level
  (`ARTIFICIAL x city_glow`, no 0.3 floor: the eye's dark adaptation takes minutes, so among
  lamp-lit streets it stays with them; a lampless road keeps the moonlight). Derevenka's lit
  parts now render as in 0.1.1740. Envmaps match the vanilla picture's strength: OMSI's
  share (mask, or pane alpha x factor) is taken as the angle-averaged reflectance, f0 =
  (share - 1/21) x 21/20 (Schlick's mean), for masked/transmap paint and for glass; the
  photo's structure is laid on in full (clamped 0.12..3.5 of its mean). Checked on the EN92
  (transmap + mask body, Fenster panes) against vanilla, and the driver's windscreen.
* **Running for the bus** (`omsi-sim::people::runners`, not Omsi.exe's): a bus on its way
  to a stop near the player and still 150 m off or more has one roll (`OMSI_RUNNER_CHANCE`,
  0.05; 0 off), and somebody is put out on the pavement of the stop's street (within 12 m of
  the line of the pavement at the stop: no paths into yards, buildings or side streets) out of the
  player's sight, walking along to the stop through the pavement network
  (`route_to_stop`): from before it, or from beyond it, coming towards the bus - put out as
  far along as they walk until the bus gets there (taken to come at 6 m/s; there first, they wait at the stop as anybody), to be 35-60 m
  before the stop or 25-45 m beyond it then. Somebody before the stop starts to run as the
  bus overtakes them, somebody beyond it once the bus stands; they run along the pavement
  and only the last 15 m straight to the doors (passengers walk straight to their target,
  through whatever is in the way). Nobody appears in sight, and nobody put out is taken
  away for room or for being far off until the bus has been. With them gone, somebody
  else walking within 50 m runs; a timetable bus with nobody near gets somebody from
  behind it out of sight; the player's bus does not - whoever runs for it was there to be
  seen. (Earlier tries: rolled as the bus pulled in, nobody could be put out of sight;
  placed once it stood, everybody came from behind out of sight; put out 320-450 m off only
  on the stop's own path, nobody came from beyond, and the ones before it stood waiting,
  too close, and were taken away beyond 400 m.) A run is a gait of its own above 2 m/s
  (`human_omsi`: longer strides, the knees a little higher - at most 100 degrees, they had
  folded through the thighs - a lean, the elbows bent; the walk below is Omsi.exe's as
  ever), at 3.0-3.6 m/s; a timetable bus waits for them up to 12 s (`holds_bus`). The
  player's bus is never held. A bus that leaves without them once it has stood: they stand
  2 s, then walk back to wait. `OMSI_DEBUG_PAX` logs who was put out where, when they ran
  and why, and why nobody could be put out.
