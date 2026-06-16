use std::collections::VecDeque;

use crate::buffer::AudioBuffer;
use crate::{AudioNode, ProcessContext, Sample, TransportState};

/// Stable handle to a node in the [`Graph`]. Returned by [`Graph::add`] and used
/// to wire connections and address live parameter changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

/// A directed audio graph: nodes wired by connections, evaluated in topological
/// order. Each node owns one output buffer; a node's input is the sum of every
/// upstream node's output. This is what lets multiple tracks feed a master bus,
/// or a send feed a reverb — things a linear chain can't express.
///
/// Topology (add/connect/topo-sort) and all allocation happen on the control
/// thread. [`process`](Graph::process) is the only RT-thread entry point and
/// neither allocates nor locks.
pub struct Graph {
    channels: usize,
    nodes: Vec<Box<dyn AudioNode>>,
    /// One output buffer per node, parallel to `nodes`. Allocated in `prepare`.
    outputs: Vec<AudioBuffer>,
    /// Directed edges `(from, to)`: `from`'s output sums into `to`'s input.
    edges: Vec<(NodeId, NodeId)>,
    /// Node evaluation order, topologically sorted. Computed in `prepare`.
    order: Vec<NodeId>,
    /// Scratch bus that a node's inputs are summed into before processing.
    input_bus: AudioBuffer,
    /// The node whose output is sent to the device.
    master: Option<NodeId>,
}

impl Graph {
    pub fn new(channels: usize) -> Self {
        Self {
            channels,
            nodes: Vec::new(),
            outputs: Vec::new(),
            edges: Vec::new(),
            order: Vec::new(),
            input_bus: AudioBuffer::new(channels, 0),
            master: None,
        }
    }

    /// Add a node and return its handle. Control thread only.
    pub fn add(&mut self, node: Box<dyn AudioNode>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(node);
        id
    }

    /// Wire `from`'s output into `to`'s input. Control thread only; takes effect
    /// after the next [`prepare`](Graph::prepare).
    pub fn connect(&mut self, from: NodeId, to: NodeId) {
        self.edges.push((from, to));
    }

    /// Choose the node whose output is streamed to the device. Control thread only.
    pub fn set_master(&mut self, node: NodeId) {
        self.master = Some(node);
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Allocate per-node buffers, topologically sort, and prepare every node.
    /// Control thread only.
    pub fn prepare(&mut self, sample_rate: f64, max_block: usize) {
        self.outputs = (0..self.nodes.len())
            .map(|_| AudioBuffer::new(self.channels, max_block))
            .collect();
        self.input_bus = AudioBuffer::new(self.channels, max_block);
        self.recompute_order();
        for node in &mut self.nodes {
            node.prepare(sample_rate, max_block);
        }
    }

    /// Kahn's algorithm. Nodes left out by a cycle are dropped from the order
    /// (and so render silence) rather than panicking. Control thread only.
    fn recompute_order(&mut self) {
        let n = self.nodes.len();
        let mut indegree = vec![0usize; n];
        for &(_, to) in &self.edges {
            indegree[to.0] += 1;
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(u) = queue.pop_front() {
            order.push(NodeId(u));
            for &(from, to) in &self.edges {
                if from.0 == u {
                    indegree[to.0] -= 1;
                    if indegree[to.0] == 0 {
                        queue.push_back(to.0);
                    }
                }
            }
        }
        self.order = order;
    }

    /// Apply a live parameter change to one node. RT-safe; out-of-range handles
    /// are ignored rather than panicking on the audio thread.
    pub fn set_param(&mut self, node: NodeId, param: u32, value: f32) {
        if let Some(n) = self.nodes.get_mut(node.0) {
            n.set_param(param, value);
        }
    }

    /// Render one block into `interleaved_out` (`[L, R, L, R, ...]`). RT-safe:
    /// no allocation, no locks. Must be called after [`prepare`](Graph::prepare).
    pub fn process(
        &mut self,
        sample_rate: f64,
        transport: TransportState,
        interleaved_out: &mut [Sample],
    ) {
        let out_channels = self.channels;
        let capacity = self.input_bus.capacity();
        let frames = (interleaved_out.len() / out_channels).min(capacity);

        let ctx = ProcessContext {
            sample_rate,
            frames,
            transport,
        };

        // Evaluate nodes in dependency order. Each is processed exactly once and
        // only after its upstreams, so reading their outputs here is sound.
        for idx in 0..self.order.len() {
            let id = self.order[idx];

            // Sum all incoming edges into the shared input bus.
            self.input_bus.clear();
            for edge_idx in 0..self.edges.len() {
                let (from, to) = self.edges[edge_idx];
                if to == id {
                    // Disjoint fields: immutable `outputs`, mutable `input_bus`.
                    self.input_bus.add_from(&self.outputs[from.0], frames);
                }
            }

            let node = &mut self.nodes[id.0];
            let output = &mut self.outputs[id.0];
            node.process(&ctx, &self.input_bus, output);
        }

        // Planar -> interleaved at the edge, from the master node's output.
        match self.master {
            Some(master) => {
                let buf = &self.outputs[master.0];
                for frame in 0..frames {
                    for ch in 0..out_channels {
                        interleaved_out[frame * out_channels + ch] = buf.channel(ch)[frame];
                    }
                }
            }
            None => interleaved_out.fill(0.0),
        }
    }
}
