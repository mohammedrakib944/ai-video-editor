// One layer of the canvas: a rotated, rounded rectangle showing a texture (or a
// flat/gradient fill), with optional border, drop shadow and color grade.
// Colors are blended premultiplied in linear light.

struct Layer {
    rect: vec4<f32>,         // center x, center y, half width, half height (canvas px)
    params: vec4<f32>,       // rotation (rad), opacity, corner radius, border width
    shadow: vec4<f32>,       // blur, offset y, quad padding, antialias width (canvas px per screen px)
    uv: vec4<f32>,           // u0, v0, u1, v1
    tint_top: vec4<f32>,     // sRGB
    tint_bottom: vec4<f32>,  // sRGB
    border_color: vec4<f32>, // sRGB
    shadow_color: vec4<f32>, // sRGB rgb + strength
    grade: vec4<f32>,        // brightness, contrast, saturation, temperature
    canvas: vec4<f32>,       // width, height, target is sRGB (1/0), has texture (1/0)
};

@group(0) @binding(0) var<uniform> layer: Layer;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u)) * 2.0 - 1.0;
    let extent = layer.rect.zw + vec2<f32>(layer.shadow.z);
    let local = corner * extent;
    let c = cos(layer.params.x);
    let s = sin(layer.params.x);
    let p = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c) + layer.rect.xy;
    var out: VsOut;
    out.pos = vec4<f32>(p.x / layer.canvas.x * 2.0 - 1.0, 1.0 - p.y / layer.canvas.y * 2.0, 0.0, 1.0);
    out.local = local;
    return out;
}

fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn apply_grade(rgb_in: vec3<f32>, g: vec4<f32>) -> vec3<f32> {
    var rgb = rgb_in;
    if (g.x != 0.0 || g.y != 0.0 || g.z != 0.0 || g.w != 0.0) {
        // Grade in display space, where these controls feel natural.
        var c = to_srgb(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
        c = c + vec3<f32>(g.x * 0.5);
        c = (c - vec3<f32>(0.5)) * (1.0 + g.y) + vec3<f32>(0.5);
        let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        c = mix(vec3<f32>(luma), c, 1.0 + g.z);
        c = c * vec3<f32>(1.0 + g.w * 0.12, 1.0 + g.w * 0.02, 1.0 - g.w * 0.12);
        rgb = to_linear(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return rgb;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half = layer.rect.zw;
    let radius = layer.params.z;
    let border = layer.params.w;
    let opacity = layer.params.y;
    let aa = max(layer.shadow.w, 0.0001);
    let d = sd_round_box(in.local, half, radius);
    let cover = clamp(0.5 - d / aa, 0.0, 1.0);

    var texel = vec4<f32>(1.0);
    if (layer.canvas.w > 0.5) {
        let f = in.local / max(half, vec2<f32>(0.0001)) * 0.5 + 0.5;
        texel = textureSample(tex, samp, mix(layer.uv.xy, layer.uv.zw, f));
    }
    let g = clamp(in.local.y / max(half.y, 0.0001) * 0.5 + 0.5, 0.0, 1.0);
    let tint = mix(layer.tint_top, layer.tint_bottom, g);
    var rgb = apply_grade(texel.rgb * to_linear(tint.rgb), layer.grade);
    var a = texel.a * tint.a;

    if (border > 0.0) {
        let inside = clamp(0.5 - (d + border) / aa, 0.0, 1.0);
        rgb = mix(to_linear(layer.border_color.rgb), rgb, inside);
        a = mix(layer.border_color.a, a, inside);
    }

    let alpha = a * cover * opacity;
    var out = vec4<f32>(rgb * alpha, alpha);

    let strength = layer.shadow_color.a;
    if (strength > 0.0) {
        let blur = max(layer.shadow.x, 1.0);
        let ds = sd_round_box(in.local - vec2<f32>(0.0, layer.shadow.y), half, radius);
        let sa = strength * opacity * (1.0 - smoothstep(-blur, blur, ds));
        out = out + vec4<f32>(to_linear(layer.shadow_color.rgb) * sa, sa) * (1.0 - alpha);
    }

    if (layer.canvas.z < 0.5 && out.a > 0.0) {
        out = vec4<f32>(to_srgb(out.rgb / out.a) * out.a, out.a);
    }
    return out;
}
