//! #349 T3a — streaming polyphony integration tests.
//!
//! Same runtime gating as `transcribe.rs`: inference tests skip when ONNX
//! Runtime is absent unless `TRANSCRIBE_REQUIRE_ORT=1` (CI) makes absence a
//! hard failure. The kill-switch test runs EVERYWHERE — it asserts the
//! constructor never panics regardless of runtime availability.

use transcribe::{PolyEngine, StreamingBasicPitch};

/// Fixtures render at 44.1 kHz — the sessions' native rate — so AC1
/// exercises the REAL incremental-resample path (round-1 gap: a 22.05 kHz
/// fixture bypasses resampling entirely).
const SR: u32 = 44_100;

fn runtime_present() -> bool {
    std::env::var("ORT_DYLIB_PATH")
        .ok()
        .map(|p| std::path::Path::new(&p).exists())
        .unwrap_or(false)
}

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

/// Sum of sines for a chord, with a soft attack ramp so onsets are clean.
fn chord(midis: &[i32], secs: f64) -> Vec<f32> {
    let n = (secs * SR as f64) as usize;
    let mut out = vec![0.0f32; n];
    for &m in midis {
        let f = 440.0 * 2f64.powf((m as f64 - 69.0) / 12.0);
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f64 / SR as f64;
            let env = (t * 50.0).min(1.0); // 20 ms attack
            *o += (env * 0.5 * (2.0 * std::f64::consts::PI * f * t).sin()) as f32;
        }
    }
    out
}

/// #349 T3 AC1: a synthetic 2-chord comping fixture — the streamed engine
/// reproduces both chords' midis (±1 semitone per note) with onsets within
/// 200 ms of the true attacks, at the 2 s window / 1 s hop cadence. Fails
/// if the windowing drops a chord, double-emits, or drifts the clock.
#[test]
fn a_two_chord_comp_streams_both_chords() {
    if should_skip_inference() {
        return;
    }
    let mut engine = StreamingBasicPitch::new().expect("runtime present");

    let c_major = [60, 64, 67];
    let f_major = [65, 69, 72];
    let mut audio = chord(&c_major, 2.0);
    audio.extend(chord(&f_major, 2.0));
    // A second of tail silence so the last hop has context to flush.
    audio.extend(std::iter::repeat_n(0.0f32, SR as usize));

    let mut notes = Vec::new();
    for w in audio.chunks(1024) {
        engine.feed(w, SR);
        notes.extend(engine.poll().expect("inference runs"));
    }
    notes.extend(engine.finish().expect("tail flush"));

    // Every chord tone appears with a tolerant match; onsets near truth.
    for (want, at) in c_major
        .iter()
        .map(|&m| (m, 0.0))
        .chain(f_major.iter().map(|&m| (m, 2.0)))
    {
        let hit = notes
            .iter()
            .find(|n| (i32::from(n.midi) - want).abs() <= 1 && (n.on_secs - at).abs() <= 0.2);
        assert!(hit.is_some(), "midi {want} at {at}s missing; got {notes:?}");
    }
    // No double emission: each (midi, ~onset) appears once.
    for n in &notes {
        let dupes = notes
            .iter()
            .filter(|m| m.midi == n.midi && (m.on_secs - n.on_secs).abs() < 0.05)
            .count();
        assert_eq!(dupes, 1, "double emission of {n:?}");
    }
}

/// Round-1 must-fix 1, now the spec: a chord HELD for four seconds emits
/// each note exactly ONCE — cross-window continuity suppresses the
/// onset-less re-detections that previously re-emitted the whole triad
/// every hop (12 phantom notes for one chord). Fails if the ringing
/// tracking is dropped.
#[test]
fn a_held_chord_emits_each_note_once() {
    if should_skip_inference() {
        return;
    }
    let mut engine = StreamingBasicPitch::new().expect("runtime present");
    let audio = chord(&[60, 64, 67], 4.0);
    let mut notes = Vec::new();
    for w in audio.chunks(1024) {
        engine.feed(w, SR);
        notes.extend(engine.poll().expect("inference runs"));
    }
    notes.extend(engine.finish().expect("tail flush"));
    for m in [60u8, 64, 67] {
        let count = notes
            .iter()
            .filter(|n| (i32::from(n.midi) - i32::from(m)).abs() <= 1)
            .count();
        assert_eq!(
            count, 1,
            "midi {m} must land exactly once over a held chord: {notes:?}"
        );
    }
}

/// Round-1 must-fix 2: an onset landing exactly ON the hop boundary
/// (1.0 s) emits exactly once — the sample-space cutoff + re-attack guard
/// close the frame-quantization sliver that double-emitted it 35 ms apart.
#[test]
fn a_boundary_onset_emits_exactly_once() {
    if should_skip_inference() {
        return;
    }
    let mut engine = StreamingBasicPitch::new().expect("runtime present");
    let mut audio = vec![0.0f32; SR as usize]; // 1 s silence
    audio.extend(chord(&[64], 2.0)); // E4 attacks at exactly 1.0 s
    let mut notes = Vec::new();
    for w in audio.chunks(1024) {
        engine.feed(w, SR);
        notes.extend(engine.poll().expect("inference runs"));
    }
    notes.extend(engine.finish().expect("tail flush"));
    let hits: Vec<_> = notes
        .iter()
        .filter(|n| (i32::from(n.midi) - 64).abs() <= 1)
        .collect();
    assert_eq!(hits.len(), 1, "one attack, one note: {hits:?}");
    assert!(
        (hits[0].on_secs - 1.0).abs() <= 0.2,
        "onset near the boundary: {}",
        hits[0].on_secs
    );
}

/// The stream's tail is not lost: a chord in the final second (after the
/// last full window) flushes at finish() — a lifted take keeps its final
/// chord (round-1 nice-to-have, promoted: T3c depends on it).
#[test]
fn finish_flushes_the_final_chord() {
    if should_skip_inference() {
        return;
    }
    let mut engine = StreamingBasicPitch::new().expect("runtime present");
    let mut audio = chord(&[60, 64, 67], 2.5);
    audio.extend(chord(&[65, 69, 72], 1.0)); // F major in the tail only
    for w in audio.chunks(1024) {
        engine.feed(w, SR);
        let _ = engine.poll().expect("inference runs");
    }
    let tail = engine.finish().expect("tail flush");
    for m in [65u8, 69, 72] {
        assert!(
            tail.iter()
                .any(|n| (i32::from(n.midi) - i32::from(m)).abs() <= 1),
            "final chord midi {m} must flush: {tail:?}"
        );
    }
}

/// #349 T3 AC4 (kill switch): construction NEVER panics — with the runtime
/// present it succeeds; absent, it returns a calm error a caller can show.
/// This test runs in both environments and fails if ort's dlopen panic
/// ever escapes the seam again.
#[test]
fn construction_never_panics_regardless_of_runtime() {
    let result = std::panic::catch_unwind(StreamingBasicPitch::new);
    let outcome = result.expect("new() must never panic — the kill switch is the seam's job");
    if runtime_present() {
        assert!(outcome.is_ok(), "runtime present → engine builds");
    } else {
        let err = outcome.err().expect("runtime absent → calm error");
        let msg = err.to_string();
        assert!(
            msg.contains("everything else works"),
            "the error is written for the player: {msg}"
        );
    }
}

/// Before a full window exists, poll is calmly empty — no inference on
/// partial context, no error, no phantom notes.
#[test]
fn polling_early_is_calmly_empty() {
    if should_skip_inference() {
        return;
    }
    let mut engine = StreamingBasicPitch::new().expect("runtime present");
    engine.feed(&chord(&[60], 0.5), SR);
    assert!(engine.poll().expect("no hop yet").is_empty());
}
