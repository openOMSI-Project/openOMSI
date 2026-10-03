//! Trace touching open rims through their incident triangle fans.
use glam::DVec3;
use std::collections::{HashMap, HashSet};

type Key = (i64, i64, i64);
type Edge = (Key, Key);

fn edge(a: Key, b: Key) -> Edge {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Rotate around `b` through adjacent faces until the next open edge is reached.
/// This also works with mixed triangle winding and welded UV seams. At an edge shared
/// by more than two faces there is no unique continuation, so only that walk is rejected.
fn successor(
    (mut a, b): Edge,
    mut face: usize,
    faces: &[[Key; 3]],
    edges: &HashMap<Edge, Vec<usize>>,
) -> Option<(Edge, usize)> {
    for _ in 0..faces.len() {
        let c = *faces[face].iter().find(|&&p| p != a && p != b)?;
        match edges.get(&edge(b, c))?.as_slice() {
            [_] => return Some(((b, c), face)),
            [first, second] => {
                face = if *first == face { *second } else { *first };
                a = c;
            }
            _ => return None,
        }
    }
    None
}

pub(super) fn trace_faces(
    faces: &[[Key; 3]],
    edges: &HashMap<Edge, Vec<usize>>,
    places: &HashMap<Key, DVec3>,
) -> Vec<Vec<DVec3>> {
    let mut starts: Vec<_> = edges
        .iter()
        .filter_map(|(&e, owners)| (owners.len() == 1).then_some(e))
        .collect();
    starts.sort_unstable();
    let mut used = HashSet::new();
    let mut rings = Vec::new();
    for start in starts {
        let mut current = start;
        let mut face = edges[&start][0];
        let mut ring = Vec::new();
        while used.insert(edge(current.0, current.1)) {
            ring.push(places[&current.0]);
            let Some((next, owner)) = successor(current, face, faces, edges) else {
                break;
            };
            current = next;
            face = owner;
            // A repeated vertex alone does not close the rim: separate triangle fans
            // can meet there. Close only on the same directed starting edge.
            if current == start {
                if ring.len() >= 3 {
                    rings.push(ring);
                }
                break;
            }
        }
    }
    rings
}

#[cfg(test)]
mod tests {
    use crate::{hole_mesh_rims, terrain_hole_walls, MeshData};
    use glam::{DVec3, Mat4, Vec3};
    use omsi_map::Terrain;

    fn face(mesh: &mut MeshData, corners: [Vec3; 3]) {
        // Distinct vertices for every face are ordinary material/UV seams.
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(corners);
        mesh.indices.extend([base, base + 1, base + 2]);
    }

    fn quad(mesh: &mut MeshData, [a, b, c, d]: [Vec3; 4]) {
        face(mesh, [a, b, c]);
        face(mesh, [d, c, a]); // Deliberately mixed winding.
    }

    #[test]
    fn touching_profile_fans_keep_separate_rims_through_uv_seams() {
        let a = Vec3::new(40.0, 40.0, 2.0);
        let profiles = [
            [
                a,
                a + Vec3::X * 10.0,
                a + Vec3::new(10.0, 6.0, 0.0),
                a + Vec3::Y * 6.0,
            ],
            [
                a,
                a - Vec3::X * 10.0,
                a - Vec3::new(10.0, 5.0, 0.0),
                a - Vec3::Y * 5.0,
            ],
            [
                a,
                a + Vec3::new(-4.0, 4.0, 0.0),
                a + Vec3::new(-4.0, 4.0, 6.0),
                a + Vec3::Z * 6.0,
            ],
        ];
        let mut mesh = MeshData::default();
        for points in profiles {
            quad(&mut mesh, points);
        }
        let transform = Mat4::from_rotation_x(0.3);
        let origin = DVec3::new(1e6, 2e6, 30.0);
        let rims = hole_mesh_rims(&mesh, &transform, origin);
        assert_eq!(rims.len(), 3); // Six open edges meet at the welded shared corner.
        for points in profiles {
            let expected = points.map(|p| origin + transform.transform_point3(p).as_dvec3());
            assert!(rims
                .iter()
                .any(|r| r.len() == 4 && expected.iter().all(|p| r.contains(p))));
        }
        assert_eq!(rims, hole_mesh_rims(&mesh, &transform, origin));

        // The projected vertical fan has no area. The two roads get complete walls,
        // but their touching corner cannot join them into a self-crossing polygon.
        let rims = hole_mesh_rims(&mesh, &Mat4::IDENTITY, DVec3::ZERO);
        let terrain = Terrain {
            cells: 1,
            heights: vec![10.0; 4],
        };
        let walls = terrain_hole_walls(&rims, &terrain);
        let area: f32 = walls
            .indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| walls.positions[t[i] as usize]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum();
        assert!((area - 496.0).abs() < 0.01, "{area}");
    }

    #[test]
    fn ambiguous_face_junction_does_not_discard_an_independent_open_rim() {
        let [a, b] = [Vec3::ZERO, Vec3::X];
        let mut mesh = MeshData::default();
        for c in [Vec3::Y, -Vec3::Y, Vec3::Z] {
            face(&mut mesh, [a, b, c]);
        }
        let separate = [
            Vec3::new(10.0, 10.0, 2.0),
            Vec3::new(12.0, 10.0, 2.0),
            Vec3::new(10.0, 12.0, 2.0),
        ];
        face(&mut mesh, separate);
        let rims = hole_mesh_rims(&mesh, &Mat4::IDENTITY, DVec3::ZERO);
        assert_eq!(rims.len(), 1);
        assert_eq!(rims[0].len(), 3);
        assert!(separate.iter().all(|p| rims[0].contains(&p.as_dvec3())));
    }
}
