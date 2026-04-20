//! Guard test for the latency benchmark.
//!
//! The latency benchmark (`benches/latency.rs`) measures how fast
//! `PitchDetector::detect()` turns a window of samples into an `AudioEvent`.
//! If that code path silently became a no-op — e.g. a future refactor made
//! `detect()` return `None` for all inputs — the benchmark would still
//! "pass" the <25 ms gate because it'd be measuring almost nothing.
//!
//! This test runs the same measured code path once and asserts we actually
//! produce AudioEvents with a detected pitch. It MUST be kept in sync with
//! the helpers in `benches/latency.rs`.

use ears::pitch::{generate_sine, PitchConfig, PitchDetector};

const SAMPLE_RATE: u32 = 44_100;
const TEST_FREQ_HZ: f64 = 440.0;
const TEST_DURATION_SECS: f64 = 1.0;

#[test]
fn bench_path_produces_audio_events_with_pitch() {
    let signal = generate_sine(
        TEST_FREQ_HZ,
        SAMPLE_RATE,
        (SAMPLE_RATE as f64 * TEST_DURATION_SECS) as usize,
    );

    let mut detector = PitchDetector::new(PitchConfig {
        sample_rate: SAMPLE_RATE,
        ..Default::default()
    })
    .expect("valid pitch config");

    let window_size = detector.window_size();
    let chunks: Vec<&[f32]> = signal.chunks_exact(window_size).collect();

    assert!(!chunks.is_empty(), "expected >=1 full window of audio");

    let events: Vec<_> = chunks.iter().map(|c| detector.detect(c)).collect();

    assert_eq!(
        events.len(),
        chunks.len(),
        "should produce one event per window"
    );
    let detected = events.iter().filter(|e| e.pitch_hz.is_some()).count();
    assert!(
        detected > 0,
        "benchmark would measure a no-op: no AudioEvents with a detected pitch"
    );
}
