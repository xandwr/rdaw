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
