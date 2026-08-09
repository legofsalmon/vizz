//! MIDI wire-format parsing.
//!
//! Pure and hardware-free, which matters: this is the part most likely to
//! be wrong, and it is fully testable without a controller plugged in.

/// The subset of MIDI vizz maps to parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiEvent {
    /// Channels are 0-based here; controllers display them 1-based.
    ControlChange { channel: u8, controller: u8, value: u8 },
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    /// 0..=16383, centre 8192.
    PitchBend { channel: u8, value: u16 },
    /// Realtime clock tick, 24 per quarter note. Carries no channel.
    Clock,
    /// Transport start: the next clock tick is the downbeat.
    Start,
    /// Transport continue: ticks resume, no downbeat implied.
    Continue,
    /// Transport stop: ticks may keep arriving on some gear; the sender
    /// has stopped meaning them.
    Stop,
}

/// Parse one MIDI message. Returns `None` for anything vizz does not map
/// (sysex, aftertouch, song position…) rather than treating it as an
/// error — unmapped traffic must not spam the log.
pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    // Realtime messages carry no channel and no data bytes. Clock and
    // the transport are the sync surface; everything else system-ish is
    // still ignored.
    match status {
        0xF8 => return Some(MidiEvent::Clock),
        0xFA => return Some(MidiEvent::Start),
        0xFB => return Some(MidiEvent::Continue),
        0xFC => return Some(MidiEvent::Stop),
        _ => {}
    }
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let channel = status & 0x0F;
    let data1 = *bytes.get(1)? & 0x7F;
    match status & 0xF0 {
        0xB0 => Some(MidiEvent::ControlChange {
            channel,
            controller: data1,
            value: *bytes.get(2)? & 0x7F,
        }),
        0x90 => {
            let velocity = *bytes.get(2)? & 0x7F;
            // Note-on with zero velocity is note-off; many controllers
            // only ever send this form, so missing it strands notes on.
            if velocity == 0 {
                Some(MidiEvent::NoteOff { channel, note: data1 })
            } else {
                Some(MidiEvent::NoteOn { channel, note: data1, velocity })
            }
        }
        0x80 => Some(MidiEvent::NoteOff { channel, note: data1 }),
        0xE0 => {
            let lsb = data1 as u16;
            let msb = (*bytes.get(2)? & 0x7F) as u16;
            Some(MidiEvent::PitchBend { channel, value: (msb << 7) | lsb })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_control_change() {
        assert_eq!(
            parse(&[0xB0, 7, 100]),
            Some(MidiEvent::ControlChange { channel: 0, controller: 7, value: 100 })
        );
        // Channel is the low nibble.
        assert_eq!(
            parse(&[0xB9, 1, 64]),
            Some(MidiEvent::ControlChange { channel: 9, controller: 1, value: 64 })
        );
    }

    #[test]
    fn realtime_clock_and_transport_parse() {
        assert_eq!(parse(&[0xF8]), Some(MidiEvent::Clock));
        assert_eq!(parse(&[0xFA]), Some(MidiEvent::Start));
        assert_eq!(parse(&[0xFB]), Some(MidiEvent::Continue));
        assert_eq!(parse(&[0xFC]), Some(MidiEvent::Stop));
        // The rest of the system range stays unmapped.
        assert_eq!(parse(&[0xF2, 0, 0]), None, "song position leaked through");
        assert_eq!(parse(&[0xFE]), None, "active sensing leaked through");
    }

    #[test]
    fn note_on_with_zero_velocity_is_note_off() {
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiEvent::NoteOff { channel: 0, note: 60 }));
        assert_eq!(
            parse(&[0x90, 60, 1]),
            Some(MidiEvent::NoteOn { channel: 0, note: 60, velocity: 1 })
        );
        assert_eq!(parse(&[0x80, 60, 64]), Some(MidiEvent::NoteOff { channel: 0, note: 60 }));
    }

    #[test]
    fn pitch_bend_is_14_bit_lsb_first() {
        // Centre: LSB 0, MSB 64 => 8192.
        assert_eq!(parse(&[0xE0, 0, 64]), Some(MidiEvent::PitchBend { channel: 0, value: 8192 }));
        assert_eq!(parse(&[0xE0, 0, 0]), Some(MidiEvent::PitchBend { channel: 0, value: 0 }));
        assert_eq!(
            parse(&[0xE0, 0x7F, 0x7F]),
            Some(MidiEvent::PitchBend { channel: 0, value: 16383 })
        );
    }

    #[test]
    fn ignores_unmapped_and_malformed_messages() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[0xF0, 1, 2]), None, "sysex");
        assert_eq!(parse(&[0xD0, 64]), None, "channel aftertouch is unmapped");
        assert_eq!(parse(&[0xB0]), None, "truncated");
        assert_eq!(parse(&[0xB0, 7]), None, "truncated");
        assert_eq!(parse(&[0x40, 7, 7]), None, "data byte as status");
    }
}
