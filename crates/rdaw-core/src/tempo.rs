//! Musical time: positions and durations expressed in bars and beats rather
//! than raw frames.
//!
//! Frames are what the engine ultimately plays, but they're the wrong unit to
//! *author* in — a downbeat lands on "bar 5, beat 1", not "frame 441000", and it
//! should stay on that downbeat when you change the tempo. This module is the
//! bridge: a [`MusicalTime`] plus a [`TimeSignature`] and a tempo convert to a
//! frame on demand, so the project can store music and the graph can be handed
//! frames (see [`build_graph`](crate::project::Project::build_graph)).
//!
//! Conventions, fixed here so the conversions are unambiguous:
//! - Tempo (`bpm`) counts **quarter notes** per minute, matching
//!   [`TransportState::beats`](crate::transport::TransportState::beats).
//! - In an `n/d` signature a bar holds `n` beats and each beat is a `1/d` note,
//!   i.e. `4/d` quarter notes (a `1/8` note is half a quarter, a `1/2` note is
//!   two). So a 4/4 bar is 4 quarters and a 6/8 bar is 3 quarters.
//! - Bars, beats and ticks are all **zero-based**: bar 0 / beat 0 / tick 0 is
//!   the very start of the timeline. (A UI is free to display them 1-based.)

use serde::{Deserialize, Serialize};

/// Tick resolution within a single beat. A beat is one denominator-note of the
/// time signature; ticks subdivide it for sub-beat placement. 960 is the usual
/// sequencer PPQ and divides cleanly by the common note fractions.
pub const TICKS_PER_BEAT: u32 = 960;

/// A time signature `numerator/denominator`, e.g. 4/4 or 6/8. The numerator is
/// beats per bar; the denominator is the note value that counts as one beat.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::new(4, 4)
    }
}

impl TimeSignature {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// How many quarter notes one beat of this signature spans (a `1/d` note is
    /// `4/d` quarters).
    pub fn quarters_per_beat(&self) -> f64 {
        4.0 / self.denominator as f64
    }

    /// How many quarter notes one whole bar spans.
    pub fn quarters_per_bar(&self) -> f64 {
        self.numerator as f64 * self.quarters_per_beat()
    }
}

/// A position (or, measured from the origin, a duration) on the musical grid:
/// `bar`, `beat` within the bar, and `tick` within the beat — all zero-based.
///
/// It carries no tempo or meter of its own; pair it with a [`TimeSignature`] and
/// a tempo via [`to_frames`](MusicalTime::to_frames) to land on an actual frame.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MusicalTime {
    pub bar: u32,
    pub beat: u32,
    pub tick: u32,
}

impl MusicalTime {
    pub const fn new(bar: u32, beat: u32, tick: u32) -> Self {
        Self { bar, beat, tick }
    }

    /// The downbeat of `bar` (beat 0, tick 0).
    pub const fn bars(bar: u32) -> Self {
        Self::new(bar, 0, 0)
    }

    /// `bar` and `beat`, on the beat (tick 0).
    pub const fn bar_beat(bar: u32, beat: u32) -> Self {
        Self::new(bar, beat, 0)
    }

    /// This position as a count of beats under `sig` (bars unrolled into beats,
    /// ticks as the fractional part).
    pub fn to_beats(self, sig: TimeSignature) -> f64 {
        self.bar as f64 * sig.numerator as f64
            + self.beat as f64
            + self.tick as f64 / TICKS_PER_BEAT as f64
    }

    /// This position in quarter notes under `sig`.
    pub fn to_quarters(self, sig: TimeSignature) -> f64 {
        self.to_beats(sig) * sig.quarters_per_beat()
    }

    /// Resolve to a timeline frame given the meter, tempo (quarter-note BPM), and
    /// sample rate. Rounds to the nearest whole frame.
    pub fn to_frames(self, sig: TimeSignature, bpm: f64, sample_rate: f64) -> u64 {
        let frames_per_quarter = sample_rate * 60.0 / bpm;
        (self.to_quarters(sig) * frames_per_quarter).round() as u64
    }
}
