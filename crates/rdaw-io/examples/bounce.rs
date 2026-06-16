//! Bounce a timeline to a WAV file with no audio device involved — the offline,
//! end-to-end proof that the core renders arranged audio correctly.
//!
//! Run: `cargo run -p rdaw-io --example bounce`
//! Then open `bounce.wav` in any player (two 440 Hz tone bursts, the second
//! starting after the first and at half volume).

use std::f64::consts::TAU;
use std::sync::Arc;

use rdaw_core::nodes::Gain;
use rdaw_core::{Clip, Graph, Timeline, Waveform};
use rdaw_io::write_wav;

/// Synthesize a mono sine burst as a reusable sample asset.
fn sine_burst(freq_hz: f64, seconds: f64, amp: f32, sample_rate: f64) -> Waveform {
    let frames = (seconds * sample_rate) as usize;
    let inc = TAU * freq_hz / sample_rate;
    let data = (0..frames)
        .map(|i| ((i as f64 * inc).sin() as f32) * amp)
        .collect();
    Waveform::from_planar(1, data)
}

fn main() -> anyhow::Result<()> {
    let sample_rate = 44_100.0;

    // One sample asset, placed twice on the timeline: at the start, and again
    // three-quarters of a second in at half volume.
    let tone = Arc::new(sine_burst(440.0, 0.5, 0.4, sample_rate));
    let timeline = Timeline::new()
        .with_clip(Clip::new(tone.clone(), 0))
        .with_clip(Clip::new(tone, (sample_rate * 0.75) as u64).with_gain(0.5));

    // timeline -> master gain -> output, stereo.
    let mut graph = Graph::new(2);
    let tl = graph.add(Box::new(timeline));
    let master = graph.add(Box::new(Gain::new(0.8)));
    graph.connect(tl, master);
    graph.set_master(master);
    graph.prepare(sample_rate, 1024);

    let total_frames = (sample_rate * 1.5) as usize;
    let out = graph.render_offline(sample_rate, total_frames, 512);

    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    write_wav("bounce.wav", &out, 2, sample_rate as u32)?;

    println!(
        "wrote bounce.wav: {} frames, {} ch, peak {:.3}",
        total_frames, 2, peak
    );
    Ok(())
}
