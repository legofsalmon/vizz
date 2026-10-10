//! The vizz design language — every token in one crate.
//!
//! This grew out of `vizz-ui`'s theme module the way that module grew out
//! of per-screen constants: the same meanings kept being restated in
//! near-miss copies, and a state language only works if it is the same
//! language everywhere. Now the whole vocabulary lives here — colour,
//! type, spacing, radii, timing, and the shared widgets that enforce the
//! interaction idioms — so vizz itself cannot drift.
//!
//! ## Where the values come from
//!
//! The values are the studio's shared design system
//! (legofsalmon/design-system), vendored as [`tokens`] by its
//! `scripts/sync.mjs` and never edited here. This crate keeps its job as
//! vizz's vocabulary: the names the screens use (`state::ARMED`,
//! `surface::GROOVE`, `accent::DRIVEN`) and the widgets, each name
//! pointing at the shared role that means the same thing. The grammar is
//! the shared one: neutral at rest, cyan is you, amber is attention, red
//! is stop, and healthy is quiet. vizz is dark only, so every colour
//! here is the dark theme's.
//!
//! A meaning only vizz has (the slot fills, the engaged punch, the groove
//! a fader sits in, "something other than your hand is driving this",
//! recording) stays a local name, built from the shared palette where
//! the palette has the value, with a doc comment saying what it is for.
//! It moves into the shared system when a second app needs it.
//!
//! ## What becomes a token
//!
//! A colour (or size, or duration) becomes a token when one *meaning*
//! appears in more than one place. Decorative values that are computed
//! (the beat glow), or that belong to a single specialised surface (the
//! node editor's category hues), stay where they are used — hoisting
//! every literal would make the system noise. The enforcement test at
//! the bottom of this crate holds the line where drift has actually
//! bitten: the state colours may not be restated as literals anywhere in
//! `vizz-ui`.
//!
//! ## Using it
//!
//! Build screens from the tokens and widgets, call [`look::apply`] once
//! on the egui context so egui's own widgets draw the shared states, and
//! follow the contracts documented on each module: every control hovers,
//! destructive clicks arm first ([`widgets::armed_button`]), numbers that
//! change wear a monospace face, one meaning one colour. The specimen
//! sheet (`cargo run -p vizz-ui --example render_specimen`) renders the
//! whole vocabulary to one image for review by eye.

use egui::Color32;

/// The shared design system's tokens, vendored. Regenerate with
/// `python3 scripts/design-system.py --sync`; never edit by hand.
#[rustfmt::skip]
#[allow(clippy::all)]
pub mod tokens;

pub mod look;
pub mod widgets;

use tokens::color::dark as role;
use tokens::palette;

/// A colour from the design system, as egui takes it.
pub const fn rgba(c: tokens::Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied_const(c.r, c.g, c.b, c.a)
}

/// A translucent role laid over an opaque surface, flattened to one
/// opaque colour.
///
/// The system's `soft` washes are translucent so a web page can lay them
/// on any surface. vizz draws its sheets over the output picture, and a
/// wash over a strobing frame strobes with it, so every bed here is
/// flattened onto the surface it is meant to sit on.
pub const fn over(top: tokens::Rgba, under: tokens::Rgba) -> Color32 {
    const fn mix(t: u8, u: u8, a: u8) -> u8 {
        ((t as u32 * a as u32 + u as u32 * (255 - a as u32) + 127) / 255) as u8
    }
    Color32::from_rgb(
        mix(top.r, under.r, top.a),
        mix(top.g, under.g, top.a),
        mix(top.b, under.b, top.a),
    )
}

/// The five semantic states, plus the inks that sit on them.
///
/// One meaning, one colour. These are the words of the state language:
/// if a screen needs to say one of these things, it uses this colour,
/// and if it needs a colour not here, it is probably saying something
/// new — add the meaning, not a lookalike.
pub mod state {
    use super::{rgba, role};
    use egui::Color32;

    /// A MIDI learn is armed and waiting for the next control to move.
    ///
    /// Attention, in the shared grammar's amber: something is held, and
    /// the next move binds. It used to be a yellow of its own a few
    /// points from `WARN`, which read as the same colour from across a
    /// room anyway; it is now honestly the same colour, and the words
    /// (the global learn banner, "waiting", "learn") and the breathing
    /// rim tell a waiting learn from a broken pad.
    pub const LEARN: Color32 = rgba(role::warn::DEFAULT);
    /// Ink on a `LEARN`-filled control.
    pub const ON_LEARN: Color32 = rgba(role::warn::ON);

    /// An output, input or clock is alive. Green, which the grammar
    /// spends only where working has to be told apart from off at a
    /// glance: an output, a link, a clock. A frame rate that is fine is
    /// not one of those — healthy is quiet.
    pub const LIVE: Color32 = rgba(role::ok::DEFAULT);

    /// Something needs attention but nothing is broken yet.
    pub const WARN: Color32 = rgba(role::warn::DEFAULT);

    /// A destructive action is armed: the next press does it.
    pub const ARMED: Color32 = rgba(role::bad::DEFAULT);
    /// Ink on an `ARMED`-filled control.
    pub const ON_ARMED: Color32 = rgba(role::bad::ON);

    /// The current item — the recalled preset, the playing pad, a toggle
    /// that is on. The shared accent: cyan is you.
    pub const CURRENT: Color32 = rgba(role::accent::DEFAULT);
    /// Ink on a `CURRENT`-filled control.
    pub const ON_CURRENT: Color32 = rgba(role::accent::ON);
}

/// The text ramp: semantic emphasis, four stops.
///
/// Anything that matters is `PRIMARY` or `SECONDARY`; `FAINT` means
/// "this is off". The ramp is the whole typography-colour story — a
/// label picks its stop by importance, never by taste. `TERTIARY` is for
/// hints, units and timestamps on the surfaces content sits on, never on
/// a raised control: text there is primary or secondary.
pub mod ink {
    use super::{rgba, role};
    use egui::Color32;

    pub const PRIMARY: Color32 = rgba(role::ink::PRIMARY);
    pub const SECONDARY: Color32 = rgba(role::ink::SECONDARY);
    pub const TERTIARY: Color32 = rgba(role::ink::TERTIARY);
    /// Off, dead, disabled — including the hollow status dot of an
    /// output that is not sending. Never for anything anyone has to read.
    pub const FAINT: Color32 = rgba(role::ink::DISABLED);
    /// Dark text on a light fill (a meter's bar, the engaged punch).
    pub const INVERSE: Color32 = rgba(role::ink::INVERSE);
}

/// The dark ground everything sits on, and its structural greys.
///
/// Levels, not hues: `BASE` is the room, `WELL` is set into it, `RAISED`
/// is anything you can touch (buttons, tracks, node bodies), and the
/// hairline/edge/tick greys draw structure without competing with state.
/// They are the shared neutrals: untinted, because a tinted chrome
/// shifts what the eye reads as white, and this is a tool for judging a
/// picture.
pub mod surface {
    use super::{palette, rgba, role};
    use egui::Color32;

    /// The application ground — panel chrome, the canvas behind nodes.
    pub const BASE: Color32 = rgba(role::surface::PANEL);
    /// Set into the base: palette frames, inset lists.
    pub const WELL: Color32 = rgba(role::surface::INSET);
    /// A slot milled into the deck — the one control on this screen that
    /// is a hole rather than a surface.
    ///
    /// Below `BASE` on purpose: the shared ground, a step under the
    /// panel. It is the only well value that becomes *more* distinct when
    /// a white output frame lifts the ground through the performance
    /// scrim, where a raised block goes the other way and all but
    /// vanishes. It also lowers the deck's total emitted light in a dark
    /// room, which is the point.
    pub const GROOVE: Color32 = rgba(role::surface::GROUND);
    /// Touchable: button fills, fader tracks, node bodies.
    pub const RAISED: Color32 = rgba(role::surface::RAISED);
    /// Touchable, under the pointer.
    pub const RAISED_HOVER: Color32 = rgba(role::surface::RAISED_HOVER);
    /// Anything that floats over the rest: notices, the quit prompt, the
    /// learn banner, menus.
    pub const OVERLAY: Color32 = rgba(role::surface::OVERLAY);

    /// An empty slot in a bank (a pad with nothing on it): a well.
    pub const SLOT_EMPTY: Color32 = rgba(role::surface::INSET);
    /// A filled slot at rest. vizz's own: a step above anything raised,
    /// so a bank reads as filled and empty at a glance.
    pub const SLOT: Color32 = rgba(palette::neutral::TONE_26);

    /// A control that is actively engaged and must read loud — the lit
    /// punch button. Near-white on purpose: it is the one thing on the
    /// screen whose job is to be unmissable. vizz's own.
    pub const ENGAGED: Color32 = rgba(palette::neutral::TONE_94);
    /// Ink on `ENGAGED`.
    pub const ON_ENGAGED: Color32 = rgba(role::ink::INVERSE);

    /// The grabbable part of a fader: a slider's thumb.
    pub const HANDLE: Color32 = rgba(role::ink::PRIMARY);

    /// Section rules, meter frames, the canvas dot grid — structure at
    /// its quietest.
    pub const HAIRLINE: Color32 = rgba(role::line::SUBTLE);
    /// Button and chip outlines: an edge, so a row of controls reads as
    /// controls rather than as caption text.
    pub const EDGE: Color32 = rgba(role::line::STRONG);
    /// The edge of something you type or tick into (a field, a box), and
    /// a wire on the node canvas: anything that has to be found at 3:1.
    pub const CONTROL_EDGE: Color32 = rgba(role::line::CONTROL);
    /// Scale ticks on tracks and phase bars. vizz's own: a step brighter
    /// than an edge, because they sit on a raised track.
    pub const TICK: Color32 = rgba(palette::neutral::TONE_32);
    /// The hover rim: without it there is no way to tell a live control
    /// from a picture of one until you have already moved it. The edge
    /// firms up under the pointer, as a shared field's does.
    pub const HOVER_EDGE: Color32 = rgba(role::ink::TERTIARY);
    /// Keyboard focus: the ring, never a wash.
    pub const FOCUS: Color32 = rgba(role::FOCUS);
}

/// Instrument accents: the recurring non-state colours with fixed jobs.
pub mod accent {
    use super::tokens::Rgba;
    use super::{over, palette, rgba, role};
    use egui::Color32;

    /// Something other than the performer's hand is driving this: a
    /// modulator moving a parameter, the autopilot walking the grid.
    ///
    /// vizz's own meaning, in the one hue the shared grammar leaves free.
    /// Modulation used to be amber and the autopilot green, which in the
    /// grammar say "look at this" and "healthy"; neither is what they
    /// mean. They are one meaning — the tool is doing this on its own,
    /// and you can take it back — so they are one colour. 4.9:1 on a
    /// hovered control, 6.6:1 on the panel.
    pub const DRIVEN: Color32 = rgba(DRIVEN_RGBA);
    const DRIVEN_RGBA: Rgba = Rgba::rgb(0xb4, 0x8c, 0xff);
    /// The autopilot's bed: the driven colour's wash (at the shared
    /// `soft` strength of the other washes) on an empty slot.
    pub const DRIVEN_BED: Color32 = over(Rgba::new(0xb4, 0x8c, 0xff, 0x24), role::surface::INSET);
    /// What the autopilot has swept so far, under its words: the driven
    /// colour at 40% on the same slot, so primary text reads across the
    /// sweep (5.0:1) and the sweep still reads against the bed.
    pub const DRIVEN_FILL: Color32 = over(Rgba::new(0xb4, 0x8c, 0xff, 0x66), role::surface::INSET);

    /// Modulation — "something else is moving this".
    pub const MOD: Color32 = DRIVEN;
    /// A global (preset-exempt) parameter's marker: said by its "g",
    /// quiet in colour.
    pub const GLOBAL: Color32 = rgba(role::ink::SECONDARY);

    /// A fader's value fill: the accent's own deep step, so a row of
    /// faders reads as yours without sixteen bright bars lighting the
    /// room. 3:1 against the groove it sits in.
    pub const FILL: Color32 = rgba(palette::cyan::TONE_42);
    /// The brighter cap edge the eye finds faster than it judges a flat
    /// block's height: the accent itself.
    pub const FILL_BRIGHT: Color32 = rgba(role::accent::DEFAULT);

    /// Live signal meters: LFO outputs, audio sparks, the beat pulse. A
    /// meter is a reading, not a state, so it is neutral.
    pub const METER: Color32 = rgba(role::ink::SECONDARY);
    /// The meter's dim companion (history, the unaccented half).
    pub const METER_DIM: Color32 = rgba(role::line::STRONG);

    /// The master fader. It stays findable by being the one grey fader
    /// at the end of the first row: red is stop in the grammar, and a
    /// master is not an error.
    pub const MASTER: Color32 = rgba(palette::neutral::TONE_59);
    /// The master, under a hand.
    pub const MASTER_BRIGHT: Color32 = rgba(palette::neutral::TONE_71);
    pub const MASTER_INK: Color32 = rgba(role::ink::PRIMARY);

    /// A transition in flight — the pad being blended to. It is where you
    /// sent the grid, so it is yours: the accent, filling as it arrives.
    pub const ARRIVING: Color32 = rgba(role::accent::DEFAULT);

    /// The autopilot's own colour: the driven colour.
    pub const AUTO: Color32 = DRIVEN;
    pub const AUTO_BED: Color32 = DRIVEN_BED;

    /// A MIDI binding chip at rest — quiet enough that a fully mapped
    /// grid does not read as sixteen alarms.
    pub const BINDING: Color32 = rgba(role::ink::SECONDARY);

    /// Recording: the chip's fill, its ink, the idle bed and its ink.
    /// Forgetting a recording is how disks fill mid-set, so this family
    /// exists to be seen. vizz's own: red because a take in progress is
    /// a tally (the picture is going somewhere), which st2110 and
    /// facetrack keep red for the same reason.
    pub const REC: Color32 = rgba(role::bad::DEFAULT);
    pub const ON_REC: Color32 = rgba(role::bad::ON);
    pub const REC_BED: Color32 = over(role::bad::SOFT, role::surface::PANEL);
    pub const REC_INK: Color32 = rgba(role::bad::INK);

    /// Node-editor category hues: where a value comes from, what bends
    /// it, where it lands. Muted wayfinding on the node headers only,
    /// never on a state; they stay vizz's own.
    pub const NODE_SOURCE: Color32 = Color32::from_rgb(70, 120, 175);
    pub const NODE_OPERATOR: Color32 = Color32::from_rgb(150, 120, 60);
    pub const NODE_SINK: Color32 = Color32::from_rgb(70, 140, 100);
}

/// Feedback chrome: what success, failure and danger sit on.
///
/// Two families. Inline text feedback (`OK_TEXT`, `ERR_TEXT`) colours a
/// line in place; sheet feedback (`*_BED` + `ON_*`) is a floating surface
/// — notices, the quit prompt, the learn banner — red enough to be
/// found at a glance, dark enough not to strobe the room.
pub mod feedback {
    use super::{over, rgba, role};
    use egui::Color32;

    /// "saved", inline. Neutral: a thing that worked as expected needs no
    /// colour, and green is not "saved". Errors must never share this
    /// colour — that is how load failures went unnoticed on the canvas
    /// once — and now they cannot: only failure has a colour.
    pub const OK_TEXT: Color32 = rgba(role::ink::SECONDARY);
    /// "load failed", inline.
    pub const ERR_TEXT: Color32 = rgba(role::bad::INK);

    /// A confirmation notice's bed and ink: the plain floating surface.
    pub const OK_BED: Color32 = rgba(role::surface::OVERLAY);
    pub const ON_OK: Color32 = rgba(role::ink::PRIMARY);

    /// Danger sheets — error notices, the quit prompt: the stop wash on
    /// the floating surface, with an edge in [`DANGER_EDGE`].
    pub const DANGER_BED: Color32 = over(role::bad::SOFT, role::surface::OVERLAY);
    /// The edge round a danger sheet.
    pub const DANGER_EDGE: Color32 = rgba(role::bad::DEFAULT);
    /// The fill of an armed destructive button.
    pub const DANGER_FILL: Color32 = rgba(role::bad::DEFAULT);
    /// Ink on a danger sheet.
    pub const ON_DANGER: Color32 = rgba(role::ink::PRIMARY);
    pub const ON_DANGER_DIM: Color32 = rgba(role::ink::SECONDARY);
    /// Ink on [`DANGER_FILL`].
    pub const ON_DANGER_FILL: Color32 = rgba(role::bad::ON);

    /// The armed-learn banner's bed and ink (the outline is
    /// [`crate::state::LEARN`]).
    pub const LEARN_BED: Color32 = over(role::warn::SOFT, role::surface::OVERLAY);
    pub const ON_LEARN_BED: Color32 = rgba(role::ink::PRIMARY);
}

/// The type scale, by role. Sizes in points.
///
/// The shared text styles at the regular density where vizz has the
/// same job, and vizz's own below the shared floor where a chip has to
/// share a 40-point pad. Rules that travel with the scale: numbers that
/// change every frame (fps, bpm, values) wear a monospace face and pad
/// to fixed width, or the line reflows underneath the eye; section
/// headers are the shared label (tracked capitals); nothing on a stage
/// screen goes below `MICRO`, and `MICRO` only where a chip shares a
/// 40-point pad.
pub mod text {
    use super::tokens::text::regular as ds;

    /// Dense chips riding on another control (a pad's binding). vizz's
    /// own: below the shared floor, on 40-point pads only.
    pub const MICRO: f32 = 8.0;
    /// Slot numbers and other indices. vizz's own, as `MICRO`.
    pub const INDEX: f32 = 9.0;
    /// Section headers (drawn strong, tracked out): the shared label.
    pub const SECTION: f32 = ds::LABEL.size;
    /// Chips, hints, learn tags.
    pub const CAPTION: f32 = ds::LABEL.size;
    /// Control labels under faders: the shared caption.
    pub const LABEL: f32 = ds::CAPTION.size;
    /// Body: buttons, rows, most prose.
    pub const BODY: f32 = ds::BODY.size;
    /// Big touch targets — the preset row.
    pub const CONTROL: f32 = ds::PROSE.size;
    /// Full-screen moments: the quit prompt's headline.
    pub const BANNER: f32 = ds::HEADING.size;
}

/// Spacing, by role: the shared 4-point grid and its half step.
pub mod space {
    use super::tokens::space as ds;

    /// Inside a chip, between a glyph and its edge.
    pub const CHIP: f32 = ds::HALF;
    /// Between siblings in a dense row (grid pads, strip items).
    pub const GAP: f32 = ds::STEP_1;
    /// A control's inset from its container.
    pub const INSET: f32 = ds::STEP_2;
    /// Between sections of one screen.
    pub const SECTION: f32 = ds::STEP_3;
    /// A screen's outer padding.
    pub const PAD: f32 = ds::STEP_4;
}

/// Corner radii, by role: the shared compact density, a desk's.
pub mod radius {
    use super::tokens::{radius::compact as ds, space};

    /// The latch pip and other markers.
    pub const PIP: f32 = space::HAIRLINE;
    /// Small chips and meters.
    pub const CHIP: f32 = space::HALF;
    /// Buttons, pads — the default touchable.
    pub const CONTROL: f32 = ds::CONTROL;
    /// Fader tracks and other tall wells.
    pub const TRACK: f32 = ds::PANEL;
    /// Sheets: banners, prompts, notices.
    pub const SHEET: f32 = ds::OVERLAY;
}

/// Timing. Feedback has a clock, and the clock is part of the language.
///
/// The behaviour timers are the shared ones and never change with motion
/// settings; the transition durations come from [`motion::mode`].
pub mod motion {
    use super::tokens::{duration, motion, timing};
    use std::sync::OnceLock;
    use std::time::Duration;

    /// How long an armed destructive control stays armed. Long enough to
    /// mean it, short enough that a stray first click cannot ambush a
    /// press half a song later.
    pub const ARM_WINDOW: f64 = timing::ARM as f64 / 1000.0;

    /// A value that arrived from somewhere other than the performer's
    /// own hand, travelling to where it now is.
    ///
    /// The shared rule is that a live value never animates, and the
    /// number printed under the fader does not: it is always the value.
    /// The bar's drawn position glides this long, because a controller
    /// sweep or a recall reads as travel and a jump reads as a glitch. A
    /// hand on the fader itself gets none of this — under your own hand a
    /// fader must be exactly where you put it, with no lag to fight.
    pub const SETTLE: f32 = duration::FAST as f32 / 1000.0;

    /// A state settling: a rim colour under a pointer. The press.
    pub const TOUCH: f32 = duration::INSTANT as f32 / 1000.0;

    /// The learn rim's breath: insistent, never a strobe.
    pub const BREATH: f64 = motion::full::cycle::PULSE as f64 / 1000.0;

    /// Inline status: a success fades, because stale feedback next to a
    /// changed surface reads as a fresh result. A failure stays until the
    /// next result replaces it — one that disappears before anyone looked
    /// at it did not happen, as far as the room is concerned.
    pub const STATUS_TTL: f64 = timing::NOTICE as f64 / 1000.0;
    pub const STATUS_ERROR_TTL: f64 = f64::INFINITY;

    /// Corner notices: confirmations go on their own, errors stay until
    /// they are clicked away.
    pub const NOTICE_TTL: Duration = Duration::from_millis(timing::NOTICE as u64);
    pub const NOTICE_ERROR_TTL: Duration = Duration::MAX;

    /// How long the pointer rests before a hover hint shows.
    pub const TOOLTIP: f32 = timing::TOOLTIP as f32 / 1000.0;

    /// The transition durations and distances, full or reduced.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Mode {
        /// Seconds: press feedback.
        pub instant: f32,
        /// Seconds: a change in place; a small thing leaving.
        pub fast: f32,
        /// Seconds: a small thing arriving, a fold opening.
        pub base: f32,
        /// Seconds: a large thing arriving, a notice from its edge.
        pub slow: f32,
        /// Points a notice travels as it arrives.
        pub offset_medium: f32,
    }

    pub const FULL: Mode = Mode {
        instant: motion::full::duration::INSTANT as f32 / 1000.0,
        fast: motion::full::duration::FAST as f32 / 1000.0,
        base: motion::full::duration::BASE as f32 / 1000.0,
        slow: motion::full::duration::SLOW as f32 / 1000.0,
        offset_medium: motion::full::offset::MEDIUM,
    };

    /// Reduced motion removes movement, not meaning: nothing travels,
    /// and the fades that say something arrived stay, shorter.
    pub const REDUCED: Mode = Mode {
        instant: motion::reduced::duration::INSTANT as f32 / 1000.0,
        fast: motion::reduced::duration::FAST as f32 / 1000.0,
        base: motion::reduced::duration::BASE as f32 / 1000.0,
        slow: motion::reduced::duration::SLOW as f32 / 1000.0,
        offset_medium: motion::reduced::offset::MEDIUM,
    };

    /// The mode this machine asked for, read once.
    ///
    /// `VIZZ_MOTION=reduced` (or `full`) decides, for a show machine
    /// short of GPU or a screen recording; otherwise macOS's Reduce
    /// motion setting does; otherwise full.
    pub fn mode() -> Mode {
        static MODE: OnceLock<Mode> = OnceLock::new();
        *MODE.get_or_init(|| match std::env::var("VIZZ_MOTION").as_deref() {
            Ok("reduced") => REDUCED,
            Ok("full") => FULL,
            _ if os_reduces_motion() => REDUCED,
            _ => FULL,
        })
    }

    #[cfg(target_os = "macos")]
    fn os_reduces_motion() -> bool {
        std::process::Command::new("defaults")
            .args(["read", "com.apple.universalaccess", "reduceMotion"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
    }

    #[cfg(not(target_os = "macos"))]
    fn os_reduces_motion() -> bool {
        false
    }

    /// A small thing arriving: most of the way at once, then a settle.
    pub const ENTER: [f32; 4] = super::tokens::easing::ENTER;
    /// A large thing arriving, or one from an edge.
    pub const EMPHASIZED_ENTER: [f32; 4] = super::tokens::easing::EMPHASIZED_ENTER;

    /// Where a CSS-style cubic-bezier curve is at `x` (0 to 1).
    pub fn ease(curve: [f32; 4], x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let [x1, y1, x2, y2] = curve;
        let at = |a: f32, b: f32, t: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // Bisect for the t whose x is ours: monotone in x for every curve
        // the system ships, and exact enough to draw with in a few steps.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..20 {
            let mid = 0.5 * (lo + hi);
            if at(x1, x2, mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        at(y1, y2, 0.5 * (lo + hi))
    }
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
    /// reads source rather than trusting the type system. The values are
    /// the shared system's now, so a copy could be written in decimal or
    /// in the hex the system prints; both are caught.
    #[test]
    fn state_colours_are_never_restated_in_vizz_ui() {
        use egui::Color32;
        let banned: [(&str, Color32); 7] = [
            ("state::WARN / state::LEARN", super::state::WARN),
            ("state::LIVE", super::state::LIVE),
            ("state::ARMED / feedback::DANGER_FILL", super::state::ARMED),
            ("state::CURRENT", super::state::CURRENT),
            ("ink::PRIMARY", super::ink::PRIMARY),
            ("accent::DRIVEN", super::accent::DRIVEN),
            ("surface::BASE", super::surface::BASE),
        ];
        let ui_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vizz-ui/src");
        let mut checked = 0;
        for entry in std::fs::read_dir(&ui_src).expect("vizz-ui/src missing") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            checked += 1;
            let src = std::fs::read_to_string(&path).unwrap().to_lowercase();
            for (token, c) in banned {
                let (r, g, b) = (c.r(), c.g(), c.b());
                for literal in [
                    format!("from_rgb({r}, {g}, {b})"),
                    format!("from_rgb({r:#04x}, {g:#04x}, {b:#04x})"),
                ] {
                    assert!(
                        !src.contains(&literal),
                        "{} restates {token} as a literal ({literal}) — use the token",
                        path.display()
                    );
                }
            }
        }
        assert!(checked >= 5, "looked at {checked} files — wrong directory?");
    }

    /// Every colour here comes from the shared system, apart from the
    /// meanings that are vizz's own and say so.
    ///
    /// The point of taking the system's values is that they cannot drift
    /// from it; a literal added here for convenience would be the first
    /// step back to three ambers. A new local colour belongs in this list
    /// with its reason in its doc comment.
    #[test]
    fn colours_come_from_the_shared_tokens() {
        let src = include_str!("lib.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        let literals: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//") && (l.contains("from_rgb(") || l.contains("Rgba::rgb(") || l.contains("Rgba::new(")))
            .collect();
        let local = [
            "const DRIVEN_RGBA: Rgba = Rgba::rgb(0xb4, 0x8c, 0xff);",
            "pub const DRIVEN_BED: Color32 = over(Rgba::new(0xb4, 0x8c, 0xff, 0x24), role::surface::INSET);",
            "pub const DRIVEN_FILL: Color32 = over(Rgba::new(0xb4, 0x8c, 0xff, 0x66), role::surface::INSET);",
            "pub const NODE_SOURCE: Color32 = Color32::from_rgb(70, 120, 175);",
            "pub const NODE_OPERATOR: Color32 = Color32::from_rgb(150, 120, 60);",
            "pub const NODE_SINK: Color32 = Color32::from_rgb(70, 140, 100);",
            // `over` builds its result from the two tokens it is given.
            "Color32::from_rgb(",
        ];
        for l in literals {
            assert!(local.contains(&l), "a colour literal that is not a shared token: {l}");
        }
    }

    /// The tokens the screens read are the shared roles, not copies of
    /// them: spot-check the mapping the adoption plan names.
    #[test]
    fn names_point_at_the_shared_roles() {
        use super::rgba;
        use super::tokens::color::dark as role;
        assert_eq!(super::state::CURRENT, rgba(role::accent::DEFAULT));
        assert_eq!(super::state::LIVE, rgba(role::ok::DEFAULT));
        assert_eq!(super::state::WARN, rgba(role::warn::DEFAULT));
        assert_eq!(super::state::ARMED, rgba(role::bad::DEFAULT));
        assert_eq!(super::surface::FOCUS, rgba(role::FOCUS));
        assert_eq!(super::ink::FAINT, rgba(role::ink::DISABLED));
    }

    /// A flattened wash is the wash laid on the surface, to the nearest
    /// step: what a browser would have drawn.
    #[test]
    fn over_flattens_a_wash_onto_its_surface() {
        use super::tokens::Rgba;
        let red = Rgba::new(255, 0, 0, 51); // 20%
        let grey = Rgba::rgb(100, 100, 100);
        let c = super::over(red, grey);
        assert_eq!((c.r(), c.g(), c.b(), c.a()), (131, 80, 80, 255));
        assert_eq!(super::over(Rgba::rgb(9, 9, 9), grey), egui::Color32::from_rgb(9, 9, 9));
    }

    /// The curves start and end where they should and never run back.
    #[test]
    fn ease_runs_from_nothing_to_all_of_it() {
        use super::motion::{EMPHASIZED_ENTER, ENTER, ease};
        for curve in [ENTER, EMPHASIZED_ENTER] {
            assert!(ease(curve, 0.0).abs() < 1e-3);
            assert!((ease(curve, 1.0) - 1.0).abs() < 1e-3);
            let mut last = 0.0;
            for i in 0..=20 {
                let y = ease(curve, i as f32 / 20.0);
                assert!(y + 1e-4 >= last, "runs back at {i}");
                last = y;
            }
        }
        // Emphasized is most of the way there early.
        assert!(ease(EMPHASIZED_ENTER, 0.3) > 0.7);
    }
}
