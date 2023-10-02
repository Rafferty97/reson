//! Band-limited basic shape oscillators using the PolyBLEP algorithm.

use std::f32::consts::PI;

/// A periodic waveform.
pub trait Waveform {
    /// Samples the waveform at the given phase.
    ///
    /// # Parameters
    /// * `phase` - Phase of the waveform to sample, between 0 and 1.
    /// * `delta_phase` - The phase increment between subsequent samples;
    /// used by some [Waveform]s to minimise aliasing by adapting to the sample rate.
    fn sample(&mut self, phase: f32, delta_phase: f32) -> f32;
}

/// A sine wave.
#[derive(Copy, Clone, Default)]
pub struct Sine {}

impl Waveform for Sine {
    fn sample(&mut self, phase: f32, _delta_phase: f32) -> f32 {
        (2.0 * PI * phase).sin()
    }
}

/// A square wave.
#[derive(Copy, Clone, Default)]
pub struct Square {}

impl Waveform for Square {
    fn sample(&mut self, phase: f32, delta_phase: f32) -> f32 {
        let mut sample = if phase < 0.5 { 1.0 } else { -1.0 };
        sample += poly_blep( phase, delta_phase);
        sample -= poly_blep((phase + 0.5).fract(), delta_phase);
        sample
    }
}

/// A sawtooth wave (ramps up).
#[derive(Copy, Clone, Default)]
pub struct Sawtooth {}

impl Waveform for Sawtooth {
    fn sample(&mut self, phase: f32, delta_phase: f32) -> f32 {
        let mut sample = 2.0 * phase - 1.0;
        sample -= poly_blep(phase, delta_phase);
        sample
    }
}

/// A triangle wave.
#[derive(Copy, Clone, Default)]
pub struct Triangle {
    inner: Integrator<Square>
}

impl Waveform for Triangle {
    fn sample(&mut self, phase: f32, delta_phase: f32) -> f32 {
        self.inner.sample(phase, delta_phase)
    }
}

#[derive(Copy, Clone, Default)]
struct Integrator<O: Waveform> {
    inner: O,
    value: f32,
}

impl<O: Waveform> Waveform for Integrator<O> {
    fn sample(&mut self, phase: f32, delta_phase: f32) -> f32 {
        let sample = self.inner.sample(phase, delta_phase);
        self.value += sample;
        self.value *= 1.0 - delta_phase; // FIXME: What factor?
        self.value
    }
}

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        2. * t - (t * t) - 1.
    } else if t > (1.0 - dt) {
        let t = (t - 1.0) / dt;
        (t * t) + 2. * t + 1.
    } else {
        0.
    }
}