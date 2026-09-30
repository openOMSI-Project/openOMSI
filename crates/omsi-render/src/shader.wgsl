struct Camera {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    world_origin: vec4<f32>, // world-space origin removed from render coordinates
    sun_dir: vec4<f32>,      // xyz direction to the sun, w intensity
    ambient: vec4<f32>,      // rgb ambient (envir light C)
    fog: vec4<f32>,          // rgb, density
    sun_color: vec4<f32>,    // rgb (envir light A), w night factor 0..1 (nightmap strength)
    sky_color: vec4<f32>,    // rgb secondary light from above (envir light B), w 1 = vanilla (as OMSI 2), 0 = Vanilla+
    light_grid: vec4<f32>,   // x,y grid origin (render-origin relative), z cell size, w cells per side
    sky: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    clouds: vec4<f32>,
    light_view_proj: mat4x4<f32>,
    light_view_proj_far: mat4x4<f32>,
    shadow: vec4<f32>,       // x enabled, y texel size, z near cascade half range
    post: vec4<f32>,         // x enhanced, y time, zw sun ndc
    inside_a: vec4<f32>,     // player vehicle box: origin xyz, sin(heading)
    inside_b: vec4<f32>,     // cos(heading), half extents xyz
    inside_c: vec4<f32>,     // box centre offset xyz, w = 1 when there is a box
    flags: vec4<f32>,        // x detail texturing, y enhanced graphics, z never set (see fs_main's end), w close cascade half range
    light_view_proj_close: mat4x4<f32>,
    wind: vec4<f32>,         // the player's vehicle's velocity (m/s, world): the airstream on its glass
};

// 1 when the point lies inside the player's vehicle (its [boundingbox], shrunk a little so
// that the outer skin, the glass and the roof stay outside): weather stays out of the cab.
// The side/front/back shrink was a bare 0.12 m, which left an interior fixture close to the
// wall (a window sill, the handbrake by the windscreen) just outside the box, wearing the
// up-facing wet/snow coat meant for the body panel beside it (see the `outside` weight
// below). A previous fix WIDENED this margin to 0.6 m, reasoning by analogy with the falling-
// particle exclusion box in `rain.rs` - but that box works the other way round (its margin
// is added, growing the excluded region outward, away from the body) from this one (whose
// margin is subtracted, shrinking the "inside" region inward, toward the body): widening it
// here shrank the cabin's own "inside" zone and threw the *whole* cabin outside it, coating
// the entire interior in snow instead of just the ledge. Narrowed to 0.03 m instead - just
// enough that the true outer skin and glass stay outside, without giving up real interior
// floor space near the wall.
// A normal of no length (a mesh saved with zero normals) is upright instead of NaN:
// normalize(0) is NaN on Vulkan and Metal and the pixel went black.
fn safe_normal(v: vec3<f32>) -> vec3<f32> {
    let d = dot(v, v);
    if (d > 1e-20) {
        return v * inverseSqrt(d);
    }
    return vec3<f32>(0.0, 0.0, 1.0);
}

fn inside_vehicle(world: vec3<f32>) -> f32 {
    if (camera.inside_c.w < 0.5) {
        return 0.0;
    }
    let d = world - camera.inside_a.xyz;
    let sh = camera.inside_a.w;
    let ch = camera.inside_b.x;
    let x = d.x * ch - d.y * sh - camera.inside_c.x;
    let y = d.x * sh + d.y * ch - camera.inside_c.y;
    let z = d.z - camera.inside_c.z;
    let h = camera.inside_b.yzw;
    let in_x = abs(x) < h.x - 0.03;
    let in_y = abs(y) < h.y - 0.03;
    let in_z = z > -h.z - 0.6 && z < h.z - 0.03;
    return select(0.0, 1.0, in_x && in_y && in_z);
}

// 1 when the point belongs to the player's vehicle: its `[boundingbox]` grown a little,
// so that the skin and the glass the cab test (`inside_vehicle`) leaves out are in.
fn near_player_vehicle(world: vec3<f32>) -> f32 {
    if (camera.inside_c.w < 0.5) {
        return 0.0;
    }
    let d = world - camera.inside_a.xyz;
    let sh = camera.inside_a.w;
    let ch = camera.inside_b.x;
    let x = d.x * ch - d.y * sh - camera.inside_c.x;
    let y = d.x * sh + d.y * ch - camera.inside_c.y;
    let z = d.z - camera.inside_c.z;
    let h = camera.inside_b.yzw + vec3<f32>(0.25);
    return select(0.0, 1.0, abs(x) < h.x && abs(y) < h.y && abs(z) < h.z);
}

// How much of the way from the camera to `world` lies outside the player's vehicle: there is
// no fog in the cab. From the driver's seat the whole saloon (seats 2 m away, the doors 5 m
// away) took the fog of that distance and in thick ground fog the interior went milky, the
// door panes nearly opaque - outside the bus there was nothing of the kind. The box is the
// one `inside_vehicle` uses; the part of the ray inside it is taken off the fogged distance.
fn fog_distance(world: vec3<f32>) -> f32 {
    let full = distance(world, camera.cam_pos.xyz);
    if (camera.inside_c.w < 0.5) {
        return full;
    }
    let sh = camera.inside_a.w;
    let ch = camera.inside_b.x;
    let to_local = fn_box_local(camera.cam_pos.xyz);
    let p1 = fn_box_local(world);
    let d = p1 - to_local;
    let h = camera.inside_b.yzw;
    // slab test of the segment against the box (x, y the plan, z height)
    var t0 = 0.0;
    var t1 = 1.0;
    for (var k = 0; k < 3; k = k + 1) {
        let o = to_local[k];
        let dk = d[k];
        let hk = h[k];
        if (abs(dk) < 1e-6) {
            if (abs(o) > hk) {
                return full;
            }
        } else {
            let a = (-hk - o) / dk;
            let b = (hk - o) / dk;
            t0 = max(t0, min(a, b));
            t1 = min(t1, max(a, b));
        }
    }
    if (t1 <= t0) {
        return full;
    }
    return max(full * (1.0 - (t1 - t0)), 0.0);
}

// A point in the player's vehicle box frame (centred, x across, y along, z up).
fn fn_box_local(world: vec3<f32>) -> vec3<f32> {
    let d = world - camera.inside_a.xyz;
    let sh = camera.inside_a.w;
    let ch = camera.inside_b.x;
    return vec3<f32>(d.x * ch - d.y * sh - camera.inside_c.x, d.x * sh + d.y * ch - camera.inside_c.y, d.z - camera.inside_c.z);
}

// 1 where the weather reaches the point. The cab test above reaches 0.6 m below the
// vehicle's box so that the floor stays dry, and that takes in the road under the bus as
// well: the snow on it vanished in a bus-shaped patch of bare asphalt (which read as the
// bus's shadow wiping the snow off). The ground - terrain, roads, painted ground, flagged
// by `terrain` or the instance's surface flag - is never inside a vehicle.
fn weather_outside(world: vec3<f32>, terrain: bool, surface: f32) -> f32 {
    if (terrain || surface > 0.5) {
        return 1.0;
    }
    return 1.0 - inside_vehicle(world);
}

// As `weather_outside`, knowing the surface's normal: the bus's own outer skin is outside
// even where it lies inside the box. A side wall leans in towards the roof (and bulges over
// the wheel arches), so the box's side plane cuts through the panel: above the cut the paint
// took the cab's light and stayed dry, below it the sky's - the wall looked half glossy,
// half matt, along a sharp line. A face within 0.35 m of a side, the front, the back or the
// roof of the box and turned out through it is skin, not cabin.
fn weather_outside_n(world: vec3<f32>, n: vec3<f32>, terrain: bool, surface: f32) -> f32 {
    if (terrain || surface > 0.5) {
        return 1.0;
    }
    // a vehicle's part (lib.rs `Instance::roof`): under its roof it is dry, whichever
    // vehicle it is - the one the camera is in, another player's, a timetable bus
    if (surface < -500.0) {
        let roof = -surface - 5000.0;
        if (world.z < roof - 0.3) {
            return 0.0;
        }
    }
    if (inside_vehicle(world) < 0.5) {
        return 1.0;
    }
    let d = world - camera.inside_a.xyz;
    let sh = camera.inside_a.w;
    let ch = camera.inside_b.x;
    let x = d.x * ch - d.y * sh - camera.inside_c.x;
    let y = d.x * sh + d.y * ch - camera.inside_c.y;
    let z = d.z - camera.inside_c.z;
    let nx = n.x * ch - n.y * sh;
    let ny = n.x * sh + n.y * ch;
    let h = camera.inside_b.yzw;
    let skin_x = h.x - abs(x) < 0.35 && nx * sign(x) > 0.5;
    let skin_y = h.y - abs(y) < 0.35 && ny * sign(y) > 0.5;
    let skin_z = h.z - z < 0.35 && n.z > 0.5;
    return select(0.0, 1.0, skin_x || skin_y || skin_z);
}
@group(0) @binding(0) var<uniform> camera: Camera;
// Whether this pipeline draws alpha-tested materials (the only ones that may `discard`).
// Pipelines of every other kind set it false, and the discard is compiled out: a fragment
// function that can discard turns the GPU's early depth test off - on Apple's GPUs the
// hidden surface removal as well - for everything it draws, and the heavy shading then
// ran for every covered layer of the city, not once per pixel.
override ALPHA_TEST: bool = true;
// Multisampled cutout pipelines can turn filtered alpha directly into sample coverage.
override ALPHA_TO_COVERAGE: bool = false;
@group(0) @binding(5) var t_shadow: texture_depth_2d;
@group(0) @binding(6) var s_shadow: sampler_comparison;
@group(0) @binding(7) var t_shadow_far: texture_depth_2d;
@group(0) @binding(8) var t_ao: texture_2d<f32>;
@group(0) @binding(9) var s_ao: sampler;

// The ambient occlusion at a pixel of the full picture. It is worked out at half size, and
// a plain bilinear lookup blended the occlusion of the ground behind an edge with that of
// the object in front: every wheel, pole and kerb stood in a pale outline on the shaded
// ground under the bus. Of the four half-size texels around the pixel only those at the
// pixel's own depth count (a depth-aware upsample); none of them: the closest in depth.
fn ao_at(frag: vec2<f32>, world: vec3<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t_ao));
    let z = (camera.view_proj * vec4<f32>(world, 1.0)).w;
    let f = frag * 0.5 - vec2<f32>(0.5);
    let base = vec2<i32>(floor(f));
    let fr = f - floor(f);
    let tol = 0.04 + 0.015 * z;
    var sum = 0.0;
    var wsum = 0.0;
    var best = 1.0;
    var best_d = 1e9;
    for (var j = 0; j < 2; j = j + 1) {
        for (var i = 0; i < 2; i = i + 1) {
            let c = clamp(base + vec2<i32>(i, j), vec2<i32>(0), size - vec2<i32>(1));
            let s = textureLoad(t_ao, c, 0);
            let bw = select(1.0 - fr.x, fr.x, i == 1) * select(1.0 - fr.y, fr.y, j == 1);
            let dz = abs(s.g - z);
            let w = bw * (1.0 - smoothstep(0.0, tol, dz)) + 1e-4 * bw;
            sum = sum + s.r * w;
            wsum = wsum + w;
            if (dz < best_d) {
                best_d = dz;
                best = s.r;
            }
        }
    }
    // no AO sample at this surface's depth: the surface is not in the depth prepass (a
    // door or panel drawn in front of what the AO saw) - the AO behind it must not show
    if (best_d > tol * 1.5) {
        return 1.0;
    }
    return select(best, sum / wsum, wsum > 0.02);
}
// the tile light maps (`.map.LM.bmp`) of the 5x5 tiles around the camera, north up, and where
// they lie: x, y the south-west corner (render-origin relative), z the side, w 1 when set
@group(0) @binding(18) var t_lmap: texture_2d<f32>;
@group(0) @binding(19) var<uniform> lmap: vec4<f32>;

// The sRGB curve both ways (the textures are sampled through it, the target writes through
// it): the classic picture multiplies on the encoded values, as Omsi.exe does.
fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

// The tile light map's light at a world point (black outside the loaded square).
fn light_map_at(p: vec3<f32>) -> vec3<f32> {
    if (lmap.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let uv = vec2<f32>((p.x - lmap.x) / lmap.z, 1.0 - (p.y - lmap.y) / lmap.z);
    let c = textureSampleLevel(t_lmap, s_ao, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
    let inside = uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
    return select(vec3<f32>(0.0), c, inside);
}

// A material lit at night by the tile light map (`[LightMapMapping]`, the splines): the
// unlit flag carries 0.35.
fn light_map_mapped(m: vec4<f32>) -> bool {
    return m.y > 0.3 && m.y < 0.45;
}
// (the model matrices as their columns, four vec4 each: some phone GPUs (Mali, Adreno) read an
// array of matrices from a storage buffer wrongly, and every mesh came out flat and far away)
@group(0) @binding(1) var<storage, read> models: array<vec4<f32>>;
fn model_matrix(e: u32) -> mat4x4<f32> {
    let k = e * 4u;
    return mat4x4<f32>(models[k], models[k + 1u], models[k + 2u], models[k + 3u]);
}
// x: alpha multiplier, y: visible (0/1), zw: uv offset
@group(0) @binding(2) var<storage, read> inst_params: array<vec4<f32>>;
// the frame's draw list: the per-draw entry of each drawn instance. Draws of the same mesh
// and material are batched, so the instance index points into this list, not at an entry.
@group(0) @binding(10) var<storage, read> draw_list: array<u32>;
struct PointLight {
    pos: vec4<f32>,   // xyz relative to the render origin, w radius (0: an enhanced-only light)
    color: vec4<f32>, // rgb, w intensity
    dir: vec4<f32>,   // spot direction, w cosine of the outer cone (< -1.5: a point light)
    extra: vec4<f32>, // enhanced path: cosine of the inner cone, core radius, beam gain, radius
};
@group(0) @binding(3) var<storage, read> lights: array<PointLight>;
// per cell CELL_CAP light indices, 0xffffffff = empty
@group(0) @binding(4) var<storage, read> grid: array<u32>;
const CELL_CAP: u32 = 32u;

@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
struct MaterialParams {
    color: vec4<f32>,
    // x: alpha mode (0 opaque, 1 test, 2 blend), y: unlit flag, z: has transmap, w: transmap uses alpha
    params: vec4<f32>,
    // x: terrain material (uv in tile space), y: detail texture repeats per tile (0 = none),
    // z: ground texture repeats per tile, w: has nightmap
    extra: vec4<f32>,
    // x: has lightmap, y: envmap factor, z: moisture, w: has [matl_envmap_mask]
    params2: vec4<f32>,
    // emissive colour (the o3d material's or a [matl_allcolor]'s)
    emissive: vec4<f32>,
    // specular colour and power of the D3D material (power in w; 0 = no highlight)
    specular: vec4<f32>,
    // x: [matl_bumpmap] factor, y: has a bump map, z/w: noZwrite/noZcheck
    bump: vec4<f32>,
    // the PBR set beside the diffuse texture: x normal map, y occlusion, z roughness,
    // w metalness (1 = the map has it; see t_pbr_normal / t_pbr_orm)
    pbr: vec4<f32>,
    // x: one of the bus's own screens (the enhanced glow and FXAA leave it alone)
    flags: vec4<f32>,
};
@group(1) @binding(2) var<uniform> material: MaterialParams;
@group(1) @binding(3) var t_trans: texture_2d<f32>;
@group(1) @binding(4) var t_night: texture_2d<f32>;
@group(1) @binding(5) var t_light: texture_2d<f32>;
@group(1) @binding(6) var t_env: texture_2d<f32>;
@group(1) @binding(7) var t_envmask: texture_2d<f32>;
@group(1) @binding(8) var t_bump: texture_2d<f32>;
@group(1) @binding(9) var t_pbr_normal: texture_2d<f32>;
@group(1) @binding(10) var t_pbr_orm: texture_2d<f32>;

// The reflection mask of a [matl_envmap] material: the alpha of its [matl_envmap_mask]
// texture when it has one, else the diffuse texture's alpha - which reads 1 for a texture
// without an alpha channel (a 24-bit bitmap, DXT1, a JPEG), so its factor alone decides.
fn has_env_mask() -> bool {
    return (u32(material.params2.w + 0.5) & 1u) != 0u;
}

// [matl_transmap] was given (its file there or not: see MaterialExtra::transmap_declared)
fn has_transmap_declared() -> bool {
    return (u32(material.params2.w + 0.5) & 2u) != 0u;
}

// [matl_texadress_border]: where the diffuse texture's coordinates leave [0, 1] (a roller
// blind's band scrolled away by [texcoordtransX/Y]) Direct3D reads the border colour, not
// the texture's edge (the sampler only clamps). Its rgb comes packed in flags.z.
fn diffuse_border(tex: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let p = u32(material.flags.z + 0.5);
    let border = vec4<f32>(
        f32((p >> 16u) & 0xffu) / 255.0,
        f32((p >> 8u) & 0xffu) / 255.0,
        f32(p & 0xffu) / 255.0,
        material.flags.w,
    );
    let outside = any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0));
    return select(tex, border, material.flags.y > 0.5 && outside);
}

fn reflection_mask(uv: vec2<f32>, diffuse_a: f32) -> f32 {
    let mask = textureSample(t_envmask, s_diffuse, uv).a;
    return select(diffuse_a, mask, has_env_mask());
}

// The D3DCOLOR Omsi.exe packs for its environment stage (0x7ff8ed): each channel the
// [matl_envmap] factor x 255 x its light (at most 1), truncated, and OR-ed together shifted
// into place without saturation - a factor over 1 spills into the neighbouring channel
// exactly as there (a factor of 10 at full light reads almost white).
fn omsi_texture_factor(factor: f32, light: vec3<f32>) -> vec3<f32> {
    let m = min(max(light, vec3<f32>(0.0)), vec3<f32>(1.0)) * max(factor, 0.0) * 255.0;
    let r = u32(m.r);
    let g = u32(m.g);
    let b = u32(m.b);
    let packed = (0xffu << 24u) | (r << 16u) | (g << 8u) | b;
    return vec3<f32>(f32((packed >> 16u) & 0xffu), f32((packed >> 8u) & 0xffu), f32(packed & 0xffu)) / 255.0;
}

// [matl_bumpmap]: Direct3D's bump-mapped environment stage moves the sphere-map lookup by
// a du/dv map times the factor. Every bump map in the stock and mod content is a grey
// height map, so the du/dv map has to be its slope; it is taken here as the forward
// differences of the height, a byte of height difference counting as a signed-byte step
// (1/127, hence the 2). The stock bodies' noise (factor 0.05-0.1) makes their reflections a
// little wavy, and the panel lines of the O530 Facelift's map (factor 1) break its
// reflection at every seam.
fn bump_offset(uv: vec2<f32>) -> vec2<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_bump));
    let h = textureSample(t_bump, s_diffuse, uv).a;
    let hx = textureSample(t_bump, s_diffuse, uv + vec2<f32>(texel.x, 0.0)).a;
    let hy = textureSample(t_bump, s_diffuse, uv + vec2<f32>(0.0, texel.y)).a;
    return clamp(vec2<f32>(h - hx, h - hy) * 2.0, vec2<f32>(-1.0), vec2<f32>(1.0)) * material.bump.x;
}

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) inst: u32,
};
struct VsOut {
    // invariant: the depth prepass and the main pass must compute the same depth to the
    // last bit, the main pass tests against the depth the prepass left
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) params: vec4<f32>,
    @location(4) params2: vec4<f32>,
    // the D3D material's highlight, lit at the vertex as Omsi.exe's fixed function lights
    // it: from the sun (light A) and from the light above (light B)
    @location(5) spec_sun: vec3<f32>,
    @location(6) spec_sky: vec3<f32>,
};

// Direct3D's specular term at a vertex (Omsi.exe switches it on in FormActivate, 0x8254e0):
// Omsi.exe's sun (light 0, 0x7089f0: directional, specular = light A) and the light from
// straight above (light 1: specular = light B), each (N.H)^power with the local viewer's
// half vector, only where the light falls on the face, times the material's specular colour
// and clamped at 1 - then interpolated across the face. Computed per pixel instead, every
// small flat part (a gear selector, a switch, a dashboard screen) caught a sharp spot of the
// sun in its middle that the original, lighting only at the corners, never shows.
fn vertex_specular(wp: vec3<f32>, n: vec3<f32>) -> array<vec3<f32>, 2> {
    var out = array<vec3<f32>, 2>(vec3<f32>(0.0), vec3<f32>(0.0));
    if (material.specular.w <= 0.0 || material.params.y >= 0.5) {
        return out;
    }
    let v = normalize(camera.cam_pos.xyz - wp);
    let p = material.specular.w;
    let l = camera.sun_dir.xyz;
    if (dot(n, l) > 0.0) {
        out[0] = camera.sun_color.rgb * camera.sun_dir.w * pow(max(dot(n, normalize(v + l)), 0.0), p);
    }
    let up = vec3<f32>(0.0, 0.0, 1.0);
    if (n.z > 0.0) {
        out[1] = camera.sky_color.rgb * pow(max(dot(n, normalize(v + up)), 0.0), p);
    }
    let total = out[0] + out[1];
    let k = min(vec3<f32>(1.0), total) / max(total, vec3<f32>(1e-6));
    out[0] = min(vec3<f32>(1.0), out[0] * k * material.specular.rgb);
    out[1] = min(vec3<f32>(1.0), out[1] * k * material.specular.rgb);
    return out;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let e = draw_list[in.inst];
    let m = model_matrix(e);
    let wp = m * vec4<f32>(in.pos, 1.0);
    var out: VsOut;
    // Legacy surfaces are pulled towards the eye along the line of sight. OMSI splines and
    // [surface] objects use a fixed 8 cm world lift; ordered scenery phases use their authored
    // world positions. Both use code 0.9 to keep weather classification without view-space pull.
    // Painted ground (0.75) stays put.
    let surf = inst_params[e * 2u + 1u].w;
    var cp = wp.xyz;
    if (surf > 0.9) {
        let to = wp.xyz - camera.cam_pos.xyz;
        let d = length(to);
        // (legacy surface objects, 1.25, a little more than the splines under them)
        let decal = select(0.0, 0.01 + 0.001 * d, surf > 1.1 && surf < 1.5);
        let pull = min(0.02 + 0.002 * d + decal, d * 0.3);
        cp = wp.xyz - to / max(d, 1e-3) * pull;
    }
    out.clip = camera.view_proj * vec4<f32>(cp, 1.0);
    out.world = wp.xyz;
    out.normal = safe_normal((m * vec4<f32>(in.normal, 0.0)).xyz);
    let sp = vertex_specular(wp.xyz, out.normal);
    out.spec_sun = sp[0];
    out.spec_sky = sp[1];
    let pr = inst_params[e * 2u];
    out.uv = in.uv + pr.zw;
    out.params = pr;
    out.params2 = inst_params[e * 2u + 1u];
    if (pr.y < 0.5) {
        // invisible: collapse the triangle
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

// Shadow map passes: depth from the sun, alpha-tested materials cut out by their texture.
// A surface that casts - a spline standing clear of the ground, a bridge deck - casts from
// half a metre further away from the sun: what lies right under it (the embankment object
// under a railway, the ground a ramp touches down on) is no darker for it, while the ground
// metres below a deck gets its shadow.
fn shadow_caster_pos(e: u32, wp: vec4<f32>) -> vec4<f32> {
    if (abs(inst_params[e * 2u + 1u].w - 1.0) < 0.01) {
        return vec4<f32>(wp.xyz - camera.sun_dir.xyz * 0.5, wp.w);
    }
    return wp;
}

@vertex
fn vs_shadow(in: VsIn) -> VsOut {
    let e = draw_list[in.inst];
    let m = model_matrix(e);
    let wp = shadow_caster_pos(e, m * vec4<f32>(in.pos, 1.0));
    var out: VsOut;
    out.clip = camera.light_view_proj * wp;
    out.world = wp.xyz;
    out.normal = in.normal;
    out.spec_sun = vec3<f32>(0.0);
    out.spec_sky = vec3<f32>(0.0);
    let pr = inst_params[e * 2u];
    out.uv = in.uv + pr.zw;
    out.params = pr;
    out.params2 = inst_params[e * 2u + 1u];
    if (pr.y < 0.5) {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

@vertex
fn vs_shadow_close(in: VsIn) -> VsOut {
    let e = draw_list[in.inst];
    let m = model_matrix(e);
    let wp = shadow_caster_pos(e, m * vec4<f32>(in.pos, 1.0));
    var out: VsOut;
    out.clip = camera.light_view_proj_close * wp;
    out.world = wp.xyz;
    out.normal = in.normal;
    out.spec_sun = vec3<f32>(0.0);
    out.spec_sky = vec3<f32>(0.0);
    let pr = inst_params[e * 2u];
    out.uv = in.uv + pr.zw;
    out.params = pr;
    out.params2 = inst_params[e * 2u + 1u];
    if (pr.y < 0.5) {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

@vertex
fn vs_shadow_far(in: VsIn) -> VsOut {
    let e = draw_list[in.inst];
    let m = model_matrix(e);
    let wp = shadow_caster_pos(e, m * vec4<f32>(in.pos, 1.0));
    var out: VsOut;
    out.clip = camera.light_view_proj_far * wp;
    out.world = wp.xyz;
    out.normal = in.normal;
    out.spec_sun = vec3<f32>(0.0);
    out.spec_sky = vec3<f32>(0.0);
    let pr = inst_params[e * 2u];
    out.uv = in.uv + pr.zw;
    out.params = pr;
    out.params2 = inst_params[e * 2u + 1u];
    if (pr.y < 0.5) {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn fs_shadow(in: VsOut) {
}

@fragment
fn fs_shadow_test(in: VsOut) {
    var duv = in.uv;
    if (material.extra.x > 0.5) {
        duv = in.uv * material.extra.z;
    }
    // Use the light-pass footprint for cutout coverage. Forcing mip 0 here aliases dense
    // foliage and alpha layers into a checkerboard when the light projection moves by a
    // texel; the mip-aware sample keeps the caster coverage coherent with its resolution.
    // For a material promoted to the shadow pass from Blend, diffuse alpha is commonly a
    // paint/gloss value rather than coverage (vehicle bodies use values such as 0.03). Only
    // alpha-test materials use diffuse alpha as a cutout; blended bodies are solid unless a
    // real transmap supplies coverage.
    var a = select(diffuse_border(textureSample(t_diffuse, s_diffuse, duv), duv).a, 1.0, material.params.x > 1.5 && material.params.z < 0.5);
    if (material.params.z > 0.5) {
        // (the transmap stays where it is: [texcoordtransX/Y] only moves the diffuse stage)
        let tm = textureSample(t_trans, s_diffuse, in.uv - in.params.zw);
        a = select(1.0, tm.a, material.params.w > 0.5);
    }
    if (a < 0.5) {
        discard;
    }
}

// Roads with feathered alpha borders stay blended while their overlaps compose.
// Once the surface phases are complete, only fully covered diffuse pixels occlude
// later scenery. The transparent borders must not become invisible depth walls.
@fragment
fn fs_surface_depth(in: VsOut) -> @location(0) vec4<f32> {
    let a = diffuse_border(textureSample(t_diffuse, s_diffuse, in.uv), in.uv).a
        * material.color.a * in.params.x;
    if (a < 0.999) {
        discard;
    }
    return vec4<f32>(0.0);
}

// A vehicle body and its windows are often one mesh/material. OMSI represents that
// material as alpha-blended because the transmap contains the window opacity, but the
// alpha-tested body pixels still have to occlude vehicles behind them. The normal depth prepass
// deliberately skips all blended materials, so those pixels would otherwise have no depth
// until the colour pass (where blend order can expose the bus through the car). Keep only
// the effectively opaque part of a transmap in a separate depth-only pass; window pixels
// remain out of the prepass and are composited normally.
@fragment
fn fs_transmap_depth(in: VsOut) {
    if (material.params.z < 0.5) {
        discard;
    }
    let tm = textureSample(t_trans, s_diffuse, in.uv - in.params.zw);
    let a = select(1.0, tm.a, material.params.w > 0.5) * in.params.x;
    // Only what the colour pass will cover completely may hide what lies behind it: a
    // texel that is merely more opaque than not (the dimmer and anti-aliased dots of a
    // display's text layer, whose transmap is its script texture) wrote depth here, the
    // display's backplate behind it was then rejected, and the half-transparent text was
    // blended over the sky - holes in the display.
    if (a < 0.99) {
        discard;
    }
}

// 1 = lit by the sun, 0 = in shadow. Two cascades: the near one (sharp, around the camera)
// and the far one covering the rest of the visible street.
const SHADOW_OFFSETS: array<vec2<f32>, 16> = array<vec2<f32>, 16>(
    vec2<f32>(-0.942, -0.399), vec2<f32>(0.945, -0.769), vec2<f32>(-0.094, -0.929), vec2<f32>(0.345, 0.293),
    vec2<f32>(-0.915, 0.458), vec2<f32>(-0.815, -0.879), vec2<f32>(-0.382, 0.276), vec2<f32>(0.974, 0.756),
    vec2<f32>(0.443, -0.975), vec2<f32>(0.537, -0.474), vec2<f32>(-0.264, -0.418), vec2<f32>(0.792, -0.184),
    vec2<f32>(-0.758, 0.827), vec2<f32>(-0.386, -0.938), vec2<f32>(-0.203, 0.768), vec2<f32>(0.147, -0.169)
);

// Depth bias of the sun shadow, in metres along the sun's ray. The maps' depth runs over
// the light box's 2199 m, and the comparison used to take off 0.0015 (near) and 0.004 (far)
// of that: 3.3 m and 8.8 m. Nothing closer to the ground than that along the ray cast a
// shadow - not a car (1.4 m), not a bus (3 m; only its roof's shadow at a low sun,
// which lay detached and offset from the bus) - and the grass and kerbs had none either.
// The receiver plane below takes care of what the big constant stood in for.
const SHADOW_DEPTH_RANGE: f32 = 2199.0;
const SHADOW_BIAS_NEAR: f32 = 0.06;
const SHADOW_BIAS_FAR: f32 = 0.3;
const SHADOW_BIAS_CLOSE: f32 = 0.025;

// How the receiver's depth in the shadow map changes across the map (per unit of uv), from
// its normal: the PCF taps around the pixel compare against the surface's own depth at the
// tap instead of the centre's, so a surface at a slant to the sun does not shadow itself
// within the filter radius (acne), which a big constant bias used to hide. `lvp` is the
// cascade's light matrix (orthographic, so the map from the world is affine).
fn shadow_receiver_slope(lvp: mat4x4<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t1 = normalize(cross(n, a));
    let t2 = cross(n, t1);
    let p1 = (lvp * vec4<f32>(t1, 0.0)).xyz;
    let p2 = (lvp * vec4<f32>(t2, 0.0)).xyz;
    // (uv = 0.5 x + 0.5, 0.5 - 0.5 y)
    let d1 = vec3<f32>(p1.x * 0.5, -p1.y * 0.5, p1.z);
    let d2 = vec3<f32>(p2.x * 0.5, -p2.y * 0.5, p2.z);
    let det = d1.x * d2.y - d2.x * d1.y;
    if (abs(det) < 1e-12) {
        return vec2<f32>(0.0);
    }
    let g = vec2<f32>(d1.z * d2.y - d2.z * d1.y, d1.x * d2.z - d2.x * d1.z) / det;
    // a surface seen edge-on by the sun: keep the slope finite (it is in shadow of itself
    // there anyway, by its normal)
    return clamp(g, vec2<f32>(-2.0), vec2<f32>(2.0));
}

// The near map is an atlas two cascades wide: the near cascade in its left half, the close
// one (`camera.light_view_proj_close`) in its right half. `uv` is the cascade's own 0..1.
// The kernel's four outermost taps (SHADOW_OFFSETS 5, 1, 7, 12: one in each corner) and
// the one nearest its middle (15, which catches a pole's shadow thinner than the kernel)
// are taken first: when they agree - all lit or all in shadow - the pixel is not in a
// penumbra and the other eleven would agree too. Most of a picture is plainly lit or
// plainly shaded, so the filter mostly costs 5 compares instead of 16.
const SHADOW_CORNERS: array<i32, 5> = array<i32, 5>(5, 1, 7, 12, 15);
const SHADOW_REST: array<i32, 11> = array<i32, 11>(0, 2, 3, 4, 6, 8, 9, 10, 11, 13, 14);

// `scale`: the share of its half of the atlas the cascade fills from the top left (the close
// cascade is drawn no bigger than 2048 texels, see `SHADOW_CLOSE_MAX` in lib.rs).
fn shadow_pcf_atlas(uv: vec2<f32>, z: f32, slope: vec2<f32>, texel: f32, half: f32, spread: f32, bias: f32, scale: f32) -> f32 {
    var corners = 0.0;
    for (var k = 0; k < 5; k = k + 1) {
        let o = SHADOW_OFFSETS[SHADOW_CORNERS[k]] * texel * spread;
        let a = vec2<f32>((uv.x + o.x) * 0.5 * scale + half, (uv.y + o.y) * scale);
        corners = corners + textureSampleCompareLevel(t_shadow, s_shadow, a, z + dot(slope, o) - bias);
    }
    if (corners <= 0.0 || corners >= 5.0) {
        return corners * 0.2;
    }
    var sum = corners;
    for (var k = 0; k < 11; k = k + 1) {
        let o = SHADOW_OFFSETS[SHADOW_REST[k]] * texel * spread;
        let a = vec2<f32>((uv.x + o.x) * 0.5 * scale + half, (uv.y + o.y) * scale);
        sum = sum + textureSampleCompareLevel(t_shadow, s_shadow, a, z + dot(slope, o) - bias);
    }
    return sum / 16.0;
}

fn shadow_pcf_near(uv: vec2<f32>, z: f32, slope: vec2<f32>, texel: f32) -> f32 {
    return shadow_pcf_atlas(uv, z, slope, texel, 0.0, 1.6, SHADOW_BIAS_NEAR / SHADOW_DEPTH_RANGE, 1.0);
}

fn shadow_pcf_close(uv: vec2<f32>, z: f32, slope: vec2<f32>, texel: f32) -> f32 {
    // (camera.post.w: the close map's size over the near map's)
    let scale = select(1.0, camera.post.w, camera.post.w > 0.0);
    return shadow_pcf_atlas(uv, z, slope, texel / scale, 0.5, 1.8, SHADOW_BIAS_CLOSE / SHADOW_DEPTH_RANGE, scale);
}

fn shadow_push_close(n: vec3<f32>, ndl: f32) -> vec3<f32> {
    return n * (0.015 + 0.03 * (1.0 - ndl));
}

// The close cascade's value and weight at a point (weight 0 outside it): it fades into the
// near cascade between 60 and 90 % of its range from the camera.
fn shadow_close(world: vec3<f32>, n: vec3<f32>, ndl: f32, thin: bool) -> vec2<f32> {
    let range = camera.flags.w;
    if (range <= 0.0) {
        return vec2<f32>(1.0, 0.0);
    }
    let radial = distance(world, camera.cam_pos.xyz);
    let w = 1.0 - smoothstep(range * 0.6, range * 0.9, radial);
    if (w <= 0.001) {
        return vec2<f32>(1.0, 0.0);
    }
    let lp = camera.light_view_proj_close * vec4<f32>(world + shadow_push_close(n, ndl), 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    let edge = max(abs(uv.x - 0.5), abs(uv.y - 0.5));
    if (edge >= 0.48 || lp.z < 0.0 || lp.z > 1.0) {
        return vec2<f32>(1.0, 0.0);
    }
    let slope = select(shadow_receiver_slope(camera.light_view_proj_close, n), vec2<f32>(0.0), thin);
    return vec2<f32>(shadow_pcf_close(uv, lp.z, slope, camera.shadow.y), w);
}

fn shadow_pcf_far(uv: vec2<f32>, z: f32, slope: vec2<f32>, texel: f32) -> f32 {
    let bias = SHADOW_BIAS_FAR / SHADOW_DEPTH_RANGE;
    // (corners first, as in `shadow_pcf_atlas`)
    var corners = 0.0;
    for (var k = 0; k < 5; k = k + 1) {
        let o = SHADOW_OFFSETS[SHADOW_CORNERS[k]] * texel * 2.2;
        corners = corners + textureSampleCompareLevel(t_shadow_far, s_shadow, uv + o, z + dot(slope, o) - bias);
    }
    if (corners <= 0.0 || corners >= 5.0) {
        return corners * 0.2;
    }
    var sum = corners;
    for (var k = 0; k < 11; k = k + 1) {
        let o = SHADOW_OFFSETS[SHADOW_REST[k]] * texel * 2.2;
        sum = sum + textureSampleCompareLevel(t_shadow_far, s_shadow, uv + o, z + dot(slope, o) - bias);
    }
    return sum / 16.0;
}

// The shadow lookup point: pushed off the surface along its normal by about a texel of the
// cascade (0.14 m near, 0.68 m far), more where the surface turns away from the sun.
fn shadow_push_near(n: vec3<f32>, ndl: f32) -> vec3<f32> {
    return n * (0.06 + 0.12 * (1.0 - ndl));
}

fn shadow_push_far(n: vec3<f32>, ndl: f32) -> vec3<f32> {
    return n * (0.3 + 0.6 * (1.0 - ndl));
}

// `thin`: an alpha-tested card (leaves, fences), whose normal says nothing about its plane
// (OMSI points a tree's normals up): no receiver plane, the lookup moves half a metre
// towards the sun instead, which keeps a crown from speckling itself.
fn sun_shadow(world_in: vec3<f32>, n: vec3<f32>, thin: bool) -> f32 {
    if (camera.shadow.x < 0.5) {
        return 1.0;
    }
    let world = world_in + select(vec3<f32>(0.0), camera.sun_dir.xyz * 0.5, thin);
    let ndl = clamp(dot(n, camera.sun_dir.xyz), 0.0, 1.0);
    let close = shadow_close(world, n, ndl, thin);
    if (close.y >= 0.999) {
        return close.x;
    }
    let near_lp = camera.light_view_proj * vec4<f32>(world + shadow_push_near(n, ndl), 1.0);
    let near_uv = vec2<f32>(near_lp.x * 0.5 + 0.5, 0.5 - near_lp.y * 0.5);
    let inside = max(abs(near_uv.x - 0.5), abs(near_uv.y - 0.5));
    let t = camera.shadow.y;
    if (inside < 0.48 && near_lp.z >= 0.0 && near_lp.z <= 1.0) {
        let nv = shadow_pcf_near(near_uv, near_lp.z, select(shadow_receiver_slope(camera.light_view_proj, n), vec2<f32>(0.0), thin), t);
        return mix(nv, close.x, close.y);
    }
    let lp = camera.light_view_proj_far * vec4<f32>(world + shadow_push_far(n, ndl), 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || lp.z < 0.0 || lp.z > 1.0) {
        return 1.0;
    }
    let sum = shadow_pcf_far(uv, lp.z, select(shadow_receiver_slope(camera.light_view_proj_far, n), vec2<f32>(0.0), thin), t);
    let edge = clamp((0.5 - max(abs(uv.x - 0.5), abs(uv.y - 0.5))) * 12.0, 0.0, 1.0);
    return mix(1.0, sum, edge);
}

// Sum of the point lights registered in the grid cell of `p`. A light is registered in
// every cell its range touches, so the point's own cell holds every light that reaches it;
// looking into the neighbouring cells as well counted a light up to four times, and how
// often depended on where the cell edges fell - lamp pools brightened and dimmed as the
// camera moved.
// `map_k`: how much of the map's lamps a surface takes (0 on a light-mapped road in the
// classic picture, whose lamps are in its light map); a vehicle's own lights (dir.x 1, see
// lib.rs `gpu_light`) always shine - the headlights lit no road at all in vanilla.
fn point_lights(p: vec3<f32>, n: vec3<f32>, map_k: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let cell = camera.light_grid.z;
    let side = u32(camera.light_grid.w);
    if (cell <= 0.0 || side == 0u) {
        return sum;
    }
    let f = (p.xy - camera.light_grid.xy) / cell;
    let x = i32(floor(f.x));
    let y = i32(floor(f.y));
    if (x >= 0 && y >= 0 && x < i32(side) && y < i32(side)) {
        let base = (u32(y) * side + u32(x)) * CELL_CAP;
        for (var j = 0u; j < CELL_CAP; j = j + 1u) {
            let li = grid[base + j];
            if (li == 0xffffffffu) {
                break;
            }
            let l = lights[li];
            let d = l.pos.xyz - p;
            let dist = length(d);
            if (dist >= l.pos.w) {
                continue;
            }
            // full intensity within 1/8 of the range, then inverse-square, cut off at the range;
            // the strength was set by eye while a street lamp was still counted about three
            // times (see above), so it is three times what it was, to keep the lamp pools
            let r0 = l.pos.w * 0.125;
            let att = min(1.0, (r0 * r0) / max(dist * dist, 0.01)) * clamp(1.0 - dist / l.pos.w, 0.0, 1.0) * 3.75;
            let ndl = max(dot(n, d / max(dist, 0.01)), 0.15);
            var k = select(map_k, 1.0, l.dir.x > 0.5 && l.dir.w < -1.5);
            if (l.dir.w >= -1.5) {
                // a spot (a vehicle's [spotlight], as Direct3D lights with it): full inside
                // the inner cone, fading to nothing at the outer one; nothing behind it
                let c = dot(-d / max(dist, 0.01), l.dir.xyz);
                k = smoothstep(l.dir.w, max(l.extra.x, l.dir.w + 1e-3), c);
            }
            sum = sum + l.color.rgb * l.color.w * att * ndl * k;
        }
    }
    return sum;
}

// [interiorlight]: the light of a vehicle's saloon lamps on a mesh that names them in its
// [illumination_interior], 1 = a lamp's full light on a seat under it. `code` is the
// instance's lamp code (lib.rs `Instance::interior_lamps`): the first of its lamp slots in
// `lights` times 64 plus how many; below 1 it is a plain brightness (a passenger standing in
// a lit bus). Each lamp is OMSI's Direct3D point light: attenuation
// 1 / (d² / core²) with `core` the lamp's [interiorlight] range, no cut-off short of 100 m,
// times N·L; the sum saturates, as Direct3D's vertex colour does.
fn interior_lamps(p: vec3<f32>, n: vec3<f32>, code: f32) -> vec3<f32> {
    if (code < 1.0) {
        return vec3<f32>(1.0, 0.96, 0.84) * clamp(code, 0.0, 1.0);
    }
    let c = u32(code + 0.5);
    // (LAMP_CODE_STRIDE: 64 - up to 63 lamps a mesh)
    let first = c >> 6u;
    let count = c & 63u;
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < count; i = i + 1u) {
        let l = lights[first + i];
        let r = l.extra.w;
        let d = l.pos.xyz - p;
        let dist2 = max(dot(d, d), 1e-4);
        if (l.color.w <= 0.0 || dist2 >= r * r) {
            continue;
        }
        let core = max(l.extra.y, 0.01);
        let att = core * core / dist2;
        let ndl = max(dot(n, d * inverseSqrt(dist2)), 0.0);
        sum = sum + l.color.rgb * l.color.w * att * ndl;
    }
    return min(sum, vec3<f32>(1.0));
}

// Value noise in three octaves: the "fractal" detail (the `detail_textures` setting) laid
// over the ground and the road surfaces close by, so that a blurred texture keeps some grain.
//
// The lattice cell is hashed as an integer. The old `fract(sin(dot(q, …)) * 43758.5)`
// hash took the cell's map coordinate - two and a half million at Spandau's 892 km - into
// a sine: in 32-bit floats its argument is a multiple of 64 there, and what came out was
// not noise but a ramp that repeated along straight lines, so every road wore thin
// diagonal streaks at an exact spacing. Cells are wrapped at `PATTERN_PERIOD` (the pattern
// coordinate is taken modulo that on the CPU, see `world_pattern_xy`), which keeps the
// hash's input small and the pattern seamless across the wrap.
const PATTERN_PERIOD: f32 = 1000.0;

fn hash_cell(c: vec2<f32>, cells: f32) -> f32 {
    // the cell index modulo the lattice's cells per period, as an exact integer
    let w = c - cells * floor(c / cells);
    var v = vec2<u32>(w) * 1664525u + vec2<u32>(1013904223u);
    // (a PCG-style mix: two rounds of multiply, cross-add and xor-shift)
    v.x = v.x + v.y * 1664525u;
    v.y = v.y + v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x = v.x + v.y * 1664525u;
    v.y = v.y + v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32(v.x >> 8u) / 16777215.0;
}

// `freq`: lattice cells per metre of `p` (a whole number of cells per `PATTERN_PERIOD`).
fn vnoise_f(p: vec2<f32>, freq: f32, offset: vec2<f32>) -> f32 {
    let q = p * freq + offset;
    let cells = round(PATTERN_PERIOD * freq);
    let i = floor(q);
    let f = q - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash_cell(i, cells);
    let b = hash_cell(i + vec2<f32>(1.0, 0.0), cells);
    let c = hash_cell(i + vec2<f32>(0.0, 1.0), cells);
    let d = hash_cell(i + vec2<f32>(1.0, 1.0), cells);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}


fn detail_noise(p: vec2<f32>) -> f32 {
    // Anti-alias: point-sampling a lattice this fine aliases wherever one pixel's
    // footprint spans more than about one cell of it - looking down a street at a
    // grazing angle, or just driving away, constantly changes that footprint, and the
    // alias pattern with it, which reads as the "fractal" texture re-rendering itself as
    // the view angle changes. `fwidth` gives the footprint in the same units as `p`; an
    // octave fades to its own mean (0.5, the hash is uniform 0..1) once a pixel can no
    // longer resolve it, which is what an infinitely dense filter of it would give anyway.
    let w = max(fwidth(p).x, fwidth(p).y);
    let fade0 = 1.0 - smoothstep(0.3, 1.2, w * 2.9);
    let fade1 = 1.0 - smoothstep(0.3, 1.2, w * 0.73);
    let fade2 = 1.0 - smoothstep(0.3, 1.2, w * 0.19);
    let n0 = mix(0.5, vnoise_f(p, 2.9, vec2<f32>(0.0)), fade0);
    let n1 = mix(0.5, vnoise_f(p, 0.73, vec2<f32>(0.0)), fade1);
    let n2 = mix(0.5, vnoise_f(p, 0.19, vec2<f32>(0.0)), fade2);
    return n0 * 0.5 + n1 * 0.3 + n2 * 0.2;
}

fn world_pattern_xy(p: vec3<f32>) -> vec2<f32> {
    // Geometry is relative to the floating render origin for f32 precision. Procedural
    // patterns are not: reconstruct their stable map-space coordinate so crossing the
    // renderer's 100 m origin cell cannot re-seed the asphalt/ground pattern. The origin
    // arrives modulo PATTERN_PERIOD (lib.rs), so this stays a small, exact number: the map
    // coordinate itself (892 km at Spandau) has a 6 cm step in 32 bits, a fifth of the
    // finest noise cell.
    return p.xy + camera.world_origin.xy;
}

// Whether every component is a finite number, tested on the bits: Metal's fast maths takes
// `x != x` and comparisons with a NaN as it likes (a NaN colour happened to come out as
// something on the Mac), Direct3D 12 keeps the NaN and draws it black.
fn all_finite(v: vec3<f32>) -> bool {
    let e = bitcast<vec3<u32>>(v) & vec3<u32>(0x7f800000u);
    return all(e != vec3<u32>(0x7f800000u));
}

fn finite_or(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    return select(fallback, v, all_finite(v));
}

fn rain_hash(p: vec2<f32>) -> vec2<f32> {
    // Integer mixing keeps neighbouring cells independent without the precision loss and
    // repeated sine evaluations of a floating-point hash.
    let cell = bitcast<vec2<u32>>(vec2<i32>(floor(p)));
    var a = cell.x * 0x9e3779b9u ^ cell.y * 0x85ebca6bu;
    a = (a ^ (a >> 16u)) * 0x7feb352du;
    a = (a ^ (a >> 15u)) * 0x846ca68bu;
    a = a ^ (a >> 16u);
    var b = a ^ 0x68bc21ebu;
    b = (b ^ (b >> 16u)) * 0x7feb352du;
    b = (b ^ (b >> 15u)) * 0x846ca68bu;
    b = b ^ (b >> 16u);
    return vec2<f32>(f32(a), f32(b)) * (1.0 / 4294967296.0);
}

// The rain on a window pane at one pixel: the water's surface and how much of the pixel it
// covers. Three kinds of water, as on a bus window in the rain:
// * a mist of droplets too small to make out one by one, which greys the glass a little;
// * drops that sit on the glass in three sizes (they land, grow and dry up again, each at
//   its own pace; more and bigger ones the wetter the pane), each a little dome flattened
//   by its weight;
// * runners: drops grown heavy enough to slide down, in fits and starts, a teardrop that
//   wipes a clear track through the mist and the drops below it and leaves a string of
//   beads behind.
// `uv` is the pane's own texture coordinate (without the [texcoordtransY] scroll: that
// moved the whole picture of drops down at once, like paper off a roll), `n` its normal,
// `wet` how wet it is. Laid out in metres over the glass and running down along the
// world's vertical as it lies on the pane, whatever way the model's texture is turned.
struct RainGlass {
    // the water's surface normal (world), pointing out of the water
    n: vec3<f32>,
    // how much of the pixel a drop or a bead covers (0..1, antialiased)
    cover: f32,
    // the pane's normal on the side the rain is on (outside the bus)
    out: vec3<f32>,
    // the fine mist of droplets (0..1)
    mist: f32,
};

// One drop of radius `r` (metres) centred `d` (metres, across / down the glass) away:
// its cover of the pixel (`px`: the pixel's size on the glass) and the slope of its dome
// (the normal's part along the glass). A drop wets the glass at about 70°: the dome is a
// spherical cap whose normal tilts by up to sin 70° at the rim.
fn rain_dome(d: vec2<f32>, r: f32, px: f32) -> vec3<f32> {
    let rr = length(d) / max(r, 1e-5);
    let aa = max(px / max(r, 1e-5), 0.02);
    // (a drop smaller than a pixel fades away rather than flicker: the mist stands in)
    let seen = smoothstep(0.3, 0.9, r / max(px, 1e-6));
    let cover = (1.0 - smoothstep(1.0 - aa, 1.0 + aa, rr)) * seen;
    let slope = d / max(r, 1e-5) * 0.94;
    let s = select(slope, slope / max(length(slope), 1e-5) * 0.94, length(slope) > 0.94);
    return vec3<f32>(s, cover);
}

// Smooth noise over the glass (0..1): the rain gathers in patches, not evenly.
fn rain_patches(q: vec2<f32>) -> f32 {
    let c = floor(q);
    let f = q - c;
    let w = f * f * (3.0 - 2.0 * f);
    let a = rain_hash(c).x;
    let b = rain_hash(c + vec2<f32>(1.0, 0.0)).x;
    let d = rain_hash(c + vec2<f32>(0.0, 1.0)).x;
    let e = rain_hash(c + vec2<f32>(1.0, 1.0)).x;
    return mix(mix(a, b, w.x), mix(d, e, w.x), w.y);
}

// A grid of its own for each layer of drops: turned by its own angle and shifted, so that
// no two layers line up and none runs along the glass's edges.
fn rain_turn(q: vec2<f32>, a: f32) -> vec2<f32> {
    let cs = vec2<f32>(cos(a), sin(a));
    return vec2<f32>(q.x * cs.x - q.y * cs.y, q.x * cs.y + q.y * cs.x);
}

fn rain_glass(world: vec3<f32>, uv: vec2<f32>, n: vec3<f32>, wet: f32, t: f32, outside_in: bool) -> RainGlass {
    var g: RainGlass;
    g.n = n;
    g.out = n;
    g.cover = 0.0;
    g.mist = 0.0;
    // (the derivatives first, while every pixel of the quad still takes part)
    let dpx = dpdx(world);
    let dpy = dpdy(world);
    let dux = dpdx(uv);
    let duy = dpdy(uv);
    // the pixel's size on the glass
    let px = max(max(length(dpx), length(dpy)), 1e-5);
    let det = dux.x * duy.y - dux.y * duy.x;
    if (wet <= 0.002 || abs(det) < 1e-12) {
        return g;
    }
    // world position's derivative along u and v (inverse of the screen Jacobian): metres
    // per unit of uv, and which way is down in uv
    let dpdu = (dpx * duy.y - dpy * dux.y) / det;
    let dpdv = (dpy * dux.x - dpx * duy.x) / det;
    let mu = length(dpdu);
    let mv = length(dpdv);
    if (mu < 1e-6 || mv < 1e-6) {
        return g;
    }
    // quantised so neighbouring triangles of one pane share the grid
    let su = exp2(round(log2(mu)));
    let sv = exp2(round(log2(mv)));
    let p = vec2<f32>(uv.x * su, uv.y * sv);
    // down in uv: against the height's gradient
    var down = -vec2<f32>(dpdu.z / mu, dpdv.z / mv);
    if (length(down) < 0.2) {
        down = vec2<f32>(0.0, 1.0); // a roof light: no way down, the drops just sit
    }
    // Snapped to the nearest eighth of a turn: `down` comes from the world's vertical, and
    // the body rolls and pitches on its springs all the time - a grid turned with it by a
    // degree moves drops a metre from its origin by more than a cell, so every drop on the
    // pane jumped to a neighbour's place and back from frame to frame (the drops trembled
    // and flickered while driving). A pane's texture is laid out square to it, so the eighth
    // turns keep the drops fixed to the glass and still running down it.
    let ang = round(atan2(down.y, down.x) / 0.7853982) * 0.7853982;
    down = vec2<f32>(cos(ang), sin(ang));
    let side = vec2<f32>(down.y, -down.x);
    // pane coordinates in metres: x across, y down the glass
    let q = vec2<f32>(dot(p, side), dot(p, down));
    // the same two directions in the world, and the pane's normal on the rain's side: the
    // film faces the eye unless the eye is in the bus behind it
    let tu = dpdu / su;
    let tv = dpdv / sv;
    let eye = normalize(camera.cam_pos.xyz - world);
    var out = safe_normal(n);
    out = select(-out, out, dot(out, eye) >= 0.0);
    out = select(out, -out, outside_in);
    g.out = out;
    var side_w = side.x * tu + side.y * tv;
    side_w = side_w - out * dot(side_w, out);
    side_w = side_w / max(length(side_w), 1e-6);
    var down_w = down.x * tu + down.y * tv;
    down_w = down_w - out * dot(down_w, out) - side_w * dot(down_w, side_w);
    down_w = down_w / max(length(down_w), 1e-6);

    var best = 0.0;
    var slope = vec2<f32>(0.0);

    // --- the airstream: the glass moves through the air with the bus, and a drop's drag
    // grows with the speed squared - at about 40 km/h it matches the drop's weight. Along
    // the glass the air drives drops back; where it meets the glass head on (the
    // windscreen) it parts, up and out. The runners go where weight and air take them
    // together (in sixteenths of a turn, so that they do not shiver with every change of
    // speed); the drops that sit hold on.
    var flow = vec2<f32>(0.0, 1.0);
    var blow = 0.0;
    if (camera.wind.w > 0.5 && near_player_vehicle(world) > 0.5) {
        let air = -camera.wind.xyz;
        var aw = vec2<f32>(dot(air, side_w), dot(air, down_w));
        aw.y = aw.y - abs(dot(air, out)) * 0.6;
        let v2 = dot(aw, aw);
        blow = clamp(v2 / 130.0, 0.0, 3.0);
        flow = flow + aw / max(sqrt(v2), 1e-4) * blow;
    }
    let fa = round(atan2(flow.x, flow.y) / 0.3926991) * 0.3926991;
    // pane coordinates turned so the runners run down +y
    let qr = rain_turn(q, fa);

    // --- the runners: one lane every 4.5 cm, a few of them with a drop sliding down
    let lane_w = 0.045;
    let lane = floor(qr.x / lane_w);
    let lh = rain_hash(vec2<f32>(lane, 7.0));
    let lk = rain_hash(vec2<f32>(lane, 19.0));
    var track = 0.0;
    if (lh.x < wet * 0.5 * (1.0 + 0.8 * blow)) {
        let speed = (0.025 + 0.08 * lh.y) * (1.0 + 2.5 * blow);
        // stick and slip: the drop pauses, then hurries on
        let tt = t * speed + lh.x * 13.0;
        let head_y = (floor(tt) + smoothstep(0.35, 1.0, fract(tt))) * 0.3;
        let span = 1.6;
        // how far above the head (along the glass, upwards), wrapped over the pane
        var above = fract((head_y - qr.y) / span) * span;
        above = select(above, above - span, above > span - 0.03);
        // its path wanders a little across the glass, the same way every time
        let path_x = (lane + 0.5 + (lk.x - 0.5) * 0.5) * lane_w
            + sin(qr.y * 23.0 + lk.y * 6.3) * 0.005 + sin(qr.y * 67.0 + lane) * 0.0015;
        let dx = qr.x - path_x;
        // the head: a teardrop 4-8 mm across, drawn out upwards
        let rh = 0.0011 + 0.0007 * lk.y;
        let dd = vec2<f32>(dx, -above);
        let head = rain_dome(dd, rh, px);
        if (head.z > best) {
            best = head.z;
            slope = rain_turn(head.xy, -fa);
        }
        // the track above it: the mist and the sitting drops wiped away...
        let trail_len = 0.08 + 0.3 * lh.y;
        let hw = rh * 0.75;
        track = (1.0 - smoothstep(hw * 0.6, hw, abs(dx))) * step(rh, above) * (1.0 - smoothstep(trail_len * 0.5, trail_len, above));
        // ...and a string of small beads left behind in it
        let bc = floor(above / 0.006);
        let bh = rain_hash(vec2<f32>(lane * 3.1 + bc, 41.0));
        if (bh.x < 0.6 && above > rh * 2.0 && above < trail_len) {
            let bcen = vec2<f32>((bh.y - 0.5) * hw * 0.9, -((bc + 0.5) * 0.006 + (bh.x - 0.3) * 0.002));
            let bead = rain_dome(vec2<f32>(dx, -above) - bcen, 0.0005 + 0.0008 * bh.y, px);
            if (bead.z > best) {
                best = bead.z;
                slope = rain_turn(bead.xy, -fa);
            }
        }
    }

    // where the glass is wetter and where drier, in patches a hand across
    let wetter = 0.45 + 1.1 * rain_patches(q * 9.0 + 3.1);

    // --- the drops that sit: four sizes, one drop a cell at most, anywhere in it (the
    // fourth the few big ones, a centimetre and more, grown from drops that ran together)
    for (var layer = 0; layer < 4; layer = layer + 1) {
        let fl = f32(layer);
        let cellsz = select(select(select(0.0045, 0.0075, layer == 1), 0.012, layer == 0), 0.015, layer == 3);
        let turn = 0.61 + fl * 1.37;
        let g2 = rain_turn(q, turn) / cellsz + vec2<f32>(fl * 17.3, fl * 5.1);
        let c = floor(g2);
        let h = rain_hash(c);
        let h2 = rain_hash(c + 3.7);
        let life = 8.0 + h.y * 18.0;
        let ph = fract(t / life + h2.x);
        // landed, grown, drying: a drop comes and goes; more of them the wetter the pane,
        // and the big ones only on a wet pane
        let dens = wet * wetter * select(select(select(0.45, 0.36, layer == 2), 0.42 * smoothstep(0.15, 0.6, wet), layer == 0), 0.1 * smoothstep(0.3, 0.8, wet), layer == 3);
        let present = step(h.x, dens) * smoothstep(0.0, 0.04, ph) * (1.0 - smoothstep(0.85, 1.0, ph)) * (1.0 - track);
        if (present <= 0.0) {
            continue;
        }
        let r = select(0.1 + 0.2 * h.y * h.y, 0.14 + 0.16 * h.y, layer == 3) * mix(0.75, 1.0, smoothstep(0.0, 0.5, ph)) * mix(0.8, 1.1, wet);
        // (anywhere in the cell it still fits in)
        let room = min(r * 1.12, 0.48);
        let centre = c + vec2<f32>(room) + rain_hash(c + 9.1) * (1.0 - 2.0 * room);
        // flattened by its weight (below the pane's own down, not the turned grid's):
        // fuller below than above
        var d = rain_turn((g2 - centre) * cellsz, -turn);
        d.y = d.y * select(1.12, 0.9, d.y > 0.0);
        // no drop is a circle: its rim wanders where the glass held it as it spread, the
        // big ones most, and a heavy one hangs drawn out downwards
        // (a pixel beyond any rim it could have: nothing more to work out)
        let dl = length(d);
        if (dl > r * cellsz * 1.5) {
            continue;
        }
        let h3 = rain_hash(c + 23.9);
        // the rim's wander as waves of 2, 3 and 4 round the drop, each turned its own way
        // (sin(k a + phase) from the direction's cosine and sine, without an atan2)
        let u = d / max(dl, 1e-6);
        let c2 = u.x * u.x - u.y * u.y;
        let s2 = 2.0 * u.x * u.y;
        let c3 = u.x * (4.0 * u.x * u.x - 3.0);
        let s3 = u.y * (3.0 - 4.0 * u.y * u.y);
        let c4 = c2 * c2 - s2 * s2;
        let s4 = 2.0 * s2 * c2;
        let p2 = h3 * 2.0 - 1.0;
        let p3 = h2.yx * 2.0 - 1.0;
        let wave = (s2 * p2.x + c2 * p2.y) * h3.y + 0.55 * (s3 * p3.x + c3 * p3.y) + 0.3 * (s4 * p2.y - c4 * p3.x);
        let wob = 1.0 + (0.03 + 0.06 * h.y) * wave;
        d = d / max(wob, 0.4);
        d.y = d.y * mix(1.0, 0.88, h.y * h.y * h3.x);
        let drop = rain_dome(d, r * cellsz, px);
        let cover = drop.z * present;
        if (cover > best) {
            best = cover;
            slope = drop.xy;
        }
    }

    // --- the mist: droplets of a millimetre or less, scattered on two turned grids
    var grain = 0.0;
    for (var k = 0; k < 2; k = k + 1) {
        let mg = rain_turn(q, 0.33 + f32(k) * 2.1) / 0.0028 + f32(k) * 7.7;
        let mc = floor(mg);
        let mh = rain_hash(mc + 71.0);
        let mr = 0.1 + 0.16 * mh.y;
        let at = mc + vec2<f32>(mr) + rain_hash(mc + 5.3) * (1.0 - 2.0 * mr);
        grain = max(grain, step(mh.x, wet * wetter * 0.35) * (1.0 - smoothstep(mr * 0.6, mr, length(mg - at))));
    }
    // (seen from further than a millimetre a pixel: their average)
    let mist = mix(wet * wetter * 0.35 * 0.12, grain, smoothstep(0.0015, 0.0006, px));
    g.mist = mist * (1.0 - track) * clamp(wet * 1.4, 0.0, 1.0);

    g.cover = best;
    let nz = sqrt(max(1.0 - dot(slope, slope), 0.02));
    g.n = normalize(out * nz + side_w * slope.x + down_w * slope.y);
    return g;
}

// The way the eye looks through a drop: bent at its dome and at the flat glass, like
// through a strong lens (the world shows upside down in it). Zero where the light is
// trapped by total internal reflection: the dark ring round every drop.
fn rain_through(g: RainGlass, v: vec3<f32>) -> vec3<f32> {
    if (dot(g.out, v) > 0.0) {
        // the eye on the rain's side: into the dome, out through the glass
        let t1 = refract(-v, g.n, 1.0 / 1.333);
        if (dot(t1, t1) < 1e-4) {
            return vec3<f32>(0.0);
        }
        return refract(normalize(t1), g.out, 1.333);
    }
    // the eye in the bus: through the glass into the water, out of the dome
    let t1 = refract(-v, -g.out, 1.0 / 1.333);
    if (dot(t1, t1) < 1e-4) {
        return vec3<f32>(0.0);
    }
    return refract(normalize(t1), -g.n, 1.333);
}

// What the eye sees along `through` (the way out of a drop, `rain_through`) from the drop
// at `world`: the street behind the glass as the last frame drew it, looked up where that
// way meets it a few metres on - through a drop's rim the way bends far round, so the
// drop holds the whole street small and upside down, the sky at its bottom, as a real
// one does. The rain film's reflection slot holds that picture (see `Renderer::glass_slot`);
// without it (a mirror's view, the first frame of the rain) or off the picture's edge,
// `fallback` (the sky's colours). `scale` takes the picture into the caller's units.
fn rain_behind(world: vec3<f32>, through: vec3<f32>, fallback: vec3<f32>, scale: f32) -> vec3<f32> {
    if (camera.flags.z > -0.5 || dot(through, through) < 1e-4) {
        return fallback;
    }
    let c = camera.view_proj * vec4<f32>(world + normalize(through) * 6.0, 1.0);
    if (c.w <= 0.05) {
        return fallback;
    }
    let ndc = c.xy / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let inside = smoothstep(0.0, 0.06, min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)));
    let seen = finite_or(textureSampleLevel(t_env, s_diffuse, clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999)), 0.0).rgb * scale, fallback);
    return mix(fallback, seen, inside);
}

// A drop lit: `refl` what the dome mirrors, `thru` what is seen through it (both
// radiance), `mist_col` the light the mist scatters, `sun` the sun's strength for the
// sparkles, `d_thru` the way through (from `rain_through`). Colour and cover.
fn rain_light(g: RainGlass, v: vec3<f32>, d_thru: vec3<f32>, refl: vec3<f32>, thru: vec3<f32>, mist_col: vec3<f32>, sun: vec3<f32>) -> vec4<f32> {
    let facing = dot(g.out, v) > 0.0;
    let cosv = clamp(abs(dot(g.n, v)), 0.0, 1.0);
    // (from inside, the dome's inner face mirrors the cab: little of it)
    let f = select(0.02, 0.02 + 0.98 * pow(1.0 - cosv, 5.0), facing);
    let trapped = dot(d_thru, d_thru) < 1e-4;
    // the trapped light at the rim: what the dome mirrors, darkened
    var c = select(thru * 0.96 * (1.0 - f), refl * 0.3, trapped) + refl * f;
    // the sun: a pinpoint where the dome mirrors it, and the drop lit up whole where it
    // focuses the sun towards the eye
    let s = camera.sun_dir.xyz;
    if (facing) {
        let r = reflect(-v, g.n);
        c = c + sun * pow(max(dot(r, s), 0.0), 900.0) * f * 60.0;
    }
    if (!trapped) {
        c = c + sun * pow(max(dot(normalize(d_thru), s), 0.0), 60.0) * 1.5;
    }
    let a_drop = g.cover * 0.94;
    let a_mist = g.mist * 0.2 * (1.0 - a_drop);
    let a = a_drop + a_mist;
    let col = (c * a_drop + mist_col * a_mist) / max(a, 1e-4);
    return vec4<f32>(col, a);
}

// What a drop shows in the classic picture: the sky from the weather's colours, the
// horizon in the fog's, the ground darker.
fn rain_env_vanilla(d: vec3<f32>) -> vec3<f32> {
    let zenith = camera.sky_color.rgb + camera.ambient.rgb * 0.5;
    let horizon = max(camera.fog.rgb, camera.ambient.rgb);
    let ground = camera.ambient.rgb * 0.4 + camera.sun_color.rgb * camera.sun_dir.w * 0.06;
    let sky = mix(horizon, zenith, smoothstep(0.0, 0.5, d.z));
    return mix(ground, sky, smoothstep(-0.06, 0.04, d.z));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (material.emissive.w > 1.5) {
        // a pane's film of water: drops, not the sliding texture
        let v = normalize(camera.cam_pos.xyz - in.world);
        let in_cab = inside_vehicle(camera.cam_pos.xyz) * near_player_vehicle(in.world) > 0.5;
        let g = rain_glass(in.world, in.uv - in.params.zw, in.normal, in.params.x, camera.post.y, in_cab);
        let through = rain_through(g, v);
        let seen = select(rain_behind(in.world, through, rain_env_vanilla(normalize(through)), 1.0), vec3<f32>(0.0), dot(through, through) < 1e-4);
        let d = rain_light(g, v, through, rain_env_vanilla(reflect(-v, g.n)), seen, rain_env_vanilla(vec3<f32>(0.0, 0.0, 0.5)) * 0.9, camera.sun_color.rgb * camera.sun_dir.w);
        // (fading out with the distance: see enhanced.wgsl)
        let near = 1.0 - smoothstep(5.0, 15.0, distance(in.world, camera.cam_pos.xyz));
        return vec4<f32>(min(d.rgb, vec3<f32>(1.5)), d.a * near);
    }
    var duv = in.uv;
    if (material.extra.x > 0.5) {
        // terrain: uv is tile space; the ground texture repeats extra.z times per tile
        duv = in.uv * material.extra.z;
    }
    var tex = diffuse_border(textureSample(t_diffuse, s_diffuse, duv), duv);
    // The texture coordinates without the [texcoordtransX/Y] offset: in Omsi.exe's
    // fixed-function pipeline the texture transform is the diffuse stage's alone, the
    // transmap, night map and light map stay in place under a scrolling roller blind.
    let buv = in.uv - in.params.zw;
    if (material.extra.x > 0.5 && material.extra.y > 0.0) {
        // the ground texture's detail texture, repeated finer than the texture itself and
        // modulated over it as the original's terrain pass does. The stock detail maps are
        // bright (noise_low averages 242, gras_det 179), so they are meant to be multiplied
        // in plainly: doubling like a grey-centred D3D detail map blows the ground out.
        let det = textureSample(t_light, s_diffuse, in.uv * material.extra.y);
        tex = vec4<f32>(clamp(tex.rgb * det.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), tex.a);
    }
    if (material.params.z > 0.5) {
        // [matl_transmap]: alpha comes from a separate map, its alpha channel (a map without one
        // is opaque, as D3D samples it: the WH UK AI cars' paint layer has a black 24-bit
        // `transmap_null.tga`, read as luminance the paint was invisible);
        // for terrain the map is the per-tile surface mask in tile space
        let tm = textureSample(t_trans, s_diffuse, buv);
        tex.a = select(1.0, tm.a, material.params.w > 0.5);
        if (material.extra.x > 0.5 && material.params.x > 1.5) {
            // A painted ground layer. The brush mask is coarse (0.6-3 m per texel) and
            // binary; the loader smooths it into a soft ramp around a smooth curve
            // (`smooth_paint_mask` in scene.rs). Sharpen the ramp and let the ground
            // texture's own light and dark decide where the new surface wins within the
            // ramp - cobbles then fray into the grass stone by stone instead of in soft
            // rectangles. (On the raw mask this sharpening is what drew the texel grid
            // as a staircase along every painted edge.)
            let lum = dot(tex.rgb, vec3<f32>(0.333, 0.333, 0.333));
            let m = tex.a + (lum - 0.5) * 0.45;
            tex.a = smoothstep(0.32, 0.68, m);
        }
    }
    let mode = material.params.x;
    // Mip-filtered alpha is the fraction of the pixel covered by leaves or fence wires.
    // Tighten the transition around the cutout edge before MSAA turns it into sample
    // coverage. The depth prepass leaves these draws out so its binary cutoff cannot hide
    // the scene behind samples that the colour pass leaves open.
    if (ALPHA_TEST && mode > 0.5 && mode < 1.5) {
        if (ALPHA_TO_COVERAGE) {
            let aa = max(fwidth(tex.a) * 0.5, 1.0 / 255.0);
            if (tex.a < 0.5 - aa) {
                discard;
            }
            tex.a = smoothstep(0.5 - aa, 0.5 + aa, tex.a);
        } else if (tex.a < 0.5) {
            discard;
        }
    }
    let n = safe_normal(in.normal);
    let ndl = max(dot(n, camera.sun_dir.xyz), 0.0);
    // three lights like the original: direct sun (A), light from above (B), ambient (C)
    let from_above = 0.5 + 0.5 * n.z;
    let shadow = sun_shadow(in.world, n, mode > 0.5 && mode < 1.5);
    // screen-space ambient occlusion darkens the indirect light (sky and ambient) in
    // corners, under the bus, between the seats - not the sun, which the shadow map handles
    var ao = 1.0;
    // (not on a blended surface: the AO is the opaque depth's, and a translucent door
    // showed the shade of what stood behind it)
    if (camera.clouds.w > 0.5 && mode < 1.5) {
        ao = ao_at(in.clip.xy, in.world);
    }
    var diffuse = camera.sun_color.rgb * camera.sun_dir.w * ndl * shadow + (camera.sky_color.rgb * from_above * (0.6 + 0.4 * shadow) + camera.ambient.xyz) * ao;
    var albedo = tex.rgb;
    if (camera.flags.x > 0.5 && (material.extra.x > 0.5 || in.params2.w > 0.5)) {
        // detail texturing (setting): procedural grain on the ground and the roads, fading out with distance
        let dist0 = distance(in.world, camera.cam_pos.xyz);
        let k = clamp(1.0 - (dist0 - 25.0) / 120.0, 0.0, 1.0);
        let nz = detail_noise(world_pattern_xy(in.world));
        albedo = albedo * (1.0 + (nz - 0.5) * 0.42 * k);
    }
    // ([nomaplighting]: params.y 0.25 - not lit by the map's lamps)
    // (a light-mapped road or plate takes the map's lamps only in Vanilla+: as OMSI 2 shows
    // it, the vanilla picture lights it from the tile light map alone)
    let lm_only = light_map_mapped(material.params) && camera.sky_color.w > 0.5;
    let map_lamps = select(1.0, 0.0, (material.params.y > 0.2 && material.params.y < 0.3) || lm_only);
    let lamp_light = point_lights(in.world, n, map_lamps);
    var light = diffuse + lamp_light;
    let light_mapped = material.params2.x > 0.5 && material.extra.x < 0.5;
    var lit = albedo * material.color.rgb * light;
    // The vanilla picture: Omsi.exe's texture stages multiply the gamma-encoded texture by
    // the vertex light (clamped at 1); here the texture is sampled linear and the target
    // encodes again, so multiplied here a light L showed as L^(1/2.2) - a night at 0.06
    // looked like 0.28, a late dusk (#300). So the product is made on the encoded texture
    // and decoded again, with the light map laid on encoded as well. (Through the sRGB curve
    // itself: taken as a power of 2.2, t v^2.2, the curve's linear foot below 0.0031 put a
    // night wall at a quarter of OMSI 2's - a texture of 0.66 under a light of 0.05 came out
    // at 2 of 255 instead of 8.)
    let classic = camera.sky_color.w > 0.5;
    if (classic && material.params.y < 0.5) {
        var v = clamp(material.emissive.rgb + material.color.rgb * (light + interior_lamps(in.world, n, in.params2.z)), vec3<f32>(0.0), vec3<f32>(1.0));
        if (light_mapped) {
            let lm = srgb_encode(textureSample(t_light, s_diffuse, buv).rgb) * clamp(in.params2.x, 0.0, 1.0);
            v = v + lm * (vec3<f32>(1.0) - v);
        }
        lit = srgb_decode(srgb_encode(albedo) * v);
    } else if (light_mapped) {
        // [matl_lightmap], as Omsi.exe's texture stages have it (0x7fe4d3..0x7fe604): the
        // light map is laid onto the vertex light with D3DTOP_ADDSMOOTH (light + map x (1 -
        // light)) before the texture is multiplied in - a lit saloon glows at night and
        // hardly shows in daylight. The vertex light is D3D's: the material's emissive and
        // its colour times every light - the saloon lamps (`[interiorlight]`, D3D lights
        // too, 0x5fa8f0) among them - clamped at 1. (Added once more after the map, the
        // saloon lamps lit a cabin twice over, flat white where the map was full; and the
        // material's colour took the map down with it.)
        let lm = textureSample(t_light, s_diffuse, buv).rgb * clamp(in.params2.x, 0.0, 1.0);
        let v = clamp(material.emissive.rgb + material.color.rgb * (light + interior_lamps(in.world, n, in.params2.z)), vec3<f32>(0.0), vec3<f32>(1.0));
        lit = albedo * (v + lm * (vec3<f32>(1.0) - v));
    }
    // the D3D material's own highlight (specular colour and power of the o3d file or a
    // [matl_allcolor]), lit at the vertices (see `vertex_specular`) and added after the
    // texture as D3D's specular is; the sun's not in its shadow
    lit = lit + in.spec_sun * shadow + in.spec_sky;
    if (material.params.y > 0.5) {
        lit = tex.rgb * material.color.rgb;
    }
    if ((!light_mapped && !classic) || material.params.y > 0.5) {
        lit = lit + tex.rgb * material.emissive.rgb;
    }
    // the tile light map, as on the terrain: the lamps' pools on the roads and the plates,
    // lighting the surface (not painted over it: added as it was, the pool lay on the road
    // as a white patch); only where it is their light at night - elsewhere the map's lamps
    // light them as they light the squares and pavements beside them
    if (lm_only) {
        lit = lit + albedo * material.color.rgb * light_map_at(in.world) * camera.sun_color.w;
    }
    // [interiorlight]: the saloon lamps on the meshes and passengers they illuminate (in
    // a light-mapped material's vertex light already, above)
    if (!light_mapped && !classic) {
        lit = lit + tex.rgb * interior_lamps(in.world, n, in.params2.z);
    }
    if (material.extra.w > 0.5) {
        // [matl_nightmap]: self-illumination that fades in with the night
        // terrain: the tile light map in tile space (north at the top row)
        let nuv = select(buv, vec2<f32>(in.uv.x, 1.0 - in.uv.y), material.extra.x > 0.5);
        let nm = textureSample(t_night, s_diffuse, nuv);
        // a [matl_item] night map is switched by its variable (warning lamps, displays):
        // it glows whenever that is on, by day as well; the others fade in with the night
        let night = select(camera.sun_color.w, 1.0, material.extra.w > 1.5);
        lit = lit + nm.rgb * night * select(clamp(in.params2.y, 0.0, 1.0), 1.0, material.extra.w > 1.5);
    }
    if (material.params2.y > 0.0) {
        // [matl_envmap]: sphere map reflection, masked by the diffuse alpha like the original
        let vdir = normalize(in.world - camera.cam_pos.xyz);
        let r = reflect(vdir, n);
        // (the headset: laid out by the bus's heading, level, not by each eye's view)
        let vr_env = camera.cam_up.w > 0.5;
        let env_right = select(camera.cam_right.xyz, vec3<f32>(cos(camera.cam_right.w), -sin(camera.cam_right.w), 0.0), vr_env);
        let env_up = select(camera.cam_up.xyz, vec3<f32>(0.0, 0.0, 1.0), vr_env);
        let rx = dot(r, env_right);
        let ry = dot(r, env_up);
        var env_uv = vec2<f32>(rx * 0.5 + 0.5, 0.5 + ry * 0.5);
        if (material.bump.y > 0.5) {
            env_uv = env_uv + bump_offset(duv);
        }
        let env = textureSample(t_env, s_diffuse, env_uv);
        let diffuse_a = diffuse_border(textureSample(t_diffuse, s_diffuse, duv), duv).a;
        if (material.params.x < 1.5) {
            // rain: a painted body goes darker and glossier when it is wet, so the bus
            // stands out against a grey street instead of fading into it
            let env_night = select(1.0 - 0.85 * clamp(camera.sun_color.w, 0.0, 1.0), 1.0, camera.sky_color.w > 0.5);
            let env_light = clamp(camera.sun_color.r * camera.sun_dir.w + camera.sky_color.r + camera.ambient.r, 0.05, 1.0) * env_night;
            let wet = camera.shadow.w * weather_outside_n(in.world, n, false, in.params2.w);
            lit = lit * (1.0 - 0.30 * wet);
            // the water film mirrors the (overcast) sky a little, mostly at grazing
            // angles; kept faint - a strong rim read as a white outline round the bus
            let facing = clamp(abs(dot(vdir, n)), 0.0, 1.0);
            let sheen = wet * min(material.params2.y, 1.0) * (0.05 + 0.22 * pow(1.0 - facing, 5.0));
            lit = mix(lit, camera.sky_color.rgb * env_light, sheen);
        }
        // Omsi.exe's environment stage (0x7fd6c4 at 0x7ff626, fixed function), for paint
        // and glass alike: the sphere map's colour as it is, over what the stages before
        // made of the lit texture - D3DTOP_LERP by the texture factor when the material has
        // neither a [matl_transmap] nor a [matl_envmap_mask], D3DTOP_BLENDCURRENTALPHA when
        // it has one - and the alpha left as the stages before made it (ALPHAARG1 CURRENT,
        // SELECTARG1, 0x7ff98a): a clear pane shows its reflection as faintly as it shows
        // itself. (A glass path of our own - a quarter reflection at least, a Fresnel rim
        // and the pane made opaque where it reflected - mirrored the street in every
        // window far more than the original does.) The factor is [matl_envmap]'s times
        // the ambient light plus `g` (the light the vehicle stands in: the lamps of the
        // tile's light map and the sky), each at most 1, packed into a D3DCOLOR without
        // saturation (0x7ff8ed: a factor over 1 spills its bits into the next channel, as
        // there). The alpha it blends by is the mask's, or the diffuse texture's times `g`
        // (0x7feacc, 0x7ff092) - never the diffuse alpha of a material without one of
        // them: taken as a reflection mask, a body whose texture carries an alpha channel
        // for other purposes was mirrored (dark) where that alpha was high.
        // `g` (0x861ca0, set per vehicle at 0x7d8735): the mean of the light the tile's
        // light map throws on it plus the day's light A (weather +0xac = the mean of
        // lightcolor A, 0x75333f - by it the stars fade), at most 1
        let g = clamp(dot(lamp_light, vec3<f32>(1.0 / 3.0)) + dot(camera.sun_color.rgb, vec3<f32>(1.0 / 3.0)), 0.0, 1.0);
        var kk = vec3<f32>(0.0);
        if (has_transmap_declared() || has_env_mask()) {
            let a = select(diffuse_a, textureSample(t_envmask, s_diffuse, duv).a, has_env_mask());
            kk = vec3<f32>(a * g);
        } else {
            kk = omsi_texture_factor(material.params2.y, camera.ambient.rgb + vec3<f32>(g));
        }
        lit = mix(lit, env.rgb, kk);
    }
    // wet road: a surface whose texture carries [moisture] darkens under rain and starts
    // to mirror the sky, strongest where you look along it (the Fresnel sheen that makes a
    // wet street read as wet)
    let outside = weather_outside_n(in.world, n, material.extra.x > 0.5, in.params2.w);
    let wet = camera.shadow.w * material.params2.z * outside;
    if (wet > 0.0) {
        let vdir = normalize(in.world - camera.cam_pos.xyz);
        let facing = clamp(-dot(vdir, n), 0.0, 1.0);
        let fresnel = pow(1.0 - facing, 4.0);
        lit = lit * mix(1.0, 0.55, wet);
        let sheen = camera.sky_color.rgb * 0.5 + camera.sun_color.rgb * camera.sun_dir.w * 0.35;
        lit = mix(lit, sheen, clamp(fresnel * wet * 0.85, 0.0, 0.8));
    }
    // snow: the ground, the roads and every upward-facing surface whiten under it
    // (not on a shadow blob: whitened, it lit the snow under the bus instead of shading it)
    // (not in vanilla: OMSI 2 shows snow only through the season's WinterSnow textures)
    let snow = camera.ambient.w * outside * select(1.0, 0.0, in.params2.w > 1.5 || camera.sky_color.w > 0.5);
    if (snow > 0.0) {
        let up = clamp(n.z, 0.0, 1.0);
        let ground = select(0.0, 1.0, material.extra.x > 0.5 || material.params2.z > 0.0);
        // only surfaces that really face up get a cover; a soft threshold keeps the snow
        // off the sides and off the grazing rims that showed as a white outline
        let cover = snow * clamp(max(ground, smoothstep(0.78, 0.95, up) * 0.8), 0.0, 1.0);
        let light = camera.sun_color.rgb * camera.sun_dir.w * ndl * shadow * 0.6 + camera.sky_color.rgb * 0.7 + camera.ambient.xyz;
        let white = vec3<f32>(0.92, 0.94, 0.98) * light * ao;
        lit = mix(lit, white, cover * (0.55 + 0.35 * tex.a));
    }
    let dist = distance(in.world, camera.cam_pos.xyz);
    let f = 1.0 - exp(-fog_distance(in.world) * camera.fog.w);
    var rgb = mix(lit, camera.fog.xyz, clamp(f, 0.0, 1.0));
    if (camera.flags.z > 0.0) {
        // Never taken: flags.z (the old enhanced look's aerial perspective) is always 0
        // now - the enhanced path has its own fragment shader (enhanced.wgsl). The branch
        // stays because Metal's fast-math contracts the fog mix above differently without
        // it, and the vanilla picture is to stay what it was to the last code value.
        let vdir = normalize(in.world - camera.cam_pos.xyz);
        let towards_sun = clamp(dot(vdir, camera.sun_dir.xyz), 0.0, 1.0);
        let k = (1.0 - exp(-dist * 0.00045)) * camera.flags.z * (0.55 + 0.45 * clamp(camera.sun_dir.w, 0.0, 1.0));
        let scatter = mix(camera.sky_color.rgb * 1.4 + camera.ambient.rgb * 0.5, camera.sun_color.rgb * camera.sun_dir.w + camera.sky_color.rgb, towards_sun * towards_sun);
        rgb = mix(rgb, scatter, clamp(k * 0.6, 0.0, 0.5));
    }
    // (a NaN anywhere above - a zero-length vector normalised, 0 x infinity - was a black
    // patch on Windows, e.g. an EN92's headlights while off, and invisible on the Mac)
    rgb = finite_or(rgb, finite_or(albedo * material.color.rgb * diffuse, albedo));
    var a = tex.a * material.color.a;
    if (mode < 0.5) {
        a = 1.0;
    }
    a = a * in.params.x;
    return vec4<f32>(rgb, a);
}
