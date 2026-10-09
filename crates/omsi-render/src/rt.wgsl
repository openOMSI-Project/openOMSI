// Enhanced+: ray-traced lighting with hardware ray queries (rt.rs).
//
// Per pixel of the depth prepass, at full size, and the same every frame (nothing noisy that
// needs frames to settle, so nothing that smears or crawls when the camera moves):
// - the sun's shadow: one ray towards a point of the sun's disc, one of 16 of a fixed
//   pattern that repeats every 4 x 4 pixels;
// - ambient occlusion: two rays over the surface's hemisphere, their directions one of 32 of
//   a fixed pattern that repeats every 4 x 4 pixels; the filter averages the pattern away
//   over 5 x 5 pixels at the same depth.

// the filter's input: this frame's rays
@group(0) @binding(5) var t_in: texture_2d<f32>;
@group(0) @binding(6) var t_raw: texture_2d<f32>;
@group(0) @binding(7) var t_out: texture_storage_2d<rgba16float, write>;

// The k-th of n points of a Hammersley set.
fn hammersley(k: u32, n: u32) -> vec2<f32> {
    return vec2<f32>((f32(k) + 0.5) / f32(n), f32(reverseBits(k)) * 2.3283064e-10);
}

@compute @workgroup_size(8, 8)
fn cs_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = vec2<i32>(gid.xy);
    if (f32(px.x) >= p.size.x || f32(px.y) >= p.size.y) {
        return;
    }
    let depth = textureLoad(t_depth, px, 0);
    if (depth <= 0.0) {
        // the sky: nothing to shade
        textureStore(t_out, px, vec4<f32>(1.0, 0.0, 1.0, 0.0));
        return;
    }
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) * p.size.zw;
    let world = world_pos(uv, depth);
    let lin = linear_depth(depth);
    let dist = length(world - p.eye.xyz);
    if (dist > p.proj.z) {
        // beyond the traced range: the shadow map (b < 0) and no occlusion
        textureStore(t_out, px, vec4<f32>(1.0, lin, -1.0, 0.0));
        return;
    }
    let n = depth_normal(px, world, depth);
    // (off the surface by more than its depth's precision and the normal's error)
    let o = world + n * (0.03 + dist * 0.002);
    // --- the sun (solid casters only: the cut-out ones are in the shadow map, see the main
    // pass)
    let cell = u32(px.x & 3) + u32(px.y & 3) * 4u;
    var vis = 1.0;
    if (p.sun.w > 0.0) {
        // (towards one of 16 points of the sun's disc, a pattern of 4 x 4 pixels the filter
        // averages: the shadow's edge softens with the distance to its caster, as the
        // sun's own does, and nothing is noisy)
        let su = hammersley(cell, 16u);
        let sb = basis(p.sun.xyz);
        let sr = sqrt(su.x) * p.sun.w;
        let sd = normalize(p.sun.xyz + sb[0] * cos(6.2831853 * su.y) * sr + sb[1] * sin(6.2831853 * su.y) * sr);
        vis = select(1.0, 0.0, ray_hit(RT_FORCE_OPAQUE | RT_FIRST_HIT, MASK_SHADOW, 0.0, 2000.0, o, sd).kind != RAY_QUERY_INTERSECTION_NONE);
        if (vis > 0.0) {
            // through a bus's or a car's window: tinted glass lets only part of it in
            vis = select(1.0, 0.45, ray_hit(RT_FORCE_OPAQUE | RT_FIRST_HIT, MASK_GLASS, 0.0, 200.0, o, sd).kind != RAY_QUERY_INTERSECTION_NONE);
        }
    }
    // --- the sky's occlusion
    let b = basis(n);
    var ao = 0.0;
    for (var j = 0u; j < 2u; j++) {
        let u = hammersley(cell * 2u + j, 32u);
        let rr = sqrt(u.x);
        let phi = 2.0 * PI * u.y;
        let d = b * vec3<f32>(rr * cos(phi), rr * sin(phi), sqrt(max(1.0 - u.x, 0.0)));
        let h = trace(o, d, p.proj.w, MASK_SEEN, vec3<u32>(cell, j, 7u), false);
        // (a hit at the far end of the radius takes away less than one right beside it)
        ao += select(1.0, mix(0.35, 1.0, smoothstep(0.0, 1.0, h.t / p.proj.w)), h.t >= 0.0);
    }
    textureStore(t_out, px, vec4<f32>(ao * 0.5, lin, vis, 1.0));
}

// A workgroup's pixels and a border of two round them, read once.
var<workgroup> tile: array<vec4<f32>, 144>;

// The rays filtered at the pixel's own depth over 5 x 5 pixels (the whole pattern of
// directions). The pattern repeats every 4 pixels, so a plain 5 x 5 box counted one row and
// one column of it twice - which ones, by the pixel's place in the pattern: a soft shadow's
// edge kept a fixed 4 x 4 checker on the screen, which crawled over the surfaces as the
// camera moved. The outer row and column on each side weigh a half: they are the same
// pattern cells, and every one of the 16 counts exactly once.
@compute @workgroup_size(8, 8)
fn cs_denoise(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let origin = vec2<i32>(wid.xy) * 8 - vec2<i32>(2);
    let size = vec2<i32>(p.size.xy);
    for (var k = i32(li); k < 144; k += 64) {
        let q = origin + vec2<i32>(k % 12, k / 12);
        tile[k] = textureLoad(t_in, clamp(q, vec2<i32>(0), size - vec2<i32>(1)), 0);
    }
    workgroupBarrier();
    let px = vec2<i32>(gid.xy);
    if (f32(px.x) >= p.size.x || f32(px.y) >= p.size.y) {
        return;
    }
    let lp = px - origin;
    let c = tile[lp.y * 12 + lp.x];
    if (c.g <= 0.0 || c.b < 0.0 || p.temporal.z == 7.0) {
        textureStore(t_out, px, c);
        return;
    }
    let tol = 0.012 * c.g + 0.02;
    var ao = 0.0;
    var aw = 0.0;
    var sun = 0.0;
    var sw = 0.0;
    for (var j = -2; j <= 2; j++) {
        for (var i = -2; i <= 2; i++) {
            let s = tile[(lp.y + j) * 12 + lp.x + i];
            if (s.g > 0.0 && s.b >= 0.0 && abs(s.g - c.g) < tol) {
                let w = select(1.0, 0.5, abs(i) == 2) * select(1.0, 0.5, abs(j) == 2);
                ao += s.r * w;
                aw += w;
                sun += s.b * w;
                sw += w;
            }
        }
    }
    textureStore(t_out, px, vec4<f32>(ao / max(aw, 1e-4), c.g, sun / max(sw, 1e-4), 1.0));
}
