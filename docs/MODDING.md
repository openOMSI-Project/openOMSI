# Modding beyond OMSI 2

For map-selected Tomorrow.io weather, add an `openomsi_weather.cfg` beside
`global.cfg`. See [weather areas](WEATHER_REGIONS.md) for the format, player setup
and the [community config editor](https://openomsi.mxnticek.eu/weather/).

openOMSI reads OMSI 2 content as it is: a bus, a map or an object made for OMSI 2 works
without changes. It also lifts limits OMSI 2 put on modders. Everything on this page is an
addition. A file that uses it still loads in OMSI 2, which ignores what it does not know.

## Interior lights: more than four per mesh

OMSI 2 lights a mesh with at most four `[interiorlight]` lamps, the four numbers of its
`[illumination_interior]`. openOMSI takes as many as you list, up to 63 per mesh. Write the
extra lamp numbers on the lines right after the first four; a blank line ends the list:

```
[mesh]
saloon.o3d

[illumination_interior]
0
1
2
3
4
5
6
7

[matl]
...
```

OMSI 2 reads the first four and skips the rest. A model may declare any number of
`[interiorlight]` lamps, and each mesh names the ones that light it. The same goes for
`[illumination_interior]` in `passengercabin.cfg`.

## Textures of any resolution

- Textures up to 16384 × 16384 pixels load: DDS (DXT1/3/5, uncompressed), TGA, BMP, PNG and
  JPG. Where the graphics card cannot take that size, the texture is halved until it fits.
- There is no 2 GB address-space limit: openOMSI is a 64-bit program.
- A texture keeps its full resolution within 150 m of the camera, so a bus's own 4K
  textures always stay sharp. Far scenery gives up detail only when the texture memory
  (`texture_memory` in the settings, or `OMSI_TEXTURE_MEMORY`, in MB) runs out, as OMSI's
  `[texmemlimit]` does.
- `[scripttexture]`, `[htmltexture]` and `[texttexture]` / `[texttexture_enh]` can be any size the card
  takes.

## PBR materials

Normal, roughness, metalness and occlusion maps beside a texture, up to 4096 × 4096. See
[PBR materials](PBR.md).

## Lights

- Each 25 m square of the world draws up to 32 point and spot lights at once (the nearest
  first), so depots, stations and lit interiors keep their lamps.
- Interior lamps of vehicles are drawn per pixel, with no count limit per vehicle.
- `[spotlight_2]` in the `model.cfg` is a spotlight of its own: low beam, main beam, fog
  lamps or a lamp over a door can light the road at the same time, each switched by its
  variable, beside the one `[spotlight]` that `Spot_Select` picks. It takes the twelve
  numbers of a `[spotlight]` (position, direction, red, green, blue, range, inner and
  outer cone angle), then the variable (0 off, 1 full, in between dimmed; a number is a
  constant) and a flag:

  ```
  [spotlight_2]
  0.95
  5.95
  0.652
  0
  1
  -0.3
  255
  255
  233
  200
  30
  80
  lights_fern
  0
  ```

  With the flag 0 (or left out) the lamp is where it says and a twin of it stands on the
  other side of the vehicle, mirrored across its axis (x and the x of the direction turned
  round): put the position on one headlamp. With 1 there is just the one lamp, for a
  light over a door or a cornering lamp. A pair is as bright as a `[spotlight]` of the same
  colour, shared between its two lamps. The position is used as written: unlike a
  `[spotlight]`'s, it is not moved onto the vehicle's front. A rear section's model may have
  its own; their variables are the bus's.

## Screens: static cameras

`[add_camera_reflexion_static]` in the `.bus` adds a camera for a screen - a CCTV monitor
of the doors, a reversing camera - to the mirrors. It takes the numbers of an
`[add_camera_reflexion]` (x, y, z, distance, field of view, yaw, pitch) and counts with the
mirrors: camera N draws into the texture `reflexionN.bmp`. Unlike a mirror's, its picture
does not move with the driver's head: it looks along its yaw (degrees clockwise from the
vehicle's forward) and pitch (up), as a driver camera does. Its picture is what it sees,
not a mirror image, so map the screen's texture the right way round.

The reflection cameras of an articulated bus's rear sections count on from those of the
section in front: with mirrors 0 to 3 in the front section's `.bus`, a camera in the rear
section's `.bus` is number 4, and a screen in the cab showing `reflexion4.bmp` shows what
that camera sees from the rear section.

## Models

- `.o3d` files with 32-bit indices (the long-index flag) are drawn with their full vertex
  and face count; everything is drawn with 32-bit indices.
- There is no limit on the number of meshes, materials, `[matl_change]` items, `[CTC]`
  entries, cameras, doors, passenger places, wheels or axles.

## Passenger doors: sixteen entries and sixteen exits

OMSI 2 has the door variables of eight `[entry]` and eight `[exit]` paths a vehicle
has (`PAX_Entry0_Open` to `PAX_Entry7_Open`, the same with `_Req` and for `PAX_Exit`). openOMSI
has them for sixteen of each: `PAX_Entry8_Open` ... `PAX_Entry15_Open`, `PAX_Entry8_Req` ...
and the same for the exits. An entry or exit past the eighth is a door of its own once the
script sets its `_Open` variable (or lists it in a varlist); then its passengers wait for that
variable and ask for the door through its own `_Req`. Without it, it opens with the eighth, as
in OMSI 2. Its passengers' requests then go through the eighth's `_Req` in openOMSI, while
OMSI 2 loses them (its request arrays have eight slots).

openOMSI also tells the script who stands in a doorway: `PAX_Entry<n>_Busy` and
`PAX_Exit<n>_Busy` (n 0 to 15) are 1 while somebody is on the door's threshold or in the
opening between it and the step outside - what a door's light barrier sees - and 0 otherwise;
the people queueing outside a shut door, in the aisle or on the deck above do not count. A
door script can keep a door open or open it again while its `_Busy` is set. Like the `_Req`,
they are written before the scripts run every frame and cleared after them; a door past the
eighth without variables of its own reports through the eighth's. OMSI 2 does not have them.

## Passenger places switched by the script

OMSI 2's `[passpos]` places are all there all the time. In openOMSI a `[passpos]` may name a
script variable on the line straight after its five values: while that variable is 0 no
passenger takes the place (whoever sits there already stays until they get off), so a bus
can have two seating layouts and switch between them with a setvar. A second name on the
line after that is a variable the engine sets to 1 while somebody is on the place and to 0
while nobody is - a tip-up seat can fold down for the person on it - without the seat
numbers `GetHumanCountOnSeat` needs:

```
[passpos]
0.94
0.04
0.92
0.43
0
layout_transverse
seat_12_taken
```

Both are ordinary variables of the bus's varlists. A place without the lines, or with a
name the scripts do not have, is always there; a blank line or the next block ends the
list, and OMSI 2 passes over the lines.

## Ticket validators: one by every door

OMSI 2 uses one `[stamper]` of a `passengercabin.cfg`, the last one written. openOMSI keeps
every `[stamper]` of every section, and a passenger with a ticket to stamp uses the one
nearest the door they came in by. Write one `[stamper]` block per validator (path point,
then x, y, z of the device, as usual); OMSI 2 still takes the last of them.

## Scripts and plugins

- Script variables and string variables have no count limit.
- Lua plugins can read and write script variables, fire triggers and react to game events:
  see [Plugins](PLUGINS.md).

## Radio: a map's stations and a bus's display

What a player sees and hears is in the [user guide](USER_GUIDE.md#radio); here is what a map
or a bus can add. Omsi.exe reads none of it, so a map or bus made for both games loses
nothing in OMSI 2.

**A map's own stations.** `radio.cfg` beside the map's `global.cfg`, written like the
player's (`name = address` a line). Its stations come first on the station buttons while the
map is driven, the player's follow (one with the same address as a map's is left out). Its `volume`
line is not read: loudness stays the player's business. The file is read again when another
map is loaded.

**Frequencies.** Behind the address may stand the frequencies the station is on, for radios
that show one: `| 94.6` is the station's frequency everywhere, `| 94.6 @ x, y` the one near
that place of the map. A station may have as many as it has transmitters along the route:

```
# name = address | frequency @ x, y | ...
Radiozurnal = https://rozhlas.stream/radiozurnal.mp3 | 94.6 @ 25500, 20000 | 90.9 @ 2307, -717
Regional    = https://example.org/regional.mp3 | 97.9
```

`x, y` are the game's metres: the tile's column and row times 300 m plus the place within
the tile. The easiest way to get them is the log: `spawned at entry point 38 "Chlum,hl.sil."
(12810.6, 3735.5, 60.3)` when a bus is put on the map, or an entry point's own numbers in
`global.cfg` (`[entrypoints]`: the place in the tile, then the tile's index into `[map]`).
The place nearest to the bus decides which frequency is shown; there is no blending, so one
place per town along the route is enough. A map's places are map positions, not
coordinates on the globe: many maps shorten and bend their routes.

**A bus's display.** A radio script that wants the station and the song in its display
declares the string variable `Snd_Radio_Text` (in a `[stringvarnamelist]` file of the bus)
and shows it in a text texture: openOMSI writes the text there, ten characters, a longer
text running through, while the radio plays. The variables the radio itself sets are those
of OMSI's radio plugins - `Snd_Radio` (1 while a cassette or the radio plays: the first
station), or `SndExt_Radio` (the station button, 0 = off) with `SndVol_Radio` (the volume,
0..1, up to 2). Dmitrij's "Magnitola" is served as it comes: while it shows its track,
its `magnitola_1` (`frequency@station`, `@` the line break) gets the map's frequency for the
place in its first line and the station and song in its second.

## What stays as in OMSI 2

The following behave as in OMSI 2 so that existing content works unchanged:

- the script stack (8 values) and registers (`l0`-`l9`, `s0`-`s9`);
- one `[spotlight]` lit at a time per vehicle (the one `Spot_Select` picks; `[spotlight_2]`,
  above, adds more);
- 100 particles per emitter.

In a LAN session, other players see up to 7 doors, 15 wheels and 127 lamps of a vehicle.
