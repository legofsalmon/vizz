// Exposure meter for the graded path, run on the post chain's HDR history
// after feedback and only while /fx/grade is up. See grade.rs.
//
// A histogram of log luminance over the lit pixels, then one thread that
// reads a high percentile off it and eases the exposure towards putting
// that percentile at `key`. Metering a highlight rather than the average
// is deliberate: a particle field is mostly black, and an average meter
// spends the whole frame lifting the background. What went wrong in the
// shipped looks is highlights clipping — 7 to 58% of lit pixels in the
// 2026-09-23 previz review — so the highlights are what is metered.
// The same scheme as Unreal's histogram auto-exposure, which also works
// from percentiles of a log-luminance histogram rather than a mean.

struct Meter {
    // Share of the way to this frame's target the exposure moves, 0..1.
    adapt: f32,
    // Bias in stops, applied after adaptation so it acts at once.
    ev: f32,
    // Largest gain the meter may apply. 1 in the live app: it may darken
    // a look that clips, but never lift a fade — a master dim or a slow
    // fade to black has to reach black, not be metered back up.
    max_gain: f32,
    // Where the metered highlight lands, in scene-linear units before the
    // curve. Around 1.5 sits it on the curve's shoulder.
    key: f32,
    // Which highlight: 0.99 is the level 1% of lit pixels are above.
    percentile: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

const BINS: u32 = 128u;
const MIN_EV: f32 = -14.0;
const MAX_EV: f32 = 6.0;
// Darker than this is background, not subject.
const LIT: f32 = 1.0e-3;
// Fewer lit samples than this and there is nothing to meter; hold.
const MIN_LIT: u32 = 16u;

@group(0) @binding(0) var<uniform> m: Meter;
@group(0) @binding(1) var meter_src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> hist: array<atomic<u32>, 128>;
// [0] exposure to apply, [1] metered log2 gain, [2] lit samples, [3] 1 once valid.
@group(0) @binding(3) var<storage, read_write> exposure: array<f32, 4>;

var<workgroup> local_hist: array<atomic<u32>, 128>;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(16, 16)
fn cs_histogram(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    if (li < BINS) {
        atomicStore(&local_hist[li], 0u);
    }
    workgroupBarrier();
    let size = textureDimensions(meter_src);
    // Every other pixel each way: a quarter of the reads for the same
    // distribution, which is all a meter needs.
    let p = gid.xy * 2u;
    if (p.x < size.x && p.y < size.y) {
        let l = luma(textureLoad(meter_src, p, 0).rgb);
        if (l > LIT) {
            let t = (log2(l) - MIN_EV) / (MAX_EV - MIN_EV);
            let bin = u32(clamp(t * f32(BINS), 0.0, f32(BINS - 1u)));
            atomicAdd(&local_hist[bin], 1u);
        }
    }
    workgroupBarrier();
    if (li < BINS) {
        let n = atomicLoad(&local_hist[li]);
        if (n > 0u) {
            atomicAdd(&hist[li], n);
        }
    }
}

@compute @workgroup_size(1)
fn cs_adapt() {
    var total = 0u;
    for (var i = 0u; i < BINS; i = i + 1u) {
        total = total + atomicLoad(&hist[i]);
    }
    exposure[2] = f32(total);
    let valid = exposure[3] > 0.5;
    if (total < MIN_LIT) {
        // Nothing lit: keep what the last lit frame chose, so a blackout
        // does not swing the exposure and flash on the way back.
        let held = select(0.0, exposure[1], valid);
        exposure[0] = exp2(held + m.ev);
        return;
    }
    let want = u32(ceil(f32(total) * clamp(m.percentile, 0.0, 1.0)));
    var seen = 0u;
    var bin = BINS - 1u;
    for (var i = 0u; i < BINS; i = i + 1u) {
        seen = seen + atomicLoad(&hist[i]);
        if (seen >= want) {
            bin = i;
            break;
        }
    }
    let level = exp2(MIN_EV + (f32(bin) + 0.5) / f32(BINS) * (MAX_EV - MIN_EV));
    let aim = log2(min(m.key / level, m.max_gain));
    // Eased in stops, so a halving and a doubling take the same time.
    let now = select(aim, mix(exposure[1], aim, clamp(m.adapt, 0.0, 1.0)), valid);
    exposure[1] = now;
    exposure[0] = exp2(now + m.ev);
    exposure[3] = 1.0;
}
