//! A script-controlled light map must keep an instrument readable at night.
//! Synthetic textures exercise the real shader without shipping vehicle assets.

use glam::{DVec3, Mat4, Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_render::{AlphaMode, Camera, Lighting, RenderOptions, Renderer};

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn an_enabled_lightmap_keeps_its_texture_readable_at_night() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            ssao: false,
            shadow_size: 512,
            fxaa: false,
            render_scale: 1.0,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let mut texture = |rgba: Vec<u8>, width| {
        renderer.add_texture(
            &mut scene,
            &omsi_texture::Image {
                width,
                height: 1,
                rgba,
                has_alpha: false,
            },
            false,
        )
    };
    let diffuse = texture(vec![60, 180, 90, 255], 1);
    // Sample inside each region, away from the bilinear transition between them.
    let mask = texture(
        vec![
            255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255,
        ],
        4,
    );
    let material = renderer.add_material_lit(
        &mut scene,
        Some(diffuse),
        AlphaMode::Opaque,
        [1.0; 4],
        false,
        None,
        None,
        Some(mask),
    );
    let mesh = renderer.add_mesh(
        &mut scene,
        &MeshData {
            positions: vec![
                Vec3::new(-4.0, 4.0, -4.0),
                Vec3::new(4.0, 4.0, -4.0),
                Vec3::new(4.0, 4.0, 4.0),
                Vec3::new(-4.0, 4.0, 4.0),
            ],
            normals: vec![-Vec3::Y; 4],
            uvs: vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y],
            indices: vec![0, 1, 2, 0, 2, 3],
            ranges: vec![(0, 6, 0)],
            one_sided: false,
        },
    );
    let panel = renderer.add_instance(
        &mut scene,
        mesh,
        DVec3::ZERO,
        Mat4::IDENTITY,
        vec![material],
    );
    let camera = Camera {
        position: DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    let lighting = Lighting {
        enhanced: true,
        night: 1.0,
        sun_dir: -Vec3::Z,
        sun_intensity: 0.0,
        shadows: false,
        detail: false,
        fog_density: 0.0,
        ..Default::default()
    };
    let pixel = |rgba: &[u8], x: usize| -> [u8; 3] {
        rgba[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 3]
            .try_into()
            .unwrap()
    };
    renderer.set_slot_light(&mut scene, panel, &[0.0]);
    let off = renderer
        .render_to_image(&mut scene, 64, 64, &camera, &lighting)
        .unwrap();
    renderer.set_slot_light(&mut scene, panel, &[1.0]);
    let on = renderer
        .render_to_image(&mut scene, 64, 64, &camera, &lighting)
        .unwrap();
    let (dark, lit, masked) = (pixel(&off, 16), pixel(&on, 16), pixel(&on, 47));
    println!("backlight off={dark:?}, on={lit:?}, black mask={masked:?}");
    assert!(
        lit[1] >= 130,
        "the enabled backlight must remain readable: off={dark:?}, on={lit:?}"
    );
    assert!(
        lit[1] > dark[1].saturating_add(40),
        "switching on must brighten the panel: {dark:?} / {lit:?}"
    );
    assert!(
        lit[1] > lit[0].saturating_add(30),
        "the backlight must retain its green colour: {lit:?}"
    );
    assert!(
        masked
            .iter()
            .zip(pixel(&off, 47))
            .all(|(a, b)| a.abs_diff(b) <= 5),
        "a black light-map region must remain unlit: {masked:?}"
    );
}
