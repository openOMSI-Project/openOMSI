# Plugins

openOMSI runs two kinds of plugins from the `plugins` folder of the game (and of every
content root):

* **Lua plugins** (`.lua`) - a text file, no compiler, works the same on Windows, macOS,
  Linux and Android, and is loaded again the moment you save it. They reach the whole game
  through the plugin API: the player's bus, the duty and timetable, the map, the AI traffic
  and the people, the weather and the clock, the camera, input, sound, panels of their own
  on the screen, storage, settings and the LAN session - about 270 functions and 60 events.
* **Compiled plugins** (`.oop`) - a plugin built with the
  [openOMSI Development Tools](https://github.com/openOMSI-org/openOMSI-Development-Tools):
  Lua, or Rust (and other languages) compiled to WebAssembly. One file for every platform,
  run in a sandbox with only the permissions it declares (see
  [below](#compiled-plugins-oop)).
* **OMSI plugins** (`.opl` + DLL) - the original's plugins, unchanged (see
  [below](#omsi-plugins-plugins-opl-dll)).

The plugin API is one registry in the game (`crates/omsi-plugin/src/api`): every function
and event is described there once, and the [reference](#reference) at the end of this page
and [`plugin-api.json`](plugin-api.json) are written from it. Other plugin languages bind to
the same registry.

## Lua plugins

### Your first plugin

Create `plugins/hello.lua` next to the game:

```lua
-- plugins/hello.lua
omsi.on("vehicle", function(name)
  omsi.message("Good morning! Today you drive the " .. name, 6)
end)

omsi.every(1, function()
  local kmh = omsi.var("Velocity")
  if kmh and kmh > 50 then
    omsi.message(string.format("Slow down: %.0f km/h", kmh), 1)
  end
end)
```

Start a map with a bus: the greeting shows up on the screen, and above 50 km/h the warning.
Edit the file while the game runs and save it - the plugin is loaded again within a second
("Lua plugin hello reloaded").

### Where plugins live

| Path | The plugin's name | Its data folder |
| --- | --- | --- |
| `plugins/<name>.lua` | a one-file plugin | `plugins/<name>.data/` |
| `plugins/<name>/main.lua` | a plugin with several files: `require("util")` loads `plugins/<name>/util.lua` (or `util/init.lua`), and its pictures, sounds and data files sit beside them | `plugins/<name>/data/` |

Other `.lua` files are not started on their own, so a folder plugin's modules stay modules.
`OMSI_NO_PLUGINS=1` leaves every plugin out, Lua ones included.

### How a plugin runs

The file's top level runs once when the game starts. After that the plugin reacts to
**events**. A handler is registered with `omsi.on(event, fn)` (as many as you like), or
by defining a global function `on_<event>`:

```lua
function on_frame(dt)
  -- runs ~60 times a second: keep it short, prefer omsi.every for slow work
end

omsi.on("stop_arrive", function(name, id, number, delay)
  omsi.ui.toast(string.format("%s, %s", name, omsi.fmt.delay(delay)))
end)
```

The [events](#events) table lists them all: the game's (a crash, a ticket sold, a trip
done), what changed (a door opened, the engine started, the bus arrived at a stop, a new
minute, a red light run), the panels' (a click, a slider moved), messages from other
plugins and from other players' games. An event the game has to work out by comparing two
frames - doors, gears, stops, the traffic light ahead - is only worked out while some plugin
listens to it.

`omsi.after`, `omsi.every` and `omsi.watch` call a function later, regularly, or when a
value changes; `omsi.emit("my_event", 1, 2)` sends an event to the plugin's own handlers
(handy between the modules of a bigger plugin), `omsi.plugin.send` to another plugin.

### The API

Every function is `omsi.<name>` - `omsi.bus.state()`, `omsi.ui.set(...)`. A few rules hold
for all of them:

* **Without the thing they read, readings are `nil`** (no bus, on foot, no duty, no
  timetable on the map), lists are empty, and changes give `false`. A plugin works on
  every map and with every bus; a bus that lacks a script variable gives `nil` for it.
* **Several results** come as several values: `local x, y, z, heading = omsi.position()`;
  none at all when there is nothing (`if omsi.position() then`).
* **Changes that can fail** give `true`, or `false` and the reason:
  `local ok, why = omsi.weather.set({ temperature = -5 })`.
* **A wrong argument is an error** that names the function and the argument
  (`omsi.var: argument 1 (name) must be a string, not table`); catch it with `pcall`
  where it may happen.
* **Units**: metres, map coordinates x east, y north, z up; headings in degrees clockwise
  from north; speeds km/h; times seconds (of the day since midnight for clock times).
* **New functions come with new openOMSI versions.** `omsi.game.has("weather.set")` tells
  whether this game has one; the reference says since which version each exists.

The parts of the API, with a taste of each (the [reference](#reference) has every
function):

```lua
-- the player's bus: a dashboard in one table, and its parts by name
local s = omsi.bus.state()            -- speed, gear, rpm, doors_open, indicator, fuel, ...
omsi.bus.toggle_door(1)               -- the front door's key
omsi.bus.set_indicator("left")
local all, seated, standing = omsi.bus.passengers()
omsi.var("elec_busbar_main")          -- any script variable; omsi.trigger("bus_horn") any trigger

-- the duty and the timetable
local d = omsi.duty.get()             -- line, tour, trip, next_stop, delay, ...
for _, stop in ipairs(omsi.duty.stops()) do print(stop.name, omsi.fmt.clock(stop.departure)) end
omsi.duty.start("136", "3")           -- as the game menu takes a duty

-- the map and moving the bus
local ground = omsi.map.ground(x, y)
omsi.map.teleport(x, y, nil, 90)      -- z nil: onto the ground there
local limit = omsi.map.speed_limit()

-- the AI traffic and the people
local ahead = omsi.traffic.ahead(80)  -- the car in front, with its distance
local light = omsi.traffic.light_ahead()
omsi.traffic.set_density(50)
local waiting = omsi.people.waiting(stop_id)

-- time and weather, read and set as the game menu does
omsi.world.set_time("07:30")
omsi.weather.set({ precipitation = "snow", precipitation_rate = 0.5, temperature = -3 })
omsi.weather.preset(omsi.weather.presets()[1].file, 60)

-- camera, input, sound
local sx, sy = omsi.camera.project(x, y, z + 3)   -- where a map point is on the screen
omsi.input.hotkey("Ctrl+KeyH", function() ... end)
local id = omsi.audio.play("gong.wav", { on_bus = true, volume = 0.8 })

-- storage, files, settings, other plugins
omsi.storage.set("best_trip", 812)
omsi.files.append("trips.csv", "136,3,+30\n")
local settings = omsi.plugin.settings({ { key = "volume", type = "number", default = 50 } })
omsi.plugin.broadcast("delay", omsi.duty.delay())

-- the LAN session
for _, p in ipairs(omsi.lan.players()) do print(p.name, p.line) end
omsi.lan.send(0, "hello")             -- to this plugin on every other player's game
```

#### `omsi.info()` in detail

`omsi.info()` is the game's state as one table (`omsi.info_value(key)` reads one key
without building the table):

`map`, `clock` (seconds since midnight), `day`, `year`, `view`, `paused`, `on_foot`,
`multiplayer`, `traffic` (AI vehicles), `speed` (km/h), `delay` (s, late positive),
`map_path` (the map's global.cfg), `version` (of openOMSI); with a bus also `tile_x`,
`tile_y` (its tile, as global.cfg's `[map]` list numbers them), `tile_pos_x`, `tile_pos_y`
(metres in that tile, x east, y north), `heading` (degrees, clockwise from north),
`vehicle_manufacturer`, `vehicle_model`, `destination` (the terminus the bus shows),
`passengers` (aboard); `crashes`, `heavy_crashes` and `pedestrians_hit` this session (as
the personnel file counts them); `situation`, the situation file the game started from (the
launcher's "continue" loads `maps/<map>/laststn.osn`), `nil` for a new game; on a duty also
`line`, `tour`, `trip` (its number in the duty), `trips`, `trip_name` (the timetable's name
of the trip), `terminus`, `stops` (how many the trip has), `trip_done` (`true` once the bus
has reached the trip's last stop, the stop was skipped, or a saved game was left there: the
trip is over, though the duty moves on to the next trip only a minute before it leaves),
`next_stop`, `next_stop_number` (from 1), `next_stop_arrival`, `next_stop_departure`,
`next_stop_id` (the stop's object ID in the map, as the timetable names it: the same name
can stand for two stops, the ID cannot), `at_stop` (`true` while the bus stands at the next
stop), `previous_stop` and `previous_stop_id` (the last stop before the next one the trip
calls at; none before the first), `next_stop_distance` and `previous_stop_distance` (metres
in a straight line from the bus, where the stop's place is known).

#### Saved data

`omsi.data` is a table that survives the session: it is written when the game ends (and
before a reload) and read back on the next start. Numbers, strings, booleans and tables of
them are kept. `omsi.save()` writes it at once. The file is `<name>.save.lua` next to a
one-file plugin, `data.save.lua` in a folder plugin's folder. `omsi.storage` is the same
idea as keys and values, kept as JSON in the plugin's data folder (and the same for every
plugin language).

```lua
-- plugins/odometer.lua: kilometres driven, over every session
omsi.data.km = omsi.data.km or 0
function on_frame(dt)
  omsi.data.km = omsi.data.km + math.abs(omsi.var("Velocity") or 0) * dt / 3600
end
omsi.every(60, function()
  omsi.message(string.format("Odometer: %.1f km", omsi.data.km), 3)
end)
```

#### Settings

`omsi.plugin.settings({...})` declares the plugin's settings and gives back the values the
player chose. The game makes a panel of them (`omsi.plugin.show_settings()`: checkboxes,
sliders, text fields, tabs; it can be dragged and closed), keeps them in the data folder and
tells the plugin each change (`setting(key, value)`).

#### Talking to other programs

`omsi.send(port, text)` sends `text` as one UDP datagram to `127.0.0.1:port`: to another
program on this computer (an overlay, a dashboard, a company's tracker), never over the
network. It does not wait and nothing comes back: a message sent while no program listens
is lost, so keep what must not be lost in `omsi.storage` as well.

| Returns | When |
| --- | --- |
| `true` | the message was handed to the system |
| `false`, reason | the port is below 1024 or one of the game's multiplayer ports (27015-27024), the message is longer than 8 KB, the plugin sent 100 messages in the last second already, or the system refused it |

```lua
-- plugins/live.lua: the speed and the next stop, twice a second, for a program on port 47800
omsi.every(0.5, function()
  local d = omsi.duty.get()
  omsi.send(47800, omsi.json.encode({ speed = omsi.speed(), next_stop = d and d.next_stop and d.next_stop.name }))
end)
```

`nc -lu 47800` in a terminal shows what arrives.

#### Between plugins and between players

`omsi.plugin.send(to, topic, data)` and `omsi.plugin.broadcast(topic, data)` reach other
plugins in their next frame (`message(from, topic, data)`); `omsi.plugin.list()` names the
plugins loaded. In a LAN game `omsi.lan.send(player, text)` reaches the same plugin on
another player's game (`lan_message(from, text)`): short texts (120 characters), about ten
a second, carried in the session's own messages - a game without the plugin ignores them.

### On-screen panels

`omsi.ui` puts panels of the plugin's own on the screen, in the game's look: Roboto, the
game's icons, dark rounded cards with a shadow, as large as the rest of the interface (the
interface size setting and the window's height scale them). A plugin describes a panel as a
table once and sets it again when its content changes - every half second, say, not every
frame; the same table again changes nothing. `omsi.ui.update(panel, element, {...})`
changes one element in place. The game lays the panel out and draws it again only when it
did change.

```lua
-- plugins/trip_panel.lua: the next stop, the speed and a button
local function show()
  local i = omsi.info()
  omsi.ui.set("trip", {
    anchor = "top_left", x = 16, y = 60, width = 300, accent = "#F47F30",
    children = {
      { type = "row", children = {
        { type = "icon", name = "directions_bus", color = "#F47F30" },
        { type = "text", text = "Line " .. (i.line or "-"), weight = "bold", grow = true },
        { type = "badge", text = string.format("%.0f km/h", i.speed or 0) },
      } },
      { type = "text", text = "Next stop: " .. (i.next_stop or "-") },
      { type = "button", id = "horn", text = "Horn", icon = "campaign" },
    },
  })
end

omsi.every(0.5, show)
omsi.on("key", function(key, down)
  if key == "F10" and down then omsi.ui.focus(not omsi.ui.focused()) end
end)
omsi.on("ui_click", function(panel, element)
  if element == "horn" then omsi.trigger("horn") end
end)
```

F10 gives the panels the mouse; a click on the button sounds the horn. The whole example is
[`docs/examples/plugins/trip_panel.lua`](examples/plugins/trip_panel.lua).

`omsi.ui.version` is `2` (`1` before the controls below): test
`if omsi.ui and omsi.ui.version >= 2` in a plugin that should also run in an older openOMSI.
The panels are the plugin's own: one plugin cannot change or remove another's, and when the
plugin stops, or is loaded again after a change of its file, its panels go (its
notifications run their time). They show over the picture and the navigator and under the
game's own lines, menus and windows, and not at all while the game menu, a list of it or the
city map is open, nor in VR.

#### The panel table

```lua
omsi.ui.set("trip", {
  anchor = "top_left",       -- top_left top top_right left center right bottom_left bottom bottom_right
  x = 16, y = 16,             -- from the anchor towards the middle (from a middle: right and down)
  width = 340,                -- the height follows the content
  padding = 12,               -- default 12
  gap = 6,                    -- between the children, default 6
  background = "#14161ACC",   -- default: the game's card colour; "#RRGGBB" or "#RRGGBBAA"
  radius = 12,                -- default 12
  accent = "#F47F30",         -- a stripe along the left edge
  visible = true,             -- false hides it and keeps it
  clickable = false,          -- true: a click anywhere on it is ui_click(id, nil)
  draggable = false,          -- true: while the panels have the mouse it can be moved by its free parts
  children = { ... },         -- elements, top to bottom
})
```

Sizes are pixels of a 1080p screen at the normal interface size. A panel stays on the screen
whatever its offset; one the player dragged keeps its new place when it is set again
(`omsi.ui.moved(id)` says how far).

#### Elements

Every element can have an `id`, a `color` and `visible = false` (left out, no room kept).

| `type` | Keys | Draws |
| --- | --- | --- |
| `text` | `text`, `size` (default 14), `weight` (`regular`, `medium`, `bold`), `align` (`left`, `center`, `right`), `wrap` (default `true`; `false`: one line, cut with "…") | a line or a paragraph, wrapped at the width it has; a line is 1.3 × `size` high |
| `icon` | `name`, `size` (default 20) | one of the game's icons (Material Symbols names: `directions_bus`, `schedule`, `payments`, `warning`, `star`, `emoji_events`... - every one in [`assets/icons/material`](../assets/icons/material)); a name the game does not have draws nothing |
| `row` | `children`, `gap` (default 8), `align` (`start`, `center`, `end`, `between`) | its children side by side, centred on each other; a child with `grow = true` takes the width the others leave (several share it); when they do not fit, texts, labels and buttons give up width alike |
| `bar` | `value` (0 to 1), `height` (default 6), `color` (the filled part, default the game's amber), `background` | a progress bar; in a row 60 wide unless it grows |
| `badge` | `text`, `color` (its fill, default amber), `text_color` | a small rounded label, 20 high |
| `divider` | `color` | a thin line (in a row: upright) |
| `space` | `size` (default 8) | empty room (in a row: across) |
| `button` | `id` (needed), `text`, `icon`, `color` | a button 34 high, across the panel (in a row as wide as its label, or what it grows to); a click on it is `ui_click(panel id, button id)` |
| `image` | `src` (a PNG, JPEG, BMP or TGA of the plugin's folder), `width`, `height` (default 64) | the picture, made smaller to fit the width it has |
| `checkbox` | `id` (needed), `text`, `checked` | a box ticked and unticked by a click: `ui_change(panel, id, true/false)` |
| `slider` | `id` (needed), `value`, `min` (0), `max` (1), `step` (0: any) | a track with a knob, set where it is pressed and followed while the button is held: `ui_change(panel, id, number)` |
| `input` | `id` (needed), `text`, `placeholder`, `max` (characters, default 100) | a text field: clicked, it takes what is typed (the keys do not reach the bus meanwhile) - `ui_change` at every key, Enter is a `ui_click` on it, Esc or a click elsewhere leaves it |
| `tabs` | `id` (needed), `tabs` (a list of texts), `selected` (from 1) | a row of tabs: `ui_change(panel, id, number)` when one is chosen |
| `chart` | `values` (up to 512 numbers), `min`, `max` (default: the values'), `height` (48), `style` (`line`, `bars`), `fill` | the values over the width: a line (filled below with `fill`) or bars |
| `table` | `columns` (header texts), `rows` (a list of lists of texts), `widths` (shares of the width), `size` (13) | columns of texts, the header bold over a line |

Another element with an `id` and `clickable = true` (a whole row, say) is clicked as a
button is. Texts are shown as they are: they are not translated into the game's language.

Keys the game does not know are left alone and an element of an unknown `type` is left out,
so a plugin written for a later openOMSI still shows its panels here. A key of the wrong
kind makes `set` return `false` and the reason. Limits, each plugin: 16 panels, 200
elements in a panel (those in rows counted), 500 characters in a text, 64 in an id, rows 8
deep, 512 values in a chart or cells in a table.

### Example plugins

Complete plugins to start from (each runs in the tests against a stand-in game):

| Plugin | What it shows |
| --- | --- |
| [`trip_panel.lua`](examples/plugins/trip_panel.lua) | a panel with the line, the next stop and a button |
| [`dispatcher/`](examples/plugins/dispatcher/main.lua) | a dispatcher and a small career: a draggable panel with tabs, the duty's next stops in a table, a chart of the delays, pay for stops and trips kept in the storage, today's duties offered and taken with a click, settings, hotkeys |
| [`telemetry_hud.lua`](examples/plugins/telemetry_hud.lua) | a HUD: the speed with a chart of the last minute, gear, rpm, fuel, passengers, the speed limit and the traffic light ahead; a warning before a red light; the values as JSON over UDP for a program beside the game |
| [`weather_controller.lua`](examples/plugins/weather_controller.lua) | the weather files as tabs, sliders for temperature, visibility and rain, snow cover, the clock moved by buttons, and a day cycle of its own |

### Safety and errors

A Lua plugin gets Lua 5.4 with the safe libraries only: `string`, `table`, `math`, `utf8`,
`coroutine`, `require` for its own folder, and `os.clock/time/date/difftime`. There is no
`io`, no `os.execute` (not through `require("os")` either), no C modules, no `dofile`, and no
binary chunks anywhere: plugin files, modules and `load` read text only (Lua has no checker
for compiled code), and `string.dump` gives no code. A plugin reads and writes files only in
its own data folder (`..` and absolute paths are refused) and reads its own folder; it
cannot reach the network either: `omsi.send` talks only to programs on this computer, and
only to ports from 1024 up. It holds at most 256 MB of memory.

* An error in a handler is written to `game.log` and shown on the screen; the other
  plugins and the game carry on. After 10 errors the plugin is switched off until you
  change its file or restart the game.
* A call into the plugin that runs longer than 50 ms (an endless loop) is stopped and the
  plugin switched off, with a line in `game.log`; loading it may take a second.
* A file that does not compile is left out, with the Lua error in `game.log`.

### Tips

* Watch `game.log` (in `~/.openomsi/`) while you write a plugin: every `omsi.log` line and
  every error is there.
* `OMSI_WATCH_VARS=Velocity,throttle` logs changes of bus variables - useful to find the
  names a bus uses; the bus's `.osc` scripts list them all, `omsi.vars()` too.
* Keep `on_frame` light; use `omsi.every`, `omsi.watch` and the events for everything that
  does not need every frame. Read what you need (`omsi.bus.velocity()`,
  `omsi.info_value("delay")`) rather than whole tables every frame.
* `omsi.ui.update` changes one element of a panel; setting the whole panel every frame
  costs more.

### Reference

Every function and event, as the game's API registry describes them (also as JSON in
[`plugin-api.json`](plugin-api.json), for tools and other plugin languages). `[x]` is an
optional argument; a permission is what a plugin packed with a list of permissions must have
declared to call the function (a plain `.lua` file has them all).

<!-- api:begin (written from the registry: OMSI_API_BLESS=1 cargo test -p omsi-plugin api_manifest) -->

#### The player's bus

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.bus.acceleration()` | across, along, up | Its acceleration in its own frame, m/s² without gravity: to the right, forwards, up. | 0.2.22 |
| `omsi.bus.action(name, [down])` | boolean | A key action of the vehicles (`[vehicles]` of keyboard.cfg: `horn`, `parking_brake_toggle`, `kw_scheinwerfer_toggle`, `blinker_left_set`, `ticket_give`, ...): pressed and let go, or held (`down` true) and let go (`false`). `true` when the bus knows it. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.controls()` | throttle, brake, clutch, steering | What the bus drives with now: the pedals (0 to 1) and the steering (-1 left to 1 right), from the keys, the mouse or a controller. | 0.2.22 |
| `omsi.bus.damage()` | table or nil | `{crashes, last_impact_kj, repair_minutes}`: the bus's crashes, the energy of the last one and how long a repair would take (`nil`: nothing to repair). | 0.2.22 |
| `omsi.bus.destination()` | name, index | The destination the bus shows and its index in `destinations` (nothing when none). | 0.2.22 |
| `omsi.bus.destinations()` | list of tables | The destinations of the bus's depot file: `{index, code, name, all_exit}` (`index` from 1, for `set_destination`). | 0.2.22 |
| `omsi.bus.dirt()` | number or nil | How dirty the bus is, 0 (clean) to 1. | 0.2.22 |
| `omsi.bus.door(n)` | boolean or nil | Whether door leaf `n` (from 1) is open. | 0.2.22 |
| `omsi.bus.door_count()` | integer | The doorways the door keys work, front to back. | 0.2.22 |
| `omsi.bus.doors()` | list of numbers | Each door leaf's position, front to back: 0 shut, 1 open (the scripts' `door_0`, `door_1`, ...). | 0.2.22 |
| `omsi.bus.doors_open()` | boolean or nil | Whether any door is open (by the passengers' door flags where the bus has them). | 0.2.22 |
| `omsi.bus.electrics()` | boolean or nil | Whether the bus's electrics are on (the main switch). | 0.2.22 |
| `omsi.bus.engine()` | running, rpm, electrics | The engine: whether it runs, its rpm (where the bus shows one) and whether the electrics are on. | 0.2.22 |
| `omsi.bus.engine_running()` | boolean or nil | Whether the engine runs. | 0.2.22 |
| `omsi.bus.file()` | string or nil | The bus's `.bus` file, relative to the game folder. | 0.2.22 |
| `omsi.bus.fuel()` | number or nil | The fuel in the tank, litres (the scripts' `engine_tank_content`). | 0.2.22 |
| `omsi.bus.gear()` | integer or nil | The gear engaged (-1 reverse, 0 neutral), where the bus shows one. | 0.2.22 |
| `omsi.bus.get_strings([names])` | table | The same for string variables. | 0.2.22 |
| `omsi.bus.get_vars([names])` | table | Many script variables at once: a table name -> value of the names given, or of every variable of the bus without a list. | 0.2.22 |
| `omsi.bus.handbrake()` | boolean or nil | Whether the parking brake is on. | 0.2.22 |
| `omsi.bus.headlights()` | integer or nil | The headlights: 0 off, 1 side lights, 2 dipped, 3 high beam. | 0.2.22 |
| `omsi.bus.horn()` | boolean or nil | Whether the horn sounds. | 0.2.22 |
| `omsi.bus.indicator()` | string or nil | The indicators: `"off"`, `"left"`, `"right"` or `"hazard"`. | 0.2.22 |
| `omsi.bus.interior_light()` | number or nil | The passenger room's light, 0 (off) to 1. | 0.2.22 |
| `omsi.bus.km_today()` | number or nil | Kilometres driven this session (as the personnel file counts them). | 0.2.22 |
| `omsi.bus.kneeling()` | boolean or nil | Whether the bus kneels. | 0.2.22 |
| `omsi.bus.mass()` | number or nil | The bus's mass in kg. | 0.2.22 |
| `omsi.bus.number()` | string or nil | The bus's fleet number. | 0.2.22 |
| `omsi.bus.odometer()` | number or nil | The bus's odometer in km. | 0.2.22 |
| `omsi.bus.orientation()` | heading, pitch, bank | Heading (degrees clockwise from north), pitch (nose up positive) and bank (right side down positive). | 0.2.22 |
| `omsi.bus.passengers()` | total, seated, standing | The people aboard the bus: all, sitting, standing. | 0.2.22 |
| `omsi.bus.play_sound(event)` | boolean | Plays the bus's own sound of this event (a trigger name of its sound files, `ev_...`). *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.retarder()` | number or nil | The retarder's step, where the bus has one. | 0.2.22 |
| `omsi.bus.rpm()` | number or nil | The engine's revolutions per minute. | 0.2.22 |
| `omsi.bus.sales()` | tickets, money | Tickets sold this session and the money taken (the game knows no currency). | 0.2.22 |
| `omsi.bus.set_destination(index)` | boolean | Shows destination `index` (from 1, as `destinations` lists them). *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.set_indicator(state)` | boolean | Sets the indicators: `"off"`, `"left"`, `"right"` or `"hazard"`. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.set_interior_light(on)` | boolean | Switches the passenger room's light on or off. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.set_line(line)` | boolean | Types a line (route) into the bus's IBIS, as the player would. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.set_vars(values)` | integer | Sets many script variables at once (a table name -> number); how many the bus has. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.shift(gear)` | boolean | Puts a manual gearbox's lever in a gear (-1 reverse, 0 neutral). *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.sound_horn(down)` | boolean | Holds the horn (`true`) or lets it go. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.start_up()` | string or nil | Starts the bus up the way Shift+U does (the battery, the electrics, the engine) - or shuts a running bus down; what the game says it does. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.state()` | table or nil | Everything a dashboard shows, in one table: `speed` (km/h, signed), `gear`, `engine` (running), `rpm`, `electrics`, `doors_open`, `indicator`, `headlights`, `dirt`, `passengers`, `odometer`, `km_today`, `throttle`, `brake`, `clutch`, `steering`, `fuel`, `handbrake`, `horn`, `stop_request` (a key is `nil` where the bus does not have it). | 0.2.22 |
| `omsi.bus.steering_angle()` | angle, max | The front wheels' angle and the most they turn, degrees (right positive). | 0.2.22 |
| `omsi.bus.stop_brake()` | boolean or nil | Whether the stop brake (the door brake) holds the bus. | 0.2.22 |
| `omsi.bus.stop_requested()` | boolean or nil | Whether a passenger has asked to stop. | 0.2.22 |
| `omsi.bus.ticket_request()` | name, price | The ticket the passenger at the cash desk asks for (nothing when nobody asks). | 0.2.22 |
| `omsi.bus.tickets()` | list of tables | The tickets the map sells: `{name, price, day_ticket}`. | 0.2.22 |
| `omsi.bus.toggle_door([n])` | boolean | Presses the key of doorway `n` (from 1; 0 or none: all doors), as the player would. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.toggle_handbrake()` | boolean | Puts the parking brake on or off, as its key does. *Permission: `vehicle_write`.* | 0.2.22 |
| `omsi.bus.trailers()` | integer or nil | Parts coupled behind the bus (an articulated bus's rear counts). | 0.2.22 |
| `omsi.bus.triggers()` | list of strings | The names of the bus's script triggers (for `trigger`, `press`). | 0.2.22 |
| `omsi.bus.velocity()` | number or nil | The bus's speed in km/h, negative backwards. | 0.2.22 |
| `omsi.bus.velocity_vector()` | x, y, z | Its velocity in the world, m/s (x east, y north, z up). | 0.2.22 |
| `omsi.bus.wheels()` | list of tables | Each wheel: `{axle, side, rpm, radius, suspension, driven}` (`side` 0 left, 1 right; `suspension` the spring's travel). | 0.2.22 |
| `omsi.distance(x, y)` | number or nil | Metres from the bus to a map point, or `nil` on foot. | 0.1.10 |
| `omsi.has_vehicle()` | boolean | `true` while the player drives a vehicle. | 0.1.5 |
| `omsi.position()` | x, y, z, heading | Where the bus is: map metres (x east, y north, z up) and its heading in degrees clockwise from north; nothing on foot. | 0.1.10 |
| `omsi.press(name)` | nil | Holds a key of the bus down: fires the trigger `name` (let it go with `release`). *Permission: `vehicle_write`.* | 0.1.5 |
| `omsi.release(name)` | nil | Lets a key go: fires `<name>_off`. *Permission: `vehicle_write`.* | 0.1.5 |
| `omsi.set_str(name, text)` | boolean | Sets a string variable; `true` when the bus has it. *Permission: `vehicle_write`.* | 0.1.5 |
| `omsi.set_var(name, value)` | boolean | Sets a script variable; `true` when the bus has that variable. *Permission: `vehicle_write`.* | 0.1.5 |
| `omsi.speed()` | number | The bus's speed in km/h, forwards or backwards (0 on foot). | 0.1.10 |
| `omsi.str(name)` | string or nil | A string variable of the bus's scripts (`IBIS_terminus_name`). | 0.1.5 |
| `omsi.sys(name)` | number or nil | A system variable of the scripts: `Time`, `Day`, `Weather_Temperature`, `SunAlt`, ... (read only). | 0.1.5 |
| `omsi.trigger(name)` | nil | A key press of the bus: fires the trigger, then `<name>_off`. *Permission: `vehicle_write`.* | 0.1.5 |
| `omsi.var(name)` | number or nil | A script variable of the bus (`Velocity`, `elec_busbar_main`, the names the `.osc` files and `.opl` lists use); `nil` without a bus or for a name it does not have. | 0.1.5 |
| `omsi.vars([kind])` | list of strings | The names of every variable of the bus's scripts, or of every string variable with `"str"`. | 0.1.10 |
| `omsi.vehicle()` | string or nil | The vehicle's name (manufacturer and type), or `nil` on foot. | 0.1.5 |
| `omsi.vehicle_manufacturer()` | string or nil | The manufacturer part of the vehicle's name, as its `[friendlyname]` has it (`"Solaris III Gen"`). | 0.2.21 |
| `omsi.vehicle_model()` | string or nil | The model part of the vehicle's name (`"Urbino 10 / 2D"`). | 0.2.21 |

#### Duty and timetable

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.duty.active()` | boolean | Whether the player drives a duty (a line and tour of the timetable). | 0.2.22 |
| `omsi.duty.at_stop()` | boolean | Whether the bus stands at the next stop (within 25 m of it). | 0.2.22 |
| `omsi.duty.delay()` | number or nil | Seconds the bus is late (early negative), worked out between the stops as the game's timetable does. | 0.2.22 |
| `omsi.duty.finish()` | boolean | Gives the duty up (the bus drives on without a timetable); `true` when there was one. *Permission: `world_write`.* | 0.2.22 |
| `omsi.duty.get()` | table or nil | The duty now: `line`, `tour`, `trip` (its number in the duty, from 1), `trips`, `trip_name`, `terminus`, `departure`, `arrival`, `stops`, `next_stop` and `previous_stop` (each a stop: `{number, name, id, arrival, departure, stops, x, y, z}`), `at_stop`, `trip_done`, `delay` (seconds, late positive). | 0.2.22 |
| `omsi.duty.next_stop()` | table or nil | The next stop of the trip (as `duty.stops` gives them). | 0.2.22 |
| `omsi.duty.skip_stop()` | string or nil | Skips the next stop (the duty goes on to the one after); the name of the stop skipped. *Permission: `world_write`.* | 0.2.22 |
| `omsi.duty.skip_to(stop)` | boolean | Makes stop `stop` (from 1) of the trip the next one, forwards or back. *Permission: `world_write`.* | 0.2.22 |
| `omsi.duty.start(line, tour, [trip], [stop])` | true, or false and the reason | Takes a duty, as the game menu's "Line and tour" does: a line and tour of `timetable.lines`, its trip (from 1 in the order they leave; default the first) and the stop to start at (from 1 among those the trip calls at; default the first). The bus stays where it is. *Permission: `world_write`.* | 0.2.22 |
| `omsi.duty.stops()` | list of tables | The stops of the trip now: `{number, name, id, arrival, departure, stops, passed, x, y, z}` (`stops` false: the bus passes it; `x, y, z` where the stop's place is known; `id` the map's object id). | 0.2.22 |
| `omsi.duty.trip_stops(trip)` | list of tables or nil | The stops of a trip of the duty (from 1), as `duty.stops` gives them. | 0.2.22 |
| `omsi.duty.trips()` | list of tables | The trips of the duty: `{number, name, line, terminus, departure, arrival, stops}`. | 0.2.22 |
| `omsi.timetable.buses()` | list of tables | The timetable buses of the AI on the road: `{id, line, tour, trip, terminus, departure, next_stop_id, at_stop, trip_done, delay, x, y, number}`. | 0.2.22 |
| `omsi.timetable.lines()` | list of tables | The map's lines: `{name, user_allowed, tours}`, each tour `{number, today, trips}` (`today`: it runs on the game's date). | 0.2.22 |
| `omsi.timetable.stop_names()` | list of tables | The stops the timetable knows: `{id, name}` (`id` the map's object id). | 0.2.22 |
| `omsi.timetable.stops(line, tour, [trip])` | list of tables | The stops of a tour's trips (or of its trip `trip`, from 1 in the order they leave) that the bus calls at: `{trip, station, name, departure}`. | 0.2.22 |

#### The map

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.map.entrypoints()` | list of tables | The map's start points: `{index, name, x, y, z, heading}` (`index` from 1; the place where the game has read its tile). | 0.2.22 |
| `omsi.map.ground(x, y)` | number or nil | The height of the ground (the road where there is one, else the terrain) at a map point, where its tile is loaded. | 0.2.22 |
| `omsi.map.info()` | table or nil | `{name, friendly_name, path, left_hand_traffic, tile_size}`: the map's names, its global.cfg and the side it drives on. | 0.2.22 |
| `omsi.map.lane(x, y)` | table or nil | The traffic lane nearest to a map point: `{index, distance, speed_limit, name, traffic_light}`. | 0.2.22 |
| `omsi.map.name()` | string or nil | The map's name (its folder's). | 0.2.22 |
| `omsi.map.object(id)` | x, y, z, heading | Where a map object is, by its id (a stop's id of the timetable, say); nothing when the game has not read its tile. | 0.2.22 |
| `omsi.map.objects_near(x, y, [radius])` | list of tables | The map objects within `radius` m (default 50, at most 2000 of them, nearest first): `{id, x, y, z, heading, distance}`. | 0.2.22 |
| `omsi.map.place_on_road(x, y)` | boolean | Puts the player's bus on the street nearest to a map point (within 300 m), along it. *Permission: `world_write`.* | 0.2.22 |
| `omsi.map.speed_limit()` | number or nil | The speed limit (km/h) of the lane the player's bus drives on (`nil` off the lanes). | 0.2.22 |
| `omsi.map.stops()` | list of tables | The bus stops of the tiles loaded (around the camera): `{id, name, x, y, z, heading}`. | 0.2.22 |
| `omsi.map.teleport(x, y, [z], [heading])` | boolean | Moves the player's bus to a map point (z none: onto the highest ground there) facing `heading` (default north), as the game menu's move does; the `service` event says `teleport`. *Permission: `world_write`.* | 0.2.22 |
| `omsi.map.teleport_to(index)` | boolean | Moves the player's bus to start point `index` (from 1, as `map.entrypoints` lists them). *Permission: `world_write`.* | 0.2.22 |
| `omsi.map.terrain(x, y)` | number or nil | The terrain's height alone at a map point. | 0.2.22 |
| `omsi.map.tile()` | tile_x, tile_y | The tile the player's bus is on (nothing on foot). | 0.2.22 |
| `omsi.map.tile_at(x, y)` | tile_x, tile_y, local_x, local_y | The tile of a map point and the metres in it (x east, y north). | 0.2.22 |
| `omsi.map.tiles()` | list of tables | The map's tiles: `{x, y, file, loaded}` (numbered as global.cfg's `[map]` list). | 0.2.22 |
| `omsi.map.to_world(tile_x, tile_y, local_x, local_y)` | x, y | A place given by its tile and the metres in it, in map coordinates. | 0.2.22 |

#### AI traffic

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.other_var(id, name)` | number or nil | A script variable of one of `others`, or `nil`. | 0.2.21 |
| `omsi.others([radius])` | list of tables | The other vehicles within `radius` m of the bus (default 300): each `{id, kind, name, x, y, z, heading}`, `kind` being `"ai"` (the traffic) or `"player"` (another player's bus in a LAN game); empty on foot. | 0.2.21 |
| `omsi.set_other_var(id, name, value)` | boolean | Sets a script variable of one of `others`; `true` when that vehicle has it. An AI vehicle keeps it until its scripts write it again; another player's bus takes its values from the network again. *Permission: `traffic_write`.* | 0.2.21 |
| `omsi.traffic.ahead([reach])` | table or nil | The AI vehicle ahead of the player's bus within `reach` m (default 100) and 20° of its heading, with its `distance`: for a distance warning or a cruise control. | 0.2.22 |
| `omsi.traffic.clear()` | integer or nil | Takes every car not running to a timetable off the road; how many went. *Permission: `traffic_write`.* | 0.2.22 |
| `omsi.traffic.counts()` | driving, buses, asleep, parked | The AI vehicles: driving, timetable buses among them, asleep out of range, parked. | 0.2.22 |
| `omsi.traffic.density()` | cars, share | How many cars the traffic keeps around the camera, and the share of them that do not run to a timetable (0 to 1). | 0.2.22 |
| `omsi.traffic.get(id)` | table or nil | One AI vehicle by its id, as `traffic.list` gives them. | 0.2.22 |
| `omsi.traffic.light_ahead([reach])` | table or nil | The traffic light the player's bus comes to within `reach` m (default 80): `{aspect, change_in, distance}` - `aspect` `"red"`, `"red_yellow"`, `"green"`, `"green_yellow"`, `"yellow"` or `"dark"`, `change_in` the seconds to its next change. | 0.2.22 |
| `omsi.traffic.list([radius])` | list of tables | The AI vehicles (within `radius` m of the player's bus, when given): `{id, kind, name, x, y, z, heading, speed, max_speed, waiting_for, standing, braking, blinker, line, distance}`. `kind`: `"car"`, `"taxi"`, `"bus"`, `"truck"`, `"timetable_bus"`, `"tram"`, `"bicycle"`; `waiting_for` why it waits or slows (`"lead"`, `"light"`, `"yield"`, `"people"`, ...); `standing` the seconds it has stood. | 0.2.22 |
| `omsi.traffic.nearest([kind])` | table or nil | The AI vehicle nearest to the player's bus (of that `kind`, when given), with its `distance`. | 0.2.22 |
| `omsi.traffic.remove(id)` | boolean | Takes an AI vehicle off the road (a timetable bus too: its timetable forgets it); its passengers get out. *Permission: `traffic_write`.* | 0.2.22 |
| `omsi.traffic.set_density(cars, [share])` | boolean | Changes the traffic's amount (as the game menu's slider: 0 to 500 cars) and, when given, the share not running to a timetable; it fills up or thins out over the next seconds. *Permission: `traffic_write`.* | 0.2.22 |

#### People

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.people.counts()` | walking, waiting, riding | The people of the map near the camera: walking, waiting at stops, riding a bus. | 0.2.22 |
| `omsi.people.density()` | number or nil | The people setting: 0 to 3, 1 the map's own amount. | 0.2.22 |
| `omsi.people.list([radius])` | list of tables | The people (within `radius` m of the player's bus, when given): `{id, x, y, z, state, aboard, ai_bus, stop, destination, ticket, complaint}`. `state`: `"strolling"`, `"idle"`, `"standing"`, `"waiting"`, `"to_bus"`, `"boarding"`, `"riding"`, `"seated"`, `"leaving"`, `"to_stop"`; `aboard` in the player's bus; `complaint` 0 to 3 (3: they leave). | 0.2.22 |
| `omsi.people.set_density(value)` | boolean | Changes the people setting (0 to 3). *Permission: `traffic_write`.* | 0.2.22 |
| `omsi.people.stops()` | list of tables | The stops near the camera where people wait: `{id, name, x, y, z, waiting}`. | 0.2.22 |
| `omsi.people.waiting(stop)` | integer | How many people wait at a stop (its map object id). | 0.2.22 |

#### Time

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.clock()` | string | The game's time of day as `"HH:MM:SS"`. | 0.1.10 |
| `omsi.world.date()` | table or nil | The game's date: `{year, month, day, weekday, day_of_year}` (`weekday` 1 Monday to 7 Sunday). | 0.2.22 |
| `omsi.world.pause()` | boolean | Pauses the game, as P does (not in a LAN game). The plugins stand still with it: the player resumes it. *Permission: `world_write`.* | 0.2.22 |
| `omsi.world.paused()` | boolean | Whether the game stands still (a plugin hears `pause` and runs no more until `resume`). | 0.2.22 |
| `omsi.world.play_time()` | number or nil | Seconds played this session (game time, never wraps). | 0.2.22 |
| `omsi.world.season()` | folder, snow | The season's texture folder (`nil`: the base textures) and whether snow lies. | 0.2.22 |
| `omsi.world.set_date(year, month, day)` | boolean | Sets the game's date (the season's textures follow). *Permission: `world_write`.* | 0.2.22 |
| `omsi.world.set_time(time)` | boolean | Sets the time of day: seconds since midnight or `"HH:MM"` / `"HH:MM:SS"`, as the game menu's clock does (the timetable starts again after a jump of minutes). Not in a LAN game as a client, nor while the clock follows the computer's. *Permission: `world_write`.* | 0.2.22 |
| `omsi.world.set_time_speed(factor)` | boolean | Sets how much faster the clock runs (1 to 30; not in a LAN game). *Permission: `world_write`.* | 0.2.22 |
| `omsi.world.sun_altitude()` | number or nil | The sun's height over the horizon, degrees. | 0.2.22 |
| `omsi.world.time()` | number or nil | The game's time of day, seconds since midnight. | 0.2.22 |
| `omsi.world.time_speed()` | number or nil | How much faster than real time the game's clock runs (1 to 30). | 0.2.22 |

#### Weather

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.weather.get()` | table or nil | The weather now: `{name, visibility (m), wind_direction (°), wind_speed (m/s), temperature (°C), humidity (%), absolute_humidity (g/m³), pressure (hPa), clouds, cloud_base (m), precipitation ("none", "rain", "snow"), precipitation_rate (0..1), snow_cover, snow_on_road, wetness (the roads, 0..1), changing, locked}` (`locked`: it follows a real weather station and cannot be set). | 0.2.22 |
| `omsi.weather.precipitation()` | kind, rate | `"none"`, `"rain"` or `"snow"`, and how hard (0 to 1). | 0.2.22 |
| `omsi.weather.preset(file, [seconds])` | true, or false and the reason | Changes to a weather file (as `weather.presets` names it; `""`: the map's own, changing with the day) over `seconds` (default 1). *Permission: `world_write`.* | 0.2.22 |
| `omsi.weather.presets()` | list of tables | The weather files installed: `{file, name}`. | 0.2.22 |
| `omsi.weather.set(values)` | true, or false and the reason | Changes the weather as the game menu's sliders do; any of `visibility`, `wind_direction`, `wind_speed`, `temperature`, `pressure`, `clouds` (`"-1"` none, `"Cumulus 1"`..`"3"`, `"Overcast 1"`), `cloud_base`, `precipitation` (`"none"`, `"rain"`, `"snow"`), `precipitation_rate` (0..1), `snow_cover`, `snow_on_road`, `wetness`; the rest stays. Held to the game's ranges; refused in a LAN game as a client or while the weather follows a station. *Permission: `world_write`.* | 0.2.22 |
| `omsi.weather.temperature()` | number or nil | The air temperature, °C. | 0.2.22 |
| `omsi.weather.visibility()` | number or nil | How far one sees, metres. | 0.2.22 |
| `omsi.weather.wetness()` | number or nil | How wet the roads are, 0 (dry) to 1. | 0.2.22 |
| `omsi.weather.wind()` | direction, speed | The wind: where it comes from (degrees) and its speed (m/s). | 0.2.22 |

#### Camera

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.camera.fov()` | number or nil | Its vertical field of view, degrees. | 0.2.22 |
| `omsi.camera.get()` | table or nil | The camera: `{view, x, y, z, yaw, pitch, roll, fov, in_cab, zoom, look_yaw, look_pitch, width, height}` - map metres, degrees (yaw clockwise from north, pitch up positive), the vertical field of view, the picture's pixels. | 0.2.22 |
| `omsi.camera.in_cab()` | boolean | Whether the camera is in the player's own bus (driver or passenger view). | 0.2.22 |
| `omsi.camera.look(yaw, pitch)` | boolean | Turns the head (driver, passenger view) or swings the outside camera round the bus: degrees from straight ahead. *Permission: `camera`.* | 0.2.22 |
| `omsi.camera.orientation()` | yaw, pitch, roll | Where it looks, degrees. | 0.2.22 |
| `omsi.camera.position()` | x, y, z | Where the camera is (map metres). | 0.2.22 |
| `omsi.camera.project(x, y, z)` | screen_x, screen_y | Where a map point is seen on the screen, in the panels' pixels (as `ui.screen` measures them); nothing when it is behind the camera. For labels over buses, stops, people. | 0.2.22 |
| `omsi.camera.set_free(x, y, z, [yaw], [pitch])` | boolean | Puts the free camera at a map point looking along `yaw` and `pitch` (degrees); the view becomes `"free"` (the player moves it on from there). *Permission: `camera`.* | 0.2.22 |
| `omsi.camera.set_view(view)` | boolean | Switches the view: `"driver"`, `"pax"`, `"outside"`, `"map"` (the free camera above the bus), `"ego"` (walking), or a `view_*` action of keyboard.cfg. *Permission: `camera`.* | 0.2.22 |
| `omsi.camera.set_zoom(zoom)` | boolean | The zoom of the view now (its field of view times this, 0.2 to 3). *Permission: `camera`.* | 0.2.22 |
| `omsi.camera.view()` | string or nil | The view: `"driver"`, `"pax"`, `"outside"`, `"free"` or `"foot"`. | 0.2.22 |

#### Input

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.input.bindings([vehicles])` | list of tables | The key bindings: `{action, key}` of the game's keys, or the vehicles' with `true` (`key` as the game writes it: `"Ctrl+D"`). | 0.2.22 |
| `omsi.input.controllers()` | list of tables | The steering wheels, pedals, joysticks and gamepads: `{name, gamepad, axes, buttons}` (`axes` each axis's value; `buttons` how many it has - the `controller_button` event tells presses). | 0.2.22 |
| `omsi.input.hotkey(keys, fn)` | integer id | Runs `fn(key)` when a key combination is pressed: `"F10"`, `"Ctrl+KeyH"`, `"Shift+Alt+Digit1"` (modifiers exact: `"KeyH"` is not `Ctrl+KeyH`). The keys still reach the bus. `cancel(id)` removes it. | 0.2.22 |
| `omsi.input.key_down(key)` | boolean | Whether a key is held now (winit's names, as the `key` event: `"KeyW"`, `"ShiftLeft"`, `"F5"`). | 0.2.22 |
| `omsi.input.keys_down()` | list of strings | Every key held now. | 0.2.22 |
| `omsi.input.mouse()` | x, y, left, right, middle | The mouse: where it is in the panels' pixels and its buttons held. | 0.2.22 |

#### Sound

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.audio.play(file, [opts])` | integer id or nil, reason | Plays a WAV file of the plugin's folder. `opts`: `volume` (1), `pitch` (1), `loop`, `range` (metres heard at full volume, 5), and either `x, y, z` (a sound at a map point) or `on_bus = true` (it moves with the player's bus); none: heard alike everywhere. The game's volume setting applies. A plugin plays 32 at most. *Permission: `audio`.* | 0.2.22 |
| `omsi.audio.playing(id)` | boolean | Whether a sound of the plugin's still plays. | 0.2.22 |
| `omsi.audio.set(id, opts)` | boolean | Changes a sound of the plugin's: `volume`, `pitch`, `range`, `x, y, z` / `on_bus`. *Permission: `audio`.* | 0.2.22 |
| `omsi.audio.stop(id)` | boolean | Stops a sound of the plugin's. *Permission: `audio`.* | 0.2.22 |
| `omsi.audio.stop_all()` | nil | Stops every sound of the plugin's. *Permission: `audio`.* | 0.2.22 |
| `omsi.audio.volume()` | number or nil | The game's volume setting, 0 to 1. | 0.2.22 |

#### On screen

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.message(text, [seconds])` | nil | A line of text on the screen (5 seconds when not given). *Permission: `ui`.* | 0.1.5 |
| `omsi.ui.clear()` | nil | Removes every panel of the plugin. *Permission: `ui`.* | 0.2.21 |
| `omsi.ui.focus(on)` | boolean | `true`: the panels get the mouse (the cursor shows, a click goes to the panel under it and none to the bus); `false`, Esc or a menu of the game gives it back. Returns the new state. *Permission: `ui`.* | 0.2.21 |
| `omsi.ui.focused()` | boolean | Whether the panels have the mouse. | 0.2.21 |
| `omsi.ui.moved(panel)` | dx, dy | How far the player dragged a panel from where its table puts it (pixels). | 0.2.22 |
| `omsi.ui.panels()` | list of strings | The ids of the plugin's panels. | 0.2.22 |
| `omsi.ui.remove(id)` | boolean | Removes a panel; `true` when there was one. *Permission: `ui`.* | 0.2.21 |
| `omsi.ui.screen()` | width, height, scale | The screen in the panels' pixels, and how many of the screen's own pixels one of them is. | 0.2.21 |
| `omsi.ui.set(id, panel)` | true, or false and the reason | Creates the panel `id` or replaces it; a table that is not right gives `false` and where (`"children[2].size: a number is expected"`). The same table again changes nothing. *Permission: `ui`.* | 0.2.21 |
| `omsi.ui.show(panel, [on])` | boolean | Shows a panel (or hides it with `false`; it is kept); `false` when there is none. *Permission: `ui`.* | 0.2.22 |
| `omsi.ui.toast(text, [opts])` | true, or false and the reason | A notification card at the top right, newest at the top; it goes after `opts.seconds` (1 to 60, default 5). `opts`: `title`, `icon`, `color`. A plugin shows 8 at most: a ninth makes its oldest go. *Permission: `ui`.* | 0.2.21 |
| `omsi.ui.toggle(panel)` | boolean | Shows a hidden panel or hides a shown one; whether it shows now. *Permission: `ui`.* | 0.2.22 |
| `omsi.ui.typing()` | boolean | Whether the player types into a text field of a plugin now (the keys then go to the field, not to the bus). | 0.2.22 |
| `omsi.ui.update(panel, element, values)` | true, or false and the reason | Changes one element of a panel in place, by its id: `text`, `color`, `value` (a bar, a slider), `checked`, `selected`, `name` (an icon), `values` (a chart), `rows` (a table), `src` (an image) - cheaper than setting the whole panel again. *Permission: `ui`.* | 0.2.22 |
| `omsi.ui.value(panel, element)` | any | The value of a checkbox, slider, text field or tabs element now (as `ui_change` gives it). *Permission: `ui`.* | 0.2.22 |

#### Events, timers and watches

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.after(seconds, fn)` | integer id | Runs `fn` once, `seconds` of game time later. | 0.1.5 |
| `omsi.cancel(id)` | boolean | Stops a timer, a watch or a hotkey; `true` when there was one. | 0.1.5 |
| `omsi.emit(event, [...])` | nil | Sends an event to this plugin's own handlers at once (handy between the modules of a bigger plugin); an error of a handler is this call's. | 0.1.5 |
| `omsi.every(seconds, fn)` | integer id | Runs `fn` every `seconds` of game time. | 0.1.5 |
| `omsi.off(event, fn)` | nil | Removes a handler added with `on`. | 0.1.5 |
| `omsi.on(event, fn)` | the function | Adds a handler of an event (see the events); several may hear one event, in the order they were added. | 0.1.5 |
| `omsi.time()` | number | Seconds of game time since the plugin started (stands still while paused). | 0.1.5 |
| `omsi.watch(kind, name, [fn])` | integer id | Runs `fn(new, old)` whenever a value changes: `watch(name, fn)` a variable of the bus, `watch(kind, name, fn)` of kind `"var"`, `"str"`, `"sys"` or `"info"` (a key of `info()`). | 0.1.5 |

#### The plugin itself

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.plugin.broadcast(topic, [data])` | nil | Sends a message to every other plugin loaded. | 0.2.22 |
| `omsi.plugin.disable([reason])` | nil | Switches the plugin off until its file changes or the game starts again (its `stop` comes). | 0.2.22 |
| `omsi.plugin.errors()` | integer | How many errors the plugin had (at 10 it is switched off). | 0.2.22 |
| `omsi.plugin.files([dir])` | list of tables | The files of the plugin's own folder (or a folder in it): `{name, dir, size}`. | 0.2.22 |
| `omsi.plugin.list()` | list of strings | The names of the plugins loaded, this one too. | 0.2.22 |
| `omsi.plugin.name()` | string | The plugin's name: its file's, or its folder's for a `main.lua`. | 0.2.22 |
| `omsi.plugin.permissions()` | list of strings | The permissions the plugin has (a plain `.lua` file has them all). | 0.2.22 |
| `omsi.plugin.read(path)` | text, or nil and the reason | A file of the plugin's own folder (a table of stops, a translation): read only, relative to the folder. | 0.2.22 |
| `omsi.plugin.send(to, topic, [data])` | nil | Sends a message to another plugin (by its name): it hears `message(from, topic, data)` in its next frame. The data is numbers, texts, booleans or tables of them. | 0.2.22 |
| `omsi.plugin.set_setting(key, value)` | boolean | Changes a setting (held to its range; saved); `true` when it took the value. | 0.2.22 |
| `omsi.plugin.setting(key)` | any | A setting's value (`nil`: no such setting). | 0.2.22 |
| `omsi.plugin.settings(settings, [title])` | table | Declares the plugin's settings: a list of `{key, type, label, default}` with `type` `"bool"`, `"number"` (`min`, `max`, `step`), `"text"` or `"choice"` (`choices`, a list of texts). The values the player chose before come back (as a table key -> value); the game makes a settings panel of them (`plugin.show_settings`), saves them in the data folder and sends `setting(key, value)` when one changes. | 0.2.22 |
| `omsi.plugin.show_settings([on])` | boolean | Shows the plugin's settings panel (or hides it with `false`); it can be dragged and closed while the panels have the mouse (`ui.focus`). `false` when the plugin declared no settings. *Permission: `ui`.* | 0.2.22 |

#### Storage and files

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.files.append(path, data)` | true, or false and the reason | Adds to the end of a file of the data folder (a log, a CSV of trips). *Permission: `storage`.* | 0.2.22 |
| `omsi.files.delete(path)` | boolean | Removes a file or an empty folder of the data folder. *Permission: `storage`.* | 0.2.22 |
| `omsi.files.dir()` | string | Where the data folder is on this computer (to tell the player; the plugin reaches it with relative paths only). *Permission: `storage`.* | 0.2.22 |
| `omsi.files.exists(path)` | boolean | Whether the data folder has this file or folder. *Permission: `storage`.* | 0.2.22 |
| `omsi.files.list([dir])` | list of tables, or nil and the reason | The entries of the data folder (or a folder in it): `{name, dir, size}`. *Permission: `storage`.* | 0.2.22 |
| `omsi.files.mkdir(path)` | boolean | Makes a folder (and those above it) in the data folder. *Permission: `storage`.* | 0.2.22 |
| `omsi.files.read(path)` | text, or nil and the reason | A file of the plugin's data folder (`path` relative to it; `..` and absolute paths are refused). *Permission: `storage`.* | 0.2.22 |
| `omsi.files.write(path, data)` | true, or false and the reason | Writes a file of the data folder (its folders are made); at most 16 MB at once and 256 MB in all. *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.all()` | table | Everything stored, as one table. *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.clear()` | nil | Removes every key. *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.delete(key)` | boolean | Removes a key; `true` when it was there. *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.get(key)` | any | A value the plugin stored, or `nil`. The storage is the plugin's own and survives the session (kept as `storage.json` in its data folder). *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.keys()` | list of strings | The keys stored, in the order they were first set. *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.save()` | nil | Writes the storage now (it is written by itself when the game ends). *Permission: `storage`.* | 0.2.22 |
| `omsi.storage.set(key, value)` | nil | Stores a value under a key: a number, text, boolean or a table of them (`nil` removes it). It is written when the game ends, the plugin is loaded again, or `storage.save()` is called. *Permission: `storage`.* | 0.2.22 |

#### Programs on this computer

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.send(port, data)` | true, or false and the reason | Sends `data` as one UDP datagram to `127.0.0.1:port`: to another program on this computer, never over the network. Not sent when the port is below 1024 or one of the game's multiplayer ports (27015-27024), the message is longer than 8 KB, or the plugin sent 100 in the last second. *Permission: `network_local`.* | 0.2.21 |

#### LAN games

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.lan.active()` | boolean | Whether this game is in a LAN session. | 0.2.22 |
| `omsi.lan.chat(text)` | true, or false and the reason | Says a line in the session's chat, as the player would (every player sees it; at most one a second). *Permission: `lan`.* | 0.2.22 |
| `omsi.lan.me()` | id, name, host | This player in the session: its id (the host is 1), its name, whether it hosts. | 0.2.22 |
| `omsi.lan.players()` | list of tables | The other players of the session: `{id, name, host, bus, line, tour, x, y, z, heading, speed, on_foot, passengers}`. | 0.2.22 |
| `omsi.lan.send(to, text)` | true, or false and the reason | Sends a short text (at most 120 characters, no `\|`) to the same plugin on another player's game (`to` its id; 0: every other player); it hears `lan_message(from, text)`. A player sends at most about ten a second; a game without the plugin ignores them. *Permission: `lan`.* | 0.2.22 |

#### The game

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.command(name)` | boolean | Does what a line of the game menu does: `refuel`, `wash`, `repair`, `shot`, `save`, `load`, `weather`, `later`, `earlier`, `info`, `timetable`, `reset`, `couple`, `uncouple`; `true` when the game knows it (it runs after the frame). *Permission: `world_write`.* | 0.1.10 |
| `omsi.debug([...])` | nil | A line of the log's debug level (shown with `RUST_LOG=debug`). | 0.2.22 |
| `omsi.error([...])` | nil | The same as an error line (the plugin goes on). | 0.2.22 |
| `omsi.game.action(name)` | boolean | A game action of keyboard.cfg's `[game]` (`sim_pause`, `view_set_map`, `view_toggle_informationdisplay`, `view_set_schedule`, ...), as its key does; `true` when the game knows it. *Permission: `world_write`.* | 0.2.22 |
| `omsi.game.api()` | integer | The version of the plugin interface (`api_abi` of an `.oop`; raised only when something is taken away or changes). | 0.2.22 |
| `omsi.game.fps()` | number or nil | Frames a second now. | 0.2.22 |
| `omsi.game.has(name)` | boolean | Whether this game has an API function (`"weather.set"`): for a plugin that should also run in an older openOMSI. | 0.2.22 |
| `omsi.game.menu_open()` | boolean | Whether the game menu is open. | 0.2.22 |
| `omsi.game.notify(text, [kind], [seconds])` | boolean | A notification of the game's own (the cards the server's messages use): `kind` `"info"`, `"warning"` or `"alert"`. *Permission: `ui`.* | 0.2.22 |
| `omsi.game.platform()` | string | The system the game runs on: `"windows"`, `"macos"`, `"linux"`, `"android"`. | 0.2.22 |
| `omsi.game.screenshot()` | string or nil | Takes a screenshot with the next frame, as the camera key does; the file it goes to (the `screenshot` event says when it is there). *Permission: `ui`.* | 0.2.22 |
| `omsi.game.settings()` | table | The settings a plugin may read: `graphics` (`"vanilla"`, `"vanilla_plus"`, `"enhanced"`), `language` (the cockpit's, `"ENG"`), `ui_language`, `ui_scale`, `volume`, `fov`, `render_scale`, `max_fps`, `fullscreen`, `vsync`, `msaa`, `shadows`, `time_speed`, `units` (`"metric"`: speeds are km/h everywhere), ... | 0.2.22 |
| `omsi.game.stats()` | table | This session's counts, as the personnel file has them: `km`, `stops_served`, `stops_skipped`, `crashes`, `heavy_crashes`, `pedestrians`, `tickets`, `cash`, `passengers`, ... | 0.2.22 |
| `omsi.game.version()` | string | The game's version (`"0.2.22"`). | 0.2.22 |
| `omsi.info()` | table | What the game is doing: `map`, `clock` (seconds since midnight), `day`, `year`, `view`, `paused`, `on_foot`, `multiplayer`, `traffic`, `speed`, `delay`, `map_path`, `version`; with a bus also `tile_x`, `tile_y`, `tile_pos_x`, `tile_pos_y`, `heading`, `vehicle_manufacturer`, `vehicle_model`, `destination`, `passengers`; `crashes`, `heavy_crashes`, `pedestrians_hit`; `situation`; on a duty also `line`, `tour`, `trip`, `trips`, `trip_name`, `terminus`, `stops`, `trip_done`, `next_stop`, `next_stop_number`, `next_stop_arrival`, `next_stop_departure`, `next_stop_id`, `at_stop`, `previous_stop`, `previous_stop_id`, `next_stop_distance`, `previous_stop_distance` (see the plugin docs for each). | 0.1.10 |
| `omsi.info_value(key)` | any | One value of `info()` without building the whole table: cheaper for a plugin that reads one or two every frame. | 0.2.22 |
| `omsi.log([...])` | nil | A line in `game.log`, tagged `[lua <name>]` (`print` does the same). | 0.1.5 |
| `omsi.warn([...])` | nil | The same as a warning. | 0.1.5 |

#### Helpers

| Function | Returns | What it does | Since |
| --- | --- | --- | --- |
| `omsi.fmt.clock(seconds, [with_seconds])` | string | Seconds since midnight as `"HH:MM"` (`"HH:MM:SS"` with `true`); past midnight wraps. | 0.2.22 |
| `omsi.fmt.delay(seconds)` | string | A delay as a timetable display shows it: `"+2:30"` late, `"-0:45"` early, `"0:00"`. | 0.2.22 |
| `omsi.fmt.distance(metres)` | string | A distance as `"350 m"` or `"2.4 km"`. | 0.2.22 |
| `omsi.fmt.duration(seconds)` | string | A length of time as people read it: `"45 s"`, `"3 min 05 s"`, `"1 h 20 min"` (negative with a minus). | 0.2.22 |
| `omsi.fmt.money(amount, [symbol])` | string | An amount with two decimals and a currency symbol after it (`"12.50 €"`; the game knows no currency of its own). | 0.2.22 |
| `omsi.fmt.number(x, [decimals], [separator])` | string | A number with `decimals` (0) and a thousands `separator` (`","`, `" "`; none by default). | 0.2.22 |
| `omsi.fmt.pad(text, width, [right])` | string | A text made `width` characters long with spaces (on the left with `right` = true, to line numbers up), or cut to it. | 0.2.22 |
| `omsi.fmt.speed(kmh, [unit])` | string | A speed as `"42 km/h"`, or in `"mph"` or `"m/s"`. | 0.2.22 |
| `omsi.fmt.split(text, [separator])` | list of strings | A text cut at every `separator` (`","` by default; plainly, no patterns). | 0.2.22 |
| `omsi.fmt.trim(text)` | string | A text without the spaces at its ends. | 0.2.22 |
| `omsi.json.decode(text)` | value, or nil and the reason | Reads JSON text: objects and arrays become tables, `null` nil. | 0.2.22 |
| `omsi.json.encode(value, [pretty])` | string | A value as JSON text (a list as an array, a table with keys as an object; `pretty`: indented). | 0.2.22 |
| `omsi.util.date([seconds])` | table | A real time (`util.now()` by default) as `{year, month, day, hour, minute, second, weekday}` in UTC (`weekday` 1 Monday). | 0.2.22 |
| `omsi.util.ms()` | number | Milliseconds of real time since the game started: for timing a plugin's own work. | 0.2.22 |
| `omsi.util.now()` | number | The real time: seconds since 1970 (UTC), with fractions. | 0.2.22 |
| `omsi.util.random([min], [max])` | number | A random number: 0 to 1 without arguments, else a whole number from `min` to `max` (as `math.random`, but not repeating the same row in every plugin). | 0.2.22 |
| `omsi.vec.angle_diff(a, b)` | number | The turn from heading `a` to heading `b`, -180 to 180 degrees (right positive). | 0.2.22 |
| `omsi.vec.bearing(x1, y1, x2, y2)` | number | The heading from one map point to another. | 0.2.22 |
| `omsi.vec.clamp(x, min, max)` | number | `x` held between `min` and `max`. | 0.2.22 |
| `omsi.vec.distance(x1, y1, x2, y2)` | number | Metres between two map points (on the ground: x and y). | 0.2.22 |
| `omsi.vec.distance3(x1, y1, z1, x2, y2, z2)` | number | Metres between two points in space. | 0.2.22 |
| `omsi.vec.dot(x1, y1, x2, y2)` | number | The dot product of two 2D vectors. | 0.2.22 |
| `omsi.vec.heading(dx, dy)` | number | The heading of a direction on the map, degrees clockwise from north (0 to 360), as the game's headings. | 0.2.22 |
| `omsi.vec.length(x, y, [z])` | number | The length of a vector (2D, or 3D with `z`). | 0.2.22 |
| `omsi.vec.lerp(a, b, t)` | number | From `a` to `b` by `t` (0 to 1, not held to it). | 0.2.22 |
| `omsi.vec.normalize(x, y, [z])` | x, y, z | The vector made 1 long (0 stays 0). | 0.2.22 |
| `omsi.vec.rotate(x, y, degrees)` | x, y | A vector turned clockwise by `degrees` (as headings turn). | 0.2.22 |
| `omsi.vec.to_local(x, y, ox, oy, heading)` | right, ahead | A map point seen from a place facing `heading`: metres to the right and ahead of it. | 0.2.22 |

#### Events

| Event | Arguments | When | Since |
| --- | --- | --- | --- |
| `start` | - | Right after the plugin was loaded (also after a reload). | 0.1.5 |
| `vehicle` | `name` | The player got into a vehicle, changed it, or left it (`nil`). | 0.1.5 |
| `frame` | `dt` | Every frame of the game, after the bus's own scripts; not while paused. `dt` is the frame's seconds of game time. | 0.1.5 |
| `stop` | - | The game ends, or the plugin is about to be loaded again. | 0.1.5 |
| `key` | `key`, `down` | A key went down (`true`) or came up: winit's name of it (`"KeyH"`, `"F5"`, `"Numpad8"`). | 0.1.10 |
| `next_stop` | `new`, `old` | The duty's next stop changed, also to one of the same name (`omsi.info().next_stop_number` tells them apart). | 0.1.10 |
| `view` | `new`, `old` | The view changed (`"driver"`, `"pax"`, `"outside"`, `"free"`, `"foot"`). | 0.1.10 |
| `duty` | `line`, `tour` | A line and tour were taken (or given up: `nil`). | 0.1.10 |
| `crash` | `energy_kj`, `speed_kmh` | The player's bus crashed: every crash, also one the same as the last (the screen's "Crash: 136 kJ"); above 50 kJ it is a heavy one. | 0.2.21 |
| `pedestrian` | `count` | The bus knocked people down. | 0.2.21 |
| `stops_skipped` | `count`, `due_at`, `now_at` | The duty jumped ahead: the bus passed stops of its trip without stopping (or was moved) and is now at a later one; the stops are numbered in the trip from 1. | 0.2.21 |
| `service` | `kind`, `by`, `amount` | The player's bus was serviced or moved. `kind`: `"refuel"` (amount: litres put in), `"wash"` (amount: the dirt left), `"repair"` (amount: the game minutes it took), `"reset"` or `"teleport"`; `by`: `"player"`, `"plugin"`, `"host"` or `"game"`. | 0.2.21 |
| `trip_done` | `trip`, `how`, `driving`, `comfort`, `tickets` | A trip of the duty ended, once: its number in the duty, `"arrived"`, `"skipped"` or `"given_up"`, then its ratings in per cent (driving, comfort, ticket selling). | 0.2.21 |
| `jolt` | `along`, `across`, `speed_kmh`, `passengers` | The bus braked, sped up or cornered hard enough to cost driving rating (m/s², signed); at most one a second. | 0.2.21 |
| `ticket_sold` | `name`, `price` | A ticket was sold at the cash desk: its name and price as the bus's ticket list has them. | 0.2.21 |
| `ui_click` | `panel`, `element` | A button (or another clickable part) of one of the plugin's panels was clicked; `element` is `nil` for the panel itself. Enter in a text field is a click on it. | 0.2.21 |
| `ui_focus` | `focused` | The panels got the mouse or gave it back (also by Esc, or a menu of the game opening). | 0.2.21 |
| `ui_change` | `panel`, `element`, `value` | A checkbox (`true`/`false`), slider (its number), text field (its text, at every key) or tabs element (the tab's number from 1) of the plugin's panels was changed by the player. | 0.2.22 |
| `message` | `from`, `topic`, `data` | Another plugin sent this one a message (`plugin.send`, `plugin.broadcast`): its name, the topic and the data (numbers, texts, tables). | 0.2.22 |
| `setting` | `key`, `value` | The player changed one of the plugin's settings in its settings panel (`plugin.settings`). | 0.2.22 |
| `door` | `door`, `open` | A door leaf of the player's bus opened (`true`) or closed: its number from 1, front to back. | 0.2.22 |
| `doors` | `open` | The bus's doors opened (`true`: one or more open) or were all closed. | 0.2.22 |
| `engine_start` | - | The player's bus's engine started running. | 0.2.22 |
| `engine_stop` | - | Its engine stopped. | 0.2.22 |
| `gear` | `new`, `old` | The gear engaged changed (-1 reverse, 0 neutral), where the bus has a gearbox it shows. | 0.2.22 |
| `indicator` | `new`, `old` | The indicators changed: `"off"`, `"left"`, `"right"` or `"hazard"`. | 0.2.22 |
| `headlights` | `new`, `old` | The headlights changed: 0 off, 1 side lights, 2 dipped, 3 high beam. | 0.2.22 |
| `horn` | `down` | The horn started (`true`) or stopped. | 0.2.22 |
| `handbrake` | `on` | The parking brake was put on (`true`) or released. | 0.2.22 |
| `stop_request` | `on` | A passenger asked to stop (`true`), or the request went out. | 0.2.22 |
| `passengers` | `count`, `old` | The number of passengers aboard the player's bus changed. | 0.2.22 |
| `passenger_board` | `count`, `stop_id` | People got into the player's bus: how many, and the stop they waited at (its map object id, `nil` when none). | 0.2.22 |
| `passenger_alight` | `count` | People got out of the player's bus. | 0.2.22 |
| `stop_arrive` | `name`, `id`, `number`, `delay` | The bus came to the duty's next stop (within 25 m of it): its name, map object id, number in the trip (from 1) and the delay in seconds (late positive). | 0.2.22 |
| `stop_depart` | `name`, `id`, `number`, `delay` | The bus left the stop it stood at (more than 35 m from it, or the duty moved on): the same values, the delay as it left. | 0.2.22 |
| `trip_start` | `trip`, `name`, `terminus` | The duty moved on to another trip: its number in the duty (from 1), the timetable's name of the trip and its terminus. | 0.2.22 |
| `duty_start` | `line`, `tour` | A duty was taken. | 0.2.22 |
| `duty_end` | `line`, `tour` | The duty was given up (or another taken: `duty_end` of the old one comes first). | 0.2.22 |
| `destination` | `new`, `old` | The destination the bus shows changed. | 0.2.22 |
| `coupled` | `parts`, `old` | Something was coupled to or uncoupled from the bus: the parts behind it now. | 0.2.22 |
| `minute` | `hour`, `minute` | The game's clock reached a new minute (also when it was set). | 0.2.22 |
| `hour` | `hour` | The game's clock reached a new hour. | 0.2.22 |
| `day` | `year`, `month`, `day` | The game's date changed. | 0.2.22 |
| `tile` | `x`, `y`, `old_x`, `old_y` | The player's bus drove onto another tile of the map (numbered as global.cfg's `[map]` list). | 0.2.22 |
| `weather` | `name` | The weather changed: another weather file, rain or snow beginning or ending, the snow cover, the clouds. | 0.2.22 |
| `light_ahead` | `aspect`, `distance` | The traffic light ahead of the bus (within 80 m) changed or a new one came: its aspect (`"red"`, `"red_yellow"`, `"green"`, `"green_yellow"`, `"yellow"`, `"dark"`, `nil` when none is ahead any more) and its distance in metres. | 0.2.22 |
| `red_light` | `speed_kmh` | The bus drove past a traffic light showing red (or red and yellow) at more than 5 km/h. | 0.2.22 |
| `speed_limit` | `kmh`, `old` | The speed limit of the lane the bus drives on changed. | 0.2.22 |
| `ai_collision` | `id`, `energy_kj` | The player's bus hit an AI vehicle (its id, as `traffic.list` gives it) with this energy. | 0.2.22 |
| `pause` | - | The game was paused (the plugins stand still until `resume`; nothing else comes in between). | 0.2.22 |
| `resume` | - | The game goes on after a pause. | 0.2.22 |
| `menu_open` | - | The game menu opened. | 0.2.22 |
| `menu_close` | - | The game menu closed. | 0.2.22 |
| `screenshot` | `file` | A screenshot was taken: its file. | 0.2.22 |
| `controller_button` | `device`, `button`, `down` | A button of a steering wheel, joystick or gamepad went down (`true`) or up: the device's name and the button's number. | 0.2.22 |
| `lan_join` | `id`, `name` | A player joined the LAN session (or came back). | 0.2.22 |
| `lan_leave` | `id`, `name` | A player left the LAN session. | 0.2.22 |
| `lan_message` | `from`, `data` | The same plugin on another player's game sent this one a message (`lan.send`): the player's id and the text. | 0.2.22 |
| `lan_chat` | `name`, `text` | A line was said in the LAN session's chat (by another player or this one). | 0.2.22 |

<!-- api:end -->

## Compiled plugins (`.oop`)

An `.oop` (openOMSI Plugin) is to a plugin what a DLL is to a program, but sandboxed: build
output, not sources. `oopc build` of the [openOMSI Development
Tools](https://github.com/openOMSI-org/openOMSI-Development-Tools) makes one from a plugin
project, and the game loads `plugins/<name>.oop` like the other plugins. Two kinds:

* **Lua**, compiled into one obfuscated chunk (its locals renamed, its strings encoded, its
  modules bundled): it behaves as the sources did, through the same API, but the sources are
  not in the file. Compiled Lua *bytecode* is refused - Lua checks no bytecode, and crafted
  bytecode could break out of the sandbox.
* **WebAssembly** (`wasm32-unknown-unknown`), usually Rust with the plugin SDK of the
  Development Tools: the module calls every function of the API by its name here
  (`openomsi.call("ui.set", ...)`, the SDK wraps them all) and gets the same events, timers
  and watches. It runs in an interpreter (wasmi) on every platform the game runs on; one call
  into it that runs too long stops the plugin, not the game, and it may use 256 MB of memory.
  A callback that the module's own call raises (its `emit` reaching its own handler) runs
  right after that call returns.

What the game does with an `.oop`:

* It checks the file whole before anything of it runs: a file changed after it was built, cut
  short or made for a newer plugin API is refused (the log says why).
* The code stays in memory; the plugin's pictures, sounds and data files go into
  `plugins/.oop-cache/<name>/`, where its API functions read them.
* **Permissions.** An `.oop` declares what it may do (`ui`, `storage`, `vehicle_write`,
  `traffic_write`, `world_write`, `camera`, `audio`, `network_local`, `lan`); a function
  that needs another one fails with an error the plugin can catch. Reading the game needs
  none. (A plain `.lua` file keeps every permission, as before.)
* **Signatures.** A signed plugin is logged with its author's key fingerprint
  (`signed by 3097:e2de:e2cb:4a34`); an unsigned one with a warning that its builder is not
  known.
* A `.lua` plugin of the same name wins over an `.oop` (the copy being worked on).

How safe the code is from copying: the file is encrypted, and the key is in the game, so it
can be decrypted with effort - but what one gets then is the stripped or obfuscated build
output, never the author's sources, which are not in the file.

## The telemetry file (for programs beside the game)

A program that only wants to follow the player's bus - a fleet map, an in-vehicle
terminal, a stream overlay - can read `~/.openomsi/telemetry.json` (on Windows
`%USERPROFILE%\.openomsi\telemetry.json`) instead of being a plugin. The game writes it
**only while the file exists**: the program (or you) creates it once, empty, and the game
fills it from then on; deleting it stops the writing. It stays on this computer.

About twice a second the file is replaced as a whole (written beside it and renamed), so a
reader never sees half of it. It holds (`version` 2): `pid`, `updated` (Unix seconds),
`paused`; `player` (`null` without a bus): `x`, `y`, `z`, `tile_x`, `tile_y`, `local_x`,
`local_y`, `heading`, `speed_kmh`, `delay_s`, `passengers`; `duty` (`null` without one):
`line`, `tour`, `trip`, `trip_index`, `trip_count`, `terminus`, `next_stop_index`,
`next_stop_id`, `next_stop_name`, `next_stop_dist`, `prev_stop_id`, `at_stop` and `stops`
(each `object_id`, `name`, `stops`, `arr`, `dep`, `x`, `y`); `ai_buses`, the timetable
buses on the road (`id`, `line`, `tour`, `trip`, `terminus`, `depart`, `next_stop_id`,
`at_stop`, `trip_done`, `delay_s`, `x`, `y`, `number`). The stop IDs are the map's, as in
`omsi.info()`. A file whose `updated` stops moving belongs to a game that ended.

## OMSI plugins (`plugins/*.opl` + DLL)

What OMSI does with plugins, and how
openOMSI does the same (`crates/omsi-plugin`, driven from `crates/omsi-app/src/plugins.rs`).

### The original

* **Finding them**: every `*.opl` under `<OMSI>\plugins`, recursively
  (`FindFilesRecursive`). Tags: `[dll]` (a path relative to `plugins\`), `[varlist]`,
  `[stringvarlist]`, `[systemvarlist]`, `[triggers]` - each a count, then that many names.
* **Loading**: `LoadLibrary`, then `GetProcAddress` for
  `PluginStart` and `PluginFinalize` (required - "Could not load plugin …: procedure … not
  found!") and `AccessVariable`, `AccessTrigger`, `AccessSystemVariable`,
  `AccessStringVariable` (optional - "Loading plugin …: procedure … not found!"). Then
  `PluginStart(AOwner)`. `PluginFinalize` runs when the game ends.
* **Every frame**, plugin by plugin:
  1. each listed system variable: `AccessSystemVariable(index: Word; var value: Single;
     var write: Boolean)`; written back when `write` is true;
  2. with a player vehicle: each listed vehicle variable (`AccessVariable`, same shape);
  3. each string variable: `AccessStringVariable(index: Word; text: PWideChar;
     var write: Boolean)` - a buffer of length + 1 wide characters with the text and its
     terminating zero; read back when `write` is true;
  4. each trigger: `AccessTrigger(index: Word; var active: Boolean)`, `active` false before
     the call. A change from the last frame is a key event: down fires the
     trigger, up fires `<trigger>_off`.

  All `stdcall`; `index` is the position in the plugin's own list. Names the vehicle does
  not have are skipped.

### openOMSI

* `omsi_plugin::Plugins::load` reads the `plugins` folder of every content root (the
  first root's copy of an `.opl` wins) and loads each library:
  * **in-process** when the running program can load it (same system and architecture -
    a plugin built for openOMSI, or a 32-bit DLL in a 32-bit Windows build);
  * otherwise in **`omsi-plugin-host32.exe`**, `omsi-plugin-host` built for 32-bit Windows,
    which loads the DLL and answers over stdin/stdout (one round trip per frame). On
    Windows it runs directly; on macOS and Linux through Wine (`wine` on the `PATH`, or
    `OMSI_WINE`). The host is found next to the game (`OMSI_PLUGIN_HOST32` overrides).
* The system variables are the scripts' (`omsi_script::SysVar`); a plugin's writes to
  them are not applied (the clock, weather and input stay the game's).
* **`openomsi_<key>`** in a `[varlist]` or `[stringvarlist]` reads the value `<key>` of
  `omsi.info()` (see above): numbers and booleans as variables, texts as string variables,
  while the player drives a bus. A plugin that reads the bus's place, its type or the map
  out of Omsi.exe's memory at fixed addresses - which cannot work here - lists
  `openomsi_tile_x`, `openomsi_tile_pos_x`, `openomsi_heading`, `openomsi_map_path`... instead.
  OMSI has no variables of these names and skips them, so one `.opl` serves both games.
  Names are matched case-insensitively. Vehicle script variables take precedence over
  this fallback. Game values are read-only: writing them does not change the game.
  Boolean values are `0` or `1`; a text requested as a number (or the reverse), an
  unknown key, or a number not representable as a finite `f32` is unavailable.
  `destination` is the selected HOF terminus's texture identifier (empty for an all-exit
  terminus); `passengers` is zero when no passenger simulation is active. `version` is
  the game's displayed version, rather than the Cargo package version.
* `OMSI_NO_PLUGINS=1` leaves every plugin out. A plugin whose host stops answering is
  left out for the rest of the session.
* `PluginStart` gets a nil owner: there is no Delphi application object. Plugins that
  open their own windows do so without a parent.

### Building the host

```bash
scripts/build-plugin-host.sh
```

needs `rustup target add i686-pc-windows-gnu` and MinGW (`brew install mingw-w64`);
Copy `dist/omsi-plugin-host32.exe` next to the game. The 32-bit build uses
`panic=abort` and a stand-in `_Unwind_Resume` (Homebrew's i686 MinGW links no unwinder the
prebuilt standard library can use).

### Tests

`cargo test -p omsi-plugin` builds `crates/omsi-plugin/demo` (a plugin with the OMSI
interface) and drives it in-process and through the host. With
`OMSI_TEST_WINE_DIR=target/i686-pc-windows-gnu/release` (after building the host and the
demo for `i686-pc-windows-gnu`) the test `windows_dll_under_wine` runs the real chain: a
32-bit Windows DLL with Delphi-style undecorated `stdcall` exports, in the 32-bit host,
under Wine.
