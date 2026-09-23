// The surface draw mode: the cloud as solid, lit, shadowed matter.
//
// Appended to particles.wgsl at build time (see surface.rs), so every
// particle is placed, coloured and oriented by the same functions the
// additive pass uses. What changes is what a particle *is*. In the additive
// pass it is light: sprites sum, nothing hides anything, and density reads
// as brightness. Here it is a surface element — a surfel, after Pfister et
// al., "Surfels: Surface Elements as Rendering Primitives", SIGGRAPH 2000 —
// an opaque disc with a depth, a normal and a colour that light falls on.
// Nearer surfels hide farther ones, lamps light the side facing them, and
// the sun casts the cloud's shadow onto itself and onto the room.
//
// Four passes a frame:
//   1. cs_eval: evaluate every particle once into a storage buffer. The
//      passes below read it rather than re-deriving each particle six
//      vertices at a time, twice over.
//   2. vs_shadow/fs_shadow: the surfels from the sun, into a depth map
//      (Williams, "Casting curved shadows on curved surfaces", SIGGRAPH
//      1978).
//   3. vs_surface/fs_surface and vs_walls/fs_walls: the surfels, and the
//      room as solid walls, floor and ceiling, from the camera into a
//      depth buffer and two colour targets — albedo, and the normal where
//      one is known.
//   4. vs_shade/fs_shade: light every covered pixel once, from what those
//      targets hold. Deferred, because the normal a cloud of surfels needs
//      is not any one surfel's: it is the orientation of the surface they
//      make together, and only the depth buffer knows that.

/// One evaluated particle. std430: 48 bytes, must match `SPLAT_BYTES` in
/// surface.rs.
struct Splat {
    pos: vec3<f32>,
    // World-space half-size, before any footprint floor.
    half: f32,
    // Albedo: palette, cloud colour and brightness, before light.
    color: vec3<f32>,
    _pad0: f32,
    // The cloud's own normal, not yet flipped towards anybody, or zero.
    normal: vec3<f32>,
    _pad1: f32,
};

/// Must match `SurfaceUniforms` in surface.rs.
struct Surface {
    // World to the sun's shadow map: orthographic, depth 0..1.
    sun_view_proj: mat4x4<f32>,
    // Clip space back to world, for rebuilding a pixel's position from
    // its depth.
    inv_view_proj: mat4x4<f32>,
    // The sun's own billboard basis, for drawing surfels into its map.
    sun_right: vec3<f32>,
    // 1 when the shadow map was drawn this frame.
    shadow_on: f32,
    sun_up: vec3<f32>,
    // World units per shadow-map texel.
    shadow_texel: f32,
    room_brightness: f32,
    room_fade: f32,
    // World units the shadow map's depth range spans.
    shadow_depth: f32,
    _pad0: f32,
    count: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};

@group(1) @binding(0) var<uniform> s: Surface;
@group(1) @binding(1) var<storage, read> splats: array<Splat>;
@group(1) @binding(2) var<storage, read_write> splats_out: array<Splat>;
@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_cmp: sampler_comparison;
@group(3) @binding(0) var g_albedo: texture_2d<f32>;
@group(3) @binding(1) var g_normal: texture_2d<f32>;
@group(3) @binding(2) var g_depth: texture_depth_2d;

/// Smallest surfel half-size in target pixels. Smaller than the additive
/// floor, because an opaque surfel cannot be dimmed by the area it gains —
/// widening it only thickens the surface — so this is just enough to stop
/// a surfel falling between pixel centres and flickering.
const SURFEL_MIN_PX: f32 = 0.75;

@compute @workgroup_size(64)
fn cs_eval(@builtin(global_invocation_id) id: vec3<u32>) {
    let pi = id.x;
    if (pi >= s.count) {
        return;
    }
    let b = body(pi);
    let centre = u.view_proj * vec4<f32>(b.p, 1.0);
    var out: Splat;
    out.pos = b.p;
    out.half = u.size * b.scale;
    out.color = body_albedo(b, centre.w);
    out.normal = body_normal(b);
    out._pad0 = 0.0;
    out._pad1 = 0.0;
    splats_out[pi] = out;
}

fn quad_corner(vi: u32) -> vec2<f32> {
    var offsets = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    return offsets[vi % 6u];
}

// --- Shadow map ---------------------------------------------------------

struct ShadowOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_shadow(@builtin(vertex_index) vi: u32) -> ShadowOut {
    let sp = splats[vi / 6u];
    let off = quad_corner(vi);
    // Face the sun, and never smaller than a texel: a surfel that falls
    // between texels casts no shadow at all, and a cloud of them casts a
    // shadow full of holes that crawl as it turns.
    let half = max(sp.half, s.shadow_texel);
    let corner = sp.pos + (s.sun_right * off.x + s.sun_up * off.y) * half;
    var out: ShadowOut;
    out.pos = s.sun_view_proj * vec4<f32>(corner, 1.0);
    out.uv = off;
    return out;
}

@fragment
fn fs_shadow(in: ShadowOut) {
    // Round, like the surfel it stands for.
    if (dot(in.uv, in.uv) > 1.0) {
        discard;
    }
}

/// How much of the sun reaches `pos`, 0..1.
///
/// 3×3 percentage-closer filtering (Reeves, Salesin & Cook, "Rendering
/// antialiased shadows with depth maps", SIGGRAPH 1987): nine depth
/// comparisons averaged, so the edge is a ramp a texel or two wide rather
/// than a staircase. The lookup point is pushed out along the normal by a
/// texel and a half first, which is what keeps a lit surface from
/// shadowing itself in speckles ("shadow acne") without a depth bias
/// large enough to detach the shadow from what casts it.
fn sun_light(pos: vec3<f32>, n: vec3<f32>) -> f32 {
    if (s.shadow_on < 0.5) {
        return 1.0;
    }
    let q = s.sun_view_proj * vec4<f32>(pos + n * s.shadow_texel * 1.5, 1.0);
    let uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5);
    // Outside the map nothing was drawn to cast a shadow.
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || q.z > 1.0) {
        return 1.0;
    }
    let bias = 2.0 * s.shadow_texel / max(s.shadow_depth, 1e-4);
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * texel;
            lit += textureSampleCompareLevel(shadow_map, shadow_cmp, uv + o, q.z - bias);
        }
    }
    return lit / 9.0;
}

/// Light arriving at a surface at `pos` facing `n` (unit, towards the
/// viewer), as a multiplier on its albedo.
///
/// The same lamps and sun as `light_at` in the additive pass, with two
/// differences that only make sense for a surface. The sun is shadowed.
/// And the ambient term is a sky rather than a flat fill: full strength on
/// a surface facing up, 0.3 facing down, the simplest hemisphere light
/// there is. A flat ambient on an opaque surface paints every surfel one
/// colour and the cloud reads as a cut-out; the hemisphere is what gives
/// it a top and a bottom with every lamp off.
fn surface_light(pos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var acc = vec3<f32>(u.light.x * (0.65 + 0.35 * n.y));
    // How much orientation counts. Always known here — every surfel has
    // at least its sphere's normal — so this is the control, undiluted.
    let shape = u.light.y;
    for (var i = 0u; i < 2u; i = i + 1u) {
        let level = u.lamp[i].w;
        if (level <= 0.001) {
            continue;
        }
        let d = pos - u.lamp[i].xyz;
        let r = max(u.lamp_tint[i].w, 0.01);
        let d2 = dot(d, d);
        let falloff = (r * r) / (d2 + r * r);
        let toward = -d / sqrt(max(d2, 1e-6));
        let ndotl = max(dot(n, toward), 0.0);
        acc += u.lamp_tint[i].rgb * (level * falloff * mix(1.0, ndotl, shape));
    }
    let sun = u.sun_dir.w;
    if (sun > 0.001) {
        let ndotl = max(dot(n, u.sun_dir.xyz), 0.0);
        if (ndotl > 0.0) {
            acc += u.sun_tint.rgb * (sun * ndotl * sun_light(pos, n));
        }
    }
    return acc;
}

// --- Surfels and walls into the G-buffer ---------------------------------

/// What the surfel and wall passes leave for the shading pass: the colour
/// light will fall on in rgb and how much of the room's own line colour the
/// surface gives off in a, and a normal in xyz with how far to trust it in
/// w — 0 where the surface's orientation is left to the depth buffer.
struct GOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
};

fn pack_normal(n: vec3<f32>, trust: f32) -> vec4<f32> {
    return vec4<f32>(n * 0.5 + 0.5, trust);
}

struct SurfelOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
};

@vertex
fn vs_surface(@builtin(vertex_index) vi: u32) -> SurfelOut {
    let sp = splats[vi / 6u];
    let off = quad_corner(vi);
    var out: SurfelOut;
    let centre = u.view_proj * vec4<f32>(sp.pos, 1.0);
    if (centre.w < 0.02) {
        out.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.color = vec3<f32>(0.0);
        out.normal = vec3<f32>(0.0);
        return out;
    }
    var half = sp.half;
    if (u.viewport_h > 0.0) {
        half = half * footprint_grow(sp.pos, half, centre, SURFEL_MIN_PX);
    }
    let corner = sp.pos + (u.cam_right * off.x + u.cam_up * off.y) * half;
    out.pos = u.view_proj * vec4<f32>(corner, 1.0);
    out.uv = off;
    out.color = sp.color;
    // Two-sided, as in the additive pass: facing the eye.
    let to_eye = u.cam_position - sp.pos;
    out.normal = sp.normal * select(-1.0, 1.0, dot(sp.normal, to_eye) >= 0.0);
    return out;
}

@fragment
fn fs_surface(in: SurfelOut) -> GOut {
    if (dot(in.uv, in.uv) > 1.0) {
        discard;
    }
    var out: GOut;
    out.albedo = vec4<f32>(in.color, 0.0);
    // Where the cloud measured its own surface — a scan — that normal is
    // mostly what it lights by, so a wall in a scan lights as a wall. A
    // fifth is left to the depth buffer, which keeps the grain.
    let known = dot(in.normal, in.normal) > 0.25;
    out.normal = select(vec4<f32>(0.5, 0.5, 0.5, 0.0), pack_normal(normalize(in.normal), 0.8), known);
    return out;
}

// --- The room as solid surfaces -----------------------------------------

/// The room's cross-section at depth `t`, as `section` in room.wgsl:
/// half-extents in xy, centre offset in zw. Built from the placement the
/// particles already carry, so the walls are exactly where the lines are.
fn room_section(t: f32) -> vec4<f32> {
    let scale = mix(1.0, u.room.converge, t);
    return vec4<f32>(
        u.room.half_x * scale,
        u.room.half_y * scale,
        u.room.vanish_x * u.room.half_x * t,
        u.room.vanish_y * u.room.half_y * t,
    );
}

/// A point on face `face` — floor, ceiling, left, right, back — at `a`
/// across it (-1..1) and `t` along it (0..1: into the room, or up the back
/// wall).
fn wall_point(face: u32, a: f32, t: f32) -> vec3<f32> {
    let z = u.room.front_z - u.room.depth * t;
    let m = room_section(t);
    switch face {
        case 0u: { return vec3<f32>(m.z + a * m.x, m.w - m.y, z); }
        case 1u: { return vec3<f32>(m.z + a * m.x, m.w + m.y, z); }
        case 2u: { return vec3<f32>(m.z - m.x, m.w + a * m.y, z); }
        case 3u: { return vec3<f32>(m.z + m.x, m.w + a * m.y, z); }
        default: {
            let b = room_section(1.0);
            return vec3<f32>(b.z + a * b.x, b.w - b.y + 2.0 * t * b.y, u.room.front_z - u.room.depth);
        }
    }
}

struct WallOut {
    @builtin(position) pos: vec4<f32>,
    // Position on the face: x across it -1..1, y along it 0..1.
    @location(0) face_uv: vec2<f32>,
    @location(1) @interpolate(flat) normal: vec3<f32>,
    @location(2) @interpolate(flat) face: u32,
};

@vertex
fn vs_walls(@builtin(vertex_index) vi: u32) -> WallOut {
    let face = vi / 6u;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi % 6u];
    let p = wall_point(face, c.x, c.y);
    // The face's normal from three of its corners — it is planar, however
    // the room is skewed — turned to face into the room: towards the
    // room's middle.
    let p0 = wall_point(face, -1.0, 0.0);
    let p1 = wall_point(face, 1.0, 0.0);
    let p2 = wall_point(face, -1.0, 1.0);
    var n = normalize(cross(p1 - p0, p2 - p0));
    let mid = room_section(0.5);
    let inside = vec3<f32>(mid.z, mid.w, u.room.front_z - 0.5 * u.room.depth);
    n = n * select(-1.0, 1.0, dot(n, inside - p0) >= 0.0);
    var out: WallOut;
    out.pos = u.view_proj * vec4<f32>(p, 1.0);
    out.face_uv = c;
    out.normal = n;
    out.face = face;
    return out;
}

/// Lines on a face, 0..1: the room's own grid, drawn into the surface.
/// Ten a side like the wireframe, a pixel wide at any distance (fwidth),
/// and anti-aliased over that pixel.
fn grid_lines(uv: vec2<f32>) -> f32 {
    let g = vec2<f32>(uv.x * 0.5 + 0.5, uv.y) * 9.0;
    let w = max(fwidth(g), vec2<f32>(1e-4));
    let d = abs(fract(g - 0.5) - 0.5) / w;
    return 1.0 - min(min(d.x, d.y), 1.0);
}

@fragment
fn fs_walls(in: WallOut) -> GOut {
    // A dark, cool plaster: light enough to catch a coloured lamp, dim
    // enough that the cloud stays the subject. The wireframe's grid is
    // kept, glowing in the plaster in its own blue whatever light falls
    // on it, so the room reads as the same room in either mode. It still
    // fades towards the back as `/room/fade` says.
    let depth_t = select(in.face_uv.y, 1.0, in.face == 4u);
    let shade = mix(1.0, 1.0 - s.room_fade, depth_t);
    let plaster = vec3<f32>(0.15, 0.16, 0.18) * s.room_brightness * shade;
    let glow = grid_lines(in.face_uv) * s.room_brightness * shade;
    var out: GOut;
    out.albedo = vec4<f32>(plaster, glow);
    out.normal = pack_normal(in.normal, 1.0);
    return out;
}

// --- Lighting --------------------------------------------------------------

@vertex
fn vs_shade(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle over the whole target.
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

/// Where the surface at pixel `px`, depth `d`, is in the world.
fn world_at(px: vec2<i32>, d: f32) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(g_depth));
    let uv = (vec2<f32>(px) + 0.5) / size;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let w = s.inv_view_proj * ndc;
    return w.xyz / w.w;
}

/// The step from `c` to its neighbour `k` pixels along `dir`, taken on
/// whichever side stays on the same surface — the nearer one in space —
/// so a normal at an edge is not bent by whatever lies behind it. Zero
/// when neither side has a surface.
fn tangent(px: vec2<i32>, c: vec3<f32>, dir: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(g_depth));
    let a = clamp(px + dir, vec2<i32>(0), size - 1);
    let b = clamp(px - dir, vec2<i32>(0), size - 1);
    let da = textureLoad(g_depth, a, 0);
    let db = textureLoad(g_depth, b, 0);
    let va = da < 1.0;
    let vb = db < 1.0;
    let fa = world_at(a, da) - c;
    let fb = c - world_at(b, db);
    if (va && (!vb || dot(fa, fa) <= dot(fb, fb))) {
        return fa;
    }
    if (vb) {
        return fb;
    }
    return vec3<f32>(0.0);
}

/// The surface's orientation from the depth buffer, from the steps to
/// neighbours along `a` and `b` — two directions across the screen.
fn depth_normal(px: vec2<i32>, c: vec3<f32>, a: vec2<i32>, b: vec2<i32>, to_eye: vec3<f32>) -> vec3<f32> {
    let n = cross(tangent(px, c, a), tangent(px, c, b));
    if (dot(n, n) < 1e-20) {
        return vec3<f32>(0.0);
    }
    let nn = normalize(n);
    return nn * select(-1.0, 1.0, dot(nn, to_eye) >= 0.0);
}

@fragment
fn fs_shade(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(frag.xy);
    let d = textureLoad(g_depth, px, 0);
    if (d >= 1.0) {
        discard;
    }
    let g = textureLoad(g_albedo, px, 0);
    let albedo = g.rgb;
    let known = textureLoad(g_normal, px, 0);
    let pos = world_at(px, d);
    let to_eye = normalize(u.cam_position - pos);
    // A normal from the depth buffer. One surfel is a flat disc facing
    // the camera; the surface is the envelope of hundreds of them, and its
    // orientation only shows across several. So the depth is read a few
    // surfels away on each side — the span follows the surfel's size on
    // screen, so a 2× render or a closer camera sees the same surface —
    // along both axes and both diagonals, at two spans, and the normals
    // averaged: the short span keeps the form's detail, the long one
    // steadies the grain.
    let c = u.view_proj * vec4<f32>(pos, 1.0);
    let e = u.view_proj * vec4<f32>(pos + u.cam_up * u.size, 1.0);
    let surfel_px = abs(e.y / e.w - c.y / c.w) * u.viewport_h * 0.5;
    let k = i32(clamp(round(4.0 * surfel_px), 2.0, 24.0));
    let k3 = 3 * k;
    let sum = depth_normal(px, pos, vec2<i32>(k, 0), vec2<i32>(0, k), to_eye)
        + depth_normal(px, pos, vec2<i32>(k, k), vec2<i32>(k, -k), to_eye)
        + depth_normal(px, pos, vec2<i32>(k3, 0), vec2<i32>(0, k3), to_eye)
        + depth_normal(px, pos, vec2<i32>(k3, k3), vec2<i32>(k3, -k3), to_eye);
    var n = select(to_eye, normalize(sum), dot(sum, sum) > 1e-8);
    if (known.w > 0.0) {
        n = normalize(mix(n, known.xyz * 2.0 - 1.0, known.w));
    }
    // The room's lines, in the wireframe's colour: given off, not lit.
    let lines = vec3<f32>(0.28, 0.38, 0.58) * g.a;
    return vec4<f32>(albedo * surface_light(pos, n) + lines, 1.0);
}
