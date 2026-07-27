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

/// Where patches live. Alongside the MIDI map, so all user state is in one
/// place someone can back up or copy between machines.
pub fn patch_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("vizz").join("patches")
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
    let tmp = path.with_extension("json.tmp");
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

#[cfg(test)]
mod tests {
    use super::*;
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
