// Procedural particle field. No vertex buffers: every particle attribute is
// derived in the vertex shader from the instance-free vertex index, so the
// CPU uploads exactly one small uniform struct per frame and the draw-call
// count parameter is just a vertex count. Two triangles per particle.

struct Uniforms {
    time: f32,          // pre-integrated on CPU: advances at `speed` rate
    aspect: f32,        // width / height
    size: f32,          // particle billboard half-size in view units
    spread: f32,        // field radius in world units
    hue: f32,           // base hue / palette offset 0..1
    saturation: f32,    // 0..1
    brightness: f32,    // value multiplier (master dim already applied)
    shape: f32,         // geometry mode, see sample_shape
    morph: f32,         // 0..1 blend into the next mode
    twist: f32,         // per-shape distortion amount
    palette: f32,       // 0 = classic HSV, 1..4 = cosine gradients
    color_spread: f32,  // how much of the palette the field spans
    color_drive: f32,   // what maps to palette position, see drive_value
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Trajectory lookup: one texel per point, row-major, attractors stacked.
@group(0) @binding(1) var t_attractor: texture_2d<f32>;

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

// --- Strange attractors -----------------------------------------------
//
// Read from a texture the CPU filled at startup, one texel per trajectory
// point, in time order (see attractor.rs for why it is not iterated here).
// Texels are consecutive in time, so adding the same offset to every
// particle's index sweeps the whole cloud forward along the path.
const ATTRACTOR_POINTS: u32 = 65536u;
const ATTRACTOR_W: u32 = 256u;

fn attractor_point(which: u32, h: f32, t: f32, jitter: vec3<f32>) -> vec3<f32> {
    // Flow rate is in points per unit of visual time, so `/particles/speed`
    // drives it like everything else.
    let flow = u32(max(t, 0.0) * 260.0);
    let idx = (u32(h * f32(ATTRACTOR_POINTS)) + flow) % ATTRACTOR_POINTS;
    let row = which * (ATTRACTOR_POINTS / ATTRACTOR_W) + idx / ATTRACTOR_W;
    let texel = textureLoad(t_attractor, vec2<u32>(idx % ATTRACTOR_W, row), 0);
    // Consecutive points are close together, so without a little spread
    // the cloud collapses onto a wire rather than reading as a volume.
    return texel.xyz + jitter * 0.02;
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
        case 4u: {
            let cz = h2 * 2.0 - 1.0;
            let sxy = sqrt(max(0.0, 1.0 - cz * cz));
            let a = h1 * TAU;
            return vec3<f32>(sxy * cos(a), cz, sxy * sin(a)) * (0.9 + 0.1 * h4);
        }
        // Lorenz: the butterfly.
        case 5u: {
            let j = vec3<f32>(h2 * 2.0 - 1.0, h3 * 2.0 - 1.0, h4 * 2.0 - 1.0);
            return attractor_point(0u, h1, t, j);
        }
        // Aizawa: rounder, shell-like, with a spike through the poles.
        default: {
            let j = vec3<f32>(h2 * 2.0 - 1.0, h3 * 2.0 - 1.0, h4 * 2.0 - 1.0);
            return attractor_point(1u, h1, t, j);
        }
    }
}

const SHAPE_COUNT: u32 = 7u;

// How much a mode's rotation should be rigid rather than per-particle.
//
// Giving each particle its own spin rate shears the field into ribbons,
// which is what makes the blobs and bodies of revolution look alive. An
// attractor is neither: the Lorenz butterfly's whole appeal is a form
// that is *not* rotationally symmetric, and differential spin smears its
// two lobes into an anonymous cone within a couple of seconds. Rotate
// those rigidly so the shape survives being turned.
fn rigidity(mode: u32) -> f32 {
    return select(0.0, 1.0, mode >= 5u);
}

// --- Colour -----------------------------------------------------------

// Inigo Quilez cosine gradients: color = a + b*cos(TAU*(c*t + d)). Four
// coefficients describe a whole smooth ramp, it costs one cosine, and it
// loops seamlessly — which matters because the drive value wraps.
fn cos_palette(id: u32, t: f32) -> vec3<f32> {
    switch id {
        // Full spectrum.
        case 1u: {
            return vec3<f32>(0.5) + vec3<f32>(0.5)
                * cos(TAU * (vec3<f32>(1.0) * t + vec3<f32>(0.0, 0.33, 0.67)));
        }
        // Amber through magenta: warm, reads well on a dark stage.
        case 2u: {
            return vec3<f32>(0.5) + vec3<f32>(0.5)
                * cos(TAU * (vec3<f32>(1.0) * t + vec3<f32>(0.0, 0.10, 0.20)));
        }
        // Teal / green / gold.
        case 3u: {
            return vec3<f32>(0.5) + vec3<f32>(0.5)
                * cos(TAU * (vec3<f32>(1.0, 1.0, 0.5) * t + vec3<f32>(0.8, 0.90, 0.30)));
        }
        // Two-tone red/blue: the most graphic of the set.
        default: {
            return vec3<f32>(0.5) + vec3<f32>(0.5)
                * cos(TAU * (vec3<f32>(2.0, 1.0, 0.0) * t + vec3<f32>(0.5, 0.20, 0.25)));
        }
    }
}

// Cosine palettes are fully saturated by construction, so
// `/particles/saturation` has to be applied by hand for the control to
// stay meaningful once you leave palette 0.
fn mix_sat(c: vec3<f32>, sat: f32) -> vec3<f32> {
    let lum = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    return mix(vec3<f32>(lum), c, sat);
}

// Palette 0 is the original HSV colouring, so the default look is
// unchanged and `/particles/hue` still means what it used to. Above 0 the
// index crossfades on through the cosine gradients.
fn palette_color(idx: f32, t: f32, sat: f32, hue: f32) -> vec3<f32> {
    let hsv = hsv2rgb(vec3<f32>(fract(hue + t), sat, 1.0));
    if (idx <= 0.0) {
        return hsv;
    }
    let ia = u32(floor(idx));
    let f = fract(idx);
    let cur = select(mix_sat(cos_palette(ia, t), sat), hsv, ia == 0u);
    let nxt = mix_sat(cos_palette(ia + 1u, t), sat);
    return mix(cur, nxt, f);
}

// What position in the palette a particle gets. Index is per-particle
// confetti; the others tie colour to the geometry, which is what makes a
// shape read as a solid rather than a cloud of unrelated dots.
fn drive_value(mode: f32, h: f32, radius: f32, depth: f32, height: f32) -> f32 {
    if (mode >= 2.5) {
        return height * 0.5 + 0.5;
    }
    if (mode >= 1.5) {
        return clamp((depth - 1.8) * 0.35, 0.0, 1.0);
    }
    if (mode >= 0.5) {
        return clamp(radius * 0.55, 0.0, 1.0);
    }
    return h;
}

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
    // Only evaluate both forms when actually between them. `blend` comes
    // from uniforms, so this branch is uniform across the draw and costs
    // nothing — it just halves the shape work whenever the knob is parked,
    // which is most of the time.
    var p: vec3<f32>;
    if (blend <= 0.001) {
        p = sample_shape(mode_a, h1, h2, h3, h4, u.time);
    } else if (blend >= 0.999) {
        p = sample_shape(mode_b, h1, h2, h3, h4, u.time);
    } else {
        let pa = sample_shape(mode_a, h1, h2, h3, h4, u.time);
        let pb = sample_shape(mode_b, h1, h2, h3, h4, u.time);
        p = mix(pa, pb, smoothstep(0.0, 1.0, blend));
    }
    p *= u.spread;

    // Rotate around Y at a per-particle rate so the field shears into
    // ribbons rather than turning as a rigid body — except for modes that
    // need their shape to survive the rotation.
    let rigid = mix(rigidity(mode_a), rigidity(mode_b), blend);
    let rate = mix(0.25 + 0.75 * h4, 0.55, rigid);
    let spin = u.time * rate * (0.4 + u.twist);
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
    let drive = drive_value(u.color_drive, h1, radius, view.z, tilted.y);
    let t = drive * u.color_spread + 0.03 * sin(u.time * 0.2);
    let col = palette_color(u.palette, t, u.saturation, u.hue) * u.brightness * fade;

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
