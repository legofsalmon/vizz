// Bloom for the graded path, run on the post chain's HDR history after
// feedback and only while /fx/grade is up. See grade.rs.
//
// A mip chain rather than a few wide taps: downsample the frame five or
// six times, then walk back up adding each level into the one above. Every
// level is a wider blur of the same light, so the sum falls off smoothly
// from a sharp core to a wide haze, the way scattering in a lens does. The
// six-tap glow cannot do that; it shows as offset copies around anything
// bright. The filters are the ones from Jimenez, "Next Generation Post
// Processing in Call of Duty: Advanced Warfare" (SIGGRAPH 2014): a 13-tap
// downsample, Karis-averaged on the first step so a single hot sprite
// cannot flicker the whole bloom, and a 3×3 tent on the way up.

@group(0) @binding(0) var bloom_src: texture_2d<f32>;
@group(0) @binding(1) var bloom_samp: sampler;

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

fn tap(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(bloom_src, bloom_samp, uv);
}

// Weight for a Karis average: bright samples count for less, so a lone
// firefly is averaged down rather than smeared into a flickering disc.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn karis(c: vec4<f32>) -> f32 {
    return 1.0 / (1.0 + luma(c.rgb));
}

fn down(uv: vec2<f32>, first: bool) -> vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(bloom_src));
    let a = tap(uv + t * vec2<f32>(-2.0, 2.0));
    let b = tap(uv + t * vec2<f32>(0.0, 2.0));
    let c = tap(uv + t * vec2<f32>(2.0, 2.0));
    let d = tap(uv + t * vec2<f32>(-2.0, 0.0));
    let e = tap(uv);
    let f = tap(uv + t * vec2<f32>(2.0, 0.0));
    let g = tap(uv + t * vec2<f32>(-2.0, -2.0));
    let h = tap(uv + t * vec2<f32>(0.0, -2.0));
    let i = tap(uv + t * vec2<f32>(2.0, -2.0));
    let j = tap(uv + t * vec2<f32>(-1.0, 1.0));
    let k = tap(uv + t * vec2<f32>(1.0, 1.0));
    let l = tap(uv + t * vec2<f32>(-1.0, -1.0));
    let mm = tap(uv + t * vec2<f32>(1.0, -1.0));
    // Five overlapping boxes: the centre one counts for half.
    let centre = (j + k + l + mm) * 0.25;
    let tl = (a + b + d + e) * 0.25;
    let tr = (b + c + e + f) * 0.25;
    let bl = (d + e + g + h) * 0.25;
    let br = (e + f + h + i) * 0.25;
    if (!first) {
        return centre * 0.5 + (tl + tr + bl + br) * 0.125;
    }
    let wc = karis(centre) * 0.5;
    let wtl = karis(tl) * 0.125;
    let wtr = karis(tr) * 0.125;
    let wbl = karis(bl) * 0.125;
    let wbr = karis(br) * 0.125;
    let sum = centre * wc + tl * wtl + tr * wtr + bl * wbl + br * wbr;
    return sum / max(wc + wtl + wtr + wbl + wbr, 1e-6);
}

@fragment
fn fs_down_first(in: VsOut) -> @location(0) vec4<f32> {
    return down(in.uv, true);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    return down(in.uv, false);
}

// Added into the level above by the pipeline's blend state.
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(bloom_src));
    var s = tap(uv_off(in.uv, t, -1.0, 1.0)) + tap(uv_off(in.uv, t, 1.0, 1.0))
        + tap(uv_off(in.uv, t, -1.0, -1.0)) + tap(uv_off(in.uv, t, 1.0, -1.0));
    s = s + 2.0 * (tap(uv_off(in.uv, t, 0.0, 1.0)) + tap(uv_off(in.uv, t, -1.0, 0.0))
        + tap(uv_off(in.uv, t, 1.0, 0.0)) + tap(uv_off(in.uv, t, 0.0, -1.0)));
    s = s + 4.0 * tap(in.uv);
    return s / 16.0;
}

fn uv_off(uv: vec2<f32>, t: vec2<f32>, x: f32, y: f32) -> vec2<f32> {
    return uv + t * vec2<f32>(x, y);
}
