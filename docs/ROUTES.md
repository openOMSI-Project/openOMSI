# How routes work

How timetables, chrono scenarios, `car_use`, hof files and IBIS behave in OMSI 2, and how
openOMSI follows that behaviour.

## Chrono scenarios

Loader the original (called by the map loader the original after `global.cfg`):

* Finds every `Chrono.cfg` under `<map>/Chrono` with `FindFilesRecursive`:
  depth first, a folder's subfolders before its own files, in the order Windows lists names
  (case-insensitive). The index in that list is the scenario's number; later ones override.
* the original parses one file into a 52-byte record (`chrono.f_10[i]`):
  `[name]`, `[description]` … `[end]`, `[startdate]` (flag bit 1, day at +0x1c),
  `[enddate]` (flag bit 2, day at +0x20), `[ticketpack]`, `[moneysystem]`,
  `[deactivate_lines]` = **a count, then that many line names** (`StrToInt`, then `ReadLn`
  count times).
* Dates are `YYYYMMDD`, converted to day numbers.
* the original(i, day)`: scenario *i* is in force when it has at least one date **and**
  `day >= start` (if given) **and** `day <= end` (if given) - both ends inclusive; a
  scenario without dates is never in force.
* the original builds the "Time Line" (every start/end change point) and the original(day)`
  sets each record's active byte (+0x29) for the current date (also called when the date
  changes: the original).

## Timetable loading

For k = last active scenario … 0, then the map's own `TTData` (k = −1):

1. `*.ttr` (tracks) and `*.ttp` (trips) of the folder.
2. `*.ttl`: the line's name is the file name. It is **skipped if a line of that name is
   already loaded** (a later scenario's file wins) **or if an active scenario with a higher
   index lists it in `[deactivate_lines]`**. So a scenario takes a line off only for the
   folders before it; a later scenario can bring the line back.
3. Then `car_use/*.ocu` of the map.

## Lines and tours (`.ttl`, reader the original)

* `[userallowed]` sets the line record's byte +4; the player's line list in the
  "select timetable" dialog (`Tform_settt.FormShow` → the original) shows **only** lines
  with it. (Berlin-Spandau: 5 of 23 lines; the rest are AI-only.) `[priority]` → +5.
* `[newtour]`: number, AI group (depot), day mask (empty → 1023). The mask is split into ten
  bytes at tour +0x20…+0x29.
* Validity: on a public holiday (`[holiday]` date) only bit 7 counts;
  otherwise the weekday bit (0 Monday … 5 Saturday, 6 Sunday). In addition, in the school
  holidays (a `[holidays]` range) bit **8** must be set, otherwise bit **9**. The printed
  timetable confirms it: bit 8 clear → "% : Not on school holidays",
  bit 9 clear → "~ : Only on school holidays".
* A tour started the previous day and running past midnight is checked against that day.

## `car_use/*.ocu`

`[valid]`, `[line]`, `[number_tour]`, `[type_tour]`, `[onlytypes]`, `[types_prefered]`. A
file whose `[line]` names no loaded line is
ignored with "has no valid [line] entry for the current chrono scenario".

## Hof files and IBIS

* `TRoadVehicle.LoadFromFile` loads **every** `*.hof` of the vehicle folder
  into its list (field 0x5e4) with `THof.LoadFromFile`.
* `TRoadVehicleInst.virtual_10` - `SetLineTo` / `ai_scheduled_settarget`;
  `TRoadVehicle.virtual_00` - `AI_target_index`.
* A hof's `[name]` is THof +4; its termini (`[addterminus]`, `[addterminus_allexit]`, 24-byte
  records: +0 code, +8 name, +0x10 strings, +0x14 all-exit flag) are THof +0x14.
* `ailists.cfg`: `[aigroup_depot]` = group name, then the **hof name** (a
  hof's `[name]`, not a file name) at group record +8; `[aigroup_depot_typgroup(_2)]` must
  come before it.
* An AI timetable bus takes the hof of its vehicle whose `[name]` equals the
  depot's hof name exactly (case-sensitive) into its selected-hof index (+0x7c8). Without
  a match the index stays 0: the vehicle folder's **first** hof (they are loaded in the
  order the folder lists them). The player's hof is the one chosen in the vehicle dialog
  (`Tform_selectVeh.ComboBox5Change`).
* Starting a trip calls `TRoadVehicleInst.virtual_10` with the
  trip's line and **terminus name** (the `.ttp`'s `[trip]` fields): the selected hof's
  terminus of that exact name gives `AI_target_index` (float, +0x674; unchanged when there
  is none), the line goes into the string variable `SetLineTo`, and the bus script's
  `ai_scheduled_settarget` trigger runs. The script does the rest (IBIS_TerminusIndex,
  codes, the displays). Coupled parts get the same call.
* So a bus without the map's hof shows the first hof's first terminus in the original. We
  go further (an addition, not a change): the depot's hof is also looked up by name in the
  other vehicle folders (`omsi_vehicle::hof::depot_anywhere`), so its termini and stops are
  the map's.

## What we changed to match

* `[deactivate_lines]` read as count + names (the count had been taken for a line name, so
  "1", "2", "17" … were taken off).
* Chrono validity inclusive at both ends; no dates → never; folders in the game's order.
* `TimetableData::load_with_chrono`: lines from the last folder back, taken off only by
  later scenarios.
* Tour masks: bit 8 = school holidays, bit 9 = school days (they were swapped, so tours
  marked for one ran on the other and showed as "not on this date" in the launcher).
* A tour whose `[ai_group]` names a plain `[aigroup_2]` pool (no `[aigroup_depot]` block)
  gets a depot file as well: Omsi.exe leaves such a bus's selected-hof index at 0 (its
  folder's first `.hof`), openOMSI takes the map's own depot of that folder where it has
  one. Without it the bus had no `SetLineTo`/`AI_target_index` and never ran
  `ai_scheduled_settarget`, and every mod whose display switches its destination picture on
  in that trigger (the HK roller blinds, the LED matrices) drove with a blank display.
