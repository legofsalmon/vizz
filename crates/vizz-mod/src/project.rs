//! Projects: one show per directory, one open at a time.
//!
//! Everything the app remembers used to sit in one flat directory, which
//! meant a machine held exactly one show. That is fine for an instrument
//! you set up once and wrong for one that goes to two gigs a week: the
//! pages, the pads, the looks, the faders and the patches are all *this
//! band's*, and the only way to keep last month's was to copy the folder
//! by hand and remember which copy was live.
//!
//! So the show-shaped state moved one level down, into
//! `projects/<name>/`, and the root keeps only what belongs to the
//! machine — the outputs, the render size, the MIDI map. Plugging into a
//! different rig should not change your show, and opening a different
//! show should not change your rig.
//!
//! # What travels, and what does not
//!
//! In the project: [`CONTENTS`] — both grids, the deck book, the presets
//! for both layers, the patches, the slider ranges, the macro layout and
//! the working modulation state.
//!
//! At the root: `settings.json` and `midi.json`. Both describe the
//! hardware in front of you rather than the set you are playing.
//!
//! # Saving
//!
//! There is no save button, and the menu says so. Every one of the files
//! above is already written the moment it changes — that predates
//! projects by a long way and is not something to give up: a performer
//! who loses an hour of pad work because they did not press a button at
//! the end of it has been failed by the program. "Save as" therefore
//! means *copy this show and work on the copy*, which is the useful half
//! of the idea and the half a live tool can honour.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context as _, Result};

/// The name a machine's first show gets. Numbered because the next one
/// wants to be "Show 2" without anybody having to think of a word.
pub const FIRST: &str = "Show 1";

/// Everything a project owns, relative to its directory.
///
/// One list, used three ways: migration moves these out of the old flat
/// root, "save as" copies them into the new directory, and the doc test
/// checks it against the paths the rest of the crate actually builds. A
/// file added to the show and forgotten here would silently stop
/// travelling, and nothing else would notice.
pub const CONTENTS: [&str; 9] = [
    "decks.json",
    "grid.json",
    "gravity-grid.json",
    "ranges.json",
    "macros.json",
    "modulation.json",
    "presets",
    "gravity",
    "patches",
];

/// Config root: `$XDG_CONFIG_HOME/vizz`, or `~/.config/vizz`.
///
/// The machine's directory. Only settings, the MIDI map and the pointer
/// at the open show live here directly; everything else is under
/// [`projects_dir`].
pub fn root() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("vizz")
}

/// Where the shows live, one directory each.
pub fn projects_dir() -> PathBuf {
    root().join("projects")
}

/// Which show is open, as a file, so it survives a crash and a restart.
fn pointer() -> PathBuf {
    root().join("open.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Pointer {
    project: String,
}

/// The resolved open name, cached against the root it was resolved under.
///
/// Cached because [`show_dir`] is on the path of every save in the app and
/// re-reading a pointer file each time is a syscall for an answer that
/// changes when a human clicks a menu. Keyed by root because the test
/// suite redirects `XDG_CONFIG_HOME`: a name resolved under one root must
/// not be handed back under another, which is exactly the stale-cache bug
/// that would only ever show up in CI.
static OPEN: OnceLock<RwLock<Option<(PathBuf, String)>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<(PathBuf, String)>> {
    OPEN.get_or_init(|| RwLock::new(None))
}

/// The name of the open show, resolving it — and migrating the old flat
/// layout — the first time it is asked for.
pub fn open() -> String {
    let root = root();
    // The read guard is dropped by the closing brace, before the write
    // below is ever attempted. An explicit block rather than an `if let`
    // chain, because taking a std `RwLock` for writing on a thread that
    // still holds it for reading deadlocks — and the version that only
    // works because of when a temporary happens to drop is not a thing to
    // rest a render loop on.
    {
        let guard = cell().read();
        if let Ok(guard) = guard.as_ref()
            && let Some((cached, name)) = guard.as_ref()
            && cached == &root
        {
            return name.clone();
        }
    }
    let mut guard = match cell().write() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    // Another thread may have resolved it while this one waited.
    if let Some((cached, name)) = guard.as_ref()
        && cached == &root
    {
        return name.clone();
    }
    let name = resolve(&root);
    let dir = projects_dir().join(crate::library::sanitize(&name));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("could not create the show directory {}: {e}", dir.display());
    }
    *guard = Some((root, name.clone()));
    name
}

/// Read the pointer, or set the machine up for the first time.
///
/// Gated on the *pointer*, not on `projects/` existing. A migration that
/// dies half way leaves the directory there with some of the show still
/// at the root; gating on the directory would call that finished and
/// orphan the rest, where gating on the pointer runs it again — and every
/// step of it is idempotent, since a file already moved simply is not
/// there to move.
fn resolve(root: &Path) -> String {
    if let Ok(bytes) = std::fs::read(pointer())
        && let Ok(p) = serde_json::from_slice::<Pointer>(&bytes)
    {
        let name = crate::library::sanitize(&p.project);
        if !name.is_empty() {
            return name;
        }
    }
    // No pointer: either a fresh machine, or a show that predates
    // projects sitting in the flat layout. Both become `FIRST`.
    let name = crate::library::sanitize(FIRST);
    let dir = root.join("projects").join(&name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("could not create the first show directory: {e}");
        return name;
    }
    for entry in CONTENTS {
        let from = root.join(entry);
        if from.exists() {
            let to = dir.join(entry);
            // Rename, not copy: within one directory tree it is atomic and
            // free, and a half-copied preset library is worse than a moved
            // one. A failure is logged and skipped rather than aborting —
            // losing the macros should not also cost you the pads.
            if let Err(e) = std::fs::rename(&from, &to) {
                log::error!(
                    "could not move {} into the first show: {e}",
                    from.display()
                );
            }
        }
    }
    if let Err(e) = write_pointer(&name) {
        // Not fatal: the next launch runs the same migration, finds
        // nothing left at the root to move, and lands in the same place.
        log::error!("could not record which show is open: {e:#}");
    }
    name
}

fn write_pointer(name: &str) -> Result<()> {
    let path = pointer();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = serde_json::to_vec_pretty(&Pointer {
        project: name.to_string(),
    })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// The open show's directory. Every show-shaped path in the app hangs off
/// this one call.
pub fn show_dir() -> PathBuf {
    projects_dir().join(crate::library::sanitize(&open()))
}

/// The shows on this machine, alphabetical.
///
/// Directory names, which are also the display names: a project is named
/// by the folder it lives in, so there is no second copy of the name to
/// drift out of step with the first.
pub fn list() -> Vec<String> {
    // Resolve first, so a machine that has never been asked has its first
    // show — and its migration — before the directory is read.
    let current = open();
    let mut names: Vec<String> = std::fs::read_dir(projects_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    if !names.iter().any(|n| n == &current) {
        names.push(current);
    }
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Make `wanted` into a name no existing show already has, by counting.
///
/// The same shape as a deck's next name and a preset's: silently writing
/// over somebody's other show because they typed a name twice is not a
/// thing a program should do, and refusing with an error is a dead end
/// when the obvious answer is "call it 2".
pub fn free_name(wanted: &str) -> String {
    let base = crate::library::sanitize(wanted);
    let taken = list();
    if !taken.iter().any(|n| n.eq_ignore_ascii_case(&base)) {
        return base;
    }
    for n in 2..1000 {
        let candidate = format!("{base} {n}");
        if !taken.iter().any(|c| c.eq_ignore_ascii_case(&candidate)) {
            return candidate;
        }
    }
    base
}

/// The name the "new show" field offers: `Show 1`, `Show 2`, and so on.
///
/// Counted rather than run through [`free_name`], which would answer
/// "Show 1 2" — the right shape for a name a person typed twice and the
/// wrong one for a name the program is inventing. The same counting the
/// deck book does for its pages.
pub fn next_name() -> String {
    let taken = list();
    (1..=taken.len() + 1)
        .map(|n| format!("Show {n}"))
        .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
        .unwrap_or_else(|| FIRST.to_string())
}

/// Create an empty show and open it. Returns the name it actually got.
///
/// The empty deck book is written on the spot rather than left to the
/// first save, and that is load-bearing: a machine with no `decks.json`
/// is how [`crate::sets::is_fresh_install`] recognises one that has never
/// had a show on it, and a deliberately empty project coming back full of
/// somebody else's set list on the next launch would be a horrible
/// surprise. Writing the file says "this one is empty on purpose".
pub fn create(name: &str) -> Result<String> {
    let name = free_name(name);
    let dir = projects_dir().join(&name);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    set_open(&name)?;
    crate::deck::save(&crate::deck::Book::default())?;
    Ok(name)
}

/// Copy the open show under a new name and work on the copy — "save as".
pub fn save_as(name: &str) -> Result<String> {
    let from = show_dir();
    let name = free_name(name);
    let to = projects_dir().join(&name);
    std::fs::create_dir_all(&to).with_context(|| format!("creating {}", to.display()))?;
    for entry in CONTENTS {
        let src = from.join(entry);
        if !src.exists() {
            continue;
        }
        copy_into(&src, &to.join(entry))
            .with_context(|| format!("copying {} into {name}", entry))?;
    }
    set_open(&name)?;
    Ok(name)
}

/// Copy a file, or a directory and everything under it.
fn copy_into(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
        for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?
        {
            let entry = entry?;
            copy_into(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(from, to).with_context(|| format!("writing {}", to.display()))?;
    }
    Ok(())
}

/// Switch to another show. The caller is responsible for reloading what
/// is now on disk — this only moves the pointer.
pub fn set_open(name: &str) -> Result<()> {
    let name = crate::library::sanitize(name);
    let dir = projects_dir().join(&name);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    write_pointer(&name)?;
    let mut guard = match cell().write() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    *guard = Some((root(), name));
    Ok(())
}

/// Rename a show. Renaming the open one keeps it open.
pub fn rename(from: &str, to: &str) -> Result<String> {
    let from = crate::library::sanitize(from);
    let to = free_name(to);
    if from == to {
        return Ok(from);
    }
    std::fs::rename(projects_dir().join(&from), projects_dir().join(&to))
        .with_context(|| format!("renaming {from} to {to}"))?;
    if open() == from {
        set_open(&to)?;
    }
    Ok(to)
}

/// Throw a show away. The last one cannot go — there would be nowhere for
/// the next pad to live and no chip left to click to make one.
///
/// Deleting the open show opens another, so the app is never pointing at
/// a directory that is not there.
pub fn remove(name: &str) -> Result<String> {
    let name = crate::library::sanitize(name);
    let others: Vec<String> = list().into_iter().filter(|n| n != &name).collect();
    let Some(next) = others.first().cloned() else {
        anyhow::bail!("this is the only show — make another one first");
    };
    let was_open = open() == name;
    if was_open {
        set_open(&next)?;
    }
    std::fs::remove_dir_all(projects_dir().join(&name))
        .with_context(|| format!("deleting {name}"))?;
    Ok(if was_open { next } else { open() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forget the cached open name, so the next call resolves under
    /// whatever root the test just pointed the environment at.
    fn forget() {
        if let Ok(mut g) = cell().write() {
            *g = None;
        }
    }

    #[test]
    fn a_fresh_machine_gets_one_show() {
        let (_guard, dir) = crate::test_env::scoped("project-fresh");
        forget();
        assert_eq!(open(), FIRST);
        assert_eq!(list(), vec![FIRST.to_string()]);
        assert_eq!(show_dir(), dir.join("vizz/projects/Show 1"));
        forget();
    }

    /// The whole point of the migration: a show sitting in the old flat
    /// layout has to arrive inside a project, not be left behind.
    #[test]
    fn the_old_flat_layout_moves_into_the_first_show() {
        let (_guard, dir) = crate::test_env::scoped("project-migrate");
        forget();
        let root = dir.join("vizz");
        std::fs::create_dir_all(root.join("presets")).unwrap();
        std::fs::write(root.join("grid.json"), b"{}").unwrap();
        std::fs::write(root.join("presets/warehouse.json"), b"{}").unwrap();
        std::fs::write(root.join("settings.json"), b"{}").unwrap();

        let show = show_dir();
        assert!(show.join("grid.json").exists(), "the grid did not travel");
        assert!(
            show.join("presets/warehouse.json").exists(),
            "the preset library did not travel"
        );
        assert!(
            !root.join("grid.json").exists(),
            "the grid was copied rather than moved, so two of them now drift apart"
        );
        assert!(
            root.join("settings.json").exists(),
            "settings describe the machine and must stay at the root"
        );
        forget();
    }

    /// A migration killed half way has to finish on the next launch
    /// rather than declare itself done because `projects/` is there.
    #[test]
    fn a_half_finished_migration_runs_again() {
        let (_guard, dir) = crate::test_env::scoped("project-halfway");
        forget();
        let root = dir.join("vizz");
        std::fs::create_dir_all(root.join("projects/Show 1")).unwrap();
        std::fs::write(root.join("projects/Show 1/grid.json"), b"{}").unwrap();
        std::fs::write(root.join("macros.json"), b"{}").unwrap();

        let show = show_dir();
        assert!(
            show.join("macros.json").exists(),
            "the leftover was orphaned at the root"
        );
        forget();
    }

    #[test]
    fn save_as_copies_the_show_and_switches_to_the_copy() {
        let (_guard, _dir) = crate::test_env::scoped("project-saveas");
        forget();
        std::fs::create_dir_all(show_dir().join("presets")).unwrap();
        std::fs::write(show_dir().join("presets/one.json"), b"{}").unwrap();

        let made = save_as("Tour").unwrap();
        assert_eq!(made, "Tour");
        assert_eq!(open(), "Tour");
        assert!(show_dir().join("presets/one.json").exists());
        // And the original is untouched — a copy, not a move.
        assert!(
            projects_dir().join("Show 1/presets/one.json").exists(),
            "save as took the original's presets with it"
        );
        forget();
    }

    /// Writing over another show because two of them were typed the same
    /// name is the one outcome this must never have.
    #[test]
    fn a_name_already_taken_counts_up_instead_of_overwriting() {
        let (_guard, _dir) = crate::test_env::scoped("project-clash");
        forget();
        std::fs::write(show_dir().join("grid.json"), b"first").unwrap();
        create("Show 1").unwrap();
        assert_eq!(open(), "Show 1 2");
        assert_eq!(
            std::fs::read(projects_dir().join("Show 1/grid.json")).unwrap(),
            b"first"
        );
        forget();
    }

    /// A project made on purpose is not a machine that has never had a
    /// show on it, and must not be filled with the built-in set when it
    /// is next opened.
    #[test]
    fn a_new_project_is_not_mistaken_for_a_fresh_install() {
        let (_guard, _dir) = crate::test_env::scoped("project-notfresh");
        forget();
        create("Empty").unwrap();
        assert!(
            crate::deck::exists(),
            "a new project must carry a deck file, or the set installs itself into it"
        );
        forget();
    }

    /// The offered name counts up: `Show 2`, not `Show 1 2`.
    #[test]
    fn the_offered_name_counts_shows_rather_than_copies() {
        let (_guard, _dir) = crate::test_env::scoped("project-nextname");
        forget();
        assert_eq!(next_name(), "Show 2");
        create("Show 2").unwrap();
        assert_eq!(next_name(), "Show 3");
        // A name a person typed twice still steps aside by copy-counting,
        // which is the right answer for that case and only that case.
        assert_eq!(free_name("Show 2"), "Show 2 2");
        forget();
    }

    #[test]
    fn deleting_the_open_show_opens_another() {
        let (_guard, _dir) = crate::test_env::scoped("project-delete");
        forget();
        create("Second").unwrap();
        assert_eq!(open(), "Second");
        let now = remove("Second").unwrap();
        assert_eq!(now, "Show 1");
        assert_eq!(open(), "Show 1");
        assert!(!projects_dir().join("Second").exists());
        forget();
    }

    /// The point of the whole thing, at the storage layer: two shows,
    /// two grids, and neither one able to see the other's.
    #[test]
    fn two_shows_keep_their_own_pads_and_looks() {
        let (_guard, _dir) = crate::test_env::scoped("project-isolation");
        forget();
        let mut first = crate::scene::Grid::default();
        first.duration = 7.0;
        crate::scene::save_kind(crate::preset::Kind::Look, &first).unwrap();
        std::fs::create_dir_all(crate::preset::Kind::Look.dir()).unwrap();
        std::fs::write(crate::preset::Kind::Look.dir().join("mine.json"), b"{}").unwrap();

        create("Other").unwrap();
        assert!(
            crate::scene::read_kind(crate::preset::Kind::Look).is_none(),
            "the new show can see the old one's grid"
        );
        assert!(
            !crate::preset::Kind::Look.dir().join("mine.json").exists(),
            "the new show can see the old one's looks"
        );

        set_open(FIRST).unwrap();
        assert_eq!(
            crate::scene::read_kind(crate::preset::Kind::Look)
                .expect("the first show lost its grid")
                .duration,
            7.0
        );
        assert!(crate::preset::Kind::Look.dir().join("mine.json").exists());
        forget();
    }

    #[test]
    fn the_last_show_cannot_be_deleted() {
        let (_guard, _dir) = crate::test_env::scoped("project-lastone");
        forget();
        assert_eq!(list().len(), 1);
        assert!(remove(FIRST).is_err());
        assert!(show_dir().exists());
        forget();
    }

    #[test]
    fn renaming_the_open_show_keeps_it_open() {
        let (_guard, _dir) = crate::test_env::scoped("project-rename");
        forget();
        std::fs::write(show_dir().join("grid.json"), b"x").unwrap();
        let now = rename(FIRST, "Warehouse").unwrap();
        assert_eq!(now, "Warehouse");
        assert_eq!(open(), "Warehouse");
        assert_eq!(std::fs::read(show_dir().join("grid.json")).unwrap(), b"x");
        forget();
    }

    /// A name that would escape the projects directory has to land inside
    /// it as a mangled folder, the same way a patch name does.
    #[test]
    fn a_name_cannot_escape_the_projects_directory() {
        let (_guard, dir) = crate::test_env::scoped("project-escape");
        forget();
        create("../../../evil").unwrap();
        assert!(show_dir().starts_with(dir.join("vizz/projects")));
        forget();
    }

    /// Every path the rest of the crate builds inside a show has to be
    /// named in `CONTENTS`, or it silently stops travelling with the
    /// project when one is copied.
    #[test]
    fn contents_names_every_file_the_show_actually_writes() {
        let (_guard, _dir) = crate::test_env::scoped("project-contents");
        forget();
        let show = show_dir();
        let mut built = vec![
            crate::deck::path(),
            crate::scene::path_for(crate::preset::Kind::Look),
            crate::scene::path_for(crate::preset::Kind::Gravity),
            crate::ranges::Ranges::path(),
            crate::perform::Macros::path(),
            crate::library::patch_dir(),
            crate::preset::Kind::Look.dir(),
            crate::preset::Kind::Gravity.dir(),
        ];
        built.push(crate::library::session_path_for_test());
        for path in built {
            let rel = path
                .strip_prefix(&show)
                .unwrap_or_else(|_| panic!("{} is not inside the show directory", path.display()));
            let head = rel
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .unwrap_or_default();
            assert!(
                CONTENTS.contains(&head.as_str()),
                "{head} is written inside a show but is not in CONTENTS, so it \
                 would not travel with a copied project"
            );
        }
        forget();
    }
}
