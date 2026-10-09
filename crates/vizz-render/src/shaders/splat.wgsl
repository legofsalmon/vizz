// Gaussian splats: every particle as a soft ellipse of colour, blended
// over the frame from the farthest to the nearest.
//
// Appended to particles.wgsl, so a splat is placed and coloured by the
// same `body` and `body_albedo` the dots are. See splat.rs for the passes.

/// Must match `SplatUniforms` in splat.rs.
struct SplatPass {
    // Particles this frame, and the sort's length: the next power of two.
    count: u32,
    padded: u32,
    _pad0: u32,
    _pad1: u32,
};

/// One step of the bitonic sort. Must match `SortStep` in splat.rs.
struct SortStep {
    k: u32,
    j: u32,
    _pad0: u32,
    _pad1: u32,
};

/// A splat as the screen sees it. Must match `PROJECTED_BYTES` in splat.rs.
struct Projected {
    // Centre in normalised device coordinates.
    centre: vec2<f32>,
    // How far out from the centre, in pixels, it is worth drawing.
    radius: f32,
    // Opacity at the centre; 0 for nothing to draw.
    alpha: f32,
    // The inverse of its covariance on screen, in pixels: (xx, xy, yy).
    conic: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    _pad1: f32,
};

@group(1) @binding(0) var<uniform> sp: SplatPass;
@group(1) @binding(1) var t_splat_shape: texture_2d<f32>;
@group(1) @binding(2) var t_splat_turn: texture_2d<f32>;
@group(1) @binding(3) var<storage, read_write> projected: array<Projected>;
@group(1) @binding(4) var<storage, read_write> keys: array<f32>;
@group(1) @binding(5) var<storage, read_write> order: array<u32>;
@group(1) @binding(6) var<storage, read> projected_in: array<Projected>;
@group(1) @binding(7) var<storage, read> order_in: array<u32>;
@group(2) @binding(0) var<uniform> sort_step: SortStep;

/// Last in the sort, and not drawn.
const SPLAT_NONE: f32 = 3.0e38;

/// The cloud slot a shape mode reads, or -1 for a procedural shape.
/// Follows `slot_normal`: of a morph pair, the one contributing more.
fn splat_slot(b: Body) -> i32 {
    let mode = select(b.mode_a, b.mode_b, b.blend > 0.5);
    if (mode == 7u) {
        return i32(select(u32(u.cloud_a), u32(u.cloud_b), u.cloud_morph > 0.5));
    }
    if (mode >= 5u) {
        return i32(mode - 5u);
    }
    return -1;
}

/// The rotation a unit quaternion (w, x, y, z) makes.
fn quat_mat(q: vec4<f32>) -> mat3x3<f32> {
    let w = q.x;
    let x = q.y;
    let y = q.z;
    let z = q.w;
    return mat3x3<f32>(
        vec3<f32>(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)),
        vec3<f32>(2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)),
        vec3<f32>(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)),
    );
}

fn splat_skip(i: u32) {
    keys[i] = SPLAT_NONE;
    order[i] = i;
    var p: Projected;
    p.alpha = 0.0;
    projected[i] = p;
}

/// Each particle's splat, on screen: the EWA splatting of Zwicker,
/// Pfister, van Baar & Gross, "EWA Volume Splatting", IEEE Visualization
/// 2001, as 3D Gaussian Splatting (Kerbl et al., SIGGRAPH 2023) uses it.
/// The ellipsoid's covariance is carried through the local linear part of
/// the projection, its Jacobian, to a 2D Gaussian on screen, widened by
/// 0.3 px² so a splat smaller than a pixel still covers one.
@compute @workgroup_size(64)
fn cs_splat_project(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= sp.padded) {
        return;
    }
    if (i >= sp.count) {
        splat_skip(i);
        return;
    }
    let b = body(i);
    let c = u.view_proj * vec4<f32>(b.p, 1.0);
    if (c.w < 0.02) {
        splat_skip(i);
        return;
    }
    // The ellipsoid: a capture's own, carried through the turn and scale
    // the shape was given, or a round one of the particle size.
    var cov = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0));
    var opacity = 1.0;
    var color = body_albedo(b, c.w);
    let slot = splat_slot(b);
    var is_round = true;
    if (slot >= 0) {
        let idx = u32(b.h1 * f32(ATTRACTOR_POINTS)) % ATTRACTOR_POINTS;
        let texel = vec2<u32>(
            idx % ATTRACTOR_W,
            u32(slot) * (ATTRACTOR_POINTS / ATTRACTOR_W) + idx / ATTRACTOR_W,
        );
        let shape = textureLoad(t_splat_shape, texel, 0);
        // A repeat filling a slot bigger than the capture, or a second
        // particle on the same texel: drawing either doubles a splat.
        let spaced = splat_spaced(splat_texels(b.mode_a, b.mode_b, b.blend));
        if (shape.w < 0.0 || (shape.w > 0.0 && f32(i) >= spaced)) {
            splat_skip(i);
            return;
        }
        if (shape.w > 0.0) {
            is_round = false;
            opacity = shape.w;
            // A capture keeps the colours it was trained to: the palette
            // tinting it would undo the photograph.
            color = cloud_color(cloud_texel(u32(slot), b.h1, u.time).w) * u.brightness;
            let ct = cos(b.turn);
            let st = sin(b.turn);
            // The same turn about y as `body_at` gives the position.
            let turn = mat3x3<f32>(vec3<f32>(ct, 0.0, st), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(-st, 0.0, ct));
            let r = turn * quat_mat(textureLoad(t_splat_turn, texel, 0));
            let m = mat3x3<f32>(r[0] * shape.x, r[1] * shape.y, r[2] * shape.z) * b.grow;
            cov = m * transpose(m);
        }
    }
    if (is_round) {
        let sigma = u.size * 0.5 * b.scale;
        cov = cov * (sigma * sigma);
    }
    // d(pixel)/d(world) at the centre: the projection's Jacobian.
    let half_px = vec2<f32>(u.viewport_h * u.aspect, u.viewport_h) * 0.5;
    let vp = u.view_proj;
    let row3 = vec3<f32>(vp[0][3], vp[1][3], vp[2][3]);
    let jx = (vec3<f32>(vp[0][0], vp[1][0], vp[2][0]) * c.w - row3 * c.x) / (c.w * c.w) * half_px.x;
    let jy = (vec3<f32>(vp[0][1], vp[1][1], vp[2][1]) * c.w - row3 * c.y) / (c.w * c.w) * half_px.y;
    let a = dot(jx, cov * jx) + 0.3;
    let bb = dot(jx, cov * jy);
    let cc = dot(jy, cov * jy) + 0.3;
    let det = a * cc - bb * bb;
    if (!(det > 1e-12)) {
        splat_skip(i);
        return;
    }
    let mid = 0.5 * (a + cc);
    let big = mid + sqrt(max(mid * mid - det, 0.0));
    var p: Projected;
    p.centre = c.xy / c.w;
    // Three standard deviations, and no more than half the frame: a splat
    // the camera is inside costs a frame's worth of fill to draw.
    p.radius = min(3.0 * sqrt(big), u.viewport_h * 0.5);
    p.alpha = clamp(opacity * u.splat.x, 0.0, 1.0);
    p.conic = vec3<f32>(cc, -bb, a) / det;
    p.color = color;
    projected[i] = p;
    // Farthest first.
    keys[i] = -c.w;
    order[i] = i;
}

/// One step of a bitonic sort over (key, index) pairs (Batcher, "Sorting
/// networks and their applications", AFIPS 1968): every element compares
/// with its partner `j` away and the pair is put in the order the block
/// of `k` it is in wants.
@compute @workgroup_size(64)
fn cs_splat_sort(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    let l = i ^ sort_step.j;
    if (i >= sp.padded || l <= i) {
        return;
    }
    let up = (i & sort_step.k) == 0u;
    let ki = keys[i];
    let kl = keys[l];
    if ((ki > kl) == up && ki != kl) {
        keys[i] = kl;
        keys[l] = ki;
        let oi = order[i];
        order[i] = order[l];
        order[l] = oi;
    }
}

struct SplatOut {
    @builtin(position) pos: vec4<f32>,
    // Pixels from the centre, y up.
    @location(0) offset: vec2<f32>,
    @location(1) conic: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) alpha: f32,
};

@vertex
fn vs_splat(@builtin(vertex_index) vi: u32) -> SplatOut {
    let p = projected_in[order_in[vi / 6u]];
    var out: SplatOut;
    if (p.alpha <= 0.0) {
        out.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        return out;
    }
    var xs = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    var ys = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    let corner = vec2<f32>(xs[vi % 6u], ys[vi % 6u]) * p.radius;
    let half_px = vec2<f32>(u.viewport_h * u.aspect, u.viewport_h) * 0.5;
    out.pos = vec4<f32>(p.centre + corner / half_px, 0.5, 1.0);
    out.offset = corner;
    out.conic = p.conic;
    out.color = p.color;
    out.alpha = p.alpha;
    return out;
}

@fragment
fn fs_splat(in: SplatOut) -> @location(0) vec4<f32> {
    let d = in.offset;
    let power = -0.5 * (in.conic.x * d.x * d.x + in.conic.z * d.y * d.y) - in.conic.y * d.x * d.y;
    let a = min(0.99, in.alpha * exp(min(power, 0.0)));
    if (a < 1.0 / 255.0) {
        discard;
    }
    // Premultiplied, for "over".
    return vec4<f32>(in.color * a, a);
}
