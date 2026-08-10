//! The vizz design language — every token in one crate.
//!
//! This grew out of `vizz-ui`'s theme module the way that module grew out
//! of per-screen constants: the same meanings kept being restated in
//! near-miss copies, and a state language only works if it is the same
//! language everywhere. Now the whole vocabulary lives here — colour,
//! type, spacing, radii, timing, and the shared widgets that enforce the
//! interaction idioms — so a sister app starts from the identical
//! language by adding one dependency, and vizz itself cannot drift.
//!
//! The structure borrows from the systems that got this right at scale.
//! From Material: tokens are *roles*, not swatches — nothing here is
//! called "orange", things are called `WARN`. From Apple's HIG: the ink
//! ramp is semantic emphasis (primary / secondary / tertiary / faint),
//! and colour states carry their "on" counterparts so text on a filled
//! state chip is part of the token, not a guess at the call site.
//!
//! Tuned throughout for what this instrument actually is: a dark-room
//! tool read at a glance from across a stage. Dark surfaces only; state
//! reads at distance; anything that matters is said in words as well as
//! colour (red against green is exactly the pair that collapses for
//! colour-blind eyes).
//!
//! ## What becomes a token
//!
//! A colour (or size, or duration) becomes a token when one *meaning*
//! appears in more than one place. Decorative values that are computed
//! (the beat glow), or that belong to a single specialised surface (the
//! node editor's port blues, wire strokes and selection chrome), stay
//! where they are used — hoisting every literal would make the system
//! noise. The enforcement test at the bottom of this crate holds the
//! line where drift has actually bitten: the state colours may not be
//! restated as literals anywhere in `vizz-ui`.
//!
//! ## Adopting this in a sister app
//!
//! Depend on `vizz-design`, build screens from the tokens and widgets,
//! and follow the contracts documented on each module: every control
//! hovers, destructive clicks arm first ([`widgets::armed_button`]),
//! numbers that change wear a monospace face, one meaning one colour.
//! The specimen sheet (`cargo run -p vizz-ui --example render_specimen`)
//! renders the whole vocabulary to one image for review by eye.

pub mod widgets;

/// The five semantic states, plus the inks that sit on them.
///
/// One meaning, one colour. These are the words of the state language:
/// if a screen needs to say one of these things, it uses this colour,
/// and if it needs a colour not here, it is probably saying something
/// new — add the meaning, not a lookalike.
pub mod state {
    use egui::Color32;

    /// A MIDI learn is armed and waiting. Matches the global learn banner.
    pub const LEARN: Color32 = Color32::from_rgb(255, 200, 90);
    /// Ink on a `LEARN`-filled control.
    pub const ON_LEARN: Color32 = Color32::from_rgb(46, 32, 12);

    /// An output, input or clock is alive.
    pub const LIVE: Color32 = Color32::from_rgb(104, 208, 132);

    /// Something needs attention but nothing is broken yet.
    pub const WARN: Color32 = Color32::from_rgb(255, 170, 90);

    /// A destructive action is armed: the next press does it.
    pub const ARMED: Color32 = Color32::from_rgb(255, 120, 90);
    /// Ink on an `ARMED`-filled control.
    pub const ON_ARMED: Color32 = Color32::from_rgb(50, 18, 12);

    /// The current item — the recalled preset, the playing pad.
    pub const CURRENT: Color32 = Color32::from_rgb(110, 180, 255);
}

/// The text ramp: semantic emphasis, Apple-style, four stops.
///
/// Anything that matters is `PRIMARY` or `SECONDARY`; `FAINT` means
/// "this is off". The ramp is the whole typography-colour story — a
/// label picks its stop by importance, never by taste.
pub mod ink {
    use egui::Color32;

    pub const PRIMARY: Color32 = Color32::from_rgb(236, 240, 246);
    pub const SECONDARY: Color32 = Color32::from_rgb(178, 187, 200);
    pub const TERTIARY: Color32 = Color32::from_rgb(132, 141, 156);
    /// Off, dead, disabled — including the hollow status dot of an
    /// output that is not sending.
    pub const FAINT: Color32 = Color32::from_rgb(94, 101, 114);
}

/// The dark ground everything sits on, and its structural greys.
///
/// Levels, not hues: `BASE` is the room, `WELL` is set into it, `RAISED`
/// is anything you can touch (buttons, tracks, node bodies), and the
/// hairline/edge/tick greys draw structure without competing with state.
pub mod surface {
    use egui::Color32;

    /// The application ground — panel chrome, the canvas behind nodes.
    pub const BASE: Color32 = Color32::from_rgb(23, 25, 30);
    /// Set into the base: palette frames, inset lists.
    pub const WELL: Color32 = Color32::from_rgb(30, 33, 38);
    /// Touchable: button fills, fader tracks, node bodies.
    pub const RAISED: Color32 = Color32::from_rgb(38, 41, 48);

    /// An empty slot in a bank (a pad with nothing on it).
    pub const SLOT_EMPTY: Color32 = Color32::from_rgb(34, 36, 42);
    /// A filled slot at rest.
    pub const SLOT: Color32 = Color32::from_rgb(58, 66, 84);

    /// A control that is actively engaged and must read loud — the lit
    /// punch button. Near-white on purpose: it is the one thing on the
    /// screen whose job is to be unmissable.
    pub const ENGAGED: Color32 = Color32::from_rgb(232, 236, 242);
    /// Ink on `ENGAGED`.
    pub const ON_ENGAGED: Color32 = Color32::from_rgb(24, 26, 32);

    /// The grabbable part of a fader.
    pub const HANDLE: Color32 = Color32::from_rgb(226, 233, 242);

    /// Section rules, meter frames, the canvas dot grid — structure at
    /// its quietest.
    pub const HAIRLINE: Color32 = Color32::from_rgb(44, 48, 56);
    /// Button and chip outlines: an edge, so a row of controls reads as
    /// controls rather than as caption text.
    pub const EDGE: Color32 = Color32::from_rgb(62, 68, 82);
    /// Scale ticks on tracks and phase bars.
    pub const TICK: Color32 = Color32::from_rgb(70, 76, 88);
    /// The hover ring: without it there is no way to tell a live control
    /// from a picture of one until you have already moved it.
    pub const FOCUS: Color32 = Color32::from_rgb(120, 150, 185);
}

/// Instrument accents: the recurring non-state colours with fixed jobs.
pub mod accent {
    use egui::Color32;

    /// Modulation — "something else is moving this". Warm against the
    /// value blues so it reads instantly, on faders and in the panel.
    pub const MOD: Color32 = Color32::from_rgb(255, 190, 90);
    /// A global (preset-exempt) parameter's marker.
    pub const GLOBAL: Color32 = Color32::from_rgb(120, 170, 220);

    /// A fader's value fill, and the brighter cap edge the eye finds
    /// faster than it judges a flat block's height.
    pub const FILL: Color32 = Color32::from_rgb(74, 128, 178);
    pub const FILL_BRIGHT: Color32 = Color32::from_rgb(96, 158, 214);

    /// Live signal meters: LFO outputs, audio sparks, the beat pulse.
    pub const METER: Color32 = Color32::from_rgb(130, 190, 255);
    /// The meter's dim companion (history, the unaccented half).
    pub const METER_DIM: Color32 = Color32::from_rgb(70, 95, 125);

    /// The master fader is red so a hand finds it without reading.
    pub const MASTER: Color32 = Color32::from_rgb(178, 78, 78);
    pub const MASTER_INK: Color32 = Color32::from_rgb(226, 150, 150);

    /// A transition in flight — the pad being blended to.
    pub const ARRIVING: Color32 = Color32::from_rgb(255, 175, 80);

    /// The autopilot's own colour. Green rather than `CURRENT` blue or
    /// `ARRIVING` amber: those say where the grid *is*, this says
    /// something is driving it.
    pub const AUTO: Color32 = Color32::from_rgb(72, 160, 104);
    pub const AUTO_BED: Color32 = Color32::from_rgb(30, 46, 36);

    /// A MIDI binding chip at rest — quiet enough that a fully mapped
    /// grid does not read as sixteen alarms.
    pub const BINDING: Color32 = Color32::from_rgb(158, 180, 206);

    /// Recording: the chip's fill, bed and ink. Forgetting a recording
    /// is how disks fill mid-set, so this family exists to be seen.
    pub const REC: Color32 = Color32::from_rgb(150, 40, 36);
    pub const REC_BED: Color32 = Color32::from_rgb(38, 26, 28);
    pub const REC_INK: Color32 = Color32::from_rgb(196, 106, 100);

    /// Node-editor category hues: where a value comes from, what bends
    /// it, where it lands.
    pub const NODE_SOURCE: Color32 = Color32::from_rgb(70, 120, 175);
    pub const NODE_OPERATOR: Color32 = Color32::from_rgb(150, 120, 60);
    pub const NODE_SINK: Color32 = Color32::from_rgb(70, 140, 100);
}

/// Feedback chrome: what success, failure and danger sit on.
///
/// Two families. Inline text feedback (`OK_TEXT`, `ERR_TEXT`) colours a
/// line in place; sheet feedback (`*_BED` + `ON_*`) is a filled surface
/// — notices, the quit prompt, the learn banner — red enough to be
/// found at a glance, dark enough not to strobe the room.
pub mod feedback {
    use egui::Color32;

    /// "saved", inline. Errors must never share this colour — that is
    /// how load failures went unnoticed on the canvas once.
    pub const OK_TEXT: Color32 = Color32::from_rgb(150, 200, 160);
    /// "load failed", inline.
    pub const ERR_TEXT: Color32 = Color32::from_rgb(235, 150, 140);

    /// A confirmation notice's bed and ink.
    pub const OK_BED: Color32 = Color32::from_rgb(26, 34, 30);
    pub const ON_OK: Color32 = Color32::from_rgb(170, 220, 185);

    /// Danger sheets — error notices, the quit prompt — and the fill of
    /// an armed destructive button.
    pub const DANGER_BED: Color32 = Color32::from_rgb(120, 40, 36);
    pub const DANGER_FILL: Color32 = Color32::from_rgb(150, 52, 46);
    pub const ON_DANGER: Color32 = Color32::from_rgb(255, 236, 232);
    pub const ON_DANGER_DIM: Color32 = Color32::from_rgb(240, 200, 194);

    /// The armed-learn banner's bed and ink (the outline is
    /// [`crate::state::LEARN`]).
    pub const LEARN_BED: Color32 = Color32::from_rgb(64, 50, 18);
    pub const ON_LEARN_BED: Color32 = Color32::from_rgb(255, 224, 170);
}

/// The type scale, by role. Sizes in points.
///
/// Rules that travel with the scale: numbers that change every frame
/// (fps, bpm, values) wear a monospace face and pad to fixed width, or
/// the line reflows underneath the eye; section headers are `SECTION`
/// small caps-ish and strong; nothing on a stage screen goes below
/// `MICRO`, and `MICRO` only where a chip shares a 40-point pad.
pub mod text {
    /// Dense chips riding on another control (a pad's binding).
    pub const MICRO: f32 = 8.0;
    /// Slot numbers and other indices.
    pub const INDEX: f32 = 9.0;
    /// Section headers (drawn strong, tracked out).
    pub const SECTION: f32 = 10.5;
    /// Chips, hints, learn tags.
    pub const CAPTION: f32 = 11.0;
    /// Control labels under faders.
    pub const LABEL: f32 = 12.0;
    /// Body: buttons, rows, most prose.
    pub const BODY: f32 = 13.0;
    /// Big touch targets — the preset row.
    pub const CONTROL: f32 = 14.0;
    /// Full-screen moments: the quit prompt's headline.
    pub const BANNER: f32 = 20.0;
}

/// Spacing, by role. A 2-point base with the steps the screens use.
pub mod space {
    /// Inside a chip, between a glyph and its edge.
    pub const CHIP: f32 = 2.0;
    /// Between siblings in a dense row (grid pads, strip items).
    pub const GAP: f32 = 4.0;
    /// A control's inset from its container.
    pub const INSET: f32 = 8.0;
    /// Between sections of one screen.
    pub const SECTION: f32 = 10.0;
    /// A screen's outer padding.
    pub const PAD: f32 = 14.0;
}

/// Corner radii, by role.
pub mod radius {
    /// The latch pip and other markers.
    pub const PIP: f32 = 1.0;
    /// Small chips and meters.
    pub const CHIP: f32 = 2.0;
    /// Buttons, pads — the default touchable.
    pub const CONTROL: f32 = 3.0;
    /// Fader tracks and other tall wells.
    pub const TRACK: f32 = 5.0;
    /// Sheets: banners, prompts, notices.
    pub const SHEET: f32 = 6.0;
}

/// Timing. Feedback has a clock, and the clock is part of the language.
pub mod motion {
    use std::time::Duration;

    /// How long an armed destructive control stays armed. Long enough to
    /// mean it, short enough that a stray first click cannot ambush a
    /// press half a song later.
    pub const ARM_WINDOW: f64 = 3.0;

    /// Inline status fades: stale feedback next to a changed surface
    /// reads as a fresh result, so it goes. Failures hold longer —
    /// the point is being seen on the *next* glance, not the current.
    pub const STATUS_TTL: f64 = 4.0;
    pub const STATUS_ERROR_TTL: f64 = 8.0;

    /// Corner notices: confirmations go quickly, errors survive until
    /// the performer next looks over.
    pub const NOTICE_TTL: Duration = Duration::from_secs(4);
    pub const NOTICE_ERROR_TTL: Duration = Duration::from_secs(15);
}

#[cfg(test)]
mod tests {
    /// The state colours may not be restated as literals in `vizz-ui`.
    ///
    /// This is the drift that motivated the whole crate: three screens
    /// each carrying their own amber, green and orange, close enough to
    /// pass a glance and different enough to read as different states.
    /// Any file needing these colours points at the tokens; a literal
    /// copy compiles fine and drifts silently, which is why this test
    /// reads source rather than trusting the type system.
    #[test]
    fn state_colours_are_never_restated_in_vizz_ui() {
        let banned = [
            ("state::LEARN", "255, 200, 90"),
            ("state::LIVE", "104, 208, 132"),
            ("state::WARN", "255, 170, 90"),
            ("state::ARMED", "255, 120, 90"),
            ("state::CURRENT", "110, 180, 255"),
            ("feedback::DANGER_FILL", "150, 52, 46"),
            ("ink::PRIMARY", "236, 240, 246"),
        ];
        let ui_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../vizz-ui/src");
        let mut checked = 0;
        for entry in std::fs::read_dir(&ui_src).expect("vizz-ui/src missing") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            checked += 1;
            let src = std::fs::read_to_string(&path).unwrap();
            for (token, triple) in banned {
                assert!(
                    !src.contains(&format!("from_rgb({triple})")),
                    "{} restates {token} as a literal — use the token",
                    path.display()
                );
            }
        }
        assert!(checked >= 5, "looked at {checked} files — wrong directory?");
    }
}
