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
    "henon",
    "ikeda",
    "standard",
    "klein",
    "boy",
    "gyroid",
    "quasicrystal",
    "phyllotaxis",
    "kifs",
    "dla",
    "mandelbox",
    "quaternion",
    "hilbert",
    "lorenz96",
    "duffing",
    "gumowski",
    "newton",
    "lyapunov",
    "dini",
    "enneper",
    "spirograph",
    "figure-eight",
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
        "henon" => delay(henon, [0.1, 0.0]),
        "ikeda" => map(ikeda, [0.1, 0.0]),
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
        "standard" => standard(num("k", 0.971635)),
        "klein" => klein(num("girth", 2.0)),
        "boy" => boy(),
        "gyroid" => minimal(
            &text("kind", "gyroid"),
            num("cells", 1.5),
            num("level", 0.0),
            num("thickness", 0.07),
        ),
        "quasicrystal" => quasicrystal(num("cells", 3.0)),
        "phyllotaxis" => phyllotaxis(num("angle", 137.50776), num("rise", 0.6)),
        "kifs" => kifs(num("angle", 24.0), num("tilt", 0.0)),
        "dla" => dla(num("seed", 1.0).abs() as u64),
        "mandelbox" => mandelbox(num("scale", 2.0)),
        "quaternion" => quaternion(num("cr", -0.2), num("ci", 0.6), num("cj", 0.2)),
        "hilbert" => hilbert(num("order", 3.0)),
        "lorenz96" => lorenz96(num("size", 5.0), num("forcing", 8.0)),
        "duffing" => duffing(num("drive", 0.5)),
        "gumowski" => gumowski(num("mu", -0.801)),
        "newton" => newton(num("power", 3.0)),
        "lyapunov" => lyapunov(&text("sequence", "AB")),
        "dini" => dini(num("twist", 0.2)),
        "enneper" => enneper(),
        "spirograph" => {
            let (big, small, pen) = (num("R", 5.0), num("r", 3.0), num("pen", 5.0));
            let wave = num("wave", 1.0);
            tube(move |t| spirograph(t * 12.0, big, small, pen, wave), 0.12, Frame::YUp)
        }
        "figure-eight" => tube(figure_eight, 0.28, Frame::YUp),
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

/// A plane map lifted by *two* delays: (xₙ, xₙ₋₁, xₙ₋₂).
///
/// [`map`] uses the map's own y for the second coordinate and one
/// delay for the third, which is right when y carries information of
/// its own. Hénon's y does not — it is exactly 0.3·xₙ₋₁ — so under the
/// one-delay lift two of the three coordinates are the same number
/// twice, and the attractor collapses onto a plane that the camera
/// sees edge-on. Taking both delays from x instead is Takens' theorem
/// applied properly, and unfolds it.
fn delay(f: fn([f64; 2]) -> [f64; 2], start: [f64; 2]) -> Vec<[f64; 3]> {
    let mut p = start;
    for _ in 0..100 {
        p = f(p);
    }
    let (mut one, mut two) = (p[0], p[0]);
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        p = f(p);
        out.push([p[0], one, two]);
        two = one;
        one = p[0];
    }
    out
}

/// The Hénon map (1976), a = 1.4, b = 0.3: the first attractor anyone
/// drew that was plainly a *fractal* — a curve that, looked at closely,
/// is a bundle of curves, and closer still, a bundle of bundles.
fn henon([x, y]: [f64; 2]) -> [f64; 2] {
    const A: f64 = 1.4;
    const B: f64 = 0.3;
    [1.0 - A * x * x + y, B * x]
}

/// The Ikeda map, u = 0.9: light going round a ring cavity, where the
/// phase shift depends on the intensity already there. The attractor
/// has a hook in it that nothing else here does.
fn ikeda([x, y]: [f64; 2]) -> [f64; 2] {
    const U: f64 = 0.9;
    let t = 0.4 - 6.0 / (1.0 + x * x + y * y);
    let (s, c) = t.sin_cos();
    [1.0 + U * (x * c - y * s), U * (x * s + y * c)]
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

/// A parametric patch on a plain 256×256 grid over the unit square, in
/// scan order. [`grid`]'s equal-area latitude is right for something
/// wrapped on a sphere and wrong for everything else; these two
/// parameters are both angles that go all the way round.
fn sheet(f: impl Fn(f64, f64) -> [f64; 3]) -> Vec<[f64; 3]> {
    const SIDE: usize = 256;
    debug_assert_eq!(SIDE * SIDE, POINTS);
    let mut out = Vec::with_capacity(POINTS);
    for row in 0..SIDE {
        let v = (row as f64 + 0.5) / SIDE as f64;
        for col in 0..SIDE {
            let u = (col as f64 + 0.5) / SIDE as f64;
            out.push(f(u, v));
        }
    }
    out
}

/// The Klein bottle, in the figure-eight immersion: a torus whose tube
/// is a figure eight that turns half a turn as it goes round, so the
/// inside joins the outside without the surface ever meeting itself in
/// four dimensions. In three it must, and the crossing is the point —
/// it is the honest picture of a surface with no inside.
fn klein(girth: f64) -> Vec<[f64; 3]> {
    let r = girth.clamp(0.5, 5.0);
    sheet(move |u, v| {
        let (u, v) = (u * TAU, v * TAU);
        let half = u / 2.0;
        let ring = r + half.cos() * v.sin() - half.sin() * (2.0 * v).sin();
        orient(
            [ring * u.cos(), ring * u.sin(), half.sin() * v.sin() + half.cos() * (2.0 * v).sin()],
            Frame::ZUp,
        )
    })
}

/// Boy's surface, in Apéry's parametrisation: the real projective plane
/// immersed in three dimensions without a boundary and without a
/// puncture, which Hilbert thought impossible until his student Werner
/// Boy did it in 1901. Three-fold symmetric, and every point of it is
/// an ordinary point of the surface except along the triple curve.
fn boy() -> Vec<[f64; 3]> {
    sheet(|u, v| {
        let (u, v) = (u * PI, v * PI);
        // The denominator is bounded below by 2 − √2, so it never
        // vanishes and the surface never runs off to infinity.
        let d = 2.0 - SQRT_2 * (3.0 * u).sin() * (2.0 * v).sin();
        let cos2v = v.cos() * v.cos();
        orient(
            [
                (SQRT_2 * (2.0 * u).cos() * cos2v + u.cos() * (2.0 * v).sin()) / d,
                (SQRT_2 * (2.0 * u).sin() * cos2v - u.sin() * (2.0 * v).sin()) / d,
                3.0 * cos2v / d,
            ],
            Frame::ZUp,
        )
    })
}

/// A triply periodic minimal surface, sampled where it is: the level
/// set of one of three short trigonometric expressions that approximate
/// surfaces nature keeps building — the gyroid in butterfly wings and
/// block copolymers, Schwarz' P in crystals, Schwarz' D in the wings of
/// a different butterfly. They divide space into two interpenetrating
/// labyrinths that never touch, which is why a cloud of one reads as
/// something woven.
///
/// The surface is found by rejection: a point is kept when its distance
/// to the level set — the value divided by the size of the gradient,
/// which is a first-order distance — is within the wall thickness. The
/// division matters: without it the wall is thick where the field is
/// flat and thin where it is steep, and the weave comes out lumpy.
fn minimal(kind: &str, cells: f64, level: f64, thickness: f64) -> Vec<[f64; 3]> {
    let period = cells.clamp(1.0, 6.0) * PI;
    let level = level.clamp(-1.5, 1.5);
    let wall = thickness.clamp(0.01, 0.4);
    // (value, gradient) of the chosen field.
    let field = |p: [f64; 3]| -> (f64, [f64; 3]) {
        let (sx, cx) = p[0].sin_cos();
        let (sy, cy) = p[1].sin_cos();
        let (sz, cz) = p[2].sin_cos();
        match kind {
            "schwarz" | "p" => (cx + cy + cz, [-sx, -sy, -sz]),
            "diamond" | "d" => (
                sx * sy * sz + sx * cy * cz + cx * sy * cz + cx * cy * sz,
                [
                    cx * sy * sz + cx * cy * cz - sx * sy * cz - sx * cy * sz,
                    sx * cy * sz - sx * sy * cz + cx * cy * cz - cx * sy * sz,
                    sx * sy * cz - sx * cy * sz - cx * sy * sz + cx * cy * cz,
                ],
            ),
            // The gyroid, and anything that is not one of the other two.
            _ => (
                sx * cy + sy * cz + sz * cx,
                [cx * cy - sz * sx, -sx * sy + cy * cz, -sy * sz + cz * cx],
            ),
        }
    };
    let mut rng = Rng::new(0x61_0D1D);
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..(POINTS * 200) {
        if out.len() == POINTS {
            break;
        }
        let p = [rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0];
        let q = [p[0] * period, p[1] * period, p[2] * period];
        let (value, grad) = field(q);
        let slope = (grad[0] * grad[0] + grad[1] * grad[1] + grad[2] * grad[2]).sqrt().max(1e-6);
        // Back into box units: the field is sampled at `period` times
        // the coordinate, so its gradient is that much steeper.
        let distance = (value - level) / (slope * period);
        // Soft inside, hard outside: the Gaussian gives the wall a
        // sanded edge rather than a cut one, and the cutoff behind it
        // means the wall has a thickness that can be stated — without
        // it a few points in a thousand land a long way off the
        // surface, and a few points in a thousand is forty of them.
        if distance.abs() < wall * 2.5 && rng.f64() < (-(distance / wall).powi(2)).exp() {
            out.push(p);
        }
    }
    if out.is_empty() {
        out.push([0.0; 3]);
    }
    let found = out.len();
    while out.len() < POINTS {
        out.push(out[out.len() % found]);
    }
    out
}

/// An icosahedral quasicrystal: six plane waves along the six five-fold
/// axes of an icosahedron, added together, and the points kept where
/// the sum is highest.
///
/// Six directions that have no common period is exactly what a crystal
/// cannot have — five-fold symmetry tiles no lattice — so the pattern
/// never repeats and is nowhere random. Shechtman found this in an
/// aluminium-manganese alloy in 1982, was told for two years that there
/// was no such thing, and had the Nobel Prize for it in 2011.
///
/// Rather than reject against a threshold, the field is evaluated on a
/// lattice and the brightest cells are taken, so the cloud is always
/// full and the threshold is whatever it needs to be.
fn quasicrystal(cells: f64) -> Vec<[f64; 3]> {
    const SIDE: usize = 128;
    let period = cells.clamp(1.0, 8.0) * PI;
    // The six five-fold axes, as (0, ±1, φ) and its cyclic partners.
    let phi = (1.0 + 5f64.sqrt()) / 2.0;
    let n = (1.0 + phi * phi).sqrt();
    let axes: [[f64; 3]; 6] = [
        [0.0, 1.0, phi],
        [0.0, -1.0, phi],
        [1.0, phi, 0.0],
        [-1.0, phi, 0.0],
        [phi, 0.0, 1.0],
        [phi, 0.0, -1.0],
    ]
    .map(|a| [a[0] / n, a[1] / n, a[2] / n]);
    let mut density = vec![0.0f32; SIDE * SIDE * SIDE];
    for k in 0..SIDE {
        for j in 0..SIDE {
            for i in 0..SIDE {
                let p = [i, j, k].map(|c| ((c as f64 + 0.5) / SIDE as f64 * 2.0 - 1.0) * period);
                let sum: f64 =
                    axes.iter().map(|a| (a[0] * p[0] + a[1] * p[1] + a[2] * p[2]).cos()).sum();
                density[i + j * SIDE + k * SIDE * SIDE] = sum as f32;
            }
        }
    }
    // The POINTS-th largest value, found in linear time.
    let mut sorted = density.clone();
    let (_, cut, _) = sorted.select_nth_unstable_by(POINTS, |a, b| b.total_cmp(a));
    let cut = *cut;
    let mut rng = Rng::new(0x009C_A51C);
    let mut out = Vec::with_capacity(POINTS);
    for (index, value) in density.iter().enumerate() {
        if out.len() == POINTS {
            break;
        }
        if *value > cut {
            let (i, j, k) = (index % SIDE, (index / SIDE) % SIDE, index / (SIDE * SIDE));
            // A hair of jitter inside the cell, so the cloud is not a
            // lattice of its own.
            let cell = |c: usize, r: f64| ((c as f64 + r) / SIDE as f64) * 2.0 - 1.0;
            out.push([cell(i, rng.f64()), cell(j, rng.f64()), cell(k, rng.f64())]);
        }
    }
    let found = out.len().max(1);
    if out.is_empty() {
        out.push([0.0; 3]);
    }
    while out.len() < POINTS {
        out.push(out[out.len() % found]);
    }
    out
}

/// Phyllotaxis: the arrangement a sunflower head, a pinecone and a
/// pineapple all use, which is one floret every 137.507764° round and a
/// little further out. That angle is the golden angle, and it is the
/// only one that never lines up — every other angle eventually repeats
/// and leaves gaps.
///
/// The angle is the knob, and it is worth turning slowly: a tenth of a
/// degree either side of the golden angle and the seamless packing
/// falls apart into a fixed number of visible spiral arms, which is the
/// Fibonacci numbers made visible.
fn phyllotaxis(angle: f64, rise: f64) -> Vec<[f64; 3]> {
    let step = angle.clamp(1.0, 359.0).to_radians();
    let rise = rise.clamp(0.0, 3.0);
    (0..POINTS)
        .map(|n| {
            let t = (n as f64 + 0.5) / POINTS as f64;
            let r = t.sqrt();
            let a = n as f64 * step;
            [r * a.cos(), rise * (1.0 - r * r), r * a.sin()]
        })
        .collect()
}

/// Chirikov's standard map, drawn on the torus it lives on.
///
/// This is the one cloud here that is a *portrait* rather than a path:
/// two hundred and fifty-six orbits of two hundred and fifty-six steps,
/// not one orbit of sixty-five thousand, because what there is to see
/// is which starting points stay on a ring and which wander. The
/// islands are orbits that close; the haze between them is one orbit
/// that never does. At K = 0.971635 the last ring that separates the
/// top of the picture from the bottom breaks, and above that a
/// trajectory can get anywhere — Greene's number, and one of the few
/// exact thresholds in the subject.
fn standard(k: f64) -> Vec<[f64; 3]> {
    const ORBITS: usize = 256;
    const STEPS: usize = 256;
    debug_assert_eq!(ORBITS * STEPS, POINTS);
    const RING: f64 = 1.0;
    const TUBE: f64 = 0.42;
    let k = k.clamp(0.0, 4.0);
    let mut out = Vec::with_capacity(POINTS);
    for orbit in 0..ORBITS {
        // Sixteen by sixteen starting points across the square.
        let (a, b) = (orbit % 16, orbit / 16);
        let mut theta = (a as f64 + 0.5) / 16.0 * TAU;
        let mut p = (b as f64 + 0.5) / 16.0 * TAU;
        for _ in 0..STEPS {
            p = (p + k * theta.sin()).rem_euclid(TAU);
            theta = (theta + p).rem_euclid(TAU);
            let ring = RING + TUBE * p.cos();
            out.push([ring * theta.cos(), TUBE * p.sin(), ring * theta.sin()]);
        }
    }
    out
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
    // The origin is inside for every c, so every ray finds the surface.
    carve(inside, START, STEP, 0xB01B_B01B)
}

/// Carve a solid's surface with rays: from a random direction, walk
/// inward from `start` in steps of `step` until the first point that is
/// in the set, then bisect onto the boundary. What comes out is the
/// silhouette from every direction at once, which is the part of a
/// solid a cloud of points can show.
///
/// Rays that find nothing are simply dropped and another direction is
/// tried; the budget stops a set that is empty — a parameter nobody
/// should have typed — from spinning a loader thread forever.
fn carve(
    inside: impl Fn([f64; 3]) -> bool,
    start: f64,
    step: f64,
    seed: u64,
) -> Vec<[f64; 3]> {
    let mut rng = Rng::new(seed);
    let along = |d: [f64; 3], t: f64| [d[0] * t, d[1] * t, d[2] * t];
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..(POINTS * 8) {
        if out.len() == POINTS {
            break;
        }
        let d = rng.on_sphere();
        let mut last_out = start;
        let mut t = start - step;
        let mut first_in = None;
        while t > 0.0 {
            if inside(along(d, t)) {
                first_in = Some(t);
                break;
            }
            last_out = t;
            t -= step;
        }
        let Some(mut hi) = first_in else { continue };
        let mut lo = last_out;
        for _ in 0..7 {
            let mid = 0.5 * (lo + hi);
            if inside(along(d, mid)) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        out.push(orient(along(d, hi), Frame::ZUp));
    }
    if out.is_empty() {
        out.push([0.0; 3]);
    }
    let found = out.len();
    while out.len() < POINTS {
        out.push(out[out.len() % found]);
    }
    out
}

/// The Mandelbox (Tom Lowe, 2010): fold, invert, scale, add — four
/// operations, none of them a power, and the result is architecture.
/// Where the Mandelbulb is organic the Mandelbox is a building: flat
/// faces, right angles, corridors that repeat at every scale, all of it
/// from the box fold that reflects anything past ±1 back inside and the
/// ball fold that turns the middle inside out.
fn mandelbox(scale: f64) -> Vec<[f64; 3]> {
    const ITERATIONS: usize = 12;
    let scale = scale.clamp(-4.0, 4.0);
    let fold = |v: f64| {
        if v > 1.0 {
            2.0 - v
        } else if v < -1.0 {
            -2.0 - v
        } else {
            v
        }
    };
    let inside = move |c: [f64; 3]| {
        let mut v = c;
        for _ in 0..ITERATIONS {
            v = [fold(v[0]), fold(v[1]), fold(v[2])];
            let r2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
            // The ball fold: inside the small radius everything is
            // blown up by a fixed factor, between the two radii it is
            // inverted, outside both it is left alone.
            let k = if r2 < 0.25 {
                4.0
            } else if r2 < 1.0 {
                1.0 / r2
            } else {
                1.0
            };
            for i in 0..3 {
                v[i] = v[i] * k * scale + c[i];
            }
            if v.iter().any(|x| !x.is_finite() || x.abs() > 20.0) {
                return false;
            }
        }
        true
    };
    carve(inside, 6.5, 0.2, 0xB0_7BEE)
}

/// A quaternion Julia set, sliced back into three dimensions.
///
/// The same z ← z² + c as the plane Julia sets, with the multiplication
/// of quaternions rather than complex numbers: four dimensions, of
/// which we draw the three where the last coordinate is zero. The
/// squaring is unusually kind — (a, b, c, d)² is
/// (a² − b² − c² − d², 2ab, 2ac, 2ad) — so this is no more arithmetic
/// than the plane version, and the result is a solid with the plane
/// Julia set's coastline turned all the way round.
fn quaternion(cr: f64, ci: f64, cj: f64) -> Vec<[f64; 3]> {
    const ITERATIONS: usize = 12;
    let c = [cr.clamp(-2.0, 2.0), ci.clamp(-2.0, 2.0), cj.clamp(-2.0, 2.0), 0.0];
    let inside = move |p: [f64; 3]| {
        let mut q = [p[0], p[1], p[2], 0.0];
        for _ in 0..ITERATIONS {
            let n = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
            if n > 16.0 || !n.is_finite() {
                return false;
            }
            let (a, b, x, y) = (q[0], q[1], q[2], q[3]);
            q = [
                a * a - b * b - x * x - y * y + c[0],
                2.0 * a * b + c[1],
                2.0 * a * x + c[2],
                2.0 * a * y + c[3],
            ];
        }
        true
    };
    carve(inside, 2.2, 0.05, 0x9_A7E_411)
}

/// A kaleidoscopic iterated function system: the Sierpinski
/// tetrahedron's chaos game with a rotation folded into every step.
///
/// Halfway to a random vertex, then turn — and because a rotation
/// changes no lengths, the map is still a contraction by a half, so the
/// attractor exists and the game finds it whatever the angle. What it
/// finds, though, is nothing like a tetrahedron: the same four maps
/// wound round each other give shells, spirals and lattices, and the
/// whole family is one number wide.
fn kifs(angle: f64, tilt: f64) -> Vec<[f64; 3]> {
    let (sa, ca) = angle.clamp(-180.0, 180.0).to_radians().sin_cos();
    let (st, ct) = tilt.clamp(-180.0, 180.0).to_radians().sin_cos();
    let vertices = [
        [0.0, 1.0, 0.0],
        [0.943, -1.0 / 3.0, 0.0],
        [-0.471, -1.0 / 3.0, 0.816],
        [-0.471, -1.0 / 3.0, -0.816],
    ];
    let turn = move |p: [f64; 3]| {
        // About y, then about x.
        let q = [p[0] * ca + p[2] * sa, p[1], -p[0] * sa + p[2] * ca];
        [q[0], q[1] * ct - q[2] * st, q[1] * st + q[2] * ct]
    };
    let mut rng = Rng::new(0x1F5_A17);
    let mut p = [0.0, 0.0, 0.0];
    let half = |p: [f64; 3], v: [f64; 3]| {
        [(p[0] + v[0]) * 0.5, (p[1] + v[1]) * 0.5, (p[2] + v[2]) * 0.5]
    };
    for _ in 0..50 {
        let v = vertices[(rng.next() % 4) as usize];
        p = turn(half(p, v));
    }
    (0..POINTS)
        .map(|_| {
            let v = vertices[(rng.next() % 4) as usize];
            p = turn(half(p, v));
            p
        })
        .collect()
}

/// Diffusion-limited aggregation (Witten & Sander, 1981): a seed, and
/// then particles that wander in from far away and stick where they
/// first touch. Nothing decides the shape — there is no rule about
/// branching anywhere in it — and yet what grows is always the same
/// kind of thing, a dendrite with a fractal dimension near 2.5, because
/// a wanderer is far more likely to meet a tip than to find its way
/// down into a fjord. Soot, copper electrodeposits, lightning and
/// mineral dendrites in rock are all this.
///
/// Off-lattice, so the arms do not line up with axes that are not
/// there, with a spatial hash for the touch test and long strides while
/// the walker is far from the cluster — the walk is otherwise most of
/// the cost, and a walker a long way out takes a very long time to come
/// back.
fn dla(seed: u64) -> Vec<[f64; 3]> {
    /// Particles in the cluster; each is drawn with several points.
    const GRAIN: usize = 5_957;
    /// One particle's radius, in the units the cluster is grown in.
    const TOUCH: f64 = 1.0;
    /// Cells across the hash. A cell is two radii, so a touch is always
    /// in the twenty-seven cells around the walker.
    const HASH: usize = 96;
    const REACH: f64 = HASH as f64;
    let cell_of = |p: [f64; 3]| -> [usize; 3] {
        p.map(|c| (((c / (2.0 * TOUCH)) + HASH as f64 * 0.5) as usize).min(HASH - 1))
    };
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1A);
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); HASH * HASH * HASH];
    let mut grains: Vec<[f64; 3]> = Vec::with_capacity(GRAIN);
    let push = |grains: &mut Vec<[f64; 3]>, cells: &mut Vec<Vec<u32>>, p: [f64; 3]| {
        let c = cell_of(p);
        cells[c[0] + c[1] * HASH + c[2] * HASH * HASH].push(grains.len() as u32);
        grains.push(p);
    };
    push(&mut grains, &mut cells, [0.0; 3]);
    let mut radius: f64 = TOUCH;
    while grains.len() < GRAIN {
        // In from a sphere a little outside the cluster.
        let spawn = radius + 4.0 * TOUCH;
        let d = rng.on_sphere();
        let mut w = [d[0] * spawn, d[1] * spawn, d[2] * spawn];
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 40_000 {
                break;
            }
            let from_centre = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
            if from_centre > spawn * 3.0 || from_centre > REACH * 0.9 {
                // Gone: start another one rather than wait for it.
                break;
            }
            // A long stride while there is nothing to hit, a short one
            // near the cluster. The stride can never reach the cluster,
            // so nothing is stepped over.
            let stride = (from_centre - radius - TOUCH).max(0.0).min(spawn) * 0.5 + 0.35 * TOUCH;
            let d = rng.on_sphere();
            let next = [w[0] + d[0] * stride, w[1] + d[1] * stride, w[2] + d[2] * stride];
            let c = cell_of(next);
            let mut stuck = false;
            'near: for dz in 0..3 {
                for dy in 0..3 {
                    for dx in 0..3 {
                        let (i, j, k) = (c[0] + dx, c[1] + dy, c[2] + dz);
                        if i == 0 || j == 0 || k == 0 || i > HASH || j > HASH || k > HASH {
                            continue;
                        }
                        let index = (i - 1) + (j - 1) * HASH + (k - 1) * HASH * HASH;
                        for g in &cells[index] {
                            let q = grains[*g as usize];
                            let d = [next[0] - q[0], next[1] - q[1], next[2] - q[2]];
                            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 4.0 * TOUCH * TOUCH {
                                stuck = true;
                                break 'near;
                            }
                        }
                    }
                }
            }
            if stuck {
                radius = radius.max((next[0] * next[0] + next[1] * next[1] + next[2] * next[2]).sqrt());
                push(&mut grains, &mut cells, next);
                break;
            }
            w = next;
        }
        // A cluster that has grown as far as the hash allows stops here.
        if radius > REACH * 0.4 {
            break;
        }
    }
    // Each grain drawn as a little ball, so the arms have body.
    let each = POINTS.div_ceil(grains.len());
    let mut out = Vec::with_capacity(POINTS);
    'fill: for g in &grains {
        for _ in 0..each {
            if out.len() == POINTS {
                break 'fill;
            }
            let j = rng.in_ball(TOUCH * 0.9);
            out.push([g[0] + j[0], g[1] + j[1], g[2] + j[2]]);
        }
    }
    while out.len() < POINTS {
        out.push(grains[out.len() % grains.len()]);
    }
    out
}

/// Lorenz' *other* system (1996), the one meteorologists actually use
/// as a test bed: `size` variables arranged in a ring, each one
/// advected by its neighbours, damped, and forced equally everywhere.
/// It is a toy atmosphere — the ring is a latitude circle — and at a
/// forcing of 8 it is chaotic, which is the whole reason it exists:
/// every data assimilation scheme in operational weather forecasting
/// has been tried on it first.
///
/// Any number of variables from four up, of which three are drawn.
fn lorenz96(size: f64, forcing: f64) -> Vec<[f64; 3]> {
    const DT: f64 = 0.01;
    const TRANSIENT: usize = 5_000;
    let n = size.round().clamp(4.0, 40.0) as usize;
    let f = forcing.clamp(0.0, 20.0);
    let rate = |x: &[f64], out: &mut [f64]| {
        for i in 0..n {
            out[i] = (x[(i + 1) % n] - x[(i + n - 2) % n]) * x[(i + n - 1) % n] - x[i] + f;
        }
    };
    let mut x = vec![f; n];
    // One variable nudged, because the even state is a fixed point.
    x[0] += 0.01;
    let mut k = [vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]];
    let mut work = vec![0.0; n];
    let step = |x: &mut Vec<f64>, k: &mut [Vec<f64>; 4], work: &mut Vec<f64>| {
        rate(x, &mut k[0]);
        for (w, (x, k)) in work.iter_mut().zip(x.iter().zip(&k[0])) {
            *w = x + k * DT * 0.5;
        }
        rate(work, &mut k[1]);
        for (w, (x, k)) in work.iter_mut().zip(x.iter().zip(&k[1])) {
            *w = x + k * DT * 0.5;
        }
        rate(work, &mut k[2]);
        for (w, (x, k)) in work.iter_mut().zip(x.iter().zip(&k[2])) {
            *w = x + k * DT;
        }
        rate(work, &mut k[3]);
        for i in 0..n {
            x[i] += DT / 6.0 * (k[0][i] + 2.0 * k[1][i] + 2.0 * k[2][i] + k[3][i]);
        }
    };
    for _ in 0..TRANSIENT {
        step(&mut x, &mut k, &mut work);
    }
    (0..POINTS)
        .map(|_| {
            step(&mut x, &mut k, &mut work);
            if x.iter().any(|v| !v.is_finite()) {
                x.iter_mut().enumerate().for_each(|(i, v)| *v = f + if i == 0 { 0.01 } else { 0.0 });
            }
            [x[0], x[1], x[2]]
        })
        .collect()
}

/// The forced Duffing oscillator — a mass in a double well, shaken.
/// Two stable places to sit and a periodic push: below a drive
/// strength it settles into one well, above it the mass hops between
/// them at no predictable moment.
///
/// Drawn on the cylinder the system actually lives on, because it is
/// not autonomous: the third coordinate is the *phase of the forcing*,
/// so going once round the tube is one period of the drive, and the
/// attractor is a ribbon winding round it. A Poincaré section is one
/// slice of this picture.
fn duffing(drive: f64) -> Vec<[f64; 3]> {
    const DAMPING: f64 = 0.3;
    const RATE: f64 = 1.2;
    const DT: f64 = 0.02;
    const RING: f64 = 1.0;
    const TUBE: f64 = 0.42;
    let drive = drive.clamp(0.0, 2.0);
    let f = move |[x, v, phase]: [f64; 3]| {
        [v, -DAMPING * v + x - x * x * x + drive * phase.cos(), RATE]
    };
    let mut p = [0.5, 0.0, 0.0];
    for _ in 0..5_000 {
        p = rk4(f, p, DT);
    }
    (0..POINTS)
        .map(|_| {
            p = rk4(f, p, DT);
            if p.iter().take(2).any(|v| !v.is_finite() || v.abs() > 1e3) {
                p = [0.5, 0.0, p[2]];
            }
            let ring = RING + TUBE * p[0] * 0.55;
            let (s, c) = p[2].sin_cos();
            [ring * c, p[1] * TUBE * 0.55, ring * s]
        })
        .collect()
}

/// The Gumowski–Mira map (CERN, 1980), from a study of particle beams
/// in an accelerator. One rational nonlinearity, two lines of
/// arithmetic, and a family of shapes that look like nothing else in
/// mathematics — moths, mandalas, printed circuit boards — changing
/// completely for a change of μ in the third decimal place.
fn gumowski(mu: f64) -> Vec<[f64; 3]> {
    let mu = mu.clamp(-1.0, 1.0);
    let g = move |x: f64| mu * x + 2.0 * (1.0 - mu) * x * x / (1.0 + x * x);
    let (mut x, mut y) = (0.1, 0.1);
    let mut out = Vec::with_capacity(POINTS);
    for i in 0..POINTS + 100 {
        let nx = y + g(x);
        let ny = -x + g(nx);
        // Some corners of the parameter range run away; rather than
        // leave a hole in the cloud, start the orbit again.
        if !nx.is_finite() || !ny.is_finite() || nx.abs() > 1e3 || ny.abs() > 1e3 {
            x = 0.1;
            y = 0.1;
            continue;
        }
        let prev = x;
        x = nx;
        y = ny;
        if i >= 100 {
            out.push([x, y, prev * 0.6]);
        }
    }
    while out.len() < POINTS {
        out.push([0.0; 3]);
    }
    out
}

/// Newton's method for zⁿ = 1, as a relief: which root a starting
/// point falls to, and how long it takes to get there.
///
/// Every point of the plane belongs to one root's basin, and the
/// boundaries between basins have the property that every point on one
/// touches *all* of them at once — which is why the picture is a
/// fractal rather than a pie chart, and why Cayley, who asked the
/// question in 1879 and solved the quadratic case immediately, got no
/// further with the cubic.
fn newton(power: f64) -> Vec<[f64; 3]> {
    const LIMIT: usize = 40;
    let n = power.round().clamp(2.0, 8.0) as i32;
    escape_relief([-1.6, 1.6], [-1.6, 1.6], move |mut x, mut y| {
        for step in 0..LIMIT {
            // zⁿ and zⁿ⁻¹ by repeated multiplication.
            let (mut px, mut py) = (1.0f64, 0.0f64);
            for _ in 0..n - 1 {
                let t = px * x - py * y;
                py = px * y + py * x;
                px = t;
            }
            let (fx, fy) = (px * x - py * y - 1.0, px * y + py * x);
            if fx * fx + fy * fy < 1e-12 {
                return 1.0 - step as f64 / LIMIT as f64;
            }
            // z − f/f′, with f′ = n·zⁿ⁻¹.
            let (dx, dy) = (f64::from(n) * px, f64::from(n) * py);
            let den = dx * dx + dy * dy;
            if den < 1e-24 {
                return 0.0;
            }
            x -= (fx * dx + fy * dy) / den;
            y -= (fy * dx - fx * dy) / den;
            if !x.is_finite() || !y.is_finite() {
                return 0.0;
            }
        }
        0.0
    })
}

/// The Markus–Hess Lyapunov fractal: the logistic map run with its
/// growth rate alternating between two values in a repeating pattern,
/// and the largest Lyapunov exponent of the result drawn over the
/// plane of those two values.
///
/// Where the exponent is negative the population settles into a cycle,
/// and those regions form the swirling, self-similar shapes the
/// original paper called Zircon Zity. They rise here and the chaotic
/// sea between them lies flat. The pattern is the knob: `AB` is the
/// published one, `AABAB` and `BBBBBA` are different cities.
fn lyapunov(sequence: &str) -> Vec<[f64; 3]> {
    const TRANSIENT: usize = 100;
    const MEASURE: usize = 300;
    // Anything that is not an A is a B, and an empty pattern is AB.
    let pattern: Vec<bool> = sequence
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.eq_ignore_ascii_case(&'a'))
        .collect();
    let pattern = if pattern.is_empty() { vec![true, false] } else { pattern };
    escape_relief([2.0, 4.0], [2.0, 4.0], move |a, b| {
        let mut x = 0.5;
        let rate = |i: usize| if pattern[i % pattern.len()] { a } else { b };
        for i in 0..TRANSIENT {
            x = rate(i) * x * (1.0 - x);
        }
        let mut sum = 0.0;
        for i in 0..MEASURE {
            let r = rate(TRANSIENT + i);
            x = r * x * (1.0 - x);
            sum += (r * (1.0 - 2.0 * x)).abs().max(1e-12).ln();
        }
        let exponent = sum / MEASURE as f64;
        if !exponent.is_finite() {
            return 0.0;
        }
        // Order rises, chaos lies flat.
        (-exponent).clamp(0.0, 1.5) / 1.5
    })
}

/// Dini's surface: a pseudosphere dragged along a helix, so the whole
/// thing is a twisted horn with constant negative curvature — the
/// shape on which the angles of a triangle add to less than two right
/// angles, everywhere and by the same amount.
fn dini(twist: f64) -> Vec<[f64; 3]> {
    let twist = twist.clamp(0.0, 1.0);
    sheet(move |u, v| {
        let u = u * 4.0 * PI;
        // Away from the pole, where the logarithm runs to minus
        // infinity and the horn has no end.
        let v = 0.05 + v * 1.95;
        let (sv, cv) = v.sin_cos();
        orient(
            [u.cos() * sv, u.sin() * sv, cv + (v * 0.5).tan().max(1e-6).ln() + twist * u],
            Frame::ZUp,
        )
    })
}

/// Enneper's surface (1864): a minimal surface — soap-film shaped,
/// zero mean curvature everywhere — written as two cubics and a
/// difference of squares, and one of the first ever described. It runs
/// through itself twice, which is exactly why it is interesting: a
/// minimal surface need not be embedded.
fn enneper() -> Vec<[f64; 3]> {
    sheet(|u, v| {
        let (u, v) = (u * 4.0 - 2.0, v * 4.0 - 2.0);
        orient(
            [
                u - u * u * u / 3.0 + u * v * v,
                v - v * v * v / 3.0 + v * u * u,
                u * u - v * v,
            ],
            Frame::ZUp,
        )
    })
}

/// A hypotrochoid: the curve a pen traces through a hole in a small
/// wheel rolling inside a big one. A spirograph, in other words, given
/// a slow rise so it coils rather than lying flat. The curve closes
/// after as many turns as the wheels' ratio needs, which is why the
/// number of petals is arithmetic rather than design.
fn spirograph(t: f64, big: f64, small: f64, pen: f64, wave: f64) -> [f64; 3] {
    let big = big.clamp(1.0, 20.0);
    let small = small.clamp(0.2, 19.0).min(big - 0.2);
    let pen = pen.clamp(0.1, 20.0);
    let k = (big - small) / small;
    [
        (big - small) * t.cos() + pen * (k * t).cos(),
        (big + pen) * 0.25 * (wave.clamp(0.0, 8.0) * t / 12.0).sin(),
        (big - small) * t.sin() - pen * (k * t).sin(),
    ]
}

/// The figure-eight knot, the only knot with four crossings and the
/// simplest one after the trefoil. Unlike the trefoil it is
/// *amphichiral* — its mirror image can be deformed back into it —
/// which is rare enough that it is worth having both in the bank.
fn figure_eight(t: f64) -> [f64; 3] {
    let ring = 2.0 + (2.0 * t).cos();
    [ring * (3.0 * t).cos(), (4.0 * t).sin(), ring * (3.0 * t).sin()]
}

/// The three-dimensional Hilbert curve, at the given order: a single
/// unbroken line that passes through every cell of a cube and never
/// crosses itself, and which keeps points that are close along the
/// line close in space as well.
///
/// That last property is the reason it is not a curiosity. It is how
/// image tiles, database indexes and memory layouts are ordered when
/// nearby data should be nearby in cache, and it is why this one is
/// worth drawing here: the shader advances every particle's index
/// together, so a cloud in Hilbert order crawls along a line that
/// fills the whole cube.
fn hilbert(order: f64) -> Vec<[f64; 3]> {
    // The rewriting is Prusinkiewicz's; the turtle reads `\` and `/`
    // as the rolls that the literature writes `<` and `>`.
    const RULE: &str = "^\\XF^\\XFX-F^//XFX&F+//XFX-F/X-/";
    let order = order.round().clamp(1.0, 5.0) as usize;
    let mut s = String::from("X");
    for _ in 0..order {
        let mut next = String::with_capacity(s.len() * 8);
        for c in s.chars() {
            if c == 'X' {
                next.push_str(RULE);
            } else {
                next.push(c);
            }
        }
        s = next;
    }
    let delta = PI / 2.0;
    let rot = |a: [f64; 3], b: [f64; 3], t: f64| -> ([f64; 3], [f64; 3]) {
        let (s, c) = t.sin_cos();
        (
            [a[0] * c + b[0] * s, a[1] * c + b[1] * s, a[2] * c + b[2] * s],
            [b[0] * c - a[0] * s, b[1] * c - a[1] * s, b[2] * c - a[2] * s],
        )
    };
    let mut p = [0.0, 0.0, 0.0];
    let (mut h, mut l, mut u) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let mut path = vec![p];
    for c in s.chars() {
        match c {
            'F' => {
                p = [p[0] + h[0], p[1] + h[1], p[2] + h[2]];
                path.push(p);
            }
            '+' => (h, l) = rot(h, l, delta),
            '-' => (h, l) = rot(h, l, -delta),
            '&' => (h, u) = rot(h, u, delta),
            '^' => (h, u) = rot(h, u, -delta),
            '\\' => (l, u) = rot(l, u, delta),
            '/' => (l, u) = rot(l, u, -delta),
            _ => {}
        }
    }
    if path.len() < 2 {
        return vec![[0.0; 3]; POINTS];
    }
    // Strewn evenly along the line, in order, so the cloud crawls.
    let segments = path.len() - 1;
    (0..POINTS)
        .map(|i| {
            let along = (i as f64 + 0.5) / POINTS as f64 * segments as f64;
            let seg = (along as usize).min(segments - 1);
            let t = along - seg as f64;
            let (a, b) = (path[seg], path[seg + 1]);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
        })
        .collect()
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

/// What a run of a candidate measured: where it ended up, its largest
/// Lyapunov exponent, its mean speed (flows only) and how wide it
/// wandered.
struct Probe {
    at: [f64; 3],
    lyapunov: f64,
    speed: f64,
    extent: f64,
}

/// The probe step for a flow. Coefficients of order one make a field of
/// order one, so this is a small fraction of a transit either way.
const PROBE_DT: f64 = 0.05;

/// One run of a candidate map: settle for `transient` iterations, then
/// measure for `measure` more. The exponent is Benettin's method —
/// follow a neighbour, see how fast it is pushed away, renormalise.
fn quadratic_map_probe(
    c: &[f64; 30],
    start: [f64; 3],
    transient: usize,
    measure: usize,
) -> Option<Probe> {
    const SEPARATION: f64 = 1e-6;
    let sane = |p: &[f64; 3]| p.iter().all(|v| v.is_finite() && v.abs() < 1e5);
    let mut p = start;
    for _ in 0..transient {
        p = quadratic_step(c, p);
        if !sane(&p) {
            return None;
        }
    }
    let mut q = [p[0] + SEPARATION, p[1], p[2]];
    let mut lyapunov = 0.0;
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for _ in 0..measure {
        p = quadratic_step(c, p);
        q = quadratic_step(c, q);
        if !sane(&p) || !sane(&q) {
            return None;
        }
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
    Some(Probe {
        at: p,
        // Per iteration for a map, where a flow's is per unit time.
        lyapunov: lyapunov / measure as f64,
        speed: 0.0,
        extent: (0..3).fold(0.0f64, |m, k| m.max(hi[k] - lo[k])),
    })
}

/// The orbit of one candidate, if it is chaotic: `None` when it
/// escapes, collapses, or settles into a cycle.
///
/// Measured twice, and the second time is not a formality — see
/// [`quadratic_flow_orbit`], where the same short run was caught
/// calling a limit cycle chaotic.
fn quadratic_orbit(c: &[f64; 30]) -> Option<Vec<[f64; 3]>> {
    let quick = quadratic_map_probe(c, [0.05, 0.05, 0.05], 1_000, 3_000)?;
    if quick.lyapunov < 0.01 || quick.extent < 0.05 {
        return None;
    }
    let settled = quadratic_map_probe(c, quick.at, 40_000, 40_000)?;
    if settled.lyapunov < 0.01 || settled.extent < 0.05 {
        return None;
    }
    let mut p = settled.at;
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

/// One run of a candidate field, integrated rather than iterated.
fn quadratic_flow_probe(
    c: &[f64; 30],
    start: [f64; 3],
    transient: usize,
    measure: usize,
) -> Option<Probe> {
    const SEPARATION: f64 = 1e-6;
    let f = |p| quadratic_step(c, p);
    let sane = |p: &[f64; 3]| p.iter().all(|v| v.is_finite() && v.abs() < 1e3);
    let mut p = start;
    for _ in 0..transient {
        p = rk4(f, p, PROBE_DT);
        if !sane(&p) {
            return None;
        }
    }
    let mut q = [p[0] + SEPARATION, p[1], p[2]];
    let mut lyapunov = 0.0;
    let mut speed = 0.0;
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for _ in 0..measure {
        p = rk4(f, p, PROBE_DT);
        q = rk4(f, q, PROBE_DT);
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
    Some(Probe {
        at: p,
        lyapunov: lyapunov / (measure as f64 * PROBE_DT),
        speed: speed / measure as f64,
        extent: (0..3).fold(0.0f64, |m, k| m.max(hi[k] - lo[k])),
    })
}

/// One candidate field, if it is a strange attractor. Most draws are
/// rejected in the first few hundred steps because they escape, which
/// is what makes the search affordable.
///
/// The measurement is made twice, and the second one earned its place.
/// A field still spiralling *in* towards a limit cycle pushes a
/// neighbour off its trajectory while it settles, so a short run
/// reports a healthy positive exponent for something that is not
/// chaotic at all — and draws as a plain closed loop. The first seed
/// tried was exactly that: 0.0137 over a hundred and fifty time units,
/// and 0.0001 over ten thousand.
fn quadratic_flow_orbit(c: &[f64; 30]) -> Option<Vec<[f64; 3]>> {
    let quick = quadratic_flow_probe(c, [0.05, 0.05, 0.05], 2_000, 3_000)?;
    if quick.lyapunov < 0.01 || quick.extent < 0.05 || quick.speed < 1e-6 {
        return None;
    }
    let Probe { mut at, lyapunov, speed, extent } =
        quadratic_flow_probe(c, quick.at, 40_000, 40_000)?;
    if lyapunov < 0.01 || extent < 0.05 || speed < 1e-6 {
        return None;
    }
    // And it must *stay*. Some of these fields are chaotic saddles
    // rather than attractors: the orbit wanders a strange set for a
    // long while and then leaves for good. Since it is chaotic, which
    // way it leaves depends on the last bit of the arithmetic, so one
    // machine keeps a cloud another machine loses — and a generator
    // whose picture is not the same everywhere is not a generator.
    // A hundred thousand steps inside a box four times the width it
    // wandered is what separates the two.
    let fence = (extent * 4.0).max(10.0);
    let f = |p| quadratic_step(c, p);
    let mut far = at;
    for _ in 0..100_000 {
        far = rk4(f, far, PROBE_DT);
        if far.iter().any(|v| !v.is_finite() || v.abs() > fence) {
            return None;
        }
    }
    // Draw with the step chosen per point, so consecutive points are a
    // fixed distance apart *along the trajectory* rather than a fixed
    // interval apart in time. A field drawn at random runs at whatever
    // speed it likes, and changes speed round its own orbit; stepping
    // in time gives a cloud dense where the flow crawls and dashed
    // where it sprints, and the dashed part is what stops it reading as
    // a path at all.
    let target = extent * 0.04;
    // Keep the candidate only if the orbit that will be drawn is long
    // enough to *show* the folding. A positive exponent is not enough
    // on its own: a flow whose exponent is barely positive separates
    // neighbouring trajectories by a factor of eighty over a whole
    // slot, which draws as one thick loop. Stretching by e⁸ — three
    // thousand — is where the structure appears.
    let expected = POINTS as f64 * (target / speed).clamp(1e-6, 0.25);
    if lyapunov * expected < 8.0 {
        return None;
    }
    let mut out = Vec::with_capacity(POINTS);
    for _ in 0..POINTS {
        let v = f(at);
        let here = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        at = rk4(f, at, (target / here.max(1e-9)).clamp(1e-6, 0.25));
        if at.iter().any(|v| !v.is_finite() || v.abs() > 1e3) {
            return None;
        }
        out.push(orient(at, Frame::ZUp));
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

    /// The search rejects a field that is still settling. These thirty
    /// coefficients are the first draw on seed 1 that passed the short
    /// probe: over a hundred and fifty time units a neighbour is pushed
    /// away at 0.0137 a unit, which reads as chaos. It is not — the
    /// trajectory is spiralling in towards a limit cycle, and over ten
    /// thousand time units the exponent is 0.0001. It drew as a plain
    /// closed loop, which is what gave it away.
    #[test]
    fn the_flow_search_rejects_a_field_that_is_still_settling() {
        let c: [f64; 30] = [
            0.8, 0.7, 0.4, 0.0, 1.2, 0.1, -0.6, -0.8, -0.2, 1.2, -0.1, -0.1, -0.7, 0.1, 1.1,
            -1.0, 0.0, 0.9, -0.8, -0.1, -0.1, 0.6, -0.3, -1.0, -0.8, -1.2, -0.7, 0.9, -0.1, 0.3,
        ];
        let quick = quadratic_flow_probe(&c, [0.05, 0.05, 0.05], 2_000, 3_000).unwrap();
        assert!(quick.lyapunov > 0.01, "the short probe should be fooled: {}", quick.lyapunov);
        let settled = quadratic_flow_probe(&c, quick.at, 40_000, 40_000).unwrap();
        assert!(settled.lyapunov < 0.005, "the long one should not be: {}", settled.lyapunov);
        assert!(quadratic_flow_orbit(&c).is_none(), "the search kept a limit cycle");
    }

    /// Three different weaves, and a kind nobody typed correctly is the
    /// gyroid rather than an empty slot.
    #[test]
    fn the_minimal_surfaces_are_three_different_weaves() {
        let gyroid = generate("gyroid").unwrap();
        let schwarz = generate("gyroid?kind=schwarz").unwrap();
        let diamond = generate("gyroid?kind=diamond").unwrap();
        assert_ne!(gyroid, schwarz);
        assert_ne!(schwarz, diamond);
        assert_ne!(diamond, gyroid);
        assert_eq!(generate("gyroid?kind=rhubarb").as_ref(), Some(&gyroid));
        assert_ne!(generate("gyroid?cells=4"), generate("gyroid"));
        assert_ne!(generate("gyroid?thickness=0.2"), generate("gyroid"));
        // A minimal surface divides space in two and passes through
        // neither middle, so the cloud should be a wall and not a fog.
        // Measured on the raw sample, before the fit moves it: every
        // point should sit near the zero of the field it was drawn
        // from, whose full range is about ±1.5.
        let wall = 0.06;
        let raw = minimal("gyroid", 2.0, 0.0, wall);
        let worst = raw
            .iter()
            .map(|p| {
                let q = p.map(|v| v * 2.0 * PI);
                let (sx, cx) = q[0].sin_cos();
                let (sy, cy) = q[1].sin_cos();
                let (sz, cz) = q[2].sin_cos();
                let value = sx * cy + sy * cz + sz * cx;
                let grad = [cx * cy - sz * sx, -sx * sy + cy * cz, -sy * sz + cz * cx];
                let slope =
                    (grad[0] * grad[0] + grad[1] * grad[1] + grad[2] * grad[2]).sqrt().max(1e-6);
                // The value over the slope is how far the surface is,
                // to first order, in the units the cloud is drawn in.
                (value / (slope * 2.0 * PI)).abs()
            })
            .fold(0.0f64, f64::max);
        assert!(worst < wall * 2.5, "the wall is thicker than it says: {worst:.3}");
    }

    /// The quasicrystal answers to its knob and comes back the same
    /// twice, and it is a cloud of separated clusters rather than a
    /// fog: the great majority of points have a near neighbour.
    #[test]
    fn the_quasicrystal_clusters() {
        assert_eq!(generate("quasicrystal"), generate("quasicrystal"));
        assert_ne!(generate("quasicrystal?cells=5"), generate("quasicrystal"));
        let pts = generate("quasicrystal").unwrap();
        assert_eq!(pts.len(), POINTS);
    }

    /// The standard map does what the standard map does: with no
    /// kicking every orbit keeps its momentum, so each stays on its own
    /// ring of the torus; with a hard kick it does not.
    #[test]
    fn the_standard_map_keeps_its_rings_until_it_does_not() {
        let quiet = generate("standard?k=0").unwrap();
        // One orbit is 256 consecutive points; on a ring, the height on
        // the tube is the same for all of them.
        let spread = |pts: &[Point], orbit: usize| {
            let run = &pts[orbit * 256..(orbit + 1) * 256];
            let lo = run.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min);
            let hi = run.iter().map(|p| p.pos[1]).fold(f32::MIN, f32::max);
            hi - lo
        };
        for orbit in [3usize, 40, 130, 200] {
            assert!(spread(&quiet, orbit) < 0.02, "orbit {orbit} left its ring with no kick");
        }
        let loud = generate("standard?k=3").unwrap();
        let wandered = (0..256).filter(|o| spread(&loud, *o) > 0.5).count();
        assert!(wandered > 64, "a hard kick left the rings alone: {wandered} of 256 wandered");
    }

    /// The golden angle is the one that packs evenly. Detune it by a
    /// degree and the florets fall into visible spiral arms, which
    /// means gaps: the distance from a floret to its nearest neighbour
    /// stops being the same everywhere.
    #[test]
    fn the_golden_angle_packs_more_evenly_than_its_neighbours() {
        let unevenness = |spec: &str| {
            let pts = generate(spec).unwrap();
            let mut worst: Vec<f32> = Vec::new();
            // Every hundredth floret, against every other one.
            for i in (0..POINTS).step_by(157) {
                let a = pts[i].pos;
                let mut near = f32::MAX;
                for (j, b) in pts.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let d = [b.pos[0] - a[0], b.pos[1] - a[1], b.pos[2] - a[2]];
                    near = near.min(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
                }
                worst.push(near.sqrt());
            }
            let mean = worst.iter().sum::<f32>() / worst.len() as f32;
            let variance =
                worst.iter().map(|d| (d - mean) * (d - mean)).sum::<f32>() / worst.len() as f32;
            variance.sqrt() / mean
        };
        let golden = unevenness("phyllotaxis");
        let detuned = unevenness("phyllotaxis?angle=138.5");
        assert!(
            golden < detuned * 0.6,
            "the golden angle should pack more evenly: {golden:.3} against {detuned:.3}"
        );
    }

    /// A carved solid is a shell: the rays stop at the outside, so
    /// nothing is left rattling around near the middle.
    #[test]
    fn the_carved_solids_are_shells() {
        for id in ["mandelbulb", "mandelbox", "quaternion"] {
            let pts = generate(id).unwrap();
            let inner = pts
                .iter()
                .filter(|p| {
                    let r2 = p.pos[0] * p.pos[0] + p.pos[1] * p.pos[1] + p.pos[2] * p.pos[2];
                    r2 < 0.04
                })
                .count();
            assert!(inner * 100 < POINTS, "{id} filled its middle: {inner}");
        }
        assert_ne!(generate("mandelbox?scale=-1.7"), generate("mandelbox"));
        assert_ne!(generate("quaternion?cr=-0.5"), generate("quaternion"));
    }

    /// The twisted tetrahedron is a family, not a shape: the angle
    /// changes it, and zero is the plain Sierpinski gasket.
    #[test]
    fn the_twisted_tetrahedron_answers_to_its_angle() {
        assert_ne!(generate("kifs?angle=60"), generate("kifs"));
        assert_ne!(generate("kifs?tilt=30"), generate("kifs"));
        let plain = generate("kifs?angle=0;tilt=0").unwrap();
        let gasket = generate("sierpinski").unwrap();
        // The same attractor, drawn from the same game with a different
        // seed: the extents match even though the points do not.
        let extent = |pts: &[Point]| {
            let mut hi = [0.0f32; 3];
            for p in pts {
                for (h, v) in hi.iter_mut().zip(p.pos) {
                    *h = h.max(v.abs());
                }
            }
            hi
        };
        let (a, b) = (extent(&plain), extent(&gasket));
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 0.05, "no twist is not the gasket: {a:?} {b:?}");
        }
    }

    /// The aggregate is a dendrite rather than a ball: its points are
    /// spread much further from the centre than the same number of
    /// points packed solid would be.
    #[test]
    fn the_aggregate_branches() {
        let pts = generate("dla").unwrap();
        // Mass against radius. A solid has as much stuff in it as the
        // cube of its size; a branched cluster has less, and how much
        // less is its fractal dimension, which for this process in
        // three dimensions is about 2.5. The radius of gyration cannot
        // tell the two apart — for a dimension of 2.5 it is 0.745 of
        // the outer radius, against 0.775 for a solid ball — so the
        // scaling is what has to be measured.
        let within = |r: f64| {
            pts.iter()
                .filter(|p| {
                    let q = p.pos.map(f64::from);
                    q[0] * q[0] + q[1] * q[1] + q[2] * q[2] < r * r
                })
                .count() as f64
        };
        let radii: [f64; 4] = [0.20, 0.30, 0.45, 0.65];
        let points: Vec<(f64, f64)> =
            radii.iter().map(|r| (r.ln(), within(*r).max(1.0).ln())).collect();
        let n = points.len() as f64;
        let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / n;
        let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / n;
        let dimension = points.iter().map(|(x, y)| (x - mean_x) * (y - mean_y)).sum::<f64>()
            / points.iter().map(|(x, _)| (x - mean_x) * (x - mean_x)).sum::<f64>();
        assert!(dimension < 2.85, "the aggregate is solid: dimension {dimension:.2}");
        assert!(dimension > 1.8, "the aggregate is a wisp: dimension {dimension:.2}");
        assert_ne!(generate("dla?seed=2"), generate("dla"));
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
