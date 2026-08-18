# Shows — design note

Everything the app remembered used to sit in one flat directory, which
meant a machine held exactly one show. That is fine for an instrument you
set up once and wrong for one that goes to two gigs a week: the pages, the
pads, the looks, the faders and the patches are all *this band's*, and the
only way to keep last month's was to copy the folder by hand and remember
which copy was live.

## What shipped

- `crates/vizz-mod/src/project.rs`: the open show, the list, and
  `new / open / save as / rename / delete`, over
  `~/.config/vizz/projects/<name>/`.
- `crates/vizz-ui/src/project_bar.rs`: one chip and its menu, drawn at the
  start of the panel and at the start of the performance strip.
- `windowed.rs::reload_show`: reading a whole show back off disk in the
  order launch reads it.
- A migration that moves an existing flat layout into `projects/Show 1/`.

## Decisions, and the alternatives they beat

**One seam, not nine.** Every show-shaped path in the app already resolved
through one call — `library::patch_dir()`, with most callers taking
`.parent()` to get back up to the config root. That `.parent()` idiom was
a hack, and it was also the whole feature: replacing it with
`project::show_dir()` moved the grids, the deck book, both preset
directories, the patches, the slider ranges, the macro layout and the
modulation session at once, and there is no tenth place a file could be
hiding, because there was never a second way to build a path.

**The rig does not travel.** `settings.json` and `midi.json` stay at the
root. Output size, audio device and controller map describe the machine in
front of you; a show is what you are playing on it. Putting them in the
project was considered and is wrong in both directions: plugging into a
different rig would silently change your show's files, and opening last
month's show at a new venue would restore last month's output size in
front of an audience.

**A show carries its own preset library.** The alternative — one global
pool of looks, per-show pages — was rejected because a deck holds
*references*, by name, into that pool. A show whose pool lived elsewhere
would copy to another machine as a page of dead pads, which is exactly the
failure the reference model already surfaces for a deleted preset and
exactly the one a portable show must not have. The cost is real and worth
saying out loud: `new show…` starts with an empty library. `save as…` is
how you carry your looks forward, the built-ins are always compiled in,
and the built-in set is one right-click away on the deck row's `+`.

**There is no save button.** Every file here has always been written the
moment it changed, and the menu says so on the line under the name. A
dirty flag and an explicit save was the other reading of the request, and
it is the one a live tool cannot honour: a performer who loses an hour of
pad work for not having pressed a button at the end of it has been failed
by the program. So "save as" means *copy this show and carry on in the
copy*, which is the useful half of the idea. Saying it in the menu matters
as much as the behaviour — `save as…` sitting alone reads as "and if you
do not, you lose it", which is both frightening and untrue.

**Migration is gated on the pointer, not on `projects/` existing.** A
migration killed half way leaves the directory there with some of the show
still at the root. Gating on the directory would call that finished and
orphan the rest; gating on `open.json` runs it again, and every step is
idempotent because a file already moved is not there to move. The pointer
is written last for the same reason.

**Moved, not copied.** `fs::rename` within one tree is atomic and free,
and a half-copied preset library is worse than a moved one. A failure on
one entry is logged and skipped rather than aborting the lot: losing the
macros should not also cost you the pads.

**A new show writes an empty `decks.json` immediately.** Not tidiness. A
machine with no deck file is how `sets::is_fresh_install` recognises one
that has never had a show on it, and a deliberately empty project coming
back full of the built-in set on the next launch would be a horrible
surprise. Writing the file is the project saying "empty on purpose".

**Opening a show re-reads everything, in launch's order.** Anything left
in memory from the show before would be written into the new one at the
next autosave, which is how one show quietly eats another. The rack's
saved-bytes comparison is reseeded for exactly that reason — without it
the next autosave tick sees "changed" and writes the show just left over
the one just opened. The order is launch's order, deliberately: two ways
to open a show is two behaviours to keep in step, and the one that runs
once per launch is not the one that would get the fix.

**A rename does not reload.** The directory moves with everything in it,
so what is in memory is still what is on disk, and reloading would only
throw away which pad is lit.

## Sharp edges worth knowing

- **A name is a directory name.** It is sanitised the way patch and preset
  names are, and shown back sanitised, because silently renaming a show
  makes it unfindable later. A name already taken counts up —
  `Warehouse 2` — rather than overwriting, which is the one outcome this
  must never have.
- **Clouds and palettes are in `settings.json`, so they stay with the
  rig.** They are paths to files elsewhere on disk rather than content, so
  this is defensible — the material is yours, the arrangement is the
  show's — but it does mean a copied show arrives without them.
- The open name is cached against the root it was resolved under.
  `show_dir()` is on the path of every save in the app, and re-reading a
  pointer file each time is a syscall for an answer that changes when a
  human clicks a menu. Keyed by root because the test suite redirects
  `XDG_CONFIG_HOME`, and a name resolved under one root handed back under
  another is a stale-cache bug that would only ever show up in CI.
- `project::CONTENTS` is the list of what a show owns, and a test holds it
  against the paths the rest of the crate actually builds. A file added to
  the show and forgotten there would silently stop travelling when one is
  copied, and nothing else would notice.

## Deferred, deliberately

- **A launch screen.** "Which show?" before the visuals come up is the
  obvious shape and the wrong one for a program you want running the
  moment it opens. The chip is always there instead.
- **Importing and exporting a show as one file.** A directory copies with
  Finder, and a zip format is a second thing to version.
- **Per-show settings overrides.** A venue profile is a real want — a show
  that remembers it plays at 3840×1080 — but it is a settings feature,
  not a project one, and mixing them would make the rig travel by
  accident.
