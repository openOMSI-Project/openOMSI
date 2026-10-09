// Enhanced+: ray-traced reflections (rt.rs), after the main pass.
//
// The enhanced pass left, per pixel, the surface a reflection starts from (its normal, its
// distance from the eye, its roughness) and how much of the reflection lands on the screen
// (GBUF_FORMAT), and drew the picture without the probe's reflection there. At half size
// (each ray from the most reflective of four pixels) a ray goes out along the mirror
// direction (one ray, the same every frame: a rough surface is blurred by the composite
// instead of jittered rays that need frames to settle) and what it meets is
// - on the screen and seen there: the picture's own colour at that place (lit, fogged,
//   reflections and all, exactly as drawn);
// - off the screen or hidden: the hit mesh's mean colour (its texture's and its
//   material's) lit by the sun (a shadow ray from there) and the sky;
// - nothing: the sky probe.
// The composite then adds it, times the weight, to the picture.

// The rays' grid: one per REFL_DIV x REFL_DIV pixels (1: every pixel).
override REFL_DIV: i32 = 1;

@group(0) @binding(5) var t_gbuf: texture_2d<f32>;
@group(0) @binding(6) var t_aux: texture_2d<f32>;
@group(0) @binding(7) var t_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var s_lin: sampler;
@group(0) @binding(9) var t_scene: texture_2d<f32>;
@group(0) @binding(10) var t_probe: texture_cube<f32>;
@group(0) @binding(11) var<uniform> enh: Enhanced;
@group(0) @binding(12) var t_hist: texture_2d<f32>;

fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    let c = enh.sh;
    let e = c[0].rgb * 0.282095
        + c[1].rgb * (0.488603 * n.y) + c[2].rgb * (0.488603 * n.z) + c[3].rgb * (0.488603 * n.x)
        + c[4].rgb * (1.092548 * n.x * n.y) + c[5].rgb * (1.092548 * n.y * n.z)
        + c[6].rgb * (0.315392 * (3.0 * n.z * n.z - 1.0)) + c[7].rgb * (1.092548 * n.x * n.z)
        + c[8].rgb * (0.546274 * (n.x * n.x - n.y * n.y));
    return max(e, vec3<f32>(0.0));
}

// The sky probe towards d at the blur of roughness `rough` (pre-exposed).
fn sky_probe(d: vec3<f32>, rough: f32) -> vec3<f32> {
    let lod = rough * (enh.lights.x - 1.0);
    return textureSampleLevel(t_probe, s_lin, d.xzy, lod).rgb * enh.fog_color.w;
}

fn finite(c: vec3<f32>) -> vec3<f32> {
    let e = bitcast<vec3<u32>>(c) & vec3<u32>(0x7f800000u);
    return select(vec3<f32>(0.0), clamp(c, vec3<f32>(0.0), vec3<f32>(6.0e4)), all(e != vec3<u32>(0x7f800000u)));
}

// The roughness a reflection is spread by (0: a mirror). OMSI's materials say little about
// it, and taken as they are a wet road at 0.12 mirrored like glass: rough surfaces are taken
// rougher.
fn glossy_rough(rough: f32) -> f32 {
    return select(0.0, min(rough * 2.2, 0.8), rough > 0.05);
}

// A GGX microfacet normal around n (alpha = rough^2) for two uniform numbers.
fn ggx_normal(n: vec3<f32>, rough: f32, u: vec2<f32>) -> vec3<f32> {
    let a = max(rough * rough, 1e-4);
    let phi = 2.0 * PI * u.x;
    let ct = sqrt((1.0 - u.y) / (1.0 + (a * a - 1.0) * u.y));
    let st = sqrt(max(1.0 - ct * ct, 0.0));
    return normalize(basis(n) * vec3<f32>(st * cos(phi), st * sin(phi), ct));
}

// Where a point is on the screen: x 1 there and seen (the picture's colour is its own),
// 2 there but the picture shows something further (a gap: the ray went through a cut-out
// texel), 3 there behind something nearer, 0 off the screen; yz its place.
fn on_screen(pt: vec3<f32>) -> vec3<f32> {
    let c = p.view_proj * vec4<f32>(pt, 1.0);
    if (c.w > 0.05) {
        let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
        if (all(uv > vec2<f32>(0.002)) && all(uv < vec2<f32>(0.998))) {
            let q = vec2<i32>(uv * p.size.xy);
            let d = textureLoad(t_depth, q, 0);
            let seen = select(1e9, linear_depth(d), d > 0.0);
            let tol = 0.04 * c.w + 0.15;
            return vec3<f32>(select(select(3.0, 2.0, seen > c.w + tol), 1.0, abs(seen - c.w) < tol), uv);
        }
    }
    return vec3<f32>(0.0);
}

// The light a reflected ray brings back from where it meets the scene: the picture's own
// colour there, else the mesh's mean colour in the sun and the sky's light.
fn hit_light(o: vec3<f32>, d: vec3<f32>, h: Hit, rough: f32) -> vec3<f32> {
    let pt = o + d * h.t;
    let sc = on_screen(pt);
    // (a cut-out mesh - leaves - seen through a gap shows what lies behind it there: the
    // picture's colour all the same, the same for the rays beside it; deciding leaf or gap
    // one ray at a time sprinkled the glass with light and dark specks, #1907)
    let cut = (records[h.record].flags & 1u) != 0u;
    if (sc.x == 1.0 || (cut && sc.x == 2.0)) {
        let seen = finite(textureSampleLevel(t_scene, s_lin, sc.yz, 0.0).rgb);
        // (towards the screen's edge the picture's colour hands over to the mesh's own,
        // which is what the rays beyond the edge get: a hard seam ran across the windows
        // where the reflected street left the screen)
        let edge = min(min(sc.y, 1.0 - sc.y), min(sc.z, 1.0 - sc.z));
        let k = smoothstep(0.0, 0.08, edge);
        if (k >= 0.999) {
            return seen;
        }
        return mix(mesh_light(pt, d, h), seen, k);
    }
    return mesh_light(pt, d, h);
}

// What a ray finds off the screen or hidden there: the hit mesh's mean colour (its
// texture's and its material's) lit by the sun (a shadow ray from there) and the sky, and
// the air along the way.
fn mesh_light(pt: vec3<f32>, d: vec3<f32>, h: Hit) -> vec3<f32> {
    let o = pt - d * h.t;
    let r = records[h.record];
    var albedo = r.color.rgb;
    if (r.tex != 0xffffffffu) {
        albedo = albedo * tex_avg[r.tex].c.rgb;
    }
    if ((r.flags & 2u) != 0u) {
        return albedo * enh.exposure.y;
    }
    // (the face turned towards the ray: no normal is known at the hit)
    let nf = -d;
    var sun = vec3<f32>(0.0);
    if (p.sun.w > 0.0 && dot(nf, p.sun.xyz) > -0.2) {
        if (ray_hit(RT_FORCE_OPAQUE | RT_FIRST_HIT, MASK_SHADOW, 0.0, 1500.0, pt - d * 0.05, p.sun.xyz).kind == RAY_QUERY_INTERSECTION_NONE) {
            sun = enh.sun.rgb * (0.35 + 0.4 * max(dot(nf, p.sun.xyz), 0.0));
        }
    }
    let sky = mix(sh_irradiance(vec3<f32>(0.0, 0.0, 1.0)), sh_irradiance(nf), 0.5) * 0.8;
    var l = albedo / PI * (sun + sky) * enh.exposure.x;
    // the air along the way: from the eye to the surface and on to the hit
    let dist = length(o - p.eye.xyz) + h.t;
    let ext = exp(-(enh.fog.x + enh.fog.w) * dist);
    l = l * ext + enh.fog_color.rgb * enh.exposure.x * (1.0 - ext);
    return finite(l);
}

@compute @workgroup_size(8, 8)
fn cs_reflect(@builtin(global_invocation_id) gid: vec3<u32>) {
    let hp = vec2<i32>(gid.xy);
    let hsize = vec2<i32>(textureDimensions(t_out));
    if (any(hp >= hsize)) {
        return;
    }
    let size = vec2<i32>(p.size.xy);
    // the most reflective of the four pixels
    var px = min(hp * REFL_DIV, size - vec2<i32>(1));
    var g = vec4<f32>(0.0);
    for (var j = 0; j < REFL_DIV; j++) {
        for (var i = 0; i < REFL_DIV; i++) {
            let q = min(hp * REFL_DIV + vec2<i32>(i, j), size - vec2<i32>(1));
            let s = textureLoad(t_gbuf, q, 0);
            if (s.w > g.w) {
                g = s;
                px = q;
            }
        }
    }
    // (a reflection that hardly shows - a pane seen from the cab, a matt surface at a steep
    // angle - is as good from the sky probe: the composite takes that where no ray went)
    if (g.w < 0.012) {
        textureStore(t_out, hp, vec4<f32>(0.0));
        return;
    }
    let aux = textureLoad(t_aux, px, 0);
    let w = g.w;
    let n = normalize(g.xyz / w);
    let dist = aux.r / w;
    let rough = clamp(aux.g / w, 0.0, 1.0);
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) * p.size.zw;
    let view = normalize(world_pos(uv, 0.001) - p.eye.xyz);
    let pt = p.eye.xyz + view * dist;
    var d = reflect(view, n);
    // A glossy surface (a wet road, paint) is no mirror: its rays spread over the GGX lobe,
    // one of 16 fixed directions a pixel in a pattern of 4 x 4 that the composite averages
    // away (the same every frame: nothing to settle, nothing that crawls). Only what is
    // really smooth - a puddle, a pane, still water - mirrors sharply.
    let gloss = glossy_rough(rough);
    if (gloss > 0.0) {
        let cell = u32(hp.x & 3) + u32(hp.y & 3) * 4u;
        let u = vec2<f32>((f32(cell) + 0.5) / 16.0, f32(reverseBits(cell)) * 2.3283064e-10);
        let dd = reflect(view, ggx_normal(n, gloss, u));
        if (dot(dd, n) > 0.02) {
            d = dd;
        }
    }
    // (a reflection into the surface would show what is under it: bent up along it)
    let below = dot(d, n);
    if (below < 0.02) {
        d = normalize(d + n * (0.02 - below));
    }
    var l: vec3<f32>;
    if (dist > p.proj.z) {
        l = sky_probe(d, rough);
    } else {
        let o = pt + n * (0.02 + dist * 0.0015);
        // (a cut-out mesh - leaves, a fence - is taken as solid, and the picture says where
        // its texels are: where it shows a gap there, the ray goes on)
        var start = 0.0;
        l = sky_probe(d, rough);
        for (var k = 0; k < 4; k++) {
            let ch = ray_hit(RT_FORCE_OPAQUE, MASK_SEEN, start, 400.0, o, d);
            if (ch.kind == RAY_QUERY_INTERSECTION_NONE) {
                break;
            }
            var h: Hit;
            h.t = ch.t;
            h.record = ch.instance_custom_data + ch.geometry_index;
            let cut = (records[h.record].flags & 1u) != 0u;
            l = hit_light(o, d, h, rough);
            if (cut) {
                // (off the screen or hidden: as much of the sky through it as its texels leave)
                let sc = on_screen(o + d * h.t).x;
                if (sc != 1.0 && sc != 2.0) {
                    l = mix(sky_probe(d, rough), l, mix(coverage(records[h.record]), 1.0, 0.5));
                }
            }
            break;
        }
    }
    // (no single ray brighter than the sky's brightest: the sun's own glint on the surface
    // is the main pass's, and a ray that found the sun's disc was a speck of light)
    let lum = dot(l, vec3<f32>(0.2126, 0.7152, 0.0722));
    let cap = 2.5 * enh.exposure.x * max(dot(sh_irradiance(vec3<f32>(0.0, 0.0, 1.0)), vec3<f32>(0.2126, 0.7152, 0.0722)), 0.05);
    l = l * min(1.0, cap / max(lum, 1e-5));
    if (p.temporal.z == 8.0) {
        l = vec3<f32>(f32(dist > p.proj.z), rough, 0.0);
    }
    textureStore(t_out, hp, vec4<f32>(l, dist));
}

// --- the composite: the traced reflection times its weight, added to the picture (blended
// additively). From the rays around the pixel at its own distance; a rough surface gathers
// from further round.
struct VsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return VsOut(vec4<f32>(x, y, 0.0, 1.0));
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let g = textureLoad(t_gbuf, px, 0);
    if (g.w < 0.002) {
        discard;
    }
    let aux = textureLoad(t_aux, px, 0);
    let dist = aux.r / g.w;
    let rough = clamp(aux.g / g.w, 0.0, 1.0);
    let hsize = vec2<i32>(textureDimensions(t_hist));
    let hq = (vec2<f32>(px) + vec2<f32>(0.5)) / f32(REFL_DIV) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(hq));
    let fr = hq - floor(hq);
    let gloss = glossy_rough(rough);
    // (a mirror-smooth pane takes the rays of the pixels round it as well: one ray per
    // pixel, the same every frame, left every odd one standing out as a speck, #1907)
    let reach = select(1, clamp(i32(round(1.5 + gloss * 4.0)), 2, 4), gloss > 0.0);
    let tol = (0.02 + gloss * 0.06) * dist + 0.05;
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var j = -reach; j <= reach + 1; j++) {
        for (var i = -reach; i <= reach + 1; i++) {
            let q = clamp(base + vec2<i32>(i, j), vec2<i32>(0), hsize - vec2<i32>(1));
            let s = textureLoad(t_hist, q, 0);
            let dx = abs(f32(i) - fr.x);
            let dy = abs(f32(j) - fr.y);
            let k = max(1.0 - dx / (f32(reach) + 1.0), 0.0) * max(1.0 - dy / (f32(reach) + 1.0), 0.0);
            // (weighed down by its brightness: a single bright ray among dark ones is a
            // speck, not the light of the place)
            let w = k * select(0.0, 1.0, s.a > 0.0 && abs(s.a - dist) < tol) / (1.0 + dot(s.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
            sum += s.rgb * w;
            wsum += w;
        }
    }
    var l: vec3<f32>;
    if (wsum < 1e-4) {
        // (no ray at this distance nearby: the sky probe the way a mirror would see)
        let uv = (vec2<f32>(px) + vec2<f32>(0.5)) * p.size.zw;
        let view = normalize(world_pos(uv, 0.001) - p.eye.xyz);
        l = sky_probe(reflect(view, normalize(g.xyz)), rough);
    } else {
        l = sum / wsum;
    }
    if (p.temporal.z >= 8.0) {
        return vec4<f32>(select(l * 0.3, vec3<f32>(0.0), p.temporal.z == 10.0) + vec3<f32>(0.0, 0.0, g.w * 10.0), 0.0);
    }
    return vec4<f32>(l * g.w, 0.0);
}
