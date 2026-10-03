//! Persistent wetness on the player's glass, cleared by the animated blade's swept area.
//! A small mask uses the existing transmap binding: no extra draw or offscreen pass.

use crate::window_drops::Drops;
use glam::{Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_render::{MaterialId, Renderer, Scene, TextureId};
use omsi_sim::VehicleInstance;

const SIZE: usize = 128;
const WIPED_FILM: f32 = -0.04;

// Eight codes below zero identify a short-lived residual film, without another texture.
fn encode_wetness(wet: f32) -> u8 {
    (wet * (247.0 / 2.0) + 8.0).round() as u8
}

fn advance_wetness(wet: f32, rate: f32, dt: f32) -> f32 {
    let ridge = (wet - 1.0).max(0.0);
    let mut base = wet.min(1.0);
    let mut remaining = dt;
    if base < 0.0 {
        let drying = -base / 0.08;
        if dt < drying {
            return base + dt * 0.08;
        }
        base = 0.0;
        remaining -= drying;
    }
    (base + rate * remaining).clamp(0.0, 1.0) + ridge
}

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
    drops: Drops,
    drops_texture: TextureId,
    paint_time: f32,
    points: Vec<Vec3>,
    // Negative is a recent wipe's residual film; 0..1 droplets; 1..2 the pushed ridge.
    wet: Vec<f32>,
    unwiped: f32,
    local: bool,
    image: omsi_texture::Image,
    bounds: [f32; 4],
}

pub(crate) struct WindowWipers {
    blades: Vec<Blade>,
    films: Vec<Film>,
    time: f64,
    runoff: Vec<f32>,
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
                    let cab = blades
                        .iter()
                        .map(|b| (b.current[0] + b.current[1]) * 0.5)
                        .sum::<Vec3>()
                        / blades.len() as f32;
                    let cab = vehicle.mesh_transforms[mesh]
                        .inverse()
                        .transform_point3(cab);
                    let Some((points, bounds)) = film_points_in_cab(&data, slot, Some(cab)) else {
                        continue;
                    };
                    // Include the cab's side windows; distant passenger panes retain
                    // the procedural path and do not need individual drop simulations.
                    if !points.iter().filter(|p| p.is_finite()).any(|p| {
                        let p = vehicle.mesh_transforms[mesh].transform_point3(*p);
                        blades.iter().any(|b| {
                            let [a, c] = b.previous;
                            let t = ((p - a).dot(c - a) / a.distance_squared(c)).clamp(0.0, 1.0);
                            p.distance_squared(a.lerp(c, t)) < 1.8 * 1.8
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
                        let depth =
                            ((pane_depth(*point, bounds) + 32.0) / 64.0 * 65535.0).round() as u16;
                        pixel[0] = (depth >> 8) as u8;
                        pixel[1] = depth as u8;
                        pixel[2] = (wetness * 255.0).round() as u8;
                        pixel[3] = encode_wetness(wetness);
                    }
                    let texture = renderer.add_data_texture(scene, &image);
                    let drops = Drops::new(
                        Vec2::new(bounds[2].abs().recip(), bounds[3].recip()),
                        wetness,
                        (mesh * 7919 + slot * 104729 + 1) as u32,
                    );
                    let drops_texture = renderer.add_data_texture(scene, &drops.image);
                    let material = renderer
                        .add_window_wetness_material(scene, base, texture, drops_texture, bounds)
                        .unwrap();
                    films.push(Film {
                        mesh,
                        slot,
                        material,
                        texture,
                        drops,
                        drops_texture,
                        paint_time: 1.0,
                        points,
                        wet: vec![wetness; SIZE * SIZE],
                        unwiped: wetness,
                        local: false,
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
            runoff: vec![0.0; SIZE * SIZE],
        }
    }

    pub(crate) fn textures(&self) -> impl Iterator<Item = TextureId> + '_ {
        self.films.iter().flat_map(|f| [f.texture, f.drops_texture])
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
        let rate = if vehicle.host.precip_rate > 0.0 || washer > 0.0 {
            vehicle.host.precip_rate * 0.10 * (1.0 + vehicle.physics.speed.abs() * 0.015)
                + washer * 0.8
        } else {
            -0.025
        };
        for blade in &mut self.blades {
            blade.current = blade
                .ends
                .map(|p| vehicle.mesh_transforms[blade.mesh].transform_point3(p));
        }
        for film in &mut self.films {
            let liquid = vehicle.host.precip_type != 2.0 || vehicle.host.temperature > 0.0;
            let previous_wetness = film.unwiped;
            if dt > 0.0 {
                film.unwiped = (film.unwiped + rate * dt).clamp(0.0, 1.0);
                // An untouched pane has uniform wetness. Once it is saturated,
                // only its moving drops need updates, not every mask pixel.
                if film.local {
                    for wet in &mut film.wet {
                        *wet = advance_wetness(*wet, rate, dt);
                    }
                } else if previous_wetness != film.unwiped {
                    film.wet.fill(film.unwiped);
                }
                // Gravity follows the pane's actual inclination. Relative airflow can
                // carry mobile water sideways/upwards; small pinned beads stay put.
                let inverse_world = scene.instances[instances[film.mesh]].transform.inverse();
                let mut air = inverse_world.transform_vector3(
                    vehicle.host.wind - crate::lights::vehicle_velocity(vehicle),
                );
                let normal_air = pane_depth(air, film.bounds).abs();
                air.z += normal_air * 0.6;
                let tangent = pane_project(air, film.bounds[2] < 0.0);
                let air_force =
                    tangent.normalize_or_zero() * (tangent.length_squared() / 130.0).min(9.0);
                let gravity = inverse_world.transform_vector3(-Vec3::Z);
                let force =
                    gravity + air.normalize_or_zero() * (air.length_squared() / 130.0).min(3.0);
                if liquid {
                    if film.local {
                        drain_water(
                            &mut film.wet,
                            &film.points,
                            film.bounds,
                            force * 0.12,
                            dt,
                            &mut self.runoff,
                        );
                    }
                    let wet = &film.wet;
                    let points = &film.points;
                    let bounds = film.bounds;
                    film.drops.advance(
                        dt,
                        vehicle.host.precip_rate + washer,
                        pane_project(gravity, bounds[2] < 0.0),
                        air_force,
                        if bounds[2] > 0.0 {
                            (normal_air * normal_air / 260.0).min(3.0)
                        } else {
                            0.0
                        },
                        |p| {
                            let i = drop_pixel(p, bounds);
                            if points[i].is_finite() {
                                wet[i]
                            } else {
                                f32::NAN
                            }
                        },
                    );
                }
                let inverse = vehicle.mesh_transforms[film.mesh].inverse();
                for blade in &self.blades {
                    if !vehicle.mesh_props[blade.mesh].visible {
                        continue;
                    }
                    let previous = blade.previous.map(|p| inverse.transform_point3(p));
                    let current = blade.current.map(|p| inverse.transform_point3(p));
                    film.local |= wipe(&mut film.wet, &film.points, film.bounds, previous, current);
                    if liquid {
                        wipe_drops(film, previous, current);
                    }
                }
            }
            film.paint_time += dt;
            if film.paint_time >= 1.0 / 30.0 {
                film.paint_time %= 1.0 / 30.0;
                if liquid {
                    // Also discard initial seeds which landed outside the mesh's slot.
                    let bounds = film.bounds;
                    film.drops.drops.retain(|d| {
                        d.pos.cmpge(Vec2::ZERO).all()
                            && (d.pos * Vec2::new(bounds[2].abs(), bounds[3]))
                                .cmplt(Vec2::ONE)
                                .all()
                            && film.points[drop_pixel(d.pos, bounds)].is_finite()
                    });
                } else {
                    film.drops.drops.clear();
                }
                if film.drops.paint() {
                    renderer.update_texture(scene, film.drops_texture, &film.drops.image);
                }
            }
            let mut changed = false;
            if film.local || previous_wetness != film.unwiped {
                film.local = false;
                let unwiped = (film.unwiped * 255.0).round() as u8;
                for (i, (&point, wet)) in film.points.iter().zip(&film.wet).enumerate() {
                    film.local |= point.is_finite() && *wet != film.unwiped;
                    let alpha = encode_wetness(if point.is_finite() {
                        *wet
                    } else {
                        film.unwiped
                    });
                    let pixel = &mut film.image.rgba[i * 4..i * 4 + 4];
                    changed |= pixel[3] != alpha || pixel[2] != unwiped;
                    pixel[2] = unwiped;
                    pixel[3] = alpha;
                }
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

// A signed inverse width selects X/Z for the front or Y/Z for a side pane.
// The same convention is used by the vertex shader and the two texture maps.
fn pane_project(p: Vec3, side: bool) -> Vec2 {
    Vec2::new(if side { p.y } else { p.x }, p.z)
}

fn pane_depth(p: Vec3, bounds: [f32; 4]) -> f32 {
    if bounds[2] < 0.0 {
        p.x
    } else {
        p.y
    }
}

fn pane_pixel(p: Vec3, bounds: [f32; 4]) -> Vec2 {
    (pane_project(p, bounds[2] < 0.0) - Vec2::new(bounds[0], bounds[1]))
        * Vec2::new(bounds[2].abs(), bounds[3])
        * SIZE as f32
}

fn drop_pixel(p: Vec2, bounds: [f32; 4]) -> usize {
    let uv = (p * Vec2::new(bounds[2].abs(), bounds[3]) * SIZE as f32)
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    uv.y as usize * SIZE + uv.x as usize
}

fn wipe_drops(film: &mut Film, previous: [Vec3; 2], current: [Vec3; 2]) {
    let triangles = [
        SweepTriangle::new(previous[0], previous[1], current[1], film.bounds[2] < 0.0),
        SweepTriangle::new(previous[0], current[1], current[0], film.bounds[2] < 0.0),
    ];
    let edge = current[1] - current[0];
    let length2 = edge.length_squared();
    if length2 < 1e-8 {
        return;
    }
    for drop in &mut film.drops.drops {
        let mut p = film.points[drop_pixel(drop.pos, film.bounds)];
        if film.bounds[2] < 0.0 {
            p.y = drop.pos.x + film.bounds[0];
        } else {
            p.x = drop.pos.x + film.bounds[0];
        }
        p.z = drop.pos.y + film.bounds[1];
        let old = drop.previous + Vec2::new(film.bounds[0], film.bounds[1]);
        let mut before = p;
        if film.bounds[2] < 0.0 {
            before.y = old.x;
        } else {
            before.x = old.x;
        }
        before.z = old.y;
        if !triangles.iter().flatten().any(|t| t.crosses(before, p)) {
            continue;
        }
        let t = ((p - current[0]).dot(edge) / length2).clamp(0.0, 1.0);
        let motion = current[0].lerp(current[1], t) - previous[0].lerp(previous[1], t);
        let across = (motion - edge * (motion.dot(edge) / length2)).normalize_or_zero();
        let pushed = current[0] + edge * t + across * 0.008;
        drop.displace(
            pane_project(pushed, film.bounds[2] < 0.0) - Vec2::new(film.bounds[0], film.bounds[1]),
            pane_project(across * 0.15, film.bounds[2] < 0.0),
        );
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

/// Rasterise only the precipitation slot, once. Mesh coordinates rather than texture UV keep
/// the mask unique when rain textures repeat or several windows share a texture atlas.
#[cfg(test)]
fn film_points(data: &MeshData, slot: usize) -> Option<(Vec<Vec3>, [f32; 4])> {
    film_points_in_cab(data, slot, None)
}

fn film_points_in_cab(
    data: &MeshData,
    slot: usize,
    cab: Option<Vec3>,
) -> Option<(Vec<Vec3>, [f32; 4])> {
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
    let (mut lo3, mut hi3) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for &p in triangles.iter().flatten() {
        lo3 = lo3.min(p);
        hi3 = hi3.max(p);
    }
    let side = hi3.y - lo3.y > (hi3.x - lo3.x) * 1.5;
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in triangles.iter().flatten() {
        let p = pane_project(*p, side);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if side {
        if let Some(cab) = cab {
            lo.x = lo.x.max(cab.y - 1.8);
            hi.x = hi.x.min(cab.y + 1.8);
        }
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
                    pane_project(a, side),
                    pane_project(b, side),
                    pane_project(c, side),
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
    Some((
        points,
        [
            lo.x,
            lo.y,
            if side { -1.0 / size.x } else { 1.0 / size.x },
            1.0 / size.y,
        ],
    ))
}

/// Only visit the small rectangle crossed this frame; stationary blades cost no scan.
fn wipe(
    wet: &mut [f32],
    points: &[Vec3],
    bounds: [f32; 4],
    previous: [Vec3; 2],
    current: [Vec3; 2],
) -> bool {
    if previous[0]
        .distance_squared(current[0])
        .max(previous[1].distance_squared(current[1]))
        < 1e-8
    {
        return false; // a parked blade must not continually erase fresh rain
    }
    let lo = previous[0].min(previous[1]).min(current[0]).min(current[1]) - Vec3::splat(0.1);
    let hi = previous[0].max(previous[1]).max(current[0]).max(current[1]) + Vec3::splat(0.1);
    let pixel = |p: Vec3| pane_pixel(p, bounds);
    let lo = pixel(lo)
        .floor()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let hi = pixel(hi)
        .ceil()
        .clamp(Vec2::ZERO, Vec2::splat((SIZE - 1) as f32));
    let triangles = [
        SweepTriangle::new(previous[0], previous[1], current[1], bounds[2] < 0.0),
        SweepTriangle::new(previous[0], current[1], current[0], bounds[2] < 0.0),
    ];
    let mut water = 0.0;
    for y in lo.y as usize..=hi.y as usize {
        for x in lo.x as usize..=hi.x as usize {
            let i = y * SIZE + x;
            if points[i].is_finite() && triangles.iter().flatten().any(|t| t.contains(points[i])) {
                water += (wet[i] - 0.004).max(0.0);
                if wet[i] > 0.004 {
                    wet[i] = WIPED_FILM; // a wet blade leaves a brief, thin residual film
                }
            }
        }
    }
    if water > 0.0 {
        if let Some(plane) = triangles.iter().flatten().next() {
            push_water(wet, points, bounds, previous, current, plane.normal, water);
        }
    }
    water > 0.0
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
    const WIDTH: f32 = 0.018;
    let edge = current[1] - current[0];
    let length2 = edge.length_squared();
    if length2 < 1e-8 {
        return;
    }
    let pixels = length2.sqrt() * WIDTH * (SIZE * SIZE) as f32 * bounds[2].abs() * bounds[3] * 0.5;
    let amount = water / pixels.max(1.0);
    let pixel = |p: Vec3| pane_pixel(p, bounds);
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

/// Conservative forward transport of mobile water. The grid stores pinned beads
/// below one; only the surplus moves. Water falling off the pane leaves the system.
fn drain_water(
    wet: &mut [f32],
    points: &[Vec3],
    bounds: [f32; 4],
    velocity: Vec3,
    dt: f32,
    scratch: &mut [f32],
) {
    let step = (pane_project(velocity, bounds[2] < 0.0)
        * Vec2::new(bounds[2].abs(), bounds[3])
        * (SIZE as f32 * dt))
        .clamp(Vec2::splat(-0.95), Vec2::splat(0.95));
    if step.length_squared() < 1e-8 {
        return;
    }
    for (dst, &src) in scratch.iter_mut().zip(wet.iter()) {
        *dst = src.min(1.0);
    }
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = y * SIZE + x;
            let water = (wet[i] - 1.0).max(0.0);
            if water == 0.0 || !points[i].is_finite() {
                continue;
            }
            let destination = Vec2::new(x as f32, y as f32) + step;
            let cell = destination.floor();
            let fraction = destination - cell;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (nx, ny) = (cell.x as i32 + dx, cell.y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= SIZE as i32 || ny >= SIZE as i32 {
                        continue;
                    }
                    let j = ny as usize * SIZE + nx as usize;
                    if !points[j].is_finite()
                        || (pane_depth(points[j], bounds) - pane_depth(points[i], bounds)).abs()
                            > 0.1
                    {
                        continue;
                    }
                    let wx = if dx == 0 {
                        1.0 - fraction.x
                    } else {
                        fraction.x
                    };
                    let wy = if dy == 0 {
                        1.0 - fraction.y
                    } else {
                        fraction.y
                    };
                    scratch[j] += water * wx * wy;
                }
            }
        }
    }
    for (dst, &src) in wet.iter_mut().zip(scratch.iter()) {
        *dst = src.min(2.0);
    }
}

struct SweepTriangle {
    origin: Vec3,
    normal: Vec3,
    u: Vec3,
    v: Vec3,
    depth: Vec3,
}

impl SweepTriangle {
    fn new(a: Vec3, b: Vec3, c: Vec3, side: bool) -> Option<Self> {
        let (ab, ac) = (b - a, c - a);
        let n = ab.cross(ac);
        let det = n.length_squared();
        if det < 1e-12 {
            return None;
        }
        let axis = if side { Vec3::X } else { Vec3::Y };
        let facing = axis.dot(n);
        if facing.abs() < 1e-8 {
            return None;
        }
        let depth = n / facing;
        let u = ac.cross(n) / det;
        let v = n.cross(ab) / det;
        Some(Self {
            origin: a,
            normal: n.normalize(),
            u: u - depth * u.dot(axis),
            v: v - depth * v.dot(axis),
            depth,
        })
    }

    fn coordinates(&self, p: Vec3) -> Option<Vec2> {
        let d = p - self.origin;
        let offset = d.dot(self.depth);
        if offset.abs() > 0.12 {
            return None;
        }
        // Project along the pane depth, not the slanted sweep's normal: an
        // offset rain mesh must keep the same blade endpoints in the glass plane.
        Some(Vec2::new(d.dot(self.u), d.dot(self.v)))
    }

    fn contains(&self, p: Vec3) -> bool {
        self.coordinates(p)
            .is_some_and(|uv| uv.min_element() >= -1e-5 && uv.element_sum() <= 1.00001)
    }

    fn crosses(&self, a: Vec3, b: Vec3) -> bool {
        let (Some(a), Some(b)) = (self.coordinates(a), self.coordinates(b)) else {
            return false;
        };
        let mut enter: f32 = 0.0;
        let mut leave: f32 = 1.0;
        for (start, end) in [
            (a.x, b.x),
            (a.y, b.y),
            (1.0 - a.element_sum(), 1.0 - b.element_sum()),
        ] {
            if start < 0.0 && end < 0.0 {
                return false;
            }
            if start < 0.0 {
                enter = enter.max(start / (start - end));
            }
            if end < 0.0 {
                leave = leave.min(start / (start - end));
            }
        }
        enter <= leave
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_glass_keeps_projected_endpoints_and_catches_crossing_runoff() {
        let sweep = SweepTriangle::new(
            Vec3::ZERO,
            Vec3::new(0.0, 0.04, 1.0),
            Vec3::new(0.1, 0.04, 1.0),
            false,
        )
        .unwrap();
        assert!(sweep.contains(Vec3::new(0.045, 0.1, 0.5)));
        assert!(!sweep.contains(Vec3::new(0.09, 0.1, 0.5)));
        assert!(sweep.crosses(Vec3::new(0.045, 0.1, 1.1), Vec3::new(0.045, 0.1, 0.2)));
        assert!(!sweep.crosses(Vec3::new(0.2, 0.1, 1.1), Vec3::new(0.2, 0.1, 0.2)));
        assert!(!sweep.contains(Vec3::new(0.045, 0.4, 0.5)));
    }

    #[test]
    fn a_wipe_leaves_brief_sheen_then_rewets_gradually_without_a_frame_rate_dependency() {
        assert!(advance_wetness(WIPED_FILM, 0.1, 0.1) < 0.0);
        let after_two_seconds = advance_wetness(WIPED_FILM, 0.1, 2.0);
        assert!(after_two_seconds > 0.1 && after_two_seconds < 0.2);
        let mut stepped = WIPED_FILM;
        for _ in 0..120 {
            stepped = advance_wetness(stepped, 0.1, 1.0 / 60.0);
        }
        assert!((stepped - after_two_seconds).abs() < 1e-5);
        assert_eq!(advance_wetness(0.0, -0.025, 2.0), 0.0);
        assert_eq!(advance_wetness(WIPED_FILM, -0.025, 2.0), 0.0);
        assert_eq!(encode_wetness(0.0), 8);
        assert!(encode_wetness(WIPED_FILM) < 8);
        assert_eq!(encode_wetness(2.0), 255);
    }

    #[test]
    fn collected_water_moves_down_or_with_airflow_without_moving_pinned_beads() {
        let points: Vec<_> = (0..SIZE * SIZE)
            .map(|i| {
                Vec3::new(
                    (i % SIZE) as f32 / SIZE as f32,
                    0.0,
                    (i / SIZE) as f32 / SIZE as f32,
                )
            })
            .collect();
        let source = SIZE * 64 + 64;
        let mut scratch = vec![0.0; SIZE * SIZE];
        for (velocity, neighbour) in [(-Vec3::Z, source - SIZE), (Vec3::X, source + 1)] {
            let mut wet = vec![0.5; SIZE * SIZE];
            wet[source] = 1.6;
            let before = wet.iter().sum::<f32>();
            drain_water(
                &mut wet,
                &points,
                [0.0, 0.0, 1.0, 1.0],
                velocity,
                0.5 / SIZE as f32,
                &mut scratch,
            );
            assert!((wet[source] - 1.3).abs() < 1e-5);
            assert!((wet[neighbour] - 0.8).abs() < 1e-5);
            assert!((wet.iter().sum::<f32>() - before).abs() < 0.01);
        }
        let mut wet = vec![0.5; SIZE * SIZE];
        drain_water(
            &mut wet,
            &points,
            [0.0, 0.0, 1.0, 1.0],
            -Vec3::Z,
            10.0,
            &mut scratch,
        );
        assert!(wet.iter().all(|w| *w == 0.5));
        wet[0] = 1.6;
        let before = wet.iter().sum::<f32>();
        drain_water(
            &mut wet,
            &points,
            [0.0, 0.0, 1.0, 1.0],
            -Vec3::Z,
            10.0,
            &mut scratch,
        );
        assert!(wet.iter().sum::<f32>() < before);
        assert!(wet.iter().all(|w| w.is_finite() && *w >= 0.0 && *w <= 2.0));
    }

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
            assert!(wipe(&mut wet, &points, bounds, from, to));
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
        assert!(!wipe(&mut dry, &points, bounds, start, end));
        assert!(dry.iter().all(|w| *w == 0.0));
        let mut wet = vec![0.6; SIZE * SIZE];
        assert!(!wipe(&mut wet, &points, bounds, start, start));
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
    fn side_panes_use_longitudinal_coordinates_and_keep_the_map_in_the_cab() {
        let data = MeshData {
            positions: vec![
                Vec3::ZERO,
                Vec3::Y * 10.0,
                Vec3::Z,
                Vec3::Y * 10.0 + Vec3::Z,
            ],
            indices: vec![0, 1, 2, 1, 3, 2],
            ranges: vec![(0, 6, 0)],
            ..Default::default()
        };
        let (points, bounds) = film_points_in_cab(&data, 0, Some(Vec3::Y * 8.0)).unwrap();
        assert!(bounds[2] < 0.0);
        assert!(points
            .iter()
            .all(|p| p.is_finite() && p.y > 6.2 && p.y < 9.8));
        let p = points[SIZE * SIZE / 2];
        assert_eq!(pane_depth(p, bounds), 0.0);
        assert!((pane_pixel(p, bounds).y - (SIZE / 2) as f32 - 0.5).abs() < 1e-4);
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
