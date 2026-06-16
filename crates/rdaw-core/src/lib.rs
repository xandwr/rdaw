//! Pure DSP data model for the DAW: audio buffers, the node trait, the
//! process graph, transport, and a handful of built-in nodes.
//!
//! This crate has **no** audio-backend dependency (no cpal). Everything here
//! can be driven from a test or an offline renderer. The real-time host lives
//! in `rdaw-engine`.

pub mod buffer;
pub mod graph;
pub mod node;
pub mod nodes;
pub mod transport;

/// Sample format used throughout the engine. Planar `f32` internally; the host
/// converts to whatever the device wants at the very edge.
pub type Sample = f32;

pub use buffer::AudioBuffer;
pub use graph::{Graph, NodeId};
pub use node::{AudioNode, ProcessContext};
pub use transport::TransportState;
