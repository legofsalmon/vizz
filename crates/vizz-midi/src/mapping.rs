//! Bindings from MIDI sources to parameters, and the logic that turns an
//! incoming event into a normalised parameter value.
//!
//! Kept separate from the device layer so all of it is testable without a
//! controller attached — which is the only way this gets verified in CI.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::message::MidiEvent;

/// What a binding listens to. Channels are 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A continuous controller. If `controller` is 0..=31 the matching
    /// LSB controller (`controller + 32`) is used automatically for
    /// 14-bit resolution when the device sends it.
    ControlChange { channel: u8, controller: u8 },
    /// Momentary: velocity while held, 0 on release.
    Note { channel: u8, note: u8 },
    PitchBend { channel: u8 },
}

impl Source {
    /// Short human-readable form for the GUI, using 1-based channels to
    /// match what controllers print on their own displays.
    pub fn label(&self) -> String {
        match self {
            Self::ControlChange { channel, controller } => {
                format!("ch{} cc{controller}", channel + 1)
            }
            Self::Note { channel, note } => format!("ch{} note{note}", channel + 1),
            Self::PitchBend { channel } => format!("ch{} bend", channel + 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub source: Source,
    /// OSC-style parameter address, e.g. `/particles/hue`.
    pub param: String,
}

/// The saved mapping set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiMap {
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

impl MidiMap {
    /// Bind `source` to `param`, replacing any existing binding for that
    /// source. One physical control driving two parameters at once is
    /// almost always a mis-learn, not an intent.
    pub fn bind(&mut self, source: Source, param: impl Into<String>) {
        let param = param.into();
        self.bindings.retain(|b| b.source != source);
        self.bindings.push(Binding { source, param });
    }

    pub fn unbind_param(&mut self, param: &str) {
        self.bindings.retain(|b| b.param != param);
    }

    pub fn param_for(&self, source: &Source) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| &b.source == source)
            .map(|b| b.param.as_str())
    }

    /// The source bound to `param`, for display next to its slider.
    pub fn source_for(&self, param: &str) -> Option<Source> {
        self.bindings.iter().find(|b| b.param == param).map(|b| b.source)
    }
}

/// Turns events into `(param, normalised value)` updates.
///
/// Holds the small amount of state MIDI requires: the high byte of any
/// 14-bit controller pair seen so far.
#[derive(Default)]
pub struct Dispatcher {
    /// (channel, msb controller) -> last MSB value.
    msb: HashMap<(u8, u8), u8>,
}

impl Dispatcher {
    /// Resolve an event against `map`. Returns the parameter address and a
    /// 0..=1 value, or `None` if nothing is bound to it.
    pub fn resolve(&mut self, event: MidiEvent, map: &MidiMap) -> Option<(String, f32)> {
        match event {
            MidiEvent::ControlChange { channel, controller, value } => {
                // Controllers 32..63 are the LSB halves of 0..31. Combine
                // them with the stored MSB for 14-bit resolution; a device
                // that only sends the MSB still works at 7 bits.
                if (32..64).contains(&controller) {
                    let msb_cc = controller - 32;
                    let msb = *self.msb.get(&(channel, msb_cc))?;
                    let source = Source::ControlChange { channel, controller: msb_cc };
                    let param = map.param_for(&source)?.to_owned();
                    let combined = ((msb as u16) << 7) | value as u16;
                    return Some((param, combined as f32 / 16383.0));
                }
                if controller < 32 {
                    self.msb.insert((channel, controller), value);
                }
                let source = Source::ControlChange { channel, controller };
                let param = map.param_for(&source)?.to_owned();
                Some((param, value as f32 / 127.0))
            }
            MidiEvent::NoteOn { channel, note, velocity } => {
                let param = map.param_for(&Source::Note { channel, note })?.to_owned();
                Some((param, velocity as f32 / 127.0))
            }
            MidiEvent::NoteOff { channel, note } => {
                let param = map.param_for(&Source::Note { channel, note })?.to_owned();
                Some((param, 0.0))
            }
            MidiEvent::PitchBend { channel, value } => {
                let param = map.param_for(&Source::PitchBend { channel })?.to_owned();
                Some((param, value as f32 / 16383.0))
            }
        }
    }

    /// The source an event came from, for MIDI-learn. Learning binds the
    /// MSB controller of a 14-bit pair, never the LSB — otherwise the
    /// binding captures only the fine adjustment.
    pub fn learn_source(event: MidiEvent) -> Source {
        match event {
            MidiEvent::ControlChange { channel, controller, .. } => Source::ControlChange {
                channel,
                controller: if (32..64).contains(&controller) { controller - 32 } else { controller },
            },
            MidiEvent::NoteOn { channel, note, .. } | MidiEvent::NoteOff { channel, note } => {
                Source::Note { channel, note }
            }
            MidiEvent::PitchBend { channel, .. } => Source::PitchBend { channel },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(channel: u8, controller: u8, value: u8) -> MidiEvent {
        MidiEvent::ControlChange { channel, controller, value }
    }

    #[test]
    fn seven_bit_cc_maps_to_full_range() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
        let mut d = Dispatcher::default();

        assert_eq!(d.resolve(cc(0, 7, 0), &map), Some(("/master/dim".into(), 0.0)));
        assert_eq!(d.resolve(cc(0, 7, 127), &map), Some(("/master/dim".into(), 1.0)));
        let (_, mid) = d.resolve(cc(0, 7, 64), &map).unwrap();
        assert!((mid - 0.504).abs() < 0.01, "got {mid}");
    }

    #[test]
    fn lsb_after_msb_gives_fourteen_bit_resolution() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 1 }, "/particles/hue");
        let mut d = Dispatcher::default();

        // MSB alone still works (7-bit devices).
        let (_, coarse) = d.resolve(cc(0, 1, 64), &map).unwrap();
        assert!((coarse - 0.504).abs() < 0.01);

        // LSB for the same controller refines it: (64<<7)|32 = 8224.
        let (param, fine) = d.resolve(cc(0, 33, 32), &map).unwrap();
        assert_eq!(param, "/particles/hue");
        assert!((fine - 8224.0 / 16383.0).abs() < 1e-4, "got {fine}");
        // Finer than 7-bit steps, which is the whole point.
        assert_ne!(fine, coarse);
    }

    #[test]
    fn lsb_without_a_preceding_msb_is_ignored() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 1 }, "/particles/hue");
        let mut d = Dispatcher::default();
        // Acting on a lone LSB would jump the parameter to near zero.
        assert_eq!(d.resolve(cc(0, 33, 100), &map), None);
    }

    #[test]
    fn msb_cache_is_per_channel() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 1, controller: 1 }, "/a");
        let mut d = Dispatcher::default();
        // MSB on channel 0 must not satisfy an LSB on channel 1.
        d.resolve(cc(0, 1, 127), &map);
        assert_eq!(d.resolve(cc(1, 33, 10), &map), None);
    }

    #[test]
    fn notes_are_momentary() {
        let mut map = MidiMap::default();
        map.bind(Source::Note { channel: 0, note: 60 }, "/master/dim");
        let mut d = Dispatcher::default();
        let (_, on) = d
            .resolve(MidiEvent::NoteOn { channel: 0, note: 60, velocity: 127 }, &map)
            .unwrap();
        assert_eq!(on, 1.0);
        let (_, off) = d.resolve(MidiEvent::NoteOff { channel: 0, note: 60 }, &map).unwrap();
        assert_eq!(off, 0.0);
    }

    #[test]
    fn pitch_bend_spans_the_range() {
        let mut map = MidiMap::default();
        map.bind(Source::PitchBend { channel: 0 }, "/particles/speed");
        let mut d = Dispatcher::default();
        let (_, centre) = d.resolve(MidiEvent::PitchBend { channel: 0, value: 8192 }, &map).unwrap();
        assert!((centre - 0.5).abs() < 0.001, "got {centre}");
        let (_, top) = d.resolve(MidiEvent::PitchBend { channel: 0, value: 16383 }, &map).unwrap();
        assert_eq!(top, 1.0);
    }

    #[test]
    fn unbound_sources_resolve_to_nothing() {
        let map = MidiMap::default();
        let mut d = Dispatcher::default();
        assert_eq!(d.resolve(cc(0, 7, 64), &map), None);
    }

    #[test]
    fn binding_a_source_replaces_its_previous_target() {
        let mut map = MidiMap::default();
        let src = Source::ControlChange { channel: 0, controller: 7 };
        map.bind(src, "/a");
        map.bind(src, "/b");
        assert_eq!(map.bindings.len(), 1);
        assert_eq!(map.param_for(&src), Some("/b"));
    }

    #[test]
    fn learn_binds_the_msb_of_a_fourteen_bit_pair() {
        // Turning a 14-bit knob emits both halves; binding the LSB would
        // capture only the fine adjustment and feel broken.
        assert_eq!(
            Dispatcher::learn_source(cc(2, 33, 5)),
            Source::ControlChange { channel: 2, controller: 1 }
        );
        assert_eq!(
            Dispatcher::learn_source(cc(2, 1, 5)),
            Source::ControlChange { channel: 2, controller: 1 }
        );
    }

    #[test]
    fn map_round_trips_through_json() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
        map.bind(Source::Note { channel: 9, note: 36 }, "/particles/speed");
        map.bind(Source::PitchBend { channel: 3 }, "/particles/hue");
        let json = serde_json::to_string_pretty(&map).unwrap();
        let back: MidiMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map, back);
    }

    #[test]
    fn empty_and_partial_json_load_cleanly() {
        // A hand-edited or older file must not abort startup.
        let empty: MidiMap = serde_json::from_str("{}").unwrap();
        assert!(empty.bindings.is_empty());
    }
}
