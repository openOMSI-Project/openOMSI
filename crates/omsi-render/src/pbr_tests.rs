use super::*;

fn image(rgba: [u8; 4]) -> omsi_texture::Image {
    omsi_texture::Image {
        width: 1,
        height: 1,
        rgba: rgba.to_vec(),
        has_alpha: true,
    }
}

#[test]
#[ignore = "requires a graphics adapter; renders authored road and terrain PBR maps"]
fn authored_pbr_shades_roads_and_terrain_without_changing_vanilla_or_coverage() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            ssao: false,
            shadow_size: 1024,
            fxaa: false,
            render_scale: 1.0,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let camera = Camera {
        position: DVec3::new(0.0, -0.105, 6.0),
        yaw: 0.0,
        pitch: -89.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    let pixel = |rgba: &[u8], x: usize| -> [u8; 3] {
        rgba[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 3]
            .try_into()
            .unwrap()
    };
    for kind in ["road", "terrain", "painted"] {
        for map in ["normal", "height", "roughness"] {
            let mut scene = renderer.new_scene();
            let plain = renderer.add_texture(&mut scene, &image([100, 100, 100, 255]), false);
            let authored = renderer.add_texture(&mut scene, &image([100, 100, 100, 255]), false);
            let mask = renderer.add_texture(&mut scene, &image([255; 4]), false);
            let empty = renderer.add_texture(&mut scene, &image([0; 4]), false);
            renderer.add_pbr_maps(
                &mut scene,
                authored,
                &omsi_texture::pbr::PbrImages {
                    normal: (map != "roughness").then(|| image([64, 128, 238, 255])),
                    orm: (map == "roughness").then(|| image([255, 15, 0, 255])),
                    flags: if map != "roughness" {
                        [if map == "height" { 3.0 } else { 1.0 }, 0.0, 0.0, 0.0]
                    } else {
                        [0.0, 0.0, 1.0, 0.0]
                    },
                },
            );
            let material = |renderer: &Renderer, scene: &mut Scene, texture, mask| match kind {
                "terrain" => renderer.add_terrain_material(
                    scene,
                    Some(texture),
                    Some(mask),
                    None,
                    2.0,
                    None,
                    0.0,
                ),
                "painted" => renderer.add_terrain_layer_material(
                    scene,
                    Some(texture),
                    mask,
                    None,
                    2.0,
                    None,
                    0.0,
                ),
                _ => {
                    renderer.add_material(scene, Some(texture), AlphaMode::Opaque, [1.0; 4], false)
                }
            };
            let left = material(&renderer, &mut scene, authored, mask);
            let right = material(&renderer, &mut scene, plain, mask);
            let cut = material(&renderer, &mut scene, authored, empty);
            let mut quad = |left: f32, right: f32, material| {
                let mesh = renderer.add_mesh(
                    &mut scene,
                    &MeshData {
                        positions: vec![
                            Vec3::new(left, -5.0, 0.0),
                            Vec3::new(right, -5.0, 0.0),
                            Vec3::new(right, 5.0, 0.0),
                            Vec3::new(left, 5.0, 0.0),
                        ],
                        normals: vec![Vec3::Z; 4],
                        uvs: vec![
                            glam::Vec2::ZERO,
                            glam::Vec2::X,
                            glam::Vec2::ONE,
                            glam::Vec2::Y,
                        ],
                        indices: vec![0, 1, 2, 0, 2, 3],
                        ranges: vec![(0, 6, 0)],
                        one_sided: false,
                    },
                );
                let id = renderer.add_instance(
                    &mut scene,
                    mesh,
                    DVec3::ZERO,
                    Mat4::IDENTITY,
                    vec![material],
                );
                scene.instances[id].render_phase = if kind == "road" {
                    RenderPhase::Spline
                } else {
                    RenderPhase::Terrain
                };
                id
            };
            let left_instance = quad(-5.0, -0.5, left);
            quad(0.5, 5.0, right);
            let lighting = Lighting {
                sun_dir: if map != "roughness" {
                    Vec3::new(0.866, 0.0, 0.5)
                } else {
                    Vec3::new(-0.435, 0.0, 0.9).normalize()
                },
                sun_intensity: 1.0,
                shadows: false,
                detail: false,
                fog_density: 0.0,
                ..Default::default()
            };
            // Use the same mesh and pixel for the negative control. Separate left/right
            // pixels have different view vectors and can differ under GGX even without
            // a map, especially near the narrow highlight in the roughness case.
            scene.instances[left_instance].materials = vec![right];
            let vanilla_plain = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &lighting)
                .unwrap();
            let enhanced = Lighting {
                enhanced: true,
                ..lighting.clone()
            };
            let enhanced_plain = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &enhanced)
                .unwrap();
            scene.instances[left_instance].materials = vec![left];
            let vanilla_mapped = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &lighting)
                .unwrap();
            let (a, b) = (pixel(&vanilla_mapped, 16), pixel(&vanilla_plain, 16));
            assert!(
                (0..3).all(|i| a[i].abs_diff(b[i]) <= 3),
                "Vanilla {kind}/{map}: {a:?} != {b:?}"
            );
            let enhanced_mapped = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &enhanced)
                .unwrap();
            let (a, b) = (pixel(&enhanced_mapped, 16), pixel(&enhanced_plain, 16));
            assert!(
                (0..3).any(|i| a[i].abs_diff(b[i]) >= 8),
                "Enhanced ignored {kind}/{map}: {a:?} vs {b:?}"
            );
            if kind != "road" {
                // An empty terrain brush/cut mask removes the authored material too.
                // Compare with the actual sky behind it, not a guessed background colour.
                scene.instances[left_instance].materials = vec![cut];
                let cut_picture = renderer
                    .render_to_image(&mut scene, 64, 64, &camera, &enhanced)
                    .unwrap();
                scene.instances[left_instance].visible = false;
                let absent_picture = renderer
                    .render_to_image(&mut scene, 64, 64, &camera, &enhanced)
                    .unwrap();
                let (a, b) = (pixel(&cut_picture, 16), pixel(&absent_picture, 16));
                assert!(
                    (0..3).all(|i| a[i].abs_diff(b[i]) <= 3),
                    "PBR changed {kind} coverage: {a:?} != {b:?}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a graphics adapter; checks physical height normals on rectangular repeats"]
fn height_normals_preserve_both_physical_axes_and_legacy_normal_conventions() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            ssao: false,
            shadow_size: 1024,
            fxaa: false,
            render_scale: 1.0,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let camera = Camera {
        position: DVec3::new(0.0, -0.035, 2.0),
        yaw: 0.0,
        pitch: -89.0,
        roll: 0.0,
        fov_deg: 65.0,
        near: 0.1,
        far: 100.0,
    };
    let lighting = Lighting {
        enhanced: true,
        sun_dir: Vec3::new(-0.5, -0.5, 0.7).normalize(),
        sun_intensity: 1.0,
        shadows: false,
        detail: false,
        fog_density: 0.0,
        ..Default::default()
    };
    for (flag, axis, mirror) in [
        (1.0, 0, 0),
        (1.0, 1, 0),
        (2.0, 0, 0),
        (2.0, 1, 0),
        (3.0, 0, 0),
        (3.0, 1, 0),
        (3.0, 2, 0),
        (3.0, 0, 1),
        (3.0, 1, 2),
    ] {
        let mut scene = renderer.new_scene();
        let plain = renderer.add_texture(&mut scene, &image([100, 100, 100, 255]), false);
        let authored = renderer.add_texture(&mut scene, &image([100, 100, 100, 255]), false);
        let rgba = if axis == 1 {
            [128, 192, 238, 255]
        } else {
            [192, 128, 238, 255]
        };
        renderer.add_pbr_maps(
            &mut scene,
            authored,
            &omsi_texture::pbr::PbrImages {
                normal: Some(image(rgba)),
                orm: None,
                flags: [flag, 0.0, 0.0, 0.0],
            },
        );
        let material = |scene: &mut Scene, texture| {
            renderer.add_terrain_material(scene, Some(texture), None, None, 1.0, None, 0.0)
        };
        let plain_material = material(&mut scene, plain);
        let authored_material = material(&mut scene, authored);
        // One repeat covers 2 x 4 metres. Compare the mapped result with the same
        // surface carrying the analytic world normal, through the complete shader.
        let mut expected =
            Vec3::new(rgba[0] as f32, rgba[1] as f32, rgba[2] as f32) / 255.0 * 2.0 - Vec3::ONE;
        if flag < 2.5 {
            // Preserve the existing DX/GL convention. With this downward camera the
            // legacy cofactors point along -X/-Y, and the shared scale halves V.
            expected.x = -expected.x;
            expected.y *= if flag > 1.5 { 0.5 } else { -0.5 };
        } else {
            // Physical slopes follow increasing U/V in the surface, independent of
            // screen orientation. Mirroring one UV axis reverses only that direction.
            if mirror == 1 {
                expected.x = -expected.x;
            }
            if mirror == 2 {
                expected.y = -expected.y;
            }
        }
        expected = if axis == 2 {
            Vec3::Z
        } else {
            expected.normalize()
        };
        let data = |normal| MeshData {
            positions: vec![
                Vec3::new(-1.0, -2.0, 0.0),
                Vec3::new(1.0, -2.0, 0.0),
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(-1.0, 2.0, 0.0),
            ],
            normals: vec![normal; 4],
            uvs: if axis == 2 {
                // One collapsed UV axis cannot define a physical tangent frame.
                vec![
                    glam::Vec2::ZERO,
                    glam::Vec2::X,
                    glam::Vec2::X,
                    glam::Vec2::ZERO,
                ]
            } else {
                [
                    glam::Vec2::ZERO,
                    glam::Vec2::X,
                    glam::Vec2::ONE,
                    glam::Vec2::Y,
                ]
                .into_iter()
                .map(|uv| match mirror {
                    1 => glam::Vec2::new(1.0 - uv.x, uv.y),
                    2 => glam::Vec2::new(uv.x, 1.0 - uv.y),
                    _ => uv,
                })
                .collect()
            },
            indices: vec![0, 1, 2, 0, 2, 3],
            ranges: vec![(0, 6, 0)],
            one_sided: false,
        };
        let authored_mesh = renderer.add_mesh(&mut scene, &data(Vec3::Z));
        let reference_mesh = renderer.add_mesh(&mut scene, &data(expected));
        let id = renderer.add_instance(
            &mut scene,
            authored_mesh,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![authored_material],
        );
        scene.instances[id].render_phase = RenderPhase::Terrain;
        let actual = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        scene.instances[id].mesh = reference_mesh;
        scene.instances[id].materials = vec![plain_material];
        let reference = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        let offset = (32 * 64 + 32) * 4;
        let (a, b) = (&actual[offset..offset + 3], &reference[offset..offset + 3]);
        assert!(
            (0..3).all(|i| a[i].abs_diff(b[i]) <= 3),
            "flag {flag}, axis {axis}, mirror {mirror}: {a:?} != analytic {b:?}"
        );
    }
}
