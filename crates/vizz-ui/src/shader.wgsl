// egui paint shader.
//
// egui composites in *gamma* (sRGB) space: vertex colours and atlas texels
// are both premultiplied sRGB, and they must be multiplied together in
// that space. Only the final result is converted to linear, and only when
// the target is an sRGB format (where the hardware re-encodes on write).
//
// Getting this wrong is silent — it renders, just too dark. Atlases are
// therefore uploaded as Rgba8Unorm so sampling does NOT linearise them.

struct Uniforms {
    screen_size_points: vec2<f32>,
    // 1 when the render target is an sRGB format.
    srgb_target: u32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var t_atlas: texture_2d<f32>;
@group(1) @binding(1) var s_atlas: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

// 0-1 linear from 0-1 sRGB gamma.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    var out: VsOut;
    // Points -> clip space, Y flipped (egui's origin is top-left).
    out.pos = vec4<f32>(
        2.0 * pos.x / u.screen_size_points.x - 1.0,
        1.0 - 2.0 * pos.y / u.screen_size_points.y,
        0.0,
        1.0,
    );
    out.uv = uv;
    // Left in gamma space on purpose - see the note at the top.
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tex_gamma = textureSample(t_atlas, s_atlas, in.uv);
    let out_gamma = in.color * tex_gamma;
    if u.srgb_target == 1u {
        return vec4<f32>(linear_from_gamma_rgb(out_gamma.rgb), out_gamma.a);
    }
    return out_gamma;
}
