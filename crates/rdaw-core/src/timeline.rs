//! The timeline: recorded audio placed against the transport.
//!
//! This is what makes the project a DAW rather than a synth. A [`Waveform`] is
//! an immutable, shareable chunk of decoded audio; a [`Clip`] places (a slice
//! of) one on the timeline at a given frame; the [`Timeline`] node reads the
//! transport's play position each block and renders whichever clips overlap it.

use std::sync::Arc;

use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext, Sample};

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
    /// Planar PCM, `channels * frames` long.
    data: Vec<Sample>,
}

impl Waveform {
    /// Build from planar data laid out as `[ch0..][ch1..]`. `data.len()` must be
    /// a multiple of `channels`.
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
            data,
        }
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
        }
    }

    /// Builder: play only `[offset, offset + len)` of the source.
    pub fn with_source_range(mut self, offset: u64, len: u64) -> Self {
        self.source_offset = offset;
        self.len = len;
        self
    }

    /// Builder: scale this clip's level.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// First timeline frame *after* the clip ends.
    pub fn end(&self) -> u64 {
        self.start + self.len
    }
}

/// A node that renders a set of [`Clip`]s positioned on the transport timeline.
/// It's a source (ignores its input); wire it into a gain or bus downstream.
///
/// Each block it figures out which clips overlap `[sample_pos, sample_pos +
/// frames)` and sums their audio into the output — so seeking the transport,
/// crossing block boundaries, and overlapping clips all just work.
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

            for ch in 0..out_channels {
                // Fan a mono source out to every channel; otherwise map 1:1 and
                // clamp so a stereo clip on a mono bus doesn't read past its end.
                let src_ch = ch.min(src_channels - 1);
                let src = clip.source.channel(src_ch);
                let dst = output.channel_mut(ch);

                for t in ov_start..ov_end {
                    let out_frame = (t - block_start) as usize;
                    let src_frame = (clip.source_offset + (t - clip.start)) as usize;
                    if src_frame < src.len() {
                        dst[out_frame] += src[src_frame] * clip.gain;
                    }
                }
            }
        }
    }
}
