//! Stand-in audio source and demo arrangement. Everything here is temporary
//! scaffolding until real file loading is wired in — keeping it in one module
//! makes it easy to delete wholesale at that point.

use std::f64::consts::TAU;
use std::sync::Arc;

use rdaw_core::{ClipData, Project, Track, Waveform};

use crate::SR;

/// A mono sine burst — a stand-in audio source until file loading is wired in.
pub fn sine_burst(freq_hz: f64, seconds: f64, amp: f32) -> Waveform {
    let frames = (seconds * SR) as usize;
    let inc = TAU * freq_hz / SR;
    let data = (0..frames)
        .map(|i| ((i as f64 * inc).sin() as f32) * amp)
        .collect();
    Waveform::from_planar(1, data).with_sample_rate(SR)
}

/// Build the demo arrangement: one track, two copies of a 440 Hz burst placed
/// at different times, so the timeline has something to draw. Returns the
/// project alongside the decoded sources (indexed the same as
/// `project.sources`).
pub fn project() -> (Project, Vec<Arc<Waveform>>) {
    let burst = Arc::new(sine_burst(440.0, 0.6, 0.4));
    let clip_len = burst.frames() as u64;

    let mut project = Project::new(120.0);
    project.master_gain = 0.8;
    let src = project.add_source("<synth-440hz>");
    project.add_track(
        Track::new("lead")
            .with_clip(ClipData::new(src, 0u64, clip_len))
            .with_clip(ClipData::new(src, clip_len * 2, clip_len)),
    );

    (project, vec![burst])
}
