//! The semantic state colours — re-exported from the design system.
//!
//! This module used to *be* the theme; it is now a doorway. The whole
//! design language — these five states, the ink ramp, surfaces,
//! accents, feedback chrome, type scale, spacing, radii, motion and the
//! shared widgets — lives in the `vizz-design` crate, where a sister
//! app can reach it too. Existing `crate::theme::ARMED`-style call
//! sites keep working through this re-export; new code can use
//! `vizz_design::…` directly for the rest of the vocabulary.

pub use vizz_design::state::*;
