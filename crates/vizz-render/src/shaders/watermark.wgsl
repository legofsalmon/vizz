// The licence mark: a dark capsule with light lettering, alpha-blended
// over the finished master. Positioned by the caller's viewport, so the
// triangle below only ever covers the mark's own rectangle.
//
// The mask is the lettering's coverage, rasterised on the CPU once at
// exactly the size it is drawn, with the capsule's padding built in — so
// the capsule is the whole viewport and one texel is one output pixel.

@group(0) @binding(0) var mask: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// How dark the bed is and how bright the letters are. The bed is what
// keeps the letters readable over a white frame; the letters are what
// keep the mark visible over a black one.
const BED_ALPHA: f32 = 0.55;
const INK_ALPHA: f32 = 0.92;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(mask));
    let p = in.uv * size;
    // Capsule: a rounded rectangle whose radius is half its height.
    let half_size = size * 0.5;
    let radius = half_size.y;
    let q = abs(p - half_size) - (half_size - vec2<f32>(radius));
    let dist = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    // One pixel of antialiasing on the edge, in output pixels because a
    // mask texel is one.
    let inside = clamp(0.5 - dist, 0.0, 1.0);
    let ink = textureSampleLevel(mask, samp, in.uv, 0.0).r;
    let color = vec3<f32>(ink);
    let alpha = inside * mix(BED_ALPHA, INK_ALPHA, ink);
    return vec4<f32>(color, alpha);
}
