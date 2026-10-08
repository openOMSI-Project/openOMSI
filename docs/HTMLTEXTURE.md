# HTML Textures

**Note:** This API is still under development, so breaking changes may occur at any time.

Previously, you had to use `scriptTexture` in Omsi 2; to make things easier for both modders and developers, openOmsi
now offers `htmlTexture`.
These can be used, for example, as IBIS or other information in vehicles, giving you a few more design options.

This documentation explains how to use the API in HTML. If anything is missing or you encounter any issues, please feel
free to open an issue with relevant information and steps to reproduce the problem.

Working html example: `docs/examples/htmltexture/demo.html`.

### Example model config:

```
[htmltexture]
1024
512
html\index.html

##########################################

[mesh]
generic\AFR4\AFR4_Display.o3d

[visible]
AFR4_Display
1

[matl]
afr4.dds
0

[useHtmlTexture]
0

[matl_lightmap]
1px_lm.bmp
0.5

[matl_freetex]
afr4.dds
AFR4_ST_texture

[matl_transmap]
\S:7

[viewpoint]
3
```

`[useHtmlTexture]` describes which `[htmlTexture]` it is; `\S:7` is the corresponding script texture ID in this case.
Note: An `[htmlTexture]` is registered as a `[scriptTexture]`, but it is also registered as an `[htmlTexture]`.

This means: If an `htmlTexture` follows `scriptTexture 6`, the `htmlTexture` is automatically `scriptTexture 7`. But it
is not `htmlTexture 7`.

### LED destination signs

A page drawn on an LED matrix is lit like the Krueger and K++ panels in Enhanced when its
material has a dot mask and a white light map: either the `\S:n` mask above, or a plain
picture of the dots as `[matl_transmap]` (black between the dots), with a `[matl_lightmap]`
that is white all over. The lit dots then burn at the `LED glow` setting's brightness and
bloom. A page's sign keeps that brightness at night even where it sits inside the cab
(behind a coach's windscreen); a script texture's panel in the cab (a dashboard's LCD) dims.

### Html file

`html\index.html` is the relative path from the bus path. Example: `Data\Vehicles\MAN_NewLionsCity\html\index.html` =
`html\index.html`

## The basic idea

Your page is normal HTML with a `<script>`. The game gives it one global object, `window.omsi`
(you can also just write `omsi`), and the page uses it for three things:

1. **Read** what the bus is doing: speed, doors, the current line, the next stop.
2. **Write** back: set a script variable, press a trigger, choose a route.
3. **React** to touches and clicks on the texture.

The smallest useful page:

```html
<div id="speed">--</div>
<script>
    window.omsi = window.omsi || {};
    window.omsi.update = function (d) {
        document.getElementById('speed').textContent = d.vehicle.motion.speedKmh + ' km/h';
    };
</script>
```

The game calls `omsi.update` whenever something the page could care about has changed. You
don't poll. You wait for `update`, look at the data and redraw what you need.

## Reading the vehicle

```js
window.omsi.update = function (d) {
    d.num, d.str      // script variables that changed since the last call (everything on the first call)
    d.vehicle         // the normalised snapshot, the same object as omsi.vehicle
    d.vars            // every variable so far, the same object as omsi.vars
};
```

|                                  |                                                                                                                      |
|----------------------------------|----------------------------------------------------------------------------------------------------------------------|
| `omsi.vehicle`                   | The normalised state of the bus, with **the same names on every bus** (see below). You can also read it from timers. |
| `omsi.vars.num`, `omsi.vars.str` | The latest value of every variable in the bus's own variable list, keyed by lower-case name.                         |
| `omsi.getVar(name)`              | One variable, any letter case. Gives a number, else text, else `undefined`.                                          |
| `omsi.apiVersion`                | `1`                                                                                                                  |

Why two ways? OMSI buses name their variables however the author likes (`door_0`,
`Fahrertuer_Rechts`, `elec_busbar_main` ...). If your page wanted to know whether the engine
runs, it would have to know every single bus. `omsi.vehicle` solves that: the game translates
the variables into one fixed layout, using the same rules it uses itself. Things a bus doesn't
have show up as `null` (numbers) or `false` (switches), never as a missing property. So
`omsi.vehicle.engine.rpm` never throws, even on a bus without an engine model.

Numbers are rounded (speed to 0.1, rpm to 1 and so on). That way a value that only wobbles in
the last digit doesn't trigger a redraw every frame.

Things that are specific to one bus (a mod's own switch, a battery voltage) are not
standardised by OMSI. Read those with `omsi.getVar("their_variable_name")`.

## Time, date, locale

|                                        |                                                                                         |
|----------------------------------------|-----------------------------------------------------------------------------------------|
| `omsi.time.hour`, `.minute`, `.second` | Simulation clock, as numbers                                                            |
| `omsi.time.asString`                   | `HH:MM:SS`                                                                              |
| `omsi.timestamp`                       | Now as a timestamp: seconds since 1970-01-01 on the simulation's calendar, no time zone |
| `omsi.date.day`, `.month`, `.year`     | Simulation date, as numbers                                                             |
| `omsi.date.asString`                   | `DD.MM.YYYY`, or `MM/DD/YYYY` when `locale` is `en`                                     |
| `omsi.locale`                          | Interface language as an ISO 639-1 code (`en`, `de` ...)                                |
| `omsi.weather`                         | The weather, `null` until it is known: `temperature` (°C), `humidity` (relative, 0..1), `visibility` (m, the weather's fog range), `clouds` (the weather's cloud type, `-1` clear), `precip` (0 none, 1 rain, 2 snow), `precipRate` (0..1) |

These are set before `omsi.update` runs and change with the simulation clock, once per second.
Use `omsi.locale` to pick your texts, and fall back to English for languages you didn't write.

## `omsi.vehicle` (version 1)

| path                                                                                              | meaning                                                                                                      |
|---------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| `info.number`, `.ident`, `.yard`, `.route`, `.nextStop`                                           | text variables of the bus                                                                                    |
| `motion.speedKmh`, `.heading`, `.pitch`, `.bank`, `.steeringDeg`, `.x`, `.y`, `.z`, `.odometerKm` | movement and position                                                                                        |
| `engine.running`, `.rpm`, `.throttle`, `.brake`, `.clutch`, `.gear`, `.tankContent`               | drive train and pedals                                                                                       |
| `electrics.on`, `.busbarMain`, `.busbarAvailable`, `.failure`                                     | on-board network                                                                                             |
| `battery.on`                                                                                      | battery switch, `null` when the bus has none                                                                 |
| `doors.count`, `.anyOpen`                                                                         | number of door leaves (`door_0`, `door_1` ...) and whether any is open                                       |
| `doors.list[i].number`, `.open` (0..1), `.isOpen`                                                 | `list[0]` is door 1                                                                                          |
| `passengers.onboard`                                                                              | people aboard                                                                                                |
| `passengers.entries[i]`, `.exits[i]`                                                              | `number`, `open`, `requested` (`PAX_Entry/Exit<n>_Open/_Req`)                                                |
| `lights.headlights`                                                                               | 0 off, 1 parking, 2 dipped, 3 main beam                                                                      |
| `lights.brake`, `.reverse`, `.fog`, `.interior`                                                   | lamps (`interior` is 0..1)                                                                                   |
| `lights.indicator`                                                                                | 0 off, 1 left, 2 right, 3 hazard. `.indicatorLeft` and `.indicatorRight` are the lamps, `.hazard` the switch |
| `brakes.parking`, `.stop`, `.kneeling`                                                            | switches                                                                                                     |
| `wipers.running`                                                                                  | windscreen wipers                                                                                            |
| `cabin.temperature`                                                                               | cabin air in °C                                                                                              |
| `condition.dirt`, `.crashes`, `.lastImpactKJ`, `.streetCondition`                                 | wear and road                                                                                                |
| `train.trailers`                                                                                  | coupled vehicles behind this one                                                                             |
| `route.*`                                                                                         | line, stops, delay: see the next section                                                                     |

`doors.anyOpen` follows what the passengers are told is open (`PAX_*_Open`) if the bus has
those variables, and the door leaves (`door_<n>`) if not. Some mod buses use `door_<n>` for
other things, so don't be surprised if a strange bus behaves oddly here.

## The route and the timetable

Everything about the current trip lives in `omsi.vehicle.route`.

| path                                                                           | meaning                                                                                                                                   |
|--------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| `route.active`                                                                 | `true` when the bus has a timetable (a duty)                                                                                              |
| `route.source`                                                                 | where the stops come from: `"timetable"`, `"ibis"` or `"none"`                                                                            |
| `route.line`                                                                   | the line text                                                                                                                             |
| `route.destination`                                                            | the text on the destination sign                                                                                                          |
| `route.stops[i]`                                                               | one stop: `index`, `name`, `arrival`, `departure` (`"HH:MM"`), `arrivalSec`, `departureSec` (seconds after midnight), `served`, `current` |
| `route.current`                                                                | the stop the bus is at or heading for (same fields as a stop)                                                                             |
| `route.terminus`                                                               | the last stop (same fields)                                                                                                               |
| `route.nextIndex`                                                              | index of the next stop. `null` when `active` is `false`                                                                                   |
| `route.delaySec`                                                               | delay in seconds. Negative means early                                                                                                    |
| `route.ibis.line`, `.suffix`, `.routeIndex`, `.terminusIndex`, `.terminusCode` | the raw numbers the IBIS itself holds                                                                                                     |

A few things worth knowing:

* `served` is `true` once a stop has been handled. When the last stop of a trip is served, the
  trip is over. A page can use that to start the return trip.
* `route.stops` can be empty. If the bus has no timetable and nobody typed a route into the
  IBIS, there is simply nothing to show. Check `stops.length` before you draw.
* Without a duty (`active` is `false`) the game doesn't advance the stops on its own. A page
  that wants to show progress then has to keep its own counter.

## Departures of a stop

`omsi.getDepartures(stop)` gives the departures at a bus stop for the next 2 hours, soonest
first, at most 20. `stop` is the stop's name as in the timetable (`Busstops.cfg`), any letter
case. Works on vehicle and scenery pages.

```js
omsi.update = function () {
    var list = omsi.getDepartures('Hauptbahnhof');
    for (var i = 0; i < list.length; i++) {
        var minutes = Math.round((list[i].time - omsi.timestamp) / 60);
        console.log(list[i].line + ' ' + list[i].destination + ' in ' + minutes + ' min');
    }
};
```

| field         | meaning                                                                  |
|---------------|--------------------------------------------------------------------------|
| `line`        | the line text                                                            |
| `destination` | the destination text                                                     |
| `time`        | departure as a timestamp, same scale as `omsi.timestamp`, delay included |
| `stopsAway`   | stops the bus still calls at up to this one, this one counted (`1`: this is its next stop, `0`: it stands here); `null` while it is not on the road |
| `load`        | how full it is, 0..1 of its seats and standing places; `null` unless its passengers are simulated |
| `delaySec`    | how late it runs (s) once it is on the road, else `null`                  |
| `lastTrip`    | `true` for the last departure of its line at this stop today              |

* The first call for a stop returns `[]`. The game fills the stop a moment later and then calls
  `omsi.update` again, so ask inside `update`.
* The game keeps the 8 stops a page asked for most recently. Asking for a ninth pushes out the one asked longest ago;
  asking for that one again later gives `[]` for a moment until it is made anew. The list is renewed about once a
  second.
* Without a timetable on the map the list stays empty.

## Acting on the vehicle

|                              |                                                                                                                                                           |
|------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| `omsi.setVar(name, value)`   | Write a script variable.                                                                                                                                  |
| `omsi.trigger(name)`         | Press a trigger of the bus's scripts, like a button in the cab does.                                                                                      |
| `omsi.setRoute(index)`       | Type route `omsi.depot.routes[index]` into the IBIS: line, route and destination, the way a driver would.                                                 |
| `omsi.setLine(text)`         | Set a line by its text. The game takes the first route of that line in the depot file. It matches the whole text (any letter case) or the leading digits. |
| `omsi.setDestination(index)` | Only change the destination sign, using `omsi.depot.destinations[index]`. The line and route stay as they are.                                            |
| `omsi.clearLine()`           | Clear the IBIS: no line, no route. Use it before a route is chosen, or to throw the current one away.                                                     |
| `omsi.setNextStop(index)`    | With a timetable: jump to stop `index` of the trip, forwards or backwards. Skipped stops are not served; going back makes them due again.                 |

Two details that trip people up:

* `setNextStop` goes **both ways**. Forwards, the stops in between are skipped. Backwards (a "back
  one stop" button), the stops from `index` on are due again and `served` is `false` for them. A
  request for the stop the bus is already heading for changes nothing. The game applies it on its
  next tick, so `route.nextIndex` follows a moment later.
* `clearLine()` only touches the IBIS. It does not end the duty in the game. If the page
  reads `route.active` afterwards it may still be `true`, so don't switch screens based on
  that alone.

Calls like these are handed to the game, which applies them a moment later. They don't take
effect inside the same script run. Read `omsi.vehicle` again in the next `update`.

## The depot file: `omsi.depot`

`omsi.depot` describes what the bus's depot file (`.hof`) offers, so your page can build a
line picker or destination list without hard-coding anything. It is filled once, before the
first `update`. If the bus has no depot file it stays empty.

**`omsi.depot.routes[i]`**

| field              | meaning                                                                       |
|--------------------|-------------------------------------------------------------------------------|
| `index`            | position in the list, the number you give to `setRoute`                       |
| `code`             | the IBIS route code (line × 100 + route number, by the depot file's own rule) |
| `name`             | name of the trip                                                              |
| `line`             | the line text                                                                 |
| `terminusCode`     | the destination code of the route                                             |
| `destinationIndex` | index in `destinations`, or `-1` if there is none                             |
| `destination`      | the text of that destination sign                                             |
| `first`, `last`    | name of the first and last stop                                               |
| `stops`            | names of all stops, in order                                                  |

Many depot files leave the line column as a placeholder like `XXX`. When that happens the
game works the line out from the route code instead (code 1203 becomes line `12`).

**`omsi.depot.lines[]`** groups the same route objects by line: each entry has `line` and its
`routes`, in the order of the depot file.

**`omsi.depot.destinations[i]`** has `index`, `code`, `id` and `name`.

A small example, a line picker that starts the first route it finds:

```js
function pickLine(text) {
    var routes = omsi.depot.routes, i;
    for (i = 0; i < routes.length; i++) {
        if (routes[i].line === text) {
            omsi.setRoute(routes[i].index);
            return;
        }
    }
}
```

A return trip is usually a route on the same line whose `first` is your `last` and whose
`last` is your `first`. Look for that pair and call `setRoute` with it.

## Touch and mouse

The app passes presses, releases and movements of the pointer on the texture to the page.
They turn into events on the element under the pointer and bubble up through its parents:

* `pointerdown`, `pointerup`, `mousedown`, `mouseup`, `click`, `mousemove`

You can listen in three ways:

```html
<button onclick="doSomething()">Press</button>
```

```js
el.onclick = function (event) { ...
};
el.addEventListener('click', function (event) { ...
});
```

The event object has `type`, `x`, `y` (in texture pixels), `target`, `stopPropagation()` and
`preventDefault()`. A click needs both the press and the release on the page.

There is no keyboard input, no hover and no drag and drop. Build keypads and lists from
buttons or boxes and react to `click`.

## Scenery objects

A scenery object (`.sco`) can show pages too: `[htmltexture]` in its model config and
`[useHtmlTexture]` on the material, the same as in a vehicle. The page is looked up in the
model folder, then in the folder of the `.sco`.

A scenery page only gets the basic API: `omsi.setVar`, `omsi.trigger`, `omsi.getVar`,
`omsi.vars`, `omsi.time`, `omsi.date` and `omsi.locale`. `omsi.vehicle`, `omsi.depot` and
`setRoute`, `setLine`, `setDestination`, `clearLine` and `setNextStop` do not exist there (`d.vehicle` in `update`
neither). The variables are those of the object's script; an object
without a script still shows its page.

The pages run only within 60 m of the camera. Clicks reach them from up to 4 m away, unless the
bus is in front of the page.

## What the HTML engine supports

The engine is deliberately small. It is not a browser, so a page that looks fine in Chrome may
need a few changes. When something doesn't show up, check this list first.

### HTML

* Nested elements, `<style>`, `<script>`, inline `style=""`, `id`, `class`, entities such as
  `&amp;`.
* Block elements: `div`, `p`, `h1`, `h2`, `h3` (with browser-like default sizes and margins).
* Inline elements: `span`, `b`, `strong`, `i`, `em`, `a`, `small`, `u`, `label`, `code`, and
  `<br>`.
* `<button>` comes with a default look (grey, rounded, padded). Boxes with
  `display:inline-block` are laid out in rows that wrap, which is what you want for keypads
  and lists. A box without a `width` is as wide as its content.
* `<img src width height>` (bmp, dds, tga, jpg, png). The file is looked up next to the page (see
  `htmlengine/images.rs`). `img.src` can be changed from a script. Pictures are cached,
  a missing file is cached too.
* `<head>`, `<title>`, `<meta>`, `<link>` are read and then ignored.

Not available: canvas, tables, lists (`ul`/`li`), forms and `input` fields, iframes, video, svg.

### CSS

* Selectors: tag, `#id`, `.class`, `*`, combinations like `div.a#b`, descendant chains (`.panel span`) and comma lists.
* Properties: `color`, `background` / `background-color`, `background-image: url()`,
  `background-size`, `background-repeat`, `background-position`, `font-size`, `font-weight`,
  `text-align`, `line-height`, `margin` (and `-top/-right/-bottom/-left`), `padding` (same),
  `width`, `height`, `display` (`none`, `inline`, `inline-block`), `visibility`,
  `border-radius`.
* Units: `px`, `%`, `em`, `rem`, `pt`, `vw`, `vh`.
* Layout is plain block flow with wrapped inline text.

Not available: flexbox, grid, floats, `position`, borders, gradients, shadows, `opacity`,
`overflow`, animations and transitions, custom fonts. Text is drawn in Roboto.

Since there is no `overflow`, a box doesn't clip its content. If a text is too long for its
box, shorten it yourself.

### JavaScript

The language is ES5 plus arrow functions.

* Language: `var`, `let`, `const`, functions, `if`, `for`, `while`, objects, arrays.
* Globals: `Math` (`round`, `floor`, `ceil`, `abs`, `min`, `max`, `sqrt`, `pow`, `trunc`,
  `sin`, `cos`, `PI`), `parseInt`, `parseFloat`, `String`, `Number`, `isNaN`, `NaN`,
  `Infinity`, `Object.keys`, `console.log/warn/error`.
* Strings: `split`, `replace`, `trim`, `indexOf`, `charAt`, `slice`, `substring`,
  `startsWith`, `endsWith`, `includes`, `toUpperCase`, `toLowerCase`, `padStart`, `padEnd`,
  `repeat`. Numbers: `toFixed`, `toString`.
* Arrays: `push`, `pop`, `shift`, `slice`, `join`, `indexOf`, `includes`, `map`, `filter`,
  `forEach`.
* Timers: `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`. They run while the
  page is being drawn and use real time, not simulation time.
* DOM: `document.getElementById`, `document.querySelector`, `document.body`,
  `createElement`, `appendChild`, `removeChild`, `remove`, `parentNode`, `getAttribute`,
  `setAttribute`, `classList` (`add`, `remove`, `toggle`, `contains`), `textContent`,
  `innerText`, `innerHTML` (with markup), `className`, `id`, and `element.style.*`.

Not available: `Date` (read the time from `omsi.time` instead), `JSON`, `fetch` and any
network access, `localStorage`, `Promise`/`async`, classes, `new`, regular expressions, and
the statements `switch`, `do ... while` and `try/catch`. Use `if/else` chains and `while`
loops instead. `typeof`, `break` and `continue` do work.

At most 64 timers can be active at once. Further `setTimeout`/`setInterval` calls are
ignored.

Two limits protect the game from runaway pages. A single run of your script may execute about
400,000 steps, and nesting (function calls, elements) is capped at 48 levels. Normal pages
never get near either. An endless loop just gets cut off.

### Redrawing

A page is redrawn when the DOM or the styles change. The simplest pattern, and the one that
works best in this engine, is to rebuild a block with `innerHTML` in your `update` function
and only do that when the data actually changed (compare against the last values you saw).
Redrawing on every call wastes time on a slow phone.

## Tips for beginners

* **Start from `demo.html`** and change one thing at a time. If the texture goes blank, the
  last thing you touched is the culprit.
* **Write to the log.** `console.log(...)` ends up in the game's log with the prefix
  `htmltexture console:`, which is the quickest way to see what `omsi.vehicle` or `omsi.depot` really contain on your
  bus.
* **Guard for missing data.** A bus without a depot file has no `omsi.depot.routes`, a bus
  without a timetable has no stops. Check the length before you loop.
* **Test on the real bus.** Each bus fills the variable list differently, and some values
  you expect will simply be `null`.
* **Keep buttons big.** The texture is often small on screen and people press it with a
  finger or a mouse. 60 px high is a good minimum.

## For developers

The snapshot is built by `omsi_sim::vehicle_api` (`snapshot` and `depot`, both unit-tested
without any game content) and handed to the backend through `HtmlRenderer::set_vehicle` and
`set_depot`, which a backend may ignore. The requests from `setRoute`, `setLine`,
`setDestination`, `clearLine` and `setNextStop` become `HtmlRequest` values that the game
picks up between two frames (`take_requests`, at most 32 per frame).

A scenery object's page is created with `PageApi::Scenery` (`HtmlTexture::with_api`), whose
`window.omsi` has none of the vehicle's parts; `drive_pages` then gets no snapshot and no depot.

To add a signal, add it to `vehicle_api::snapshot`, write a test next to the others, and list
it in the table above and in the module docs. Existing names stay stable within an API
version.

What the engine itself understands is listed at the top of
`crates/omsi-sim/src/htmlengine/mod.rs`. If you add a feature there, add it to the "What the
HTML engine supports" section here as well.

### Benchmark

`bench_htmlengine` times the engine (parse + scripts, cold and warm render, an update with a
frame, the pointer hit test) at 512x256, 1024x512 and 2048x1024. It is ignored in the normal
test run; start it in release mode, debug numbers mean nothing:

```
cargo test -p omsi-sim --release --lib bench_htmlengine -- --ignored --nocapture
```

`HTMLBENCH_PAGE=path/to/page.html` measures your own page instead of `demo.html`, and
`HTMLBENCH_RUNS=n` sets the number of timed runs (default 200).