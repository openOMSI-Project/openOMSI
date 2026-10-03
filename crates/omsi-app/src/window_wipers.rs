//! Persistent wetness on the player's glass, cleared by the animated blade's swept area.
//! A small mask uses the existing transmap binding: no extra draw or offscreen pass.

use glam::{Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_render::{MaterialId, Renderer, Scene, TextureId};
use omsi_sim::VehicleInstance;

const SIZE: usize = 128;

struct Blade {
    mesh: usize,
    ends: [Vec3; 2],
    previous: [Vec3; 2],
    current: [Vec3; 2],
}

struct Film {
    mesh: usize,
    slot: usize,
    material: MaterialId,
    texture: TextureId,
    points: Vec<Vec3>,
    // 0..1 is droplet wetness; 1..2 holds the water ridge pushed by a blade.
    wet: Vec<f32>,
    unwiped: f32,
    image: omsi_texture::Image,
    bounds: [f32; 4],
}

pub(crate) struct WindowWipers {
    blades: Vec<Blade>,
    films: Vec<Film>,
    time: f64,
}

impl WindowWipers {
    pub(crate) fn new(
        renderer: &Renderer,
        scene: &mut Scene,
        vehicle: &VehicleInstance,
        render: &crate::scene::VehicleRender,
    ) -> Self {
        let instances = &render.instances;
        let mut blades = Vec::new();
        for (mesh, vm) in vehicle.ty.meshes.iter().enumerate() {
            let def = &vehicle.ty.model.meshes[vm.def_index];
            let name = def.file.to_ascii_lowercase();
            let animated = def.animations.iter().any(|a| {
                let v = a.variable.to_ascii_lowercase();
                v.contains("wiper") || v.contains("wisch")
            });
            // Prefer a separate blade; some buses (C2) export blade and arm together.
            if !animated
                || !["wiper", "wisch"].iter().any(|s| name.contains(s))
                || ["wash", "wasser", "schalter", "switch", "hebel", "motor"]
                    .iter()
                    .any(|s| name.contains(s))
            {
                continue;
            }
            let Some(data) = vehicle.ty.mesh_data(mesh) else {
                continue;
            };
            let Some(ends) = blade_ends(&data.positions) else {
                continue;
            };
            let ends = if name.contains("arm") {
                if vehicle.ty.model.meshes.iter().any(|d| {
                    let n = d.file.to_ascii_lowercase();
                    n.contains("wischerblatt")
                        || n.contains("wiperblade")
                        || n.contains("wiper_blade")
                }) {
                    continue;
                }
                let Some(ends) = combined_blade_ends(&data.positions) else {
                    continue;
                };
                ends
            } else {
                ends
            };
            let previous = ends.map(|p| vehicle.mesh_transforms[mesh].transform_point3(p));
            blades.push(Blade {
                mesh,
                ends,
                previous,
                current: previous,
            });
        }
        let mut films = Vec::new();
        if !blades.is_empty() {
            for (mesh, vm) in vehicle.ty.meshes.iter().enumerate() {
                for slot in 0..vm.materials.len() {
                    let controlled = vm.overrides.iter().any(|m| {
                        omsi_sim::vehicle::override_slot(&vm.materials, m) == Some(slot)
                            && m.alphascale.as_deref().is_some_and(|v| {
                                matches!(
                                    v.trim().to_ascii_lowercase().as_str(),
                                    "rain_window_front_wetness"
                                        | "rain_window_wiped_wetness"
                                        | "rain_window_norm_wetness"
                                )
                            })
                    });
                    if !controlled
                        || render
                            .variants
                            .iter()
                            .any(|v| v.mesh == mesh && v.slot == slot)
                        || vm.overrides.iter().any(|m| {
                            omsi_sim::vehicle::override_slot(&vm.materials, m) == Some(slot)
                                && (m.use_script_texture.is_some() || m.use_text_texture.is_some())
                        })
                    {
                        continue;
                    }
                    let Some(data) = vehicle.ty.mesh_data(mesh) else {
                        continue;
                    };
                    let Some((points, bounds)) = film_points(&data, slot) else {
                        continue;
                    };
                    // Leave panes away from the blades on their existing script path.
                    if !points.iter().filter(|p| p.is_finite()).any(|p| {
                        let p = vehicle.mesh_transforms[mesh].transform_point3(*p);
                        blades.iter().any(|b| {
                            let [a, c] = b.previous;
                            let t = ((p - a).dot(c - a) / a.distance_squared(c)).clamp(0.0, 1.0);
                            p.distance_squared(a.lerp(c, t)) < 0.35 * 0.35
                        })
                    }) {
                        continue;
                    }
                    let base = scene.instances[instances[mesh]].materials[slot];
                    // Keep custom transmaps and material variants under their script's control.
                    if scene.materials[base].transmap.is_some() {
                        continue;
                    }
                    let wetness = vehicle
                        .var("Rain_Window_Norm_Wetness")
                        .map(|wet| (wet * 1.8).clamp(0.0, 1.0))
                        .unwrap_or(vehicle.mesh_props[mesh].slot_alpha[slot]);
                    let mut image = omsi_texture::Image {
                        width: SIZE as u32,
                        height: SIZE as u32,
                        rgba: vec![255; SIZE * SIZE * 4],
                        has_alpha: true,
                    };
                    for (pixel, point) in image.rgba.chunks_exact_mut(4).zip(&points) {
                        // Store depth to distinguish panes which overlap in X/Z (a
                        // wraparound windscreen's side must not inherit a front wipe).
                        let depth = ((point.y + 32.0) / 64.0 * 65535.0).round() as u16;
                        pixel[0] = (depth >> 8) as u8;
                        pixel[1] = depth as u8;
                        pixel[2] = (wetness * 255.0).round() as u8;
                        pixel[3] = (wetness * 127.5).round() as u8;
                    }
                    let texture = renderer.add_data_texture(scene, &image);
                    let material = renderer
                        .add_window_wetness_material(scene, base, texture, bounds)
                        .unwrap();
                    films.push(Film {
                        mesh,
                        slot,
                        material,
                        texture,
                        points,
                        wet: vec![wetness; SIZE * SIZE],
                        unwiped: wetness,
                        image,
                        bounds,
                    });
                }
            }
        }
        log::debug!(
            "window wipers: {} blades, {} wetness masks",
            blades.len(),
            films.len()
        );
        Self {
            blades,
            films,
            time: vehicle.host.clock.run_time,
        }
    }

    pub(crate) fn textures(&self) -> impl Iterator<Item = TextureId> + '_ {
        self.films.iter().map(|f| f.texture)
    }

    pub(crate) fn materials(&self) -> impl Iterator<Item = MaterialId> + '_ {
        self.films.iter().map(|f| f.material)
    }

    pub(crate) fn update(
        &mut self,
        renderer: &Renderer,
        scene: &mut Scene,
        vehicle: &VehicleInstance,
        instances: &[usize],
    ) {
        if self.films.is_empty() {
            return;
        }
        let now = vehicle.host.clock.run_time;
        let dt = (now - self.time).max(0.0) as f32;
        self.time = now;
        let washer = vehicle.var("wiper_wash").unwrap_or(0.0).clamp(0.0, 1.0);
        // Do not read Front/Wiped wetness: the script resets those for the whole pane.
        // Keep rainfall local and let a dry pane evaporate gradually after the shower.
        let deposit = if vehicle.host.precip_rate > 0.0 || washer > 0.0 {
            (vehicle.host.precip_rate * 0.45 * (1.0 + vehicle.physics.speed.abs() * 0.025)
                + washer * 0.8)
                * dt
        } else {
            -0.025 * dt
        };
        for blade in &mut self.blades {
            blade.current = blade
                .ends
                .map(|p| vehicle.mesh_transforms[blade.mesh].transform_point3(p));
        }
        for film in &mut self.films {
            if dt > 0.0 {
                film.unwiped = (film.unwiped + deposit).clamp(0.0, 1.0);
                for wet in &mut film.wet {
                    // Water piled up by a blade drains back into beads; ordinary rain
                    // fills the cleared film without instantly erasing that moving ridge.
                    let ridge = (*wet - 1.0).max(0.0) * (1.0 - dt * 3.0).max(0.0);
                    *wet = ((*wet).min(1.0) + deposit).clamp(0.0, 1.0) + ridge;
                }
                let inverse = vehicle.mesh_transforms[film.mesh].inverse();
                for blade in &self.blades {
                    if !vehicle.mesh_props[blade.mesh].visible {
                        continue;
                    }
                    let previous = blade.previous.map(|p| inverse.transform_point3(p));
                    let current = blade.current.map(|p| inverse.transform_point3(p));
                    wipe(&mut film.wet, &film.points, film.bounds, previous, current);
                }
            }
            let mut changed = false;
            for (i, (&point, wet)) in film.points.iter().zip(&mut film.wet).enumerate() {
                let alpha = (if point.is_finite() {
                    *wet
                } else {
                    film.unwiped
                } * 127.5)
                    .round() as u8;
                let unwiped = (film.unwiped * 255.0).round() as u8;
                let pixel = &mut film.image.rgba[i * 4..i * 4 + 4];
                changed |= pixel[3] != alpha || pixel[2] != unwiped;
                pixel[2] = unwiped;
                pixel[3] = alpha;
            }
            if changed {
                renderer.update_texture(scene, film.texture, &film.image);
            }
            let inst = instances[film.mesh];
            renderer.set_material(scene, inst, film.slot, film.material);
            let mut alpha = scene.instances[inst].slot_alpha.clone();
            alpha[film.slot] = 1.0; // the mask owns wetness; scripts still own visibility
            let visible = scene.instances[inst].visible;
            renderer.set_params(
                scene,
                inst,
                &alpha,
                visible,
                &vehicle.mesh_props[film.mesh].slot_uv,
            );
        }
        for blade in &mut self.blades {
            blade.previous = blade.current;
        }
    }
}

/// An arm biases the principal axis of a combined mesh away from the rubber. Find
/// its dominant long, narrow strip first, using a bounded set of candidate lines.
/// This runs once on loading; both density and length favour the blade over its joints.
fn combined_blade_ends(points: &[Vec3]) -> Option<[Vec3; 2]> {
    if points.len() < 2 {
        return None;
    }
    let mut best = None;
    let mut score = 0.0;
    for i in 0..64 {
        let a = points[i * points.len() / 64];
        // A fixed stride samples across exporter vertex groups without a runtime RNG.
        let b = points[(i * 811 + points.len() / 2) % points.len()];
        if a.distance_squared(b) < 0.09 {
            continue;
        }
        let axis = (b - a).normalize();
        let mut count = 0;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for &p in points {
            let d = p - a;
            if d.cross(axis).length_squared() < 0.015 * 0.015 {
                count += 1;
                let t = d.dot(axis);
                lo = lo.min(t);
                hi = hi.max(t);
            }
        }
        let candidate = count as f32 * (hi - lo).powi(2);
        if candidate > score {
            score = candidate;
            best = Some((a, axis));
        }
    }
    let (a, axis) = best?;
    let contact: Vec<_> = points
        .iter()
        .copied()
        .filter(|p| (*p - a).cross(axis).length_squared() < 0.015 * 0.015)
        .collect();
    blade_ends(&contact)
}

/// Principal axis of the narrow blade mesh; box diagonals include its thickness and
/// give the wrong contact line on a blade exported at an angle.
fn blade_ends(points: &[Vec3]) -> Option<[Vec3; 2]> {
    if points.len() < 2 {
        return None;
    }
    let centre = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let mut axis = points
        .iter()
        .map(|p| *p - centre)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize_or_zero();
    for _ in 0..8 {
        axis = points
            .iter()
            .map(|p| {
                let d = *p - centre;
                d * d.dot(axis)
            })
            .sum::<Vec3>()
            .normalize_or_zero();
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in points {
        let t = (*p - centre).dot(axis);
        lo = lo.min(t);
        hi = hi.max(t);
    }
    (hi - lo > 0.15 && hi - lo < 2.0).then_some([centre + axis * lo, centre + axis * hi])
}

/// Rasterise only the precipitation slot, once. Mesh X/Z rather than texture UV keeps
/// the mask unique when rain textures repeat or several windows share a texture atlas.
fn film_points(data: &MeshData, slot: usize) -> Option<(Vec<Vec3>, [f32; 4])> {
    let triangles: Vec<[Vec3; 3]> = data
        .ranges
        .iter()
        .filter(|r| r.2 as usize == slot)
        .flat_map(|&(start, count, _)| {
            data.indices[start as usize..(start + count) as usize].chunks_exact(3)
        })
        .map(|i| {
            [
                data.positions[i[0] as usize],
                data.positions[i[1] as usize],
                data.positions[i[2] as usize],
            ]
        })
        .collect();
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in triangles.iter().flatten() {
        let p = Vec2::new(p.x, p.z);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let size = hi - lo;
    if !size.is_finite() || size.min_element() < 0.05 {
        return None;
    }
    let mut points = vec![Vec3::NAN; SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = lo
                + size
                    * Vec2::new(
                        (x as f32 + 0.5) / SIZE as f32,
                        (y as f32 + 0.5) / SIZE as f32,
                    );
            for &[a, b, c] in &triangles {
                let (a2, b2, c2) = (
                    Vec2::new(a.x, a.z),
                    Vec2::new(b.x, b.z),
                    Vec2::new(c.x, c.z),
                );
                let det = (b2 - a2).perp_dot(c2 - a2);
                if det.abs() < 1e-8 {
                    continue;
                }
                let u = (p - a2).perp_dot(c2 - a2) / det;
                let v = (b2 - a2).perp_dot(p - a2) / det;
                if u >= -0.001 && v >= -0.001 && u + v <= 1.001 {
                    points[y * SIZE + x] = a + (b - a) * u + (c - a) * v;
                    break;
                }
            }
        }
    }
    Some((points, [lo.x, lo.y, 1.0 / size.x, 1.0 / size.y]))
}

/// Only visit the small rectangle crossed this frame; stationary blades cost no scan.
fn wipe(
    wet: &mut [f32],
    points: &[Vec3],
    bounds: [f32; 4],
    previous: [Vec3; 2],
    current: [Vec3; 2],
) {
    if previous[0]
        .distance_squared(current[0])
        .max(previous[1].distance_squared(current[1]))
        < 1e-8
    {
        return; // a parked blade must not continually erase fresh rain
    }
    let lo = previous[0].min(previous[1]).min(current[0]).min(current[1]) - Vec3::splat(0.1);
    let hi = previous[0].max(previous[1]).max(current[0]).max(current[1]) + Vec3::splat(0.1);
    let pixel = |p: Vec3| {
        Vec2::new((p.x - bounds[0]) * bounds[2], (p.z - bounds[1]) * bounds[3]) * SIZE as f32
    };
    let lo = pixel(lo)
        .floor()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let hi = pixel(hi)
        .ceil()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let triangles = [
        SweepTriangle::new(previous[0], previous[1], current[1]),
        SweepTriangle::new(previous[0], current[1], current[0]),
    ];
    let mut water = 0.0;
    for y in lo.y as usize..=hi.y as usize {
        for x in lo.x as usize..=hi.x as usize {
            let i = y * SIZE + x;
            if points[i].is_finite() && triangles.iter().flatten().any(|t| t.contains(points[i])) {
                water += (wet[i] - 0.004).max(0.0);
                wet[i] = wet[i].min(0.004); // a clean blade leaves a very thin residual film
            }
        }
    }
    if water > 0.0 {
        if let Some(plane) = triangles.iter().flatten().next() {
            push_water(wet, points, bounds, previous, current, plane.normal, water);
        }
    }
}

/// Move collected water to the leading side of the blade. The bounded ridge holds
/// a little water; excess runs off, rather than growing an opaque wall indefinitely.
fn push_water(
    wet: &mut [f32],
    points: &[Vec3],
    bounds: [f32; 4],
    previous: [Vec3; 2],
    current: [Vec3; 2],
    normal: Vec3,
    water: f32,
) {
    const WIDTH: f32 = 0.035;
    let edge = current[1] - current[0];
    let length2 = edge.length_squared();
    if length2 < 1e-8 {
        return;
    }
    let pixels = length2.sqrt() * WIDTH * (SIZE * SIZE) as f32 * bounds[2] * bounds[3] * 0.5;
    let amount = water / pixels.max(1.0);
    let pixel = |p: Vec3| {
        Vec2::new((p.x - bounds[0]) * bounds[2], (p.z - bounds[1]) * bounds[3]) * SIZE as f32
    };
    let lo = pixel(current[0].min(current[1]) - Vec3::splat(WIDTH))
        .floor()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let hi = pixel(current[0].max(current[1]) + Vec3::splat(WIDTH))
        .ceil()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    for y in lo.y as usize..=hi.y as usize {
        for x in lo.x as usize..=hi.x as usize {
            let i = y * SIZE + x;
            let p = points[i];
            if !p.is_finite() {
                continue;
            }
            let t = (p - current[0]).dot(edge) / length2;
            if !(0.0..=1.0).contains(&t) {
                continue;
            }
            let motion = (current[0] - previous[0]).lerp(current[1] - previous[1], t);
            let across = (motion - edge * (motion.dot(edge) / length2)).normalize_or_zero();
            let d = p - current[0] - edge * t;
            let distance = d.dot(across);
            if d.dot(normal).abs() <= 0.1 && distance > 0.0 && distance < WIDTH {
                wet[i] = (wet[i] + amount * (1.0 - distance / WIDTH)).min(2.0);
            }
        }
    }
}

struct SweepTriangle {
    origin: Vec3,
    normal: Vec3,
    u: Vec3,
    v: Vec3,
}

impl SweepTriangle {
    fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let (ab, ac) = (b - a, c - a);
        let n = ab.cross(ac);
        let det = n.length_squared();
        if det < 1e-12 {
            return None;
        }
        Some(Self {
            origin: a,
            normal: n.normalize(),
            u: ac.cross(n) / det,
            v: n.cross(ab) / det,
        })
    }

    fn contains(&self, p: Vec3) -> bool {
        let d = p - self.origin;
        let (u, v) = (d.dot(self.u), d.dot(self.v));
        // The rain layer is offset from the rubber, and windshields can be curved.
        d.dot(self.normal).abs() <= 0.1 && u >= -1e-5 && v >= -1e-5 && u + v <= 1.00001
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_mesh_contact_follows_the_blade_instead_of_the_arm() {
        let mut points = Vec::new();
        for i in 0..120 {
            let t = i as f32 / 119.0;
            points.push(Vec3::new(t, 0.0, 0.5));
            points.push(Vec3::new(t, 0.01, 0.5));
            points.push(Vec3::new(0.5 + t * 0.5, 0.02, 0.5 - t * 0.4));
        }
        let ends = combined_blade_ends(&points).unwrap();
        assert!(ends[0].distance(ends[1]) > 0.98);
        assert!(ends.iter().all(|p| (p.z - 0.5).abs() < 0.01));
    }

    #[test]
    fn only_the_blades_traversed_strip_is_cleared_in_both_directions() {
        let start = [Vec3::new(0.0, 0.03, 0.0), Vec3::new(0.0, 0.03, 1.0)];
        let end = start.map(|p| p + Vec3::X * 0.4);
        let data = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::X + Vec3::Z],
            indices: vec![0, 1, 2, 1, 3, 2],
            ranges: vec![(0, 6, 0)],
            ..Default::default()
        };
        let (points, bounds) = film_points(&data, 0).unwrap();
        for (from, to) in [(start, end), (end, start)] {
            let mut wet = vec![1.0; SIZE * SIZE];
            wipe(&mut wet, &points, bounds, from, to);
            for (&p, &wet) in points.iter().zip(&wet) {
                if p.x < 0.4 {
                    assert!(wet <= 0.004, "{p:?}: {wet}");
                } else if p.x > 0.435 || to == start {
                    assert_eq!(wet, 1.0, "{p:?}");
                }
            }
            if to == end {
                assert!(points.iter().zip(&wet).any(|(p, w)| p.x > 0.4 && *w > 1.0));
            }
        }
        let mut wet = vec![1.0; SIZE * SIZE];
        wipe(
            &mut wet,
            &points,
            bounds,
            end,
            start.map(|p| p + Vec3::X * 0.2),
        );
        assert!(points.iter().zip(&wet).any(|(p, w)| p.x < 0.2 && *w > 1.0));
        assert!(wet.iter().all(|w| *w <= 2.0));
        let mut dry = vec![0.0; SIZE * SIZE];
        wipe(&mut dry, &points, bounds, start, end);
        assert!(dry.iter().all(|w| *w == 0.0));
        let mut wet = vec![0.6; SIZE * SIZE];
        wipe(&mut wet, &points, bounds, start, start);
        assert!(wet.iter().all(|w| *w == 0.6));
        let off_glass = start.map(|p| p - Vec3::Y * 0.3);
        wipe(
            &mut wet,
            &points,
            bounds,
            off_glass,
            off_glass.map(|p| p + Vec3::X * 0.4),
        );
        assert!(wet.iter().all(|w| *w == 0.6));
    }

    #[test]
    fn masks_use_geometry_when_texture_coordinates_repeat() {
        let data = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::X + Vec3::Z],
            indices: vec![0, 1, 2, 1, 3, 2],
            ranges: vec![(0, 6, 0)],
            uvs: vec![Vec2::ZERO; 4],
            ..Default::default()
        };
        let (points, bounds) = film_points(&data, 0).unwrap();
        assert_eq!(bounds, [0.0, 0.0, 1.0, 1.0]);
        assert!(points.iter().all(|p| p.is_finite()));
        assert!(points[0].x < 0.01 && points[SIZE * SIZE - 1].x > 0.99);
        assert!(film_points(&data, 1).is_none());
    }

    #[test]
    fn diagonal_blade_contact_uses_its_long_axis() {
        let axis = Vec3::new(0.6, 0.0, 0.8);
        let points = [
            axis * -0.4,
            axis * 0.4,
            axis * -0.4 + Vec3::Y * 0.02,
            axis * 0.4 + Vec3::Y * 0.02,
        ];
        let ends = blade_ends(&points).unwrap();
        assert!((ends[0].distance(ends[1]) - 0.8).abs() < 1e-4);
        assert!(ends.iter().all(|p| (p.y - 0.01).abs() < 1e-4));
    }
}
