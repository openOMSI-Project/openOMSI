// Enhanced graphics: what happens to the pre-exposed high-range picture before it reaches
// the screen - nothing that paints over it. A glow that only real highlights produce
// (a wide blur of the picture mixed in at a few per cent: a lamp a hundred times brighter
// than white spreads, a white wall does not), automatic exposure that meters the picture
// and follows it slowly within a narrow range, a photographic tone curve (a camera's
// contrast in the middle tones, a soft shoulder), dithering against banding, FXAA.
// The vanilla path never runs this.
struct PostParams {
    // x glow strength, y how far the metering may darken (EV), z brighten (EV),
    // w adaptation this frame (0..1; 1 = at once)
    a: vec4<f32>,
    // x how fast the exposure follows a darker picture, y a brighter one (fractions of the
    // way per frame), z the metering target (log2 of the mean luminance), w exposure bias (EV)
    b: vec4<f32>,
    // x how much of the metered difference is corrected (the light model's own exposure is
    // an incident meter: snow stays white and a night dark; the metering only helps where
    // it cannot know, a dark cab or an underpass), y night vision strength, z the scene's
    // pre-exposure (for absolute luminance), w how much an LED panel's dots count for in
    // the glow's source (0 = not at all, `Led glow`)
    c: vec4<f32>,
    // x Enhanced+'s grade (0/1), y the vignette's strength, z sharpening, w the tone
    // curve's contrast about mid grey (1 = none; a camera's by day, less at night)
    d: vec4<f32>,
};
@group(0) @binding(0) var<uniform> p: PostParams;
@group(0) @binding(1) var t_src: texture_2d<f32>;
@group(0) @binding(2) var s_lin: sampler;
@group(0) @binding(3) var t_base: texture_2d<f32>;
@group(0) @binding(4) var t_adapt: texture_2d<f32>;
// (the tone mapping's: the screen mask, whose r under 0.5 is MIST_MASK x the share of the
// light the player's bus's misted glass scatters there - enhanced.wgsl `pane_condensation`)
@group(0) @binding(5) var t_mask: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var out: VsOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// A pixel that is not a finite number (a NaN or an infinity from the scene) as black -
// tested on the bits, which fast maths cannot drop. Unguarded, the glow's down- and
// up-sampling spread one such pixel over the picture as half and fully black squares
// (Vulkan and Direct3D keep NaNs, Metal's fast maths happened to hide them).
fn clean(c: vec3<f32>) -> vec3<f32> {
    let e = bitcast<vec3<u32>>(c) & vec3<u32>(0x7f800000u);
    let ok = all(e != vec3<u32>(0x7f800000u));
    return select(vec3<f32>(0.0), clamp(c, vec3<f32>(0.0), vec3<f32>(65000.0)), ok);
}

fn src(uv: vec2<f32>, texel: vec2<f32>, x: f32, y: f32) -> vec3<f32> {
    return clean(textureSampleLevel(t_src, s_lin, uv + vec2<f32>(x, y) * texel, 0.0).rgb);
}

// The picture without the bus's own screens (the screen mask is the first glow level's
// t_base): a lit display glows no halo over its own letters. An LED panel's dots are the
// panel's own light, though - they stay in the source (the mask's g) and count for several
// times their colour there (`p.c.w`), so that the faint mix the glow is blooms a halo
// around the panel without the dots themselves having to burn.
//
// What glows is only the light above the display's white (`GLARE_WHITE`, in the picture's
// pre-exposed terms): the viewer's own eye scatters the light of everything the screen can
// show, as it does in a street - only what lies beyond the screen's range has to have its
// scattered light drawn (Spencer et al., "Physically-based glare effects for digital
// images", 1995). The alpha carries the picture's whole luminance down the chain for the
// metering.
const GLARE_WHITE: f32 = 1.0;
fn is_screen_mask(m: vec4<f32>) -> bool {
    // Clear panes attenuate both screen channels together. Condensation adds only to
    // r; water also has b. Recover the screen code before interpreting a low r as mist.
    let code = m.g / max(m.r, 1e-3);
    return m.r >= 0.5 || (m.r > 0.004 && m.b < 0.004 &&
        ((code > 0.2 && code < 0.3) || (code > 0.9 && code < 1.1)));
}

fn src_unmasked(uv: vec2<f32>, texel: vec2<f32>, x: f32, y: f32) -> vec4<f32> {
    let at = uv + vec2<f32>(x, y) * texel;
    let m = textureSampleLevel(t_base, s_lin, at, 0.0);
    let c = clean(textureSampleLevel(t_src, s_lin, at, 0.0).rgb);
    let screen = select(0.0, 1.0, is_screen_mask(m));
    let led = step(0.5, m.g);
    let picture = c * (1.0 - screen);
    let over = min(max(picture - vec3<f32>(GLARE_WHITE), vec3<f32>(0.0)), vec3<f32>(64.0)) + c * (led * p.c.w) * screen;
    return vec4<f32>(over, luma(picture + c * (led * p.c.w) * screen));
}

fn src4(uv: vec2<f32>, texel: vec2<f32>, x: f32, y: f32) -> vec4<f32> {
    let t = textureSampleLevel(t_src, s_lin, uv + vec2<f32>(x, y) * texel, 0.0);
    return vec4<f32>(clean(t.rgb), clamp(select(0.0, t.a, t.a == t.a), 0.0, 65000.0));
}

// --- the glow: 13-tap downsampling (the first level with Karis' average, so that a single
// bright pixel does not flicker), tent upsampling

@fragment
fn fs_down_first(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_src));
    let uv = in.uv;
    let a = src_unmasked(uv, texel, -2.0, -2.0);
    let b = src_unmasked(uv, texel, 0.0, -2.0);
    let c = src_unmasked(uv, texel, 2.0, -2.0);
    let d = src_unmasked(uv, texel, -2.0, 0.0);
    let e = src_unmasked(uv, texel, 0.0, 0.0);
    let f = src_unmasked(uv, texel, 2.0, 0.0);
    let g = src_unmasked(uv, texel, -2.0, 2.0);
    let h = src_unmasked(uv, texel, 0.0, 2.0);
    let i = src_unmasked(uv, texel, 2.0, 2.0);
    let j = src_unmasked(uv, texel, -1.0, -1.0);
    let k = src_unmasked(uv, texel, 1.0, -1.0);
    let l = src_unmasked(uv, texel, -1.0, 1.0);
    let m = src_unmasked(uv, texel, 1.0, 1.0);
    let g0 = (j + k + l + m) * 0.25;
    let g1 = (a + b + d + e) * 0.25;
    let g2 = (b + c + e + f) * 0.25;
    let g3 = (d + e + g + h) * 0.25;
    let g4 = (e + f + h + i) * 0.25;
    // (Karis' average: no single bright pixel flickers through. A small, very bright source -
    // a lamp, a headlight - gets its halo from the eye's glare drawn round it instead
    // (fog_lamps.wgsl `point_glare`); the glow is the halo of what is wide and bright)
    let w0 = 0.5 / (1.0 + luma(g0.rgb));
    let w1 = 0.125 / (1.0 + luma(g1.rgb));
    let w2 = 0.125 / (1.0 + luma(g2.rgb));
    let w3 = 0.125 / (1.0 + luma(g3.rgb));
    let w4 = 0.125 / (1.0 + luma(g4.rgb));
    let sum = g0.rgb * w0 + g1.rgb * w1 + g2.rgb * w2 + g3.rgb * w3 + g4.rgb * w4;
    // (the luminance for the metering: the plain average)
    let lum = (g0.a * 0.5 + (g1.a + g2.a + g3.a + g4.a) * 0.125);
    return vec4<f32>(sum / (w0 + w1 + w2 + w3 + w4), lum);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_src));
    let uv = in.uv;
    let outer = src4(uv, texel, -2.0, -2.0) + src4(uv, texel, 2.0, -2.0) + src4(uv, texel, -2.0, 2.0) + src4(uv, texel, 2.0, 2.0);
    let arms = src4(uv, texel, 0.0, -2.0) + src4(uv, texel, -2.0, 0.0) + src4(uv, texel, 2.0, 0.0) + src4(uv, texel, 0.0, 2.0);
    let inner = src4(uv, texel, -1.0, -1.0) + src4(uv, texel, 1.0, -1.0) + src4(uv, texel, -1.0, 1.0) + src4(uv, texel, 1.0, 1.0);
    return src4(uv, texel, 0.0, 0.0) * 0.125 + outer * 0.03125 + arms * 0.0625 + inner * 0.125;
}

@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_src));
    let uv = in.uv;
    // (a tent a texel and a half wide: one a texel wide left the coarse levels' texels as
    // steps round a bright source - a headlight's halo came out square)
    let tent = (src(uv, texel, -1.5, -1.5) + src(uv, texel, 1.5, -1.5) + src(uv, texel, -1.5, 1.5) + src(uv, texel, 1.5, 1.5)
        + (src(uv, texel, 0.0, -1.5) + src(uv, texel, -1.5, 0.0) + src(uv, texel, 1.5, 0.0) + src(uv, texel, 0.0, 1.5)) * 2.0
        + src(uv, texel, 0.0, 0.0) * 4.0) / 16.0;
    let base = clean(textureSampleLevel(t_base, s_lin, uv, 0.0).rgb);
    // (each level keeps a quarter of its own and passes three quarters of the wider ones
    // on: the levels' shares follow the eye's scattering - about 5.7 % of the light lands
    // 0.3-1 degree from where it should, 2.4 % at 1-3 degrees, 1.7 % at 3-10 and some 3.6 %
    // further out, CIE 146:2002's glare function integrated over those rings)
    return vec4<f32>(mix(base, tent, 0.75), 1.0);
}

// --- automatic exposure: the mean log luminance of the smallest glow level, weighted to
// the middle of the picture, and its slow follower

@fragment
fn fs_meter(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t_src));
    var sum = 0.0;
    var wsum = 0.0;
    for (var y = 0; y < dims.y; y = y + 1) {
        for (var x = 0; x < dims.x; x = x + 1) {
            let c = textureLoad(t_src, vec2<i32>(x, y), 0).a;
            let q = (vec2<f32>(f32(x), f32(y)) + 0.5) / vec2<f32>(dims) * 2.0 - 1.0;
            // the middle and the lower half count most: the sky is not what one looks at
            let w = exp(-dot(q, q) * 1.5) * (0.6 + 0.4 * clamp(q.y + 0.5, 0.0, 1.0));
            sum = sum + log2(clamp(select(1e-4, c, c == c), 1e-4, 64.0)) * w;
            wsum = wsum + w;
        }
    }
    return vec4<f32>(sum / max(wsum, 1e-6), 0.0, 0.0, 1.0);
}

@fragment
fn fs_adapt(in: VsOut) -> @location(0) vec4<f32> {
    let now = textureLoad(t_src, vec2<i32>(0, 0), 0).r;
    let before_raw = textureLoad(t_base, vec2<i32>(0, 0), 0).r;
    // (a NaN once in the adaptation would stay there and turn the whole picture black)
    let before = select(now, before_raw, (bitcast<u32>(before_raw) & 0x7f800000u) != 0x7f800000u);
    // the eye takes to the light quickly and to the dark slowly
    var k = select(p.b.y, p.b.x, now < before);
    k = max(k, p.a.w);
    return vec4<f32>(mix(before, now, k), 0.0, 0.0, 1.0);
}

// --- the picture

// Night vision: where the scene is darker than a lit street (about a candela per square
// metre) the eye's rods take over from its cones: colours fade and what is left of them
// shifts towards blue-green (the Purkinje shift). A lamp's pool keeps its colour, the
// street beyond it turns to a bluish grey. `pre` is the pre-exposure, which gives the
// absolute luminance (1 = 10 000 cd/m²).
fn night_vision(c: vec3<f32>, strength: f32, pre: f32) -> vec3<f32> {
    let lum = luma(c);
    let cd = lum / max(pre, 1e-6) * 10000.0;
    // fully scotopic below 0.01 cd/m², photopic above 3
    let rods = 1.0 - smoothstep(-2.0, 0.5, log(max(cd, 1e-6)) / log(10.0));
    // the rods' sensitivity peaks at 507 nm: blue and green count, red hardly
    let scot = dot(c, vec3<f32>(0.05, 0.62, 0.45));
    let tint = vec3<f32>(0.86, 0.95, 1.12);
    let night = tint * scot * (lum / max(dot(tint * scot, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-8));
    return mix(c, night, rods * strength);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn from_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow(max((c + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// The shoulder of `natural_tone` for one value: identity up to the knee, then an
// exponential approach to white with the slope kept at the knee.
const KNEE: f32 = 0.66;
fn soft_shoulder(x: vec3<f32>) -> vec3<f32> {
    let d = 1.0 - KNEE;
    let over = vec3<f32>(KNEE) + d * (vec3<f32>(1.0) - exp(-(x - vec3<f32>(KNEE)) / d));
    return select(x, over, x > vec3<f32>(KNEE));
}

// The tone curve of a photograph rather than of a renderer: a camera's curve gives the
// middle tones some contrast (about mid grey, per channel in log space - colours gain a
// little with it, as they do on film and on a phone's picture) where a plain linear
// mapping leaves a sunny street grey and flat, with lifted shadows and no white in it;
// above the knee the highlights roll off softly, partly per channel (a low sun's sky
// near the disc runs to a warm white, as on film) and partly by the brightest channel
// (a lit yellow bus stays yellow), and what lies far above white bleaches out.
fn natural_tone(color: vec3<f32>, contrast: f32) -> vec3<f32> {
    let x = max(color, vec3<f32>(0.0));
    let y = 0.18 * pow(x / 0.18 + vec3<f32>(1e-7), vec3<f32>(contrast));
    let peak = max(y.r, max(y.g, y.b));
    if (peak <= KNEE) {
        return y;
    }
    let np = soft_shoulder(vec3<f32>(peak)).x;
    let by_peak = y * (np / peak);
    let o = mix(by_peak, soft_shoulder(y), 0.4);
    let g = 1.0 - 1.0 / (0.3 * (peak - np) + 1.0);
    return mix(o, vec3<f32>(np), g);
}

// Enhanced+'s grade: the natural curve, the light a shade warmer.
fn filmic_grade(c: vec3<f32>) -> vec3<f32> {
    return natural_tone(c * vec3<f32>(1.015, 1.0, 0.985), max(p.d.w, 1.0));
}

// What a misted pane shows of what lies behind it: half the light its film scatters is
// diffracted forward by the droplets, within some 2 degrees for droplets of 10 um (0.61
// lambda / radius) - about a fortieth of the picture's height at the usual field of view -
// so the street goes soft and every light behind the glass spreads into a glowing patch:
// that share of the picture averaged over the lobe, from the same misted glass only (the
// frame and the dashboard beside it keep their edges).
const MIST_MASK: f32 = 0.24;
fn misted(uv: vec2<f32>, c: vec3<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(t_src));
    let px = vec2<i32>(uv * dims);
    let mask = textureLoad(t_mask, clamp(px, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1)), 0);
    let m = mask.r;
    if (m < 0.004 || is_screen_mask(mask)) {
        return c;
    }
    let fog = clamp(m / MIST_MASK, 0.0, 1.0);
    let radius = dims.y * 0.025;
    var sum = c;
    var w = 1.0;
    for (var k = 1; k < 24; k = k + 1) {
        let a = f32(k) * 2.3999632;
        let r = sqrt(f32(k) / 23.0) * radius;
        let q = uv + vec2<f32>(cos(a), sin(a)) * r / dims;
        let qm = textureLoad(t_mask, clamp(vec2<i32>(q * dims), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1)), 0);
        if (qm.r > 0.004 && !is_screen_mask(qm)) {
            sum = sum + clean(textureSampleLevel(t_src, s_lin, q, 0.0).rgb);
            w = w + 1.0;
        }
    }
    return mix(c, sum / w, 0.5 * fog);
}

// The tone-mapped picture, encoded for the display (gamma), dithered.
fn graded(in: VsOut) -> vec3<f32> {
    let hdr = misted(in.uv, clean(textureSampleLevel(t_src, s_lin, in.uv, 0.0).rgb));
    let glow = clean(textureSampleLevel(t_base, s_lin, in.uv, 0.0).rgb);
    // the eye's scattered light of what is brighter than the screen, added
    var c = hdr + glow * p.a.x;
    let metered = textureLoad(t_adapt, vec2<i32>(0, 0), 0).r;
    let ev = clamp((p.b.z - metered) * p.c.x, -p.a.y, p.a.z) + p.b.w;
    c = max(c, vec3<f32>(0.0));
    if (p.c.y > 0.0) {
        c = night_vision(c, p.c.y, p.c.z);
    }
    if (p.d.x > 0.5) {
        c = filmic_grade(c * pow(2.0, ev));
        // the lens: the corners a little darker
        let q = in.uv * 2.0 - vec2<f32>(1.0);
        c = c * (1.0 - p.d.y * pow(clamp(dot(q, q) * 0.5, 0.0, 1.0), 1.6));
    } else {
        c = natural_tone(c * pow(2.0, ev), max(p.d.w, 1.0));
    }
    var e = to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    // triangular dither of one code value: no bands in the sky's gradient
    let px = in.clip.xy;
    let n1 = fract(52.9829189 * fract(dot(px, vec2<f32>(0.06711056, 0.00583715))));
    let n2 = fract(52.9829189 * fract(dot(px + vec2<f32>(17.0, 31.0), vec2<f32>(0.06711056, 0.00583715))));
    e = e + vec3<f32>((n1 + n2 - 1.0) / 255.0);
    return e;
}

// straight to the (sRGB) target
@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(from_srgb(clamp(graded(in), vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

// into the gamma-encoded target FXAA reads, with its luma in alpha
@fragment
fn fs_tonemap_encoded(in: VsOut) -> @location(0) vec4<f32> {
    let e = clamp(graded(in), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(e, dot(e, vec3<f32>(0.299, 0.587, 0.114)));
}

// Enhanced+: contrast-adaptive sharpening of the anti-aliased picture (in its gamma
// encoding): a pixel is pushed away from its four neighbours' mean by as much as their
// spread leaves room for, so that flat areas stay calm and fine detail comes out.
fn sharpened(uv: vec2<f32>, c: vec3<f32>) -> vec3<f32> {
    if (p.d.z <= 0.0) {
        return c;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(t_src));
    let n = textureSampleLevel(t_src, s_lin, uv + vec2<f32>(0.0, -texel.y), 0.0).rgb;
    let s = textureSampleLevel(t_src, s_lin, uv + vec2<f32>(0.0, texel.y), 0.0).rgb;
    let e = textureSampleLevel(t_src, s_lin, uv + vec2<f32>(texel.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(t_src, s_lin, uv + vec2<f32>(-texel.x, 0.0), 0.0).rgb;
    let lo = min(c, min(min(n, s), min(e, w)));
    let hi = max(c, max(max(n, s), max(e, w)));
    let room = clamp(min(lo, vec3<f32>(1.0) - hi) / max(hi, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0));
    let k = sqrt(room) * p.d.z * 0.25;
    return clamp(c + (c * 4.0 - (n + s + e + w)) * k, vec3<f32>(0.0), vec3<f32>(1.0));
}

// --- FXAA 3.11 (quality, 12 search steps) over the tone-mapped picture

fn lum_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(t_src, s_lin, uv, 0.0).a;
}

@fragment
fn fs_fxaa(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_src));
    let uv = in.uv;
    let rgbm = textureSampleLevel(t_src, s_lin, uv, 0.0);
    // the bus's own screens as they are: FXAA took half the contrast out of their
    // letters (the screen mask is this pass's t_base)
    if (is_screen_mask(textureSampleLevel(t_base, s_lin, uv, 0.0))) {
        return vec4<f32>(from_srgb(rgbm.rgb), 1.0);
    }
    let m = rgbm.a;
    let n = lum_at(uv + vec2<f32>(0.0, -1.0) * texel);
    let s = lum_at(uv + vec2<f32>(0.0, 1.0) * texel);
    let e = lum_at(uv + vec2<f32>(1.0, 0.0) * texel);
    let w = lum_at(uv + vec2<f32>(-1.0, 0.0) * texel);
    let hi = max(m, max(max(n, s), max(e, w)));
    let lo = min(m, min(min(n, s), min(e, w)));
    let range = hi - lo;
    if (range < max(0.0312, hi * 0.125)) {
        return vec4<f32>(from_srgb(sharpened(uv, rgbm.rgb)), 1.0);
    }
    let nw = lum_at(uv + vec2<f32>(-1.0, -1.0) * texel);
    let ne = lum_at(uv + vec2<f32>(1.0, -1.0) * texel);
    let sw = lum_at(uv + vec2<f32>(-1.0, 1.0) * texel);
    let se = lum_at(uv + vec2<f32>(1.0, 1.0) * texel);
    // sub-pixel aliasing
    let avg = (2.0 * (n + s + e + w) + nw + ne + sw + se) / 12.0;
    let sub = clamp(abs(avg - m) / range, 0.0, 1.0);
    let sub2 = smoothstep(0.0, 1.0, sub);
    let blend_sub = sub2 * sub2 * 0.75;
    // edge direction
    let horz = abs(nw + sw - 2.0 * w) + abs(n + s - 2.0 * m) * 2.0 + abs(ne + se - 2.0 * e);
    let vert = abs(nw + ne - 2.0 * n) + abs(w + e - 2.0 * m) * 2.0 + abs(sw + se - 2.0 * s);
    let is_h = horz >= vert;
    var stp = select(texel.x, texel.y, is_h);
    let l1 = select(w, n, is_h);
    let l2 = select(e, s, is_h);
    let g1 = abs(l1 - m);
    let g2 = abs(l2 - m);
    let neg = g1 >= g2;
    let grad = max(g1, g2) * 0.25;
    let edge_l = 0.5 * (m + select(l2, l1, neg));
    if (neg) {
        stp = -stp;
    }
    var cuv = uv;
    if (is_h) {
        cuv.y = cuv.y + stp * 0.5;
    } else {
        cuv.x = cuv.x + stp * 0.5;
    }
    let dir = select(vec2<f32>(0.0, texel.y), vec2<f32>(texel.x, 0.0), is_h);
    // walk along the edge both ways
    let steps = array<f32, 12>(1.0, 1.0, 1.0, 1.0, 1.0, 1.5, 2.0, 2.0, 2.0, 2.0, 4.0, 8.0);
    var p_uv = cuv + dir;
    var n_uv = cuv - dir;
    var p_end = lum_at(p_uv) - edge_l;
    var n_end = lum_at(n_uv) - edge_l;
    var p_done = abs(p_end) >= grad;
    var n_done = abs(n_end) >= grad;
    for (var i = 1; i < 12; i = i + 1) {
        if (p_done && n_done) {
            break;
        }
        if (!p_done) {
            p_uv = p_uv + dir * steps[i];
            p_end = lum_at(p_uv) - edge_l;
            p_done = abs(p_end) >= grad;
        }
        if (!n_done) {
            n_uv = n_uv - dir * steps[i];
            n_end = lum_at(n_uv) - edge_l;
            n_done = abs(n_end) >= grad;
        }
    }
    var dp: f32;
    var dn: f32;
    if (is_h) {
        dp = p_uv.x - uv.x;
        dn = uv.x - n_uv.x;
    } else {
        dp = p_uv.y - uv.y;
        dn = uv.y - n_uv.y;
    }
    let near_neg = dn <= dp;
    let end_l = select(p_end, n_end, near_neg);
    let span = dp + dn;
    var blend_edge = 0.0;
    // only when the end of the edge is on the other side of the middle
    if ((m - edge_l < 0.0) != (end_l < 0.0)) {
        blend_edge = 0.5 - min(dp, dn) / span;
    }
    let blend = max(blend_edge, blend_sub);
    var fuv = uv;
    if (is_h) {
        fuv.y = fuv.y + blend * stp;
    } else {
        fuv.x = fuv.x + blend * stp;
    }
    return vec4<f32>(from_srgb(textureSampleLevel(t_src, s_lin, fuv, 0.0).rgb), 1.0);
}
