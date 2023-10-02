use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use reson::{MidiEvent, Note, Portamento, Synth, SynthOpts, Tuning, Voice};
use ringbuf::HeapRb;
use std::sync::mpsc;
use std::time::Duration;
use reson::blep::{Triangle, Waveform};

fn main() {
    // A channel for sending MIDI events to the synth
    let (midi_tx, midi_rx) = mpsc::sync_channel::<MidiEvent>(1024);
    // A ring buffer for sending audio from the synth to the audio card
    let (mut audio_tx, mut audio_rx) = HeapRb::<f32>::new(2048).split();
    // A channel for the audio card to signal that it is ready for more input
    let (audio_tx2, audio_rx2) = mpsc::sync_channel::<()>(0);

    read_midi(move |ev| {
        midi_tx.send(ev).ok();
    });

    let sample_rate = play_audio(move |buffer| {
        audio_rx.pop_slice(buffer);
        audio_tx2.try_send(()).ok();
    });

    let mut synth = Synth::new(
        SynthOpts {
            tuning: Tuning::concert_pitch(),
            max_voices: 2,
            max_block_size: 256,
            mono: false,
            portamento: Portamento::Off,
            max_pitch_bend: 2.0,
        },
        SimpleVoice::<Triangle>::new(),
    );
    synth.set_sample_rate(sample_rate);

    const BLOCK_SIZE: usize = 128;
    let [mut left, mut right] = [[0.0; BLOCK_SIZE]; 2];
    let mut stereo = [0.0; 2 * BLOCK_SIZE];

    loop {
        // Wait until the buffer has space
        while audio_tx.free_len() <= 2 * BLOCK_SIZE {
            audio_rx2.recv().unwrap();
        }

        // Recieve MIDI events
        while let Ok(event) = midi_rx.try_recv() {
            synth.midi_event(event);
        }

        // Synthesise audio
        synth.process([&mut left, &mut right]);

        // Interleave and write to the ring buffer
        interleave_stereo(&left, &right, &mut stereo);
        audio_tx.push_slice(&stereo);
    }
}

fn read_midi(mut tx: impl FnMut(MidiEvent) + Send + 'static) {
    let mut midi_in = midir::MidiInput::new("MIDI input").unwrap();
    midi_in.ignore(midir::Ignore::ActiveSense);
    let in_ports = midi_in.ports();

    if !in_ports.is_empty() {
        println!("Connecting to first MIDI port.");

        // Create a callback to handle incoming MIDI messages
        let callback = move |_, message: &[u8], _: &mut ()| {
            if let Some(event) = MidiEvent::from_raw(message) {
                tx(event);
            }
        };

        // Connect to the selected MIDI input port
        let cxn = midi_in
            .connect(&in_ports[0], "midi-read-connection", callback, ())
            .unwrap();

        // Prevent the connection from being dropped
        Box::leak(Box::new(cxn));
    } else {
        println!("No MIDI input ports available. Playing generated notes.");
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2000));
            loop {
                let off = |note: Note| MidiEvent::NoteOff {
                    channel: 0,
                    note,
                    velocity: 0,
                };
                let on = |note: Note| MidiEvent::NoteOn {
                    channel: 0,
                    note,
                    velocity: 127,
                };
                for note in [60, 64, 67, 64] {
                    tx(on(note));
                    std::thread::sleep(Duration::from_millis(250));
                    tx(off(note));
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        });
    }
}

pub fn play_audio(mut rx: impl FnMut(&mut [f32]) + Send + 'static) -> u32 {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let config = device.default_output_config().unwrap();
    let sample_rate = config.sample_rate();

    let stream = device
        .build_output_stream(
            &config.into(),
            move |data, _| rx(data),
            move |err| {
                eprintln!("an error occurred on stream: {}", err);
            },
            None,
        )
        .unwrap();

    stream.play().unwrap();
    Box::leak(Box::new(stream));

    sample_rate.0
}

#[derive(Copy, Clone)]
pub struct SimpleVoice<W: Waveform> {
    inv_sample_rate: f32,
    osc: W,
    phase: f32,
    on: bool,
    amp: f32,
}

impl<W: Waveform + Default> SimpleVoice<W> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<W: Waveform + Default> Default for SimpleVoice<W> {
    fn default() -> Self {
        Self {
            inv_sample_rate: 0.0,
            osc: W::default(),
            phase: 0.0,
            on: false,
            amp: 0.0,
        }
    }
}

impl<W: Waveform> Voice for SimpleVoice<W> {
    fn set_sample_rate(&mut self, sample_rate: u32) {
        self.inv_sample_rate = (sample_rate as f32).recip();
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.on = false;
    }

    fn trigger(&mut self, _note: Note, _velocity: u8) {
        self.on = true;
    }

    fn release(&mut self) {
        self.on = false;
    }

    fn process(&mut self, pitch: f32, output: [&mut [f32]; 2]) -> bool {
        let [left, right] = output;
        if self.on || self.amp > 0.0 {
            let delta_amp = self.inv_sample_rate * 20.0 * if self.on { 1.0 } else { -1.0 };
            for sample in left.iter_mut() {
                let delta_phase = self.inv_sample_rate * pitch;
                *sample = self.amp * self.osc.sample(self.phase, delta_phase);
                self.phase = (self.phase + delta_phase).fract();
                self.amp = (self.amp + delta_amp).clamp(0.0, 1.0);
            }
            right.copy_from_slice(left);
            true
        } else {
            left.fill(0.0);
            right.fill(0.0);
            false
        }
    }
}

/// Interleaves the two channels of a stereo signal.
pub fn interleave_stereo(left: &[f32], right: &[f32], output: &mut [f32]) {
    let lr = left.iter().zip(right.iter());
    for (i, (&ls, &rs)) in lr.enumerate() {
        output[2 * i] = ls;
        output[2 * i + 1] = rs;
    }
}