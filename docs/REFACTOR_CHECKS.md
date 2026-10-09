# Refactor checks

A refactor (code moved, functions extracted, duplication removed) must not change what the
game does. These checks show it did not, beside `cargo test`.

## Golden pictures

`scripts/golden.sh` renders a fixed set of offscreen scenes with one `openomsi` binary, and
compares two such sets pixel by pixel. Build the binary of the commit before the change and
the one after it, render both, compare:

```sh
export OMSI_ROOT="/path/to/OMSI 2"          # the original install (default: "OMSI 2 Original" beside the checkout)
scripts/golden.sh render /path/to/before/openomsi /tmp/golden/before
scripts/golden.sh render /path/to/after/openomsi  /tmp/golden/after
scripts/golden.sh compare /tmp/golden/before /tmp/golden/after
```

`compare` prints `identical` for each scene that matches byte for byte; otherwise the share of
pixels that differ and the largest and mean channel difference (0..255), and writes an
amplified difference picture into `<after>/diff/`. It exits 1 when any scene differs (beyond
its allowance, below). `render <bin> <dir> <scene> ...` renders only the named scenes, and
`compare` then compares only the scenes that are there. Each run writes `<scene>.log` beside
the picture; the first line names the build.

The binary needs `libsteam_api.dylib` (macOS) beside it: copy both out of `target/release`.
The pictures are made from licensed content: keep the output directories outside the
repository (the script refuses a directory inside it) and never commit them.

### The scenes

All at 960×540 (`GOLDEN_SIZE`), `OMSI_SEED=1` (`GOLDEN_SEED`), the default date (1989-05-30),
Grundorf unless named otherwise, the camera at the map's editor camera unless a bus is
spawned.

| Scene | Arguments | Covers | Allowance |
|---|---|---|---|
| `grundorf_day` | `--time 12:00` | terrain, splines, objects, sky, sun shadows | none |
| `grundorf_night` | `--time 23:30` | night maps, lamps, coronas | none |
| `grundorf_rain` | `--time 14:00 --weather Weather/Schmuddelwetter.owt` | rain, wet ground, puddles | none |
| `grundorf_fog_dusk` | `--time 20:45 --weather Weather/Daemmerungsnebel.owt` | fog, dusk light | none |
| `grundorf_enhanced` | `--time 18:30 --enhanced` | the Enhanced (physically based) renderer | 0.01 % |
| `grundorf_bus_outside` | `--time 10:00 --bus Vehicles/MAN_SD200/MAN_SD80.bus --view outside` | a bus from outside: model, materials, reflections | none |
| `grundorf_cockpit` | `--time 10:00 --bus Vehicles/MAN_SD202/MAN_D86.bus --view driver` | the cab: interior, gauges, mirrors, HUD | 0.01 % |
| `grundorf_drive_traffic` | SD202 at stop Bauernhof (`--spawn 500,748,339`), started by its switches (`--triggers ...`), `--drive 12 --traffic 15 --view driver` | engine start, gearbox, driving physics, AI traffic, script state after 12 s | 1 % |
| `spandau_day` | `--map maps/Berlin-Spandau/global.cfg --time 11:00` | a second, larger map | 0.01 % |

### Why runs are repeatable, and what is not

* Every scene runs with a fresh, empty `$HOME`, so the settings are the defaults and nothing
  remembered from earlier runs (the settings file, the install path, the driver profile)
  takes part.
* The settings file of each run turns `windy_trees` off: the renderer's animation clock is
  real time since start-up, so swaying trees never come out twice alike.
* What else is animated by that clock is covered by the scene allowances (share of pixels
  allowed to differ): the drive scene's exhaust smoke in the left mirror (about 0.3 % of the
  pixels), a dozen pixels of an instrument in the cab, a few pixels off by one in the Enhanced scene, and once in a while a single pixel off
  by one on Spandau. Everything else matched byte for byte.
* The simulation itself is repeatable: two runs of the drive scene log the same distance,
  speed, engine revolutions and traffic figures.

Measured with the binary of `main` at c85b2e0d (Oct 7 2026): three renders, all scenes
identical or within their allowance, every run.

Worth comparing too: the end of the logs (`drove ... in 12 s, now ... km/h`, `traffic: ...`)
of `grundorf_drive_traffic`; they carry no wall-clock figures.

## Frame time

A change meant to make the game faster is measured on the window's own frame, without a
window on the screen: `scripts/bench-window.sh <binary> <out prefix> [plain|enhanced]` runs
Spandau with traffic, passengers and the timetable in a window that is never shown
(`OMSI_HIDDEN_WINDOW`), drives off, and writes `OMSI_PROFILE`'s exit summary (frame-time
percentiles, every stage of the frame in milliseconds, the process's CPU time per frame).
`scripts/compare-performance.py before.json after.json` compares two of them. The stages'
times hold still from run to run far better than the frame times; take several runs of
each binary and their medians all the same.

## Tests that need the original install

Tests that read the original OMSI 2 install are `#[ignore]`d, so CI shows them as ignored
instead of passing them without looking at anything. They find the install through
`tools/test-support/original_root.rs` (`include!`d into their test modules): `$OMSI_ROOT`,
else `OMSI 2 Original` beside the checkout. Run them with

```sh
OMSI_ROOT="/path/to/OMSI 2" cargo test -p <crate> -- --ignored
```

Some need add-on content the stock install does not have (the AA-FR bus bundle, the Volvo
Wright family, the MB O530 Facelift): their `ignore` reason names it. An ignored test that is
run and finds its content missing fails, naming the file.

## Size budget

`scripts/size-budget.sh` (in the pull request checks) fails when a Rust file or function
grows more than 5 % past its size in `scripts/size-budget.txt`, or a new one passes 1500
(file) or 300 (function) lines. Functions are recorded by crate and name, so one moved to
another file of its crate keeps its allowance. After a split, `scripts/size-budget.sh
--update` lowers the recorded sizes; it never raises them. `--list` shows every file with
its longest function.
