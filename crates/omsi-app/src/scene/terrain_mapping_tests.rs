//! Exercise the production tile upload with a painted, cut terrain and mapped
//! surfaces. No installed map content or live game state is needed.
use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "openomsi-terrain-mapping-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(path), Ok(root)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize()) {
            if path.starts_with(&root) && path != root {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

fn triangle(z: f32) -> MeshData {
    MeshData {
        positions: vec![
            glam::Vec3::new(0.0, 0.0, z),
            glam::Vec3::new(10.0, 0.0, z),
            glam::Vec3::new(0.0, 10.0, z),
        ],
        normals: vec![glam::Vec3::Z; 3],
        uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2],
        ranges: vec![(0, 3, 0)],
        ..Default::default()
    }
}

#[test]
#[ignore = "requires a graphics adapter; uploads an isolated synthetic tile"]
fn mapped_splines_and_objects_use_uncut_base_while_ground_keeps_paint() {
    let fixture = Fixture::new();
    fixture.write("global.cfg", "[name]\nTerrain mapping regression\n");
    fixture.write("mapped.png.cfg", "[terrainmapping]\n");
    fixture.write(
        "mapped.sco",
        "[groups]\n1\nRegression\n[friendlyname]\nMapped island\n[mesh]\nisland.x\n",
    );
    fixture.write("island.x", r#"xof 0303txt 0032
        Mesh island {
            3; 0;0;2;, 10;0;2;, 0;10;2;;
            1; 3;0,1,2;;
            MeshTextureCoords {3;0;0;,1;0;,0;1;;}
            MeshMaterialList {1;1;0;; Material {1;1;1;1;;0;0;0;0;;0;0;0;; TextureFilename {"mapped.png";} }}
        }
    "#);
    for (name, color) in [
        ("base.png", [0, 255, 0, 255]),
        ("paint.png", [255, 0, 0, 255]),
        ("mapped.png", [0, 0, 255, 255]),
    ] {
        image::save_buffer(fixture.0.join(name), &color, 1, 1, image::ColorType::Rgba8).unwrap();
    }
    let mut world = World::open(&fixture.0, &fixture.0.join("global.cfg"), 20261001).unwrap();
    world.global.ground_textures = ["base.png", "paint.png"]
        .into_iter()
        .map(|texture| omsi_map::global::GroundTex {
            texture: texture.into(),
            params: [8.0, 4.0, 8.0],
            ..Default::default()
        })
        .collect();
    let object = world
        .object_type("mapped.sco")
        .expect("synthetic scenery must load");
    assert_eq!(object.meshes.len(), 1);
    let spline = Arc::new(SplineType {
        dir: fixture.0.clone(),
        surf: Vec::new(),
        surface: Vec::new(),
        def: Spline {
            path: fixture.0.join("mapped.sli"),
            textures: vec![omsi_scenery::sli::SplineTexture {
                file: "mapped.png".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    let instance = crate::graphics_instance();
    let renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        omsi_render::RenderOptions {
            msaa: 1,
            shadow_size: 1024,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let prepared = Prepared {
        tx: 0,
        ty: 0,
        terrain: Some(triangle(0.0)),
        hole_walls: triangle(-0.5),
        paint_masks: Vec::new(),
        paint: vec![(1, TextureData::from_image(Image::solid([255; 4])), 1.0)],
        wall_paint: vec![(1, TextureData::from_image(Image::solid([255; 4])))],
        water: None,
        // Both upload paths must agree: a spatially batched mapped spline and one
        // left in the per-spline path (OMSI_NO_GROUND_SPLINE_BATCHING).
        ground_splines: batch_ground_splines(vec![Arc::new(triangle(0.5))]),
        splines: vec![(Arc::new(triangle(1.0)), spline, false, DVec3::ZERO)],
        objects: vec![PlacedObject {
            ot: object,
            pos: DVec3::ZERO,
            xf: Mat4::IDENTITY,
            lamp: None,
            map_id: 1,
            key: 1,
            controller: None,
            strings: Vec::new(),
            warped: None,
            var_parent: None,
            parked: false,
            editable: false,
            script: None,
        }],
        trees: Vec::new(),
        origin: DVec3::ZERO,
        light_map: Some(TextureData::from_image(Image::solid([80, 70, 60, 255]))),
        cut: Some(TextureData::from_image(Image::solid([255, 255, 255, 0]))),
        images: Arc::new(HashMap::new()),
    };
    let mut upload = world.begin_upload(prepared);
    while !world.upload_step(&renderer, &mut scene, &mut upload, None) {}
    while !world.place_step(&renderer, &mut scene, &mut upload, None) {}
    let base_texture = world.gpu.lock().ground.as_ref().unwrap().ground_id.unwrap();
    let terrain: Vec<_> = scene
        .instances
        .iter()
        .filter(|i| i.render_phase == RenderPhase::Terrain)
        .collect();
    assert_eq!(
        terrain.len(),
        4,
        "the ground and its exposed sides retain their base and painted layers"
    );
    let ground_base = &scene.materials[terrain[0].materials[0]];
    assert_eq!(ground_base.texture, Some(base_texture));
    assert!(
        ground_base.transmap.is_some(),
        "the actual ground retains its road cut"
    );
    let wall_base = &scene.materials[terrain[1].materials[0]];
    assert_eq!(wall_base.texture, Some(base_texture));
    assert_eq!(
        wall_base.transmap, None,
        "the hole mask must not erase its sides"
    );
    assert_eq!(wall_base.nightmap, ground_base.nightmap);
    let paint = &scene.materials[terrain[2].materials[0]];
    assert_ne!(paint.texture, Some(base_texture));
    assert!(terrain[2].ground_layer);
    let wall_paint = &scene.materials[terrain[3].materials[0]];
    assert_eq!(wall_paint.texture, paint.texture);
    assert!(terrain[3].ground_layer);
    assert_ne!(
        wall_paint.transmap, paint.transmap,
        "wall brush masks stay uncut"
    );

    let mapped: Vec<_> = scene
        .instances
        .iter()
        .filter(|i| {
            i.render_phase != RenderPhase::Terrain && !scene.meshes[i.mesh].ranges.is_empty()
        })
        .collect();
    assert_eq!(
        mapped.len(),
        3,
        "exactly one base-ground draw for each mapped spline/object, with no painted overlays"
    );
    assert_eq!(
        mapped
            .iter()
            .filter(|i| i.render_phase == RenderPhase::Spline)
            .count(),
        2
    );
    for instance in mapped {
        assert_eq!(instance.materials.len(), 1);
        let material = &scene.materials[instance.materials[0]];
        assert_eq!(material.texture, Some(base_texture));
        assert_eq!(
            material.transmap, None,
            "road holes must not cut mapped geometry"
        );
        assert_eq!(
            material.nightmap, ground_base.nightmap,
            "retain the tile's illumination"
        );
        assert!(material.nightmap.is_some());
        assert!(!instance.ground_layer);
    }
}

/// #954: a route arrow the map's author put up (a `[helparrow]` object) is placed with its
/// tile but drawn only while OMSI 2's route arrows are on, and goes with its tile.
#[test]
#[ignore = "requires a graphics adapter; uploads an isolated synthetic tile"]
fn a_maps_own_route_arrow_shows_only_with_the_route_arrows() {
    let fixture = Fixture::new();
    fixture.write("global.cfg", "[name]\nRoute arrow regression\n");
    fixture.write(
        "arrow.sco",
        "[groups]\n1\nUtilities\n[friendlyname]\nRoute Arrow Left\n[helparrow]\n[nocollision]\n[mesh]\narrow.x\n",
    );
    fixture.write("arrow.x", r#"xof 0303txt 0032
        Mesh arrow {
            3; 0;0;2;, 1;0;2;, 0;1;2;;
            1; 3;0,1,2;;
            MeshMaterialList {1;1;0;; Material {1;1;1;1;;0;0;0;0;;0;0;0;;}}
        }
    "#);
    let world = World::open(&fixture.0, &fixture.0.join("global.cfg"), 20261001).unwrap();
    let object = world.object_type("arrow.sco").expect("synthetic route arrow must load");
    assert!(object.sco.is_help_arrow);
    let instance = crate::graphics_instance();
    let renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        omsi_render::RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let prepared = Prepared {
        tx: 0,
        ty: 0,
        terrain: Some(triangle(0.0)),
        hole_walls: MeshData::default(),
        paint_masks: Vec::new(),
        paint: Vec::new(),
        wall_paint: Vec::new(),
        water: None,
        ground_splines: Vec::new(),
        splines: Vec::new(),
        objects: vec![PlacedObject {
            ot: object,
            pos: DVec3::new(5.0, 5.0, 0.0),
            xf: Mat4::IDENTITY,
            lamp: None,
            map_id: 1,
            key: 1,
            controller: None,
            strings: vec!["Krankenhs.".into()],
            warped: None,
            var_parent: None,
            parked: false,
            editable: false,
            script: None,
        }],
        trees: Vec::new(),
        origin: DVec3::ZERO,
        light_map: None,
        cut: None,
        images: Arc::new(HashMap::new()),
    };
    let mut upload = world.begin_upload(prepared);
    while !world.upload_step(&renderer, &mut scene, &mut upload, None) {}
    while !world.place_step(&renderer, &mut scene, &mut upload, None) {}
    world.commit_upload(upload, &mut LoadStats::default());
    let arrows = world.help_arrows.lock().get(&(0, 0)).cloned().unwrap_or_default();
    assert!(!arrows.is_empty(), "the arrow is put up with its tile");
    let shown = |scene: &Scene| arrows.iter().map(|i| scene.instances[*i].visible).collect::<Vec<_>>();
    assert!(shown(&scene).iter().all(|v| !v), "hidden while the route arrows are off");
    assert!(arrows.iter().all(|i| !scene.instances[*i].casts_shadow));
    world.show_help_arrows(&renderer, &mut scene, true);
    assert!(shown(&scene).iter().all(|v| *v), "drawn once they are switched on");
    world.show_help_arrows(&renderer, &mut scene, false);
    assert!(shown(&scene).iter().all(|v| !v), "hidden again once they are off");
    world.unload_tile(&renderer, &mut scene, (0, 0), None);
    assert!(world.help_arrows.lock().is_empty(), "the tile's arrows go with it");
}
