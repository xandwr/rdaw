use std::thread::sleep;
use std::time::Duration;

use rdaw_core::Graph;
use rdaw_core::nodes::{Gain, SineOsc};
use rdaw_engine::{Command, Engine};

fn main() -> anyhow::Result<()> {
    // Build the graph on the control thread: 440 Hz sine -> gain.
    let mut graph = Graph::new(2);
    graph.push(Box::new(SineOsc::new(440.0, 0.5)));
    graph.push(Box::new(Gain::new(0.25)));

    // Hand it to the engine; cpal starts pulling immediately.
    let mut engine = Engine::new(graph)?;

    println!("playing 440 Hz for 2s...");
    engine.send(Command::Play);
    sleep(Duration::from_secs(2));

    // Drop the master gain, then stop.
    engine.send(Command::SetMasterGain(0.3));
    sleep(Duration::from_millis(500));
    engine.send(Command::Stop);
    sleep(Duration::from_millis(100));

    println!("done");
    Ok(())
}
