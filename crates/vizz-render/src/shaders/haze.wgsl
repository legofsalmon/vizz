// Haze: a lit, participating medium in front of the camera, thickened
// where the particles are, so the lamps and the sun cut beams through
// smoke and the cloud casts shafts into it.
//
// Appended to particles.wgsl, so the particles are placed by the same
// `body` the dots are. See haze.rs for the passes.

/// Must match `HazeUniforms` in haze.rs.
struct Haze {
    inv_view_proj: mat4x4<f32>,
    // Froxels across, down and deep, and the particles this frame.
    dims: vec4<u32>,
    // Near and far ends of the volume, as distances along the ray.
    near: f32,
    far: f32,
    // The medium everywhere, per world unit, and how much each particle
    // adds by the share of a froxel it fills.
    density: f32,
    smoke: f32,
    // Henyey-Greenstein's g: 0 scatters evenly, towards 1 mostly forward.
    scatter: f32,
    // 1 where the scene left a depth to stop at, 0 where it did not.
    use_depth: f32,
    // The distance a pixel with no depth is taken to be at.
    no_depth: f32,
    _pad0: f32,
};

@group(1) @binding(0) var<uniform> hz: Haze;
@group(1) @binding(1) var<storage, read_write> haze_counts: array<atomic<u32>>;
@group(1) @binding(10) var<storage, read> haze_counts_in: array<u32>;
@group(1) @binding(2) var haze_density_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(3) var haze_density: texture_3d<f32>;
@group(1) @binding(4) var haze_linear: sampler;
@group(1) @binding(5) var haze_light_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(6) var haze_light: texture_3d<f32>;
@group(1) @binding(7) var haze_sum_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(8) var haze_sum: texture_3d<f32>;
@group(1) @binding(9) var haze_depth: texture_depth_2d;

/// Slices are spaced evenly in log distance, so near froxels are short
/// and far ones long, as the screen sees them.
fn haze_slice_t(z: f32) -> f32 {
    return hz.near * pow(hz.far / hz.near, z / f32(hz.dims.z));
}

fn haze_slice_of(t: f32) -> f32 {
    return log(max(t, hz.near) / hz.near) / log(hz.far / hz.near) * f32(hz.dims.z);
}

/// Which way the camera looks through screen point `ndc`.
fn haze_ray(ndc: vec2<f32>) -> vec3<f32> {
    let w = hz.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(w.xyz / w.w - u.cam_position);
}

/// The centre of froxel `f`, in the world.
fn haze_world(f: vec3<u32>) -> vec3<f32> {
    let d = vec2<f32>(hz.dims.xy);
    let ndc = vec2<f32>((f32(f.x) + 0.5) / d.x * 2.0 - 1.0, 1.0 - (f32(f.y) + 0.5) / d.y * 2.0);
    return u.cam_position + haze_ray(ndc) * haze_slice_t(f32(f.z) + 0.5);
}

/// Where world point `p` falls in the volume, 0..1 on each axis, or a
/// value outside that for a point the volume does not reach.
fn haze_uvw(p: vec3<f32>) -> vec3<f32> {
    let c = u.view_proj * vec4<f32>(p, 1.0);
    if (c.w < 1e-3) {
        return vec3<f32>(-1.0);
    }
    let ndc = c.xy / c.w;
    let t = distance(p, u.cam_position);
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, haze_slice_of(t) / f32(hz.dims.z));
}

fn haze_index(f: vec3<u32>) -> u32 {
    return (f.z * hz.dims.y + f.y) * hz.dims.x + f.x;
}

@compute @workgroup_size(64)
fn cs_haze_clear(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < hz.dims.x * hz.dims.y * hz.dims.z) {
        atomicStore(&haze_counts[id.x], 0u);
    }
}

/// What a whole particle counts for in a froxel: the counts are fixed
/// point, so a particle can be shared between the eight it lies among.
const HAZE_ONE: f32 = 256.0;

/// Each particle counted into the eight froxels round it, by how near it
/// is to each, so the smoke is as smooth as the particles are and the
/// froxel grid does not show as blocks.
@compute @workgroup_size(64)
fn cs_haze_count(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= hz.dims.w) {
        return;
    }
    let uvw = haze_uvw(body(id.x).p);
    if (any(uvw < vec3<f32>(0.0)) || any(uvw >= vec3<f32>(1.0))) {
        return;
    }
    let g = uvw * vec3<f32>(hz.dims.xyz) - 0.5;
    let base = floor(g);
    let fr = g - base;
    for (var c = 0u; c < 8u; c = c + 1u) {
        let o = vec3<f32>(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u));
        let at = base + o;
        if (any(at < vec3<f32>(0.0)) || any(at >= vec3<f32>(hz.dims.xyz))) {
            continue;
        }
        let w = mix(1.0 - fr, fr, o);
        let share = u32(w.x * w.y * w.z * HAZE_ONE + 0.5);
        if (share > 0u) {
            atomicAdd(&haze_counts[haze_index(vec3<u32>(at))], share);
        }
    }
}

/// How thick the medium is at froxel `f`: the haze everywhere, and the
/// particles in it by their density in space, as a share of the whole
/// field. A field of any count spread through about four cubic units, a
/// cloud's usual size, reads as the same smoke: the look is the shape,
/// not how many particles drew it.
@compute @workgroup_size(4, 4, 4)
fn cs_haze_density(@builtin(global_invocation_id) f: vec3<u32>) {
    if (any(f >= hz.dims.xyz)) {
        return;
    }
    // The froxel's volume: its slice's depth times its footprint, which
    // grows with the square of the distance.
    let t0 = haze_slice_t(f32(f.z));
    let t1 = haze_slice_t(f32(f.z) + 1.0);
    let tm = 0.5 * (t0 + t1);
    let across = 2.0 * tm / abs(u.view_proj[0][0]) / f32(hz.dims.x);
    let down = 2.0 * tm / abs(u.view_proj[1][1]) / f32(hz.dims.y);
    let volume = max(across * down * (t1 - t0), 1e-6);
    let share = f32(haze_counts_in[haze_index(f)]) / HAZE_ONE / max(f32(hz.dims.w), 1.0);
    let crowd = share / volume * 4.0;
    let sigma = hz.density * (0.25 + hz.smoke * 6.0 * crowd);
    textureStore(haze_density_out, f, vec4<f32>(sigma, 0.0, 0.0, 0.0));
}

/// The medium's thickness at a world point, sampled between froxels.
fn haze_sigma_at(p: vec3<f32>) -> f32 {
    let uvw = haze_uvw(p);
    if (any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0))) {
        return hz.density * 0.25;
    }
    return textureSampleLevel(haze_density, haze_linear, uvw, 0.0).r;
}

/// How much light from `toward` (unit), up to `reach` away, gets to `p`
/// through the medium: the shadow the smoke casts into itself, which is
/// what makes beams. Sixteen steps through the thickness volume, enough
/// that smoke a tenth of a unit thick casts a shadow from two units off.
fn haze_reach(p: vec3<f32>, toward: vec3<f32>, reach: f32) -> f32 {
    let steps = 16;
    let dt = min(reach, 4.0) / f32(steps);
    var depth = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        depth += haze_sigma_at(p + toward * (dt * (f32(i) + 0.5))) * dt;
    }
    return exp(-depth);
}

/// Henyey-Greenstein, scaled so that even scattering is 1.
fn haze_phase(cos_theta: f32) -> f32 {
    let g = hz.scatter;
    let d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / max(d * sqrt(d), 1e-4);
}

/// The light each froxel scatters towards the camera, per unit length.
@compute @workgroup_size(4, 4, 4)
fn cs_haze_light(@builtin(global_invocation_id) f: vec3<u32>) {
    if (any(f >= hz.dims.xyz)) {
        return;
    }
    let sigma = textureLoad(haze_density, f, 0).r;
    let p = haze_world(f);
    let to_eye = normalize(u.cam_position - p);
    // A trace of the ambient, so haze in an unlit room is still faintly
    // there; most of what it shows is the lamps and the sun.
    var light = vec3<f32>(0.04 * u.light.x);
    for (var i = 0u; i < 2u; i = i + 1u) {
        let level = u.lamp[i].w;
        if (level <= 0.001) {
            continue;
        }
        let d = u.lamp[i].xyz - p;
        let d2 = dot(d, d);
        let r = max(u.lamp_tint[i].w, 0.01);
        // The lamps' own falloff, as on the particles.
        let falloff = (r * r) / (d2 + r * r);
        let dist = sqrt(max(d2, 1e-6));
        let toward = d / dist;
        let lit = u.lamp_tint[i].rgb * (level * falloff);
        // Light reaching the froxel travels away from the lamp; the
        // angle that matters is between that and the way to the eye, so
        // looking towards a lamp through the haze is looking into its
        // forward scatter.
        light += lit * haze_phase(dot(-toward, to_eye)) * haze_reach(p, toward, dist);
    }
    if (u.sun_dir.w > 0.001) {
        let toward = normalize(u.sun_dir.xyz);
        light += u.sun_tint.rgb * u.sun_dir.w * haze_phase(dot(-toward, to_eye)) * haze_reach(p, toward, 4.0);
    }
    textureStore(haze_light_out, f, vec4<f32>(light * sigma, sigma));
}

/// Front to back along each column: the light gathered so far and how
/// much of what lies behind still shows. The step is the analytic one
/// for a slab of constant medium (Hillaire, "Physically Based and
/// Unified Volumetric Rendering in Frostbite", SIGGRAPH 2015), so the
/// result does not brighten as the slices thin.
@compute @workgroup_size(8, 8, 1)
fn cs_haze_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= hz.dims.x || id.y >= hz.dims.y) {
        return;
    }
    var gathered = vec3<f32>(0.0);
    var through = 1.0;
    var t_prev = 0.0;
    for (var z = 0u; z < hz.dims.z; z = z + 1u) {
        let f = vec3<u32>(id.x, id.y, z);
        let l = textureLoad(haze_light, f, 0);
        let sigma = max(l.a, 1e-6);
        // The first slice starts at the camera, not at `near`.
        let t = haze_slice_t(f32(z) + 1.0);
        let slab = exp(-sigma * (t - t_prev));
        gathered += through * (l.rgb / sigma) * (1.0 - slab);
        through *= slab;
        t_prev = t;
        textureStore(haze_sum_out, f, vec4<f32>(gathered, through));
    }
}

struct HazeOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_haze(@builtin(vertex_index) vi: u32) -> HazeOut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    var out: HazeOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

/// Over the frame: what the haze gathered in front of each pixel, and the
/// pixel dimmed by what the haze hides of it. Blended as
/// `out = src + dst * src.a`.
@fragment
fn fs_haze(in: HazeOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(u.viewport_h * u.aspect, u.viewport_h);
    let uv = in.pos.xy / size;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    var t = hz.no_depth;
    if (hz.use_depth > 0.5) {
        let dsize = vec2<f32>(textureDimensions(haze_depth));
        let d = textureLoad(haze_depth, vec2<i32>(uv * dsize), 0);
        if (d < 1.0) {
            let w = hz.inv_view_proj * vec4<f32>(ndc, d, 1.0);
            t = distance(w.xyz / w.w, u.cam_position);
        } else {
            t = hz.far;
        }
    }
    let w = clamp(haze_slice_of(t) / f32(hz.dims.z), 0.0, 1.0);
    // The integrated value at a slice's far edge is stored at its index,
    // so step back half a slice to sample at the distance itself.
    let s = textureSampleLevel(haze_sum, haze_linear, vec3<f32>(uv, max(w - 0.5 / f32(hz.dims.z), 0.0)), 0.0);
    return vec4<f32>(s.rgb, s.a);
}
