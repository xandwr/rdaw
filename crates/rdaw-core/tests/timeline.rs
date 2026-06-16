//! End-to-end timeline behaviour, exercised through the offline renderer so it
//! runs with no audio device and is fully deterministic.

use std::sync::Arc;

use rdaw_core::nodes::Gain;
use rdaw_core::{Clip, Graph, LoopRegion, Timeline, Transport, TransportState, Waveform};

/// A mono waveform whose sample values are `1, 2, 3, ...` so positions are
/// trivially checkable in the output.
fn ramp(len: usize) -> Arc<Waveform> {
    let data = (0..len).map(|i| (i + 1) as f32).collect();
    Arc::new(Waveform::from_planar(1, data))
}

/// Render a single-node (timeline = master) mono graph and return the samples.
fn render_mono(timeline: Timeline, total: usize, block: usize) -> Vec<f32> {
    let mut graph = Graph::new(1);
    let t = graph.add(Box::new(timeline));
    graph.set_master(t);
    graph.prepare(48_000.0, 512);
    graph.render_offline(48_000.0, total, block)
}

#[test]
fn clip_lands_at_its_start_frame() {
    let tl = Timeline::new().with_clip(Clip::new(ramp(4), 2));
    let out = render_mono(tl, 8, 8);
    // Silence until frame 2, then the ramp, then silence again.
    assert_eq!(out, vec![0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 0.0, 0.0]);
}

#[test]
fn playback_is_identical_across_block_boundaries() {
    let clip = Clip::new(ramp(4), 2);
    let whole = render_mono(Timeline::new().with_clip(clip.clone()), 8, 8);
    // A block size that slices straight through the clip must produce the same
    // audio — this is the real test that sample_pos drives positioning.
    let chunked = render_mono(Timeline::new().with_clip(clip), 8, 3);
    assert_eq!(whole, chunked);
}

#[test]
fn overlapping_clips_sum() {
    let tl = Timeline::new()
        .with_clip(Clip::new(ramp(2), 0)) // frames 0,1 -> 1,2
        .with_clip(Clip::new(ramp(2), 1)); // frames 1,2 -> 1,2
    let out = render_mono(tl, 4, 4);
    assert_eq!(out, vec![1.0, 3.0, 2.0, 0.0]);
}

#[test]
fn source_offset_and_len_window_the_sample() {
    // Play only the middle two samples (3, 4) of a 1..=6 ramp.
    let tl = Timeline::new().with_clip(Clip::new(ramp(6), 0).with_source_range(2, 2));
    let out = render_mono(tl, 4, 4);
    assert_eq!(out, vec![3.0, 4.0, 0.0, 0.0]);
}

#[test]
fn clip_gain_scales_contribution() {
    let tl = Timeline::new().with_clip(Clip::new(ramp(3), 0).with_gain(0.5));
    let out = render_mono(tl, 3, 3);
    assert_eq!(out, vec![0.5, 1.0, 1.5]);
}

fn render_mono_looped(timeline: Timeline, total: usize, block: usize, lr: LoopRegion) -> Vec<f32> {
    let mut graph = Graph::new(1);
    let t = graph.add(Box::new(timeline));
    graph.set_master(t);
    graph.prepare(48_000.0, 512);
    graph.render_offline_looped(48_000.0, total, block, lr)
}

#[test]
fn loop_repeats_the_region() {
    // Source [1, 2]; loop over the whole 2-frame region, render 6 frames.
    let tl = Timeline::new().with_clip(Clip::new(ramp(2), 0));
    let out = render_mono_looped(tl, 6, 6, LoopRegion::new(0, 2));
    assert_eq!(out, vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
}

#[test]
fn loop_wrap_is_sample_accurate_across_blocks() {
    // The same loop, but with a block size that does NOT divide the loop length,
    // so blocks straddle the wrap point. Output must be identical — this is the
    // test that the wrap is split mid-block rather than quantized to the block.
    let clip = Clip::new(ramp(2), 0);
    let whole = render_mono_looped(
        Timeline::new().with_clip(clip.clone()),
        6,
        6,
        LoopRegion::new(0, 2),
    );
    let chunked = render_mono_looped(Timeline::new().with_clip(clip), 6, 4, LoopRegion::new(0, 2));
    assert_eq!(whole, chunked);
}

#[test]
fn loop_can_start_partway_through_the_timeline() {
    // Source [1,2,3,4], loop [1, 3) -> play 0, then bounce between frames 1 and 2.
    let tl = Timeline::new().with_clip(Clip::new(ramp(4), 0));
    let out = render_mono_looped(tl, 6, 6, LoopRegion::new(1, 3));
    assert_eq!(out, vec![1.0, 2.0, 3.0, 2.0, 3.0, 2.0]);
}

#[test]
fn transport_splits_a_block_at_the_loop_boundary() {
    // Drive the controller directly and record the segments it emits.
    let mut transport = Transport::new(120.0);
    transport.playing = true;
    transport.set_loop(Some(LoopRegion::new(0, 2)));

    let mut segments: Vec<(u64, usize, usize)> = Vec::new();
    transport.render_block(5, |state: TransportState, offset, len| {
        segments.push((state.sample_pos, offset, len));
    });

    // 5 frames over a 2-frame loop: [0..2], [0..2], [0..1].
    assert_eq!(segments, vec![(0, 0, 2), (0, 2, 2), (0, 4, 1)]);
    // And the play head ends inside the loop, ready for the next block.
    assert_eq!(transport.position(), 1);
}

#[test]
fn mono_source_fans_out_and_mixes_through_a_gain() {
    // timeline -> gain(0.5) -> master, stereo bus, mono source.
    let tl = Timeline::new().with_clip(Clip::new(ramp(2), 0));
    let mut graph = Graph::new(2);
    let t = graph.add(Box::new(tl));
    let m = graph.add(Box::new(Gain::new(0.5)));
    graph.connect(t, m);
    graph.set_master(m);
    graph.prepare(48_000.0, 512);

    let out = graph.render_offline(48_000.0, 2, 2);
    // Interleaved L,R,L,R: the mono ramp (1,2) appears on both channels, halved.
    assert_eq!(out, vec![0.5, 0.5, 1.0, 1.0]);
}
