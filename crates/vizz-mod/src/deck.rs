//! Decks: the pads as pages, one per song.
//!
//! Sixteen scenes and sixteen gravity slots is a generous evening if the
//! set is one continuous thing, and nowhere near enough if it is twelve
//! songs that each want their own looks. The choice a fixed grid forces is
//! between preparing four songs properly and preparing twelve badly, and
//! it is not a choice anyone should be making at soundcheck.
//!
//! A deck is a page of both grids together. Switching decks turns the page
//! on the whole desk at once — scenes and gravity — because they are
//! played as one thing and a song whose looks arrived but whose wells
//! stayed behind is a song with the wrong picture. This is Resolume's
//! model and a Resolume user should find nothing surprising here.
//!
//! # What a deck is not
//!
//! It is not a copy of the looks. Presets live in one pool and a deck
//! holds *references*, exactly as a single grid always has — so the same
//! look can sit on a pad in every song, refining it once improves all
//! twelve, and a deck costs a few hundred bytes rather than a megabyte of
//! duplicated parameters.
//!
//! It is not the picture either. Switching pages does not fire anything
//! and does not touch a parameter: whatever is on screen stays on screen
//! until you press a pad. A page turn that changed the output would be
//! unusable, because every page turn happens in front of an audience.
//!
//! # Where it lives
//!
//! `decks.json`, beside `grid.json` and `gravity-grid.json`. The active
//! deck's cells stay mirrored into those two files, which is the reason
//! this file can be lost without costing you the set you are playing
//! tonight — and the reason a build that predates decks still opens the
//! show, on the deck that was live when it was saved.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::scene::{Cell, Grid, SLOTS};

/// Pages available.
///
/// Twenty-four. It was sixteen, to match the pad row — until the first
/// real set arrived at twenty songs and the cap was the thing standing in
/// front of it. A ceiling that a show is expected to hit is not a
/// ceiling, it is a bug with a constant's name.
///
/// Twenty-four rather than twenty: a set arriving exactly at the limit
/// leaves nowhere to put an encore, and the row wraps to a second line
/// long before this many chips anyway.
///
/// It is fixed rather than derived from the deck list because the
/// parameter registry's topology is built once at startup, and
/// `/deck/select` needs a range on the frame the app opens. Adding a deck
/// mid-show must not reshape the registry under a running show.
pub const MAX_DECKS: usize = 24;

/// Where a deck's column 1 sits in Resolume's composition, when the
/// columns are following. See [`Deck::origin`].
const DEFAULT_ORIGIN: u32 = 1;

/// One page: both grids' pads, and what the song is called.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deck {
    /// What the chip says. A song title, in practice.
    pub name: String,
    /// Scene pads, [`SLOTS`] long.
    #[serde(default)]
    pub scenes: Vec<Option<Cell>>,
    /// Gravity pads, [`SLOTS`] long.
    #[serde(default)]
    pub gravity: Vec<Option<Cell>>,
    /// Which Resolume column this deck's column 1 follows.
    ///
    /// One-based, to match the numbers Resolume itself puts on screen —
    /// asking a performer to subtract one from what Arena is showing them
    /// is asking for the wrong column at the wrong moment.
    ///
    /// A composition with one long grid and a song every sixteen columns
    /// sets this to 1, 17, 33 and so on, and each deck then follows its
    /// own stretch. Left at 1 — which is what every deck starts at — every
    /// deck follows the same first sixteen columns, which is the right
    /// answer when the Resolume side is one page too.
    #[serde(default = "default_origin")]
    pub origin: u32,
}

fn default_origin() -> u32 {
    DEFAULT_ORIGIN
}

impl Deck {
    /// An empty page under a name.
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scenes: vec![None; SLOTS],
            gravity: vec![None; SLOTS],
            origin: DEFAULT_ORIGIN,
        }
    }

    /// Whether any pad on either grid holds anything.
    pub fn is_empty(&self) -> bool {
        self.scenes.iter().chain(&self.gravity).all(|c| c.is_none())
    }

    /// Both cell rows the right length, whatever the file said.
    fn fit(&mut self) {
        for row in [&mut self.scenes, &mut self.gravity] {
            row.resize(SLOTS, None);
            row.truncate(SLOTS);
        }
        self.origin = self.origin.max(1);
    }
}

/// The pages, and which one is live.
///
/// A `Book` never holds fewer than one deck. An empty deck row would be a
/// row of pads belonging to nothing, and every operation here would need a
/// branch for a state the user cannot see or fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    decks: Vec<Deck>,
    active: usize,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            decks: vec![Deck::empty("deck 1")],
            active: 0,
        }
    }
}

impl Book {
    /// A book of one deck holding the grids as they stand.
    ///
    /// The migration path: a show prepared before decks existed becomes
    /// deck 1, with everything exactly where it was. Nothing else would
    /// be acceptable — the grid file is an evening's preparation.
    pub fn from_grids(scenes: &Grid, gravity: &Grid) -> Self {
        let mut book = Self::default();
        book.store(scenes, gravity);
        book
    }

    pub fn decks(&self) -> &[Deck] {
        &self.decks
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn len(&self) -> usize {
        self.decks.len()
    }

    pub fn is_empty(&self) -> bool {
        // Never true — `Book` keeps at least one deck. Present because
        // clippy asks for it beside `len`, and answering honestly is
        // cheaper than an allow.
        self.decks.is_empty()
    }

    /// The deck now playing. Always present.
    pub fn current(&self) -> &Deck {
        &self.decks[self.active.min(self.decks.len() - 1)]
    }

    /// Where the active deck's column 1 sits in Resolume's composition.
    pub fn origin(&self) -> u32 {
        self.current().origin
    }

    /// Take the live grids into the active page, where there are any.
    ///
    /// A grid whose file was missing or unreadable arrives as `None`
    /// rather than as sixteen empty pads, and is skipped. Adopting the
    /// empty one would let a lost `grid.json` erase the page this file is
    /// now the only copy of — the two mirrors of the live page are meant
    /// to protect each other, and that is the direction in which it
    /// matters.
    fn adopt_live(&mut self, scenes: Option<&Grid>, gravity: Option<&Grid>) {
        let active = self.active.min(self.decks.len() - 1);
        let deck = &mut self.decks[active];
        if let Some(scenes) = scenes {
            deck.scenes = scenes.cells().to_vec();
        }
        if let Some(gravity) = gravity {
            deck.gravity = gravity.cells().to_vec();
        }
    }

    /// Replace every page, landing on the first.
    ///
    /// For installing a set. Deliberately not a merge: a set is a show,
    /// and half of one interleaved with half of somebody else's is
    /// neither. The caller decides whether replacing is allowed — see
    /// [`crate::sets::is_fresh_install`].
    pub fn replace(&mut self, decks: Vec<Deck>) {
        if decks.is_empty() {
            return;
        }
        self.decks = decks;
        self.active = 0;
        self.fit();
    }

    /// Copy the live pads into the active deck.
    ///
    /// Called before every switch and before every save. A deck is not a
    /// snapshot taken when you made it: filling a pad during a song has to
    /// still be there next time that song comes round, and the live grid
    /// is the only place that edit exists until this runs.
    pub fn store(&mut self, scenes: &Grid, gravity: &Grid) {
        let active = self.active.min(self.decks.len() - 1);
        let deck = &mut self.decks[active];
        deck.scenes = scenes.cells().to_vec();
        deck.gravity = gravity.cells().to_vec();
    }

    /// Turn to a page, storing the one being left.
    ///
    /// Returns whether anything moved: switching to the deck already
    /// showing, or to an index that does not exist, is not a failure worth
    /// reporting to the performer — a controller sweeping across a bank of
    /// buttons will do both — but it must not count as a change either,
    /// or the app saves a file every frame.
    pub fn switch(&mut self, index: usize, scenes: &mut Grid, gravity: &mut Grid) -> bool {
        if index >= self.decks.len() || index == self.active {
            return false;
        }
        self.store(scenes, gravity);
        self.active = index;
        let deck = self.decks[index].clone();
        scenes.adopt_cells(deck.scenes);
        gravity.adopt_cells(deck.gravity);
        true
    }

    /// Load the active deck's pads into the grids without storing first.
    ///
    /// For startup, where the grids are the thing being replaced and there
    /// is nothing worth keeping in them.
    pub fn restore(&self, scenes: &mut Grid, gravity: &mut Grid) {
        let deck = self.current().clone();
        scenes.adopt_cells(deck.scenes);
        gravity.adopt_cells(deck.gravity);
    }

    /// Add an empty page and make it live. `None` when the book is full.
    pub fn add(&mut self, scenes: &mut Grid, gravity: &mut Grid) -> Option<usize> {
        self.insert(Deck::empty(self.next_name()), scenes, gravity)
    }

    /// Copy a page and make the copy live. The usual way to start the next
    /// song: most of a set is a variation on the song before it, and
    /// twelve songs built from scratch is a night nobody has.
    pub fn duplicate(&mut self, index: usize, scenes: &mut Grid, gravity: &mut Grid) -> Option<usize> {
        let source = self.decks.get(index)?;
        // Taken from the live grids when duplicating what is showing, so
        // an edit made this song is in the copy. Anything else would
        // silently duplicate the version last stored.
        let mut copy = source.clone();
        if index == self.active {
            copy.scenes = scenes.cells().to_vec();
            copy.gravity = gravity.cells().to_vec();
        }
        copy.name = self.next_name();
        self.insert(copy, scenes, gravity)
    }

    fn insert(&mut self, deck: Deck, scenes: &mut Grid, gravity: &mut Grid) -> Option<usize> {
        if self.decks.len() >= MAX_DECKS {
            return None;
        }
        self.store(scenes, gravity);
        self.decks.push(deck);
        self.active = self.decks.len() - 1;
        let fresh = self.decks[self.active].clone();
        scenes.adopt_cells(fresh.scenes);
        gravity.adopt_cells(fresh.gravity);
        Some(self.active)
    }

    /// Rename a page. An empty name is refused rather than stored: a chip
    /// with nothing on it cannot be told apart from the one beside it, and
    /// there would be no way to click it and put the name back.
    pub fn rename(&mut self, index: usize, name: impl Into<String>) -> bool {
        let name = name.into();
        let name = name.trim();
        let Some(deck) = self.decks.get_mut(index) else {
            return false;
        };
        if name.is_empty() || deck.name == name {
            return false;
        }
        deck.name = name.to_string();
        true
    }

    /// Point a page at a stretch of Resolume's columns. See
    /// [`Deck::origin`].
    pub fn set_origin(&mut self, index: usize, origin: u32) -> bool {
        let origin = origin.max(1);
        let Some(deck) = self.decks.get_mut(index) else {
            return false;
        };
        if deck.origin == origin {
            return false;
        }
        deck.origin = origin;
        true
    }

    /// Remove a page. The last one cannot go — a show with no decks has
    /// nowhere to put a pad — and removing the live one moves to its
    /// neighbour and loads it.
    pub fn remove(&mut self, index: usize, scenes: &mut Grid, gravity: &mut Grid) -> bool {
        if self.decks.len() <= 1 || index >= self.decks.len() {
            return false;
        }
        self.decks.remove(index);
        // Removing a page before the live one shifts it down; removing the
        // live one itself lands on whatever moved into its place, or on
        // the new last page when it was the end of the book.
        let landed = if index < self.active {
            self.active - 1
        } else {
            self.active.min(self.decks.len() - 1)
        };
        let reload = index == self.active;
        self.active = landed;
        if reload {
            self.restore(scenes, gravity);
        }
        true
    }

    /// "deck 3" where 3 is the first number not already taken, so
    /// deleting the middle of a set does not produce two decks called the
    /// same thing.
    fn next_name(&self) -> String {
        (1..=MAX_DECKS + 1)
            .map(|n| format!("deck {n}"))
            .find(|name| !self.decks.iter().any(|d| &d.name == name))
            .unwrap_or_else(|| "deck".into())
    }

    /// Every deck the right length and the active index in range,
    /// whatever the file said.
    fn fit(&mut self) {
        if self.decks.is_empty() {
            self.decks.push(Deck::empty("deck 1"));
        }
        self.decks.truncate(MAX_DECKS);
        for deck in &mut self.decks {
            deck.fit();
        }
        self.active = self.active.min(self.decks.len() - 1);
    }
}

/// Where the deck book lives, beside the two grid files it pages through.
pub fn path() -> PathBuf {
    crate::project::show_dir().join("decks.json")
}

/// Whether a set list has ever been written here.
///
/// The one thing `load` cannot say: it returns a book either way, and a
/// default book and a book that was saved as a single empty page are the
/// same value. Only the file's existence tells you whether anybody has
/// ever set this machine up — which is what a first run has to know
/// before it installs a show over the top.
pub fn exists() -> bool {
    path().exists()
}

/// Written and renamed, like the grids: a crash part-way through must not
/// destroy the set list that was already there.
pub fn save(book: &Book) -> Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = crate::library::tmp_path(&path);
    std::fs::write(&tmp, serde_json::to_vec_pretty(book)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// The deck book, or one built from the grids already loaded.
///
/// A missing file is the normal case for every show prepared before decks
/// existed, and for every first run: the grids become deck 1 and nothing
/// is lost. A corrupt one is quarantined and treated the same way, which
/// is the point of keeping the active deck mirrored in the grid files —
/// the worst a broken `decks.json` can cost you is the songs you are not
/// currently playing.
/// `scenes` and `gravity` are what [`crate::scene::read_kind`] actually
/// read, so `None` means the grid file was missing or unreadable rather
/// than empty — the difference between a page the user cleared and a page
/// whose only surviving copy is this file.
pub fn load(scenes: Option<&Grid>, gravity: Option<&Grid>) -> Book {
    let path = path();
    let fallback = || {
        let mut book = Book::default();
        book.adopt_live(scenes, gravity);
        book
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return fallback();
    };
    match serde_json::from_slice::<Book>(&bytes) {
        Ok(mut book) => {
            book.fit();
            // The grid files are the truth about the deck that was live:
            // they are written on every pad edit, and this file only on a
            // deck gesture. Adopting them here means a pad filled and then
            // never followed by a page turn is still there next launch.
            book.adopt_live(scenes, gravity);
            book
        }
        Err(e) => {
            log::error!(
                "could not read {}: {e:#} — starting from the grids as they stand",
                path.display()
            );
            crate::library::quarantine(&path);
            fallback()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::Kind;

    fn grids() -> (Grid, Grid) {
        (Grid::new(), Grid::for_kind(Kind::Gravity))
    }

    fn names(grid: &Grid) -> Vec<Option<String>> {
        grid.cells()
            .iter()
            .map(|c| c.as_ref().map(|c| c.preset.clone()))
            .collect()
    }

    /// The gesture the whole feature exists for: fill a page, turn to a
    /// fresh one, fill that, and find the first one intact on the way
    /// back.
    #[test]
    fn a_deck_keeps_its_pads_while_another_deck_is_played() {
        let (mut scenes, mut gravity) = grids();
        scenes.assign(0, "intro");
        gravity.assign(2, "wide well");
        let mut book = Book::from_grids(&scenes, &gravity);

        let second = book.add(&mut scenes, &mut gravity).expect("a second deck");
        assert_eq!(names(&scenes)[0], None, "the new deck arrived with the old deck's pads");
        assert_eq!(names(&gravity)[2], None);
        scenes.assign(0, "chorus");

        assert!(book.switch(0, &mut scenes, &mut gravity));
        assert_eq!(names(&scenes)[0].as_deref(), Some("intro"));
        assert_eq!(names(&gravity)[2].as_deref(), Some("wide well"));

        assert!(book.switch(second, &mut scenes, &mut gravity));
        assert_eq!(
            names(&scenes)[0].as_deref(),
            Some("chorus"),
            "the pad filled on the second deck did not survive the round trip"
        );
    }

    /// Both grids turn together. A song whose looks arrived but whose
    /// wells stayed behind is a song with the wrong picture, and the two
    /// grids are otherwise so independent that keeping them in step is
    /// exactly the kind of thing that quietly stops happening.
    #[test]
    fn a_page_turn_takes_gravity_with_it() {
        let (mut scenes, mut gravity) = grids();
        gravity.assign(5, "pull");
        let mut book = Book::from_grids(&scenes, &gravity);
        book.add(&mut scenes, &mut gravity).unwrap();
        assert_eq!(names(&gravity)[5], None, "gravity kept the old deck's pads");
        book.switch(0, &mut scenes, &mut gravity);
        assert_eq!(names(&gravity)[5].as_deref(), Some("pull"));
    }

    /// An edit made during a song is in the copy, not the version last
    /// stored. Duplicating is how the next song gets started, and it
    /// happens after the current one has been tweaked, not before.
    #[test]
    fn duplicating_the_live_deck_copies_what_is_on_the_pads_now() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::from_grids(&scenes, &gravity);
        scenes.assign(3, "late edit");

        let copy = book.duplicate(0, &mut scenes, &mut gravity).expect("a copy");
        assert_eq!(names(&scenes)[3].as_deref(), Some("late edit"));
        book.switch(0, &mut scenes, &mut gravity);
        assert_eq!(names(&scenes)[3].as_deref(), Some("late edit"));
        book.switch(copy, &mut scenes, &mut gravity);
        assert_eq!(names(&scenes)[3].as_deref(), Some("late edit"));
    }

    /// The last deck cannot be deleted. A show with no decks has nowhere
    /// to put a pad, and every operation here would need a branch for a
    /// state the performer can neither see nor undo.
    #[test]
    fn the_last_deck_cannot_be_removed() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        assert!(!book.remove(0, &mut scenes, &mut gravity));
        assert_eq!(book.len(), 1);
    }

    /// Deleting the deck you are on lands somewhere real and loads it,
    /// rather than leaving the index pointing past the end.
    #[test]
    fn removing_the_live_deck_lands_on_a_neighbour_and_loads_it() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        scenes.assign(0, "first");
        book.store(&scenes, &gravity);
        book.add(&mut scenes, &mut gravity).unwrap();
        scenes.assign(1, "second");

        assert!(book.remove(1, &mut scenes, &mut gravity));
        assert_eq!(book.len(), 1);
        assert_eq!(book.active(), 0);
        assert_eq!(
            names(&scenes)[0].as_deref(),
            Some("first"),
            "the surviving deck's pads were not loaded"
        );
        assert_eq!(names(&scenes)[1], None, "the deleted deck's pads are still on screen");
    }

    /// Removing a page ahead of the live one must not shift the live one
    /// out from under the performer.
    #[test]
    fn removing_an_earlier_deck_keeps_you_on_the_one_you_were_playing() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        book.add(&mut scenes, &mut gravity).unwrap();
        book.add(&mut scenes, &mut gravity).unwrap();
        scenes.assign(4, "third deck");
        assert_eq!(book.active(), 2);

        assert!(book.remove(0, &mut scenes, &mut gravity));
        assert_eq!(book.active(), 1, "the live deck moved to a different page");
        assert_eq!(
            names(&scenes)[4].as_deref(),
            Some("third deck"),
            "the pads were reloaded for a deck that did not change"
        );
    }

    /// Switching to the deck already showing does nothing at all — not
    /// even a store. A pad row on a controller sweeps across its buttons,
    /// and a switch that counted as a change would write a file per frame.
    #[test]
    fn switching_to_the_deck_already_showing_is_not_a_change() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        assert!(!book.switch(0, &mut scenes, &mut gravity));
        assert!(!book.switch(99, &mut scenes, &mut gravity));
    }

    /// Names are what tell two chips apart, so an empty one is refused
    /// and a new deck never reuses a name still in the book.
    #[test]
    fn deck_names_stay_distinct_and_never_go_blank() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        book.add(&mut scenes, &mut gravity).unwrap();
        let taken: Vec<&str> = book.decks().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(taken, vec!["deck 1", "deck 2"]);

        assert!(book.rename(0, "opener"));
        assert!(!book.rename(0, "   "), "a blank name was accepted");
        assert_eq!(book.decks()[0].name, "opener");

        // "deck 1" is free again, so the next deck takes it rather than
        // minting "deck 3" and leaving a gap.
        book.add(&mut scenes, &mut gravity).unwrap();
        assert_eq!(book.decks()[2].name, "deck 1");
    }

    /// The book fills up rather than growing without limit — the deck row
    /// is as wide as the pad row it pages through.
    #[test]
    fn the_book_stops_at_the_ceiling() {
        let (mut scenes, mut gravity) = grids();
        let mut book = Book::default();
        for _ in 1..MAX_DECKS {
            assert!(book.add(&mut scenes, &mut gravity).is_some());
        }
        assert_eq!(book.len(), MAX_DECKS);
        assert!(book.add(&mut scenes, &mut gravity).is_none(), "the book grew past its ceiling");
        assert_eq!(book.len(), MAX_DECKS);
    }

    /// A file from another build could say anything. Every field is
    /// brought back into range rather than trusted, because the
    /// alternative is a panic on the one file that represents the set.
    #[test]
    fn a_file_from_another_build_is_fitted_rather_than_trusted() {
        let mut book = Book {
            decks: vec![Deck {
                name: "short".into(),
                scenes: vec![None; 3],
                gravity: vec![None; 40],
                origin: 0,
            }],
            active: 12,
        };
        book.fit();
        assert_eq!(book.decks()[0].scenes.len(), SLOTS);
        assert_eq!(book.decks()[0].gravity.len(), SLOTS);
        assert_eq!(book.decks()[0].origin, 1, "column 0 does not exist in Resolume");
        assert_eq!(book.active(), 0, "the active index pointed past the end");
    }

    /// The origin is what makes a deck follow its own stretch of a long
    /// Resolume composition.
    #[test]
    fn a_deck_can_be_pointed_at_its_own_stretch_of_columns() {
        let mut book = Book::default();
        assert_eq!(book.origin(), 1, "a fresh deck does not follow column 1");
        assert!(book.set_origin(0, 17));
        assert_eq!(book.origin(), 17);
        assert!(!book.set_origin(0, 17), "setting the origin it already had counted as a change");
        assert!(book.set_origin(0, 0));
        assert_eq!(book.origin(), 1, "column 0 does not exist in Resolume");
    }

    /// A show prepared before decks existed becomes deck 1 with
    /// everything where it was. The grid file is an evening's
    /// preparation; losing it to a feature nobody asked for would be
    /// unforgivable.
    #[test]
    fn a_show_from_before_decks_becomes_deck_one_intact() {
        let (mut scenes, mut gravity) = grids();
        scenes.assign(7, "prepared");
        gravity.assign(1, "prepared well");
        let book = Book::from_grids(&scenes, &gravity);
        assert_eq!(book.len(), 1);
        assert_eq!(book.decks()[0].scenes[7].as_ref().map(|c| c.preset.as_str()), Some("prepared"));
        assert_eq!(
            book.decks()[0].gravity[1].as_ref().map(|c| c.preset.as_str()),
            Some("prepared well")
        );
        // And the grids themselves are untouched by being read.
        assert_eq!(names(&scenes)[7].as_deref(), Some("prepared"));
    }

    /// A lost or unreadable `grid.json` must not take a song with it.
    ///
    /// The two files mirror each other so that either can be lost — but
    /// the adoption ran unconditionally, and an unreadable grid loads as
    /// sixteen empty pads, which is indistinguishable from a page the
    /// user emptied. So the file that survived was overwritten by the
    /// wreckage of the one that did not, on the next launch, silently.
    #[test]
    fn a_grid_file_that_did_not_load_does_not_erase_its_page() {
        let (mut scenes, mut gravity) = grids();
        scenes.assign(0, "only copy");
        gravity.assign(1, "only well");
        let mut book = Book::from_grids(&scenes, &gravity);

        // What `read_kind` returns when the file is gone.
        book.adopt_live(None, None);
        assert_eq!(
            book.decks()[0].scenes[0].as_ref().map(|c| c.preset.as_str()),
            Some("only copy"),
            "an unreadable scene grid emptied the page"
        );
        assert_eq!(
            book.decks()[0].gravity[1].as_ref().map(|c| c.preset.as_str()),
            Some("only well")
        );

        // One file lost and the other not: the survivor is still adopted.
        scenes.assign(0, "edited since");
        book.adopt_live(Some(&scenes), None);
        assert_eq!(
            book.decks()[0].scenes[0].as_ref().map(|c| c.preset.as_str()),
            Some("edited since"),
            "a grid that did load was not adopted"
        );
        assert_eq!(
            book.decks()[0].gravity[1].as_ref().map(|c| c.preset.as_str()),
            Some("only well"),
            "the lost gravity grid took the page with it after all"
        );

        // And a grid that loaded genuinely empty still empties the page —
        // clearing every pad is a thing people do, and it has to stick.
        let (empty, _) = grids();
        book.adopt_live(Some(&empty), None);
        assert_eq!(book.decks()[0].scenes[0], None, "clearing every pad did not stick");
    }

    /// A round trip through the file format keeps every field. Serde
    /// derives make this look free; it is not, because the two cell rows
    /// carry `#[serde(default)]` and a rename on either side would
    /// silently produce empty pads rather than an error.
    #[test]
    fn a_book_survives_the_file_format() {
        let (mut scenes, mut gravity) = grids();
        scenes.assign(0, "a");
        gravity.assign(9, "b");
        let mut book = Book::from_grids(&scenes, &gravity);
        book.add(&mut scenes, &mut gravity).unwrap();
        book.rename(1, "encore");
        book.set_origin(1, 33);

        let json = serde_json::to_vec(&book).unwrap();
        let mut back: Book = serde_json::from_slice(&json).unwrap();
        back.fit();
        assert_eq!(back, book);
        assert_eq!(back.decks()[0].scenes[0].as_ref().map(|c| c.preset.as_str()), Some("a"));
        assert_eq!(back.decks()[1].name, "encore");
        assert_eq!(back.decks()[1].origin, 33);
    }
}
