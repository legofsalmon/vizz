//! Per-parameter working ranges for the panel's sliders.
//!
//! A parameter's real bounds have to be wide enough for everything it can
//! usefully do: `/particles/count` spans 0 to half a million because a
//! sparse field and a dense one are both legitimate. That makes the slider
//! useless for fine work — a pixel is a thousand particles, and there is
//! no way to nudge.
//!
//! A working range narrows what the *slider* covers without touching what
//! the parameter accepts. OSC, MIDI and presets keep the full range; only
//! the mouse is constrained, because the mouse is the control with a fixed
//! number of pixels to spend.
//!
//! Stored beside macros rather than with patches: how finely you want to
//! work is a property of your setup, not of a modulation graph, and
//! loading someone else's patch should not re-scale your sliders.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Address → (low, high), covering only parameters that have been
/// narrowed. Absent means "use the parameter's own bounds".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ranges {
    #[serde(default)]
    pub spans: BTreeMap<String, (f32, f32)>,
}

impl Ranges {
    pub fn path() -> PathBuf {
        crate::library::patch_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
            .join("ranges.json")
    }

    /// Load, falling back to empty. A corrupt file costs you your slider
    /// ranges, which is not a reason to refuse to start.
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            // Was fully silent: a corrupt file became defaults with no
            // trace, and the first range edit overwrote it.
            Err(e) => {
                log::warn!("could not read {}: {e} — using default ranges", path.display());
                crate::library::quarantine(&path);
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let dir = path.parent().context("ranges path has no parent")?;
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The span to show for a parameter, clamped inside its real bounds.
    ///
    /// A stored range from a build whose bounds have since changed is
    /// clamped rather than honoured: a slider that can reach outside what
    /// the parameter accepts would look broken at its own extremes.
    pub fn span(&self, addr: &str, min: f32, max: f32) -> (f32, f32) {
        let Some(&(lo, hi)) = self.spans.get(addr) else {
            return (min, max);
        };
        let lo = lo.clamp(min, max);
        let hi = hi.clamp(min, max);
        // An inverted or collapsed span would produce a slider that cannot
        // be moved; fall back rather than present one.
        if hi > lo { (lo, hi) } else { (min, max) }
    }

    pub fn is_narrowed(&self, addr: &str) -> bool {
        self.spans.contains_key(addr)
    }

    pub fn set(&mut self, addr: &str, lo: f32, hi: f32) {
        self.spans.insert(addr.to_string(), (lo, hi));
    }

    pub fn clear(&mut self, addr: &str) {
        self.spans.remove(addr);
    }

    /// Narrow to a window around the current value, which is what "zoom in
    /// here" means in practice — you are already near what you want and
    /// need finer control around it.
    ///
    /// `fraction` is how much of the full range to keep.
    pub fn zoom_around(&mut self, addr: &str, value: f32, min: f32, max: f32, fraction: f32) {
        let full = (max - min).abs();
        if full <= 0.0 {
            return;
        }
        let half = full * fraction.clamp(0.001, 1.0) * 0.5;
        // Shifted rather than clipped at the ends, so zooming near a bound
        // still gives the full window instead of half of one.
        let mut lo = value - half;
        let mut hi = value + half;
        if lo < min {
            hi += min - lo;
            lo = min;
        }
        if hi > max {
            lo -= hi - max;
            hi = max;
        }
        self.set(addr, lo.max(min), hi.min(max));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_parameter_uses_its_own_bounds() {
        let r = Ranges::default();
        assert_eq!(r.span("/particles/count", 0.0, 500_000.0), (0.0, 500_000.0));
        assert!(!r.is_narrowed("/particles/count"));
    }

    #[test]
    fn a_narrowed_parameter_uses_its_span() {
        let mut r = Ranges::default();
        r.set("/particles/count", 20_000.0, 60_000.0);
        assert_eq!(r.span("/particles/count", 0.0, 500_000.0), (20_000.0, 60_000.0));
        r.clear("/particles/count");
        assert_eq!(r.span("/particles/count", 0.0, 500_000.0), (0.0, 500_000.0));
    }

    /// Bounds change between releases. A stored span that no longer fits
    /// must be clamped, not honoured — a slider reaching past what the
    /// parameter accepts looks broken at both ends.
    #[test]
    fn a_stale_span_is_clamped_into_the_current_bounds() {
        let mut r = Ranges::default();
        r.set("/fx/glow", -5.0, 900.0);
        assert_eq!(r.span("/fx/glow", 0.0, 1.0), (0.0, 1.0));
        r.set("/fx/glow", 0.2, 4.0);
        assert_eq!(r.span("/fx/glow", 0.0, 1.0), (0.2, 1.0));
    }

    /// An inverted or zero-width span would give a slider that cannot
    /// move. Fall back to the real bounds rather than show one.
    #[test]
    fn an_unusable_span_falls_back_to_the_real_bounds() {
        let mut r = Ranges::default();
        r.set("/fx/glow", 0.8, 0.2);
        assert_eq!(r.span("/fx/glow", 0.0, 1.0), (0.0, 1.0));
        r.set("/fx/glow", 0.5, 0.5);
        assert_eq!(r.span("/fx/glow", 0.0, 1.0), (0.0, 1.0));
    }

    /// Zooming keeps the current value inside the new window — the point
    /// is finer control around where you already are.
    #[test]
    fn zooming_centres_on_the_current_value() {
        let mut r = Ranges::default();
        r.zoom_around("/particles/count", 60_000.0, 0.0, 500_000.0, 0.1);
        let (lo, hi) = r.span("/particles/count", 0.0, 500_000.0);
        assert!(lo < 60_000.0 && hi > 60_000.0, "value fell outside {lo}..{hi}");
        assert!((hi - lo - 50_000.0).abs() < 1.0, "window was {} wide", hi - lo);
    }

    /// Zooming next to a bound must still give a full-width window,
    /// shifted inward, rather than half of one.
    #[test]
    fn zooming_at_a_bound_keeps_the_window_full_width() {
        let mut r = Ranges::default();
        r.zoom_around("/fx/glow", 0.0, 0.0, 1.0, 0.2);
        let (lo, hi) = r.span("/fx/glow", 0.0, 1.0);
        assert_eq!(lo, 0.0);
        assert!((hi - 0.2).abs() < 1e-6, "window was {lo}..{hi}, expected a 0.2 span");
    }

    #[test]
    fn round_trips_through_disk() {
        let (_guard, dir) = crate::test_env::scoped("ranges");
        assert!(Ranges::load().spans.is_empty());
        let mut r = Ranges::default();
        r.set("/fx/glow", 0.1, 0.4);
        r.save().unwrap();
        assert_eq!(Ranges::load(), r);
        std::fs::write(Ranges::path(), b"{not json").unwrap();
        assert!(Ranges::load().spans.is_empty(), "corrupt file should not stop startup");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
