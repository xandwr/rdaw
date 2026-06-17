//! Real-time host. Owns the cpal output stream and the audio callback, and
//! exposes a lock-free channel for the control thread to drive playback.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use rdaw_core::{Graph, LoopRegion, NodeId, Sample, Transport};

/// Messages from the control thread to the audio thread. Kept small and
/// `Copy` so they move through the ring buffer without allocation.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Play,
    Stop,
    /// Halt playback without moving the play head. Unlike [`Command::Stop`],
    /// which rewinds to the start like a tape transport, this is pause-in-place.
    Pause,
    /// Move the play head to an absolute timeline frame. Works whether or not
    /// transport is playing.
    Seek {
        frame: u64,
    },
    /// Enable looping over `[start, end)` timeline frames. An empty/invalid
    /// region clears the loop.
    SetLoop {
        start: u64,
        end: u64,
    },
    /// Disable looping; playback continues linearly from the current position.
    ClearLoop,
    /// Change one parameter of one node live. `param` indices are defined by the
    /// node type (e.g. `SineOsc::FREQ`).
    SetParam {
        node: NodeId,
        param: u32,
        value: f32,
    },
}

/// Capacity of the command ring buffer (messages buffered between audio blocks).
const COMMAND_CAPACITY: usize = 256;
/// Generous upper bound on frames-per-callback. cpal may pick a smaller block;
/// we never render more than this without reallocating.
const MAX_BLOCK: usize = 8192;

/// Lives entirely on the audio thread. Drains commands, runs the graph,
/// converts to the device sample type.
struct RtProcessor {
    graph: Graph,
    commands: rtrb::Consumer<Command>,
    transport: Transport,
    sample_rate: f64,
    channels: usize,
    /// Pre-allocated interleaved `f32` scratch; converted to `T` per callback.
    scratch: Vec<Sample>,
    /// Published play-head position for the control/UI thread to poll. Written
    /// once per callback with a single relaxed store; never read on this thread.
    playhead: Arc<AtomicU64>,
}

impl RtProcessor {
    fn render<T: SizedSample + FromSample<Sample>>(&mut self, output: &mut [T]) {
        // 1. Apply any pending control changes. Lock-free pop, bounded work.
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Play => self.transport.playing = true,
                Command::Stop => {
                    // Stop returns the play head to the start, like a tape
                    // transport. Use Seek for pause-in-place semantics.
                    self.transport.playing = false;
                    self.transport.set_position(0);
                }
                Command::Pause => self.transport.playing = false,
                Command::Seek { frame } => self.transport.set_position(frame),
                Command::SetLoop { start, end } => {
                    self.transport.set_loop(Some(LoopRegion::new(start, end)))
                }
                Command::ClearLoop => self.transport.set_loop(None),
                Command::SetParam { node, param, value } => {
                    self.graph.set_param(node, param, value)
                }
            }
        }

        let frames = output.len() / self.channels;
        let scratch = &mut self.scratch[..frames * self.channels];

        if self.transport.playing {
            // Disjoint field borrows so the render closure can hold the graph
            // while the transport drives segmentation.
            let graph = &mut self.graph;
            let sample_rate = self.sample_rate;
            let channels = self.channels;
            self.transport.render_block(frames, |state, offset, len| {
                let seg = &mut scratch[offset * channels..(offset + len) * channels];
                graph.process(sample_rate, state, seg);
            });
        } else {
            scratch.fill(0.0);
        }

        // 2. Convert to the device's native sample format at the edge.
        for (dst, &src) in output.iter_mut().zip(scratch.iter()) {
            *dst = T::from_sample(src);
        }

        // 3. Publish the play head for the UI to poll. Relaxed is fine: this is a
        // monotonic display value, not a synchronization point.
        self.playhead
            .store(self.transport.position(), Ordering::Relaxed);
    }
}

/// Handle to a running audio engine. Dropping it stops the stream.
pub struct Engine {
    _stream: cpal::Stream,
    commands: rtrb::Producer<Command>,
    /// Shared with the audio thread; read with [`Engine::playhead`].
    playhead: Arc<AtomicU64>,
}

impl Engine {
    /// Open the default output device and start streaming `graph`.
    pub fn new(graph: Graph) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default output device"))?;
        let supported = device
            .default_output_config()
            .context("querying default output config")?;

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate as f64;

        if graph.channels() != channels {
            return Err(anyhow!(
                "graph has {} channels but device wants {}",
                graph.channels(),
                channels
            ));
        }

        let (tx, rx) = rtrb::RingBuffer::<Command>::new(COMMAND_CAPACITY);

        let mut graph = graph;
        graph.prepare(sample_rate, MAX_BLOCK);

        let playhead = Arc::new(AtomicU64::new(0));

        let mut proc = RtProcessor {
            graph,
            commands: rx,
            transport: Transport::default(),
            sample_rate,
            channels,
            scratch: vec![0.0; MAX_BLOCK * channels],
            playhead: Arc::clone(&playhead),
        };

        let err_fn = |e| eprintln!("audio stream error: {e}");
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |out: &mut [f32], _| proc.render(out),
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config,
                move |out: &mut [i16], _| proc.render(out),
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                config,
                move |out: &mut [u16], _| proc.render(out),
                err_fn,
                None,
            ),
            other => return Err(anyhow!("unsupported sample format: {other:?}")),
        }
        .context("building output stream")?;

        stream.play().context("starting stream")?;

        Ok(Self {
            _stream: stream,
            commands: tx,
            playhead,
        })
    }

    /// Send a command to the audio thread. Non-blocking; drops the command if
    /// the ring buffer is full (the control thread must not stall on audio).
    pub fn send(&mut self, cmd: Command) {
        let _ = self.commands.push(cmd);
    }

    /// Current play-head position in timeline frames, as last published by the
    /// audio thread. Lock-free; poll this each UI frame to draw the play head.
    /// Lags real output by at most one callback (a few ms), which is below the
    /// threshold of visible drift for a timeline cursor.
    pub fn playhead(&self) -> u64 {
        self.playhead.load(Ordering::Relaxed)
    }
}
