/// A small buffer used to gracefully fade out voices which have been voice-stolen.
pub struct FadeBuffer<const N: usize> {
    /// Contains the faded out audio in stereo.
    buffer: [[f32; N]; 2],
    /// The next sample to read from the buffer, which is `N` at completion.
    index: usize,
}

impl<const N: usize> Default for FadeBuffer<N> {
    fn default() -> Self {
        Self {
            buffer: [[0.0; N]; 2],
            index: N
        }
    }
}

impl<const N: usize> FadeBuffer<N> {
    /// Creates an empty [FadeBuffer].
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a voice to fade out to the internal buffer.
    pub fn add_voice(&mut self, f: impl FnOnce([&mut [f32]; 2])) {
        // Retain the old buffer contents
        let temp = self.buffer;

        // Process the voice into the internal buffer
        let [left, right] = &mut self.buffer;
        f([left, right]);

        // Apply the fade and add the contents from the retained buffer
        for ch in 0..2 {
            for i in 0..N {
                self.buffer[ch][i] *= 1.0 - (i as f32 / N as f32);
            }
            for i in 0..(N - self.index) {
                self.buffer[ch][i] += temp[ch][self.index + i];
            }
        }

        // Reset the read index
        self.index = 0;
    }

    /// Reads from the internal buffer and adds it to the output.
    pub fn process(&mut self, output: [&mut [f32]; 2]) {
        debug_assert!(output[0].len() == output[1].len());

        let len = usize::min(output[0].len(), N - self.index);
        for idx in 0..len {
            output[0][idx] += self.buffer[0][self.index + idx];
            output[1][idx] += self.buffer[1][self.index + idx];
        }
        self.index += len;
    }
}