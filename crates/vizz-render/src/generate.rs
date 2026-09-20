//! Point clouds from equations.
//!
//! A generator is a function from nothing to a slot's worth of points —
//! [`POINTS`], the size of one row of the cloud bank — made once on the
//! CPU and uploaded exactly as a file is. Everything that already works
//! for a file works for it unchanged: it is chosen by name, crossed to
//! with `/cloud/morph`, captured by a preset as the look's source, lit
//! when it has normals to fit, and it costs the shader nothing a scan
//! does not.
//!
//! Why a slot and not a `/shape/mode` entry: the mode sweep is an
//! address. Presets hold its numbers and the label list is pinned, so a
//! form added there moves "cloud pair" for every saved look. A slot is a
//! name, and a bank of names can grow.
//!
//! Why the CPU: the same argument as `attractor.rs`. A flow only looks
//! like itself after its transient decays, a surface wants an ordered
//! sweep, and a fractal wants a search — none of which belongs in a
//! vertex shader that runs six times per particle per frame.
//!
//! **Order matters.** Consecutive points are consecutive in *time* for a
//! flow, consecutive along the *curve* for a knot and consecutive in
//! *scan order* for a surface, because the shader advances every
//! particle's index together, and that is what makes a cloud crawl
//! along itself instead of shimmering.
//!
//! Every generator is deterministic — its own seeded generator, no
//! clock, no thread — so a `gen:` entry in the settings comes back as
//! exactly the cloud that was on screen, the way a `text:` entry does.

use std::f64::consts::{PI, SQRT_2, TAU};

use crate::attractor::POINTS;
use crate::pointcloud::Point;

/// Every generator this crate can make, by the id `gen:<id>` names it.
///
/// The catalogue the panel lists — names, blurbs, families — lives in
/// vizz-mod, which cannot see this crate; a test in vizz-app holds the
/// two lists to each other.
pub const IDS: &[&str] = &[
    "thomas",
    "halvorsen",
    "dadras",
    "rossler",
    "four-wing",
    "chen",
    "clifford",
    "dejong",
    "supershape",
    "harmonic",
    "lissajous",
    "torus-knot",
    "hopf",
    "chladni",
    "sierpinski",
    "menger",
    "mandelbulb",
];

/// Make the cloud `id` names, or `None` for an id this crate does not
/// know. Always exactly [`POINTS`] points, centred, the widest axis
/// spanning `[-1, 1]` — the box every other cloud is fitted to, so
/// `/particles/spread` means the same thing whatever is in the slot.
pub fn generate(id: &str) -> Option<Vec<Point>> {
    let raw = match id {
        // Flows: integrated from a point on the attractor, in time order.
        "thomas" => flow(thomas, [0.1, 0.0, 0.0], 0.05, Frame::Diagonal),
        "halvorsen" => flow(halvorsen, [-1.48, -1.51, 2.04], 0.005, Frame::Diagonal),
        "dadras" => flow(dadras, [1.1, 2.1, -2.0], 0.005, Frame::ZUp),
        "rossler" => flow(rossler, [1.0, 1.0, 1.0], 0.02, Frame::ZUp),
        "four-wing" => flow(four_wing, [1.3, -0.18, 0.01], 0.01, Frame::ZUp),
        "chen" => flow(chen, [-0.1, 0.5, -0.6], 0.002, Frame::ZUp),
        // Maps: iterated, and lifted into depth by delay embedding.
        "clifford" => map(clifford, [0.1, 0.0]),
        "dejong" => map(dejong, [0.1, 0.1]),
        // Surfaces, in scan order.
        "supershape" => supershape(),
        "harmonic" => harmonic(),
        // Curves, along the curve, thickened into tubes.
        "lissajous" => tube(lissajous, 0.05, Frame::YUp),
        "torus-knot" => tube(torus_knot, 0.04, Frame::YUp),
        "hopf" => hopf(),
        // The rest: sampled.
        "chladni" => chladni(),
        "sierpinski" => sierpinski(),
        "menger" => menger(),
        "mandelbulb" => mandelbulb(),
        _ => return None,
    };
    Some(finish(raw))
}

// --- Flows -----------------------------------------------------------

/// Thomas' cyclically symmetric attractor (Thomas, 1999), b = 0.208186:
/// three sines, one per axis, and a slow flow that ties itself into a
/// knot of ribbons around the (1,1,1) diagonal.
fn thomas([x, y, z]: [f64; 3]) -> [f64; 3] {
    const B: f64 = 0.208186;
    [y.sin() - B * x, z.sin() - B * y, x.sin() - B * z]
}

/// Halvorsen's attractor, a = 1.89: cyclically symmetric like Thomas,
/// but three lobes of folded sheet rather than ribbons.
fn halvorsen([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 1.89;
    [
        -A * x - 4.0 * y - 4.0 * z - y * y,
        -A * y - 4.0 * z - 4.0 * x - z * z,
        -A * z - 4.0 * x - 4.0 * y - x * x,
    ]
}

/// Dadras' tri-scroll (Dadras & Momeni, 2009), a=3 b=2.7 c=1.7 d=2 e=9.
fn dadras([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 3.0;
    const B: f64 = 2.7;
    const C: f64 = 1.7;
    const D: f64 = 2.0;
    const E: f64 = 9.0;
    [y - A * x + B * y * z, C * y - x * z + z, D * x * y - E * z]
}

/// Rössler (1976), a = b = 0.2, c = 5.7: a flat spiral with a fold.
fn rossler([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.2;
    const B: f64 = 0.2;
    const C: f64 = 5.7;
    [-y - z, x + A * y, B + z * (x - C)]
}

/// The four-wing attractor, a = 0.2, b = 0.01, c = -0.4.
fn four_wing([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.2;
    const B: f64 = 0.01;
    const C: f64 = -0.4;
    [A * x + y * z, B * x + C * y - x * z, -z - x * y]
}

/// Chen's system (1999), a = 35, b = 3, c = 28: the Lorenz family's
/// wider, more tangled member.
fn chen([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 35.0;
    const B: f64 = 3.0;
    const C: f64 = 28.0;
    [A * (y - x), (C - A) * x - x * z + C * y, x * y - B * z]
}

/// Which way is up, per system. The flows are written with z as their
/// axis of symmetry and the camera wants height in y; the two cyclic
/// ones are symmetric about the (1,1,1) diagonal instead, and stand up
/// straightest with that diagonal vertical.
#[derive(Clone, Copy)]
enum Frame {
    YUp,
    ZUp,
    Diagonal,
}

fn orient([x, y, z]: [f64; 3], frame: Frame) -> [f64; 3] {
    match frame {
        Frame::YUp => [x, y, z],
        Frame::ZUp => [x, z, y],
        Frame::Diagonal => {
            // Rodrigues' rotation taking (1,1,1)/√3 to (0,1,0): the axis
            // is (-1,0,1)/√2, cos 1/√3, sin √(2/3).
            let k = [-1.0 / SQRT_2, 0.0, 1.0 / SQRT_2];
            let (c, s) = (1.0 / 3f64.sqrt(), (2.0f64 / 3.0).sqrt());
            let v = [x, y, z];
            let kxv = [
                k[1] * v[2] - k[2] * v[1],
                k[2] * v[0] - k[0] * v[2],
                k[0] * v[1] - k[1] * v[0],
            ];
            let kv = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
            [
                v[0] * c + kxv[0] * s + k[0] * kv * (1.0 - c),
                v[1] * c + kxv[1] * s + k[1] * kv * (1.0 - c),
                v[2] * c + kxv[2] * s + k[2] * kv * (1.0 - c),
            ]
        }
    }
}

/// One Runge–Kutta step. Fourth order rather than the Euler step the
/// built-in attractors use: some of these systems are stiff (Chen's
/// timescale is Lorenz's), and a fourth-order step at the same cost as
/// four Euler steps stays on the manifold where Euler drifts off it.
fn rk4(f: fn([f64; 3]) -> [f64; 3], p: [f64; 3], dt: f64) -> [f64; 3] {
    let add = |a: [f64; 3], b: [f64; 3], s: f64| [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s];
    let k1 = f(p);
    let k2 = f(add(p, k1, dt * 0.5));
    let k3 = f(add(p, k2, dt * 0.5));
    let k4 = f(add(p, k3, dt));
    [
        p[0] + dt / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
        p[1] + dt / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
        p[2] + dt / 6.0 * (k1[2] + 2.0 * k2[2] + 2.0 * k3[2] + k4[2]),
    ]
}

/// Integrate a flow: run in from an arbitrary start until the transient
/// has decayed, then record a slot's worth of steps in time order.
fn flow(f: fn([f64; 3]) -> [f64; 3], start: [f64; 3], dt: f64, frame: Frame) -> Vec<[f64; 3]> {
    const TRANSIENT: usize = 5_000;
    let mut p = start;
    for _ in 0..TRANSIENT {
        p = rk4(f, p, dt);
    }
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        let q = rk4(f, p, dt);
        // A step that left the manifold — a stiff system at too coarse a
        // step — repeats the last good point rather than poisoning the
        // slot with a NaN that the shader would draw at infinity.
        if q.iter().all(|v| v.is_finite() && v.abs() < 1e6) {
            p = q;
        }
        out.push(orient(p, frame));
    }
    out
}

// --- Maps ------------------------------------------------------------

/// Pickover's Clifford attractor, a=-1.4 b=1.6 c=1.0 d=0.7.
fn clifford([x, y]: [f64; 2]) -> [f64; 2] {
    const A: f64 = -1.4;
    const B: f64 = 1.6;
    const C: f64 = 1.0;
    const D: f64 = 0.7;
    [(A * y).sin() + C * (A * x).cos(), (B * x).sin() + D * (B * y).cos()]
}

/// Peter de Jong's attractor (Scientific American, 1987), a=1.4 b=-2.3
/// c=2.4 d=-2.1.
fn dejong([x, y]: [f64; 2]) -> [f64; 2] {
    const A: f64 = 1.4;
    const B: f64 = -2.3;
    const C: f64 = 2.4;
    const D: f64 = -2.1;
    [(A * y).sin() - (B * x).cos(), (C * x).sin() - (D * y).cos()]
}

/// Iterate a plane map and lift it into depth by delay embedding: the
/// third coordinate is the previous iterate's x. Takens' theorem says a
/// delay coordinate unfolds the dynamics rather than merely decorating
/// them, and it shows — the sheet gets a genuine third dimension, not
/// an extrusion.
fn map(f: fn([f64; 2]) -> [f64; 2], start: [f64; 2]) -> Vec<[f64; 3]> {
    let mut p = start;
    for _ in 0..100 {
        p = f(p);
    }
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        let prev = p;
        p = f(p);
        // Depth at six tenths of the face: a delay coordinate has the
        // face's range, and a cube of it reads as a block rather than a
        // relief.
        out.push([p[0], p[1], prev[0] * 0.6]);
    }
    out
}

// --- Surfaces ---------------------------------------------------------

/// Gielis' superformula: the radius at angle `phi` of a shape with
/// `m`-fold symmetry, the exponents deciding pinched from bloated.
fn superformula(phi: f64, m: f64, n1: f64, n2: f64, n3: f64) -> f64 {
    let t = m * phi / 4.0;
    (t.cos().abs().powf(n2) + t.sin().abs().powf(n3)).powf(-1.0 / n1)
}

/// A surface sampled on a 256×256 longitude/latitude grid, in scan
/// order. Latitude is spaced equal-area rather than equal-angle, so the
/// poles do not pile up.
fn grid(f: impl Fn(f64, f64) -> [f64; 3]) -> Vec<[f64; 3]> {
    const SIDE: usize = 256;
    debug_assert_eq!(SIDE * SIDE, POINTS);
    let mut out = Vec::with_capacity(POINTS);
    for row in 0..SIDE {
        let lat = (2.0 * (row as f64 + 0.5) / SIDE as f64 - 1.0).asin();
        for col in 0..SIDE {
            let lon = (col as f64 + 0.5) / SIDE as f64 * TAU - PI;
            out.push(f(lon, lat));
        }
    }
    out
}

/// The spherical product of two superformulas — Gielis' 3D form — with
/// the (7, 2, 8, 4) parameters, a seven-fold flower.
fn supershape() -> Vec<[f64; 3]> {
    grid(|lon, lat| {
        let r1 = superformula(lon, 7.0, 2.0, 8.0, 4.0);
        let r2 = superformula(lat, 7.0, 2.0, 8.0, 4.0);
        orient(
            [r1 * lon.cos() * r2 * lat.cos(), r1 * lon.sin() * r2 * lat.cos(), r2 * lat.sin()],
            Frame::ZUp,
        )
    })
}

/// A sphere rippled by a spherical harmonic: three waves round, two
/// waves up.
fn harmonic() -> Vec<[f64; 3]> {
    grid(|lon, lat| {
        let r = 1.0 + 0.45 * (3.0 * lon).cos() * (2.0 * lat).sin();
        orient([r * lon.cos() * lat.cos(), r * lon.sin() * lat.cos(), r * lat.sin()], Frame::ZUp)
    })
}

// --- Curves -----------------------------------------------------------

/// A 3:4:7 Lissajous knot: pairwise coprime frequencies, phases off the
/// values that would let it cross itself.
fn lissajous(t: f64) -> [f64; 3] {
    [(3.0 * t + 0.5).cos(), (4.0 * t + 1.3).cos(), (7.0 * t).cos()]
}

/// A (3,7) torus knot: three times round the axis, seven times through
/// the hole. The trefoil in `/shape/mode` is the (2,3) of the same family.
fn torus_knot(t: f64) -> [f64; 3] {
    const P: f64 = 3.0;
    const Q: f64 = 7.0;
    const R: f64 = 0.7;
    const RADIUS: f64 = 0.3;
    let ring = R + RADIUS * (Q * t).cos();
    [ring * (P * t).cos(), RADIUS * (Q * t).sin(), ring * (P * t).sin()]
}

/// A closed curve traced once, thickened into a fuzzy tube: each point
/// on the curve gets a random offset inside a ball of `radius`, so the
/// cloud reads as a volume rather than a wire.
fn tube(curve: fn(f64) -> [f64; 3], radius: f64, frame: Frame) -> Vec<[f64; 3]> {
    let mut rng = Rng::new(0x7A5E_C0DE);
    (0..POINTS)
        .map(|i| {
            let c = curve(i as f64 / POINTS as f64 * TAU);
            let j = rng.in_ball(radius);
            orient([c[0] + j[0], c[1] + j[1], c[2] + j[2]], frame)
        })
        .collect()
}

/// The Hopf fibration, stereographically projected: four rings of base
/// points on the 2-sphere, each lifting to a torus of linked circles in
/// the 3-sphere, projected to nested tori of Villarceau circles here.
/// Point by point along each circle, so the cloud crawls along the
/// fibres.
fn hopf() -> Vec<[f64; 3]> {
    const RINGS: usize = 4;
    const FIBRES: usize = 32;
    const ALONG: usize = 512;
    debug_assert_eq!(RINGS * FIBRES * ALONG, POINTS);
    let mut out = Vec::with_capacity(POINTS);
    for ring in 0..RINGS {
        // Polar angles well off the north pole, whose fibre projects to
        // the axis itself and would run off to infinity.
        let psi = (65.0 + 20.0 * ring as f64).to_radians();
        for k in 0..FIBRES {
            let alpha = k as f64 / FIBRES as f64 * TAU;
            let (a, b, c) = (psi.sin() * alpha.cos(), psi.sin() * alpha.sin(), psi.cos());
            let s = 1.0 / (2.0 * (1.0 + c)).sqrt();
            for j in 0..ALONG {
                let th = j as f64 / ALONG as f64 * TAU;
                // The fibre over (a, b, c) as unit quaternions
                // q(a,b,c) · (cos θ + k sin θ).
                let w = s * (1.0 + c) * th.cos();
                let x = s * (a * th.sin() - b * th.cos());
                let y = s * (a * th.cos() + b * th.sin());
                let z = s * (1.0 + c) * th.sin();
                // Stereographic projection from (1, 0, 0, 0), then a
                // gentle radial compression so the outer tori do not
                // dwarf the inner ones once the box is fitted.
                let d = 1.0 - w;
                let p = [x / d, y / d, z / d];
                let n = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                let k = 1.0 / (1.0 + 0.12 * n);
                out.push(orient([p[0] * k, p[1] * k, p[2] * k], Frame::ZUp));
            }
        }
    }
    out
}

// --- Sampled ----------------------------------------------------------

/// Chladni's sand: a plate vibrating in its (5, 2) mode, points kept
/// where it stands still. Rejection-sampled with a Gaussian acceptance on
/// the displacement, so the lines have the soft width sand has.
fn chladni() -> Vec<[f64; 3]> {
    const N: f64 = 5.0;
    const M: f64 = 2.0;
    const SIGMA: f64 = 0.07;
    let mut rng = Rng::new(0x51CE_D5A7);
    let mut out = Vec::with_capacity(POINTS);
    while out.len() < POINTS {
        let x = rng.f64() * 2.0 - 1.0;
        let y = rng.f64() * 2.0 - 1.0;
        let psi = (N * PI * x).cos() * (M * PI * y).cos() - (M * PI * x).cos() * (N * PI * y).cos();
        if rng.f64() < (-(psi / SIGMA).powi(2)).exp() {
            // The plate lies flat; a hair of height so it is not a
            // single plane the camera can edge-on into nothing.
            out.push([x, (rng.f64() - 0.5) * 0.02, y]);
        }
    }
    out
}

/// The Sierpinski tetrahedron by the chaos game: halfway to a random
/// vertex, forever. Apex up.
fn sierpinski() -> Vec<[f64; 3]> {
    let v = [
        [0.0, 1.0, 0.0],
        [0.943, -1.0 / 3.0, 0.0],
        [-0.471, -1.0 / 3.0, 0.816],
        [-0.471, -1.0 / 3.0, -0.816],
    ];
    let mut rng = Rng::new(0x5E1E_A5C0);
    let mut p = [0.0, 0.0, 0.0];
    for _ in 0..20 {
        let t = v[(rng.next() % 4) as usize];
        p = [(p[0] + t[0]) * 0.5, (p[1] + t[1]) * 0.5, (p[2] + t[2]) * 0.5];
    }
    (0..POINTS)
        .map(|_| {
            let t = v[(rng.next() % 4) as usize];
            p = [(p[0] + t[0]) * 0.5, (p[1] + t[1]) * 0.5, (p[2] + t[2]) * 0.5];
            p
        })
        .collect()
}

/// The Menger sponge by the chaos game: one of the twenty sub-cubes that
/// survive each level, a third the size, forever.
fn menger() -> Vec<[f64; 3]> {
    let mut cells = Vec::with_capacity(20);
    for i in -1..=1 {
        for j in -1..=1 {
            for k in -1..=1 {
                let zeros = [i, j, k].iter().filter(|c| **c == 0).count();
                if zeros <= 1 {
                    cells.push([i as f64, j as f64, k as f64]);
                }
            }
        }
    }
    debug_assert_eq!(cells.len(), 20);
    let mut rng = Rng::new(0x3E9C_E251);
    let mut p = [0.0, 0.0, 0.0];
    let step = |p: [f64; 3], c: [f64; 3]| {
        [p[0] / 3.0 + c[0] * 2.0 / 3.0, p[1] / 3.0 + c[1] * 2.0 / 3.0, p[2] / 3.0 + c[2] * 2.0 / 3.0]
    };
    for _ in 0..20 {
        p = step(p, cells[(rng.next() % 20) as usize]);
    }
    (0..POINTS)
        .map(|_| {
            p = step(p, cells[(rng.next() % 20) as usize]);
            p
        })
        .collect()
}

/// The power-eight Mandelbulb's surface: a ray from a random direction
/// marched inward until the point stops escaping, then bisected onto
/// the boundary. Eight iterations is enough for the silhouette and the
/// large lobes, which is what a cloud of sixty-five thousand can show.
fn mandelbulb() -> Vec<[f64; 3]> {
    const ITERATIONS: usize = 8;
    const START: f64 = 1.25;
    const STEP: f64 = 0.06;
    fn inside(c: [f64; 3]) -> bool {
        let [mut x, mut y, mut z] = c;
        for _ in 0..ITERATIONS {
            let r = (x * x + y * y + z * z).sqrt();
            if r > 2.0 {
                return false;
            }
            let theta = (z / r.max(1e-12)).acos();
            let phi = y.atan2(x);
            let r8 = r.powi(8);
            let (t8, p8) = (8.0 * theta, 8.0 * phi);
            x = r8 * t8.sin() * p8.cos() + c[0];
            y = r8 * t8.sin() * p8.sin() + c[1];
            z = r8 * t8.cos() + c[2];
        }
        true
    }
    let scale = |d: [f64; 3], t: f64| [d[0] * t, d[1] * t, d[2] * t];
    let mut rng = Rng::new(0xB01B_B01B);
    let mut out = Vec::with_capacity(POINTS);
    while out.len() < POINTS {
        let d = rng.on_sphere();
        // The origin is inside for every c, so every ray finds the
        // surface: walk in until the first inside point, then bisect
        // between it and the last outside one.
        let mut last_out = START;
        let mut t = START - STEP;
        let mut first_in = None;
        while t > 0.0 {
            if inside(scale(d, t)) {
                first_in = Some(t);
                break;
            }
            last_out = t;
            t -= STEP;
        }
        let Some(mut hi) = first_in else { continue };
        let mut lo = last_out;
        for _ in 0..5 {
            let mid = 0.5 * (lo + hi);
            if inside(scale(d, mid)) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        out.push(orient(scale(d, hi), Frame::ZUp));
    }
    out
}

// --- Plumbing ---------------------------------------------------------

/// Centre the cloud and fit its widest axis to `[-1, 1]` — uniform, so
/// the shape is not squashed — and pack it as white points that take the
/// palette, with no normal so the loader fits one where a surface wants
/// it.
fn finish(points: Vec<[f64; 3]>) -> Vec<Point> {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in &points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let extent = (0..3).fold(0.0f64, |m, i| m.max(hi[i] - lo[i]));
    let scale = if extent > 0.0 { 2.0 / extent } else { 1.0 };
    points
        .into_iter()
        .map(|p| {
            Point::new(
                ((p[0] - centre[0]) * scale) as f32,
                ((p[1] - centre[1]) * scale) as f32,
                ((p[2] - centre[2]) * scale) as f32,
            )
        })
        .collect()
}

/// A small deterministic generator (xorshift64), so the same id makes
/// the same cloud on every machine and every launch.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn on_sphere(&mut self) -> [f64; 3] {
        let u = self.f64() * 2.0 - 1.0;
        let a = self.f64() * TAU;
        let s = (1.0 - u * u).sqrt();
        [s * a.cos(), s * a.sin(), u]
    }

    fn in_ball(&mut self, radius: f64) -> [f64; 3] {
        let d = self.on_sphere();
        let r = radius * self.f64().cbrt();
        [d[0] * r, d[1] * r, d[2] * r]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract every slot relies on: a full slot of finite points,
    /// centred, the widest axis spanning the box, and a shape that
    /// occupies it in more than one direction.
    #[test]
    fn every_generator_fills_its_slot_inside_the_box() {
        for id in IDS {
            let pts = generate(id).unwrap_or_else(|| panic!("{id} did not generate"));
            assert_eq!(pts.len(), POINTS, "{id}");
            let mut hi = [0.0f32; 3];
            for p in &pts {
                for (h, v) in hi.iter_mut().zip(p.pos) {
                    assert!(v.is_finite(), "{id} has a non-finite point");
                    assert!(v.abs() <= 1.001, "{id} leaves the box: {:?}", p.pos);
                    *h = h.max(v.abs());
                }
            }
            let widest = hi.iter().cloned().fold(0.0f32, f32::max);
            assert!(widest > 0.99, "{id} does not fill its box: {hi:?}");
            let wide_axes = hi.iter().filter(|h| **h > 0.15).count();
            assert!(wide_axes >= 2, "{id} collapsed to a line: {hi:?}");
        }
    }

    /// A flow is a path: consecutive points are close. Too coarse a time
    /// step would fail this before it failed the eye.
    #[test]
    fn flows_are_paths_not_scatter() {
        for id in ["thomas", "halvorsen", "dadras", "rossler", "four-wing", "chen"] {
            let pts = generate(id).unwrap();
            let longest = pts
                .windows(2)
                .map(|w| {
                    let d = [w[1].pos[0] - w[0].pos[0], w[1].pos[1] - w[0].pos[1], w[1].pos[2] - w[0].pos[2]];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .fold(0.0f32, f32::max);
            assert!(longest < 0.2, "{id} jumps {longest} between consecutive points");
        }
    }

    /// The same id is the same cloud twice: a `gen:` entry in the
    /// settings must come back as what was on screen.
    #[test]
    fn generation_is_deterministic() {
        for id in ["thomas", "clifford", "chladni", "menger", "hopf"] {
            assert_eq!(generate(id), generate(id), "{id} differs between runs");
        }
        assert!(generate("no such thing").is_none());
    }

    /// The diagonal frame stands (1,1,1) upright, and only rotates.
    #[test]
    fn the_diagonal_frame_is_a_rotation_to_upright() {
        let up = orient([1.0, 1.0, 1.0], Frame::Diagonal);
        assert!((up[0]).abs() < 1e-9 && (up[1] - 3f64.sqrt()).abs() < 1e-9 && up[2].abs() < 1e-9, "{up:?}");
        let v = orient([0.3, -0.7, 0.2], Frame::Diagonal);
        let before = (0.3f64 * 0.3 + 0.7 * 0.7 + 0.2 * 0.2).sqrt();
        let after = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((before - after).abs() < 1e-9, "the frame changed a length");
    }
}
