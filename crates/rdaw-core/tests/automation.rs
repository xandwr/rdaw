//! Parameter automation, both the bare [`Envelope`] evaluator and end-to-end
//! through the offline renderer. Rendering at block size 1 makes the
//! control-rate automation effectively sample-accurate, so the output traces the
//! envelope exactly.

use std::sync::Arc;

use rdaw_core::nodes::Gain;
use rdaw_core::{
    AutomationLane, AutomationTarget, Clip, Envelope, Graph, Interp, Project, Timeline, Waveform,
};

#[test]
fn empty_envelope_yields_nothing() {
    let env = Envelope::new(Interp::Linear);
    assert!(env.is_empty());
    assert_eq!(env.value_at(0), None);
}

#[test]
fn linear_envelope_interpolates_and_holds_at_the_ends() {
    let env = Envelope::from_points(Interp::Linear, [(0, 0.0), (4, 1.0)]);
    assert_eq!(env.value_at(0), Some(0.0)); // on the first point
    assert_eq!(env.value_at(2), Some(0.5)); // halfway between
    assert_eq!(env.value_at(4), Some(1.0)); // on the last point
    assert_eq!(env.value_at(9), Some(1.0)); // past the end: held
}

#[test]
fn before_the_first_point_holds_the_first_value() {
    let env = Envelope::from_points(Interp::Linear, [(10, 0.5), (20, 1.0)]);
    assert_eq!(env.value_at(0), Some(0.5));
    assert_eq!(env.value_at(15), Some(0.75));
}

#[test]
fn step_envelope_holds_until_the_next_point() {
    let env = Envelope::from_points(Interp::Step, [(0, 0.2), (4, 0.8)]);
    assert_eq!(env.value_at(0), Some(0.2));
    assert_eq!(env.value_at(3), Some(0.2)); // still the earlier value
    assert_eq!(env.value_at(4), Some(0.8)); // jumps on the breakpoint
}

#[test]
fn adding_a_point_at_an_existing_frame_overwrites_it() {
    let env = Envelope::new(Interp::Step)
        .with_point(5, 0.1)
        .with_point(5, 0.9);
    assert_eq!(env.value_at(5), Some(0.9));
}

/// A flat mono "DC" waveform of `value`, long enough to cover a render.
fn dc(value: f32, len: usize) -> Arc<Waveform> {
    Arc::new(Waveform::from_planar(1, vec![value; len]))
}

#[test]
fn graph_automation_rides_a_gain() {
    // A flat 1.0 source through a Gain whose level ramps 0 -> 1 over 4 frames.
    let mut graph = Graph::new(1);
    let tl = graph.add(Box::new(
        Timeline::new().with_clip(Clip::new(dc(1.0, 8), 0)),
    ));
    let gain = graph.add(Box::new(Gain::new(1.0)));
    graph.connect(tl, gain);
    graph.set_master(gain);
    graph.automate(
        gain,
        Gain::GAIN,
        Envelope::from_points(Interp::Linear, [(0, 0.0), (4, 1.0)]),
    );
    graph.prepare(48_000.0, 8);

    // Block size 1 => the envelope is evaluated at every frame.
    let out = graph.render_offline(48_000.0, 5, 1);
    assert_eq!(out, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
}

#[test]
fn project_master_gain_automation_builds_and_renders() {
    // Whole-document path: a master-gain lane resolves to the master node.
    let project = Project::new(120.0)
        .with_track(rdaw_core::Track::new("t").with_clip(rdaw_core::ClipData::new(0, 0u64, 8u64)))
        .with_automation(
            AutomationLane::new(AutomationTarget::MasterGain, Interp::Linear)
                .with_point(0u64, 0.0)
                .with_point(4u64, 1.0),
        );

    let sources = vec![dc(1.0, 8)];
    let mut graph = project.build_graph(1, 48_000.0, &sources);
    graph.prepare(48_000.0, 8);

    let out = graph.render_offline(48_000.0, 5, 1);
    assert_eq!(out, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
}

#[test]
fn automation_survives_a_json_round_trip() {
    let project = Project::new(120.0).with_automation(
        AutomationLane::new(AutomationTarget::TrackGain(0), Interp::Step)
            .with_point(0u64, 0.3)
            .with_point(100u64, 0.7),
    );
    let json = project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    assert_eq!(project, back);
}
