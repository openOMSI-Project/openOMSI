# Plugins

openOMSI runs two kinds of plugins from the `plugins` folder of the game (and of every
content root):

* **Lua plugins** (`.lua`) - new in openOMSI 0.1.5: a text file, no compiler, works the
  same on Windows, macOS, Linux and Android, and is loaded again the moment you save it.
* **OMSI plugins** (`.opl` + DLL) - the original's plugins, unchanged (see
  [below](#omsi-plugins-plugins-opl-dll)).

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

| Path | The plugin's name |
| --- | --- |
| `plugins/<name>.lua` | a one-file plugin |
| `plugins/<name>/main.lua` | a plugin with several files: `require("util")` loads `plugins/<name>/util.lua` (or `util/init.lua`) |

Other `.lua` files are not started on their own, so a folder plugin's modules stay modules.
`OMSI_NO_PLUGINS=1` leaves every plugin out, Lua ones included.

### How a plugin runs

The file's top level runs once when the game starts. After that the plugin reacts to
**events**. A handler is registered with `omsi.on(event, fn)` (as many as you like), or
by defining a global function `on_<event>`:

| Event | Arguments | When |
| --- | --- | --- |
| `start` | - | right after the file was loaded (also after a reload) |
| `vehicle` | name or `nil` | the player got into a vehicle, changed it, or left it |
| `frame` | `dt` (seconds) | every frame of the game, after the bus's own scripts; not while paused |
| `stop` | - | the game ends, or the file is about to be loaded again |
| `key` | key name, `true`/`false` | a key went down / came up (`"KeyH"`, `"F5"`, `"Numpad8"`, ...) |
| `next_stop` | new, old | the duty's next stop changed |
| `view` | new, old | the view changed (`"driver"`, `"pax"`, `"outside"`, `"free"`, `"foot"`) |
| `duty` | line, tour | a line and tour were taken (or given up: `nil`) |

```lua
function on_frame(dt)
  -- runs ~60 times a second: keep it short, prefer omsi.every for slow work
end
```

You can send your own events too: `omsi.emit("my_event", 1, 2)` calls every
`omsi.on("my_event", ...)` handler (handy between the modules of a bigger plugin).

### The `omsi` table

#### The player's bus

Every name is a variable, string variable or trigger of the bus's scripts - the same names
the `.osc` files and the `.opl` lists use (`Velocity`, `elec_busbar_main`,
`IBIS_terminus_name`, `bus_doorfront0`, ...). Without a bus, or for a name the bus does not
have, reads give `nil` and writes do nothing.

| Function | What it does |
| --- | --- |
| `omsi.has_vehicle()` | `true` while the player drives a vehicle |
| `omsi.vehicle()` | the vehicle's name (manufacturer and type), or `nil` |
| `omsi.var(name)` | a script variable, a number |
| `omsi.set_var(name, value)` | sets it; `true` when the bus has that variable |
| `omsi.str(name)` | a string variable |
| `omsi.set_str(name, text)` | sets it; `true` when the bus has it |
| `omsi.sys(name)` | a system variable: `Time`, `Day`, `Weather_Temperature`, ... (read only) |
| `omsi.trigger(name)` | a key press: fires the trigger, then `<name>_off` |
| `omsi.press(name)` / `omsi.release(name)` | holds a key down / lets it go (`name`, later `name_off`) |
| `omsi.position()` | `x, y, z, heading` of the bus (map metres, degrees), or nothing on foot |
| `omsi.others(radius)` | the other vehicles within `radius` m of the bus (default 300): a list of `{id, kind, name, x, y, z, heading}`, `kind` being `"ai"` (the traffic) or `"player"` (another player's bus in a LAN game); empty on foot |
| `omsi.other_var(id, name)` | a script variable of one of them, or `nil` |
| `omsi.set_other_var(id, name, value)` | sets it; `true` when that vehicle has the variable. An AI vehicle keeps it until its scripts write it again; another player's bus takes its values from the network again |

#### The game

| Function | What it does |
| --- | --- |
| `omsi.info()` | a table of what the game is doing: `map`, `clock` (seconds since midnight), `day`, `year`, `view`, `paused`, `on_foot`, `multiplayer`, `traffic` (AI vehicles), `speed` (km/h), `delay` (s, late positive); on a duty also `line`, `tour`, `trip`, `trips`, `terminus`, `next_stop`, `next_stop_arrival`, `next_stop_departure` |
| `omsi.clock()` | the game's time of day as `"HH:MM:SS"` |
| `omsi.speed()` | the bus's speed in km/h (0 on foot) |
| `omsi.distance(x, y)` | metres from the bus to a map point, or `nil` on foot |
| `omsi.vars()` / `omsi.vars("str")` | the names of every variable / string variable of the bus's scripts |
| `omsi.command(name)` | does what a line of the game menu does: `refuel`, `wash`, `repair`, `shot`, `save`, `load`, `weather`, `later`, `earlier`, `info`, `timetable`, `reset`, `couple`, `uncouple`; `true` when the game knows it |

```lua
-- H: the time and the next stop on the screen
omsi.on("key", function(key, down)
  if key == "KeyH" and down then
    local i = omsi.info()
    omsi.message(omsi.clock() .. (i.next_stop and ("  next: " .. i.next_stop) or ""), 4)
  end
end)
```

#### Time, timers and watches

| Function | What it does |
| --- | --- |
| `omsi.time()` | seconds of game time since the plugin started (stands still while paused) |
| `omsi.after(seconds, fn)` | runs `fn` once, later; returns an id |
| `omsi.every(seconds, fn)` | runs `fn` every `seconds`; returns an id |
| `omsi.watch(name, fn)` | runs `fn(new, old)` whenever the bus variable changes |
| `omsi.watch(kind, name, fn)` | the same for `"var"`, `"str"` or `"sys"` |
| `omsi.cancel(id)` | stops a timer or a watch |
| `omsi.on(event, fn)` / `omsi.off(event, fn)` | adds / removes an event handler |
| `omsi.emit(event, ...)` | sends an event to the handlers |

```lua
-- a message when the bus comes to a stop after driving
omsi.watch("Velocity", function(v, old)
  if old and old >= 1 and v < 1 then omsi.message("Stopped", 2) end
end)
```

#### On screen and in the log

| Function | What it does |
| --- | --- |
| `omsi.message(text, seconds)` | a line of text on the screen (5 seconds when not given) |
| `omsi.log(...)` / `print(...)` | a line in `game.log`, tagged `[lua <name>]` |
| `omsi.warn(...)` | the same as a warning |
| `omsi.name` / `omsi.version` | the plugin's name / the game's version |

#### Saved data

`omsi.data` is a table that survives the session: it is written when the game ends (and
before a reload) and read back on the next start. Numbers, strings, booleans and tables of
them are kept. `omsi.save()` writes it at once. The file is `<name>.save.lua` next to a
one-file plugin, `data.save.lua` in a folder plugin's folder.

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

### A bigger example: a stop announcer

```lua
-- plugins/announcer/main.lua
local say = require("say")   -- plugins/announcer/say.lua: return function(t) omsi.message(t, 4) end
local last

omsi.watch("str", "IBIS_busstop_name", function(stop)
  if stop and stop ~= "" and stop ~= last then
    last = stop
    say("Next stop: " .. stop)
    omsi.data.announced = (omsi.data.announced or 0) + 1
  end
end)

function on_stop()
  omsi.log("announced", omsi.data.announced or 0, "stops this time")
end
```

### Safety and errors

A Lua plugin gets Lua 5.4 with the safe libraries only: `string`, `table`, `math`, `utf8`,
`coroutine`, `require` for its own folder, and `os.clock/time/date/difftime`. There is no
`io`, no `os.execute`, no C modules and no `dofile`, so a plugin you download cannot touch
your files beyond its own saved data.

* An error in a handler is written to `game.log` and shown on the screen; the other
  plugins and the game carry on. After 10 errors the plugin is switched off until you
  change its file or restart the game.
* A handler that runs longer than a second (an endless loop) is stopped with an error.
* A file that does not compile is left out, with the Lua error in `game.log`.

### Tips

* Watch `game.log` (in `~/.openomsi/`) while you write a plugin: every `omsi.log` line and
  every error is there.
* `OMSI_WATCH_VARS=Velocity,throttle` logs changes of bus variables - useful to find the
  names a bus uses; the bus's `.osc` scripts list them all.
* Keep `on_frame` light; use `omsi.every` and `omsi.watch` for everything that does not
  need every frame.

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
