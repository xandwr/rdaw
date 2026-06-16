//! Musical-time conversions: bars/beats/ticks into quarters and frames under a
//! given tempo and meter.

use rdaw_core::tempo::TICKS_PER_BEAT;
use rdaw_core::{MusicalTime, TimeSignature};

#[test]
fn time_signature_quarter_spans() {
    let four_four = TimeSignature::new(4, 4);
    assert_eq!(four_four.quarters_per_beat(), 1.0);
    assert_eq!(four_four.quarters_per_bar(), 4.0);

    // In 6/8 a beat is an eighth note (half a quarter); a bar is three quarters.
    let six_eight = TimeSignature::new(6, 8);
    assert_eq!(six_eight.quarters_per_beat(), 0.5);
    assert_eq!(six_eight.quarters_per_bar(), 3.0);
}

#[test]
fn bars_to_frames_in_four_four() {
    // 120 BPM, 44.1 kHz: a quarter note is 22 050 frames, a 4/4 bar 88 200.
    let sig = TimeSignature::new(4, 4);
    assert_eq!(MusicalTime::bars(0).to_frames(sig, 120.0, 44_100.0), 0);
    assert_eq!(MusicalTime::bars(1).to_frames(sig, 120.0, 44_100.0), 88_200);
    assert_eq!(
        MusicalTime::bars(2).to_frames(sig, 120.0, 44_100.0),
        176_400
    );
}

#[test]
fn beats_and_ticks_subdivide_the_bar() {
    let sig = TimeSignature::new(4, 4);
    // Beat 1 (zero-based) = one quarter in = 22 050 frames at 120 BPM / 44.1k.
    assert_eq!(
        MusicalTime::bar_beat(0, 1).to_frames(sig, 120.0, 44_100.0),
        22_050
    );
    // Half a beat in via ticks lands halfway through that quarter.
    let half = MusicalTime::new(0, 0, TICKS_PER_BEAT / 2);
    assert_eq!(half.to_frames(sig, 120.0, 44_100.0), 11_025);
}

#[test]
fn meter_changes_where_a_bar_lands() {
    // The same bar index is shorter in 6/8 (3 quarters) than 4/4 (4 quarters).
    let bar = MusicalTime::bars(1);
    let common = bar.to_frames(TimeSignature::new(4, 4), 120.0, 48_000.0);
    let compound = bar.to_frames(TimeSignature::new(6, 8), 120.0, 48_000.0);
    assert_eq!(common, 96_000);
    assert_eq!(compound, 72_000);
}

#[test]
fn to_quarters_is_tempo_independent() {
    let sig = TimeSignature::new(4, 4);
    // Quarters depend only on the grid + meter, never the tempo.
    assert_eq!(MusicalTime::bar_beat(2, 2).to_quarters(sig), 10.0);
}
