//! Built-in nodes. Just enough to make sound and prove the chain works.

use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext};

use std::f64::consts::TAU;

/// A band-limited-enough-for-now sine oscillator. Generator: ignores its input
/// and writes the tone to every output channel.
pub struct SineOsc {
    freq_hz: f64,
    amp: f32,
    phase: f64,
    phase_inc: f64,
    sample_rate: f64,
}

impl SineOsc {
    /// Parameter index: oscillator frequency in Hz.
    pub const FREQ: u32 = 0;
    /// Parameter index: linear amplitude.
    pub const AMP: u32 = 1;

    pub fn new(freq_hz: f64, amp: f32) -> Self {
        Self {
            freq_hz,
            amp,
            phase: 0.0,
            phase_inc: 0.0,
            sample_rate: 0.0,
        }
    }

    fn recompute_inc(&mut self) {
        self.phase_inc = TAU * self.freq_hz / self.sample_rate;
    }
}

impl AudioNode for SineOsc {
    fn prepare(&mut self, sample_rate: f64, _max_block: usize) {
        self.sample_rate = sample_rate;
        self.recompute_inc();
    }

    fn process(&mut self, ctx: &ProcessContext, _input: &AudioBuffer, output: &mut AudioBuffer) {
        let channels = output.channels();
        for frame in 0..ctx.frames {
            let s = (self.phase.sin() as f32) * self.amp;
            self.phase += self.phase_inc;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
            for ch in 0..channels {
                output.channel_mut(ch)[frame] = s;
            }
        }
    }

    fn set_param(&mut self, param: u32, value: f32) {
        match param {
            Self::FREQ => {
                self.freq_hz = value as f64;
                self.recompute_inc();
            }
            Self::AMP => self.amp = value,
            _ => {}
        }
    }
}

/// Linear gain. Effect: scales whatever the upstream nodes summed to.
pub struct Gain {
    gain: f32,
}

impl Gain {
    /// Parameter index: linear gain.
    pub const GAIN: u32 = 0;

    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl AudioNode for Gain {
    fn prepare(&mut self, _sample_rate: f64, _max_block: usize) {}

    fn process(&mut self, ctx: &ProcessContext, input: &AudioBuffer, output: &mut AudioBuffer) {
        let channels = output.channels();
        for ch in 0..channels {
            let src = &input.channel(ch)[..ctx.frames];
            let dst = &mut output.channel_mut(ch)[..ctx.frames];
            for (d, s) in dst.iter_mut().zip(src) {
                *d = *s * self.gain;
            }
        }
    }

    fn set_param(&mut self, param: u32, value: f32) {
        if param == Self::GAIN {
            self.gain = value;
        }
    }
}
