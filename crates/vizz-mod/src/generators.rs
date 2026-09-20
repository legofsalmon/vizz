//! The clouds the app can make from an equation, as the panel and the
//! preset families see them.
//!
//! The mathematics lives in `vizz-render` (`generate.rs`), which this
//! crate cannot see; this is the catalogue — what each generator is
//! called, what it is, and which shelf a look built on it files under.
//! A test in `vizz-app`, which sees both, pins the two lists together
//! so a generator cannot exist in one without the other.

use crate::preset::Family;

/// One knob a generator or a simulation takes, as `key=value` after a
/// `?` in its spec — `gen:plant?rule=F[+X]F;angle=22`. Values are kept
/// as the text that was typed: the spec is a line in a settings file
/// and on a command line, and text round-trips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub about: &'static str,
    pub default: &'static str,
    pub kind: Kind,
}

/// What a knob is, for the panel to draw the right control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A number in a range.
    Number { min: f64, max: f64 },
    /// A whole number that seeds a search: any value is as good as any
    /// other, which is what a "roll" button is for.
    Seed,
    /// A string — a rule, a word.
    Text,
}

/// One generator, as the menu lists it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Generator {
    /// The key it is asked for by: `gen:<id>` in the saved cloud list and
    /// on the command line. Lowercase ASCII, stable forever — a saved
    /// settings file names it.
    pub id: &'static str,
    /// What the slot is called once it is made, and what a look built on
    /// it records as its source.
    pub name: &'static str,
    /// One line for the hover: what it is and what it looks like.
    pub about: &'static str,
    /// Which shelf a look built on it goes on.
    pub family: Family,
    /// The knobs it takes, in the order the panel shows them. Empty for
    /// most: a named attractor's parameters are what make it that one.
    pub params: &'static [Param],
}

/// The shipped set, in menu order: the flows first, then the surfaces
/// and the fractals.
pub const CATALOGUE: &[Generator] = &[
    Generator {
        id: "thomas",
        name: "Thomas",
        about: "Thomas' cyclically symmetric attractor — three sines; a knot of ribbons round the diagonal",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "halvorsen",
        name: "Halvorsen",
        about: "Halvorsen's attractor — three lobes, cyclically symmetric; sheets folding into each other",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "dadras",
        name: "Dadras",
        about: "Dadras' tri-scroll — three rolled sheets joined by a spine",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "rossler",
        name: "Rössler",
        about: "Rössler's funnel — a flat spiral that lifts and folds back on itself",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "four-wing",
        name: "Four-wing",
        about: "the four-wing attractor — four lobes off a saddle",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "chen",
        name: "Chen",
        about: "Chen's system — Lorenz's cousin, wider and more tangled",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "clifford",
        name: "Clifford",
        about: "Pickover's Clifford map, lifted into depth by its own previous step",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "dejong",
        name: "de Jong",
        about: "the Peter de Jong map of 1987, lifted the same way",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "supershape",
        name: "supershape",
        about: "Gielis' superformula as a solid — a seven-fold flower",
        family: Family::Shape,
        params: &[
            Param { key: "m", label: "m", about: "fold: how many times round the symmetry repeats", default: "7", kind: Kind::Number { min: 1.0, max: 16.0 } },
            Param { key: "n1", label: "n1", about: "overall roundness — small pinches, large bloats", default: "2", kind: Kind::Number { min: 0.1, max: 10.0 } },
            Param { key: "n2", label: "n2", about: "the cosine exponent", default: "8", kind: Kind::Number { min: 0.1, max: 20.0 } },
            Param { key: "n3", label: "n3", about: "the sine exponent", default: "4", kind: Kind::Number { min: 0.1, max: 20.0 } },
        ],
    },
    Generator {
        id: "harmonic",
        name: "harmonic",
        about: "a sphere rippled by a spherical harmonic",
        family: Family::Shape,
        params: &[
            Param { key: "round", label: "round", about: "waves round the equator", default: "3", kind: Kind::Number { min: 1.0, max: 8.0 } },
            Param { key: "up", label: "up", about: "waves pole to pole", default: "2", kind: Kind::Number { min: 1.0, max: 8.0 } },
        ],
    },
    Generator {
        id: "lissajous",
        name: "Lissajous",
        about: "a 3:4:7 Lissajous knot, as a tube",
        family: Family::Shape,
        params: &[
            Param { key: "a", label: "a", about: "frequency on x", default: "3", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "b", label: "b", about: "frequency on y", default: "4", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "c", label: "c", about: "frequency on z", default: "7", kind: Kind::Number { min: 1.0, max: 12.0 } },
        ],
    },
    Generator {
        id: "torus-knot",
        name: "torus knot",
        about: "a (3,7) torus knot — three times round, seven times through",
        family: Family::Shape,
        params: &[
            Param { key: "p", label: "p", about: "times round the axis", default: "3", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "q", label: "q", about: "times through the hole", default: "7", kind: Kind::Number { min: 1.0, max: 12.0 } },
        ],
    },
    Generator {
        id: "hopf",
        name: "Hopf",
        about: "the Hopf fibration — nested tori of linked circles, projected from the 3-sphere",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "chladni",
        name: "Chladni",
        about: "sand on a vibrating plate — points settle where the plate stands still",
        family: Family::Shape,
        params: &[
            Param { key: "n", label: "n", about: "the first mode number", default: "5", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "m", label: "m", about: "the second; equal numbers make no pattern", default: "2", kind: Kind::Number { min: 1.0, max: 9.0 } },
        ],
    },
    Generator {
        id: "sierpinski",
        name: "Sierpinski",
        about: "the Sierpinski tetrahedron, by the chaos game",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "menger",
        name: "Menger",
        about: "the Menger sponge, by the chaos game",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "mandelbulb",
        name: "Mandelbulb",
        about: "the power-eight Mandelbulb's surface, found by marching rays inward",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "sprott-b",
        name: "Sprott B",
        about: "Sprott's case B — two quadratic terms, and chaos",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "nose-hoover",
        name: "Nosé–Hoover",
        about: "the Nosé–Hoover oscillator — a thermostatted particle wandering a sea of tori and chaos",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "arneodo",
        name: "Arneodo",
        about: "Arneodo's attractor — a jerk system with one cubic term",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "burke-shaw",
        name: "Burke–Shaw",
        about: "Burke–Shaw — two scrolls with the symmetry of a propeller",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "chua",
        name: "Chua",
        about: "Chua's circuit — the double scroll, from a real circuit with a nonlinear diode",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "hadley",
        name: "Hadley",
        about: "the Hadley circulation — Lorenz's 1984 atmosphere in three variables",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "rucklidge",
        name: "Rucklidge",
        about: "Rucklidge's convection model — a tall, folded ribbon",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "three-scroll",
        name: "three-scroll",
        about: "the three-scroll unified system — three scrolls in one fast flow",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "rabinovich",
        name: "Rabinovich–Fabrikant",
        about: "Rabinovich–Fabrikant — leaves and ribbons, from plasma physics",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "plant",
        name: "plant",
        about: "a plant grown by an L-system — five generations of branching, thick wood to thin twigs",
        family: Family::Shape,
        params: &[
            Param { key: "rule", label: "rule", about: "the production for X — F draws, + − & ^ \\ / turn, [ ] branch, X grows again", default: "F[+&X][-^X]/F[\\X]X", kind: Kind::Text },
            Param { key: "angle", label: "angle", about: "degrees per turn", default: "25", kind: Kind::Number { min: 5.0, max: 60.0 } },
        ],
    },
    Generator {
        id: "quadratic",
        name: "quadratic",
        about: "Sprott's search: random quadratic maps until one is chaotic — a new attractor every roll",
        family: Family::Attractor,
        params: &[Param {
            key: "seed",
            label: "seed",
            about: "which search; the same seed is the same attractor on every machine",
            default: "1",
            kind: Kind::Seed,
        }],
    },
    Generator {
        id: "aizawa",
        name: "Aizawa",
        about: "Aizawa's attractor — a rotating sphere with a spindle driven through its poles",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "newton-leipnik",
        name: "Newton–Leipnik",
        about: "Newton–Leipnik — a tumbling rigid body, and two attractors in one system",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "sakarya",
        name: "Sakarya",
        about: "the Sakarya system — two lobes crossing at an angle, like a bow tie in wire",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "rikitake",
        name: "Rikitake",
        about: "the Rikitake dynamo — why the Earth's magnetic field reverses, and never on schedule",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "shimizu-morioka",
        name: "Shimizu–Morioka",
        about: "Shimizu–Morioka — the butterfly's simplest relative, two wings and one quadratic term",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "finance",
        name: "finance",
        about: "the finance system — interest rate, investment demand and price index, refusing to settle",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "coullet",
        name: "Coullet",
        about: "Coullet's jerk system — one variable's third derivative, with a cubic pull",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "genesio-tesi",
        name: "Genesio–Tesi",
        about: "Genesio–Tesi — the other classic jerk system, square rather than cubic",
        family: Family::Attractor,
        params: &[],
    },
    Generator {
        id: "quadratic-flow",
        name: "quadratic flow",
        about: "the same search with an integrator inside: random quadratic flows until one is chaotic — smooth ribbons where the map gives dust",
        family: Family::Attractor,
        params: &[Param {
            key: "seed",
            label: "seed",
            about: "which search; the same seed is the same attractor on every machine",
            default: "1",
            kind: Kind::Seed,
        }],
    },
    Generator {
        id: "orbital",
        name: "orbital",
        about: "a real spherical harmonic as a balloon — the shape a textbook draws for an atomic orbital",
        family: Family::Shape,
        params: &[
            Param { key: "l", label: "l", about: "degree: how many nodal lines in all", default: "3", kind: Kind::Number { min: 0.0, max: 8.0 } },
            Param { key: "m", label: "m", about: "order: how many of them run through the poles; negative turns the lobes", default: "2", kind: Kind::Number { min: -8.0, max: 8.0 } },
        ],
    },
    Generator {
        id: "fern",
        name: "fern",
        about: "a fern by Lindenmayer's rewriting — fronds off a curling spine, rolling as they go",
        family: Family::Shape,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "22", kind: Kind::Number { min: 5.0, max: 60.0 } }],
    },
    Generator {
        id: "coral",
        name: "coral",
        about: "a three-way branching coral — every tip splits into three, a third of a turn apart",
        family: Family::Shape,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "30", kind: Kind::Number { min: 5.0, max: 60.0 } }],
    },
    Generator {
        id: "tree",
        name: "tree",
        about: "a tree with a trunk — long wood below, short twigs above, branches off three sides",
        family: Family::Shape,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "20", kind: Kind::Number { min: 5.0, max: 60.0 } }],
    },
    Generator {
        id: "voronoi",
        name: "foam",
        about: "Voronoi foam — the walls between cells scattered in a box, meeting three at an edge as soap films do",
        family: Family::Shape,
        params: &[
            Param { key: "cells", label: "cells", about: "how many centres to scatter", default: "24", kind: Kind::Number { min: 4.0, max: 64.0 } },
            Param { key: "seed", label: "seed", about: "where they land", default: "1", kind: Kind::Seed },
        ],
    },
    Generator {
        id: "mandelbrot",
        name: "Mandelbrot",
        about: "the Mandelbrot set as a relief — the set a plateau, the escape time the country round it",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "julia",
        name: "Julia",
        about: "a Julia set as a relief, c = −0.8 + 0.156i",
        family: Family::Shape,
        params: &[
            Param { key: "cr", label: "c real", about: "the real part of c", default: "-0.8", kind: Kind::Number { min: -2.0, max: 2.0 } },
            Param { key: "ci", label: "c imaginary", about: "the imaginary part of c", default: "0.156", kind: Kind::Number { min: -2.0, max: 2.0 } },
        ],
    },
];

/// The clouds that keep moving: run as a live source, `sim:<id>`, and
/// driven by the audio. Same shape as a generator for the menu's sake;
/// the difference is that these are made sixty times a second.
pub const SIMULATIONS: &[Generator] = &[
    Generator {
        id: "fluid",
        name: "fluid",
        about: "Stam's stable fluids — the Navier–Stokes equations on an endless sheet; the loudness stirs it, the kick bursts it, the snare spins it, the highs roughen it",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "reaction",
        name: "reaction",
        about: "Gray–Scott reaction–diffusion — spots that grow, split and heal; the kick plants new ones",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "flock",
        name: "flock",
        about: "Reynolds' boids — four thousand of them drawing streaks; the loudness is their pace, the kick a predator, the snare a scatter",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "wind",
        name: "wind",
        about: "curl noise — tracers in a divergence-free noise field, a fluid with no solve; the kick is a gust, the highs roughen it",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "kuramoto",
        name: "kuramoto",
        about: "Kuramoto's coupled oscillators on a torus — a loud passage locks them into a ribbon, quiet frees them; the kick scatters half",
        family: Family::Shape,
        params: &[],
    },
    Generator {
        id: "life",
        name: "life",
        about: "a three-dimensional cellular automaton in the Clouds rule — slow masses that keep reshaping; the kick drops a seed",
        family: Family::Shape,
        params: &[
            Param { key: "rule", label: "rule", about: "survive / born, as neighbour counts: 13-26/13-14,17-19 is Clouds; 4/4 is Bays' 4-4; 5-7/6 is a slow builder", default: "13-26/13-14,17-19", kind: Kind::Text },
        ],
    },
];

/// The generator saved or asked for as `gen:<spec>` — the id, with or
/// without its settings.
pub fn by_id(spec: &str) -> Option<&'static Generator> {
    let (id, _) = split_spec(spec);
    CATALOGUE.iter().find(|g| g.id == id)
}

/// The simulation asked for as `sim:<spec>`.
pub fn simulation_by_id(spec: &str) -> Option<&'static Generator> {
    let (id, _) = split_spec(spec);
    SIMULATIONS.iter().find(|g| g.id == id)
}

/// Take a spec apart: `plant?rule=F[+X]F;angle=22` is the id `plant`
/// and two settings. `;` separates settings rather than `&`, because
/// `&` is a turtle turn in an L-system rule.
pub fn split_spec(spec: &str) -> (&str, Vec<(&str, &str)>) {
    match spec.split_once('?') {
        None => (spec, Vec::new()),
        Some((id, rest)) => (
            id,
            rest.split(';')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.trim(), v.trim()))
                .filter(|(k, _)| !k.is_empty())
                .collect(),
        ),
    }
}

/// Put a spec together, leaving out every setting still at its default
/// so the saved line stays short and a default that improves later is
/// taken up.
pub fn spec(g: &Generator, settings: &[(&str, String)]) -> String {
    let changed: Vec<String> = g
        .params
        .iter()
        .filter_map(|p| {
            let value = settings.iter().find(|(k, _)| *k == p.key)?.1.trim();
            (!value.is_empty() && value != p.default).then(|| format!("{}={value}", p.key))
        })
        .collect();
    if changed.is_empty() {
        g.id.to_string()
    } else {
        format!("{}?{}", g.id, changed.join(";"))
    }
}

/// What a made cloud is called: the name, and the seed when the
/// generator was a search — "quadratic #7" is one attractor and
/// "quadratic #8" is another.
pub fn slot_name(spec: &str) -> Option<String> {
    let g = by_id(spec)?;
    let (_, settings) = split_spec(spec);
    let seed = g
        .params
        .iter()
        .find(|p| p.kind == Kind::Seed)
        .map(|p| settings.iter().find(|(k, _)| *k == p.key).map_or(p.default, |(_, v)| *v));
    Some(match seed {
        Some(seed) => format!("{} #{seed}", g.name),
        None => g.name.to_string(),
    })
}

/// The generator or simulation a slot or a look's source names.
pub fn by_name(name: &str) -> Option<&'static Generator> {
    CATALOGUE.iter().chain(SIMULATIONS).find(|g| g.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id is an address in a settings file and on the command line:
    /// lowercase, unique, and never a name that could be a file.
    #[test]
    fn ids_are_stable_addresses() {
        let mut ids: Vec<_> = CATALOGUE.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOGUE.len(), "two generators share an id");
        for g in CATALOGUE {
            assert!(
                g.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} is not a lowercase id",
                g.id
            );
            assert!(!g.about.is_empty() && !g.name.is_empty());
        }
        let mut names: Vec<_> = CATALOGUE.iter().map(|g| g.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOGUE.len(), "two generators share a name");
    }

    /// A look built on a generated cloud files under the generator's
    /// family, not under "clouds" with the scans.
    #[test]
    fn a_generated_look_is_shelved_by_its_family() {
        assert_eq!(by_id("thomas").map(|g| g.name), Some("Thomas"));
        assert_eq!(crate::preset::family(Some("Thomas")), Family::Attractor);
        assert_eq!(crate::preset::family(Some("Hopf")), Family::Shape);
        assert_eq!(crate::preset::family(Some("torso-scan.ply")), Family::Cloud);
        assert!(by_id("no such thing").is_none());
        assert!(by_name("Thomas").is_some());
        assert_eq!(crate::preset::family(Some("fluid")), Family::Shape);
        assert!(simulation_by_id("fluid").is_some() && simulation_by_id("thomas").is_none());
    }

    /// A spec is an id and its settings, and comes back together the
    /// same — with defaults left out, and a turtle's `&` left alone.
    #[test]
    fn a_spec_takes_apart_and_goes_back_together() {
        let (id, settings) = split_spec("plant?rule=F[+&X]F;angle=22");
        assert_eq!(id, "plant");
        assert_eq!(settings, vec![("rule", "F[+&X]F"), ("angle", "22")]);
        assert_eq!(split_spec("thomas"), ("thomas", vec![]));
        let plant = by_id("plant?angle=30").unwrap();
        assert_eq!(plant.id, "plant");
        assert_eq!(spec(plant, &[("angle", "25".into()), ("rule", plant.params[0].default.into())]), "plant");
        assert_eq!(spec(plant, &[("angle", "30".into())]), "plant?angle=30");
        assert_eq!(slot_name("quadratic?seed=7").as_deref(), Some("quadratic #7"));
        assert_eq!(slot_name("quadratic").as_deref(), Some("quadratic #1"));
        assert_eq!(slot_name("thomas").as_deref(), Some("Thomas"));
        assert!(simulation_by_id("life?rule=4/4").is_some());
        for g in CATALOGUE.iter().chain(SIMULATIONS) {
            for p in g.params {
                if let Kind::Number { min, max } = p.kind {
                    let v: f64 = p.default.parse().unwrap_or_else(|_| panic!("{}.{} default is not a number", g.id, p.key));
                    assert!(v >= min && v <= max, "{}.{} default is out of its range", g.id, p.key);
                }
            }
        }
    }
}
