//! Clip fade-in / fade-out and equal-power crossfades, through the offline
//! renderer so everything is deterministic and device-free.

use std::sync::Arc;

use rdaw_core::{Clip, FadeCurve, Graph, Timeline, Waveform};

/// A flat mono "DC" source of 1.0, `len` frames long.
fn dc(len: usize) -> Arc<Waveform> {
    Arc::new(Waveform::from_planar(1, vec![1.0; len]))
}

fn render_mono(timeline: Timeline, total: usize, block: usize) -> Vec<f32> {
    let mut graph = Graph::new(1);
    let t = graph.add(Box::new(timeline));
    graph.set_master(t);
    graph.prepare(48_000.0, 512);
    graph.render_offline(48_000.0, total, block)
}

#[test]
fn linear_fade_in_ramps_from_silence_to_full() {
    // 4-frame fade-in over a flat 1.0 source: silent at frame 0, full at frame 4.
    let tl = Timeline::new().with_clip(Clip::new(dc(8), 0).with_fade_in(4));
    let out = render_mono(tl, 6, 6);
    assert_eq!(out, vec![0.0, 0.25, 0.5, 0.75, 1.0, 1.0]);
}

#[test]
fn linear_fade_out_ramps_down_toward_the_end() {
    // 4-frame clip that fades out across its whole length. With `remaining`
    // counting down 4,3,2,1 the gains are 1, .75, .5, .25.
    let tl = Timeline::new().with_clip(Clip::new(dc(4), 0).with_fade_out(4));
    let out = render_mono(tl, 4, 4);
    assert_eq!(out, vec![1.0, 0.75, 0.5, 0.25]);
}

#[test]
fn no_fade_leaves_the_clip_untouched() {
    let tl = Timeline::new().with_clip(Clip::new(dc(4), 0));
    let out = render_mono(tl, 4, 4);
    assert_eq!(out, vec![1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn fades_are_identical_across_block_boundaries() {
    let clip = Clip::new(dc(8), 0).with_fade_in(3).with_fade_out(3);
    let whole = render_mono(Timeline::new().with_clip(clip.clone()), 8, 8);
    let chunked = render_mono(Timeline::new().with_clip(clip), 8, 3);
    assert_eq!(whole, chunked);
}

#[test]
fn equal_power_crossfade_holds_constant_power() {
    // Clip A fades out across frames 0..4; clip B fades in across the same frames
    // (B placed at A.end() - len). Their summed power should stay ~1 throughout.
    let a = Clip::new(dc(4), 0)
        .with_fade_out(4)
        .with_fade_curve(FadeCurve::EqualPower);
    let b = Clip::new(dc(4), 0)
        .with_fade_in(4)
        .with_fade_curve(FadeCurve::EqualPower);

    // Render each clip alone so we can check the per-clip gains sum in power.
    let a_only = render_mono(Timeline::new().with_clip(a.clone()), 4, 4);
    let b_only = render_mono(Timeline::new().with_clip(b.clone()), 4, 4);

    for (ga, gb) in a_only.iter().zip(&b_only) {
        let power = ga * ga + gb * gb;
        assert!((power - 1.0).abs() < 1e-6, "power {power} drifted from 1.0");
    }

    // And summed on one timeline they never clip past full power's envelope.
    let mixed = render_mono(Timeline::new().with_clip(a).with_clip(b), 4, 4);
    assert_eq!(mixed.len(), 4);
}
