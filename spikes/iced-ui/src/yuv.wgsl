// Draws a YUV 4:2:0 video frame letterboxed into the widget bounds.

struct Uniforms {
    // Fraction of the bounds the frame covers on each axis (aspect fit).
    scale: vec2<f32>,
    // 1.0 when the target is an sRGB surface (we must output linear light).
    srgb_target: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex_y: texture_2d<f32>;
@group(0) @binding(2) var tex_u: texture_2d<f32>;
@group(0) @binding(3) var tex_v: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    var out: VsOut;
    out.pos = vec4<f32>((corner * 2.0 - 1.0) * vec2<f32>(u.scale.x, -u.scale.y), 0.0, 1.0);
    out.uv = corner;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // BT.709, limited range.
    let y = (textureSample(tex_y, samp, in.uv).r - 16.0 / 255.0) * 1.164383;
    let cb = textureSample(tex_u, samp, in.uv).r - 0.5;
    let cr = textureSample(tex_v, samp, in.uv).r - 0.5;
    var rgb = clamp(vec3<f32>(
        y + 1.792741 * cr,
        y - 0.213249 * cb - 0.532909 * cr,
        y + 2.112402 * cb,
    ), vec3<f32>(0.0), vec3<f32>(1.0));
    if (u.srgb_target > 0.5) {
        rgb = to_linear(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
