// Hard-edged vector layers, composited in one fullscreen pass.
//
// Every layer is a procedural pattern evaluated per fragment — nothing is
// rasterized from geometry, so edges are exact at any resolution and the
// interference between two near-frequency layers (the moiré this look is
// built on) appears at fragment rate, not texture rate.
//
// Anti-aliasing is analytic, not derivative-based. fwidth() lies at the
// discontinuities these patterns are made of — fract() wraps and atan2's
// branch cut both produce one-pixel garbage derivatives — and naga's
// uniformity analysis dislikes derivatives under uniform control flow.
// Instead, every layer transform here is a similarity transform (and the
// kaleido fold an isometry), so a single scalar "pattern units per output
// pixel" propagates exactly by chain rule, and each generator knows its
// own gradient magnitude. Over-Nyquist patterns then converge to their
// duty-cycle tone instead of sparkling: moiré stays, aliasing dissolves.
//
// Blending happens in sRGB-ENCODED space, deliberately: multiply/screen
// on encoded values is what print-era compositors do, and the darker
// encoded product IS the look. The one conversion to linear happens at
// the very end, so an sRGB target re-encodes to exactly the intended
// bytes and a float target receives honest linear light.

const MAX_LAYERS: u32 = 8u;
const PALETTE_SLOTS: u32 = 4u;
const TAU: f32 = 6.28318530718;

// Lane maps must match LayerU / StackU in vector.rs.
struct Layer {
    xform: vec4<f32>, // x,y translate | z rotation (turns) | w scale
    pat: vec4<f32>,   // x kind | y frequency | z phase | w duty
    shape: vec4<f32>, // x sides | y star inset | z kaleido segs | w invert
    style: vec4<f32>, // x blend | y opacity | z palette slot | w reserved
};

struct Stack {
    globals: vec4<f32>, // x aspect | y time | z master dim | w layer count
    bg: vec4<f32>,      // rgb paper (sRGB-encoded) | w target height px
    palette: array<vec4<f32>, PALETTE_SLOTS>,
    layers: array<Layer, MAX_LAYERS>,
};

@group(0) @binding(0) var<uniform> stack: Stack;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // The single-triangle fullscreen trick, as post.wgsl.
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x, y) * 0.5 + 0.5;
    return out;
}

fn rot2(turns: f32) -> mat2x2<f32> {
    let a = turns * TAU;
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(vec2<f32>(c, s), vec2<f32>(-s, c));
}

// Mirror-fold the plane into a wedge of TAU/segs, kaleidoscope-style.
// An isometry: distances survive, so the pixel footprint passes through
// unchanged and the seams between wedges stay antialiased for free.
fn fold(p: vec2<f32>, segs: f32) -> vec2<f32> {
    let sector = TAU / segs;
    let a = atan2(p.y, p.x);
    let folded = abs(fract(a / sector) - 0.5) * sector;
    return length(p) * vec2<f32>(cos(folded), sin(folded));
}

// Shared edge rule: signed distance d and its pixel footprint w, in the
// SAME units, whatever those units are — each generator does its own
// chain rule so this one line is the entire AA policy. Linear step, not
// smoothstep: a linear coverage ramp is the correct box-filter answer
// for a straight edge, and it reads cleaner in flat-colour work.
fn cov(d: f32, w: f32) -> f32 {
    return clamp(0.5 - d / max(w, 1e-5), 0.0, 1.0);
}

// Periodic band coverage in a wrapped coordinate t: ink where fract(t)
// lands inside a band of width `duty`. Distance is measured in t units;
// the caller supplies |∇t| per pixel as w.
fn bands(t: f32, duty: f32, w: f32) -> f32 {
    let d = abs(fract(t) - 0.5) - duty * 0.5;
    return cov(d, w);
}

// --- Generators. Each returns coverage 0..1 for its ink. -------------
// p is in layer space; px is the size of one output pixel in that space.

fn gen_rings(p: vec2<f32>, px: f32, freq: f32, phase: f32, duty: f32) -> f32 {
    // t = r·freq: |∇t| = freq everywhere, including across the wrap.
    return bands(length(p) * freq + phase, duty, px * freq);
}

fn gen_stripes(p: vec2<f32>, px: f32, freq: f32, phase: f32, duty: f32) -> f32 {
    return bands(p.x * freq + phase, duty, px * freq);
}

fn gen_checker(p: vec2<f32>, px: f32, freq: f32, phase: f32) -> f32 {
    // XOR of two square waves. Composed from the coverages rather than a
    // combined SDF, because coverage-XOR (a + b − 2ab) antialiases the
    // four-way corners acceptably where an exact corner SDF would cost
    // far more than the one soft pixel it saves.
    let a = bands(p.x * freq + phase, 0.5, px * freq);
    let b = bands(p.y * freq + phase, 0.5, px * freq);
    return a + b - 2.0 * a * b;
}

fn gen_polygon(p: vec2<f32>, px: f32, sides: f32) -> f32 {
    // Regular n-gon: fold the angle into one sector, then the edge is a
    // half-plane. Distance in scene units, so the footprint is px itself.
    let sector = TAU / max(sides, 2.0);
    let a = atan2(p.y, p.x);
    let folded = (fract(a / sector + 0.5) - 0.5) * sector;
    let d = length(p) * cos(folded) - 0.6;
    return cov(d, px);
}

fn gen_star(p: vec2<f32>, px: f32, sides: f32, inset: f32) -> f32 {
    // Star as a polygon whose radius alternates between 0.6 and
    // 0.6·inset across half-sectors. Not a true SDF near the spike tips
    // — the radial approximation divides by the gradient of r(θ), which
    // is exact on the edges and merely conservative at the vertices —
    // but the error is sub-pixel at any size a layer scale reaches.
    let n = max(sides, 2.0);
    let sector = TAU / n;
    let a = atan2(p.y, p.x);
    let half = fract(a / sector) - 0.5;
    let tip = 1.0 - abs(half) * 2.0; // 1 at spike centre, 0 at valley
    let radius = 0.6 * mix(max(inset, 0.05), 1.0, tip);
    // Radial distance normalised by the edge slope so px stays honest.
    let slope = length(vec2<f32>(1.0, 0.6 * (1.0 - inset) * 2.0 / max(sector, 1e-3)));
    let d = (length(p) - radius) / slope;
    return cov(d, px);
}

fn gen_rays(p: vec2<f32>, px: f32, count: f32, phase: f32, duty: f32) -> f32 {
    // t = θ/τ·count: |∇t| = count/(τ·r), which blows up at the centre —
    // clamping r keeps the footprint finite there, and the widening w
    // makes the hub converge to flat duty-cycle tone instead of noise.
    let r = max(length(p), 0.02);
    let t = atan2(p.y, p.x) / TAU * count + phase;
    return bands(t, duty, px * count / (TAU * r));
}

fn gen_dots(p: vec2<f32>, px: f32, freq: f32, phase: f32, duty: f32) -> f32 {
    // A disc per grid cell, in cell units (p·freq), so w = px·freq.
    let cell = fract(p * freq + phase) - 0.5;
    let d = length(cell) - duty * 0.35;
    return cov(d, px * freq);
}

fn eval_layer(l: Layer, p_scene: vec2<f32>, px_scene: f32) -> f32 {
    // Inverse similarity transform into layer space; the footprint
    // scales by exactly the factor the coordinates do.
    var p = rot2(-l.xform.z) * (p_scene - l.xform.xy) / max(l.xform.w, 1e-3);
    let px = px_scene / max(l.xform.w, 1e-3);
    if (l.shape.z >= 2.0) {
        p = fold(p, floor(l.shape.z));
    }
    let freq = l.pat.y;
    let phase = l.pat.z;
    let duty = l.pat.w;
    var c = 0.0;
    switch u32(l.pat.x) {
        case 1u: { // rings
            c = gen_rings(p, px, freq, phase, duty);
        }
        case 2u: { // stripes
            c = gen_stripes(p, px, freq, phase, duty);
        }
        case 3u: { // checker
            c = gen_checker(p, px, freq, phase);
        }
        case 4u: { // polygon
            c = gen_polygon(p, px, l.shape.x);
        }
        case 5u: { // star
            c = gen_star(p, px, l.shape.x, l.shape.y);
        }
        case 6u: { // rays
            c = gen_rays(p, px, freq, phase, duty);
        }
        case 7u: { // dots
            c = gen_dots(p, px, freq, phase, duty);
        }
        default: {
            c = 0.0;
        }
    }
    return mix(c, 1.0 - c, step(0.5, l.shape.w));
}

// Blend modes on sRGB-encoded values — see the header comment for why.
fn blend_mode(mode: u32, d: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 1u: { // multiply
            return d * s;
        }
        case 2u: { // screen
            return 1.0 - (1.0 - d) * (1.0 - s);
        }
        case 3u: { // add
            return min(d + s, vec3<f32>(1.0));
        }
        case 4u: { // difference
            return abs(d - s);
        }
        case 5u: { // exclusion
            return d + s - 2.0 * d * s;
        }
        case 6u: { // subtract
            return max(d - s, vec3<f32>(0.0));
        }
        default: { // normal
            return s;
        }
    }
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Scene space: y in [-1, 1] with up positive, x scaled by aspect so
    // a circle is a circle. One output pixel spans 2/height scene units.
    let aspect = stack.globals.x;
    let p_scene = vec2<f32>(
        (in.uv.x * 2.0 - 1.0) * aspect,
        1.0 - in.uv.y * 2.0,
    );
    let px_scene = 2.0 / max(stack.bg.w, 1.0);

    var col = stack.bg.rgb;
    let n = min(u32(stack.globals.w), MAX_LAYERS);
    for (var i = 0u; i < n; i = i + 1u) {
        let l = stack.layers[i];
        if (u32(l.pat.x) == 0u) {
            continue; // off
        }
        let c = eval_layer(l, p_scene, px_scene);
        let ink = stack.palette[min(u32(l.style.z), PALETTE_SLOTS - 1u)].rgb;
        // Photoshop order: blend full-strength, then lerp by coverage ×
        // opacity — so a half-covered pixel is half the blended colour,
        // not a blend with a half-strength source.
        let blended = blend_mode(u32(l.style.x), col, ink);
        col = mix(col, blended, c * l.style.y);
    }

    // The one space conversion: everything above was sRGB-encoded, the
    // target wants linear. Master dim rides the encoded value so a fade
    // to black tracks perceived brightness the way a printed tint does.
    return vec4<f32>(srgb_to_linear(col * stack.globals.z), 1.0);
}
