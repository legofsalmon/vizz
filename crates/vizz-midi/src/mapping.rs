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

/// `Eq` is deliberately absent: `value` is a float, and two bindings being
/// "equal" is only ever asked about their source, which is compared
/// directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub source: Source,
    /// OSC-style parameter address, e.g. `/particles/hue`.
    pub param: String,
    /// A fixed value a press sends, instead of the control's position
    /// spread across the parameter's range.
    ///
    /// Some parameters are not ranges at all — `/scene/fire` and
    /// `/preset/recall` address a *slot*, and the interesting values are
    /// integers a few apart. Spreading a control across them is wrong in
    /// both directions: a button is fully on or fully off, so it could only
    /// ever reach the top of the range, and a fader sweeping from slot 1 to
    /// slot 16 fires all sixteen on the way past.
    ///
    /// So a binding may name the value instead. The control decides
    /// *whether*, the binding decides *what*, and sixteen buttons can
    /// address sixteen pads.
    ///
    /// Absent by default, so a `midi.json` written before this existed
    /// loads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f32>,
}

/// What an event should do to the parameter it resolved to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Update {
    /// A position, 0..=1, to be spread across the parameter's range. A
    /// fader, a knob, a bend.
    Range(f32),
    /// The parameter's own value, used as it stands. A button that
    /// addresses one slot.
    Absolute(f32),
}

/// The saved mapping set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MidiMap {
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

impl MidiMap {
    /// Bind `source` to `param`, replacing any existing binding for that
    /// source. One physical control driving two parameters at once is
    /// almost always a mis-learn, not an intent.
    pub fn bind(&mut self, source: Source, param: impl Into<String>) {
        self.insert(source, param.into(), None);
    }

    /// Bind `source` as a trigger for one `value` of `param`. See
    /// [`Binding::value`].
    pub fn bind_value(&mut self, source: Source, param: impl Into<String>, value: f32) {
        self.insert(source, param.into(), Some(value));
    }

    fn insert(&mut self, source: Source, param: String, value: Option<f32>) {
        self.bindings.retain(|b| b.source != source);
        self.bindings.push(Binding { source, param, value });
    }

    pub fn unbind_param(&mut self, param: &str) {
        self.bindings.retain(|b| b.param != param);
    }

    /// Drop just the trigger for one value, leaving the other slots of the
    /// same parameter alone. Sixteen pads share `/scene/fire`, so
    /// [`unbind_param`](Self::unbind_param) would clear the whole grid's
    /// mapping to unmap one pad.
    pub fn unbind_value(&mut self, param: &str, value: f32) {
        self.bindings
            .retain(|b| b.param != param || b.value != Some(value));
    }

    pub fn param_for(&self, source: &Source) -> Option<&str> {
        self.binding(source).map(|b| b.param.as_str())
    }

    pub fn binding(&self, source: &Source) -> Option<&Binding> {
        self.bindings.iter().find(|b| &b.source == source)
    }

    /// The source bound to `param`, for display next to its slider.
    ///
    /// Trigger bindings are skipped: a parameter can have any number of
    /// them, so showing one beside a slider would name an arbitrary member
    /// of a set and imply the slider itself was mapped. Those are shown on
    /// the pads they address instead, by
    /// [`source_for_value`](Self::source_for_value).
    pub fn source_for(&self, param: &str) -> Option<Source> {
        self.bindings
            .iter()
            .find(|b| b.param == param && b.value.is_none())
            .map(|b| b.source)
    }

    /// The source that triggers one particular value of `param`.
    pub fn source_for_value(&self, param: &str, value: f32) -> Option<Source> {
        self.bindings
            .iter()
            .find(|b| b.param == param && b.value == Some(value))
            .map(|b| b.source)
    }
}

/// Is this event a press, for a trigger binding?
///
/// Notes and controllers answer this differently and it is not a detail.
/// A pad is velocity sensitive, so a gentle hit sends velocity 20 —
/// thresholding it at halfway would make soft playing do nothing, which
/// reads as a dead pad rather than as a threshold. `parse` has already
/// turned note-on-with-zero-velocity into a note-off, so any `NoteOn`
/// reaching here is a real press.
///
/// A controller sending its pads as CC has no such nuance: it sends 127
/// and 0, and halfway is both the obvious split and what every other host
/// uses.
fn pressed(event: MidiEvent, position: f32) -> bool {
    match event {
        MidiEvent::NoteOn { .. } => true,
        MidiEvent::NoteOff { .. } => false,
        _ => position >= 0.5,
    }
}

/// Turns events into `(param, update)` pairs.
///
/// Holds the small amount of state MIDI requires: the high byte of any
/// 14-bit controller pair seen so far.
#[derive(Default)]
pub struct Dispatcher {
    /// (channel, msb controller) -> last MSB value.
    msb: HashMap<(u8, u8), u8>,
}

impl Dispatcher {
    /// Resolve an event against `map`. Returns the parameter address and
    /// what to do to it, or `None` if nothing is bound to it.
    pub fn resolve(&mut self, event: MidiEvent, map: &MidiMap) -> Option<(String, Update)> {
        let (binding, position) = self.position(event, map)?;
        let update = match binding.value {
            None => Update::Range(position),
            // A trigger. Held means the value, released means the bottom of
            // the range — which for a slot parameter is "nothing selected".
            //
            // Releasing matters as much as pressing: firing is edge
            // triggered, so a button that only ever sent slot 5 would fire
            // once and then be dead until something else moved the
            // parameter. Returning to rest is what makes the second press
            // work.
            Some(v) if pressed(event, position) => Update::Absolute(v),
            Some(_) => Update::Range(0.0),
        };
        Some((binding.param.clone(), update))
    }

    /// Where the control sits, 0..=1, and what it is bound to.
    fn position<'m>(
        &mut self,
        event: MidiEvent,
        map: &'m MidiMap,
    ) -> Option<(&'m Binding, f32)> {
        match event {
            MidiEvent::ControlChange { channel, controller, value } => {
                // Controllers 32..63 are the LSB halves of 0..31. Combine
                // them with the stored MSB for 14-bit resolution; a device
                // that only sends the MSB still works at 7 bits.
                if (32..64).contains(&controller) {
                    let msb_cc = controller - 32;
                    let msb = *self.msb.get(&(channel, msb_cc))?;
                    let source = Source::ControlChange { channel, controller: msb_cc };
                    let binding = map.binding(&source)?;
                    let combined = ((msb as u16) << 7) | value as u16;
                    return Some((binding, combined as f32 / 16383.0));
                }
                if controller < 32 {
                    self.msb.insert((channel, controller), value);
                }
                let source = Source::ControlChange { channel, controller };
                Some((map.binding(&source)?, value as f32 / 127.0))
            }
            MidiEvent::NoteOn { channel, note, velocity } => {
                let binding = map.binding(&Source::Note { channel, note })?;
                Some((binding, velocity as f32 / 127.0))
            }
            MidiEvent::NoteOff { channel, note } => {
                Some((map.binding(&Source::Note { channel, note })?, 0.0))
            }
            MidiEvent::PitchBend { channel, value } => {
                let binding = map.binding(&Source::PitchBend { channel })?;
                Some((binding, value as f32 / 16383.0))
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

    /// The 0..=1 position of a resolved update, for the tests that are
    /// about the wire format rather than about triggers.
    fn range(update: Update) -> f32 {
        match update {
            Update::Range(t) => t,
            Update::Absolute(v) => panic!("expected a range, got the fixed value {v}"),
        }
    }

    #[test]
    fn seven_bit_cc_maps_to_full_range() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
        let mut d = Dispatcher::default();

        assert_eq!(d.resolve(cc(0, 7, 0), &map), Some(("/master/dim".into(), Update::Range(0.0))));
        assert_eq!(d.resolve(cc(0, 7, 127), &map), Some(("/master/dim".into(), Update::Range(1.0))));
        let mid = range(d.resolve(cc(0, 7, 64), &map).unwrap().1);
        assert!((mid - 0.504).abs() < 0.01, "got {mid}");
    }

    #[test]
    fn lsb_after_msb_gives_fourteen_bit_resolution() {
        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 0, controller: 1 }, "/particles/hue");
        let mut d = Dispatcher::default();

        // MSB alone still works (7-bit devices).
        let coarse = range(d.resolve(cc(0, 1, 64), &map).unwrap().1);
        assert!((coarse - 0.504).abs() < 0.01);

        // LSB for the same controller refines it: (64<<7)|32 = 8224.
        let (param, update) = d.resolve(cc(0, 33, 32), &map).unwrap();
        let fine = range(update);
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
        let on = range(
            d.resolve(MidiEvent::NoteOn { channel: 0, note: 60, velocity: 127 }, &map)
                .unwrap()
                .1,
        );
        assert_eq!(on, 1.0);
        let off = range(d.resolve(MidiEvent::NoteOff { channel: 0, note: 60 }, &map).unwrap().1);
        assert_eq!(off, 0.0);
    }

    #[test]
    fn pitch_bend_spans_the_range() {
        let mut map = MidiMap::default();
        map.bind(Source::PitchBend { channel: 0 }, "/particles/speed");
        let mut d = Dispatcher::default();
        let centre =
            range(d.resolve(MidiEvent::PitchBend { channel: 0, value: 8192 }, &map).unwrap().1);
        assert!((centre - 0.5).abs() < 0.001, "got {centre}");
        let top =
            range(d.resolve(MidiEvent::PitchBend { channel: 0, value: 16383 }, &map).unwrap().1);
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
        map.bind_value(Source::Note { channel: 9, note: 40 }, "/scene/fire", 5.0);
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

    /// A `midi.json` written before triggers existed has no `value` field
    /// at all. Every binding anyone has already made is in one of those
    /// files, so failing to load it would cost the most laborious setup in
    /// the app — and silently, at the launch after an update.
    #[test]
    fn a_map_saved_before_triggers_existed_still_loads() {
        let old = r#"{"bindings":[
            {"source":{"kind":"control_change","channel":0,"controller":7},"param":"/master/dim"}
        ]}"#;
        let map: MidiMap = serde_json::from_str(old).unwrap();
        assert_eq!(map.bindings.len(), 1);
        assert_eq!(map.bindings[0].value, None);
        // And it still behaves as a sweep, not as a trigger.
        let mut d = Dispatcher::default();
        assert_eq!(range(d.resolve(cc(0, 7, 127), &map).unwrap().1), 1.0);
    }

    /// The bug this whole mechanism exists for.
    ///
    /// `/scene/fire` runs 0..16 and addresses a *slot*. A plain note
    /// binding sends velocity as a position, so every pad on the
    /// controller resolved to the top of the range — sixteen buttons that
    /// all fired scene 16, and nothing that could fire scene 3.
    #[test]
    fn a_button_bound_to_a_slot_sends_that_slot_not_the_top_of_the_range() {
        let mut map = MidiMap::default();
        map.bind_value(Source::Note { channel: 0, note: 36 }, "/scene/fire", 3.0);
        map.bind_value(Source::Note { channel: 0, note: 37 }, "/scene/fire", 7.0);
        let mut d = Dispatcher::default();

        let hit = |d: &mut Dispatcher, note, velocity| {
            d.resolve(MidiEvent::NoteOn { channel: 0, note, velocity }, &map).unwrap().1
        };
        assert_eq!(hit(&mut d, 36, 127), Update::Absolute(3.0));
        assert_eq!(hit(&mut d, 37, 127), Update::Absolute(7.0));
        // A pad is velocity sensitive. Playing it softly must still fire
        // it — and fire the same slot, not a lower one.
        assert_eq!(hit(&mut d, 36, 12), Update::Absolute(3.0));
    }

    /// Firing is edge triggered, so a button that only ever sent its slot
    /// would work once and then be dead. Release has to return it to rest.
    #[test]
    fn releasing_a_trigger_returns_it_to_rest_so_the_next_press_lands() {
        let mut map = MidiMap::default();
        map.bind_value(Source::Note { channel: 0, note: 36 }, "/scene/fire", 3.0);
        let mut d = Dispatcher::default();
        let press = MidiEvent::NoteOn { channel: 0, note: 36, velocity: 100 };
        let release = MidiEvent::NoteOff { channel: 0, note: 36 };

        assert_eq!(d.resolve(press, &map).unwrap().1, Update::Absolute(3.0));
        // Rest is the bottom of the parameter's range, which for a slot
        // parameter is "nothing selected".
        assert_eq!(d.resolve(release, &map).unwrap().1, Update::Range(0.0));
        assert_eq!(d.resolve(press, &map).unwrap().1, Update::Absolute(3.0));
    }

    /// Plenty of controllers send their pads and pedals as CC rather than
    /// as notes. 64 is the sustain pedal, and the switch controllers live
    /// from there up — below 32 is a continuous MSB and 32..64 is its LSB
    /// half, neither of which is a button.
    #[test]
    fn a_cc_switch_triggers_on_the_top_half_and_rests_on_the_bottom() {
        let mut map = MidiMap::default();
        map.bind_value(Source::ControlChange { channel: 0, controller: 64 }, "/scene/fire", 9.0);
        let mut d = Dispatcher::default();
        assert_eq!(d.resolve(cc(0, 64, 127), &map).unwrap().1, Update::Absolute(9.0));
        assert_eq!(d.resolve(cc(0, 64, 0), &map).unwrap().1, Update::Range(0.0));
    }

    /// Sixteen pads share one parameter, so unmapping one must not clear
    /// the other fifteen.
    #[test]
    fn unmapping_one_pad_leaves_the_rest_of_the_grid_mapped() {
        let mut map = MidiMap::default();
        map.bind_value(Source::Note { channel: 0, note: 36 }, "/scene/fire", 1.0);
        map.bind_value(Source::Note { channel: 0, note: 37 }, "/scene/fire", 2.0);
        map.bind_value(Source::Note { channel: 0, note: 38 }, "/scene/fire", 3.0);

        map.unbind_value("/scene/fire", 2.0);
        assert_eq!(map.bindings.len(), 2);
        assert_eq!(map.source_for_value("/scene/fire", 2.0), None);
        assert_eq!(
            map.source_for_value("/scene/fire", 3.0),
            Some(Source::Note { channel: 0, note: 38 })
        );
    }

    /// A parameter can carry any number of triggers, so the one shown
    /// beside its slider would be an arbitrary pick — and would say the
    /// slider was mapped when it was not.
    #[test]
    fn a_slider_does_not_claim_a_pads_binding_as_its_own() {
        let mut map = MidiMap::default();
        map.bind_value(Source::Note { channel: 0, note: 36 }, "/scene/fire", 1.0);
        assert_eq!(map.source_for("/scene/fire"), None);

        // A sweep binding on the same parameter is its own thing and does
        // show.
        let fader = Source::ControlChange { channel: 0, controller: 20 };
        map.bind(fader, "/scene/fire");
        assert_eq!(map.source_for("/scene/fire"), Some(fader));
    }
}
