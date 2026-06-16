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
/// - [`process`](AudioNode::process) **must not** — no allocation, no locks,
///   no syscalls. It runs inside the audio callback where blocking causes
///   audible dropouts.
pub trait AudioNode: Send {
    /// Called off the RT thread whenever sample rate or block size changes.
    fn prepare(&mut self, sample_rate: f64, max_block: usize);

    /// Render `ctx.frames` frames into `buffer` (planar). Generators overwrite;
    /// effects read and write in place. RT-safe.
    fn process(&mut self, ctx: &ProcessContext, buffer: &mut AudioBuffer);
}
