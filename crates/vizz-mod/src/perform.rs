//! Macro assignments for the performance layout.
//!
//! A macro is a slot pointing at a parameter address. The performance view
//! shows those slots as large faders and nothing else, so what you can
//! reach in a dark room is a decision made in advance rather than a hunt
//! through a full parameter list mid-set.
//!
//! Stored separately from patches on purpose: which parameters you want
//! under your fingers is a property of *how you play*, not of the
//! modulation graph. Loading someone else's patch should not rearrange
//! your faders.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Sixteen slots, laid out as two rows of eight.
///
/// This was eight, on the argument that a hard limit keeps every fader
/// large. The limit was real but the number was wrong: eight does not cover
/// one look's worth of controls, so playing meant leaving the layout to
/// reach the ninth thing — which is the one failure this screen exists to
/// prevent. Sixteen still fits at a size you can hit without aiming on any
/// display wide enough to run a set from, and matches the grid above it.
///
/// Growing this is safe for existing files: [`Macros::ensure_len`] pads
/// short lists with empty slots on load.
pub const MACRO_COUNT: usize = 16;

/// Fewest faders a set can be reduced to.
///
/// Not zero and not one. A performance screen with no faders is a
/// screen with nothing to play, and the desk below the pads would
/// collapse to a caption — which is a worse thing to have shipped than
/// four faders somebody is not using.
pub const MACRO_MIN: usize = 4;

/// Most faders a set can grow to.
///
/// Twenty-four is where the arithmetic stops working rather than an
/// arbitrary round number. They wrap at eight to a row, so 24 is three
/// full rows; at a 1400-point desk that is about 170 points a column,
/// which still carries a name, a value and a MIDI chip on three lines.
/// A fourth row would take the height back off the output pane, which
/// is the thing the whole layout exists to protect — and faders you
/// cannot see the picture past are not more control, they are less.
pub const MACRO_MAX: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macros {
    /// OSC-style parameter address per slot; `None` is an empty fader.
    pub slots: Vec<Option<String>>,
}

impl Default for Macros {
    fn default() -> Self {
        // A useful starting set rather than sixteen blanks: these are the
        // controls most worth having under a hand during a set.
        //
        // Ordered so the first row is the one you reach for constantly —
        // size, motion, shape, the two effects that change the whole
        // picture — and the second row is the colour and framing you set
        // up once and revisit. A row is a reach, so what shares a row
        // matters as much as what is present.
        let defaults = [
            // Row one: the things that get moved during a track.
            "/particles/size",
            "/particles/speed",
            "/particles/count",
            "/shape/mode",
            "/shape/morph",
            "/fx/trail",
            "/fx/glow",
            "/fx/mirror",
            // Row two: colour and framing.
            "/particles/hue",
            "/particles/saturation",
            "/particles/brightness",
            "/color/palette",
            "/color/spread",
            "/shape/twist",
            "/fx/zoom",
            "/particles/spread",
        ];
        Self {
            slots: defaults.iter().map(|s| Some((*s).to_string())).collect(),
        }
    }
}

impl Macros {
    pub fn path() -> PathBuf {
        crate::project::show_dir().join("macros.json")
    }

    /// Load, falling back to the defaults. A corrupt or absent file is a
    /// normal condition — losing your fader layout should not stop the app
    /// starting, least of all at a venue.
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(mut m) => {
                // Tolerate a file written by a build with a different slot
                // count rather than panicking on index.
                // Clamped rather than forced to sixteen: the count is a
                // saved preference now, so a file asking for twenty is
                // honoured and a file asking for two hundred is not.
                let want = m.slots.len().clamp(MACRO_MIN, MACRO_MAX);
                m.slots.resize(want, None);
                m
            }
            // Was fully silent: a corrupt file became defaults with no
            // trace, and the first fader assignment overwrote it.
            Err(e) => {
                log::warn!(
                    "could not read {}: {e} — using default fader assignments",
                    path.display()
                );
                crate::library::quarantine(&path);
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let dir = path.parent().context("macros path has no parent")?;
        std::fs::create_dir_all(dir)?;
        let tmp = crate::library::tmp_path(&path);
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        self.slots.get(i).and_then(|s| s.as_deref())
    }

    /// How many faders this set has.
    pub fn count(&self) -> usize {
        self.slots.len().clamp(MACRO_MIN, MACRO_MAX)
    }

    /// Add a fader, up to [`MACRO_MAX`]. The new one starts empty.
    ///
    /// Returns whether anything changed, so the caller knows whether to
    /// persist and whether to say something.
    pub fn grow(&mut self) -> bool {
        if self.count() >= MACRO_MAX {
            return false;
        }
        self.slots.resize(self.count() + 1, None);
        true
    }

    /// Remove the last fader, down to [`MACRO_MIN`].
    ///
    /// Takes the last one rather than the last *empty* one: which fader
    /// goes has to be predictable from looking at the row, and "the one
    /// on the end" is the only rule that is. Its assignment is dropped
    /// with it, which is why the caller warns when it was not empty.
    pub fn shrink(&mut self) -> bool {
        if self.count() <= MACRO_MIN {
            return false;
        }
        self.slots.truncate(self.count() - 1);
        true
    }

    /// What the last fader holds, for a caller deciding whether removing
    /// it is worth a warning.
    pub fn last_assigned(&self) -> Option<&str> {
        self.slots.last().and_then(|s| s.as_deref())
    }

    pub fn set(&mut self, i: usize, addr: Option<String>) {
        if self.slots.len() < MACRO_MIN {
            self.slots.resize(MACRO_MIN, None);
        }
        if let Some(slot) = self.slots.get_mut(i) {
            *slot = addr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fader count is a preference, within limits that keep the
    /// screen usable.
    #[test]
    fn the_count_grows_and_shrinks_between_its_limits() {
        let mut m = Macros::default();
        assert_eq!(m.count(), MACRO_COUNT, "the default set is not the default count");

        while m.grow() {}
        assert_eq!(m.count(), MACRO_MAX, "growing did not stop at the maximum");
        assert!(!m.grow(), "growing past the maximum reported success");

        while m.shrink() {}
        assert_eq!(m.count(), MACRO_MIN, "shrinking did not stop at the minimum");
        assert!(!m.shrink(), "shrinking past the minimum reported success");
    }

    /// A file asking for an absurd count is clamped, not obeyed.
    ///
    /// The count is saved now, so it is untrusted input like any other
    /// file: a hand-edited or corrupt macros.json must not be able to
    /// ask for two hundred faders and get a screen that cannot draw.
    #[test]
    fn a_saved_count_is_clamped_on_the_way_in() {
        let mut m = Macros { slots: vec![None; 500] };
        assert_eq!(m.count(), MACRO_MAX, "an absurd count was taken at face value");
        // And it can still be edited without panicking.
        m.set(0, Some("/particles/size".into()));
        assert_eq!(m.get(0), Some("/particles/size"));

        let tiny = Macros { slots: vec![None; 1] };
        assert_eq!(tiny.count(), MACRO_MIN, "a too-small count was taken at face value");
    }

    /// Shrinking takes the fader on the end, and says what it held.
    ///
    /// Which one goes has to be predictable from looking at the row.
    #[test]
    fn shrinking_takes_the_last_fader_and_reports_what_it_held() {
        let mut m = Macros::default();
        let last = m.count() - 1;
        m.set(last, Some("/fx/glow".into()));
        assert_eq!(m.last_assigned(), Some("/fx/glow"));
        assert!(m.shrink());
        assert_eq!(m.count(), MACRO_COUNT - 1);
        // The one before it is untouched.
        assert_eq!(m.get(last), None, "shrinking left the removed slot readable");
    }

    #[test]
    fn defaults_fill_every_slot() {
        let m = Macros::default();
        assert_eq!(m.slots.len(), MACRO_COUNT);
        assert!(m.slots.iter().all(|s| s.is_some()), "{:?}", m.slots);
    }

    /// A file from a build with fewer slots must not make `get` panic or
    /// silently drop the layout — it should widen to the current count.
    #[test]
    fn a_short_file_widens_rather_than_panicking() {
        let mut m = Macros {
            slots: vec![Some("/a".into()), Some("/b".into())],
        };
        m.slots.resize(MACRO_COUNT, None);
        assert_eq!(m.slots.len(), MACRO_COUNT);
        assert_eq!(m.get(0), Some("/a"));
        assert_eq!(m.get(7), None);
        // And out of range is None, not a panic.
        assert_eq!(m.get(99), None);
    }

    #[test]
    fn setting_past_the_end_is_ignored_not_a_panic() {
        let mut m = Macros { slots: vec![None] };
        // A short set is brought up to the minimum, so the early slots
        // are addressable.
        m.set(3, Some("/fx/glow".into()));
        assert_eq!(m.get(3), Some("/fx/glow"));
        // Far past the end is ignored rather than growing to fit. This
        // used to assert the length was exactly MACRO_COUNT, because
        // `set` forced every set to sixteen slots — which is no longer
        // true or wanted: the count is a saved preference now, and a
        // stray write must not be able to change it.
        m.set(999, Some("/nope".into()));
        assert_eq!(m.get(999), None, "an out-of-range write landed somewhere");
        assert!(
            (MACRO_MIN..=MACRO_MAX).contains(&m.count()),
            "a stray write moved the count to {}",
            m.count()
        );
    }

    #[test]
    fn round_trips_through_disk() {
        let (_guard, dir) = crate::test_env::scoped("macros");

        // Absent file yields the defaults rather than an error.
        assert!(Macros::load().slots.iter().all(|s| s.is_some()));

        let mut m = Macros::default();
        m.set(0, None);
        m.set(1, Some("/fx/shift".into()));
        m.save().expect("save failed");

        let back = Macros::load();
        assert_eq!(back.get(0), None);
        assert_eq!(back.get(1), Some("/fx/shift"));

        // Corrupt file falls back to defaults instead of refusing to start.
        std::fs::write(Macros::path(), b"{not json").unwrap();
        assert!(Macros::load().slots.iter().all(|s| s.is_some()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
