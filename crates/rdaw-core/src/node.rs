use crate::{AudioBuffer, TransportState};

/// Everything a node needs to know to render one block.
#[derive(Clone, Copy, Debug)]
pub struct ProcessContext {
    pub sample_rate: f64,
    /// Number of valid frames in this block (`<= buffer.capacity()`).
    pub frames: usize,
    pub transport: TransportState,
}

/// A unit of audio processing. Nodes are built and prepared on the control
/// thread, then executed on the real-time thread.
///
/// The contract that makes this a DAW and not a toy:
/// - [`prepare`](AudioNode::prepare) may allocate, lock, do I/O.
/// - [`process`](AudioNode::process) **must not**: no allocation, no locks,
///   no syscalls. It runs inside the audio callback where blocking causes
///   audible dropouts.
pub trait AudioNode: Send {
    /// Called off the RT thread whenever sample rate, block size, or channel
    /// count changes. `channels` is the graph's channel count: the width of the
    /// buffers this node will be handed: so a node that keeps per-channel state
    /// (a filter's delay line, say) can allocate it here rather than on the RT
    /// thread.
    fn prepare(&mut self, sample_rate: f64, max_block: usize, channels: usize);

    /// Render `ctx.frames` frames. `input` is the summed mix of every upstream
    /// node (silence for a source); write the result into `output` (planar).
    /// Generators ignore `input`; effects read it. RT-safe.
    fn process(&mut self, ctx: &ProcessContext, input: &AudioBuffer, output: &mut AudioBuffer);

    /// Apply a live parameter change addressed by index. Called on the RT thread
    /// from the command queue, so it must not allocate or block. Parameter
    /// indices are defined per node type (see the node's associated constants).
    fn set_param(&mut self, _param: u32, _value: f32) {}
}
