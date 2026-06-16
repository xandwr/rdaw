/// Playback state handed to every node each block. Cheap to copy.
#[derive(Clone, Copy, Debug)]
pub struct TransportState {
    pub playing: bool,
    /// Sample position of the *first* frame in the current block since play start.
    pub sample_pos: u64,
    pub tempo_bpm: f64,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: false,
            sample_pos: 0,
            tempo_bpm: 120.0,
        }
    }
}

impl TransportState {
    /// Musical position of the block start, in quarter-note beats.
    pub fn beats(&self, sample_rate: f64) -> f64 {
        let seconds = self.sample_pos as f64 / sample_rate;
        seconds * (self.tempo_bpm / 60.0)
    }
}

/// A half-open loop window `[start, end)` in timeline frames. Considered active
/// only when `end > start`; anything else is treated as "no loop".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRegion {
    pub start: u64,
    pub end: u64,
}

impl LoopRegion {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Length in frames (0 if the region is empty/invalid).
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// The live playback controller. Owns the authoritative play position and the
/// loop region, and knows how to advance — wrapping at the loop boundary,
/// splitting a block when the wrap lands mid-block. Nodes never see this; they
/// only get the per-segment [`TransportState`] snapshot it hands out, so looping
/// stays entirely a scheduling concern.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    pub playing: bool,
    pub tempo_bpm: f64,
    position: u64,
    loop_region: Option<LoopRegion>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            playing: false,
            tempo_bpm: 120.0,
            position: 0,
            loop_region: None,
        }
    }
}

impl Transport {
    pub fn new(tempo_bpm: f64) -> Self {
        Self {
            tempo_bpm,
            ..Self::default()
        }
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    /// Move the play head (seek). Safe to call while playing.
    pub fn set_position(&mut self, position: u64) {
        self.position = position;
    }

    pub fn loop_region(&self) -> Option<LoopRegion> {
        self.loop_region
    }

    /// Set or clear the loop window. An empty/invalid region clears the loop.
    pub fn set_loop(&mut self, region: Option<LoopRegion>) {
        self.loop_region = region.filter(|r| !r.is_empty());
    }

    /// The snapshot handed to nodes for a segment starting at the current
    /// position.
    fn snapshot(&self) -> TransportState {
        TransportState {
            playing: self.playing,
            sample_pos: self.position,
            tempo_bpm: self.tempo_bpm,
        }
    }

    /// Render `frames` of output by calling `render(state, offset, len)` once per
    /// contiguous linear segment, advancing the play head and wrapping at the
    /// loop boundary. `offset` is the segment's start within this block; `len`
    /// its length. A block straddling the loop end is split, so the wrap is
    /// sample-accurate. RT-safe (no allocation) as long as `render` is.
    pub fn render_block<F>(&mut self, frames: usize, mut render: F)
    where
        F: FnMut(TransportState, usize, usize),
    {
        // If the head sits at or past an active loop's end, pull it to the start
        // before rendering anything.
        if let Some(lr) = self.loop_region
            && !lr.is_empty()
            && self.position >= lr.end
        {
            self.position = lr.start;
        }

        let mut offset = 0;
        let mut remaining = frames;
        while remaining > 0 {
            // How many frames until we'd hit the loop end (whole block if no loop
            // is active or we're outside it).
            let seg = match self.loop_region {
                Some(lr) if !lr.is_empty() && self.position < lr.end => {
                    ((lr.end - self.position) as usize).min(remaining)
                }
                _ => remaining,
            };

            render(self.snapshot(), offset, seg);

            self.position += seg as u64;
            offset += seg;
            remaining -= seg;

            if let Some(lr) = self.loop_region
                && !lr.is_empty()
                && self.position >= lr.end
            {
                self.position = lr.start;
            }
        }
    }
}
