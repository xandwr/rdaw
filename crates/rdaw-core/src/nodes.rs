//! Built-in nodes. Just enough to make sound and prove the chain works.

use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext};

use std::f64::consts::TAU;

/// A band-limited-enough-for-now sine oscillator. Generator: overwrites the
/// buffer on every channel.
pub struct SineOsc {
    freq_hz: f64,
    amp: f32,
    phase: f64,
    phase_inc: f64,
}

impl SineOsc {
    pub fn new(freq_hz: f64, amp: f32) -> Self {
        Self {
            freq_hz,
            amp,
            phase: 0.0,
            phase_inc: 0.0,
        }
    }
}

impl AudioNode for SineOsc {
    fn prepare(&mut self, sample_rate: f64, _max_block: usize) {
        self.phase_inc = TAU * self.freq_hz / sample_rate;
    }

    fn process(&mut self, ctx: &ProcessContext, buffer: &mut AudioBuffer) {
        let channels = buffer.channels();
        for frame in 0..ctx.frames {
            let s = (self.phase.sin() as f32) * self.amp;
            self.phase += self.phase_inc;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
            for ch in 0..channels {
                buffer.channel_mut(ch)[frame] = s;
            }
        }
    }
}

/// Linear gain. Effect: scales whatever the upstream node produced.
pub struct Gain {
    pub gain: f32,
}

impl Gain {
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl AudioNode for Gain {
    fn prepare(&mut self, _sample_rate: f64, _max_block: usize) {}

    fn process(&mut self, ctx: &ProcessContext, buffer: &mut AudioBuffer) {
        let channels = buffer.channels();
        for ch in 0..channels {
            for s in &mut buffer.channel_mut(ch)[..ctx.frames] {
                *s *= self.gain;
            }
        }
    }
}
