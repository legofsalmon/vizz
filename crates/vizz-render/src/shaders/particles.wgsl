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
    _pad: f32,
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

    // Uniform distribution inside a sphere, then orbit around Y with a
    // per-particle rate so the field shears into ribbons over time.
    let cz = h2 * 2.0 - 1.0;
    let sxy = sqrt(max(0.0, 1.0 - cz * cz));
    let radius = pow(h3, 1.0 / 3.0) * u.spread;
    let orbit = h1 * TAU + u.time * (0.25 + 0.75 * h4) * (1.2 - 0.6 * radius / max(u.spread, 1e-3));
    var p = vec3<f32>(sxy * cos(orbit), cz, sxy * sin(orbit)) * radius;

    // Slow breathing and a vertical drift band keep the field alive even
    // with all controls parked.
    p *= 1.0 + 0.12 * sin(u.time * 0.5 + radius * 3.0);
    p.y += 0.15 * u.spread * sin(u.time * 0.31 + h1 * TAU);

    // Simple perspective: camera on -Z looking at the origin.
    let view = p + vec3<f32>(0.0, 0.0, 3.5);
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
