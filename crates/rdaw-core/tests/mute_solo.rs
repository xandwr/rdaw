//! Track mute and solo, exercised through the project's `build_graph` and the
//! offline renderer, plus the live `Channel::MUTE` parameter. Solo is a
//! project-wide rule: once any track is soloed, the non-soloed tracks fall
//! silent; otherwise mute is what decides audibility.

use std::sync::Arc;

use rdaw_core::nodes::Channel;
use rdaw_core::{Clip, ClipData, Graph, Project, Timeline, Track, Waveform};

/// A mono ramp `1, 2, 3, ...` so a track's contribution is identifiable in the
/// summed output.
fn ramp(len: usize) -> Arc<Waveform> {
    let data = (0..len).map(|i| (i + 1) as f32).collect();
    Arc::new(Waveform::from_planar(1, data))
}

#[test]
fn muted_track_is_silent() {
    let project = Project::new(120.0).with_track(
        Track::new("t")
            .with_mute(true)
            .with_clip(ClipData::new(0, 0, 4)),
    );
    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 4, 4);
    assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn soloing_one_track_silences_the_others() {
    // Two tracks playing the same ramp; soloing the second means only it sounds.
    let project = Project::new(120.0)
        .with_track(Track::new("a").with_clip(ClipData::new(0, 0, 4)))
        .with_track(
            Track::new("b")
                .with_solo(true)
                .with_clip(ClipData::new(0, 0, 4)),
        );
    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 4, 4);
    // Only track "b" sounds, so the sum is a single ramp, not double.
    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn with_no_solo_all_unmuted_tracks_sum() {
    let project = Project::new(120.0)
        .with_track(Track::new("a").with_clip(ClipData::new(0, 0, 4)))
        .with_track(Track::new("b").with_clip(ClipData::new(0, 0, 4)));
    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 4, 4);
    // Both tracks sound, so the ramp is doubled.
    assert_eq!(out, vec![2.0, 4.0, 6.0, 8.0]);
}

#[test]
fn mute_wins_over_solo_on_the_same_track() {
    // A track that is both soloed and muted stays silent: mute is absolute.
    let project = Project::new(120.0)
        .with_track(
            Track::new("a")
                .with_solo(true)
                .with_mute(true)
                .with_clip(ClipData::new(0, 0, 4)),
        )
        .with_track(Track::new("b").with_clip(ClipData::new(0, 0, 4)));
    let sources = vec![ramp(4)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 512);
    let out = graph.render_offline(48_000.0, 4, 4);
    // "a" is muted; "b" isn't soloed so it's silenced too. Everything is quiet.
    assert_eq!(out, vec![0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn channel_mute_param_toggles_live() {
    // The Channel MUTE parameter mutes and unmutes without rebuilding the graph.
    let mut graph = Graph::new(1);
    let tl = graph.add(Box::new(Timeline::new().with_clip(Clip::new(ramp(4), 0))));
    let strip = graph.add(Box::new(Channel::new(1.0, 0.0)));
    graph.connect(tl, strip);
    graph.set_master(strip);
    graph.prepare(48_000.0, 512);

    let open = graph.render_offline(48_000.0, 4, 4);
    assert_eq!(open, vec![1.0, 2.0, 3.0, 4.0]);

    graph.set_param(strip, Channel::MUTE, 1.0);
    let muted = graph.render_offline(48_000.0, 4, 4);
    assert_eq!(muted, vec![0.0, 0.0, 0.0, 0.0]);

    graph.set_param(strip, Channel::MUTE, 0.0);
    let unmuted = graph.render_offline(48_000.0, 4, 4);
    assert_eq!(unmuted, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn mute_and_solo_survive_a_json_round_trip() {
    let project = Project::new(120.0)
        .with_track(Track::new("a").with_mute(true))
        .with_track(Track::new("b").with_solo(true));
    let json = project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    assert_eq!(project, back);
}
