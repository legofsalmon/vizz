//! The semantic state colours, shared by every screen.
//!
//! These existed as per-module constants and drifted: the performance
//! layout's "learn" amber was a different hue from the panel's and the
//! grid's, its "live" green and "warning" orange were near-miss copies
//! of the panel's, and the same meaning read differently depending on
//! which screen you were looking at. One meaning, one colour — a state
//! language only works if it is the same language everywhere.
//!
//! Layout inks (the text ramps, track fills, panel backgrounds) stay
//! per-module: those are typography, and the two screens legitimately
//! set type differently. Only *state* lives here.

use egui::Color32;

/// A MIDI learn is armed and waiting. Matches the global learn banner.
pub const LEARN: Color32 = Color32::from_rgb(255, 200, 90);

/// An output, input or clock is alive.
pub const LIVE: Color32 = Color32::from_rgb(104, 208, 132);

/// Something needs attention but nothing is broken yet.
pub const WARN: Color32 = Color32::from_rgb(255, 170, 90);

/// A destructive action is armed: the next press does it.
pub const ARMED: Color32 = Color32::from_rgb(255, 120, 90);

/// The current item — the recalled preset, the playing pad.
pub const CURRENT: Color32 = Color32::from_rgb(110, 180, 255);
