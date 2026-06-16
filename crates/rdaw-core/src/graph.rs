use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext, Sample, TransportState};

/// A processing chain: nodes run in series, each reading and writing the same
/// planar scratch buffer.
///
/// This is deliberately a linear chain, not a full DAG. A real mixer needs a
/// directed graph with a topological sort and per-edge buffers; a chain is
/// enough to exercise the node trait and the RT boundary end-to-end. Swapping
/// in a DAG later only changes this type — nodes and the host are unaffected.
pub struct Graph {
    channels: usize,
    nodes: Vec<Box<dyn AudioNode>>,
    buffer: Option<AudioBuffer>,
}

impl Graph {
    pub fn new(channels: usize) -> Self {
        Self {
            channels,
            nodes: Vec::new(),
            buffer: None,
        }
    }

    /// Append a node to the chain. Control thread only.
    pub fn push(&mut self, node: Box<dyn AudioNode>) -> &mut Self {
        self.nodes.push(node);
        self
    }

    /// Allocate the scratch buffer and prepare every node. Control thread only.
    pub fn prepare(&mut self, sample_rate: f64, max_block: usize) {
        self.buffer = Some(AudioBuffer::new(self.channels, max_block));
        for node in &mut self.nodes {
            node.prepare(sample_rate, max_block);
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Render one block into `interleaved_out` (`[L, R, L, R, ...]`). RT-safe:
    /// no allocation, no locks. Must be called after [`prepare`](Graph::prepare).
    pub fn process(
        &mut self,
        sample_rate: f64,
        transport: TransportState,
        interleaved_out: &mut [Sample],
    ) {
        let buffer = self
            .buffer
            .as_mut()
            .expect("Graph::process called before prepare");

        let out_channels = self.channels;
        let frames = (interleaved_out.len() / out_channels).min(buffer.capacity());

        let ctx = ProcessContext {
            sample_rate,
            frames,
            transport,
        };

        buffer.clear();
        for node in &mut self.nodes {
            node.process(&ctx, buffer);
        }

        // Planar -> interleaved at the edge.
        for frame in 0..frames {
            for ch in 0..out_channels {
                interleaved_out[frame * out_channels + ch] = buffer.channel(ch)[frame];
            }
        }
    }
}
