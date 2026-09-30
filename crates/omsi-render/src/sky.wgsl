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
};

fn hash2(q: vec2<f32>) -> f32 {
    return fract(sin(dot(q, vec2<f32>(127.1, 311.7))) * 43758.5453);
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = normalize(in.dir);
    let az = atan2(d.x, d.y);
    let u = fract((az - camera.sky.x) / 6.2831853 + 0.5);
    let elev = asin(clamp(d.z, -1.0, 1.0));
    // horizon row at the bottom; below the horizon keep the horizon colour
    let v = clamp(1.0 - elev / 1.5707963, 0.0, 0.995);
    let uv = vec2<f32>(u, v);
    let c = textureSample(t_day, s_sky, uv).rgb * camera.sky.y + textureSample(t_twilight, s_sky, uv).rgb * camera.sky.z * 0.8 + textureSample(t_night, s_sky, uv).rgb * camera.sky.w * 0.6;
    var col = c;
    if (camera.clouds.x > 0.001 && d.z > 0.01) {
        // a flat layer 1500 m up drawn from the cloud field (weather_setup::cloud_field)
        let t = 1500.0 / d.z;
        let p = cloud_ground(d, t);
        // how many texels of the field a pixel covers there: the far clouds are drawn from a
        // smaller mip (no shimmering, no grain)
        let lod = log2(max(t * length(fwidth(d)) / max(d.z, 0.05) / CLOUD_FIELD_TILE * 512.0, 1.0));
        let c = cloud_cover_at(p, lod);
        // a closing cover (Overcast: a sky that rains) is one grey deck, no blue between
        let closed = smoothstep(0.8, 1.0, camera.clouds.x);
        let cover = mix(smoothstep(0.0, 1.0, c.x), 1.0, closed) * clamp((d.z - 0.01) * 7.0, 0.0, 1.0);
        // lit by the same light as the scene (envir.cfg's sun and ambient at this sun
        // height): white by day, warm at sunrise, and at night barely lighter than the night
        // sky - never the pale blots a fixed twilight colour made at night; a little grey in
        // the thick middle of a big cloud
        let core = 1.0 - 0.2 * smoothstep(0.45, 1.0, c.x) * (0.5 + 0.5 * c.y);
        let sun_up = clamp(camera.sun_dir.z * 4.0 + 0.3, 0.0, 1.0);
        let lit = camera.sun_color.rgb * 1.1 * sun_up + camera.ambient.rgb * mix(0.5, 1.5, sun_up) + camera.sky_color.rgb * 0.3;
        var cloud_col = max(min(lit, vec3<f32>(0.97)) * core, col * 1.12);
        // the deck greys over
        cloud_col = cloud_col * (1.0 - 0.3 * closed) * mix(1.0, 0.85 + 0.3 * c.x, closed);
        col = mix(col, cloud_col, cover);
    }
    // fog swallows the horizon, and a thick fog (a few hundred metres of sight) the whole
    // sky: the blue does not show through ground fog
    let horizon = clamp(1.0 - elev / 0.12, 0.0, 1.0) * clamp(camera.fog.w * 1500.0, 0.0, 1.0);
    let whole = clamp(camera.fog.w * 150.0 - 0.15, 0.0, 1.0) * clamp(1.0 - elev / 1.2, 0.35, 1.0);
    let f = max(horizon, whole);
    return vec4<f32>(mix(col, camera.fog.xyz, f), 1.0);
}
