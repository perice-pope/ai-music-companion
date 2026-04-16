//! Integration tests for the audio capture ring buffer pipeline.
//!
//! Note: Tests that require actual audio hardware (AudioCapture::new) are
//! ignored by default since CI has no mic. Run with `cargo test -- --ignored`
//! on a machine with audio hardware.

use ears::capture::{create_sample_buffer, AudioCapture, CaptureConfig};

#[test]
fn sample_buffer_roundtrip_integrity() {
    let (mut producer, mut consumer) = create_sample_buffer(2048);

    // Simulate an audio callback writing 512-sample chunks
    let chunk: Vec<f32> = (0..512).map(|i| (i as f32 * 0.001).sin()).collect();

    let written = producer.push_samples(&chunk);
    assert_eq!(written, 512);

    // Consumer reads from another "thread"
    let mut output = vec![0.0f32; 512];
    let read = consumer.read_samples(&mut output);
    assert_eq!(read, 512);

    // Verify sample integrity
    for (i, (&expected, &actual)) in chunk.iter().zip(output.iter()).enumerate() {
        assert!(
            (expected - actual).abs() < f32::EPSILON,
            "sample {i} mismatch: expected {expected}, got {actual}"
        );
    }
}

#[test]
fn multiple_write_read_cycles() {
    let (mut producer, mut consumer) = create_sample_buffer(1024);

    for cycle in 0..10 {
        let samples: Vec<f32> = (0..64).map(|i| (cycle * 64 + i) as f32).collect();
        producer.push_samples(&samples);

        let mut output = vec![0.0f32; 64];
        let read = consumer.read_samples(&mut output);
        assert_eq!(read, 64);
        assert_eq!(output, samples);
    }
}

#[test]
fn partial_reads() {
    let (mut producer, mut consumer) = create_sample_buffer(256);

    let samples = [1.0f32; 100];
    producer.push_samples(&samples);

    // Read in smaller chunks
    let mut buf = vec![0.0f32; 30];
    assert_eq!(consumer.read_samples(&mut buf), 30);
    assert_eq!(consumer.available(), 70);

    assert_eq!(consumer.read_samples(&mut buf), 30);
    assert_eq!(consumer.available(), 40);
}

#[test]
#[ignore] // Requires audio hardware — run manually
fn audio_capture_opens_device() {
    let config = CaptureConfig::default();
    let capture = AudioCapture::new(config);
    // On CI this will fail with NoInputDevice — that's expected
    assert!(
        capture.is_ok(),
        "Failed to open audio device: {:?}",
        capture.err()
    );
}

#[test]
#[ignore] // Requires audio hardware — run manually
fn audio_capture_reads_samples() {
    let config = CaptureConfig {
        buffer_capacity: 8192,
        preferred_sample_rate: Some(44100),
    };
    let mut capture = AudioCapture::new(config).expect("need audio device");
    assert!(capture.sample_rate() > 0);
    assert!(capture.channels() > 0);

    // Wait briefly for some audio data
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut buf = vec![0.0f32; 4096];
    let read = capture.read_samples(&mut buf);
    // Should have captured something (even silence = zeros)
    assert!(read > 0, "Expected some audio samples after 100ms");
}
