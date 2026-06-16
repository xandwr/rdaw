//! Clip sample-rate conversion, exercised through the offline renderer so it
//! runs with no audio device and is fully deterministic.

use std::sync::Arc;

use rdaw_core::{Clip, Graph, Timeline, Waveform};

/// A mono waveform of `1, 2, 3, ...` recorded at `rate` Hz.
fn ramp_at(len: usize, rate: f64) -> Arc<Waveform> {
    let data = (0..len).map(|i| (i + 1) as f32).collect();
    Arc::new(Waveform::from_planar(1, data).with_sample_rate(rate))
}

/// Render a single-node (timeline = master) mono graph at `graph_rate`.
fn render_mono_at(timeline: Timeline, graph_rate: f64, total: usize, block: usize) -> Vec<f32> {
    let mut graph = Graph::new(1);
    let t = graph.add(Box::new(timeline));
    graph.set_master(t);
    graph.prepare(graph_rate, 512);
    graph.render_offline(graph_rate, total, block)
}

#[test]
fn matching_rate_is_an_exact_copy() {
    // Source rate == graph rate: ratio is exactly 1, so playback is bit-identical
    // to the un-resampled path.
    let tl = Timeline::new().with_clip(Clip::new(ramp_at(4, 48_000.0), 0));
    let out = render_mono_at(tl, 48_000.0, 4, 4);
    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn unspecified_rate_is_an_exact_copy() {
    // A waveform with no stated rate also plays 1:1 (the from_planar default),
    // so all the existing timeline behaviour is unchanged.
    let tl = Timeline::new().with_clip(Clip::new(
        Arc::new(Waveform::from_planar(1, vec![1.0, 2.0, 3.0, 4.0])),
        0,
    ));
    let out = render_mono_at(tl, 48_000.0, 4, 4);
    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn half_rate_source_is_upsampled_by_linear_interpolation() {
    // 24 kHz source played on a 48 kHz graph advances half a source frame per
    // output frame, so we read 0, 0.5, 1.0, 1.5, ... into the 1,2,3,4 ramp.
    // Clip length is in *timeline* frames, so 8 frames cover the 4-frame source.
    let tl = Timeline::new().with_clip(Clip::new(ramp_at(4, 24_000.0), 0).with_len(8));
    let out = render_mono_at(tl, 48_000.0, 8, 8);
    // src positions: 0,0.5,1,1.5,2,2.5,3 -> last (3.5) holds the final sample.
    assert_eq!(out, vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.0]);
}

#[test]
fn resampling_is_identical_across_block_boundaries() {
    // The interpolation is driven by the absolute timeline position, so slicing
    // the same clip into uneven blocks must produce identical audio.
    let clip = Clip::new(ramp_at(4, 24_000.0), 0).with_len(8);
    let whole = render_mono_at(Timeline::new().with_clip(clip.clone()), 48_000.0, 8, 8);
    let chunked = render_mono_at(Timeline::new().with_clip(clip), 48_000.0, 8, 3);
    assert_eq!(whole, chunked);
}
