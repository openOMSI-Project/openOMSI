# OMSI 2 content formats

The file formats of OMSI 2 content (maps, objects, vehicles, scripts, timetables …) as openOMSI
reads them.

## Text encoding

* Map files (`global.cfg`, `tile_x_y.map`, Chrono `*.map`, `*.chronoterrrelevant`) are UTF-16LE with
  BOM (`FF FE`).
* Every other text file is read by OMSI in the system's ANSI code page (Delphi `AnsiString`):
  the stock content is Windows-1252, but a mod is written in its author's code page -
  Windows-1251 for the Russian ones (LiAZ, PAZ, the Scania Citywide's cockpit), 1250 for the
  Polish and Czech ones - and some newer files are UTF-8. openOMSI has no system code page
  to borrow and decides per file (`omsi-cfg::codepage::detect`): valid UTF-8; 1251 when at
  least half of the letters `0xC0..` stand in runs of three or more (Russian words; German
  has at most two in a row, "Größe"); 1250 when letters that are signs in 1252 (ł ą ś Ł Ś Ż)
  stand next to letters; 1252 otherwise. A font may still have been read in another code
  page than the text it shows, so a glyph is also looked up as the same byte in 1251/1250/1252.
* File names beyond ASCII may have picked up another spelling on the way: a zip stores a
  name without its UTF-8 flag in the OEM code page of the machine that made it (CP866 on a
  Russian one) and the unpacker guesses another (the Scania's `верх.png` arrived as
  `óąÓň.png`, CP866 read as CP852). A name is also looked for as its bytes in CP437, CP852,
  1252 or 1250 read again in CP866, 1251, 1250 or 1252 (`codepage::name_variants`).
* Line endings are CR LF. Numbers use `.` as decimal separator and may carry an exponent
  (`6.21874087223886E-7`).

## The `[keyword]` block format (used by .bus .ovh .sco .sli .cfg .hof .ttp .ttl .ttr .owt .oop .odr .otp .osn .dsc .hum .txt …)

Quoting the original .bus header comment: *the program ignores everything except keywords like
`[mesh]` when they stand at the start of a line, alone on the line. The following lines are then
read as the parameters, one per line. Afterwards the program searches for the next keyword.*

Consequences implemented in `omsi-cfg` (settled on the loaders of OMSI, which compare
the line they read with their keyword literals by plain Delphi string equality):

* A keyword line is the **whole line**: no leading whitespace (indented `[keyword]` lines are
  the stock files' help texts and switched-off blocks - the SD202 cabin's `[exit]` help, the
  F90's second rear axle, whose mesh does not exist, the EN92's fourth door-slam sound) and no
  trailing whitespace.
* The keyword is spelled as the original spells it: `[matl_noZwrite]`, `[LOD]`,
  `[NightMapMode]`, `[noDistanceCheck]` (a mod's `[matl_nozWrite]` or `[NoMapLighting]` is free
  text). Keywords the original does not know are compared without regard to case.
* Exceptions: `.hof` cuts trailing tabs, spaces, CR/LF and `"` off every line (the stock depot
  files are spreadsheet exports); `ailists.cfg` lower-cases its lines.
* Sub-commands (`attach_trans`/`attach_rot_*` after `[new_attachment]`, `origin_*`/`anim_*`/
  `offset`/`delay`/`maxspeed` after `[newanim]`) are compared the same way, as lines of their
  own anywhere after their block (the stock `Timetable_Terminus_Pole_S.sco` has its
  `attach_trans` after `[complexity]`).
* `-<DISABLED>-` … `-<ENABLED>-` (each alone on its line) switches blocks off in `.bus`/`.ovh`,
  `.sco`, `model.cfg`, `passengercabin.cfg`, `paths.cfg`, `.hum` and `envir.cfg`.
* Parameter lines are consumed verbatim in order; an empty line is a legal (empty) parameter.
* Free text between blocks is ignored.

File names in content files are resolved as Windows resolves them: `\` and `/` both separate,
names compare without regard to case (umlauts included), a folder name loses one trailing dot
and the last name all trailing dots and spaces (Ahlheim's `anz-oben.jpg.`).

Content roots (openOMSI's content folder, mounted archives, the OMSI 2 installation) are
searched as if they had been copied over each other, highest priority last - with one
exception: a vehicle pack (`Vehicles/<folder>`) present in two roots in two *versions* does
not mix. The copy under a higher-priority root patches the one a vehicle was loaded from
only when it has no `.bus`/`.ovh`/`.zug` of its own or only ones that pack has too; a pack
with vehicles the other lacks is another version (Ahlheim V5 bundles its own Citaro
Facelift next to an installed older one), whose files are only used for what the
vehicle's own pack lacks. Mixed, the Ahlheim Citaro ran the installed pack's varlists
and constfiles under its own scripts and its displays stayed dark.

## Scripts (.osc) - unit `mc_exprcalc`

An 8-float *working stack* (`st[0..8]`) and a 10-float *register file* (`l0..l9` load,
`s0..s9` store without popping; the exe's `TCache` is `array[0..9] of Single`). Values push: the new value goes to `st[0]`, everything shifts
down one slot, `st[7]` is dropped. **Binary operators pop both operands and push the result**;
popping shifts the stack up and refills `st[7]` with 0, so underflow reads zeros. (The old
`Info_scriptcode.txt` table suggests operands stay on the stack, but stock scripts such as
`(L.L.a) 0.5 > (L.L.b) 0.5 < (L.L.c) 4 = ! || &&` only make sense as `A && (B || C)` with
popping semantics.)

Binary ops: `+ - * / % = < > <= >= && || min max` with `a b -` meaning `a - b`. Division (and
`%`) by zero gives 0 (the exe's `divide` handler compares the divisor with 0.0 and stores 0;
the LiAZ 5292 charges its battery by `Timegap V_generator /` and relies on it). Unary ops replace `st[0]`: `sin arcsin arctan exp sqrt sqr sgn abs
trunc ! /-/` (negate); `d` duplicates. `pi` pushes π, `random` replaces `st[0]` by a random
whole number in `[0, n)` with `n = |round(st[0])|` (the exe calls `Random(Abs(Round(x)))`). A reached `{else}` jumps to the next `{endif}` even when a stray `{endif}` has already closed its
`{if}` (the Procity's dashboard script has one; ignoring that `{else}` blanked its odometer every
frame). `$SetLengthL n` keeps the left n characters or pads on the right, `$SetLengthR n` keeps the
right n or pads on the left (read off the exe's `str_SetLengthL/R`); re-checked Sept 24 2026 in `TXPC_calcblock_func` op 0x25 at 0x5d584a: `Copy(s, len-n+1, n)`). Mod scripts that cannot show what they were written for under these semantics are patched before compiling by `omsi_script::compat` (recognised by their text: the LiAZ 5292's 4-character line matrix pads the line to `4 $SetLengthL` and then keeps `3 $SetLengthR`, which left lines 1-9 blank and 5E as `   E`; its number is now written `"03" $IntToStrEnh` → `005E`. Aachen's ibox keeps route digits in float `ibox_eingabe` and blanks the display at 0, so leading zeros never appeared; mode-10 route entry uses a string digit buffer on `ibox_Zahleingabe` (empty until typed — no `"00"` / `"02" $IntToStrEnh` auto-pad): typed `0` then `1` shows `01`, typed `1` alone shows `1`; mode-7 line entry stays empty for free typing with no auto leading zeros). `{if}` does **not** pop its condition (28 spots in the stock scripts
work on it: `(L.L.bremse_feststell) {if} ! (S.L.bremse_feststell)` releases the parking brake,
`cond {if} (L.L.IBIS_busstop) 0 > &&` in the IBIS; the chura matrix writes `x d -1 = ! {if} *`).
Tokens found in the executable's operator table: `/-/ <= >= && || sin arcsin arctan min max exp
sqrt sqr sgn pi random abs trunc` plus the single-character ones.

Variable access is `(X.Y.name)`:

| 1st letter | meaning | 2nd letter | meaning |
|---|---|---|---|
| `L` | load float | `L` | local (object) variable |
| `S` | store `st[0]` | `S` | system variable (`Timegap`, `GetTime`, …) |
| `C` | constant from constfile | `M` | map variable |
| `F` | curve from constfile: pushes `curve(st[0])` | `$` | string variable (`L.$`, `S.$`) |
| `M` | macro call (`M.L.name`) or callback (`M.V.name`) | `V` | host callback |
| `T` | sound trigger (`T.L.name`, `T.F.name`) | | |

Strings: `"literal"` (may contain spaces) pushes on the string stack. String ops (from the exe):
`$+ $= $d $* $< $> $<= $>= $length $msg $RemoveSpaces $cutBegin $cutEnd $SetLengthL $SetLengthR
$SetLengthC $IntToStr $IntToStrEnh $StrToFloat`. `$IntToStrEnh` takes a format string whose first
character is the pad character and the rest the width (`"02"`, `" 2"`). `$length` pushes the length
and leaves the string (the chura matrix drops strings itself with `(S.$.t) $length (S.L.n) 0 $* $+`);
`$StrToFloat` gives -1 for a text that is no number (`"NORDSPITZE"`), which scripts test for.

Blocks: `{init}`, `{frame}`, `{frame_ai}`, `{macro:name}`, `{trigger:name}` … `{end}`; control flow
`{if} … {else} … {endif}` on `st[0] != 0` (the condition is *not* popped). Comments start with `'`.
`%stackdump%` dumps the stack to the log. Macro, trigger and variable names are case-insensitive.
A trigger or macro defined in two script files is the **later** file's: the O530 Citaro pack's
`engine.osc` redefines `cp_batterietrennschalter_toggle` of its `cockpit.osc` (the ignition key
turns one notch per press of E: key in, electrics, ignition, starter while held - the bus could
not start otherwise), its ALMEX redefines `IBIS_Zahlentasten`. Operator names are compared as
spelled (`Min`, `$CutEnd` are not operators). A `$` word that is not one of the string
operators compiles to nothing, silently (a lone `$`, `$=>`, `$++`, `$SetLengthM` in mods);
another unknown word is an error.

Constfiles: `[const] name value`, `[newcurve] name` followed by `[pnt] x y` (piece-wise linear,
clamped). Varlists / stringvarlists: one name per line. A `(F.L.name)` whose curve no constfile
defines is logged as `SC_ErrorInCommand_functioninvalid` and still takes its argument, giving
0 (the O530 Facelift's ZF-6AP-1300/1700 lack two converter curves that its `zf-eco.osc` uses
in the line that computes `M_Wheel`).

Errors in the exe: `SC_ErrorInCommand_{varinvalid,macroinvalid,constantinvalid,functioninvalid}`,
`SC_ErrorInConstfile`, `SC_pnt_before_newcurve`.

System variables (`varlist_system.txt`): Timegap GetTime NoSound Pause Time Day Month Year DayOfYear
mouse_x mouse_y PrecipType PrecipRate coll_pos_x/y/z coll_energy Weather_Temperature
Weather_AbsHum wearlifespan AutoClutch SunAlt.
`mouse_x`/`mouse_y` are the movement of the frame while a `[mouseevent]` mesh is held: its
`<event>_drag` trigger runs every frame the button is down, with 0 when the hand keeps
still. The stock scripts depend on it - the EN92 cash desk takes its swing speed from
`(pos - lastpos) / Timegap` in that trigger, the rollers of the SD202 likewise, and the
door scripts set their push per trigger and clear it at the end of the frame.

## o3d (binary mesh) - unit `mc_o3dfiles`

```
84 19            magic
ver              u8   (1,3,4,5,7 seen)
[flags]          u8   (ver >= 3)  bit0: 32-bit triangle indices
[key]            u32  (ver >= 4)  usually 0
sections, tagged by one byte, until EOF:
  0x17 vertices  count (u16, or u32 if ver>=3) × {x y z nx ny nz u v} f32
  0x49 triangles count (u16/u32) × {i0 i1 i2 (u16 or u32), material u16}
  0x26 materials count u16 × {diffuse rgba, specular rgb, emissive rgb, power} f32×11, name (u8 len + bytes)
  0x79 matrix    16 × f32 (row-major, row 3 = translation)
  0x54 bones     count u16 × {name (u8 len), weight count u16 × {vertex idx (u16/u32), weight f32}}
```
Section order in stock content is always `17 49 26 79 [54]`. Verified on 4026/4026 stock files.
Any other byte where a tag is expected is passed over, one byte at a time (the loader's `case`
has no else branch). Mods rely on it: 50 version-7 files with flags 3 (the MB Sprinter 412D
XLWB pack, "protected" meshes) carry three junk bytes inside the 0x79 matrix and a few zero
bytes after it; refusing them lost the whole body of the bus.

**Axes.** Mesh files use Direct3D's frame: X right, **Y up, Z forward**. Vertices are stored in
the parent (object / vehicle) frame; the 0x79 matrix is the mesh's own pivot frame, used by
`origin_from_mesh` animations, and must *not* be applied when rendering. The engine converts
to its world frame (X east, Y north, Z up) by swapping Y and Z.

A data object in a `.x` may carry a name (`FrameTransformMatrix relative {`,
`TextureFilename tex {`), as the 3ds Max exporter of the Solaris Urbino 18 and the Novi Sad
objects writes them.

**DirectX `.x` meshes** (text `xof 0303txt`, mod content: the BVG Citaro's Atron terminal,
the O530's instrument glass) are flattened like `D3DXLoadMeshFromX` does: every `Frame`'s
`FrameTransformMatrix` applies to the meshes inside it, parent after child. The 16 numbers are
row-major for row vectors (translation in elements 12-14), i.e. read column by column they are
already the column-vector matrix; Blender's exporter puts a Y/Z swap (or a Z flip) in the root
frame and each object's placement in its own frame. Normals go by the inverse transpose (the
Ruede Trafohaus scales its frame unevenly). 27 stock scenery `.x` files have translated frames
(the Verkehrszeichen_MC street name signs, Ruede's chestnut trees, bus stop poles and old
shelter, Trafohaus, Buildings_MC's wohn_03/04 and hoch_01).

**Scrambled vertices (version ≥ 4).** The `key` field (`0xFFFFFFFF` = none) selects a vertex
scrambling reproduced from the original loader (`omsi-o3d`):

```
state = (key + version - 4 + (flags&2 ? 0x17D : 0)) mod 0xFDE8     (u16)
n     = vertex_count mod 0xFDE8;  b = 0
for each vertex (raw x y z nx ny nz u v):
    if key == 0: state = flags&2 ? 0x130 : 0
    state = (state*n + n*b) mod 8000
    b     = trunc(|frac(x)·frac(y)·frac(z)|·600) mod 256          (from the raw position)
    if state < 1000: swap(x,y) elif state < 3000: swap(x,z) elif state > 7000: swap(y,z)
    if state % 4 == 0: nx = -nx;  if state % 6 == 0: ny = -ny;  if state % 7 == 0: nz = -nz
    if state < 600: swap(ny,nz) elif state > 4500: swap(nx,ny) elif state > 6500: swap(nx,nz)
    if state % 5 == 0: u -= (state%100)² / 10000
    if state % 3 == 0: v -= (state%50)²  / 2500
```
Triangles, materials, matrix and bones are stored plainly. The original also refuses keys that
are not in its registration list; that check is not reproduced.

## Map - units `mc_mapclass`, `mc_terrain_2`, `mc_chrono`

`global.cfg` keywords (from the exe): name friendlyname description/end version NextIDCode
worldcoordinates dynhelperactive entrypoints realrail LHT mapcam standarddepot moneysystem
repair_time_min years realyearoffset ticketpack splineObjTypes scenObjList backgroundimage groundtex
map addseason trafficdensity_road trafficdensity_passenger. Calendar: Holidays.txt (`[holidays]`
range + name, `[holiday]` day + name), timezone.txt (`[timezone]`, `[DST]`), `[location]`.

`[entrypoints]` records: index, object id, 0, x, height, y (within the tile), quaternion
(x, y, z, w; heading = 2·atan2(y, w)), the index of the object's tile in the `[map]` list,
name. Tile indices of the map's files (entry points, `[splineAttachement_repeater]`,
timetable tracks) count every `[map]` entry, a tile listed twice included (Westcountry 3
lists 33 twice). Object ids are not unique across a map joined from two.

`tile_x_y.map` keywords: version terrain water variable_terrainlightmap variable_terrain object
attachObj spline spline_h splineAttachement splineAttachement_repeater varparent
spline_terrain_align spline_terrain_align_2 rule kill_rule; chrono patch files add selobject
selspline delete typ relabel. Rule kinds: speedlimit overtaking_prohib bus trucks no_cars
trafficdensity priority. A `[rule]` belongs to the `[object]` or `[spline]` written before
it, and most of them follow an object: Berlin-Spandau has 5402 trafficdensity, 693
speedlimit, 576 trucks and 64 no_cars on objects against 3114/810/284/4 on splines. Its
first line is the `[path]` index within that element.

`<map>/Chrono/<folder>/Chrono.cfg`: `[startdate]` and `[enddate]` (YYYYMMDD, `enddate` 0 =
never), `[ticketpack]`, `[moneysystem]` and `[deactivate_lines]` (a block of line names, one
per line). The folders are taken in name order and one is active when `startdate ≤ date`
and (`enddate` = 0 or `date < enddate`); its `.map` patches, `ailists.cfg` and timetable
files are laid over the map's. A line a chrono takes off does not run on any later date
either - Berlin-Spandau on 2026-09-17 has 27 lines and not the "5 & 5N" that the timetable
change `1000_FPW_19910602` of 1991-06-02 removed.

`[object]`: `0`, path, IDCode, **x, y, z** (z relative to the terrain, except for `[absheight]`
objects and objects that carry `[splinehelper]` connectors such as crossings and switches,
whose z is absolute like the splines they connect to),
heading, pitch, bank (degrees, heading clockwise from north), then the object's **labels**:
a count and exactly that many lines, whatever they say (an empty one, or one that looks like
a keyword, is a label too). They are what the editor's Labels dialog edits - sign texts
(`@` breaks a line), line numbers, a tree's texture, height and height/width ratio, a bus
stop's name and timetable data - and fill the object's string variables in order; the count
is no type (trees write 3 or 4). `[attachObj]`: `0`, path, IDCode, the parent's IDCode, the
parent's instance (only `0` loads: Omsi.exe refuses objects on later objects of a spline
attachment row), the index of the parent's `[new_attachment]` point, heading, pitch, bank,
labels. The parent is looked up among the records ([object], [attachObj],
[splineAttachement]) written **before** it in the tile; an attachment whose parent comes
later is not loaded.
Older tile versions write shorter records (TMapKachel.loadMapFile): the leading detail level
(`0`) only from version 9 on (an object above the detail setting is not loaded), IDCodes
from 6 on (older records are numbered as they load), an `[attachObj]` parent by IDCode from 10
on (before: its index among the tile's records so far), its heading from 8 on, pitch and bank
from 12 on, labels from 4 on. A `[spline]` before version 11 has one line instead of the two
neighbour IDs (-1: none, else it continues the spline written before it), cant from 5 on,
skew from 14 on, the texture offset from 11 on and the `mirror` line from 7 on. `[spline]` / `[spline_h]`: `0`, path, IDCode,
previous, next, **x, height, y**, heading, length, radius (0 straight, > 0 turns right),
gradient start/end (%), cant start/end, skew start/end, alignment length, optional `mirror`.
Field orders were verified by prev/next continuity and bus-stop link distances of the stock maps.
Street names are nowhere in a map but on its street name signs: a `StreetSign_*` object
(Verkehrszeichen_MC; 670 on Berlin-Spandau, 32 on Grundorf) has type flag 1 and the name as its
first string, and its heading is the named road's heading + 90° (the plate runs along the road:
1281 of 1429 pairs of same-name signs 150-700 m apart lie within 15° of that). The navigator
reads them for the city map.

`[splineAttachement]`: `0`, path, IDCode, spline index (in the tile's spline order), lateral
offset, height, start distance, heading, pitch, bank, interval, range, tilt flag, string
count, strings; `[splineAttachement_repeater]` has two more lines after the `0`: the index of
the master's tile in global.cfg's `[map]` list (counting the tiles whose files are missing -
Berlin-Spandau lists 369 and ships 329) and the index of the row's first object on its
spline. Object j of a row lies start distance + j·interval **along the chain from its start**
(the splines before the master's, following the `prev` links with the direction flips where
two splines meet end to end) while j·interval ≤ range: that gives 177 of the 180 repeater
indices of Berlin-Spandau that can be checked (165 counting from the master's own spline) and
puts the buffer stops a few metres before the ends of their tracks. A record places its row
on its own spline and on the following splines of the chain as long as they lie in its tile;
where the chain enters another tile, a repeater there carries the row on. Every one of the
169 Spandau repeaters with objects lies on the first spline of a new tile, and 413 later
splines in a record's own tile carry objects without one (a car park row with a 10 m master
spline has its repeater say "object 14" on the far side of a 180 m spline of the same tile).
An `[attachObj]` may name a row's IDCode: it hangs on the row's first object, with the
row's own type (a car park's, whatever car stands there).
Tile edge = 300 m; with `[worldcoordinates]` in global.cfg (Berlin-Spandau) the tile grid
is 1/300 degree and the edge is **371.9 m** on both axes (measured on 271 cross-tile spline
links, spread 0.14 m); local coordinates inside a tile then run 0..372 and objects may be
saved in a neighbouring tile's file with coordinates beyond the edge. `tile.map.terrain` = `u32 (=60)` + 61×61 f32 heights. `.map.LM.bmp` =
256×256 24-bit light map. `.map.prt` = precache list (text: sco path, min id, max id, -1).
`[crossing_heightdeformation] mesh.o3d` (scenery) is how a junction plate meets the ground.
The plate is one flat object a couple of hundred metres across, placed at an absolute
height; the named mesh is a coarse version of the same plate. Every vertex of the object is
moved by the difference between the ground under it and that base mesh, so the plate keeps
its kerbs and camber while its arms come down onto the roads that run into them, and the
plate's paths take their heights from it (Omsi.exe 0x7ba818). The ground is not pressed into
it at load: nothing in Omsi.exe reads that mesh for the terrain, and objects stand on the
`.terrain` heights.

`[spline_terrain_align]` (no parameter) and `[spline_terrain_align_2] <n>` follow a
`[spline]` in a tile file (Berlin-Spandau: 33 and 203 of 2486 splines). The editor's
"align terrain" wrote the ground's heights into the `.terrain` file already; at load the
flag (1, or `n`) only makes the spline cut its outline out of the ground (Omsi.exe: tile
parser 0x794e1a -> spline +0x35 -> segment +0x205 -> Generate 0x5b1178 -> GenerateTerrain,
"Terrain hole cutting: Spline"). The outline is the spline type's `[terrainholeprofile]`
extruded along the spline: right edge, far end, left edge, near end; `n` 2 and 4 keep the
far end at the spline's end, 3 and 4 the near end at its start, otherwise each point's
third value moves it past the end. A `.sli` without `[terrainholeprofile]` gets one per
string of joined `[profile]`s (0x5ab908): its left end and right end 3 cm in and 3 mm
down, a bottom 10 cm under its lowest point reaching 0.5 m past both ends. An outline
that crosses itself cuts nothing. Each `[terrainhole] <mesh.o3d>` in a `.sco` or model.cfg names an
object-wide cutter, including declarations before the first `[mesh]` and repeated commands.
The cutter sits next to its declaring file or in its `model` folder; declarations in a `.sco`
also apply when it references a separate model.cfg. Explicit cutters apply even to deep
excavations; the optional automatic road-cut height limit does not restrict them.
Both object and spline cutters matter: without them the terrain
stands over the carriageway, which from the driver's seat looks like a missing road.

`[groundtex]` (global.cfg) is one ground texture the map may be painted with: texture,
detail texture, then three numbers - the painting mask's size as a power of two, how often
the texture repeats across a tile, how often the detail texture does. The first entry is
what the whole map starts as and has no mask; each further one has a per-tile mask in
`texture/map/tile_x_y.map.<index>.dds`, an 8-bit alpha DDS (DDPF_ALPHA, 0xff alpha mask,
sometimes with mipmaps) of exactly that size, whose **first row is the north edge** -
verified where a painted strip crosses the seam between Grundorf's tiles (0,-1) and (0,-2)
at identical columns. The detail texture is multiplied in plainly; the stock ones are bright
(noise_low averages 242, gras_det 179) and doubling them like a grey-centred D3D detail map
blows the ground out. Grundorf's ground then measures (86, 91, 66) against the (91, 94, 69)
of the screenshot the map ships as `picture.jpg`.

`texture/water.tga` and `texture/water_envmap.bmp` of a map are its water: the stock swatch
is 8x8 pixels of 47, 74, 83 with alpha 192.

`[newanim]` blocks of a mesh (model.cfg) are composed **in file order, Direct3D style**: the
first block listed transforms the mesh first and the later ones act on the result, and every
angle is used with the sign the file gives it. With column vectors that is
`M = origin_n·R_n·origin_n⁻¹ · … · origin_1·R_1·origin_1⁻¹`. The SD200's door leaves are the
test case: each is two rotations about vertical axes (−170° about the fold, +80.5° back about
the post), and composed the other way round they stand across the middle of the doorway
instead of folding to its sides. `anim_rot` turns about the origin frame's x axis and
`anim_trans` slides along it; `origin_from_mesh` takes the pivot matrix stored in the `.o3d`.
Vehicle variables follow from that: `Wheel_Rotation_*` and `Axle_Steering_*` are radians
(the stock model.cfg multiplies a wheel by 180/π and the SD200's steering wheel by 1450),
positive steering is to the right, and `n_Wheel` is rpm (`antrieb.osc` computes power as
`M_Wheel · n_Wheel · π / 30000` kW).

A texture may carry a `<texture>.<ext>.cfg` sidecar: `[terrainmapping]` maps it in world
coordinates, `[moisture]` marks a surface that darkens when it rains, `[puddles]` one that
collects puddles, and `[surface] n` says what it is made of (0 asphalt, 1 concrete,
2 cobblestone, 3 dirt, 4 grass, 5 gravel, 6 snow, 7 deep snow) - the id the vehicle scripts
read as `Axle_SurfaceID_`.

`.map.water` = `u32 count` + 4 f32: one water surface over the tile with a height at each
corner (every stock file has count 1). The riverbed is ordinary terrain and the water plane
sits over it, so the shoreline is wherever the terrain rises through it. `.terrain_x.rdy` = editor cache, ignored.

## Splines (.sli) - unit `mc_splines`

length texture scaleTexByLength patchwork_chain heightprofile profile profilepnt path path_2
rail_enh third_rail halfcantwidth onlyeditor terrainholeprofile terrainholeprofilepnt
(`[terrainholeprofile]` begins a profile, each `[terrainholeprofilepnt]` - x, height,
end offset - goes to the last one begun).

`[path]` (5 lines): kind (0 street, 1 sidewalk, 2 rail), lateral offset x (right positive),
height z, width, direction (0 along the spline, 1 backwards, 2 both). The lane runs the
whole spline at that offset; lanes of consecutive splines meet at the ends.

## Traffic paths of objects (`[path]` / `[path_2]` in .sco)

12 lines (`path_2`: 14): start x, y, z in the object frame (x right, y forward, z up),
heading (degrees, clockwise, relative to the object), radius (0 straight, > 0 right turn),
length, gradient at the start and at the end (rise per metre: the height runs as their
integral along the path - checked on the stock objects, where 50 of 53 linked paths meet
the next one's start height that way, #1617), kind (0 street, 1 sidewalk, 2 rail), width, direction (0/1/2 as
above), turn indicator (0 none, 2 left, 3 right; used for the AI blinkers); `path_2` adds two
zero fields. Verified on `Einm_Spandauer_Koelner_1990.sco`: the arc `(1.5,-7.75) h0 r6.248
l4.635` ends exactly at the start of the next path `(3.142,-3.529) h42.5`.
`[use_traffic_light] n` after a path binds it (the preceding path) to `[traffic_light]`
index n; `[traffic_light] name` + `[phase] state seconds` (0 red, 3 red+yellow, 6 green,
8 yellow, 9 all-red; the last `0 0` phase = red until the `[traffic_lights_group]` cycle
restarts). `[trafficlight]` lamp objects are placed with map flag 1, the extra line = light
index, and `[varparent] id` = the crossing; their `[visible] red|yellow|green 1` meshes follow.

## Timetables (`TTData`)

* `Busstops.cfg`: `[busstop] name tile-index object-id offset 0 0` (tile index = position of
  the tile in global.cfg's `[map]` list).
* `.ttp` trip: `[trip]` + 3 lines (the track it runs on, empty for a bus trip that goes by
  its station links; terminus; line string), `[station_typ2] object-id`…, `[profile] name
  minutes` (+ `profile_man_arr_time/dep_time`). A "type 1 (old)" trip (trains, and whole mod
  maps such as Novi Sad) has `[station]` records of 8 lines instead - object id, index of
  the track entry the stop lies on, name, tile index, then four numbers - and runs on the
  track its `[trip]` names ("1 Klisa-Liman I" → `1_Klisa-Liman1.ttr`), which need not be
  named like the trip. AI buses follow a route in whichever direction it goes along a path:
  station links and tracks do drive one-way paths backwards (invisible helper streets).
* Map splines with the `mirror` flag have their cross-section turned over: every path lies
  at the negated offset and runs the other way (forward ↔ backward).
* `.ttr` track: `[track_entry] id path-index tile-index internal-path-no length 0` - the lane
  sequence of the trip (`id` = spline/object id in that tile, `path-index` = the `[path]`
  index in its .sli/.sco; the internal number is OMSI's per-tile path array index, a cache).
* `.ttl` line: `[userallowed]`, `[priority]`, `[newtour] number ai-group extra`,
  `[addtrip] trip profile departure-minutes`.
* `StnLinks.cfg`: `[StnLink] length from to …` + `[StnLink_entry] id path tile length -1 0 0`.
  A link often runs on a few paths past its station (the last entries mostly with length 0,
  into a turn lane or round a corner) while the next link starts at the stop on another
  path - in 122 of Spandau's 505 joins; those extra paths are not driven. Lanes are linked
by proximity of end/start points (≤ 1.5 m, heading within 40°), which reproduces the
`prev`/`next` spline chains and the `[splinehelper]` connections of crossings.

## Scenery objects (.sco) - unit `mc_complMapObj`

friendlyname groups onlyeditor complexity rendertype(presurface|surface|on_surface)
LightMapMapping nomaplighting NightMapMode fixed absheight collision_mesh crossing_heightdeformation
nocollision surface switch traffic_lights_group traffic_light phase approachdist traffic_light_stop
traffic_light_jump splinehelper path path_2 use_traffic_light blockpath crossingproblem switchdir
model scriptshare varnamelist stringvarnamelist script constfile sound sound_ai paths passengercabin
busstop entrypoint carpark_p trafficlight signal helparrow depot petrolstation tree
add_camera_reflexion(_2) mass momentofintertia cog boundingbox crashmode_pole new_attachment
(attach_trans attach_rot_x/y/z) maplight rail_enh third_rail triggerbox_new triggerbox_setreverb
plus the whole model.cfg vocabulary inline.

`[rendertype] presurface` draws the object before terrain and ordinary scenery, keeping
its mesh/material order. Alpha-blended materials still write depth at transparent texels:
an invisible cover can keep terrain from hiding an excavation already drawn below it.
Alpha-tested materials retain their cutouts, and `[matl_noZwrite]` disables blended depth writes.

## Model (.cfg) - unit `mc_complobj`

LOD VFDmaxmin detail_factor tex_detail_factor noDistanceCheck terrainhole CTC CTCTexture
scripttexture texttexture texttexture_enh mesh item setvar mesh_ident viewpoint shadow isshadow
mouseevent setbone smoothskin animparent newanim (origin_trans origin_rot_x/y/z origin_from_mesh
anim_rot anim_trans offset delay maxspeed) visible illumination illumination_interior light
light_enh light_enh_2 spotlight interiorlight texchanges matl matl_change matl_item
matl_raindropmap matl_texadress_mirror/clamp/border/mirroronce texcoordtransX/Y useScriptTexture
useTextTexture alphascale matl_freetex matl_lightmap matl_nightmap matl_allcolor smoke
particle_emitter PS_attachTo; material manager: matl_alpha matl_noZwrite matl_noZcheck matl_Zbias
matl_envmap matl_envmaprealtime matl_bumpmap matl_envmap_mask matl_transmap.

**`[smoke]` particles (Omsi.exe, established Oct 2026).** Nineteen lines: position (3),
direction (3), speed and its spread, frequency (a second), lifetime (s), brake factor,
gravity, start size, growth (a second), initial alpha, a line Omsi.exe skips, red, green,
blue; numbers or variable names (TRauch, read at 0x5f5e58; the final alpha stays 0, so a puff
fades out over its life). Every frame (0x5a238c) a particle's velocity is multiplied by the
brake factor raised to 20 x dt - 0x5a145c keeps 20 ln(max(brake, 0.1)) per emitter, so the
factor is per twentieth of a second at any frame rate - its level speed is drawn towards the
weather's wind (0x753428) by what that takes off (not yet in openOMSI, where it slows to a
standstill), and 9.81 x gravity x dt is taken off its vertical speed (a negative gravity
lifts it). Size and alpha go linearly with its age (0x5a183c).
Nothing stops it at the ground: it falls on through the road until its life is over (the
stock buses' and cars' wheel spray has gravity 1 and is under the road within a fifth of a
second). It is drawn (0x5a47d4 -> 0x5a2b5c builds the quads per camera, 0x5a4180 draws them
from the scene pass 0x6f1520 after the scenery and before the camera's own vehicle) as a
square facing the screen, reaching `size / view depth` from its centre in projection space
and turned by its own random angle (0x5a1d54), its depth moved 0.1 m towards the eye; with
fog and lighting off, `ALPHABLENDENABLE` on, `ZWRITEENABLE` off but the depth test on,
`SRCALPHA`/`INVSRCALPHA`, the colour the vertex colour alone (`COLOROP SELECTARG2`: the
particle's colour lit by the weather's light and the one lamp of 0x858efc on the CPU, white
for `--PS_emissive--`) and the alpha rauch.tga's times the vertex's (a `[particle_emitter]`
whose `--PS_bitmap--` is not marked as alpha: `ONE`/`INVSRCCOLOR`, texture times vertex
colour). So the road cuts every
puff that sinks into it off in a straight line; openOMSI fades a puff out over the lowest
6-25 cm above the ground it was set off over instead (the plane its vehicle's wheels stand on,
an object's own z = 0), and leaves out one wholly under it (`omsi_render::SmokeParticle`).

**Object visibility (OMSI 0x5fdc7c, established Sept 2026).** OMSI decides per *object*
(scenery object, vehicle), never per mesh, with the model's radius R, `[detail_factor]` D
(default 1; parsed into the model at +0xa8) and `[noDistanceCheck]` (+0xad, a flag of the whole
model wherever it stands). With d the distance to the camera and z the depth along the view:
the object is dropped when `d > R + maxObjDist` or `(z − R) / maxObjDist > 1` (both skipped by
`[noDistanceCheck]`), and when `size / D < minObjSize` where
`size = 2R / (z · fov · π/180)` - the object's diameter over its depth as a share of the
camera's vertical field of view (degrees, camera +0x38). The same `size` (not divided by D)
chooses the `[LOD]` level, so the `[LOD]` values of a model are in this measure. The values
come from `options.cfg`: `[performance_maxObjDist]` (750 in the shipped file, 900 in the high
presets, 1200 for Chicago), `[performance_minObjSize]` (0.013; 0.020 in "PC 2006") and, for
reflections, `[performance_minObjSizeRefl]` (0.046). A detail factor above 1 therefore makes
an object vanish sooner (clutter), below 1 keeps it longer.

**`[LOD]` choice (OMSI 0x5ef860, Sept 2026).** Each `[LOD] x` appends x to the model's list and
makes its index the level of the meshes after it. With that `size`, OMSI takes the *first*
level in file order whose x ≤ size, else the *last* level whatever its own x (a model with one
`[LOD]` is drawn at any size; the stock Sv signals' `[LOD] 0.1` is the signal and `[LOD] 1`
its far version). A mesh written before the first `[LOD]` gets the loader's level index
before any `[LOD]` has set it (an uninitialised local); here it joins the first level - the
WH UK AI cars put their shadow there.

**`[matl_transmap]`** is the effect's `gMatlTransMapOn` map (the material's `AlphaMap`, +0x58):
its *alpha* is the slot's alpha, and a picture without an alpha channel is opaque as Direct3D
samples it (the WH UK AI cars' paint layer uses a black 24-bit `transmap_null.tga`).

`[texchanges] <file>` names a `chtex_*.cfg`, relative to the vehicle's root folder for
vehicles and to the model-config folder for scenery. The examples are
`texture\chtex_SD.cfg` and `..\Anzeigen\Rollband_SD79\chtex_rollband.cfg`. It holds
`[newtexchangemaster]` blocks of two lines - the texture name as it
appears in the o3d, and a script variable - each followed by `[entries] n` and n texture
files that live next to that cfg. The variable's integer value picks the entry, 0 first
(`rollband.osc` clamps `rlbnd_lnN` to 0…15 for sixteen entries and stores `trunc()+0.001`).
The named texture usually does not exist on disk at all: the mesh carries it only as a key.
The masters are model-wide even though `[texchanges]` is written inside a mesh block.

Scenery `[CTC] <variable> <folder> <value>` groups read the folder's `.cti` items. Their
`[CTCTexture] <name> <default-file>` entries link each item to a material texture. The
script's integer variable selects an item by zero-based index; each `[CTC]` group has its own
variable and can select independently. Scenery `[texchanges]` masters use the same indexed
script-variable selection, with each entry replacing the master texture key. Both mechanisms
are applied to scenery material slots at runtime, so they can drive adverts on shelter panels,
building signs, and other props. An out-of-range or negative value leaves the model's own
texture active.

`[matl_freetex] <texture> <string variable>` is the same idea with a file name the script
builds at run time: the SD200's destination roller sets `Rollband_Tex_V` from the map's
`.hof` depot and terminus strings. `$.yard` is the depot's `[name]`, which is how
`Linienlisten\<yard>_RLB.jpg` resolves.

`[texcoordtransX/Y] <variable>` scrolls **one material slot's** texture, not the whole mesh:
the SD200's roller-blind mesh has five of them (three line bands and two destination bands),
each with its own variable.

`[newanim]` semantics (verified on the SD202 wipers, doors, blind, wheels): `anim_rot`
rotates about the **x axis** of the origin frame and `anim_trans` moves along it. Doors turn
that axis up with `origin_rot_y -90`, wheels spin about it directly, the sun blind slides
along it. `origin_from_mesh` uses the o3d pivot matrix: its first row (D3D frame, x right,
y up, z forward) is that axis, its fourth row the origin. OMSI evaluates the rotations in
its left-handed D3D frame, so in a right-handed (x right, y forward, z up) frame every
angle - `origin_rot_*` and the animated one - changes sign: the wiper arm's `-84` raises it,
the blade's `+84` about the arm pivot (frame `origin_rot_z 92`, i.e. 90° plus the windshield
rake) follows the arm, the blade's `+84` about its own pivot keeps it upright (pantograph).
Animations of one mesh compose in file order, the first one innermost.

`[matl_envmap] tex factor`: reflectivity = diffuse alpha × factor, the factor saturating at 1
like a D3D texture factor (SD202 bodies write `10`, their paint alpha is 0.12-0.19 → a
gloss, not a mirror; windows have alpha 0.5 with factor 1). The sphere map has the sky at
the bottom. It is read as Direct3D's `D3DTSS_TCI_SPHEREMAP` reads it, at the vertex, from the
reflection R in camera space (x right, y up, z ahead): u = Rx/m + 0.5, v = Ry/m + 0.5 with
m = 2|R - (0, 0, 1)|. The map's middle is what a face turned to the camera mirrors, and its
rim the reflection running on away from the camera.

### Mirrors, shadows, parked cars, cabin paths, announcements

* `[add_camera_reflexion] x y z dist fov yaw pitch` (`_2` adds one more number) in the
  .bus: rear-view mirror cameras (yaw clockwise from forward: 201 = back, turned towards
  the bus flank). Camera N draws into the texture named `reflexionN.bmp` on the model's
  mirror mesh; the mesh UVs are already mirrored, so the camera image is used as is.
  `[add_camera_reflexion_static]` (openOMSI, see MODDING.md) is a camera that looks along
  its own yaw and pitch; a rear section's cameras are numbered on after the front's.
* `[isshadow]` mesh (`D_schatten.o3d` with `Shadow.tga`, alpha blend, no z check/write):
  the vehicle's shadow blob lying at model z = 0; OMSI draws it on the ground under the
  vehicle. Scenery objects use it the same way. A vehicle's z = 0 is the plane its tyres
  touch with the springs *unloaded*, so a vehicle at rest has it 10-16 cm under the road
  (the springs' sag); only the missing depth test shows the blob. Here it is laid onto the
  plane the wheels stand on and left out while the sun shadow map is drawn.
* `[carpark_p]` `[onlyeditor]` scenery objects (`Generic\car_park.sco`) are parking spaces:
  the map's `parklist_p.txt` lists parked-car scenery objects (`Vehicles\X\parked_*.sco`,
  `[CTC]` colour schemes from the vehicle's `.cti`); one is placed per space at random,
  some spaces stay empty. A numeric first caption on a parking object or spline attachment
  chooses an indexed list: `1` reads `parklist_p_1.txt`, `2` reads `parklist_p_2.txt`.
  An empty or nonnumeric caption uses the ordinary `parklist_p.txt`.
* `.ovh`/`.bus` `[type]`: 2 = rail vehicle (also `[rail_body_osc]`, `[contact_shoe]`),
  3 = aircraft; spline `[path]` type 3 = flight path. Street traffic only uses type 0
  vehicles on street paths, aircraft fly the flight paths, rail vehicles come from `.zug`
  trains of the timetable.
* `paths.cfg` (`[paths]` of the .bus): `[pathpnt] x y z` (bus frame) with
  `[next_roomheight]`/`[next_stepsound]`, `[pathlink] a b`, `[pathlink_oneway] a b`,
  `[stepsoundpack]`; the cabin's `[entry] n`/`[exit] n` name path points. Passengers walk
  the links: entry → cash desk → seat and seat → nearest exit.
* Announcements: the hof's global string 0 is a folder under `Vehicles\Announcements\`;
  IBIS-2 builds `..\..\Announcements\<folder>\<busstop ident>[_#terminus].wav` (relative
  to the sound folder) and fires `(T.F.ev_IBIS_Ansagen)`: a trigger that takes the file
  from the string stack. The sound.cfg entry `[sound] N` (a number instead of a file)
  listening to that trigger plays it. `$msg` shows the top string without popping it.

* global.cfg `[addseason] kind start_day end_day` (day of year): 1 spring, 2 autumn, 3 winter,
  4 winter with snow, 5 dry summer; the season's textures live in
  `texture\Spring|Fall|Winter|WinterSnow|SummerDry\` subfolders next to the normal textures
  (same file names) and take precedence. A texture without a winter picture takes its autumn
  one. A weather with `[snow]` takes the `WinterSnow` pictures whatever the season, and with
  `[snowOnRoad]` as well the `WinterSnowfall` ones before them (the stock roads keep their
  snowy asphalt there); a texture with no snow picture takes its winter one, else its autumn
  one (Omsi.exe 0x7f910c).
  `[trafficdensity_road]` / `[trafficdensity_passenger] hour factor` lines form a curve
  over the day that scales AI traffic and waiting passengers.
* Scenery `.sco` `[sound] sound\x.cfg` uses the vehicle sound.cfg format, driven by the
  object's script variables and triggers (ambient sound objects are `[onlyeditor]`).

* `tile_x_y.map.LM.bmp` (256×256): the tile's **night light map** - pools of street lamp
  light on the ground (not shadows), north at the top row; added to the terrain at night.
  It covers the tile **and its eight neighbours**: the tile itself is the middle third
  (texels 85⅓..170⅔ each way). Neighbouring light maps are the same picture shifted by a
  third - 85 texels between two tiles, 171 between every other one, on all stock maps.
  A tile with `[variable_terrainlightmap]` (296 of Spandau's 329) has its light map baked
  from the `[maplight]`s of the objects of the nine tiles when they are loaded (Omsi.exe
  writes it over the file, unless options.cfg has `[no_generateTerrLightMaps]`), so the file
  is only what the last OMSI run left: openOMSI bakes it the same way. A texel, on the ground
  at its south-west corner, takes each lamp's colour × min(1, (radius / distance)²), the
  distance from the lamp's own height over its object, added up, held at 1 and truncated;
  a lamp more than 15.96 radii away along x or y adds nothing.
* Spline profiles (`[profilepnt] x z u v`) are extruded as-is: a road's outer points sit at
  the kerb height (0.25 m on the Marcel street splines) with no skirt down to the terrain,
  so the roadway is a slab standing on the ground. The terrain is only cut away under
  surfaces that lie flush with it (within ~12 cm); cutting under raised roads left a gap
  between the kerb and the grass.
* The engine fires `{trigger:collision}` on the vehicle after a crash; the stock scripts add
  `coll_energy` into `collision_energy` and knock out the electrics, doors and lights above
  their thresholds. `coll_energy` is that crash's energy in kJ and reads the same however
  often the block asks: the SD200/SD202/NL202 block adds it to the general account and then
  again to the engine's, which it only does for a hit behind `coll_pos_y` < -4.70 and below
  `coll_pos_z` < 1.10 (vehicle frame, m) - so `coll_pos_*` is where the bodies meet, down at
  the bumpers, not the height of the centre of gravity.
* Engine callbacks the stock scripts still needed: `(M.V.GetHeightAbovePoint)` takes x y z in
  the vehicle frame and returns how high that point stands over the ground below it
  (positive with room underneath: the NL/NG lift may drop by it, up to 0.3 m, and the Solaris
  Urbino's kneeling sensor reads under 0.05 once the body touches the kerb - its level
  control vents the right-hand bellows only above that); `(M.V.GetHumanCountOnPathLink)`
  takes a `paths.cfg` link index and returns how many passengers stand on it (the NL/NG fare
  gate swings out of their way). `GetTTTerminusIndex` spells "TT" with two T's before
  "Terminus".
* Situations `.osn` are written UTF-16 LE with a BOM and CR LF: `[name]`, `[description]`
  … `[end]`, `[map]`, optional `[weather]`, `[time] year day_of_year h m s`,
  `[centerkachel] tx ty`, `[mapcam]` (6), `[egopos]` (5), optional `[tt_active]`, then one
  `[vehicle]` block per vehicle (file, x height y, 7 orientation numbers, tile x y, id,
  depot name, optional `[coupledWith]`, `[ismyVehicle]`, `[vars] n` name/value pairs,
  `[stringvars] n`, optional `[settimetable]`), finally `[myvehicle]` and `[view]`.
* Weather `.owt`: `[wind] direction_deg speed_m/s`; `[clouds] type value` with type
  `-1` (clear), `Cumulus 1..3`, `Overcast 1`; the cover is drawn from `Texture\clouds.tga`
  (a grey density map) as a layer ~1500 m up drifting with the wind.

* Scenery `[texttexture] string-index font w h fullcolor r g b` + `[useTextTexture] n` on a
  `[matl]`: the text comes from the map object's string lines (`[object]` type flag 1);
  the texture is drawn as it is (the street name signs that seemed to want it turned by
  180° were `.x` meshes read with transposed frames); text is centred and squeezed to fit.
  A `[texttexture]` whose variable is one of the object's script string variables (the stock
  stop departure displays) is drawn from the script whenever it calls `Refresh_Strings`.
  `[helparrow]` objects (route arrows a map's author puts up) are drawn only while OMSI 2's
  route arrows are on (`nav_arrows`, the game menu's "Route arrows"), as in Omsi.exe.
* `[matl_envmap]`: the reflectivity mask is the diffuse alpha; textures without an alpha
  channel (DXT1 paint schemes such as the GN92 HVL livery) do not reflect at all.

## Vehicles (.bus/.ovh) - unit `mc_roadvehicle`

type friendlyname friendlyname_inv ai_veh_type coupling_back/front control_cable_back/front
couple_back/front coupling_front_character boogies sinus rail_body_osc contact_shoe rowdy_factor
ai_brakeperformance add_camera_driver add_camera_pax view_schedule view_ticketselling set_camera_std
set_camera_outside_center schwerpunkt rollwiderstand rot_pnt_long inv_min_turnradius ai_deltaheight
newachse (achse_long achse_maxwidth achse_minwidth achse_raddurchmesser achse_feder achse_maxforce
achse_daempfer achse_antrieb achse_inertia_inv) number registration_automatic registration_list
registration_free kmcounter_init + model/script/sound/paths/passengercabin.
Built-in vehicle variables are listed in `program/varlist_roadvehicle.txt`; generated per axle:
`Wheel_Rotation_ Wheel_RotationSpeed_ Axle_Steering_ Axle_Suspension_ Axle_Springfactor_
Axle_Brakeforce_ Axle_SurfaceID_` × `{n}_{L|R}`, `PAX_Entry{n}_Open/_Req`, `PAX_Exit{n}_…`,
`Debug_0..5`; `PAX_Entry8..15` and `PAX_Exit8..15` besides Omsi.exe's eight, and
`PAX_Entry{n}_Busy` / `PAX_Exit{n}_Busy` (somebody in that doorway; see MODDING.md).
Callbacks: `program/callbacklist_*.txt`.

Passenger cabin: entry ({noticketsale} {withbutton}) exit linkToNextVeh linkToPrevVeh stamper
ticket_sale ticket_sale_money_point(_2) ticket_sale_change_point(_2) passpos drivpos
illumination_interior. Paths: stepsoundpack pathpnt pathlink pathlink_oneway next_roomheight
next_stepsound.

Sound (.cfg): next_random loopsound sound 3d dir noloop important viewpoint volcurve pnt
conditionSingle conditionInt conditionBool trigger checkloading onlyone.

## HOF - `mc_station`/`mc_roadvehicle`

name global_strings servicetrip stringcount_terminus stringcount_busstop addterminus_allexit
addterminus addterminus_list {ALLEX} end addbusstop addbusstop_list infosystem_trip
infosystem_busstop_list.

## Timetable (TTData) - `mc_timetable`

`Trains/*.zug`: pairs of lines, vehicle file (or pool group name) and a reverse flag (1 =
coupled with its rear end forward: its `[couple_back]` point meets the leading vehicle and
the car is drawn turned around); the first entry leads.

Busstops.cfg `[busstop]`; StnLinks.cfg `[StnLink]` `[StnLink_entry]`; `*.ttp` trip trainreverse
station station_typ2 profile profile_man_arr_time profile_man_dep_time profile_otherstopping;
`*.ttr` track_entry; `*.ttl` userallowed priority newtour addtrip; car_use `*.ocu` valid line
onlytypes end types_prefered number_tour.

## Misc

* Weather .owt: name description end fog clouds precip groundwet snow snowOnRoad wind temp press.
* Options (.cfg/.oop): see `[performance_*]` table in the exe (full list in `omsi-options`).
* Fonts .oft: `[newfont] name bitmap alpha height gap` + `[char] ch x0 x1 y`.
* Languages .olf: `KEY<TAB>text` lines, first line = language code.
* Money: `[currency] name decimals`, `[coin]/[bill] o3d value`. Tickets .otp: ticketpack voicepath ticket ticket_2.
* Humans .hum: model seatheight walk_param humangeom links voice age.
* Drivers .odr: ident busstops hektom crashs tickets rating perbusinfo. The personnel file
  is UTF-16 LE with a BOM like a situation. `[ident]` is name, sex, date of birth, date of
  hire; `[busstops]` counts the stops served and, of those, the ones reached too late and the
  ones left too early (in that order, like Omsi.exe's driver record); `[hektom]` is the distance driven in hectometres; `[crashs]` counts crashes, hurt
  pedestrians, abscondings and, of those, the heavy ones; `[tickets]` the tickets sold and
  the takings; `[rating]` the ratings the personnel dialog shows (driving on a 0 = excellent
  to 10 = perilous scale, passenger comfort, ticket selling) followed by two accumulators
  (the stock file has them all at zero, so the last two are ours: the distance the ratings
  were averaged over, and the number of jolts).
* Situations .osn: name description end map actuWeather time centerkachel mapcam egopos TT_active
  myvehicle view vehicle coupledwith ismyVehicle vars stringvars settimetable.
* Descriptions `.dsc`: `[name]` (or `[friendlyname]`) lines and a `[description]` text, for a
  map's `global.cfg`, a `.bus`/`.ovh` or a `.owt`, in a file named after the stem:
  `global_ENG.dsc` beside `global.cfg`. The German text is the file's own description, so
  only other languages have a `.dsc`, and a language without one falls back to `_ENG`. They
  are Latin-1 like the rest, except that some were saved as UTF-16 with a byte-order mark.
* Sound conditions: `[conditionSingle]` / `[conditionInt]` give the variable, the **value**
  and then the relation - 0 `<>`, 1 `=`, 2 `<`, 3 `>`, 4 `<=`, 5 `>=` (`engine_n 200 3` =
  the engine runs, `velocity 2 2` = standing); `[conditionBool]` has no relation and
  compares for equality. How an entry plays (the exe's `TSound` update): with a
  `[trigger]` it plays once each time the trigger fires, from the start and never looped (a
  `[loopsound]` too), **without looking at its conditions**; without one it loops for as
  long as its conditions hold and it is audible (the long horn, rain, the compressor, the
  heater) - `[sound]` and `[loopsound]` both loop by default - unless it has `[noloop]`,
  which makes it play once when its conditions start to hold (a rising edge; the mod
  buses' ECAS kneeling, parking brake valve and start-up chime are written like that, and
  looped they hissed for ever). A `[loopsound]` plays at `|pitch variable| · rate / reference`
  Hz and is silent below DirectSound's 100 Hz. The rear section of an articulated bus has
  its own `[sound]` (on a pusher the engine is there) and plays it on the shared scripts'
  triggers and variables.
* `AutoClutch` (system variable) is 1 unless the options say `[no_automaticClutch]`: the
  manual-gearbox scripts (F90, T3, the Sprinter, PAZ and LAZ mods) work their clutch
  themselves then; at 0 a gear put in without the clutch pedal is thrown out at once.
* `Weather_Temperature` is known before a vehicle's `{init}` runs: the engine and heater
  scripts start from it (a carburettor PAZ made at 0 °C in May wanted its choke).
* Textures D3DX reads and GDI refuses: a palette bitmap counting more colours than its bit
  depth holds (4-bit, `biClrUsed` 17: the A21's and the Urbino's `LCD-Innenanzeige.bmp`),
  and DDS files whose fourcc is a D3DFORMAT number (36, 113 and 116: 16-bit and float RGBA).
* `wearlifespan` is a plain vehicle variable the engine sets, and it must be 1: at 0 every
  random part lifetime the stock scripts draw is 0, which makes the SD200's rear door
  reopen by itself for ever.
* Input: keyboard.cfg `[game]/[vehicles]` + `[entry] name scancode modifier` (the modifier
  is a mask, as Omsi.exe reads it (0x6478d0): 1 the action is told the key's state every
  frame - the throttle, brake and steering keys, " *" in OMSI's key list - 2 Shift, 4 Ctrl;
  Omsi.exe has no Alt, openOMSI's own Alt is 8, which OMSI leaves alone);
  gamectrler.cfg ctrl axis buttons FFScale.
* Startup order (logfile.txt) documents the manager creation sequence, mirrored in `omsi-sim`.

## Radio variables (plugins)

OMSI plays no music itself. The radios of the buses only set variables that plugins read:
`Snd_Radio` - the cassette player (`KR_play`/`KR_stop`) of the stock SD200/SD202/NL and of
most mods, 1 while it plays (no sound.cfg uses it); the Sound Extension plugin's
`SndExt_Radio` - the station button pressed (1..n, 0 = off; the W906 Sprinter and the
Procity show their own station names for it) with `SndVol_Radio` - the volume knob (0..1,
the Sprinter's goes to 2) and `SndExt_RadioPlaylist` for its USB/CD modes. openOMSI
plays internet stations for them (`~/.openomsi/radio.cfg`). Streams in HE-AAC with a
program config element (some `.aacp` stations) are not decoded; MP3, AAC-LC and Ogg are.

A radio whose display is a text of its script gets the station and the song that play,
ten characters a line, a longer text running through: into `Snd_Radio_Text` where the
script has that variable, and into the second line of `magnitola_1` (`frequency@station`,
`@` the line break) on Dmitrij's "Magnitola" radio, while it shows its track
(`mp3_display_track_name`).

A map may bring stations of its own: a `radio.cfg` beside its `global.cfg` (not an OMSI
file; Omsi.exe does not read it), a line `name = address` each. They come first, on the
first station buttons, and the player's follow. Its `volume` line is not read. Behind the
address the frequencies the station is on may stand, for the display: `| 94.6` is its
frequency everywhere, `| 94.6 @ x, y` the one near that place of the map, in the game's
metres (tile column and row times 300 m plus the place within the tile; the log gives a
bus's place when it is put on the map). The frequency of the place nearest to the bus is
shown:

```
Radiozurnal = https://example.org/radiozurnal.mp3 | 94.6 @ 25500, 20000 | 90.9 @ 2300, -720
Regional = https://example.org/regional.mp3 | 97.9
```

## Textures on the GPU - unit `mc_texMan`

* DDS files with DXT1-DXT5 data (and DX10 BC1-BC3) go to a GPU that takes block formats
  (every Apple silicon Mac does) as they are: sRGB BC1/BC2/BC3 with the file's own mip chain
  (the header's mip count; a partial chain is kept as it is), or, for a file without one,
  with levels made the way D3DX makes them - level 0 decoded, filtered in RGBA (here in
  linear light, like the RGBA path's GPU blits) and compressed again; level 0 stays the
  file's own. A DXT1 block with c0 <= c1 using index 3 is transparent black, and a DXT1
  file with such a block counts as having alpha (the reflection-mask rule below).
* Uncompressed files (BMP, TGA, JPG, PNG, raw DDS) are compressed on loading, to BC1 when
  they have no alpha (or alpha 255 everywhere), else BC3 (the alpha is a reflection mask,
  a cut-out or a blend), when the result is close: luma-weighted colour PSNR at least 33 dB
  and alpha PSNR at least 30 dB. Noisy cobbles and asphalt (27-30 dB, about one BMP in
  eleven on Ahlheim) stay RGBA. Block formats need sides that are multiples of four: a
  picture whose sides are not (the 2550² Ahlheim repaints, 771×1024 display pictures) is
  resampled to the nearest multiple of four with a Catmull-Rom filter that keeps the texel
  centres where they were in texture space; pictures with a side under 128 texels, or under
  64² texels in all, stay RGBA.
* A tile's own pictures (the `.map.LM.bmp` light map, the roads' cut, the ground paint
  masks) keep their single level; a mask of nothing but 0 and 255 is 1-bit BC1 (only its
  alpha is read), a soft one BC3.
* Text and script textures and render targets stay RGBA. Scenery `[texttexture]` pictures
  are shared by what they show (font, text, size, colour).
* A picture the thread that draws has to read itself (a car of the random traffic, a
  roller-blind picture) goes up as RGBA with its chain made on the GPU, as before, and is
  compressed on a worker and swapped in afterwards.
* `options.cfg`: `[texture]` has two values (0 and 1 in the stock file), taken to be the
  resolution reduction and the compression switch of the options dialog (not verified);
  `[texmemlimit]` (401.0 in the stock file) is the texture memory in MB above which OMSI
  lowers the resolution of distant textures. openOMSI's own keys are
  `texture_compression=` (on by default) and `texture_memory=` (MB; `texmemlimit=` is read
  too; an eighth of the machine's memory when unset). `OMSI_NO_BC=1` uploads everything as
  RGBA, `OMSI_NO_TEXCOMPRESS=1` keeps DXT files as blocks and the rest RGBA.

## Lighting (envir.cfg, model lights)

* `envir.cfg`: `[sky_textures]` day/twilight/night, `[twilight_start_end]` sun altitudes,
  `[lightcolor_A/B/C]` = 5 RGB stops (nadir, twilight start, sunrise, twilight end, zenith)
  for direct sun, light from above and ambient.
* `[light_enh]` (mesh): pos, rgb 0-255, size (m), fading variable (name or constant),
  brightness factor, z offset, effect bits, fade time, optional texture - Omsi.exe reads it
  into the same lamp as a `[light_enh_2]` (omnidirectional, turned to the viewer) and draws
  it alike. `[light_enh_2]`: pos, dir, up, omni, rotating, rgb, size, cone
  inner/outer, variable, factor, z-offset, parameters, cone, timeconst, bitmap. The lights
  count whatever detail level their mesh belongs to: 40 stock models (the Spandau neon,
  sodium and gas street lamps, the Sv signals, the ICE and RE160 coaches) declare theirs
  after the far `[LOD] 0` mesh, and no stock model repeats a light in two levels.
* `[spotlight]` (vehicle): pos, dir, rgb, range, inner, outer; the active one is chosen by the
  `Spot_Select` variable (negative = none). `[spotlight_2]` (openOMSI, see MODDING.md): the same twelve
  numbers, a switching variable and a no-mirror flag; any number lit at once. `[interiorlight] variable range r g b x y z`.
* `[maplight] x y z r g b radius` (scenery): point light at night, full within `radius`,
  inverse-square beyond, out of range at six times it. The colour is the brightness: a petrol
  station's red sign declares 0.1 red over a 20 m core and is meant to glow by its pumps, not
  to tint the street a hundred metres away. `[matl_nightmap]` = self-illumination texture, `[matl_change]
  texture index variable` + `[matl_item]` = texture variants by variable (`NightlightA` for
  street lights).

## Script textures and the IBIS callbacks

`[scripttexture] w h` (model.cfg) declares an RGBA image per index; `[useScriptTexture] n`
puts it on a material, `[matl_transmap] \S:n` uses its alpha as transparency map. The
scripts draw with `(M.V.ST…)` callbacks, arguments pushed in order (index first):
`STNewTex(i)`, `STLock(i)`, `STUnlock(i)` (upload), `STFilter(i)`, `STSetColor(i, a, r, g, b)`,
`STDrawPixel(i, x, y)`, `STDrawRect(i, x1, y1, x2, y2)`, `STTextOut(i, x, y, font, mode,
letter-spacing, "text")` (mode bit 0: colour from the font's colour bitmap, else the
`STSetColor` colour; alpha from the font's mask; `mode & 3 == 2` writes only covered pixels,
other modes the whole glyph cell), `STReadPixel(i, x, y)` + `STGetR/G/B/A(0)`, `STLoadTex("path", i)`
(pushes nothing - OMSI 0x7bba4e only logs a missing file; the path is relative to the
object's `Texture` folder, then to the global `Texture`, so the Krüger
matrix's `..\..\Anzeigen\Krueger\x.bmp` is `Vehicles\Anzeigen\Krueger\x.bmp`), `STCopyColor(a, b)`.
`GetFontIndex("name")` registers an .oft font and answers **-1** for a font that does not exist
(the chura matrix falls back from missing custom fonts with `1 + 31416 * l1 1 + max 31415 % 1 -`,
which only works for -1); fonts are looked up by `[newfont]` name in every content root's
`Fonts`. openOMSI draws a missing weight with another one of the same family and size
("churafont++ Numeric 26x11 Bold" → "churafont++ Numeric 26x11"; the Citaro pack asks for
weights it never shipped and lost its line number). `TextLength(font, "text")` = drawn width
(glyph advance x1−x0 plus the font gap). A work texture keeps its picture in the colour
channels with alpha 0 (the Krüger matrix draws `STSetColor(0, 0, 255, 0, 0)`), the LED
textures in alpha only (`[matl_transmap] \S:n`, one texture per LED colour).
Glyph pixels of an .oft `[char] c x0 x1 y` sit in columns x0..x1 (x1 exclusive as drawn).
In colour mode a glyph pixel takes the colour bitmap's pixel at the same row and column as
its alpha pixel (0x5d67bc), whatever size the colour bitmap has: often a small swatch of one
colour (stock `EFADfont.bmp`, 128×128 under a 128×200 alpha). For a row past the colour
bitmap's height Omsi.exe's `TBitmap.ScanLine` raises a range error (0x4763b0) and the text
stops being drawn there; a column past its width reads into the neighbouring scanline's
bytes. openOMSI repeats the colour bitmap in both cases instead.
`NrSpecRandom(seed)` = stable pseudo random in [0, 1).

Depot (.hof) callbacks: `GetRouteIndex(code)` (code = line×100 + route → `[infosystem_trip]`
index), `GetRouteTerminusIndex(route)`, `GetTerminusCode(idx)`, `GetTerminusIndex(code)`,
`GetTerminusString(idx, n)`, `GetBusstopCount(route)`, `GetRouteBusstopIdent(route, k)`,
`GetBusstopIndex("ident")`, `GetBusstopString(idx, n)`, `GetDepotStringGlobal(n)`.
`[addterminus]`/`[addterminus_allexit]` records are code, ident, then `stringcount_terminus`
strings (no separate station line in any shipped file). AI buses get their destination the
original way: string `SetLineTo` + `AI_target_index` (terminus index) and the
`ai_scheduled_settarget` trigger - and, as every trip a bus starts calls
`TRoadVehicleInst.virtual_10`, also when the depot file has no row for the trip's terminus or
the group names no depot file at all (`AI_target_index` stays as it was). Depot callbacks with
index -1 (what the lookups answer for an unknown code) return "" / -1, never entry 0. A depot
file belongs to a map: when the bus folder has none of the name the map's `ailists.cfg` wants
(a mod bus brings only its own map's), openOMSI takes it from another vehicle folder
(`omsi_vehicle::hof::depot_anywhere`); a bus of a plain `[aigroup_2]` pool, whose group names
no depot, takes the map's depot of its folder where it has one, else the folder's first
(`schedule::pool_depot`, Omsi.exe's selected-hof index 0). The FloFix
"Atron" IBIS of many mods starts in a PIN mode (`IBIS_mode` 10) and wants the `PIN` constant
of its constfile typed and confirmed before the mode keys work.

## Vehicle list and coupled parts

OMSI offers the `.bus`/`.ovh` files that have a `[friendlyname]` (all stock buses; the GN92's
rear section, most `_KI` AI variants and the AI cars have none). A rear section has
`[coupling_front]` (and `[scriptshare]`: no scripts of its own, the front's variables drive
it) and is named by the front's `[couple_back] file reversed`, resolved from the front's
folder in whichever content root has it. openOMSI also keeps a coupled part with a copied
`[friendlyname]` out of the list, spawns the front section when a rear one is asked for, and
refuses a rear section nothing couples. With `[scriptshare]` the rear section's `\S:n`
materials are the front's script textures (the O530G's rear display declares no
`[scripttexture]`) and its `[texttexture]`s read the front's string variables. A vehicle folder
without any vehicle file (a repaint such as "RBB VSN" for a bus that is not installed) is not
offered.

The joint of an articulated bus is reported to the front section's scripts as
`articulation_<n>_alpha` (about the vertical axis, clockwise degrees) and
`articulation_<n>_beta` (about the transverse axis); `n` is the coupled part's position in
the train, not a mesh index. The stock jackknife protection watches alpha and brakes over
47° while reversing, and the joint's plates turn with it. The bellows are `[smoothskin]`
meshes whose `[setbone]` ids are **the bone mesh's position in the model's first LOD**, and
they follow the joint's dummy meshes; the modelled pose is the straight one.

The passenger cabins of the sections are joined by `[linkToNextVeh]` / `[linkToPrevVeh]`
path points: the rear section's seats and exits are numbered after the front's, which is
how the stock door scripts count them, and a person walking from one section to the other
crosses the joint on those links. Omsi.exe joins two coupled cabins only where both give
these points. openOMSI also crosses a bus joint whose cabins give none, between the ends of
the two aisles, but not a trailer on a lorry's hitch (`[coupling_front_character]` type 0):
without the points that is a cabin of its own in openOMSI, and passengers whose place is in
it get in and out by its own `[entry]` and `[exit]` doors (a place there is not offered when
the trailer has no entry or no exit).

## Humans (.hum)

`[model]`, `[seatheight]`, `[humangeom] feetdist height`, `[links]` = 22 numbers: hip xyz,
knee xyz, waist yz, shoulder xyz, elbow xyz, neck yz, hand xyz, finger xyz (right side, model
frame x right / y forward / z up; the mesh is stored in a T-pose), `[voice]`, `[walk_param]`,
`[mass]`. The model's `[setbone] name id` binds o3d bones to engine ids: −2/−3 thighs L/R,
−4/−5 shins, −6/−7 upper arms, −8/−9 forearms, −10 hip, −11 torso, −12 head, −13/−14 hands.
Left is −x in every stock and add-on mesh checked (20 stock, 3 GSPNS). `[links]` values are
not always trustworthy: the GSPNS man04/man041 give the hip at x 0.6 and the knee at x 0.83
(the mesh's thigh is at 0.08) - harmless for rotations about x, fatal for a leg IK, so a
leg joint outside its limb's vertices is taken from the mesh.
A map's `humans.txt` (one `Humans\<group>\<file>.hum` per line) is the list its pedestrians
and passengers are drawn from; Berlin-Spandau and Grundorf name 15 stock types (not the DBC
staff in uniform, not aXYZ man01), so an add-on's people installed for another map never
walk there.
Clothing variants work like a bus's paint schemes: the human model's `[CTC] Colorscheme
<folder> 0` names a folder **relative to the .hum file** (`Texture\man01` =
Humans/Other/texture/man01) whose `.cti` `[item]`s replace the `[CTCTexture] farbschema
<default>` texture (stock people have one or two variants besides the default). The body
texture of man02 and the women's hair carry `[matl_alpha] 1` (alpha test).
The `[passengercabin]` file gives `[passpos] x y z seat-height rot` seats - x, y, z is the
**hip** ("Attachpunkt Arsch") when the seat height above the floor is greater than zero and
the **foot** when it is zero, which marks a standing place - plus `[entry]`/`[exit]`
path points, `[ticket_sale]`, `[stamper]`. In openOMSI a `[passpos]` may go on with one or
two script variable names on the lines straight after its five values (see MODDING.md). `[entry]` may carry `{noticketsale}`: passengers
who use that door walk straight to a seat instead of past the cash desk. It and `{withbutton}`
are lines of their own that mark the entry read last, wherever they stand between the blocks
(the stock cabins write a blank line before them), as Omsi.exe reads them.

The entries and exits are numbered in file order, and that number is how the engine and the
bus script talk about the doors. The engine writes `PAX_Entry<i>_Req` while somebody at the
kerb wants in through entry `i` and `PAX_Exit<i>_Req` while a rider wants out through exit
`i`; the script answers with `PAX_Entry<i>_Open` / `PAX_Exit<i>_Open`, and only then does
anybody move. The stock SD202 `door.osc` sets `PAX_Entry0_Open` from `door_0 > 0.9`, keeps
the stop-request lamp lit while `PAX_Exit0_Req` or `PAX_Exit1_Req` is set, and sets
`haltewunsch` when the door release is on and somebody presses a rear outside opener
(`PAX_Entry2_Req`/`PAX_Entry3_Req`). Writes to an index the cabin does not declare are
ignored, which is how one script serves the two-door and three-door variants of a bus.
Pressing an outside opener also fires the engine trigger `door_aussenoeffner`.
The AI door scripts (`door-3_X10_AI.osc`, `door_X10_AI.osc`, `ai.osc` of the SD202) drop
the stop request as soon as the rear door is open and keep that door open only while a
`PAX_Exit<i>_Req` stands, so every rider on the way out (walking to the door, waiting at
it, stepping through it) keeps asking.

`AI_Scheduled_AtStation` is a handshake between the engine and an AI vehicle's script:
the engine writes 1 at a stop (the script opens the doors), -1 when the bus is to leave
(the script closes them, releases the stop brake and writes 0 back), and the bus pulls
away once it reads the 0. Writing 0 straight away leaves the stock scripts with nothing
to do: the doors stay open and the bus drives off with them. The EN92 script answers
once its rear door is shut, while the front leaves may still be closing.

Weather and wear reach the model through engine-written vehicle variables. `PrecipRate`
is signed: `rain.osc` adds `Timegap * PrecipRate` to `Rain_Window_Front_Wetness`,
`Rain_Window_Norm_Wetness` and `Rain_Window_Wiped_Wetness`, so a positive rate soaks the
glass and a negative one dries it; `wiper.osc` clears the wiped one as the blade sweeps,
which is why the arc in front of the driver stays clear while the rest of the window is
covered. `DirtRate` is how fast the windscreen soils right now (`dirt.osc` accumulates it
into `Dirt_Wiped`), and `Dirt_Norm` is how dirty the body is; both are the `[alphascale]`
of a dirt overlay material. `veh_wash` clears them.

`[matl_change] texture index variable` followed by one `[matl_item]` block (every stock
file has exactly one): the block's `[matl_nightmap]`, `[matl_lightmap]`, `[matl_allcolor]` …
describe the material *variant*. Omsi.exe (0x5fd6xx) rounds the variable to the nearest
whole number (ties to even) and shows item n for 1 ≤ n ≤ the number of items, the plain
material for anything else - a variable at 2 with one item is dark. A variable no script
declares is registered by the model loader at 0 (the stock MANs' spare buttons are switched
by `*Noch nicht belegt*`, "not assigned yet", and stay dark). The `[matl_change]` block
itself changes the plain material; each `[matl_item]` starts as a copy of the plain
material as it is at that point (openOMSI draws the first item). The item's `[matl_nightmap]` glows at full strength while the
variable is on, by day as well - warning lamps (`lights_blinkgeber`, `cockpit_light_*`,
`haltewunschlampe`) and dashboard screens drawn only in the night map (the Procity's pressure
screen, switched by `elec_busbar_main`) depend on it. A vehicle's plain `[matl_nightmap]` glows
the same way whenever its mesh is drawn, by day as well: dashboard lamps and the "stop
requested" sign are meshes switched by `[visible]` with a night map on a plain `[matl]` (#497);
a scenery object's fades in with the night. Street lights use `NightlightA`, bus panels
`elec_busbar_main`, displays their light switches. The item inherits the plain `[matl]` of
the slot (alpha mode, transmap) and may replace them: `[matl_transmap]` + `[matl_alpha] 2`
items are washer water / condensation that only exist while the variable is on;
`[matl_allcolor]` items (diffuse rgba, ambient rgb, specular rgb, emissive rgb, power) tint
the variant and add `texture × emissive` self-illumination (lower-deck lighting).
`[matl_lightmap]` is a light *mask* multiplied with the diffuse texture (the SD202 maps
`D86_02_L1.bmp` are grey masks of the lit atlas regions), scaled by its variable.

`[matl_glow] <texture> <value>` is **an openOMSI extension** (Omsi.exe's model unit has no
such keyword and skips it as it skips any unknown line; no stock content uses it): it makes the
slot **its own light**. The named texture is a **greyscale mask** of how much shines where
(white full, black none) and the light is the material's **own colour** - a destination
panel's text keeps the colour its display draws. `value` is its brightness on the scale of the
LED panels' `Led glow` setting (the shader's strength is `value * 0.25`, as that setting's
levels are): `6` is as bright as that setting's default, `20` brighter than its top. The setting
itself does not scale it. Only the enhanced picture draws it, as an emission held against the
metering as an LED panel's dots are, so it shows by day as well as by night; the glare round it
comes from how bright it is, as any light's does. An unlit slot (a script's or a text texture,
drawn at its own brightness already) does not take it.
The mask is bound in the light map's slot: a slot that also has a `[matl_lightmap]` keeps its
light map and has no glow, and the classic picture draws the slot as if the keyword were not
there. A glow that is switched (a tail lamp) is a `[matl_item]` of a `[matl_change]` with the
keyword; an item without a light map or glow of its own keeps its base material's. The last
`[matl_glow]` of a slot counts.
