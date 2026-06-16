use std::f64::consts::TAU;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use rdaw_core::{ClipData, MusicalTime, Project, Track, Waveform};
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
    // Source: a WAV passed as the first arg, otherwise a synthesized tone. We
    // keep both the decoded audio and the path it came from — the project stores
    // the path, the graph is fed the decoded samples.
    let (source, sr, source_path) = match std::env::args().nth(1) {
        Some(path) => {
            let loaded = read_wav(&path)?;
            println!("loaded {path} ({} Hz)", loaded.sample_rate);
            (loaded.waveform, loaded.sample_rate as f64, path)
        }
        None => {
            println!("no WAV given; synthesizing a 440 Hz tone");
            (
                Arc::new(sine_burst(440.0, 0.5, 0.4, ASSUMED_SR)),
                ASSUMED_SR,
                "<synth-440hz>".to_string(),
            )
        }
    };

    let clip_len = source.frames() as u64;

    // Describe the arrangement as a document: one source, two tracks that place
    // it differently and pan to opposite sides, summing into an 0.8 master. The
    // "lead" sits at frame 0; the "echo" is placed *musically* on beat 3 of the
    // first bar (zero-based beat 2), so it tracks the tempo rather than a frame.
    let mut project = Project::new(120.0); // 4/4 by default
    project.master_gain = 0.8;
    let src = project.add_source(&source_path);
    project.add_track(
        Track::new("lead")
            .with_pan(-0.4)
            .with_clip(ClipData::new(src, 0u64, clip_len)),
    );
    project.add_track(
        Track::new("echo")
            .with_gain(0.5)
            .with_pan(0.4)
            .with_clip(ClipData::new(src, MusicalTime::bar_beat(0, 2), clip_len)),
    );

    // Round-trip the document through a JSON file to prove it persists.
    println!("\nproject:\n{}", project.to_json()?);
    let mut path = std::env::temp_dir();
    path.push("rdaw_demo_project.json");
    project.save(&path)?;
    let project = Project::load(&path)?;
    println!("saved + reloaded {}\n", path.display());

    // Compile the (reloaded) document into a runnable graph. The decoded source
    // is supplied in index order; here we have just the one. Musical positions
    // resolve to frames here using the project's tempo + meter at this rate.
    let graph = project.build_graph(2, sr, &[source]);
    let mut engine = Engine::new(graph)?;

    println!("play from the top...");
    engine.send(Command::Play);
    sleep(Duration::from_millis(1400));

    println!("seek back to the first clip and replay it...");
    engine.send(Command::Seek { frame: 0 });
    sleep(Duration::from_millis((clip_len as f64 / sr * 1000.0) as u64));

    // Loop the first beat (musically defined) a few times, then drop the loop.
    // The bound is computed from bars/beats via the project, not hand-counted
    // frames — the same conversion the clips above went through.
    let loop_end = project.frames_at(MusicalTime::bar_beat(0, 1), sr);
    println!("loop the first beat (frames 0..{loop_end}) a few times...");
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
