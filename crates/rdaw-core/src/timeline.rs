//! The timeline: recorded audio placed against the transport.
//!
//! This is what makes the project a DAW rather than a synth. A [`Waveform`] is
//! an immutable, shareable chunk of decoded audio; a [`Clip`] places (a slice
//! of) one on the timeline at a given frame; the [`Timeline`] node reads the
//! transport's play position each block and renders whichever clips overlap it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext, Sample};

/// The shape a clip fade traces from silence to full level (and back).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FadeCurve {
    /// A straight line in linear gain. Simple; dips ~3 dB at the midpoint of an
    /// equal-and-opposite crossfade.
    #[default]
    Linear,
    /// A sine/cosine quarter-curve. Two of these, fading out and in over the same
    /// region, hold constant power across the overlap — the right default for
    /// crossfading two unrelated clips.
    EqualPower,
}

impl FadeCurve {
    /// Map a normalized fade progress `x` in `[0, 1]` (0 = silent, 1 = full) to a
    /// linear gain.
    fn gain(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => x,
            FadeCurve::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
        }
    }
}

/// An immutable, multi-channel block of decoded PCM, stored planar
/// (`[ch0 frame0..N][ch1 frame0..N]...`) to match [`AudioBuffer`].
///
/// Wrap it in an [`Arc`] and share it across as many [`Clip`]s as you like:
/// cloning a clip then costs one atomic increment, never a copy of the audio,
/// so placing the same sample a hundred times on the timeline is free and the
/// real-time thread only ever reads.
pub struct Waveform {
    channels: usize,
    frames: usize,
    /// Native sample rate of this audio in Hz, or `0.0` if unspecified. When it
    /// differs from the graph rate a [`Clip`] using this waveform is resampled on
    /// playback so it sounds at its recorded pitch; `0.0` is taken to mean "same
    /// as the graph" and skips resampling entirely.
    sample_rate: f64,
    /// Planar PCM, `channels * frames` long.
    data: Vec<Sample>,
}

impl Waveform {
    /// Build from planar data laid out as `[ch0..][ch1..]`. `data.len()` must be
    /// a multiple of `channels`. The native sample rate is left unspecified;
    /// chain [`with_sample_rate`](Waveform::with_sample_rate) if the audio was
    /// recorded at a rate other than the graph's.
    pub fn from_planar(channels: usize, data: Vec<Sample>) -> Self {
        assert!(channels > 0, "waveform needs at least one channel");
        assert!(
            data.len().is_multiple_of(channels),
            "planar data length {} is not a multiple of {channels} channels",
            data.len()
        );
        let frames = data.len() / channels;
        Self {
            channels,
            frames,
            sample_rate: 0.0,
            data,
        }
    }

    /// Build from interleaved data (`[L, R, L, R, ...]`), deinterleaving into the
    /// planar layout. Convenient for decoded files, which are interleaved.
    pub fn from_interleaved(channels: usize, interleaved: &[Sample]) -> Self {
        assert!(channels > 0, "waveform needs at least one channel");
        let frames = interleaved.len() / channels;
        let mut data = vec![0.0; channels * frames];
        for frame in 0..frames {
            for ch in 0..channels {
                data[ch * frames + frame] = interleaved[frame * channels + ch];
            }
        }
        Self {
            channels,
            frames,
            sample_rate: 0.0,
            data,
        }
    }

    /// Builder: record the audio's native sample rate in Hz. A clip of this
    /// waveform is resampled to the graph rate on playback (see [`Clip`]).
    pub fn with_sample_rate(mut self, sample_rate: f64) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Native sample rate in Hz, or `0.0` if unspecified ("same as graph").
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Total frames of audio.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// One channel's samples.
    pub fn channel(&self, ch: usize) -> &[Sample] {
        let start = ch * self.frames;
        &self.data[start..start + self.frames]
    }
}

/// A placement of (part of) a [`Waveform`] on the timeline.
#[derive(Clone)]
pub struct Clip {
    /// The audio this clip plays. Shared, never copied.
    pub source: Arc<Waveform>,
    /// Timeline frame at which the clip starts sounding.
    pub start: u64,
    /// First frame of `source` to play (lets a clip start partway in).
    pub source_offset: u64,
    /// How many frames to play. Capped by what `source` actually has.
    pub len: u64,
    /// Linear gain applied to this clip's contribution.
    pub gain: f32,
    /// Fade-in length in timeline frames: the clip ramps from silence to full
    /// over its first `fade_in` frames. `0` means no fade-in.
    pub fade_in: u64,
    /// Fade-out length in timeline frames: the clip ramps from full to silence
    /// over its last `fade_out` frames. `0` means no fade-out.
    pub fade_out: u64,
    /// The curve both fades follow.
    pub fade_curve: FadeCurve,
}

impl Clip {
    /// Play the whole of `source`, starting at timeline frame `start`.
    pub fn new(source: Arc<Waveform>, start: u64) -> Self {
        let len = source.frames() as u64;
        Self {
            source,
            start,
            source_offset: 0,
            len,
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
            fade_curve: FadeCurve::default(),
        }
    }

    /// Builder: play only `[offset, offset + len)` of the source.
    pub fn with_source_range(mut self, offset: u64, len: u64) -> Self {
        self.source_offset = offset;
        self.len = len;
        self
    }

    /// Builder: set how many *timeline* frames the clip occupies. For a resampled
    /// clip this differs from the source frame count — e.g. a 4-frame source at
    /// half the graph rate spans 8 timeline frames.
    pub fn with_len(mut self, len: u64) -> Self {
        self.len = len;
        self
    }

    /// Builder: scale this clip's level.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Builder: fade in over the first `frames` timeline frames.
    pub fn with_fade_in(mut self, frames: u64) -> Self {
        self.fade_in = frames;
        self
    }

    /// Builder: fade out over the last `frames` timeline frames.
    pub fn with_fade_out(mut self, frames: u64) -> Self {
        self.fade_out = frames;
        self
    }

    /// Builder: set the curve both fades follow.
    pub fn with_fade_curve(mut self, curve: FadeCurve) -> Self {
        self.fade_curve = curve;
        self
    }

    /// First timeline frame *after* the clip ends.
    pub fn end(&self) -> u64 {
        self.start + self.len
    }

    /// The fade gain at `local` frames into the clip (`0..len`). 1.0 outside any
    /// fade; the product of the fade-in and fade-out shapes where they overlap on
    /// a very short clip. The fade-in is silent at frame 0 and full at frame
    /// `fade_in`; the fade-out is full entering its last `fade_out` frames and
    /// approaches silence at the clip's end. With [`FadeCurve::EqualPower`], a
    /// fade-out and a fade-in of equal length laid over the same frames (place
    /// the later clip at `earlier.end() - len`) sum to constant power.
    fn fade_gain(&self, local: u64) -> f32 {
        let mut g = 1.0;
        if self.fade_in > 0 && local < self.fade_in {
            g *= self.fade_curve.gain(local as f32 / self.fade_in as f32);
        }
        if self.fade_out > 0 {
            // Frames remaining after this one, counting down to 0 on the last.
            let remaining = self.len.saturating_sub(local);
            if remaining <= self.fade_out {
                g *= self
                    .fade_curve
                    .gain(remaining as f32 / self.fade_out as f32);
            }
        }
        g
    }
}

/// A node that renders a set of [`Clip`]s positioned on the transport timeline.
/// It's a source (ignores its input); wire it into a gain or bus downstream.
///
/// Each block it figures out which clips overlap `[sample_pos, sample_pos +
/// frames)` and sums their audio into the output — so seeking the transport,
/// crossing block boundaries, and overlapping clips all just work.
///
/// A clip whose source was recorded at a different rate than the graph
/// ([`Waveform::sample_rate`]) is linearly resampled on the fly, so imported
/// audio always plays back at its recorded pitch regardless of the device rate.
#[derive(Default)]
pub struct Timeline {
    clips: Vec<Clip>,
}

impl Timeline {
    pub fn new() -> Self {
        Self { clips: Vec::new() }
    }

    /// Add a clip. Control thread only (mutates the clip list).
    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
    }

    /// Builder form of [`add_clip`](Timeline::add_clip).
    pub fn with_clip(mut self, clip: Clip) -> Self {
        self.clips.push(clip);
        self
    }
}

impl AudioNode for Timeline {
    fn prepare(&mut self, _sample_rate: f64, _max_block: usize) {}

    fn process(&mut self, ctx: &ProcessContext, _input: &AudioBuffer, output: &mut AudioBuffer) {
        let frames = ctx.frames;
        let out_channels = output.channels();

        // A generator owns its output, so start from silence each block.
        for ch in 0..out_channels {
            output.channel_mut(ch)[..frames].fill(0.0);
        }

        let block_start = ctx.transport.sample_pos;
        let block_end = block_start + frames as u64;

        for clip in &self.clips {
            // Overlap between this block's window and the clip on the timeline.
            let ov_start = block_start.max(clip.start);
            let ov_end = block_end.min(clip.end());
            if ov_start >= ov_end {
                continue;
            }

            let src_channels = clip.source.channels();
            if src_channels == 0 {
                continue;
            }

            // Source frames consumed per output frame. A waveform with no stated
            // rate (or one matching the graph) plays 1:1 with `frac == 0`, so the
            // interpolation below reduces to an exact sample copy.
            let src_rate = clip.source.sample_rate();
            let ratio = if src_rate > 0.0 {
                src_rate / ctx.sample_rate
            } else {
                1.0
            };

            for ch in 0..out_channels {
                // Fan a mono source out to every channel; otherwise map 1:1 and
                // clamp so a stereo clip on a mono bus doesn't read past its end.
                let src_ch = ch.min(src_channels - 1);
                let src = clip.source.channel(src_ch);
                let dst = output.channel_mut(ch);

                for t in ov_start..ov_end {
                    let out_frame = (t - block_start) as usize;
                    let local = t - clip.start;
                    // Fractional read position into the source for this output
                    // frame, then linearly interpolate the two bracketing samples.
                    let src_pos = clip.source_offset as f64 + local as f64 * ratio;
                    let i0 = src_pos.floor() as usize;
                    if i0 >= src.len() {
                        continue;
                    }
                    let frac = (src_pos - i0 as f64) as f32;
                    let s0 = src[i0];
                    // Hold the last sample at the very end rather than reading past
                    // it (and toward zero), which would add a click.
                    let s1 = if i0 + 1 < src.len() { src[i0 + 1] } else { s0 };
                    let sample = s0 + (s1 - s0) * frac;
                    dst[out_frame] += sample * clip.gain * clip.fade_gain(local);
                }
            }
        }
    }
}
