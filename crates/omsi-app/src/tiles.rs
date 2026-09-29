//! Placement of the map's attached objects, the map-wide index, and tile streaming.
//!
//! * `[attachObj]` hangs an object on one of the `[new_attachment]` points of its parent
//!   (traffic lights on a whip beam, line plates on a bus stop sign, the next house of a
//!   terrace) - none of them was placed before, which is most of what "objects missing" and
//!   "a sign floating in the air" were on the stock maps.
//! * `[splineAttachement]` puts a row of objects along a spline (street lamps, parking
//!   bays, catenary masts) and the splines after it in its tile;
//!   `[splineAttachement_repeater]` continues the row where the chain enters another tile,
//!   and the position there depends on the length of the chain from its start - which may
//!   lie in other tiles. [`MapIndex`] knows every spline of the map for that walk, and every
//!   object for entry points and bus stops of tiles that are not loaded.
//! * [`Streamer`] loads the tiles around the camera and the player's bus on a worker
//!   thread, uploads what they produced a little each frame and unloads the far ones.

use glam::{DVec2, DVec3, Mat4, Vec3};
use hashbrown::HashMap;
use omsi_geometry::SplineCurve;
use omsi_map::{tile_size, MapSpline, SplineAttachment, Tile};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// A spline of the map as far as a repeater's walk along its chain needs it.
#[derive(Debug, Clone, Copy)]
pub struct IndexedSpline {
    pub length: f64,
    pub prev: i64,
    pub next: i64,
}

/// What the map holds outside the loaded tiles: its splines (lengths and links), the spline
/// every `[splineAttachement]` row starts on, and where every object stands.
#[derive(Default)]
pub struct MapIndex {
    pub splines: HashMap<i64, IndexedSpline>,
    /// (tile index in global.cfg, attachment id) → (spline id, distance of the row's first
    /// object from the start of that spline - negative when it lies before it).
    pub masters: HashMap<(usize, i64), (i64, f64)>,
    /// Object id → (tile, world position with the ground under it, rotation).
    pub objects: HashMap<i64, ((i32, i32), DVec3, [f64; 3])>,
    /// The objects whose id another tile uses as well, by (tile, id): a map joined from two
    /// maps (two towns you cannot drive between) repeats ids, and an entry point named by
    /// its id was looked for in the other town.
    pub duplicates: HashMap<((i32, i32), i64), (DVec3, [f64; 3])>,
    /// Every object and spline file the map names (as written, lower case), with the number
    /// of records naming it and one tile that does.
    pub files: HashMap<String, (usize, (i32, i32))>,
    /// Tile → the world rectangle (x0, y0, x1, y1) its tile square and its splines (with
    /// room for their width) cover.
    pub covers: HashMap<(i32, i32), [f64; 4]>,
    pub tiles_read: usize,
    pub tiles_failed: usize,
}

impl MapIndex {
    /// Read every tile file of the map (with the active chrono patches) - no meshes, no
    /// object types, just the records. Spandau's 329 tiles take a fraction of a second on
    /// the worker pool; a tile that cannot be read is logged and left out. `tiles` are
    /// (index in global.cfg's `[map]` list, x, y, file): repeaters and timetable tracks name
    /// a tile by that index, which a missing tile file must not shift.
    pub fn build(tiles: &[(usize, i32, i32, PathBuf)], chrono_dirs: &[PathBuf]) -> MapIndex {
        /// What one tile adds besides its own index part: its rows (key, spline, start
        /// distance, interval) and its repeaters (master key, spline, first object index).
        type RowParts = (Vec<((usize, i64), i64, f64, f64)>, Vec<((usize, i64), i64, usize)>);
        let t0 = std::time::Instant::now();
        let parts: Vec<Option<(MapIndex, RowParts)>> = tiles
            .par_iter()
            .map(|(gi, tx, ty, path)| {
                let tile = read_tile(path, chrono_dirs)?;
                let mut part = MapIndex::default();
                let mut rows: RowParts = Default::default();
                for s in tile.splines.iter().filter(|s| !s.deleted) {
                    part.splines.insert(s.id, IndexedSpline { length: s.length, prev: s.prev_id, next: s.next_id });
                }
                for a in &tile.spline_attachments {
                    let Some(s) = tile.splines.get(a.spline_index.max(0) as usize) else { continue };
                    match a.repeater {
                        None => rows.0.push(((*gi, a.id), s.id, a.offset[2], a.interval)),
                        Some((master_tile, first)) => rows.1.push(((master_tile, a.id), s.id, first)),
                    }
                }
                let terrain = omsi_map::Terrain::load(&crate::scene::tile_companion(path, ".terrain")).ok();
                let origin = DVec2::new(*tx as f64 * tile_size(), *ty as f64 * tile_size());
                part.covers.insert((*tx, *ty), spline_cover(&tile, origin));
                let mut name = |f: &str| {
                    if f.trim().is_empty() {
                        return;
                    }
                    let e = part.files.entry(f.trim().replace('/', "\\").to_ascii_lowercase()).or_insert((0, (*tx, *ty)));
                    e.0 += 1;
                };
                for f in tile.objects.iter().map(|o| &o.file).chain(tile.attach_objects.iter().map(|o| &o.file)).chain(tile.spline_attachments.iter().map(|a| &a.file)).chain(tile.splines.iter().filter(|s| !s.deleted).map(|s| &s.file)) {
                    name(f);
                }
                for o in &tile.objects {
                    let ground = terrain.as_ref().map(|t| t.sample(o.pos[0].clamp(0.0, tile_size()) as f32, o.pos[1].clamp(0.0, tile_size()) as f32) as f64).unwrap_or(0.0);
                    part.objects.insert(o.id, ((*tx, *ty), DVec3::new(origin.x + o.pos[0], origin.y + o.pos[1], o.pos[2] + ground), o.rot));
                }
                // an object put on a spline (`[splineAttachement]`: an entry point or a stop
                // on the road): where the row's first object stands on its own spline - enough
                // to find it and load its tiles (the placed object gives the exact place; a
                // Novi Sad entry point was "not in the map" before)
                for a in tile.spline_attachments.iter().filter(|a| a.repeater.is_none()) {
                    let Some(s) = tile.splines.get(a.spline_index.max(0) as usize) else { continue };
                    let Some(first) = row_start(a, s, None).and_then(|st| place_on(a, s, origin, None, st).into_iter().next()) else { continue };
                    part.objects.entry(a.id).or_insert(((*tx, *ty), first.pose.pos, [first.pose.heading(), 0.0, 0.0]));
                }
                part.tiles_read = 1;
                Some((part, rows))
            })
            .collect();
        let mut index = MapIndex::default();
        let mut rows: RowParts = Default::default();
        for p in parts {
            match p {
                Some((p, (r, q))) => {
                    index.splines.extend(p.splines);
                    for (id, v) in p.objects {
                        if let Some(prev) = index.objects.get(&id).filter(|prev| prev.0 != v.0) {
                            index.duplicates.insert((prev.0, id), (prev.1, prev.2));
                            index.duplicates.insert((v.0, id), (v.1, v.2));
                        }
                        index.objects.insert(id, v);
                    }
                    index.covers.extend(p.covers);
                    for (f, (n, t)) in p.files {
                        index.files.entry(f).or_insert((0, t)).0 += n;
                    }
                    index.tiles_read += 1;
                    rows.0.extend(r);
                    rows.1.extend(q);
                }
                None => index.tiles_failed += 1,
            }
        }
        let mut intervals: HashMap<(usize, i64), f64> = HashMap::new();
        for (key, spline, d, interval) in rows.0 {
            index.masters.insert(key, (spline, d - chain_offset(&index, spline)));
            intervals.insert(key, interval);
        }
        // how well the chain model matches the editor: a repeater names the first object of
        // the row that lies on its spline
        let (mut checked, mut agree) = (0usize, 0usize);
        for (key, spline, first) in &rows.1 {
            let (Some(&(master_spline, d)), Some(&interval)) = (index.masters.get(key), intervals.get(key)) else { continue };
            if interval <= 0.0 {
                continue;
            }
            if let Some((acc, _)) = chain_distance(&index, master_spline, *spline, f64::MAX) {
                checked += 1;
                let ours = ((acc - d) / interval + 1e-6).ceil().max(0.0) as usize;
                agree += (ours == *first) as usize;
                if ours != *first && omsi_cfg::env::var_os("OMSI_DEBUG_REPEATERS").is_some() {
                    log::info!("repeater {:?} on spline {spline}: the map says object {first}, the chain {ours} (chain {acc:.2} m, start {d:.2} m, interval {interval} m: the map's first at {:.2} m, ours at {:.2} m)", key, d + *first as f64 * interval - acc, d + ours as f64 * interval - acc);
                }
            }
        }
        log::info!("map index: {} tiles read ({} unreadable), {} splines, {} objects, {} attachment rows ({} repeaters, {agree} of the {checked} on a known chain start where the map says), {} object and spline files in {:.2} s", index.tiles_read, index.tiles_failed, index.splines.len(), index.objects.len(), index.masters.len(), rows.1.len(), index.files.len(), t0.elapsed().as_secs_f64());
        index
    }

    /// The files the map names that this installation (with its content folder and archives)
    /// does not have, grouped by the add-on folder they belong to (the folder under
    /// `Sceneryobjects` / `Splines`): (folder, [(file, records, a tile)]).
    pub fn missing_files(&self, root: &Path) -> Vec<(String, Vec<(String, usize, (i32, i32))>)> {
        let named: Vec<(&String, &(usize, (i32, i32)))> = self.files.iter().collect();
        let mut missing: Vec<(String, usize, (i32, i32))> = named
            .into_par_iter()
            .filter(|(f, _)| !omsi_cfg::vfs::is_file(&omsi_cfg::resolve_path(root, f)))
            .map(|(f, (n, t))| (f.clone(), *n, *t))
            .collect();
        missing.sort();
        let mut groups: Vec<(String, Vec<(String, usize, (i32, i32))>)> = Vec::new();
        for m in missing {
            let folder = m.0.split('\\').take(2).collect::<Vec<_>>().join("\\");
            match groups.last_mut() {
                Some((g, list)) if *g == folder => list.push(m),
                _ => groups.push((folder, vec![m])),
            }
        }
        groups
    }
}

/// Room for a spline's width on either side of its centre line.
const SPLINE_HALF_WIDTH: f64 = 50.0;

/// The world rectangle a tile's square and its splines cover. A damaged length is bounded,
/// so one bad record cannot make a tile a neighbour of the whole map.
fn spline_cover(tile: &Tile, origin: DVec2) -> [f64; 4] {
    let ts = tile_size();
    let mut c = [origin.x, origin.y, origin.x + ts, origin.y + ts];
    for s in tile.splines.iter().filter(|s| !s.deleted && !s.file.trim().is_empty()) {
        let curve = SplineCurve::from_map(s, origin);
        let len = if s.length.is_finite() { s.length.clamp(0.0, 5000.0) } else { 0.0 };
        let steps = (len / 10.0).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let p = curve.point_at(len * i as f64 / steps as f64);
            if !(p.x.is_finite() && p.y.is_finite()) {
                continue;
            }
            c = [c[0].min(p.x - SPLINE_HALF_WIDTH), c[1].min(p.y - SPLINE_HALF_WIDTH), c[2].max(p.x + SPLINE_HALF_WIDTH), c[3].max(p.y + SPLINE_HALF_WIDTH)];
        }
    }
    c
}

/// A tile file with the chrono folders active on the sim date applied; None (logged) when
/// the file cannot be read.
pub fn read_tile(path: &Path, chrono_dirs: &[PathBuf]) -> Option<Tile> {
    let mut tile = match Tile::load(path) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("{e}");
            return None;
        }
    };
    if let Some(name) = path.file_name() {
        for c in chrono_dirs {
            let p = omsi_cfg::resolve_path(c, &name.to_string_lossy());
            if !omsi_cfg::vfs::is_file(&p) {
                continue;
            }
            match Tile::load(&p) {
                Ok(patch) => {
                    let unmatched = tile.apply_chrono(&patch);
                    if unmatched > 0 {
                        log::debug!("chrono {}: {unmatched} selections name nothing in the tile", p.display());
                    }
                }
                Err(e) => log::warn!("{e}"),
            }
        }
        // (after the chrono patches: they are written in the tile's own measure)
        if let Some((_, ty)) = omsi_map::tile_index_of(&name.to_string_lossy()) {
            tile.fit_to_world_grid(ty);
        }
    }
    Some(tile)
}

/// The transform of `[new_attachment]` point `a` in its parent's frame (x right, y forward,
/// z up). The operations act on the attached object in the order they are listed, like the
/// original's Direct3D matrices (`attach_rot_z 180` then `attach_trans` turns a traffic
/// light round and then hangs it at the end of the beam), and the angles have the sign
/// convention of the model's origin rotations.
pub fn attachment_matrix(a: &omsi_scenery::sco::Attachment) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    for (op, v) in &a.ops {
        let step = match (op.as_str(), v.as_slice()) {
            ("attach_trans", [x, y, z, ..]) => Mat4::from_translation(Vec3::new(*x, *y, *z)),
            ("attach_rot_x", [r, ..]) => Mat4::from_rotation_x(-r.to_radians()),
            ("attach_rot_y", [r, ..]) => Mat4::from_rotation_y(-r.to_radians()),
            ("attach_rot_z", [r, ..]) => Mat4::from_rotation_z(-r.to_radians()),
            _ => continue,
        };
        m = step * m;
    }
    m
}

/// A world pose: position, and the rotation of the object's frame.
#[derive(Debug, Clone, Copy)]
pub struct Pose {
    pub pos: DVec3,
    pub rot: Mat4,
}

impl Pose {
    /// The pose of an object hanging on attachment point `attach` of this pose, turned by
    /// its own heading/pitch/bank `own`.
    pub fn attached(&self, attach: &Mat4, own: [f64; 3]) -> Pose {
        let local = attach.transform_point3(Vec3::ZERO);
        let pos = self.pos + self.rot.transform_vector3(local).as_dvec3();
        let (_, r, _) = attach.to_scale_rotation_translation();
        let rot = self.rot * Mat4::from_quat(r) * omsi_geometry::object_rotation(own);
        Pose { pos, rot }
    }

    /// Heading (degrees, clockwise from north) of the pose's forward axis.
    pub fn heading(&self) -> f64 {
        let f = self.rot.transform_vector3(Vec3::Y);
        (f.x as f64).atan2(f.y as f64).to_degrees().rem_euclid(360.0)
    }
}

/// One object of a spline attachment row.
#[derive(Debug, Clone, Copy)]
pub struct RowObject {
    /// Index of the object in its row.
    pub index: usize,
    pub pose: Pose,
}

/// How long the chain from the start of spline `from` is before `to` starts, following the
/// `next` links (and the direction flips where two splines meet end to end), and whether
/// `to` is then run backwards. None when `to` is not on the chain within `limit` metres.
pub fn chain_distance(index: &MapIndex, from: i64, to: i64, limit: f64) -> Option<(f64, bool)> {
    let mut cur = from;
    let mut forward = true;
    let mut acc = 0.0;
    let mut seen: Vec<i64> = Vec::new();
    loop {
        let s = index.splines.get(&cur)?;
        if cur == to && !seen.is_empty() {
            return Some((acc, !forward));
        }
        if seen.contains(&cur) || seen.len() > 500 || acc > limit {
            return None;
        }
        seen.push(cur);
        acc += s.length;
        let next_id = if forward { s.next } else { s.prev };
        let next = index.splines.get(&next_id)?;
        if next.prev == cur {
            forward = true;
        } else if next.next == cur {
            forward = false;
        }
        cur = next_id;
    }
}

/// How far the start of spline `id` lies from the start of its chain: the length of the
/// splines before it, following the `prev` links back (and the direction flips where two
/// splines meet end to end) until the chain begins or comes round to a spline already
/// passed. A row's start distance counts from there: with it, 176 of the 180 repeaters of
/// Berlin-Spandau that can be checked start at the object the map names (165 counting from
/// the master's own spline), and the buffer stops stand a few metres before their track
/// ends instead of in the middle of the rails.
pub fn chain_offset(index: &MapIndex, id: i64) -> f64 {
    let mut cur = id;
    let mut forward = true;
    let mut acc = 0.0;
    let mut seen: Vec<i64> = vec![id];
    while let Some(s) = index.splines.get(&cur) {
        let prev_id = if forward { s.prev } else { s.next };
        let Some(p) = index.splines.get(&prev_id) else { break };
        if seen.contains(&prev_id) || seen.len() > 500 {
            break;
        }
        if p.next == cur {
            forward = true;
        } else if p.prev == cur {
            forward = false;
        }
        acc += p.length;
        seen.push(prev_id);
        cur = prev_id;
    }
    acc
}

/// How far past the end of a chain an object still stands at its end: the editor's sums of
/// spline lengths drift by a little (Spandau has buffer stops 0.1 and 0.5 m past theirs).
const CHAIN_END_TOLERANCE: f64 = 1.0;

/// Where a row record's objects begin on its own spline.
#[derive(Debug, Clone, Copy)]
struct RowStart {
    /// Distance of the first object from the start of the spline (the objects before the
    /// start are skipped).
    s: f64,
    /// Index of that object in the row.
    j: usize,
    /// The spline runs against the row.
    backwards: bool,
    /// The row's start and this spline's start, as distances along the chain from the start
    /// of the master's spline.
    d0: f64,
    acc: f64,
}

/// The first object of a row starting at `d0` that lies after `acc` along the chain (one
/// standing right on a joint belongs to the spline before): (its distance past `acc`, its
/// index).
fn first_after(d0: f64, acc: f64, interval: f64) -> Option<(f64, usize)> {
    if interval > 0.0 {
        let j = ((acc - d0) / interval + 1e-6).ceil().max(0.0) as usize;
        Some((d0 + j as f64 * interval - acc, j))
    } else if d0 >= acc - 1e-6 {
        Some((d0 - acc, 0))
    } else {
        None
    }
}

/// Where the objects of row record `att` begin on `spline`, its own spline.
fn row_start(att: &SplineAttachment, spline: &MapSpline, index: Option<&MapIndex>) -> Option<RowStart> {
    let interval = att.interval.max(0.0);
    let d = att.offset[2];
    let Some((master_tile, first)) = att.repeater else {
        // the start distance counts from the start of the chain
        let d0 = index.map(|ix| d - chain_offset(ix, spline.id)).unwrap_or(d);
        return Some(RowStart { s: d0, j: 0, backwards: false, d0, acc: 0.0 });
    };
    let master = index.and_then(|ix| ix.masters.get(&(master_tile, att.id)).copied());
    if let (Some(ix), Some((master_spline, d0))) = (index, master) {
        let limit = d0.max(0.0) + att.range.max(0.0) + spline.length.max(0.0) + 1000.0;
        let Some((acc, backwards)) = chain_distance(ix, master_spline, spline.id, limit) else {
            if interval > 0.0 {
                log::debug!("spline attachment {} ({}): spline {} not on the chain of {master_spline}", att.id, att.file, spline.id);
            }
            return None;
        };
        return first_after(d0, acc, interval).map(|(s, j)| RowStart { s, j, backwards, d0, acc });
    }
    if interval > 0.0 && index.is_some() {
        // the master is not in the map (a broken chain in a mod map): keep the row's
        // spacing with its own count, starting at the interval's phase
        let s = d % interval;
        return Some(RowStart { s, j: first, backwards: false, d0: s - first as f64 * interval, acc: 0.0 });
    }
    None
}

/// The objects of row record `att` on `spline` from `start` on, while `j * interval <= range`.
fn place_on(att: &SplineAttachment, spline: &MapSpline, origin: DVec2, index: Option<&MapIndex>, start: RowStart) -> Vec<RowObject> {
    let curve = SplineCurve { half_cant_width: omsi_geometry::half_cant_width_of(&spline.file), ..SplineCurve::from_map(spline, origin) };
    let len = spline.length.max(0.0);
    let interval = att.interval.max(0.0);
    let range = att.range.max(0.0);
    let (x, h) = (att.offset[0], att.offset[1]);
    let RowStart { mut s, mut j, backwards, .. } = start;
    // where the chain ends with this spline (in the row's direction), an object a little
    // past the end stands at the end
    let far = if backwards { spline.prev_id } else { spline.next_id };
    let chain_ends = index.map(|ix| !ix.splines.contains_key(&far)).unwrap_or(far == 0);
    let end = len + if chain_ends { CHAIN_END_TOLERANCE } else { 1e-6 };
    let mut out = Vec::new();
    loop {
        if interval > 0.0 && j as f64 * interval > range + 1e-6 {
            break;
        }
        if s > end {
            break;
        }
        if s >= -1e-6 {
            let along = s.clamp(0.0, len);
            // a spline run against the row: the row's distance counts from its far end and
            // the row's right is the spline's left
            let (u, side, turn) = if backwards { (len - along, -x, 180.0) } else { (along, x, 0.0) };
            let pos = curve.offset_point(u, side, h);
            let heading = curve.heading_at(u) + turn + att.rot[0];
            let [_, mut pitch, mut bank] = omsi_geometry::map_rotation(att.rot);
            if att.tilt {
                let sign = if backwards { -1.0 } else { 1.0 };
                pitch += sign * curve.slope_at(u).atan().to_degrees();
                // (the original: the cant's angle, atan of the percentage, and only
                // for an object standing within the half cant width)
                if side.abs() < curve.half_cant_width {
                    bank += sign * (curve.cant_at(u) / 100.0).atan().to_degrees();
                }
            }
            out.push(RowObject { index: j, pose: Pose { pos, rot: omsi_geometry::object_rotation([heading, pitch, bank]) } });
        }
        if interval <= 0.0 {
            break;
        }
        s += interval;
        j += 1;
        if out.len() > 4096 {
            log::warn!("spline attachment {} ({}): more than 4096 objects on one spline, row cut short", att.id, att.file);
            break;
        }
    }
    out
}

/// The objects of row record `att` of a tile whose splines are `splines`, whose curves start
/// at `origin` (the tile origin). Object `j` of a row lies `d + j * interval` along the chain
/// from the chain's start (see [`chain_offset`]) while `j * interval <= range`: on the
/// record's own spline and on the splines after it in the chain as long as they are in the
/// same tile - where the
/// chain enters another tile, a repeater there carries the row on (every one of the 169
/// repeaters of Berlin-Spandau with objects lies on the first spline of a new tile, and 413
/// later splines in the record's own tile have none). (index into `splines`, object).
pub fn tile_row_objects(att: &SplineAttachment, splines: &[MapSpline], origin: DVec2, index: Option<&MapIndex>) -> Vec<(usize, RowObject)> {
    let si = att.spline_index.max(0) as usize;
    let Some(own) = splines.get(si).filter(|s| !s.deleted) else { return Vec::new() };
    let Some(start) = row_start(att, own, index) else { return Vec::new() };
    let interval = att.interval.max(0.0);
    let last = start.d0 + if interval > 0.0 { (att.range.max(0.0) / interval + 1e-6).floor() * interval } else { 0.0 };
    let mut out: Vec<(usize, RowObject)> = place_on(att, own, origin, index, start).into_iter().map(|o| (si, o)).collect();
    let (mut cur, mut acc, mut backwards) = (si, start.acc, start.backwards);
    let mut seen = vec![si];
    loop {
        acc += splines[cur].length.max(0.0);
        if last <= acc + 1e-6 {
            break;
        }
        let c = &splines[cur];
        let next_id = if backwards { c.prev_id } else { c.next_id };
        let Some(ni) = splines.iter().position(|s| s.id == next_id && !s.deleted) else { break };
        if seen.contains(&ni) || seen.len() > 500 {
            break;
        }
        seen.push(ni);
        let next = &splines[ni];
        if next.prev_id == c.id {
            backwards = false;
        } else if next.next_id == c.id {
            backwards = true;
        }
        let Some((s, j)) = first_after(start.d0, acc, interval) else { break };
        out.extend(place_on(att, next, origin, index, RowStart { s, j, backwards, d0: start.d0, acc }).into_iter().map(|o| (ni, o)));
        cur = ni;
    }
    out
}

/// A finished batch from the worker: the prepared tiles, their statistics and how long the
/// preparation took.
/// A finished batch: the tiles asked for, what was made of them (a tile that could not be
/// made is missing), statistics and seconds.
type Batch = (Vec<(i32, i32)>, Vec<crate::scene::Prepared>, crate::scene::LoadStats, f64);

/// The threads tiles are prepared on while the game runs: a pool of their own, a third of
/// the cores, so that the frame's parallel work (culling, the AI scripts) never waits for a
/// tile to finish on the shared pool - and at a lower priority, so that they do not take
/// the cores those workers need either (see `threads`).
fn loader_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) / 3).max(2);
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("tile loader {i}")).start_handler(|_| crate::threads::lower_thread_priority()).build().expect("tile loader pool")
    })
}

/// Candidates whose tile rectangles may touch `radius` around a center. The exact
/// circular distance check follows in `missing`.
fn tile_candidates(
    lookup: &hashbrown::HashMap<(i32, i32), Vec<usize>>,
    tile_count: usize,
    centers: &[DVec3],
    radius: f64,
) -> Vec<usize> {
    let ts = tile_size();
    let mut candidates: Vec<usize> = Vec::new();
    for c in centers {
        let min_x = ((c.x - radius) / ts).floor() as i32 - 1;
        let max_x = ((c.x + radius) / ts).floor() as i32;
        let min_y = ((c.y - radius) / ts).floor() as i32 - 1;
        let max_y = ((c.y + radius) / ts).floor() as i32;
        let cells = (max_x as i64 - min_x as i64 + 1) * (max_y as i64 - min_y as i64 + 1);
        if cells > tile_count as i64 * 2 {
            candidates.extend(0..tile_count);
            break;
        }
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                if let Some(indices) = lookup.get(&(x, y)) {
                    candidates.extend_from_slice(indices);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

/// Loads the tiles around a few points as they move (the camera, and the player's bus,
/// which must not lose the ground under it when the free camera flies off), like OMSI's
/// tile streaming: the tiles within `load_radius` of any of them are read, tessellated and
/// placed on a worker thread (a few at a time, nearest first), put on the GPU a little each
/// frame, and the tiles beyond `unload_radius` of all of them are taken away again, so the
/// memory a map needs is bounded by the view distance and not by the size of the map.
pub struct Streamer {
    world: std::sync::Arc<crate::scene::World>,
    tiles: Vec<(i32, i32, PathBuf)>,
    /// Indices into `tiles`, including duplicate coordinates, in map-file order.
    tile_lookup: hashbrown::HashMap<(i32, i32), Vec<usize>>,
    pub load_radius: f64,
    pub unload_radius: f64,
    tx: std::sync::mpsc::Sender<Batch>,
    rx: std::sync::mpsc::Receiver<Batch>,
    inflight: bool,
    queue: std::collections::VecDeque<crate::scene::PendingUpload>,
    /// Tiles in flight or waiting for upload.
    requested: hashbrown::HashSet<(i32, i32)>,
    /// Tiles that could not be made (a damaged file): not asked for again.
    failed: hashbrown::HashSet<(i32, i32)>,
    /// The first area around the start: (tiles, of which uploaded).
    initial: Option<(hashbrown::HashSet<(i32, i32)>, usize)>,
    pub stats: crate::scene::LoadStats,
    pub loaded_total: usize,
    pub unloaded_total: usize,
    /// Seconds the worker spent preparing, and the longest upload work of one frame.
    pub prepare_secs: f64,
    pub worst_upload_ms: f64,
    /// The longest frame share of streaming after the first area (upload, unload and the
    /// lists), in ms, and how often it took more than 16 ms.
    pub worst_frame_ms: f64,
    pub slow_frames: usize,
    started: std::time::Instant,
    last_summary: std::time::Instant,
    /// Tiles loaded and unloaded when the heap's free pages were last given back, and when.
    relieved_at: (usize, usize, std::time::Instant),
}

impl Streamer {
    /// Start streaming around `centers`: the tiles within `initial_radius` of them form the
    /// first area, which is what the loading screen waits for.
    pub fn new(world: std::sync::Arc<crate::scene::World>, centers: &[DVec3], load_radius: f64, initial_radius: f64) -> Streamer {
        let tiles = world.select_tiles(None, None);
        let mut tile_lookup: hashbrown::HashMap<(i32, i32), Vec<usize>> = hashbrown::HashMap::new();
        for (i, t) in tiles.iter().enumerate() {
            tile_lookup.entry((t.0, t.1)).or_default().push(i);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = Streamer {
            world,
            tiles,
            tile_lookup,
            load_radius,
            unload_radius: load_radius + tile_size() * 1.5,
            tx,
            rx,
            inflight: false,
            queue: Default::default(),
            requested: Default::default(),
            failed: Default::default(),
            initial: None,
            stats: Default::default(),
            loaded_total: 0,
            unloaded_total: 0,
            prepare_secs: 0.0,
            worst_upload_ms: 0.0,
            worst_frame_ms: 0.0,
            slow_frames: 0,
            started: std::time::Instant::now(),
            last_summary: std::time::Instant::now(),
            relieved_at: (0, 0, std::time::Instant::now()),
        };
        let first: hashbrown::HashSet<(i32, i32)> = s.tiles.iter().filter(|t| Self::nearest(centers, t.0, t.1) <= initial_radius.min(load_radius)).map(|t| (t.0, t.1)).collect();
        log::info!("tile streaming: {} tiles in the map, load radius {:.0} m around {} points ({} tiles now), first area {} tiles", s.tiles.len(), load_radius, centers.len(), s.tiles.iter().filter(|t| Self::nearest(centers, t.0, t.1) <= load_radius).count(), first.len());
        s.initial = Some((first, 0));
        s
    }

    /// Distance from `p` to the nearest point of tile (tx, ty).
    pub fn distance(p: DVec3, tx: i32, ty: i32) -> f64 {
        let ts = tile_size();
        let (x0, y0) = (tx as f64 * ts, ty as f64 * ts);
        let dx = (x0 - p.x).max(0.0).max(p.x - (x0 + ts));
        let dy = (y0 - p.y).max(0.0).max(p.y - (y0 + ts));
        (dx * dx + dy * dy).sqrt()
    }

    /// Distance from the nearest of `centers` to tile (tx, ty).
    pub fn nearest(centers: &[DVec3], tx: i32, ty: i32) -> f64 {
        centers.iter().map(|c| Self::distance(*c, tx, ty)).fold(f64::INFINITY, f64::min)
    }

    /// Read tiles `keys` again (a chrono scenario changed them, the season's textures
    /// changed): those loaded go and come back with the next batches. `None`: every loaded
    /// tile.
    pub fn reload(&mut self, renderer: &omsi_render::Renderer, scene: &mut omsi_render::Scene, keys: Option<&[(i32, i32)]>, audio: Option<&omsi_audio::AudioEngine>) {
        let keys: Vec<(i32, i32)> = match keys {
            Some(k) => k.to_vec(),
            None => {
                self.world.forget_all_staged();
                self.world.loaded_tiles()
            }
        };
        let mut freed = false;
        for k in &keys {
            self.failed.remove(k);
            if !self.requested.contains(k) {
                freed |= self.world.unload_tile(renderer, scene, *k, audio);
            }
        }
        if freed {
            self.world.trim_object_types();
        }
        self.world.refresh_tile_lists();
        log::info!("tile streaming: {} tiles to be read again", keys.len());
    }

    /// (uploaded, total) of the first area while it is still loading.
    pub fn initial_progress(&self) -> Option<(usize, usize)> {
        self.initial.as_ref().filter(|(set, done)| *done < set.len()).map(|(set, done)| (*done, set.len()))
    }

    fn missing(&self, centers: &[DVec3]) -> Vec<(f64, (i32, i32, PathBuf))> {
        let loaded: hashbrown::HashSet<(i32, i32)> = self.world.loaded_tiles().into_iter().collect();
        let mut out: Vec<(f64, (i32, i32, PathBuf))> = tile_candidates(&self.tile_lookup, self.tiles.len(), centers, self.load_radius)
            .into_iter()
            .map(|i| &self.tiles[i])
            .filter(|t| !loaded.contains(&(t.0, t.1)) && !self.requested.contains(&(t.0, t.1)) && !self.failed.contains(&(t.0, t.1)))
            .map(|t| (Self::nearest(centers, t.0, t.1), t.clone()))
            .filter(|(d, _)| *d <= self.load_radius)
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// One frame of streaming around `centers`: take in what the worker finished, upload and
    /// place for about `budget` (always a little), unload what is far from all of them (for
    /// about half a budget more, at least one tile), and start the next batch. Returns true
    /// when the loaded tiles changed (the caller then refreshes whatever copies world data).
    pub fn update(&mut self, renderer: &omsi_render::Renderer, scene: &mut omsi_render::Scene, centers: &[DVec3], budget: std::time::Duration, audio: Option<&omsi_audio::AudioEngine>) -> bool {
        let mut changed = false;
        while let Ok((asked, prepared, stats, secs)) = self.rx.try_recv() {
            self.inflight = false;
            // a tile the worker could not make is let go (else it stayed "requested" for
            // ever: never retried, never unloaded, and the loading screen waited for it)
            let made: hashbrown::HashSet<(i32, i32)> = prepared.iter().map(|p| (p.tx, p.ty)).collect();
            for k in asked.into_iter().filter(|k| !made.contains(k)) {
                log::warn!("tile streaming: tile {},{} could not be loaded; left out", k.0, k.1);
                self.requested.remove(&k);
                self.failed.insert(k);
                if let Some((_, done)) = self.initial.as_mut().filter(|(set, _)| set.contains(&k)) {
                    *done += 1;
                    changed = true;
                }
            }
            self.prepare_secs += secs;
            self.stats.add_prepared(&stats);
            self.queue.extend(prepared.into_iter().map(|p| self.world.begin_upload(p)));
        }
        let t0 = std::time::Instant::now();
        let mut uploaded = 0usize;
        let mut freed_types = false;
        let deadline = t0 + budget;
        while let Some(mut p) = self.queue.pop_front() {
            let key = p.key();
            let initial = self.initial.as_ref().map(|(set, _)| set.contains(&key)).unwrap_or(false);
            if !initial && Self::nearest(centers, key.0, key.1) > self.unload_radius {
                // gone out of range while it was being prepared: what it already holds on
                // the GPU goes back with it
                self.requested.remove(&key);
                self.world.abandon_upload(renderer, scene, p, audio);
                changed = true;
                continue;
            }
            let t = std::time::Instant::now();
            // its new textures and object types first, then the tile itself, a little a frame
            let ready = self.world.upload_step(renderer, scene, &mut p, Some(deadline)) && self.world.place_step(renderer, scene, &mut p, Some(deadline));
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if self.initial.as_ref().map(|(_, done)| *done > 0).unwrap_or(true) {
                self.worst_upload_ms = self.worst_upload_ms.max(ms);
            }
            if !ready {
                self.queue.push_front(p);
                break;
            }
            self.requested.remove(&key);
            let mut stats = crate::scene::LoadStats::default();
            self.world.commit_upload(p, &mut stats);
            self.stats.objects += stats.objects;
            self.stats.trees += stats.trees;
            self.stats.splines += stats.splines;
            self.stats.tiles += 1;
            self.loaded_total += 1;
            if let Some((_, done)) = self.initial.as_mut().filter(|(set, _)| set.contains(&key)) {
                *done += 1;
            }
            changed = true;
            uploaded += 1;
            if t0.elapsed() >= budget {
                break;
            }
        }
        let t_upload = t0.elapsed();
        let mut unloaded = 0usize;
        if self.initial.as_ref().map(|(set, done)| *done >= set.len()).unwrap_or(false) {
            let (set, _) = self.initial.take().unwrap();
            log::info!("tile streaming: first area of {} tiles loaded in {:.2} s", set.len(), self.started.elapsed().as_secs_f64());
        }
        // Far tiles go (never the ones on their way in), the farthest first and only as many
        // as fit in the budget (at least one a frame): a camera jump across a big map had
        // 42 tiles taken away in one frame of 400 ms. Unloading gets its own share of the
        // budget, so that a busy upload queue cannot hold the memory of the old area.
        if self.initial.is_none() {
            let now = std::time::Instant::now();
            let unload_deadline = deadline.max(now + budget / 2);
            let mut far: Vec<(f64, (i32, i32))> = self.world.loaded_tiles().into_iter().filter(|k| !self.requested.contains(k)).map(|k| (Self::nearest(centers, k.0, k.1), k)).filter(|(d, _)| *d > self.unload_radius).collect();
            far.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, key) in far {
                if unloaded > 0 && std::time::Instant::now() >= unload_deadline {
                    break;
                }
                freed_types |= self.world.unload_tile(renderer, scene, key, audio);
                self.unloaded_total += 1;
                unloaded += 1;
                changed = true;
            }
        }
        if freed_types {
            self.world.trim_object_types();
        }
        if unloaded > 0 {
            // the tiles only they depended on go too
            self.world.trim_staged(&self.requested);
        }
        let t_unload = t0.elapsed();
        if changed {
            self.world.refresh_tile_lists();
        }
        // Loading and unloading tiles leaves the allocator with pages it keeps for later: they
        // count as the game's memory until they are handed back (half a gigabyte on
        // Ahlheim). At most every few seconds, on a thread of its own.
        if (self.loaded_total, self.unloaded_total) != (self.relieved_at.0, self.relieved_at.1) && self.relieved_at.2.elapsed().as_secs_f32() >= 4.0 && self.queue.is_empty() && !self.inflight {
            self.relieved_at = (self.loaded_total, self.unloaded_total, std::time::Instant::now());
            self.world.compact_slots(renderer, scene);
            crate::release_free_memory();
        }
        if self.last_summary.elapsed().as_secs_f32() >= 10.0 && omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
            self.last_summary = std::time::Instant::now();
            let at: Vec<String> = centers.iter().map(|c| format!("({:.0}, {:.0})", c.x, c.y)).collect();
            log::info!("tile streaming at {}: {} loaded / {} unloaded so far; {}", at.join(" "), self.loaded_total, self.unloaded_total, self.world.gpu_summary(scene));
            if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                log::info!("  {}", self.world.cpu_summary());
            }
        }
        let total = t0.elapsed();
        if self.initial.is_none() && total.as_secs_f64() * 1000.0 > 16.0 {
            self.slow_frames += 1;
            if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                log::info!("tile streaming: {:.0} ms this frame ({uploaded} uploaded in {:.0} ms, {unloaded} unloaded in {:.0} ms, lists {:.0} ms)", total.as_secs_f64() * 1000.0, t_upload.as_secs_f64() * 1000.0, (t_unload - t_upload).as_secs_f64() * 1000.0, (total - t_unload).as_secs_f64() * 1000.0);
            }
        }
        if self.initial.is_none() {
            self.worst_frame_ms = self.worst_frame_ms.max(total.as_secs_f64() * 1000.0);
        }
        if !self.inflight {
            let missing = self.missing(centers);
            if !missing.is_empty() {
                // the first area in bigger bites (the loading screen shows the progress),
                // then a few tiles at a time
                let n = if self.initial.is_some() { 6 } else { 3 };
                let batch: Vec<(i32, i32, PathBuf)> = missing.into_iter().take(n).map(|(_, t)| t).collect();
                let keys: Vec<(i32, i32)> = batch.iter().map(|t| (t.0, t.1)).collect();
                self.requested.extend(keys.iter().copied());
                self.inflight = true;
                let world = self.world.clone();
                let tx = self.tx.clone();
                // the first area gets every core; later tiles only the loader's own threads
                let first = self.initial.is_some();
                let spawned = std::thread::Builder::new().name("tile loader".into()).spawn(move || {
                    let t = std::time::Instant::now();
                    // (a panic on a damaged file must not end the streaming: the batch comes
                    // back empty and its tiles are let go)
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| if first { world.prepare_tiles(&batch) } else { loader_pool().install(|| world.prepare_tiles(&batch)) }));
                    let (prepared, stats) = made.unwrap_or_else(|_| {
                        log::error!("tile streaming: loading tiles {:?} failed", batch.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>());
                        Default::default()
                    });
                    let _ = tx.send((batch.iter().map(|t| (t.0, t.1)).collect(), prepared, stats, t.elapsed().as_secs_f64()));
                });
                if let Err(e) = spawned {
                    log::warn!("tile loader thread: {e}");
                    self.inflight = false;
                    for k in keys {
                        self.requested.remove(&k);
                    }
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_tile_search_matches_full_distance_scan() {
        let coords: Vec<(i32, i32)> = (-5..=5).flat_map(|x| (-5..=5).map(move |y| (x, y))).collect();
        let mut lookup: hashbrown::HashMap<(i32, i32), Vec<usize>> = hashbrown::HashMap::new();
        for (i, &key) in coords.iter().enumerate() {
            lookup.entry(key).or_default().push(i);
        }
        let ts = tile_size();
        for centers in [
            vec![DVec3::new(0.0, 0.0, 0.0)],
            vec![DVec3::new(-0.3 * ts, 1.8 * ts, 0.0), DVec3::new(3.2 * ts, -2.1 * ts, 0.0)],
        ] {
            for radius in [0.0, 0.7 * ts, 2.5 * ts, 20.0 * ts] {
                let expected: Vec<usize> = coords.iter().enumerate()
                    .filter_map(|(i, &(x, y))| (Streamer::nearest(&centers, x, y) <= radius).then_some(i))
                    .collect();
                let actual: Vec<usize> = tile_candidates(&lookup, coords.len(), &centers, radius)
                    .into_iter()
                    .filter(|&i| Streamer::nearest(&centers, coords[i].0, coords[i].1) <= radius)
                    .collect();
                assert_eq!(actual, expected, "centers {centers:?}, radius {radius}");
            }
        }
    }

    /// The objects row record `att` puts on its own spline `spline`.
    fn row_objects(att: &SplineAttachment, spline: &MapSpline, origin: DVec2, index: Option<&MapIndex>) -> Vec<RowObject> {
        row_start(att, spline, index).map(|start| place_on(att, spline, origin, index, start)).unwrap_or_default()
    }

    fn spline(id: i64, prev: i64, next: i64, length: f64) -> MapSpline {
        MapSpline { file: String::new(), id, prev_id: prev, next_id: next, pos: [0.0, 0.0, 10.0], heading: 0.0, length, ..Default::default() }
    }

    fn row(interval: f64, range: f64, d: f64, repeater: Option<(usize, usize)>) -> SplineAttachment {
        SplineAttachment { file: "lamp.sco".into(), id: 5, spline_index: 0, offset: [3.0, 0.25, d], interval, range, repeater, ..Default::default() }
    }

    #[test]
    fn master_row() {
        let s = spline(1, 0, 2, 100.0);
        let objs = row_objects(&row(30.0, 1000.0, 10.0, None), &s, DVec2::ZERO, None);
        let at: Vec<f64> = objs.iter().map(|o| o.pose.pos.y).collect();
        // 10, 40, 70 and 100 m: the one on the joint belongs to this spline
        assert_eq!(at.len(), 4);
        assert!((at[0] - 10.0).abs() < 1e-6 && (at[3] - 100.0).abs() < 1e-6, "{at:?}");
        // right of a spline heading north is east, the height is above the spline
        assert!((objs[0].pose.pos.x - 3.0).abs() < 1e-6 && (objs[0].pose.pos.z - 10.25).abs() < 1e-6);
        // range 45 m: objects 0 and 1 only
        assert_eq!(row_objects(&row(30.0, 45.0, 10.0, None), &s, DVec2::ZERO, None).len(), 2);
        // a single object beyond the end belongs to the next spline of the chain
        assert!(row_objects(&row(0.0, 0.0, 130.0, None), &s, DVec2::ZERO, None).is_empty());
        // ... unless the chain ends here: a little past the end is at the end
        let dead_end = spline(1, 0, 0, 100.0);
        let one = row_objects(&row(0.0, 0.0, 100.6, None), &dead_end, DVec2::ZERO, None);
        assert_eq!(one.len(), 1);
        assert!((one[0].pose.pos.y - 100.0).abs() < 1e-6);
        assert!(row_objects(&row(0.0, 0.0, 102.0, None), &dead_end, DVec2::ZERO, None).is_empty());
    }

    /// The chain of the Spandau buffer stop 3212811: a dead-end track of 250 m behind 600 m
    /// of predecessors, one of them joined end to end.
    fn buffer_stop_chain() -> MapIndex {
        let mut ix = MapIndex::default();
        ix.splines.insert(10, IndexedSpline { length: 400.0, prev: 0, next: 11 });
        // spline 11 runs against the chain: its end meets 10, its start meets 12
        ix.splines.insert(11, IndexedSpline { length: 200.0, prev: 12, next: 10 });
        ix.splines.insert(12, IndexedSpline { length: 250.0, prev: 11, next: 0 });
        ix
    }

    #[test]
    fn row_start_counts_from_the_chain_start() {
        let ix = buffer_stop_chain();
        assert_eq!(chain_offset(&ix, 10), 0.0);
        assert_eq!(chain_offset(&ix, 12), 600.0);
        // a row on spline 11 runs its way, from the joint with 12: the chain before it is 12
        assert_eq!(chain_offset(&ix, 11), 250.0);
        // d = 845.3 on the last spline: 4.7 m before the end of the track, not 95.3 m along
        let track = spline(12, 11, 0, 250.0);
        let objs = row_objects(&row(10.0, 0.0, 845.3, None), &track, DVec2::ZERO, Some(&ix));
        assert_eq!(objs.len(), 1);
        assert!((objs[0].pose.pos.y - 245.3).abs() < 1e-6, "{:?}", objs[0].pose.pos);
        // a loop has no start: the walk stops where it comes round
        let mut ring = MapIndex::default();
        ring.splines.insert(1, IndexedSpline { length: 10.0, prev: 3, next: 2 });
        ring.splines.insert(2, IndexedSpline { length: 20.0, prev: 1, next: 3 });
        ring.splines.insert(3, IndexedSpline { length: 30.0, prev: 2, next: 1 });
        assert_eq!(chain_offset(&ring, 1), 50.0);
    }

    #[test]
    fn row_runs_on_through_its_tile() {
        let mut ix = buffer_stop_chain();
        // spline 11 runs against the chain: its end meets 10 at y = 400, its start meets 12
        let s10 = spline(10, 0, 11, 400.0);
        let s11 = MapSpline { pos: [0.0, 600.0, 10.0], heading: 180.0, ..spline(11, 12, 10, 200.0) };
        let s12 = MapSpline { pos: [0.0, 600.0, 10.0], ..spline(12, 11, 0, 250.0) };
        // a row on spline 10 with objects at 20, 220, 420 and 620 m along the chain
        let master = SplineAttachment { id: 7, ..row(200.0, 600.0, 20.0, None) };
        let tile = [s10.clone(), s11.clone(), s12.clone()];
        let objs = tile_row_objects(&master, &tile, DVec2::ZERO, Some(&ix));
        let at: Vec<(usize, usize, f64)> = objs.iter().map(|(si, o)| (*si, o.index, o.pose.pos.y)).collect();
        assert_eq!(at.len(), 4, "{at:?}");
        for (k, (si, j, y)) in [(0, 0, 20.0), (0, 1, 220.0), (1, 2, 420.0), (2, 3, 620.0)].into_iter().enumerate() {
            assert!(at[k].0 == si && at[k].1 == j && (at[k].2 - y).abs() < 1e-6, "{at:?}");
        }
        // the row's right is the spline's left on the reversed spline: still east of the chain
        assert!(objs[2].1.pose.pos.x > 2.9, "{:?}", objs[2].1.pose.pos);
        // where the chain leaves the tile, the row stops (a repeater there carries it on)
        let objs = tile_row_objects(&master, &tile[..2], DVec2::ZERO, Some(&ix));
        assert_eq!(objs.iter().map(|(_, o)| o.index).collect::<Vec<_>>(), vec![0, 1, 2]);
        // ... like this one on spline 12, which then goes on through its own tile
        ix.masters.insert((3, 7), (10, 20.0));
        let repeater = SplineAttachment { id: 7, spline_index: 0, repeater: Some((3, 3)), ..row(200.0, 600.0, 20.0, None) };
        let objs = tile_row_objects(&repeater, &[s12], DVec2::ZERO, Some(&ix));
        assert_eq!(objs.len(), 1);
        assert!(objs[0].1.index == 3 && (objs[0].1.pose.pos.y - 620.0).abs() < 1e-6, "{:?}", objs[0].1.pose.pos);
        // a single object past the end of its spline goes onto the next one in the tile
        let single = SplineAttachment { id: 8, ..row(0.0, 0.0, 450.0, None) };
        let objs = tile_row_objects(&single, &tile, DVec2::ZERO, Some(&ix));
        assert_eq!(objs.len(), 1);
        assert!(objs[0].0 == 1 && (objs[0].1.pose.pos.y - 450.0).abs() < 1e-6, "{:?}", objs[0].1.pose.pos);
    }

    #[test]
    fn repeater_continues_the_row() {
        let mut ix = MapIndex::default();
        ix.splines.insert(1, IndexedSpline { length: 100.0, prev: 0, next: 2 });
        ix.splines.insert(2, IndexedSpline { length: 50.0, prev: 1, next: 3 });
        // spline 3 is joined end to end: it runs against the chain
        ix.splines.insert(3, IndexedSpline { length: 80.0, prev: 0, next: 2 });
        assert_eq!(chain_distance(&ix, 1, 3, 1e9), Some((150.0, true)));
        ix.masters.insert((0, 5), (1, 20.0));
        // the master row: 20, 50, 80 m on spline 1
        let s1 = spline(1, 0, 2, 100.0);
        assert_eq!(row_objects(&row(30.0, 1000.0, 20.0, None), &s1, DVec2::ZERO, None).len(), 3);
        // spline 2 (100..150 m of the chain): objects 3 (110 m) and 4 (140 m)
        let s2 = MapSpline { pos: [0.0, 100.0, 10.0], ..spline(2, 1, 3, 50.0) };
        let objs = row_objects(&row(30.0, 1000.0, 20.0, Some((0, 3))), &s2, DVec2::ZERO, Some(&ix));
        assert_eq!(objs.iter().map(|o| o.index).collect::<Vec<_>>(), vec![3, 4]);
        assert!((objs[0].pose.pos.y - 110.0).abs() < 1e-6 && (objs[1].pose.pos.y - 140.0).abs() < 1e-6);
        // on the reversed spline object 5 (170 m) lies 20 m from the joint, 60 m from its start
        let s3 = MapSpline { pos: [0.0, 230.0, 10.0], heading: 180.0, ..spline(3, 0, 2, 80.0) };
        let objs = row_objects(&row(30.0, 1000.0, 20.0, Some((0, 5))), &s3, DVec2::ZERO, Some(&ix));
        assert_eq!(objs[0].index, 5);
        assert!((objs[0].pose.pos.y - 170.0).abs() < 1e-6, "{:?}", objs[0].pose.pos);
        // the row's right is the spline's left there: still east of the chain
        assert!(objs[0].pose.pos.x > 2.9, "{:?}", objs[0].pose.pos);
    }

    #[test]
    fn attachment_order() {
        let a = omsi_scenery::sco::Attachment { ops: vec![("attach_rot_z".into(), vec![180.0]), ("attach_trans".into(), vec![-4.0, 0.4, 5.0])] };
        let m = attachment_matrix(&a);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p - Vec3::new(-4.0, 0.4, 5.0)).length() < 1e-5, "{p:?}");
        // the attached object is turned round, its forward axis points backwards
        assert!((m.transform_vector3(Vec3::Y) - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-5);
        let parent = Pose { pos: DVec3::new(100.0, 200.0, 30.0), rot: omsi_geometry::object_rotation([90.0, 0.0, 0.0]) };
        let child = parent.attached(&m, [0.0; 3]);
        // parent faces east: its left (-x) is north
        assert!((child.pos - DVec3::new(100.4, 204.0, 35.0)).length() < 1e-4, "{:?}", child.pos);
        assert!((child.heading() - 270.0).abs() < 1e-3, "{}", child.heading());
    }
}
