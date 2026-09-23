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

/// What a generator *is*, for the menu to put it under a heading. The
/// family decides which shelf a look built on it files under, which is
/// a different question: a Julia set and a torus knot are both shapes
/// to a preset list, and nobody browsing for one would look in the same
/// place for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Integrated in time order; the cloud crawls along itself.
    Flow,
    /// Iterated, and lifted into depth by delay embedding.
    Map,
    /// Found by searching until something is chaotic.
    Searched,
    /// Swept in scan order over two parameters.
    Surface,
    /// Traced along a curve and thickened.
    Curve,
    /// Self-similar at every scale.
    Fractal,
    /// Grown by a rule, step by step.
    Grown,
    /// Sampled where a pattern is.
    Pattern,
    /// A simulation of a field on a grid.
    Field,
    /// A simulation of many bodies.
    Bodies,
}

impl Group {
    /// Every group, in menu order.
    pub const ALL: &'static [Group] = &[
        Group::Flow,
        Group::Map,
        Group::Searched,
        Group::Surface,
        Group::Curve,
        Group::Fractal,
        Group::Grown,
        Group::Pattern,
        Group::Field,
        Group::Bodies,
    ];

    /// The heading the menu shows.
    pub fn label(self) -> &'static str {
        match self {
            Group::Flow => "flows",
            Group::Map => "maps",
            Group::Searched => "searched",
            Group::Surface => "surfaces",
            Group::Curve => "curves",
            Group::Fractal => "fractals",
            Group::Grown => "grown",
            Group::Pattern => "patterns",
            Group::Field => "fields",
            Group::Bodies => "many bodies",
        }
    }
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
    /// Which heading the menu lists it under.
    pub group: Group,
    /// The knobs it takes, in the order the panel shows them. Empty for
    /// most: a named attractor's parameters are what make it that one.
    pub params: &'static [Param],
    /// Where it comes from: whose paper, and when. Every one of these
    /// is somebody's work, and the catalogue is the right place to say
    /// so — the panel can show it, and the site's page is built from
    /// it, so the credit cannot drift away from the code.
    pub cite: &'static str,
    /// Somewhere to read about it, or empty when there is no page
    /// worth sending anyone to.
    pub link: &'static str,
}

/// The shipped set, in menu order: the flows first, then the surfaces
/// and the fractals.
pub const CATALOGUE: &[Generator] = &[
    Generator {
        id: "thomas",
        name: "Thomas",
        about: "Thomas' cyclically symmetric attractor — three sines; a knot of ribbons round the diagonal",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "René Thomas, “Deterministic chaos seen in terms of feedback circuits”, Int. J. Bifurcation and Chaos 9 (1999)",
        link: "https://en.wikipedia.org/wiki/Thomas%27_cyclically_symmetric_attractor",
    },
    Generator {
        id: "halvorsen",
        name: "Halvorsen",
        about: "Halvorsen's attractor — three lobes, cyclically symmetric; sheets folding into each other",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Arne Dehli Halvorsen’s cyclically symmetric system, as catalogued by Sprott",
        link: "https://sprott.physics.wisc.edu/chaos/comchaos.htm",
    },
    Generator {
        id: "dadras",
        name: "Dadras",
        about: "Dadras' tri-scroll — three rolled sheets joined by a spine",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Sara Dadras & Hamid Reza Momeni, Physics Letters A 373 (2009)",
        link: "",
    },
    Generator {
        id: "rossler",
        name: "Rössler",
        about: "Rössler's funnel — a flat spiral that lifts and folds back on itself",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Otto Rössler, “An equation for continuous chaos”, Physics Letters A 57 (1976)",
        link: "https://en.wikipedia.org/wiki/R%C3%B6ssler_attractor",
    },
    Generator {
        id: "four-wing",
        name: "Four-wing",
        about: "the four-wing attractor — four lobes off a saddle",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Wang, Sun & van Wyk, the four-wing system (2009)",
        link: "",
    },
    Generator {
        id: "chen",
        name: "Chen",
        about: "Chen's system — Lorenz's cousin, wider and more tangled",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Guanrong Chen & Tetsushi Ueta, “Yet another chaotic attractor”, Int. J. Bifurcation and Chaos 9 (1999)",
        link: "https://en.wikipedia.org/wiki/Multiscroll_attractor",
    },
    Generator {
        id: "clifford",
        name: "Clifford",
        about: "Pickover's Clifford map, lifted into depth by its own previous step",
        family: Family::Attractor,
        group: Group::Map,
        params: &[],
        cite: "Clifford Pickover, Computers, Pattern, Chaos and Beauty (1990)",
        link: "https://paulbourke.net/fractals/clifford/",
    },
    Generator {
        id: "dejong",
        name: "de Jong",
        about: "the Peter de Jong map of 1987, lifted the same way",
        family: Family::Attractor,
        group: Group::Map,
        params: &[],
        cite: "Peter de Jong, in A. K. Dewdney’s Computer Recreations, Scientific American (1987)",
        link: "https://paulbourke.net/fractals/peterdejong/",
    },
    Generator {
        id: "supershape",
        name: "supershape",
        about: "Gielis' superformula as a solid — a seven-fold flower",
        family: Family::Shape,
        group: Group::Surface,
        params: &[
            Param { key: "m", label: "m", about: "fold: how many times round the symmetry repeats", default: "7", kind: Kind::Number { min: 1.0, max: 16.0 } },
            Param { key: "n1", label: "n1", about: "overall roundness — small pinches, large bloats", default: "2", kind: Kind::Number { min: 0.1, max: 10.0 } },
            Param { key: "n2", label: "n2", about: "the cosine exponent", default: "8", kind: Kind::Number { min: 0.1, max: 20.0 } },
            Param { key: "n3", label: "n3", about: "the sine exponent", default: "4", kind: Kind::Number { min: 0.1, max: 20.0 } },
        ],
        cite: "Johan Gielis, “A generic geometric transformation that unifies a wide range of natural and abstract shapes”, Am. J. Botany 90 (2003)",
        link: "https://en.wikipedia.org/wiki/Superformula",
    },
    Generator {
        id: "harmonic",
        name: "harmonic",
        about: "a sphere rippled by a spherical harmonic",
        family: Family::Shape,
        group: Group::Surface,
        params: &[
            Param { key: "round", label: "round", about: "waves round the equator", default: "3", kind: Kind::Number { min: 1.0, max: 8.0 } },
            Param { key: "up", label: "up", about: "waves pole to pole", default: "2", kind: Kind::Number { min: 1.0, max: 8.0 } },
        ],
        cite: "A sphere modulated by a spherical harmonic; the classical functions of Laplace (1782)",
        link: "https://en.wikipedia.org/wiki/Spherical_harmonics",
    },
    Generator {
        id: "lissajous",
        name: "Lissajous",
        about: "a 3:4:7 Lissajous knot, as a tube",
        family: Family::Shape,
        group: Group::Curve,
        params: &[
            Param { key: "a", label: "a", about: "frequency on x", default: "3", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "b", label: "b", about: "frequency on y", default: "4", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "c", label: "c", about: "frequency on z", default: "7", kind: Kind::Number { min: 1.0, max: 12.0 } },
        ],
        cite: "Jules Antoine Lissajous (1857); the knotted case is Bogle, Hearst, Jones & Stoilov (1994)",
        link: "https://en.wikipedia.org/wiki/Lissajous_knot",
    },
    Generator {
        id: "torus-knot",
        name: "torus knot",
        about: "a (3,7) torus knot — three times round, seven times through",
        family: Family::Shape,
        group: Group::Curve,
        params: &[
            Param { key: "p", label: "p", about: "times round the axis", default: "3", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "q", label: "q", about: "times through the hole", default: "7", kind: Kind::Number { min: 1.0, max: 12.0 } },
        ],
        cite: "The classical (p,q) torus knots",
        link: "https://en.wikipedia.org/wiki/Torus_knot",
    },
    Generator {
        id: "hopf",
        name: "Hopf",
        about: "the Hopf fibration — nested tori of linked circles, projected from the 3-sphere",
        family: Family::Shape,
        group: Group::Surface,
        params: &[],
        cite: "Heinz Hopf, “Über die Abbildungen der dreidimensionalen Sphäre auf die Kugelfläche”, Math. Annalen 104 (1931)",
        link: "https://en.wikipedia.org/wiki/Hopf_fibration",
    },
    Generator {
        id: "chladni",
        name: "Chladni",
        about: "sand on a vibrating plate — points settle where the plate stands still",
        family: Family::Shape,
        group: Group::Pattern,
        params: &[
            Param { key: "n", label: "n", about: "the first mode number", default: "5", kind: Kind::Number { min: 1.0, max: 9.0 } },
            Param { key: "m", label: "m", about: "the second; equal numbers make no pattern", default: "2", kind: Kind::Number { min: 1.0, max: 9.0 } },
        ],
        cite: "Ernst Chladni, Entdeckungen über die Theorie des Klanges (1787)",
        link: "https://en.wikipedia.org/wiki/Ernst_Chladni",
    },
    Generator {
        id: "sierpinski",
        name: "Sierpinski",
        about: "the Sierpinski tetrahedron, by the chaos game",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[],
        cite: "Wacław Sierpiński (1915), by Michael Barnsley’s chaos game",
        link: "https://en.wikipedia.org/wiki/Sierpi%C5%84ski_triangle",
    },
    Generator {
        id: "menger",
        name: "Menger",
        about: "the Menger sponge, by the chaos game",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[],
        cite: "Karl Menger (1926)",
        link: "https://en.wikipedia.org/wiki/Menger_sponge",
    },
    Generator {
        id: "mandelbulb",
        name: "Mandelbulb",
        about: "the power-eight Mandelbulb's surface, found by marching rays inward",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[],
        cite: "Daniel White and Paul Nylander (2009)",
        link: "https://en.wikipedia.org/wiki/Mandelbulb",
    },
    Generator {
        id: "sprott-b",
        name: "Sprott B",
        about: "Sprott's case B — two quadratic terms, and chaos",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Julien Clinton Sprott, “Some simple chaotic flows”, Phys. Rev. E 50 (1994) — case B",
        link: "https://sprott.physics.wisc.edu/chaos/comchaos.htm",
    },
    Generator {
        id: "nose-hoover",
        name: "Nosé–Hoover",
        about: "the Nosé–Hoover oscillator — a thermostatted particle wandering a sea of tori and chaos",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Shūichi Nosé (1984) and William Hoover (1985)",
        link: "https://en.wikipedia.org/wiki/Nos%C3%A9%E2%80%93Hoover_thermostat",
    },
    Generator {
        id: "arneodo",
        name: "Arneodo",
        about: "Arneodo's attractor — a jerk system with one cubic term",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Alain Arnéodo, Pierre Coullet & Charles Tresser (1981)",
        link: "",
    },
    Generator {
        id: "burke-shaw",
        name: "Burke–Shaw",
        about: "Burke–Shaw — two scrolls with the symmetry of a propeller",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Bill Burke & Robert Shaw (1981)",
        link: "",
    },
    Generator {
        id: "chua",
        name: "Chua",
        about: "Chua's circuit — the double scroll, from a real circuit with a nonlinear diode",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Leon Chua (1983); Matsumoto, Chua & Komuro, IEEE Trans. Circuits and Systems 32 (1985)",
        link: "https://en.wikipedia.org/wiki/Chua%27s_circuit",
    },
    Generator {
        id: "hadley",
        name: "Hadley",
        about: "the Hadley circulation — Lorenz's 1984 atmosphere in three variables",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Edward Lorenz, “Irregularity: a fundamental property of the atmosphere”, Tellus 36A (1984)",
        link: "",
    },
    Generator {
        id: "rucklidge",
        name: "Rucklidge",
        about: "Rucklidge's convection model — a tall, folded ribbon",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Alastair Rucklidge, “Chaos in models of double convection”, J. Fluid Mech. 237 (1992)",
        link: "",
    },
    Generator {
        id: "three-scroll",
        name: "three-scroll",
        about: "the three-scroll unified system — three scrolls in one fast flow",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "The three-scroll unified chaotic system; Dequan Li, Physics Letters A 372 (2008)",
        link: "",
    },
    Generator {
        id: "rabinovich",
        name: "Rabinovich–Fabrikant",
        about: "Rabinovich–Fabrikant — leaves and ribbons, from plasma physics",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Mikhail Rabinovich & Anatoly Fabrikant, Sov. Phys. JETP 50 (1979)",
        link: "https://en.wikipedia.org/wiki/Rabinovich%E2%80%93Fabrikant_equations",
    },
    Generator {
        id: "plant",
        name: "plant",
        about: "a plant grown by an L-system — five generations of branching, thick wood to thin twigs",
        family: Family::Shape,
        group: Group::Grown,
        params: &[
            Param { key: "rule", label: "rule", about: "the production for X — F draws, + − & ^ \\ / turn, [ ] branch, X grows again", default: "F[+&X][-^X]/F[\\X]X", kind: Kind::Text },
            Param { key: "angle", label: "angle", about: "degrees per turn", default: "25", kind: Kind::Number { min: 5.0, max: 60.0 } },
        ],
        cite: "Aristid Lindenmayer (1968); Przemysław Prusinkiewicz & Lindenmayer, The Algorithmic Beauty of Plants (1990)",
        link: "https://en.wikipedia.org/wiki/L-system",
    },
    Generator {
        id: "quadratic",
        name: "quadratic",
        about: "Sprott's search: random quadratic maps until one is chaotic — a new attractor every roll",
        family: Family::Attractor,
        group: Group::Searched,
        params: &[Param {
            key: "seed",
            label: "seed",
            about: "which search; the same seed is the same attractor on every machine",
            default: "1",
            kind: Kind::Seed,
        }],
        cite: "Julien Clinton Sprott, Strange Attractors: Creating Patterns in Chaos (1993)",
        link: "https://sprott.physics.wisc.edu/sa.htm",
    },
    Generator {
        id: "aizawa",
        name: "Aizawa",
        about: "Aizawa's attractor — a rotating sphere with a spindle driven through its poles",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Yoji Aizawa & Tatsuo Uezu (1982), as catalogued by Chaoscope",
        link: "",
    },
    Generator {
        id: "newton-leipnik",
        name: "Newton–Leipnik",
        about: "Newton–Leipnik — a tumbling rigid body, and two attractors in one system",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Tim Newton & Roy Leipnik, “Double strange attractors in rigid body motion”, SIAM J. Appl. Math. 41 (1981)",
        link: "",
    },
    Generator {
        id: "sakarya",
        name: "Sakarya",
        about: "the Sakarya system — two lobes crossing at an angle, like a bow tie in wire",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "The Sakarya system (2010)",
        link: "",
    },
    Generator {
        id: "rikitake",
        name: "Rikitake",
        about: "the Rikitake dynamo — why the Earth's magnetic field reverses, and never on schedule",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Tsuneji Rikitake, “Oscillations of a system of disk dynamos”, Math. Proc. Cambridge Phil. Soc. 54 (1958)",
        link: "https://en.wikipedia.org/wiki/Dynamo_theory",
    },
    Generator {
        id: "shimizu-morioka",
        name: "Shimizu–Morioka",
        about: "Shimizu–Morioka — the butterfly's simplest relative, two wings and one quadratic term",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Tsutomu Shimizu & Nobuo Morioka, Physics Letters A 76 (1980)",
        link: "",
    },
    Generator {
        id: "finance",
        name: "finance",
        about: "the finance system — interest rate, investment demand and price index, refusing to settle",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Junhai Ma & Yushu Chen, Applied Mathematics and Mechanics 22 (2001)",
        link: "",
    },
    Generator {
        id: "coullet",
        name: "Coullet",
        about: "Coullet's jerk system — one variable's third derivative, with a cubic pull",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Pierre Coullet, Charles Tresser & Alain Arnéodo (1979) — a jerk system",
        link: "https://en.wikipedia.org/wiki/Jerk_(physics)",
    },
    Generator {
        id: "genesio-tesi",
        name: "Genesio–Tesi",
        about: "Genesio–Tesi — the other classic jerk system, square rather than cubic",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[],
        cite: "Roberto Genesio & Alberto Tesi, Automatica 28 (1992)",
        link: "",
    },
    Generator {
        id: "quadratic-flow",
        name: "quadratic flow",
        about: "the same search with an integrator inside: random quadratic flows until one is chaotic — smooth ribbons where the map gives dust",
        family: Family::Attractor,
        group: Group::Searched,
        params: &[Param {
            key: "seed",
            label: "seed",
            about: "which search; the same seed is the same attractor on every machine",
            default: "1",
            kind: Kind::Seed,
        }],
        cite: "The same search over quadratic flows; Sprott, “Some simple chaotic flows”, Phys. Rev. E 50 (1994)",
        link: "https://sprott.physics.wisc.edu/chaos/comchaos.htm",
    },
    Generator {
        id: "orbital",
        name: "orbital",
        about: "a real spherical harmonic as a balloon — the shape a textbook draws for an atomic orbital",
        family: Family::Shape,
        group: Group::Surface,
        params: &[
            Param { key: "l", label: "l", about: "degree: how many nodal lines in all", default: "3", kind: Kind::Number { min: 0.0, max: 8.0 } },
            Param { key: "m", label: "m", about: "order: how many of them run through the poles; negative turns the lobes", default: "2", kind: Kind::Number { min: -8.0, max: 8.0 } },
        ],
        cite: "The real spherical harmonics, as a chemistry textbook draws an atomic orbital",
        link: "https://en.wikipedia.org/wiki/Spherical_harmonics",
    },
    Generator {
        id: "fern",
        name: "fern",
        about: "a fern by Lindenmayer's rewriting — fronds off a curling spine, rolling as they go",
        family: Family::Shape,
        group: Group::Grown,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "22", kind: Kind::Number { min: 5.0, max: 60.0 } }],
        cite: "The same rewriting, in the fern’s own rule",
        link: "https://en.wikipedia.org/wiki/L-system",
    },
    Generator {
        id: "coral",
        name: "coral",
        about: "a three-way branching coral — every tip splits into three, a third of a turn apart",
        family: Family::Shape,
        group: Group::Grown,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "30", kind: Kind::Number { min: 5.0, max: 60.0 } }],
        cite: "The same rewriting, branching three ways a third of a turn apart",
        link: "https://en.wikipedia.org/wiki/L-system",
    },
    Generator {
        id: "tree",
        name: "tree",
        about: "a tree with a trunk — long wood below, short twigs above, branches off three sides",
        family: Family::Shape,
        group: Group::Grown,
        params: &[Param { key: "angle", label: "angle", about: "degrees per turn", default: "20", kind: Kind::Number { min: 5.0, max: 60.0 } }],
        cite: "The same rewriting, with a trunk that keeps going",
        link: "https://en.wikipedia.org/wiki/L-system",
    },
    Generator {
        id: "voronoi",
        name: "foam",
        about: "Voronoi foam — the walls between cells scattered in a box, meeting three at an edge as soap films do",
        family: Family::Shape,
        group: Group::Pattern,
        params: &[
            Param { key: "cells", label: "cells", about: "how many centres to scatter", default: "24", kind: Kind::Number { min: 4.0, max: 64.0 } },
            Param { key: "seed", label: "seed", about: "where they land", default: "1", kind: Kind::Seed },
        ],
        cite: "Georgy Voronoy (1908); the walls are the set of points equidistant from the two nearest centres",
        link: "https://en.wikipedia.org/wiki/Voronoi_diagram",
    },
    Generator {
        id: "henon",
        name: "Hénon",
        about: "the Hénon map — the first attractor anyone drew that was plainly a fractal",
        family: Family::Attractor,
        group: Group::Map,
        params: &[],
        cite: "Michel Hénon, “A two-dimensional mapping with a strange attractor”, Comm. Math. Phys. 50 (1976)",
        link: "https://en.wikipedia.org/wiki/H%C3%A9non_map",
    },
    Generator {
        id: "ikeda",
        name: "Ikeda",
        about: "the Ikeda map — light in a ring cavity, with a hook in the attractor nothing else here has",
        family: Family::Attractor,
        group: Group::Map,
        params: &[],
        cite: "Kensuke Ikeda, Optics Communications 30 (1979)",
        link: "https://en.wikipedia.org/wiki/Ikeda_map",
    },
    Generator {
        id: "standard",
        name: "standard map",
        about: "Chirikov's standard map on its torus — islands that close, and one orbit that never does",
        family: Family::Attractor,
        group: Group::Map,
        params: &[Param {
            key: "k",
            label: "kick",
            about: "how hard each turn is kicked; at 0.9716 the last ring across the picture breaks",
            default: "0.971635",
            kind: Kind::Number { min: 0.0, max: 4.0 },
        }],
        cite: "Boris Chirikov, “A universal instability of many-dimensional oscillator systems”, Physics Reports 52 (1979); the last-torus threshold is John Greene’s (1979)",
        link: "https://en.wikipedia.org/wiki/Standard_map",
    },
    Generator {
        id: "klein",
        name: "Klein bottle",
        about: "the figure-eight immersion — a surface with no inside, and the crossing that three dimensions force on it",
        family: Family::Shape,
        group: Group::Surface,
        params: &[Param { key: "girth", label: "girth", about: "how fat the tube is against the ring", default: "2", kind: Kind::Number { min: 0.5, max: 5.0 } }],
        cite: "Felix Klein (1882); the figure-eight immersion is the standard one",
        link: "https://en.wikipedia.org/wiki/Klein_bottle",
    },
    Generator {
        id: "boy",
        name: "Boy's surface",
        about: "the projective plane immersed without a boundary — three-fold symmetric, and thought impossible until 1901",
        family: Family::Shape,
        group: Group::Surface,
        params: &[],
        cite: "Werner Boy (1901); this parametrisation is François Apéry’s (1986)",
        link: "https://en.wikipedia.org/wiki/Boy%27s_surface",
    },
    Generator {
        id: "gyroid",
        name: "gyroid",
        about: "a triply periodic minimal surface — two labyrinths that fill space and never touch; also Schwarz's P and D",
        family: Family::Shape,
        group: Group::Surface,
        params: &[
            Param { key: "kind", label: "kind", about: "gyroid, schwarz or diamond", default: "gyroid", kind: Kind::Text },
            Param { key: "cells", label: "cells", about: "how many periods across the box; more is denser and harder to read", default: "1.5", kind: Kind::Number { min: 1.0, max: 6.0 } },
            Param { key: "level", label: "level", about: "0 is the minimal surface; either side of it thickens one labyrinth and thins the other", default: "0", kind: Kind::Number { min: -1.5, max: 1.5 } },
            Param { key: "thickness", label: "wall", about: "how thick to draw the wall", default: "0.07", kind: Kind::Number { min: 0.01, max: 0.4 } },
        ],
        cite: "Alan Schoen, Infinite Periodic Minimal Surfaces Without Self-Intersections, NASA TN D-5541 (1970); Schwarz’ P and D are from 1865",
        link: "https://en.wikipedia.org/wiki/Gyroid",
    },
    Generator {
        id: "quasicrystal",
        name: "quasicrystal",
        about: "six plane waves on the five-fold axes of an icosahedron — a pattern that never repeats and is nowhere random",
        family: Family::Shape,
        group: Group::Pattern,
        params: &[Param { key: "cells", label: "cells", about: "how fine the pattern is", default: "3", kind: Kind::Number { min: 1.0, max: 8.0 } }],
        cite: "Dan Shechtman et al., Phys. Rev. Lett. 53 (1984); Nobel Prize in Chemistry, 2011",
        link: "https://en.wikipedia.org/wiki/Quasicrystal",
    },
    Generator {
        id: "phyllotaxis",
        name: "phyllotaxis",
        about: "the sunflower's own packing — one floret every 137.5°, the only angle that never lines up",
        family: Family::Shape,
        group: Group::Pattern,
        params: &[
            Param { key: "angle", label: "angle", about: "degrees between florets; a tenth off the golden angle and the spiral arms appear", default: "137.50776", kind: Kind::Number { min: 1.0, max: 359.0 } },
            Param { key: "rise", label: "rise", about: "how far the head domes", default: "0.6", kind: Kind::Number { min: 0.0, max: 3.0 } },
        ],
        cite: "The golden angle; Helmut Vogel’s sunflower model (1979)",
        link: "https://en.wikipedia.org/wiki/Phyllotaxis",
    },
    Generator {
        id: "kifs",
        name: "twisted gasket",
        about: "the Sierpinski tetrahedron with a turn folded into every step — shells, spirals and lattices, one number wide",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[
            Param { key: "angle", label: "twist", about: "degrees of turn per step; 0 is the plain gasket", default: "24", kind: Kind::Number { min: -180.0, max: 180.0 } },
            Param { key: "tilt", label: "tilt", about: "degrees of turn about the other axis", default: "0", kind: Kind::Number { min: -180.0, max: 180.0 } },
        ],
        cite: "The Sierpiński chaos game with a rotation in the loop — the kaleidoscopic IFS of the fractal-rendering community",
        link: "https://en.wikipedia.org/wiki/Iterated_function_system",
    },
    Generator {
        id: "dla",
        name: "aggregate",
        about: "diffusion-limited aggregation — particles that wander in and stick where they touch; soot, lightning and copper all do this",
        family: Family::Shape,
        group: Group::Grown,
        params: &[Param { key: "seed", label: "seed", about: "which walk; the same seed is the same dendrite", default: "1", kind: Kind::Seed }],
        cite: "Thomas Witten & Leonard Sander, “Diffusion-limited aggregation, a kinetic critical phenomenon”, Phys. Rev. Lett. 47 (1981)",
        link: "https://en.wikipedia.org/wiki/Diffusion-limited_aggregation",
    },
    Generator {
        id: "mandelbox",
        name: "Mandelbox",
        about: "fold, invert, scale, add — where the Mandelbulb is organic, this is architecture",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[Param { key: "scale", label: "scale", about: "the multiplier in the iteration; negative values turn it inside out", default: "2", kind: Kind::Number { min: -4.0, max: 4.0 } }],
        cite: "Tom Lowe (2010)",
        link: "https://en.wikipedia.org/wiki/Mandelbox",
    },
    Generator {
        id: "quaternion",
        name: "quaternion Julia",
        about: "z ← z² + c in four dimensions, sliced back into three",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[
            Param { key: "cr", label: "c real", about: "the real part of c", default: "-0.2", kind: Kind::Number { min: -2.0, max: 2.0 } },
            Param { key: "ci", label: "c i", about: "the first imaginary part", default: "0.6", kind: Kind::Number { min: -2.0, max: 2.0 } },
            Param { key: "cj", label: "c j", about: "the second imaginary part", default: "0.2", kind: Kind::Number { min: -2.0, max: 2.0 } },
        ],
        cite: "Alan Norton, “Generation and display of geometric fractals in 3-D”, SIGGRAPH 1982",
        link: "https://en.wikipedia.org/wiki/Julia_set",
    },
    Generator {
        id: "hilbert",
        name: "Hilbert curve",
        about: "one unbroken line through every cell of a cube — the cloud crawls along a curve that fills space",
        family: Family::Shape,
        group: Group::Curve,
        params: &[Param { key: "order", label: "order", about: "how many times the curve folds into itself", default: "3", kind: Kind::Number { min: 1.0, max: 5.0 } }],
        cite: "David Hilbert, “Über die stetige Abbildung einer Linie auf ein Flächenstück”, Math. Annalen 38 (1891)",
        link: "https://en.wikipedia.org/wiki/Hilbert_curve",
    },
    Generator {
        id: "lorenz96",
        name: "Lorenz-96",
        about: "Lorenz's toy atmosphere: a ring of variables advecting each other; the test bed every weather forecasting scheme meets first",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[
            Param { key: "size", label: "variables", about: "how many round the ring", default: "5", kind: Kind::Number { min: 4.0, max: 40.0 } },
            Param { key: "forcing", label: "forcing", about: "how hard it is driven; 8 is the chaotic one", default: "8", kind: Kind::Number { min: 0.0, max: 20.0 } },
        ],
        cite: "Edward Lorenz, “Predictability: a problem partly solved”, ECMWF Seminar on Predictability (1996)",
        link: "https://en.wikipedia.org/wiki/Lorenz_96_model",
    },
    Generator {
        id: "duffing",
        name: "Duffing",
        about: "a mass in a double well, shaken — drawn on the cylinder of the forcing phase, so once round the tube is one period",
        family: Family::Attractor,
        group: Group::Flow,
        params: &[Param { key: "drive", label: "drive", about: "how hard it is shaken", default: "0.5", kind: Kind::Number { min: 0.0, max: 2.0 } }],
        cite: "Georg Duffing (1918); the chaotic forced case is Ueda’s (1979)",
        link: "https://en.wikipedia.org/wiki/Duffing_equation",
    },
    Generator {
        id: "gumowski",
        name: "Gumowski–Mira",
        about: "from a study of particle beams at CERN — moths and mandalas that change completely in the third decimal place",
        family: Family::Attractor,
        group: Group::Map,
        params: &[Param { key: "mu", label: "μ", about: "the one number; try small changes", default: "-0.801", kind: Kind::Number { min: -1.0, max: 1.0 } }],
        cite: "Igor Gumowski & Christian Mira, CERN (1980)",
        link: "",
    },
    Generator {
        id: "newton",
        name: "Newton",
        about: "Newton's method for zⁿ = 1 as a relief — basins whose boundary touches every basin at once",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[Param { key: "power", label: "roots", about: "how many roots to chase", default: "3", kind: Kind::Number { min: 2.0, max: 8.0 } }],
        cite: "Arthur Cayley asked the question in 1879 and could not answer it for the cubic",
        link: "https://en.wikipedia.org/wiki/Newton_fractal",
    },
    Generator {
        id: "lyapunov",
        name: "Lyapunov",
        about: "the Markus–Hess fractal — where the logistic map settles into a cycle, drawn as towers over the chaotic sea",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[Param { key: "sequence", label: "pattern", about: "which rate, in turn: AB is the published one, AABAB is another city", default: "AB", kind: Kind::Text }],
        cite: "Mario Markus & Benno Hess, “Lyapunov exponents of the logistic map with periodic forcing”, Computers & Graphics 13 (1989)",
        link: "https://en.wikipedia.org/wiki/Lyapunov_fractal",
    },
    Generator {
        id: "dini",
        name: "Dini",
        about: "a pseudosphere dragged along a helix — constant negative curvature, everywhere the same",
        family: Family::Shape,
        group: Group::Surface,
        params: &[Param { key: "twist", label: "twist", about: "how fast the horn climbs", default: "0.2", kind: Kind::Number { min: 0.0, max: 1.0 } }],
        cite: "Ulisse Dini (1870); a pseudosphere dragged along a helix",
        link: "https://en.wikipedia.org/wiki/Dini%27s_surface",
    },
    Generator {
        id: "enneper",
        name: "Enneper",
        about: "a minimal surface from 1864 that runs through itself twice, which is exactly the interesting part",
        family: Family::Shape,
        group: Group::Surface,
        params: &[],
        cite: "Alfred Enneper (1864)",
        link: "https://en.wikipedia.org/wiki/Enneper_surface",
    },
    Generator {
        id: "spirograph",
        name: "spirograph",
        about: "the curve a pen traces through a hole in a wheel rolling inside another, given a slow rise so it coils",
        family: Family::Shape,
        group: Group::Curve,
        params: &[
            Param { key: "R", label: "wheel", about: "the big wheel", default: "5", kind: Kind::Number { min: 1.0, max: 20.0 } },
            Param { key: "r", label: "roller", about: "the small wheel; the ratio decides the petals", default: "3", kind: Kind::Number { min: 0.2, max: 19.0 } },
            Param { key: "pen", label: "pen", about: "how far the pen sits from the roller's centre", default: "5", kind: Kind::Number { min: 0.1, max: 20.0 } },
            Param { key: "wave", label: "rise", about: "how many times it climbs and falls", default: "1", kind: Kind::Number { min: 0.0, max: 8.0 } },
        ],
        cite: "The hypotrochoid, and Denys Fisher’s 1965 toy",
        link: "https://en.wikipedia.org/wiki/Hypotrochoid",
    },
    Generator {
        id: "figure-eight",
        name: "figure-eight knot",
        about: "the only knot with four crossings, and the simplest one that is its own mirror image",
        family: Family::Shape,
        group: Group::Curve,
        params: &[],
        cite: "The 4₁ knot, the simplest amphichiral one",
        link: "https://en.wikipedia.org/wiki/Figure-eight_knot_(mathematics)",
    },
    Generator {
        id: "mandelbrot",
        name: "Mandelbrot",
        about: "the Mandelbrot set as a relief — the set a plateau, the escape time the country round it",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[],
        cite: "Benoit Mandelbrot (1980); the smooth escape count is Douady and Hubbard’s",
        link: "https://en.wikipedia.org/wiki/Mandelbrot_set",
    },
    Generator {
        id: "julia",
        name: "Julia",
        about: "a Julia set as a relief, c = −0.8 + 0.156i",
        family: Family::Shape,
        group: Group::Fractal,
        params: &[
            Param { key: "cr", label: "c real", about: "the real part of c", default: "-0.8", kind: Kind::Number { min: -2.0, max: 2.0 } },
            Param { key: "ci", label: "c imaginary", about: "the imaginary part of c", default: "0.156", kind: Kind::Number { min: -2.0, max: 2.0 } },
        ],
        cite: "Gaston Julia (1918) and Pierre Fatou",
        link: "https://en.wikipedia.org/wiki/Julia_set",
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
        group: Group::Field,
        params: &[],
        cite: "Jos Stam, “Stable Fluids”, SIGGRAPH 1999, and “Real-Time Fluid Dynamics for Games”, GDC 2003; vorticity confinement from Fedkiw, Stam & Jensen, SIGGRAPH 2001",
        link: "https://www.wdv.com/Aerospace/Fluids/StamFluidsforGames.pdf",
    },
    Generator {
        id: "reaction",
        name: "reaction",
        about: "Gray–Scott reaction–diffusion — spots that grow, split and heal; the kick plants new ones",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "The Gray–Scott system in John Pearson’s parameterisation, Science 261 (1993); the mitosis constants are Karl Sims’",
        link: "https://www.karlsims.com/rd.html",
    },
    Generator {
        id: "flock",
        name: "flock",
        about: "Reynolds' boids — four thousand of them drawing streaks; the loudness is their pace, the kick a predator, the snare a scatter",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Craig Reynolds, “Flocks, Herds and Schools: A Distributed Behavioral Model”, SIGGRAPH 1987",
        link: "https://en.wikipedia.org/wiki/Boids",
    },
    Generator {
        id: "wind",
        name: "wind",
        about: "curl noise — tracers in a divergence-free noise field, a fluid with no solve; the kick is a gust, the highs roughen it",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "Robert Bridson, Jim Hourihan & Marcus Nordenstam, “Curl-Noise for Procedural Fluid Flow”, SIGGRAPH 2007",
        link: "https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph2007-curlnoise.pdf",
    },
    Generator {
        id: "slime",
        name: "slime",
        about: "Physarum: sixty-five thousand agents that leave a trail and follow the strongest one they can see — a transport network out of three rules and no plan; the kick starts a new front",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Jeff Jones, “Characteristics of pattern formation and evolution in approximations of Physarum transport networks”, Artificial Life 16 (2010); Tero et al., Science 327 (2010)",
        link: "https://en.wikipedia.org/wiki/Physarum_polycephalum",
    },
    Generator {
        id: "swarm",
        name: "swarmalators",
        about: "particles that swarm and synchronise at once, each depending on the other — five states from two numbers; the loudness and the mids move between them",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Kevin O’Keeffe, Hyunsuk Hong & Steven Strogatz, “Oscillators that sync and swarm”, Nature Communications 8 (2017)",
        link: "https://www.nature.com/articles/s41467-017-01190-3",
    },
    Generator {
        id: "cloth",
        name: "cloth",
        about: "a sheet of sixty-five thousand particles hung from its top edge in an Arnold–Beltrami–Childress wind; the kick is a gust",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Xavier Provot, “Deformation constraints in a mass-spring model”, Graphics Interface 1995; Thomas Jakobsen, “Advanced Character Physics”, GDC 2001",
        link: "",
    },
    Generator {
        id: "sand",
        name: "sandpile",
        about: "grains dropped and toppled — the model that named self-organised criticality, and a fractal nobody designed; the kick drops a load somewhere else",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "Per Bak, Chao Tang & Kurt Wiesenfeld, “Self-organized criticality”, Phys. Rev. Lett. 59 (1987); Deepak Dhar on the Abelian property (1990)",
        link: "https://en.wikipedia.org/wiki/Abelian_sandpile_model",
    },
    Generator {
        id: "spiral",
        name: "spirals",
        about: "the Belousov–Zhabotinsky reaction — rotating waves that annihilate where they meet, the same dynamics as a heartbeat; the kick seeds fresh defects",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "Boris Belousov (1951) and Anatol Zhabotinsky (1964); the cellular form is Alasdair Turner’s",
        link: "https://en.wikipedia.org/wiki/Belousov%E2%80%93Zhabotinsky_reaction",
    },
    Generator {
        id: "kuramoto",
        name: "kuramoto",
        about: "Kuramoto's coupled oscillators on a torus — a loud passage locks them into a ribbon, quiet frees them; the kick scatters half",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Yoshiki Kuramoto (1975); Steven Strogatz, “From Kuramoto to Crawford”, Physica D 143 (2000)",
        link: "https://en.wikipedia.org/wiki/Kuramoto_model",
    },
    Generator {
        id: "smoke",
        name: "smoke",
        about: "the same solver in three dimensions, with heat — a plume that rises, shears and rolls up; the kick is a blast, the snare a shove, the highs the roughness",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "The same solver in three dimensions; buoyancy after Fedkiw, Stam & Jensen, “Visual Simulation of Smoke”, SIGGRAPH 2001",
        link: "https://www.wdv.com/Aerospace/Fluids/StamFluidsforGames.pdf",
    },
    Generator {
        id: "liquid",
        name: "liquid",
        about: "four thousand particles of water in a tilting box — gravity swings round once a bar, so it pours corner to corner; the kick throws it at the ceiling",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Miles Macklin & Matthias Müller, “Position Based Fluids”, SIGGRAPH 2013; kernels from Müller, Charypar & Gross, SCA 2003",
        link: "https://en.wikipedia.org/wiki/Smoothed-particle_hydrodynamics",
    },
    Generator {
        id: "orbits",
        name: "orbits",
        about: "five hundred bodies pulling on each other round a heavy centre — the loudness is the clock, the kick a shockwave, the snare knocks the disc out of its plane",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Direct-summation gravity, with Plummer softening (1911)",
        link: "https://en.wikipedia.org/wiki/N-body_simulation",
    },
    Generator {
        id: "pendulum",
        name: "pendulum",
        about: "four thousand double pendulums hung in a sheet from neighbouring angles — it swings as one surface, creases, then tears; the loudness is gravity and the kick hangs it again",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "The double pendulum, the standard demonstration that four numbers can be unpredictable",
        link: "https://en.wikipedia.org/wiki/Double_pendulum",
    },
    Generator {
        id: "cyclic",
        name: "cyclic",
        about: "states in a ring, each waiting to be eaten by the next: noise organises itself into scroll waves that turn for ever, because a spiral's own wave comes back round to feed it",
        family: Family::Shape,
        group: Group::Field,
        params: &[],
        cite: "Robert Fisch, Janko Gravner & David Griffeath, “Cyclic cellular automata in two dimensions” (1991); David Griffeath on the spirals’ four snapshots (1994)",
        link: "https://en.wikipedia.org/wiki/Cyclic_cellular_automaton",
    },
    Generator {
        id: "tangle",
        name: "tangle",
        about: "one long elastic rod loose in a wind, tying itself in knots — it cannot stretch and it cannot pass through itself, which is the difference between a tangle and a scribble",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Thomas Jakobsen, “Advanced Character Physics”, GDC (2001); Miklós Bergou et al., “Discrete Elastic Rods”, SIGGRAPH (2008)",
        link: "https://en.wikipedia.org/wiki/Verlet_integration",
    },
    Generator {
        id: "crystal",
        name: "crystal",
        about: "Reiter's snow crystal on a hexagonal lattice — plates, sectored plates and stellar dendrites, sorted by the vapour the mids and highs set; each flake grows out and the next one starts",
        family: Family::Shape,
        group: Group::Grown,
        params: &[],
        cite: "Clifford A. Reiter, “A local cellular model for snow crystal growth”, Chaos, Solitons & Fractals 23 (2005); the morphologies are Ukichiro Nakaya’s (1954)",
        link: "https://en.wikipedia.org/wiki/Snowflake",
    },
    Generator {
        id: "vortex",
        name: "vortex",
        about: "the thin cores smoke rings are made of, moving each other by Biot–Savart — rings that catch the one in front and thread through it; the loudness is their circulation and the kick throws a new one in",
        family: Family::Shape,
        group: Group::Bodies,
        params: &[],
        cite: "Hermann von Helmholtz on vortex motion (1858); Louis Rosenhead’s desingularised kernel (1930); Anthony Leonard, “Vortex methods for flow simulation” (1980)",
        link: "https://en.wikipedia.org/wiki/Vortex_ring",
    },
    Generator {
        id: "life",
        name: "life",
        about: "a three-dimensional cellular automaton in the Pyroclastic rule — a lattice that boils, endlessly; the kick drops a seed",
        family: Family::Shape,
        group: Group::Field,
        params: &[
            Param { key: "rule", label: "rule", about: "survive / born / states, as neighbour counts: 4-7/6-8/10 is Pyroclastic and boils; 2,6,9/4,6,8-9/10 builds; 4/4/5 is Bays' 4-4-5, a spiking crystal; 13-26/13-14,17-19 is Clouds, which grows lovely masses and then stops, so it is started again", default: "4-7/6-8/10", kind: Kind::Text },
        ],
        cite: "Carter Bays, “Candidates for the Game of Life in Three Dimensions”, Complex Systems 1 (1987); the multi-state rules as catalogued by Softology",
        link: "https://softologyblog.wordpress.com/2019/12/28/3d-cellular-automata-3/",
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
            // Lowercase, hyphens and digits: a settings file and a
            // command line both carry these unchanged, and a system
            // named after a year has the year in its name.
            assert!(
                g.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
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

    /// Every one of these is somebody's work, and every entry says
    /// whose. A link is optional — plenty of these have no page worth
    /// sending anyone to — but where there is one it has to be a real
    /// address rather than a note to self.
    #[test]
    fn everything_says_where_it_came_from() {
        for g in CATALOGUE.iter().chain(SIMULATIONS) {
            assert!(g.cite.len() > 12, "{} has no source", g.id);
            assert!(
                g.link.is_empty() || g.link.starts_with("https://"),
                "{} has a link that is not one: {}",
                g.id,
                g.link
            );
        }
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
