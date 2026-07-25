// Procedural particle field. No vertex buffers: every particle attribute is
// derived in the vertex shader from the instance-free vertex index, so the
// CPU uploads exactly one small uniform struct per frame and the draw-call
// count parameter is just a vertex count. Two triangles per particle.

struct Uniforms {
    time: f32,        // pre-integrated on CPU: advances at `speed` rate
    aspect: f32,      // width / height
    size: f32,        // particle billboard half-size in view units
    spread: f32,      // field radius in world units
    hue: f32,         // base hue 0..1
    saturation: f32,  // 0..1
    brightness: f32,  // value multiplier (master dim already applied)
    shape: f32,       // geometry mode, see sample_shape
    morph: f32,       // 0..1 blend into the next mode
    twist: f32,       // per-shape distortion amount
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
};

const TAU: f32 = 6.28318530718;

// Dave Hoskins-style hash without sine: stable across GPUs.
fn hash11(p: f32) -> f32 {
    var x = fract(p * 0.1031);
    x = x * (x + 33.33);
    x = x * (x + x);
    return fract(x);
}

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(c.xxx + k) * 6.0 - vec3<f32>(3.0));
    return c.z * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

// Point positions for each geometry mode, all derived from the same four
// hashes so a particle keeps its identity as the shape morphs — the field
// flows between forms instead of being re-scattered.
fn sample_shape(mode: u32, h1: f32, h2: f32, h3: f32, h4: f32, t: f32) -> vec3<f32> {
    switch mode {
        // Solid sphere: uniform by volume, not by angle.
        case 0u: {
            let cz = h2 * 2.0 - 1.0;
            let sxy = sqrt(max(0.0, 1.0 - cz * cz));
            let a = h1 * TAU;
            return vec3<f32>(sxy * cos(a), cz, sxy * sin(a)) * pow(h3, 1.0 / 3.0);
        }
        // Torus.
        case 1u: {
            let major = h1 * TAU;
            let minor = h2 * TAU;
            let r = 0.32 + 0.1 * h3;
            return vec3<f32>(
                (0.68 + r * cos(minor)) * cos(major),
                r * sin(minor),
                (0.68 + r * cos(minor)) * sin(major),
            );
        }
        // Trefoil knot, thickened into a tube.
        case 2u: {
            let u = h1 * TAU;
            let core = vec3<f32>(
                sin(u) + 2.0 * sin(2.0 * u),
                -cos(u) + 2.0 * cos(2.0 * u),
                -sin(3.0 * u),
            ) * 0.26;
            let a = h2 * TAU;
            let jitter = vec3<f32>(cos(a), sin(a), cos(a + 1.7)) * 0.07 * h3;
            return core + jitter;
        }
        // Grid plane: the calm one, good for contrast against the others.
        case 3u: {
            let n = 48.0;
            let ix = floor(h1 * n) / n - 0.5;
            let iz = floor(h2 * n) / n - 0.5;
            return vec3<f32>(ix * 2.0, 0.04 * sin(t + (ix + iz) * 12.0), iz * 2.0);
        }
        // Hollow shell: same sphere, but only near the surface.
        default: {
            let cz = h2 * 2.0 - 1.0;
            let sxy = sqrt(max(0.0, 1.0 - cz * cz));
            let a = h1 * TAU;
            return vec3<f32>(sxy * cos(a), cz, sxy * sin(a)) * (0.9 + 0.1 * h4);
        }
    }
}

const SHAPE_COUNT: u32 = 5u;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let pi = vi / 6u;
    let corner = vi % 6u;
    var offsets = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let off = offsets[corner];

    let fpi = f32(pi);
    let h1 = hash11(fpi + 0.123);
    let h2 = hash11(fpi + 7.456);
    let h3 = hash11(fpi + 3.789);
    let h4 = hash11(fpi + 9.321);

    // Blend between adjacent modes so `shape` sweeps continuously rather
    // than cutting — a swept knob is playable, a stepped one is not.
    let mode_a = u32(floor(u.shape)) % SHAPE_COUNT;
    let mode_b = (mode_a + 1u) % SHAPE_COUNT;
    let blend = clamp(fract(u.shape) + u.morph, 0.0, 1.0);
    let pa = sample_shape(mode_a, h1, h2, h3, h4, u.time);
    let pb = sample_shape(mode_b, h1, h2, h3, h4, u.time);
    var p = mix(pa, pb, smoothstep(0.0, 1.0, blend)) * u.spread;

    // Rotate around Y at a per-particle rate so the field shears into
    // ribbons rather than turning as a rigid body.
    let spin = u.time * (0.25 + 0.75 * h4) * (0.4 + u.twist);
    let cs = cos(spin);
    let sn = sin(spin);
    p = vec3<f32>(p.x * cs - p.z * sn, p.y, p.x * sn + p.z * cs);

    // Twist: rotation that increases with height, the classic taffy pull.
    let tw = u.twist * p.y * 3.0;
    let tc = cos(tw);
    let ts = sin(tw);
    p = vec3<f32>(p.x * tc - p.z * ts, p.y, p.x * ts + p.z * tc);

    // Slow breathing keeps the field alive with every control parked.
    let radius = length(p);
    p *= 1.0 + 0.08 * sin(u.time * 0.5 + radius * 3.0);

    // Camera looks slightly down rather than dead-on. Without this the
    // grid mode sits exactly edge-on and vanishes, and the volumetric
    // shapes lose most of their depth cues.
    let el = 0.34;
    let ce = cos(el);
    let se = sin(el);
    let tilted = vec3<f32>(p.x, p.y * ce - p.z * se, p.y * se + p.z * ce);

    // Simple perspective: camera on -Z looking at the origin.
    let view = tilted + vec3<f32>(0.0, 0.0, 3.5);
    if (view.z < 0.15) {
        // Behind the near plane: emit a degenerate off-screen vertex.
        var cull: VsOut;
        cull.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        cull.uv = vec2<f32>(0.0);
        cull.color = vec3<f32>(0.0);
        return cull;
    }
    let persp = 1.8 / view.z;
    var clip = vec2<f32>(view.x * persp / u.aspect, view.y * persp);
    let half = u.size * persp;
    clip += off * vec2<f32>(half / u.aspect, half);

    // Distance fade so depth reads even without a depth buffer.
    let fade = clamp(1.7 - view.z * 0.28, 0.15, 1.0);
    let hue = fract(u.hue + 0.12 * h1 + 0.03 * sin(u.time * 0.2));
    let col = hsv2rgb(vec3<f32>(hue, u.saturation, u.brightness)) * fade;

    var out: VsOut;
    out.pos = vec4<f32>(clip, 0.0, 1.0);
    out.uv = off;
    out.color = col;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Soft round sprite; additive blending means alpha is irrelevant.
    let d = length(in.uv);
    let a = smoothstep(1.0, 0.1, d);
    return vec4<f32>(in.color * a * a, 1.0);
}
