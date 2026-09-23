// The room the cloud floats in.
//
// A wireframe box drawn with the same view/projection as the particles, so
// moving the camera parallaxes the room against the cloud. That parallax is
// the whole point: a static backdrop reads as wallpaper, while one that
// shifts against the foreground reads as space.
//
// Geometry is generated from the vertex index — no buffers, same as the
// particles: floor and ceiling grids, side walls, and the back wall.
//
// Each line is a screen-aligned quad with analytic coverage rather than a
// hardware line. A hardware line is one pixel wide at whatever resolution
// it is drawn, so rendering above 1× and downscaling thinned it: at 2× the
// room kept half its light, at 4× a quarter. The quad is `line_px` render
// pixels wide — one *output* pixel whatever the render scale — and its
// edge coverage is the exact overlap of a one-pixel box with the line, so
// the light it puts into each output pixel does not depend on how finely
// it was drawn. That is also what anti-aliases it: the staircase a
// hardware line draws at 1× is gone (Gupta & Sproull, "Filtering edges
// for gray-scale displays", SIGGRAPH 1981 — the box-filter case).

struct Room {
    view_proj: mat4x4<f32>,
    // Half-extents. x and y come from the camera frustum so the front face
    // lands exactly on the frame edge; z is how deep the room runs.
    half_x: f32,
    half_y: f32,
    depth: f32,
    // Where the front face sits along z, in world space.
    front_z: f32,
    brightness: f32,
    fade: f32,
    // Back rect relative to the front: size, and centre offset. Together
    // these are the perspective controls — see room.rs.
    converge: f32,
    vanish_x: f32,
    vanish_y: f32,
    // Size of the target being drawn into, in pixels, and how wide a line
    // is in those pixels. Filled in by Room::render, not by the caller.
    viewport_w: f32,
    viewport_h: f32,
    line_px: f32,
};

@group(0) @binding(0) var<uniform> u: Room;

/// Half-extents and centre of the cross-section at depth `t` (0 at the
/// opening, 1 at the back wall).
///
/// Interpolating the *rectangle* rather than drawing a box is what lets the
/// perspective be steered independently of the lens: the opening stays
/// pinned to the frame while the far end shrinks and slides.
fn section(t: f32) -> vec4<f32> {
    let scale = mix(1.0, u.converge, t);
    return vec4<f32>(
        u.half_x * scale,
        u.half_y * scale,
        u.vanish_x * u.half_x * t,
        u.vanish_y * u.half_y * t,
    );
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) shade: f32,
    // Signed distance from the line's centre, in pixels. Interpolated in
    // screen space (`linear`, not perspective-correct): the quad is offset
    // in screen space, so that is the space in which the distance is
    // affine.
    @location(1) @interpolate(linear) across: f32,
};

/// Lines per face along each axis. Kept modest: a dense grid reads as
/// texture rather than as structure, and structure is what gives depth.
const N: u32 = 10u;

/// Endpoints for line `i`, laid out face by face.
///
/// Depth lines (running away from the viewer) do the work — they are the
/// ones that converge toward a vanishing point and tell the eye how far
/// away the back wall is. Cross lines mostly measure the depth lines.
fn line_endpoints(i: u32) -> array<vec3<f32>, 2> {
    let front = u.front_z;
    let back = u.front_z - u.depth;

    let per_face = N * 2u;   // depth lines + cross lines
    let face = i / per_face;
    let k = i % per_face;
    // 0..1 across the face, and 0..1 along the depth.
    let t = f32(k % N) / f32(N - 1u);
    let is_cross = k >= N;
    let a = -1.0 + 2.0 * t;
    let z = mix(front, back, t);

    // Front and back sections, plus the one at this cross line's depth.
    let f = section(0.0);
    let b = section(1.0);
    let m = section(t);

    switch face {
        // Floor.
        case 0u: {
            if (is_cross) {
                return array(vec3(m.z - m.x, m.w - m.y, z), vec3(m.z + m.x, m.w - m.y, z));
            }
            return array(
                vec3(f.z + a * f.x, f.w - f.y, front),
                vec3(b.z + a * b.x, b.w - b.y, back),
            );
        }
        // Ceiling.
        case 1u: {
            if (is_cross) {
                return array(vec3(m.z - m.x, m.w + m.y, z), vec3(m.z + m.x, m.w + m.y, z));
            }
            return array(
                vec3(f.z + a * f.x, f.w + f.y, front),
                vec3(b.z + a * b.x, b.w + b.y, back),
            );
        }
        // Left wall.
        case 2u: {
            if (is_cross) {
                return array(vec3(m.z - m.x, m.w - m.y, z), vec3(m.z - m.x, m.w + m.y, z));
            }
            return array(
                vec3(f.z - f.x, f.w + a * f.y, front),
                vec3(b.z - b.x, b.w + a * b.y, back),
            );
        }
        // Right wall.
        case 3u: {
            if (is_cross) {
                return array(vec3(m.z + m.x, m.w - m.y, z), vec3(m.z + m.x, m.w + m.y, z));
            }
            return array(
                vec3(f.z + f.x, f.w + a * f.y, front),
                vec3(b.z + b.x, b.w + a * b.y, back),
            );
        }
        // Back wall: the grid the depth lines converge onto, and therefore
        // what sets the sense of distance.
        default: {
            if (is_cross) {
                return array(vec3(b.z - b.x, b.w + a * b.y, back), vec3(b.z + b.x, b.w + a * b.y, back));
            }
            return array(vec3(b.z + a * b.x, b.w - b.y, back), vec3(b.z + a * b.x, b.w + b.y, back));
        }
    }
}

/// Clip-space w below which a point counts as behind the camera.
const NEAR_W: f32 = 1e-3;

/// Pull `a` along the segment toward `b` until it is in front of the
/// camera. A hardware line is clipped for free; a quad built by dividing
/// by w is not, and an endpoint behind the eye would flip to the far side
/// of the screen.
fn clip_near(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    if (a.w >= NEAR_W) {
        return a;
    }
    let t = (NEAR_W - a.w) / (b.w - a.w);
    return mix(a, b, t);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Six vertices per line: two triangles over (end, side).
    let line = vi / 6u;
    let corner = vi % 6u;
    // 0,1,2 / 2,1,3 over corners numbered end + 2*side.
    var corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let c = corners[corner];
    let end = c & 1u;
    let side = f32(c >> 1u) * 2.0 - 1.0;

    let ends = line_endpoints(line);
    let p = ends[end];
    var a = u.view_proj * vec4<f32>(ends[0], 1.0);
    var b = u.view_proj * vec4<f32>(ends[1], 1.0);

    var out: VsOut;
    // Fade with depth into the room. Without this the back wall is as
    // bright as the front and the box reads flat — the gradient is most of
    // what makes it read as receding at all.
    let d = clamp((u.front_z - p.z) / max(u.depth, 1e-4), 0.0, 1.0);
    out.shade = mix(1.0, 1.0 - u.fade, d);

    if (a.w < NEAR_W && b.w < NEAR_W) {
        // Wholly behind the camera: a degenerate triangle draws nothing.
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.across = 0.0;
        return out;
    }
    let a2 = clip_near(a, b);
    let b2 = clip_near(b, a);
    a = a2;
    b = b2;

    // Endpoints in pixels.
    let half_vp = vec2<f32>(u.viewport_w, u.viewport_h) * 0.5;
    let sa = a.xy / a.w * half_vp;
    let sb = b.xy / b.w * half_vp;
    let len = length(sb - sa);
    let dir = select(vec2<f32>(1.0, 0.0), (sb - sa) / len, len > 1e-6);
    let normal = vec2<f32>(-dir.y, dir.x);

    // Half a line plus half a pixel on each side: the widest reach at
    // which a pixel's box still overlaps the line. Ends extend by half a
    // line so a corner where two lines meet is closed.
    let reach = 0.5 * u.line_px + 0.5;
    let q = select(a, b, end == 1u);
    let along = select(-1.0, 1.0, end == 1u) * 0.5 * u.line_px;
    let offset_px = normal * side * reach + dir * along;
    out.pos = vec4<f32>(q.xy + offset_px / half_vp * q.w, q.z, q.w);
    out.across = side * reach;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Exact overlap of this pixel's one-pixel box with a line `line_px`
    // wide. Integrates to `line_px` across the line, whatever the
    // subpixel position — so a line thinner than a pixel dims rather than
    // flickering, and the total light is fixed.
    let dist = abs(in.across);
    let half_w = 0.5 * u.line_px;
    let cover = clamp(min(dist + 0.5, half_w) - max(dist - 0.5, -half_w), 0.0, 1.0);
    // Cool and dim: the room is a container, not a subject. Additive, so
    // it sits under the particles rather than occluding them.
    let c = vec3<f32>(0.28, 0.38, 0.58) * u.brightness * in.shade * cover;
    return vec4<f32>(c, 1.0);
}
