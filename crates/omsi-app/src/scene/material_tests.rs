use super::*;

#[test]
fn null_texture_names() {
    assert!(is_null_texture("null.bmp"));
    assert!(is_null_texture(" NULL.BMP "));
    assert!(is_null_texture("texture\\null.tga"));
    assert!(is_null_texture(""));
    assert!(!is_null_texture("D_Matrix.bmp"));
    assert!(!is_null_texture("nullschild.bmp"));
}

#[test]
fn d3d_material_colours() {
    // a Blender export: grey diffuse with alpha 0, a bogus specular, no emissive
    let m = omsi_o3d::Material {
        diffuse: [0.64, 0.64, 0.64, 0.0],
        specular: [255.0, 255.0, 355.0],
        emissive: [0.0; 3],
        specular_power: 96.0,
        texture: "int_glass.tga".into(),
    };
    let (color, emissive, specular, ambient) = d3d_material(&m, None, true);
    // textured: the texture's alpha alone counts
    assert_eq!(color, [0.64, 0.64, 0.64, 1.0]);
    assert_eq!(emissive, [0.0; 3]);
    assert_eq!(specular, [1.0, 1.0, 1.0, 96.0]);
    // Omsi.exe's o3d slot: a white ambient, whatever the diffuse colour (0x7c62f8)
    assert_eq!(ambient, [1.0; 3]);
    // untextured: the material's alpha
    assert_eq!(d3d_material(&m, None, false).0[3], 0.0);
    // no specular colour, no highlight whatever the power
    let plain = omsi_o3d::Material {
        specular_power: 25.0,
        ..Default::default()
    };
    assert_eq!(d3d_material(&plain, None, true).2[3], 0.0);
    // a lit display: emissive white
    let lcd = omsi_o3d::Material {
        emissive: [1.0; 3],
        ..Default::default()
    };
    assert_eq!(d3d_material(&lcd, None, true).1, [1.0; 3]);
    // [matl_allcolor] replaces the o3d material (the stock lower-deck lighting item)
    let all = [
        1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.24, 0.23, 0.2, 0.0,
    ];
    let (c, e, s, _) = d3d_material(&m, Some(all), true);
    assert_eq!(c, [1.0; 4]);
    // [matl_allcolor]'s own ambient
    let mut dim = all;
    dim[4..7].copy_from_slice(&[0.2, 0.3, 0.4]);
    assert_eq!(d3d_material(&m, Some(dim), true).3, [0.2, 0.3, 0.4]);
    assert_eq!(e, [0.24, 0.23, 0.2]);
    assert_eq!(s[3], 0.0);
}

/// A second `[matl]` of the same slot with `[matl_alpha] 1` makes the slot
/// alpha-tested; a later `[matl_alpha] 0` makes it opaque again,
/// and a later `[matl]` without one keeps the mode.
#[test]
fn later_matl_of_the_same_slot_sets_its_alpha() {
    let mats = [omsi_o3d::Material { texture: "Chain.dds".into(), ..Default::default() }];
    let def = |alpha: Option<i32>| MaterialDef {
        texture: "chain.dds".into(),
        alpha: alpha.unwrap_or(0),
        alpha_set: alpha.is_some(),
        ..Default::default()
    };
    assert_eq!(material_alpha(&mats, 0, &[def(None), def(Some(1))]), AlphaMode::Test);
    assert_eq!(material_alpha(&mats, 0, &[def(Some(1)), def(None)]), AlphaMode::Test);
    assert_eq!(material_alpha(&mats, 0, &[def(Some(2)), def(Some(0))]), AlphaMode::Opaque);
    assert_eq!(material_alpha(&mats, 0, &[def(None), def(None)]), AlphaMode::Opaque);
}

#[test]
fn transmap_mask_does_not_make_opaque_body_blend() {
    let mats = [omsi_o3d::Material {
        texture: "body.tga".into(),
        ..Default::default()
    }];
    let defs = [
        MaterialDef {
            texture: "body.tga".into(),
            index: 0,
            alpha: 0,
            transmap: Some("body_mask.tga".into()),
            ..Default::default()
        },
        MaterialDef {
            texture: "body.tga".into(),
            index: 0,
            alpha: 0,
            alphascale: Some("Rain_Window_Front_Wetness".into()),
            ..Default::default()
        },
    ];
    let alpha = material_alpha(&mats, 0, &defs);
    assert_eq!(alpha, AlphaMode::Opaque);
    assert_eq!(Renderer::clamp_slot_alpha(0.35, alpha, false), 1.0);
}

#[test]
fn blended_body_alpha_repair_does_not_touch_glass() {
    assert!(is_vehicle_body_material(
        "12m/wagenkasten_embl_eev.o3d",
        "01white_FL.tga",
        true,
        false,
        false,
        true
    ));
    assert!(is_vehicle_body_material(
        "A21_EEV/body.o3d",
        "a21_body.png",
        true,
        false,
        false,
        true
    ));
    assert!(is_vehicle_body_material(
        "Exterior/unnamed_shell.o3d",
        "paint.tga",
        true,
        false,
        false,
        true
    ));
    assert!(!is_vehicle_body_material(
        "A21/windows.o3d",
        "a21_body_windows.png",
        true,
        false,
        false,
        false
    ));
    assert!(!is_vehicle_body_material(
        "Exterior/unnamed_window.o3d",
        "glass.tga",
        true,
        false,
        false,
        true
    ));
    assert!(!is_vehicle_body_material(
        "12m/wagenkasten.o3d",
        "01white_FL.tga",
        true,
        true,
        false,
        true
    ));
    assert!(!is_vehicle_body_material(
        "12m/wagenkasten.o3d",
        "01white_FL.tga",
        true,
        false,
        true,
        true
    ));
}

/// The ICU400 controller's screen layer: a script texture as its transmap declares one.
#[test]
fn script_transmap_is_declared() {
    let text = "[mesh]\nscreen.o3d\n\n[matl]\nScreen.dds\n0\n[matl_transmap]\n\\S:1\n[alphascale]\nsignController_alphaScale\n[matl_alpha]\n2\n\n[matl]\nPlain.dds\n0\n";
    let m = omsi_model::Model::parse(&omsi_cfg::CfgFile::from_str("model.cfg", text));
    let mats = &m.meshes[0].materials;
    let screen = mats.iter().find(|d| d.texture == "Screen.dds").unwrap();
    let plain = mats.iter().find(|d| d.texture == "Plain.dds").unwrap();
    assert_eq!(screen.transmap.as_deref(), Some("\\S:1"));
    assert!(material_extra(&[screen], None, None, [0.0; 4]).transmap_declared);
    assert!(!material_extra(&[plain], None, None, [0.0; 4]).transmap_declared);
}

#[test]
fn material_extra_from_commands() {
    let glass = MaterialDef {
        texture: "wischwasser.tga".into(),
        alpha: 2,
        no_z_write: true,
        no_z_check: true,
        ..Default::default()
    };
    let decal = MaterialDef {
        texture: "bw_jul_mod5.bmp".into(),
        z_bias: 16,
        ..Default::default()
    };
    let e = material_extra(
        &[&glass, &decal],
        Some(7),
        Some((3, 0.1)),
        [0.2, 0.2, 0.2, 10.0],
    );
    assert!(e.no_z_write && !e.no_z_check);
    assert_eq!(e.z_bias, 16);
    assert_eq!(e.env_mask, Some(7));
    assert_eq!(e.specular, [0.2, 0.2, 0.2, 10.0]);
    assert_eq!(e.bump, Some((3, 0.1)));
    assert_eq!(
        material_extra(&[], None, None, [0.0; 4]),
        MaterialExtra::default()
    );
    // a factor of 0 moves nothing: no bump map to sample
    assert_eq!(
        material_extra(&[], None, Some((3, 0.0)), [0.0; 4]).bump,
        None
    );
    // [matl_texadress_border]: the colour in bytes, as 0..1
    let roller = MaterialDef {
        texture: "rlb_512.tga".into(),
        tex_address: omsi_model::TexAddress::Border,
        border_color: [255.0, 255.0, 255.0, 0.0],
        ..Default::default()
    };
    assert_eq!(
        material_extra(&[&roller], None, None, [0.0; 4]).border,
        Some([1.0, 1.0, 1.0, 0.0])
    );
    let clamped = MaterialDef {
        tex_address: omsi_model::TexAddress::Clamp,
        ..roller.clone()
    };
    assert_eq!(material_extra(&[&roller, &clamped], None, None, [0.0; 4]).border, None);
    // the slot's last addressing command decides how its textures repeat
    use omsi_render::TexAddressing as R;
    let mirror = MaterialDef { tex_address: omsi_model::TexAddress::Mirror, ..roller.clone() };
    let once = MaterialDef { tex_address: omsi_model::TexAddress::MirrorOnce, ..roller.clone() };
    let plain = MaterialDef::default();
    assert_eq!(tex_addressing([&plain].into_iter()), R::Wrap);
    assert_eq!(tex_addressing([&clamped, &mirror, &plain].into_iter()), R::Mirror);
    assert_eq!(tex_addressing([&mirror, &once].into_iter()), R::MirrorOnce);
    assert_eq!(tex_addressing([&once, &roller].into_iter()), R::Clamp);
}

#[test]
fn scenery_freetex_name_resolution() {
    let ov1 = MaterialDef {
        freetex: Some(("placeholder.bmp".into(), "Textur".into())),
        ..Default::default()
    };
    let ov2 = MaterialDef {
        freetex: Some(("placeholder2.bmp".into(), "Textur2".into())),
        ..Default::default()
    };
    let overrides = vec![ov1.clone(), ov2.clone()];

    // 1. Script variable takes precedence when available
    let mut prog = omsi_script::Program::default();
    prog.declare_str_var("Textur");
    let script = omsi_sim::scenery::SceneryInstance::new(
        Arc::new(prog),
        &[],
        omsi_sim::SimClock::default(),
        &["from_script.bmp".into()],
    );
    let from_strings = vec!["from_strings.bmp".to_string()];
    let name = resolve_scenery_freetex_name("Textur", &ov1, &overrides, Some(&script), None, &from_strings);
    assert_eq!(name, Some("from_script.bmp"));

    // 2. Fallback to strings by explicit numeric index (e.g. var = "1")
    let strings = vec!["zero.bmp".to_string(), "\"quoted_one.bmp\"".to_string()];
    let name = resolve_scenery_freetex_name("1", &ov1, &overrides, None, None, &strings);
    assert_eq!(name, Some("quoted_one.bmp"));

    // 3. Fallback to strings by freetex declaration order
    let name_first = resolve_scenery_freetex_name("Textur", &overrides[0], &overrides, None, None, &strings);
    assert_eq!(name_first, Some("zero.bmp"));
    let name_second = resolve_scenery_freetex_name("Textur2", &overrides[1], &overrides, None, None, &strings);
    assert_eq!(name_second, Some("quoted_one.bmp"));

    // 4. Returns None when no matching string exists
    let name_empty = resolve_scenery_freetex_name("Missing", &ov1, &overrides, None, None, &[]);
    assert_eq!(name_empty, None);

    // 5. A string variable the object declares owns the slot: an empty one means the
    //    slot has no picture of its own, never another variable's placement string.
    //    (A bus stop sign's route number stood in a `[matl_freetex]` slot this way,
    //    #1756.)
    let mut prog2 = omsi_script::Program::default();
    prog2.declare_str_var("Textur");
    let empty_script = omsi_sim::scenery::SceneryInstance::new(
        Arc::new(prog2),
        &[],
        omsi_sim::SimClock::default(),
        &[String::new()],
    );
    assert_eq!(
        resolve_scenery_freetex_name("Textur", &ov1, &overrides, Some(&empty_script), None, &strings),
        None
    );
    // a variable the object does not declare keeps the positional fallback
    assert_eq!(
        resolve_scenery_freetex_name("Textur2", &ov1, &overrides, Some(&empty_script), None, &strings),
        Some("zero.bmp")
    );
}

/// `[matl_glow] <texture> <value>`: the material is its own light - the strength the
/// shader reads is the .cfg value x0.25 (the `Led glow` setting's own levels), the mask is
/// bound in the light map's slot, and the last command of the slot wins, as the other
/// `[matl_*]` commands do. A `[matl_lightmap]` keeps the slot: no glow then.
#[test]
fn matl_glow_gives_a_material_its_own_light() {
    let plain = MaterialDef { texture: "body.dds".into(), ..Default::default() };
    assert_eq!(material_extra(&[&plain], None, None, [0.0; 4]).glow, 0.0);
    assert_eq!(glow_mask(&[&plain]), None);
    let lit = MaterialDef { texture: "body.dds".into(), glow: Some(("mask.png".into(), 20.0)), ..Default::default() };
    assert_eq!(material_extra(&[&lit], None, None, [0.0; 4]).glow, 5.0);
    assert_eq!(glow_mask(&[&lit]), Some("mask.png"));
    // 6 is the `Led glow` slider's own default level
    let softer = MaterialDef { texture: "body.dds".into(), glow: Some(("soft.png".into(), 6.0)), ..Default::default() };
    assert_eq!(material_extra(&[&lit, &softer], None, None, [0.0; 4]).glow, 1.5);
    assert_eq!(glow_mask(&[&lit, &softer]), Some("soft.png"));
    // a negative value lights nothing (it is not a light that can be subtracted)
    let odd = MaterialDef { texture: "body.dds".into(), glow: Some(("m.png".into(), -3.0)), ..Default::default() };
    assert_eq!(material_extra(&[&odd], None, None, [0.0; 4]).glow, 0.0);
    // the light map keeps its slot
    let mapped = MaterialDef { lightmap: Some(("l.bmp".into(), "lights".into())), ..lit.clone() };
    assert_eq!(material_extra(&[&mapped], None, None, [0.0; 4]).glow, 0.0);
    assert_eq!(glow_mask(&[&mapped]), None);
    assert!(own_light_slot(&[&mapped]) && own_light_slot(&[&lit]) && !own_light_slot(&[&plain]));
}

/// A `[matl_alpha] 2` slot's picture decides how the slot is drawn: nothing to blend by
/// (opaque), a cut-out (the parked HK buses' bodies, drawn alpha-tested so the object writes
/// depth), or real partial transparency (a soft shadow blob, a translucent side: a blend).
#[test]
fn a_pictures_alpha_decides_how_a_blend_is_drawn() {
    use super::staging::{picture_blend_fade, BlendFade};
    let dir = std::env::temp_dir().join(format!("omsi_blend_fade_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, alphas: &[u8]| {
        let mut b = vec![0u8; 18];
        b[2] = 2; // uncompressed true-colour
        b[12..14].copy_from_slice(&(alphas.len() as u16).to_le_bytes());
        b[14..16].copy_from_slice(&1u16.to_le_bytes());
        b[16] = 32;
        b[17] = 0x08 | 0x20; // 8 alpha bits, top-down
        for &a in alphas {
            b.extend_from_slice(&[10, 20, 30, a]);
        }
        let p = dir.join(name);
        std::fs::write(&p, b).unwrap();
        p
    };
    // 255 throughout: the GG2 signs' pictures, nothing to blend by
    assert_eq!(picture_blend_fade(&write("solid.tga", &[255; 64])), BlendFade::Solid);
    // a cut-out: clear and solid, with one antialiased outline texel (the buses: 0.3-0.5 %
    // of the texels are like it)
    let mut cut = vec![255u8; 64];
    cut[..4].fill(0);
    cut[4] = 128;
    assert_eq!(picture_blend_fade(&write("cutout.tga", &cut)), BlendFade::Cutout);
    // real partial transparency: glass, a feathered road border, a soft shadow blob
    assert_eq!(picture_blend_fade(&write("glass.tga", &[128; 64])), BlendFade::Partial);
    // a missing picture keeps the blend it declared
    assert_eq!(picture_blend_fade(&dir.join("absent.tga")), BlendFade::Partial);
    std::fs::remove_dir_all(dir).unwrap();
}
