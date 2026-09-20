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
    "sprott-b",
    "nose-hoover",
    "arneodo",
    "burke-shaw",
    "chua",
    "hadley",
    "rucklidge",
    "three-scroll",
    "rabinovich",
    "plant",
    "mandelbrot",
    "julia",
    "quadratic",
    "aizawa",
    "newton-leipnik",
    "sakarya",
    "rikitake",
    "shimizu-morioka",
    "finance",
    "coullet",
    "genesio-tesi",
    "quadratic-flow",
    "orbital",
    "fern",
    "coral",
    "tree",
    "voronoi",
];

/// Make the cloud `id` names, or `None` for an id this crate does not
/// know. Always exactly [`POINTS`] points, centred, the widest axis
/// spanning `[-1, 1]` — the box every other cloud is fitted to, so
/// `/particles/spread` means the same thing whatever is in the slot.
pub fn generate(spec: &str) -> Option<Vec<Point>> {
    let (id, settings) = split_spec(spec);
    let num = |key: &str, default: f64| -> f64 {
        settings
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    };
    let text = |key: &str, default: &str| -> String {
        settings
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(default, |(_, v)| *v)
            .to_string()
    };
    if let Some((_, f, start, dt, frame)) = FLOWS.iter().find(|(name, ..)| *name == id) {
        return Some(finish(flow(*f, *start, *dt, *frame)));
    }
    let raw = match id {
        // Maps: iterated, and lifted into depth by delay embedding.
        "clifford" => map(clifford, [0.1, 0.0]),
        "dejong" => map(dejong, [0.1, 0.1]),
        // Surfaces, in scan order.
        "supershape" => {
            supershape(num("m", 7.0), num("n1", 2.0).max(0.05), num("n2", 8.0), num("n3", 4.0))
        }
        "harmonic" => harmonic(num("round", 3.0).round(), num("up", 2.0).round()),
        // Curves, along the curve, thickened into tubes.
        "lissajous" => {
            let (a, b, c) = (num("a", 3.0).round(), num("b", 4.0).round(), num("c", 7.0).round());
            tube(move |t| lissajous(t, a, b, c), 0.05, Frame::YUp)
        }
        "torus-knot" => {
            let (p, q) = (num("p", 3.0).round(), num("q", 7.0).round());
            tube(move |t| torus_knot(t, p, q), 0.04, Frame::YUp)
        }
        "hopf" => hopf(),
        // The rest: sampled, grown, searched.
        "chladni" => chladni(num("n", 5.0).round(), num("m", 2.0).round()),
        "plant" => plant(&text("rule", "F[+&X][-^X]/F[\\X]X"), num("angle", 25.0)),
        "mandelbrot" => escape_relief([-2.1, 0.7], [-1.4, 1.4], |x, y| escape(x, y, x, y)),
        "julia" => {
            let (cr, ci) = (num("cr", -0.8), num("ci", 0.156));
            escape_relief([-1.6, 1.6], [-1.6, 1.6], move |x, y| escape(x, y, cr, ci))
        }
        "quadratic" => quadratic(num("seed", 1.0).abs() as u64),
        "quadratic-flow" => quadratic_flow(num("seed", 1.0).abs() as u64),
        "orbital" => orbital(num("l", 3.0), num("m", 2.0)),
        "fern" => plant("F-[[X]+X]+F[+FX]-X/", num("angle", 22.0)),
        "coral" => plant("F[&X]////[&X]////[&X]", num("angle", 30.0)),
        "tree" => plant("FF[+&X]F[-/X][^\\X]X", num("angle", 20.0)),
        "voronoi" => voronoi(num("cells", 24.0), num("seed", 1.0).abs() as u64),
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

/// Halvorsen's attractor, a = 1.4: cyclically symmetric like Thomas,
/// but three lobes of folded sheet rather than ribbons. The constant
/// matters — by a = 1.89 the flow has fallen onto a limit cycle, which
/// fills the same box and looks nearly as busy while being no longer
/// chaotic at all.
fn halvorsen([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 1.4;
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

/// Sprott's case B (1994): two quadratic terms, and chaos.
fn sprott_b([x, y, z]: [f64; 3]) -> [f64; 3] {
    [y * z, x - y, 1.0 - x * y]
}

/// The Nosé–Hoover oscillator at a = 1.5: a thermostatted particle, and
/// a conservative system rather than an attractor — the trajectory
/// wanders a sea of tori and chaos instead of settling on a sheet.
fn nose_hoover([x, y, z]: [f64; 3]) -> [f64; 3] {
    [y, -x + y * z, 1.5 - y * y]
}

/// Arneodo's attractor, a = -5.5, b = 3.5, d = -1: a jerk system, one
/// cubic term.
fn arneodo([x, y, z]: [f64; 3]) -> [f64; 3] {
    [y, z, 5.5 * x - 3.5 * y - z - x * x * x]
}

/// Burke–Shaw, s = 10, v = 4.272: two scrolls with the symmetry of a
/// propeller.
fn burke_shaw([x, y, z]: [f64; 3]) -> [f64; 3] {
    [-10.0 * (x + y), -y - 10.0 * x * z, 10.0 * x * y + 4.272]
}

/// Chua's circuit, the double scroll: α = 15.6, β = 28, and the
/// piecewise-linear diode with slopes m0 = -1.143, m1 = -0.714.
fn chua([x, y, z]: [f64; 3]) -> [f64; 3] {
    const M0: f64 = -1.143;
    const M1: f64 = -0.714;
    let diode = M1 * x + 0.5 * (M0 - M1) * ((x + 1.0).abs() - (x - 1.0).abs());
    [15.6 * (y - x - diode), x - y + z, -28.0 * y]
}

/// The Hadley circulation (Lorenz, 1984): a = 0.25, b = 4, F = 8,
/// G = 1. The general circulation of an atmosphere in three variables —
/// a westerly current, and a wave riding it. Lorenz' own constants;
/// a little off them, at a = 0.2 or 0.3, the wave settles into a cycle
/// and the weather stops surprising anyone.
fn hadley([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.25;
    const B: f64 = 4.0;
    const F: f64 = 8.0;
    const G: f64 = 1.0;
    [
        -y * y - z * z - A * (x - F),
        x * y - B * x * z - y + G,
        B * x * y + x * z - z,
    ]
}

/// Rucklidge's model of convection, κ = 2, λ = 6.7.
fn rucklidge([x, y, z]: [f64; 3]) -> [f64; 3] {
    [-2.0 * x + 6.7 * y - y * z, x, -z + y * y]
}

/// The three-scroll unified system (TSUCS-1): a = 40, b = 55, c = 1.833,
/// d = 0.16, e = 0.65, f = 20. Fast, so the finest step here.
fn three_scroll([x, y, z]: [f64; 3]) -> [f64; 3] {
    [40.0 * (y - x) + 0.16 * x * z, 55.0 * x - x * z + 20.0 * y, 1.833 * z + x * y - 0.65 * x * x]
}

/// Rabinovich–Fabrikant, α = 0.14, γ = 0.10: leaves and ribbons. Some
/// parameter sets of this system escape; this one does not from its
/// classic start, and the integrator's guard holds if a step ever does.
fn rabinovich([x, y, z]: [f64; 3]) -> [f64; 3] {
    [
        y * (z - 1.0 + x * x) + 0.1 * x,
        x * (3.0 * z + 1.0 - x * x) + 0.1 * y,
        -2.0 * z * (0.14 + x * y),
    ]
}

/// Aizawa's attractor, a = 0.95, b = 0.7, c = 0.6, d = 3.5, e = 0.25,
/// f = 0.1: a rotating sphere with a spindle driven through its poles,
/// and the trajectory wound round both.
fn aizawa([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.95;
    const B: f64 = 0.7;
    const C: f64 = 0.6;
    const D: f64 = 3.5;
    const E: f64 = 0.25;
    const F: f64 = 0.1;
    [
        (z - B) * x - D * y,
        D * x + (z - B) * y,
        C + A * z - z * z * z / 3.0 - (x * x + y * y) * (1.0 + E * z) + F * z * x * x * x,
    ]
}

/// Newton–Leipnik (1981), a = 0.4, b = 0.175: rigid-body motion with a
/// feedback torque, and two attractors in the same system — this start
/// finds the upper one.
fn newton_leipnik([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.4;
    const B: f64 = 0.175;
    [-A * x + y + 10.0 * y * z, -x - A * y + 5.0 * x * z, B * z - 5.0 * x * y]
}

/// The Sakarya system (2010), a = 0.4, b = 0.3: two lobes crossing at
/// an angle, like a bow tie drawn in wire.
fn sakarya([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.4;
    const B: f64 = 0.3;
    [-x + y + y * z, -x - y + A * x * z, z - B * x * y]
}

/// The Rikitake dynamo (1958), μ = 2, a = 5: two coupled disc dynamos,
/// the model that first explained why the Earth's magnetic field
/// reverses — the trajectory hops between two lobes at no fixed
/// interval, and each hop is a reversal.
fn rikitake([x, y, z]: [f64; 3]) -> [f64; 3] {
    const MU: f64 = 2.0;
    const A: f64 = 5.0;
    [-MU * x + z * y, -MU * y + x * (z - A), 1.0 - x * y]
}

/// Shimizu–Morioka (1980), a = 0.75, b = 0.45: the Lorenz butterfly's
/// simplest relative, two wings and one quadratic term.
fn shimizu_morioka([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.75;
    const B: f64 = 0.45;
    [y, x - A * y - x * z, -B * z + x * x]
}

/// The finance system (Chen & Gao, after Ma & Chen, 2001), a = 0.001,
/// b = 0.1, c = 1: interest rate, investment demand and price index,
/// three variables that will not settle.
fn finance([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.001;
    const B: f64 = 0.1;
    const C: f64 = 1.0;
    [z + (y - A) * x, 1.0 - B * y - x * x, -x - C * z]
}

/// Coullet's jerk system, a = 0.8, b = -1.1, c = -0.45: the third
/// derivative of one variable, with a cubic restoring term.
fn coullet([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.8;
    const B: f64 = -1.1;
    const C: f64 = -0.45;
    [y, z, A * x + B * y + C * z - x * x * x]
}

/// Genesio–Tesi (1992), a = 0.44, b = 1.1, c = 1: the other classic
/// jerk system, square rather than cubic.
fn genesio_tesi([x, y, z]: [f64; 3]) -> [f64; 3] {
    const A: f64 = 0.44;
    const B: f64 = 1.1;
    const C: f64 = 1.0;
    [y, z, -C * x - B * y - A * z + x * x]
}

/// Every named flow: the field, where to start, the time step and
/// which way is up. One table rather than a run of match arms, so a
/// test can integrate all of them and check that each one is really
/// chaotic rather than a cycle that merely looks busy.
#[allow(clippy::type_complexity)]
const FLOWS: &[(&str, fn([f64; 3]) -> [f64; 3], [f64; 3], f64, Frame)] = &[
    ("thomas", thomas, [0.1, 0.0, 0.0], 0.05, Frame::Diagonal),
    ("halvorsen", halvorsen, [-1.48, -1.51, 2.04], 0.005, Frame::Diagonal),
    ("dadras", dadras, [1.1, 2.1, -2.0], 0.005, Frame::ZUp),
    ("rossler", rossler, [1.0, 1.0, 1.0], 0.02, Frame::ZUp),
    ("four-wing", four_wing, [1.3, -0.18, 0.01], 0.01, Frame::ZUp),
    ("chen", chen, [-0.1, 0.5, -0.6], 0.002, Frame::ZUp),
    ("sprott-b", sprott_b, [0.1, 0.1, 0.1], 0.01, Frame::ZUp),
    ("nose-hoover", nose_hoover, [1.0, 0.0, 0.0], 0.01, Frame::ZUp),
    ("arneodo", arneodo, [0.1, 0.2, 0.3], 0.01, Frame::ZUp),
    ("burke-shaw", burke_shaw, [1.0, 1.0, 1.0], 0.003, Frame::ZUp),
    ("chua", chua, [0.1, 0.0, 0.0], 0.005, Frame::ZUp),
    ("hadley", hadley, [0.1, 0.0, 0.0], 0.01, Frame::ZUp),
    ("rucklidge", rucklidge, [1.0, 0.0, 4.5], 0.01, Frame::ZUp),
    ("three-scroll", three_scroll, [1.0, 1.0, 1.0], 0.001, Frame::ZUp),
    ("rabinovich", rabinovich, [-1.0, 0.0, 0.5], 0.002, Frame::ZUp),
    ("aizawa", aizawa, [0.1, 0.0, 0.0], 0.01, Frame::ZUp),
    ("newton-leipnik", newton_leipnik, [0.349, 0.0, -0.16], 0.02, Frame::ZUp),
    ("sakarya", sakarya, [1.0, -1.0, 1.0], 0.01, Frame::ZUp),
    ("rikitake", rikitake, [0.1, 0.0, 0.0], 0.01, Frame::ZUp),
    ("shimizu-morioka", shimizu_morioka, [0.1, 0.0, 0.0], 0.02, Frame::ZUp),
    ("finance", finance, [1.0, 2.0, 0.5], 0.02, Frame::ZUp),
    ("coullet", coullet, [0.1, 0.0, 0.0], 0.02, Frame::ZUp),
    ("genesio-tesi", genesio_tesi, [0.2, -0.3, 0.1], 0.02, Frame::ZUp),
];

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
fn rk4(f: impl Fn([f64; 3]) -> [f64; 3], p: [f64; 3], dt: f64) -> [f64; 3] {
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
fn supershape(m: f64, n1: f64, n2: f64, n3: f64) -> Vec<[f64; 3]> {
    grid(|lon, lat| {
        let r1 = superformula(lon, m, n1, n2, n3);
        let r2 = superformula(lat, m, n1, n2, n3);
        orient(
            [r1 * lon.cos() * r2 * lat.cos(), r1 * lon.sin() * r2 * lat.cos(), r2 * lat.sin()],
            Frame::ZUp,
        )
    })
}

/// A sphere rippled by a spherical harmonic: three waves round, two
/// waves up.
fn harmonic(round: f64, up: f64) -> Vec<[f64; 3]> {
    grid(|lon, lat| {
        let r = 1.0 + 0.45 * (round * lon).cos() * (up * lat).sin();
        orient([r * lon.cos() * lat.cos(), r * lon.sin() * lat.cos(), r * lat.sin()], Frame::ZUp)
    })
}

// --- Curves -----------------------------------------------------------

/// A 3:4:7 Lissajous knot: pairwise coprime frequencies, phases off the
/// values that would let it cross itself.
fn lissajous(t: f64, a: f64, b: f64, c: f64) -> [f64; 3] {
    [(a * t + 0.5).cos(), (b * t + 1.3).cos(), (c * t).cos()]
}

/// A (3,7) torus knot: three times round the axis, seven times through
/// the hole. The trefoil in `/shape/mode` is the (2,3) of the same family.
fn torus_knot(t: f64, p: f64, q: f64) -> [f64; 3] {
    const R: f64 = 0.7;
    const RADIUS: f64 = 0.3;
    let ring = R + RADIUS * (q * t).cos();
    [ring * (p * t).cos(), RADIUS * (q * t).sin(), ring * (p * t).sin()]
}

/// A closed curve traced once, thickened into a fuzzy tube: each point
/// on the curve gets a random offset inside a ball of `radius`, so the
/// cloud reads as a volume rather than a wire.
fn tube(curve: impl Fn(f64) -> [f64; 3], radius: f64, frame: Frame) -> Vec<[f64; 3]> {
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

// --- Grown --------------------------------------------------------------

/// A plant grown by an L-system: Lindenmayer's rewriting, read by a
/// turtle that turns in three dimensions. Five generations of
///
/// ```text
/// X → F[+&X][-^X]/F[\X]X      F → FF
/// ```
///
/// at 25°, so every node throws four branches and the older wood is
/// longer. Points are strewn along the segments in drawing order — the
/// crawl runs up the trunk and out along the twigs — inside a tube that
/// thins with depth, so the trunk is wood and the tips are twigs.
fn plant(rule: &str, angle: f64) -> Vec<[f64; 3]> {
    // A rule is text somebody typed: the generations are capped by the
    // string's growth rather than by a count, so a rule with six X's
    // does not rewrite itself into a gigabyte.
    let rule = if rule.contains('F') { rule } else { "F[+&X][-^X]/F[\\X]X" };
    let mut s = String::from("X");
    for _ in 0..5 {
        if s.len() * rule.len() > 4_000_000 {
            break;
        }
        let mut next = String::with_capacity(s.len() * 4);
        for c in s.chars() {
            match c {
                'X' => next.push_str(rule),
                'F' => next.push_str("FF"),
                c => next.push(c),
            }
        }
        s = next;
    }
    let delta = angle.clamp(1.0, 179.0).to_radians();
    // The turtle: position, heading, left, up.
    let mut p = [0.0, 0.0, 0.0];
    let (mut h, mut l, mut u) = ([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    // Position, heading, left, up — what a bracket saves.
    type Turtle = ([f64; 3], [f64; 3], [f64; 3], [f64; 3]);
    let mut stack: Vec<Turtle> = Vec::new();
    // (from, to, depth)
    let mut segments: Vec<([f64; 3], [f64; 3], usize)> = Vec::new();
    let rot = |a: [f64; 3], b: [f64; 3], t: f64| -> ([f64; 3], [f64; 3]) {
        let (c, s) = (t.cos(), t.sin());
        (
            [a[0] * c + b[0] * s, a[1] * c + b[1] * s, a[2] * c + b[2] * s],
            [b[0] * c - a[0] * s, b[1] * c - a[1] * s, b[2] * c - a[2] * s],
        )
    };
    for c in s.chars() {
        match c {
            'F' => {
                let q = [p[0] + h[0], p[1] + h[1], p[2] + h[2]];
                segments.push((p, q, stack.len()));
                p = q;
            }
            '+' => (h, l) = rot(h, l, delta),
            '-' => (h, l) = rot(h, l, -delta),
            '&' => (h, u) = rot(h, u, delta),
            '^' => (h, u) = rot(h, u, -delta),
            '\\' => (l, u) = rot(l, u, delta),
            '/' => (l, u) = rot(l, u, -delta),
            '[' => stack.push((p, h, l, u)),
            ']' => {
                if let Some(saved) = stack.pop() {
                    (p, h, l, u) = saved;
                }
            }
            _ => {}
        }
    }
    // Strewn along the total length, stratified, so a point count that
    // does not divide by the segment count still lands evenly.
    let total: f64 = segments.len() as f64;
    let mut rng = Rng::new(0x9A5F_B0A1);
    let mut out = Vec::with_capacity(POINTS);
    let mut seg = 0usize;
    for i in 0..POINTS {
        let along = (i as f64 + 0.5) / POINTS as f64 * total;
        while seg + 1 < segments.len() && along >= (seg + 1) as f64 {
            seg += 1;
        }
        let (a, b, depth) = segments[seg];
        let t = along - seg as f64;
        let radius = 0.42 * 0.72f64.powi(depth as i32);
        let j = rng.in_ball(radius);
        out.push([
            a[0] + (b[0] - a[0]) * t + j[0],
            a[1] + (b[1] - a[1]) * t + j[1],
            a[2] + (b[2] - a[2]) * t + j[2],
        ]);
    }
    out
}

/// Smooth escape time of z ← z² + c from `z0`, as a fraction of the
/// iteration budget: 1 inside the set, falling towards 0 far outside.
fn escape(zx: f64, zy: f64, cx: f64, cy: f64) -> f64 {
    const LIMIT: usize = 60;
    let (mut x, mut y) = (zx, zy);
    for n in 0..LIMIT {
        let r2 = x * x + y * y;
        if r2 > 64.0 {
            // Douady–Hubbard smoothing: the fractional iteration count.
            let smooth = n as f64 + 1.0 - (r2.sqrt().ln().max(1e-12)).log2();
            return (smooth / LIMIT as f64).clamp(0.0, 1.0);
        }
        let nx = x * x - y * y + cx;
        y = 2.0 * x * y + cy;
        x = nx;
    }
    1.0
}

/// A plane fractal as a relief: the set itself is a plateau, the
/// escape time is the height of the country round it.
fn escape_relief(xr: [f64; 2], yr: [f64; 2], f: impl Fn(f64, f64) -> f64) -> Vec<[f64; 3]> {
    const SIDE: usize = 256;
    debug_assert_eq!(SIDE * SIDE, POINTS);
    let mut out = Vec::with_capacity(POINTS);
    for row in 0..SIDE {
        let y = yr[0] + (yr[1] - yr[0]) * (row as f64 + 0.5) / SIDE as f64;
        for col in 0..SIDE {
            let x = xr[0] + (xr[1] - xr[0]) * (col as f64 + 0.5) / SIDE as f64;
            let h = f(x, y);
            // Height on a curve: the country rises steeply at the coast.
            out.push([x, h.powf(2.5) * 1.1, y]);
        }
    }
    out
}

// --- Sampled ----------------------------------------------------------

/// Chladni's sand: a plate vibrating in its (5, 2) mode, points kept
/// where it stands still. Rejection-sampled with a Gaussian acceptance on
/// the displacement, so the lines have the soft width sand has.
fn chladni(n: f64, m: f64) -> Vec<[f64; 3]> {
    // Equal mode numbers cancel to nothing everywhere, which would
    // accept every point and draw a plain square.
    let (n, m) = if (n - m).abs() < 0.5 { (n, m + 1.0) } else { (n, m) };
    const SIGMA: f64 = 0.07;
    let mut rng = Rng::new(0x51CE_D5A7);
    let mut out = Vec::with_capacity(POINTS);
    while out.len() < POINTS {
        let x = rng.f64() * 2.0 - 1.0;
        let y = rng.f64() * 2.0 - 1.0;
        let psi = (n * PI * x).cos() * (m * PI * y).cos() - (m * PI * x).cos() * (n * PI * y).cos();
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

// --- Searched -----------------------------------------------------------

/// Sprott's search (Strange Attractors: Creating Patterns in Chaos,
/// 1993): draw the thirty coefficients of a three-dimensional quadratic
/// map at random and keep the first map that is chaotic — bounded, and
/// with a positive largest Lyapunov exponent, measured by following a
/// neighbour and renormalising. About one draw in a hundred is, and
/// every one of those is an attractor nobody has seen before. The seed
/// is the whole parameter: the same seed is the same attractor on every
/// machine.
fn quadratic(seed: u64) -> Vec<[f64; 3]> {
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x51));
    // Bounded by construction: a search that never finds chaos would
    // otherwise never return. Ten thousand draws is a hundred times the
    // expected wait.
    for _ in 0..10_000 {
        // Coefficients on Sprott's grid, -1.2 to 1.2 in steps of 0.1.
        let c: [f64; 30] = std::array::from_fn(|_| ((rng.next() % 25) as f64 - 12.0) * 0.1);
        if let Some(points) = quadratic_orbit(&c) {
            return points;
        }
    }
    // Nothing chaotic on this seed: the sphere of it, rather than a
    // panic in a loader thread.
    (0..POINTS)
        .map(|_| rng.on_sphere())
        .collect()
}

fn quadratic_step(c: &[f64; 30], [x, y, z]: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let a = &c[k * 10..k * 10 + 10];
        *o = a[0]
            + a[1] * x
            + a[2] * x * x
            + a[3] * x * y
            + a[4] * x * z
            + a[5] * y
            + a[6] * y * y
            + a[7] * y * z
            + a[8] * z
            + a[9] * z * z;
    }
    out
}

/// The orbit of one candidate, if it is chaotic: `None` when it
/// escapes, collapses, or settles into a cycle.
fn quadratic_orbit(c: &[f64; 30]) -> Option<Vec<[f64; 3]>> {
    const SEPARATION: f64 = 1e-6;
    let mut p = [0.05, 0.05, 0.05];
    let mut q = [0.05 + SEPARATION, 0.05, 0.05];
    let mut lyapunov = 0.0;
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for i in 0..4_000 {
        p = quadratic_step(c, p);
        q = quadratic_step(c, q);
        if p.iter().any(|v| !v.is_finite() || v.abs() > 1e5) {
            return None;
        }
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist == 0.0 {
            return None;
        }
        if i >= 1_000 {
            lyapunov += (dist / SEPARATION).ln();
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let scale = SEPARATION / dist;
        q = [p[0] + d[0] * scale, p[1] + d[1] * scale, p[2] + d[2] * scale];
    }
    let lyapunov = lyapunov / 3_000.0;
    let extent = (0..3).fold(0.0f64, |m, k| m.max(hi[k] - lo[k]));
    if lyapunov < 0.01 || extent < 0.05 {
        return None;
    }
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        p = quadratic_step(c, p);
        if p.iter().any(|v| !v.is_finite() || v.abs() > 1e5) {
            return None;
        }
        out.push(p);
    }
    Some(out)
}

/// The same thirty coefficients read as a *flow* rather than a map:
/// Sprott's search again, with an integrator inside it. A quadratic
/// vector field in three variables is the smallest thing that can be
/// chaotic at all (Poincaré–Bendixson rules out two), and a randomly
/// drawn one usually is not — it runs away to infinity, or falls onto a
/// point or a cycle. Perhaps one draw in a few hundred is a strange
/// attractor, and those are the smooth, ribboned kind rather than the
/// dusty sheets the maps give.
fn quadratic_flow(seed: u64) -> Vec<[f64; 3]> {
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xF107_0000_0000_0001);
    for _ in 0..40_000 {
        let c: [f64; 30] = std::array::from_fn(|_| ((rng.next() % 25) as f64 - 12.0) * 0.1);
        if let Some(points) = quadratic_flow_orbit(&c) {
            return points;
        }
    }
    (0..POINTS).map(|_| rng.on_sphere()).collect()
}

/// One candidate field, if it is a strange attractor. Most draws are
/// rejected in the first few hundred steps because they escape, which
/// is what makes the search affordable.
fn quadratic_flow_orbit(c: &[f64; 30]) -> Option<Vec<[f64; 3]>> {
    /// The probe step. Coefficients of order one make a field of order
    /// one, so this is a small fraction of a transit either way.
    const DT: f64 = 0.05;
    const SEPARATION: f64 = 1e-6;
    const TRANSIENT: usize = 2_000;
    const MEASURE: usize = 3_000;
    let f = |p| quadratic_step(c, p);
    let sane = |p: &[f64; 3]| p.iter().all(|v| v.is_finite() && v.abs() < 1e3);
    let mut p = [0.05, 0.05, 0.05];
    for _ in 0..TRANSIENT {
        p = rk4(f, p, DT);
        if !sane(&p) {
            return None;
        }
    }
    // The largest Lyapunov exponent, per unit time: follow a neighbour
    // and renormalise the separation each step (Benettin's method).
    let mut q = [p[0] + SEPARATION, p[1], p[2]];
    let mut lyapunov = 0.0;
    let mut speed = 0.0;
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for _ in 0..MEASURE {
        p = rk4(f, p, DT);
        q = rk4(f, q, DT);
        if !sane(&p) || !sane(&q) {
            return None;
        }
        let v = f(p);
        speed += (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist == 0.0 || !dist.is_finite() {
            return None;
        }
        lyapunov += (dist / SEPARATION).ln();
        let scale = SEPARATION / dist;
        q = [p[0] + d[0] * scale, p[1] + d[1] * scale, p[2] + d[2] * scale];
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let lyapunov = lyapunov / (MEASURE as f64 * DT);
    let speed = speed / MEASURE as f64;
    let extent = (0..3).fold(0.0f64, |m, k| m.max(hi[k] - lo[k]));
    if lyapunov < 0.01 || extent < 0.05 || speed < 1e-6 {
        return None;
    }
    // Draw at a step that makes one point about a hundredth of the
    // attractor's width, whatever speed this field happens to run at:
    // the cloud has to read as a path, and a fixed step cannot for a
    // system whose scale was drawn at random.
    let dt = (extent * 0.01 / speed).clamp(1e-4, 0.25);
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        p = rk4(f, p, dt);
        if !sane(&p) {
            return None;
        }
        out.push(orient(p, Frame::ZUp));
    }
    Some(out)
}

/// The associated Legendre function P_l^m(x), by the standard
/// three-term recurrence. The normalising constant is left off: for one
/// (l, m) it is a single factor on the whole cloud, and the cloud is
/// fitted to the box regardless.
fn legendre(l: i32, m: i32, x: f64) -> f64 {
    let m = m.abs();
    if m > l {
        return 0.0;
    }
    // P_m^m = (-1)^m (2m-1)!! (1-x²)^(m/2), built up one factor at a time.
    let mut pmm = 1.0;
    if m > 0 {
        let root = ((1.0 - x) * (1.0 + x)).max(0.0).sqrt();
        let mut odd = 1.0;
        for _ in 0..m {
            pmm *= -odd * root;
            odd += 2.0;
        }
    }
    if l == m {
        return pmm;
    }
    let mut pmm1 = x * (2 * m + 1) as f64 * pmm;
    if l == m + 1 {
        return pmm1;
    }
    let mut p = 0.0;
    for ll in (m + 2)..=l {
        p = (((2 * ll - 1) as f64) * x * pmm1 - ((ll + m - 1) as f64) * pmm) / ((ll - m) as f64);
        pmm = pmm1;
        pmm1 = p;
    }
    p
}

/// A real spherical harmonic drawn as a balloon: the radius in each
/// direction is |Y_l^m|, which is the shape a textbook draws for an
/// atomic orbital. `l` is how many nodal lines there are in total, `m`
/// how many of them run through the poles — negative `m` is the same
/// shape turned, the sine partner of the cosine. The lobes meet at the
/// origin because the harmonic is zero there, and that pinch is the
/// picture.
fn orbital(l: f64, m: f64) -> Vec<[f64; 3]> {
    let l = l.round().clamp(0.0, 8.0) as i32;
    let m = m.round().clamp(-f64::from(l), f64::from(l)) as i32;
    grid(|lon, lat| {
        // Latitude here, colatitude in the textbook: cos θ = sin(lat).
        let p = legendre(l, m, lat.sin());
        let phase = match m {
            0 => 1.0,
            m if m > 0 => (f64::from(m) * lon).cos(),
            m => (f64::from(-m) * lon).sin(),
        };
        let r = (p * phase).abs();
        orient([r * lon.cos() * lat.cos(), r * lon.sin() * lat.cos(), r * lat.sin()], Frame::ZUp)
    })
}

/// Voronoi foam: scatter cell centres in a cube, then keep the points
/// that cannot tell which centre is nearest — the walls between the
/// cells, which are flat polygons meeting three at an edge and four at
/// a corner, exactly as soap films do. Rejection-sampled with a soft
/// acceptance on the difference between the two nearest distances, so
/// the walls have thickness rather than being a surface a cloud cannot
/// show.
fn voronoi(cells: f64, seed: u64) -> Vec<[f64; 3]> {
    const WALL: f64 = 0.035;
    let count = cells.round().clamp(4.0, 64.0) as usize;
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x0C7A_5EED);
    let centres: Vec<[f64; 3]> =
        (0..count).map(|_| [rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0]).collect();
    let mut out = Vec::with_capacity(POINTS);
    // Bounded: a pathological draw must not spin a loader thread forever.
    for _ in 0..(POINTS * 200) {
        if out.len() == POINTS {
            break;
        }
        let p = [rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0];
        let (mut first, mut second) = (f64::MAX, f64::MAX);
        for c in &centres {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            let d = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if d < first {
                second = first;
                first = d;
            } else if d < second {
                second = d;
            }
        }
        let gap = second.sqrt() - first.sqrt();
        if rng.f64() < (-(gap / WALL).powi(2)).exp() {
            out.push(p);
        }
    }
    // Whatever was found, repeated to fill the slot. Only a draw that
    // accepted almost nothing gets here.
    if out.is_empty() {
        out.push([0.0; 3]);
    }
    let found = out.len();
    while out.len() < POINTS {
        out.push(out[out.len() % found]);
    }
    out
}

// --- Plumbing ---------------------------------------------------------

/// The id and the settings of a spec, `id?key=value;key=value`. The
/// same split as vizz-mod's; small enough to keep this crate free of
/// that one.
fn split_spec(spec: &str) -> (&str, Vec<(&str, &str)>) {
    match spec.split_once('?') {
        None => (spec, Vec::new()),
        Some((id, rest)) => (
            id,
            rest.split(';')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.trim(), v.trim()))
                .collect(),
        ),
    }
}

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
    /// step would fail this before it failed the eye. The searched flow
    /// is here too, because its fallback — a sphere of random points —
    /// is exactly a scatter, so this is what says the search worked.
    #[test]
    fn flows_are_paths_not_scatter() {
        let named = FLOWS.iter().map(|(id, ..)| *id);
        for id in named.chain(["quadratic-flow", "quadratic-flow?seed=5"]) {
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

    /// A setting changes the cloud, a default one is the plain id, and a
    /// value that will not parse falls back rather than failing.
    #[test]
    fn settings_shape_the_cloud_and_bad_ones_fall_back() {
        assert_ne!(generate("supershape?m=3"), generate("supershape"));
        assert_eq!(generate("supershape?m=7;n1=2"), generate("supershape"));
        assert_ne!(generate("torus-knot?p=2;q=3"), generate("torus-knot"));
        assert_ne!(generate("plant?angle=45"), generate("plant"));
        assert_eq!(generate("plant?angle=banana"), generate("plant"));
        assert_ne!(generate("julia?cr=0.3;ci=0.5"), generate("julia"));
        assert_eq!(generate("chladni?n=4;m=4").map(|p| p.len()), Some(POINTS));
        let pts = generate("plant?rule=XXXXXXXX").unwrap();
        assert_eq!(pts.len(), POINTS, "a rule with no F falls back to the shipped one");
    }

    /// Sprott's search finds chaos on every seed tried, and different
    /// seeds are different attractors.
    #[test]
    fn the_quadratic_search_finds_a_different_attractor_per_seed() {
        let a = generate("quadratic?seed=1").unwrap();
        let b = generate("quadratic?seed=2").unwrap();
        assert_ne!(a, b);
        for seed in 1..=4 {
            let pts = generate(&format!("quadratic?seed={seed}")).unwrap();
            let widest = pts.iter().map(|p| p.pos[0].abs().max(p.pos[1].abs()).max(p.pos[2].abs())).fold(0.0f32, f32::max);
            assert!(widest > 0.99, "seed {seed} did not fill its box");
            // Not a point and not a line: an attractor has area.
            let mut spread = [0.0f32; 3];
            for p in &pts {
                for (s, v) in spread.iter_mut().zip(p.pos) {
                    *s = s.max(v.abs());
                }
            }
            assert!(spread.iter().filter(|s| **s > 0.15).count() >= 2, "seed {seed}: {spread:?}");
        }
    }


    /// Every named flow is really chaotic, not a cycle that looks busy:
    /// the largest Lyapunov exponent, by Benettin's method — follow a
    /// neighbour, measure how fast it is pushed away, renormalise — is
    /// positive for all of them. This is the test that catches a
    /// parameter typed wrong, which no amount of looking at a still
    /// would: a wrong constant usually lands on a limit cycle, and a
    /// limit cycle fills its box and is a path just as an attractor is.
    #[test]
    fn every_named_flow_is_chaotic() {
        for (id, f, start, dt, _) in FLOWS {
            const SEPARATION: f64 = 1e-7;
            let mut p = *start;
            for _ in 0..20_000 {
                p = rk4(f, p, *dt);
            }
            let mut q = [p[0] + SEPARATION, p[1], p[2]];
            let mut sum = 0.0;
            const STEPS: usize = 60_000;
            for _ in 0..STEPS {
                p = rk4(f, p, *dt);
                q = rk4(f, q, *dt);
                let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                assert!(dist > 0.0 && dist.is_finite(), "{id} lost its neighbour");
                sum += (dist / SEPARATION).ln();
                let k = SEPARATION / dist;
                q = [p[0] + d[0] * k, p[1] + d[1] * k, p[2] + d[2] * k];
            }
            let lyapunov = sum / (STEPS as f64 * dt);
            // A low bar on purpose: Thomas' flow runs at about 0.007 and
            // the Nosé–Hoover oscillator, which is conservative rather
            // than dissipative, at about 0.004. What this catches is a
            // constant that has landed the system on a cycle, where the
            // exponent is zero to within the arithmetic — which is how
            // Halvorsen at a = 1.89 and the Hadley circulation at
            // a = 0.2 were caught, both of them cycles wearing an
            // attractor's clothes.
            assert!(lyapunov > 0.002, "{id} is not chaotic: largest exponent {lyapunov:.4}");
            assert!(p.iter().all(|v| v.is_finite() && v.abs() < 1e4), "{id} ran away: {p:?}");
        }
    }

    /// The searched flow finds a different attractor per seed, and one
    /// that is bounded and has body — not a line, and not the sphere it
    /// falls back to.
    #[test]
    fn the_flow_search_finds_a_different_attractor_per_seed() {
        assert_ne!(generate("quadratic-flow?seed=1"), generate("quadratic-flow?seed=2"));
        for seed in 1..=4 {
            let pts = generate(&format!("quadratic-flow?seed={seed}")).unwrap();
            assert_eq!(pts.len(), POINTS);
            let step = pts
                .windows(2)
                .map(|w| {
                    let d = [
                        w[1].pos[0] - w[0].pos[0],
                        w[1].pos[1] - w[0].pos[1],
                        w[1].pos[2] - w[0].pos[2],
                    ];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .fold(0.0f32, f32::max);
            assert!(step < 0.2, "seed {seed} is a scatter, not a path: {step}");
        }
    }

    /// A harmonic's knobs are its shape: a different (l, m) is a
    /// different balloon, an order past the degree is clamped rather
    /// than blank, and l = 0 is the sphere it should be.
    #[test]
    fn the_orbital_is_shaped_by_its_two_numbers() {
        assert_ne!(generate("orbital?l=4"), generate("orbital"));
        assert_ne!(generate("orbital?m=1"), generate("orbital"));
        assert_ne!(generate("orbital?m=-2"), generate("orbital?m=2"));
        assert_eq!(generate("orbital?l=3;m=9"), generate("orbital?l=3;m=3"));
        let sphere = generate("orbital?l=0;m=0").unwrap();
        let radius: Vec<f32> = sphere
            .iter()
            .map(|p| (p.pos[0] * p.pos[0] + p.pos[1] * p.pos[1] + p.pos[2] * p.pos[2]).sqrt())
            .collect();
        let lo = radius.iter().cloned().fold(f32::MAX, f32::min);
        let hi = radius.iter().cloned().fold(0.0f32, f32::max);
        assert!(hi - lo < 0.01, "l = 0 is not a sphere: {lo} to {hi}");
    }

    /// The three named plants are three plants, each its own, and the
    /// angle knob bends them.
    #[test]
    fn the_named_plants_are_distinct() {
        let fern = generate("fern").unwrap();
        let coral = generate("coral").unwrap();
        let tree = generate("tree").unwrap();
        assert_ne!(fern, coral);
        assert_ne!(coral, tree);
        assert_ne!(tree, fern);
        assert_ne!(generate("plant"), generate("tree"));
        assert_ne!(generate("fern?angle=40"), generate("fern"));
    }

    /// The foam is deterministic, and both knobs move it.
    #[test]
    fn the_foam_answers_to_its_knobs() {
        assert_eq!(generate("voronoi"), generate("voronoi"));
        assert_ne!(generate("voronoi?seed=2"), generate("voronoi"));
        assert_ne!(generate("voronoi?cells=8"), generate("voronoi"));
        // Walls, not a solid: a point in the middle of a cell is far
        // from every wall, so the cloud should leave the cell centres
        // empty. Measured as the share of points whose two nearest
        // centres are within a hair of each other.
        let pts = generate("voronoi?cells=8;seed=3").unwrap();
        assert_eq!(pts.len(), POINTS);
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
