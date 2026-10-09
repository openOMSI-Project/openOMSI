// Enhanced graphics: the light the air scatters towards the camera that the main pass
// leaves out - the lamps' in the weather's fog, rain or falling snow (`lamp_airlight` in
// lamp_air.wgsl) and the sun's shafts between the shadows (`sun_shafts`) - added onto the
// drawn picture before its glow and its metering. Once per pixel over the depth prepass: worked out in
// the main pass it was paid again for every layer drawn over a pixel (painted ground,
// decals), several milliseconds a frame in a foggy night.
struct Camera {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    // (kept in lockstep with `CameraUniform` in lib.rs, as sky.wgsl's)
    world_origin: vec4<f32>,
    sun_dir: vec4<f32>,
    ambient: vec4<f32>,
    fog: vec4<f32>,
    sun_color: vec4<f32>,
    sky_color: vec4<f32>,
    light_grid: vec4<f32>,
    sky: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    clouds: vec4<f32>,
    light_view_proj: mat4x4<f32>,
    light_view_proj_far: mat4x4<f32>,
    shadow: vec4<f32>,
    post: vec4<f32>,
    inside_a: vec4<f32>,
    inside_b: vec4<f32>,
    inside_c: vec4<f32>,
    flags: vec4<f32>,
    light_view_proj_close: mat4x4<f32>,
    wind: vec4<f32>,
    lamp_view_proj: array<mat4x4<f32>, 4>,
    lamp_shadow: vec4<f32>,
    tree_wind: vec4<f32>,
    inside2_a: vec4<f32>,
    inside2_b: vec4<f32>,
    inside2_c: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct FogLampParams {
    // clip space (reversed Z) back to the render-origin relative world
    inv_view_proj: mat4x4<f32>,
    // xy the picture's size in pixels, z 1: the sun's glare is added, w the share of the
    // weather's extinction that is mist and fog (not rain)
    size: vec4<f32>,
};
@group(1) @binding(0) var<uniform> fp: FogLampParams;
@group(1) @binding(1) var t_depth: texture_depth_2d;
// the half-size result (rgb pre-exposed light, a the distance it was worked out to)
@group(1) @binding(2) var t_fog: texture_2d<f32>;

struct FogVsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_fog_lamps(@builtin(vertex_index) i: u32) -> FogVsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var out: FogVsOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// The camera, the direction through a full-size pixel position and the distance to what
// the prepass holds there (3 km for the sky).
fn pixel_ray(at: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t_depth));
    let px = clamp(vec2<i32>(at), vec2<i32>(0), dims - vec2<i32>(1));
    let d = textureLoad(t_depth, px, 0);
    let ndc = vec2<f32>(at.x / fp.size.x * 2.0 - 1.0, 1.0 - at.y / fp.size.y * 2.0);
    let c = camera.cam_pos.xyz;
    let mid = fp.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let dir = normalize(mid.xyz / mid.w - c);
    var len = 3000.0;
    if (d > 0.0) {
        let h = fp.inv_view_proj * vec4<f32>(ndc, d, 1.0);
        len = distance(h.xyz / h.w, c);
    }
    return vec4<f32>(dir, len);
}

// The sun's shadow maps (shader.wgsl's bindings and constants).
@group(0) @binding(5) var t_shadow: texture_depth_2d;
@group(0) @binding(6) var s_shadow: sampler_comparison;
@group(0) @binding(7) var t_shadow_far: texture_depth_2d;
const SHAFT_DEPTH_RANGE: f32 = 2199.0;
const SHAFT_STEPS: u32 = 16u;
// how far the shafts are followed (the near cascade's reach, a little less)
const SHAFT_REACH: f32 = 180.0;

// Whether the sun reaches a point in the air (1) or a house or a crown stands between (0).
fn sun_reaches(x: vec3<f32>) -> f32 {
    let lp = camera.light_view_proj * vec4<f32>(x, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (max(abs(uv.x - 0.5), abs(uv.y - 0.5)) < 0.48 && lp.z >= 0.0 && lp.z <= 1.0) {
        return textureSampleCompareLevel(t_shadow, s_shadow, vec2<f32>(uv.x * 0.5, uv.y), lp.z - 0.15 / SHAFT_DEPTH_RANGE);
    }
    let fp2 = camera.light_view_proj_far * vec4<f32>(x, 1.0);
    let fuv = vec2<f32>(fp2.x * 0.5 + 0.5, 0.5 - fp2.y * 0.5);
    if (fuv.x < 0.0 || fuv.x > 1.0 || fuv.y < 0.0 || fuv.y > 1.0 || fp2.z < 0.0 || fp2.z > 1.0) {
        return 1.0;
    }
    // (the far map takes the top of its texture, see shader.wgsl `shadow_pcf_far`)
    return textureSampleCompareLevel(t_shadow_far, s_shadow, vec2<f32>(fuv.x, fuv.y * 0.8), fp2.z - 0.6 / SHAFT_DEPTH_RANGE);
}

// Shafts of sunlight - crepuscular rays: the air between the camera and what it sees
// scatters the sun towards the eye wherever the sun reaches it, and not in the shadow of a
// house or a tree. The picture's air (`air` in enhanced_common.wgsl) holds that light as
// if the sun reached the air everywhere; along the first stretch of the ray, which the
// shadow maps cover, what lies in shadow is taken out again here (a negative radiance):
// lit and dark shafts through a hazy evening, a misty morning wood, a dusty street. The
// air scatters the way `air` has it - the weather's fog with g 0.55, the clear air's haze
// and dust forward with g 0.7 (its Rayleigh part is small near the ground).
fn sun_shafts(c: vec3<f32>, d: vec3<f32>, len: f32, jitter: f32) -> vec3<f32> {
    let s = camera.sun_dir.xyz;
    let e_sun = enh.sun.rgb;
    if (camera.shadow.x < 0.5 || s.z <= 0.0 || max(e_sun.r, e_sun.g) < 1e-4) {
        return vec3<f32>(0.0);
    }
    let end = min(len, SHAFT_REACH);
    let dt = end / f32(SHAFT_STEPS);
    let cos_t = dot(s, d);
    let p_fog = hg_phase(cos_t, 0.55) * 0.9;
    let p_air = hg_phase(cos_t, 0.7);
    let h0 = c.z - enh.fog.z;
    // (the most the shadows could take out along the way, on the screen: where that is
    // too little to see - most of the picture, looking away from the sun - nothing is done)
    let most = (enh.fog.x * p_fog + enh.fog.w * p_air) * max(e_sun.r, e_sun.g) * end * enh.exposure.x;
    if (most < 0.004) {
        return vec3<f32>(0.0);
    }
    var shade = vec3<f32>(0.0);
    for (var i = 0u; i < SHAFT_STEPS; i = i + 1u) {
        let t = (f32(i) + jitter) * dt;
        let x = c + d * t;
        let lit = sun_reaches(x);
        if (lit >= 0.999) {
            continue;
        }
        let h = max(x.z - enh.fog.z, 0.0);
        let s_fog = enh.fog.x * exp(-enh.fog.y * h);
        let s_air = enh.fog.w * exp(-h / 1500.0);
        let tau = layer_depth(enh.fog.x, enh.fog.y, h0, h, t) + layer_depth(enh.fog.w, 1.0 / 1500.0, h0, h, t);
        shade = shade + vec3<f32>((1.0 - lit) * (s_fog * p_fog + s_air * p_air) * exp(-tau) * dt);
    }
    return -shade * e_sun;
}

// at half size: one ray per four pixels
@fragment
fn fs_fog_lamps(in: FogVsOut) -> @location(0) vec4<f32> {
    // (the texture's last row: how much of the sun is seen, worked out once for the frame
    // in its first texel)
    // (t_fog is not this pass's target: the row is the one under the half picture)
    if (i32(in.clip.y) >= i32(ceil(fp.size.y * 0.5))) {
        var seen = 0.0;
        if (i32(in.clip.x) == 0 && fp.size.z > 0.5 && camera.sun_dir.z > -0.02) {
            seen = sun_seen(camera.cam_pos.xyz, camera.sun_dir.xyz);
        }
        return vec4<f32>(seen, 0.0, 0.0, 0.0);
    }
    let r = pixel_ray(floor(in.clip.xy) * 2.0 + vec2<f32>(0.5));
    let ign = fract(52.9829189 * fract(dot(floor(in.clip.xy), vec2<f32>(0.06711056, 0.00583715))));
    // (the lamps' halo and cones only from the mist's and fog's droplets: fp.size.w)
    let l = (lamp_airlight(camera.cam_pos.xyz, r.xyz, 0.0, r.w, ign) * fp.size.w + sun_shafts(camera.cam_pos.xyz, r.xyz, r.w, ign)) * enh.exposure.x;
    return vec4<f32>(clamp(l, vec3<f32>(-4000.0), vec3<f32>(4000.0)), r.w);
}

// How much of the sun's disc is seen (0..1): houses, trees and the bus in front of it
// are in the depth prepass (the clouds and the fog are in `enh.sun` already).
fn sun_seen(c: vec3<f32>, s: vec3<f32>) -> f32 {
    let r = enh.sun.w;
    let up0 = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(s.z) > 0.9);
    let a = normalize(cross(s, up0));
    let b = cross(s, a);
    var seen = 0.0;
    var n = 0.0;
    for (var k = 0; k < 13; k = k + 1) {
        var o = vec2<f32>(0.0);
        if (k > 0) {
            let ang = f32(k) * 2.3999632;
            o = vec2<f32>(cos(ang), sin(ang)) * sqrt(f32(k) / 12.0) * 0.9;
        }
        let dir = normalize(s + (a * o.x + b * o.y) * r);
        let clip = camera.view_proj * vec4<f32>(c + dir * 1000.0, 1.0);
        if (clip.w <= 0.0) {
            return 0.0;
        }
        let ndc = clip.xy / clip.w;
        // (beyond the picture's edge nothing is known: what stands at the edge is taken)
        let px = vec2<f32>((ndc.x * 0.5 + 0.5) * fp.size.x, (0.5 - ndc.y * 0.5) * fp.size.y);
        let dims = vec2<i32>(textureDimensions(t_depth));
        let q = clamp(vec2<i32>(px), vec2<i32>(0), dims - vec2<i32>(1));
        seen = seen + select(0.0, 1.0, textureLoad(t_depth, q, 0) <= 0.0);
        n = n + 1.0;
    }
    return seen / n;
}

// Glare: the eye's own scattered light round a bright source - the sun seen directly
// blinds, a veil of light spreads over everything near it and the shadows beside it are
// lost. The veiling luminance per unit of the source's illuminance at the eye is the CIE's
// general disability glare function (CIE 146:2002, Vos and van den Berg): 10/theta^3 +
// (5/theta^2 + 0.1 p/theta)(1 + (A/62.5)^4) + 0.0025 p, theta in degrees (a young eye,
// A 25, of middle pigmentation, p 0.5). Close to the source the scattering by the lens
// fibres and the vitreous shows as a ciliary corona of fine radial streaks and, a couple
// of degrees out, the lenticular halo - a faint ring, red outside and blue inside
// (Simpson 1953; Spencer, Shirley, Zimmerman and Greenberg, "Physically-based glare
// effects for digital images", SIGGRAPH 1995).
fn sun_glare(dir: vec3<f32>) -> vec3<f32> {
    let s = camera.sun_dir.xyz;
    if (max(enh.sun.r, enh.sun.g) < 1e-5 || s.z < -0.02) {
        return vec3<f32>(0.0);
    }
    let cos_t = clamp(dot(dir, s), -1.0, 1.0);
    let theta = max(degrees(acos(cos_t)), 0.25);
    if (theta > 100.0) {
        return vec3<f32>(0.0);
    }
    let seen = textureLoad(t_fog, vec2<i32>(0, i32(textureDimensions(t_fog).y) - 1), 0).r;
    if (seen <= 0.0) {
        return vec3<f32>(0.0);
    }
    let e = enh.sun.rgb * seen;
    let age = 1.0 + pow(25.0 / 62.5, 4.0);
    let pig = 0.5;
    var f = 10.0 / (theta * theta * theta) + (5.0 / (theta * theta) + 0.1 * pig / theta) * age + 0.0025 * pig;
    // the ciliary corona: streaks round the source, fading by some ten degrees
    let up0 = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(s.z) > 0.9);
    let a = normalize(cross(s, up0));
    let b = cross(s, a);
    let around = atan2(dot(dir, b), dot(dir, a));
    let k = around * 57.29578 * 1.5;
    let h1 = fract(sin(floor(k) * 12.9898) * 43758.547);
    let h2 = fract(sin((floor(k) + 1.0) * 12.9898) * 43758.547);
    let streak = mix(h1, h2, smoothstep(0.0, 1.0, fract(k)));
    let corona = 1.0 + 1.2 * (pow(streak, 3.0) - 0.25) * exp(-theta / 4.0);
    f = f * max(corona, 0.2);
    // the lenticular halo (its radius per wavelength: 615, 550 and 465 nm)
    let rings = vec3<f32>(2.35, 2.15, 1.9);
    let dr = (vec3<f32>(theta) - rings) / 0.12;
    let halo = 0.25 * (5.0 / (theta * theta)) * exp(-dr * dr);
    return e * f + e * halo;
}

// at full size, added onto the picture: the four nearest half-size rays, each as far as
// it was worked out to a distance like this pixel's (the light in front of a near pole
// is not the light in front of the houses behind it)
@fragment
fn fs_fog_composite(in: FogVsOut) -> @location(0) vec4<f32> {
    let ray = pixel_ray(in.clip.xy);
    let len = ray.w;
    let hp = in.clip.xy * 0.5 - vec2<f32>(0.5);
    let base = vec2<i32>(floor(hp));
    let f = hp - floor(hp);
    // (the last row is the sun's, see `fs_fog_lamps`)
    let hd = vec2<i32>(textureDimensions(t_fog)) - vec2<i32>(1, 2);
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    // (a twig or a wire none of the four rays met: the one nearest in distance)
    var best = vec3<f32>(0.0);
    var best_d = 1e9;
    // (a tent over the 4 x 4 nearest: the edge of a headlight's beam in the haze, worked
    // out at half size, showed its 2 x 2 blocks under a plain bilinear one)
    for (var k = 0; k < 16; k = k + 1) {
        let o = vec2<i32>(k & 3, k >> 2) - vec2<i32>(1);
        let s = textureLoad(t_fog, clamp(base + o, vec2<i32>(0), hd), 0);
        let dx = abs(f32(o.x) - f.x);
        let dy = abs(f32(o.y) - f.y);
        let bil = max(1.0 - dx * 0.5, 0.0) * max(1.0 - dy * 0.5, 0.0);
        let dd = abs(s.a - len);
        let w = (bil + 1e-3) * exp(-dd / (0.05 * len + 0.5));
        sum = sum + s.rgb * w;
        wsum = wsum + w;
        if (dd < best_d) {
            best_d = dd;
            best = s.rgb * min(len / max(s.a, 0.1), 1.0);
        }
    }
    var air_light = sum / max(wsum, 1e-6);
    if (wsum < 1e-4) {
        air_light = best;
    }
    // the eye's glare from the sun, on top of everything (fp.size.z: on)
    var glare = vec3<f32>(0.0);
    if (fp.size.z > 0.5) {
        glare = sun_glare(ray.xyz) * enh.exposure.x;
    }
    return vec4<f32>(air_light + min(glare, vec3<f32>(4000.0)), 0.0);
}

