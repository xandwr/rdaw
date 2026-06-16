//! Pure DSP data model for the DAW: audio buffers, the node trait, the
//! process graph, transport, and a handful of built-in nodes.
//!
//! This crate has **no** audio-backend dependency (no cpal). Everything here
//! can be driven from a test or an offline renderer. The real-time host lives
//! in `rdaw-engine`.

pub mod automation;
pub mod buffer;
pub mod graph;
pub mod node;
pub mod nodes;
pub mod project;
pub mod tempo;
pub mod timeline;
pub mod transport;

/// Sample format used throughout the engine. Planar `f32` internally; the host
/// converts to whatever the device wants at the very edge.
pub type Sample = f32;

pub use automation::{Envelope, Interp};
pub use buffer::AudioBuffer;
pub use graph::{Graph, NodeId};
pub use node::{AudioNode, ProcessContext};
pub use project::{AutomationLane, AutomationTarget, ClipData, Project, Source, Time, Track};
pub use tempo::{MusicalTime, TimeSignature};
pub use timeline::{Clip, FadeCurve, Timeline, Waveform};
pub use transport::{LoopRegion, Transport, TransportState};
