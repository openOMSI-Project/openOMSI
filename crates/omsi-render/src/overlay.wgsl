// Screen-space textured quad (HUD overlay), premultiplied-alpha blend.
struct Rect { rect: vec4<f32>, opts: vec4<f32>, };   // x0, y0, x1, y1 in NDC; opts.x: premultiplied, opts.y: on its side
@group(0) @binding(0) var<uniform> r: Rect;
@group(0) @binding(1) var t_img: texture_2d<f32>;
@group(0) @binding(2) var s_img: sampler;
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32>, };
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let c = corners[vid];
    var out: VsOut;
    out.clip = vec4<f32>(mix(r.rect.x, r.rect.z, c.x), mix(r.rect.y, r.rect.w, c.y), 0.0, 1.0);
    out.uv = select(vec2<f32>(c.x, c.y), vec2<f32>(c.y, c.x), r.opts.y > 0.5);
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_img, s_img, in.uv);
    if (r.opts.x > 0.5) {
        return t;
    }
    return vec4<f32>(t.rgb * t.a, t.a);
}
