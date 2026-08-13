//! Lighting the controller up to show what the app is doing.
//!
//! A grid controller with dark pads is a keyboard: you have to remember
//! which of thirty-two buttons holds anything, and the sequencer's
//! position is on a screen you are not looking at. Lit, it is the
//! instrument's own display — which pads are loaded, which one is
//! playing, and where the autopilot is heading next.
//!
//! Kept apart from the input path on purpose. Output is best-effort:
//! a device that will not take a note must never be able to stall the
//! frame, drop an incoming message, or turn a working controller into a
//! broken one.

use crate::profile::{Lights, Profile};

/// The pads on one bank, as the app wants them lit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BankState {
    /// Bit per slot: this pad holds something.
    pub loaded: u16,
    /// The slot playing now.
    pub playing: Option<u8>,
    /// Where the sequencer is heading, when it is running. Lit
    /// differently from `playing`, because "what is on screen" and
    /// "what is about to be" are different questions and a performer
    /// needs both — that is the whole reason to light a grid at all.
    pub next: Option<u8>,
}

/// Everything the controller should be showing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Surface {
    pub scenes: BankState,
    pub gravity: BankState,
}

impl BankState {
    pub fn is_loaded(&self, slot: usize) -> bool {
        slot < 16 && self.loaded & (1 << slot) != 0
    }

    pub fn set_loaded(&mut self, slot: usize, loaded: bool) {
        if slot >= 16 {
            return;
        }
        if loaded {
            self.loaded |= 1 << slot;
        } else {
            self.loaded &= !(1 << slot);
        }
    }

    /// What colour a slot should be, given a profile's palette.
    ///
    /// Playing beats next beats loaded. An empty pad is dark: a grid
    /// where everything glows tells you nothing, and the thing worth
    /// seeing across a stage is which few pads have something on them.
    fn velocity(&self, slot: usize, lights: &Lights) -> u8 {
        if self.playing == Some(slot as u8) {
            lights.playing
        } else if self.next == Some(slot as u8) {
            lights.next
        } else if self.is_loaded(slot) {
            lights.loaded
        } else {
            lights.off
        }
    }
}

/// The bytes that would take a controller from one surface to another.
///
/// A diff rather than a full refresh. Thirty-two notes at sixty frames a
/// second is two thousand messages a second down a bus shared with the
/// clock — enough to make the clock jitter, which would mean the
/// feedback feature degrading the thing it is reporting on.
pub fn diff(from: &Surface, to: &Surface, profile: &Profile) -> Vec<[u8; 3]> {
    let Some(lights) = &profile.lights else { return Vec::new() };
    let mut out = Vec::new();
    let mut bank = |old: &BankState, new: &BankState, note_of: fn(usize) -> Option<u8>| {
        for slot in 0..16 {
            let (was, now) = (old.velocity(slot, lights), new.velocity(slot, lights));
            if was == now {
                continue;
            }
            let Some(note) = note_of(slot) else { continue };
            // Note-on with the colour as velocity, which is how this
            // family of devices addresses its LEDs. Velocity zero is
            // off, and is a note-on rather than a note-off because a
            // note-off carries no colour and some firmware ignores it.
            out.push([0x90 | (lights.channel & 0x0F), note, now]);
        }
    };
    bank(&from.scenes, &to.scenes, lights.scene_note);
    bank(&from.gravity, &to.gravity, lights.gravity_note);
    out
}

/// Every pad dark, for handing the controller back on the way out.
///
/// Leaving a grid lit after quitting is leaving the room with the
/// lights on: the next thing that opens the device inherits a display
/// that means nothing and cannot be cleared from the front panel.
pub fn blackout(profile: &Profile) -> Vec<[u8; 3]> {
    let Some(lights) = &profile.lights else { return Vec::new() };
    let mut out = Vec::new();
    for slot in 0..16 {
        for note_of in [lights.scene_note, lights.gravity_note] {
            if let Some(note) = note_of(slot) {
                out.push([0x90 | (lights.channel & 0x0F), note, lights.off]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

    fn apc() -> &'static Profile {
        profile::for_port("APC40 mkII").expect("no APC profile")
    }

    /// A surface that has not changed sends nothing.
    ///
    /// This is the property the whole design rests on: the sender runs
    /// every frame, and without it the bus carries two thousand
    /// messages a second beside the clock it is trying not to disturb.
    #[test]
    fn an_unchanged_surface_sends_nothing() {
        let mut s = Surface::default();
        s.scenes.set_loaded(0, true);
        s.scenes.playing = Some(0);
        s.gravity.set_loaded(4, true);
        assert!(diff(&s, &s, apc()).is_empty(), "an idle frame sent traffic");
    }

    /// Only what changed is sent, and it is sent to the right pad.
    #[test]
    fn firing_a_scene_relights_only_the_two_pads_involved() {
        let mut before = Surface::default();
        before.scenes.set_loaded(0, true);
        before.scenes.set_loaded(1, true);
        before.scenes.playing = Some(0);
        let mut after = before;
        after.scenes.playing = Some(1);

        let msgs = diff(&before, &after, apc());
        assert_eq!(msgs.len(), 2, "expected two pads to change: {msgs:?}");
        let lights = apc().lights.as_ref().unwrap();
        let pad0 = (lights.scene_note)(0).unwrap();
        let pad1 = (lights.scene_note)(1).unwrap();
        // The one that stopped goes back to loaded, the one that started
        // goes to playing.
        assert!(
            msgs.contains(&[0x90, pad0, lights.loaded]),
            "the old scene did not go back to loaded: {msgs:?}"
        );
        assert!(
            msgs.contains(&[0x90, pad1, lights.playing]),
            "the new scene did not light: {msgs:?}"
        );
    }

    /// Where the sequencer is heading is lit differently from where it
    /// is — which is the point of lighting it at all.
    #[test]
    fn the_next_step_is_a_different_colour_from_the_playing_one() {
        let mut s = Surface::default();
        for slot in 0..4 {
            s.scenes.set_loaded(slot, true);
        }
        s.scenes.playing = Some(1);
        s.scenes.next = Some(2);

        let msgs = diff(&Surface::default(), &s, apc());
        let lights = apc().lights.as_ref().unwrap();
        let at = |slot: usize| {
            let note = (lights.scene_note)(slot).unwrap();
            msgs.iter().find(|m| m[1] == note).map(|m| m[2])
        };
        assert_eq!(at(1), Some(lights.playing));
        assert_eq!(at(2), Some(lights.next));
        assert_eq!(at(0), Some(lights.loaded));
        assert_ne!(lights.playing, lights.next, "playing and next are the same colour");
        assert_ne!(lights.loaded, lights.playing);
        // An empty pad is dark, so it sends nothing against a dark start.
        assert_eq!(at(9), None, "an empty pad was lit");
    }

    /// Clearing a pad takes its light with it.
    #[test]
    fn emptying_a_pad_puts_it_out() {
        let mut before = Surface::default();
        before.gravity.set_loaded(3, true);
        let after = Surface::default();

        let lights = apc().lights.as_ref().unwrap();
        let note = (lights.gravity_note)(3).unwrap();
        assert_eq!(
            diff(&before, &after, apc()),
            vec![[0x90, note, lights.off]],
            "clearing a pad left it lit"
        );
    }

    /// Quitting hands the device back dark.
    #[test]
    fn blackout_covers_every_pad_the_profile_uses() {
        let msgs = blackout(apc());
        assert_eq!(msgs.len(), 32, "blackout missed pads: {}", msgs.len());
        assert!(msgs.iter().all(|m| m[2] == 0), "blackout sent a colour");
    }
}
