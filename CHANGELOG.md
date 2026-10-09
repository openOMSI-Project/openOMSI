# Changelog

Every push to `main` is released as `MAJOR.MINOR.COMMIT` (see
[docs/VERSIONING.md](docs/VERSIONING.md)); the downloads are on the
[Releases](https://github.com/openOMSI-Project/openOMSI/releases) page.

## Unreleased

### New
- **Public dedicated server browser**: Multiplayer → Servers → Favorites | Public. Dedicated
  servers with `public = auto` (default) or `1` post to the lobby when their tunnel is ready;
  `public = 0` stays private. The Public tab lists online servers (verified with `/status`)
  and can add them to Favorites.
- **`dispatch` in `server.cfg`**: turns the local web dispatch console on or off
  (`http://127.0.0.1:<web_port>/dispatch`; needs `admin_password`).

## 0.2.20 - 2026-10-08

### New
- **Gamepads**: ready-made Xbox, PS4 and PS5 profiles with default buttons, a Gamepad settings page (stick dead zone, steering speed), and the Menu/Options button opens the main menu [#1939](https://github.com/openOMSI-Project/openOMSI/pull/1939); a controller action to open and close the main menu [#1594](https://github.com/openOMSI-Project/openOMSI/pull/1594).
- **Key bindings**: "Add binding" on the Controls page lists every action - the installed buses' own script triggers too, with how many buses use each - searchable [#1437](https://github.com/openOMSI-Project/openOMSI/pull/1437).
- **Platform screen doors**: light paths marked as a turn work as detectors for that indicator, for every vehicle, so BRT platform doors open for the bus that stops there [#1889](https://github.com/openOMSI-Project/openOMSI/pull/1889).
- **HTML LED destination signs** glow at night like the LED panels, with the same LED glow setting [#1905](https://github.com/openOMSI-Project/openOMSI/pull/1905).
- Lua plugins: a `service` event (refuel, wash, repair, reset, teleport) [#1882](https://github.com/openOMSI-Project/openOMSI/pull/1882), and `trip_done` with the trip's ratings, `jolt` and `ticket_sold` events [#1937](https://github.com/openOMSI-Project/openOMSI/pull/1937).
- `telemetry.json` for outside tools (the player's bus, the duty and its stops, the AI buses): written only while the file exists, see `docs/PLUGINS.md` [#1909](https://github.com/openOMSI-Project/openOMSI/pull/1909).
- Translations: Dutch completed [#1944](https://github.com/openOMSI-Project/openOMSI/pull/1944), German reviewed [#1936](https://github.com/openOMSI-Project/openOMSI/pull/1936), Chinese [#1587](https://github.com/openOMSI-Project/openOMSI/pull/1587), French [#1713](https://github.com/openOMSI-Project/openOMSI/pull/1713), the game menu in every language [#1540](https://github.com/openOMSI-Project/openOMSI/pull/1540), the stock weather names [#1598](https://github.com/openOMSI-Project/openOMSI/pull/1598); messages with a number or a name in them are translated too [#1724](https://github.com/openOMSI-Project/openOMSI/pull/1724).

### Fixes
- Dedicated server and LAN host: AI cars no longer appear or vanish in plain view of the other players [#1893](https://github.com/openOMSI-Project/openOMSI/pull/1893).
- Enhanced at night: no more black disc under street lamps whose light sits inside the lamp (the Ufo lamps of Spandau), and the lamp shadows got faster [#1915](https://github.com/openOMSI-Project/openOMSI/pull/1915).
- Faster in scenes with excavations (U-Bahn entrances): the MSAA depth prepass is kept, about 1.5 ms less per frame there [#1614](https://github.com/openOMSI-Project/openOMSI/pull/1614).
- A Shift, Ctrl or Alt whose release was missed (after a screenshot shortcut) no longer sticks [#1901](https://github.com/openOMSI-Project/openOMSI/pull/1901).
- Vehicle descriptions are shown in full, and the phone launcher's vehicle sheet scrolls [#1932](https://github.com/openOMSI-Project/openOMSI/pull/1932).
- The German tutorial pages show their special characters [#1552](https://github.com/openOMSI-Project/openOMSI/pull/1552).
- Documentation fixes [#1917](https://github.com/openOMSI-Project/openOMSI/pull/1917), [#1918](https://github.com/openOMSI-Project/openOMSI/pull/1918), [#1919](https://github.com/openOMSI-Project/openOMSI/pull/1919); a deprecated call replaced [#1947](https://github.com/openOMSI-Project/openOMSI/pull/1947).

## 0.2.19 - 2026-10-08

### Fixes
- Mouse steering shows its "+" again: since 0.2.17 the cursor is held while the mouse steers (so the wheel reaches its full lock past the screen's edge), and the crosshair went with it. The cross is now drawn at the point that steers and takes your clicks.

## 0.2.18 - 2026-10-08

### New
- **Quality presets for each graphics mode**, the same in the launcher and the in-game options; the mode is chosen first and a preset never switches it. Your current settings stay as they are [#1873](https://github.com/openOMSI-Project/openOMSI/pull/1873).
- **Cloud quality** (Settings → Graphics): "Low" draws the same physical clouds with fewer steps, about 0.8 ms faster on an M4; the cloud noise is kept on disk so later starts make it no more [#1872](https://github.com/openOMSI-Project/openOMSI/pull/1872).
- **"DXT/BC textures stay compressed on the GPU"** can be switched off for drivers that draw compressed textures wrongly (the textures then take about twice the video memory) [#1873](https://github.com/openOMSI-Project/openOMSI/pull/1873).
- **Searchable vehicle lists**, and placing, swapping or reloading a vehicle loads it in the background instead of freezing the game (1.4 s less on an articulated bus) [#1867](https://github.com/openOMSI-Project/openOMSI/pull/1867).
- **Support package** (Setup → Export diagnostics): a ZIP for a bug report with the versions, the graphics card and driver, the graphics settings, the controllers and the last map and bus - no folders, names, chat, LAN codes or addresses. It stays on your computer and its folder opens, so you can look inside before attaching it [#1870](https://github.com/openOMSI-Project/openOMSI/pull/1870).
- Mods: `[matl_glow] <texture> <value>` lets a script make a material glare (an openOMSI extension; stock content is unchanged) [#1853](https://github.com/openOMSI-Project/openOMSI/pull/1853).
- Performance captures: an `OMSI_PROFILE` run now logs the frame-time percentiles, `OMSI_PROFILE_JSON=<file>` saves them, and `scripts/compare-performance.py` compares two runs [#1869](https://github.com/openOMSI-Project/openOMSI/pull/1869).

### Fixes
- A duty started from the game menu starts at the current time, not at the first trip of the day [#1864](https://github.com/openOMSI-Project/openOMSI/pull/1864).
- Articulated buses: every exit follows its own door, passengers on foot find the nearest section's doors [#1863](https://github.com/openOMSI-Project/openOMSI/pull/1863).
- LAN: a weather change by the host reaches the other players at once [#1865](https://github.com/openOMSI-Project/openOMSI/pull/1865).
- Launcher: a key binding keeps the Shift, Ctrl or Alt it was pressed with [#1866](https://github.com/openOMSI-Project/openOMSI/pull/1866).
- HafenCity: the harbour backdrop no longer stands across the road at Landungsbrücken [#1861](https://github.com/openOMSI-Project/openOMSI/pull/1861).
- Enhanced at night: lamp shadows are no longer looked up for surfaces facing away from the lamp (same picture, a little faster) [#1875](https://github.com/openOMSI-Project/openOMSI/pull/1875).
- The launcher opens faster: its bus preview no longer prepares resources it doesn't use [#1871](https://github.com/openOMSI-Project/openOMSI/pull/1871).
- Build: no compiler warnings left, and two examples no longer share an output name [#1862](https://github.com/openOMSI-Project/openOMSI/pull/1862); more content tests [#1868](https://github.com/openOMSI-Project/openOMSI/pull/1868).

## 0.2.17 - 2026-10-08

A big one: early, mid and late seasons with trees that turn one by one, mouse steering to full
lock, the wipers really clearing the glass, collisions with real momentum and dents, encrypted
LAN mods, a DirectX 11 option for old graphics cards, and the game's code reorganised from the
ground up so that it stays easy to change.

### New
- **Early, mid and late seasons.** Pick a season and under it Early, Mid or Late. The phase
  sets a real date in that part of the season (the map's hemisphere decides which months), so
  the sun's path, the length of the day, the natural weather's temperatures and snow, the
  street lamps, the map's dated changes and the timetable all follow it by themselves. "By
  date" works as before. Also on the command line: `--season autumn-late`.
- **Trees that turn one by one.** In between two seasons the trees no longer change all
  together: each tree takes one of the two looks, the same every time and for every LAN
  player - early autumn is green with the first yellow trees, late autumn mostly bare with a
  few still in leaf, early spring bare with the first green, late summer partly dry.
- **Wipers clear the rain and snow only where the blade has passed**, along its sweep, pushing
  the water ahead of it; drops merge and run off, and in snow the flakes settle on the glass
  and build up at the edge of the wiped area [#1828](https://github.com/openOMSI-Project/openOMSI/pull/1828).
- **Collisions with AI vehicles**: a car that is hit recoils by momentum and finds its way back
  onto its lane, crash energy follows the masses and speeds of both vehicles, both the bus and
  the AI cars get dents where they were hit, and glass shows cracks, lit like the pane it is in
  [#1819](https://github.com/openOMSI-Project/openOMSI/pull/1819). Mouse pedal strength is a new
  setting [#1819](https://github.com/openOMSI-Project/openOMSI/pull/1819).
- **"ANGLE (DirectX 11)" graphics API on Windows**, for graphics chips whose DirectX 12, Vulkan
  and OpenGL drivers don't run the game (Intel HD Graphics 2000-4000, Radeon HD 5000/6000). It
  is tried by itself when the others fail, or chosen in Settings. The DLLs are built from
  Google's ANGLE source in [angle-openomsi](https://github.com/openOMSI-Project/angle-openomsi)
  and checked by their SHA-256.
- **LAN mods are encrypted** on the network and on disk: the files a host sends can't be read
  off the network, and copying them out of the game's folders gives you scrambled data, while
  the game reads them normally. (This stops casual copying, not a determined person: the game
  is open source and decrypts on the player's computer.) Hosts and players need 0.2.17 both.
- The information bar shows how many passengers fit next to how many are aboard
  [#1826](https://github.com/openOMSI-Project/openOMSI/pull/1826).
- Plugins: `omsi.info()` names the stops on either side of the bus with their IDs, the
  distance to each, and whether the bus stands at a stop
  [#1588](https://github.com/openOMSI-Project/openOMSI/pull/1588).
- New option (Settings → Gameplay → Traffic, off by default): timetable buses running ahead
  of time wait for their departure only at the stops the timetable times itself, instead of
  at every stop as in OMSI [#1773](https://github.com/openOMSI-Project/openOMSI/pull/1773).

### Fixes
- Mouse steering reaches full lock both ways: the wheel used to stop as soon as the cursor
  hit the edge of the screen (on macOS often at a quarter of the lock). The cursor is now
  held while you steer and comes back when you look around, open a menu or switch windows;
  the mouse pedals reach full throttle and full brake too.
- Dedicated server: people walk the pavements and wait at the stops around every player
  again, and board the timetable buses there
  [#1817](https://github.com/openOMSI-Project/openOMSI/pull/1817).
- The offscreen pictures (and so the game's own picture checks) are taken exactly as the
  window draws the game: the clock goes on, the AI's lights follow the daylight, the
  collision settings count, the wet roads and the cabin air develop, the lighting is built
  the same way.
- No more crashes when the graphics driver fails in some ways it used to take the game down
  with: running out of video memory while recording, a driver that can't compile a shader,
  adapter checks that fail, closing an OpenGL context. Shader compile failures now say what
  the driver said.
- OpenGL: no more "Could not lock adapter context" crashes while another part of the game
  waits for the GPU (#843, #898, #1110).
- The LAN tests no longer fail now and then.

### Under the hood
- The game's code was reorganised without changing what it does (checked frame by frame
  against the previous version): the frame is a sequence of named steps shared by the window
  and the offscreen run; the AI traffic, the people and the timetable simulate in `omsi-sim`
  with no graphics in them, and one place hands their state to the renderer; the renderer's
  passes and pipelines each have a module of their own; the game's state is grouped by
  subsystem; every `OMSI_*` switch is listed in `docs/DEBUG_FLAGS.md` and read through one
  registry. No function is longer than 500 lines any more (there were ten), and a check on
  every pull request keeps files and functions from growing past their size again.
- openOMSI now builds on [wgpu-openomsi](https://github.com/openOMSI-Project/wgpu-openomsi),
  the project's fork of wgpu 29.0.4 with the graphics fixes above (and a video-memory query,
  which the log now shows).
- `docs/ARCHITECTURE.md` describes the architecture; its history moved to `docs/HISTORY.md`.

## 0.2.16 - 2026-10-07

### New
- Lua plugins can show panels (texts, icons, progress bars, buttons) and sliding notifications of their own in the game's look (`omsi.ui`); F10 gives the mouse to the panels, Esc back to the bus. See `docs/PLUGINS.md` and the `trip_panel.lua` example [#1788](https://github.com/openOMSI-Project/openOMSI/pull/1788).
- Native TrackIR head tracking on Windows (NaturalPoint's NPClient), with sensitivity and inversion per axis in the launcher; opentrack keeps working as before [#1763](https://github.com/openOMSI-Project/openOMSI/pull/1763).
- Night brightness (Settings → Graphics): brightens the Enhanced picture after sunset only, Off by default [#1799](https://github.com/openOMSI-Project/openOMSI/pull/1799).
- Plugins: `trip_done` in `omsi.info()` [#1810](https://github.com/openOMSI-Project/openOMSI/pull/1810).

### Fixes
- Refuelling and the bus wash take their time, litre by litre, and stop when the bus drives off [#1785](https://github.com/openOMSI-Project/openOMSI/issues/1785).
- Pedestrians run over are counted once each, within the bus's real box [#1805](https://github.com/openOMSI-Project/openOMSI/issues/1805).
- Enhanced+: the buildings of a depot or other surface object cast their shadow up close too [#1783](https://github.com/openOMSI-Project/openOMSI/issues/1783).
- Smoke, exhaust and other particles drift with the wind [#1798](https://github.com/openOMSI-Project/openOMSI/issues/1798).
- The automated manual gearbox can be switched on in the launcher's driving settings [#1780](https://github.com/openOMSI-Project/openOMSI/issues/1780).
- A traffic light no longer reacts to a bus on the neighbouring road or bay: the depot-gate request applies only off the lanes [#1790](https://github.com/openOMSI-Project/openOMSI/issues/1790).
- A duty started at the real time keeps the clock instead of jumping to ten minutes before its first trip [#1792](https://github.com/openOMSI-Project/openOMSI/pull/1792).
- Puddles are smaller, with sharp irregular edges [#1804](https://github.com/openOMSI-Project/openOMSI/pull/1804).
- Dedicated server: AI traffic and pedestrians move smoothly on the players' screens and at their real speed; the server's clock keeps to real time [#1807](https://github.com/openOMSI-Project/openOMSI/pull/1807).
- Hong Kong maps: AI buses of a plain `[aigroup_2]` group show their destination again [#1757](https://github.com/openOMSI-Project/openOMSI/pull/1757), and bus stop signs show their route numbers and pictures [#1765](https://github.com/openOMSI-Project/openOMSI/pull/1765).
- AI vehicles wait before a junction when a queue on the short road pieces beyond it leaves no room, instead of blocking it [#1761](https://github.com/openOMSI-Project/openOMSI/pull/1761).

## 0.2.15 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.14 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.13 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.12 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.11 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.10 - 2026-10-07

### Bug Fixes & Improvements

## 0.2.9 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.8 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.7 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.6 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.5 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.4 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.3 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.2 - 2026-10-06

### Bug Fixes & Improvements

## 0.2.1 - 2026-10-06

### Bug Fixes & Improvements
- Android: a black screen instead of the game since 0.2.0 (#1633, #1597) on phones whose graphics driver cannot compile one of 0.2's new shaders (the snowfall, the lamps in the fog, the street lamps' shadow maps): the driver's error lost the whole graphics device. The game now notices this while it starts, opens the device again without those three and draws as before (snow then falls without flakes). The log names the shaders it was compiling when the device was lost.

## 0.2.0 - 2026-10-06

A release about light and weather: the Enhanced and Enhanced+ pictures worked out from the physics of the eye, the air and the lamps, and a new snowfall for every graphics mode.

### Night (Enhanced, Enhanced+)
- The night is dark again: the eye's adaptation follows a lightness-perception model (Krawczyk et al. 2005) instead of nearly full adaptation, the automatic metering no longer brightens a night, and the eye adapts to the lamps and headlights actually in view (their log-average) rather than to a fixed city level.
- Street lamps cast real shadows: shadow maps for the four lamps lighting the camera's surroundings most (the bus, poles, signs and trees throw their shadows on the street).
- A street lamp's light goes down and out, not into the sky: tree crowns and upper floors above the lamps stay dark.
- The lit ground throws light back up: a bus's flank or a facade beside a lit street is no longer black; much more so over snow.
- Moonlit nights: the moon is a directional light with its own shadows; on a dark country road under a full moon the eye takes to the moonlight.
- The night sky's glow comes from the lamps round the camera (Walker's law): a village keeps its stars, a city glows orange-grey.

### Weather (Enhanced, Enhanced+)
- Fog and mist light up round every lamp and in front of every headlight (single scattering by the droplets, a forward peak and back-scatter): halos, cones under the lamps, the glow of a bus's own beams in the fog. Rain does not do this (its drops scatter too narrowly), so headlights keep a single glow in a drizzle.
- Shafts of sunlight between the shadows of houses and trees in haze, mist and fog.
- Falling snow takes the view as a real snowfall does (heavy snow: some 400 m).
- Wet porous surfaces (soil, paving, plaster, bark) turn darker and deeper in colour in the rain, not only the asphalt.
- An overcast deck lets through light by its thickness: a raining nimbostratus is darker, a grey day over snow brighter.
- Raindrops and snowflakes are lit by what is round them - the sky, the sun, the lamps - instead of glowing at one level in the dark.

### Light and the eye (Enhanced, Enhanced+)
- Looking at the sun blinds: the eye's scattered light round it (CIE 146 glare function) with a faint ciliary corona and lenticular halo.
- Eye adaptation: the picture darkens when the sun comes into view and brightens in a dark cab or an underpass, quickly towards the light and slowly towards the dark.
- Lit windows and lights have a light glow; only what is brighter than the screen's white glows.

### Snowfall (all graphics modes)
- A new snowfall: up to 150 000 flakes worked out on the graphics card (nothing per flake on the processor), fixed in the world so the bus drives through the snow, falling gently at about a metre a second, swaying and drifting with the wind; a light fall is fewer, smaller crystals. No snow falls inside the player's bus.

## 0.1.1740 - 2026-10-05

### Merged pull requests
- Headlamps: the grass and hedges beside the road are no longer washed out at night, the beam keeps to the lane, and a full beam reaches far down the road [#1563](https://github.com/openOMSI-Project/openOMSI/pull/1563); a spot declared ahead of the bus's lamps shines from both headlamps, not from the middle [#1565](https://github.com/openOMSI-Project/openOMSI/pull/1565).
- Enhanced lighting uses a material's own ambient colour: depot interiors with black diffuse and white ambient (Thüringer Wald, OVR Lichtentanne) are no longer black [#1556](https://github.com/openOMSI-Project/openOMSI/pull/1556).
- Raised floors, markings and rails stay visible above the road surfaces at a distance, without a depth bias that moved with the camera angle [#1557](https://github.com/openOMSI-Project/openOMSI/pull/1557).
- Linux: wheels such as the Logitech G29 are no longer taken for gamepads: linear steering and native force feedback again [#1566](https://github.com/openOMSI-Project/openOMSI/pull/1566).
- Performance: no more regular 30-45 ms hitches near heavy collision meshes (Grand Paris-Moulon's street lamps) [#1561](https://github.com/openOMSI-Project/openOMSI/pull/1561).

## 0.1.1727 - 2026-10-05

### Fixes
- The Linux build of the release no longer runs out of disk space after the workspace tests (the release of 0.1.1726 did not build).

## 0.1.1726 - 2026-10-05

### Merged pull requests
- Controllers: a gilrs panic on a controller's first event no longer ends the game on Windows [#1537](https://github.com/openOMSI-Project/openOMSI/pull/1537); an idle joystick nobody set up no longer takes the arrow keys for looking [#1548](https://github.com/openOMSI-Project/openOMSI/pull/1548).
- Czech and Slovak content is read as Windows-1250, so stop names keep their ř, ě and ů [#1542](https://github.com/openOMSI-Project/openOMSI/pull/1542).
- A `.owt` weather's second `[temp]` value is read as the dew point: winter weathers give the scripts a real humidity (exhaust steam in the frost, the heating's misted panes) [#1533](https://github.com/openOMSI-Project/openOMSI/pull/1533).
- Destinations of legacy depot files show their names in the menu and on the displays, and a destination picked by hand runs the scripts' own trigger, as in OMSI 2 [#1535](https://github.com/openOMSI-Project/openOMSI/pull/1535).
- Vulkan: running out of graphics memory on a demanding map frees the old render targets before falling back, and far textures are reduced right after streaming [#1551](https://github.com/openOMSI-Project/openOMSI/pull/1551).

## 0.1.1711 - 2026-10-05

### Fixes
- The launcher's download of a server's map and buses before joining (#1486, with #1511) is taken out again for now.
- The switches for sharing and downloading mods in multiplayer (#1432) are taken out again for now.

## 0.1.1707 - 2026-10-05

### Merged pull requests
- Headlamps in Enhanced and Enhanced+ light the road as real ones do: evenly from the bumper on, wide, with a low beam's cut-off, instead of one bright pool where the lamp's axis meets the road; lamps pointing down (over a door) keep their cone [#1522](https://github.com/openOMSI-Project/openOMSI/pull/1522).
- Performance: meshes share pages of vertex and index buffers, about a third less drawing work on the CPU; Enhanced+ keeps one ray tracing structure per mesh in its page [#1340](https://github.com/openOMSI-Project/openOMSI/pull/1340).
- Catalan as an interface language [#1521](https://github.com/openOMSI-Project/openOMSI/pull/1521); Portuguese (Brazil and Portugal) complete again [#1501](https://github.com/openOMSI-Project/openOMSI/pull/1501).
- Launcher: buttons that set the start to the current time and date [#1517](https://github.com/openOMSI-Project/openOMSI/pull/1517); no automatic update from a server while downloading missing mods is off [#1511](https://github.com/openOMSI-Project/openOMSI/pull/1511).
- The personnel file stores late arrivals before early departures, as OMSI 2 does: the counts no longer swap between the two games [#1499](https://github.com/openOMSI-Project/openOMSI/pull/1499).
- Dependency updates [#1510](https://github.com/openOMSI-Project/openOMSI/pull/1510) [#1509](https://github.com/openOMSI-Project/openOMSI/pull/1509) [#1508](https://github.com/openOMSI-Project/openOMSI/pull/1508) [#1507](https://github.com/openOMSI-Project/openOMSI/pull/1507) [#1506](https://github.com/openOMSI-Project/openOMSI/pull/1506); a plugin DLL that does not load says the system's reason again.

## 0.1.1674 - 2026-10-05

### Merged pull requests
- AI trams no longer leave their rails to overtake over the oncoming lane: they wait behind what blocks them [#1512](https://github.com/openOMSI-Project/openOMSI/pull/1512).

## 0.1.1671 - 2026-10-05

### Merged pull requests
- Junction plates are raised by their height field only where the field covers them: the road connection at U Ruhleben (Berlin-Spandau) is level again, without the gap and the bump, and AI lanes follow the field there too [#1493](https://github.com/openOMSI-Project/openOMSI/pull/1493).

## 0.1.1668 - 2026-10-05

### Merged pull requests
- Controls: keyboard, wheels and gamepads can be set up while playing [#1382](https://github.com/openOMSI-Project/openOMSI/pull/1382); DirectInput wheels recover after a failed read or a replug, retrying a device that will not open at growing intervals [#1489](https://github.com/openOMSI-Project/openOMSI/pull/1489); H-pattern shifters can return to neutral when a gear is let go [#1490](https://github.com/openOMSI-Project/openOMSI/pull/1490); stale gamepad events after a focus change are ignored [#1388](https://github.com/openOMSI-Project/openOMSI/pull/1388); the menu shows the default keys of the timetable (Insert) and the information bar (Shift+Y) [#1425](https://github.com/openOMSI-Project/openOMSI/pull/1425).
- VR: the head's pitch is applied [#1373](https://github.com/openOMSI-Project/openOMSI/pull/1373).
- Launcher: the roadbook keeps a picked trip when the start follows the real time [#1460](https://github.com/openOMSI-Project/openOMSI/pull/1460); the tour list filters by route or stop, starts at the trip found and hides ended tours [#1461](https://github.com/openOMSI-Project/openOMSI/pull/1461); a shared `HOFs` folder serves every vehicle, listed in the depot too [#1491](https://github.com/openOMSI-Project/openOMSI/pull/1491); the map zooms smoothly on big maps [#1139](https://github.com/openOMSI-Project/openOMSI/pull/1139).
- Multiplayer: joining players get the map's ground textures [#1485](https://github.com/openOMSI-Project/openOMSI/pull/1485); the launcher downloads a server's map and buses before joining, and only changed files update by themselves [#1486](https://github.com/openOMSI-Project/openOMSI/pull/1486); controls for mod transfers [#1432](https://github.com/openOMSI-Project/openOMSI/pull/1432); a game the server sends or turns away ends and says why, while a refused reconnect plays on [#1429](https://github.com/openOMSI-Project/openOMSI/pull/1429); traffic light cycles no longer stall on simultaneous inactive events [#1398](https://github.com/openOMSI-Project/openOMSI/pull/1398) and stay under the host's clock [#1399](https://github.com/openOMSI-Project/openOMSI/pull/1399); a host's natural weather or weather cycle reaches the clients.
- Lua plugins: crash, pedestrian and skipped-stops events, trip, stop and vehicle details in `omsi.info()` [#1447](https://github.com/openOMSI-Project/openOMSI/pull/1447); `omsi.send` sends UDP messages (up to 8 KB) to other programs on this computer [#1448](https://github.com/openOMSI-Project/openOMSI/pull/1448); `.opl` plugins read the game's values through `openomsi_*` variables [#1361](https://github.com/openOMSI-Project/openOMSI/pull/1361).
- Graphics: trees' pictures are no longer repeated and have no line along their top [#1480](https://github.com/openOMSI-Project/openOMSI/pull/1480); no invisible road walls from thin triangles [#1454](https://github.com/openOMSI-Project/openOMSI/pull/1454); the high cloud layer's noise no longer jitters [#1357](https://github.com/openOMSI-Project/openOMSI/pull/1357); transparent terrain costs less [#1426](https://github.com/openOMSI-Project/openOMSI/pull/1426); MSAA depth prepass for mixed-material views on Apple GPUs [#1465](https://github.com/openOMSI-Project/openOMSI/pull/1465); lamps light the walls next to them softer and a doubled maplight counts once [#1372](https://github.com/openOMSI-Project/openOMSI/pull/1372); OpenGL no longer deadlocks prefetching vehicles [#1416](https://github.com/openOMSI-Project/openOMSI/pull/1416).
- Performance: a new pose rescans only the instances drawing reshaped meshes [#1466](https://github.com/openOMSI-Project/openOMSI/pull/1466); draws of materials that look alike are batched together [#1456](https://github.com/openOMSI-Project/openOMSI/pull/1456).
- Translations: Traditional Chinese expanded [#1445](https://github.com/openOMSI-Project/openOMSI/pull/1445).
- Build: the whole workspace's tests run on Windows and Linux before a release [#1431](https://github.com/openOMSI-Project/openOMSI/pull/1431); shared crates declared once, an unused patch fails the audit, Dependabot waits a week [#1403](https://github.com/openOMSI-Project/openOMSI/pull/1403) and updates wgpu with naga [#1370](https://github.com/openOMSI-Project/openOMSI/pull/1370); dependency updates [#1349](https://github.com/openOMSI-Project/openOMSI/pull/1349) [#1352](https://github.com/openOMSI-Project/openOMSI/pull/1352) [#1347](https://github.com/openOMSI-Project/openOMSI/pull/1347) [#1353](https://github.com/openOMSI-Project/openOMSI/pull/1353).

## 0.1.1554 - 2026-10-05

### Fixes
- Enhanced+ ray tracing works on Windows and Linux again (RTX and RDNA 2 cards and newer, Direct3D 12 and Vulkan), not only on Apple silicon: on Direct3D 12 its shaders were refused by the shader compiler, so every frame was thrown away and the picture stood still on the loading screen. Cut-out leaves and fences also cast their full traced shadows on Vulkan and Direct3D 12 now.
- Should a graphics driver refuse the ray tracing all the same, Enhanced+ falls back to Enhanced instead of freezing.

## 0.1.1553 - 2026-10-05

### Fixes
- Enhanced+ no longer stops drawing at the end of the loading screen on Windows and Linux: its ray tracing is used on Apple silicon (Metal) only for now, and elsewhere Enhanced+ draws as Enhanced. The graphics library's ray tracing on Vulkan and Direct3D 12 is still experimental; `OMSI_RT=1` tries it.

## 0.1.1552 - 2026-10-05

### Fixes
- "Playing now" stays up all day: the game reports every ten minutes instead of every three and waits half an hour when the counter is busy, and the website asks every five minutes - the counter had run out of its daily requests and answered nobody until midnight UTC.

## 0.1.1551 - 2026-10-05

### New
- Enhanced+ graphics (Settings → Graphics, `--enhanced-plus`): hardware ray tracing on Apple M3/M4 and newer, RTX and RDNA 2 cards - ray-traced sun shadows (soft away from their caster, crisp at the contact), ambient occlusion and reflections: wet roads, puddles, water, glass and chrome mirror what really stands around them, off the screen too. Elsewhere it draws as Enhanced.
- Natural light in Enhanced and Enhanced+: the sky is computed from the physics of the atmosphere (multiple scattering, ozone, the day's haze and its particle size, a stratospheric layer), so the blue hour, the twilight's purple and every sunset look their own. Clouds glow after the sun has set for the street, a veil of high cloud gives a milky sun with soft pale shadows, passing cumulus take the sun away and bring it back, the moon stands at its real place with its real phase and lights the night, stars show in a dark sky and a city lights its own clouds. A camera's tone curve and exposure, street lamps as bright points with a little glare (wide halos in mist and rain).
- Natural weather (the default when no weather is chosen, `--weather natural`): a physical weather model instead of one fixed state. Highs and lows pass through, cumulus grows in the afternoon and dissolves in the evening, calm clear nights leave morning fog, fronts bring a grey deck and rain, snow lies in winter and thaws in a mild spell, rain leaves clear air behind - grey days, blue evenings and sunny mornings follow from the days before, as the season and the map's latitude allow.

### Fixes
- Vehicle shadow blobs no longer flicker against the road, and in Enhanced and Enhanced+ they darken the ground under the bus as in OMSI 2.

## 0.1.1541 - 2026-10-04

### Merged pull requests
- Performance: vehicle scripts run about a quarter faster [#1328](https://github.com/openOMSI-Project/openOMSI/pull/1328), and finding the ground under the wheels costs about a quarter less CPU [#1334](https://github.com/openOMSI-Project/openOMSI/pull/1334), with the same results.
- Texture memory: on Linux with NVIDIA's driver the card's memory is read from Vulkan, and cards over 6 GB get more automatic texture memory [#1320](https://github.com/openOMSI-Project/openOMSI/pull/1320).
- Automatic start-up no longer releases the starter too early (Volvo 7900H, HH109, WSW C2), the stock buses unchanged [#1147](https://github.com/openOMSI-Project/openOMSI/pull/1147).
- Trains: cars whose bogies are declared reversed face the right way, and cars are spaced by their declared couplings as in OMSI 2 [#1186](https://github.com/openOMSI-Project/openOMSI/pull/1186).
- Camera: an optional precision curve for zooming with both mouse buttons, and a zoom cursor while the right button zooms [#903](https://github.com/openOMSI-Project/openOMSI/pull/903); the driver's eye is back on the authored seat point [#1337](https://github.com/openOMSI-Project/openOMSI/pull/1337).
- Repository: new issues are sorted by topic and milestone, repeated crash reports point to the existing issue, and pull requests get translation and file size checks; dependency updates and a weekly security audit [#1329](https://github.com/openOMSI-Project/openOMSI/pull/1329).
- Code clean-ups after review [#1322](https://github.com/openOMSI-Project/openOMSI/pull/1322).

## 0.1.1518 - 2026-10-04

### New
- The chat has a size of its own: Ctrl + the mouse wheel over it, or Settings → General → Chat size (50-300 %), for large and 4K screens.

## 0.1.1517 - 2026-10-04

### Fixes
- The README's "playing now" badge keeps its label while the counter cannot be reached.

## 0.1.1515 - 2026-10-04

### New
- Multiplayer: every variable of the other players' buses (floats and strings: gearbox, displays, IBIS, ticket printer, plugin values) is synced, so their buses look and behave as they do for their drivers.
- Updates during a session: the game looks for a new version, downloads it in the background and says so over the navigator; it is installed when the session ends. The cards can be switched off (Settings → General → "Tell me about a new version during a session").
- The launcher looks for updates again every 30 minutes while it is open and after a game ends, not only when it starts.
- "Playing now": the website and the README show how many people play openOMSI right now. The game sends only a random per-session id, its version and the kind of system; switch it off under Settings → General.

### Fixes
- Updates no longer fail with "Timeout" on a slow or unsteady connection: requests are tried again, a broken download goes on where it stopped, and github.com is asked when the GitHub API does not answer.

## 0.1.1512 - 2026-10-04

### Merged pull requests
- Mirror panels: only the player's bus notes its mirrors' glass, so the panels no longer take another bus's mirrors [#1171](https://github.com/openOMSI-Project/openOMSI/pull/1171).
- Translations: Simplified Chinese numbers in their single-character forms [#1282](https://github.com/openOMSI-Project/openOMSI/pull/1282), Traditional Chinese labels refined [#1264](https://github.com/openOMSI-Project/openOMSI/pull/1264).

## 0.1.1502 - 2026-10-04

### Merged pull requests
- Force feedback: the wheel vibrates with the road surface and the engine, and jolts fade out instead of stopping dead, with three new sliders in the driving settings [#867](https://github.com/openOMSI-Project/openOMSI/pull/867).
- The navigator shows buses, trolleybuses and trams as pictograms pointing the way they go, not as dots [#1315](https://github.com/openOMSI-Project/openOMSI/pull/1315).
- Camera: the driver's head pitch can be set (-45 to +45 degrees) under Camera → Seat position [#1179](https://github.com/openOMSI-Project/openOMSI/pull/1179); the view can glide to where the mouse turned it (Look smoothing, off by default) [#998](https://github.com/openOMSI-Project/openOMSI/pull/998); the head can sway a little while the bus waits (Head sway at a standstill, off by default) [#1204](https://github.com/openOMSI-Project/openOMSI/pull/1204); a snappier F1 camera switch and an eased Space return [#870](https://github.com/openOMSI-Project/openOMSI/pull/870).
- Controllers: the gamepad's right stick turning the view can be switched off [#1170](https://github.com/openOMSI-Project/openOMSI/pull/1170), and an idle device nobody set up no longer stops the gamepad stick from steering [#1165](https://github.com/openOMSI-Project/openOMSI/pull/1165).
- Mouse steering switched on with O starts from the centre of the window [#1301](https://github.com/openOMSI-Project/openOMSI/pull/1301).
- Mirror panels (Ctrl+M) show each mirror as its glass does, and the panel editor lists its keys and resizes panels [#1171](https://github.com/openOMSI-Project/openOMSI/pull/1171).
- Passengers press the stop button at a random distance before their stop instead of all at the same point [#1191](https://github.com/openOMSI-Project/openOMSI/pull/1191).
- Timetable: after serving the terminus, driving off starts the next trip early when it is due within five minutes [#1199](https://github.com/openOMSI-Project/openOMSI/pull/1199).
- Servers: the status page counts the shared world (AI cars, buses, people) and an admin can set the AI traffic [#1206](https://github.com/openOMSI-Project/openOMSI/pull/1206); the dispatch can take a player's duty back [#1207](https://github.com/openOMSI-Project/openOMSI/pull/1207); the world options have a button to clear the AI traffic [#1234](https://github.com/openOMSI-Project/openOMSI/pull/1234).
- The launcher's Join can be told to connect over UDP or WebSocket [#958](https://github.com/openOMSI-Project/openOMSI/pull/958).
- Lua plugins can see and drive the vehicles round the bus (omsi.others) [#894](https://github.com/openOMSI-Project/openOMSI/pull/894).
- Player screenshots leave the touch controls out [#1188](https://github.com/openOMSI-Project/openOMSI/pull/1188).
- Texture memory is no longer capped at 2 GB on cards with more, and the VRAM of AMD cards on Linux is read [#1137](https://github.com/openOMSI-Project/openOMSI/pull/1137).
- Android: a black screen on Adreno 6xx, 7xx and 8xx chips is avoided by rebuilding their shader cache [#1310](https://github.com/openOMSI-Project/openOMSI/pull/1310).
- Translations: Simplified Chinese [#1282](https://github.com/openOMSI-Project/openOMSI/pull/1282), Traditional Chinese [#1264](https://github.com/openOMSI-Project/openOMSI/pull/1264), Polish [#1262](https://github.com/openOMSI-Project/openOMSI/pull/1262) and French [#1251](https://github.com/openOMSI-Project/openOMSI/pull/1251).

## 0.1.1423 - 2026-10-04

- Build: the small-BAR fix for NVIDIA cards ([#905](https://github.com/openOMSI-Project/openOMSI/pull/905)) now comes from the project's own copy of gpu-allocator, not a personal fork.

## 0.1.1422 - 2026-10-04

### Merged pull requests
- Performance: script names are looked up without allocating and debug switches read once [#1292](https://github.com/openOMSI-Project/openOMSI/pull/1292), a wheel's ground probe walks its cell's faces once [#1287](https://github.com/openOMSI-Project/openOMSI/pull/1287), a render origin moved sideways rewrites only the models' translations [#1283](https://github.com/openOMSI-Project/openOMSI/pull/1283), an instance given another mesh is updated alone [#1280](https://github.com/openOMSI-Project/openOMSI/pull/1280), the AI vehicles' heaviest scripts each get a job of their own [#1260](https://github.com/openOMSI-Project/openOMSI/pull/1260), traffic light lamps are set again only when what they show changed [#1259](https://github.com/openOMSI-Project/openOMSI/pull/1259), and the instances are culled by blocks of 128 before one by one [#1256](https://github.com/openOMSI-Project/openOMSI/pull/1256).
- AI cars no longer drive through red lights, and a car put on the road in front of a red light comes in slowly enough to stop [#1253](https://github.com/openOMSI-Project/openOMSI/pull/1253).
- The outside camera is no longer trapped under buses whose orbit centre lies below the ground clearance [#1246](https://github.com/openOMSI-Project/openOMSI/pull/1246).
- Crash reports carry the computer, its graphics card and the map, and their title is the error alone [#1225](https://github.com/openOMSI-Project/openOMSI/pull/1225).
- Dedicated servers: a player who drives a bus the `vehicles` list of `server.cfg` does not allow is sent away and told which buses the server has [#1222](https://github.com/openOMSI-Project/openOMSI/pull/1222) (the vehicle menu offers only those since 0.1.1382).
- Bus stops put on splines are placed right in the map index before their tiles load, so a duty's stops and the announcements are right from the start [#1208](https://github.com/openOMSI-Project/openOMSI/pull/1208).
- Terrain holes: the ground's exposed edges along spline and object cuts are closed by walls textured like the ground around them [#1069](https://github.com/openOMSI-Project/openOMSI/pull/1069).
- Linux: a gear shifter or a button box (a device with buttons only) can be set up [#1059](https://github.com/openOMSI-Project/openOMSI/pull/1059).
- NVIDIA cards without Resizable BAR (Vulkan): uploads no longer go to the small BAR heap, which made loading take minutes and the game run at a few fps [#905](https://github.com/openOMSI-Project/openOMSI/pull/905).
- [#1223](https://github.com/openOMSI-Project/openOMSI/pull/1223): the same OpenGL fix (Intel HD 2500, Mali) was already in 0.1.1382.

## 0.1.1382 - 2026-10-04

Bug Fixes & Improvements

## 0.1.1313 - 2026-10-03

### Triple screens
- Native triple-screen support with the HUD on the centre screen
  ([#1127](https://github.com/openOMSI-Project/openOMSI/pull/1127)), merged with fixes: the three
  panels are drawn at one size (the depth, AO and rain targets were made anew twice a frame),
  SSAO on the angled side panels, traffic and pedestrians no longer appear or vanish on them,
  fullscreen and Alt+Enter leave a spanned window alone, nothing is drawn after a lost device,
  and the Display page shows the rig's settings only while it is on. A single screen draws
  exactly as before.

## 0.1.1302 - 2026-10-03

### Fixes
- The Drive page's map no longer fills with the interface's words after a game or a lost
  graphics device: the launcher kept the map's texture number from its old device, and a
  number past the new device's textures was drawn with the font atlas.
- The game starts again on every system: Steam's rich presence ([#996](https://github.com/openOMSI-Project/openOMSI/pull/996)) is built only where
  its library exists (Windows x64, Linux x64, macOS), the library is put beside the program
  (also for the servers and a Linux build, which could not find it), and Android, Windows ARM64
  and macOS build again (0.1.1246 was not released because of it).
- The window losing the focus or being minimised lets go of every key, button and mouse
  steering held, and takes no input until it is back; a minimised game runs at 30 fps.

### Merged pull requests
- Voice chat in sessions through GreenTeaSpeak, players heard where they are [#1102](https://github.com/openOMSI-Project/openOMSI/pull/1102) - with a
  key handshake between the game and the plugin (a web page can no longer drive it), the voice
  server's id required, and the plugin putting the user's name and channel back.
- Resumed sessions keep the bus displays and the timetable's place, and the IBIS moves on to
  the next trip [#1087](https://github.com/openOMSI-Project/openOMSI/pull/1087).
- The server's notifications over the navigator [#974](https://github.com/openOMSI-Project/openOMSI/pull/974), a duty given by the server [#985](https://github.com/openOMSI-Project/openOMSI/pull/985),
  a fresh hello after the WebSocket is made again [#980](https://github.com/openOMSI-Project/openOMSI/pull/980), the paint the bus wears now seen by
  the other players [#1022](https://github.com/openOMSI-Project/openOMSI/pull/1022).
- Controllers: latching switches [#1060](https://github.com/openOMSI-Project/openOMSI/pull/1060), Linux wheels no longer buzz [#1042](https://github.com/openOMSI-Project/openOMSI/pull/1042); one key between
  cabin and outside [#1007](https://github.com/openOMSI-Project/openOMSI/pull/1007).
- Rendering: the render origin no longer flips while the camera is near it [#1126](https://github.com/openOMSI-Project/openOMSI/pull/1126), the
  ICU400 sign controller shows its text [#993](https://github.com/openOMSI-Project/openOMSI/pull/993), fewer meshes turned inside out [#1105](https://github.com/openOMSI-Project/openOMSI/pull/1105);
  Chinese AI car plates [#1002](https://github.com/openOMSI-Project/openOMSI/pull/1002); Volvo Wright rear doors [#1125](https://github.com/openOMSI-Project/openOMSI/pull/1125).
- Tile loading diagnostics [#1111](https://github.com/openOMSI-Project/openOMSI/pull/1111), cloudflared without console windows [#1001](https://github.com/openOMSI-Project/openOMSI/pull/1001), Simplified
  Chinese [#1028](https://github.com/openOMSI-Project/openOMSI/pull/1028), the radio documented [#1124](https://github.com/openOMSI-Project/openOMSI/pull/1124), the release history filled in [#1130](https://github.com/openOMSI-Project/openOMSI/pull/1130).

## 0.1.1246 - 2026-10-03

### AI traffic
- A car stopped behind a parked car just past a joint between road pieces pulls out round it:
  a lane change now carries on across the joint, so cars no longer queue for good behind a
  parked car on short road pieces ([#1055](https://github.com/openOMSI-Project/openOMSI/issues/1055)).
- A car held for a long time by nothing anybody can see is taken away even in view, and a
  car creeping a few centimetres at a time counts as standing; people and cars on a bridge
  above or in a subway below no longer stop a car.
- AI cars give way to the player's bus by the same rules as to each other - at junctions,
  where lanes merge, and when the bus is in the junction - and let a bus out of a stop when it
  indicates towards the road (for 20 s at most).

### Maps
- Route arrows show street and stop names in Cyrillic and other scripts their font lacks,
  drawn with the interface font in the arrow's colour.

## 0.1.1234 - 2026-10-03

### Launcher
- The Drive page is laid out again, in three steps - the bus, the day and the weather, the map
  and the duty. The column on the left is a solid panel; the bus or the map has a stage of its
  own on the right, the roadbook beside the map, and a foot under it sums the choice up with
  the buttons in a row. Nothing lies over the map or shows through a panel any more, the line
  and tour rows have room for their text, stop names keep off each other and inside the map,
  the lists stretch with the window, and the roadbook opens on your first trip.

### Graphics and devices
- OpenGL chips without storage buffers in the vertex shader (Intel HD 2500, Mali on GLES) draw
  the scene from textures instead of failing to start (`OMSI_GPU_ARRAYS=textures` forces it). [#770](https://github.com/openOMSI-Project/openOMSI/issues/770) [#316](https://github.com/openOMSI-Project/openOMSI/issues/316)
- The Vulkan and OpenGL interfaces are only started when DirectX 12 cannot draw, so their
  loader no longer breaks a DirectX 12 start on dual-GPU laptops. [#1058](https://github.com/openOMSI-Project/openOMSI/issues/1058) [#1044](https://github.com/openOMSI-Project/openOMSI/issues/1044)
- A chip without a DirectX 12 driver draws on its own Vulkan or OpenGL instead of Microsoft's
  software renderer. [#770](https://github.com/openOMSI-Project/openOMSI/issues/770)
- Script, text and HTML textures larger than the chip takes are halved until they fit.
- A window the system cannot open ends with a message instead of a panic report.
- A script value that is not a number no longer turns the bus to NaN and stops the game. [#1045](https://github.com/openOMSI-Project/openOMSI/issues/1045)

## 0.1.1223 - 2026-10-03

### Driving and controls
- The cab's steering wheel turns as far left as right: `Axle_Steering_N_L` and `_R` carry the
  axle's one angle, as Omsi.exe hands it to the scripts, not each tyre's Ackermann angle. [#953](https://github.com/openOMSI-Project/openOMSI/issues/953) [#1073](https://github.com/openOMSI-Project/openOMSI/issues/1073)
- A steering or pedal key let go while Shift is held, or while the game menu is open, lets go
  instead of turning the wheel on to full lock. [#1040](https://github.com/openOMSI-Project/openOMSI/issues/1040) [#1050](https://github.com/openOMSI-Project/openOMSI/issues/1050) [#1053](https://github.com/openOMSI-Project/openOMSI/issues/1053)
- Mouse steering switched off in the game menu keeps the brake on, as the O key does. [#517](https://github.com/openOMSI-Project/openOMSI/issues/517) [#760](https://github.com/openOMSI-Project/openOMSI/issues/760)
- Ctrl+click on the ground in the free view (F4) moves the bus to the nearest street there. [#1039](https://github.com/openOMSI-Project/openOMSI/issues/1039)
- The engine-off hint names the player's own key for the electrics. [#461](https://github.com/openOMSI-Project/openOMSI/issues/461)

### Timetable and passengers
- An early timetable bus waits at its stop until 20 s before its departure (a train 2 min), as
  in Omsi.exe, instead of leaving after 40 s. [#1012](https://github.com/openOMSI-Project/openOMSI/issues/1012)
- A tour that repeats the same trip no longer leaves its bus at the trip's last stop for good. [#976](https://github.com/openOMSI-Project/openOMSI/issues/976)
- Stops hung on another object (`[attachObj]`) are on the route map, and a duty's stop whose
  tile was not loaded at the start is reached once it loads. [#1014](https://github.com/openOMSI-Project/openOMSI/issues/1014) [#975](https://github.com/openOMSI-Project/openOMSI/issues/975)
- People leaving a bus walk onto the pavement at their own pace instead of sliding sideways. [#1033](https://github.com/openOMSI-Project/openOMSI/issues/1033) [#1079](https://github.com/openOMSI-Project/openOMSI/issues/1079)

### AI traffic and maps
- A map without `unsched_vehgroups.txt` takes its random traffic from the default AI group alone,
  as Omsi.exe does. [#1025](https://github.com/openOMSI-Project/openOMSI/issues/1025)
- AI cars whose road is drawn more than 0.3 m under their lane come down onto it instead of
  driving in the air. [#876](https://github.com/openOMSI-Project/openOMSI/issues/876)
- A signal that names no crossing is still a lamp, and reads phase 0 as in Omsi.exe. [#988](https://github.com/openOMSI-Project/openOMSI/issues/988)
- A terrain object lying outside its own tile is not drawn, as in OMSI (leftovers of copied
  tiles no longer stand in the road). [#787](https://github.com/openOMSI-Project/openOMSI/issues/787)

### Graphics
- Texture names stored in a Korean, Chinese or Japanese code page are found. [#990](https://github.com/openOMSI-Project/openOMSI/issues/990)
- A `[rendertype] surface` object with `[matl_alpha] 2` on a texture without alpha is opaque, so
  roads behind it no longer show through. [#1008](https://github.com/openOMSI-Project/openOMSI/issues/1008)
- Enhanced: the custom weather's brightness no longer darkens the mirrors; a white light map no
  longer bleaches displays and gauges at night. [#1018](https://github.com/openOMSI-Project/openOMSI/issues/1018) [#827](https://github.com/openOMSI-Project/openOMSI/issues/827)
- Vanilla+: the sphere map is blended on encoded colours as in Vanilla and Omsi.exe, so
  reflections are no longer twice too strong over dark surfaces. [#780](https://github.com/openOMSI-Project/openOMSI/issues/780)
- A mesh borrowed from another vehicle pack keeps the winding of its own pack. [#977](https://github.com/openOMSI-Project/openOMSI/issues/977) [#1054](https://github.com/openOMSI-Project/openOMSI/issues/1054)

### Menus and platforms
- Scroll bars of the game menu's drop-downs and of a line's stop list can be dragged. [#794](https://github.com/openOMSI-Project/openOMSI/issues/794)
- Names macOS stores decomposed (the weather "Eiseskälte") show their accented letters.
- A bus whose `[boundingbox]` has a negative size no longer stops the game. [#986](https://github.com/openOMSI-Project/openOMSI/issues/986)
- macOS: a fresh openOMSI.app in Applications keeps its content in `~/.openomsi/content` instead
  of creating OMSI's folders among the applications; content already installed there stays. [#1043](https://github.com/openOMSI-Project/openOMSI/issues/1043)

## 0.1.1196 - 2026-10-03

### Roads and driving
- Roads are drawn at their own height again and win over the ground by the surfaces' depth bias,
  as in Omsi.exe; the 8 cm lift is gone, so footways no longer stand over the ground aligned to
  them and markings no longer sink under the road. [#1077](https://github.com/openOMSI-Project/openOMSI/pull/1077)
- The bumps of a road texture's `.surf` map (cobbles, slabs, broken asphalt) are felt under the
  wheels, on splines and on junction objects alike (`OMSI_NO_SURF=1` for an A/B). [#1056](https://github.com/openOMSI-Project/openOMSI/pull/1056)
- Train cars stand end to end by their model bodies, not by their declared coupling points
  (a CR200J's second car no longer sits 2.6 m inside the first). [#1016](https://github.com/openOMSI-Project/openOMSI/pull/1016)

### Graphics
- Wet-road reflections work in Vanilla and Vanilla+ as well as Enhanced, also seen through the
  bus's own windows; rain films no longer feed the previous frame back into themselves. [#970](https://github.com/openOMSI-Project/openOMSI/pull/970)
- The texture fallback index looks into archives and content installed while the game runs. [#1029](https://github.com/openOMSI-Project/openOMSI/pull/1029)

### Maps and launcher
- On the navigator and city map, trolleybuses, buses and trams have their own colours and show
  their line. [#992](https://github.com/openOMSI-Project/openOMSI/pull/992)
- The Drive page shows the map itself: its roads, entry points and the chosen line's route. [#943](https://github.com/openOMSI-Project/openOMSI/pull/943)
- A server whose map is not installed here can be joined: the map comes with the server's mods. [#1076](https://github.com/openOMSI-Project/openOMSI/pull/1076)

## 0.1.1169 - 2026-10-03

### Graphics
- A declared `[matl_transmap]` whose image is missing still keeps its opaque texture slot, as
  Omsi.exe does, so traffic bodies that use diffuse alpha as a reflection mask are no longer
  half transparent.

## 0.1.1166 - 2026-10-02

### Graphics
- A vehicle's blended layers write depth unless `[matl_noZwrite]`, exactly as Omsi.exe sets its
  states (0x7fd6c4), and are drawn in model order with no reordering: stacked panes (door glass
  with dirt, decals on glass) no longer see through each other. The vehicle the camera is in is
  drawn after the whole scene and its own lamp flares after it, as in Omsi.exe. [#211](https://github.com/openOMSI-Project/openOMSI/issues/211) [#596](https://github.com/openOMSI-Project/openOMSI/issues/596)
- A 16-bit TGA keeps its alpha bit (A1R5G5B5, as D3DX reads it).

### Sound
- Sounds are mixed as in Omsi.exe's sound update (0x750340): the volume is checked against
  0 dB after the distance factor (DirectSound keeps the last volume when asked for more), a loop's
  pitch outside 100-200,000 Hz is refused and the last rate kept (the LiAZ and trolleybus
  "whine" played 30x too fast), AI vehicles use viewpoint 4, sounds of other vehicles heard from
  the cab are x(0.2 + Snd_OutsideVol) with no invented filter, only `[3d]` sounds are panned
  (5 dB at most), Doppler only on loop sounds, conditions compared exactly.

### Passengers and cash desk
- Coins lie flat on the change tray at random spots and turns, as Omsi.exe places them, instead
  of a tower that grew 3 mm a coin.
- People come from one pool, as in Omsi.exe: `ai_max_humans` (OMSI's `[AIMaxCountRandom]`,
  200 by default), walkers at most half of it. A waiting place another stop's person stands on
  is not free, so no two people stand inside each other.
- A duty moves on to the next trip when the bus stands at its first stop a minute before
  departure, so the IBIS is no longer left on the old terminus with riders refusing to board.

### Physics
- AI wheels each stand on the highest drawn face up to 3 m above them, as Omsi.exe's ground
  query (0x7a0814): no more cars sunk into cambered roads or hidden inside roads above the lane.
- A wheel no longer falls through the hair-wide seam between two spline segments (an 18 cm drop).

### Menu and website
- The pause menu uses the launcher's greys, switches, sliders and scroll bars, follows the
  Interface size, and lists destination codes in a column.
- The website has a light theme (and a theme button), readable colours, and no sideways
  scrolling on phones.

## 0.1.1147 - 2026-10-02

### Mirrors
- In the cab, Ctrl+M can put copies of the bus's mirrors over the picture; Ctrl+Shift+M opens
  an editor to move, resize, add, aim and zoom them. Layouts are saved per bus. [#920](https://github.com/openOMSI-Project/openOMSI/pull/920)

### LAN and chat
- After a LAN connection times out, the client keeps trying the host and can rejoin without
  restarting the game; `/reconnect` retries immediately. [#926](https://github.com/openOMSI-Project/openOMSI/pull/926)
- Long chat messages wrap onto following rows instead of being cut with an ellipsis. [#969](https://github.com/openOMSI-Project/openOMSI/pull/969)
- A dedicated server behind a local web gateway logs the player's forwarded address, so the
  operator can distinguish clients that would otherwise all appear as loopback. [#962](https://github.com/openOMSI-Project/openOMSI/pull/962)

### Radio
- A bus radio display can show the station and song that are actually playing. A map can supply
  its own stations in `radio.cfg` and give them frequencies by position on the map. [#945](https://github.com/openOMSI-Project/openOMSI/pull/945)

### Graphics and textures
- In Enhanced, a soaked road has puddles in patches instead of becoming one mirror from kerb
  to kerb. [#950](https://github.com/openOMSI-Project/openOMSI/pull/950)
- A missing scenery or spline texture can be found from another folder of the same content
  type when the installation contains a matching image. [#912](https://github.com/openOMSI-Project/openOMSI/pull/912)
- In Vanilla+, terrain under street lamps keeps its ground colour instead of getting a white
  or beige veil at night. [#935](https://github.com/openOMSI-Project/openOMSI/pull/935)

### Routes and input
- Route numbers with symbols, such as `-10`, are kept as written from a HOF or manual input
  and work on destination displays; the free route-number field accepts keyboard symbols. [#967](https://github.com/openOMSI-Project/openOMSI/pull/967)

## 0.1.1122 - 2026-10-02

### Graphics
- In Vanilla+, terrain no longer gets a white veil at night.
  [#935](https://github.com/openOMSI-Project/openOMSI/pull/935)

## 0.1.1120 - 2026-10-02

### Passengers and doors
- A timetable bus waits only for the people walking up to its doors from the stop it serves and
  those getting off, as Omsi.exe does, so a full bus no longer stands with open doors for ever. [#767](https://github.com/openOMSI-Project/openOMSI/issues/767)
- People off a bus walk on along the pavement instead of milling round each other at the stop. [#913](https://github.com/openOMSI-Project/openOMSI/issues/913)
- A controller button or a keyboard.cfg key can work door 1-9 front to back, or all doors
  (`door_1` ... `door_9`, `doors_all`). [#916](https://github.com/openOMSI-Project/openOMSI/issues/916)
- A second Shift+1 shuts both front leaves of the SD202 again.

### Graphics
- A transmapped car body is see-through only where its transmap is: AI cars are no longer half
  transparent, with wheel arches showing through. [#928](https://github.com/openOMSI-Project/openOMSI/issues/928) [#932](https://github.com/openOMSI-Project/openOMSI/issues/932)
- A night or light map named in a paint scheme's `[CTCTexture]` is the scheme's picture. [#895](https://github.com/openOMSI-Project/openOMSI/issues/895)
- An active chrono event that reshapes a tile brings its own terrain and water. [#923](https://github.com/openOMSI-Project/openOMSI/issues/923) [#925](https://github.com/openOMSI-Project/openOMSI/issues/925)
- A full beam reaches as much further than the low beam as its `[spotlight]` range says. [#941](https://github.com/openOMSI-Project/openOMSI/issues/941)
- A see-through layer drawn in model order (a sticker on a window) is no longer painted over by
  the opaque parts listed after it. [#918](https://github.com/openOMSI-Project/openOMSI/issues/918)

### Traffic
- A scripted child of a crossing that names one of its lights reads that light's phase, so such
  traffic lights no longer flash yellow. [#922](https://github.com/openOMSI-Project/openOMSI/issues/922)

### Vehicles
- The rear section's wheels of an articulated bus spring on the road under each of them. [#901](https://github.com/openOMSI-Project/openOMSI/issues/901)
- Textures waiting to be compressed go up at half size meanwhile, so a big articulated bus no
  longer runs a 3 GB graphics card out of memory while loading. [#921](https://github.com/openOMSI-Project/openOMSI/issues/921)
- A bus takes its own depot file of the map's place before another bus's. [#896](https://github.com/openOMSI-Project/openOMSI/issues/896)
- A four-digit line such as 7110 keeps its number on the IBIS. [#459](https://github.com/openOMSI-Project/openOMSI/issues/459)

### Launcher, menu and input
- The launcher keeps "Hold manual gear buttons", and the pause menu's switch applies to the bus.
- The passenger view (F2) turns all the way round. [#909](https://github.com/openOMSI-Project/openOMSI/issues/909)
- "Camera..." in the pause menu opens the driver's view settings. [#908](https://github.com/openOMSI-Project/openOMSI/issues/908)
- On OpenGL no thread polls the GPU beside the one drawing (a crash at start). [#898](https://github.com/openOMSI-Project/openOMSI/issues/898)
- The minimap can be dragged anywhere on the screen. [#940](https://github.com/openOMSI-Project/openOMSI/issues/940)
- Ctrl+Up / Ctrl+Down (gear up and down) are keys of the list that can be moved or cleared. [#907](https://github.com/openOMSI-Project/openOMSI/issues/907) [#930](https://github.com/openOMSI-Project/openOMSI/issues/930)
- A window size can be chosen (Graphics > Window size); under gamescope the game opens full screen. [#904](https://github.com/openOMSI-Project/openOMSI/issues/904)

## 0.1.1098 - 2026-10-02

### Passengers
- Riders complain about hard braking, fast bends and a jerky foot on the pedals (TooBad_A/B/C
  of the ticket packs), as in OMSI; the third time they get off at the next stop. [#862](https://github.com/openOMSI-Project/openOMSI/issues/862) [#873](https://github.com/openOMSI-Project/openOMSI/issues/873)
- After the bus was removed, the mouse wheel zooms on foot and the field of view holds. [#837](https://github.com/openOMSI-Project/openOMSI/issues/837)

### LAN
- A stop whose waiting people another player's bus took fills up again only once that bus has
  left, instead of a passenger a frame: no more endless streams of riders. [#842](https://github.com/openOMSI-Project/openOMSI/issues/842) [#840](https://github.com/openOMSI-Project/openOMSI/issues/840) [#830](https://github.com/openOMSI-Project/openOMSI/issues/830)
- The chat line opens on the key left of 1 where '/' is the manual gearbox's gear down.

### Graphics
- A compressed smooth dark texture keeps its colour, no longer grainy with green and blue. [#845](https://github.com/openOMSI-Project/openOMSI/issues/845)
- A mesh with no matrix or the identity is drawn as wound, as Omsi.exe draws it, so houses
  are no longer shown inside out. [#874](https://github.com/openOMSI-Project/openOMSI/issues/874)
- The see-through part of a blended layer shows no reflection (stickers). [#861](https://github.com/openOMSI-Project/openOMSI/issues/861)
- A map's WinterSnow textures show their own snow, with no white laid over them. [#879](https://github.com/openOMSI-Project/openOMSI/issues/879)
- Falling snow covers the windscreen as rain does, and the wipers clear it. [#883](https://github.com/openOMSI-Project/openOMSI/issues/883)
- In Enhanced the map's water is drawn as water: small waves, mirroring the sky. [#841](https://github.com/openOMSI-Project/openOMSI/issues/841)
- Street lamps' light map lights the roads as it lights the ground beside them. [#847](https://github.com/openOMSI-Project/openOMSI/issues/847)
- Road markings and zebra crossings lie on the road again instead of under it. [#871](https://github.com/openOMSI-Project/openOMSI/issues/871)

### Maps
- Objects beside a crossing stand on the ground the map gives them: the terrain is no longer
  pressed into a crossing's height mesh, which Omsi.exe uses for its paths only
  (`OMSI_CROSSING_DEFORM=1` brings the old way back). [#860](https://github.com/openOMSI-Project/openOMSI/issues/860)

### Vehicles
- `gear_up` shifts a gear lever past first gear (VW T3, Peugeot 106, Manta), and a force
  feedback wheel that is not set up steers without a dead zone. [#866](https://github.com/openOMSI-Project/openOMSI/issues/866)
- An automated manual gearbox (in-game driving settings, off by default). [#713](https://github.com/openOMSI-Project/openOMSI/issues/713)
- `A_Trans_*` is taken over OMSI's thirtieth-of-a-second frames, so rattle scripts rattle on
  rough roads at any frame rate. [#772](https://github.com/openOMSI-Project/openOMSI/issues/772) [#886](https://github.com/openOMSI-Project/openOMSI/issues/886)
- The game menu swaps the driven vehicle for another in its place, or reloads it. [#728](https://github.com/openOMSI-Project/openOMSI/issues/728)

### Launcher, menu and input
- "A right click ends the mouse steering" is back in the menu and the launcher. [#878](https://github.com/openOMSI-Project/openOMSI/issues/878)
- "The launcher rests while a game runs" is a setting. [#834](https://github.com/openOMSI-Project/openOMSI/issues/834)
- Mouse look sensitivity is a setting (100% = OMSI). [#859](https://github.com/openOMSI-Project/openOMSI/issues/859)
- Parked cars can be left out altogether (Parked cars: None). [#864](https://github.com/openOMSI-Project/openOMSI/issues/864)
- "Indicators cancel themselves" can be switched off. [#451](https://github.com/openOMSI-Project/openOMSI/issues/451)
- The touch wheel turns as far as Wheel rotation and Full lock say. [#856](https://github.com/openOMSI-Project/openOMSI/issues/856)
- An action can be given another key, and a mod's own trigger can be added to the list. [#854](https://github.com/openOMSI-Project/openOMSI/issues/854)
- Any route number can be typed (Route number > Type a route number...). [#836](https://github.com/openOMSI-Project/openOMSI/issues/836)
- A gamepad's right stick turns the driver's head. [#454](https://github.com/openOMSI-Project/openOMSI/issues/454)
- Buses can be starred in the bus list ("Favourites only"). [#524](https://github.com/openOMSI-Project/openOMSI/issues/524)
- On OpenGL a wait for the GPU no longer holds the GL context for seconds (a crash). [#843](https://github.com/openOMSI-Project/openOMSI/issues/843)
- On a phone, a crash long after the shaders were compiled is no longer blamed on Vulkan. [#848](https://github.com/openOMSI-Project/openOMSI/issues/848)

### Merged pull requests
- [#892](https://github.com/openOMSI-Project/openOMSI/pull/892) fractional `achse_antrieb` drives the axle, [#888](https://github.com/openOMSI-Project/openOMSI/pull/888) free camera fly keys,
  [#885](https://github.com/openOMSI-Project/openOMSI/pull/885) manual gearbox detection for touch controls, [#891](https://github.com/openOMSI-Project/openOMSI/pull/891) staged ignition start-up,
  [#881](https://github.com/openOMSI-Project/openOMSI/pull/881) Portuguese, [#839](https://github.com/openOMSI-Project/openOMSI/pull/839) mobile launcher options, [#789](https://github.com/openOMSI-Project/openOMSI/pull/789) custom weather editor,
  [#792](https://github.com/openOMSI-Project/openOMSI/pull/792) real-time reflections setting for the mirrors, [#910](https://github.com/openOMSI-Project/openOMSI/pull/910) mirror budget counted once.

## 0.1.1068 - 2026-10-02

### Mirrors
- Mirrors are no longer redrawn twice as often as their refresh budget allows.
  [#910](https://github.com/openOMSI-Project/openOMSI/pull/910)

## 0.1.1066 - 2026-10-02

### Mirrors
- Mirror reflections have a real-time setting with None, Economical and Full modes.
  [#792](https://github.com/openOMSI-Project/openOMSI/pull/792)

## 0.1.1061 - 2026-10-02

### Weather
- Custom weather can be edited from the launcher and in-game, including visibility, wind,
  temperature, humidity, clouds, precipitation, road wetness and snow. METAR ICAO stations
  can be entered directly, the controls are available on mobile too, and LAN clients follow
  a host's custom weather.
  [#789](https://github.com/openOMSI-Project/openOMSI/pull/789)

## 0.1.1050 - 2026-10-02

### Vehicles
- Very small resting speeds count as stopped, so systems that require a stationary vehicle,
  including passenger boarding, do not get held up by tiny residual motion.
  [#897](https://github.com/openOMSI-Project/openOMSI/pull/897)

## 0.1.1043 - 2026-10-02

### Tests
- Gearbox tests declare their script variables the same way a bus varlist does, including the
  shared-cockpit manual gearbox case.

## 0.1.1042 - 2026-10-02

### Vehicles
- An axle with a fractional `achse_antrieb` share is treated as a driven axle.
  [#892](https://github.com/openOMSI-Project/openOMSI/pull/892)

## 0.1.1038 - 2026-10-02

### Camera and controls
- Free-camera fly keys no longer fire OMSI `[game]` actions at the same time.
  [#888](https://github.com/openOMSI-Project/openOMSI/pull/888)

## 0.1.1036 - 2026-10-02

### Force feedback
- Road seams no longer kick the steering wheel.
  [#821](https://github.com/openOMSI-Project/openOMSI/pull/821)

## 0.1.1028 - 2026-10-02

### Passengers
- People getting off who wait at a shut exit keep to its point; they are no longer lifted up
  inside the bus and stacked there, blocking everyone behind them. [#709](https://github.com/openOMSI-Project/openOMSI/issues/709)
- Getting off, passengers head for the nearest open exit, not a shut door, as in OMSI. [#493](https://github.com/openOMSI-Project/openOMSI/issues/493)
- `GetHumanCountOnSeat` numbers the seats with the driver's place, as OMSI does (seats were one
  off for scripts).
- The driver no longer stands in a T-pose in the aisle before he is first seated.

### LAN
- Passengers handed over to a client's bus ride to a stop instead of getting off at once. [#813](https://github.com/openOMSI-Project/openOMSI/issues/813)
- The chat keys are in the key list (Controls) and can be moved off a key the bus needs. [#130](https://github.com/openOMSI-Project/openOMSI/issues/130)

### Traffic and maps
- A junction runs its traffic lights for a lamp without `[trafficlight]`, as Omsi.exe does, so
  mod traffic lights work again and AI cars stop at them. [#822](https://github.com/openOMSI-Project/openOMSI/issues/822) [#818](https://github.com/openOMSI-Project/openOMSI/issues/818)
- Timetable AI buses pass a stop where nobody gets off or waits, as in OMSI. [#703](https://github.com/openOMSI-Project/openOMSI/issues/703)
- An object with traffic paths keeps the height the map gives it (mod road pieces). [#828](https://github.com/openOMSI-Project/openOMSI/issues/828)
- A car standing behind a vehicle that waits to move over lets it in (a deadlock). [#414](https://github.com/openOMSI-Project/openOMSI/issues/414)
- A `[terrainhole]` cuts the ground along the cutter's rim, so no grass stands over the edges of
  a junction's carriageway. [#823](https://github.com/openOMSI-Project/openOMSI/issues/823) [#672](https://github.com/openOMSI-Project/openOMSI/issues/672)

### Graphics
- A bus's outer skin below the roof is lit as outside, not as cab: no seam round the body and
  its reflections are back in Enhanced. [#805](https://github.com/openOMSI-Project/openOMSI/issues/805)
- `Envir_Brightness` is the night light plus the street lamps' light on the vehicle, so bus glass
  no longer turns clear at night. [#624](https://github.com/openOMSI-Project/openOMSI/issues/624)
- Signal lenses follow their `[alphascale]` and `[matl_lightmap]` variables, and scenery objects
  get their light maps. [#826](https://github.com/openOMSI-Project/openOMSI/issues/826)

### Vehicles
- The rear section of an articulated bus finds a viaduct's deck again after a gap. [#135](https://github.com/openOMSI-Project/openOMSI/issues/135)
- A rear section turns about its own `[rot_pnt_long]` line, so steered rear axles work. [#322](https://github.com/openOMSI-Project/openOMSI/issues/322)
- `A_Trans_X/Y/Z` are the body's acceleration without gravity, as in Omsi.exe (`A_Trans_Z` read
  9.81 standing, so air suspension scripts never re-levelled).

### Launcher, menu and input
- The launcher brought forward while a game runs is drawn and answers again. [#825](https://github.com/openOMSI-Project/openOMSI/issues/825)
- A long tour no longer makes the Timetable page lose its sidebar. [#666](https://github.com/openOMSI-Project/openOMSI/issues/666)
- Save slots: the game menu saves to a new slot, and the launcher continues from any. [#341](https://github.com/openOMSI-Project/openOMSI/issues/341)
- A bus deleted from `Mods/installed` is uninstalled (its folder kept in `Mods/uninstalled`). [#819](https://github.com/openOMSI-Project/openOMSI/issues/819)
- The exit key of keyboard.cfg (Ctrl+Q) ends the game. [#817](https://github.com/openOMSI-Project/openOMSI/issues/817)
- Mouse steering goes on in the map camera (F4), as in OMSI. [#516](https://github.com/openOMSI-Project/openOMSI/issues/516)
- The paper timetable's rows are as high as GDI makes them and stay on the paper's lines. [#629](https://github.com/openOMSI-Project/openOMSI/issues/629)
- On Linux the new launcher is started after an update, not the old one. [#811](https://github.com/openOMSI-Project/openOMSI/issues/811)

## 0.1.1000 - 2026-10-02

### Menu and UI
- Interaction text is hidden when the object cannot actually be reached.
- Loading a graphics profile works from the in-game menu, and 8x MSAA is available there.
- The in-game server code can be copied, cursor flicker is fixed, and menu selection/rendering
  is less distracting.
  [#846](https://github.com/openOMSI-Project/openOMSI/pull/846)

## 0.1.983 - 2026-10-02

### Launcher and controls
- A tour can be started from a chosen trip instead of only from the duty's first one.
- Real-time and real-date options keep the launcher's choice on the system clock.
- Controller actions use readable OMSI names, and Alt+Enter switches between a window and
  borderless full screen.
  [#758](https://github.com/openOMSI-Project/openOMSI/pull/758)

## 0.1.973 - 2026-10-02

### Driver
- The driver's pose and arms are more natural: the torso sits straighter, shoulders stay
  visible through turns, and both hands remain on the wheel except while shifting.
  [#734](https://github.com/openOMSI-Project/openOMSI/pull/734)

## 0.1.968 - 2026-10-02

### Discord
- Discord Rich Presence can show the launcher or the current map, bus, line and tour, and can
  be disabled in Settings.
  [#779](https://github.com/openOMSI-Project/openOMSI/pull/779)

## 0.1.965 - 2026-10-02

### Documentation
- The changelog was expanded with 29 issue fixes from the preceding development work.

## 0.1.960 - 2026-10-02

### Passengers
- A stop is no longer a destination of itself or of a stop of the same name, so riders on
  circular lines no longer board and get straight off again. [#795](https://github.com/openOMSI-Project/openOMSI/issues/795)
- Riders know their stop by its timetable name as well as by its map label, so they get off
  at intermediate stops where the two names differ, not only at the terminus. [#773](https://github.com/openOMSI-Project/openOMSI/issues/773) [#748](https://github.com/openOMSI-Project/openOMSI/issues/748) [#736](https://github.com/openOMSI-Project/openOMSI/issues/736)

### Traffic and maps
- Cars drive into the end of a road at speed and are taken off there, as in OMSI, instead of
  stopping before it one by one; released AI buses no longer queue at the map's end. [#765](https://github.com/openOMSI-Project/openOMSI/issues/765) [#598](https://github.com/openOMSI-Project/openOMSI/issues/598)
- The player's bus holds up only traffic on its own level, not cars on a bridge above it. [#753](https://github.com/openOMSI-Project/openOMSI/issues/753)
- A depot file's stop list belongs to the trip written before it, as Omsi.exe reads it; a trip
  without a list no longer gives every later route another trip's stops in the IBIS. [#667](https://github.com/openOMSI-Project/openOMSI/issues/667)
- On a Chinese, Japanese or Korean Windows, Russian HOF and map text keeps its stop names. [#785](https://github.com/openOMSI-Project/openOMSI/issues/785)
- A continued situation resumes at the trip of the tour it was saved on, with the rest of the
  tour, and is saved again with that trip. [#653](https://github.com/openOMSI-Project/openOMSI/issues/653)

### Vehicles and input
- Door hit sounds with a `doorSpeed` volume curve are heard: a triggered sound reads its
  volume curves as they stand when the trigger fires (player's and AI buses). [#676](https://github.com/openOMSI-Project/openOMSI/issues/676)
- A switch let go with the right button held stays where it is. [#769](https://github.com/openOMSI-Project/openOMSI/issues/769)
- On foot, the switches of an articulated bus's rear section are in reach by that section. [#715](https://github.com/openOMSI-Project/openOMSI/issues/715)
- A key bound in both [game] and [vehicles] works the vehicle too. [#745](https://github.com/openOMSI-Project/openOMSI/issues/745)
- The brake stays on when the mouse steering is switched off. [#517](https://github.com/openOMSI-Project/openOMSI/issues/517) [#760](https://github.com/openOMSI-Project/openOMSI/issues/760)
- The built-in view keys step aside for keys the player bound themselves. [#701](https://github.com/openOMSI-Project/openOMSI/issues/701)
- The city map (Shift+M) and the navigator (Shift+N) open on foot. [#705](https://github.com/openOMSI-Project/openOMSI/issues/705)
- The ticket desk camera keeps where it was turned (Home recentres only when unbound). [#733](https://github.com/openOMSI-Project/openOMSI/issues/733)
- Head tracking stays on when its UDP port is taken by opentrack. [#775](https://github.com/openOMSI-Project/openOMSI/issues/775)

### Launcher
- Dropdown lists scroll by dragging their bar, and sliding an open list no longer scrolls the
  page behind it. [#794](https://github.com/openOMSI-Project/openOMSI/issues/794) [#774](https://github.com/openOMSI-Project/openOMSI/issues/774)
- Typing into an open dropdown filters it (entry points, buses, fleet numbers). [#747](https://github.com/openOMSI-Project/openOMSI/issues/747)
- The game's and the launcher's windows fit the screen and open in its middle. [#771](https://github.com/openOMSI-Project/openOMSI/issues/771)

### Graphics
- No rain inside the rear section of articulated buses. [#777](https://github.com/openOMSI-Project/openOMSI/issues/777)
- Clouds no longer vanish when a sky texture cannot be read; 24-bit bitfield BMPs load. [#749](https://github.com/openOMSI-Project/openOMSI/issues/749)
- The VR headset shows the Enhanced graphics. [#784](https://github.com/openOMSI-Project/openOMSI/issues/784)
- Fleet numbers and plates no longer glow in the dark; only text with a light or night map
  does. [#698](https://github.com/openOMSI-Project/openOMSI/issues/698)
- The loading screen remakes a Vulkan swapchain that no longer fits instead of freezing. [#776](https://github.com/openOMSI-Project/openOMSI/issues/776)

## 0.1.936 - 2026-10-02

### Graphics
- Text textures no longer break up when viewed from a distance.
  [#806](https://github.com/openOMSI-Project/openOMSI/pull/806)

## 0.1.929 - 2026-10-02

### VR
- VR navigator settings are restored in the new pause menu
  [#808](https://github.com/openOMSI-Project/openOMSI/pull/808)

## 0.1.927 - 2026-10-02

### Menu
- A new in-game menu is implemented, together with real-time and METAR synchronisation.
  [#679](https://github.com/openOMSI-Project/openOMSI/pull/679)

### Maps
- Far stand-in models are drawn only from the tiles OMSI loads together with them, so
  stand-ins no longer appear for tiles that are not part of the loaded set.
  [#743](https://github.com/openOMSI-Project/openOMSI/pull/743)
- Scenery sign text is aligned correctly again.
  [#764](https://github.com/openOMSI-Project/openOMSI/pull/764)

### Input
- Steering wheel hats are read on Linux, and the list scrolls to the pressed button.
  [#788](https://github.com/openOMSI-Project/openOMSI/pull/788)

### Physics
- Wheels no longer jolt when driving over stacked road surfaces.
  [#757](https://github.com/openOMSI-Project/openOMSI/pull/757)

## 0.1.864 - 2026-10-02

### AI traffic
- Cars simulated out of range take the same routes that an active car on the road would take.
  [#782](https://github.com/openOMSI-Project/openOMSI/pull/782)

## 0.1.862 - 2026-10-02

### AI buses
- AI buses can pull into a stop bay immediately after a junction instead of missing it.
  [#737](https://github.com/openOMSI-Project/openOMSI/pull/737)

## 0.1.857 - 2026-10-02

### Displays
- Masterbus destination displays work again.
  [#768](https://github.com/openOMSI-Project/openOMSI/pull/768)

## 0.1.853 - 2026-10-02

### Graphics
- Vehicle glow bitmaps are no longer upside down and use the same sizing as OMSI 2.
  [#741](https://github.com/openOMSI-Project/openOMSI/pull/741)

## 0.1.851 - 2026-10-02

### Maps
- Attached objects (`[attachObj]`) and object labels are read as Omsi.exe reads them. The
  records of a tile depend on its `[version]`: before version 9 there is no detail level
  line, before 6 no IDCodes, before 10 an attached object names its parent by its place in
  the tile instead of its IDCode, before 8 it has no heading, and before 11 a spline has one
  line linking it to the spline before it instead of both neighbours. Older tiles were read
  as version 14 ones, so their objects hung on the wrong parents and pieces of road stood
  in the air. An object's labels are now exactly as many lines as it says (an empty label
  or one in brackets no longer cuts the rest off), an attachment whose parent is written
  after it or that hangs on a later object of a spline attachment row is not loaded, as in
  OMSI.

## 0.1.850 - 2026-10-02

### Launcher
- Big installations no longer show an empty launcher for minutes: the maps appear at once and
  the buses as their folders are read, several folders at a time, with the progress in the
  status line. A poll no longer starts the whole reading over while it runs (the growing cache
  changed the content stamp, so a large OMSI folder never finished loading). Depot files are
  read for their name only, and the search of every vehicle folder for a map's depot runs
  once per session.

### Driving
- Mouse steering stays on: the right button looks round without ending it (also in the
  pause), the cursor goes back to where it steered when the button is let go, and the game
  starts with mouse steering as it was left, the cursor in the middle of the window. OMSI's
  right click that ends mouse steering is a setting (Esc > Options > "A right click ends the
  mouse steering").
- Door keys of UK buses: on Road-hog123's door script (London Citybus 400, Enviro400s and
  many more) `bus_doorfront0` opens the door and `bus_doorfront1` closes it, and Shift+1 fired
  both, so the door never opened. Door key triggers are now tried on the scripts first, and
  when they undo each other only the one that moves the door is fired.

### People
- Passengers get off double-deckers again: the once-a-second check for another open door ran
  every frame, pulling everyone back to the nearest path point, so people coming down from
  the upper deck stayed on the stairs. People held up face to face in the aisle or on the
  stairs squeeze past after two seconds.
- Passengers paying at the cash desk hold the money out to the tray instead of raising the
  arm up and forward: the arm's reach had its lift and turn the wrong way round.
- Standing passengers keep their feet on the ground at every tick, as Omsi.exe does, not at
  the height of their waiting place's object.
- A long bus station stop gets the waiting places of objects along its whole length.

### Performance
- People standing still are not skinned and uploaded again every frame (2.6 ms to 0.15 ms a
  frame for thirty waiting passengers).
- `OMSI_PROFILE` lists triangles per asset and names the stages of slow people ticks;
  `OMSI_SKIP_PIPE` and `OMSI_CHECK_GROUND` help measuring.

### Merged pull requests
- #661 shadow blobs lie on the road under a bridge, and can be switched off; #696 glass found
  by its texture's alpha (lamps show through every bus's windows); #699 "The game is running"
  in the launcher; #704, #687, #689, #675, #688 dedicated server administration (`tell`,
  weather by name, `/status` weather and clock, `POST /admin` from the same machine - refused
  through a tunnel or proxy); #706, #710 outside camera; #707 16x anisotropic filtering;
  #711 8x MSAA; #708 a Windows test; #714 changelog.

## 0.1.810 - 2026-10-02

### Launcher
- Buttons that contain only an icon now centre it correctly instead of leaving the spacing
  reserved for a missing label; this fixes the livery arrows on Drive > Bus and the mobile
  file browser's folder-up button (#678).

## 0.1.808 - 2026-10-02

### Traffic
- Traffic-light programs that use conditional backwards jumps to extend a phase no longer
  repeat the same jump indefinitely and get stuck on one signal combination. The extension
  is replayed once before the normal cycle continues (#692).

### VR
- VR has an adjustable cockpit navigator attached to the bus. It can be moved and rotated,
  its distance and size can be adjusted, and position, rotation, size, opacity and visibility
  are saved separately for each bus (#693).
- `Ctrl+Shift+M` enters navigator placement mode and `Ctrl+Shift+N` shows or hides it; both
  actions can be rebound. The same placement settings are available in the VR options menu
  (#693).
- The VR pause menu is smaller for a more comfortable fit (#693).

## 0.1.800 - 2026-10-02

### Server
- A dedicated server can optionally expose `GET /players` for live web maps. The JSON list
  contains each player's name, bus, line, destination, tour, position, heading and speed,
  and latitude/longitude on maps with `[worldcoordinates]` (#674).
- Player position sharing is off by default and must be enabled with `share_positions = 1`
  in `server.cfg`; otherwise `/players` returns 404 (#674).
- A player on foot is reported at the walker's position, and the walker is preferred over
  a parked bus when both exist (#674).

## 0.1.796 - 2026-10-01

### Graphics
- Enhanced graphics can reflect buses, buildings and scenery in wet-road puddles. Reflections
  follow the local road height, slope and camber, and nearby articulated bus sections are
  included (#686).
- Puddle reflections use bounded half-resolution screen-space rendering and are skipped on
  dry roads, full snow, mirror views, OpenGL and when reflections are disabled, limiting
  their cost when they are not needed (#686).

## 0.1.791 - 2026-10-01

### Controls
- DirectInput now finds generic controllers and button boxes with no axes, including custom
  Arduino, Pro Micro and STM32 devices, instead of silently leaving them disconnected.
  Keyboards, mice and screen pointers remain filtered out (#71, #662).
- Holding both opposite steering keys keeps the wheel at its current position, as in OMSI,
  instead of always giving the left key priority. Releasing either key immediately continues
  steering in the remaining direction (#663).
- `Toggle game controllers` follows its configured key binding instead of also having a
  hard-coded `K`. Leaving the action unbound now frees `K`, and rebinding it to another key
  works as expected (#649, #670).

## 0.1.785 - 2026-10-01

### Multiplayer
- In LAN multiplayer, a client's bus can take the host's waiting passengers again. They are
  claimed from the stop where the client's bus is listed and handed over as that client's
  waiting passengers.

### Project
- Repository, updater, website, documentation and release links now use the project's new
  `openOMSI-Project/openOMSI` home after the repository moved to the openOMSI-Project
  organization.

## 0.1.782 - 2026-10-01

### People
- Passengers are rewritten after Omsi.exe: the same tasks (waiting, the bus coming,
  walking to the bus, to a place, to the exit, sitting), waiting places along the stop's
  platform by its length and side, a bus taken from 60 m out and only when its terminus
  is one of theirs, the nearest open door (or one with a button), the cabin's path network
  walked by its own routing tables, a free place reserved at random (none free: they stay
  behind), the ticket stamped or bought at the desk with the game's dialogue, and everybody
  out at the terminus. The made-up queues, door waits and aisle shuffling are gone: people
  no longer stand at a door for long or crowd into the bus.
- Passengers and people on foot are posed and animated as in OMSI (its joint angles, gait
  curves, stride and stoop) instead of with leg IK. Pedestrians on the pavements stay as
  they were.
- Timetable buses no longer set off with a made-up number of riders by the hour.

## 0.1.780 - 2026-10-01

### Controls
- The left and right turn signals can be toggled by keys or wheel buttons (automatic
  cancelling still applies), and the driver's view can follow the steering with an
  adjustable angle and response (#657, #646).

## 0.1.774 - 2026-10-01

### Vehicles
- `GetTTTerminusIndex` answers the depot terminus named like the trip's terminus (or -1),
  as OMSI does: the IBIS's automatic destination showed a wrong or random terminus (#623,
  #545). Switching to free drive clears the bus's timetable values (#623).
- The cab's paper timetable is OMSI's: one Courier New text, the stop names cut or dotted to
  25 characters, 24 rows a column, one time a stop and the arrival/departure words of the
  game's language instead of German everywhere (#629).

### Controls
- The combined throttle/brake axis puts the throttle on the right half, as OMSI does (#577).
- A vJoy device no longer crashes the game when vibration starts (#655).

### People
- People on foot step only onto surfaces up to 0.5 m higher, as in OMSI, and no longer onto
  low roofs or benches (#600).
- Busy stops no longer drop the frame rate (#656).

### Traffic
- A map's own AI cars load when their `.ovh` leaves the registration affixes out (#644).

## 0.1.759 - 2026-10-01

### Traffic
- AI vehicles switch their lights on below OMSI's light value of 0.75, before the street
  lamps, and keep them on after the lamps go out (#620).
- The street lamps come on below a light value of 0.6 as in OMSI, and each scenery object's
  `NightlightA` at its own threshold (a random 0.3-0.75 with `[NightMapMode]`) (#620).
- A timetable bus at the end of its tour's last trip leaves the road at once instead of
  driving on and queueing at the map's end (#598, #536).
- The amount of street traffic follows the paths' `trafficdensity`; density-0 and no-car
  paths stay empty (#625, #161).
- Fewer stutters when AI vehicles appear (#641).

### Controls
- Looking round in a view of the bus follows the cursor and scales with the field of view, as
  in OMSI (#622, #398); the cursor shows up-down arrows while zooming (#621).
- Force feedback direction is detected and saved per wheel (#637).

### Graphics
- The scene shader compiles on OpenGL and GLES again (black screen and crashes on some
  Android and Linux machines, #617, #610, #556, #456).
- A model takes the ambient light with OMSI's white material ambient (or its
  `[matl_allcolor]`), not its diffuse colour (#530).
- Coloured lamp sprites are blended as OMSI blends them and keep their colour (#601).

### Interface
- IBIS picks the route by the trip's stops (#631); the information bar (Ctrl+Y) shows the
  passengers aboard (#628).

### Multiplayer
- A server seeds bus-stop passengers near every player (#626).

## 0.1.737 - 2026-10-01

### Sound
- A triggered sound keeps its loudest volume while it plays, as in OMSI: door sounds are no
  longer cut off when the door stops moving (#473, #611, #542).

### Passengers
- A passenger's stop request fires `int_haltewunsch` as in OMSI, not the cab's stop button
  triggers, which played the driver's switch and brake sounds on some buses (#569).

### Traffic
- The map's path rules `bus` and `trucks` open a path to AI buses and trucks by their
  `ai_veh_type`, as in OMSI: trucks no longer avoid exactly the roads marked for them, and
  bus roads are no longer closed to cars (#612).

### Graphics
- Every camera clips at 0.1 m as in OMSI: a wide field of view no longer cuts into the cab
  (#587).
- A light's sprite keeps its own colours, tinted by the light's colour (#601).
- `STLoadTex`/`STNewTex` size a script texture as D3DX does (the bitmap's power of two,
  stretched), and a resized one no longer keeps the old picture's memory (#607).
- A weather chosen from the menu comes at once, as in OMSI, and the clouds drift on smoothly
  while a weather blends in (#609).

### Controls
- Gamepad triggers on Windows are no longer read as buttons, and the D-pad works (#602).

### On foot
- The ground is the floor at the walker's level, not a stop's roof over it (#600).

### Launcher
- No "missing pack" for a part the game finds from the vehicle's own folders (#582).
- The pause notice appears once and is translated (#616).

## 0.1.721 - 2026-10-01

Pull requests from the community: #455, #560, #563, #566, #570, #580, #583, #585, #588,
#591, #592, #594, #597, #599, #604, #605, #608.

### Traffic
- AI cars no longer wait at junctions without traffic lights for no reason (#591).
- AI buses at a stop on the left open their left doors (#592).

### Controls
- Force feedback fixes and improvements (#585); manual gear buttons can be held, letting go
  returns to neutral (#570).

### Launcher
- Mods can be imported from .7z and RAR archives as well (#570).
- Better vehicle selection with a livery preview (#583); the fleet number is set before the
  bus starts up (#560).
- The launcher gives its graphics device up while a game runs (#599).

### Interface
- Interior and exterior temperatures in the HUD and the navigator (#605), stop requests on the
  minimap (#604), better roads and stop markers in the navigator (#563).
- Route numbers that start with a letter (X10, M48, N9) on the destination display (#588).

### Graphics
- Scripted LED traffic lamps get their materials, and a same-name DDS texture is preferred
  (#566); `[matl_freetex]` paths relative to the OMSI folder work (#580).
- VR: the mirrors can refresh faster, up to every frame (#608).

### Passengers
- A map's `humans.txt` may name people in nested pack folders (#455).

### Fixes
- On foot: walking stays on the bus's right level (#594); a dedicated server updates the
  passenger density and the time of day every tick (#597).

## 0.1.623 - 2026-10-01

### Passengers
- Passengers see the doors open on buses whose script sets `PAX_Entry`/`PAX_Exit` without
  listing them in its varlist (stock MAN NL, MAN NLC, many Citaros), and on 3-door buses the
  middle and rear doors let people out again; `door<i>` without the underscore works too
  (#532, #535, #512, #537, #179, from #551).
- They walk to the nearest open door or one with a request button, as OMSI does, and switch
  when the driver opens a nearer one, instead of all crowding the front door (#535).
- Waiting at a door that is still shut, they press the request and stay, and the bus
  creeping a metre or two to line up no longer sends them back to the shelter (#532, #512).

### Controls
- With the default keys plain Left/Right switch the interior camera as in OMSI (A/D steer;
  the "Arrow keys only" preset still steers with them), and the camera keys go on into the
  rear section's driver cameras of an articulated bus (#519, #525, #464).
- The viewpoint key steps through the driver, passenger, outside and map views as in OMSI
  (#519).
- Wheels: the scripts' shaking (`FF_Vib_Amp`, `FF_Vib_Period`) plays as a real periodic
  force-feedback effect (#501).
- Head tracking on Windows also reads opentrack's freetrack output (TrackIR protocol) (#522).

### Launcher
- The fleet number can be picked from the bus's number list before driving (also in a duty
  and with `--number`), as in OMSI's vehicle dialog (#538, #133).

### Graphics
- A `[matl_lightmap]` on a variable the bus does not have is always lit, as in OMSI, not
  always dark (dashboard lamps, #475, #352, #231).
- Painted ground no longer darkens through stacked layers at shallow angles, and painted
  edges no longer shift with the texture's mip level (#552).

### Maps
- `[surface]` objects stand on the terrain like every other object without `[absheight]`,
  as in OMSI (#531, #453, #460, #417).
- Stairs and excavations of presurface objects are no longer filled by the terrain (#526),
  and declared `[terrainhole]`s cut railway cuttings and underpasses however deep (#550).
- Terrain-mapped textures of a map's own folder win over global seasonal copies (#548).
- On a Windows with a Chinese, Japanese or Korean code page, content text is read in that
  code page, as OMSI does.

### Traffic
- Random AI cars at a dead end or the map edge leave at once, as in OMSI, instead of standing
  there and blocking the cars behind (#536, #327).

## 0.1.592 - 2026-10-01

### Passengers
- People getting off wait at the exit they chose when they pressed the stop button, as in
  OMSI, instead of running to whichever door opens first, usually the front one (#493, #336).
- As many people wait at a stop as the map's passenger counts for it and the passengers
  setting give, up to its waiting places, as in OMSI. Before, never more than seven: 200%
  changed nothing (#458, from #513).

### Controls
- Game controller axes use their characteristic from gamectrler.cfg (progressive,
  degressive, bi-..., range extension), as in OMSI; the launcher's controller page sets it.
  Before, a G25 set to bi-progressive steered linearly (#479).
- Plain Left/Right switch the interior camera again unless a wheel steers (#464, #519).
- The hints for a bus that does not move name the player's own keys (#461).
- P pauses and resumes without opening the menu; a LAN session cannot be paused (#527).

### Graphics
- Sun shadows stay until the sun is about a degree below the horizon, as in OMSI (they went
  at 4.6°), and overcast weather or fog under 350 m casts none (#518, #506).
- Route arrows cast no shadow (#508).
- Scenery text textures (street signs) are lit like their object and no longer glow at night
  (#470).

### Fixes
- A crash in the interface's texture cleanup (#534).

## 0.1.576 - 2026-10-01

### Passengers
- People no longer freeze at an open door before boarding (#498).
- Buses whose scripts set `PAX_Entry0_Open` for the front door only (the SD200, SD202) let
  people out of the rear doors again: an exit without a `PAX_Exit` variable follows its
  `door_<i>` (#486).

### Controls
- Steering wheels (G27, G29, Driving Force GT...): the pedals no longer act as buttons and
  buttons no longer fire twice (#480).
- The weather's METAR airport can be typed as an ICAO code (#492).

### Interface
- In-game interface size and opacity, and the notes in the corner, are settings; sharper
  text; the pause menu's Options list one line a setting (#446).
- The in-cab schedule sheet shows the player's timetable (#491).
- AI-only lines are no longer offered for driving (#483).

### Graphics
- LED panels: a "LED mip strength" slider (0-4) instead of the on/off switch (#490).
- Dashboard lamps built as `[visible]` meshes with a plain `[matl_nightmap]` light up by
  day as well, as in OMSI (#507).
- `[terrainmapping]` takes the map's first ground texture only, as Omsi.exe does; ground
  lighting and tile seams fixed (#436).
- VR: the bus mirrors are no longer black when looking round (#487).
- The driver figure's arms and the hand on a manual gear lever (#465).

### Maps
- Surface objects modelled away from their origin (bridges) stand at the road's height
  (#496); scenery `[matl_freetex]` works without a script (#474).

### Traffic
- AI cyclists ride at 15-21 km/h and no longer show green boxes on the rider (#502).

## 0.1.538 - 2026-10-01

### Graphics
- Enhanced: flipdot destination displays (the stock MAN NL/NG Krueger matrix, the flipdot
  variants of the NEOMAN and Sprinter mods), whose light map is a picture of their dots, no
  longer glow and bloom like LED panels. LED panels (a plain white light map) still glow,
  and go dark when the bus's power or lights are switched off (#413).
- Enhanced: the mirrors are no longer far darker than the view through the windscreen at
  dusk and in daytime rain; they dim only once the sun is below the horizon (#432).
- The driver figure is lit by the four interior lamps of its seat, as OMSI lights a seated
  person - coloured, by distance and from the lamp's side - instead of a flat glow of every
  lamp in the bus that left it overexposed (#206).
- `[matl_texadress_mirror]` and `[matl_texadress_mirroronce]` mirror the textures beyond
  their edges as in OMSI, instead of smearing the edge (#145).
- A TGA picture under a `.png`, `.bmp` or `.jpg` name is read by its content, as OMSI does;
  it used to leave the slot without a texture (#414).

### Maps
- A tile has water only when its `.map` says `[water]`: a leftover `.water` file no longer
  floods a tile whose water was removed (#414).

### Controls
- New setting "Dynamic steering" (Driving tab), OMSI's `redSteerSpd`: the steering keys and
  the wheel's return to centre slow down with speed (#347).
- Camera tab: "Right mouse button turns the view" switched off gives OMSI's own mouse: the
  right button zooms (up widens the cab view, backs the outside camera away) and the wheel
  button turns the view. On (the default) right-drag looks round as before and Shift+right
  zooms (#398, #104).

### Sound
- Passenger footsteps in a bus use the step sounds its `paths.cfg` gives each walkway: on the
  SD200 the stairs sound like stairs and the upper deck has its own floor. Before, every
  step picked from all of OMSI's floor samples (#311).
- The bus's outside sounds (the SD200's exterior engine) are heard from the cab through an
  open door or window, at the level the bus script gives (`Snd_OutsideVol`), as in OMSI.
- The cab no longer plays a made-up low engine rumble under the bus's own sounds: OMSI makes
  no vehicle sound of its own.

### People
- `[walk_param]` is read as OMSI reads it (stride, then arm angle), and people walk at OMSI's
  1.1 m/s ± 0.2: the stride was taken for a speed (#393).

### Vehicles
- Script callbacks round their arguments to the nearest whole number as OMSI does, instead of
  cutting them off: a terminus code of 1100.9999 out of a script's arithmetic now finds
  terminus 1101, not 1100 (#312, #197).

## 0.1.523 - 2026-10-01

### Controls
- In mouse steering the cursor sets the pedals every frame, as in OMSI: a brake held on the
  keyboard (with "The keyboard brake stays on until the throttle") no longer stays on under
  the mouse (#395).
- Letting go of a key fires only that key's own `_off` trigger, as in Omsi.exe. On some mods
  it fired an alias's release (e.g. `parking_brake_mouse_off`, `kw_blinker_*_off`), which
  undid the parking brake or the turn signal just set with the keyboard (#420).
- The hand cursor appears over every clickable part of the cockpit, including large ones
  such as door leaves, the cockpit door and the steering column (#411).

### Vehicles
- Articulated buses: the rear section stops at the joint's maximum angle from
  `[coupling_front_character]` (52.5° on the GN92) instead of folding through the front
  section when turning tightly or reversing (#410).
- The odometer starts at a used bus's reading, as in OMSI: from the year and the km a year
  in `[kmcounter_init]` (1980 and 60000 when a bus has none), ±20% from bus to bus. Before,
  such buses showed 000000; reversing now also takes the counter back (#305).
- Number plates: a repaint's own `[registration_list]` plate is used for AI buses whatever
  the template says afterwards; the player's bus takes it as the vehicle dialog does. Before,
  they showed the automatic prefix and the fleet number (#133).
- The passenger cameras of an articulated bus's rear section hang on its body like the
  front section's: `[add_camera_pax]` distance, pitch and roll (#174, #126).
- `[matl_change]`: the first `[matl_item]` is made of its own block only, no longer of the
  later items' maps (an LED matrix showed its script texture at value 1) (#210).
- A `[matl_freetex]` declared inside a `[matl_item]` changes the powered material only, and
  named material references in `.x` models are resolved (from #436).

### Maps
- Crossings draped over the ground (`[crossing_heightdeformation]`) get normals rebuilt from
  their faces, as Omsi.exe does: some junctions looked as if their faces were turned
  inside out (#428).

### Sound
- `[important]` sounds keep their place when more sounds play than OMSI's
  `[sound_maxcount]` of 200 (#434).

### Graphics
- Mirrors can be switched off (Settings, Graphics, Mirrors: Off) on slower machines (#433).

### Multiplayer
- Numpad ÷ opens the front door again instead of the chat line (#130).

### Modding
- HTML pages: `omsi.getDepartures(stop)` lists the next departures of a stop, and
  `omsi.setNextStop(index)` can also go back to an earlier stop (#437).

### Android
- The content folders and the OMSI 2 installation get a `.nomedia` file, so the gallery no
  longer lists thousands of textures as photos - the media scan kept phones busy and people
  deleted the "pictures", leaving buses white (#443).

## 0.1.486 - 2026-10-01

### Launcher
- The Settings page is split into six tabs - Graphics, Driving, Camera, Sound, Gameplay,
  General - each fitting the window, instead of about ninety controls in three long
  columns; on a phone too (#430).

## 0.1.483 - 2026-10-01

### Graphics
- PBR maps (`_nn`, `_rr`, `_ao`, ...) work on map textures: roads, splines and scenery
  objects were drawn flat - only the textures loaded on the spot got their maps, not the
  ones a tile's preparation brought, which are nearly all of a map's. Maps put into the
  same folder in the openOMSI content folder, beside a texture of the OMSI install, are
  found too.

## 0.1.481 - 2026-10-01

### Controls
- Holding both mouse buttons and moving the mouse zooms as in OMSI (its "M_Zoom"): up
  zooms in on the dashboard, down back out to the seat's view; outside, the camera moves
  further away or closer. It did nothing - the right button only looked round.
- Pressing the right button while a switch or lever is held with the left one no longer
  turns the view and stops the drag.

## 0.1.479 - 2026-10-01

### Maps
- Objects placed along a spline that follow its slope and cant (railings, posts, signs,
  lights) turn within the spline's inclined surface: one turned sideways to the road no
  longer leans across it and off the ground on a sloped or canted street, and one on a
  chain running backwards no longer tips downhill (from #409).

### Android
- No "fs_blur" shader error on OpenGL devices: the ambient occlusion pipelines, which
  OpenGL ES cannot compile, are left out there - AO stays available on Vulkan (#422).

## 0.1.476 - 2026-10-01

### Controls
- Mouse steering sensitivity can be set in the pause menu's Options (Mouse steering
  sensitivity + / -, 10% to 300%; 100% is OMSI's), and the launcher's slider goes as far.
- The log says how each game controller came in (its layout, DirectInput or the system's)
  and, the first time a stick or axis moves, whether it steers - for reports of sticks
  that do nothing.

### Windows
- The game and the dedicated server start on a PC without the Visual C++ Redistributable:
  its runtime DLLs ship beside `openomsi.exe` ("The code execution cannot proceed because
  VCRUNTIME140_1.dll was not found").

## 0.1.471 - 2026-10-01

### Physics
- Buses no longer fall through the road at junctions over a buried embankment slope
  (Cotterell, the junction by the park): a road face lying more than a metre under the
  drawn ground beneath the wheel is no road there, and the bus stands on the ground, as
  with Omsi.exe's highest-face query (#424, #423).
- The wheels roll on the road where it is drawn: splines and `[surface]` objects are drawn
  8 cm over their authored height and the bus now stands on them there, not 8 cm into
  the asphalt (#421).

### Maps
- An object stored in a neighbouring tile's file, past its own tile's edge, stands on the
  ground under it instead of the height at its tile's border (#421).

## 0.1.463 - 2026-10-01

### Passengers
- With the door release on, the SD200's automatic rear door no longer opens on its own
  while riders board at the front: riders no longer press the outside door opener, which
  Omsi.exe never does - they only ask for a door through `PAX_Entry<n>_Req` (#416, #415).

### Project
- Pull requests get a template, and ones opened from a `main` branch or an organisation
  account are closed with a note (#418).

## 0.1.455 - 2026-10-01

### Traffic
- Emergency vehicles have right of way at crossings: a vehicle whose script sets
  `TrafficPriority` goes before the others and they give way to it, as Omsi.exe does for
  any vehicle, not only the player's bus (#356).
- Timetable buses stop at the stop on their own side of the road: on a route back along
  the same street, they stopped at the stop across the road on the way out. Stops are
  matched in the trip's order, on the lane they stand beside.

### Graphics
- Enhanced: the far road and ground no longer go dark at grazing angles - what reflects
  nothing keeps its light, and a wet road reflects the sky (#374).

### Graphics cards
- Cards of up to 4 GB use the allocator's small memory blocks, and a texture budget larger
  than the card holds is taken down to its size: 2 GB cards lost their device to "out of
  memory" in the first frames (#332, #295, #323).

## 0.1.451 - 2026-10-01

### Vehicles
- Wheels stay under their hub caps: the drawn tyres are seated on the physical hub at the
  point they turn about (the rotation's `origin_trans`), not at the .o3d's own pivot. A tyre
  without a pivot (the NEOMAN's right front) was measured at a point circling the hub and
  moved up and down by centimetres as it turned, so its cap seemed to roll off it.

### View
- Smooth camera transitions when changing and entering views, a setting in the pause menu
  as well (#408, by shloooo).

### Graphics (Vanilla, at night)
- The map's lamps leave `[tree]`s dark, as in OMSI 2 (#407, by Sulamufor).
- The terrain's light map lights the ground instead of glowing over it (#406, by Sulamufor).

## 0.1.438 - 2026-10-01

### View
- The driver's hands in the cab view are a setting now (Settings and the pause Options,
  "Driver's hands in the cab view"), off by default.

### Graphics
- Vanilla: reflections blend in gamma like the rest of the classic picture; at night the
  MAN NL/NG instrument glass no longer lies milky white over the unlit gauges (#401, by
  Sulamufor).

## 0.1.434 - 2026-10-01

### Driving
- An automatic gearbox is no longer taken for a manual one. A bus counted as manual when
  its scripts answered to the gate keys (`kw_s_1`, `kw_s_2`) and read a `Clutch` anywhere -
  many automatics do both (gear hold keys, a torque converter's own clutch) - and the
  automatic clutch of the settings then worked their clutch at every stop and pull-away,
  and the phone showed a manual's gate. Now a gearbox is manual when it has the gates and
  no automatic's `automatic_D`, or when its first gate itself asks for the clutch, or when
  it works a clutch of its own through `AutoClutch` (checked on the LiAZ MKPP/GMP, the
  Sprinter G32/G-tronic, the SD202 and the NEOMAN A23).

## 0.1.433 - 2026-10-01

### Graphics
- Enhanced graphics are there again on every device and graphics API (0.1.402 left them
  out on phones and OpenGL). They are built whenever Enhanced is chosen; only a phone or
  OpenGL device not set to Enhanced skips compiling them, as it never draws them - that
  compile is what killed Mali and Adreno drivers at the start.

### Driving and view
- Scripts: a trigger starts with 1 on its stack, as in OMSI - a trigger guarded by a bare
  `{if}` did nothing (the S315 UL-GT's ticket printer switch) (#388, by hannsadrian).
- The interior camera glides between viewpoints (OMSI's `driverview_smooth`, a setting)
  (#388).
- The driver's hands are seen in the cab view, the rest of the figure folded away (#376, by
  Neblina666).

### Displays and translations
- The Atron ticket machine shows its stop text and keeps its sales screen (#390, by
  TruckiHD).
- Brazilian Portuguese improved, European Portuguese added (#396, by isaacsa2).

## 0.1.402 - 2026-09-30

### Graphics
- Rain on the windows: drops no longer run down in lanes of wavy lines all at once. Now and
  then a single drop breaks loose, slides a few centimetres to a hand's width in jerks,
  nearly straight with a little drift, and stops again - each at its own moment.

### Vehicles (compared with Omsi.exe)
- A `[matl_change]` with several `[matl_item]`s shows item n at value n, as Omsi.exe does:
  only the first was kept and shown at 1, so e.g. the MAN New Lion's City's door buttons
  (2 = lit while the door is open) stayed dark (#352).
- `[animparent]` hangs a mesh on the last mesh before it that carries the name, as Omsi.exe
  resolves it while reading: door variants reusing their arms' names moved the later
  variant's leaves with the first one's arm (Solaris Urbino III "Bode new", #348).
- Station displays and other scenery text show a string that arrives after their first
  frame (they were only redrawn on `Refresh_Strings`, #367).

### Driving
- A throttle pedal takes off a brake the keyboard holds, as the throttle key does: the
  bus was driven against its brakes (#377).

### Traffic
- Cars change only onto lanes open to their own traffic group: trucks no longer take the
  cycle paths beside a road (#327); trucks keep to the speed limit (up to 80-90 km/h, it was
  38-47 on every road), bicycles ride at 20-28 km/h.
- In multiplayer the host has traffic round itself again (#342).

### Phones and crash reports
- Phones and OpenGL leave out the Enhanced graphics' pipelines: they are never drawn there,
  and compiling the ray-marched clouds' sky killed Mali and Adreno drivers before the first
  frame (#364, #333, #316, #371).
- A crash report takes its title from the run itself: an error before the game started, or
  one the game got over, titled reports of games that died much later (#381, #331).

## 0.1.400 - 2026-09-30

### Performance
- Macs and phones: a fixed render scale keeps to the same pixel budget as the automatic
  one on high-resolution screens, the navigator draws without multisampling, the picture's
  depth is not written back where nothing reads it, and two frames are in flight so the
  graphics chip works while the next frame is prepared (M1 Pro, Thüringer Wald: 20 to 57
  fps; one frame more of input delay with V-sync) (#385, by hannsadrian).
- Linux builds link with lld (#384, by no-felix).

## 0.1.394 - 2026-09-30

### Graphics
- Rain drops on the windows are smaller again (the big ones of 0.1.381 were far too big and
  lumpy), and a drop running down leaves only a cleared track and a few beads - no more
  thin tail drawn behind it.

### Driving
- A clutch pedal pushed to the floor is fully in: a wheel's pedal reads 0.93-0.99 there, and
  the LiAZ/PAZ gearboxes part the engine from the wheels only above 0.95 and take a gear only
  at 1 - holding the clutch at a stop still stalled the engine. All pedals' last few per
  cent count as their ends; the phone's clutch is in from three quarters down.

### Passengers
- Riders who stood up as the bus pulled in get off at their stop: they lost "this is my
  stop" as the bus came to a stand and stayed at the door (#336).
- Riders of timetable buses no longer lose their stop whenever the player's bus stops
  somewhere (#317); somebody held off the exit by a pole gets off from there.

### Controllers and launcher
- Controller buttons can pause, take a screenshot, quicksave and switch mouse steering or the
  controllers (#380, by isaacsa2); DirectInput devices with unusual layouts (button boxes
  without axes) are taken (#379, by isaacsa2).
- Text fields in the launcher can be clicked into, selected and overwritten (#373, by XiZyno).
- The dedicated server starts on machines without a graphics card again (#375, by no-felix;
  #368).
- Translations completed and corrected, Traditional Chinese in full (#383, by EFour4).

## 0.1.381 - 2026-09-30

### Graphics
- Rain on the bus windows looks like real drops, in all three graphics modes (Vanilla had
  OMSI's sliding texture until now):
  - every drop is a lens: it shows the street behind the glass through itself, small and
    upside down, with the sky at its bottom (from the last frame's picture);
  - drops come in four sizes up to over a centimetre, with uneven rims, the heavy ones
    drawn out downwards, and clear glass between them;
  - a runner leaves a thin stream of water in its track;
  - while the bus drives, the airstream takes the runners up and out across the windscreen
    and back along the side windows, harder the faster it goes.
  It costs about half a millisecond at 1080p.

## 0.1.380 - 2026-09-30

### Game menu
- The pause menu's Options are kept: most switches (collisions, camera collisions, view
  turning with the steering, force feedback, keyboard brake hold, automatic clutch, head
  tracking) were written in a form the settings file read as "not set" and came back at
  their defaults, and LED glow and LED mipmaps were not saved at all.
- The launcher takes over the settings a game changed instead of writing its older copy back
  over them when it saved something of its own.

### Passengers
- Somebody at the front of a queue whom a railing, pole or shelter wall holds off the door
  boards from where they stand; they stood a metre from the open door until the bus left.

## 0.1.378 - 2026-09-30

### Performance
- Busy maps run much faster: the frame's render preparation (culling, shadow casters, draw
  lists, bundle recording) is spread over the render threads instead of one core, mirrors
  are drawn only when in view and at most 30 times a second, the navigator's map is redrawn
  at most 30 times a second, and the cab's hover pick tests only the triangles near the
  cursor. On St-Servan with traffic and passengers: 37 to 60 fps (#369, by ThiBot77).

### Website
- Download: the "Your system" badge no longer breaks across the card title, and the
  Download buttons line up.

## 0.1.374 - 2026-09-30

### Graphics
- Vanilla lights textures through the sRGB curve instead of a plain power of 2.2, so dark
  colours are no longer crushed (#343, by Sulamufor).
- Small dashboard indicator lights are drawn again instead of being dropped as too small
  (#346, by no-felix).

### Driving
- Turning the wheel can turn the driver's view into the bend, as in OMSI's "look with the
  steering wheel" (#363, by shloooo).
- AI emergency vehicles sound their siren when something holds them up (#357, by Sulamufor).
- System gamepads on Windows can have their buttons bound again (#358, by EpixIXIx).

### Game and launcher
- The game menu's scrollbar can be dragged (#354, by XiZyno).
- When the game was closed by the system (out of memory), the launcher says so and points
  to the settings that help (#355, by no-felix).
- A saved situation keeps each vehicle's livery; read-only content folders fall back to a
  writable place (#365, by no-felix).

### Multiplayer
- LAN protocol 6: a vehicle's state carries up to 63 values, the rear section's sounds and
  what is seen first come first, and INFO messages are sent at most four times a second
  (#353, with the updated #338 and #334, by Jaja80330). Players and servers need this
  version together.

### Displays
- A character a font lacks is left out, as Omsi.exe does (its glyph lookup gives none),
  instead of being drawn as the font's first glyph; spaces keep their width. The MAN Lion's
  City's odometer and trip meter lose the `|` in front of them (#370, by no-felix; #360).

### Website
- Download: the Windows button downloaded the dedicated server (its zip ends the same way).
  Every build now has its own button - Windows and Windows on ARM, macOS for Apple silicon
  and Intel, Linux and Linux on ARM, Android - the one for your system first, and the four
  dedicated servers apart below. Installing on macOS is explained too.

## 0.1.371 - 2026-09-30

### Website
- Every build has its own download button instead of matching files only by the end of their
  names. Windows ARM64, macOS Intel, Linux ARM64 and the dedicated-server builds are listed,
  the visitor's platform is shown first, and macOS installation is explained.

## 0.1.344 - 2026-09-30

### Driving
- Braking and pulling away pitch the bus as much as in OMSI (the tyres' forces act about
  the hubs), and it no longer pitches at a standstill.
- Turning the wheel no longer kicks the body into a roll: while the tyres hold, the bus
  leans only by the bend's pull, as Omsi.exe does.
- A wheel in the air stays where it hangs at rest (`Axle_Suspension`), as in OMSI.

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

## 0.1.182 - 2026-09-30

### Website
- Ko-fi was added alongside Buy Me a Coffee, and the site's corner Donate button opens a menu
  with both options.

## 0.1.180 - 2026-09-30

### Multiplayer
- Joining a server by code starts on the host's map even when the server welcome arrives after
  loading has begun; maps not installed locally can still come with the host's mods.

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

## 0.1.175 - 2026-09-30

### Passengers
- Waiting passengers decide individually whether to watch an approaching bus, and once it
  stops only the people who intend to board it keep looking at it.

## 0.1.174 - 2026-09-30

### Trains and multiplayer
- Train and coupled-vehicle consists are assembled in OMSI's order, including reversed cars
  and couplings from every unit. The same orientation rules apply to the player, AI and LAN
  vehicles.

## 0.1.173 - 2026-09-30

### AI traffic
- A car that has waited a long time at a junction keeps its turn instead of being starved by
  every new arrival.
- Two cars that end up treating each other as their lead no longer wait for each other
  forever.
- A car committed to changing lanes is no longer held back by parked cars in the lane it is
  leaving.

## 0.1.171 - 2026-09-30

### Performance
- Main-view culling buffers are reused between frames, reducing allocation and camera-motion
  overhead without changing visibility or LOD decisions.
  [#175](https://github.com/openOMSI-Project/openOMSI/pull/175)

## 0.1.160 - 2026-09-30

### Roads and splines
- A spline type's half cant width is derived from its height profiles, as Omsi.exe does;
  splines without height profiles have no cant.

## 0.1.146 - 2026-09-29

### Scenery
- Scripted advertisements can be rendered on scenery objects.
  [#147](https://github.com/openOMSI-Project/openOMSI/pull/147)

## 0.1.144 - 2026-09-29

### Controllers
- Linux wheels get force feedback and stable button numbering.
  [#146](https://github.com/openOMSI-Project/openOMSI/pull/146)

## 0.1.139 - 2026-09-29

### Mouse steering
- A right click can end mouse steering again.
  [#162](https://github.com/openOMSI-Project/openOMSI/issues/162)

## 0.1.138 - 2026-09-29

### Diagnostics
- `OMSI_DEBUG_TRAILERS` logs coupled parts whose level does not match the vehicle pulling
  them.

## 0.1.136 - 2026-09-29

### Graphics
- Model-order vehicle drawing is off by default again after it exposed saloons through body
  panels on several buses; `OMSI_MODEL_ORDER=1` keeps the ordered path available for tests.

## 0.1.135 - 2026-09-29

### Scripting
- `$cutBegin`, `$cutEnd` and `$SetLength` round their counts like Omsi.exe, and `$*`
  repeats a pattern to fill the requested length.

## 0.1.133 - 2026-09-29

### Passengers
- Waiting passengers stop at the pavement instead of stepping into traffic, and people under
  shelters stand on the floor at their own height rather than on the roof above them.

## 0.1.132 - 2026-09-29

### Passengers
- Passengers choose buses from the stops and termini served by the timetable, as Omsi.exe
  does, instead of using the old random "take the next bus" rule.

## 0.1.131 - 2026-09-29

### Text and displays
- Text textures follow Omsi.exe's font rules for missing glyphs, clipping and glyph spacing,
  fixing destination words that could run together or be squeezed into a texture.

## 0.1.129 - 2026-09-29

### Graphics
- Vehicle materials follow Omsi.exe's fixed-function texture stages more closely, including
  environment maps, transmaps and model-order blend/depth behaviour.

## 0.1.126 - 2026-09-29

### AI traffic
- Cars no longer brake for the player's bus merely following behind them.
  [#139](https://github.com/openOMSI-Project/openOMSI/issues/139)

## 0.1.121 - 2026-09-29

### Diagnostics
- `OMSI_JOINT_ANGLE` can hold an articulated bus's rear section at a chosen angle in
  offscreen tests, making joints and bellows easier to inspect.

## 0.1.119 - 2026-09-29

### Passengers
- Passengers meeting an arriving bus step only toward the kerb and stay on the pavement
  instead of standing out in the road.
  [#123](https://github.com/openOMSI-Project/openOMSI/issues/123)

## 0.1.117 - 2026-09-29

### Graphics
- Bus `[matl_alpha]` modes are respected again, so alpha-tested cut-outs and overlay layers
  are not forced opaque by the body-depth repair.
  [#127](https://github.com/openOMSI-Project/openOMSI/issues/127)

## 0.1.116 - 2026-09-29

### Camera
- The outside-view zoom changes in steps again instead of compounding every frame and jumping
  straight to its minimum or maximum.
  [#126](https://github.com/openOMSI-Project/openOMSI/issues/126)

## 0.1.110 - 2026-09-29

### Maps
- Objects attached to splines are placed at their stored chain offsets.
  [#124](https://github.com/openOMSI-Project/openOMSI/pull/124)

## 0.1.101 - 2026-09-29

### Graphics and startup
- The game and launcher try the requested graphics backend and then other available Vulkan,
  DirectX 12 and OpenGL adapters instead of giving up on the first device that cannot open.
- Windows asks switchable-graphics drivers for the high-performance GPU, and a complete
  failure now reports an error cleanly instead of panicking.
  [#97](https://github.com/openOMSI-Project/openOMSI/issues/97)

## 0.1.99 - 2026-09-29

### Localisation
- English interface strings remain the lookup keys, so renamed text does not silently lose
  every translation; the Hungarian improvements are kept.

## 0.1.96 - 2026-09-29

### Performance
- Main-thread frame time is reduced on busy maps.
  [#119](https://github.com/openOMSI-Project/openOMSI/issues/119)

## 0.1.95 - 2026-09-29

### Graphics
- Graphics cards with about 2 GB of VRAM give textures a smaller share of memory, avoiding
  device loss on maps such as Grundorf with MSAA, SSAO and shadows enabled.
  [#114](https://github.com/openOMSI-Project/openOMSI/issues/114)

## 0.1.92 - 2026-09-29

### Maps
- Map pitch and bank are converted into the world's rotation convention correctly, fixing
  scenery such as Thüringer Wald rock faces.
  [#117](https://github.com/openOMSI-Project/openOMSI/issues/117)

## 0.1.91 - 2026-09-29

### Materials
- When a material slot has several `[matl_change]` records, the item is shown while any of
  their variables is active, matching Omsi.exe.
  [#112](https://github.com/openOMSI-Project/openOMSI/issues/112)

## 0.1.87 - 2026-09-29

### Controllers
- DirectInput now logs why a force-feedback wheel could not get exclusive access or create its
  constant-force effect instead of silently running without forces.
  [#99](https://github.com/openOMSI-Project/openOMSI/issues/99)

## 0.1.84 - 2026-09-29

### Maps and graphics
- Only splines whose profiles are entirely overhead are excluded from the ground surface;
  walls, embankments and watersides still shape the ground.
- Mesh files are looked up beside their model first, and the vehicle body-depth repair keeps
  applying where it is needed.

## 0.1.79 - 2026-09-29

### Materials
- When several plain `[matl]` blocks address the same slot, a later `[matl_alpha]` can set
  the slot's alpha mode, matching OMSI's material handling.
  [#95](https://github.com/openOMSI-Project/openOMSI/issues/95)

## 0.1.78 - 2026-09-29

### Build
- `Cargo.lock` now records the Windows dependency used by `omsi-render` for DXGI.

## 0.1.68 - 2026-09-29

### Controllers
- Controllers plugged in while the game is running are detected again without periodic
  polling stutter, and DirectInput device names with non-Latin letters match their
  `gamectrler.cfg` entries.
  [#71](https://github.com/openOMSI-Project/openOMSI/issues/71)

## 0.1.60 - 2026-09-29

### Documentation
- The README links the modding guide, and the build guide documents the Android toolchain and
  build command.
  [#66](https://github.com/openOMSI-Project/openOMSI/issues/66)

## 0.1.59 - 2026-09-29

### Documentation
- The format documentation now covers model LOD selection, transmap alpha, entry-point records
  and map tile numbering.

## 0.1.57 - 2026-09-29

### AI traffic
- A car stuck at a dead end leaves sooner when a queue is building behind it.

## 0.1.56 - 2026-09-29

### AI traffic and graphics
- UK AI car packs are visible again, their paint/transmap layers render correctly, and object
  LOD selection follows OMSI's model order.

## 0.1.55 - 2026-09-29

### Physics
- Stacked road markings are ignored as extra steps by the wheel-ground query.

## 0.1.54 - 2026-09-29

### Maps
- Tile numbers include repeated `[map]` entries, as in OMSI, and entry points are resolved on
  their own tile instead of a different tile with the same object id.

## 0.1.53 - 2026-09-29

### Audio
- Sound follows the system output device when headphones or a Bluetooth headset are connected
  or disconnected while the game is running.

## 0.1.52 - 2026-09-29

### Vehicles and maps
- A bus is put down on the surface its wheels should stand on near the entry point's height,
  rather than on the highest nearby surface such as a wall top or bridge deck.

## 0.1.51 - 2026-09-29

### Passengers
- Seated passengers on high seats let their feet hang naturally instead of stretching their
  legs straight down to the floor.

## 0.1.48 - 2026-09-29

### Passengers
- People keep to the floor at their own height instead of being placed on a roof or deck above
  them.

## 0.1.45 - 2026-09-29

### Localisation
- Fourteen interface languages were added, bringing the built-in total to eighteen, with the
  launcher and game-menu strings filled in and system fonts used for Chinese, Japanese and
  Hindi text.

## 0.1.44 - 2026-09-29

### Launcher
- The bus list is rebuilt only when its search, content or host list changes instead of every
  frame.

## 0.1.42 - 2026-09-29

### Graphics
- Puddle splashes are rendered as a widening, scene-lit mist instead of glowing sprite rings.

## 0.1.40 - 2026-09-29

### Controllers
- Pressing a controller button highlights it in the controller list, making bindings easier to
  identify.

## 0.1.37 - 2026-09-29

### Controls
- OMSI's held keyboard-pedal behaviour is available as an option: a tapped throttle or brake
  can keep its position until the opposite pedal is used.

## 0.1.34 - 2026-09-29

### Doors and spawning
- Berlin buses' rear doors close on Shift+2 again.
- An entry point whose marker lies below empty space places the bus on the ground above it,
  while real lower levels are preserved.

## 0.1.32 - 2026-09-29

### Website
- Buy Me a Coffee links were added to the README and project website.

## 0.1.31 - 2026-09-29

### Mirrors
- `reflexionN.bmp` camera pictures work wherever a vehicle material refers to them, including
  switched material items and light/night maps.

## 0.1.29 - 2026-09-29

### Traffic
- Barriers can open for the player's bus when it approaches off the traffic lanes, such as in
  depot yards.

## 0.1.27 - 2026-09-29

### Diagnostics
- Missing objects, splines and textures are reported by add-on in
  `~/.openomsi/missing_content.txt`.
- The game log records the system, launch settings, controls, views, pauses and regular runtime
  status so broken maps and sessions are easier to diagnose.

## 0.1.26 - 2026-09-29

### World
- A bus on a genuine lower level is no longer "rescued" onto a roof, and teleports that specify
  a height keep the intended level.

## 0.1.24 - 2026-09-29

### Controls and releases
- Door keys are assigned from the model's actual doorways and the triggers that move their
  leaves, fixing unusual mod door layouts.
- Release notes are generated from changelog lines added since the preceding release instead
  of falling back to a generic message.

## 0.1.22 - 2026-09-29

### Camera and controls
- Mouse steering can keep turning beyond the window edge, free-camera keys no longer drive the
  bus at the same time, and the mouse wheel zooms in free camera and on foot.
- Snow is matte instead of taking the wet-road gloss.

## 0.1.21 - 2026-09-29

### Physics
- A spline height profile far above the geometry it describes is taken at the drawn height
  instead of becoming an invisible wall; `OMSI_CHECK_WHEELS=1` can report what wheels meet.

## 0.1.20 - 2026-09-29

### Doors and controls
- Automatic rear doors close again instead of having their timer constantly restarted by
  waiting passengers.
- Wall tops stay walls, route numbers can be chosen correctly, and a force-feedback wheel
  recovers properly after a pause.

## 0.1.19 - 2026-09-29

### Graphics and diagnostics
- Automatic render scale stays at full size up to 4K on Windows and Linux.
- `OMSI_CHECK_SPLINES=1` can list spline chains whose ends do not meet.

## 0.1.18 - 2026-09-29

### Game and Discord
- The clock can be moved forward or back from the game menu, including an option to put a duty
  back on time with its timetable.
- Discord Rich Presence shows the current bus, map and line.

## 0.1.17 - 2026-09-29

### AI traffic
- A junction claim expires when its car has been crawling outside the junction for too long,
  preventing chains of traffic from waiting behind a stalled claimant.

## 0.1.15 - 2026-09-29

### Driving and vehicles
- Wheels prefer the road surface over terrain cutting through it, removing invisible bumps and
  walls where roads pass through the ground.
- Bus auto-start waits longer for delayed starters and can select D with the brake held.
- Manual gearboxes can be shifted with Ctrl+Up/Ctrl+Down, with an optional automatic clutch.
- Continued situations keep the livery, mouse steering survives a look-around, glance keys
  return to the road, and the bus can be moved from the map.
- The pause menu no longer flickers its top selection, and the phone wheel follows the bus wheel
  one to one.

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

## 0.1.13 - 2026-09-28

### Menu
- P opens the pause menu and resumes the game from it.

## 0.1.12 - 2026-09-28

### Driving and physics
- A bus spawned inside an obstacle can move out of it instead of being permanently stuck.
- Road paint close above another road face no longer lifts the wheels.
- With a steering wheel, the arrow keys look around as in OMSI.
- Ctrl+Alt+arrow turns the mirror being viewed, with the adjustment saved per bus in
  `mirrors.cfg`.

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
