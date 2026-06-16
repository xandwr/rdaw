use rdaw_io::{read_wav, write_wav};

#[test]
fn float_wav_round_trips_exactly() {
    // Interleaved stereo: distinctive per-channel values, incl. negatives.
    let samples = vec![0.0f32, -1.0, 0.5, -0.25, 0.123_456, 1.0];
    let dir = std::env::temp_dir();
    let path = dir.join("rdaw_roundtrip.wav");

    write_wav(&path, &samples, 2, 44_100).unwrap();
    let loaded = read_wav(&path).unwrap();

    assert_eq!(loaded.sample_rate, 44_100);
    assert_eq!(loaded.waveform.channels(), 2);
    assert_eq!(loaded.waveform.frames(), 3);

    // Channel 0 = even indices, channel 1 = odd indices of the interleaved input.
    assert_eq!(loaded.waveform.channel(0), &[0.0, 0.5, 0.123_456]);
    assert_eq!(loaded.waveform.channel(1), &[-1.0, -0.25, 1.0]);

    let _ = std::fs::remove_file(&path);
}
