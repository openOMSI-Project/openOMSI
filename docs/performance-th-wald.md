# TH_Wald CPU rendering bottleneck (#284)

The sustained slowdown at spawn `1792.7,1561.9,187,6.0` is dominated by draw
preparation and command recording, rather than tile streaming or GPU shading.
The asset audit identifies thousands of short curb, grass and pavement splines.
Every segment was uploaded as a distinct mesh, so instancing could not combine
them. `[terrainmapping]` sections additionally draw with the tile's ground layers.
Enhanced rendering processes the resulting geometry again in its depth passes.

For example, the unbatched main view recorded 5,349 batches for
`Randstein06_mG.sli`, 3,418 for `Gras_1m.sli`, 2,066 for `Randstein06.sli`,
and 1,868 for `Gras_3m.sli`. The first audit grouped the terrain-mapped parts under
procedural geometry; both halves now carry their source asset name.

## Change

- Join short spline segments of the same type in 48 m spatial cells on the loading
  worker, before GPU upload. Keep shadow flags, winding and material-slot order
  separate. Long segments keep their own bounds. Positions, normals and UVs are
  copied exactly, and original staging geometry still defines collision and terrain
  deformation. Tile unloading owns and frees the merged meshes as before.
- Combine adjacent profiles with identical material slots in a merged mesh.
- Compatible spline types also share a cell when all used slots have identical
  texture filenames, lookup directories and alpha modes. Unused terrain slots
  do not prevent merging. UV generation has already finished; no material is
  approximated. Blended spline segments retain their individual meshes and placement
  origins for the current main branch's far-to-near sorting. The audit labels a
  shared opaque/cutout mesh with its representative type.
- Split terrain-mapped spline faces before batching. In each 48 m cell, pool faces
  from different source spline types that use the same tile ground materials.
  Keep vertical cells and winding separate. Calculate tile UVs before merging,
  and draw the same base and paint layers as before.
- Replace two linear scans per blended mesh with a hash lookup for the object's
  nearest mesh distance. Preserve signed-zero equality and full f64 coordinates.
- Add asset audits every ten seconds under `OMSI_PROFILE`, per-encoder finish and
  join timings, and elapsed-time-corrected one-second FPS intervals.
- Cache transformed instance spheres and scale once when instance or mesh data
  changes; reuse them in the main view, shadows and mirrors. Mesh deformation,
  mesh replacement and recycling invalidate them. Floating origins remain per-view.
- Reuse up to three dedicated encoding workers for render bundles and encoder
  finishing, with the main thread participating. Size the pool by available CPU
  parallelism, independently of simulation workers. Preserve batch replay order.

The implementation uses ordinary meshes and draws on all rendering backends; it
does not change shaders or lower graphics settings. Spatial batching slightly
increases conservative culling, so GPU-limited hardware still needs benchmarking.

## Local release measurements before main integration, 2026-09-30

Windows/DirectX 12, RTX 5070, 1920x1080, V-sync enabled, Enhanced, 4x MSAA, SSAO, 2048 shadows,
1024 mirrors, object distance 734 m. Same stationary spawn, date, seed, 20 traffic
target, scheduled buses and passengers; MB C2 articulated player bus as in the
supplied audit. Runs were sequential, 110 seconds each. FPS uses the median of the
last 30 one-second intervals after loading. Phase and batch statistics are the
renderer averages printed at shutdown, not the warm-up-excluded FPS window.

| Metric | Spline batching disabled | Enabled |
| --- | ---: | ---: |
| Stationary FPS median | 17.8 | 36.7 |
| Main batches/frame | 31,221 | 6,135 |
| Prepass batches/frame | 30,902 | 5,817 |
| Main triangles/frame | 1.789 million | 1.821 million |
| Draw preparation | 6.34 ms | 1.43 ms |
| Bundle recording | 5.12 ms | 1.50 ms |
| Encoder finishing wall time | 13.77 ms | 3.49 ms |
| Main-view culling | 3.19 ms | 1.77 ms |
| Mirror culling | 3.14 ms | 1.92 ms |

This A/B comparison uses the same executable with `OMSI_NO_SPLINE_BATCHING=1`
for the disabled case. Both cases already include the hashed transparency lookup.
An earlier untouched release executable also reproduced the sustained slowdown.
The historical first regressing commit between 0.1.144 and 0.1.221 has not been
established by these measurements. Results on other hardware are not measured.

The live before/after screenshots at 80 seconds were inspected. Mean absolute RGB
difference was 0.103 on the 0–255 scale; 0.254% of pixels differed by more than 8 in
any RGB channel. These are live sessions with moving traffic and HUD differences,
so this is a visual sanity check, not a deterministic pixel-equivalence test.

Logs and screenshots are in `target/performance-284/final-before.*` and
`final-after.*`; generated artifacts are not committed.

### Additional ground batching, same C2 scene

The follow-up comparison uses one new executable. `ground-before` disables only
the additional cross-type ground batching; the first spline optimization remains
enabled. `ground-after` enables both. Same settings and 110-second procedure:

| Metric | First spline optimization | Also pool ground faces |
| --- | ---: | ---: |
| Stationary FPS median | 36.8 | 40.9 |
| Main batches/frame | 6,136 | 4,849 |
| Prepass batches/frame | 5,818 | 4,530 |
| Main triangles/frame | 1.821 million | 1.829 million |
| Main render CPU | 10.1 ms | 9.1 ms |
| Mirrors CPU | 6.0 ms | 5.2 ms |
| Encoder finishing wall time | 3.34 ms | 2.91 ms |
| Main GPU pass | 2.59 ms | 2.68 ms |

This is another 11.3% FPS increase and 21% fewer main batches. The screenshot
comparison had mean absolute RGB difference 0.041 on the 0–255 scale; 0.076% of
pixels differed by more than 8 in a channel. Both images were inspected; moving
traffic and HUD still make this a sanity check. No O550 follow-up tests were run.
Artifacts: `target/performance-284/ground-before.*` and `ground-after.*`.

### Bounds cache and material-compatible splines

Another same-executable comparison enables/disables these two changes together,
leaving both earlier spline optimizations enabled. `bounds-material-before` uses
`OMSI_NO_BOUNDS_CACHE=1` and `OMSI_NO_MATERIAL_SPLINE_BATCHING=1`.

| Metric | Disabled | Enabled |
| --- | ---: | ---: |
| Stationary FPS median | 39.8 | 42.3 |
| Main-view culling | 1.71 ms | 1.41 ms |
| Mirror culling | 1.79 ms | 1.06 ms |
| Shadow draw preparation | 1.46 ms | 0.97 ms |
| Main batches/frame | 4,848 | 4,765 |

The 6.4% gain corresponds to about 1.5 ms less frame time. Run-to-run traffic
and background activity still affect FPS; these stages are shutdown averages.
The GPU integration test exercises bounds after transforms, skinning, render
origin changes, freeing, recycling and mesh replacement. The comparison images
had mean RGB difference 0.125/255, with 0.304% of pixels above an 8-level change.
Artifacts: `target/performance-284/bounds-material-before.*` and
`bounds-material-after.*`.

### Persistent encoding workers

`pool-before`/`pool-after` use the same final executable, toggling only
`OMSI_NO_RENDER_POOL=1`. Both include all batching and bounds changes.

| Metric | New threads per picture | Persistent workers |
| --- | ---: | ---: |
| Stationary FPS median | 36.9 | 43.2 |
| Main bundle recording | 1.35 ms | 1.10 ms |
| Mirror bundle recording | 1.07 ms | 0.59 ms |
| Main render CPU | 9.2 ms | 8.5 ms |
| Mirrors CPU | 5.3 ms | 4.4 ms |
| Encoder finishing wall time | 3.00 ms | 2.76 ms |

Traffic and scheduling variation also affect the FPS comparison. The reduced
encoding cost is measured directly; the full FPS difference should not be assumed
for every machine or scene. The pre-integration outside-view median is 43.2 FPS, compared
with 36.7 FPS after the first spline change. Stable 60 FPS at these same Enhanced
settings has not been achieved in this outside scene. It would require frame time
below 16.7 ms instead of the measured 23.1 ms.

Artifacts: `target/performance-284/pool-before.*` and `pool-after.*`.

The final C2 driver's-seat sanity run (`c2-driver-final`) measured 40.1 FPS
median, also with the existing Enhanced settings. The scene and mirrors were
inspected. This is a different camera, so it is not an outside-view A/B result.

## Integration with current main

Upstream `main` at `97b2519` was integrated, followed by the newly published
`200336e` (0.1.328), before opening the PR. Preserve its
ordered world passes, alpha-test/blend distinction and metric surface lift.
Opaque/cutout spline cells may merge; declared blended segments keep their own
placement origins and individual meshes for far-to-near composition. The grouping
test checks both their separation and their exact placement origins. Ground cells
use the same `Spline` phase and 8 cm lift as unbatched terrain-mapped splines.

The performance tables above describe the pre-integration executable; they are
not a claim that current main reproduces the identical baseline. After integration,
18 geometry unit tests, 21 renderer unit tests (including shader validation),
13 app scene tests and both GPU integration tests passed.

A live sanity run of the first integrated build (`9f3afc7`, main `97b2519`)
measured **35.5 FPS** median (30 final one-second samples, range 30.7-36.5),
5,744 main batches and 3,027 prepass batches. Main render CPU averaged 10.1 ms,
mirrors 5.8 ms and encoder finishing 3.44 ms. Its screenshot was inspected.
These are different upstream implementations and the merge also preserves the
new transparent-spline behavior, so the pre-integration 43.2 FPS is not presented
as the integrated build's FPS or as a current-main A/B measurement.
Artifacts: `target/performance-284/main-integrated-final.*`.

The final build (`be8e7f0`, upstream `200336e` / 0.1.328) was rebuilt and checked
with the same 110-second live procedure: **36.0 FPS** median (last 30 intervals,
range 31.7-38.8), 5,744 main batches and 3,027 prepass batches. Main render CPU
averaged 9.5 ms, mirrors 5.7 ms and encoder finishing 3.01 ms. Its screenshot was
inspected. All 54 executed unit/integration tests and the release build passed
on this final upstream snapshot. No current-main disabled-optimization run was
performed, so this remains a sanity measurement rather than a current-main A/B.
Artifacts: `target/performance-284/main-328-final.*`.

## Repeat

Build with `cargo build -p omsi-app --release --offline` (omit `--offline` if
dependencies are not cached). On Windows, from the repository:

```powershell
./scripts/profile-th-wald.ps1 -Root 'D:\OMSI 2' -Name before -NoSplineBatching
./scripts/profile-th-wald.ps1 -Root 'D:\OMSI 2' -Name after
./scripts/profile-th-wald.ps1 -Root 'D:\OMSI 2' -Name first-fix -NoGroundSplineBatching
./scripts/profile-th-wald.ps1 -Root 'D:\OMSI 2' -Name no-pool -NoRenderPool
./scripts/profile-th-wald.ps1 -Root 'D:\OMSI 2' -Name driver -View driver
```

The default player bus is `Vehicles/MB_C2_EN_BVG/MB_C2_E6_Gn_BVG_main.bus`. Pass `-Bus`
to use another installed vehicle, or `-Exe` to test a saved executable. The script
restores its temporary environment variables. Existing graphics settings apply;
match those settings across runs. Do not compare the whole-session average FPS
(which includes loading), overlap benchmarks, or compile during the stationary
measurement window. A detailed audit can add some CPU overhead.

Validation: geometry and renderer unit tests (including shader validation), the
spline grouping tests in the app, bounds lifecycle and presurface GPU integration tests, release
build, and live map runs. The existing automatic-scale test assumed the Mac pixel
budget on Windows; its expectations now account for both budgets.
