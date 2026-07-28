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

impl Preset {
    /// Capture every parameter's current target.
    pub fn capture(reg: &ParamRegistry) -> Self {
        let values = reg
            .iter()
            .filter(|(_, def)| !excluded(&def.addr))
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
    preset_dir().join(format!("{}.json", crate::library::sanitize(name)))
}

/// User preset names, alphabetical. Empty when the directory does not
/// exist — a fresh install has no user presets and that is not an error.
pub fn list() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(preset_dir()) else {
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

/// Save under a sanitised name, returning the name actually used.
///
/// Written to a temporary file and renamed, so a crash or a full disk
/// part-way through cannot destroy the preset that was already there.
pub fn save(name: &str, preset: &Preset) -> Result<String> {
    let dir = preset_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let path = path_for(name);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(preset)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(crate::library::sanitize(name))
}

pub fn load(name: &str) -> Result<Preset> {
    let path = path_for(name);
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
