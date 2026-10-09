// Sky dome: the envir.cfg gradient textures (day / twilight / night), u = azimuth relative
// to the sun, v = elevation (0 zenith … 1 horizon), blended by the sun altitude.
struct Camera {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    // Kept in lockstep with `CameraUniform` in lib.rs.  The sky does not use the
    // floating-origin reconstruction itself, but omitting this member shifts every
    // following field one vec4 earlier: vanilla then reads the light-grid as `sky`
    // weights (usually all zero) and clears/draws a black sky.
    world_origin: vec4<f32>,
    sun_dir: vec4<f32>,
    ambient: vec4<f32>,
    fog: vec4<f32>,
    sun_color: vec4<f32>,
    sky_color: vec4<f32>,
    light_grid: vec4<f32>,
    sky: vec4<f32>,          // x sun azimuth (rad), y day weight, z twilight weight, w night weight
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    clouds: vec4<f32>,       // x density 0..1, yz texture offset (wind drift)
    light_view_proj: mat4x4<f32>,
    light_view_proj_far: mat4x4<f32>,
    shadow: vec4<f32>,
    post: vec4<f32>,         // x enhanced, y time
    inside_a: vec4<f32>,
    inside_b: vec4<f32>,
    inside_c: vec4<f32>,
    flags: vec4<f32>,
    light_view_proj_close: mat4x4<f32>,
    wind: vec4<f32>,
    // Enhanced: the street lamps' shadow maps (the tiles under the far map), and the
    // lights they belong to (-1: none)
    lamp_view_proj: array<mat4x4<f32>, 4>,
    lamp_shadow: vec4<f32>,
    tree_wind: vec4<f32>,
    inside2_a: vec4<f32>,
    inside2_b: vec4<f32>,
    inside2_c: vec4<f32>,
};

// A hash of a lattice point, from its integer bits.
//
// It was the old `fract(sin(dot(q, k)) * 43758)` trick, and that is what put flat slabs across
// the top of the sky. `cloud_fbm` below walks `q` up by 2.1 an octave from a world coordinate
// that is kilometres wide, and once the argument of the `sin` reaches a few times 1e5 a
// 32-bit float can no longer say where the fractional part of it lands: the hash comes out
// constant over a whole lattice cell, so the noise reads as flat squares with hard edges in a
// grid that turns with the camera - and only where the layer is thin enough to see through,
// which is why it showed as a "seam" in the sky "only when there are clouds": the high thin
// layer is the only thing up there in a clear sky, and it is faint. Integer bits do not lose
// precision with distance.
fn hash2(q: vec2<f32>) -> f32 {
    let x = bitcast<u32>(i32(floor(q.x)));
    let y = bitcast<u32>(i32(floor(q.y)));
    // (integer arithmetic in WGSL wraps, so these multiplications are modular already)
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0xffffu) / 65535.0;
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

// Cloud density field: the map's cloud texture as the large shape, fractal noise as the
// small one, two layers at different heights drifting at different speeds.
fn cloud_fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * vnoise(q);
        q = q * 2.1 + vec2<f32>(17.0, 9.0);
        a = a * 0.5;
    }
    return v;
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var t_day: texture_2d<f32>;
@group(1) @binding(1) var t_twilight: texture_2d<f32>;
@group(1) @binding(2) var t_night: texture_2d<f32>;
@group(1) @binding(3) var s_sky: sampler;
@group(1) @binding(4) var t_clouds: texture_2d<f32>;
@group(1) @binding(5) var s_repeat: sampler;

/// How far the cloud field reaches before it repeats (m): a cumulus is then half a
/// kilometre to two across.
const CLOUD_FIELD_TILE: f32 = 14000.0;

// The ground point under the sky at distance `t` along the view ray `d`, in the clouds' own
// frame (world coordinates modulo 70 km, lib.rs `CLOUD_ORIGIN_PERIOD`), so that the cloud
// field stays where it is when the floating render origin moves on.
// Where the clouds are seen from, relative to the camera (the enhanced sky cube's own eye
// while it is drawn; 0 everywhere else).
var<private> eye_off: vec3<f32> = vec3<f32>(0.0);

fn cloud_ground(d: vec3<f32>, t: f32) -> vec2<f32> {
    return camera.cam_pos.xy + eye_off.xy + camera.world_origin.zw + d.xy * t;
}

// The cloud cover over ground point p (x, 0..1, its edges frayed by the billows), how tall
// the cloud there grows (y, 0..1) and the cover without the billows (z: the enhanced sky
// raises its rounded tops on that, the billows would stand on them as spikes). From the
// cloud field: its equalised shape (G) cut at 1 - the cover, the weather's own picture (R)
// nudging where the clouds gather, billows (B) fraying the edges near by.
fn cloud_cover_at(p: vec2<f32>, lod: f32) -> vec3<f32> {
    let uv = p / CLOUD_FIELD_TILE + camera.clouds.yz * (2500.0 / CLOUD_FIELD_TILE);
    let t = textureSampleLevel(t_clouds, s_repeat, uv, lod);
    let thr = 1.0 - clamp(camera.clouds.x, 0.0, 1.0);
    let smooth_shape = t.g + (t.r - 0.5) * 0.15;
    var shape = smooth_shape;
    // the billows only where they are big enough to see
    let fray = 1.0 - smoothstep(2.0, 5.0, lod);
    if (fray > 0.0) {
        let detail = textureSampleLevel(t_clouds, s_repeat, uv * 3.7 + vec2<f32>(0.31, 0.73), lod + 1.9).b;
        shape = shape + (detail - 0.5) * 0.2 * fray;
    }
    return vec3<f32>(clamp((shape - thr) / 0.14, 0.0, 1.0), t.a, clamp((smooth_shape - thr) / 0.14, 0.0, 1.0));
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) dir: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VsOut {
    var out: VsOut;
    // the dome rides with the camera; pushed to the far plane (0 with reversed Z)
    let wp = camera.cam_pos.xyz + pos * 4000.0;
    var clip = camera.view_proj * vec4<f32>(wp, 1.0);
    clip.z = clip.w * 0.000001;
    out.clip = clip;
    out.dir = pos;
    return out;
}

// The classic sky's weather (lib.rs `VanillaSky`): the cloud type's own texture and
// cloud: x height H (m over the map's zero), y the type's size (m a tile; 0: no clouds),
// zw the wind's drift (m); haze: x the weather's fog range (m), y the visibility (m),
// z 1 for an overcast type, w the render origin's height.
struct VanillaSkyUniform {
    cloud: vec4<f32>,
    haze: vec4<f32>,
};
@group(1) @binding(9) var t_vclouds: texture_2d<f32>;
@group(1) @binding(10) var<uniform> vsky: VanillaSkyUniform;

// The v of the sky textures towards elevation `e` (rad), as Omsi.exe's dome
// (helper\skybox.x) has it: rings at the horizon, at 45 degrees and at the zenith with v
// 0.9833, 0.4868 and 0.0236, flat bands between them along which v runs linearly - along
// the chord, not with the angle - and the horizon's v all the way down below it.
fn dome_v(e: f32) -> f32 {
    if (e <= 0.0) {
        return 0.9833;
    }
    let upper = e > 0.7853982;
    let e0 = select(0.0, 0.7853982, upper);
    let e1 = select(0.7853982, 1.5707963, upper);
    let v0 = select(0.9833, 0.4868, upper);
    let v1 = select(0.4868, 0.0236, upper);
    // (where the ray at e meets the chord from the ring at e0 to the one at e1)
    let t = sin(e - e0) / ((sin(e1) - sin(e0)) * cos(e) - (cos(e1) - cos(e0)) * sin(e));
    return mix(v0, v1, clamp(t, 0.0, 1.0));
}

// The haze Omsi.exe lays over its sky (0x5d8e98): the dome again with Texture\nebel.tga,
// white with an alpha running linearly over its 32 rows from 0 at the zenith to 1 at the
// horizon, in the fog colour, its alpha moved by the fog range f and the cloud height H:
// down by 1 - 4 asin(H/f)/pi where f > H sqrt 2 (no haze at all under a clear sky, H 0),
// up by 1 - 4 acos(H/f)/pi below that, and the whole sky fog where f <= H.
fn vanilla_haze(v: f32) -> f32 {
    let f = vsky.haze.x;
    let h = vsky.cloud.x;
    if (f <= 0.0) {
        return 0.0;
    }
    if (f <= h) {
        return 1.0;
    }
    let tex_a = clamp((v * 32.0 - 0.5) / 31.0, 0.0, 1.0);
    let q = h / f;
    if (q < 0.70710678) {
        return clamp(tex_a - (1.0 - 4.0 * asin(q) / 3.14159265), 0.0, 1.0);
    }
    return clamp(tex_a + (1.0 - 4.0 * acos(q) / 3.14159265), 0.0, 1.0);
}

// The classic cloud layer towards d: rgb (in the sky's own terms, `fog` its fog colour) and
// how much it covers. Omsi.exe (0x754e44) draws helper\clouds.o3d, a flat cone over the
// camera - its apex H over the map's zero, its rim on the zero 9.363 H away - with the cloud
// type's texture laid on the ground plan, one tile every `size` metres, moved by the wind;
// its colour is the texture times the fog colour, its alpha the texture's (an `ovc` deck is
// opaque), and the fixed-function fog takes it into the fog colour by its depth in the
// view over the visibility.
fn vanilla_clouds(d: vec3<f32>, fog: vec3<f32>, tex: vec4<f32>, r: f32, encoded: bool) -> vec4<f32> {
    let hl = max(length(d.xy), 1e-4);
    // (the hit point lies along d, r / hl from the camera; its depth along the view's axis)
    let fwd = normalize(cross(camera.cam_up.xyz, camera.cam_right.xyz));
    let depth = r / hl * dot(d, fwd);
    let vis = max(vsky.haze.y, 1.0);
    let k = clamp(1.0 - depth / vis, 0.0, 1.0);
    var rgb = select(tex.rgb, srgb_encode(tex.rgb), encoded) * fog;
    rgb = mix(fog, rgb, k);
    return vec4<f32>(rgb, select(tex.a, 1.0, vsky.haze.z > 0.5));
}

// How far (over the ground plan) the ray d meets the cloud cone, and whether it does.
fn vanilla_cloud_reach(d: vec3<f32>) -> vec2<f32> {
    let h = vsky.cloud.x;
    let h0 = camera.cam_pos.z + vsky.haze.w;
    let hl = max(length(d.xy), 1e-4);
    let rim = 9.363 * h;
    let den = d.z / hl + 1.0 / 9.363;
    if (vsky.cloud.y <= 0.0 || h <= h0 || h <= 0.0) {
        return vec2<f32>(rim, 0.0);
    }
    // (below the rim the cone's skirt runs down under the horizon, behind the ground: its
    // rim stands in for it)
    var r = rim;
    if (den > 0.0) {
        r = min((h - h0) / den, rim);
    }
    return vec2<f32>(r, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = normalize(in.dir);
    // the dome turned so that the texture's column 0.5036 faces the sun, u growing
    // anticlockwise (from above) away from it (0x5d92a0: the dome rotated by the sun's
    // azimuth)
    let az = atan2(d.x, d.y);
    let u = 0.503645 + (camera.sky.x - az) / 6.2831853;
    let elev = asin(clamp(d.z, -1.0, 1.0));
    let v = dome_v(elev);
    let uv = vec2<f32>(u, v);
    // (Omsi.exe blends two of its pictures by the sun's height, see `Daylight::sky_weights`;
    // the sky textures have no mip levels, so the seam of u behind the sun is not seen)
    let sky = textureSample(t_day, s_sky, uv).rgb * camera.sky.y + textureSample(t_twilight, s_sky, uv).rgb * camera.sky.z + textureSample(t_night, s_sky, uv).rgb * camera.sky.w;
    // the cloud texture on the ground plan where the ray meets the cone, sampled outside
    // any branch (its mip levels by the screen's derivatives)
    let reach = vanilla_cloud_reach(d);
    let hl = max(length(d.xy), 1e-4);
    let ground = camera.cam_pos.xy + camera.world_origin.zw + d.xy / hl * reach.x;
    let size = max(vsky.cloud.y, 1.0);
    let cuv = (ground + vsky.cloud.zw) / size + vec2<f32>(0.5, -0.5) * (vsky.cloud.x / size);
    let ctex = textureSample(t_vclouds, s_repeat, vec2<f32>(cuv.x, cuv.y));
    // Vanilla blends in the 8-bit sRGB terms of Omsi.exe's fixed-function stages,
    // Vanilla+ in linear light
    let encoded = camera.sky_color.w > 0.5;
    let fog = camera.fog.xyz;
    var col = select(sky, srgb_encode(sky), encoded);
    col = mix(col, fog, vanilla_haze(v));
    if (reach.y > 0.5) {
        let c = vanilla_clouds(d, fog, ctex, reach.x, encoded);
        col = mix(col, c.rgb, c.a);
    }
    if (encoded) {
        return vec4<f32>(srgb_decode(col), 1.0);
    }
    return vec4<f32>(col, 1.0);
}
