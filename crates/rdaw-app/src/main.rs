use std::f64::consts::TAU;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use rdaw_core::nodes::Gain;
use rdaw_core::{Clip, Graph, Waveform};
use rdaw_core::Timeline;
use rdaw_engine::{Command, Engine};
use rdaw_io::read_wav;

/// We synthesize sources at this rate when no file is supplied. The live device
/// may run at a different rate; resampling is a later concern.
const ASSUMED_SR: f64 = 44_100.0;

/// A mono sine burst, used when no WAV path is given on the command line.
fn sine_burst(freq_hz: f64, seconds: f64, amp: f32, sample_rate: f64) -> Waveform {
    let frames = (seconds * sample_rate) as usize;
    let inc = TAU * freq_hz / sample_rate;
    let data = (0..frames)
        .map(|i| ((i as f64 * inc).sin() as f32) * amp)
        .collect();
    Waveform::from_planar(1, data)
}

fn main() -> anyhow::Result<()> {
    // Source: a WAV passed as the first arg, otherwise a synthesized tone.
    let (source, sr) = match std::env::args().nth(1) {
        Some(path) => {
            let loaded = read_wav(&path)?;
            println!("loaded {path} ({} Hz)", loaded.sample_rate);
            (loaded.waveform, loaded.sample_rate as f64)
        }
        None => {
            println!("no WAV given; synthesizing a 440 Hz tone");
            (Arc::new(sine_burst(440.0, 0.5, 0.4, ASSUMED_SR)), ASSUMED_SR)
        }
    };

    let clip_len = source.frames() as u64;

    // timeline (two placements of the source) -> master gain -> device.
    let timeline = Timeline::new()
        .with_clip(Clip::new(source.clone(), 0))
        .with_clip(Clip::new(source, (sr * 0.75) as u64).with_gain(0.5));

    let mut graph = Graph::new(2);
    let _tl = graph.add(Box::new(timeline));
    let master = graph.add(Box::new(Gain::new(0.8)));
    graph.connect(_tl, master);
    graph.set_master(master);

    let mut engine = Engine::new(graph)?;

    println!("play from the top...");
    engine.send(Command::Play);
    sleep(Duration::from_millis(1400));

    println!("seek back to the first clip and replay it...");
    engine.send(Command::Seek { frame: 0 });
    sleep(Duration::from_millis((clip_len as f64 / sr * 1000.0) as u64));

    // Loop the first half of the source four times, then drop the loop.
    let loop_end = clip_len / 2;
    println!("loop frames 0..{loop_end} a few times...");
    engine.send(Command::Seek { frame: 0 });
    engine.send(Command::SetLoop {
        start: 0,
        end: loop_end,
    });
    sleep(Duration::from_millis(
        (loop_end as f64 / sr * 1000.0 * 4.0) as u64,
    ));

    println!("clear the loop and let it play out...");
    engine.send(Command::ClearLoop);
    sleep(Duration::from_millis(400));

    engine.send(Command::Stop);
    sleep(Duration::from_millis(100));
    println!("done");
    Ok(())
}
