// The room the cloud floats in.
//
// A wireframe box drawn with the same view/projection as the particles, so
// moving the camera parallaxes the room against the cloud. That parallax is
// the whole point: a static backdrop reads as wallpaper, while one that
// shifts against the foreground reads as space.
//
// Geometry is generated from the vertex index — no buffers, same as the
// particles — as a LineList: floor and ceiling grids, side walls, and the
// back wall.

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
    lines: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Room;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) shade: f32,
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
    let hx = u.half_x;
    let hy = u.half_y;
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

    switch face {
        // Floor.
        case 0u: {
            if (is_cross) {
                return array(vec3(-hx, -hy, z), vec3(hx, -hy, z));
            }
            return array(vec3(a * hx, -hy, front), vec3(a * hx, -hy, back));
        }
        // Ceiling.
        case 1u: {
            if (is_cross) {
                return array(vec3(-hx, hy, z), vec3(hx, hy, z));
            }
            return array(vec3(a * hx, hy, front), vec3(a * hx, hy, back));
        }
        // Left wall.
        case 2u: {
            if (is_cross) {
                return array(vec3(-hx, -hy, z), vec3(-hx, hy, z));
            }
            return array(vec3(-hx, a * hy, front), vec3(-hx, a * hy, back));
        }
        // Right wall.
        case 3u: {
            if (is_cross) {
                return array(vec3(hx, -hy, z), vec3(hx, hy, z));
            }
            return array(vec3(hx, a * hy, front), vec3(hx, a * hy, back));
        }
        // Back wall: a flat grid, which is what the depth lines converge
        // onto and therefore what sets the sense of distance.
        default: {
            if (is_cross) {
                return array(vec3(-hx, a * hy, back), vec3(hx, a * hy, back));
            }
            return array(vec3(a * hx, -hy, back), vec3(a * hx, hy, back));
        }
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let ends = line_endpoints(vi / 2u);
    let p = ends[vi % 2u];
    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(p, 1.0);
    // Fade with depth into the room. Without this the back wall is as
    // bright as the front and the box reads flat — the gradient is most of
    // what makes it read as receding at all.
    let d = clamp((u.front_z - p.z) / max(u.depth, 1e-4), 0.0, 1.0);
    out.shade = mix(1.0, 1.0 - u.fade, d);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Cool and dim: the room is a container, not a subject. Additive, so
    // it sits under the particles rather than occluding them.
    let c = vec3<f32>(0.28, 0.38, 0.58) * u.brightness * in.shade;
    return vec4<f32>(c, 1.0);
}
