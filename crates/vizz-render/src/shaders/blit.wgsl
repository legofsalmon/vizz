// Fullscreen-triangle blit: samples the master texture onto the preview
// swapchain. Aspect fitting is done by the caller via the viewport.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // 3 vertices covering the whole viewport: (0,0) (2,0) (0,2) in uv.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

/// Most bilinear taps per axis. Each covers two texels, so this filters
/// a reduction of up to eight times properly — a 4K master in a preview
/// panel under 500 pixels wide — and degrades gracefully past it.
const MAX_TAPS: i32 = 4;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // How many source texels one destination pixel covers, per axis.
    //
    // A single bilinear tap only averages the 2×2 texels nearest the pixel
    // centre, so shrinking the master by more than two drops the rest: a
    // field of one-pixel sprites turns into sparkle that is not in the
    // show. Averaging over the whole footprint is what a mip chain would
    // do, without regenerating one every frame for a preview.
    let size = vec2<f32>(textureDimensions(src));
    let footprint = vec2<f32>(
        max(abs(dpdx(in.uv.x)), abs(dpdy(in.uv.x))),
        max(abs(dpdx(in.uv.y)), abs(dpdy(in.uv.y))),
    ) * size;
    // One tap at 2× or less, which is what a 1:1 publish blit always is,
    // so that path stays a plain copy.
    let taps = clamp(vec2<i32>(ceil(footprint * 0.5)), vec2<i32>(1), vec2<i32>(MAX_TAPS));
    if (taps.x == 1 && taps.y == 1) {
        return textureSampleLevel(src, samp, in.uv, 0.0);
    }
    let step = footprint / (size * vec2<f32>(taps));
    let origin = in.uv - 0.5 * step * vec2<f32>(taps - vec2<i32>(1));
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < taps.y; j = j + 1) {
        for (var i = 0; i < taps.x; i = i + 1) {
            sum += textureSampleLevel(src, samp, origin + step * vec2<f32>(f32(i), f32(j)), 0.0);
        }
    }
    return sum / f32(taps.x * taps.y);
}
