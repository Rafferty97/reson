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
    /// The portamento setting. This only has an effect is `mono` is true.
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

/// Contextual information provided to a [VoiceHandle] when triggered or released.
struct VoiceCtx {
    /// The sample rate in Hz.
    sample_rate: u32,
    /// The current portamento setting.
    portamento: Portamento,
    /// The current value of the monotonic counter.
    counter: usize
}

struct VoiceHandle<V: Voice> {
    /// The voice itself, which produces the audio.
    voice: V,
    /// The phase of this voice, which may be actively playing, releasing, or inactive.
    phase: VoicePhase,
    /// The pitch of the currently playing note.
    pitch: f32,
    /// Information about the current note glide, if one is in progress.
    glide: Option<GlideState>,
    /// The value of the monotonic counter at the time this voice was last triggered/released.
    counter: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VoicePhase {
    On(Note),
    Released(Note),
    Off,
}

/// Represents the pitch of a voice, which may be in the middle of a glide.
#[derive(Clone, Copy)]
struct GlideState {
    /// The base-2 logarithm of the start pitch.
    start: f32,
    /// The base-2 logarithm of the target pitch.
    target: f32,
    /// The duration of the glide in samples.
    duration: usize,
    /// The current elapsed time of the glide in samples.
    time: usize
}

impl<V: Voice + Clone> Synth<V> {
    /// Creates a new polyphonic synth with a fixed number of voices.
    ///
    /// # Parameters
    /// * `opts` - Configuration options for the polyphonic synth.
    /// * `voice` - A prototypical voice from which the bank of voices will be cloned.
    pub fn new(opts: SynthOpts, voice: V) -> Self {
        Self::validate_opts(&opts);
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
        Self::validate_opts(&self.opts);
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
        let ctx = self.voice_ctx();

        let voice = if self.opts.mono {
            &mut self.voices[0]
        } else {
            let voice = self
                .voices
                .iter_mut()
                .min_by_key(|v| v.priority(note))
                .unwrap();

            if voice.active() {
                // Voice is stolen, so fade out
                self.fade_out.add_voice(|buf| voice.process(self.pitch_bend, buf));
                voice.reset();
            }

            voice
        };

        let pitch = self.opts.tuning.pitch(note);
        voice.trigger(note, velocity, pitch, &ctx);
        self.counter += 1;
    }

    /// Releases a note.
    pub fn release(&mut self, note: Note) {
        let ctx = self.voice_ctx();

        let voice = if self.opts.mono {
            let voice = &mut self.voices[0];
            (voice.note_on() == Some(note)).then_some(voice)
        } else {
            self.voices.iter_mut().find(|v| v.note_on() == Some(note))
        };

        if let Some(voice) = voice {
            voice.release(&ctx);
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
        let voices = if self.opts.mono {
            &mut self.voices[..1]
        } else {
            &mut self.voices
        };
        for handle in voices {
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

    /// Validates the synthesiser options.
    fn validate_opts(opts: &SynthOpts) {
        if opts.max_voices == 0 {
            panic!("Synth must have at least one voice.");
        }
    }

    /// Gets the context to pass to a voice being triggered/released.
    fn voice_ctx(&self) -> VoiceCtx {
        VoiceCtx {
            sample_rate: self.sample_rate,
            portamento: self.opts.portamento,
            counter: self.counter
        }
    }
}

impl<V: Voice> VoiceHandle<V> {
    fn new(voice: V) -> Self {
        Self {
            voice,
            phase: VoicePhase::Off,
            pitch: 0.0,
            glide: None,
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

    /// Resets the voice.
    fn reset(&mut self) {
        self.voice.reset();
        self.phase = VoicePhase::Off;
    }

    /// Triggers a note.
    fn trigger(&mut self, note: Note, velocity: u8, pitch: f32, ctx: &VoiceCtx) {
        if let Some(glide) = self.calc_glide(pitch, ctx) {
            self.glide = Some(glide);
        } else {
            self.voice.trigger(note, velocity);
            self.glide = None;
        }

        self.pitch = pitch;
        self.phase = VoicePhase::On(note);
        self.counter = ctx.counter;
    }

    /// Releases the current note.
    pub fn release(&mut self, ctx: &VoiceCtx) {
        let note = match self.phase {
            VoicePhase::On(note) => note,
            VoicePhase::Released(note) => note,
            VoicePhase::Off => return,
        };

        self.voice.release();
        self.phase = VoicePhase::Released(note);
        self.counter = ctx.counter;
    }

    /// Processes the voice into the provided output buffer.
    fn process(&mut self, pitch_bend: f32, output: [&mut [f32]; 2]) {
        let num_samples = output[0].len();

        // Process audio
        let active = self.voice.process(self.pitch() * pitch_bend, output);
        if !active {
            self.phase = VoicePhase::Off;
        }

        // Update glide state
        if let Some(glide) = &mut self.glide {
            glide.time += num_samples;
            if glide.time >= glide.duration {
                self.glide = None;
            }
        }
    }

    /// Calculates the current pitch, accounting for glide but not pitch bend.
    fn pitch(&self) -> f32 {
        if let Some(glide) = self.glide {
            let t = (glide.time as f32) / (glide.duration as f32);
            2_f32.powf(glide.start + t * (glide.target - glide.start))
        } else {
            self.pitch
        }
    }

    /// Calculates the glide which should be performed, if any, when a note is triggered.
    ///
    /// # Parameters
    /// * `target_pitch` - Pitch of the triggered note in Hz.
    /// * `ctx` - The context from the synth.
    fn calc_glide(&self, target_pitch: f32, ctx: &VoiceCtx) -> Option<GlideState> {
        // Only glide when a note is triggered while another is playing
        if !matches!(self.phase, VoicePhase::On(_)) {
            return None;
        }

        match ctx.portamento {
            Portamento::Fixed(time) => {
                let start = self.pitch().log2();
                let target = target_pitch.log2();
                let duration = (time * ctx.sample_rate as f32) as usize;
                println!("{} {} {}", start, target, duration);
                Some(GlideState { start, target, time: 0, duration })
            }
            Portamento::Variable(rate) => {
                let start = self.pitch().log2();
                let target = target_pitch.log2();
                let distance = (start - target).abs();
                let duration = (rate * distance * ctx.sample_rate as f32) as usize;
                Some(GlideState { start, target, time: 0, duration })
            }
            Portamento::Off => None
        }
    }
}

fn add_buffers(dst: &mut [f32], src: &[f32]) {
    assert_eq!(src.len(), dst.len());
    for i in 0..src.len() {
        dst[i] += src[i];
    }
}
