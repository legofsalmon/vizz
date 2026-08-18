//! Patch storage: saving, loading and listing modulation graphs.
//!
//! A patch is the whole graph — nodes, wiring, and canvas layout — as JSON
//! on disk. Layout is included deliberately: a patch that reloads with its
//! nodes rearranged has to be re-read from scratch, which defeats the point
//! of saving it.
//!
//! Names are user-supplied and become filenames, so they are sanitised
//! rather than trusted. `../../../.ssh/config` is a patch name someone can
//! type, and it must land in the patch directory as a mangled filename, not
//! anywhere else.

use std::path::PathBuf;

use anyhow::{Context as _, Result};

use crate::graph::NodeGraph;

/// Where patches live: inside the open show, beside its presets and its
/// grids. A patch is something you built for a set, so it travels with
/// the set — see [`crate::project`].
pub fn patch_dir() -> PathBuf {
    crate::project::show_dir().join("patches")
}

/// Reduce a user-typed name to something safe to use as a filename.
///
/// Everything outside a conservative allowlist becomes `_`, which also
/// disposes of path separators, `..`, NUL and Windows reserved characters
/// in one step rather than by enumerating attacks.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | ' ' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        // Long enough for a descriptive name, short of any filesystem limit.
        trimmed.chars().take(64).collect()
    }
}

fn path_for(name: &str) -> PathBuf {
    patch_dir().join(format!("{}.json", sanitize(name)))
}

/// Patch names, alphabetical. Empty when the directory does not exist yet —
/// a fresh install has no patches and that is not an error.
pub fn list() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(patch_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "json").then(|| p.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn save(name: &str, graph: &NodeGraph) -> Result<PathBuf> {
    let path = path_for(name);
    let dir = path.parent().context("patch path has no parent")?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    // Write to a temporary file and rename over the target: a crash or a
    // full disk mid-write must not destroy the patch that was already
    // there. Rename within a directory is atomic on every platform we
    // target.
    let tmp = tmp_path(&path);
    std::fs::write(&tmp, serde_json::to_vec_pretty(graph)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(path)
}

pub fn load(name: &str) -> Result<NodeGraph> {
    let path = path_for(name);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

pub fn delete(name: &str) -> Result<()> {
    let path = path_for(name);
    std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))
}

/// Whether a name would overwrite an existing patch, so the UI can warn
/// before clobbering rather than after.
pub fn exists(name: &str) -> bool {
    path_for(name).exists()
}

/// Used by tests and by anything that needs to know where a name lands.
pub fn resolved_path(name: &str) -> PathBuf {
    path_for(name)
}

/// Set an unreadable state file aside instead of leaving it in place.
///
/// Every loader here falls back to defaults when its file will not parse
/// — the right call at startup, an unreadable file must never stop the
/// show. But the file then still sat at its own path, and the *first
/// save* — an autosave, a knob turned — overwrote it. Corruption became
/// permanent loss precisely because the app kept running well: a truncated
/// modulation.json from a power cut held an evening of routing that a
/// text editor could have recovered, for exactly as long as nothing
/// saved.
///
/// Renamed to `<name>.broken`, replacing any previous quarantine — the
/// newest corpse is the one worth keeping.
/// Temp-file path for an atomic write, unique per process.
///
/// Two instances sharing one deterministic ".json.tmp" name could
/// truncate each other's half-written file, landing a torn rename —
/// defeating the very atomicity the tmp dance exists for. A double
/// launch, or one window per projector, is exactly when the state files
/// matter most.
pub fn tmp_path(path: &std::path::Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".tmp.{}", std::process::id()));
    PathBuf::from(name)
}

pub fn quarantine(path: &std::path::Path) {
    let mut broken = path.as_os_str().to_owned();
    broken.push(".broken");
    match std::fs::rename(path, &broken) {
        Ok(()) => log::warn!(
            "set the unreadable file aside as {} — it may be hand-recoverable",
            std::path::Path::new(&broken).display()
        ),
        Err(e) => log::warn!("could not set {} aside: {e}", path.display()),
    }
}

/// Where the working modulation state is kept between launches.
///
/// Not in the patch directory: this is not a patch someone named and chose
/// to keep, and putting it there would list it in the load menu as if it
/// were one.
fn session_path() -> PathBuf {
    crate::project::show_dir().join("modulation.json")
}

/// The same path, for the test that checks every show-shaped file is
/// named in [`crate::project::CONTENTS`]. Not public API: the working
/// modulation state is not something to open by name.
#[cfg(test)]
pub(crate) fn session_path_for_test() -> PathBuf {
    session_path()
}

/// Write the whole modulation state — clock, LFOs, routes and graph.
///
/// Every other piece of user state in the app comes back on the next
/// launch: the scene grids, the macro assignments, the slider ranges, the
/// MIDI map, the palettes, the point clouds. Modulation did not. A patch
/// could be saved *by name* from the canvas, which covers the graph and
/// only if you thought to do it — and it covers none of the routes, which
/// are made one click at a time from the parameter list and had no save of
/// any kind. Quitting threw them away silently.
///
/// Temp file and rename, like every other persisted artefact here.
pub fn save_session(engine: &crate::ModEngine) -> Result<()> {
    let path = session_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = tmp_path(&path);
    std::fs::write(&tmp, serde_json::to_vec_pretty(engine)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
}

/// Restore it. `None` on a fresh install, or if the file cannot be read —
/// a corrupt session must start the app with defaults rather than refuse
/// to start.
pub fn load_session() -> Option<crate::ModEngine> {
    let path = session_path();
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(engine) => Some(engine),
        Err(e) => {
            log::warn!(
                "could not read {}: {e:#} — starting with default modulation",
                path.display()
            );
            quarantine(&path);
            None
        }
    }
}

/// The bytes `save_session` would write, for deciding whether it needs to.
///
/// The graph has a `dirty` flag but it belongs to the topological sort and
/// is cleared by it, and neither it nor anything else is set when a node's
/// parameters are edited or a node is dragged. Comparing what would be
/// written catches every edit wherever it happens, which a flag threaded
/// through the mutation sites would not — and a patch is small enough that
/// serialising it a couple of times a minute costs nothing.
pub fn session_bytes(engine: &crate::ModEngine) -> Vec<u8> {
    serde_json::to_vec(engine).unwrap_or_default()
}

/// A modulation patch shipped with the app, mirroring
/// [`crate::preset::BUILTINS`]: always present, read-only, and safe from
/// a cleared config directory. The graph is built in code rather than
/// parsed from bundled JSON, so a patch that names a node variant which
/// no longer exists fails to compile instead of failing to load.
pub struct BuiltinPatch {
    pub name: &'static str,
    pub about: &'static str,
    pub build: fn() -> crate::graph::NodeGraph,
}

/// The shipped patches. One for now: the patch that makes the vector
/// layers move with music the moment it is loaded, because a modulation
/// system whose first patch you must wire yourself is a system most
/// people never hear.
pub const BUILTIN_PATCHES: &[BuiltinPatch] = &[BuiltinPatch {
    name: "Pulse",
    about: "Kick gates a snap envelope into layer 2's opacity; a four-beat \
            phasor drifts layer 1's phase. The vector layers, on the beat.",
    build: pulse,
}];

/// Kick band -> gate -> envelope -> /l2/opacity, and a slow phasor into
/// /l1/phase. Laid out left-to-right on the canvas the way a hand-built
/// patch would be, because a shipped patch is also a worked example.
fn pulse() -> crate::graph::NodeGraph {
    use crate::graph::{NodeGraph, NodeKind};
    let mut g = NodeGraph::default();
    let band = g.add(NodeKind::Band(0), [40.0, 60.0]);
    let gate = g.add(NodeKind::Gate { threshold: 0.5 }, [220.0, 60.0]);
    let env = g.add(
        NodeKind::Envelope { attack: 0.005, decay: 0.18 },
        [400.0, 60.0],
    );
    let opacity = g.add(
        NodeKind::Param { addr: "/l2/opacity".into(), depth: -0.9 },
        [580.0, 60.0],
    );
    g.connect(band, gate, 0);
    g.connect(gate, env, 0);
    g.connect(env, opacity, 0);

    let phasor = g.add(NodeKind::Phasor { beats: 4.0 }, [40.0, 220.0]);
    let phase = g.add(
        NodeKind::Param { addr: "/l1/phase".into(), depth: 1.0 },
        [220.0, 220.0],
    );
    g.connect(phasor, phase, 0);
    g
}

/// Every patch name the load menu should offer: shipped first, then the
/// user's files. Mirrors `preset::all_names`.
pub fn all_names() -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_PATCHES.iter().map(|b| b.name.to_string()).collect();
    names.extend(list());
    names
}

/// Load by name, preferring the shipped patches — a user file cannot
/// shadow one, for the same reason a preset cannot: "put it back how it
/// shipped" has to stay available.
pub fn by_name(name: &str) -> Option<crate::graph::NodeGraph> {
    if let Some(b) = BUILTIN_PATCHES.iter().find(|b| b.name == name) {
        return Some((b.build)());
    }
    load(name).ok()
}

/// A save name that cannot collide with a shipped patch, mirroring
/// `preset::capture_name`.
pub fn patch_save_name(wanted: &str) -> String {
    let shadowed = |n: &str| BUILTIN_PATCHES.iter().any(|b| b.name == n);
    if !shadowed(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|i| format!("{wanted} {i}"))
        .find(|n| !shadowed(n))
        .expect("the integers do not run out")
}

#[cfg(test)]
mod tests {

    use super::*;

    /// The routes are the part with no other way back.
    ///
    /// A patch could always be saved by name from the canvas, and covers
    /// the graph if you remembered to. Routes are made one click at a time
    /// from the parameter list and had no save of any kind — so an evening
    /// of assigning LFOs to parameters was thrown away by quitting, in
    /// silence.
    #[test]
    fn atomic_write_tmp_names_do_not_collide_across_processes() {
        let path = std::path::Path::new("/state/vizz/modulation.json");
        let tmp = tmp_path(path);
        // Same directory (rename must stay atomic), same visible stem,
        // and carrying this process's id so another instance writes
        // elsewhere.
        assert_eq!(tmp.parent(), path.parent());
        let name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("modulation.json.tmp."), "{name}");
        assert!(name.ends_with(&std::process::id().to_string()), "{name}");
    }

    #[test]
    fn the_modulation_session_comes_back_with_its_routes() {
        let (_guard, _tmp) = crate::test_env::scoped("session-routes");
        assert!(load_session().is_none(), "a fresh install has no session");

        let mut engine = crate::ModEngine::with_defaults();
        engine.routes.push(crate::Route {
            source: crate::Source::Lfo(0),
            param: "/particles/hue".into(),
            depth: 0.3,
            enabled: true,
        });
        engine.clock.bpm = 138.0;
        engine.lfos[0].rate = crate::Rate::Beats(8.0);
        save_session(&engine).unwrap();

        let back = load_session().expect("the session did not come back");
        assert_eq!(back.routes.len(), 1);
        assert_eq!(back.routes[0].param, "/particles/hue");
        assert!((back.routes[0].depth - 0.3).abs() < 1e-6);
        assert!((back.clock.bpm - 138.0).abs() < 1e-6);
        assert_eq!(back.lfos[0].rate, crate::Rate::Beats(8.0));
    }

    /// The node graph rides along with it, layout included.
    #[test]
    fn the_session_carries_the_node_graph_too() {
        let (_guard, _tmp) = crate::test_env::scoped("session-graph");
        let mut engine = crate::ModEngine::with_defaults();
        let a = engine.graph.add(crate::graph::NodeKind::Level, [40.0, 60.0]);
        let b = engine.graph.add(
            crate::graph::NodeKind::Param { addr: "/particles/hue".into(), depth: 0.5 },
            [220.0, 60.0],
        );
        engine.graph.connect(a, b, 0);
        save_session(&engine).unwrap();

        let back = load_session().unwrap();
        assert_eq!(back.graph.nodes.len(), 2);
        assert_eq!(back.graph.edges.len(), 1);
        // Layout survives: a patch that reloads rearranged has to be read
        // from scratch, which is most of the reason to save it.
        assert_eq!(back.graph.nodes[0].pos, [40.0, 60.0]);
    }

    /// The autosave is driven by comparing what would be written, because
    /// no flag in the graph covers editing a node or dragging one. If the
    /// bytes did not change on an edit, the autosave would never fire.
    #[test]
    fn an_edit_changes_the_bytes_the_autosave_compares() {
        let (_guard, _tmp) = crate::test_env::scoped("session-bytes");
        let mut engine = crate::ModEngine::with_defaults();
        let before = session_bytes(&engine);
        assert_eq!(before, session_bytes(&engine), "unchanged state must be stable");

        // Adding a node.
        let id = engine.graph.add(crate::graph::NodeKind::Level, [10.0, 10.0]);
        let added = session_bytes(&engine);
        assert_ne!(before, added, "adding a node went unnoticed");

        // Dragging one — no flag anywhere is set for this.
        engine.graph.nodes[id.0].pos = [80.0, 10.0];
        assert_ne!(added, session_bytes(&engine), "moving a node went unnoticed");
    }

    /// A corrupt session must not stop the app starting. It is written on
    /// a timer, so a power cut mid-write is exactly when it would be torn.
    #[test]
    fn a_corrupt_session_starts_with_defaults_instead_of_failing() {
        let (_guard, _tmp) = crate::test_env::scoped("session-corrupt");
        let path = session_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert!(load_session().is_none());
    }

    /// And the corrupt file must survive what comes next. Falling back to
    /// defaults is right; leaving the damaged file where the very next
    /// autosave overwrites it turned recoverable corruption into
    /// permanent loss — the evening of routing a text editor could have
    /// salvaged, gone five seconds after the app came up.
    #[test]
    fn a_corrupt_session_is_set_aside_before_the_next_save_can_destroy_it() {
        let (_guard, _tmp) = crate::test_env::scoped("session-quarantine");
        let path = session_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let damaged = b"{ \"clock\": { \"bpm\": 174.0, TRUNCATED".to_vec();
        std::fs::write(&path, &damaged).unwrap();

        assert!(load_session().is_none());
        save_session(&crate::ModEngine::with_defaults()).unwrap();

        let mut broken = path.as_os_str().to_owned();
        broken.push(".broken");
        let kept = std::fs::read(std::path::Path::new(&broken))
            .expect("the damaged file must be set aside, not clobbered");
        assert_eq!(kept, damaged, "the quarantined bytes must be the originals");
    }
    
    use crate::graph::NodeKind;

    /// The important property: a hostile name cannot escape the patch
    /// directory. Names come from a text field, and "save as" is exactly
    /// where someone pastes something odd.
    #[test]
    fn names_cannot_escape_the_patch_directory() {
        // Take the guard even though this test writes nothing.
        // `patch_dir()` reads the environment, and a *writing* test
        // running beside it swaps XDG_CONFIG_HOME under our feet — so the
        // resolved path and the directory it is compared against came
        // from two different config homes and the test reported an escape
        // that never happened. Flaky at about one run in eight.
        let (_guard, _tmp) = crate::test_env::scoped("patch-escape");
        // And read it once: comparing two separate calls is what let the
        // race in, so make it impossible rather than merely unlikely.
        let dir = patch_dir();
        for evil in [
            "../../../.ssh/authorized_keys",
            "..",
            "../evil",
            "/etc/passwd",
            "foo/bar",
            "foo\\bar",
            "with\0nul",
            ".",
            "....",
        ] {
            let path = resolved_path(evil);
            assert!(
                path.starts_with(&dir),
                "{evil:?} escaped to {}",
                path.display()
            );
            // And it must still be a single file, not a nested path.
            assert_eq!(
                path.parent().unwrap(),
                dir,
                "{evil:?} landed in a subdirectory: {}",
                path.display()
            );
        }
    }

    #[test]
    fn sanitize_keeps_ordinary_names_readable() {
        assert_eq!(sanitize("Warehouse set 2"), "Warehouse set 2");
        assert_eq!(sanitize("kick-driven (fast)"), "kick-driven (fast)");
        // Unicode and punctuation are replaced rather than dropped, so two
        // different names cannot silently collapse to the same file.
        assert_eq!(sanitize("café/bar"), "caf__bar");
        assert_eq!(sanitize(""), "untitled");
        assert_eq!(sanitize("   "), "untitled");
    }

    #[test]
    fn sanitize_bounds_length() {
        let long = "a".repeat(500);
        assert!(sanitize(&long).chars().count() <= 64);
    }

    /// A patch must survive the round trip with its wiring *and* its
    /// layout, and must still evaluate afterwards.
    #[test]
    fn patches_round_trip_through_disk() {
        let (_guard, dir) = crate::test_env::scoped("patch");

        let mut g = NodeGraph::default();
        let src = g.add(NodeKind::Band(2), [11.0, 22.0]);
        let curve = g.add(
            NodeKind::Curve { shape: crate::graph::CurveShape::Exp4, amount: 0.75 },
            [33.0, 44.0],
        );
        let sink = g.add(NodeKind::Param { addr: "/fx/glow".into(), depth: 0.6 }, [55.0, 66.0]);
        g.connect(src, curve, 0);
        g.connect(curve, sink, 0);

        assert!(list().is_empty(), "fresh directory should hold no patches");
        assert!(!exists("Warehouse"));
        save("Warehouse", &g).expect("save failed");
        assert!(exists("Warehouse"));
        assert_eq!(list(), vec!["Warehouse".to_string()]);

        let back = load("Warehouse").expect("load failed");
        assert_eq!(back.nodes, g.nodes, "nodes or layout changed");
        assert_eq!(back.edges, g.edges, "wiring changed");
        assert_eq!(back.nodes[curve.0].pos, [33.0, 44.0], "layout not preserved");

        // Saving over an existing patch must replace it, not append or fail.
        let mut g2 = NodeGraph::default();
        g2.add(NodeKind::Level, [1.0, 2.0]);
        save("Warehouse", &g2).expect("overwrite failed");
        assert_eq!(load("Warehouse").unwrap().nodes.len(), 1);
        assert_eq!(list().len(), 1, "overwrite created a second file");

        delete("Warehouse").expect("delete failed");
        assert!(list().is_empty());
        // No stray temporary file left behind by the atomic write.
        let leftovers: Vec<_> = std::fs::read_dir(patch_dir())
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "left files behind: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_a_missing_patch_is_an_error_not_a_panic() {
        let (_guard, _dir) = crate::test_env::scoped("patch-miss");
        assert!(load("nothing-here").is_err());
        assert!(!exists("nothing-here"));
    }
}
