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

/// Eight is a deliberate limit. Enough for the things worth reaching for,
/// few enough that each one can be large and unambiguous under stage
/// lighting — the constraint is the point.
pub const MACRO_COUNT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macros {
    /// OSC-style parameter address per slot; `None` is an empty fader.
    pub slots: Vec<Option<String>>,
}

impl Default for Macros {
    fn default() -> Self {
        // A useful starting set rather than eight blanks: these are the
        // controls most worth having under a hand during a set.
        let defaults = [
            "/particles/size",
            "/particles/speed",
            "/shape/mode",
            "/shape/morph",
            "/fx/trail",
            "/fx/glow",
            "/fx/mirror",
            "/particles/hue",
        ];
        Self {
            slots: defaults.iter().map(|s| Some((*s).to_string())).collect(),
        }
    }
}

impl Macros {
    pub fn path() -> PathBuf {
        crate::library::patch_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
            .join("macros.json")
    }

    /// Load, falling back to the defaults. A corrupt or absent file is a
    /// normal condition — losing your fader layout should not stop the app
    /// starting, least of all at a venue.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Self>(&b).ok()) {
            Some(mut m) => {
                // Tolerate a file written by a build with a different slot
                // count rather than panicking on index.
                m.slots.resize(MACRO_COUNT, None);
                m
            }
            None => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let dir = path.parent().context("macros path has no parent")?;
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        self.slots.get(i).and_then(|s| s.as_deref())
    }

    pub fn set(&mut self, i: usize, addr: Option<String>) {
        if self.slots.len() < MACRO_COUNT {
            self.slots.resize(MACRO_COUNT, None);
        }
        if let Some(slot) = self.slots.get_mut(i) {
            *slot = addr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut m = Macros { slots: vec![Some("/a".into()), Some("/b".into())] };
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
        m.set(3, Some("/fx/glow".into()));
        assert_eq!(m.get(3), Some("/fx/glow"));
        m.set(999, Some("/nope".into()));
        assert_eq!(m.slots.len(), MACRO_COUNT);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("vizz-macro-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: single-threaded test, set before any path is read.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

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
