//! Decode-path tests. These do not need ONNX Runtime.

use transcribe::{decode_audio, TranscribeError};

/// Build a minimal 16-bit PCM mono WAV in memory.
fn wav_mono_16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[test]
fn decodes_wav_to_mono_samples() {
    let sr = 16_000;
    let pcm: Vec<i16> = (0..sr)
        .map(|i| {
            let t = i as f64 / sr as f64;
            ((2.0 * std::f64::consts::PI * 220.0 * t).sin() * 16_000.0) as i16
        })
        .collect();
    let bytes = wav_mono_16(&pcm, sr as u32);

    let (samples, rate) = decode_audio(bytes, Some("wav")).expect("wav should decode");
    assert_eq!(rate, sr as u32);
    // Symphonia may emit in blocks; length should match within a small slack.
    assert!(
        (samples.len() as i64 - pcm.len() as i64).abs() <= 2048,
        "decoded {} samples, expected ~{}",
        samples.len(),
        pcm.len()
    );
    assert!(
        samples.iter().any(|&s| s.abs() > 0.1),
        "decoded audio should not be silent"
    );
}

#[test]
fn rejects_garbage_bytes() {
    let err = decode_audio(vec![0u8; 64], None).expect_err("garbage must not decode");
    assert!(matches!(err, TranscribeError::Decode(_)));
}
