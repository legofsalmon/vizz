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

/// One evaluated particle. std430: 64 bytes, must match `SPLAT_BYTES` in
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
    // Which way the particle is travelling, unit, for orienting a glyph;
    // a fixed direction of its own when it is not moving.
    axis: vec3<f32>,
    // The glyph's roll about that axis, in radians.
    roll: f32,
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
    // The sphere-traced solid's one knob: power, scale or depth by kind.
    solid_param: f32,
    count: u32,
    // 0 none, 1 Mandelbulb, 2 Mandelbox, 3 quaternion Julia, 4 Menger.
    solid_kind: u32,
    // 0 off, 1 outlines, 2 hatching, 3 stipple; and the pen's weight.
    ink: u32,
    ink_weight: f32,
    // 0..1: the surfels smoothed into one liquid surface.
    liquid: f32,
    _pad3: f32,
    _pad4: f32,
    _pad5: f32,
};

@group(1) @binding(0) var<uniform> s: Surface;
@group(1) @binding(1) var<storage, read> splats: array<Splat>;
@group(1) @binding(2) var<storage, read_write> splats_out: array<Splat>;
@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_cmp: sampler_comparison;
@group(3) @binding(0) var g_albedo: texture_2d<f32>;
@group(3) @binding(1) var g_normal: texture_2d<f32>;
@group(3) @binding(2) var g_depth: texture_depth_2d;
// The liquid's smoothed depth, read in place of `g_depth` when it is on.
@group(3) @binding(3) var g_smooth: texture_2d<f32>;
// The liquid pass's own inputs, at bindings the G-buffer does not use.
@group(3) @binding(5) var l_depth: texture_depth_2d;
@group(3) @binding(7) var l_normal: texture_2d<f32>;

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
    // Where the particle was a moment ago, for the way it is heading. Only
    // paid for when a glyph will use it.
    let h = vec3<f32>(hash01(pi, 1u), hash01(pi, 2u), hash01(pi, 3u)) * 2.0 - 1.0;
    var axis = normalize(h + vec3<f32>(1e-3, 0.0, 0.0));
    if (glyph_kind() > 0u) {
        let was = body_at(pi, u.time - GLYPH_LOOKBACK, 0.0).p;
        let d = b.p - was;
        if (dot(d, d) > 1e-12) {
            axis = normalize(d);
        }
    }
    out.axis = axis;
    // A roll of its own, turning slowly, so a still field of glyphs
    // glints instead of sitting like a printed pattern.
    out.roll = hash01(pi, 0u) * TAU + u.time * (0.3 + hash01(pi, 2u));
    splats_out[pi] = out;
}

// --- Glyphs -------------------------------------------------------------
//
// A surfel is a disc facing the eye, which is right for a surface sampled
// by points and wrong for a cloud of *things*. A glyph replaces the disc
// with a small solid — a tetrahedron, a cube, an octahedron or a long
// shard — turned to the way the particle travels and lit by its own
// faces, so a field of them reads as confetti, scales or debris. The
// glyph-based visualisation literature is the source: Borgo et al.,
// "Glyph-based Visualization: Foundations, Design Guidelines, Techniques
// and Applications", Eurographics State of the Art Reports, 2013.

/// Visual time to look back for a particle's heading.
const GLYPH_LOOKBACK: f32 = 0.05;
/// Vertices drawn per particle when glyphs are on: the largest mesh,
/// the cube's twelve triangles. Smaller meshes leave the rest degenerate.
const GLYPH_VERTS_MAX: u32 = 36u;

/// 0 for discs; 1 tetrahedron, 2 cube, 3 octahedron, 4 shard, 5 a mix.
fn glyph_kind() -> u32 {
    return u32(u.stroke.w + 0.5);
}

/// The four meshes as flat triangle lists, unit circumradius, centred on
/// the origin; the shard is a triangular bipyramid stretched along x.
/// Generated as the convex hulls of their vertices.
const GLYPH_VERTS = array<vec3<f32>, 90>(
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(-0.577350, 0.577350, 0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, 0.577350),
    vec3<f32>(-0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(-0.577350, -0.577350, -0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, -0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, -0.577350),
    vec3<f32>(0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, -0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, 0.577350),
    vec3<f32>(-0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, 0.577350),
    vec3<f32>(-0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(-0.577350, 0.577350, -0.577350),
    vec3<f32>(-0.577350, 0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, -0.577350, -0.577350),
    vec3<f32>(0.577350, 0.577350, -0.577350),
    vec3<f32>(0.577350, -0.577350, 0.577350),
    vec3<f32>(0.577350, 0.577350, -0.577350),
    vec3<f32>(0.577350, 0.577350, 0.577350),
    vec3<f32>(1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, 1.000000),
    vec3<f32>(1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, -1.000000),
    vec3<f32>(1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, 1.000000),
    vec3<f32>(1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, -1.000000),
    vec3<f32>(-1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, 1.000000),
    vec3<f32>(-1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, -1.000000),
    vec3<f32>(-1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, 1.000000),
    vec3<f32>(-1.000000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -1.000000, 0.000000),
    vec3<f32>(0.000000, 0.000000, -1.000000),
    vec3<f32>(1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 0.450000, 0.000000),
    vec3<f32>(0.000000, -0.225000, 0.389711),
    vec3<f32>(1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 0.450000, 0.000000),
    vec3<f32>(0.000000, -0.225000, -0.389711),
    vec3<f32>(1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -0.225000, 0.389711),
    vec3<f32>(0.000000, -0.225000, -0.389711),
    vec3<f32>(-1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 0.450000, 0.000000),
    vec3<f32>(0.000000, -0.225000, 0.389711),
    vec3<f32>(-1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, 0.450000, 0.000000),
    vec3<f32>(0.000000, -0.225000, -0.389711),
    vec3<f32>(-1.600000, 0.000000, 0.000000),
    vec3<f32>(0.000000, -0.225000, 0.389711),
    vec3<f32>(0.000000, -0.225000, -0.389711),
);

/// (first vertex, vertex count) of each mesh in `GLYPH_VERTS`.
fn glyph_range(kind: u32) -> vec2<u32> {
    switch kind {
        case 1u: { return vec2<u32>(0u, 12u); }
        case 2u: { return vec2<u32>(12u, 36u); }
        case 3u: { return vec2<u32>(48u, 24u); }
        default: { return vec2<u32>(72u, 18u); }
    }
}

struct GlyphVertex {
    pos: vec3<f32>,
    normal: vec3<f32>,
    // False for the spare vertices past the end of a smaller mesh.
    valid: bool,
};

/// Vertex `vi` of particle `sp`'s glyph, in world space, and the normal
/// of the face it belongs to. `pi` picks the mesh when the kind is a mix.
fn glyph_vertex(sp: Splat, pi: u32, vi: u32, half: f32) -> GlyphVertex {
    var kind = glyph_kind();
    if (kind >= 5u) {
        kind = 1u + hash_u32(pi * 4u + 3u) % 4u;
    }
    let range = glyph_range(kind);
    var out: GlyphVertex;
    out.valid = vi < range.y;
    let tri = min(vi, range.y - 1u) / 3u;
    let base = range.x + tri * 3u;
    let a = GLYPH_VERTS[base];
    let b = GLYPH_VERTS[base + 1u];
    let c = GLYPH_VERTS[base + 2u];
    let local = GLYPH_VERTS[range.x + min(vi, range.y - 1u)];
    var n = normalize(cross(b - a, c - a));
    // Outward: every mesh is convex and centred, so the face's centroid
    // says which side is out whatever order the triangle was listed in.
    n = n * select(-1.0, 1.0, dot(n, a + b + c) >= 0.0);

    // A frame with x along the heading, rolled about it.
    let x = sp.axis;
    var helper = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(x.y) > 0.9) {
        helper = vec3<f32>(1.0, 0.0, 0.0);
    }
    let y0 = normalize(cross(helper, x));
    let z0 = cross(x, y0);
    let cr = cos(sp.roll);
    let sr = sin(sp.roll);
    let y = y0 * cr + z0 * sr;
    let z = z0 * cr - y0 * sr;
    out.pos = sp.pos + (x * local.x + y * local.y + z * local.z) * half;
    out.normal = x * n.x + y * n.y + z * n.z;
    return out;
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
    if (glyph_kind() > 0u) {
        let pi = vi / GLYPH_VERTS_MAX;
        let sp = splats[pi];
        let g = glyph_vertex(sp, pi, vi % GLYPH_VERTS_MAX, max(sp.half, s.shadow_texel));
        var out: ShadowOut;
        out.pos = select(vec4<f32>(4.0, 4.0, 2.0, 1.0), s.sun_view_proj * vec4<f32>(g.pos, 1.0), g.valid);
        out.uv = vec2<f32>(0.0);
        return out;
    }
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
    // 1 on a glyph's face, whose normal is exact; 0 on a disc.
    @location(3) @interpolate(flat) facet: f32,
};

@vertex
fn vs_surface(@builtin(vertex_index) vi: u32) -> SurfelOut {
    if (glyph_kind() > 0u) {
        return glyph_surfel(vi);
    }
    let sp = splats[vi / 6u];
    let off = quad_corner(vi);
    var out: SurfelOut;
    let centre = u.view_proj * vec4<f32>(sp.pos, 1.0);
    if (centre.w < 0.02) {
        out.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.color = vec3<f32>(0.0);
        out.normal = vec3<f32>(0.0);
        out.facet = 0.0;
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
    out.facet = 0.0;
    return out;
}

/// A glyph's vertex for the G-buffer. Its normal is its face's and is
/// trusted fully, so the facets light as facets rather than being smoothed
/// into the envelope the depth buffer sees.
fn glyph_surfel(vi: u32) -> SurfelOut {
    let pi = vi / GLYPH_VERTS_MAX;
    let sp = splats[pi];
    var out: SurfelOut;
    let centre = u.view_proj * vec4<f32>(sp.pos, 1.0);
    if (centre.w < 0.02) {
        out.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.color = vec3<f32>(0.0);
        out.normal = vec3<f32>(0.0);
        out.facet = 0.0;
        return out;
    }
    var half = sp.half;
    if (u.viewport_h > 0.0) {
        half = half * footprint_grow(sp.pos, half, centre, SURFEL_MIN_PX);
    }
    let g = glyph_vertex(sp, pi, vi % GLYPH_VERTS_MAX, half);
    out.pos = select(vec4<f32>(4.0, 4.0, 2.0, 1.0), u.view_proj * vec4<f32>(g.pos, 1.0), g.valid);
    out.uv = vec2<f32>(0.0);
    out.color = sp.color;
    out.normal = g.normal;
    out.facet = 1.0;
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
    let trust = select(0.8, 1.0, in.facet > 0.5);
    out.normal = select(vec4<f32>(0.5, 0.5, 0.5, 0.0), pack_normal(normalize(in.normal), trust), known);
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

// --- Sphere-traced solids --------------------------------------------------
//
// A fractal drawn as the surface it is rather than as points sampled near
// it: every pixel marches a ray in steps as long as a distance estimate
// says is safe — sphere tracing, Hart, "Sphere tracing: a geometric method
// for the antialiased ray tracing of implicit surfaces", The Visual
// Computer 12 (1996) — and writes what it hits into the same G-buffer the
// surfels do, so the solid takes the same lamps, sun, sky and shadows,
// and the cloud and the solid hide each other where they cross.
//
// The estimates: the Mandelbulb's is White & Nylander's (2009) running
// derivative; the Mandelbox's Tom Lowe's (2010) folds with the scale
// tracked through them; the quaternion Julia set's Hart, Sandin &
// Kauffman's "Ray tracing deterministic 3-D fractals" (SIGGRAPH 1989);
// and the Menger sponge's Iñigo Quilez's folded box (2011).

/// How many steps a ray takes before giving up, and how many iterations
/// each estimate runs.
const SOLID_STEPS: u32 = 160u;
const SOLID_ITER: u32 = 10u;

struct Hit {
    d: f32,
    // A smooth record of the orbit, for colour.
    trap: f32,
};

fn de_bulb(p: vec3<f32>, power: f32) -> Hit {
    var z = p;
    var dr = 1.0;
    var r = 0.0;
    var trap = 1e9;
    for (var i = 0u; i < SOLID_ITER; i = i + 1u) {
        r = length(z);
        if (r > 2.0) {
            break;
        }
        trap = min(trap, r);
        let theta = acos(clamp(z.z / max(r, 1e-9), -1.0, 1.0)) * power;
        let phi = atan2(z.y, z.x) * power;
        dr = pow(r, power - 1.0) * power * dr + 1.0;
        let zr = pow(r, power);
        z = zr * vec3<f32>(sin(theta) * cos(phi), sin(phi) * sin(theta), cos(theta)) + p;
    }
    return Hit(0.5 * log(max(r, 1e-9)) * r / dr, trap);
}

fn de_box(p: vec3<f32>, scale: f32) -> Hit {
    var z = p;
    var dr = 1.0;
    var trap = 1e9;
    for (var i = 0u; i < SOLID_ITER + 2u; i = i + 1u) {
        z = clamp(z, vec3<f32>(-1.0), vec3<f32>(1.0)) * 2.0 - z;
        let r2 = dot(z, z);
        if (r2 < 0.25) {
            z = z * 4.0;
            dr = dr * 4.0;
        } else if (r2 < 1.0) {
            z = z / r2;
            dr = dr / r2;
        }
        z = z * scale + p;
        dr = dr * abs(scale) + 1.0;
        trap = min(trap, r2);
    }
    return Hit(length(z) / abs(dr), sqrt(trap));
}

fn qmul(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        a.x * b.x - dot(a.yzw, b.yzw),
        a.x * b.yzw + b.x * a.yzw + cross(a.yzw, b.yzw),
    );
}

fn de_julia(p: vec3<f32>, c: vec4<f32>) -> Hit {
    var z = vec4<f32>(p, 0.0);
    var dz = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    var trap = 1e9;
    for (var i = 0u; i < SOLID_ITER + 2u; i = i + 1u) {
        dz = 2.0 * qmul(z, dz);
        z = qmul(z, z) + c;
        let m = dot(z, z);
        trap = min(trap, m);
        if (m > 16.0) {
            break;
        }
    }
    let r = length(z);
    return Hit(0.5 * r * log(max(r, 1e-9)) / max(length(dz), 1e-9), sqrt(trap));
}

fn de_menger(p: vec3<f32>, depth: f32) -> Hit {
    let q = abs(p) - vec3<f32>(1.0);
    var d = length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
    var s = 1.0;
    var trap = 1.0;
    let levels = u32(clamp(depth, 1.0, 6.0));
    for (var i = 0u; i < levels; i = i + 1u) {
        // Each level's cell, centred so its middle is the hole.
        let ps = p * s;
        let a = ps - 2.0 * floor(ps * 0.5) - 1.0;
        s = s * 3.0;
        let r = abs(1.0 - 3.0 * abs(a));
        let da = max(r.x, r.y);
        let db = max(r.y, r.z);
        let dc = max(r.z, r.x);
        let c = (min(da, min(db, dc)) - 1.0) / s;
        if (c > d) {
            d = c;
            trap = f32(i) / f32(levels);
        }
    }
    return Hit(d, trap);
}

/// How big the solid's own space is, in its units, for each kind: the
/// world radius the cloud is spread to maps onto this.
fn solid_extent(kind: u32, param: f32) -> f32 {
    switch kind {
        case 1u: { return 1.05; }
        case 2u: {
            // A box of positive scale k fills ±2(k+1)/(k−1) (Lowe); the
            // negative ones stay inside ±2. Then the same margin as the
            // sponge, so both cubes come out the same size.
            if (param > 0.0) {
                let k = max(param, 1.5);
                return 1.5 * 2.0 * (k + 1.0) / (k - 1.0);
            }
            return 1.5 * 2.0;
        }
        case 3u: { return 1.1; }
        default: { return 1.5; }
    }
}

fn solid_de(kind: u32, param: f32, p: vec3<f32>) -> Hit {
    switch kind {
        case 1u: { return de_bulb(p, clamp(param, 2.0, 16.0)); }
        case 2u: { return de_box(p, param); }
        case 3u: {
            // The same constant as the quaternion generator, turned by
            // the knob so it can be played.
            let a = param * 0.5;
            return de_julia(p, vec4<f32>(-0.2, 0.6 * cos(a), 0.2, 0.6 * sin(a)));
        }
        default: { return de_menger(p, param); }
    }
}

/// The solid's frame: where the cloud is placed, how big, and the turn
/// the rigid shapes take, so the solid sits and spins where they do.
struct SolidFrame {
    centre: vec3<f32>,
    // World units per solid unit.
    scale: f32,
    spin: f32,
};

fn solid_frame() -> SolidFrame {
    let placed = room_place(vec3<f32>(0.0));
    var f: SolidFrame;
    f.centre = placed.xyz;
    f.scale = u.spread * placed.w / solid_extent(s.solid_kind, s.solid_param);
    f.spin = u.time * 0.55 * (0.4 + u.twist);
    return f;
}

fn to_solid(f: SolidFrame, w: vec3<f32>) -> vec3<f32> {
    let q = (w - f.centre) / f.scale;
    let c = cos(f.spin);
    let sn = sin(f.spin);
    // The inverse of the cloud's turn about y.
    return vec3<f32>(q.x * c + q.z * sn, q.y, -q.x * sn + q.z * c);
}

struct SolidOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@vertex
fn vs_solid(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_solid(@builtin(position) frag: vec4<f32>) -> SolidOut {
    let size = vec2<f32>(u.viewport_h * u.aspect, u.viewport_h);
    let uv = frag.xy / size;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // Any depth short of the far plane will do for a direction, and
    // the far plane itself may be at infinity.
    let far = s.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let origin = u.cam_position;
    let dir = normalize(far.xyz / far.w - origin);
    let f = solid_frame();
    // Only the bounding sphere is worth marching through.
    let oc = origin - f.centre;
    // Wide enough for a cube's corners, which the box and the sponge are.
    let reach = f.scale * solid_extent(s.solid_kind, s.solid_param) * 1.8;
    let b = dot(oc, dir);
    let disc = b * b - (dot(oc, oc) - reach * reach);
    if (disc < 0.0) {
        discard;
    }
    let root = sqrt(disc);
    var t = max(-b - root, 0.0);
    let t_end = -b + root;
    // A pixel's width at unit distance, so the surface is found to the
    // precision the screen can show and no finer.
    let pixel = 2.0 / (u.viewport_h * abs(u.view_proj[1][1]));
    var hit = false;
    var steps = 0u;
    var h: Hit;
    for (var i = 0u; i < SOLID_STEPS; i = i + 1u) {
        let w = origin + dir * t;
        h = solid_de(s.solid_kind, s.solid_param, to_solid(f, w));
        let d = h.d * f.scale;
        if (d < pixel * t * 0.75) {
            hit = true;
            steps = i;
            break;
        }
        t = t + d * 0.9;
        if (t > t_end) {
            break;
        }
    }
    if (!hit) {
        discard;
    }
    let w = origin + dir * t;
    let q = to_solid(f, w);
    // The normal from the estimate's gradient, by four samples on a
    // tetrahedron (Quilez), at the size the hit was found to.
    let e = max(pixel * t * 0.5 / f.scale, 1e-5);
    let k1 = vec3<f32>(1.0, -1.0, -1.0);
    let k2 = vec3<f32>(-1.0, -1.0, 1.0);
    let k3 = vec3<f32>(-1.0, 1.0, -1.0);
    let k4 = vec3<f32>(1.0, 1.0, 1.0);
    let gq = k1 * solid_de(s.solid_kind, s.solid_param, q + k1 * e).d
        + k2 * solid_de(s.solid_kind, s.solid_param, q + k2 * e).d
        + k3 * solid_de(s.solid_kind, s.solid_param, q + k3 * e).d
        + k4 * solid_de(s.solid_kind, s.solid_param, q + k4 * e).d;
    // Back out of the solid's turn into the world.
    let c = cos(f.spin);
    let sn = sin(f.spin);
    let gw = vec3<f32>(gq.x * c - gq.z * sn, gq.y, gq.x * sn + gq.z * c);
    let n = normalize(select(-dir, gw, dot(gw, gw) > 1e-20));
    // Steps taken is a cheap occlusion: a ray that had to creep in
    // through a crevice found it dark in there.
    let occlusion = 1.0 - 0.7 * f32(steps) / f32(SOLID_STEPS);
    let tone = h.trap * u.color_spread + 0.03 * sin(u.time * 0.2);
    let colour = palette_color(u.palette, tone, u.saturation, u.hue) * u.brightness * occlusion;
    var out: SolidOut;
    out.albedo = vec4<f32>(colour, 0.0);
    out.normal = pack_normal(n, 1.0);
    let clip = u.view_proj * vec4<f32>(w, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 0.9999999);
    return out;
}

// --- Liquid ----------------------------------------------------------------
//
// The surfels smoothed into one surface: screen-space fluid rendering,
// after van der Laan, Green and Sainz, "Screen Space Fluid Rendering with
// Curvature Flow", I3D 2009. The depth the surfels left is blurred as a
// distance along each pixel's ray, with a bilateral weight so the blur
// follows the surface and does not bleed across a gap in depth to
// whatever is behind it; the shading then reads its normals from the
// smoothed depth, and the bumps of the separate discs are gone. One pass
// on a sparse grid of taps rather than a separable pair: a bilateral
// weight does not separate, and a pair of them leaves streaks along the
// axes.
// Surfaces with exact normals — glyphs, a solid, the walls — are left as
// they are, and are not blurred into.

/// Which way pixel `px` of a `size` target looks.
fn liquid_ray(px: vec2<i32>, size: vec2<f32>) -> vec3<f32> {
    let uv = (vec2<f32>(px) + 0.5) / size;
    let w = s.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.5, 1.0);
    return normalize(w.xyz / w.w - u.cam_position);
}

/// The distance along the ray to the surface at `px`, or -1 for none or
/// for an exact surface, which the liquid leaves alone.
fn liquid_distance(px: vec2<i32>, size: vec2<f32>) -> f32 {
    let d = textureLoad(l_depth, px, 0);
    if (d >= 1.0 || textureLoad(l_normal, px, 0).w > 0.99) {
        return -1.0;
    }
    let uv = (vec2<f32>(px) + 0.5) / size;
    let w = s.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return length(w.xyz / w.w - u.cam_position);
}

/// Taps from the centre to the edge of the blur, each way.
const LIQUID_TAPS: i32 = 5;

@fragment
fn fs_liquid(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(l_depth));
    let isize = vec2<i32>(size);
    let px = vec2<i32>(frag.xy);
    let raw = textureLoad(l_depth, px, 0);
    let centre = liquid_distance(px, size);
    if (centre < 0.0) {
        return vec4<f32>(raw);
    }
    // A few surfels' width, so the bumps between them go but the form
    // does not; reached in a fixed number of taps, spaced to fit.
    let surfel_px = u.size / max(centre, 1e-3) * abs(u.view_proj[1][1]) * size.y * 0.5;
    let reach = clamp(s.liquid * 3.0 * surfel_px, 1.0, 40.0);
    let spacing = max(reach / f32(LIQUID_TAPS), 1.0);
    let sigma = reach * 0.5 + 0.5;
    // The depth range: a little more than a surfel, so neighbouring
    // discs merge and a surface behind does not.
    let range = 3.0 * u.size;
    var sum = 0.0;
    var weight = 0.0;
    for (var j = -LIQUID_TAPS; j <= LIQUID_TAPS; j = j + 1) {
        for (var i = -LIQUID_TAPS; i <= LIQUID_TAPS; i = i + 1) {
            let off = vec2<f32>(f32(i), f32(j)) * spacing;
            let q = clamp(px + vec2<i32>(round(off)), vec2<i32>(0), isize - 1);
            let d = liquid_distance(q, size);
            if (d < 0.0) {
                continue;
            }
            let x = length(off) / sigma;
            let r = (d - centre) / range;
            let w = exp(-0.5 * (x * x + r * r));
            sum += d * w;
            weight += w;
        }
    }
    // Back to a depth, so everything downstream reads the liquid as it
    // reads the surfels' own.
    let p = u.cam_position + liquid_ray(px, size) * (sum / max(weight, 1e-6));
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    return vec4<f32>(clamp(clip.z / clip.w, 0.0, 0.9999999));
}

/// The depth the shading reads: the surfels' own, or the liquid's.
fn scene_depth(p: vec2<i32>) -> f32 {
    if (s.liquid > 0.001) {
        return textureLoad(g_smooth, p, 0).r;
    }
    return textureLoad(g_depth, p, 0);
}

/// A liquid's look: what light gets in tinted by its colour, and on top
/// what its surface reflects — the sky and the highlights of the lamps
/// and the sun — more of it at a glancing angle (Schlick's Fresnel term,
/// with water's 2% head on).
fn liquid_shade(pos: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>, light: vec3<f32>) -> vec3<f32> {
    let v = normalize(u.cam_position - pos);
    // Clamped: two unit vectors can dot to a hair over 1, and pow of a
    // negative is NaN, which the bloom then spreads over half the frame.
    let k = clamp(1.0 - dot(n, v), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * k * k * k * k * k;
    let r = reflect(-v, n);
    let sky = mix(vec3<f32>(0.02, 0.02, 0.03), vec3<f32>(0.5, 0.56, 0.66), smoothstep(-0.3, 0.9, r.y)) * u.light.x;
    var spec = vec3<f32>(0.0);
    for (var i = 0u; i < 2u; i = i + 1u) {
        let level = u.lamp[i].w;
        if (level <= 0.001) {
            continue;
        }
        let toward = normalize(u.lamp[i].xyz - pos);
        spec += u.lamp_tint[i].rgb * level * pow(max(dot(r, toward), 0.0), 160.0);
    }
    if (u.sun_dir.w > 0.001) {
        spec += u.sun_tint.rgb * u.sun_dir.w * pow(max(dot(r, u.sun_dir.xyz), 0.0), 160.0) * sun_light(pos, n);
    }
    return albedo * light * (1.0 - fresnel) + (sky + spec * 4.0) * fresnel + spec;
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
    let da = scene_depth(a);
    let db = scene_depth(b);
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
    let d = scene_depth(px);
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
    // A surfel's own facing is the bump the liquid smooths away, so the
    // liquid lets go of it; exact normals are kept.
    let trust = select(known.w * (1.0 - s.liquid), known.w, known.w > 0.99);
    if (trust > 0.0) {
        n = normalize(mix(n, known.xyz * 2.0 - 1.0, trust));
    }
    // The room's lines, in the wireframe's colour: given off, not lit.
    let lines = vec3<f32>(0.28, 0.38, 0.58) * g.a;
    // The smoothed surface runs through the middle of the surfels, so in
    // the shadow map they stand proud of it and shade it in crescents:
    // the liquid is lit from where the surfels' fronts are.
    let lift = select(u.size * 2.0 * s.liquid, 0.0, known.w > 0.99);
    let light = surface_light(pos + n * lift, n);
    var lit = albedo * light;
    if (s.liquid > 0.001 && known.w < 0.99) {
        lit = mix(lit, liquid_shade(pos, n, albedo, light), s.liquid);
    }
    if (s.ink > 0u) {
        return vec4<f32>(ink(px, pos, lit, light, albedo) + lines, 1.0);
    }
    return vec4<f32>(lit + lines, 1.0);
}

// --- Ink -------------------------------------------------------------------
//
// The lit surface as a pen drawing, done on the finished G-buffer so it
// costs a few reads a pixel and draws whatever the surface pass drew:
// surfels, glyphs, a solid, the walls.
//
// Outlines are where the depth breaks: at a silhouette, where a
// neighbour is open background, and wherever the depth's second
// difference across a pixel is large against its distance — a fold or
// an overlap, not the steady slope of a plane seen at an angle. That is
// Saito and Takahashi's G-buffer edges, "Comprehensible Rendering of 3-D
// Shapes", SIGGRAPH 1990.
//
// Hatching lays the tone down as up to three layers of parallel lines,
// each adding where the surface is darker than the last — the tonal art
// map of Praun, Hoppe, Webb and Finkelstein, "Real-Time Hatching",
// SIGGRAPH 2001, reduced to three fixed screen-space layers. Stipple lays
// it down as dots on a jittered grid, larger where it is darker, after
// Secord's weighted Voronoi stippling (NPAR 2002) without the relaxation
// step. Both are fixed to the screen, so the marks stay put as the form
// turns under them; the outlines move with it.

const PAPER = vec3<f32>(0.93, 0.91, 0.86);
const PEN = vec3<f32>(0.04, 0.04, 0.06);

/// How far from the camera the surface at `p` is, or -1 where the pixel
/// is open background.
fn ink_distance(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(g_depth));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return -1.0;
    }
    let d = scene_depth(p);
    if (d >= 1.0) {
        return -1.0;
    }
    return length(world_at(p, d) - u.cam_position);
}

/// 1 on an edge, 0 off one, `k` pixels out on each side.
fn ink_edge(px: vec2<i32>, centre: f32, k: i32) -> f32 {
    var edge = 0.0;
    var axes = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1), vec2<i32>(1, -1));
    // A glyph's face, a solid or a wall has its exact normal and a
    // smooth depth; a surfel cloud's depth is bumpy at the scale of one
    // disc. The bar for a fold sits above the bumps where there are
    // bumps, and above what a coarse frame's few-pixel steps show.
    let known = textureLoad(g_normal, px, 0);
    let exact = known.w > 0.99;
    let footprint = 2.0 * centre / (f32(textureDimensions(g_depth).y) * abs(u.view_proj[1][1]));
    var bar = max(0.004 * centre, 3.0 * footprint * f32(k));
    if (!exact) {
        bar = max(max(bar, 0.012 * centre), 3.0 * u.size);
    }
    let n = known.xyz * 2.0 - 1.0;
    for (var i = 0; i < 4; i = i + 1) {
        let pa = px + axes[i] * k;
        let pb = px - axes[i] * k;
        let a = ink_distance(pa);
        let b = ink_distance(pb);
        if (a < 0.0 || b < 0.0) {
            // Open background on a side: a silhouette, unless it is only
            // a pinhole between surfels, which closes again a step on.
            let a2 = ink_distance(px + axes[i] * 2 * k);
            let b2 = ink_distance(px - axes[i] * 2 * k);
            if ((a < 0.0 && a2 < 0.0) || (b < 0.0 && b2 < 0.0)) {
                return 1.0;
            }
            continue;
        }
        edge = max(edge, smoothstep(bar, 2.0 * bar, abs(a + b - 2.0 * centre)));
        // Where the normals are exact, a crease is where they turn: the
        // edge of a cube's face, the lip of a sponge's hole.
        if (exact) {
            let na = textureLoad(g_normal, pa, 0);
            if (na.w > 0.99) {
                edge = max(edge, 1.0 - smoothstep(0.6, 0.8, dot(n, na.xyz * 2.0 - 1.0)));
            }
        }
    }
    return edge;
}

fn ink_hash(c: vec2<i32>, stream: u32) -> f32 {
    let h = hash_u32(bitcast<u32>(c.x) * 0x9E3779B1u ^ hash_u32(bitcast<u32>(c.y) + stream * 0x85EBCA77u));
    return f32(h >> 8u) / 16777216.0;
}

/// How much of a line `across` pixels from a line's centre covers the
/// pixel, for a line `width` pixels wide.
fn ink_line(across: f32, width: f32) -> f32 {
    return 1.0 - smoothstep(width * 0.5 - 0.5, width * 0.5 + 0.5, across);
}

fn ink(px: vec2<i32>, pos: vec3<f32>, lit: vec3<f32>, light: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
    // Marks are sized for a 1200-pixel-high frame, which is a 600-line
    // output at the 2× the scene renders at, and scale with it.
    let scale = f32(textureDimensions(g_depth).y) / 1200.0;
    // Never finer than a pixel can draw, however small the frame.
    let weight = mix(0.5, 2.0, clamp(s.ink_weight, 0.0, 1.0)) * max(scale, 0.5);
    let centre = length(pos - u.cam_position);
    let edge = ink_edge(px, centre, max(i32(round(weight)), 1));
    if (s.ink == 1u) {
        return mix(lit, PEN, edge);
    }
    // Tone: how much light fell on the surface, not how bright its
    // colour is, so a dark palette is not drawn as a dark form; and as a
    // share of all the light there is, so a brighter rig does not wash
    // the drawing out to paper. Unlit, that is the sky's shading alone:
    // a top is paper and an underside takes all three layers.
    let l = dot(light, vec3<f32>(0.2126, 0.7152, 0.0722));
    var all = u.light.x + u.sun_dir.w;
    for (var i = 0u; i < 2u; i = i + 1u) {
        all += u.lamp[i].w;
    }
    let tone = clamp(l / max(all, 1e-3), 0.0, 1.0);
    // The paper takes a little of the surface's own colour.
    let tint = albedo / max(max(albedo.r, max(albedo.g, albedo.b)), 1e-3);
    let paper = PAPER * mix(vec3<f32>(1.0), tint, 0.18);
    let f = vec2<f32>(px) + 0.5;
    var cover = 0.0;
    if (s.ink == 2u) {
        let spacing = 9.0 * weight;
        let width = 2.0 * weight;
        var dirs = array<vec2<f32>, 3>(
            vec2<f32>(0.7071, 0.7071),
            vec2<f32>(0.7071, -0.7071),
            vec2<f32>(1.0, 0.0),
        );
        var below = array<f32, 3>(0.8, 0.55, 0.3);
        for (var i = 0; i < 3; i = i + 1) {
            // Each layer fades in over a band of tone, so a gradient
            // gains its lines gradually rather than at a contour.
            let on = 1.0 - smoothstep(below[i] - 0.08, below[i] + 0.08, tone);
            let a = dot(f, dirs[i]) / spacing + f32(i) * 0.37;
            let across = abs(fract(a) - 0.5) * spacing;
            cover = max(cover, on * ink_line(across, width));
        }
    } else {
        let cell = 5.0 * weight;
        let here = vec2<i32>(floor(f / cell));
        let dark = clamp(1.0 - tone, 0.0, 1.0);
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            for (var dx = -1; dx <= 1; dx = dx + 1) {
                let c = here + vec2<i32>(dx, dy);
                // A dot per cell, somewhere in it, and only some of
                // them in the light: area and count both follow the
                // tone.
                if (ink_hash(c, 2u) > dark * 1.4) {
                    continue;
                }
                let at = (vec2<f32>(c) + vec2<f32>(ink_hash(c, 0u), ink_hash(c, 1u))) * cell;
                let r = cell * 0.62 * sqrt(dark) + 0.35;
                cover = max(cover, 1.0 - smoothstep(r - 0.6, r + 0.6, length(f - at)));
            }
        }
    }
    cover = max(cover, edge);
    return mix(paper, PEN, cover);
}
