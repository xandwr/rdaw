use std::collections::VecDeque;

use crate::automation::Envelope;
use crate::buffer::AudioBuffer;
use crate::{AudioNode, LoopRegion, ProcessContext, Sample, Transport, TransportState};

/// Binds an [`Envelope`] to one parameter of one node. Evaluated each block at
/// the play position and applied through the node's `set_param`.
struct AutomationLane {
    node: NodeId,
    param: u32,
    envelope: Envelope,
}

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
    /// Parameter automation, evaluated at the start of every processed block.
    automation: Vec<AutomationLane>,
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
            automation: Vec::new(),
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

    /// Drive `param` of `node` from `envelope` as the transport plays. The
    /// envelope is sampled at the start of every block and applied through the
    /// node's `set_param`, so it rides whatever the parameter does manually.
    /// Control thread only. Multiple lanes may target the same parameter; they
    /// are applied in registration order (the last one wins for that block).
    pub fn automate(&mut self, node: NodeId, param: u32, envelope: Envelope) {
        self.automation.push(AutomationLane {
            node,
            param,
            envelope,
        });
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

        // Apply parameter automation for this block before any node runs. Indexed
        // so the immutable lane borrow is dropped before the mutable node borrow.
        for li in 0..self.automation.len() {
            let lane = &self.automation[li];
            if let Some(value) = lane.envelope.value_at(transport.sample_pos) {
                let (node, param) = (lane.node, lane.param);
                if let Some(n) = self.nodes.get_mut(node.0) {
                    n.set_param(param, value);
                }
            }
        }

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

    /// Offline, non-real-time render: play from frame 0 for `total_frames`,
    /// stepping the transport in `block`-sized chunks exactly as the live host
    /// would, and return the result interleaved. Allocates — this is for tests
    /// and bouncing to disk, never the audio thread. Call
    /// [`prepare`](Graph::prepare) first with `max_block >= block`.
    pub fn render_offline(
        &mut self,
        sample_rate: f64,
        total_frames: usize,
        block: usize,
    ) -> Vec<Sample> {
        let mut transport = Transport::new(120.0);
        transport.playing = true;
        self.render_with(sample_rate, total_frames, block, &mut transport)
    }

    /// Like [`render_offline`](Graph::render_offline) but with an active loop
    /// region, so the play head wraps within `loop_region` for the whole render.
    pub fn render_offline_looped(
        &mut self,
        sample_rate: f64,
        total_frames: usize,
        block: usize,
        loop_region: LoopRegion,
    ) -> Vec<Sample> {
        let mut transport = Transport::new(120.0);
        transport.playing = true;
        transport.set_loop(Some(loop_region));
        self.render_with(sample_rate, total_frames, block, &mut transport)
    }

    /// Drive the graph offline through an arbitrary [`Transport`], honoring its
    /// loop region and (sample-accurate) wrapping — the same path the live host
    /// uses. Allocates the output buffer; for tests and bouncing only.
    pub fn render_with(
        &mut self,
        sample_rate: f64,
        total_frames: usize,
        block: usize,
        transport: &mut Transport,
    ) -> Vec<Sample> {
        let channels = self.channels;
        let mut out = vec![0.0; total_frames * channels];

        let mut done = 0;
        while done < total_frames {
            let n = block.min(total_frames - done);
            let chunk = &mut out[done * channels..(done + n) * channels];
            // A single block may be split into several linear segments at the
            // loop boundary; each renders into its own slice of the chunk.
            transport.render_block(n, |state, offset, len| {
                let seg = &mut chunk[offset * channels..(offset + len) * channels];
                self.process(sample_rate, state, seg);
            });
            done += n;
        }
        out
    }
}
