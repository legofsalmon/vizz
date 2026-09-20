//! The clouds the app can make from an equation, as the panel and the
//! preset families see them.
//!
//! The mathematics lives in `vizz-render` (`generate.rs`), which this
//! crate cannot see; this is the catalogue — what each generator is
//! called, what it is, and which shelf a look built on it files under.
//! A test in `vizz-app`, which sees both, pins the two lists together
//! so a generator cannot exist in one without the other.

use crate::preset::Family;

/// One generator, as the menu lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// The shipped set, in menu order: the flows first, then the surfaces
/// and the fractals.
pub const CATALOGUE: &[Generator] = &[
    Generator {
        id: "thomas",
        name: "Thomas",
        about: "Thomas' cyclically symmetric attractor — three sines; a knot of ribbons round the diagonal",
        family: Family::Attractor,
    },
    Generator {
        id: "halvorsen",
        name: "Halvorsen",
        about: "Halvorsen's attractor — three lobes, cyclically symmetric; sheets folding into each other",
        family: Family::Attractor,
    },
    Generator {
        id: "dadras",
        name: "Dadras",
        about: "Dadras' tri-scroll — three rolled sheets joined by a spine",
        family: Family::Attractor,
    },
    Generator {
        id: "rossler",
        name: "Rössler",
        about: "Rössler's funnel — a flat spiral that lifts and folds back on itself",
        family: Family::Attractor,
    },
    Generator {
        id: "four-wing",
        name: "Four-wing",
        about: "the four-wing attractor — four lobes off a saddle",
        family: Family::Attractor,
    },
    Generator {
        id: "chen",
        name: "Chen",
        about: "Chen's system — Lorenz's cousin, wider and more tangled",
        family: Family::Attractor,
    },
    Generator {
        id: "clifford",
        name: "Clifford",
        about: "Pickover's Clifford map, lifted into depth by its own previous step",
        family: Family::Attractor,
    },
    Generator {
        id: "dejong",
        name: "de Jong",
        about: "the Peter de Jong map of 1987, lifted the same way",
        family: Family::Attractor,
    },
    Generator {
        id: "supershape",
        name: "supershape",
        about: "Gielis' superformula as a solid — a seven-fold flower",
        family: Family::Shape,
    },
    Generator {
        id: "harmonic",
        name: "harmonic",
        about: "a sphere rippled by a spherical harmonic",
        family: Family::Shape,
    },
    Generator {
        id: "lissajous",
        name: "Lissajous",
        about: "a 3:4:7 Lissajous knot, as a tube",
        family: Family::Shape,
    },
    Generator {
        id: "torus-knot",
        name: "torus knot",
        about: "a (3,7) torus knot — three times round, seven times through",
        family: Family::Shape,
    },
    Generator {
        id: "hopf",
        name: "Hopf",
        about: "the Hopf fibration — nested tori of linked circles, projected from the 3-sphere",
        family: Family::Shape,
    },
    Generator {
        id: "chladni",
        name: "Chladni",
        about: "sand on a vibrating plate — points settle where the plate stands still",
        family: Family::Shape,
    },
    Generator {
        id: "sierpinski",
        name: "Sierpinski",
        about: "the Sierpinski tetrahedron, by the chaos game",
        family: Family::Shape,
    },
    Generator {
        id: "menger",
        name: "Menger",
        about: "the Menger sponge, by the chaos game",
        family: Family::Shape,
    },
    Generator {
        id: "mandelbulb",
        name: "Mandelbulb",
        about: "the power-eight Mandelbulb's surface, found by marching rays inward",
        family: Family::Shape,
    },
    Generator {
        id: "sprott-b",
        name: "Sprott B",
        about: "Sprott's case B — two quadratic terms, and chaos",
        family: Family::Attractor,
    },
    Generator {
        id: "nose-hoover",
        name: "Nosé–Hoover",
        about: "the Nosé–Hoover oscillator — a thermostatted particle wandering a sea of tori and chaos",
        family: Family::Attractor,
    },
    Generator {
        id: "arneodo",
        name: "Arneodo",
        about: "Arneodo's attractor — a jerk system with one cubic term",
        family: Family::Attractor,
    },
    Generator {
        id: "burke-shaw",
        name: "Burke–Shaw",
        about: "Burke–Shaw — two scrolls with the symmetry of a propeller",
        family: Family::Attractor,
    },
    Generator {
        id: "chua",
        name: "Chua",
        about: "Chua's circuit — the double scroll, from a real circuit with a nonlinear diode",
        family: Family::Attractor,
    },
    Generator {
        id: "hadley",
        name: "Hadley",
        about: "the Hadley circulation — Lorenz's 1984 atmosphere in three variables",
        family: Family::Attractor,
    },
    Generator {
        id: "rucklidge",
        name: "Rucklidge",
        about: "Rucklidge's convection model — a tall, folded ribbon",
        family: Family::Attractor,
    },
    Generator {
        id: "three-scroll",
        name: "three-scroll",
        about: "the three-scroll unified system — three scrolls in one fast flow",
        family: Family::Attractor,
    },
    Generator {
        id: "rabinovich",
        name: "Rabinovich–Fabrikant",
        about: "Rabinovich–Fabrikant — leaves and ribbons, from plasma physics",
        family: Family::Attractor,
    },
    Generator {
        id: "plant",
        name: "plant",
        about: "a plant grown by an L-system — five generations of branching, thick wood to thin twigs",
        family: Family::Shape,
    },
    Generator {
        id: "mandelbrot",
        name: "Mandelbrot",
        about: "the Mandelbrot set as a relief — the set a plateau, the escape time the country round it",
        family: Family::Shape,
    },
    Generator {
        id: "julia",
        name: "Julia",
        about: "a Julia set as a relief, c = −0.8 + 0.156i",
        family: Family::Shape,
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
    },
    Generator {
        id: "reaction",
        name: "reaction",
        about: "Gray–Scott reaction–diffusion — spots that grow, split and heal; the kick plants new ones",
        family: Family::Shape,
    },
    Generator {
        id: "flock",
        name: "flock",
        about: "Reynolds' boids — four thousand of them drawing streaks; the loudness is their pace, the kick a predator, the snare a scatter",
        family: Family::Shape,
    },
    Generator {
        id: "wind",
        name: "wind",
        about: "curl noise — tracers in a divergence-free noise field, a fluid with no solve; the kick is a gust, the highs roughen it",
        family: Family::Shape,
    },
    Generator {
        id: "kuramoto",
        name: "kuramoto",
        about: "Kuramoto's coupled oscillators on a torus — a loud passage locks them into a ribbon, quiet frees them; the kick scatters half",
        family: Family::Shape,
    },
    Generator {
        id: "life",
        name: "life",
        about: "a three-dimensional cellular automaton in the Clouds rule — slow masses that keep reshaping; the kick drops a seed",
        family: Family::Shape,
    },
];

/// The generator saved or asked for as `gen:<id>`.
pub fn by_id(id: &str) -> Option<&'static Generator> {
    CATALOGUE.iter().find(|g| g.id == id)
}

/// The simulation asked for as `sim:<id>`.
pub fn simulation_by_id(id: &str) -> Option<&'static Generator> {
    SIMULATIONS.iter().find(|g| g.id == id)
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
}
