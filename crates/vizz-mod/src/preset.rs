//! Presets: the whole parameter set, captured and recalled by name.
//!
//! Distinct from a patch. A patch is the modulation *graph* — what moves
//! what. A preset is where every knob is sitting. You want them apart:
//! recalling a look should not rewire your LFOs, and loading someone
//! else's patch should not jump your visuals to their settings.
//!
//! Recall does not snap. Values go in as parameter *targets*, and the
//! registry's per-parameter smoothing carries them from wherever they are,
//! so a preset arrives as a glide of a few hundred milliseconds rather
//! than a cut. That is a property of the parameter store rather than
//! anything done here, and it is most of what makes presets usable during
//! a set instead of only between tracks.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use vizz_params::ParamRegistry;

/// Parameters a preset never captures and never writes.
///
/// `/master/dim` is the panic fader. A preset that restores it would mean
/// recalling a look could black out the show — or, worse, silently undo a
/// blackout someone reached for deliberately. It stays wherever the
/// performer left it.
///
/// `/preset/recall` is excluded because a preset containing it would fire
/// another preset on load.
///
/// The `/scene/*` controls are excluded for the same reason and one worse.
/// A scene cell is a captured preset, so a cell storing `/scene/fire`
/// would fire a scene the moment it arrived — including itself, forever.
/// The rest of them describe *how* you move between scenes, which belongs
/// to the performer and the track rather than to the look being moved to:
/// a scene that reset your blend time every time you reached it would be
/// unplayable.
pub const EXCLUDED: &[&str] = &[
    "/master/dim",
    "/preset/recall",
    "/scene/fire",
    "/scene/time",
    "/scene/curve",
    "/scene/auto",
    "/scene/bars",
    // The gravity grid's transport, for exactly the reasons above: a
    // gravity preset holding "/gravity/fire" would fire itself forever.
    "/gravity/fire",
    "/gravity/time",
    "/gravity/curve",
    "/gravity/auto",
    "/gravity/bars",
];

fn excluded(addr: &str) -> bool {
    EXCLUDED.contains(&addr)
}

/// A snapshot of parameter values, addressed the same way OSC addresses
/// them.
///
/// Stored by address rather than by index so a file keeps working when
/// parameters are added, removed or reordered — which they are, every
/// release. An address this build does not have is skipped on load.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// Address → value. `BTreeMap` so the JSON is ordered and two saves of
    /// the same look produce identical files.
    pub values: BTreeMap<String, f32>,
}

/// Which layer a preset belongs to.
///
/// The two are independent on purpose. Gravity sits *over* the look: you
/// pick a shape and a palette, and separately you decide what bends it.
/// If a look captured the gravity parameters then firing a scene would
/// silently reset the wells, and the layering would be a fiction — you
/// would have two grids that fight each other rather than two that
/// compose. So each kind captures only its own addresses, and neither can
/// disturb the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Shape, colour, camera, room, effects. Everything but gravity.
    ///
    /// The default, so a grid deserialised without one behaves as the
    /// original single-layer grid did rather than as an empty gravity one.
    #[default]
    Look,
    /// The gravity wells and nothing else.
    Gravity,
}

impl Kind {
    /// Whether this kind captures and applies the given parameter.
    ///
    /// Takes the definition rather than the address so transport comes
    /// from [`ParamDef::transport`] — one source of truth, rather than the
    /// hand-maintained list this used to consult and which had already
    /// drifted once when a layer was added.
    pub fn owns_def(self, def: &vizz_params::ParamDef) -> bool {
        if def.transport || excluded(&def.addr) {
            return false;
        }
        let gravity = def.addr.starts_with("/gravity/");
        match self {
            Kind::Look => !gravity,
            Kind::Gravity => gravity,
        }
    }

    /// Whether this kind captures and applies the given address.
    pub fn owns(self, addr: &str) -> bool {
        let gravity = addr.starts_with("/gravity/");
        match self {
            // The transport parameters are excluded from both: they say
            // *when* things happen, not what anything looks like.
            Kind::Look => !gravity && !excluded(addr),
            Kind::Gravity => gravity && !excluded(addr),
        }
    }

    pub fn dir(self) -> PathBuf {
        let base = crate::library::patch_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        match self {
            Kind::Look => base.join("presets"),
            Kind::Gravity => base.join("gravity"),
        }
    }
}

impl Preset {
    /// Capture every parameter's current target.
    pub fn capture(reg: &ParamRegistry) -> Self {
        Self::capture_kind(reg, Kind::Look)
    }

    /// Capture only the parameters belonging to one layer.
    pub fn capture_kind(reg: &ParamRegistry, kind: Kind) -> Self {
        let values = reg
            .iter()
            .filter(|(_, def)| kind.owns_def(def))
            .map(|(id, def)| (def.addr.clone(), reg.target(id)))
            .collect();
        Self { values }
    }

    /// Write the preset into the registry.
    ///
    /// Unknown addresses are skipped, not an error: a preset written by a
    /// newer build, or one carrying a parameter since renamed, should
    /// still recall everything it *can*. Refusing the whole thing because
    /// one key moved is the wrong trade at a venue.
    ///
    /// Returns how many parameters were applied, which the UI uses to
    /// tell "recalled" from "recalled nothing".
    pub fn apply(&self, reg: &ParamRegistry) -> usize {
        self.values
            .iter()
            .filter(|(addr, _)| !excluded(addr))
            // `set_by_addr` clamps to the parameter's range, so a file
            // hand-edited to something absurd cannot push a value out of
            // bounds — it lands at the end of the range instead.
            .filter(|(addr, value)| reg.set_by_addr(addr, **value))
            .count()
    }
}

/// Where user presets live, beside patches and the MIDI map so all user
/// state is in one directory to back up or copy between machines.
pub fn preset_dir() -> PathBuf {
    crate::library::patch_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("presets")
}

fn path_for(name: &str) -> PathBuf {
    path_for_kind(Kind::Look, name)
}

fn path_for_kind(kind: Kind, name: &str) -> PathBuf {
    kind.dir().join(format!("{}.json", crate::library::sanitize(name)))
}

/// User preset names, alphabetical. Empty when the directory does not
/// exist — a fresh install has no user presets and that is not an error.
pub fn list() -> Vec<String> {
    list_kind(Kind::Look)
}

/// As [`list`], for one layer.
pub fn list_kind(kind: Kind) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(kind.dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            (path.extension()? == "json").then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    names
}

/// How long a cached listing is trusted before the directory is read
/// again. Only external changes — a file dropped into the folder by hand —
/// need this at all; anything the app does refreshes the cache itself.
const RESCAN: std::time::Duration = std::time::Duration::from_secs(2);

/// A cached listing of the preset library.
///
/// The panel and both grids ask the same two questions every frame: what
/// presets are there, and does this pad's preset still exist. Answered
/// straight from the filesystem, the first is a directory scan per layer
/// and the second was a file read *and a JSON parse* per filled pad —
/// `by_name` loads the whole preset to decide whether it is there.
///
/// A full grid on both layers made that around fifty file operations per
/// frame, on the render thread, sixty times a second, whether or not the
/// panel was even on screen. On a laptop with the library on a network
/// home directory that is not a micro-optimisation.
///
/// The library changes when the app saves or deletes something, which
/// refreshes this directly, or when someone edits the folder behind its
/// back, which the interval catches.
pub struct Library {
    looks: Vec<String>,
    gravity: Vec<String>,
    scanned: std::time::Instant,
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    pub fn new() -> Self {
        Self {
            looks: list_kind(Kind::Look),
            gravity: list_kind(Kind::Gravity),
            scanned: std::time::Instant::now(),
        }
    }

    /// Rescan if the listing is stale. Cheap enough to call every frame —
    /// it is a clock comparison in all but one frame in a hundred and
    /// twenty.
    pub fn tick(&mut self) {
        if self.scanned.elapsed() >= RESCAN {
            self.refresh();
        }
    }

    /// Rescan now. Called after the app itself writes to the library, so
    /// a saved preset appears on the pads in the same frame rather than
    /// whenever the interval next comes round.
    pub fn refresh(&mut self) {
        self.looks = list_kind(Kind::Look);
        self.gravity = list_kind(Kind::Gravity);
        self.scanned = std::time::Instant::now();
    }

    /// User preset names for one layer, alphabetical.
    pub fn user(&self, kind: Kind) -> &[String] {
        match kind {
            Kind::Look => &self.looks,
            Kind::Gravity => &self.gravity,
        }
    }

    /// Everything a pad can name, in the order `/preset/recall` numbers
    /// them: built-ins first, then what is on disk.
    pub fn all(&self, kind: Kind) -> Vec<String> {
        match kind {
            Kind::Look => BUILTINS
                .iter()
                .map(|b| b.name.to_string())
                .chain(self.looks.iter().cloned())
                .collect(),
            Kind::Gravity => self.gravity.clone(),
        }
    }

    /// Can this name still be resolved? The question a pad asks to decide
    /// whether it is pointing at nothing.
    ///
    /// Built-ins count for looks and are checked first, matching
    /// [`by_name`] — a pad naming a built-in is never dangling, whatever
    /// is on disk.
    ///
    /// The listing holds file stems, which are sanitised; a cell holds
    /// whatever was typed. `load` sanitises on the way to a path, so
    /// comparing the two directly would report every preset with a
    /// character the filesystem does not take as missing — a pad outlined
    /// in red that fires perfectly well.
    pub fn has(&self, kind: Kind, name: &str) -> bool {
        if kind == Kind::Look && BUILTINS.iter().any(|b| b.name == name) {
            return true;
        }
        let wanted = crate::library::sanitize(name);
        self.user(kind).contains(&wanted)
    }
}

/// Save under a sanitised name, returning the name actually used.
///
/// Written to a temporary file and renamed, so a crash or a full disk
/// part-way through cannot destroy the preset that was already there.
pub fn save(name: &str, preset: &Preset) -> Result<String> {
    save_kind(Kind::Look, name, preset)
}

/// As [`save`], for one layer.
pub fn save_kind(kind: Kind, name: &str, preset: &Preset) -> Result<String> {
    let dir = kind.dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let path = path_for_kind(kind, name);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(preset)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(crate::library::sanitize(name))
}

pub fn load(name: &str) -> Result<Preset> {
    load_kind(Kind::Look, name)
}

/// As [`load`], for one layer.
pub fn load_kind(kind: Kind, name: &str) -> Result<Preset> {
    let path = path_for_kind(kind, name);
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

pub fn delete(name: &str) -> Result<()> {
    let path = path_for(name);
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))
}

pub fn exists(name: &str) -> bool {
    path_for(name).exists()
}

/// A preset compiled into the binary.
pub struct Builtin {
    pub name: &'static str,
    /// One line on what it is for, shown as a tooltip.
    pub about: &'static str,
    pub values: &'static [(&'static str, f32)],
}

impl Builtin {
    pub fn preset(&self) -> Preset {
        Preset {
            values: self
                .values
                .iter()
                .map(|(a, v)| ((*a).to_string(), *v))
                .collect(),
        }
    }
}

/// The presets that ship with the app.
///
/// Compiled in rather than written to disk on first run: they are then
/// always present, cannot be half-installed, and cannot be lost by
/// clearing the config directory. They are also read-only — "reset to how
/// it shipped" has to stay available, and a starting point you can destroy
/// is not a starting point.
///
/// Each sets only the parameters that matter to its look. Everything else
/// stays where the performer left it, so recalling a preset mid-set
/// changes the thing you asked for and nothing else.
pub const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "Slow bloom",
        about: "Wide breathing sphere. A neutral opener that sits under anything.",
        values: &[
            ("/particles/count", 90_000.0),
            ("/particles/size", 0.013),
            ("/particles/speed", 0.35),
            ("/particles/spread", 1.35),
            ("/particles/brightness", 1.0),
            ("/shape/mode", 0.0),
            ("/shape/morph", 0.0),
            ("/shape/twist", 0.0),
            ("/fx/trail", 0.55),
            ("/fx/zoom", 1.0),
            ("/fx/glow", 0.45),
            ("/fx/mirror", 0.0),
            ("/fx/shift", 0.06),
            ("/color/palette", 2.0),
            ("/color/spread", 0.3),
            ("/color/drive", 1.0),
            ("/camera/distance", 3.6),
            ("/camera/elevation", 0.3),
            ("/camera/fov", 0.9),
            ("/camera/defocus", 0.12),
            ("/room/brightness", 0.0),
        ],
    },
    Builtin {
        name: "Butterfly",
        about: "The Lorenz attractor, lit tight and close. Reads as a solid object.",
        values: &[
            ("/particles/count", 140_000.0),
            ("/particles/size", 0.007),
            ("/particles/speed", 0.8),
            ("/particles/spread", 1.1),
            ("/particles/brightness", 0.75),
            ("/shape/mode", 5.0),
            ("/shape/morph", 0.0),
            ("/shape/twist", 0.0),
            ("/fx/trail", 0.3),
            ("/fx/glow", 0.35),
            ("/fx/mirror", 0.0),
            ("/fx/shift", 0.0),
            ("/color/palette", 3.0),
            ("/color/spread", 0.5),
            ("/color/drive", 2.0),
            ("/camera/distance", 3.2),
            ("/camera/elevation", 0.62),
            ("/camera/fov", 0.85),
            ("/camera/defocus", 0.0),
            ("/room/brightness", 0.0),
        ],
    },
    Builtin {
        name: "Tunnel",
        about: "Grid plane driven into feedback. The high-energy one.",
        values: &[
            ("/particles/count", 120_000.0),
            ("/particles/size", 0.01),
            ("/particles/speed", 0.9),
            ("/particles/spread", 1.5),
            ("/particles/brightness", 0.9),
            ("/shape/mode", 3.0),
            ("/shape/twist", 0.25),
            ("/fx/trail", 0.9),
            ("/fx/zoom", 1.035),
            ("/fx/spin", 0.012),
            ("/fx/glow", 0.5),
            ("/fx/mirror", 0.0),
            ("/fx/shift", 0.25),
            ("/color/palette", 1.0),
            ("/color/spread", 0.55),
            ("/color/drive", 1.0),
            ("/camera/distance", 2.4),
            ("/camera/elevation", 0.12),
            ("/camera/fov", 1.15),
            ("/room/brightness", 0.0),
        ],
    },
    Builtin {
        name: "Stage",
        about: "Cloud sitting inside the room, forced perspective on. Depth without motion.",
        values: &[
            ("/particles/count", 80_000.0),
            ("/particles/size", 0.012),
            ("/particles/speed", 0.25),
            ("/particles/spread", 0.9),
            ("/particles/brightness", 1.1),
            ("/shape/mode", 4.0),
            ("/shape/twist", 0.0),
            ("/fx/trail", 0.35),
            ("/fx/glow", 0.3),
            ("/fx/mirror", 0.0),
            ("/fx/shift", 0.0),
            ("/color/palette", 0.0),
            ("/particles/hue", 0.55),
            ("/particles/saturation", 0.55),
            ("/color/spread", 0.15),
            ("/camera/distance", 3.5),
            ("/camera/elevation", 0.05),
            ("/camera/fov", 0.95),
            ("/room/brightness", 0.75),
            ("/room/depth", 9.0),
            ("/room/fade", 0.8),
            ("/room/converge", 0.3),
            ("/room/anchor", 0.45),
            ("/room/embed", 1.0),
        ],
    },
    Builtin {
        name: "Confetti",
        about: "Dense, fast, per-particle colour. Good over a busy track.",
        values: &[
            ("/particles/count", 260_000.0),
            ("/particles/size", 0.005),
            ("/particles/speed", 1.6),
            ("/particles/spread", 1.6),
            ("/particles/brightness", 0.85),
            ("/shape/mode", 0.0),
            ("/shape/morph", 0.35),
            ("/shape/twist", 0.5),
            ("/fx/trail", 0.2),
            ("/fx/glow", 0.6),
            ("/fx/mirror", 0.0),
            ("/fx/shift", 0.15),
            ("/color/palette", 4.0),
            ("/color/spread", 1.0),
            ("/color/drive", 0.0),
            ("/camera/distance", 3.2),
            ("/camera/elevation", 0.25),
            ("/camera/fov", 1.0),
            ("/room/brightness", 0.0),
        ],
    },
    Builtin {
        name: "Ribbon",
        about: "Torus sheared into ribbons by twist. Slow and readable.",
        values: &[
            ("/particles/count", 110_000.0),
            ("/particles/size", 0.009),
            ("/particles/speed", 0.5),
            ("/particles/spread", 1.25),
            ("/particles/brightness", 1.0),
            ("/shape/mode", 1.0),
            ("/shape/morph", 0.0),
            ("/shape/twist", 0.85),
            ("/fx/trail", 0.65),
            ("/fx/zoom", 1.0),
            ("/fx/glow", 0.4),
            ("/fx/mirror", 2.0),
            ("/fx/shift", 0.1),
            ("/color/palette", 2.0),
            ("/color/spread", 0.4),
            ("/color/drive", 3.0),
            ("/camera/distance", 3.4),
            ("/camera/elevation", 0.45),
            ("/camera/fov", 0.9),
            ("/room/brightness", 0.0),
        ],
    },
];

/// Built-in names followed by user names, which is the order the UI shows
/// and the order `/preset/recall` indexes. Built-ins first and fixed, so
/// an index learned onto a MIDI button keeps meaning the same preset even
/// after saving new ones.
pub fn all_names() -> Vec<String> {
    BUILTINS
        .iter()
        .map(|b| b.name.to_string())
        .chain(list())
        .collect()
}

/// Resolve a name to a preset, preferring built-ins. A user preset saved
/// under a built-in's name therefore cannot shadow it — the shipped
/// starting points stay reachable whatever is on disk.
pub fn by_name(name: &str) -> Option<Preset> {
    if let Some(b) = BUILTINS.iter().find(|b| b.name == name) {
        return Some(b.preset());
    }
    load(name).ok()
}

/// Resolve by position in [`all_names`], for `/preset/recall`.
pub fn by_index(i: usize) -> Option<(String, Preset)> {
    let name = all_names().into_iter().nth(i)?;
    by_name(&name).map(|p| (name, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listing is of file *stems*, which are sanitised, while a pad
    /// holds the name as it was typed. Comparing the two directly reports
    /// every preset whose name has a character the filesystem will not
    /// take as missing — a pad outlined in red, refusing to be played,
    /// that would fire perfectly well if it were pressed.
    #[test]
    fn a_preset_whose_name_needed_sanitising_is_still_found() {
        let (_guard, _tmp) = crate::test_env::scoped("library-sanitise");
        let name = "Warehouse: 2am";
        let saved = save(name, &Preset { values: Default::default() }).unwrap();
        assert_ne!(saved, name, "this test needs a name that gets rewritten");

        let lib = Library::new();
        assert!(lib.has(Kind::Look, name), "the name as typed was reported missing");
        assert!(lib.has(Kind::Look, &saved), "the name as saved was reported missing");
        // And it agrees with the slow path it replaced.
        assert!(by_name(name).is_some());
    }

    /// A built-in is never dangling, whatever is on disk — matching
    /// `by_name`, which prefers built-ins and cannot be shadowed.
    #[test]
    fn a_pad_naming_a_builtin_is_never_reported_missing() {
        let (_guard, _tmp) = crate::test_env::scoped("library-builtin");
        let lib = Library::new();
        let builtin = BUILTINS[0].name;
        assert!(lib.has(Kind::Look, builtin));
        assert!(!lib.has(Kind::Look, "nothing called this"));
        // Built-ins are looks only: a gravity pad naming one is dangling,
        // because gravity has no built-in library to resolve it against.
        assert!(!lib.has(Kind::Gravity, builtin));
    }

    /// The cache exists so the panel stops reading the disk every frame;
    /// it is only correct if a save the app itself makes shows up at once
    /// rather than whenever the rescan interval next comes round.
    #[test]
    fn saving_a_preset_shows_up_after_a_refresh() {
        let (_guard, _tmp) = crate::test_env::scoped("library-refresh");
        let mut lib = Library::new();
        assert!(!lib.has(Kind::Look, "later"));

        save("later", &Preset { values: Default::default() }).unwrap();
        // Deliberately stale until told: this is what makes it a cache.
        assert!(!lib.has(Kind::Look, "later"));
        lib.refresh();
        assert!(lib.has(Kind::Look, "later"));
        assert!(lib.all(Kind::Look).iter().any(|n| n == "later"));
    }

    /// The two layers are separate libraries, and a cache that merged them
    /// would put looks on the gravity pads' assign menu.
    #[test]
    fn the_cache_keeps_the_two_layers_apart() {
        let (_guard, _tmp) = crate::test_env::scoped("library-layers");
        save_kind(Kind::Look, "a look", &Preset { values: Default::default() }).unwrap();
        save_kind(Kind::Gravity, "a well", &Preset { values: Default::default() }).unwrap();
        let lib = Library::new();

        assert!(lib.has(Kind::Look, "a look"));
        assert!(!lib.has(Kind::Look, "a well"));
        assert!(lib.has(Kind::Gravity, "a well"));
        assert!(!lib.has(Kind::Gravity, "a look"));
        // Gravity has no built-ins, so its list is exactly what is on disk.
        assert_eq!(lib.all(Kind::Gravity), vec!["a well".to_string()]);
    }

    /// The two layers must not be able to disturb each other.
    ///
    /// This is the property the whole idea rests on. Gravity sits *over*
    /// the look: if a look captured the wells then firing a scene would
    /// silently reset them, and two grids that were supposed to compose
    /// would fight instead — with no way to tell from the screen which
    /// one had won.
    #[test]
    fn a_look_and_a_gravity_preset_capture_disjoint_parameters() {
        let mut b = ParamRegistry::builder();
        b.add(vizz_params::ParamDef::new("/particles/size", 0.0, 1.0, 0.3));
        b.add(vizz_params::ParamDef::new("/gravity/amount", 0.0, 1.0, 0.7));
        b.add(vizz_params::ParamDef::new("/gravity/0/strength", -2.0, 2.0, 1.5));
        b.add(vizz_params::ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.add(vizz_params::ParamDef::new("/gravity/fire", 0.0, 16.0, 3.0));
        let reg = b.build();

        let look = Preset::capture_kind(&reg, Kind::Look);
        let gravity = Preset::capture_kind(&reg, Kind::Gravity);

        assert!(look.values.contains_key("/particles/size"));
        assert!(
            !look.values.keys().any(|k| k.starts_with("/gravity/")),
            "a look captured gravity: {:?}",
            look.values.keys().collect::<Vec<_>>()
        );

        assert!(gravity.values.contains_key("/gravity/amount"));
        assert!(gravity.values.contains_key("/gravity/0/strength"));
        assert!(
            gravity.values.keys().all(|k| k.starts_with("/gravity/")),
            "a gravity preset captured something else: {:?}",
            gravity.values.keys().collect::<Vec<_>>()
        );

        // Neither captures the panic fader or either transport, and the
        // two sets share nothing at all.
        assert!(!look.values.contains_key("/master/dim"));
        assert!(!gravity.values.contains_key("/gravity/fire"));
        assert!(
            look.values.keys().all(|k| !gravity.values.contains_key(k)),
            "the two layers overlap"
        );
    }
    use vizz_params::{ParamDef, ParamRegistry};

    fn registry() -> ParamRegistry {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/particles/size", 0.001, 0.2, 0.015));
        b.add(ParamDef::new("/shape/mode", 0.0, 8.0, 0.0));
        b.add(ParamDef::new("/fx/glow", 0.0, 1.0, 0.25));
        b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.add(ParamDef::new("/preset/recall", 0.0, 63.0, 0.0));
        b.build()
    }

    #[test]
    fn a_captured_preset_restores_what_it_captured() {
        let reg = registry();
        reg.set_by_addr("/fx/glow", 0.8);
        reg.set_by_addr("/shape/mode", 5.0);
        let snap = Preset::capture(&reg);

        reg.set_by_addr("/fx/glow", 0.1);
        reg.set_by_addr("/shape/mode", 1.0);
        assert!(snap.apply(&reg) >= 2);

        assert!((reg.target(reg.id("/fx/glow").unwrap()) - 0.8).abs() < 1e-6);
        assert!((reg.target(reg.id("/shape/mode").unwrap()) - 5.0).abs() < 1e-6);
    }

    /// The master dim is the fader you reach for when something is wrong.
    /// A preset restoring it could black out a running show, or quietly
    /// undo a blackout somebody meant. It must be untouched in both
    /// directions.
    #[test]
    fn presets_never_touch_the_master_dim() {
        let reg = registry();
        let captured = Preset::capture(&reg);
        assert!(!captured.values.contains_key("/master/dim"));

        // Even a hand-written file naming it explicitly is ignored.
        let mut hostile = Preset::default();
        hostile.values.insert("/master/dim".into(), 0.0);
        reg.set_by_addr("/master/dim", 1.0);
        hostile.apply(&reg);
        assert_eq!(
            reg.target(reg.id("/master/dim").unwrap()),
            1.0,
            "a preset blacked out the output"
        );
    }

    /// Likewise the recall control itself, or loading a preset would fire
    /// another one and the two could ping-pong.
    #[test]
    fn presets_never_capture_the_recall_control() {
        let reg = registry();
        reg.set_by_addr("/preset/recall", 3.0);
        assert!(!Preset::capture(&reg).values.contains_key("/preset/recall"));
    }

    /// Parameters come and go every release, so a file will eventually
    /// name something this build does not have. It must recall the rest
    /// rather than refusing — losing one knob beats losing the look.
    #[test]
    fn unknown_addresses_are_skipped_not_fatal() {
        let reg = registry();
        let mut p = Preset::default();
        p.values.insert("/fx/glow".into(), 0.7);
        p.values.insert("/fx/from-the-future".into(), 1.0);
        assert_eq!(p.apply(&reg), 1, "should apply the one it knows");
        assert!((reg.target(reg.id("/fx/glow").unwrap()) - 0.7).abs() < 1e-6);
    }

    /// A hand-edited file must not push a parameter outside its range.
    #[test]
    fn out_of_range_values_clamp() {
        let reg = registry();
        let mut p = Preset::default();
        p.values.insert("/fx/glow".into(), 99.0);
        p.apply(&reg);
        assert_eq!(reg.target(reg.id("/fx/glow").unwrap()), 1.0);
    }

    /// Built-ins name real parameters and stay in range — asserted in
    /// `vizz-app`, where the actual registry lives. Here we can only check
    /// the shape.
    #[test]
    fn builtins_are_non_empty_and_exclude_nothing_dangerous() {
        for b in BUILTINS {
            assert!(!b.values.is_empty(), "{} is empty", b.name);
            assert!(!b.about.is_empty(), "{} has no description", b.name);
            for (addr, _) in b.values {
                assert!(!excluded(addr), "{}: {addr} must not be in a preset", b.name);
            }
        }
    }

    /// Names must be distinct: the UI lists them and `/preset/recall`
    /// indexes them, so a duplicate makes one unreachable.
    #[test]
    fn builtin_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for b in BUILTINS {
            assert!(seen.insert(b.name), "duplicate built-in name: {}", b.name);
        }
    }

    /// A user preset must not be able to hide a shipped one — "put it back
    /// how it was" has to stay reachable.
    #[test]
    fn a_user_preset_cannot_shadow_a_builtin() {
        let (_guard, dir) = crate::test_env::scoped("preset-shadow");
        let mut impostor = Preset::default();
        impostor.values.insert("/fx/glow".into(), 0.99);
        save(BUILTINS[0].name, &impostor).unwrap();

        let got = by_name(BUILTINS[0].name).unwrap();
        assert_eq!(got, BUILTINS[0].preset(), "a file shadowed a built-in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_through_disk() {
        let (_guard, dir) = crate::test_env::scoped("preset");
        assert!(list().is_empty(), "fresh config dir should have no presets");

        let mut p = Preset::default();
        p.values.insert("/fx/glow".into(), 0.42);
        let saved = save("Warehouse set 2", &p).unwrap();
        assert_eq!(saved, "Warehouse set 2");
        assert_eq!(list(), vec!["Warehouse set 2"]);
        assert_eq!(load("Warehouse set 2").unwrap(), p);
        assert!(exists("Warehouse set 2"));

        delete("Warehouse set 2").unwrap();
        assert!(list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Preset names are user-typed and become filenames, exactly like
    /// patch names. Same guarantee, asserted separately because it is a
    /// different directory and a separate code path.
    #[test]
    fn names_cannot_escape_the_preset_directory() {
        let (_guard, _tmp) = crate::test_env::scoped("preset-escape");
        let dir = preset_dir();
        for evil in ["../../../.ssh/authorized_keys", "..", "/etc/passwd", "a/b", "."] {
            let path = path_for(evil);
            assert!(path.starts_with(&dir), "{evil:?} escaped to {}", path.display());
            assert_eq!(path.parent().unwrap(), dir, "{evil:?} left the directory");
        }
    }

    /// Built-ins come first and keep their order, so a recall index bound
    /// to a MIDI button does not change meaning when a user preset is
    /// saved.
    #[test]
    fn saving_a_user_preset_does_not_renumber_the_builtins() {
        let (_guard, dir) = crate::test_env::scoped("preset-order");
        let before = all_names();
        save("aaa-sorts-first", &Preset::default()).unwrap();
        let after = all_names();
        assert_eq!(&after[..BUILTINS.len()], &before[..BUILTINS.len()]);
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(by_index(0).unwrap().0, BUILTINS[0].name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
