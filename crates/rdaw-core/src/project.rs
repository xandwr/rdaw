//! The project: the persistent arrangement a session saves to disk and a GUI
//! binds to.
//!
//! Everything in the engine so far has been runtime state — graphs, nodes,
//! buffers — built up imperatively. A [`Project`] is the other half: a plain,
//! serializable description of *what the music is* (tempo, tracks, where each
//! clip sits) that knows nothing about buffers or the audio thread. Load one,
//! call [`Project::build_graph`], and you get a runnable [`Graph`]; the project
//! itself stays a document you can round-trip through JSON.
//!
//! Audio files are referenced, not embedded. The project lists its [`Source`]s
//! by path and clips point at them by index, so the document stays small and the
//! PCM lives in its own files. Decoding those paths into [`Waveform`]s is the
//! caller's job (see `rdaw-io`) — this crate stays free of any codec dependency,
//! exactly as `lib.rs` promises.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::automation::{Envelope, Interp};
use crate::nodes::{Channel, Gain};
use crate::tempo::{MusicalTime, TimeSignature};
use crate::timeline::FadeCurve;
use crate::{Clip, Graph, NodeId, Timeline, Waveform};

/// A point on (or, from the origin, a span of) the timeline, authored either as
/// a raw frame count or musically as bars/beats. Musical values are resolved to
/// frames against the project's tempo and meter when the graph is built, so they
/// follow tempo changes; frame values are absolute.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Time {
    /// An absolute position/length in sample frames.
    Frames(u64),
    /// A musical position/length, resolved with the project's tempo + meter.
    Musical(MusicalTime),
}

impl Time {
    /// Resolve to a frame count under the given meter, tempo, and sample rate.
    fn to_frames(self, sig: TimeSignature, bpm: f64, sample_rate: f64) -> u64 {
        match self {
            Time::Frames(f) => f,
            Time::Musical(m) => m.to_frames(sig, bpm, sample_rate),
        }
    }
}

impl Default for Time {
    /// Frame zero / zero length — the neutral value for an absent fade.
    fn default() -> Self {
        Time::Frames(0)
    }
}

impl From<u64> for Time {
    fn from(frames: u64) -> Self {
        Time::Frames(frames)
    }
}

impl From<MusicalTime> for Time {
    fn from(m: MusicalTime) -> Self {
        Time::Musical(m)
    }
}

/// An audio file the project depends on. Stored as a path; the actual samples
/// are loaded separately at [`build_graph`](Project::build_graph) time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    /// Path to the audio file, relative to the project or absolute.
    pub path: String,
}

impl Source {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// A clip placed on a track. The serializable twin of the runtime [`Clip`]: it
/// names its audio by index into [`Project::sources`] instead of holding an
/// `Arc<Waveform>`, so it can be written to disk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipData {
    /// Index into [`Project::sources`] of the audio this clip plays.
    pub source: usize,
    /// Where the clip starts sounding on the timeline (frames or bars/beats).
    pub start: Time,
    /// First frame of the source to play (lets a clip start partway in). This is
    /// an offset into the *source audio*, so it stays in frames.
    pub source_offset: u64,
    /// How long the clip plays (frames or a musical duration).
    pub len: Time,
    /// Linear gain applied to this clip's contribution.
    pub gain: f32,
    /// Fade-in length (frames or a musical duration). Defaulted so projects
    /// saved before fades existed still load.
    #[serde(default)]
    pub fade_in: Time,
    /// Fade-out length (frames or a musical duration).
    #[serde(default)]
    pub fade_out: Time,
    /// The curve both fades follow.
    #[serde(default)]
    pub fade_curve: FadeCurve,
}

impl ClipData {
    /// Play `source` for `len` starting at `start`. Both positions accept either
    /// a frame count (`u64`) or a [`MusicalTime`](crate::tempo::MusicalTime), via
    /// `into()`:
    ///
    /// ```
    /// # use rdaw_core::{ClipData, MusicalTime};
    /// // frame-accurate
    /// ClipData::new(0, 44_100, 22_050);
    /// // two bars in, one bar long
    /// ClipData::new(0, MusicalTime::bars(2), MusicalTime::bars(1));
    /// ```
    pub fn new(source: usize, start: impl Into<Time>, len: impl Into<Time>) -> Self {
        Self {
            source,
            start: start.into(),
            source_offset: 0,
            len: len.into(),
            gain: 1.0,
            fade_in: Time::default(),
            fade_out: Time::default(),
            fade_curve: FadeCurve::default(),
        }
    }

    /// Builder: play only `[offset, offset + len)` of the source. `offset` is in
    /// source frames; `len` may be musical.
    pub fn with_source_range(mut self, offset: u64, len: impl Into<Time>) -> Self {
        self.source_offset = offset;
        self.len = len.into();
        self
    }

    /// Builder: scale this clip's level.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Builder: fade in over `len` (frames or a musical duration).
    pub fn with_fade_in(mut self, len: impl Into<Time>) -> Self {
        self.fade_in = len.into();
        self
    }

    /// Builder: fade out over `len` (frames or a musical duration).
    pub fn with_fade_out(mut self, len: impl Into<Time>) -> Self {
        self.fade_out = len.into();
        self
    }

    /// Builder: set the curve both fades follow.
    pub fn with_fade_curve(mut self, curve: FadeCurve) -> Self {
        self.fade_curve = curve;
        self
    }

    /// Resolve to a runtime [`Clip`] against an already-decoded source list,
    /// converting any musical positions with the project's meter/tempo and the
    /// device sample rate. Returns `None` if the source index is out of range.
    fn to_clip(
        &self,
        sources: &[Arc<Waveform>],
        sig: TimeSignature,
        bpm: f64,
        sample_rate: f64,
    ) -> Option<Clip> {
        let source = sources.get(self.source)?.clone();
        Some(Clip {
            source,
            start: self.start.to_frames(sig, bpm, sample_rate),
            source_offset: self.source_offset,
            len: self.len.to_frames(sig, bpm, sample_rate),
            gain: self.gain,
            fade_in: self.fade_in.to_frames(sig, bpm, sample_rate),
            fade_out: self.fade_out.to_frames(sig, bpm, sample_rate),
            fade_curve: self.fade_curve,
        })
    }
}

/// A track: a named lane of clips with its own level and pan. Compiles to a
/// [`Timeline`] feeding a [`Channel`] strip when the graph is built.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub name: String,
    /// Linear track gain.
    pub gain: f32,
    /// Pan position in `[-1, 1]` (`-1` left, `0` center, `1` right).
    pub pan: f32,
    pub clips: Vec<ClipData>,
}

impl Track {
    /// A new, empty track at unity gain and centered.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            gain: 1.0,
            pan: 0.0,
            clips: Vec::new(),
        }
    }

    /// Builder: set the track's level.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Builder: set the track's pan position.
    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }

    /// Builder: add a clip.
    pub fn with_clip(mut self, clip: ClipData) -> Self {
        self.clips.push(clip);
        self
    }

    /// Add a clip. Control-side only.
    pub fn add_clip(&mut self, clip: ClipData) {
        self.clips.push(clip);
    }
}

/// Which parameter an [`AutomationLane`] rides. Expressed in document terms
/// (track indices, the master bus) rather than graph node handles, since the
/// nodes don't exist until [`build_graph`](Project::build_graph) resolves them.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutomationTarget {
    /// Linear gain of the track at this index.
    TrackGain(usize),
    /// Pan position (`[-1, 1]`) of the track at this index.
    TrackPan(usize),
    /// Linear gain of the master bus.
    MasterGain,
}

/// One breakpoint of an automation curve: a value at a point in time. The time
/// may be a frame count or a musical position, resolved against the project's
/// tempo and meter when the graph is built — so musical automation follows tempo
/// just like clip placement does.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutomationPoint {
    pub time: Time,
    pub value: f32,
}

impl AutomationPoint {
    pub fn new(time: impl Into<Time>, value: f32) -> Self {
        Self {
            time: time.into(),
            value,
        }
    }
}

/// The serializable twin of a graph automation lane: a target parameter, an
/// interpolation mode, and the breakpoints that shape it. Compiled to a runtime
/// [`Envelope`] bound to the resolved node in [`build_graph`](Project::build_graph).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    pub interp: Interp,
    pub points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// A new, empty lane targeting `target` with the given interpolation.
    pub fn new(target: AutomationTarget, interp: Interp) -> Self {
        Self {
            target,
            interp,
            points: Vec::new(),
        }
    }

    /// Builder: add a breakpoint. Time accepts a frame count or [`MusicalTime`].
    pub fn with_point(mut self, time: impl Into<Time>, value: f32) -> Self {
        self.points.push(AutomationPoint::new(time, value));
        self
    }

    /// Resolve to a runtime [`Envelope`], converting each breakpoint's time to a
    /// frame under the project's meter, tempo, and the device sample rate.
    fn to_envelope(&self, sig: TimeSignature, bpm: f64, sample_rate: f64) -> Envelope {
        Envelope::from_points(
            self.interp,
            self.points
                .iter()
                .map(|p| (p.time.to_frames(sig, bpm, sample_rate), p.value)),
        )
    }
}

/// A whole arrangement: tempo, the audio it references, its tracks, and the
/// master level. This is the unit that gets saved and loaded.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    /// Tempo in quarter-note beats per minute. Drives the musical-to-frame
    /// conversion for any clip placed in bars/beats.
    pub tempo_bpm: f64,
    /// Meter, e.g. 4/4. Used alongside the tempo to resolve musical positions.
    pub time_signature: TimeSignature,
    /// Linear gain of the master bus every track feeds into.
    pub master_gain: f32,
    /// The audio files this project references; clips point in here by index.
    pub sources: Vec<Source>,
    pub tracks: Vec<Track>,
    /// Parameter automation over the arrangement. Defaulted when absent so
    /// projects saved before automation existed still load.
    #[serde(default)]
    pub automation: Vec<AutomationLane>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            tempo_bpm: 120.0,
            time_signature: TimeSignature::default(),
            master_gain: 1.0,
            sources: Vec::new(),
            tracks: Vec::new(),
            automation: Vec::new(),
        }
    }
}

impl Project {
    /// An empty project at the given tempo.
    pub fn new(tempo_bpm: f64) -> Self {
        Self {
            tempo_bpm,
            ..Self::default()
        }
    }

    /// Builder: set the meter.
    pub fn with_time_signature(mut self, sig: TimeSignature) -> Self {
        self.time_signature = sig;
        self
    }

    /// Register a source file and return the index clips should use to refer to
    /// it. Adding the same path twice yields two entries; dedup at the call site
    /// if you want shared indices.
    pub fn add_source(&mut self, path: impl Into<String>) -> usize {
        let idx = self.sources.len();
        self.sources.push(Source::new(path));
        idx
    }

    /// Builder: append a track.
    pub fn with_track(mut self, track: Track) -> Self {
        self.tracks.push(track);
        self
    }

    /// Add a track. Control-side only.
    pub fn add_track(&mut self, track: Track) {
        self.tracks.push(track);
    }

    /// Builder: append an automation lane.
    pub fn with_automation(mut self, lane: AutomationLane) -> Self {
        self.automation.push(lane);
        self
    }

    /// Add an automation lane. Control-side only.
    pub fn add_automation(&mut self, lane: AutomationLane) {
        self.automation.push(lane);
    }

    /// Compile this project into a runnable [`Graph`].
    ///
    /// `sources` must be the project's [`Project::sources`] already decoded to
    /// waveforms, in the same order (index `i` of `sources` is what
    /// [`ClipData::source`] `== i` refers to). Loading them is the caller's job,
    /// which keeps this crate codec-free.
    ///
    /// `sample_rate` is the rate the graph will play at; it's needed now because
    /// any clip placed musically (in bars/beats) is resolved to frames here,
    /// using the project's tempo and meter. Pass the same rate you'll later
    /// [`prepare`](Graph::prepare) the graph with.
    ///
    /// The shape built is: each track's clips form a [`Timeline`] that feeds a
    /// [`Channel`] (the track's gain + pan), and every channel sums into a master
    /// [`Gain`]. A clip naming a missing source index is skipped. The returned
    /// graph still needs [`Graph::prepare`] before it can render.
    pub fn build_graph(
        &self,
        channels: usize,
        sample_rate: f64,
        sources: &[Arc<Waveform>],
    ) -> Graph {
        let mut graph = Graph::new(channels);
        let sig = self.time_signature;
        let bpm = self.tempo_bpm;

        // Every track feeds this; it is the node streamed to the device.
        let master = graph.add(Box::new(Gain::new(self.master_gain)));
        graph.set_master(master);

        // The channel-strip node per track, in track order, so automation lanes
        // can address a track by its index.
        let mut strips: Vec<NodeId> = Vec::with_capacity(self.tracks.len());
        for track in &self.tracks {
            let mut timeline = Timeline::new();
            for clip in &track.clips {
                if let Some(clip) = clip.to_clip(sources, sig, bpm, sample_rate) {
                    timeline.add_clip(clip);
                }
            }

            let tl = graph.add(Box::new(timeline));
            let strip = graph.add(Box::new(Channel::new(track.gain, track.pan)));
            graph.connect(tl, strip);
            graph.connect(strip, master);
            strips.push(strip);
        }

        // Resolve each automation lane to a concrete node + parameter and attach
        // it. A lane naming a missing track is skipped, mirroring how a clip with
        // a bad source index is dropped.
        for lane in &self.automation {
            let (node, param) = match lane.target {
                AutomationTarget::TrackGain(i) => match strips.get(i) {
                    Some(&strip) => (strip, Channel::GAIN),
                    None => continue,
                },
                AutomationTarget::TrackPan(i) => match strips.get(i) {
                    Some(&strip) => (strip, Channel::PAN),
                    None => continue,
                },
                AutomationTarget::MasterGain => (master, Gain::GAIN),
            };
            graph.automate(node, param, lane.to_envelope(sig, bpm, sample_rate));
        }

        graph
    }

    /// Resolve a position or length to frames using this project's tempo and
    /// meter — handy for computing loop bounds in bars/beats:
    ///
    /// ```
    /// # use rdaw_core::{MusicalTime, Project};
    /// let project = Project::new(120.0); // 4/4
    /// // bar 2 (zero-based) at 120 BPM / 44.1 kHz is two 2-second bars in.
    /// assert_eq!(project.frames_at(MusicalTime::bars(2), 44_100.0), 176_400);
    /// ```
    pub fn frames_at(&self, time: impl Into<Time>, sample_rate: f64) -> u64 {
        time.into()
            .to_frames(self.time_signature, self.tempo_bpm, sample_rate)
    }

    /// Serialize to pretty-printed JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Parse from JSON.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Write the project to `path` as JSON. A malformed-document serialization
    /// error surfaces as [`std::io::ErrorKind::InvalidData`].
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let json = self
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// Read a project back from a JSON file written by [`save`](Project::save).
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Self::from_json(&json).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
