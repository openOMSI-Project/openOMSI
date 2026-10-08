// Snow on the carriageways: how far it has built up, the ruts the traffic keeps in it and
// the fresh tracks the tyres press into it (see `Renderer::set_snow_track_rect`).
//
// The field is a square of `place.z` metres that wraps around the world (a torus): texel
// (i, j) holds the world's point (x, y) where floor(x / texel) and floor(y / texel) are i and
// j modulo its size, so the app only rewrites the tiles the camera moves into. It is kept
// tile after tile (`TRACK_TILE` texels a side), each row after row, a texel an RGBA8 word:
//   r: the ruts of the lanes' wheel tracks (0..1)
//   g: a tyre has pressed the snow here (how much of the texel it covered)
//   b, a: how much snow had fallen when a tyre last ran here (16 bits, see `state.y`; for
//         a rut no tyre has run in yet, a moment as long ago as its lane's traffic makes
//         likely): the track and the rut fill as more falls

struct SnowTrack {
    // xy: the render origin modulo the field's side (m), z: the side (m), w: 1 while the
    // field holds the area around the camera
    place: vec4<f32>,
    // x: how far the snow covers the roads (0..1; below 0 the weather's "snow on road"
    // decides, as before), y: the snow fallen so far (thousandths of a full cover, modulo
    // 65536), z: how deep the lanes' ruts show (0..1), w: unused
    state: vec4<f32>,
};

@group(0) @binding(20) var<storage, read> snow_track_px: array<u32>;
@group(0) @binding(21) var<uniform> snow_track: SnowTrack;

const TRACK_TEXELS: i32 = 2048;
const TRACK_TILE: i32 = 256;

// One texel's rut and fresh track, each filled by the snow fallen since a tyre last ran
// over it.
fn snow_track_texel(p: vec2<i32>) -> vec2<f32> {
    let q = ((p % TRACK_TEXELS) + TRACK_TEXELS) % TRACK_TEXELS;
    let tile = (q.y / TRACK_TILE) * (TRACK_TEXELS / TRACK_TILE) + q.x / TRACK_TILE;
    let i = (tile * TRACK_TILE + q.y % TRACK_TILE) * TRACK_TILE + q.x % TRACK_TILE;
    let t = unpack4x8unorm(snow_track_px[i]);
    let then = round(t.b * 255.0) * 256.0 + round(t.a * 255.0);
    let since = (snow_track.state.y - then + 65536.0) % 65536.0;
    // (a track begins to blur after 8 % of a cover has fallen on it and is gone at 28 %; a
    // rut, worn deeper by the traffic, begins to fill at 15 % and is gone at 52 % - in a
    // heavy snowfall (Starker Schneefall) one or two minutes, and four or seven, without a
    // car: a driver who waits that long at a stop sees it happen)
    let fresh = t.g * (1.0 - smoothstep(80.0, 280.0, since));
    let rut = t.r * (1.0 - smoothstep(150.0, 520.0, since));
    return vec2<f32>(rut, fresh);
}

// The rut (x) and the fresh track (y) at a point of the render frame, blended between the
// four texels around it (each decoded first: the fill time does not blend).
fn snow_track_at(world: vec2<f32>) -> vec2<f32> {
    if (snow_track.place.w < 0.5) {
        return vec2<f32>(0.0);
    }
    let q = (world + snow_track.place.xy) * (f32(TRACK_TEXELS) / snow_track.place.z) - 0.5;
    let b = floor(q);
    let f = q - b;
    let i = vec2<i32>(b);
    let a00 = snow_track_texel(i);
    let a10 = snow_track_texel(i + vec2<i32>(1, 0));
    let a01 = snow_track_texel(i + vec2<i32>(0, 1));
    let a11 = snow_track_texel(i + vec2<i32>(1, 1));
    return mix(mix(a00, a10, f.x), mix(a01, a11, f.x), f.y);
}

// The snow on a vehicle's part (`code`: its instance's surface code, below -500, whose ten
// thousands carry how much snow its roof has gathered, 0..40, see lib.rs `roof_snow`),
// at `local`, the point in the part's own mesh: patches at first, a closed layer as it
// goes on, laid out on the vehicle so that they ride with it.
fn vehicle_snow(code: f32, local: vec3<f32>) -> f32 {
    let amount = floor(-code / 10000.0) / 40.0;
    if (amount <= 0.0) {
        return 0.0;
    }
    let p = local.xy;
    let p2 = vec2<f32>(p.x * 0.8 - p.y * 0.6, p.x * 0.6 + p.y * 0.8);
    let patches = vnoise_f(p, 0.9, vec2<f32>(4.3, 1.9)) * 0.65 + vnoise_f(p2, 3.1, vec2<f32>(8.1, 6.7)) * 0.35;
    return smoothstep(patches - 0.3, patches + 0.08, amount * 1.3 - 0.12);
}

// Whether the roads' snow is the built-up kind (`state.x` at or above 0) rather than the
// weather's on/off "snow on road".
fn road_snow_dynamic() -> bool {
    return snow_track.state.x >= 0.0;
}

// The snow on a carriageway at `world` (render frame): x the cover (0..1), y the slush in
// the ruts (0..1: the wheels keep it wet and grey), z a fresh tyre track (0..1).
fn road_snow(world: vec3<f32>) -> vec3<f32> {
    let base = clamp(snow_track.state.x, 0.0, 1.0);
    if (base <= 0.0) {
        return vec3<f32>(0.0);
    }
    // the map's own coordinates (modulo the pattern's period): the patches stay put
    let m = world.xy + camera.world_origin.xy;
    // It does not settle evenly: first a dusting in patches a stride or two across and in
    // the grain of the asphalt, which close up as it goes on falling.
    // (three octaves, the finer ones turned against the coarse: a single lattice's cells
    // showed as square blots where the cover's edge ran through them)
    let m2 = vec2<f32>(m.x * 0.8 - m.y * 0.6, m.x * 0.6 + m.y * 0.8);
    let m3 = vec2<f32>(m.x * 0.28 + m.y * 0.96, m.y * 0.28 - m.x * 0.96);
    let patches = vnoise_f(m, 0.45, vec2<f32>(3.1, 7.7)) * 0.55 + vnoise_f(m2, 1.3, vec2<f32>(11.3, 2.9)) * 0.3 + vnoise_f(m3, 3.7, vec2<f32>(5.9, 13.1)) * 0.15;
    var cover = smoothstep(patches - 0.35, patches + 0.1, base * 1.35 - 0.15);
    let tracks = snow_track_at(world.xy);
    // The ruts: the wheels of the traffic keep two strips of each lane down to slush, the
    // more of it the more snow there is to churn; with a mere dusting they show the asphalt.
    let rut = tracks.x * snow_track.state.z;
    let slush = rut * smoothstep(0.15, 0.7, base);
    cover = cover * (1.0 - rut * 0.85);
    // A fresh track: the tread pressed the snow flat and grey. Its edge sharpened from the
    // texels' blend (a soft ramp over a whole texel read as a blur), and the pressed snow
    // grainy - the tread's blocks, as fine as the eye can still tell apart.
    let edge = smoothstep(0.2, 0.75, tracks.y);
    let grain = 0.75 + 0.25 * vnoise_f(m2, 14.0, vec2<f32>(1.7, 9.3));
    let track = edge * grain;
    cover = cover * (1.0 - track * 0.6);
    return vec3<f32>(cover, max(slush, track * 0.5 * base), track);
}
