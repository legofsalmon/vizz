// Plexus: lines between particles that come near each other.
//
// Appended to particles.wgsl, so a node is placed and coloured by the same
// `body` and `body_albedo` the dots are, and a link always ends on a dot.
// See plexus.rs for the passes.

/// Must match `PlexusUniforms` in plexus.rs.
struct Plexus {
    // How many nodes there are this frame, and which particle each is:
    // node n is particle n * stride, so the nodes spread over the whole
    // field whatever its count.
    nodes: u32,
    stride: u32,
    // How near two nodes must be to link, in world units.
    reach: f32,
    // How bright a link is at no distance.
    strength: f32,
    // Line width in pixels of the target.
    width: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

struct Node {
    pos: vec3<f32>,
    _pad0: f32,
    color: vec3<f32>,
    _pad1: f32,
};

/// Links kept per node: its nearest within reach. Must match `LINKS` in
/// plexus.rs.
const PLEXUS_LINKS: u32 = 4u;
const NO_LINK: u32 = 0xffffffffu;

@group(1) @binding(0) var<uniform> pl: Plexus;
@group(1) @binding(1) var<storage, read> nodes: array<Node>;
@group(1) @binding(2) var<storage, read> links: array<u32>;
@group(1) @binding(3) var<storage, read_write> nodes_out: array<Node>;
@group(1) @binding(4) var<storage, read_write> links_out: array<u32>;

@compute @workgroup_size(64)
fn cs_plexus_nodes(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = id.x;
    if (n >= pl.nodes) {
        return;
    }
    let b = body(n * pl.stride);
    let centre = u.view_proj * vec4<f32>(b.p, 1.0);
    var out: Node;
    out.pos = b.p;
    out._pad0 = 0.0;
    out.color = body_albedo(b, centre.w);
    out._pad1 = 0.0;
    nodes_out[n] = out;
}

/// Each node's nearest few within reach, by looking at every other node.
/// Brute force, deliberately: at a few thousand nodes it is a few million
/// distance tests, which a GPU does in well under a millisecond, and it
/// needs no grid to build or bound.
@compute @workgroup_size(64)
fn cs_plexus_links(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = id.x;
    if (n >= pl.nodes) {
        return;
    }
    let p = nodes_out[n].pos;
    var best = array<u32, 4>(NO_LINK, NO_LINK, NO_LINK, NO_LINK);
    var dist = array<f32, 4>(1e30, 1e30, 1e30, 1e30);
    let r2 = pl.reach * pl.reach;
    for (var m = 0u; m < pl.nodes; m = m + 1u) {
        if (m == n) {
            continue;
        }
        let d = nodes_out[m].pos - p;
        let d2 = dot(d, d);
        if (d2 >= r2 || d2 >= dist[3]) {
            continue;
        }
        // Insert, keeping the four in order.
        var k = 3u;
        while (k > 0u && dist[k - 1u] > d2) {
            dist[k] = dist[k - 1u];
            best[k] = best[k - 1u];
            k = k - 1u;
        }
        dist[k] = d2;
        best[k] = m;
    }
    for (var k = 0u; k < PLEXUS_LINKS; k = k + 1u) {
        links_out[n * PLEXUS_LINKS + k] = best[k];
    }
}

struct PlexusOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    // -1..1 across the line, for its soft edge.
    @location(1) across: f32,
};

fn plexus_cull() -> PlexusOut {
    var out: PlexusOut;
    out.pos = vec4<f32>(4.0, 4.0, 2.0, 1.0);
    out.color = vec3<f32>(0.0);
    out.across = 0.0;
    return out;
}

@vertex
fn vs_plexus(@builtin(vertex_index) vi: u32) -> PlexusOut {
    let l = vi / 6u;
    let corner = vi % 6u;
    let n = l / PLEXUS_LINKS;
    let m = links[l];
    if (n >= pl.nodes || m == NO_LINK) {
        return plexus_cull();
    }
    // A pair that lists each other is one line, drawn from the lower
    // index, rather than two on top of each other at double brightness.
    if (m < n) {
        for (var k = 0u; k < PLEXUS_LINKS; k = k + 1u) {
            if (links[m * PLEXUS_LINKS + k] == n) {
                return plexus_cull();
            }
        }
    }
    let a = nodes[n];
    let b = nodes[m];
    let ca = u.view_proj * vec4<f32>(a.pos, 1.0);
    let cb = u.view_proj * vec4<f32>(b.pos, 1.0);
    if (ca.w < 0.02 || cb.w < 0.02) {
        return plexus_cull();
    }
    // A screen-space quad along the segment, `width` pixels wide.
    let size = vec2<f32>(u.viewport_h * u.aspect, u.viewport_h);
    let sa = ca.xy / ca.w;
    let sb = cb.xy / cb.w;
    var dir = (sb - sa) * size;
    if (dot(dir, dir) < 1e-6) {
        dir = vec2<f32>(1.0, 0.0);
    }
    let side = normalize(vec2<f32>(-dir.y, dir.x)) / size * (pl.width + 1.0);
    var along = array<f32, 6>(0.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    var offs = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    let t = along[corner];
    let o = offs[corner];
    let clip = mix(ca, cb, t);
    // Fades out towards the reach, so a link appears and goes softly as
    // two particles drift together and apart, rather than snapping.
    let d = distance(a.pos, b.pos);
    let fade = 1.0 - smoothstep(0.0, 1.0, d / max(pl.reach, 1e-6));
    var out: PlexusOut;
    out.pos = vec4<f32>(clip.xy + side * o * clip.w, clip.z, clip.w);
    out.color = mix(a.color, b.color, t) * pl.strength * fade * fade;
    out.across = o;
    return out;
}

@fragment
fn fs_plexus(in: PlexusOut) -> @location(0) vec4<f32> {
    // The quad is a pixel wider than the line, so its edge can fall off
    // instead of stepping.
    let edge = 1.0 - smoothstep(pl.width / (pl.width + 1.0) - 0.15, 1.0, abs(in.across));
    let c = in.color * edge;
    // Coverage in alpha, as the dots write it, so a transparent output
    // carries the lines too.
    return vec4<f32>(c, clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0));
}
