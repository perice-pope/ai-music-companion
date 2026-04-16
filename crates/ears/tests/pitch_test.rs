//! Integration tests for pitch detection.

use ears::pitch::{generate_sine, PitchConfig, PitchDetector};

#[test]
fn a4_440hz_detected_accurately() {
    let config = PitchConfig {
        sample_rate: 44100,
        ..Default::default()
    };
    let mut detector = PitchDetector::new(config);
    let samples = generate_sine(440.0, 44100, detector.hop_size());

    let event = detector.detect(&samples);
    let hz = event.pitch_hz.expect("Should detect A4");
    assert!(
        (hz - 440.0).abs() < 5.0,
        "A4 detection: expected ~440 Hz, got {hz:.1} Hz"
    );
    assert!(event.confidence > 0.7);
}

#[test]
fn low_e2_82hz_detected() {
    let config = PitchConfig {
        sample_rate: 44100,
        freq_min_hz: 60.0,
        ..Default::default()
    };
    let mut detector = PitchDetector::new(config);
    let samples = generate_sine(82.41, 44100, detector.hop_size());

    let event = detector.detect(&samples);
    let hz = event.pitch_hz.expect("Should detect E2");
    assert!(
        (hz - 82.41).abs() < 3.0,
        "E2 detection: expected ~82.41 Hz, got {hz:.1} Hz"
    );
}

#[test]
fn high_c6_1047hz_detected() {
    let config = PitchConfig {
        sample_rate: 44100,
        ..Default::default()
    };
    let mut detector = PitchDetector::new(config);
    let samples = generate_sine(1046.5, 44100, detector.hop_size());

    let event = detector.detect(&samples);
    let hz = event.pitch_hz.expect("Should detect C6");
    assert!(
        (hz - 1046.5).abs() < 10.0,
        "C6 detection: expected ~1046.5 Hz, got {hz:.1} Hz"
    );
}

#[test]
fn silence_yields_no_pitch() {
    let config = PitchConfig::default();
    let mut detector = PitchDetector::new(config);
    let samples = vec![0.0f32; detector.hop_size()];

    let event = detector.detect(&samples);
    assert!(event.pitch_hz.is_none());
    assert!(event.confidence < 0.1);
}

#[test]
fn very_quiet_signal_yields_no_pitch() {
    let config = PitchConfig::default();
    let mut detector = PitchDetector::new(config);
    // Very quiet 440 Hz — below the silence gate
    let samples: Vec<f32> = generate_sine(440.0, 44100, detector.hop_size())
        .into_iter()
        .map(|s| s * 0.001)
        .collect();

    let event = detector.detect(&samples);
    assert!(
        event.pitch_hz.is_none(),
        "Sub-threshold signal should not detect"
    );
}

#[test]
fn multiple_detections_advance_timestamp() {
    let config = PitchConfig {
        sample_rate: 44100,
        ..Default::default()
    };
    let mut detector = PitchDetector::new(config);
    let samples = generate_sine(440.0, 44100, detector.hop_size());

    let e1 = detector.detect(&samples);
    let e2 = detector.detect(&samples);
    let e3 = detector.detect(&samples);

    assert!(e1.timestamp_secs < e2.timestamp_secs);
    assert!(e2.timestamp_secs < e3.timestamp_secs);
}

#[test]
fn different_frequencies_detected_differently() {
    let config = PitchConfig::default();
    let mut detector = PitchDetector::new(config);

    let samples_a = generate_sine(440.0, 44100, detector.hop_size());
    let event_a = detector.detect(&samples_a);

    let samples_e = generate_sine(329.63, 44100, detector.hop_size());
    let event_e = detector.detect(&samples_e);

    let hz_a = event_a.pitch_hz.expect("Should detect A4");
    let hz_e = event_e.pitch_hz.expect("Should detect E4");

    assert!(
        (hz_a - hz_e).abs() > 50.0,
        "A4 ({hz_a:.1}) and E4 ({hz_e:.1}) should be clearly different"
    );
}
