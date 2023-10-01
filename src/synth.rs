use crate::fade::FadeBuffer;
use crate::tuning::Tuning;
use crate::voice::Voice;
use crate::{MidiEvent, Note};
use std::sync::Arc;

/// A polyphonic synthesizer.
pub struct Synth<V: Voice + Clone> {
    /// The configuration options.
    opts: SynthOpts,
    /// Buffer used to hold the output from each voice before mixing.
    buffer: Vec<f32>,
    /// The prototype voice used to instantiate new voices.
    voice: V,
    /// The bank of voices.
    voices: Vec<VoiceHandle<V>>,
    /// Monotonic counter used to track the order in which voices were triggered and released.
    counter: usize,
    /// Small buffer used to gracefully fade out stolen voices
    fade_out: FadeBuffer<256>,
    /// The current pitch bend ratio, to be multiplied with the base frequency of each voice.
    pitch_bend: f32,
    /// The sample rate.
    sample_rate: u32,
}

/// Configuration options for [Synth].
#[derive(Clone)]
pub struct SynthOpts {
    /// The tuning system, which relates notes to their pitch in Hz.
    pub tuning: Arc<Tuning>,
    /// The maximum number of samples that will be requested in one call to `process`.
    /// Used for allocating the internal buffer.
    pub max_block_size: usize,
    /// The maximum number of voices that can be simultaneously played.
    pub max_voices: usize,
    /// If `true`, the synthesizer acts as a monophonic synth, despite the value of `max_voices`.
    pub mono: bool,
    /// The portamento setting.
    pub portamento: Portamento,
    /// The maximum pitch bend of a MIDI pitch bend event in semitones.
    pub max_pitch_bend: f32,
}

/// The portamento setting for a synthesizer.
#[derive(Copy, Clone)]
pub enum Portamento {
    /// Portamento is disabled.
    Off,
    /// The voice will glide from one note to the next in a fixed duration,
    /// denoted in seconds.
    Fixed(f32),
    /// The voice will glide from one note to the next at a fixed rate,
    /// denoted in seconds per octave.
    Variable(f32),
}

struct VoiceHandle<V: Voice> {
    voice: V,
    phase: VoicePhase,
    pitch: f32,
    counter: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VoicePhase {
    On(Note),
    Released(Note),
    Off,
}

impl<V: Voice + Clone> Synth<V> {
    /// Creates a new polyphonic synth with a fixed number of voices.
    ///
    /// # Parameters
    /// * `opts` - Configuration options for the polyphonic synth.
    /// * `voice` - A prototypical voice from which the bank of voices will be cloned.
    pub fn new(opts: SynthOpts, voice: V) -> Self {
        let mut out = Self {
            opts,
            buffer: vec![],
            voice,
            voices: vec![],
            counter: 0,
            fade_out: FadeBuffer::new(),
            pitch_bend: 1.0,
            sample_rate: 0,
        };
        out.update_opts(|_| {});
        out
    }

    /// Updates the settings for the synth.
    ///
    /// This might result in the allocation of memory, if for example,
    /// the maximum number of voices is increased or the maximum block size is increased.
    pub fn update_opts(&mut self, f: impl FnOnce(&mut SynthOpts)) {
        f(&mut self.opts);
        self.voices.resize_with(self.opts.max_voices, || {
            VoiceHandle::new(self.voice.clone())
        });
        self.buffer.resize(self.opts.max_block_size * 2, 0.0);
    }

    /// Updates the bank of voices by cloning the provided prototype voice.
    ///
    /// This results in all notes being immediately reset and silenced.
    pub fn update_voice(&mut self, voice: V) {
        self.voice = voice;
        for voice in &mut self.voices {
            *voice = VoiceHandle::new(self.voice.clone());
        }
    }

    /// Sets the sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.voice.set_sample_rate(sample_rate);
        for voice in &mut self.voices {
            voice.set_sample_rate(sample_rate);
        }
    }

    /// Triggers a note.
    ///
    /// # Parameters
    /// * `note` - The MIDI note being triggered, between 0 and 127.
    /// * `velocity` - The velocity of the note, between 0 and 127.
    pub fn trigger(&mut self, note: Note, velocity: u8) {
        let voice = self
            .voices
            .iter_mut()
            .min_by_key(|v| v.priority(note))
            .unwrap();
        let pitch = self.opts.tuning.pitch(note);
        voice.trigger(note, velocity, pitch, self.counter);
        self.counter += 1;

        // FIXME: Process fade out for voice stealing
    }

    /// Releases a note.
    pub fn release(&mut self, note: Note) {
        let voice = self.voices.iter_mut().find(|v| v.note_on() == Some(note));
        if let Some(voice) = voice {
            voice.release(self.counter);
            self.counter += 1;
        }
    }

    /// Sets the global pitch bend as a raw 14-bit MIDI value.
    pub fn set_pitch_bend_raw(&mut self, value: u16) {
        let semitones = ((value as f32 - 8192.0) / 8192.0) * self.opts.max_pitch_bend;
        self.set_pitch_bend(semitones);
    }

    /// Sets the global pitch bend in semitones.
    pub fn set_pitch_bend(&mut self, semitones: f32) {
        self.pitch_bend = 2f32.powf(semitones / 12.0);
    }

    /// Processes a MIDI message.
    pub fn midi_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { note, velocity, .. } => self.trigger(note, velocity),
            MidiEvent::NoteOff { note, .. } => self.release(note),
            MidiEvent::PitchBend { value, .. } => self.set_pitch_bend_raw(value),
        }
    }

    /// Synthesizes a block of audio into `output`.
    pub fn process(&mut self, output: [&mut [f32]; 2]) {
        let [left, right] = output;

        let len = left.len();
        assert_eq!(right.len(), len);
        assert!(len <= self.opts.max_block_size);

        // Prepare temporary buffers for each voice's output.
        let (left_temp, right_temp) = self.buffer[..2 * len].split_at_mut(len);

        // Track whether any audio has been written to output.
        let mut written = false;

        // Process each active voice in turn.
        for handle in &mut self.voices {
            if !handle.active() {
                continue;
            }
            if written {
                handle.process(self.pitch_bend, [left_temp, right_temp]);
                add_buffers(left, left_temp);
                add_buffers(right, right_temp);
            } else {
                handle.process(self.pitch_bend, [left, right]);
                written = true;
            }
        }

        // If no voices are sounding, ensure the output buffer is filled with silence.
        if !written {
            left.fill(0.0);
            right.fill(0.0);
        }

        // Apply the fade buffer
        self.fade_out.process([left, right]);
    }
}

impl<V: Voice> VoiceHandle<V> {
    fn new(voice: V) -> Self {
        Self {
            voice,
            phase: VoicePhase::Off,
            pitch: 0.0,
            counter: 0,
        }
    }

    /// Returns `true` if the voice is not producing silence.
    fn active(&self) -> bool {
        self.phase != VoicePhase::Off
    }

    /// Gets the note that the voice is currently playing, if it is in the `On` phase.
    fn note_on(&self) -> Option<Note> {
        if let VoicePhase::On(note) = self.phase {
            Some(note)
        } else {
            None
        }
    }

    /// Gets the priority used for voice allocation, with the lowest priority being preferred.
    fn priority(&self, note: Note) -> usize {
        match self.phase {
            // Note has been re-triggered
            VoicePhase::On(n) if n == note => 0,
            // Unused voice
            VoicePhase::Off => 1,
            // Released voice for the same note
            VoicePhase::Released(n) if n == note => 2,
            // Oldest released note
            VoicePhase::Released(_) => 3 + self.counter,
            // Oldest triggered note
            VoicePhase::On(_) => usize::MAX / 2 + self.counter,
        }
    }

    /// Sets the sample rate.
    fn set_sample_rate(&mut self, sample_rate: u32) {
        self.voice.set_sample_rate(sample_rate);
    }

    /// Triggers a note.
    fn trigger(&mut self, note: Note, velocity: u8, pitch: f32, counter: usize) {
        self.voice.trigger(note, velocity);
        self.phase = VoicePhase::On(note);
        self.pitch = pitch; // TODO: Portamento
        self.counter = counter;
    }

    /// Releases the current note.
    pub fn release(&mut self, counter: usize) {
        let note = match self.phase {
            VoicePhase::On(note) => note,
            VoicePhase::Released(note) => note,
            VoicePhase::Off => return,
        };

        self.voice.release();
        self.phase = VoicePhase::Released(note);
        self.counter = counter;
    }

    /// Processes the voice into the provided output buffer.
    fn process(&mut self, pitch_bend: f32, output: [&mut [f32]; 2]) {
        let active = self.voice.process(self.pitch * pitch_bend, output);
        if !active {
            self.phase = VoicePhase::Off;
        }
    }
}

fn add_buffers(dst: &mut [f32], src: &[f32]) {
    assert_eq!(src.len(), dst.len());
    for i in 0..src.len() {
        dst[i] += src[i];
    }
}
