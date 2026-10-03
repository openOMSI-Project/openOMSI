// Light coronas ([light_enh]): camera-facing additive sprites.
struct Camera {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    // This remains part of the shared camera layout even though coronas only use
    // `view_proj` and `cam_pos` today.  Keeping the prefix aligned prevents later
    // corona effects from reading every lighting field one vec4 too early.
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
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var t_corona: texture_2d<f32>;
@group(1) @binding(1) var s_corona: sampler;

struct CoronaIn {
    @location(0) pos: vec3<f32>,      // relative to the render origin
    @location(1) size: f32,           // world radius
    @location(2) color: vec4<f32>,    // rgb, alpha = brightness (0..2)
    @location(3) dir: vec4<f32>,      // xyz facing direction (0 = omni), w cos of the outer cone
    @location(4) up: vec4<f32>,       // xyz up / rotation axis, w rotating mode (0 flat, 1 about up, 2 billboard)
    @location(5) extra: vec4<f32>,    // x cos of the inner cone, y z offset (<0 default), z parameter bits
    @builtin(vertex_index) vid: u32,
};
struct CoronaOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // 1 for what is drawn at the scene's level (a precipitation streak, lit fog: a cone or
    // a halo), 0 for a light's glare
    @location(2) kind: f32,
    // 1 when the light is drawn with a star (`[light_enh_2]` parameter bit 1)
    @location(3) star: f32,
    // 1 for a light's cone in the fog (OMSI's fan, see below)
    @location(4) beam: f32,
    // the cone's inner and outer half angles (radians)
    @location(5) cone: vec2<f32>,
};

@vertex
fn vs_main(in: CoronaIn) -> CoronaOut {
    var out: CoronaOut;
    let corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[in.vid % 6u];
    if (in.extra.w > 1.5) {
        // the halo round a light in fog: licht.bmp turned to the
        // viewer, pulled towards them by its size, seen from in front of the light (for a
        // directional one the more, the further out of its cone's side the viewer stands)
        let to_cam = camera.cam_pos.xyz - in.pos;
        let dist = length(to_cam);
        let vdir = to_cam / max(dist, 0.001);
        var k = in.color.a;
        if (length(in.dir.xyz) > 0.5) {
            let axis = normalize(in.dir.xyz);
            let inner = in.extra.x;
            let outer = in.dir.w;
            let ang = acos(clamp(dot(axis, vdir), -1.0, 1.0));
            var from_side = select(0.0, 1.0, ang > inner);
            if (outer > inner + 0.0001) {
                from_side = clamp((ang - inner) / (outer - inner), 0.0, 1.0);
            }
            let side = min(from_side, clamp(3.0 - ang, 0.0, 1.0));
            let front = (1.0 - side) * clamp(1.0 - (ang - 2.0) * 0.5, 0.5, 1.0);
            k = k * front * front * select(0.0, 1.0, ang < 1.5705);
        }
        let fwd = normalize(cross(camera.cam_up.xyz, camera.cam_right.xyz));
        let on_screen = 2.0 * dot(fwd, -vdir) - 1.0;
        let far = clamp(1.0 - dist / max(in.extra.y, 1.0), 0.0, 1.0);
        k = k * max(on_screen, 0.0) * far;
        let right = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), vdir));
        let up = cross(vdir, right);
        var hout: CoronaOut;
        let centre = in.pos + vdir * min(in.size, dist * 0.9);
        hout.clip = camera.view_proj * vec4<f32>(centre + (right * c.x + up * c.y) * in.size, 1.0);
        hout.uv = c * 0.5 + 0.5;
        hout.color = vec4<f32>(in.color.rgb, k);
        hout.kind = 1.0;
        hout.star = 0.0;
        hout.beam = 0.0;
        hout.cone = vec2<f32>(0.0, 0.0);
        if (k <= 0.001 || on_screen <= 0.01) {
            hout.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
        return hout;
    }
    if (in.extra.w > 0.5) {
        // OMSI's cone in the fog: a flat
        // fan from the lamp, `size` in radius, spreading to the outer half angle either side
        // of the light's axis and turned about that axis to face the eye.
        // It is drawn here as a square in that plane; the fragment cuts the fan out of it
        // and maps light_cone.bmp onto it as the fan's vertices do.
        let axis = normalize(in.dir.xyz);
        let inner = in.extra.x;
        let outer = in.dir.w;
        let to_cam = camera.cam_pos.xyz - in.pos;
        let dist = length(to_cam);
        let vdir = to_cam / max(dist, 0.001);
        var side = cross(vdir, axis);
        if (length(side) < 0.001) {
            side = cross(vec3<f32>(0.0, 0.0, 1.0), axis);
        }
        side = normalize(side);
        // seen from the side it shows, from within its inner cone the glow does instead,
        // and from behind (2..3 rad off its axis) it fades out
        let ang = acos(clamp(dot(axis, vdir), -1.0, 1.0));
        var from_side = select(0.0, 1.0, ang > inner);
        if (outer > inner + 0.0001) {
            from_side = clamp((ang - inner) / (outer - inner), 0.0, 1.0);
        }
        let behind = clamp(3.0 - ang, 0.0, 1.0);
        // towards the edge of the view it fades (2 cos - 1 of the angle off the view's axis)
        let fwd = normalize(cross(camera.cam_up.xyz, camera.cam_right.xyz));
        let on_screen = 2.0 * dot(fwd, -vdir) - 1.0;
        // and it goes with the lamp's distance through the fog
        let far = clamp(1.0 - dist / max(in.extra.y, 1.0), 0.0, 1.0);
        let k = in.color.a * min(from_side, behind) * max(on_screen, 0.0) * far;
        var bout: CoronaOut;
        bout.clip = camera.view_proj * vec4<f32>(in.pos + (axis * c.x + side * c.y) * in.size, 1.0);
        bout.uv = c;
        bout.color = vec4<f32>(in.color.rgb, k);
        bout.kind = 1.0;
        bout.star = 0.0;
        bout.beam = 1.0;
        bout.cone = vec2<f32>(inner, outer);
        if (k <= 0.001 || on_screen <= 0.01) {
            bout.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
        return bout;
    }
    let to_cam = camera.cam_pos.xyz - in.pos;
    let dist = length(to_cam);
    let view_dir = to_cam / max(dist, 0.001);
    let streak = in.dir.w < -1.5;
    let directed = length(in.dir.xyz) > 0.5 && !streak;
    var brightness = in.color.a;
    if (directed) {
        // a directional light is seen inside its outer cone, in full inside the inner one,
        // linearly in the angle between them
        let ang = acos(clamp(dot(normalize(in.dir.xyz), view_dir), -1.0, 1.0));
        let outer = acos(clamp(in.dir.w, -1.0, 1.0));
        let inner = select(0.0, acos(clamp(in.extra.x, -1.0, 1.0)), in.extra.x >= -1.0);
        brightness = brightness * clamp((outer - ang) / max(outer - inner, 0.0001), 0.0, 1.0);
    }
    // a star grows with the light's strength; every sprite's strength stops at 1
    let star_sprite = (u32(in.extra.z + 0.5) & 8u) != 0u && !streak;
    let grow = select(1.0, brightness, star_sprite);
    if (!streak) {
        brightness = min(brightness, 1.0);
    }
    // the sprite's axes: flat and facing its direction (rotating 0), turned to the viewer
    // about its up axis (1), or turned to the viewer entirely (2)
    let mode = u32(in.up.w + 0.5);
    var right = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), view_dir));
    var up = cross(view_dir, right);
    let axis = select(vec3<f32>(0.0, 0.0, 1.0), normalize(in.up.xyz), length(in.up.xyz) > 0.5);
    if (!streak && mode == 0u && directed) {
        let n = normalize(in.dir.xyz);
        var u = axis - n * dot(axis, n);
        if (length(u) < 0.01) {
            u = vec3<f32>(0.0, 0.0, 1.0) - n * n.z;
        }
        up = normalize(u);
        right = cross(up, n);
    } else if (!streak && mode == 1u) {
        let r = cross(axis, view_dir);
        if (length(r) > 0.01) {
            right = normalize(r);
            up = axis;
        }
    }
    // coronas keep a minimum on-screen size in the distance like the original;
    // precipitation particles (cone < -1.5) follow their motion relative to the eye.
    // A sprite's radius, from the light's size (its diameter) and that distance floor. The
    // 0.9 is measured against Omsi.exe: at the size the game files ask for, every glow reads
    // a shade too wide beside the original, which draws the sprite a little inside the
    // diameter its `size` names. (Streaks keep their size: they are rain, not a light.)
    let size = select(max(in.size * grow, dist * 0.002) * 0.9, in.size, streak);
    let stretch = select(vec2<f32>(1.0, 1.0), vec2<f32>(0.06, 4.0), streak);
    var upv = up;
    if (streak) {
        upv = vec3<f32>(0.0, 0.0, 1.0);
        if (length(in.dir.xyz) > 0.5) { upv = normalize(in.dir.xyz); }
        let across = cross(upv, view_dir);
        if (length(across) > 0.001) { right = normalize(across); }
    }
    // the spot moved towards the viewer by its z offset, so that a lamp inside its housing
    // shows (without one: half its size, at most half a metre)
    let pull = select(min(size * 0.5, 0.5), in.extra.y, in.extra.y >= 0.0);
    let wp = in.pos + (right * c.x * stretch.x + upv * c.y * stretch.y) * size + select(view_dir * min(pull, dist * 0.9), vec3<f32>(0.0), streak);
    out.clip = camera.view_proj * vec4<f32>(wp, 1.0);
    out.uv = c * 0.5 + 0.5;
    out.color = vec4<f32>(in.color.rgb, brightness);
    out.kind = select(0.0, 1.0, streak);
    out.star = 0.0;
    out.beam = 0.0;
    out.cone = vec2<f32>(0.0, 0.0);
    // (not `<= 0.001`: a NaN brightness is not smaller, and Direct3D drew it as a black
    // square where Metal's fast maths had dropped it)
    if (!(brightness > 0.001) || (bitcast<u32>(brightness) & 0x7f800000u) == 0x7f800000u) {
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

// The sprite's light: the round texture (in its own colours), and for a star light four thin rays across it.
fn corona_shape(in: CoronaOut) -> vec3<f32> {
    // (the sprite keeps the bitmap's own colours, modulated by the light's colour)
    // Read the right way up. `in.uv` is built the way the HUD's is (overlay.wgsl), where
    // v = 0 is the picture's *first*, topmost row; but a sprite's corners are placed by its
    // own `up`, which runs up the screen, so v = 1 comes to rest at its top. Taken as it
    // stands the picture is drawn upside down - invisible on a round glow or a star, plain
    // on an asymmetric effect, such as the light streak some buses' `[light_enh_2]`
    // `lights_abbl`/`lights_stand` bitmaps draw. The fan below keeps the unflipped `in.uv`:
    // it maps light_cone.bmp from its own `fan_uv` and only takes `r` and `phi` from here,
    // which its geometry needs as they stand.
    let t = textureSample(t_corona, s_corona, vec2<f32>(in.uv.x, 1.0 - in.uv.y)).rgb;
    // the cone's fan: its apex has uv (0, 1) and a rim vertex (sin a, 1 - cos a)
    // (u = sin a, v = 1 - cos a), a being 0.05 inside
    // the inner cone and rising to 0.9 pi/2 + 0.05 at the outer edge: light_cone.bmp's
    // bright left column runs down the light's axis and fades towards the fan's edges.
    // (With sin and cos the other way round the streaks ran along both edges, a "V" of
    // light reaching out sideways and up from every lamp.)
    let r = length(in.uv);
    let phi = abs(atan2(in.uv.y, in.uv.x));
    let span = max(in.cone.y - in.cone.x, 0.0001);
    let a = 0.05 + 0.9 * 1.5708 * clamp((phi - in.cone.x) / span, 0.0, 1.0);
    let fan_uv = clamp(vec2<f32>(0.0, 1.0) + min(r, 1.0) * vec2<f32>(sin(a), -cos(a)), vec2<f32>(0.0), vec2<f32>(1.0));
    let tb = textureSample(t_corona, s_corona, fan_uv).r;
    if (in.beam > 0.5) {
        let inside = select(0.0, 1.0, r <= 1.0 && phi <= in.cone.y);
        return vec3<f32>(tb * inside);
    }
    return t;
}

@fragment
fn fs_main(in: CoronaOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * corona_shape(in) * in.color.a, 1.0);
}

// Enhanced graphics: the corona in the pre-exposed high-range picture - the glare of a
// lamp is drawn relative to what the eye is adapted to, brighter than white so that the
// glow filter picks it up; precipitation streaks keep the vanilla level.
@fragment
fn fs_enhanced(in: CoronaOut) -> @location(0) vec4<f32> {
    // (a fog cone is lit fog, not glare: it keeps the scene's level too)
    let scale = select(enh.exposure.w, enh.exposure.y, in.kind > 0.5);
    return vec4<f32>(in.color.rgb * corona_shape(in) * in.color.a * scale, 1.0);
}

// Smoke ([smoke] particles): the smoke texture tinted with the particle's colour, lit by the
// scene's ambient and sun light, blended with its alpha (the particle's times the texture's).
fn smoke_color(in: CoronaOut) -> vec4<f32> {
    // upside up, as the sprite's own picture above
    let t = textureSample(t_corona, s_corona, vec2<f32>(in.uv.x, 1.0 - in.uv.y));
    let light = min(camera.ambient.rgb + camera.sun_color.rgb * 0.6, vec3<f32>(1.2));
    return vec4<f32>(in.color.rgb * t.rgb * light, clamp(t.a * in.color.a, 0.0, 1.0));
}

@fragment
fn fs_smoke(in: CoronaOut) -> @location(0) vec4<f32> {
    return smoke_color(in);
}

@fragment
fn fs_smoke_enhanced(in: CoronaOut) -> @location(0) vec4<f32> {
    let c = smoke_color(in);
    return vec4<f32>(c.rgb * enh.exposure.y, c.a);
}
