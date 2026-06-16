use std::thread::sleep;
use std::time::Duration;

use rdaw_core::Graph;
use rdaw_core::nodes::{Gain, SineOsc};
use rdaw_engine::{Command, Engine};

fn main() -> anyhow::Result<()> {
    // graph: osc -> master gain -> device
    let mut graph = Graph::new(2);
    let osc = graph.add(Box::new(SineOsc::new(440.0, 0.5)));
    let master = graph.add(Box::new(Gain::new(0.25)));
    graph.connect(osc, master);
    graph.set_master(master);

    let mut engine = Engine::new(graph)?;
    engine.send(Command::Play);

    println!("sweep up to 880 Hz...");
    for freq in (440..=880).step_by(5) {
        engine.send(Command::SetParam {
            node: osc,
            param: SineOsc::FREQ,
            value: freq as f32,
        });
        sleep(Duration::from_millis(15));
    }

    println!("fade out via master gain...");
    for step in 0..=20 {
        let g = 0.25 * (1.0 - step as f32 / 20.0);
        engine.send(Command::SetParam {
            node: master,
            param: Gain::GAIN,
            value: g,
        });
        sleep(Duration::from_millis(25));
    }

    engine.send(Command::Stop);
    sleep(Duration::from_millis(100));
    println!("done");
    Ok(())
}
