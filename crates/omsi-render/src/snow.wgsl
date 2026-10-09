// The snowfall, every mode: a field of flakes that lives on the GPU alone - each flake's
// place is worked out here from its number, the time, the wind and the camera, so that a
// heavy fall is a hundred and fifty thousand flakes for the cost of one draw (nothing is
// simulated or sent per flake). The field is the world's, repeating every few metres round
// the camera in three layers - dense close by, where every flake is a few pixels wide,
// thinner further out, where it is a speck; beyond them the snowfall's extinction takes the
// view (Enhanced). Flakes are a few millimetres across (an exponential spread, Gunn and
// Marshall 1958; a light fall single crystals of a millimetre), fall at about a metre a
// second (aggregates at 0.8 D^0.16 m/s, Locatelli and Hobbs 1974), sway as they go and
// drift with the wind; opaque and lit by what lights them - the classic picture's
// envir.cfg light, or the enhanced picture's sky, sun and lamps (`precip_light`).
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

struct SnowParams {
    // xyz the wind the flakes drift with (m/s), w the time (s)
    wind: vec4<f32>,
    // x how hard it snows (0..1), y the flakes' mean diameter (m), zw how many flakes the
    // first two layers hold (the rest are the third's)
    fall: vec4<f32>,
};
@group(1) @binding(0) var<uniform> sp: SnowParams;

struct SnowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // the flake's cover (alpha), and its place (for its light)
    @location(1) cover: f32,
    @location(2) pos: vec3<f32>,
};

// The layers round the camera: half width, bottom and top (m).
const LAYER_H = array<vec3<f32>, 3>(vec3<f32>(4.0, -2.5, 5.5), vec3<f32>(10.0, -3.0, 9.0), vec3<f32>(24.0, -3.0, 16.0));

fn hash4(n: u32) -> vec4<f32> {
    var x = vec4<u32>(n, n * 747796405u + 2891336453u, n ^ 0x9e3779b9u, n * 1664525u + 1013904223u);
    x = x * 1664525u + 1013904223u;
    x.x = x.x + x.y * x.w;
    x.y = x.y + x.z * x.x;
    x.z = x.z + x.x * x.y;
    x.w = x.w + x.y * x.z;
    x = x ^ (x >> vec4<u32>(16u));
    x.x = x.x + x.y * x.w;
    x.y = x.y + x.z * x.x;
    x.z = x.z + x.x * x.y;
    return vec4<f32>(x & vec4<u32>(0xffffffu)) / 16777216.0;
}

// 1 inside the player's own vehicle (its box, a little wider): no snow falls in the cab
fn in_own_vehicle(p: vec3<f32>) -> bool {
    // (the front and, of an articulated bus, the rear section: it snowed in the back, #1967;
    // without branches - two boxes tested with early returns and short-circuits brought
    // Apple's M4 shader compiler service down, XPC_ERROR_CONNECTION_INTERRUPTED, and the
    // game fell back to no snowfall)
    return any(vec2<bool>(in_box(p, camera.inside_a, camera.inside_b, camera.inside_c), in_box(p, camera.inside2_a, camera.inside2_b, camera.inside2_c)));
}

fn in_box(p: vec3<f32>, a: vec4<f32>, b: vec4<f32>, c: vec4<f32>) -> bool {
    let d = p - a.xyz;
    let sh = a.w;
    let ch = b.x;
    let q = vec3<f32>(d.x * ch - d.y * sh - c.x, d.x * sh + d.y * ch - c.y, d.z - c.z);
    let h = b.yzw + vec3<f32>(0.3);
    return select(false, all(abs(q) < h), c.w >= 0.5);
}

@vertex
fn vs_snow(@builtin(vertex_index) vid: u32, @builtin(instance_index) k: u32) -> SnowOut {
    let corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vid % 6u];
    var out: SnowOut;
    out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    let n0 = u32(sp.fall.z);
    let n1 = u32(sp.fall.w);
    var layer = 2u;
    if (k < n0) {
        layer = 0u;
    } else if (k < n0 + n1) {
        layer = 1u;
    }
    let lh = LAYER_H[layer];
    let size = vec3<f32>(2.0 * lh.x, 2.0 * lh.x, lh.z - lh.y);
    let h1 = hash4(k * 2u + 1u);
    let h2 = hash4(k * 2u + 7919u);
    // the flake: its diameter, fall speed and sway
    let r = sp.fall.x;
    let d = min(0.0005 - sp.fall.y * log(max(1.0 - h1.w, 1e-6)), 0.015);
    let fall = max(0.8 * pow(d * 1000.0, 0.16) * (0.75 + 0.25 * r) * (0.85 + 0.3 * h2.x), 0.3);
    let t = sp.wind.w;
    let pace = 1.5 + 2.5 * h2.y;
    let sway = 0.05 + 0.15 * h2.z;
    let ph = h2.w * 6.2831853 + pace * t;
    // its place in the world: where it started, carried by the wind, swaying round, falling;
    // then the copy of the field nearest the camera (the field's figures in the world's
    // own, modulo a kilometre, so that it stays put while the render origin moves)
    let world_cam = camera.cam_pos.xyz + vec3<f32>(camera.world_origin.xy, 0.0);
    let start = h1.xyz * size;
    let drift = sp.wind.xyz * (0.6 * t) + vec3<f32>(cos(ph), sin(ph), 0.0) * sway - vec3<f32>(0.0, 0.0, fall * t);
    let lo = world_cam + vec3<f32>(-lh.x, -lh.x, lh.y);
    let q = lo + (((start + drift - lo) % size) + size) % size;
    let p = q - vec3<f32>(camera.world_origin.xy, 0.0);
    if (in_own_vehicle(p)) {
        return out;
    }
    // a denser layer fades out towards its edges, into the thinner one round it
    let rel = q - lo;
    let soft = lh.x * 0.3;
    var fade = 1.0;
    if (layer < 2u) {
        let e = min(rel, size - rel) / soft;
        fade = clamp(min(min(e.x, e.y), e.z), 0.0, 1.0);
    }
    let to_cam = camera.cam_pos.xyz - p;
    let dist = length(to_cam);
    let vdir = to_cam / max(dist, 0.001);
    // a flake close by is out of focus (its blur as wide as the pupil, some 2.5 mm), one far
    // off at least a pixel and a half wide; as bright as it is seen - a small point's
    // brightness goes with the square root of its light (Stevens' power law)
    let rad = d * 0.5;
    let rs = max(sqrt(rad * rad + 0.0025 * 0.0025), dist * 0.0016);
    out.cover = fade * sqrt(rad / rs);
    if (out.cover < 0.004) {
        return out;
    }
    var ax = cross(vec3<f32>(0.0, 0.0, 1.0), vdir);
    ax = select(vec3<f32>(1.0, 0.0, 0.0), normalize(ax), length(ax) > 1e-4);
    let ay = cross(vdir, ax);
    out.clip = camera.view_proj * vec4<f32>(p + (ax * c.x + ay * c.y) * rs, 1.0);
    out.uv = c;
    out.pos = p;
    return out;
}

fn flake_alpha(in: SnowOut) -> f32 {
    let r = length(in.uv);
    return clamp(in.cover * (1.0 - smoothstep(0.55, 1.0, r)), 0.0, 1.0);
}

// the classic picture: white, lit as the scene is (envir.cfg's ambient and sun light)
@fragment
fn fs_snow(in: SnowOut) -> @location(0) vec4<f32> {
    let a = flake_alpha(in);
    let light = min(camera.ambient.rgb + camera.sun_color.rgb * 0.5, vec3<f32>(1.0));
    return vec4<f32>(light * 0.9 * a, a);
}

// the enhanced picture: what the flake scatters of the sky's, the sun's and the lamps'
// light, at the albedo of snow
@fragment
fn fs_snow_enhanced(in: SnowOut) -> @location(0) vec4<f32> {
    let a = flake_alpha(in);
    let to_eye = normalize(camera.cam_pos.xyz - in.pos);
    // (a flake hangs under the sky: its light from above as much as from all round)
    let l = (precip_light(in.pos, to_eye, true) + sh_irradiance(vec3<f32>(0.0, 0.0, 1.0)) / PI) * 0.5 * 0.85 * enh.exposure.x;
    return vec4<f32>(min(l, vec3<f32>(60.0)) * a, a);
}
