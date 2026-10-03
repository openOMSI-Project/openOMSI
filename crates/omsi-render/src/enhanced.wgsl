// Enhanced graphics: the main pass's own lighting model. Energy-conserving Lambert diffuse
// and GGX specular with Fresnel, lit by the sun (soft shadows whose penumbra grows with the
// distance to the blocker), the sky and the ground (spherical harmonics, occluded by the
// SSAO), a reflection probe of the sky for the reflections, point and spot lights falling
// off with the square of the distance, and the air in between. OMSI's material data
// decides the surface: [matl_envmap] factor x mask is how much it reflects and how smooth
// it is, the o3d specular power how rough a surface without one is, [matl_bumpmap] bends
// the normal. Everything is drawn pre-exposed into the high-range target.

@group(0) @binding(12) var t_probe: texture_cube<f32>;

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Height-correlated Smith visibility (G / (4 n.l n.v)).
fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

fn f_schlick(f0: vec3<f32>, c: f32) -> vec3<f32> {
    let f = pow(1.0 - clamp(c, 0.0, 1.0), 5.0);
    return f0 + (vec3<f32>(1.0) - f0) * f;
}

// The split-sum environment term (Karis' analytic fit).
fn env_brdf(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = c0 * rough + c1;
    let a004 = min(r.x * r.x, pow(2.0, -9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + vec3<f32>(ab.y);
}

// Ambient occlusion that lets bright surfaces bounce some light back into their own
// corners (Jimenez' multi-bounce fit), so white walls do not get grey creases.
fn ao_bounce(ao: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - vec3<f32>(0.3324);
    let b = -4.7951 * albedo + vec3<f32>(0.6417);
    let c = 2.7552 * albedo + vec3<f32>(0.6903);
    return max(vec3<f32>(ao), ((a * ao + b) * ao + c) * ao);
}

// World direction to the probe's cube coordinates (and back: the swap is its own inverse).
fn to_cube(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(d.x, d.z, d.y);
}

// What a raindrop on a pane shows (`rain_light`): the sky probe that way, blurred a little
// by the drop's small lens, and low down the street's light in place of the probe's
// horizon (see the reflections in `shade_enhanced`).
fn rain_env_enhanced(d: vec3<f32>, lod: f32) -> vec3<f32> {
    let e = textureSampleLevel(t_probe, s_lin, to_cube(d), lod).rgb * enh.fog_color.w;
    let surround = enh.fog_color.rgb * (0.25 / 0.9);
    return mix(surround, e, smoothstep(-0.05, 0.35, d.z));
}

// A fixed, deterministic PCF kernel (shader.wgsl's SHADOW_OFFSETS). The old PCSS blocker
// search was unstable for alpha-tested foliage: a few leaves entering or leaving its
// 12-sample search changed the penumbra radius, producing checkerboard patches and
// camera-driven strips. The offsets are fixed in shadow-map space, so the only thing that
// can change is the actual caster. `thin`: foliage, whose normals OMSI points up for even
// lighting - no receiver plane can be taken from them, a leaf compares at its own depth.
fn sun_shadow_soft(world_in: vec3<f32>, n: vec3<f32>, thin: bool) -> f32 {
    if (camera.shadow.x < 0.5) {
        return 1.0;
    }
    // (half a metre towards the sun for a leaf, as in `sun_shadow`)
    let world = world_in + select(vec3<f32>(0.0), camera.sun_dir.xyz * 0.5, thin);
    let ndl = clamp(dot(n, camera.sun_dir.xyz), 0.0, 1.0);
    let texel = camera.shadow.y;
    let close = shadow_close(world, n, ndl, thin);
    if (close.y >= 0.999) {
        return close.x;
    }
    let near_lp = camera.light_view_proj * vec4<f32>(world + shadow_push_near(n, ndl), 1.0);
    let near_uv = vec2<f32>(near_lp.x * 0.5 + 0.5, 0.5 - near_lp.y * 0.5);
    let near_edge = max(abs(near_uv.x - 0.5), abs(near_uv.y - 0.5));
    let near_valid = near_edge < 0.48 && near_lp.z >= 0.0 && near_lp.z <= 1.0;
    var near_slope = shadow_receiver_slope(camera.light_view_proj, n);
    var far_slope = shadow_receiver_slope(camera.light_view_proj_far, n);
    if (thin) {
        near_slope = vec2<f32>(0.0);
        far_slope = vec2<f32>(0.0);
    }
    // Blend cascades by radial distance, not by the near map's projected square.  The
    // projected-edge test made the blend factor change when the camera moved sideways or
    // turned, even for a stationary receiver; that reads as a full shadow shimmer at the
    // cascade boundary.  The light boxes still validate coverage above, while this gives
    // the overlap a camera-centred, continuous transition.
    let radial = distance(world, camera.cam_pos.xyz);
    let near_weight = select(
        0.0,
        1.0 - smoothstep(camera.shadow.z * 0.62, camera.shadow.z * 0.92, radial),
        near_valid,
    );
    // Each cascade is filtered only where it counts: sampling both everywhere (32 compares
    // a pixel) halved the enhanced picture's frame rate against shadows off.
    var near_value = 1.0;
    if (near_weight > 0.001) {
        near_value = shadow_pcf_near(near_uv, near_lp.z, near_slope, texel);
    }
    near_value = mix(near_value, close.x, close.y);
    if (near_weight >= 0.999) {
        return near_value;
    }
    let flp = camera.light_view_proj_far * vec4<f32>(world + shadow_push_far(n, ndl), 1.0);
    let fuv = vec2<f32>(flp.x * 0.5 + 0.5, 0.5 - flp.y * 0.5);
    let far_valid = fuv.x >= 0.0 && fuv.x <= 1.0 && fuv.y >= 0.0 && fuv.y <= 1.0 && flp.z >= 0.0 && flp.z <= 1.0;
    var far_safe = 1.0;
    if (far_valid) {
        let far_value = shadow_pcf_far(fuv, flp.z, far_slope, texel);
        let far_edge = max(abs(fuv.x - 0.5), abs(fuv.y - 0.5));
        far_safe = mix(1.0, far_value, clamp((0.5 - far_edge) * 12.0, 0.0, 1.0));
    }
    return mix(far_safe, near_value, near_weight);
}

// The sun's shadow at a point by one compare in the finest cascade that holds it (the close
// one, else the near one; lit beyond): for glass (see fs_enhanced).
fn sun_shadow_hard(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (camera.shadow.x < 0.5) {
        return 1.0;
    }
    let ndl = clamp(dot(n, camera.sun_dir.xyz), 0.0, 1.0);
    let range = camera.flags.w;
    if (range > 0.0 && distance(world, camera.cam_pos.xyz) < range * 0.75) {
        let lp = camera.light_view_proj_close * vec4<f32>(world + shadow_push_close(n, ndl), 1.0);
        let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
        if (max(abs(uv.x - 0.5), abs(uv.y - 0.5)) < 0.48 && lp.z >= 0.0 && lp.z <= 1.0) {
            let scale = select(1.0, camera.post.w, camera.post.w > 0.0);
            let a = vec2<f32>(uv.x * 0.5 * scale + 0.5, uv.y * scale);
            return textureSampleCompareLevel(t_shadow, s_shadow, a, lp.z - SHADOW_BIAS_CLOSE / SHADOW_DEPTH_RANGE);
        }
    }
    let lp = camera.light_view_proj * vec4<f32>(world + shadow_push_near(n, ndl), 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (max(abs(uv.x - 0.5), abs(uv.y - 0.5)) < 0.48 && lp.z >= 0.0 && lp.z <= 1.0) {
        return textureSampleCompareLevel(t_shadow, s_shadow, vec2<f32>(uv.x * 0.5, uv.y), lp.z - SHADOW_BIAS_NEAR / SHADOW_DEPTH_RANGE);
    }
    return 1.0;
}

// What a surface reflects and scatters: diffuse albedo, specular colour at normal
// incidence, perceptual roughness.
struct Surface {
    albedo: vec3<f32>,
    f0: vec3<f32>,
    rough: f32,
};

// The point and spot lights of the pixel's grid cell: diffuse and specular.
// `thin`: foliage, lit from whichever side the lamp is on (see the sun below).
fn lamp_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, sf: Surface, thin: bool) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let cell = camera.light_grid.z;
    let side = u32(camera.light_grid.w);
    if (cell <= 0.0 || side == 0u) {
        return sum;
    }
    let f = (p.xy - camera.light_grid.xy) / cell;
    let x = i32(floor(f.x));
    let y = i32(floor(f.y));
    if (x < 0 || y < 0 || x >= i32(side) || y >= i32(side)) {
        return sum;
    }
    let a = max(sf.rough * sf.rough, 0.02);
    let nv = max(dot(n, v), 1e-4);
    let base = (u32(y) * side + u32(x)) * CELL_CAP;
    for (var j = 0u; j < CELL_CAP; j = j + 1u) {
        let li = grid[base + j];
        if (li == 0xffffffffu) {
            break;
        }
        let l = lights[li];
        let d = l.pos.xyz - p;
        let dist2 = dot(d, d);
        // (pos.w is 0 on a spot light, which the vanilla shader is to pass by)
        let range = l.extra.w;
        if (dist2 >= range * range) {
            continue;
        }
        let dist = sqrt(dist2);
        let ld = d / max(dist, 1e-3);
        // OMSI's rule: full within the core, inverse-square beyond it; windowed to zero at
        // the range so the grid cut-off does not show
        var core = l.extra.y;
        if (core <= 0.0) {
            core = range * 0.125;
        }
        let q = dist2 / (range * range);
        let window = (1.0 - q * q) * (1.0 - q * q);
        var e = min(1.0, core * core / max(dist2, 1e-3)) * window;
        if (l.dir.w > -1.5) {
            let cd = dot(-ld, l.dir.xyz);
            e = e * smoothstep(l.dir.w, l.extra.x, cd);
            if (l.extra.z > 0.0) {
                // a low beam: brightest just under its cut-off, where it reaches far down
                // the road (the gain keeps the light on a flat road from falling off with
                // the cube of the drop angle), back to the plain cone above the horizon
                let drop = ld.z;
                let axis = max(-l.dir.z, 0.05);
                let gain = clamp(axis * axis / max(drop * drop, 1e-6), 1.0, l.extra.z);
                e = e * mix(1.0, gain, smoothstep(-0.04, 0.0, drop));
            }
        }
        if (e <= 0.0) {
            continue;
        }
        let irr = l.color.rgb * l.color.w * enh.lights.y * e;
        let nl = dot(n, ld);
        if (thin) {
            sum = sum + irr * (0.45 + 0.25 * nl) * sf.albedo / PI;
            continue;
        }
        if (nl <= 0.0) {
            continue;
        }
        let h = normalize(ld + v);
        let spec = d_ggx(max(dot(n, h), 0.0), a) * v_smith(nv, nl, a) * f_schlick(sf.f0, dot(v, h));
        sum = sum + irr * nl * (sf.albedo / PI + spec);
    }
    return sum;
}

// The light inside a cab relative to the average light outside: a bus's big windows let in
// most of the sky, and the dashboard lies right under the windscreen.
const CAB_AMBIENT: f32 = 1.15;

// How far the puddle threshold drops with the wetness: see the puddle mask in `shade_enhanced`.
const PUDDLE_SPREAD: f32 = 0.45;

/// The mip level a pixel's footprint asks for, in levels of the texture whose size is
/// `texels` (the usual `log2` of the larger derivative, held at 0 and up). An LED panel is
/// sampled with this, held at `enh.led.y` (`Lighting::led_mips`): 0 point-samples it, which
/// is the sharpest and shimmers worst - a regular dot grid is the worst case for a point
/// sample - and every level the chain is allowed to take is a 2x2 average that a point
/// sample of the level below does not have. (The derivatives have to be taken in uniform
/// control flow: the panels are per draw, so a call inside the material's branch is not.)
fn led_lod(uv: vec2<f32>, texels: vec2<f32>) -> f32 {
    let dx = dpdx(uv) * texels;
    let dy = dpdy(uv) * texels;
    return max(0.5 * log2(max(dot(dx, dx), dot(dy, dy))), 0.0);
}

// A tangent-space normal `tn` (a PBR normal map's) turned into the world about the
// surface normal `n`, with the tangent frame taken from how the position and the uv change
// across the pixel (no tangents in OMSI's meshes). `tn.y` down the texture, as Direct3D's
// normal maps have it.
fn perturb_normal(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, tn: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let m = max(dot(t, t), dot(b, b));
    if (m < 1e-20) {
        return n;
    }
    let k = inverseSqrt(m);
    return safe_normal(t * k * tn.x + b * k * tn.y + n * max(tn.z, 0.05));
}

// The enhanced pass's two targets: the picture, and the screen mask (r: 1 on the bus's own
// screens, carried by the coverage of what is drawn over them; g: 1 on an LED panel's own
// dots, see MASK_FORMAT; b is the reflected-light weight of wet puddles).
struct EnhancedOut {
    @location(0) color: vec4<f32>,
    @location(1) mask: vec4<f32>,
};

@fragment
fn fs_enhanced(in: FsIn) -> EnhancedOut {
    var puddle_weight = vec2<f32>(0.0);
    let c = shade_enhanced(in, &puddle_weight, false, camera.cam_pos.xyz);
    let screen = material.flags.x > 0.5;
    // an LED panel's dots stay in the glow's source (`post.wgsl`), the other screens'
    // letters stay out of it
    let led = select(0.0, 1.0, material.emissive.w < -1.5);
    var out: EnhancedOut;
    out.color = c;
    // The sub-0.5 range of g carries water's occluded sky weight; LED detection uses
    // step(0.5, g). This keeps scene hits independent of sky ambient occlusion.
    // A vehicle's shadow is light blocked from the road, not a new dry surface. Its
    // colour still blends normally, but it must preserve the road's reflection mask.
    let coverage = select(select(c.a, 1.0, screen), 0.0, in.params2.w > 1.5);
    out.mask = vec4<f32>(select(0.0, 1.0, screen), max(led, puddle_weight.y * 0.49), puddle_weight.x, coverage);
    return out;
}

// A self-lit picture (a display, a light-mapped surface lit fully) at its own colour after
// the tone curve: PBR Neutral leaves colours below 0.76 as they are, less a black offset of
// 0.04, and bleaches what is brighter - a display's yellow-green text came out olive. So
// the colour is kept under the knee and the offset added back.
fn display_level(t: vec3<f32>) -> vec3<f32> {
    let peak = max(t.r, max(t.g, t.b));
    let tk = t * min(1.0, 0.76 / max(peak, 1e-3));
    return tk + 0.04 * smoothstep(vec3<f32>(0.0), vec3<f32>(0.08), tk);
}

fn shade_enhanced(in: FsIn, puddle_weight: ptr<function, vec2<f32>>, capture: bool, eye: vec3<f32>) -> vec4<f32> {
    if (material.emissive.w > 1.5) {
        // a pane's film of water: drops, not the sliding texture (see `rain_glass`), each a
        // lens that mirrors the sky probe and shows it upside down through itself
        let v = camera.cam_pos.xyz - in.world;
        let vn = normalize(v);
        let in_cab = inside_vehicle(camera.cam_pos.xyz) * near_player_vehicle(in.world) > 0.5;
        let g = rain_glass(in.world, in.uv - in.params.zw, in.normal, window_wetness(in), camera.post.y, in_cab);
        if (g.cover <= 0.001 && g.mist <= 0.001) { return vec4<f32>(0.0); }
        let through = rain_through(g, vn);
        let valid = dot(through, through) > 1e-4;
        // (the picture behind is as the HDR pass drew it: exposed already)
        let seen = select(vec3<f32>(0.0), rain_behind(in.world, through, rain_env_enhanced(normalize(select(g.out, through, valid)), 2.0), 1.0 / max(enh.exposure.x, 1e-6), g.mist), valid);
        let mirrored = rain_env_enhanced(reflect(-vn, g.n), 1.0);
        let d = rain_light(g, vn, through, mirrored, seen, sh_irradiance(g.out) / PI * 0.9, enh.sun.rgb / PI);
        let aer = air(-normalize(v), fog_distance(in.world), camera.cam_pos.z - enh.fog.z, in.world.z - enh.fog.z);
        // Drops a few pixels across are a pane's sparkle close up; further off each
        // darker rim was a black fleck, and a bus seen from the pavement in the rain wore
        // windows peppered black. Drops smaller than a pixel give way to the mist now
        // (`rain_dome`), and the rest fade out over the first fifteen metres.
        let near = 1.0 - smoothstep(5.0, 15.0, length(v));
        return vec4<f32>(d.rgb * enh.exposure.x * aer.a, d.a * near);
    }
    // --- the surface's texture and alpha, exactly as the vanilla pass reads them
    let terrain = material.extra.x > 0.5;
    var duv = tex_address(in.uv);
    if (terrain) {
        duv = in.uv * material.extra.z;
    }
    // An LED panel is sampled at the level its screen footprint asks for, held at
    // `enh.led.y` (`Led mip strength`): its dots keep their gaps much further out than the
    // full chain allows, and the shimmer is a fraction of a full-resolution sample's. The
    // levels are worked out here and not inside the branch: derivatives are undefined in
    // non-uniform control flow, and which of the two samples runs is per draw. When the
    // setting does not bite, the plain (anisotropic) sample of the hardware is the better
    // one and stays.
    let pic_lod = led_lod(duv, vec2<f32>(textureDimensions(t_diffuse)));
    let led_pic = material.emissive.w < -1.5 && enh.led.y < pic_lod;
    var tex = diffuse_border(textureSample(t_diffuse, s_diffuse, duv), duv);
    if (led_pic) {
        tex = diffuse_border(textureSampleLevel(t_diffuse, s_diffuse, duv, enh.led.y), duv);
    }
    let diffuse_a = tex.a;
    // (without the [texcoordtransX/Y] offset: the transmap, night map and light map stay
    // in place, see fs_main)
    let buv = tex_address(in.uv - in.params.zw);
    let msk_lod = led_lod(buv, vec2<f32>(textureDimensions(t_trans)));
    if (terrain && material.extra.y > 0.0) {
        let det = textureSample(t_light, s_diffuse, in.uv * material.extra.y);
        tex = vec4<f32>(clamp(tex.rgb * det.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), tex.a);
    }
    if (material.params.z > 0.5) {
        // (an LED panel's `\S:n` mask is taken the same way: the dots stay dots when the
        // panel is small, without the full-resolution shimmer)
        var tm = sample_transmap(buv);
        if (material.emissive.w < -1.5 && enh.led.y < msk_lod) {
            tm = textureSampleLevel(t_trans, s_diffuse, buv, enh.led.y);
        }
        tex.a = select(1.0, tm.a, material.params.w > 0.5);
        if (terrain && material.params.x > 1.5) {
            // Coverage belongs to the brush mask, not the angle-dependent diffuse mip.
            tex.a = smoothstep(0.32, 0.68, tex.a);
        }
    }
    let mode = material.params.x;
    // Keep the filtered fractional coverage, but tighten its transition around the cutout
    // edge before MSAA turns it into sample coverage. The MSAA depth prepass skips these
    // draws so uncovered samples keep the depth and colour of the scene behind them.
    if ((ALPHA_TEST || capture) && mode > 0.5 && mode < 1.5) {
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
    var alpha = tex.a * material.color.a;
    if (mode < 0.5) {
        alpha = 1.0;
    }
    alpha = alpha * min(window_wetness(in), 1.0);
    let pre = enh.exposure.x;
    let to_cam = eye - in.world;
    let dist = length(to_cam);
    let v = to_cam / max(dist, 1e-4);
    let h_cam = camera.cam_pos.z - enh.fog.z;
    let h_pt = in.world.z - enh.fog.z;
    // (no fog inside the cab: only the part of the way outside the bus is misty)
    let aer = air(-v, fog_distance(in.world), h_cam, h_pt);
    if (material.params.y > 0.5) {
        // unlit (mirror glass, script and text textures): shown at their own brightness,
        // under the tone curve's knee (`display_level`), and `exposure.y` undoes the
        // metering (see ExposureLog in lib.rs)
        let t = tex.rgb * material.color.rgb;
        // A mirror (params.y 0.9) is no display: its picture is the street drawn a moment
        // ago, dark at night. Brightened like a display by the metering (up to 1.6 in the
        // dark) it showed a street far brighter than the one through the windscreen.
        let lift = select(enh.exposure.y, min(enh.exposure.y, 1.0), material.params.y < 0.95);
        let c = display_level(t) * lift;
        return vec4<f32>(c * aer.a + aer.rgb * pre, alpha);
    }
    let outside = weather_outside_n(in.world, safe_normal(in.normal), terrain, in.params2.w);
    // the normal as the content gives it: never turned towards the viewer (Direct3D does
    // not either, and OMSI's foliage points every leaf's normal up so the whole crown is lit
    // evenly - turned round, the crown went dark above the horizon line)
    var n = safe_normal(in.normal);
    // the map's water (`MaterialExtra::water`): small waves running over it ripple its
    // normal - fading with the distance, where they would only flicker - so that what it
    // mirrors breaks up as on a lake instead of standing in it as in a pane of glass (#841)
    let is_water = material.ambient.w > 1.5;
    if (is_water) {
        n = normalize(n + vec3<f32>(water_ripple(world_pattern_xy(in.world), camera.post.y) * clamp(1.0 - dist / 250.0, 0.2, 1.0), 0.0));
    }
    // Leaves and fences transmit light. A terrain road-cut mask only removes ground:
    // its remaining pixels must shade like the uncut ground on terrain-mapped splines.
    let thin = !terrain && mode > 0.5 && mode < 1.5;
    let has_env = material.params2.y > 0.0;
    // A blended transmap body is a masked paint surface, not glass. Traffic cars often
    // use this material layout for their body; depth-disabled blends remain glass.
    // Window assets are not consistent about carrying an envmap.  The reliable signal
    // is an explicitly named window, or a depth-disabled blended layer with env/transmap
    // data that identifies a pane rather than a dirt/text overlay. This keeps bus
    // panes on the reflection/transmission path after the material-depth repair.
    // Painted terrain also blends a transmap without writing depth. It is never glass:
    // Fresnel opacity on its empty mask pixels darkens every lower layer at grazing angles.
    let glass = !terrain && mode > 1.5 && (material.bump.z > 0.5 || material.emissive.w > 0.5) &&
        (has_env || material.params.z > 0.5 || material.emissive.w > 0.5);
    let painted_transmap = material.params.z > 0.5 && !glass;
    // An envmap on opaque vehicle paint is legacy material data, not a request to make
    // the whole body behave like glass. Keep the diffuse/sun response, but use the
    // ordinary rough dielectric path for the body.
    // Foliage and other alpha-tested assets are diffuse silhouettes, not polished surfaces.
    // Letting the generic probe term reflect them creates white sparkles at grazing angles;
    // the photographed envmap is reserved for materials that explicitly request it.
    // (opaque materials reflect as well - chrome handrails, bumpers and wheel trims are
    // opaque with a sphere map, and left out they showed no reflection at all in the
    // enhanced picture, #266 - but through the clear-coat path below: metal only where a
    // mask of its own says so)
    // (not the water: its sphere map is OMSI's stand-in for the sky it mirrors, which the
    // probe holds itself - drawn as paint's photo, it lay on the water as green specks)
    let reflective_env = has_env && !painted_transmap && !thin && !is_water;
    // see-through glass is seen from either side: from inside the bus its normal points
    // away (the vanilla pass takes the angle either way round too); taken as it is, the
    // grazing Fresnel turned the whole windscreen into a milky mirror
    var geo_n = n;
    if (glass && dot(n, v) < 0.0) {
        n = -n;
        geo_n = n;
    }
    if (reflective_env && material.bump.y > 0.5) {
        // [matl_bumpmap]: the height map's slope bends the normal (tangent frame from the
        // screen-space derivatives of position and texture coordinate)
        let slope = bump_offset(duv);
        let dp1 = dpdx(in.world);
        let dp2 = dpdy(in.world);
        let du1 = dpdx(duv);
        let du2 = dpdy(duv);
        let dp2p = cross(dp2, n);
        let dp1p = cross(n, dp1);
        let t = dp2p * du1.x + dp1p * du2.x;
        let b = dp2p * du1.y + dp1p * du2.y;
        let inv = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-12));
        n = normalize(n - (t * inv * slope.x + b * inv * slope.y) * 0.6);
    }
    var albedo = tex.rgb * material.color.rgb;
    var detail_factor = 1.0;
    if (camera.flags.x > 0.5 && (terrain || in.params2.w > 0.5)) {
        let k = clamp(1.0 - (dist - 25.0) / 120.0, 0.0, 1.0);
        let pattern_xy = world_pattern_xy(in.world);
        detail_factor = 1.0 + (detail_noise(pattern_xy) - 0.5) * 0.42 * k;
        albedo = albedo * detail_factor;
    }
    // --- the material in physical terms
    var refl = 0.0;
    if (reflective_env) {
        refl = clamp(min(material.params2.y, 1.0) * reflection_mask(duv, diffuse_a), 0.0, 1.0);
    }
    var metal = 0.0;
    var f0 = vec3<f32>(0.04);
    var rough = 0.8;
    if (terrain) {
        rough = 0.92;
        f0 = vec3<f32>(0.03);
    } else if (glass) {
        // OMSI's window materials are deliberately very transparent. Raise the base
        // dielectric reflection slightly so the outside scene remains readable through
        // the pane instead of leaving a flat pale surface at normal incidence.
        rough = 0.04;
        // the pane's [matl_envmap] factor says how much it mirrors (its alpha is its
        // transparency, never a mask): from glass's own 4 % up to 12 % for a factor of 1
        // (0.4 on the Scania's panes). Up to 26 % as before, every window of every bus
        // was a mirror - far more than the original's panes reflect (issue #176).
        f0 = vec3<f32>(clamp(0.04 + 0.08 * min(material.params2.y, 1.0), 0.04, 0.12));
    } else if (reflective_env) {
        // Paint reflects its few per cent through a smooth clear coat; much more than a few
        // per cent is polished metal - but only where the model says so with a mask of its
        // own ([matl_envmap_mask]). Nearly every car body carries `[matl_envmap] … 1` with
        // no mask, which OMSI takes as "reflect the sphere map fully" and not as chrome:
        // read as metalness it made a Golf's bonnet a mirror, in which the envmap photo's
        // trees stood as contour lines across the paint at close range.
        let masked = (u32(material.params2.w + 0.5) & 1u) != 0u;
        let metal_ok = (u32(material.params2.w + 0.5) & 4u) != 0u;
        metal = select(0.0, smoothstep(0.3, 0.85, refl), masked || metal_ok);
        f0 = mix(vec3<f32>(clamp(refl, 0.02, 0.08)), mix(albedo, vec3<f32>(1.0), 0.4) * refl, metal);
        rough = mix(max(0.3 - 0.12 * smoothstep(0.0, 0.25, refl), select(0.22, 0.0, masked || metal_ok)), 0.14, metal);
    } else if (!thin && material.specular.w > 0.0 && dot(material.specular.rgb, vec3<f32>(1.0)) > 0.05) {
        // the o3d material's Blinn-Phong power as GGX roughness
        rough = clamp(sqrt(sqrt(2.0 / (material.specular.w + 2.0))), 0.4, 0.9);
    } else if (thin) {
        rough = 0.7;
    }
    if (is_water) {
        // water: a dielectric of 2 % at normal incidence, nearly a mirror where it is seen
        // flat, smooth but for its waves
        f0 = vec3<f32>(0.02);
        rough = 0.06;
        metal = 0.0;
    }
    // --- a PBR set beside the diffuse texture (`foo_n.png`, `foo_r` / `_m` / `_ao` or
    // `foo_orm`: see omsi_texture::pbr): the normal map bends the normal, the packed map
    // gives the occlusion, roughness and metalness in place of the guesses above
    var pbr_ao = 1.0;
    if (material.pbr.x > 0.5 && !terrain) {
        var tn = textureSample(t_pbr_normal, s_diffuse, duv).xyz * 2.0 - vec3<f32>(1.0);
        // (an OpenGL-style map, green up: `_gl` in its name)
        if (material.pbr.x > 1.5) {
            tn.y = -tn.y;
        }
        n = perturb_normal(n, in.world, duv, tn);
    }
    if (material.pbr.y + material.pbr.z + material.pbr.w > 0.5 && !terrain) {
        let orm = textureSample(t_pbr_orm, s_diffuse, duv).rgb;
        if (material.pbr.y > 0.5) {
            pbr_ao = orm.r;
        }
        if (material.pbr.z > 0.5) {
            rough = clamp(orm.g, 0.03, 1.0);
        }
        if (material.pbr.w > 0.5) {
            metal = orm.b;
            f0 = mix(vec3<f32>(0.04), albedo, metal);
        }
    }
    // rain: roads with [moisture] turn dark and smooth, everything outside a little glossier
    // (not under snow: a snowy road took the rain's gloss and the snow on it shone like
    // plastic in every headlight)
    let dry_snow = 1.0 - clamp(enh.weather.y, 0.0, 1.0);
    let wet_road = camera.shadow.w * material.params2.z * outside * dry_snow;
    let wet_any = camera.shadow.w * outside * select(0.35, 0.0, glass) * dry_snow;
    var puddle = 0.0;
    if (wet_road > 0.0) {
        albedo = albedo * mix(1.0, 0.5, wet_road);
        rough = mix(rough, 0.12, wet_road);
        f0 = mix(f0, vec3<f32>(0.02), wet_road);
        // Puddles: standing water pools in low, flat spots rather than spreading evenly
        // over the whole carriageway the way the sheen above does. A world-space mask at a
        // frequency of its own (unrelated to the road's detail noise, so the two do not
        // look tied together) picks out patches that grow with the wetness and vanish again
        // as the road dries; a puddle goes glass-flat and mirrors what is above it. While it
        // is actually raining rather than merely wet, and never while it is snow settling
        // instead (`enh.weather.z` is fed by any precipitation, `.y` singles snow back out),
        // concentric rings cross a puddle where drops land, fading as they widen - the other
        // half of the wheel splashes in `puddles.rs`.
        let pattern_xy = world_pattern_xy(in.world);
        // (the pools: see `road_puddle_coverage`, shared with the classic picture)
        puddle = road_puddle_coverage(in.world, n, wet_road);
        if (puddle > 0.001) {
            // A drop is a few millimetres across and its ring dies away within a hand's
            // breadth, so the shared ripple grid is 12.5 cm wide and a ring grows
            // to RIPPLE_MAX of a cell, about 5 cm: at 2 m cells with rings a metre and a half
            // across, every drop looked like a puddle of its own. Cells run at their own pace
            // and only some of them carry a drop at a time, so the surface reads as many small
            // impacts rather than one pulsing pattern.
            let raining = enh.weather.z * (1.0 - enh.weather.y);
            let ripple = puddle_ripple(pattern_xy, camera.post.y, raining, puddle);
            // Standing water hides most of the fine asphalt grain. Keep dry and damp
            // asphalt's detail, but let the reflected image read across a filled pool.
            albedo = albedo * (1.0 - 0.68 * puddle) / mix(1.0, detail_factor, puddle * 0.8);
            rough = clamp(mix(rough, 0.03, puddle) - ripple.z * 0.12, 0.02, 1.0);
            f0 = mix(f0, vec3<f32>(enh.debug.y), puddle);
            n = normalize(mix(n, geo_n, puddle) + vec3<f32>(ripple.xy, 0.0));
        }
    } else if (wet_any > 0.0 && !terrain) {
        rough = mix(rough, rough * 0.6, wet_any);
    }
    // snow lies on what faces up
    // (not on a shadow blob: whitened, it lit the snow under the bus instead of shading it)
    // (nor on a texture that is the season's snow picture, which shows the map's own snow
    // as OMSI 2 does: see the vanilla shader, #879)
    let snow = enh.weather.y * outside * select(1.0, 0.0, in.params2.w > 1.5 || material.ambient.w > 0.5);
    if (snow > 0.0) {
        let up = clamp(n.z, 0.0, 1.0);
        let ground = select(0.0, 1.0, terrain || material.params2.z > 0.0);
        let cover = snow * clamp(max(ground, smoothstep(0.78, 0.95, up) * 0.8), 0.0, 1.0) * (0.55 + 0.35 * tex.a);
        albedo = mix(albedo, vec3<f32>(0.82, 0.84, 0.88), cover);
        // fresh snow is all but matte: it scatters the light and shows no highlight
        rough = mix(rough, 0.95, cover);
        f0 = mix(f0, vec3<f32>(0.02), cover);
        metal = metal * (1.0 - cover);
    }
    // specular antialiasing: where the normal turns quickly across a pixel (a low-poly
    // bonnet close up, a curved body far away) the surface cannot reflect sharper than that
    // spread, or the interpolated normals show as bands in a mirror-like finish
    if (reflective_env && !glass) {
        let dndx = dpdx(n);
        let dndy = dpdy(n);
        let spread = min((dot(dndx, dndx) + dot(dndy, dndy)) * 2.0, 0.4);
        rough = sqrt(min(rough * rough + spread, 1.0));
    }
    var sf: Surface;
    sf.albedo = albedo * (1.0 - metal);
    sf.f0 = f0;
    sf.rough = rough;
    let nv = clamp(dot(n, v), 1e-4, 1.0);
    // --- the sun
    let s = camera.sun_dir.xyz;
    let nl = dot(n, s);
    var direct = vec3<f32>(0.0);
    if (enh.lights.w > 0.0 && max(enh.sun.r, enh.sun.g) > 1e-5) {
        var shadow = 0.0;
        if (glass && nl > 0.0) {
            // a pane lets nearly all of it through: the sun on it is its glint, for which one
            // compare says enough (the soft filter over a cab's windscreen, doors and side
            // windows was a millisecond of the frame)
            shadow = sun_shadow_hard(in.world, n);
        } else if (nl > 0.0 || thin) {
            shadow = sun_shadow_soft(in.world, n, thin);
        }
        let e_sun = enh.sun.rgb * shadow;
        if (thin) {
            // foliage: a crown of leaves facing every way, whose normals OMSI points up
            // only to light it evenly - lit by the sun from any side (the shadow map
            // darkens its far side), a little through the leaves as well
            direct = e_sun * (0.3 + 0.4 * max(nl, 0.0) + 0.1 * max(-nl, 0.0)) * sf.albedo / PI;
        } else if (nl > 0.0) {
            // the sun is a disc, not a point: no highlight sharper than it
            let a = max(rough * rough, 0.012);
            let h = normalize(s + v);
            let spec = d_ggx(max(dot(n, h), 0.0), a) * v_smith(nv, nl, a) * f_schlick(f0, dot(v, h));
            direct = e_sun * nl * (sf.albedo / PI * (vec3<f32>(1.0) - f_schlick(f0, nl)) + spec);
        }
    }
    // --- sky and ground
    var ao = 1.0;
    if (camera.clouds.w > 0.5 && !capture) {
        ao = ao_at(in.clip.xy, in.world);
    }
    // AO is generated from the opaque depth buffer.  A transparent bus pane therefore
    // samples the seats/dashboard behind it and makes their dark outlines crawl across the
    // glass as the camera moves.  The pane has its own transmission/reflection path; it must
    // not inherit occlusion belonging to geometry on the far side of the window.
    // (nor any other blended surface: a van's translucent door showed the shade of what
    // stood behind it)
    if (glass || mode > 1.5) {
        ao = 1.0;
    }
    ao = ao * pbr_ao;
    let fr = f_schlick(f0, nv);
    // inside the player's vehicle the light comes in through the windows and off the
    // cabin's own walls: less of it, and neither as blue nor as directional as the sky's
    let in_cab = 1.0 - outside;
    // The cab's light is one even ambient already (cab_e below); the screen-space occlusion
    // on top of it went black in the hollow under the windscreen - the steering wheel and
    // the dashboard stood dark beside a brightly lit cash desk. Inside the cab it is taken
    // at a third of its strength.
    ao = mix(ao, 1.0 - (1.0 - ao) * 0.35, in_cab);
    let avg_e = enh.fog_color.rgb * (PI / 0.9);
    let cab_e = vec3<f32>(dot(avg_e, vec3<f32>(0.2126, 0.7152, 0.0722))) * vec3<f32>(1.0, 0.98, 0.95);
    let e_amb = mix(sh_irradiance(n), cab_e * CAB_AMBIENT, 0.6 * in_cab) * ao_bounce(ao, sf.albedo);
    var ambient = e_amb * sf.albedo / PI * (vec3<f32>(1.0) - fr * (1.0 - rough));
    // --- reflections: the sky probe, as sharp as the surface is smooth
    var r = reflect(-v, n);
    // a reflection pointing into the surface would show the probe's ground: bend it up
    let below = dot(r, geo_n);
    if (below < 0.0) {
        r = normalize(r - geo_n * below * 1.02);
    }
    let lod = rough * (enh.lights.x - 1.0);
    var env = textureSampleLevel(t_probe, s_lin, to_cube(r), lod).rgb * enh.fog_color.w;
    // the probe holds only the sky: in a street the low sky is hidden behind houses and
    // trees, which reflect about as much light as a quarter-white wall (without this every
    // surface wore a milky sheen of the bright horizon, strongest against a low sun)
    let surround = enh.fog_color.rgb * (0.25 / 0.9);
    let open = smoothstep(-0.05, 0.35, r.z);
    env = mix(surround, env, mix(open, 1.0, select(0.25, 0.5, reflective_env)));
    // the cabin reflects the cabin, not the sky - and so does the bus's own glass seen
    // from the driver's seat
    let cab_view = max(in_cab, near_player_vehicle(in.world) * inside_vehicle(camera.cam_pos.xyz));
    // From inside the player bus the pane is near the cabin, but it must still show the
    // outside sky/street. Using the opaque-surface cabin weight here made every window a
    // flat pale patch and hid the useful reflection while driving.
    // A pane is the boundary between the cabin and the outside. Mixing the cabin
    // irradiance into its reflection made every windscreen look like a pale grey sheet
    // from the driver's seat; the interior is already behind the alpha-blended pane.
    let cabin_reflection = select(0.85, 0.0, glass);
    env = mix(env, cab_e * 0.35 / PI, cabin_reflection * cab_view);
    // ... but the inner face of the bus's own glass cannot mirror the sky, which lies
    // behind it: it mirrors the cab, far darker than the street seen through it. Taken as
    // the sky, the doors (seen at a grazing angle from the driver's seat) went a milky
    // grey sheet in fog and snow, when the probe is bright all round.
    let own_pane = select(0.0, near_player_vehicle(in.world) * inside_vehicle(camera.cam_pos.xyz), glass);
    if (reflective_env) {
        // OMSI's photographed environment gives the reflection its structure (trees,
        // houses, the street), the probe its light
        // Looked up by the reflection's direction in the world, not in the camera's frame
        // as Direct3D's sphere map does: taken from the camera, what a standing bus
        // mirrored changed whenever the view turned - one picture from one angle,
        // another from the next, trees sliding in from nowhere. Height gives the photo's
        // rows (sky, the horizon's trees and houses, the ground), the compass direction
        // a gentle drift across it that stays put in the world.
        let az = atan2(r.y, r.x);
        var env_uv = vec2<f32>(0.5 + 0.3 * sin(az), 0.5 + 0.45 * clamp(r.z, -1.0, 1.0));
        if (material.bump.y > 0.5) {
            env_uv = env_uv + bump_offset(duv);
        }
        // (with the mip level its footprint needs on top of the blur: a fixed level showed
        // the photo's trees as contour lines across a car's curved bonnet close up, and
        // paint takes it as blurred as the vanilla path does)
        let photo = textureSampleBias(t_env, s_diffuse, env_uv, max(rough * 16.0, select(2.0, 0.0, glass))).rgb;
        let photo_avg = textureSampleLevel(t_env, s_diffuse, vec2<f32>(0.5, 0.5), 12.0).rgb;
        let lum_avg = max(dot(photo_avg, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.02);
        // (a tint, not a picture: an unbounded ratio drew the photo's trees as bands)
        let ratio = clamp(photo / lum_avg, vec3<f32>(0.35), vec3<f32>(2.0));
        let band = 1.0 - smoothstep(0.25, 0.7, abs(r.z));
        // Only a smooth surface mirrors the photo; paint a quarter of its contrast.
        // A car's bonnet is a handful of triangles, and its interpolated normals turned the
        // photographed trees into contour lines across the paint at close range.
        let sharpness = 1.0 - smoothstep(0.05, 0.25, rough);
        // ... and only as far as the reflection is of the outside at all: the driver's own
        // windscreen reflects the cab, and the photo's trees drawn over that stood as
        // contour lines across everything seen through it (a car's bonnet close up)
        let outside_env = 1.0 - cab_view;
        // ... and not in a fog: there is nothing sharp out there to mirror. The photo's
        // trees stood as flat grey silhouettes on every pane in fog, in front of the real
        // (fogged) trees behind the glass - outlines of trees that are not there
        let clear_air = exp(-enh.fog.x * 150.0);
        env = env * mix(vec3<f32>(1.0), ratio, band * 0.65 * mix(0.25, 1.0, sharpness) * outside_env * clear_air * enh.debug.z);
    }
    // what the SSAO darkens, it also keeps reflections out of
    let spec_occ = clamp(pow(nv + ao, exp2(-16.0 * rough - 1.0)) - 1.0 + ao, 0.0, 1.0);
    // (a PBR set's roughness or metalness map says how it reflects: the probe, as for an envmap)
    let pbr_reflects = (material.pbr.z > 0.5 || material.pbr.w > 0.5) && !terrain;
    // (a wet road mirrors the sky probe as well; and what reflects nothing keeps the light
    // the Fresnel term took off its ambient above - at a grazing angle that term is near 1,
    // and the far road and ground went dark with no reflection in its place, #374)
    let reflects = reflective_env || glass || pbr_reflects || is_water || wet_road > 0.0;
    var reflection = select(vec3<f32>(0.0), env * env_brdf(f0, rough, nv) * spec_occ * select(1.0, wet_road, !(reflective_env || glass || pbr_reflects || is_water)), reflects);
    if (!reflects) {
        ambient = e_amb * sf.albedo / PI;
    }
    if (glass) {
        // Transparent bus panes need a readable outside reflection from the driver's
        // viewpoint; opaque paint must never receive this boost.
        reflection = reflection * 0.75 * (1.0 - 0.85 * own_pane);
    }
    // --- the lamps, the cabin light and what glows by itself
    // ([nomaplighting] objects are not lit by the map's lamps; light-mapped roads are, with
    // the tile light map on top)
    let lamps = lamp_light(in.world, n, v, sf, thin) * select(1.0, 0.0, material.params.y > 0.2 && material.params.y < 0.3);
    // [interiorlight]: OMSI adds its lamps' light to the lit meshes whatever the daylight,
    // so a switched-on saloon is brighter by day as well and only stands out more at night.
    // Taken as a lamp against the daylight exposure it vanished by day altogether.
    // (in the picture's own terms, as the vanilla pass adds it: scaled with the metering it
    // grew with the night and whitened the saloon)
    // (the lamps' light does not reach into the gaps under the seats and round the
    // handrails either: without the ambient occlusion on it they glowed through there)
    let cabin_light = interior_lamps(in.world, n, in.params2.z);
    let cabin = sf.albedo * cabin_light * mix(1.0, ao, 0.85);
    var rgb = (direct + ambient + lamps) * pre + cabin;
    var emit = tex.rgb * material.emissive.rgb * max(enh.exposure.z * 2.0, 0.8);
    // (the tile light map on the splines and [LightMapMapping] objects is the vanilla
    // path's: here the map's lamps light them, tinted from that map, as they light every
    // other surface - added on top it lit the roads twice, with a hard edge where a road
    // met a square that is an object)
    if (material.extra.w > 0.5) {
        let nuv = select(buv, vec2<f32>(in.uv.x, 1.0 - in.uv.y), terrain);
        let switched = material.extra.w > 1.5;
        let night = select(camera.sun_color.w, 1.0, switched);
        // (a switched one is the display's own state: not dimmed with the instance's night
        // lighting, which is 0 by day and left the Procity's pressure screen black)
        let nm = sample_nightmap(nuv).rgb * night * select(clamp(in.params2.y, 0.0, 1.0), 1.0, switched);
        if (terrain) {
            // the tile's light map: the lamps' light on the ground
            rgb = rgb + sf.albedo / PI * nm * enh.lights.y * 3.0 * pre;
        } else {
            // lit windows and signs; a switched lamp or display holds up against daylight
            // as the self-illuminated materials do
            emit = emit + nm * select(enh.exposure.z, max(enh.exposure.z * 2.0, 0.8), switched);
        }
    }
    if (material.params2.x > 0.5 && !terrain) {
        // [matl_lightmap]: laid onto the light with ADDSMOOTH in Omsi.exe (see the vanilla
        // shader), so it adds what the surface's own light leaves: little by day, fully at
        // night, and less where the saloon lamps light the surface already. (As an emission
        // at 0.6 of the texture by day and on top of the lamps at night, a switched-on
        // cabin's light-mapped parts were flat white.)
        let lm = textureSample(t_light, s_diffuse, buv).rgb;
        let night = clamp(camera.sun_color.w, 0.0, 1.0);
        let left = (vec3<f32>(1.0) - clamp(cabin_light, vec3<f32>(0.0), vec3<f32>(1.0))) * (0.12 + 0.88 * night);
        let w = lm * left * clamp(in.params2.x, 0.0, 1.0);
        if (material.emissive.w < -1.5) {
            emit = emit + tex.rgb * w * max(enh.exposure.z * 2.0, 0.6);
        } else {
            // ... and as there it lifts the surface to its texture's own colour at the
            // most, shown as a display is (see the unlit path): taken as a light of the
            // night's level on top, a white light map (`weiss.bmp`, laid on a display so
            // it shows at night) went past the tone curve's knee and bleached its colours -
            // the Procity's red and blue gauges pink and lavender (#827). (An LED panel's
            // light map stays as it was: its dots are meant to burn above their colour.)
            emit = emit + max(display_level(tex.rgb) * enh.exposure.y - rgb, vec3<f32>(0.0)) * w;
        }
    }
    if (material.emissive.w < -1.5) {
        // an LED panel (see MaterialExtra::led): the lit dots - the alpha the `\S:n` script
        // texture carries, in the colour of the panel's own texture - are the panel's own
        // light, drawn as bright as the settings ask for (`Led glow`, 16 levels, 0 = off).
        // The glow takes them where it leaves every other screen out of its source
        // (`post.wgsl`) and blooms a halo around the panel. (Kept at their own brightness
        // however the metering treats the scene, as a display's text is.) Its light is its
        // white light map's, so it goes out with that map's variable (the busbar, the
        // lights) as the Omsi.exe stage does.
        let lm_gate = select(1.0, clamp(in.params2.x, 0.0, 1.0), material.params2.x > 0.5);
        emit = emit + tex.rgb * enh.led.x * alpha * lm_gate * max(enh.exposure.z * 2.0, 0.8);
    } else if (material.emissive.w < -0.5) {
        // a display's text (see MaterialExtra::display)
        emit = emit + tex.rgb * 0.35 * max(enh.exposure.z * 2.0, 0.8);
    }
    rgb = rgb + emit;
    if (enh.debug.x > 0.5) {
        // OMSI_DEBUG_ENHANCED: one term alone, as it lands on the screen
        let dm = i32(enh.debug.x);
        var dc = vec3<f32>(0.0);
        switch dm {
            case 1: { dc = vec3<f32>(sun_shadow_soft(in.world, n, thin)); }
            case 2: { dc = vec3<f32>(ao); }
            case 3: { dc = n * 0.5 + vec3<f32>(0.5); }
            case 4: { dc = vec3<f32>(aer.a); }
            case 5: { dc = e_amb * pre * 0.1; }
            case 6: { dc = reflection * pre; }
            case 7: { dc = sf.albedo; }
            case 8: { dc = direct * pre; }
            case 9: { dc = aer.rgb * pre; }
            case 11: { dc = vec3<f32>(fract(dist / 10.0), dist / 1000.0, in.clip.z * 100.0); }
            case 12: { dc = vec3<f32>(mode * 0.5, f32(terrain), in.params2.w); }
            case 13: { dc = vec3<f32>(in_cab, ao, spec_occ); }
            case 14: { dc = (ambient + direct) * pre; }
            case 15: { dc = lamps * pre; }
            case 16: { dc = emit; }
            case 17: { dc = vec3<f32>(rough, f0.g * 10.0, metal); }
            default: { dc = vec3<f32>(alpha, f32(glass), f32(has_env)); }
        }
        return vec4<f32>(dc, 1.0);
    }
    if (glass) {
        // see-through glass: the reflection is added on top of what shows through, so
        // the blend keeps it where the glass itself is faint - but not where the texture
        // is not there at all: Omsi.exe leaves the alpha as the texture has it, so a
        // see-through part of a blended layer (the clear ground of a sticker on the
        // cab's wall, #861) shows no reflection; made up to a quarter opaque by it, the
        // whole rectangle of the sticker mirrored the sky
        let cover = smoothstep(0.0, 0.05, alpha);
        let refl_rgb = reflection * pre * cover;
        let rl = dot(refl_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        // (a pane passes most light: a reflection making it up to 60 % opaque laid a grey
        // veil over the destination display behind the windscreen and the saloon)
        let a2 = clamp(alpha + (1.0 - alpha) * clamp(rl * 0.25 + fr.g, 0.0, 0.25) * (1.0 - 0.85 * own_pane) * cover, alpha, 1.0);
        let c = (rgb * alpha + refl_rgb) / max(a2, 1e-3);
        return vec4<f32>(c * aer.a + aer.rgb * pre, a2);
    }
    rgb = rgb + reflection * pre;
    // The later screen-space pass replaces only this fraction of the sky reflection.
    // A miss adds zero, so the existing sky, Fresnel and fog stay the fallback. Colour
    // and coverage are resolved together, including terrain painting and glass over roads.
    if (!glass && !reflective_env) {
        let weight = clamp(puddle * env_brdf(f0, rough, nv).g
            * select(wet_road, 1.0, pbr_reflects) * aer.a, 0.0, 1.0);
        *puddle_weight = vec2<f32>(weight, weight * spec_occ);
    }
    // water hides what lies under it as far as it reflects (Fresnel): see-through from
    // above, a mirror of the sky towards the horizon
    let a_out = select(alpha, clamp(alpha + (1.0 - alpha) * fr.g, alpha, 1.0), is_water);
    return vec4<f32>(rgb * aer.a + aer.rgb * pre, a_out);
}

// Small waves on the water at map point `p` (m) and time `t` (s): the slope of four wave
// trains of 1.3 to 4.7 m running different ways.
fn water_ripple(p: vec2<f32>, t: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    let dirs = array<vec2<f32>, 4>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.45, 0.89), vec2<f32>(0.96, -0.28), vec2<f32>(-0.7, -0.71));
    let lens = array<f32, 4>(4.7, 2.9, 1.9, 1.3);
    for (var i = 0; i < 4; i = i + 1) {
        let k = 6.2831853 / lens[i];
        // deep-water waves: their speed goes with the square root of their length
        let w = sqrt(9.81 * k);
        g = g + dirs[i] * cos(dot(p, dirs[i]) * k - w * t) * 0.045;
    }
    return g;
}

// Shade the complete local vehicle from the reflected eye. Main-camera ambient
// occlusion is not applicable to this view beneath the road.
@fragment
fn fs_puddle_vehicle(input: FsIn) -> @location(0) vec4<f32> {
    // One reflected camera for the complete vehicle, including its transparent panes.
    let plane = vehicle_reflection.plane;
    let height = dot(plane.xyz, input.world) - plane.w;
    if (height < 0.0 || material.emissive.w > 1.5) { discard; }
    var unused = vec2<f32>(0.0);
    let eye = camera.cam_pos.xyz - 2.0 * plane.xyz * (dot(plane.xyz, camera.cam_pos.xyz) - plane.w);
    if (camera.post.x < 0.5) {
        var unused_vanilla = 0.0;
        return shade_vanilla(input, &unused_vanilla, eye);
    }
    return shade_enhanced(input, &unused, true, eye);
}

@fragment
fn fs_puddle_chassis() -> @location(0) vec4<f32> {
    if (camera.post.x < 0.5) { return vec4<f32>(camera.ambient.rgb * 0.025, 1.0); }
    return vec4<f32>(sh_irradiance(vec3<f32>(0.0, 0.0, -1.0)) * enh.exposure.x * 0.025, 1.0);
}
