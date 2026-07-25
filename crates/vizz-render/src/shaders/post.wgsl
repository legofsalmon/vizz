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
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: Post;
@group(0) @binding(1) var t_scene: texture_2d<f32>;
@group(0) @binding(2) var t_history: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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
    return vec4<f32>(mix(scene.rgb, history.rgb, u.trail), 1.0);
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
    var color = textureSample(t_scene, samp, uv).rgb;

    // Cheap bloom: a few wide taps added back, enough to make additive
    // particles read as luminous without a separate blur chain.
    if (u.glow > 0.001) {
        let d = 0.004 + 0.02 * u.glow;
        var sum = vec3<f32>(0.0);
        sum += textureSample(t_scene, samp, uv + vec2<f32>( d,  0.0)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(-d,  0.0)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(0.0,  d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(0.0, -d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>( d,  d)).rgb;
        sum += textureSample(t_scene, samp, uv + vec2<f32>(-d, -d)).rgb;
        color += sum * (u.glow * 0.16);
    }

    // Gentle shoulder: glow can push past 1, and a hard clip turns
    // highlights into flat white blobs. Weak enough to leave midtones
    // essentially untouched.
    color = color / (vec3<f32>(1.0) + color * 0.15);
    return vec4<f32>(color, 1.0);
}
