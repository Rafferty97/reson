use std::sync::{Arc, OnceLock};
use crate::Note;

/// Denotes the pitch in Hz for each MIDI note.
#[derive(Copy, Clone)]
pub struct Tuning {
    notes: [f32; 128]
}

impl Tuning {
    /// Creates a new equal temperament tuning, based on the provided pitch for the note A4.
    pub fn equal_temperament(a4: f32) -> Self {
        let mut notes = [0.0; 128];
        for note in 0..128 {
            notes[note] = a4 * 2.0f32.powf((note as f32 - 69.0) / 12.0);
        }
        Self { notes }
    }

    /// Gets an reference to the standard tuning system in which A4 is 440Hz.
    pub fn concert_pitch() -> Arc<Self> {
        static TUNING: OnceLock<Arc<Tuning>> = OnceLock::new();
        TUNING.get_or_init(|| Arc::new(Self::equal_temperament(440.0))).clone()
    }

    /// Gets the pitch of the provided MIDI note, which must be between 0 and 127.
    pub fn pitch(&self, note: Note) -> f32 {
        *self.notes.get(note as usize)
            .expect("MIDI note must be between 0 and 127.")
    }
}