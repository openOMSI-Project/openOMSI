//! Where the bus starts on a duty. As in OMSI 2 the bus stands at one of the map's entry
//! points (the parking places its author laid out: depots, termini, lay-bys); the launcher's
//! "Automatic" (`--auto-entry`) picks the one with the shortest way by road to the first
//! stop of the trip the duty starts with. (Putting the bus onto the trip's route in front of
//! the stop instead left it in the middle of a junction on line 31 at Maulbeerallee.)
//! A first stop no entry point leads to - Spandau's line 13 enters the map 6 m before
//! Zitadellenweg, at the end of the road - is left out: the duty starts at the next one.

use super::*;

/// The world `place_on_duty` opened, for `open_world` to go on with: map file, date and the
/// world with its map index and navigation map already made (it was opened twice at every
/// duty start, a second or more on a big map).
static PREOPENED: parking_lot::Mutex<Option<(PathBuf, i32, World)>> = parking_lot::Mutex::new(None);

/// The world opened for the duty start, if it is the one wanted now.
pub(crate) fn take_preopened(map: &Path, date: i32) -> Option<World> {
    let mut p = PREOPENED.lock();
    match p.take() {
        Some((m, d, w)) if m == map && d == date => Some(w),
        _ => None,
    }
}

/// The player's duty as the command line asks for it: a free drive along one route of a
/// line (`--free-line`), one put together of several tours (`--duty-leg`), else a tour of a
/// line (`--line`, `--tour`, `--trip`).
pub(crate) fn player_duty(sch: &mut schedule::Schedule, world: &World, args: &Args, now: f64) -> Result<schedule::PlayerDuty, String> {
    if let Some(route) = args.free_line.as_deref().filter(|r| !r.trim().is_empty()) {
        return sch.free_line_duty(world, args.line.as_deref().unwrap_or(""), route, now);
    }
    if args.duty_legs.is_empty() {
        let line = args.line.as_deref().unwrap_or("");
        sch.player_duty(world, line, args.tour.as_deref().unwrap_or(""), now, args.trip.as_deref(), args.whole_tour)
    } else {
        sch.player_plan(world, &args.duty_legs)
    }
}

/// With a duty (`--schedule --line`): the trip it starts with and the stop of it (`args.
/// duty_trip`, `args.duty_first_stop`), and with `--auto-entry` the entry point nearest to
/// that stop by road (`args.entry`).
pub(crate) fn place_on_duty(args: &mut Args) {
    if args.line.is_none() {
        return;
    }
    if !args.schedule || args.bus.is_none() || args.spawn.is_some() || args.is_resuming() {
        return;
    }
    let t0 = Instant::now();
    let clock = start_clock(args);
    let map_cfg = omsi_cfg::resolve_path(&args.root, &args.map);
    let world = match World::open(&args.root, &map_cfg, clock.date_code()) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("duty start: {e:#}");
            return;
        }
    };
    world.index();
    // whatever this finds, the world goes on to the game when the function ends
    struct Keep(Option<(PathBuf, i32, World)>);
    impl Drop for Keep {
        fn drop(&mut self) {
            *PREOPENED.lock() = self.0.take();
        }
    }
    let keep = Keep(Some((map_cfg, clock.date_code(), world)));
    let world = &keep.0.as_ref().unwrap().2;
    let mut sch = schedule::Schedule::new(&args.root, world, &clock);
    let now = parse_time(&args.time);
    let Ok(mut duty) = player_duty(&mut sch, world, args, now) else { return };
    let map = world.navigation_map();
    duty.learn_places(&map.places);
    let mut net = omsi_sim::traffic::Network { lanes: map.lanes, ..Default::default() };
    net.link(1.5);
    // the entry points and where they stand (heading from the object)
    let entries: Vec<(usize, String, DVec3, f64)> = {
        let pos = world.object_positions.lock();
        world
            .global
            .entry_points
            .iter()
            .enumerate()
            .filter_map(|(k, e)| pos.get(&e.object_id).map(|(p, r)| (k, e.name.clone(), *p, r[0])))
            .collect()
    };
    // the trip: the one the bus could start in time from the nearest entry (from the one
    // chosen by hand), trying the trips from the one under way on
    let first_k = duty.start_trip(now);
    for k in first_k..duty.trips.len().min(first_k + 3) {
        let trip = duty.trips[k].clone();
        let route = sch.trip_route_in(&net, &trip.name);
        if route.is_empty() {
            continue;
        }
        // per stop: the route lanes up to it (the way in must not skip it)
        for (j, stop) in trip.stops.iter().enumerate().take(4) {
            let Some(p) = stop.position else { continue };
            let Some(li) = route.iter().position(|&l| net.lanes[l].nearest_point(p).map(|q| q.1 < 25.0).unwrap_or(false)) else { continue };
            let targets: Vec<usize> = route[li.saturating_sub(60)..=li].to_vec();
            let stop_s = net.lanes[route[li]].nearest_point(p).map(|q| q.0).unwrap_or(0.0);
            let cost = |from: DVec3, heading: f64| -> Option<f64> {
                // an entry point on the route itself, before the stop (the terminus in
                // front of a bus put down at its entry): there already. The way round the
                // network to the start of the lane it stands on was all that was looked
                // for, and the first stop counted as out of reach.
                let before = targets.iter().any(|&l| {
                    net.lanes[l].nearest_point(from).is_some_and(|(s, d)| d < 6.0 && (l != route[li] || s <= stop_s + 1.0))
                });
                if before {
                    return Some((p - from).truncate().length());
                }
                let (path, _) = navigator::way_back(&net, from, heading, &targets, 40_000.0)?;
                Some(path.iter().map(|&l| net.lanes[l].length() as f64).sum())
            };
            // A trip that begins well before its first stop (a run out of the depot, whose
            // first stop is the line's terminus) starts at the entry point where its route
            // begins: the nearest way to the stop put the bus at the terminus and the depot
            // run was skipped (#1642).
            let route_start = net.lanes[route[0]].start();
            let depot = (args.auto_entry && (p - route_start).truncate().length() > 100.0)
                .then(|| {
                    entries
                        .iter()
                        .map(|e| (e, (e.2 - route_start).truncate().length()))
                        .filter(|(_, d)| *d < 40.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(e, _)| (e, route[..=li].iter().map(|&l| net.lanes[l].length() as f64).sum::<f64>()))
                })
                .flatten();
            let pick = if let Some(d) = depot {
                Some(d)
            } else if args.auto_entry {
                entries.iter().filter_map(|e| cost(e.2, e.3).map(|c| (e, c))).min_by(|a, b| a.1.total_cmp(&b.1))
            } else {
                entries.iter().find(|e| e.0 == args.entry.min(entries.len().saturating_sub(1))).and_then(|e| cost(e.2, e.3).map(|c| (e, c)))
            };
            let Some(((ei, name, _, _), way)) = pick else {
                log::info!("duty start: no entry point leads to stop {} '{}' of trip {}", j, stop.name.trim(), trip.name);
                continue;
            };
            log::info!(
                "duty start: trip {} ({} at {}) from stop {} '{}', the bus at entry point {} '{}' ({:.0} m by road){}, {:.1} s",
                k + 1,
                trip.name,
                schedule::hhmm(trip.departure),
                j,
                stop.name.trim(),
                ei,
                name.trim(),
                way,
                if args.auto_entry { " (automatic)" } else { "" },
                t0.elapsed().as_secs_f64()
            );
            args.entry = *ei;
            args.duty_trip = Some(k);
            args.duty_first_stop = j;
            // A start long before the tour's first trip (a duty picked at 00:00 whose first
            // bus leaves at 04:30) left the driver 270 minutes early, with nothing to do but
            // wait: the clock goes on to when the bus has to leave the entry point for the
            // stop - at town speed, with ten minutes to start the bus and set the IBIS.
            // (Not for a joining player: the clock is the host's.)
            let leave = stop.arr.max(trip.departure) - way / 7.0 - 600.0;
            if args.lan_join.is_none() && !crate::real_time::started_synced() && leave - now > 15.0 * 60.0 {
                let from = schedule::hhmm(now);
                args.time = format!("{:02}:{:02}:00", (leave / 3600.0) as i64 % 24, (leave / 60.0) as i64 % 60);
                log::info!("duty start: the tour's first trip leaves at {}: the clock goes from {from} to {}", schedule::hhmm(trip.departure), args.time);
                args.clock_moved = Some(omsi_ui::tr("The tour starts at %{time}: the clock was moved from %{from} to %{to}").replace("%{time}", &schedule::hhmm(trip.departure)).replace("%{from}", &from).replace("%{to}", &args.time[..5]));
            }
            return;
        }
    }
    log::info!("duty start: no stop of the next trips can be reached from the entry points; the timetable decides, {:.1} s", t0.elapsed().as_secs_f64());
}