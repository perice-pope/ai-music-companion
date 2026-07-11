//! #353: the reveal trigger layer, driven end-to-end from realistic frames.
//!
//! VA test #347 found the flagship reveal loop dark: 30+ seconds of steady
//! one-key playing produced zero cards. The live key signal had been made
//! honest (#321/#325 note-gating) while `REVEAL_MIN_CONFIDENCE` stayed
//! calibrated to the old flappy per-frame signal, so no steady stream could
//! clear it. These tests pin the trigger conditions at the layer the issue
//! names: pitch frames → `PerceptionTracker` → `KeySnapshot` →
//! `MusicalContext` → reveal, so a future recalibration of either side that
//! re-mutes the loop goes red here, not in a VA run.

use brain::connections::{reveal_for, reveal_on_phrase, MusicalContext, DEFAULT_REVEAL_CADENCE};
use brain::perception::{PerceptionSnapshot, PerceptionTracker};
use ears::AudioEvent;

fn pitched(hz: f64, t: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: Some(hz),
        confidence: 0.9,
        amplitude: 0.5,
        timestamp_secs: t,
        is_onset: false,
        note_info: None,
    }
}

/// Feed one sustained note as ~45 Hz analysis frames (the rate the pipeline's
/// detect loop actually produces). Returns the time after the note.
fn feed_note_frames(p: &mut PerceptionTracker, hz: f64, start: f64, dur: f64) -> f64 {
    let mut t = start;
    while t < start + dur {
        p.observe(&pitched(hz, t));
        t += 0.022;
    }
    t
}

/// The context the frontend builds from a snapshot's key before calling
/// `get_reveal` (see `practiceStore.requestReveal`).
fn context_of(snapshot: &PerceptionSnapshot) -> Option<MusicalContext> {
    snapshot.key.as_ref().map(|key| MusicalContext {
        tonic: key.tonic,
        mode: key.mode.clone(),
        confidence: key.confidence,
    })
}

/// #353 AC1: a steady one-key melodic stream fires a reveal context. The
/// material is the VA's own repro — five equal-length scale tones
/// (do-re-mi-fa-sol), held in one key for 30+ seconds — which is also the
/// least tonic-emphasized stream a player realistically produces, so it pins
/// the gate's worst honest case. Fails when `REVEAL_MIN_CONFIDENCE` sits
/// above what the note-gated signal reads on it (the #347 regression: the
/// old 0.72 gate over a signal that tops out ~0.67 here).
#[test]
fn steady_one_key_stream_fires_a_reveal_context() {
    let mut p = PerceptionTracker::new();
    // Note: with no tonic emphasis the tracker reads this C-D-E-F-G stream
    // as F major (tonic 5) — a defensible hearing of the same five notes.
    // What matters here is that the reveal rides whatever the header shows
    // (display honesty), so the asserts compare against the snapshot's own
    // key, never a hard-coded tonic.
    let five = [261.63, 293.66, 329.63, 349.23, 392.0]; // C D E F G
    let mut t = 0.0;
    for _ in 0..20 {
        for &hz in &five {
            t = feed_note_frames(&mut p, hz, t, 0.3);
        }
    }
    let snapshot = p.snapshot(t);
    let ctx = context_of(&snapshot).expect("30 s of one-key material must read a key");

    let reveal = reveal_for(&ctx, 0).unwrap_or_else(|| {
        panic!("steady one-key playing must clear the reveal gate; got {ctx:?}")
    });
    assert_eq!(
        reveal.tonic, ctx.tonic,
        "the reveal must carry the key it was generated for"
    );

    // And the full phrase-trigger path: on a cadence phrase the reveal
    // actually surfaces (this is the call `get_reveal` makes).
    let on_cadence = DEFAULT_REVEAL_CADENCE - 1;
    assert!(
        reveal_on_phrase(&ctx, on_cadence, DEFAULT_REVEAL_CADENCE).is_some(),
        "the cadence phrase must surface the reveal"
    );
}

/// #353 AC2: atonal noodling still fires nothing. A semi-chromatic walk at
/// frame rate never earns a reveal, whatever key the tracker tentatively
/// holds — checked at EVERY note boundary, because the walk's confidence
/// transiently peaks early (~0.56, before the rolling window decays it to
/// ~0.4) and a transient commit is exactly when a wrong card would fire.
/// Fails if the gate drops into the noodling band on the REAL signal, not
/// just against a hand-typed constant.
#[test]
fn atonal_noodling_fires_no_reveal() {
    let mut p = PerceptionTracker::new();
    // Aimless half/whole-step wandering across the full chromatic set.
    let walk_pcs = [
        0, 1, 3, 2, 4, 6, 5, 7, 9, 8, 10, 11, 9, 7, 8, 6, 4, 5, 3, 1, 2, 0,
    ];
    let mut committed = 0usize;
    let mut t = 0.0;
    for _ in 0..8 {
        for &pc in &walk_pcs {
            // pc → a frequency in the 4th octave.
            let hz = 440.0 * f64::powf(2.0, (f64::from(pc) - 9.0) / 12.0);
            t = feed_note_frames(&mut p, hz, t, 0.25);
            if let Some(ctx) = context_of(&p.snapshot(t)) {
                committed += 1;
                assert!(
                    reveal_for(&ctx, 0).is_none(),
                    "chromatic noodling must never clear the reveal gate; got {ctx:?}"
                );
            }
        }
    }
    // The tracker DOES tentatively commit keys over this material (from
    // ~0.4 confidence) — if it ever stops, the loop above asserts nothing
    // and this test must fail rather than pass vacuously.
    assert!(
        committed > 0,
        "the walk must exercise the gate against committed readings"
    );
}
