use crate::Sample;

/// A planar, fixed-capacity audio buffer: `channels` blocks of `capacity`
/// samples laid out contiguously as `[ch0 frame0..N][ch1 frame0..N]...`.
///
/// Planar (rather than interleaved) keeps per-channel DSP simple — a node gets
/// a plain `&mut [Sample]` slice per channel. The host interleaves only once,
/// at the boundary with the device.
pub struct AudioBuffer {
    channels: usize,
    capacity: usize,
    data: Vec<Sample>,
}

impl AudioBuffer {
    /// Allocate a buffer. Call this off the real-time thread — it allocates.
    pub fn new(channels: usize, capacity: usize) -> Self {
        Self {
            channels,
            capacity,
            data: vec![0.0; channels * capacity],
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Maximum frames per channel this buffer can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Immutable view of one channel's full capacity.
    pub fn channel(&self, ch: usize) -> &[Sample] {
        let start = ch * self.capacity;
        &self.data[start..start + self.capacity]
    }

    /// Mutable view of one channel's full capacity.
    pub fn channel_mut(&mut self, ch: usize) -> &mut [Sample] {
        let start = ch * self.capacity;
        &mut self.data[start..start + self.capacity]
    }

    /// Zero every sample. RT-safe (no allocation).
    pub fn clear(&mut self) {
        self.data.fill(0.0);
    }
}
