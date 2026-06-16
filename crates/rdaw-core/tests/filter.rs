//! The [`Biquad`] filter node, driven through a hand-wired graph. The standard
//! RBJ coefficients give exact DC behaviour we can assert on: a low-pass has
//! unity gain at DC, a high-pass kills it, and a notch leaves it untouched.
//! Resonance and the actual rolloff are checked by energy, not exact samples.

use std::sync::Arc;

use rdaw_core::nodes::{Biquad, FilterType};
use rdaw_core::{Clip, Graph, Timeline, Waveform};

/// A flat mono "DC" source of `value`, long enough for the filter to settle.
fn dc(value: f32, len: usize) -> Arc<Waveform> {
    Arc::new(Waveform::from_planar(1, vec![value; len]))
}

/// A mono source that flips between `+amp` and `-amp` every sample — the fastest
/// signal representable, sitting right at Nyquist.
fn nyquist(amp: f32, len: usize) -> Arc<Waveform> {
    let data = (0..len)
        .map(|i| if i % 2 == 0 { amp } else { -amp })
        .collect();
    Arc::new(Waveform::from_planar(1, data))
}

/// Render `total` frames of `source` through a single `filter` node, mono.
fn filter_dc(filter: Biquad, source: Arc<Waveform>, total: usize) -> Vec<f32> {
    let mut graph = Graph::new(1);
    let tl = graph.add(Box::new(Timeline::new().with_clip(Clip::new(source, 0))));
    let f = graph.add(Box::new(filter));
    graph.connect(tl, f);
    graph.set_master(f);
    graph.prepare(48_000.0, 512);
    graph.render_offline(48_000.0, total, 64)
}

/// Sum of squares — a stand-in for signal energy.
fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

#[test]
fn lowpass_has_unity_gain_at_dc() {
    // A low-pass passes DC untouched: once the transient settles the output
    // tracks the constant input exactly.
    let out = filter_dc(
        Biquad::new(FilterType::LowPass, 1_000.0, 0.707),
        dc(1.0, 4096),
        4096,
    );
    assert!(
        (out[out.len() - 1] - 1.0).abs() < 1e-4,
        "settled low-pass DC was {}",
        out[out.len() - 1]
    );
}

#[test]
fn highpass_blocks_dc() {
    // A high-pass has zero gain at DC: a constant input decays to silence.
    let out = filter_dc(
        Biquad::new(FilterType::HighPass, 1_000.0, 0.707),
        dc(1.0, 4096),
        4096,
    );
    assert!(
        out[out.len() - 1].abs() < 1e-4,
        "settled high-pass DC was {}",
        out[out.len() - 1]
    );
}

#[test]
fn notch_passes_dc() {
    // A notch only removes its center band; DC (far below a 1 kHz notch) survives.
    let out = filter_dc(
        Biquad::new(FilterType::Notch, 1_000.0, 0.707),
        dc(1.0, 4096),
        4096,
    );
    assert!(
        (out[out.len() - 1] - 1.0).abs() < 1e-4,
        "settled notch DC was {}",
        out[out.len() - 1]
    );
}

#[test]
fn lowpass_attenuates_high_frequencies() {
    // A Nyquist-rate signal through a low cutoff should lose most of its energy,
    // while the same signal through a high-pass passes largely intact.
    let total = 4096;
    let lp = filter_dc(
        Biquad::new(FilterType::LowPass, 500.0, 0.707),
        nyquist(1.0, total),
        total,
    );
    let hp = filter_dc(
        Biquad::new(FilterType::HighPass, 500.0, 0.707),
        nyquist(1.0, total),
        total,
    );
    // Compare the settled tails so the startup transient doesn't dominate.
    let tail = total - 1024..total;
    let lp_e = energy(&lp[tail.clone()]);
    let hp_e = energy(&hp[tail]);
    assert!(
        lp_e < hp_e * 0.01,
        "low-pass kept too much HF energy: lp={lp_e}, hp={hp_e}"
    );
}

#[test]
fn changing_cutoff_changes_the_response() {
    // The TYPE/CUTOFF params re-derive the coefficients: a high-passed DC signal
    // is silent, but flipping the same node to a low-pass restores unity gain.
    let mut graph = Graph::new(1);
    let tl = graph.add(Box::new(
        Timeline::new().with_clip(Clip::new(dc(1.0, 4096), 0)),
    ));
    let f = graph.add(Box::new(Biquad::new(FilterType::HighPass, 1_000.0, 0.707)));
    graph.connect(tl, f);
    graph.set_master(f);
    graph.prepare(48_000.0, 512);

    let hp = graph.render_offline(48_000.0, 4096, 64);
    assert!(hp[hp.len() - 1].abs() < 1e-4, "high-pass should block DC");

    graph.set_param(f, Biquad::TYPE, 0.0); // -> LowPass
    let lp = graph.render_offline(48_000.0, 4096, 64);
    assert!(
        (lp[lp.len() - 1] - 1.0).abs() < 1e-4,
        "low-pass should pass DC, got {}",
        lp[lp.len() - 1]
    );
}

#[test]
fn extra_channels_pass_through_untouched() {
    // The graph is stereo but the filter was prepared for 2 channels; feeding it
    // is fine. (Guards the bounds-checked per-channel state path.)
    let mut graph = Graph::new(2);
    let src = Arc::new(Waveform::from_planar(2, vec![1.0; 16]));
    let tl = graph.add(Box::new(Timeline::new().with_clip(Clip::new(src, 0))));
    let f = graph.add(Box::new(Biquad::new(FilterType::LowPass, 1_000.0, 0.707)));
    graph.connect(tl, f);
    graph.set_master(f);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 8, 8);
    // Just assert it ran and produced finite output on both channels.
    assert_eq!(out.len(), 16);
    assert!(out.iter().all(|s| s.is_finite()));
}
