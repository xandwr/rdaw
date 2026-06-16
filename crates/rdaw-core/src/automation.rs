//! Parameter automation: time-varying values that drive node parameters as the
//! transport plays.
//!
//! So far a parameter only changes through a one-shot
//! [`set_param`](crate::node::AudioNode::set_param) — fine for "turn this knob
//! now", useless for "ride the master fader down across the last bar". An
//! [`Envelope`] closes that gap: a sorted list of breakpoints in timeline frames
//! that the [`Graph`](crate::graph::Graph) samples each block and feeds into the
//! very same `set_param`, so nodes need no special support.
//!
//! Evaluation is **control-rate**: the envelope is read once per processed block,
//! at the block's start frame, and held for the block. Rendering at a smaller
//! block size therefore gives finer automation resolution; at block size 1 it is
//! sample-accurate. This keeps the real-time path allocation- and branch-light
//! while leaving sample-accurate ramping as a later refinement.

use serde::{Deserialize, Serialize};

/// How an [`Envelope`] bridges the gap between two breakpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interp {
    /// Hold the earlier breakpoint's value until the next one (a stepped knob).
    Step,
    /// Straight-line ramp from one breakpoint to the next.
    #[default]
    Linear,
}

/// A sorted set of `(frame, value)` breakpoints describing how one parameter
/// moves over time. Before the first point and after the last it holds the
/// nearest value (no extrapolation); in between it interpolates per [`Interp`].
#[derive(Clone, Debug, Default)]
pub struct Envelope {
    /// Breakpoints, kept sorted by frame. Built on the control thread.
    points: Vec<(u64, f32)>,
    interp: Interp,
}

impl Envelope {
    /// An empty envelope with the given interpolation. Empty envelopes evaluate
    /// to `None` and so leave their target parameter untouched.
    pub fn new(interp: Interp) -> Self {
        Self {
            points: Vec::new(),
            interp,
        }
    }

    /// Build directly from `(frame, value)` pairs in any order. Control thread
    /// only (it sorts).
    pub fn from_points(interp: Interp, points: impl IntoIterator<Item = (u64, f32)>) -> Self {
        let mut env = Self::new(interp);
        for (frame, value) in points {
            env.add_point(frame, value);
        }
        env
    }

    /// Insert a breakpoint, keeping the list sorted. A second point at the same
    /// frame overwrites the first (so an envelope never holds a discontinuity by
    /// accident). Control thread only.
    pub fn add_point(&mut self, frame: u64, value: f32) {
        match self.points.binary_search_by(|p| p.0.cmp(&frame)) {
            Ok(i) => self.points[i].1 = value,
            Err(i) => self.points.insert(i, (frame, value)),
        }
    }

    /// Builder form of [`add_point`](Envelope::add_point).
    pub fn with_point(mut self, frame: u64, value: f32) -> Self {
        self.add_point(frame, value);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The envelope's value at timeline `frame`, or `None` if it has no points.
    /// Real-time safe: a binary search over the breakpoints, no allocation.
    pub fn value_at(&self, frame: u64) -> Option<f32> {
        if self.points.is_empty() {
            return None;
        }
        // Index of the first point at or after `frame`.
        match self.points.binary_search_by(|p| p.0.cmp(&frame)) {
            // Exactly on a breakpoint.
            Ok(i) => Some(self.points[i].1),
            // Before the first point: hold the first value.
            Err(0) => Some(self.points[0].1),
            Err(i) if i >= self.points.len() => {
                // Past the last point: hold the last value.
                Some(self.points[self.points.len() - 1].1)
            }
            // Between points i-1 and i.
            Err(i) => {
                let (f0, v0) = self.points[i - 1];
                let (f1, v1) = self.points[i];
                match self.interp {
                    Interp::Step => Some(v0),
                    Interp::Linear => {
                        let t = (frame - f0) as f32 / (f1 - f0) as f32;
                        Some(v0 + (v1 - v0) * t)
                    }
                }
            }
        }
    }
}
