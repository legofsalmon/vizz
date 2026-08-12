//! Default mappings for controllers vizz recognises by name.
//!
//! Learn works and is the general answer, but "plug it in and the grid
//! is the grid" is a different quality of experience from forty learn
//! gestures done one at a time in a dark room. A profile is what a
//! controller *would* have been mapped to by somebody who knew the
//! layout, applied the moment it is plugged in.
//!
//! Applied only to bindings the user does not already have. A profile is
//! a starting point, never an opinion that overrules one — plugging a
//! controller back in must not silently undo an evening's mapping.

use crate::mapping::{Binding, MidiMap, Source};

/// A controller vizz knows the layout of.
pub struct Profile {
    /// What the profile is called, for the log and the panel.
    pub name: &'static str,
    /// Whether a port name is this device. Substring rather than exact:
    /// hosts decorate port names ("APC40 mkII", "APC40 mkII Port 1",
    /// "2- APC40 mkII"), and the decoration is not ours to predict.
    pub port: &'static str,
    /// The bindings it ships with.
    pub bindings: fn() -> Vec<Binding>,
    /// How to light it up, when it has lights.
    pub lights: Option<Lights>,
}

/// Where a device's lit pads live, so feedback can be written without
/// the sender knowing which controller it is talking to.
///
/// Notes rather than a general command language: every device this is
/// likely to grow to speaks "note on, velocity is the colour" for its
/// pads, and a richer abstraction with one implementation is a guess
/// about the second one.
pub struct Lights {
    /// Channel every pad note is sent on.
    pub channel: u8,
    /// The note for scene pad `slot` (0..16), if the device has one.
    pub scene_note: fn(usize) -> Option<u8>,
    /// The note for gravity pad `slot` (0..16).
    pub gravity_note: fn(usize) -> Option<u8>,
    /// Velocity for a pad holding something, currently playing, and
    /// empty. Velocities are colours on these devices.
    pub loaded: u8,
    pub playing: u8,
    pub next: u8,
    pub off: u8,
}

/// The shipped profiles.
pub const PROFILES: &[Profile] = &[APC40_MK2];

/// Which profile, if any, matches a port name.
pub fn for_port(port_name: &str) -> Option<&'static Profile> {
    let lower = port_name.to_ascii_lowercase();
    PROFILES
        .iter()
        .find(|p| lower.contains(&p.port.to_ascii_lowercase()))
}

/// Apply a profile's bindings to a map, keeping everything already
/// there.
///
/// "Already there" means *the parameter is already reachable* — by any
/// control, not just the one the profile wants. Checking only whether
/// the profile's own source is free would let a profile bind pad 3 to a
/// scene the user had deliberately put on a different button, and they
/// would then have two.
///
/// Returns how many bindings were added, so the caller can say whether
/// anything happened.
pub fn apply(map: &mut MidiMap, profile: &Profile) -> usize {
    let mut added = 0;
    for b in (profile.bindings)() {
        let taken = match b.value {
            Some(v) => map.source_for_value(&b.param, v).is_some(),
            None => map.source_for(&b.param).is_some(),
        };
        // And never steal a control the user has pointed somewhere else.
        if taken || map.param_for(&b.source).is_some() {
            continue;
        }
        match b.value {
            Some(v) => map.bind_value(b.source, b.param, v),
            None => map.bind(b.source, b.param),
        }
        added += 1;
    }
    added
}

// --- APC40 mkII -----------------------------------------------------
//
// The 8x5 clip-launch grid sends note-on per pad on one channel, with
// the notes running left to right along each row and the rows running
// bottom to top: note 0 is the bottom-left pad, note 39 the top-right.
//
// One constant decides that orientation, and it is the one thing here
// most likely to be upside down on a device nobody testing this owns.
// It is written as a function of a row counted *from the top* so that
// flipping it is a single edit rather than a rewrite of four tables —
// and so that if it is wrong, the failure is the grid appearing on the
// other two rows rather than nothing working.

/// Notes per row on the clip grid.
const APC_COLS: usize = 8;
/// Rows on the clip grid.
const APC_ROWS: usize = 5;

/// The note for a pad, addressed by row from the *top* (0 = top row)
/// and column from the left.
const fn apc_note(row_from_top: usize, col: usize) -> Option<u8> {
    if row_from_top >= APC_ROWS || col >= APC_COLS {
        return None;
    }
    // Rows run bottom-to-top in note order, so the top row is the
    // highest block.
    let row_from_bottom = APC_ROWS - 1 - row_from_top;
    Some((row_from_bottom * APC_COLS + col) as u8)
}

/// Sixteen pads laid across two rows of eight, starting at `top_row`.
const fn apc_pad(slot: usize, top_row: usize) -> Option<u8> {
    if slot >= 16 {
        return None;
    }
    apc_note(top_row + slot / APC_COLS, slot % APC_COLS)
}

/// Gravity lives on the top two rows, scenes on the bottom two, with
/// the middle row left alone.
///
/// Deliberately not four contiguous rows: the empty row between them is
/// what stops a hand reaching for scene 1 and landing on gravity 16.
/// On a grid you find by feel, a gap is a landmark.
fn apc_gravity_note(slot: usize) -> Option<u8> {
    apc_pad(slot, 0)
}

fn apc_scene_note(slot: usize) -> Option<u8> {
    apc_pad(slot, 3)
}

/// The channel the clip grid speaks on.
const APC_CH: u8 = 0;

/// Colours, as velocities into the device's palette.
///
/// Conservative picks: these are the entries whose meaning is stable
/// across the whole family, and getting a hue slightly wrong costs a
/// shade while getting the *behaviour* wrong costs the feature.
const APC_OFF: u8 = 0;
const APC_DIM: u8 = 1;
const APC_GREEN: u8 = 21;
const APC_AMBER: u8 = 9;

const APC40_MK2: Profile = Profile {
    name: "Akai APC40 mkII",
    port: "APC40",
    bindings: apc40_mk2_bindings,
    lights: Some(Lights {
        channel: APC_CH,
        scene_note: apc_scene_note,
        gravity_note: apc_gravity_note,
        loaded: APC_DIM,
        playing: APC_GREEN,
        next: APC_AMBER,
        off: APC_OFF,
    }),
};

/// The nine faders, as the device sends them.
///
/// Track faders are CC 7 on channels 1-8; the master is CC 14 on
/// channel 1. Both are documented parts of the mkII's protocol and are
/// the two things every host maps identically, which makes them the
/// safest thing to ship a default for.
const APC_TRACK_FADER_CC: u8 = 7;
const APC_MASTER_FADER_CC: u8 = 14;

fn apc40_mk2_bindings() -> Vec<Binding> {
    let mut out = Vec::new();
    // The pads. `value` rather than a range, because these address a
    // *slot*: a button is on or off, so a range binding could only ever
    // reach the top of it. See `Binding::value`.
    for slot in 0..16 {
        if let Some(note) = apc_scene_note(slot) {
            out.push(Binding {
                source: Source::Note { channel: APC_CH, note },
                param: "/scene/fire".into(),
                value: Some(slot as f32 + 1.0),
            });
        }
        if let Some(note) = apc_gravity_note(slot) {
            out.push(Binding {
                source: Source::Note { channel: APC_CH, note },
                param: "/gravity/fire".into(),
                value: Some(slot as f32 + 1.0),
            });
        }
    }
    // The master fader goes to the master dim, which is the one control
    // whose position on a desk is never in doubt.
    out.push(Binding {
        source: Source::ControlChange { channel: 0, controller: APC_MASTER_FADER_CC },
        param: "/master/dim".into(),
        value: None,
    });
    // The eight track faders take the eight parameters the shipped
    // performance layout puts on its first row — so the hardware under
    // your left hand and the faders on screen are the same eight things
    // in the same order.
    const TRACK_PARAMS: [&str; 8] = [
        "/particles/size",
        "/particles/speed",
        "/particles/count",
        "/shape/mode",
        "/shape/morph",
        "/fx/trail",
        "/fx/glow",
        "/fx/mirror",
    ];
    for (i, param) in TRACK_PARAMS.iter().enumerate() {
        out.push(Binding {
            source: Source::ControlChange {
                channel: i as u8,
                controller: APC_TRACK_FADER_CC,
            },
            param: (*param).into(),
            value: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device is recognised however the host decorates its name.
    #[test]
    fn the_apc_is_found_under_the_names_hosts_give_it() {
        for name in [
            "APC40 mkII",
            "APC40 mkII Port 1",
            "2- APC40 mkII",
            "apc40 mkii",
        ] {
            assert!(for_port(name).is_some(), "not recognised: {name}");
        }
        assert!(for_port("Launchpad Mini").is_none());
        assert!(for_port("IAC Driver Bus 1").is_none());
    }

    /// Every pad on the grid is a distinct note, and the two banks do
    /// not overlap.
    ///
    /// A collision here would be two of the app's pads firing from one
    /// button — which looks like a broken controller and is impossible
    /// to diagnose from the front.
    #[test]
    fn the_grid_maps_to_thirty_two_distinct_notes() {
        let mut seen = Vec::new();
        for slot in 0..16 {
            for note in [apc_scene_note(slot), apc_gravity_note(slot)] {
                let note = note.unwrap_or_else(|| panic!("slot {slot} has no note"));
                assert!(note < 40, "note {note} is off the clip grid");
                assert!(!seen.contains(&note), "note {note} is used twice");
                seen.push(note);
            }
        }
        assert_eq!(seen.len(), 32);
        // And the middle row is untouched, which is the landmark the
        // hand navigates by.
        let middle: Vec<u8> = (0..8).map(|c| apc_note(2, c).unwrap()).collect();
        for note in middle {
            assert!(!seen.contains(&note), "the middle row was used after all");
        }
    }

    /// Gravity sits above scenes, reading top to bottom, left to right.
    ///
    /// The user asked for gravity on the top two rows and scenes on the
    /// bottom two. Written out as the actual notes rather than as the
    /// same arithmetic the code uses — restating the formula would pass
    /// against any orientation, including upside down.
    #[test]
    fn gravity_is_on_the_top_two_rows_and_scenes_on_the_bottom_two() {
        // Notes run bottom-to-top, so the top row is 32..40 and the
        // bottom is 0..8.
        assert_eq!(apc_gravity_note(0), Some(32), "gravity 1 is not top-left");
        assert_eq!(apc_gravity_note(7), Some(39), "gravity 8 is not top-right");
        assert_eq!(apc_gravity_note(8), Some(24), "gravity 9 did not wrap to the second row");
        assert_eq!(apc_gravity_note(15), Some(31));

        assert_eq!(apc_scene_note(0), Some(8), "scene 1 is not on the fourth row");
        assert_eq!(apc_scene_note(7), Some(15));
        assert_eq!(apc_scene_note(8), Some(0), "scene 9 is not on the bottom row");
        assert_eq!(apc_scene_note(15), Some(7));

        assert_eq!(apc_pad(16, 0), None, "a seventeenth pad exists");
    }

    /// A profile fills in what is missing and touches nothing else.
    #[test]
    fn applying_a_profile_keeps_every_binding_the_user_already_made() {
        let mut map = MidiMap::default();
        // The user has already put scene 1 somewhere of their own, and
        // pointed the APC's own scene-2 pad at something else entirely.
        map.bind_value(
            Source::Note { channel: 5, note: 60 },
            "/scene/fire",
            1.0,
        );
        let apc_scene_2 = Source::Note { channel: APC_CH, note: apc_scene_note(1).unwrap() };
        map.bind(apc_scene_2, "/fx/glow");

        let profile = for_port("APC40 mkII").unwrap();
        let added = apply(&mut map, profile);
        assert!(added > 0, "the profile added nothing at all");

        // Their scene 1 is untouched, and the profile did not add a
        // second control for it.
        assert_eq!(
            map.source_for_value("/scene/fire", 1.0),
            Some(Source::Note { channel: 5, note: 60 }),
            "the profile overrode a binding the user had made"
        );
        // Their re-purposed pad still does what they said.
        assert_eq!(map.param_for(&apc_scene_2), Some("/fx/glow"));
        // And the rest of the grid did land.
        assert!(map.source_for_value("/scene/fire", 3.0).is_some());
        assert!(map.source_for_value("/gravity/fire", 1.0).is_some());
        assert!(map.source_for("/master/dim").is_some());

        // Applying twice is a no-op: plugging the controller in again
        // must not double anything.
        assert_eq!(apply(&mut map, profile), 0, "a second apply added more bindings");
    }

    /// Every binding the profile ships is reachable and distinct.
    #[test]
    fn the_shipped_bindings_do_not_collide() {
        for p in PROFILES {
            let bindings = (p.bindings)();
            let mut sources = Vec::new();
            for b in &bindings {
                assert!(
                    !sources.contains(&b.source),
                    "{}: {:?} is bound twice",
                    p.name,
                    b.source
                );
                sources.push(b.source);
            }
            assert!(!bindings.is_empty(), "{} ships no bindings", p.name);
        }
    }
}
