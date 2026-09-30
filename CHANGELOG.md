# Changelog

Every push to `main` is released as `MAJOR.MINOR.COMMIT` (see
[docs/VERSIONING.md](docs/VERSIONING.md)); the downloads are on the
[Releases](https://github.com/turbo-devv/openOMSI/releases) page.

## 0.1.342 - 2026-09-30

### Performance
- Busy spline scenes cost far less CPU: short static kerb, grass and pavement splines are
  drawn together per 48 m cell, and `[terrainmapping]` spline faces share the tile's ground
  materials (#340, by TruckiHD; for #284).

### Multiplayer
- Another player's articulated bus has its rear section lit, with its displays and its
  sounds (#338); its roller blind shows the line number (#334); a passenger in another
  player's bus hears it from inside (#330) - all by Jaja80330. The network format changed
  with #334: players and servers need this version together.

## 0.1.330 - 2026-09-30

### Driving
- The driver's and the passengers' views ride with the bus: their cameras hang on the body
  as Omsi.exe's do, pitching under braking and leaning in bends with the cab, the mouse
  look turned in the bus's frame. The level view with the cab rocking about it was most of
  the "boat" - the body's own heave, pitch and roll already settle as Omsi.exe's do.
- Mods with physics of their own: `Brakeforce` and every `Axle_Brakeforce_*` go back to 0
  after the physics read them, as Omsi.exe clears them each frame before the scripts run -
  a script that brakes only now and then (a retarder, a stop brake, custom physics) no
  longer leaves the brakes on for good.

### Traffic
- Timetable buses pull into the bay: they move over to the `[busstop]` box as Omsi.exe
  moves them - the kerb-side flank 0.3 m past the box's centre, from the stop's docking
  distance (30 m) out - whether or not a path leads into the bay (#241).

## 0.1.328 - 2026-09-30

### Vehicles
- The wheels stand on the road: the suspension's spring point is where Omsi.exe puts it,
  on the model's origin plane under each wheel, measured straight up. Measured from the hub
  less the `.bus` file's tyre radius, a mod whose tyre mesh is larger than that stood with
  its wheels sunk a few centimetres into the spline.

### Pictures
- Enhanced: chrome and metal parts of a vehicle - opaque, with a sphere map, not the body -
  are metal by their `[matl_envmap]` factor, as the vanilla picture shows them; their bump
  maps bend the reflection. The body stays paint unless it has a mask of its own.
- LED destination panels glow in the enhanced picture; "LED glow" and "LED masks keep
  their mipmaps" are settings (#324, by NACHN).

### Sound
- People walking in the street no longer sound as if they walked on a bus floor (the
  passengers' step samples, `Sounds\Passengers`, are for passengers aboard), and the stair
  samples are left out of the steps (#236).

### Game
- Teleporting to a street picked on the city map works across a big map: the navigator's
  lanes of the whole map are taken when the loaded tiles have no street there, and the bus
  waits at the street's height for its tiles instead of dropping through (#235).

## 0.1.323 - 2026-09-30

Crash reports from phones, a lost graphics device on DirectX 12, the vanilla night, and five
pull requests.

### Crashes and reports
- Phones: an app the system ended in the background (or that was swiped away) is no longer
  reported as a crash at the next start - most of the "closed without a word" reports were
  that. A report sent to GitHub carries the end of the log, and the whole report is on the
  clipboard as well; the renderer names each stage it compiles, so a report says where a
  driver gave up.
- A phone whose Vulkan driver went down while the shaders were being compiled (the reports
  that end at "cloud noise made") draws with OpenGL from then on (Settings → Graphics API
  takes it back).
- A graphics device lost on DirectX 12 starts the game again on Vulkan, as one lost on
  Vulkan starts it on DirectX 12. The launcher, too, makes its device again on the other
  interface instead of drawing on a dead one with thousands of errors (#274, an AMD Radeon).
- The automatic texture budget stays at 2.5 GB: since 0.1.237 a PC with 32-64 GB let the
  textures take 4-8 GB (#277).

### Pictures
- Vanilla: the texture times the light as Omsi.exe multiplies them, in gamma space - nights
  were several times too bright, a late dusk instead of the dark (#300).
- Night maps switch on with the street lamps, fully, as in Omsi.exe, instead of fading in
  with the dusk (a clear evening showed lit windows at a fraction, #276).
- Enhanced: chrome and other opaque sphere-mapped parts reflect again (#266, #264).

### Vehicles
- `[kmcounter_init]` starts the odometer at the bus's years in service times its
  kilometres a year (#305).
- Phones: a manual gearbox whose dashboard answers to the automatic's keys shows the manual
  gate (#279).
- The automatic clutch's help is for gearboxes that read the clutch pedal only: an
  automatic with number-key gears had its clutch pressed at stops (#234); a script without
  `engine_n` no longer keeps the clutch down for good (#260).

### Pull requests
- Merged: #298 (backwards meshes of exporters with a positive determinant: the Citelis'
  dashboard lamps, by ThiBot77), #307 (force feedback on Logitech and Moza wheels, by
  tistron), #310 (a warning when the driver uploads far too slowly, by ThiBot77), #313 (all
  buttons of a Linux wheel in the launcher, by ThiBot77), and #240's scenery-object support
  for HTML textures (by shloooo).

## 0.1.307 - 2026-09-30

Passengers, bus physics and light maps checked against Omsi.exe once more, and ten pull
requests.

### Passengers
- A bus that is not in service (no valid destination, or a "$allexit$" one such as
  Betriebsfahrt) or that stands at its own terminus empties there and takes nobody on, as
  Omsi.exe does (0x61f3e3). People boarded buses showing nothing; nobody got out at the
  last stop of a late trip (its stop index started again at 0 with the next trip - riders
  now go by the stop itself as well).
- Riders get up as the bus pulls in to their stop, not once it stands.
- Everyone on the way out holds the door request the whole way, as in OMSI: the automatic
  rear door no longer shuts on the next person walking up and opens again ("the door
  doesn't know whether people are getting off"). The requests are pulses, cleared after the
  vehicle's scripts each frame (0x7d6214): a timetable bus out of the passengers' reach no
  longer keeps its door open for good.
- The front of a queue stands aside while people get off: both used the same spot at the
  door and each waited for the other (#253).

### Physics
- `[momentofintertia]` on Omsi.exe's axes: roll is the third value, yaw the second (the
  SD202 rolled on 80 t m² instead of 300 - twice as fast, rocking over every uneven patch:
  the "boat").
- Speed bumps, cushions, manhole covers, lowered kerbs and slab edges are felt again: only
  faces under 2 cm over the road count as paint (4.5 cm took them away, and the bottom of
  every bump's ramp).
- When nothing is found under a wheel the ground is looked for up to 3 m above, as
  Omsi.exe's ground query does - a bus no longer falls through where it sank into a joint.
- An articulated bus's rear section rides on springs: a bump under its axle is a jolt.

### Light maps
- A light-mapped material is lit as D3D lights it: the material's own light and colour
  times every light - the saloon lamps included - clamped, then the light map laid on with
  ADDSMOOTH. The saloon lamps are no longer counted twice (flat white where both were on).
- Enhanced lays the light maps the same way: little by day, fully at night.
- Several maps on a slot chain as ADDSMOOTH and switch on at 0.5.

### Duties and traffic
- On a circular or turn-back route the duty no longer jumps to the stop over the road:
  stops are told apart by the direction the trip runs through them (#254).
- AI cars follow bends tighter than their model's lock instead of running wide through
  kerbs and corner houses (#249: 851 → 301 moments of a car over 1.5 m off its path in
  150 s of Spandau traffic).
- The view reset restores the zoom and the outside camera's distance (#244, with #281).

### Pull requests
- Merged: #237 (own number plate), #240 (HTML textures, by shloooo), #257 and #263 (AI
  bus displays and rear sections, by NACHN), #281 (H-pattern `kw_s_*_fest` gates, tour
  start and end in the chooser, by isaacsa2; #280 is the same), #282 (road markings and
  rails near the camera, by TruckiHD), #287 (tile light maps on their middle third, by
  Sulamufor), #288 (issue templates, by shloooo), #290 (mouse throttle reaches full, so
  automatic gearboxes kick down, by Sulamufor - taken without its build folder).

## 0.1.238 - 2026-09-30

Manual gearboxes, dashboard lamps, phones that crash or run slowly (#226, #231, #229, #225).

### Manual gearboxes (#226)
- With the automatic clutch on (the default), a gear chosen with a key, a phone's gear
  button or a controller comes with the clutch pressed and let up again, as OMSI's clutch
  key does, for gearbox scripts that only take a gear with the pedal right down and do not
  work the clutch themselves (the LiAZ and PAZ KPP: `(L.L.clutch) 1 =`). Before, a player
  without a clutch pedal - every phone - could not put such a bus in gear at all, forwards
  or backwards. Scripts that read `AutoClutch` (the Sprinters' G32) still do it themselves.
- The automatic clutch's pull-away help (the clutch bites as the throttle goes down, so the
  engine does not stall) works for these scripts' `antrieb_getr_gang` too, and is left to
  the scripts that work the clutch themselves.
- "Automatic clutch" can be switched in the launcher (Controls) and in the game menu; with
  it off, a phone shows its clutch pedal.
- The phone's gear buttons light the gear engaged for either kind of script.

### Dashboard lamps (#231)
- A `[matl_change]` variant shows as Omsi.exe shows it (0x5fd6xx): the variable rounded to
  the nearest whole number picks the `[matl_item]` (1 = the first), anything else the plain
  material - a lamp whose variable stands at 2 with one item is dark. A variable no script
  declares counts as 0, as the model loader registers it: the stock MANs' spare buttons
  (switched by `*Noch nicht belegt*`, "not assigned yet") and mods' door button lamps were
  lit all the time.

### Phones (#229, #225)
- The game's log is written on the phone too (`game.log` in the app's folder, the previous
  run's as `game-prev.log`), with the device's maker and model. A run that closed in the
  middle of a drive - a graphics driver taking the app down without a word - is shown by
  the launcher at the next start, with the end of its log for "Copy report".
- The first drive after such a closing starts with safer graphics, and on OpenGL when the
  one that closed drew with Vulkan.
- A phone's graphics chip always gets the light picture (no SSAO, no MSAA, small shadow
  maps), whatever type its driver reports.
- The automatic render scale has a fourth step, 55 %, for a chip that is still too slow at
  70 %.

### Checks
- `OMSI_DEBUG_VARS` with `OMSI_DEBUG_VARS_EVERY=<s>` logs the variables through an
  offscreen `--drive`; a manual gate given with `--triggers` comes with the automatic clutch
  as from the keys.

## 0.1.237 - 2026-09-30

The testers' second round: the crashes with "the graphics device was lost", weak cards and
phones, traffic that stood on free roads and roundabouts, passengers' necks, VR, gamepads,
and six pull requests.

### Crashes and graphics cards
- A lost graphics device (the driver reset the card: "the graphics device was lost",
  #219, #223) no longer ends the drive: the game saves the situation and starts again on
  it by itself with lighter graphics (no MSAA, no SSAO, smaller shadow maps, mirrors and
  texture budget; on Windows DirectX 12 when it was Vulkan that was lost), at most twice. The launcher does not
  report such a restart as a crash.
- Windows tries DirectX 12 before Vulkan.
- When the card runs out of memory, the textures are cut down (to 60 % of the budget each
  time, not below 300 MB) before the driver gives up.
- The interface's vertex buffers are made where a failure can be seen: after the card ran
  out of memory, one invalid buffer was written to every frame, flooding the log with
  thousands of GPU errors and taking the frame rate down to 12 fps (#217). A failed one
  is now made again at the next frame and nothing draws from it meanwhile.
- The automatic render scale moves in three steps (100, 85, 70 %), at most every five
  seconds. Every 5 % step every two seconds made all the picture's targets anew -
  hundreds of MB each time - a stutter and memory the driver ran out of. At the smallest
  scale and still too slow, SSAO and then the shadows go off.
- A small or shared graphics chip (integrated graphics outside a Mac, a phone, a card of up
  to 2.5 GB, OpenGL) is drawn without SSAO and MSAA; a card of up to 4 GB without SSAO and
  with at most 2x MSAA. `OMSI_FULL_GPU=1` keeps the settings as they are.
- The status log shows the GPU memory the textures and meshes take; on Windows the
  machine's memory sets the texture budget.
- Textures shrunk while far away come back whole at once when they are near again:
  buildings right in front of the bus stayed blurred on a map that filled the budget.

### Traffic
- A roundabout's entry no longer waits at its line for a gap at the far side of the ring:
  a nine-second gap that never came kept the queue standing for minutes (Westcountry: no
  car stuck any more, mean speed 13 -> 21 km/h).
- A hold of one frame winds a waiting driver's reaction back only a little: a junction
  "free, not free" by turns kept cars about to go for good, on open roads too.
- A car at the stop line when the light turns green goes; a green of a second let nobody
  through before.
- Nobody waits for a car of the ring that is itself creeping in a queue.
- `OMSI_DEBUG_STUCK` names the light programs and the hidden reasons a car holds.

### People
- Passengers who look at the bus turn their shoulders with it, and the head turns no more
  than 45 degrees on them. The people have no neck bone: a head turned 60 degrees on still
  shoulders twisted the neck.

### VR
- The bus's own head movement is off in the headset (the cab swayed before the eyes).
- The sphere-map reflections are laid out by the bus's heading, not by each eye's view:
  they no longer swim with every turn of the head.

### Controllers
- Gamepads (#200): the stick sets where the wheel turns to, on a gentler curve and less the
  faster the bus goes, and the wheel follows at a hand's pace; the bus no longer swerves
  with every touch of the stick.
- An Xbox pad on Windows named in OMSI's `gamectrler.cfg` keeps its sticks and triggers
  (#171).
- Force feedback (#224, #230, by tistron): DirectInput wheels have their own centring
  spring turned off before they are acquired, and again when they are acquired anew; the
  steering is lighter while turning, heavier when parking, centres itself under control
  and follows the bus's sideways acceleration; the front wheels' bumps and kerbs are felt
  as short vibrations (also in a gamepad's rumble). Steering force and vibration are set
  per controller under Controls -> Game controllers and kept in `Inputs/gamectrler.cfg`.

### Pictures
- Raindrops on the glass are lenses (#228, by Jaja80330): each drop shows the world behind
  it upside down and mirrors the sky; drops sit in three sizes on turned grids, a mist of
  droplets greys the pane, and runners slide down in fits and starts, wiping a track and
  leaving beads behind. Storms are denser and less regular (#222, by TruckiHD).
- `[rendertype] presurface` objects draw before the terrain, so excavations under the
  ground show through their invisible covers (#218 by TruckiHD, #215).

### Sound
- The player's bus's own sounds keep their pitch while the camera follows it: sound and
  listener were moved at different moments and the Doppler shift made them waver (#214,
  by TruckiHD).

### Launcher
- The timetable chooser shows the chosen trip's duration, in words as OMSI's BBS writes
  them (#233, by tistron).

### Checks
- `OMSI_AUTOPILOT=<km/h>` (offscreen): the player's bus follows the lanes and logs where
  it stands against the ground, for roundabouts and places buses fall through.

### Pull requests
- Merged: #214, #218, #222, #224 / #230 (the same commits), #228 (with #222's hash; its
  own patches replace #222's density field), #233.

## 0.1.221 - 2026-09-30

Everything since 0.1.178: the testers' reports from Fikcyjny Szczecin (MAN NL/NG Enhanced),
KS Węglin, Cotterell and The Adstow Project, multiplayer, the trains, ten GitHub issues and
six pull requests. Where OMSI 2 has the behaviour, it was taken from Omsi.exe itself (the
addresses are in the commits).

### Driving and physics
- The suspension is Omsi.exe's: the body hangs on a spring and damper at each wheel over the
  ground point under it (`achse_feder`, `achse_daempfer`, `Axle_Springfactor`, capped at
  `achse_maxforce`), with no tyre spring, wheel mass or bump stop of our own in between.
  Buses no longer float over the road "like a boat"; every bus drives on its own `.bus`
  values. (`OMSI_TYRE_SUSPENSION=1` brings the old model back for comparison.)
- The driver's head moves as in OMSI: thrown by the body at the eye, up and down always,
  sideways and fore and aft with `[driverview_moving]`, never more than 10 cm.
- Mouse steering switched off (right click, O, the menu) leaves the wheel where it is; the
  keys go on from there (#184).
- While the view is turned with the mouse, the cursor shows OMSI's four arrows (#185).

### Keyboard
- `Inputs/keyboard.cfg` is read as Omsi.exe reads it (#195): the third value's bit 1 means
  "the action follows the key's state" (throttle, brake, steering), 2 is Shift and 4 is Ctrl.
  The stock driving keys no longer need Shift; parking lights are Shift+L, the quicksave
  Ctrl+S, the screenshot Ctrl+Shift+P, the information display Shift+Y, the timetable Insert
  and the ticket desk camera Home. Rebinding a key in the launcher keeps the entry's own
  "held" bit; an Alt chord of our own is bit 8, which OMSI ignores.

### Pictures, lights and mirrors
- Material highlights are Direct3D's per-vertex specular term from the sun and the light
  above, as in OMSI: gear selectors, buttons and screens no longer catch sharp sun spots.
- `[matl_envmap]` on glass and paint blends as Omsi.exe blends it: windows are no longer
  mirrors of the street; the enhanced picture's glass reflects 4-12 %.
- `[matl_lightmap]` is on or off at its variable's 0.5 and is added to the light before the
  texture (ADDSMOOTH): lit saloons glow at night and hardly show by day.
- Mirrors and door monitors (the BMC Procity's `camera_TFT` among them) show what they
  reflect: each frame the ray from the eye to the mirror is reflected in the mirror's face,
  as Omsi.exe does; left mirrors, kerb-side blind-spot mirrors and middle-door monitors no
  longer look into the saloon or the sky (#192).
- A lamp's `[light_enh]` lights move with the mesh they belong to: a level crossing's
  barrier lamps stay on the barrier.

### Map objects
- Parked cars stand on the ground as Omsi.exe puts them, with the map's own pitch and bank,
  no longer tilted by the slope under them (Cotterell).
- Attached objects turn as Omsi.exe turns them (own rotation, then the parent's).
- The stop helper (`routearrows_busstop.sco`) stands on the stop object with its rotation;
  it no longer "cuts" into the bus beside it.
- A far AI bus keeps its destination sign instead of a flat colour beyond 50 m.
- Scenery whose free-texture filename is built in `{frame}` shows its texture (#198).
- `model.cfg` `[item]`/`[setvar]` are paint schemes of the model, as in Omsi.exe, and the
  chosen scheme's variables are there for the scripts' `{init}` (#190).
- Checks for road builders: `OMSI_CHECK_SPIKES`, `OMSI_HOLE_PHOTO`, `OMSI_ROAD_PHOTO_N`;
  `--cam` takes a field of view.

### Passengers and people
- Passengers get off where Omsi.exe sends them: each rider's stop is drawn among the stops
  ahead by the stops' "passengers alighting" numbers. They no longer all leave after one or
  two stops.
- Waiting passengers keep to their nearest door while it opens: a bus whose rear doors open a
  moment before the front one no longer sends the people at the front to the back.
- People on foot wait for a car or bus standing in their way on a crossing, then go round
  it, instead of pressing against its side.
- F2 reaches the passenger cameras of an articulated bus's rear section.

### Traffic
- Traffic keeps to the middle of its lane beside parked cars (narrow British streets, The
  Adstow Project).
- Random traffic keeps to its pool's path densities (`unsched_vehgroups.txt` pools with their
  own `[rule] trafficdensity`), and a positive density as low as 0.001 still lets cars on
  (#201, #199, from Aurora Studio). A car whose pool may go nowhere at a junction goes on
  where cars may instead of standing there.

### Trains
- `[trainreverse]` works as in Omsi.exe: a train whose next trip runs the other way is turned
  round where it stands - its last car leads - and goes on with the trip. The Berlin U-Bahn
  no longer drives off the end of its siding while another train appears for the trip back.
- Trains stop with their front at the station, as Omsi.exe measures it (half the train's
  length and the `[ai_brakeperformance]` holding offset): the S-Bahn and U-Bahn no longer
  stand half a car past the end of the platform. Train cars without `[boundingbox]` take
  their model's length (18 m, not 12 m).

### Multiplayer and servers
- No more micro-teleports: states are stamped with the moment of the frame they show, and the
  other players' buses and the host's traffic are drawn by a clock that runs smoothly
  instead of jumping with every datagram. Another player's bus: speed jitter per frame
  median 11 % -> 1.4 %; the host's cars at a client are drawn within 2 cm of where the host
  has them.
- A joining player sees the host's traffic and people whatever their own traffic settings
  ("passengers but no traffic" on a server).
- A server no longer stalls when someone drives a bus it cannot load: it tries again after
  half a minute, loads only the buses its `vehicles` list allows and shows the first of them
  for any other.
- Joining by code starts on the host's map.
- Session codes end in a full group of four characters (#152); old codes are still read.
- Each camera keeps where it was turned, as in OMSI 2.
- A door whose entry point lies on the aisle opens to the kerb: the left where traffic keeps
  left.

### VR (Windows)
- OpenXR VR support (#168, by EpixXx): stereo rendering with head tracking, a spatial Esc
  menu and cockpit pointer, right-click zoom, the headset picture on the monitor, its own
  settings and keys (Ctrl+Shift+R recentre, F7 monitor picture, F8 VR / desktop). See
  [docs/VR.md](docs/VR.md).

### Phones and on-screen controls
- With `OMSI_TOUCH=1` on a computer, the mouse works the on-screen controls as a finger
  (from #202).

### Translations
- Hungarian refined (from #143, by agost4002).

### GitHub issues closed
- #127 (an overlay layer drawn opaque), #151 (default specular), #152 (session code), #176
  (shiny windows), #184 (mouse steering), #185 (look cursor), #187 (envmap brightness), #190
  (`[setvar]`), #192 (mirrors and door monitors), #195 (keyboard.cfg bits).

### Pull requests
- Merged: #168 (VR), #198, #199, #201. Taken in part: #202 (the mouse as a finger; its fixed
  gear panel and the committed rustup installer were left out), #143 (the Hungarian lines;
  its edits to the English texts would have dropped those lines in every language).

## 0.1.178 - 2026-09-30

Everything since 0.1.146. Where OMSI 2 has the behaviour, it was taken from Omsi.exe itself.

### Roads, splines and the ground
- Roads no longer disappear under the grass. Splines the map marks `[spline_terrain_align]`
  cut their outline out of the ground, as Omsi.exe does: whole stretches of Spandau's roads,
  the six-lane Falkenseer Chaussee among them, were buried. The cut is exact to a few
  centimetres: no sky along the kerbs, and narrow medians stay green.
- The ground is no longer taken away under every road in rough 1.5-3 m steps (the "holes in
  the world" beside kerbs and car parks); only where the map says.
- Road cant takes its width from the spline's height profiles, as in Omsi.exe.

### Vehicles
- Bellows of articulated buses bend with the rear section on slopes instead of away from it.
- Skinned meshes (bellows, levers of mod buses such as the AA-FR Agora) deform as in OMSI 2.
- Headlights in the classic picture shine forward from the lamps, one beam per headlamp,
  as bright as in OMSI 2, and no longer light up the bus's own saloon and dashboard.
- Roller-blind destination displays (`[texcoordtransY]`, `[matl_freetex]`, borders) work.
- Thüringer Wald buses keep their roof at night.
- Mirrors see closer and further (0.1 m to the objects' range, as Omsi.exe).

### Trains
- Trains are put together as in OMSI 2: every unit with its cars, the last car turned round
  (Berlin U-Bahn A3, S-Bahn BR 275).

### AI traffic and passengers
- AI cars no longer wait for each other for ever: a long wait at a side road now gets its
  turn, and two cars that each waited for the other drive on.
- Passengers at a stop no longer all stare at the driver: each watches a coming bus on their
  own, and only the people it takes keep looking once it stands.
- Timetable buses' door handshake follows Omsi.exe (a trace: `OMSI_DEBUG_DOORS=1`).

### Weather and administration
- Weather cycle (launcher, phone launcher, `weather = cycle` in server.cfg): a new weather
  every 25-60 game minutes, fitting the month; every weather change blends in over 4 minutes.
- Server admins: set any installed weather, switch the cycle on and off, clear jammed traffic.

### Multiplayer
- The official server: type `openomsi` to join "openOMSI | Official Server"; it is first in
  the server list.
- Any server address works: an IP, a host name, host:port or a link.
- Parked cars are the same for everybody: a car that drove off at the host is gone for the
  other players too (their buses drove through cars only one side had).
- Joining keeps the duty on the host's map; the launcher never hangs on a job that died.

### Phones
- A launcher made for phones: tabs at the bottom, a Play screen, full-screen choice sheets.
- Manual gearboxes on the touch controls, with a clutch pedal.
- Installing mods works again (it stood at "reading the archive's table of contents").
- On foot, the own bus answers clicks.

### Performance
- Less stutter when the camera moves (culling buffers are kept between frames).

## 0.1.14 - 2026-09-28

### More fixes
- Esc → More → *Set the clock...*: the clock one, five, fifteen or sixty minutes on or back,
  and on a duty *On time with the timetable* (early or late by six minutes: the clock is
  put where the bus is on time).
- Discord shows "Playing openOMSI" with the bus, the map and the line (Rich Presence, over
  Discord's local connection; `discord_app_id` in the settings, `discord_status=0` turns it
  off).
- AI traffic no longer stands for minutes on a free road: a car crawling in a jam of its own
  kept its claim on the junction ahead, and the cars that give way to it waited behind it
  in a chain (one Golf stood 81 s on Spandau; none now in four minutes of 80 cars).
- On a road the wheels stand on the road, as in OMSI: the terrain over or through the
  carriageway (an embankment the road runs under, ground poking through the asphalt) was an
  invisible wall under bridges and a bump that threw the bus.
- Mod buses with a lamp test after the key (the GX7767 E500 MMC waits four seconds) start
  with Shift+U: the starter is tried for longer; an automatic gearbox that takes D only
  with the brake held (ZF, `(L.L.Brake) 0 >`) is put into D by the auto-start.
- Manual gearboxes: Ctrl+Up / Ctrl+Down shift up and down (gear levers with a trigger per
  gate, `kw_s_1`...`kw_s_10`, `kw_s_N`, `kw_s_R`, as the LiAZ MKPP), the clutch let up as
  OMSI's clutch key lets it; with *automatic clutch* on, the clutch bites by itself when
  pulling away, as far as the engine keeps its revs. *gear_up* / *gear_down* for buttons.
- A situation loaded with *Continue* keeps the bus's livery.
- Mouse steering keeps the wheel and pedals while the right button looks round, as in OMSI.
- The arrow keys' look with a wheel is a glance: held, the head turns (at most 140 degrees);
  let go, it comes back to the road.
- Esc → More → *Move the bus on the map...*: click a street on the city map and the bus is
  put there (Ctrl+click on the map does it too, now also on a duty).
- The pause menu no longer flickers its top line while the mouse moves over it.
- Phones: the on-screen wheel turns the bus's wheel one to one.
- Windows and Linux: the automatic render scale draws at full size up to 4K (it drew a 4K
  screen at 58 %, and enhanced graphics looked like low-quality textures).
- Automatic rear doors (SD202, SD200 and the like) close again: passengers walking to the
  door or standing at the back of the queue kept asking for it, and the script starts its
  closing time again on every request. A request now opens a shut door; an open one is held
  only by somebody in the doorway, as by the light barrier.
- Walls with a height profile on their top (the stone and brick walls of UK maps) are walls
  to the wheels, not a road: where a wall's top met the road the bus drove up onto it and
  along it as the road fell away.
- A spline's height profile lying well over everything the spline draws (Westcountry's
  yellow surface marking: paint 10 cm up, height profile 50 cm) is taken at the drawn
  height: it was an invisible wall across the road. `OMSI_CHECK_WHEELS=1` with `--offscreen`
  lists what the wheels meet along the driving lanes (for map makers).
- Mouse steering turns on to the full lock past the window's edge: with the cursor at the
  edge, moving the mouse on outwards keeps turning the wheel (the width of the window is a
  smaller part of the lock the faster the bus goes, as in OMSI, and at 30 km/h the edge was
  a third of it); moving back gives that turn back first.
- The free camera (F4): its keys (W A S D Q E, Space, Shift, the arrows) no longer work the
  bus as well (W switched the wipers on), and the mouse wheel zooms there and on foot
  (Ctrl+wheel moves the camera on).
- Snow is matte: it no longer takes the rain's gloss and shines like plastic in the lights.
- Modding: a mesh can be lit by up to 63 interior lamps (the extra numbers on the lines
  after the four of `[illumination_interior]`; OMSI 2 reads the first four), PBR maps up to
  4096 px, and 32 lights per 25 m of the world instead of 16. What openOMSI allows beyond
  OMSI 2 is written down in docs/MODDING.md and on the site.
- Door keys: Shift+1, Shift+2 ... are the bus's doors front to back, found from the model
  (where each door leaf sits along the bus and which leaves each door trigger moves). The
  LiAZ's Shift+1 opened its middle and rear doors together and its front door had no key;
  a mod door script that mentions a closing variable while opening had its two leaves on
  two keys (one leaf moved, the other needed its own press). The game's log lists the keys
  of each bus ("door keys: ...").
- The release notes list what changed since the release before (they said "Small changes
  and fixes" for every release without a section of its own here).
- Phones: 60 frames a second by default (the settings took the PC OMSI's limit of 30 from
  its options.cfg), and dragging the view turns it the way the finger moves (it was the
  other way round, left for right and up for down).
- A bus on a lower level (a car park under a building, a road under a bridge) is no longer
  taken for one fallen through the world and put up on the roof: it has fallen only with
  nothing under it at all. A teleport to a place with a height lands on that level.
- A map that uses objects or splines that are not installed says so when it loads (how
  many, and the add-on folders they come from), and every missing object, spline and
  texture is listed by add-on in `~/.openomsi/missing_content.txt` (written again when the
  game ends, with the tiles loaded on the way). Holes, bare roads and white objects of such a
  map are a missing download, not a fault of the game - now one can tell.
- The game's log records the whole session: the system (OS, processor, memory), the command
  line and every setting at the start; then everything said on the screen, each view, pause
  and resume, every key action and door key, and a status line every minute (frame rate and
  the worst frame, where the bus is, its speed, the view, the time, the traffic).
- The bus is no longer put down inside scenery: an object no taller than a vehicle that
  stands for a third or more where the bus is put (a mod map's static buses in its depot,
  a sign) is taken away for the session, as the object editor takes one away (the map's
  files are not changed).
- Barriers (depot and car park gates on a light program) open for the player's bus off the
  lanes too: a gate whose lane starts up to 25 m ahead, the way the bus faces, is asked for
  (in a depot yard the bus stood beside every lane and the barrier stayed down).
- Passengers: the queue at a front door no longer goes on round the bus's nose (it stops
  short of the front and turns out along the kerb - people stood across the road in front
  of the windscreen, facing the bus), and a door shut for a moment no longer sends the
  waiting people away: they wait on 25 s after a door of the standing bus was last open
  (they turned away at once and came back when it opened again).
- Camera monitors: `reflexionN.bmp` is camera N's picture wherever a vehicle's material
  names it (its light map, night map, a `[matl_item]` switched on by the script), not only
  as the plain texture - monitors that show the camera once switched on were white.
- "Doors are open" follows what the passengers are told is open (`PAX_Entry/Exit<n>_Open`)
  - mods use `door_<n>` for other things, and a bus with its doors shut said they were open;
  "Air pressure is low" is no longer said with the tanks full (the spring brake is then held
  by the bus's own parking brake).
- The rear doors of the Berlin buses (SD, NL, EN/GN) close on Shift+2: switching their
  release off with the doors open shuts them at once, rather than when the passengers'
  last request has lapsed.
- An entry point whose marker lies under the ground (nothing under its height at all) puts
  the bus on the ground above it, not in the void under the map; a real lower level (a car
  park's floor) is kept.
- Settings → Controllers → *Force feedback and vibration* (and Esc → More → Options):
  switches the wheel's forces and a pad's rumble off altogether (a pad left plugged in
  shook all the time).
- Spaces on displays and signs: a font without a space character (many display fonts have
  none) leaves the width of a narrow letter between the words; the words of a destination
  ran into one another.
- OMSI's held keyboard pedals: Settings → Controllers → *Keyboard pedals stay where they
  are* (and Esc → More → Options). Tap the brake and it keeps that pressure until the
  throttle is tapped, and the other way round.
- Settings → *Reset all settings...*: every setting back to how it came (the language, the
  drivers, the key bindings and the game folder stay), after a dialog that asks first. The
  quality presets are under Performance.
- The bus radio also plays the stations of OMSI's radio plugins (SuperRadio's `.opl` and
  its lists under `plugins`): every stream address found there is a station, after the
  ones of `~/.openomsi/radio.cfg`.
- Controls → Game controllers: a button pressed on the wheel lights its line in the list for
  a few seconds, and the status line says which button it is and what it does - press it and
  give it an action right there.
- Railway signals clear for the player's own train as well (driven on the rails): its
  signals stayed at stop, as only an AI train ever asked for them.
- Puddle splashes are a mist of water - soft, lit by the scene, widening and thinning out as
  it sinks - instead of rings of glowing light flying off the wheels.
- Enhanced graphics: the sky is drawn again after a third of the way it waited before, so
  the clouds no longer drift and jump back into place when the camera flies fast.
- Launcher: the bus list is built again only when the search, the buses or the host's list
  change (it was rebuilt every frame, every name copied - scrolling it stuttered on phones).
- 14 more interface languages: Українська, Беларуская, Қазақша, Polski, Čeština, Magyar,
  Español, Português (Brasil), Italiano, Nederlands, Türkçe, 日本語, 中文 (简体), हिन्दी -
  with English, German, French and Russian 18 in all (Settings → Language), and every text
  of the launcher and the game menus the tables lacked now translated in all of them. The
  tables are in the program, so they work on every system (the machine translation, which
  runs on Macs with Apple silicon only, is not needed for them). Chinese, Japanese and Hindi
  are drawn with the system's own fonts. OMSI's own texts (key names, descriptions) show in
  English where OMSI has no such language.
- Shift+U after a crash starts the bus again: a bus under power whose engine had died was
  taken for a running one and "switched off" round and round ("Shutting down..." for good).
  An auto-start that has gone on for 20 s is begun again by the next Shift+U.
- The weather turning to snow (Next weather, or the weather file) no longer drops the bus
  through the world: every tile is read again with the winter textures, and while the one
  under the bus is away the bus is held where it stands (it fell, was put back in the sky
  and fell again).
- Passengers in an indoor station stand on its floor, not on its roof: walking, they took
  any surface over them for a kerb to step up on. They now keep to the floor within a step
  of where they are (a station's floor under its roof, a car park's level under the deck).
- Keys the player set in `Inputs/keyboard.cfg` are theirs, also when they edited the
  installation's own file: Z / X / C (the indicators), Shift+number (the doors), W A S D and
  the arrows no longer take over a key bound to something else. What counts as changed is
  told from OMSI 2's own assignment, built into the game, with Shift held as well.
- The hazard lights go off again (X, and the phone's hazard button): pressed with them on,
  the key let go of the indicator lever instead of their own switch.
- Seated passengers on a high seat (on a podium, over a wheel arch) let their feet hang as a
  sitting body does, instead of stretching the legs straight down through the seat's front
  to the floor far below.
- The bus no longer spawns floating on a wall's top: the place it is put down at is the face
  its wheels stand on near the entry point's height (a road, a deck, an underground floor),
  not the highest surface of the map's height raster there, which is a wall's top beside a
  pavement (London) or a deck over the road.
- The sound follows the system's output device: a Bluetooth headset or headphones connected
  while the game runs take the sound over, and disconnected, the sound comes back on the
  speakers (it had stayed on the speakers, or stopped for good).
- Maps whose `[map]` list names a tile twice (Westcountry 3 names 33 tiles twice): the tile
  numbers the map's files use count those entries, as in OMSI. Counted without them, every
  number after the first repeat named the wrong tile: rows of objects repeated along a road
  (fences, bollards, lamps) hung from another tile's row and stood across the road or were
  missing (3 of 1536 rows found their start on Westcountry 3, now 165, 159 of them where the
  map says), timetable tracks ran over the wrong tiles, and entry points were looked for on
  the wrong tile.
- An entry point is found on its own tile: a map joined from two (two towns you cannot drive
  between) repeats object ids, and choosing a stop in one town put the bus on the grass of
  the other, where the other object of that id stands.
- Road markings laid over road markings (where lines cross, a box junction over a lane's
  arrows) are no step for the wheels: every layer of paint is looked through, not only the
  first.
- Traffic of the UK car packs (WH UK AI: Westcountry, London and others) is no longer
  invisible, only shadows and lamps driving about: a mesh written before a model's first
  `[LOD]` belongs to that level, as in OMSI. These cars put their shadow there; as a level
  of its own it was all a moving car had.
- Their paint: a `[matl_transmap]` picture without an alpha channel is opaque, as Direct3D
  reads it (the cars' paint layer has a black 24-bit `transmap_null.tga` and was invisible),
  and a layer drawn over another mesh of the same shape keeps its blending (the baked
  shading over the paint had been made opaque: black cars, black roofs).
- An object's `[LOD]` level is chosen as OMSI does: the first level in the model's order
  whose size the object reaches, else the last. The stock Sv signals list their detailed
  level before their low one; sorted by size, the low one stood in close up and the signal
  vanished in the distance. A model with a single `[LOD]` is drawn at any size.
- A car that has reached a dead end goes after 25 seconds, even in view, when others are
  waiting behind it: a fire engine at the end of a dead-end street held a queue of fourteen
  cars for two and a half minutes, and the junctions before it jammed full (Westcountry 3,
  38 cars stuck for over a minute in five minutes of traffic, now none).
- No more sky showing through the road in stars and stripes at junctions: a spline made
  only of blended layers (Westcountry's lane darkeners laid over the junctions' painted
  ground) no longer cuts the ground away under itself; the ground is what it darkens.
- `OMSI_DEBUG_LAMPS=1` lists every traffic light object, the crossing it belongs to and
  those that name none (and so stay dark).
- Esc → Destination display → *Route number*: the route (line) number on the displays, from
  the depot file's routes and the map's timetable; the destination stays.
- Windows: a force feedback wheel (G29) no longer pulls itself to the middle after the pause
  (taken back by the game, its own centring spring came back on).
- `OMSI_CHECK_SPLINES=1` with `--offscreen` lists the map's spline chains whose ends do not
  meet (for map makers).

### Controllers
- No hidden dead zone on wheels any more: gilrs's default filters took 10 % of every axis (90
  degrees either side on a wheel of 1800) and held back small movements; Windows: the
  driver's own DirectInput dead zone and saturation are cleared, as OMSI does (PXN V99).
- macOS: a device with sliders or the simulation page's axes is read as a wheel from its HID
  elements even when SDL's list calls it a gamepad (HORI Truck Control System: accelerator
  and brake stayed merged and the steering had a gamepad's dead zone).
- The Controllers page offers the view actions for buttons: *view_look_left/right/up/down*
  (look round while held), the interior cameras, the views. OMSI's view actions on buttons
  work in the game.
- Force feedback in every view of the bus, not only the driver's.
- The mouse no longer freezes the picture: a gaming mouse's thousands of moves a second each
  looked for the switch under the cursor, and no frame was drawn while the mouse moved.
- Mouse steering shows a cross as the cursor, as in OMSI.
- Phones: the on-screen wheel turns one and a half turns to the lock, as a bus's does, and
  comes back by itself when let go.

### Game
- P pauses into the pause menu in a LAN session too (the session goes on for the others).
- The launcher starts the game that came with it, not a path remembered from an older
  installation (on macOS it kept starting a build of the days before the rename).
- Esc → More → *Depot file (HOF)...*: choose the bus's depot file by hand. Placing a vehicle
  asks for its livery and depot file.
- A bus that could not be loaded is tried once more, and the reason is shown on the screen
  (it started on foot without a word).

### World
- Parked cars have their paint (the paint scheme's pictures were looked for in the wrong
  folder and the cars stood white) and lean with an inclined street.
- A street running through an object's `[boundingbox]` (a bridge, a gantry, a hall) makes
  that box no wall: mod maps' invisible walls across the road.
- The automatic rear door closes: the passengers' request button was never let go, so the
  stop request stayed on. A passenger who cannot get in stops asking after a while.
- Passengers turn their heads about the middle of the neck (aXYZ man02's neck point lies at
  the back of his neck: the head swung off the collar).

### Crashes
- "RenderBundleEncoder::finish: Validation Error" after the card ran out of memory no longer
  ends the game: the part of the picture is left out.

## 0.1.10 - 2026-09-28

### Performance and crashes
- Big mod maps (Grande Porto, Novi Sad) no longer freeze for up to two seconds while
  driving: the night copies of object textures (`night\` folder) were decoded on the
  thread that draws. They are now read with the rest of a tile in the background; the
  slowest object upload on Novi Sad went from 1760 ms to 24 ms.
- Fixed the crash "Error in Buffer::get_mapped_range: Validation Error" (Windows): the
  vertex updates of a frame no longer go through one staging buffer that could outgrow the
  graphics card's buffer limit.

### Driving and physics
- Steering no longer eats the engine's power: both front wheels turned by the same angle
  and the tyres fought each other, so at 60 % steering a bus barely moved and at full lock
  not at all. The inner wheel now turns further than the outer one (Ackermann) - buses
  and cars take tight corners at the speed you give them.
- Modded maps: the bus no longer hops over invisible things. Only solid objects (`[fixed]`,
  not `[nocollision]`) give the wheels a step to climb; the low collision meshes of helper
  and sensor objects did too.
- Trains stay on their track: a track under a bridge counted as a "neighbouring lane", and
  a train changed lanes down through the viaduct.
- The automatic rear door (MAN SD200, NL202/EN92) closes after the passengers are out:
  one passenger held up on the way out kept the stop request on for good.

### Vehicles and mods
- Add-ons that name their meshes from another folder than their model (Studio Polygon's
  `Configuration Files`, packs that borrow from the vehicle folder or the game folder) find
  them.
- The side mirrors show the bus's own flanks, as in OMSI (the outside-only meshes are drawn
  in the mirrors from the cab).
- Esc → *Destination display...*: choose any destination of the bus's depot file by hand
  (roller blinds, matrix displays, custom blinds).

### Controllers (macOS)
- Wheels and pedals are read from their HID elements: two axes of the same kind stay two
  (HORI Truck Control System: the brake pedal moved the accelerator), a 16-bit wheel uses
  its whole range (no dead zone of a quarter turn), and the simulation page's steering,
  clutch, accelerator and brake are read on wheels that use it.
- Windows: the hat switches (D-pads of wheel rims, Moza among them) can be given keys like
  buttons (*Hat 1 up* ... on the Controllers page).

### Settings
- *Throttle pedal strength* / *Brake pedal strength*: a softer or stronger response of the
  analog pedals (launcher, and Esc → Options).
- *Seat position*: the driver's eye forward/back, up/down, left/right, with *Reset the seat
  position* (launcher, and Esc → Options).
- *Camera collisions*: the outside camera no longer jumps in when something passes behind
  it (it is pulled in over a tenth of a second); switched off, it goes through everything,
  as in OMSI.
- The field of view applies to the free camera and the view on foot too.

### More from the players
- A bus put down inside an obstacle (a shelter's or a depot's collision box - GPM) is no
  longer held there: what it spawned in is left alone until it has driven out of it.
- Road markings are paint, not steps: a road face within 4.5 cm over another one (markings
  made as `[surface]` objects or as splines with a height profile - Horizon) no longer lifts
  the wheels, so the bus stops hopping over lines at stops and roundabouts. Kerbs stay kerbs.
- With a steering wheel the arrow keys look around again, as in OMSI (a G29's buttons set to
  the arrow keys turned the view there; here they steered).
- Mirrors can be turned: Ctrl+Alt+arrows in the cab turn the mirror you look at, kept per bus
  in `~/.openomsi/mirrors.cfg`.

### Passengers
- The aXYZ man in the grey jacket no longer looks as if his neck were broken: a head turns
  about a point under its middle, not about the `[links]` neck point at the back of the
  neck, which swung the head off the collar whenever he looked to the side.

### Head tracking and time
- Head tracking: Settings → *Head tracking* takes the head's pose from opentrack's
  "UDP over network" output (port 4242) - TrackIR, Tobii, webcams and phones through
  opentrack. `head_tracking_invert=yaw,pitch,roll` in `settings.cfg` turns an axis round.
- The clock can be changed gradually: hold Ctrl+Shift+Page Up / Page Down (faster the longer
  it is held), or Esc → More → *Clock +10 minutes* / *-10 minutes*.

### Interface
- P pauses into the pause menu (P or *Resume* go on); it was only a line of text before.
- The pause menu shows the everyday lines first (*Resume, Options, Line and tour,
  Destination display, City map, Timetable, Save, ...*); the rest is under *More...*.
  The mouse wheel scrolls the menu instead of moving the highlight, and only the line under
  the mouse is lit.
- The timetable shows departure times (a stop with a wait shows both), the trip number of
  the tour and the next trip.
- Android: dropdowns no longer close (or pick something) the moment they open.

### Lua plugins
- `omsi.info()` (map, clock, view, speed, delay, line, tour, trip, next stop, ...),
  `omsi.command(name)` (refuel, wash, repair, screenshot, save, weather, clock ...),
  `omsi.vars()`, `omsi.clock()`, `omsi.speed()`, `omsi.distance(x, y)`, and the events
  `key`, `next_stop`, `view` and `duty`. See [Plugins](docs/PLUGINS.md).

### Website
- A link to the Discord server on the website and in the README.
- The website shows the releases (with their notes and downloads) and the issues (open and
  closed, searchable, with their discussion) on pages of its own.

### AI traffic
- A car already in a crossing on its green no longer stops again at a light of a path it
  joins inside that crossing (the cross traffic's red, an invisible stop line mid-turn).
- Depot buses wear the repaint of their fleet number when the bus's `[number]` lists name
  one (a `.org` file per repaint); they were painted at random.
- The AI vehicles are heard round the camera: a free camera following an AI bus lost its
  sound 250 m from the player's bus.

## 0.1.9 - 2026-09-28

### Graphics cards and crashes
- A graphics validation error no longer ends the game: it is written to the log and the game
  goes on.
- "The graphics device was lost" (RTX 4060 reports, a few seconds into big maps) is the
  Vulkan driver giving up. On Windows the game can now draw with DirectX 12 instead:
  Settings → *Graphics API*, or *Use DirectX 12* in the launcher after such a crash.
- Graphics cards without Vulkan (GeForce GT 530 and other older ones) run the game: it asks
  Vulkan, then DirectX 12 (Windows), then OpenGL, and takes the first that draws. Settings →
  *Graphics API* chooses one; the Windows download brings DirectX 12's shader compiler.
- Phones where every mesh came out flat and far away (the bus in the launcher too): the
  shaders no longer read the objects' matrices in the way some phone GPUs get wrong.
- Cards below the usual limits get a smaller shadow map, and textures larger than the card
  takes are scaled down instead of stopping the game.
- When a game ends on an error, the launcher says so, with *Copy report*, *Report on
  GitHub* and, after a lost graphics device on Windows, *Use DirectX 12*.

### Collisions
- Only the objects OMSI makes solid stop the bus (`[fixed]` ones and poles). Signs on stop
  poles, gantries and the bridges of mod maps not marked so were invisible walls - under the
  bridges of Saint Servant, for example.
- Collisions with objects can be switched off, as in OMSI: Settings → *Collisions with
  objects*, or Esc → Options in the game (OMSI's own `no_collision` setting is taken over).

### Steering, pedals and controllers
- Mouse steering works in the outside and passenger views too, and the wheel follows the
  cursor smoothly: it crept on by itself and came back in steps.
- New settings, off by default: *Steering linearity* (the steering keys turn the wheel at
  OMSI's steady pace) and *Old Steering* (the wheel stays where you leave it, as in OMSI 2 -
  turn it back yourself).
- The clutch key works as in OMSI: the pedal goes down at once and comes up slowly.
- Settings → *Wheel rotation* and *Full lock at*: a wheel of 900° can steer like a real bus
  (the full lock at, say, 540° of the wheel), *Reset wheel settings* goes back to OMSI's
  (the whole wheel is the full lock). *Invert force feedback* for wheels that push the wrong
  way (G29).
- Settings → *Field of view* for the views from the bus (Default: the bus's own cameras);
  the mouse wheel, = and - still zoom as in OMSI.
- A steering wheel listed twice (Logitech G29: once as a wheel, once as a gamepad) is listed
  once, and *Use this device* on the Controllers page switches any device off.

### Multiplayer
- A joining game plays on the host's map when it is installed, whichever map was chosen
  before joining. On big add-on maps it often stayed on its own map - where nobody met it:
  - the host lists its mods for the joining players after it starts, which takes a while on
    a big map, and the joining game gave up waiting for that list after 25 s;
  - a host busy loading a heavy area answered later than the 3 s the joining game waited;
  - a dedicated server answered nobody until it had loaded its whole map.
- The host answers joining players while it loads its world, and a joining game stays in
  the session while its own map loads.
- A joining game with everything the host uses installed no longer needs 3 GB of free disk.
- Mods installed into openOMSI's content folder inside the OMSI 2 folder are passed on to
  joining players (they were taken for OMSI's own files).
- A player's info (bus, destination, display texts) always fits one datagram: long paths and
  texts made it too big to arrive, and the others never saw which bus the player drove.

### Maps and vehicles
- Parked cars, people and objects whose `Texture` or `model` folder is spelt with another
  case are no longer white or missing on Linux (TH_Zafira and others).
- The warning lamps' glass of the Thüringer Wald buses (S 315 UL, S 317 UL, O 550) and the
  MB O 407 is see-through again instead of a row of white tiles ("glas" is glass too).
- Free roam (no duty): people only board a bus that shows a destination, not one showing
  nothing or "not in service".
- Drive → *Depot file*: the depot file (HOF) can be chosen by hand; *Automatic* follows the
  map and the date.

### Sound
- A limiter on the whole mix: many loud sounds at once no longer clip (heard as squeaks and
  crackles).

### Launcher
- Icons missing on some phones: the launcher's picture atlas grows when a screen needs more
  room than it had (high-resolution phones).
- The README has an installation guide and what to do when something goes wrong.

## 0.1.8 - 2026-09-28

### Controls
- Changing a key on the Controls page now takes effect: the page says which driving keys are
  in use, and changing any key switches *Driving keys* to *Custom controls* by itself (before,
  with W A S D the edited keys were silently ignored).
- Keys you bind yourself now beat the ready-made layouts: with W A S D chosen, a D you gave
  to the gearbox is the gearbox, not "steer right". The layouts' extra keys (Z/X/C for the
  indicators, I for the saloon lights) no longer apply with Custom controls (C is OMSI's
  "look ahead" there again).
- **Space** looks ahead again (OMSI's `view_reset_all_directions`); it was swallowed by the
  W A S D layout. Every view keeps its own direction: turning the outside camera (F3) no
  longer turns the driver's head (F1).
- Zoom inside the bus: the mouse wheel, **=** / **-** and a pinch narrow the view in the
  driver's and passenger views, as in OMSI.
- Mouse steering follows Omsi.exe exactly, now including its pedals (throttle from the middle
  of the window to the top edge, brake to the bottom edge, no dead zone). Settings → *Mouse
  steering* makes it more or less sensitive (100 % = OMSI).

### Wheels, pedals, joysticks
- On Windows the game controllers are read through DirectInput, as OMSI reads them: every
  device Windows lists as a game controller (wheels with their makers' drivers included), up
  to 128 buttons, and force feedback on wheels - the centring that grows with the speed, the
  heavy steering of a bus standing still, and the scripts' shaking.
- Buttons are numbered as DirectInput numbers them (they were counted in the order they were
  first pressed), and every button of a device can be given a key: the list stopped at the
  ten that fitted on the page (T16000M).
- *Set up step by step*: turn the wheel to the left, press each pedal - the axes, their
  direction and combined pedals are found by themselves. A connected device nobody has set up
  yet steers with its X axis and says where to set it up.

### Multiplayer
- Hosting no longer stops working after a while: the router's port forwarding was asked for
  two hours and never renewed, and the rendezvous relay was asked every second and refused the
  host after an hour or two. The forwarding is renewed every 20 minutes, the relay is asked
  every few seconds with a growing pause after a refusal, and a Cloudflare tunnel that ends is
  started again.

### Launcher
- Phones: the launcher is drawn at least at the system's text size, the settings stand in one
  column, a finger on a list at its end scrolls the page on, page titles no longer run under
  the tabs.
- The OMSI 2 folder is found when openOMSI was unpacked into it (openOMSI keeps its own content
  in an `openOMSI` folder there), when the path is pasted with quotes, or when `Omsi.exe` or a
  folder inside the game is chosen; a folder that is not a complete OMSI 2 is reported with
  what it lacks.
- Timetable: changes stay while you move between lines and are saved together (*Save all*);
  *New line*; *Repeat* makes a whole day of tours (every *n* minutes up to a last departure).
- Settings → Graphics → *Reflection maps* switches the materials' reflections
  (`[matl_envmap]`) off.

### Sound
- Distance as in OMSI: full volume up to the `[3d]` reference distance, then falling as 1/d
  (DirectSound's law). It fell much faster, so most sounds were far too quiet.
- The bus's own sounds are no longer muffled in the cab unless they are other vehicles':
  interior sounds such as the indicator relay were cut to a quarter and dulled, and only came
  through with a door or window open.
- Footsteps outside are heard through the bodywork from the cab (and the saloon's from the
  street); people in the street sounded as if they walked inside the bus.

### Maps and vehicles
- Matrix displays drawn by scripts (script textures as the LED mask, `\S:n`, e.g. churaPixel/
  Krüger++ matrices) show their dots instead of a fully lit panel: a `[matl_change]` ahead of
  the slot's `[matl]` made it opaque.
- Objects put on a road spline (`[splineAttachement]`), such as an entry point or a stop, are
  found by their id: a Novi Sad start point was "not in the map".
- An entry point whose object comes out on another level than the map recorded (under a
  bridge) starts the bus at the recorded height.

### Game
- The depot file (HOF) follows the date as the map's chrono says: Berlin in 1994 has line 137
  where 1986 had 92 - on every map with chrono depot changes.
- Phone: the pause menu scrolls with the finger; a finger put down to scroll no longer picks
  the line under it.
- With V-sync one frame waits for the screen instead of two: less input delay.

### Builds
- New downloads: Windows ARM64, macOS Intel, Linux ARM64, and the dedicated server for Windows
  (x64, ARM64) and Linux ARM64. The launcher updates itself on all of them.
- The release notes on GitHub list what changed (this changelog) instead of a link to the
  code changes.

## 0.1.7 - 2026-09-28

### Driving physics as the .bus file makes it
- The bus now follows its steering the way OMSI's own physics does: it turns exactly as far
  as its wheels point and only slides when a bend asks more grip than the road has. Before,
  every bus turned at 70 % of what its steering asked, 0.8 s late and drifting sideways -
  the "boat" feeling, and the same for every bus.
- Springs, dampers and their limits act where OMSI takes them (`achse_feder`,
  `achse_daempfer`, `achse_maxforce`, `achse_minwidth`/`achse_maxwidth`), every axle steers
  towards `[rot_pnt_long]`, and body pitch and roll are damped as in OMSI. Each bus feels as
  its author made it: stiffer springs, stronger dampers, a higher centre of gravity all show.
- `cargo run --release -p omsi-sim --example handling -- <file.bus>` prints how a bus
  handles (yaw response, side slip, body roll and how fast it settles).

### Steering
- Mouse steering (O) as in OMSI: the whole window width is the full lock, and above 10 km/h
  the same hand movement turns the wheel less and less (at 50 km/h a fifth as far). No more
  jumps when the cursor passes the middle.
- Phone: the on-screen wheel turns with the finger round it (a third of a turn is the full
  lock). It used to stop at about half a turn and spring back.

### Graphics and world
- Mirrors follow the bus's pitch and roll, use the camera distance from the `.bus` file, and
  only the mirrors in view are redrawn (the radius of `[add_camera_reflexion_2]`).
- The bus's own screens (IBIS, matrix displays, dashboard LCDs) are sharp again in Enhanced
  graphics: FXAA and the glow no longer blur them.
- Trees have the width the map gives them: slim trees such as firs were drawn up to six
  times too wide.
- Map tiles of older editor versions are read correctly (object tilt and strings).

### Updates
- The launcher updates itself from the GitHub releases: when a newer version is out it asks
  at the start, downloads it (checked against GitHub's SHA-256), replaces the program and
  starts again. Mods, content and settings stay as they are.
- On Android the system's installer is used: Update replaces the app and starts it again,
  Cancel leaves it as it was.
- Settings → Updates: look for updates at the start, install without asking, Check now.

### Fixes
- Android: vibration of the on-screen buttons works (the calls never reached the app).
- Android: `openOMSI/env.txt` takes the `OMSI_*` switches a computer takes from its
  environment (for looking into problems).

## 0.1.6 - 2026-09-27
- Phone: the on-screen wheel is drawn cleanly; calmer steering, tilt steering reaches the
  full lock at 45°.
- A version built again updates its release instead of failing.

## 0.1.5 - 2026-09-27
- Lua plugins (`plugins/<name>.lua` or `plugins/<name>/main.lua`) on every platform, next to
  the original DLL plugins: bus variables and triggers, events, timers, saved data, hot
  reload, sandboxed. See [docs/PLUGINS.md](docs/PLUGINS.md).

## 0.1.4 - 2026-09-27
- Android: the launcher and the game on phones and tablets, with on-screen driving controls
  (wheel or tilt, pedals, gearbox, doors, indicators, cab panel, cameras). See
  [docs/ANDROID.md](docs/ANDROID.md).

## 0.1.3 - 2026-09-27
- Small changes.

## 0.1.2 - 2026-09-27
- Website and repository improvements and fixes.

## 0.1.1 - 2026-09-27
- Website and repository improvements and fixes.

## 0.1.0 - 2026-09-27
- First public release as openOMSI: a from-scratch recreation of OMSI 2 in Rust that runs
  every map and mod (an original OMSI 2 is needed). Builds for Windows, macOS and Linux and
  a dedicated server, released automatically on every push.
