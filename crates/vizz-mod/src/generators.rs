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
];

/// The generator saved or asked for as `gen:<id>`.
pub fn by_id(id: &str) -> Option<&'static Generator> {
    CATALOGUE.iter().find(|g| g.id == id)
}

/// The generator a slot or a look's source names.
pub fn by_name(name: &str) -> Option<&'static Generator> {
    CATALOGUE.iter().find(|g| g.name == name)
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
    }
}
