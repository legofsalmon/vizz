// Post-processing: feedback trails, then mirroring and glow.
//
// Two passes over the same shader. The feedback pass is what makes the
// output read as VJ material rather than a particle demo: last frame is
// zoomed/rotated slightly and mixed back in, so motion leaves trails and
// a sustained zoom builds a tunnel.

struct Post {
    trail: f32,   // how much of the previous frame survives, 0..~0.98
    zoom: f32,    // per-frame scale applied to the history (1 = still)
    spin: f32,    // per-frame rotation of the history, radians
    mirror: f32,  // 0 none, 1 horizontal, 2 quad, 3 kaleidoscope
    glow: f32,    // extra bloom-ish lift
    aspect: f32,
    shift: f32,   // radial RGB split, 0 = off
    flash: f32,   // punch: mix toward white, 0..1
    invert: f32,  // punch: invert after the shoulder, 0..1
    black: f32,   // punch: darken rgb (alpha untouched), 0..1
    downsample: f32,  // 1 = scene is 1–2× the output; filter it down
    // One output pixel, in uv. Two scalars rather than a vec2, which
    // would align to 8 bytes and leave a hole the Rust side does not have.
    out_texel_x: f32,
    out_texel_y: f32,
    grade: f32,       // 0 = the original picture, 1 = fully graded
    ev: f32,          // (meter input; the composite reads the result)
    adapt: f32,       // (meter input)
    max_gain: f32,    // (meter input)
    bloom_levels: f32, // levels summed into t_bloom
    bg_r: f32,        // the background the scene cleared to, linear
    bg_g: f32,
    bg_b: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Post;
@group(0) @binding(1) var t_scene: texture_2d<f32>;
@group(0) @binding(2) var t_history: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
// The graded path's inputs, from grade.rs: the bloom chain's top level,
// and the metered exposure in [0].
@group(0) @binding(4) var t_bloom: texture_2d<f32>;
@group(0) @binding(5) var<storage, read> exposure: array<f32, 4>;

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

// Rotate/scale around the centre, correcting for aspect so a circle stays
// a circle rather than shearing into an ellipse.
fn transform_uv(uv: vec2<f32>, scale: f32, angle: f32, aspect: f32) -> vec2<f32> {
    var p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let c = cos(angle);
    let s = sin(angle);
    p = vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c) / max(scale, 1e-4);
    return p / vec2<f32>(aspect, 1.0) + vec2<f32>(0.5);
}

@fragment
fn fs_feedback(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSample(t_scene, samp, in.uv);
    // Sampling the *transformed* history is what creates the tunnel: each
    // frame the previous image is nudged outward (or inward) a little.
    let warped = transform_uv(in.uv, u.zoom, u.spin, u.aspect);
    var history = vec4<f32>(0.0);
    // Outside the frame there is no history; sampling clamped edges would
    // smear the border inward over time.
    if (warped.x >= 0.0 && warped.x <= 1.0 && warped.y >= 0.0 && warped.y <= 1.0) {
        history = textureSample(t_history, samp, warped);
    }
    // Blend rather than accumulate. Adding the history outright makes a
    // geometric series with gain 1/(1-trail) — at trail 0.96 that is 25x
    // the scene, which saturates to flat white within a second no matter
    // how hard the tone-map works. A lerp keeps the steady state at the
    // scene's own level while still holding bright cores for a long time.
    // Alpha travels with the colour it belongs to. A trail that faded in
    // brightness while staying fully opaque would punch a solid hole in a
    // transparent output wherever the field had ever been.
    return vec4<f32>(
        mix(scene.rgb, history.rgb, u.trail),
        mix(scene.a, history.a, u.trail),
    );
}

// AgX, Troy Sobotka's filmic view transform (2022), as the polynomial fit
// in Benjamin Wrensch's "Minimal AgX implementation" (iolite, 2023).
//
// Scene-linear in, display-linear out. What it does that the old
// `c / (1 + 0.15c)` shoulder cannot: compress a highlight towards white in
// log space over about sixteen stops, and desaturate as it goes, so a
// dense core of saturated sprites reads as a hot white centre fading into
// its colour instead of a flat disc of clipped primary. That flat disc is
// most of why the shipped looks read as "blown out".
fn agx(c: vec3<f32>) -> vec3<f32> {
    // Column-major, as the reference writes them.
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(c, vec3<f32>(0.0));
    v = clamp(log2(max(v, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    let x2 = v * v;
    let x4 = x2 * x2;
    v = 15.5 * x4 * x2 - 40.14 * x4 * v + 31.96 * x4 - 6.868 * x2 * v + 0.4298 * x2 + 0.1191 * v - 0.00232;
    v = outset * v;
    // The fit's output is display-encoded with a 2.2 power; back to linear
    // for the sRGB (or float) master to encode.
    return pow(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

// Fold UV space for mirror/kaleidoscope modes.
fn fold(uv: vec2<f32>, mode: f32, aspect: f32) -> vec2<f32> {
    if (mode < 0.5) {
        return uv;
    }
    if (mode < 1.5) {
        // Mirror left/right.
        return vec2<f32>(0.5 - abs(uv.x - 0.5), uv.y);
    }
    if (mode < 2.5) {
        // Quad mirror: both axes.
        return vec2<f32>(0.5 - abs(uv.x - 0.5), 0.5 - abs(uv.y - 0.5));
    }
    // Kaleidoscope: six wedges in polar space.
    var p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let r = length(p);
    var a = atan2(p.y, p.x);
    let wedge = 3.14159265 / 3.0;
    a = abs(a - wedge * floor(a / wedge + 0.5));
    p = vec2<f32>(cos(a), sin(a)) * r;
    return p / vec2<f32>(aspect, 1.0) + vec2<f32>(0.5);
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let uv = fold(in.uv, u.mirror, u.aspect);
    var sampled = textureSample(t_scene, samp, uv);
    // Between 1× and 2× the single tap above reads only the 2×2 texels
    // at the pixel centre, and the output pixel covers more than that.
    // Four taps a quarter of an output pixel from the centre cover the
    // whole footprint. Not taken at 1× or exactly 2×, where one tap is
    // already exact, so those stay the picture they always were.
    if (u.downsample > 0.5) {
        let q = 0.25 * vec2<f32>(u.out_texel_x, u.out_texel_y);
        sampled = 0.25 * (
            textureSample(t_scene, samp, fold(in.uv + vec2<f32>(-q.x, -q.y), u.mirror, u.aspect)) +
            textureSample(t_scene, samp, fold(in.uv + vec2<f32>( q.x, -q.y), u.mirror, u.aspect)) +
            textureSample(t_scene, samp, fold(in.uv + vec2<f32>(-q.x,  q.y), u.mirror, u.aspect)) +
            textureSample(t_scene, samp, fold(in.uv + vec2<f32>( q.x,  q.y), u.mirror, u.aspect))
        );
    }
    var color = sampled.rgb;
    var alpha = sampled.a;

    // Radial RGB split. Offsetting the channels along the vector from
    // centre — rather than by a fixed amount — is what makes this read as
    // a lens rather than as blur: the middle of the frame stays sharp and
    // the fringing grows towards the edges, like real chromatic
    // aberration. Green is left alone so the image does not shift hue
    // overall, only fringe.
    if (u.shift > 0.001) {
        let radial = (uv - vec2<f32>(0.5)) * (u.shift * 0.06);
        color.r = textureSample(t_scene, samp, uv + radial).r;
        color.b = textureSample(t_scene, samp, uv - radial).b;
    }

    // The graded picture, when asked for. Computed from the same sample
    // before the old glow and shoulder touch it, and mixed over the old
    // picture at the end, so the knob fades between the two.
    var graded = vec3<f32>(0.0);
    var graded_alpha = alpha;
    if (u.grade > 0.0) {
        let bloom = textureSample(t_bloom, samp, uv) / max(u.bloom_levels, 1.0);
        // Energy-conserving: the bloom is mixed in rather than added, so
        // raising it spreads the light rather than making more of it.
        let spread = clamp(u.glow * 0.3, 0.0, 0.3);
        let hdr = mix(color, bloom.rgb, spread);
        // Grade the light, not the backdrop: take the background out, grade
        // what is left, and put it back as it was. Graded whole, a navy
        // chosen to match a venue comes out nearly black — the curve's toe
        // is doing its job, on something that is not the subject.
        let bg = vec3<f32>(u.bg_r, u.bg_g, u.bg_b);
        graded = bg + agx(max(hdr - bg, vec3<f32>(0.0)) * exposure[0]);
        graded_alpha = clamp(mix(alpha, bloom.a, spread), 0.0, 1.0);
    }

    // Cheap bloom: a few wide taps added back, enough to make additive
    // particles read as luminous without a separate blur chain.
    if (u.glow > 0.001 && u.grade < 1.0) {
        let d = 0.004 + 0.02 * u.glow;
        var sum = vec3<f32>(0.0);
        var sum_a = 0.0;
        sum += textureSample(t_scene, samp, uv + vec2<f32>( d,  0.0)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(-d,  0.0)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(0.0,  d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(0.0, -d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>( d,  d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(-d, -d)).rgb;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>( d,  0.0)).a;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>(-d,  0.0)).a;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>(0.0,  d)).a;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>(0.0, -d)).a;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>( d,  d)).a;
        sum_a += textureSample(t_scene, samp, uv + vec2<f32>(-d, -d)).a;
        color += sum * (u.glow * 0.16);
        // The halo has to be present in alpha too. Glow that brightened
        // pixels without covering them would show as a bloom that vanishes
        // the moment the output is composited over anything.
        alpha += sum_a * (u.glow * 0.16);
    }

    // Gentle shoulder: glow can push past 1, and a hard clip turns
    // highlights into flat white blobs. Weak enough to leave midtones
    // essentially untouched.
    color = color / (vec3<f32>(1.0) + color * 0.15);
    if (u.grade > 0.0) {
        color = mix(color, graded, clamp(u.grade, 0.0, 1.0));
        alpha = mix(alpha, graded_alpha, clamp(u.grade, 0.0, 1.0));
    }

    // Punch gestures, last so they act on the finished picture. Invert
    // sits after the shoulder on purpose — inverting HDR would make
    // negative light. Flash raises alpha too (a flash must cover), black
    // leaves alpha alone (a blackout dims the light, not the layer —
    // the same contract as the master dim).
    color = mix(color, vec3<f32>(1.0) - color, u.invert);
    color = color * (1.0 - u.black);
    color = mix(color, vec3<f32>(1.0), u.flash);
    alpha = max(alpha, u.flash);
    return vec4<f32>(color, clamp(alpha, 0.0, 1.0));
}
