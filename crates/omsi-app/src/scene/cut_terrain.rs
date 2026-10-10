//! Surface rasters: the terrain cut under roads and crossings.
use super::*;

/// OMSI_CHECK_ROADS: the ground points a tile's old cut rule would have taken away with
/// nothing under them, the covered points, and the worst of them.
type Check = (usize, usize, Vec<(f64, f64, f32)>);

impl World {
    /// Surface rasters: cut the terrain under roads and crossings, keep their heights (for
    /// the wheels and the feet), and decode the textures the upload will want. The roads that
    /// reach a tile and the surfaces and cutters of the tiles next to it count as much as its
    /// own, each as it finally stands.
    pub(super) fn cut_terrain(
        &self,
        prepared: &mut [Prepared],
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        layout: &TileLayout,
    ) {
        let debug_raster = omsi_cfg::flags::OMSI_DEBUG_RASTER.var().and_then(|q| {
            q.split_once(',')
                .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
        });
        let check_roads = omsi_cfg::flags::OMSI_CHECK_ROADS.is_set();
        let debug = omsi_cfg::flags::OMSI_DEBUG_SPLINES.is_set();
        let debug_physics = omsi_cfg::flags::OMSI_DEBUG_PHYSICS.is_set();
        let results: Vec<(Arc<TileSurface>, Option<Image>, Check, usize)> = prepared
            .par_iter_mut()
            .map(|p| {
                self.cut_tile(p, staged, layout, debug_raster, check_roads, debug)
            })
            .collect();
        let (mut holes, mut cells) = (0usize, 0usize);
        let mut where_: Vec<(f64, f64, f32)> = Vec::new();
        let (mut tris, mut wheel_meshes) = (0usize, 0usize);
        for (p, (ts, cut, check, wheels)) in prepared.iter_mut().zip(results) {
            p.cut = cut.map(|c| if omsi_cfg::flags::OMSI_CUT_PLAIN.is_set() { omsi_texture::gpu::TextureData { gpu_mips: false, ..omsi_texture::gpu::TextureData::from_image(c) } } else { tile_texture(c, true) });
            tris += ts.drive.tris.len();
            wheel_meshes += wheels;
            // the wheel surfaces come and go with the tile (World::unload_tile)
            self.surfaces.write().insert((p.tx, p.ty), ts);
            holes += check.0;
            cells += check.1;
            where_.extend(check.2);
        }
        if debug_physics {
            log::info!("wheel surfaces: {tris} faces on {} tiles from {wheel_meshes} meshes", prepared.len());
        }
        if check_roads {
            where_.sort_by(|a, b| b.2.total_cmp(&a.2));
            log::info!("ground-cut check: {holes} of {cells} covered ground points would be cut away with nothing under them ({:.2} %)", holes as f32 / cells.max(1) as f32 * 100.0);
            let over = self.over_road.load(std::sync::atomic::Ordering::Relaxed);
            log::info!("ground-over-road check: {over} of {cells} road points lie under the ground (3 cm to 1.5 m)");
            if let Ok(mut w) = self.over_road_at.lock() {
                w.sort_by(|a, b| b.2.total_cmp(&a.2));
                let by = |lo: f32, hi: f32| w.iter().filter(|p| p.2 >= lo && p.2 < hi).count();
                log::info!("   by depth: 3-10 cm {}, 10-30 cm {}, 30-60 cm {}, 60 cm-1.5 m {}", by(0.0, 0.1), by(0.1, 0.3), by(0.3, 0.6), by(0.6, 9.0));
                for (x, y, d, z) in w.iter().filter(|p| p.2 > 0.08 && p.2 < 0.3).step_by(97).take(6) {
                    log::info!("   (shallow) ground {d:.2} m over the road at ({x:.1}, {y:.1}, {z:.1})");
                }
                for (x, y, d, _) in w.iter().take(8) {
                    log::info!("   ground {d:.2} m over the road at ({x:.1}, {y:.1})");
                }
            }
            for (x, y, d) in where_.iter().take(5) {
                log::info!("   {d:.1} m of nothing under the ground at ({x:.0}, {y:.0})");
            }
        }
        self.decode_wanted_textures(prepared);
    }

    /// One tile's surface raster and the ground cut and paint that follow from it.
    fn cut_tile(
        &self,
        p: &mut Prepared,
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        layout: &TileLayout,
        debug_raster: Option<(f64, f64)>,
        check_roads: bool,
        debug: bool,
    ) -> (Arc<TileSurface>, Option<Image>, Check, usize) {
        let key = (p.tx, p.ty);
        let (tx, ty) = key;
        let (x0, y0) = (tx as f64 * tile_size(), ty as f64 * tile_size());
        let (mut ts, hole_rims, wheel_meshes) = self.tile_surface(p, staged, layout, debug_raster);
        // (before the paint is cut: under a road the ground's layer is never asked for)
        ts.sound = self.ground_sound(p, staged.get(&key).map(|q| q.as_ref()), &ts.drive);
        if omsi_cfg::flags::OMSI_DEBUG_SURFACES.is_set() {
            let faces: Vec<String> = ts.drive.surface_examples().iter().map(|(id, n, m)| format!("{id}: {n} faces e.g. ({:.0}, {:.0}, {:.1})", x0 + m.x as f64, y0 + m.y as f64, m.z)).collect();
            log::info!("tile ({tx}, {ty}): wheel faces by surface {}", faces.join(", "));
        }
        let tile_terrain = self.terrains.read().get(&key).cloned();
        p.hole_walls = tile_terrain
            .as_ref()
            .map(|terrain| omsi_geometry::terrain_hole_walls_with_roads(&hole_rims, terrain, &ts.drive))
            .unwrap_or_default();
        // How much of the ground the old cut rule ("anything below the terrain takes
        // it away") would have removed with nothing to put in its place: a hole in
        // the world you can see the sky through.
        let check = ground_cut_check(&ts, check_roads, tile_terrain.as_ref(), x0, y0, (&self.over_road, &self.over_road_at));
        let terrain_at = move |x: f32, y: f32| {
            tile_terrain.as_ref().map(|t| t.sample(x, y)).unwrap_or(0.0)
        };
        // the hole the roads cut into the ground, as an alpha image in tile space
        let cut = if ts.cuts_anything(&terrain_at, surface_flush()) {
            if debug {
                log::info!("tile ({tx}, {ty}): terrain cut under flush surfaces");
            }
            let rgba = ts.mask_image(&terrain_at, surface_flush());
            if let Some(dir) = omsi_cfg::flags::OMSI_DUMP_CUT.var() {
                let a: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
                if let Some(img) = image::GrayImage::from_raw(ts.size as u32, ts.size as u32, a) {
                    let _ = image::imageops::flip_vertical(&img).save(format!("{dir}/cut_{tx}_{ty}.png"));
                }
            }
            Some(Image {
                width: ts.size as u32,
                height: ts.size as u32,
                rgba,
                has_alpha: true,
            })
        } else {
            None
        };
        // the painted ground layers: where the roads cut the ground away the paint
        // goes too, and a layer with nothing left on the tile is not drawn
        cut_paint(p, cut.as_ref());
        (Arc::new(ts), cut, check, wheel_meshes)
    }

    /// The surface raster of a tile: the roads that reach it and the surfaces and cutters
    /// of the tiles next to it rasterized, with the rims of the holes (tile space) and the
    /// number of meshes the wheels stand on.
    fn tile_surface(
        &self,
        p: &Prepared,
        staged: &HashMap<(i32, i32), Arc<StagedTile>>,
        layout: &TileLayout,
        debug_raster: Option<(f64, f64)>,
    ) -> (TileSurface, Vec<Vec<DVec3>>, usize) {
        let key = (p.tx, p.ty);
        let (tx, ty) = key;
        let (x0, y0) = (tx as f64 * tile_size(), ty as f64 * tile_size());
        let (x1, y1) = (x0 + tile_size(), y0 + tile_size());
        let outside = |b: &[f64; 4]| b[2] < x0 || b[0] > x1 || b[3] < y0 || b[1] > y1;
        let src = Self::sources(layout, staged, key);
        let mut order: Vec<&Arc<StagedTile>> = src.values().collect();
        order.sort_by_key(|q| (q.tx, q.ty));
        let mut ts = TileSurface::new(SURFACE_RASTER);
        let mut hole_rims = Vec::new();
        // meshes the wheels stand on, and of them low objects they climb
        let mut wheel_meshes = 0usize;
        let report = |mesh: &MeshData,
                      xf: &Mat4,
                      o: DVec3,
                      b: &[f64; 4],
                      label: &dyn Fn() -> String| {
            let Some((qx, qy)) = debug_raster else { return };
            if qx < b[0] || qx > b[2] || qy < b[1] || qy > b[3] {
                return;
            }
            let inside = mesh.indices.chunks_exact(3).any(|t| {
                let w = |i: u32| {
                    let v = xf.transform_point3(mesh.positions[i as usize]).as_dvec3() + o;
                    (v.x, v.y)
                };
                let (a, bb, c) = (w(t[0]), w(t[1]), w(t[2]));
                let s1 = (bb.0 - a.0) * (qy - a.1) - (bb.1 - a.1) * (qx - a.0);
                let s2 = (c.0 - bb.0) * (qy - bb.1) - (c.1 - bb.1) * (qx - bb.0);
                let s3 = (a.0 - c.0) * (qy - c.1) - (a.1 - c.1) * (qx - c.0);
                (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0)
                    || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
            });
            if inside {
                log::info!("point ({qx},{qy}) covered by {} (bounds {:?})", label(), b);
            }
        };
        // every road that reaches the tile; a railway embankment or a bridge deck is
        // a surface (the ground is cut under it) but not something the wheels stand
        // on: only splines that carry a road or footway path count as drivable
        for q in &order {
            for sp in &q.splines {
                // (a blended layer - Westcountry's lane darkeners over the painted
                // ground of its junctions - cuts no ground away: under it the ground
                // is what shows through, and cut away it was the sky)
                // (nor do wires overhead: see `SPLINE_OVERHEAD`)
                if !sp.cuts_terrain || sp.overlay || outside(&sp.bounds) {
                    continue;
                }
                report(&sp.shape, &Mat4::IDENTITY, q.origin, &sp.bounds, &|| {
                    format!("spline {}", sp.ty.def.path.display())
                });
                ts.rasterize_kind(
                    &sp.shape,
                    &Mat4::IDENTITY,
                    q.origin,
                    tx,
                    ty,
                    sp.drivable,
                );
            }
            // what the wheels roll on: the splines' height profiles
            for (hp, b, surf) in &q.drive {
                if !outside(b) {
                    ts.add_spline_drive(
                        hp,
                        surf.as_ref(),
                        q.origin,
                        tx,
                        ty,
                    );
                    wheel_meshes += 1;
                }
            }
        }
        // the surfaces and [terrainhole] cutters of this tile and the ones around it
        for q in &order {
            if (q.tx - tx).abs() > 1 || (q.ty - ty).abs() > 1 {
                continue;
            }
            let Some(res) =
                self.resolve((q.tx, q.ty), &Self::sources(layout, staged, (q.tx, q.ty)))
            else {
                continue;
            };
            if !omsi_cfg::flags::OMSI_NO_SPLINE_HOLES.is_set() {
                for rim in &q.hole_rims {
                    let ring: Vec<_> = rim.iter().map(|v| v.truncate()).collect();
                    ts.add_outline(&ring, tx, ty);
                    hole_rims.push(rim.iter().map(|v| *v - p.origin).collect());
                }
            }
            for (oi, (o, pose)) in q.objects.iter().zip(res.poses.iter()).enumerate() {
                let Some(pose) = pose else { continue };
                let ot = &o.ot;
                // Editor-only helpers and trees do not cut terrain.
                if ot.sco.tree.is_some()
                    || ot.sco.only_editor
                    || ot.sco.is_help_arrow
                {
                    continue;
                }
                for h in &ot.holes {
                    if !outside(&mesh_bounds(h, &pose.rot, pose.pos)) {
                        ts.rasterize_hole(h, &pose.rot, pose.pos, tx, ty);
                        // and cut exactly along its rim, as along a spline's outline:
                        // by texel alone the ground stood a metre into the road at
                        // the edges of a junction (Spandau, Bahnstr./Hansastr.)
                        for rim in omsi_geometry::hole_mesh_rims(h, &pose.rot, pose.pos) {
                            let ring: Vec<_> = rim.iter().map(|v| v.truncate()).collect();
                            if !omsi_geometry::outline_crosses_itself(&ring) {
                                ts.add_outline(&ring, tx, ty);
                                hole_rims.push(rim.iter().map(|v| *v - p.origin).collect());
                            }
                        }
                    }
                }
                // An explicit cutter is independent of the object's render meshes.
                if ot.meshes.is_empty() {
                    continue;
                }
                // Laid on the ground (the terrain is cut under it): a `[surface]` object
                // and one drawn as a ground layer (`[rendertype]`).
                let surface =
                    ot.sco.render_type.is_ground_layer()
                        || ot.sco.surface;
                if !surface {
                    continue;
                }
                let meshes: Vec<&MeshData> = match res.warped.get(&oi) {
                    Some(w) => w.iter().collect(),
                    None => ot.meshes.iter().map(|(m, _, _)| m).collect(),
                };
                // What the wheels stand on is Omsi.exe's ground query (0x7a0814): the
                // terrain, the splines, and of the objects only the `[surface]` ones
                // (the tile's list of them, 0x79eb63) - and of those only the first
                // `[mesh]` of the model, a ray cast down into it (0x5f9218 with only
                // mesh 0). A collision mesh is never ground (it only shapes the crash
                // body), nor is an object drawn as a ground layer without `[surface]`
                // (the road markings), nor are the other meshes of a surface object
                // (the Spandau depot's buildings stand on its yard, `Betr_S_Boden`,
                // its first mesh). Every one of those lifted the wheels here: the bus
                // hopped over markings, low collision meshes and whatever a surface
                // object carried - bumps nobody could see.
                let ground_mesh = ot.sco.surface.then(|| ot.mesh_def_index.iter().position(|&d| d == 0)).flatten();
                for (k, mesh) in meshes.into_iter().enumerate() {
                    let b = mesh_bounds(mesh, &pose.rot, pose.pos);
                    if outside(&b) {
                        continue;
                    }
                    report(mesh, &pose.rot, pose.pos, &b, &|| {
                        format!(
                            "object {} rendertype={:?} surface={}",
                            ot.sco.path.display(),
                            ot.sco.render_type,
                            ot.sco.surface
                        )
                    });
                    ts.rasterize_kind(mesh, &pose.rot, pose.pos, tx, ty, true);
                    if Some(k) == ground_mesh {
                        // (its textures' `.surf` maps: cobbled junctions shake the bus too)
                        let dirs = ot.texture_dirs(&self.root);
                        let dirs: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
                        let maps: Vec<_> = ot.meshes.get(k).map(|m| m.1.iter().map(|m| surf_map(&m.texture, &dirs)).collect()).unwrap_or_default();
                        let ids: Vec<u8> = ot.meshes.get(k).map(|m| m.1.iter().map(|m| surface_id(&m.texture, &dirs)).collect()).unwrap_or_default();
                        ts.add_drive_mesh_surf(
                            mesh,
                            &pose.rot,
                            pose.pos,
                            tx,
                            ty,
                            omsi_geometry::SurfFaces::tagged(mesh, &maps, &ids).as_ref(),
                        );
                        wheel_meshes += 1;
                    }
                }
            }
        }
        ts.finish();
        (ts, hole_rims, wheel_meshes)
    }

    // The textures of object types, splines and trees that are not on the GPU yet are
    // decoded here instead of on the thread that draws. A big batch (a whole map at
    // once) decodes them as it uploads instead: all at once they would not fit.
    fn decode_wanted_textures(&self, prepared: &mut [Prepared]) {
        if prepared.len() <= 16 {
            for p in prepared.iter_mut() {
                for o in p.objects.iter_mut() {
                    let freetex = o.ot.meshes.iter().any(|(_, _, ov)| ov.iter().any(|m| !m.item && m.freetex.is_some()));
                    if o.lamp.is_some() || (o.ot.dynamic_textures.is_empty() && !freetex) {
                        continue;
                    }
                    if let Some(program) = o.ot.program.clone() {
                        o.script = Some(omsi_sim::scenery::SceneryInstance::new(program, &o.ot.mesh_defs(), self.script_clock(), &o.strings));
                    }
                }
            }
            let mut wanted: Vec<(String, Vec<PathBuf>)> = prepared
                .iter()
                .flat_map(|p| self.wanted_textures(p))
                .collect();
            wanted.sort();
            wanted.dedup();
            let decoded: Vec<(PathBuf, Arc<TextureData>)> = wanted
                .par_iter()
                .filter_map(|(name, dirs)| {
                    let dirs_ref: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
                    let path = omsi_texture::find_texture(name, &dirs_ref)?;
                    let (img, _) = omsi_texture::gpu::load_gpu(&path).ok()?;
                    Some((path, Arc::new(img)))
                })
                .collect();
            let decoded: Arc<HashMap<PathBuf, Arc<TextureData>>> =
                Arc::new(decoded.into_iter().collect());
            for p in prepared.iter_mut() {
                p.images = decoded.clone();
            }
        }
    }
}

/// OMSI_CHECK_ROADS on one tile (see [`Check`]); the ground over its roads goes to
/// `over_road`.
#[allow(clippy::type_complexity)]
fn ground_cut_check(
    ts: &TileSurface,
    check_roads: bool,
    tile_terrain: Option<&Arc<Terrain>>,
    x0: f64,
    y0: f64,
    (over_road, over_road_at): (&std::sync::atomic::AtomicUsize, &std::sync::Mutex<Vec<(f64, f64, f32, f32)>>),
) -> Check {
    let mut check: Check = (0, 0, Vec::new());
    if let (true, Some(t)) = (check_roads, tile_terrain.as_ref()) {
        let n = ts.size;
        let cell = tile_size() as f32 / n as f32;
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                if !ts.covered(k) {
                    continue;
                }
                check.1 += 1;
                let th = t.sample((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                // the ground over a road: shows through it (Omsi.exe cuts nothing)
                if ts.road_covered(k) && th > ts.road_height(k) + 0.03 && th < ts.road_height(k) + 1.5 && !ts.cut_at((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell, th, surface_flush()) {
                    over_road.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if let Ok(mut w) = over_road_at.lock() {
                        w.push((x0 + ((i as f32 + 0.5) * cell) as f64, y0 + ((j as f32 + 0.5) * cell) as f64, th - ts.road_height(k), th));
                    }
                }
                let old_rule = th >= ts.low_height(k) - surface_flush();
                let new_rule = ts.low_height(k) - surface_flush() <= th
                    && th <= ts.height(k) + surface_flush();
                if old_rule && !new_rule {
                    check.0 += 1;
                    if check.2.len() < 100 {
                        check.2.push((
                            x0 + (i as f32 * cell) as f64,
                            y0 + (j as f32 * cell) as f64,
                            th - ts.height(k),
                        ));
                    }
                }
            }
        }
    }
    check
}

impl World {
    /// What the ambience hears of a tile: the surface of its ground layers where they show,
    /// its trees and how many objects stand on it.
    fn ground_sound(&self, p: &Prepared, q: Option<&StagedTile>, drive: &omsi_geometry::DriveGrid) -> omsi_geometry::GroundSound {
        let root = [self.root.as_path()];
        let ids: Vec<u8> = self.global.ground_textures.iter().map(|g| surface_id(&g.texture, &root)).collect();
        // (a map without ground textures is a meadow)
        let base = ids.first().copied().unwrap_or(4);
        let layers: Vec<(u8, usize, usize, &[u8])> = p
            .paint_masks
            .iter()
            .filter_map(|(layer, img)| Some((*ids.get(*layer)?, img.width as usize, img.height as usize, img.rgba.as_slice())))
            .collect();
        let mut g = omsi_geometry::GroundSound::from_layers(GROUND_SOUND_CELLS, base, &layers);
        let (x0, y0) = (p.tx as f64 * tile_size(), p.ty as f64 * tile_size());
        g.trees = p.trees.iter().map(|t| [(t.2.x - x0) as f32, (t.2.y - y0) as f32, t.3 as f32, if crate::soundscape::catalog::is_conifer(&t.0.sco, &t.1) { 1.0 } else { 0.0 }]).collect();
        g.objects = p.objects.len() as u32;
        self.sound_places(p, q, drive, &mut g);
        if omsi_cfg::flags::OMSI_DEBUG_SURFACES.is_set() {
            // where each surface of the ground shows on this tile (a place to try it)
            let n = g.size;
            let cell = tile_size() / n as f64;
            let mut seen: Vec<String> = Vec::new();
            for id in 0..=8u8 {
                let cells: Vec<usize> = (0..n * n).filter(|k| g.ids[*k] == id).collect();
                if let Some(k) = cells.get(cells.len() / 2) {
                    seen.push(format!("{id}: {:.0} % e.g. ({:.0}, {:.0})", cells.len() as f32 * 100.0 / (n * n) as f32, x0 + (k % n) as f64 * cell + cell / 2.0, y0 + (k / n) as f64 * cell + cell / 2.0));
                }
            }
            log::info!("tile ({}, {}): ground surfaces {}; {} trees, {} objects; for the ambience {} buildings, {} known objects, {} lines, {} water cells (water {:?})", p.tx, p.ty, seen.join(", "), g.trees.len(), g.objects, g.buildings.len(), g.spots.len(), g.lines.len(), g.water.len(), p.water);
        }
        g
    }
}

/// Cells per tile edge of the ground's surface classes (some 5 m on a Berlin tile: the
/// painted car parks and paths are wider than that).
const GROUND_SOUND_CELLS: usize = 64;

/// The painted ground layers of a tile without what the roads cut away (`p.paint`), and
/// the same masks uncut for the walls of the holes (`p.wall_paint`).
fn cut_paint(p: &mut Prepared, cut: Option<&Image>) {
    let masks = std::mem::take(&mut p.paint_masks);
    p.wall_paint.clear();
    p.paint = masks
        .into_iter()
        .filter_map(|(layer, img)| {
            let (mut rgba, w, h) = smooth_paint_mask(
                &img.rgba,
                img.width as usize,
                img.height as usize,
            );
            if !p.hole_walls.indices.is_empty()
                && rgba.chunks_exact(4).any(|v| v[3] > 8)
            {
                p.wall_paint.push((
                    layer,
                    tile_texture(
                        Image {
                            width: w as u32,
                            height: h as u32,
                            rgba: rgba.clone(),
                            has_alpha: true,
                        },
                        true,
                    ),
                ));
            }
            let img = Image {
                width: w as u32,
                height: h as u32,
                rgba: Vec::new(),
                has_alpha: true,
            };
            let mut painted = 0usize;
            for j in 0..h {
                for i in 0..w {
                    let a = &mut rgba[(j * w + i) * 4 + 3];
                    if let Some(c) = &cut {
                        // The cut as the terrain's alpha test sees it: sampled
                        // bilinearly at this texel's centre, cut below one half.
                        // Taken from the nearest cut texel (1.5-3 m each), the
                        // paint kept teeth over the hole the road left in the
                        // ground - drawn on top of the carriageway, a staircase
                        // of asphalt or cobbles reaching into the road.
                        if bilinear_alpha(c, (i as f32 + 0.5) / w as f32, (j as f32 + 0.5) / h as f32) < 0.5 {
                            *a = 0;
                        }
                    }
                    if *a > 8 {
                        painted += 1;
                    }
                }
            }
            (painted > 0).then(|| {
                (
                    layer,
                    tile_texture(
                        Image {
                            width: img.width,
                            height: img.height,
                            rgba,
                            has_alpha: true,
                        },
                        true,
                    ),
                    painted as f32 / (w * h).max(1) as f32,
                )
            })
        })
        .collect();
}

impl World {
    /// The rest of what the ambience hears of a tile: its buildings (a model as tall and as
    /// wide as a house), the objects it knows by what their authors filed them as, the lanes,
    /// tracks, wires and tunnels along its splines, and where its water stands over the
    /// ground.
    fn sound_places(&self, p: &Prepared, q: Option<&StagedTile>, drive: &omsi_geometry::DriveGrid, g: &mut omsi_geometry::GroundSound) {
        use crate::soundscape::catalog;
        let (x0, y0) = (p.tx as f64 * tile_size(), p.ty as f64 * tile_size());
        let local = |w: DVec3| [(w.x - x0) as f32, (w.y - y0) as f32, w.z as f32];
        // (the model's box per type: the same house stands many times)
        let mut boxes: HashMap<*const ObjectType, Option<(glam::Vec3, glam::Vec3)>> = HashMap::new();
        for o in &p.objects {
            let kind = catalog::classify(&o.ot.sco);
            if let Some(k) = kind {
                g.spots.push(omsi_geometry::SoundSpot { kind: k as u16, pos: local(o.pos), id: o.map_id });
            }
            let Some((lo, hi)) = *boxes.entry(Arc::as_ptr(&o.ot)).or_insert_with(|| model_box(&o.ot)) else { continue };
            // (placed: turned and scaled as the map has it)
            let corners = [lo, hi, glam::Vec3::new(lo.x, hi.y, lo.z), glam::Vec3::new(hi.x, lo.y, hi.z)].map(|c| o.xf.transform_vector3(c));
            let (mut a, mut b) = (corners[0], corners[0]);
            for c in corners {
                a = a.min(c);
                b = b.max(c);
            }
            let size = b - a;
            if size.z >= BUILDING_HEIGHT && size.x.min(size.y) >= BUILDING_WIDTH {
                let l = local(o.pos);
                g.buildings.push([l[0], l[1], size.z]);
            }
        }
        if let Some(q) = q {
            for (kind, speed, points) in &q.sound_lines {
                g.lines.push(omsi_geometry::SoundLine { kind: *kind, speed: *speed, points: points.iter().map(|w| local(*w)).collect() });
            }
        }
        // the water: cells whose surface stands over the ground
        // (only where the ground is known: without it nothing says the water lies over it)
        if let (Some(w), Some(terrain)) = (p.water, self.terrains.read().get(&(p.tx, p.ty)).cloned()) {
            let level = (w[0] + w[1] + w[2] + w[3]) / 4.0;
            let n = WATER_CELLS;
            let cell = tile_size() as f32 / n as f32;
            for j in 0..n {
                for i in 0..n {
                    let (x, y) = ((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
                    // (and nothing laid over it: under a road or a quay the ground often
                    // lies below the water's level, and no water is seen there)
                    let covered = drive.surface_at(x, y, level + 30.0).is_some_and(|(z, _)| z > level - 1.0);
                    if terrain.sample(x, y) < level - 0.2 && !covered {
                        g.water.push([x, y]);
                    }
                }
            }
        }
    }
}

/// The box of an object type's model (its first level of detail), in its own frame.
fn model_box(ot: &ObjectType) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut points = ot.meshes.iter().flat_map(|(m, _, _)| m.positions.iter().copied());
    let first = glam::Vec3::from(points.next()?);
    let (mut lo, mut hi) = (first, first);
    for p in points {
        let p = glam::Vec3::from(p);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    Some((lo, hi))
}

/// A building to the ambience: at least this tall (m) …
const BUILDING_HEIGHT: f32 = 4.0;
/// … and this wide both ways (m) - a wall, a fence, a mast is not.
const BUILDING_WIDTH: f32 = 4.0;
/// Cells per tile edge where the ambience looks for water (some 19 m on a Berlin tile).
const WATER_CELLS: usize = 16;
