// Procedural particle field. No vertex buffers: every particle attribute is
// derived in the vertex shader from the instance-free vertex index, so the
// CPU uploads exactly one small uniform struct per frame and the draw-call
// count parameter is just a vertex count. Two triangles per particle.

struct Uniforms {
    // Mat4 first: it needs 16-byte alignment, and putting it anywhere else
    // makes the layout depend on how many scalars happen to precede it.
    view_proj: mat4x4<f32>,
    cam_right: vec3<f32>,
    focus: f32,
    cam_up: vec3<f32>,
    defocus: f32,
    cam_position: vec3<f32>,
    _pad_cam: f32,
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
    cloud_a: f32,       // slot index for the A cloud
    cloud_b: f32,       // slot index for the B cloud
    cloud_morph: f32,   // 0 = A, 1 = B
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
const CLOUD_SLOTS: u32 = 4u;

/// Raw texel for a slot, so callers can use the packed colour too.
fn cloud_texel(which: u32, h: f32, t: f32) -> vec4<f32> {
    let flow = u32(max(t, 0.0) * 260.0);
    let idx = (u32(h * f32(ATTRACTOR_POINTS)) + flow) % ATTRACTOR_POINTS;
    let row = (which % CLOUD_SLOTS) * (ATTRACTOR_POINTS / ATTRACTOR_W) + idx / ATTRACTOR_W;
    return textureLoad(t_attractor, vec2<u32>(idx % ATTRACTOR_W, row), 0);
}

/// Colour packed into the w channel by the CPU: 8 bits per channel.
/// Imported scans carry colour; procedural slots store white and so take
/// the palette instead.
fn cloud_color(w: f32) -> vec3<f32> {
    let bits = bitcast<u32>(w);
    return vec3<f32>(
        f32((bits >> 16u) & 255u),
        f32((bits >> 8u) & 255u),
        f32(bits & 255u),
    ) / 255.0;
}

fn attractor_point(which: u32, h: f32, t: f32, jitter: vec3<f32>) -> vec3<f32> {
    // Flow rate is in points per unit of visual time, so `/particles/speed`
    // drives it like everything else.
    // Consecutive points are close together, so without a little spread
    // the cloud collapses onto a wire rather than reading as a volume.
    return cloud_texel(which, h, t).xyz + jitter * 0.02;
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
        case 6u: {
            let j = vec3<f32>(h2 * 2.0 - 1.0, h3 * 2.0 - 1.0, h4 * 2.0 - 1.0);
            return attractor_point(1u, h1, t, j);
        }
        // Cloud pair: any two slots, blended by /cloud/morph. The
        // shape-mode sweep only reaches *adjacent* modes, so morphing an
        // imported scan into an attractor needs its own control. Particles
        // keep their index across the blend, so the same point travels
        // from one cloud to the other rather than the field being
        // re-scattered.
        default: {
            let j = vec3<f32>(h2 * 2.0 - 1.0, h3 * 2.0 - 1.0, h4 * 2.0 - 1.0);
            let a = attractor_point(u32(u.cloud_a), h1, t, j);
            let b = attractor_point(u32(u.cloud_b), h1, t, j);
            return mix(a, b, smoothstep(0.0, 1.0, u.cloud_morph));
        }
    }
}

const SHAPE_COUNT: u32 = 8u;

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

/// Imported clouds carry their own colour; procedural ones store white and
/// take the palette. Blended the same way the positions are, so a morph
/// crossfades colour along with shape.
fn cloud_tint(mode_a: u32, mode_b: u32, blend: f32, h: f32, t: f32) -> vec3<f32> {
    let ca = slot_tint(mode_a, h, t);
    let cb = slot_tint(mode_b, h, t);
    return mix(ca, cb, blend);
}

fn slot_tint(mode: u32, h: f32, t: f32) -> vec3<f32> {
    if (mode == 7u) {
        return mix(
            cloud_color(cloud_texel(u32(u.cloud_a), h, t).w),
            cloud_color(cloud_texel(u32(u.cloud_b), h, t).w),
            smoothstep(0.0, 1.0, u.cloud_morph),
        );
    }
    if (mode >= 5u) {
        return cloud_color(cloud_texel(mode - 5u, h, t).w);
    }
    return vec3<f32>(1.0);
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

    // Real projection, so the camera can move and the room can line up
    // with the frame. The old fixed transform could not express either.
    let centre = u.view_proj * vec4<f32>(p, 1.0);
    if (centre.w < 0.02) {
        // Behind the camera: emit a degenerate off-screen vertex rather
        // than letting the perspective divide flip it back into view.
        var cull: VsOut;
        cull.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        cull.uv = vec2<f32>(0.0);
        cull.color = vec3<f32>(0.0);
        return cull;
    }

    // Depth of field, done by resizing the sprite rather than blurring the
    // frame: a defocused point light *is* a larger, dimmer disc, so this is
    // closer to the real thing than a post-process blur and costs nothing.
    let dist = distance(p, u.cam_position);
    let coc = 1.0 + abs(dist - u.focus) * u.defocus * 2.5;
    let half = u.size * coc;

    // Billboard in world space against the camera basis, so sprites face
    // the camera from any angle instead of only from straight on.
    // Named apart from the quad-corner index above.
    let corner_pos = p + (u.cam_right * off.x + u.cam_up * off.y) * half;
    var clip4 = u.view_proj * vec4<f32>(corner_pos, 1.0);

    // Spreading the same energy over a wider disc dims it; without this,
    // defocusing brightens the frame instead of softening it.
    let bokeh = 1.0 / (coc * coc);
    // Distance fade so depth still reads without a depth buffer.
    let fade = clamp(1.7 - centre.w * 0.28, 0.15, 1.0) * bokeh;
    // `w` after the view-projection is the view-space depth, which is what
    // the depth-driven palette wants.
    let drive = drive_value(u.color_drive, h1, radius, centre.w, p.y);
    let t = drive * u.color_spread + 0.03 * sin(u.time * 0.2);
    // An imported cloud's own colour multiplies the palette rather than
    // replacing it, so the palette still works as a tint and a white
    // procedural cloud is unaffected.
    let tint = cloud_tint(mode_a, mode_b, smoothstep(0.0, 1.0, blend), h1, u.time);
    let col = palette_color(u.palette, t, u.saturation, u.hue) * tint * u.brightness * fade;

    var out: VsOut;
    out.pos = clip4;
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
