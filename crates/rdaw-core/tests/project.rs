//! The project document model: serialization round-trips and the graph it
//! compiles to, exercised through the offline renderer so it needs no device.

use std::sync::Arc;

use rdaw_core::{ClipData, MusicalTime, Project, TimeSignature, Track, Waveform};

/// A mono waveform whose sample values are `1, 2, 3, ...` so positions are
/// trivially checkable in the output.
fn ramp(len: usize) -> Arc<Waveform> {
    let data = (0..len).map(|i| (i + 1) as f32).collect();
    Arc::new(Waveform::from_planar(1, data))
}

/// A small but fully-populated project: two sources, two tracks with distinct
/// gain/pan, and a windowed clip: enough to prove every field survives a trip
/// through JSON.
fn sample_project() -> Project {
    Project::new(140.0)
        .with_track(
            Track::new("drums")
                .with_gain(0.8)
                .with_pan(-0.5)
                .with_clip(ClipData::new(0, 0, 4))
                .with_clip(ClipData::new(0, 8, 4).with_gain(0.5)),
        )
        .with_track(
            Track::new("bass")
                .with_pan(0.25)
                .with_clip(ClipData::new(1, 2, 6).with_source_range(1, 3))
                // A musically-placed clip too, so both `Time` variants round-trip.
                .with_clip(ClipData::new(1, MusicalTime::bars(1), MusicalTime::bars(1))),
        )
}

#[test]
fn json_round_trip_preserves_the_document() {
    let mut project = sample_project();
    project.add_source("drums.wav");
    project.add_source("bass.wav");
    project.master_gain = 0.9;

    let json = project.to_json().expect("serialize");
    let back = Project::from_json(&json).expect("deserialize");

    assert_eq!(project, back);
}

#[test]
fn add_source_returns_sequential_indices() {
    let mut project = Project::new(120.0);
    assert_eq!(project.add_source("a.wav"), 0);
    assert_eq!(project.add_source("b.wav"), 1);
    assert_eq!(project.sources.len(), 2);
}

#[test]
fn built_graph_places_clips_like_a_hand_wired_timeline() {
    // One mono track, two placements of a 1..=4 ramp, rendered mono so pan and
    // bus widening don't enter into it.
    let project = Project::new(120.0).with_track(
        Track::new("t")
            .with_clip(ClipData::new(0, 0, 4))
            .with_clip(ClipData::new(0, 6, 4)),
    );

    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 10, 10);

    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn track_gain_scales_the_whole_lane() {
    let project = Project::new(120.0).with_track(
        Track::new("t")
            .with_gain(0.5)
            .with_clip(ClipData::new(0, 0, 3)),
    );

    let sources = vec![ramp(3)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 3, 3);

    assert_eq!(out, vec![0.5, 1.0, 1.5]);
}

#[test]
fn hard_pan_routes_a_mono_track_to_one_side() {
    // A mono ramp panned hard left should appear only on L of the stereo bus.
    let project = Project::new(120.0).with_track(
        Track::new("left")
            .with_pan(-1.0)
            .with_clip(ClipData::new(0, 0, 2)),
    );

    let sources = vec![ramp(2)];
    let mut graph = project.build_graph(2, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 2, 2);

    // Interleaved L,R,L,R: ramp (1,2) on L, silence on R.
    assert_eq!(out, vec![1.0, 0.0, 2.0, 0.0]);
}

#[test]
fn center_pan_is_constant_power() {
    // Centered, both sides should carry the source at the -3 dB constant-power
    // level (1/sqrt(2)), so the two channels stay equal and ~0.707 of the input.
    let project =
        Project::new(120.0).with_track(Track::new("center").with_clip(ClipData::new(0, 0, 1)));

    let sources = vec![ramp(1)]; // single sample of value 1.0
    let mut graph = project.build_graph(2, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 1, 1);

    let expected = std::f32::consts::FRAC_1_SQRT_2;
    assert!((out[0] - expected).abs() < 1e-6, "L was {}", out[0]);
    assert!((out[1] - expected).abs() < 1e-6, "R was {}", out[1]);
}

#[test]
fn clip_with_out_of_range_source_is_skipped() {
    // Source index 5 doesn't exist; that clip is dropped, the valid one renders.
    let project = Project::new(120.0).with_track(
        Track::new("t")
            .with_clip(ClipData::new(5, 0, 2))
            .with_clip(ClipData::new(0, 0, 2)),
    );

    let sources = vec![ramp(2)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 2, 2);

    assert_eq!(out, vec![1.0, 2.0]);
}

#[test]
fn musical_clip_resolves_to_frames_via_tempo() {
    // 120 BPM in 4/4. We render at a sample rate of 4 so that one quarter-note
    // beat is exactly 2 frames (sr * 60 / bpm = 4 * 0.5 = 2). Beat 1 of bar 0
    // then lands at frame 2: no hand-computed frame numbers in the document.
    let project = Project::new(120.0).with_track(Track::new("t").with_clip(ClipData::new(
        0,
        MusicalTime::bar_beat(0, 1),
        4u64,
    )));

    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 4.0, &sources);
    graph.prepare(4.0, 512);
    let out = graph.render_offline(4.0, 8, 8);

    // Silence for the first beat, then the ramp.
    assert_eq!(out, vec![0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 0.0, 0.0]);
}

#[test]
fn doubling_the_tempo_halves_a_musical_position() {
    let slow = Project::new(120.0);
    let fast = Project::new(240.0);
    let pos = MusicalTime::bars(1);
    // Same bar, twice the tempo => the downbeat arrives in half the frames.
    assert_eq!(
        fast.frames_at(pos, 44_100.0) * 2,
        slow.frames_at(pos, 44_100.0)
    );
}

#[test]
fn time_signature_changes_the_bar_length() {
    // At 120 BPM / 48 kHz a quarter note is 24 000 frames.
    let four_four = Project::new(120.0);
    let six_eight = Project::new(120.0).with_time_signature(TimeSignature::new(6, 8));
    let bar = MusicalTime::bars(1);

    assert_eq!(four_four.frames_at(bar, 48_000.0), 96_000); // 4 quarters
    assert_eq!(six_eight.frames_at(bar, 48_000.0), 72_000); // 3 quarters
}

#[test]
fn save_and_load_round_trips_through_a_file() {
    let mut project = sample_project();
    project.add_source("drums.wav");
    project.add_source("bass.wav");

    let mut path = std::env::temp_dir();
    path.push("rdaw_project_roundtrip.json");

    project.save(&path).expect("save");
    let back = Project::load(&path).expect("load");
    let _ = std::fs::remove_file(&path);

    assert_eq!(project, back);
}
