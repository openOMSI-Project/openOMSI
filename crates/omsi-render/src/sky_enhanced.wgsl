// Enhanced graphics: the sky - the physical sky table, clouds lit by the sun and the sky,
// the sun's disc and the air - and the reflection probe drawn from it.

// Clouds: a ray-marched volume on the curved Earth, built the way Guerrilla's Horizon Zero
// Dawn clouds are (and bevy-volumetric-clouds and Frostbite after them):
// * the layer is a spherical shell 1400-2800 m over the ground of an Earth of 6371 km, so
//   the clouds sink to the horizon and thin out into the distance as real ones do;
// * the shape map (clouds.rs: Perlin cut by Worley cells, repeating every 13 km) says where
//   the heaps stand and how much cover each needs to appear; its blue channel and a height
//   gradient round their tops and flatten their bases; the weather's cover (Cumulus 1-3,
//   Overcast) and the weather's own cloud picture nudge how much of it appears;
// * the detail volume (3-D Worley billows, repeating every 420 m) eats the edges, most at
//   the thin rims and least in the dense cores;
// * light: the sun through the cloud towards it (six steps growing 1.7 times, 1.4 km in
//   all) with Hillaire's approximation of multiple scattering (three octaves of weaker
//   extinction and a flatter phase), a two-lobe Henyey-Greenstein phase (a silver lining
//   towards the sun, a little back-scatter away from it), the sky's light from above and
//   the ground's from below by height, integrated per step the energy-conserving
//   Frostbite way; far clouds take on the air's colour.
// The sky cube keeps the result in a fixed world frame (lib.rs `SKY_CUBE_SIZE`): each
// redraw starts the steps at another random point and is blended into what is there, so
// the grain of a few dozen steps averages out over the frames.
const CLOUD_BOTTOM: f32 = 1400.0;
const CLOUD_TOP: f32 = 2800.0;
const EARTH_R: f32 = 6371000.0;
// Extinction per metre of the densest cloud.
const CLOUD_SIGMA: f32 = 0.035;
const CLOUD_SHAPE_PERIOD: f32 = 13000.0;
const CLOUD_DETAIL_PERIOD: f32 = 420.0;
const CLOUD_DETAIL_STRENGTH: f32 = 0.3;
const CLOUD_EDGE_SOFTNESS: f32 = 0.12;
// How much cloud stands inside a heap, over the whole of what it has rather than as a step.
// The rim above saturates a tenth of the way in and then says nothing more, so a heap's
// middle was as thick as its neighbour's and a deck's whole underside came out one flat
// grey with no picture in it (seen from below at a cover of 0.75 and up, where the gaps
// between heaps show the overcast table rather than blue and there is nothing else left to
// carry a shape). The heap's depth is now read over `CLOUD_BODY_RANGE` of the same margin,
// at `CLOUD_BODY_WEIGHT` of the result - a thin veil stays a veil, a deep heap is twice as
// thick as a shallow one standing beside it, and the rim itself is untouched.
const CLOUD_BODY_RANGE: f32 = 0.7;
const CLOUD_BODY_WEIGHT: f32 = 0.65;
// How much deeper than the mean heap this one stands. The extinction follows it: a deep heap
// is a longer column of droplets than the shallow one beside it, and it reads darker from
// below because less of the sky's light comes down through it - the deck's underside then
// carries the shape of its own heaps instead of the one flat grey it was (which is what the
// cover's top half looked like from underneath, whatever the weather said). A heap's height
// goes with `0.7 + b` in `cloud_base_shape`, and the map's median heap has `b = 0.30`
// (measured), so the typical cloud comes out at 1.0 and the spread - half as thick to nearly
// three times - is what is new.
fn cloud_depth(b: f32) -> f32 {
    let k = 0.7 + b;
    return k * k;
}
const CLOUD_BOTTOM_SOFTNESS: f32 = 0.2;
const CLOUD_MAX_DIST: f32 = 60000.0;
const CLOUD_MS_GAIN: f32 = 2.4;

@group(1) @binding(6) var t_cloud_shape: texture_2d<f32>;
@group(1) @binding(7) var t_cloud_detail: texture_3d<f32>;
@group(1) @binding(8) var s_cloud: sampler;

// Where in its first step this pixel's ray starts (0..1; 0.5 for the reflection probe).
var<private> cloud_jitter: f32 = 0.5;

fn linearstep(a: f32, b: f32, v: f32) -> f32 {
    return clamp((v - a) / (b - a), 0.0, 1.0);
}

// Height over the curved ground of the point t metres along d from the camera.
fn cloud_height(d: vec3<f32>, t: f32) -> f32 {
    return camera.cam_pos.z + eye_off.z + d.z * t + t * t * (1.0 - d.z * d.z) / (2.0 * EARTH_R);
}

// Distance along d at which the ray reaches height h over the curved ground (the far root
// when the camera is above h; -1 when it never does).
fn cloud_shell(d: vec3<f32>, h: f32) -> f32 {
    let a = (1.0 - d.z * d.z) / (2.0 * EARTH_R);
    let b = d.z;
    let c = camera.cam_pos.z + eye_off.z - h;
    if (a < 1e-12) {
        return select(-1.0, -c / b, abs(b) > 1e-6 && -c / b > 0.0);
    }
    let disc = b * b - 4.0 * a * c;
    if (disc < 0.0) {
        return -1.0;
    }
    let q = -0.5 * (b + select(-1.0, 1.0, b >= 0.0) * sqrt(disc));
    let r0 = q / a;
    let r1 = c / q;
    let lo = min(r0, r1);
    let hi = max(r0, r1);
    if (lo > 0.0) {
        return lo;
    }
    return select(-1.0, hi, hi > 0.0);
}

// The sun's irradiance at height z (as `cloud_height` gives it) in the cloud layer:
// between the base, middle and top of `enh.cloud_sun`.
fn cloud_sun_at(z: f32) -> vec3<f32> {
    let t = clamp((z - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM), 0.0, 1.0) * 2.0;
    return select(mix(enh.cloud_sun[1].rgb, enh.cloud_sun[2].rgb, t - 1.0), mix(enh.cloud_sun[0].rgb, enh.cloud_sun[1].rgb, t), t < 1.0);
}

// How much of the sky the weather covers here: its type (camera.clouds.x) and, over tens
// of kilometres, its own cloud picture.
fn cloud_coverage(p: vec2<f32>) -> f32 {
    let cover = camera.clouds.x;
    let uv = p / CLOUD_FIELD_TILE + camera.clouds.yz * (2500.0 / CLOUD_FIELD_TILE);
    let field = textureSampleLevel(t_clouds, s_repeat, uv * 0.35, 5.0).g;
    return clamp(0.3 + cover * 0.55 + (field - 0.5) * 0.25, 0.0, 1.0);
}

// The heap before its billows (h: 0 at the base, 1 at the top of the layer). The map's
// rounding channel comes back with it, for the extinction to follow the heap's own depth
// (`cloud_depth`).
fn cloud_base_shape(p: vec3<f32>, h: f32, lod: f32) -> vec2<f32> {
    let drift = camera.clouds.yz * 2500.0;
    let s = textureSampleLevel(t_cloud_shape, s_cloud, (p.xy + drift) / CLOUD_SHAPE_PERIOD, lod);
    let lo = s.g - 1.0;
    // the heap narrows upwards (every heap: with the map's rounding alone, a heap where it
    // was small rose as a column with a flat lid against the top of the layer)
    let n = h * h * (0.7 + s.b) + pow(1.0 - h, 16.0);
    let m = (s.r - n - lo) / (1.0 - lo);
    return vec2<f32>(m * (linearstep(0.0, 0.1, h) - linearstep(0.6, 1.0, h)), s.b);
}

// Extinction (1/m) at p; `detail` false for the light towards the sun far from the point.
fn cloud_sigma(p: vec3<f32>, h: f32, coverage: f32, lod: f32, detail: bool) -> f32 {
    if (h <= 0.0 || h >= 1.0) {
        return 0.0;
    }
    let shape = cloud_base_shape(p, h, lod);
    var m = shape.x;
    // (the billows only take away)
    if (m + coverage - 1.0 <= 0.0) {
        return 0.0;
    }
    if (detail) {
        let drift = camera.clouds.yz * 2500.0 * 1.3;
        let q = vec3<f32>(p.xy + drift, p.z) / CLOUD_DETAIL_PERIOD;
        let dl = textureSampleLevel(t_cloud_detail, s_cloud, q, max(lod - 2.0, 0.0)).r;
        m = m - dl * smoothstep(1.0, 0.5, m) * CLOUD_DETAIL_STRENGTH;
    }
    let margin = m + coverage - 1.0;
    let edge = smoothstep(0.0, CLOUD_EDGE_SOFTNESS, margin);
    let body = clamp(margin / CLOUD_BODY_RANGE, 0.0, 1.0);
    m = edge * mix(1.0, body, CLOUD_BODY_WEIGHT) * cloud_depth(shape.y);
    m = m * min(h / CLOUD_BOTTOM_SOFTNESS, 1.0);
    return m * CLOUD_SIGMA;
}

// The clouds towards d in front of `below` (the sky behind them): rgb the picture, a how
// much the clouds cover.
fn cloud_layer(d: vec3<f32>, below: vec3<f32>, pix: f32) -> vec4<f32> {
    if (camera.clouds.x <= 0.001 || d.z <= -0.01 || camera.cam_pos.z + eye_off.z > CLOUD_BOTTOM) {
        return vec4<f32>(below, 0.0);
    }
    let sd = normalize(camera.sun_dir.xyz);
    // a closed cover (or a sky that rain or snow falls from) is the grey deck of the table
    let closed = max(smoothstep(0.85, 1.0, camera.clouds.x), enh.weather.w);
    let t0 = cloud_shell(d, CLOUD_BOTTOM);
    if (t0 < 0.0 || t0 > CLOUD_MAX_DIST) {
        return vec4<f32>(below, 0.0);
    }
    let t1 = min(cloud_shell(d, CLOUD_TOP), min(t0 + 12000.0, CLOUD_MAX_DIST + 6000.0));
    // (the march's steps: `CLOUD_STEPS` in lib.rs, fewer with the low cloud quality)
    let steps = max(i32(enh.eye.w), 1);
    let ds = (t1 - t0) / f32(steps);
    // how many texels of the shape map a pixel spans where the ray meets the clouds
    let lod = log2(max(t0 * pix * f32(textureDimensions(t_cloud_shape).x) / CLOUD_SHAPE_PERIOD, 1.0));
    // (the sun before the clouds - at their own height, through the air from up there:
    // how much of it the cover lets through, lights.w, is what this march works out itself)
    let sky_top = sh_irradiance(vec3<f32>(0.0, 0.0, 1.0)) / PI;
    let ground = sh_irradiance(vec3<f32>(0.0, 0.0, -1.0)) / PI;
    let cos_sun = dot(d, sd);
    let coverage = cloud_coverage(cloud_ground(d, t0));
    var trans = 1.0;
    var acc = vec3<f32>(0.0);
    var hit = 0.0;
    var hit_w = 0.0;
    var t = t0 + ds * cloud_jitter;
    for (var i = 0; i < steps; i = i + 1) {
        let p = vec3<f32>(cloud_ground(d, t), cloud_height(d, t));
        let h = (p.z - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
        let sigma = cloud_sigma(p, h, coverage, lod, true);
        if (sigma > 1e-6) {
            // the sunlight reaching p through the cloud towards the sun
            var od = 0.0;
            var ls = 40.0;
            var lt = ls * 0.5;
            for (var k = 0; k < 6; k = k + 1) {
                let q = p + sd * lt;
                let hq = (q.z - CLOUD_BOTTOM) / (CLOUD_TOP - CLOUD_BOTTOM);
                if (hq >= 1.0) {
                    break;
                }
                od = od + cloud_sigma(q, hq, coverage, lod + 1.0, k < 2) * ls;
                ls = ls * 1.7;
                lt = lt + ls;
            }
            // multiple scattering (Hillaire 2016): octaves of weaker extinction, weaker
            // light and a flatter phase
            let sun = cloud_sun_at(p.z);
            var direct = vec3<f32>(0.0);
            var a = 1.0;
            var b = 1.0;
            var c = 1.0;
            for (var o = 0; o < 3; o = o + 1) {
                let phase = mix(hg_phase(cos_sun, 0.8 * c), hg_phase(cos_sun, -0.2 * c), 0.5);
                direct = direct + sun * a * phase * exp(-od * b);
                a = a * 0.5;
                b = b * 0.4;
                c = c * 0.5;
            }
            // (single scattering and three octaves hold only part of the light a cloud
            // scatters on inside it; a cumulus's sunlit side is about as bright as white
            // paper in the sun, E/π, which this factor brings it to)
            // The sky's own light is read through how much cloud stands over this point:
            // `od` is the depth towards the sun, the local measure of it, and its exponent
            // here is far gentler than the light's own. At the absolute depth of a real
            // layer the sun's term is long dead, and a deck whose underside was lit by
            // `amb` alone came out as one flat grey sheet with no heaps in it, at any cover
            // above a half (there the gaps between heaps show the overcast table rather
            // than blue, so nothing was left to give it a shape). Read softer, the same
            // depth leaves an overcast's underside a picture of itself - thicker where its
            // heaps stand, paler where the deck thins - and deepens a cumulus's shadowed
            // side without touching its sunlit top, where the depth towards the sun is
            // small.
            let amb = mix(ground * 0.45 + sky_top * 0.55, sky_top * 1.1, clamp(h * 1.4, 0.0, 1.0)) * (0.40 + 0.60 * exp(-od * 0.055));
            let light = direct * CLOUD_MS_GAIN + amb;
            // Frostbite: the light scattered over the step, dimmed by the cloud before it
            let dt = exp(-sigma * ds);
            acc = acc + trans * light * (1.0 - dt);
            hit = hit + t * trans * (1.0 - dt);
            hit_w = hit_w + trans * (1.0 - dt);
            trans = trans * dt;
            if (trans < 0.01) {
                break;
            }
        }
        t = t + ds;
    }
    // far clouds take on the colour of the air in front of them, and beyond 40 km fade out
    let dist = select(t0, hit / max(hit_w, 1e-4), hit_w > 1e-4);
    let aerial = 1.0 - exp(-dist / 22000.0);
    let fade = 1.0 - smoothstep(40000.0, CLOUD_MAX_DIST, dist);
    let horizon_fade = smoothstep(-0.01, 0.02, d.z);
    let a = (1.0 - trans) * fade * horizon_fade;
    acc = mix(acc, below * (1.0 - trans), aerial) * fade * horizon_fade;
    var col = acc + (1.0 - a) * below;
    let deck = below * (0.75 + 0.35 * cloud_cover_at(cloud_ground(d, max(t0, 1.0)), 4.0).x);
    col = mix(col, deck, closed);
    // the high, thin layer
    let drift = camera.clouds.yz * 2500.0;
    let t_hi = 7000.0 / max(d.z, 0.02);
    let p_hi = cloud_ground(d, t_hi);
    let hi = cloud_fbm((p_hi + drift * 1.7) / 2500.0);
    // (and the veil of the weather, as thick as its optical depth along the view makes it,
    // fibrous where the high layer's texture is)
    let veil = enh.cloud_sun[0].w;
    let veil_cover = (1.0 - exp(-veil / max(d.z, 0.08) * (0.55 + 0.45 * hi))) * 0.8;
    let hi_cover = max(clamp((hi - 0.6 + camera.clouds.x * 0.2) * 2.0, 0.0, 1.0) * 0.35, veil_cover) * clamp(d.z * 8.0, 0.0, 1.0) * (1.0 - a) * (1.0 - closed);
    // (lit by the sun at its own height, through thin ice: forward scattering towards the
    // sun, a dull grey away from it)
    let hi_sun = enh.cloud_sun[3].rgb * (0.12 + 0.5 * hg_phase(cos_sun, 0.6) * 4.0 * PI * 0.25);
    col = mix(col, (hi_sun * (1.0 - closed) / PI + sky_top) * 0.9, hi_cover);
    return vec4<f32>(col, max(max(a, hi_cover), closed));
}

// Sky radiance towards d without the sun's disc: the table, the clouds, the air.
fn sky_radiance(d: vec3<f32>, pix: f32) -> vec3<f32> {
    var col = sky_table(d);
    let c = cloud_layer(d, col, pix);
    col = c.rgb;
    // the weather's fog along the whole way to the horizon (the clear air is in the table
    // already: taken again, it whitened the whole sky)
    let h0 = camera.cam_pos.z - enh.fog.z;
    let dist = 30000.0;
    let a = air_of(d, dist, h0, h0 + dist * max(d.z, 0.0), 0.0);
    return col * a.a + a.rgb;
}

// The sky as the sky cube holds it (drawn a face a frame by `fs_sky_cube`, divided by the
// table scale): rgb, and how much cloud covers the direction.
@group(0) @binding(17) var t_sky_cube: texture_cube<f32>;

@fragment
fn fs_enhanced(in: VsOut) -> @location(0) vec4<f32> {
    return enhanced_sky(in);
}

fn enhanced_sky(in: VsOut) -> vec4<f32> {
    let d = normalize(in.dir);
    let pre = enh.exposure.x;
    // the cube is drawn from its own eye (lib.rs Probe::cube_eye): look the clouds' base up
    // from there, so the sky does not slide with a camera that moved since
    var ld = d;
    let tb = cloud_shell(d, CLOUD_BOTTOM);
    if (tb > 0.0 && camera.cam_pos.z < CLOUD_BOTTOM) {
        ld = normalize(d * min(tb, CLOUD_MAX_DIST) - enh.eye.xyz);
    }
    let cube = textureSampleLevel(t_sky_cube, s_lin, vec3<f32>(ld.x, ld.z, ld.y), 0.0);
    var col = cube.rgb * enh.ground.w;
    var sun_px = vec3<f32>(0.0);
    // the sun: a limb-darkened disc as bright as its irradiance spread over its size,
    // behind whatever cloud there is
    let sd = normalize(camera.sun_dir.xyz);
    let r = enh.sun.w;
    let cosang = dot(d, sd);
    if (cosang > cos(r * 1.5) && d.z > -0.02) {
        let x = clamp(acos(clamp(cosang, -1.0, 1.0)) / r, 0.0, 1.5);
        let disc = 1.0 - smoothstep(0.85, 1.1, x);
        let limb = 1.0 - 0.6 * (1.0 - sqrt(max(1.0 - min(x, 1.0) * min(x, 1.0), 0.0)));
        let cover = cube.a;
        let h0 = camera.cam_pos.z - enh.fog.z;
        let t = air_of(d, 30000.0, h0, h0 + 30000.0 * max(d.z, 0.0), 0.0).a;
        let l = enh.sun_disc.rgb / (PI * r * r) * disc * limb * (1.0 - cover) * enh.lights.w * t;
        sun_px = l;
    }
    col = col + night_sky(d, cube.a, fwidth(d));
    // the dome is drawn pre-exposed; the disc only a little over white: the eye's glare round
    // it is drawn from its light (fog_lamps.wgsl `sun_glare`), and from the disc's pixels
    // as well the glow would have counted it twice
    return vec4<f32>(min(col * pre, vec3<f32>(4000.0)) + min(sun_px * pre, vec3<f32>(24.0)), 1.0);
}

// The mirrors of an Enhanced picture are drawn with the plain shading (lib.rs, the cost of
// the enhanced shader in a small picture), but their sky is this one, tone-mapped here
// with the window's curve (post.wgsl `natural_tone`): the plain envir.cfg sky showed a blue
// dusk in the mirrors over the window's grey one, or the other way round (#1754, #1449).
// The meter's correction is left out (by day a few tenths of a stop at most).
fn mirror_tone(color: vec3<f32>, contrast: f32) -> vec3<f32> {
    let knee = 0.66;
    let x = max(color, vec3<f32>(0.0));
    let y = 0.18 * pow(x / 0.18 + vec3<f32>(1e-7), vec3<f32>(contrast));
    let peak = max(y.r, max(y.g, y.b));
    if (peak <= knee) {
        return y;
    }
    let d = 1.0 - knee;
    let shoulder = vec3<f32>(knee) + d * (vec3<f32>(1.0) - exp(-(y - vec3<f32>(knee)) / d));
    let soft = select(y, shoulder, y > vec3<f32>(knee));
    let np = knee + d * (1.0 - exp(-(peak - knee) / d));
    let o = mix(y * (np / peak), soft, 0.4);
    let g = 1.0 - 1.0 / (0.3 * (peak - np) + 1.0);
    return mix(o, vec3<f32>(np), g);
}

@fragment
fn fs_enhanced_mirror(in: VsOut) -> @location(0) vec4<f32> {
    let hdr = enhanced_sky(in).rgb;
    return vec4<f32>(clamp(mirror_tone(hdr, max(enh.debug.w, 1.0)), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

fn star_hash(c: vec3<i32>) -> vec4<f32> {
    var x = vec4<u32>(bitcast<vec3<u32>>(c), 0x9e3779b9u).xyzw;
    x = x * 1664525u + 1013904223u;
    x.x = x.x + x.y * x.w;
    x.y = x.y + x.z * x.x;
    x.z = x.z + x.x * x.y;
    x.w = x.w + x.y * x.z;
    x = x ^ (x >> vec4<u32>(16u));
    x.x = x.x + x.y * x.w;
    x.y = x.y + x.z * x.x;
    x.z = x.z + x.x * x.y;
    x.w = x.w + x.y * x.z;
    return vec4<f32>(x & vec4<u32>(0xffffffu)) / 16777216.0;
}

// The moon and the stars towards d (radiance), behind what cloud covers it. The moon is a
// lit sphere: its lit side faces the sun, its phase is what the sun's direction makes of
// it, and its grey seas darken parts of the disc. The stars are a few thousand points
// fixed in the sky, of a few magnitudes, slightly coloured; they show only where the sky
// behind them is dark - a lit city's glow and the haze leave the brightest few.
fn night_sky(d: vec3<f32>, cover: f32, pix: vec3<f32>) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    if (d.z < -0.01) {
        return col;
    }
    let h0 = camera.cam_pos.z - enh.fog.z;
    let through = air_of(d, 30000.0, h0, h0 + 30000.0 * max(d.z, 0.0), 0.0).a * (1.0 - cover);
    if (through < 0.01) {
        return col;
    }
    let m = enh.moon.xyz;
    let r = enh.moon.w;
    let cm = dot(d, m);
    if (cm > cos(r * 1.3) && max(enh.moon_disc.r, enh.moon_disc.g) > 0.0) {
        // the point on the moon's sphere seen in direction d
        let right = normalize(cross(m, vec3<f32>(0.0, 0.0, 1.0)) + vec3<f32>(1e-5, 0.0, 0.0));
        let up = cross(right, m);
        let q = vec2<f32>(dot(d, right), dot(d, up)) / r;
        let q2 = dot(q, q);
        let rim = 1.0 - smoothstep(0.92, 1.05, sqrt(q2));
        let nz = sqrt(max(1.0 - q2, 0.0));
        let n = normalize(right * q.x + up * q.y - m * nz);
        let sun_side = smoothstep(-0.04, 0.08, dot(n, normalize(camera.sun_dir.xyz)));
        // the seas: a few broad dark patches fixed on the face
        let sea = smoothstep(0.45, 0.7, cloud_fbm(q * 1.7 + vec2<f32>(3.1, 7.7)));
        let albedo = 1.0 - 0.38 * sea;
        // (as bright as the lit part's share of the irradiance spread over the disc; the
        // dark part keeps a trace of earthshine)
        let lit_share = max(0.5 + 0.5 * dot(-m, normalize(camera.sun_dir.xyz)), 0.03);
        let l = enh.moon_disc.rgb / (PI * r * r * lit_share) * (sun_side + 0.004) * albedo * rim;
        col = col + l * through;
    }
    // the stars: one candidate in some of the cells of a grid on the sphere
    let vis = enh.moon_disc.w;
    if (vis > 0.0 && d.z > 0.0) {
        let cells = 140.0;
        let g = d * cells;
        let c = vec3<i32>(floor(g));
        let h = star_hash(c);
        let density = 0.014;
        if (h.x < density) {
            let p = (vec3<f32>(c) + vec3<f32>(0.25) + h.yzw * 0.5) / cells;
            let sd = normalize(p);
            let ang = acos(clamp(dot(d, sd), -1.0, 1.0));
            // a point of light about a pixel across
            let size = clamp(length(pix) * 0.6, 1e-5, 0.004);
            let spot = exp(-ang * ang / (size * size));
            // brightness: many faint ones, a few bright (a power law of the magnitudes)
            let b = pow(1.0 - h.x / density, 4.0);
            let tint = mix(vec3<f32>(1.0, 0.86, 0.72), vec3<f32>(0.8, 0.88, 1.0), h.w);
            // the faintest that shows against the sky behind it (in the picture's own
            // units): a dark country sky shows thousands, a city's glow the few dozen
            // brightest, a twilight none
            let sky_l = dot(sky_table(d), vec3<f32>(0.2126, 0.7152, 0.0722)) * enh.exposure.x;
            let limit = clamp(sky_l * 90.0 + (1.0 - vis), 0.0, 1.2);
            let shows = smoothstep(limit, limit + 0.15, b) * (1.0 - smoothstep(0.03, 0.1, sky_l));
            col = col + tint * (0.04 + 1.4 * b) * spot * shows * through / max(enh.exposure.x, 1e-6) * smoothstep(0.0, 0.15, d.z);
        }
    }
    return col;
}

// --- the reflection probe: a cube map of the sky seen from the camera, drawn now and then
// and blurred into its mip levels for rough surfaces (divided by the table scale, which
// the main pass multiplies back in).
struct ProbeParams {
    // x first face of this pass (0 or 3), y roughness, z face size of level 0, w level
    p: vec4<f32>,
};
@group(0) @binding(15) var<uniform> probe: ProbeParams;
@group(0) @binding(16) var t_probe_src: texture_cube<f32>;

// Direction of a texel of cube face f (u, v in -1..1, v down), in cube coordinates.
fn face_dir(f: i32, u: f32, v: f32) -> vec3<f32> {
    var c = vec3<f32>(-u, -v, -1.0);
    switch f {
        case 0: { c = vec3<f32>(1.0, -v, -u); }
        case 1: { c = vec3<f32>(-1.0, -v, u); }
        case 2: { c = vec3<f32>(u, 1.0, v); }
        case 3: { c = vec3<f32>(u, -1.0, -v); }
        case 4: { c = vec3<f32>(u, -v, 1.0); }
        default: {}
    }
    return normalize(c);
}

fn cube_to_world(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x, c.z, c.y);
}

struct ProbeOut {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
};

struct FsIn {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_probe(@builtin(vertex_index) i: u32) -> FsIn {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return FsIn(vec4<f32>(x, y, 0.0, 1.0));
}

fn probe_sky(f: i32, uv: vec2<f32>) -> vec4<f32> {
    let c = face_dir(f, uv.x, uv.y);
    let d = cube_to_world(c);
    var col: vec3<f32>;
    if (d.z >= 0.0) {
        // the sky cube (lib.rs binds it as level 0's source), already divided by the table
        // scale: marching the clouds again for the probe took 3.4 ms every time it was drawn
        return vec4<f32>(textureSampleLevel(t_probe_src, s_lin, c, 0.0).rgb, 1.0);
    } else {
        // below the horizon: the table's distant ground, with the houses and trees of a
        // town darkening the band just under the horizon
        col = sky_table(d);
        let band = smoothstep(0.02, 0.25, -d.z);
        col = mix(col, enh.ground.rgb * 0.8, 1.0 - band * 0.6);
    }
    return vec4<f32>(col / max(enh.ground.w, 1e-8), 1.0);
}

@fragment
fn fs_probe_sky(in: FsIn) -> ProbeOut {
    let size = probe.p.z;
    let uv = in.clip.xy / size * 2.0 - vec2<f32>(1.0);
    let f0 = i32(probe.p.x);
    return ProbeOut(probe_sky(f0, uv), probe_sky(f0 + 1, uv), probe_sky(f0 + 2, uv));
}

fn radical_inverse(i: u32) -> f32 {
    var b = i;
    b = (b << 16u) | (b >> 16u);
    b = ((b & 0x55555555u) << 1u) | ((b & 0xAAAAAAAAu) >> 1u);
    b = ((b & 0x33333333u) << 2u) | ((b & 0xCCCCCCCCu) >> 2u);
    b = ((b & 0x0F0F0F0Fu) << 4u) | ((b & 0xF0F0F0F0u) >> 4u);
    b = ((b & 0x00FF00FFu) << 8u) | ((b & 0xFF00FF00u) >> 8u);
    return f32(b) * 2.3283064365386963e-10;
}

// GGX-filtered radiance around n (the split-sum assumption: view = normal), sampling the
// sharper levels at a lod that matches each sample's footprint.
fn prefilter(n: vec3<f32>) -> vec3<f32> {
    let rough = probe.p.y;
    let a = rough * rough;
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
    let tx = normalize(cross(up, n));
    let ty = cross(n, tx);
    let count = 32u;
    let texel_sa = 4.0 * PI / (6.0 * probe.p.z * probe.p.z);
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let xi = vec2<f32>(f32(i) / f32(count), radical_inverse(i));
        let phi = 2.0 * PI * xi.x;
        let cos_t = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let h = tx * (cos(phi) * sin_t) + ty * (sin(phi) * sin_t) + n * cos_t;
        let l = 2.0 * dot(n, h) * h - n;
        let nl = dot(n, l);
        if (nl <= 0.0) {
            continue;
        }
        let nh = max(dot(n, h), 0.0);
        let d2 = nh * nh * (a * a - 1.0) + 1.0;
        let dd = a * a / (PI * d2 * d2);
        let pdf = dd * 0.25;
        let sample_sa = 1.0 / (f32(count) * pdf + 1e-4);
        let lod = clamp(0.5 * log2(sample_sa / texel_sa) + 1.0, 0.0, probe.p.w - 1.0);
        sum = sum + textureSampleLevel(t_probe_src, s_lin, l, lod).rgb * nl;
        wsum = wsum + nl;
    }
    return sum / max(wsum, 1e-4);
}

@fragment
fn fs_probe_filter(in: FsIn) -> ProbeOut {
    let size = probe.p.z / pow(2.0, probe.p.w);
    let uv = in.clip.xy / size * 2.0 - vec2<f32>(1.0);
    let f0 = i32(probe.p.x);
    return ProbeOut(
        vec4<f32>(prefilter(face_dir(f0, uv.x, uv.y)), 1.0),
        vec4<f32>(prefilter(face_dir(f0 + 1, uv.x, uv.y)), 1.0),
        vec4<f32>(prefilter(face_dir(f0 + 2, uv.x, uv.y)), 1.0),
    );
}

// One face of the sky cube: the sky radiance (the table, the clouds, the air) divided by the
// table scale, and the cloud cover in alpha (the sun's disc is hidden by it).
@fragment
fn fs_sky_cube(in: FsIn) -> @location(0) vec4<f32> {
    let size = probe.p.z;
    let uv = in.clip.xy / size * 2.0 - vec2<f32>(1.0);
    let d = cube_to_world(face_dir(i32(probe.p.x), uv.x, uv.y));
    let pix = 1.4 / size;
    // a new start point for the steps each redraw (p.y counts the redraws): interleaved
    // gradient noise over the face, shifted by the golden ratio
    let ign = fract(52.9829189 * fract(dot(in.clip.xy, vec2<f32>(0.06711056, 0.00583715))));
    cloud_jitter = fract(ign + probe.p.y * 0.61803399);
    eye_off = enh.eye.xyz;
    var col = sky_table(d);
    let c = cloud_layer(d, col, pix);
    col = c.rgb;
    let h0 = camera.cam_pos.z + eye_off.z - enh.fog.z;
    let dist = 30000.0;
    let a = air_of(d, dist, h0, h0 + dist * max(d.z, 0.0), 0.0);
    col = col * a.a + a.rgb;
    return vec4<f32>(col / max(enh.ground.w, 1e-8), c.a);
}
