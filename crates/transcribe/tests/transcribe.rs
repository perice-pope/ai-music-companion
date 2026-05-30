//! Integration tests for the transcription pipeline.
//!
//! The inference tests need ONNX Runtime at run time (`ORT_DYLIB_PATH` or a
//! system `libonnxruntime`). When it is unavailable they are skipped — unless
//! `TRANSCRIBE_REQUIRE_ORT=1` is set (CI sets it), in which case a missing
//! runtime is a hard failure so the assertions can never silently vanish.

use midly::{MidiMessage, Smf, TrackEventKind};
use transcribe::{audio_to_midi, TranscribeError};

const SR: u32 = 22_050;

/// Synthesize a mono sine tone at `freq` Hz for `secs` seconds.
fn sine(freq: f64, secs: f64, sr: u32) -> Vec<f32> {
    let n = (secs * sr as f64) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr as f64;
            (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * 0.7
        })
        .collect()
}

fn midi_to_hz(m: i32) -> f64 {
    440.0 * 2f64.powf((m as f64 - 69.0) / 12.0)
}

/// Collect distinct note-on pitches from emitted MIDI bytes.
fn note_on_pitches(bytes: &[u8]) -> Vec<u8> {
    let smf = Smf::parse(bytes).expect("emitted MIDI must parse");
    let mut pitches = Vec::new();
    for track in &smf.tracks {
        for ev in track {
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { key, vel },
                ..
            } = ev.kind
            {
                if vel.as_int() > 0 {
                    pitches.push(key.as_int());
                }
            }
        }
    }
    pitches
}

/// Whether ONNX Runtime is available to load (ort's `load-dynamic` backend
/// *panics* on a missing dylib rather than returning an error, so we must gate
/// before calling in).
fn runtime_present() -> bool {
    std::env::var("ORT_DYLIB_PATH")
        .ok()
        .map(|p| std::path::Path::new(&p).exists())
        .unwrap_or(false)
}

/// Returns true if the inference tests should be skipped (runtime absent and not
/// required). When `TRANSCRIBE_REQUIRE_ORT=1` (set in CI), a missing runtime is
/// a hard failure so the assertions can never silently disappear.
fn should_skip_inference() -> bool {
    if runtime_present() {
        return false;
    }
    if std::env::var("TRANSCRIBE_REQUIRE_ORT").as_deref() == Ok("1") {
        panic!(
            "ONNX Runtime required (TRANSCRIBE_REQUIRE_ORT=1) but ORT_DYLIB_PATH is not set/found"
        );
    }
    eprintln!("skipping inference test: ONNX Runtime unavailable (set ORT_DYLIB_PATH)");
    true
}

#[test]
fn transcribes_monophonic_scale_to_correct_pitches() {
    if should_skip_inference() {
        return;
    }
    // A short C-major-ish run of sustained sine tones at known MIDI pitches.
    let expected = [60i32, 62, 64, 65, 67]; // C4 D4 E4 F4 G4
    let mut samples = Vec::new();
    for &m in &expected {
        samples.extend(sine(midi_to_hz(m), 0.6, SR));
    }

    let bytes = audio_to_midi(&samples, SR).expect("transcription should succeed");

    let detected = note_on_pitches(&bytes);
    assert!(
        !detected.is_empty(),
        "expected detected notes for a clear monophonic scale"
    );

    // Every expected pitch should appear within ±1 semitone (octave-robust:
    // basic-pitch can occasionally drop an octave on a pure sine).
    for &m in &expected {
        let hit = detected.iter().any(|&d| {
            let d = d as i32;
            (d - m).abs() <= 1 || (d - m).abs() == 12
        });
        assert!(
            hit,
            "expected a note near MIDI {m}, got pitches {detected:?}"
        );
    }
}

#[test]
fn silence_yields_no_notes() {
    if should_skip_inference() {
        return;
    }
    let samples = vec![0.0_f32; SR as usize]; // 1s of digital silence
    match audio_to_midi(&samples, SR) {
        Err(TranscribeError::Empty) => {} // expected: nothing detected
        Ok(bytes) => {
            // Tolerable only if it genuinely contains no note-ons.
            assert!(
                note_on_pitches(&bytes).is_empty(),
                "silence must not produce note-ons"
            );
        }
        Err(e) => panic!("unexpected error on silence: {e}"),
    }
}

#[test]
fn empty_input_is_handled() {
    // No samples at all must not panic, and must not need ONNX Runtime.
    match audio_to_midi(&[], SR) {
        Err(TranscribeError::Empty) => {}
        Ok(bytes) => assert!(note_on_pitches(&bytes).is_empty()),
        Err(e) => panic!("unexpected error on empty input: {e}"),
    }
}
