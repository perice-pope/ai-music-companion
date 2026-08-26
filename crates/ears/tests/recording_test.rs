//! Integration test: full record → flush → read-back roundtrip.
//!
//! Writes one second of a 440 Hz sine wave at 16-bit mono 44.1 kHz,
//! then re-parses the resulting WAV header and data bytes to confirm
//! the recorder produced a valid PCM WAV whose samples recover the
//! source signal within i16 quantization.

use std::f32::consts::PI;
use std::fs;

use ears::recorder::{AudioRecorder, RecorderConfig};
use tempfile::TempDir;

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;
const FREQ_HZ: f32 = 440.0;

#[test]
fn end_to_end_record_and_readback() {
    let tmp = TempDir::new().unwrap();
    let cfg = RecorderConfig {
        output_dir: tmp.path().to_path_buf(),
        max_bytes_per_file: 10 * 1024 * 1024,
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
    };
    let mut rec = AudioRecorder::new(cfg);

    // 1 second of 440 Hz sine at 0.5 amplitude.
    let source: Vec<f32> = (0..SAMPLE_RATE)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            0.5 * (2.0 * PI * FREQ_HZ * t).sin()
        })
        .collect();

    let path = rec.start("sess-roundtrip", "trumpet").unwrap();
    rec.write_samples(&source).unwrap();
    let meta = rec.stop().unwrap();

    // Duration must match 1.0s exactly.
    assert!((meta.duration_secs - 1.0).abs() < 1e-9);
    assert_eq!(meta.samples_written, u64::from(SAMPLE_RATE));
    assert_eq!(meta.channels, CHANNELS);
    assert_eq!(meta.sample_rate, SAMPLE_RATE);

    // Re-read the WAV from disk.
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.len() >= 44, "file too small to be a WAV");

    // Header checks.
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    assert_eq!(&bytes[36..40], b"data");

    let sample_rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let channels = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
    let bps = u16::from_le_bytes(bytes[34..36].try_into().unwrap());
    let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;

    assert_eq!(sample_rate, SAMPLE_RATE);
    assert_eq!(channels, CHANNELS);
    assert_eq!(bps, BITS_PER_SAMPLE);
    assert_eq!(data_size, source.len() * 2);

    // RIFF size patching — file_size - 8.
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(riff_size as usize + 8, bytes.len());

    // Decode i16 samples and compare to source with a quantization-tolerant
    // bound. i16::MAX scaling means one LSB is ~3.05e-5 in f32 amplitude.
    let data = &bytes[44..44 + data_size];
    let decoded: Vec<f32> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| {
            let v = i16::from_le_bytes(*c);
            f32::from(v) / f32::from(i16::MAX)
        })
        .collect();

    assert_eq!(decoded.len(), source.len());

    // Max absolute error should be bounded by one i16 step + rounding slack.
    let tolerance = 2.0 / f32::from(i16::MAX);
    let mut max_err: f32 = 0.0;
    for (orig, round) in source.iter().zip(decoded.iter()) {
        let err = (orig - round).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err <= tolerance,
        "roundtrip error {max_err} exceeded tolerance {tolerance}"
    );
}
