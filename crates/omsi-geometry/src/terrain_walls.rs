//! Connections from a terrain cut to the authored three-dimensional cutter rim.
use super::{outline_crosses_itself, tile_size, DVec2, DVec3, DriveGrid, MeshData, Terrain};

const EPS: f64 = 1e-8;

struct Ring {
    points: Vec<DVec2>,
    bounds: [f64; 4],
}

impl Ring {
    fn contains(&self, p: DVec2) -> bool {
        if p.x < self.bounds[0]
            || p.y < self.bounds[1]
            || p.x > self.bounds[2]
            || p.y > self.bounds[3]
        {
            return false;
        }
        let mut inside = false;
        for i in 0..self.points.len() {
            let (a, b) = (self.points[i], self.points[(i + 1) % self.points.len()]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
                inside = !inside;
            }
        }
        inside
    }
}

#[derive(Clone, Copy)]
struct Edge {
    a: DVec3,
    b: DVec3,
}

impl Edge {
    fn at(self, t: f64) -> DVec3 {
        self.a.lerp(self.b, t)
    }

    /// Parameter at a point on this projected edge, including its endpoints.
    fn parameter(self, p: DVec2) -> Option<f64> {
        let a = self.a.truncate();
        let d = (self.b - self.a).truncate();
        if d.perp_dot(p - a).abs() > EPS * d.length() {
            return None;
        }
        let t = (p - a).dot(d) / d.length_squared();
        (-EPS..=1.0 + EPS).contains(&t).then_some(t.clamp(0.0, 1.0))
    }

    /// Split at crossing rims, touching endpoints, and overlapping collinear rims.
    fn intersections(self, other: Edge, split: &mut Vec<f64>) {
        let (a, b) = (self.a.truncate(), other.a.truncate());
        let (d, e) = ((self.b - self.a).truncate(), (other.b - other.a).truncate());
        let cross = d.perp_dot(e);
        if cross.abs() > EPS * d.length() * e.length() {
            let t = (b - a).perp_dot(e) / cross;
            let u = (b - a).perp_dot(d) / cross;
            if (-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&u) {
                split.push(t.clamp(0.0, 1.0));
            }
        } else if d.perp_dot(b - a).abs() <= EPS * d.length() {
            let len = d.length_squared();
            let t0 = (b - a).dot(d) / len;
            let t1 = (other.b.truncate() - a).dot(d) / len;
            let lo = t0.min(t1).max(0.0);
            let hi = t0.max(t1).min(1.0);
            if hi < lo {
                return;
            }
            split.extend([lo, hi]);
            // Coincident rims can have different depths. Keep the lower authored rim;
            // split where that ownership changes rather than stacking coplanar walls.
            let difference = |t| {
                let p = self.at(t);
                p.z - other.at(other.parameter(p.truncate()).unwrap_or(0.0)).z
            };
            let (z0, z1) = (difference(lo), difference(hi));
            if z0 * z1 < 0.0 {
                split.push(lo + (hi - lo) * z0 / (z0 - z1));
            }
        }
    }
}

fn clip(edge: Edge, side: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for (a, d) in [
        (edge.a.x, edge.b.x - edge.a.x),
        (edge.a.y, edge.b.y - edge.a.y),
    ] {
        if d.abs() < EPS {
            if a < 0.0 || a > side {
                return None;
            }
        } else {
            let (t0, t1) = (-a / d, (side - a) / d);
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    (hi - lo > EPS).then_some((lo, hi))
}

/// Exact height on the drawn triangles, including the last row/column. Terrain::sample
/// keeps queries just inside the last cell; walls on a tile seam must share its vertices.
fn height(terrain: &Terrain, p: DVec2, side: f64) -> f64 {
    let n = terrain.cells;
    let f = (p / side * n as f64).clamp(DVec2::ZERO, DVec2::splat(n as f64));
    let (i, j) = (
        (f.x.floor() as usize).min(n - 1),
        (f.y.floor() as usize).min(n - 1),
    );
    let (u, v) = (f.x - i as f64, f.y - j as f64);
    let h00 = terrain.height_at(i, j) as f64;
    let h10 = terrain.height_at(i + 1, j) as f64;
    let h01 = terrain.height_at(i, j + 1) as f64;
    let h11 = terrain.height_at(i + 1, j + 1) as f64;
    if u >= v {
        h00 + (h10 - h00) * u + (h11 - h10) * v
    } else {
        h00 + (h11 - h01) * u + (h01 - h00) * v
    }
}

fn triangle(mesh: &mut MeshData, mut p: [DVec3; 3], inward: DVec2, side: f64) {
    let cross = (p[1] - p[0]).cross(p[2] - p[0]);
    if cross.length_squared() < EPS * EPS {
        return;
    }
    if cross.truncate().dot(inward) < 0.0 {
        p.swap(1, 2);
    }
    // Tiny slivers can collapse when the renderer's tile-local vertices become f32.
    let drawn = p.map(|v| v.as_vec3());
    if (drawn[1] - drawn[0])
        .cross(drawn[2] - drawn[0])
        .length_squared()
        == 0.0
    {
        return;
    }
    let normal = inward.extend(0.0).as_vec3();
    let first = mesh.positions.len() as u32;
    for v in p {
        mesh.positions.push(v.as_vec3());
        mesh.normals.push(normal);
        mesh.uvs.push((v.truncate() / side).as_vec2());
    }
    mesh.indices.extend([first, first + 1, first + 2]);
}

/// Join the terrain to explicit cutter rims. All coordinates are tile-local, with rim
/// heights in the terrain's vertical frame. Clips to this tile's [0, tile_size()] square
/// without creating a cap along tile borders. UVs remain tile space for ground materials.
///
/// The terrain grid and its diagonals split the top edge, so it meets the actual drawn
/// triangles even on uneven ground. Only boundaries of the union of the holes get walls;
/// overlapping or adjacent cutters cannot put a barrier through a road. Invalid outlines
/// are ignored, matching the cut-mask policy. Rims come from the authored geometry;
/// no height is inferred from a raster or a road.
/// Keep a terrain cut open where a road continues across its rim. A tunnel's cutter
/// can end at road height even though the terrain above it reaches the tunnel roof;
/// connecting those two heights would put a grass curtain across the carriageway.
pub fn terrain_hole_walls_with_roads(rims: &[Vec<DVec3>], terrain: &Terrain, roads: &DriveGrid) -> MeshData {
    build_walls(rims, terrain, Some(roads))
}

pub fn terrain_hole_walls(rims: &[Vec<DVec3>], terrain: &Terrain) -> MeshData {
    build_walls(rims, terrain, None)
}

/// Split at road triangle edges as well as terrain edges: a narrow entrance need
/// not contain the midpoint of a five-metre terrain cell. Reuse the wheel grid's
/// spatial index instead of comparing every cutter with every road on the tile.
fn road_splits(edge: Edge, lo: f64, hi: f64, roads: &DriveGrid, split: &mut Vec<f64>) {
    if roads.cells == 0 {
        return;
    }
    let (a, b) = (edge.at(lo), edge.at(hi));
    let bin = |v: f64| ((v / roads.cell as f64).floor().max(0.0) as usize).min(roads.cells - 1);
    let mut nearby = Vec::new();
    for y in bin(a.y.min(b.y))..=bin(a.y.max(b.y)) {
        for x in bin(a.x.min(b.x))..=bin(a.x.max(b.x)) {
            let k = y * roads.cells + x;
            nearby.extend_from_slice(&roads.items[roads.start[k] as usize..roads.start[k + 1] as usize]);
        }
    }
    nearby.sort_unstable();
    nearby.dedup();
    for i in nearby {
        if roads.ridge[i as usize] {
            continue;
        }
        let tri = roads.tris[i as usize];
        for j in 0..3 {
            edge.intersections(Edge { a: tri[j].as_dvec3(), b: tri[(j + 1) % 3].as_dvec3() }, split);
        }
    }
}

fn road_crosses_rim(roads: &DriveGrid, at: DVec3, normal: DVec2) -> bool {
    // Cutters and road surfaces commonly differ by a few centimetres. A road
    // higher up (a bridge over an excavation) must not erase the excavation wall.
    const RIM_REACH: f32 = 0.25;
    let road_at = |p: DVec2| {
        roads.probe(p.x as f32, p.y as f32, at.z as f32 + RIM_REACH).below
            .is_some_and(|z| (z - at.z as f32).abs() <= RIM_REACH)
    };
    // Sample beyond the wheel query's seam tolerance. A road ending at the rim,
    // rather than continuing across it, still needs its exposed terrain wall.
    let step = normal * 0.05;
    road_at(at.truncate() - step) && road_at(at.truncate() + step)
}

fn build_walls(rims: &[Vec<DVec3>], terrain: &Terrain, roads: Option<&DriveGrid>) -> MeshData {
    let mut mesh = MeshData::default();
    let side = tile_size();
    let samples = terrain.cells.checked_add(1).and_then(|n| n.checked_mul(n));
    if terrain.cells == 0
        || samples != Some(terrain.heights.len())
        || !side.is_finite()
        || side <= 0.0
        || terrain.heights.iter().any(|h| !h.is_finite())
    {
        return mesh;
    }
    let mut rings = Vec::new();
    let mut edges = Vec::new();
    for rim in rims {
        if rim.len() < 3 || rim.iter().any(|p| !p.is_finite()) {
            continue;
        }
        let points: Vec<_> = rim.iter().map(|p| p.truncate()).collect();
        if outline_crosses_itself(&points) {
            continue;
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in &points {
            bounds[0] = bounds[0].min(p.x);
            bounds[1] = bounds[1].min(p.y);
            bounds[2] = bounds[2].max(p.x);
            bounds[3] = bounds[3].max(p.y);
        }
        if bounds[2] < 0.0 || bounds[3] < 0.0 || bounds[0] > side || bounds[1] > side {
            continue;
        }
        rings.push(Ring { points, bounds });
        for i in 0..rim.len() {
            let edge = Edge {
                a: rim[i],
                b: rim[(i + 1) % rim.len()],
            };
            if (edge.b - edge.a).truncate().length_squared() > EPS * EPS
                && clip(edge, side).is_some()
            {
                edges.push(edge);
            }
        }
    }
    // A tile-local broad phase avoids comparing every edge of every loaded cutter with
    // every other edge. Full rings are retained for inside tests, even across tile borders.
    let bins = terrain.cells.clamp(1, 128);
    let mut spatial = vec![Vec::<usize>::new(); bins * bins];
    let bounds = |edge: Edge| {
        let (lo, hi) = clip(edge, side).unwrap();
        let (a, b) = (edge.at(lo).truncate(), edge.at(hi).truncate());
        let to_bin = |v: f64| ((v / side * bins as f64).floor().max(0.0) as usize).min(bins - 1);
        [
            to_bin(a.x.min(b.x)),
            to_bin(a.y.min(b.y)),
            to_bin(a.x.max(b.x)),
            to_bin(a.y.max(b.y)),
        ]
    };
    for (i, edge) in edges.iter().enumerate() {
        let [x0, y0, x1, y1] = bounds(*edge);
        for y in y0..=y1 {
            for x in x0..=x1 {
                spatial[y * bins + x].push(i);
            }
        }
    }
    let in_hole = |p| rings.iter().any(|r| r.contains(p));
    let cell = side / terrain.cells as f64;
    for (index, &edge) in edges.iter().enumerate() {
        let (lo, hi) = clip(edge, side).unwrap();
        let mut split = vec![lo, hi];
        let [x0, y0, x1, y1] = bounds(edge);
        let mut nearby = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                nearby.extend_from_slice(&spatial[y * bins + x]);
            }
        }
        nearby.sort_unstable();
        nearby.dedup();
        for &j in &nearby {
            if j != index {
                edge.intersections(edges[j], &mut split);
            }
        }
        if let Some(roads) = roads {
            road_splits(edge, lo, hi, roads, &mut split);
        }
        // x/y cell boundaries and x-y=k*cell diagonals are precisely the terrain edges.
        for (a, b) in [
            (edge.a.x, edge.b.x),
            (edge.a.y, edge.b.y),
            (edge.a.x - edge.a.y, edge.b.x - edge.b.y),
        ] {
            if (b - a).abs() < EPS {
                continue;
            }
            let (start, end) = (a + (b - a) * lo, a + (b - a) * hi);
            for k in (start.min(end) / cell).ceil() as i64..=(start.max(end) / cell).floor() as i64
            {
                split.push((k as f64 * cell - a) / (b - a));
            }
        }
        split.retain(|t| *t >= lo && *t <= hi);
        split.sort_by(f64::total_cmp);
        split.dedup_by(|a, b| (*a - *b).abs() < EPS);
        for ts in split.windows(2) {
            let (a, b) = (edge.at(ts[0]), edge.at(ts[1]));
            let d = (b - a).truncate();
            let length = d.length();
            if length < EPS {
                continue;
            }
            let middle = (a + b) * 0.5;
            let left = DVec2::new(-d.y, d.x) / length;
            let step = left * 1e-5f64.min(length * 0.001);
            let (inside_left, inside_right) = (
                in_hole(middle.truncate() + step),
                in_hole(middle.truncate() - step),
            );
            if inside_left == inside_right {
                continue;
            }
            // Coincident outer edges are one wall, independent of winding or duplicates.
            let hidden = nearby.iter().copied().filter(|j| *j != index).any(|j| {
                edges[j].parameter(middle.truncate()).is_some_and(|t| {
                    let z = edges[j].at(t).z;
                    z < middle.z - EPS || ((z - middle.z).abs() <= EPS && j < index)
                })
            });
            if hidden {
                continue;
            }
            let inward = if inside_left { left } else { -left };
            if roads.is_some_and(|roads| road_crosses_rim(roads, middle, inward)) {
                continue;
            }
            let top_a = a.truncate().extend(height(terrain, a.truncate(), side));
            let top_b = b.truncate().extend(height(terrain, b.truncate(), side));
            let (da, db) = (top_a.z - a.z, top_b.z - b.z);
            if da * db < 0.0 {
                // The rim crosses the ground. Two triangular ends, never a twisted quad.
                let at = da / (da - db);
                let join = a.lerp(b, at);
                triangle(&mut mesh, [top_a, a, join], inward, side);
                triangle(&mut mesh, [join, b, top_b], inward, side);
            } else {
                triangle(&mut mesh, [top_a, a, top_b], inward, side);
                triangle(&mut mesh, [top_b, a, b], inward, side);
            }
        }
    }
    if !mesh.indices.is_empty() {
        mesh.ranges.push((0, mesh.indices.len() as u32, 0));
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hole_mesh_outlines, hole_mesh_rims, spline_hole_outlines, spline_hole_rims, SplineCurve,
    };
    use glam::{Mat4, Vec3};
    use omsi_map::MapSpline;
    use omsi_scenery::Spline;

    fn terrain(cells: usize, sample: impl Fn(f64, f64) -> f32) -> Terrain {
        let mut heights = Vec::new();
        for j in 0..=cells {
            for i in 0..=cells {
                heights.push(sample(
                    i as f64 * tile_size() / cells as f64,
                    j as f64 * tile_size() / cells as f64,
                ));
            }
        }
        Terrain { cells, heights }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> Vec<DVec3> {
        vec![
            DVec3::new(x0, y0, z),
            DVec3::new(x1, y0, z),
            DVec3::new(x1, y1, z),
            DVec3::new(x0, y1, z),
        ]
    }

    fn area(mesh: &MeshData) -> f64 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let p = [0, 1, 2].map(|i| mesh.positions[t[i] as usize].as_dvec3());
                (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
            })
            .sum()
    }

    fn road(x0: f32, y0: f32, x1: f32, y1: f32, z: f32) -> DriveGrid {
        let [a, b, c, d] = [
            Vec3::new(x0, y0, z), Vec3::new(x1, y0, z),
            Vec3::new(x1, y1, z), Vec3::new(x0, y1, z),
        ];
        let mut grid = DriveGrid::default();
        grid.push([a, b, c]);
        grid.push([a, c, d]);
        grid.build(tile_size() as f32);
        grid
    }

    #[test]
    fn road_crossing_hole_boundary_keeps_portals_open_at_its_exact_width() {
        let t = terrain(1, |_, _| 10.0);
        let rims = vec![rect(20.0, 20.0, 40.0, 40.0, 0.1)];
        // Even a narrow road within one coarse terrain cell must get an opening.
        for width in [1.0f32, 10.0] {
            let roads = road(30.0 - width / 2.0, 10.0, 30.0 + width / 2.0, 50.0, 0.1);
            let mesh = terrain_hole_walls_with_roads(&rims, &t, &roads);
            valid(&mesh);
            let expected = (80.0 - 2.0 * width as f64) * 9.9;
            assert!((area(&mesh) - expected).abs() < 0.01,
                "a generated wall must not cap either end of the road: {} vs {expected}", area(&mesh));
        }
    }

    #[test]
    fn road_ending_at_a_cut_and_a_bridge_above_it_keep_excavation_walls() {
        let t = terrain(1, |_, _| 10.0);
        let rims = vec![rect(20.0, 20.0, 40.0, 40.0, 0.1)];
        let expected = area(&terrain_hole_walls(&rims, &t));
        for roads in [road(25.0, 10.0, 35.0, 20.0, 0.1), road(25.0, 10.0, 35.0, 50.0, 5.0)] {
            let mesh = terrain_hole_walls_with_roads(&rims, &t, &roads);
            valid(&mesh);
            assert!((area(&mesh) - expected).abs() < 0.01,
                "only a continuous road at the cutter's rim opens a wall");
        }
    }

    fn valid(mesh: &MeshData) {
        for t in mesh.indices.chunks_exact(3) {
            let p = [0, 1, 2].map(|i| mesh.positions[t[i] as usize]);
            let cross = (p[1] - p[0]).cross(p[2] - p[0]);
            assert!(cross.length_squared() > 1e-12);
            for &i in t {
                let n = mesh.normals[i as usize];
                assert!(n.is_finite() && (n.length() - 1.0).abs() < 1e-5 && n.z.abs() < 1e-5);
                assert!(cross.dot(n) > 0.0, "winding must face into the hole");
                assert!(mesh.uvs[i as usize].is_finite());
            }
        }
    }

    #[test]
    fn spline_rims_keep_authored_height_cant_mirror_and_end_modes() {
        let def = Spline {
            half_cant_width: Some(2.0),
            terrain_hole_profiles: vec![vec![[-2.0, 0.25, -0.5], [2.0, 0.75, -0.25]]],
            ..Default::default()
        };
        let curve = SplineCurve::from_map(
            &MapSpline {
                pos: [40.0, 40.0, 10.0],
                length: 20.0,
                grad_start: 10.0,
                grad_end: 10.0,
                cant_start: 10.0,
                cant_end: 10.0,
                ..Default::default()
            },
            DVec2::ZERO,
        );
        for mirror in [false, true] {
            for mode in 1..=4 {
                let rims = spline_hole_rims(&def, &curve, mirror, mode);
                let projected: Vec<Vec<_>> = rims
                    .iter()
                    .map(|r| r.iter().map(|p| p.truncate()).collect())
                    .collect();
                assert_eq!(spline_hole_outlines(&def, &curve, mirror, mode), projected);
                for p in &rims[0] {
                    let x = p.x - 40.0;
                    let authored_x = if mirror { -x } else { x };
                    let z = if authored_x < 0.0 { 0.25 } else { 0.75 };
                    assert!((p.z - (10.0 + (p.y - 40.0) * 0.1 + z - x * 0.1)).abs() < 1e-6);
                }
                let min_y = rims[0].iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
                let max_y = rims[0]
                    .iter()
                    .map(|p| p.y)
                    .fold(f64::NEG_INFINITY, f64::max);
                assert_eq!(min_y, if mode > 2 { 40.0 } else { 39.5 });
                assert_eq!(max_y, if mode & 1 == 0 { 60.0 } else { 60.5 });
            }
        }
    }

    #[test]
    fn automatic_holes_reject_inverted_wire_profiles_but_keep_authored_narrow_cuts() {
        use omsi_scenery::sli::{SplineProfile, SplineProfilePoint};
        let profile = |x0, x1, z| SplineProfile {
            points: [x0, x1]
                .into_iter()
                .map(|x| SplineProfilePoint {
                    x,
                    z,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut def = Spline {
            profiles: vec![
                profile(-2.0, 2.0, 0.0),
                profile(0.0, 0.02, 5.0), // The two 3 cm insets invert a thin wire.
                profile(0.0, 0.06, 6.0), // An exactly collapsed trough also has no width.
            ],
            ..Default::default()
        };
        let curve = SplineCurve::from_map(
            &MapSpline {
                pos: [40.0, 40.0, 10.0],
                length: 20.0,
                ..Default::default()
            },
            DVec2::ZERO,
        );
        assert_eq!(crate::terrain_hole_profiles(&def).len(), 1);
        for mirror in [false, true] {
            let rims = spline_hole_rims(&def, &curve, mirror, 1);
            assert_eq!(rims.len(), 1);
            let walls = terrain_hole_walls(&rims, &terrain(1, |_, _| 12.0));
            assert!(!walls.indices.is_empty());
            assert!(walls.positions.iter().all(|p| p.z <= 12.0));
        }
        // Width validity is independent of elevation: an authored elevated surface
        // with enough room for both insets still connects to the terrain.
        def.profiles.push(profile(3.0, 5.0, 20.0));
        assert_eq!(crate::terrain_hole_profiles(&def).len(), 2);
        let rims = spline_hole_rims(&def, &curve, false, 1);
        assert!(rims.iter().flatten().any(|p| p.z > 29.0));

        let authored = vec![vec![[0.0, 5.0, 0.0], [0.02, 5.0, 0.0]]];
        def.terrain_hole_profiles = authored.clone();
        assert_eq!(crate::terrain_hole_profiles(&def), authored);
        let rims = spline_hole_rims(&def, &curve, false, 1);
        assert_eq!(rims.len(), 1);
        let walls = terrain_hole_walls(&rims, &terrain(1, |_, _| 12.0));
        assert!(!walls.indices.is_empty());
        assert!(walls.positions.iter().any(|p| p.z == 15.0));
    }

    #[test]
    fn object_rims_weld_seams_keep_transformed_heights_and_reject_closed_meshes() {
        let a = Vec3::new(0.0, 0.0, -2.0);
        let b = Vec3::new(4.0, 0.0, -1.0);
        let c = Vec3::new(4.0, 3.0, 0.0);
        let d = Vec3::new(0.0, 3.0, 1.0);
        let mesh = MeshData {
            positions: vec![a, b, c, a, c, d],
            indices: vec![0, 1, 2, 3, 4, 5],
            ..Default::default()
        };
        let transform = Mat4::from_rotation_x(0.3);
        let origin = DVec3::new(1e6, 2e6, 50.0);
        let rims = hole_mesh_rims(&mesh, &transform, origin);
        assert_eq!(rims.len(), 1);
        assert_eq!(rims[0].len(), 4);
        for p in [a, b, c, d] {
            assert!(rims[0].contains(&(origin + transform.transform_point3(p).as_dvec3())));
        }
        let outline = hole_mesh_outlines(&mesh, &transform, origin);
        assert_eq!(outline[0].len(), 4);
        assert!(rims[0].iter().all(|p| outline[0].contains(&p.truncate())));
        let closed = MeshData {
            positions: vec![a, b, c, d],
            indices: vec![0, 1, 2, 0, 3, 1, 1, 3, 2, 0, 2, 3],
            ..Default::default()
        };
        assert!(hole_mesh_rims(&closed, &transform, origin).is_empty());
        let branched = MeshData {
            positions: vec![a, b, c, d, Vec3::new(-2.0, 1.0, 0.0)],
            indices: vec![0, 1, 2, 0, 3, 4],
            ..Default::default()
        };
        let rims = hole_mesh_rims(&branched, &transform, origin);
        assert_eq!(rims.len(), 2);
        assert!(rims.iter().all(|r| r.len() == 3));
    }

    #[test]
    fn walls_close_the_hole_with_inward_normals_independent_of_winding() {
        let s = tile_size();
        let t = terrain(2, |_, _| 10.0);
        let mut ring = rect(s * 0.1, s * 0.2, s * 0.4, s * 0.6, 2.0);
        for _ in 0..2 {
            let mesh = terrain_hole_walls(&[ring.clone()], &t);
            valid(&mesh);
            assert!((area(&mesh) - 2.0 * (s * 0.3 + s * 0.4) * 8.0).abs() < 0.01);
            let center = DVec2::new(s * 0.25, s * 0.4);
            for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
                assert!((center - p.truncate().as_dvec2()).dot(n.truncate().as_dvec2()) > 0.0);
            }
            ring.reverse();
        }
    }

    #[test]
    fn overlapping_adjacent_nested_and_duplicate_cutters_have_only_outer_walls() {
        let s = tile_size();
        let t = terrain(3, |_, _| 10.0);
        let a = rect(s * 0.1, s * 0.2, s * 0.5, s * 0.6, 2.0);
        for (b, width) in [
            (rect(s * 0.3, s * 0.2, s * 0.7, s * 0.6, 2.0), 0.6),
            (rect(s * 0.5, s * 0.2, s * 0.7, s * 0.6, 2.0), 0.6),
            (rect(s * 0.2, s * 0.3, s * 0.4, s * 0.5, -5.0), 0.4),
            (a.clone(), 0.4),
        ] {
            let mesh = terrain_hole_walls(&[a.clone(), b], &t);
            valid(&mesh);
            assert!((area(&mesh) - 2.0 * s * (width + 0.4) * 8.0).abs() < 0.05);
        }
    }

    #[test]
    fn wall_tops_follow_each_terrain_triangle_not_a_chord_across_the_grid() {
        let s = tile_size();
        let t = Terrain {
            cells: 2,
            heights: vec![0.0, 10.0, 0.0, 10.0, 0.0, 10.0, 0.0, 10.0, 0.0],
        };
        let ring = rect(s * 0.1, s * 0.2, s * 0.9, s * 0.7, -5.0);
        let mesh = terrain_hole_walls(&[ring], &t);
        valid(&mesh);
        let mut top_edges = 0;
        for tri in mesh.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (
                    mesh.positions[tri[k] as usize].as_dvec3(),
                    mesh.positions[tri[(k + 1) % 3] as usize].as_dvec3(),
                );
                if (a.z - height(&t, a.truncate(), s)).abs() < 1e-4
                    && (b.z - height(&t, b.truncate(), s)).abs() < 1e-4
                {
                    let mid = (a + b) * 0.5;
                    assert!((mid.z - height(&t, mid.truncate(), s)).abs() < 1e-4);
                    top_edges += 1;
                }
            }
        }
        assert!(
            top_edges > 8,
            "terrain grid and its diagonals must subdivide the rim"
        );
    }

    #[test]
    fn walls_meet_across_tile_seams_without_a_cap_and_split_where_the_rim_crosses_ground() {
        let s = tile_size();
        let ring = rect(s * 0.9, s * 0.2, s * 1.1, s * 0.8, 0.0);
        let shifted: Vec<_> = ring.iter().map(|p| *p - DVec3::new(s, 0.0, 0.0)).collect();
        let left = terrain_hole_walls(
            &[ring],
            &terrain(2, |x, y| (10.0 + x * 0.01 + y * 0.02) as f32),
        );
        let right = terrain_hole_walls(
            &[shifted],
            &terrain(2, |x, y| (10.0 + (x + s) * 0.01 + y * 0.02) as f32),
        );
        for (mesh, seam) in [(&left, s as f32), (&right, 0.0)] {
            valid(mesh);
            assert!(!mesh
                .indices
                .chunks_exact(3)
                .any(|t| t
                    .iter()
                    .all(|i| (mesh.positions[*i as usize].x - seam).abs() < 1e-4)));
        }
        let seam = |mesh: &MeshData, x: f32| {
            let mut pts: Vec<_> = mesh
                .positions
                .iter()
                .filter(|p| (p.x - x).abs() < 1e-4)
                .map(|p| ((p.y * 1000.0).round() as i64, (p.z * 1000.0).round() as i64))
                .collect();
            pts.sort_unstable();
            pts.dedup();
            pts
        };
        assert!(!seam(&left, s as f32).is_empty());
        assert_eq!(seam(&left, s as f32), seam(&right, 0.0));
        let t = terrain(1, |_, _| 10.0);
        assert!(terrain_hole_walls(&[rect(10.0, 10.0, 20.0, 20.0, 10.0)], &t).is_empty());
        let mut crossing = rect(10.0, 10.0, 20.0, 20.0, 8.0);
        crossing[1].z = 12.0;
        crossing[2].z = 12.0;
        let mesh = terrain_hole_walls(&[crossing], &t);
        valid(&mesh);
        for tri in mesh.indices.chunks_exact(3) {
            let dz = tri.iter().map(|i| mesh.positions[*i as usize].z - 10.0);
            let lo = dz.clone().fold(f32::INFINITY, f32::min);
            let hi = dz.fold(f32::NEG_INFINITY, f32::max);
            assert!(
                lo * hi >= -1e-6,
                "a crossing must not produce a twisted wall"
            );
        }
    }
}
