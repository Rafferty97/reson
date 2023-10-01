pub use midi::*;
pub use synth::*;
pub use tuning::*;
pub use voice::*;

mod fade;
mod midi;
mod synth;
mod tuning;
mod voice;

/// A MIDI note between 0 and 127.
pub type Note = u8;
