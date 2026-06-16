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
    fn prepare(&mut self, sample_rate: f64, _max_block: usize, _channels: usize) {
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
    fn prepare(&mut self, _sample_rate: f64, _max_block: usize, _channels: usize) {}

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

/// A track channel strip: linear gain followed by a stereo pan. This is the node
/// a [`Track`](crate::project::Track) compiles down to, sitting between the
/// track's timeline and the master bus.
///
/// Pan is a position in `[-1, 1]` (`-1` hard left, `0` center, `1` hard right)
/// applied with a constant-power law: the left/right gains trace a quarter
/// circle, so a sound swept across the field keeps a steady perceived loudness
/// and sits 3 dB down at center. Pan only touches channels 0 (L) and 1 (R); a
/// mono bus gets gain alone, and any channel beyond the first stereo pair passes
/// through at unity pan.
///
/// A muted channel renders silence regardless of its gain: the path a track's
/// mute (or a solo elsewhere silencing it) compiles to. Mute is a live parameter
/// like gain and pan, so it can be toggled from the control thread or ridden by
/// automation.
pub struct Channel {
    gain: f32,
    pan: f32,
    muted: bool,
}

impl Channel {
    /// Parameter index: linear gain.
    pub const GAIN: u32 = 0;
    /// Parameter index: pan position in `[-1, 1]`.
    pub const PAN: u32 = 1;
    /// Parameter index: mute. Any non-zero value mutes; `0.0` unmutes.
    pub const MUTE: u32 = 2;

    pub fn new(gain: f32, pan: f32) -> Self {
        Self {
            gain,
            pan: pan.clamp(-1.0, 1.0),
            muted: false,
        }
    }

    /// Builder: start muted (rendering silence until unmuted).
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// The (left, right) constant-power gains for the current pan position.
    fn pan_gains(&self) -> (f32, f32) {
        // Map [-1, 1] onto a quarter turn [0, PI/2]; cos/sin then give a pair
        // whose squares always sum to 1 (constant power), equal at center.
        let theta = (self.pan + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
        (theta.cos(), theta.sin())
    }
}

impl AudioNode for Channel {
    fn prepare(&mut self, _sample_rate: f64, _max_block: usize, _channels: usize) {}

    fn process(&mut self, ctx: &ProcessContext, input: &AudioBuffer, output: &mut AudioBuffer) {
        let channels = output.channels();

        // A muted strip contributes nothing; emit silence and skip the pan math.
        if self.muted {
            for ch in 0..channels {
                output.channel_mut(ch)[..ctx.frames].fill(0.0);
            }
            return;
        }

        let (left, right) = self.pan_gains();
        for ch in 0..channels {
            // Pan is only meaningful for a stereo pair; mono and surplus channels
            // pass through at unity pan so they aren't silently attenuated.
            let pan = if channels >= 2 {
                match ch {
                    0 => left,
                    1 => right,
                    _ => 1.0,
                }
            } else {
                1.0
            };
            let g = self.gain * pan;
            let src = &input.channel(ch)[..ctx.frames];
            let dst = &mut output.channel_mut(ch)[..ctx.frames];
            for (d, s) in dst.iter_mut().zip(src) {
                *d = *s * g;
            }
        }
    }

    fn set_param(&mut self, param: u32, value: f32) {
        match param {
            Self::GAIN => self.gain = value,
            Self::PAN => self.pan = value.clamp(-1.0, 1.0),
            Self::MUTE => self.muted = value != 0.0,
            _ => {}
        }
    }
}

/// The response shape of a [`Biquad`] filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterType {
    /// Passes frequencies below the cutoff, rolls off above it. Unity gain at DC.
    #[default]
    LowPass,
    /// Passes frequencies above the cutoff, rolls off below it. Silences DC.
    HighPass,
    /// Passes a band around the cutoff, rolls off either side. 0 dB at the peak.
    BandPass,
    /// Passes everything but a narrow notch at the cutoff.
    Notch,
}

impl FilterType {
    /// The `f32` carried over [`set_param`](AudioNode::set_param) for each type,
    /// matched (after rounding) in [`Biquad::set_param`].
    fn from_param(value: f32) -> Option<Self> {
        match value.round() as i32 {
            0 => Some(FilterType::LowPass),
            1 => Some(FilterType::HighPass),
            2 => Some(FilterType::BandPass),
            3 => Some(FilterType::Notch),
            _ => None,
        }
    }
}

/// A second-order (biquad) filter: the engine's first real effect, and the
/// building block of every EQ band. One filter section, applied independently to
/// each channel with [Direct Form I][df1] state, using the standard [RBJ cookbook]
/// coefficients for its [`FilterType`].
///
/// The coefficients are recomputed off the RT thread in [`prepare`](AudioNode::prepare)
/// and whenever a parameter changes via `set_param`, so [`process`](AudioNode::process)
/// is just the difference equation: no `sin`/`cos`, no allocation. Cutoff and Q
/// are live parameters, so a filter sweep can be automated exactly like a gain.
///
/// [df1]: https://en.wikipedia.org/wiki/Digital_biquad_filter#Direct_form_1
/// [RBJ cookbook]: https://www.w3.org/TR/audio-eq-cookbook/
pub struct Biquad {
    kind: FilterType,
    cutoff_hz: f64,
    q: f64,
    sample_rate: f64,
    /// Feed-forward coefficients, already normalized by `a0`.
    b: [f32; 3],
    /// Feedback coefficients `a1`, `a2`, normalized by `a0`.
    a: [f32; 2],
    /// Per-channel Direct Form I state: `[x[n-1], x[n-2], y[n-1], y[n-2]]`,
    /// sized to the graph's channel count in `prepare`.
    state: Vec<[f32; 4]>,
}

impl Biquad {
    /// Parameter index: cutoff (center) frequency in Hz.
    pub const CUTOFF: u32 = 0;
    /// Parameter index: quality factor `Q` (resonance / bandwidth).
    pub const Q: u32 = 1;
    /// Parameter index: [`FilterType`], passed as its discriminant (`0` low-pass,
    /// `1` high-pass, `2` band-pass, `3` notch).
    pub const TYPE: u32 = 2;

    /// A filter of `kind` at `cutoff_hz` with quality factor `q` (≈ `0.707` for a
    /// flat, non-resonant response). Coefficients are computed once the node is
    /// prepared at a known sample rate.
    pub fn new(kind: FilterType, cutoff_hz: f64, q: f64) -> Self {
        Self {
            kind,
            cutoff_hz,
            q: q.max(1e-4),
            sample_rate: 0.0,
            // Identity (pass-through) until `prepare` computes real coefficients.
            b: [1.0, 0.0, 0.0],
            a: [0.0, 0.0],
            state: Vec::new(),
        }
    }

    /// Recompute the normalized coefficients from the current type, cutoff, Q and
    /// sample rate (RBJ cookbook). Control thread / `prepare` only: calls `sin`
    /// and `cos`. A no-op until a sample rate is known.
    fn recompute_coeffs(&mut self) {
        if self.sample_rate <= 0.0 {
            return;
        }
        // Clamp the cutoff just inside (0, Nyquist) so the coefficients stay
        // finite at the extremes.
        let nyquist = self.sample_rate * 0.5;
        let f0 = self.cutoff_hz.clamp(1.0, nyquist - 1.0);
        let w0 = TAU * f0 / self.sample_rate;
        let (sin_w0, cos_w0) = (w0.sin(), w0.cos());
        let alpha = sin_w0 / (2.0 * self.q);

        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            FilterType::LowPass => {
                let b1 = 1.0 - cos_w0;
                (
                    b1 / 2.0,
                    b1,
                    b1 / 2.0,
                    1.0 + alpha,
                    -2.0 * cos_w0,
                    1.0 - alpha,
                )
            }
            FilterType::HighPass => {
                let b0 = (1.0 + cos_w0) / 2.0;
                (
                    b0,
                    -(1.0 + cos_w0),
                    b0,
                    1.0 + alpha,
                    -2.0 * cos_w0,
                    1.0 - alpha,
                )
            }
            FilterType::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha),
            FilterType::Notch => (
                1.0,
                -2.0 * cos_w0,
                1.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
        };

        self.b = [(b0 / a0) as f32, (b1 / a0) as f32, (b2 / a0) as f32];
        self.a = [(a1 / a0) as f32, (a2 / a0) as f32];
    }
}

impl AudioNode for Biquad {
    fn prepare(&mut self, sample_rate: f64, _max_block: usize, channels: usize) {
        self.sample_rate = sample_rate;
        // One filter state per channel, zeroed; allocated here, never on the RT
        // thread.
        self.state = vec![[0.0; 4]; channels];
        self.recompute_coeffs();
    }

    fn process(&mut self, ctx: &ProcessContext, input: &AudioBuffer, output: &mut AudioBuffer) {
        let channels = output.channels();
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        for ch in 0..channels {
            // A channel with no allocated state (more channels than we prepared
            // for) passes through untouched rather than indexing out of bounds.
            let Some(st) = self.state.get_mut(ch) else {
                let src = &input.channel(ch)[..ctx.frames];
                let dst = &mut output.channel_mut(ch)[..ctx.frames];
                dst.copy_from_slice(src);
                continue;
            };
            let [mut x1, mut x2, mut y1, mut y2] = *st;
            let src = &input.channel(ch)[..ctx.frames];
            let dst = &mut output.channel_mut(ch)[..ctx.frames];
            for (d, &x0) in dst.iter_mut().zip(src) {
                let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                x2 = x1;
                x1 = x0;
                y2 = y1;
                y1 = y0;
                *d = y0;
            }
            *st = [x1, x2, y1, y2];
        }
    }

    fn set_param(&mut self, param: u32, value: f32) {
        match param {
            Self::CUTOFF => self.cutoff_hz = value as f64,
            Self::Q => self.q = (value as f64).max(1e-4),
            Self::TYPE => match FilterType::from_param(value) {
                Some(kind) => self.kind = kind,
                None => return,
            },
            _ => return,
        }
        // A live parameter change re-derives the coefficients. This calls
        // `sin`/`cos`; acceptable on the command path between blocks, the same
        // place gain/pan changes already land.
        self.recompute_coeffs();
    }
}
