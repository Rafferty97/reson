use crate::Note;

/// An individual voice used to synthesize audio for a single note.
pub trait Voice {
    /// Sets the sample rate.
    ///
    /// This may be called multiple times.
    fn set_sample_rate(&mut self, sample_rate: u32);

    /// Resets the state of the voice.
    ///
    /// This is invoked on an active or releasing voice that is stolen for a new note.
    fn reset(&mut self);

    /// Triggers a note to be played.
    ///
    /// If the voice is still active, then [reset] is invoked immediately before this
    /// method is called. Therefore, this method is always called on inactive voices.
    ///
    /// [reset]: Self::reset
    ///
    /// # Parameters
    /// * `note` - The MIDI note being triggered, between 0 and 127.
    /// * `velocity` - The velocity of the note, between 0 and 127.
    fn trigger(&mut self, note: Note, velocity: u8);

    /// Triggers a note to be glided to.
    ///
    /// Most voices don't need to do anything when a glide is triggered,
    /// so the default implementation of this method does nothing.
    ///
    /// # Parameters
    /// * `note` - The MIDI note being triggered, between 0 and 127.
    /// * `velocity` - The velocity of the note, between 0 and 127.
    fn glide(&mut self, note: Note, velocity: u8) {
        let _ = (note, velocity);
    }

    /// Releases the currently playing note.
    fn release(&mut self);

    /// Synthesizes audio in stereo.
    ///
    /// # Parameters
    /// * `pitch` - The current pitch in Hz, accounting for glides and pitch bending.
    /// * `output` - The left and right audio buffers for writing the output.
    ///
    /// # Return
    /// Returns `false` if the voice is inactive and will only produce silence until
    /// a note is triggered.
    fn process(&mut self, pitch: f32, output: [&mut [f32]; 2]) -> bool;
}