use crate::Note;

/// A MIDI event that can be interpreted by [Synth].
///
/// [Synth]: crate::Synth
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum MidiEvent {
    NoteOn {
        channel: u8,
        note: Note,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: Note,
        velocity: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
}

impl MidiEvent {
    /// Creates a MIDI event from raw bytes.
    pub fn from_raw(data: &[u8]) -> Option<Self> {
        Some(match *data {
            [a @ 0x80..=0x8f, note, velocity] => MidiEvent::NoteOff {
                channel: a & 0x0f,
                note,
                velocity,
            },
            [a @ 0x90..=0x9f, note, velocity] => MidiEvent::NoteOn {
                channel: a & 0x0f,
                note,
                velocity,
            },
            [a @ 0xe0..=0xef, lsb, msb] => MidiEvent::PitchBend {
                channel: a & 0x0f,
                value: lsb as u16 | ((msb as u16) << 7),
            },
            _ => return None,
        })
    }
}
