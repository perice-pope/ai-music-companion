//! Integration tests for the session recorder + SQLite store pipeline.
//!
//! These tests exercise the public API only (nothing `pub(crate)`) to
//! guarantee the types remain usable from downstream crates such as the
//! Tauri shell.

use async_trait::async_trait;
use brain::phrase::{DynamicsStats, PhraseSummary, PitchStats};
use brain::session::{
    PracticeMode, RecapGenerator, RecapInput, SessionError, SessionRecap, SessionRecorder,
};
use brain::store::SessionStore;
use chrono::{Duration, Utc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock recap generator
// ---------------------------------------------------------------------------

/// A deterministic recap generator for integration tests: returns a
/// canned recap whose `duration_secs` and `phrase_count` are taken
/// from the real `RecapInput` so assertions stay faithful to what the
/// recorder actually captured.
struct MockRecapGenerator {
    call_count: Arc<AtomicUsize>,
}

impl MockRecapGenerator {
    fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl RecapGenerator for MockRecapGenerator {
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(SessionRecap {
            score_summary: None,
            overall_assessment: format!(
                "You played {} phrases on {}.",
                input.phrases.len(),
                input.instrument
            ),
            strengths: vec!["Consistent tone.".to_owned()],
            areas_to_improve: vec!["Intonation drifts sharp.".to_owned()],
            next_session_suggestions: vec!["Practice with a drone.".to_owned()],
            duration_secs: input.duration_secs,
            phrase_count: input.phrases.len(),
            instrument: input.instrument.clone(),
            fingerprint: None,
            intonation_display: None,
            groove_display: None,
            flavour: None,
            idiom_notes: input.idiom_notes.clone(),
            connections: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn phrase_at(idx: usize) -> PhraseSummary {
    PhraseSummary {
        phrase_index: idx,
        start_time: idx as f64 * 2.0,
        end_time: idx as f64 * 2.0 + 1.5,
        duration_secs: 1.5,
        note_count: 10,
        pitch_stats: PitchStats {
            mean_hz: 440.0,
            min_hz: 435.0,
            max_hz: 445.0,
            range_cents: 40.0,
            pitches: vec![440.0; 10],
        },
        dynamics: DynamicsStats {
            mean_amplitude: 0.55,
            min_amplitude: 0.3,
            max_amplitude: 0.8,
            dynamic_range: 0.5,
        },
        stability: 0.88,
        score_position: None,
        tone: None,
        key: None,
        onsets_secs: Vec::new(),
        score_span: None,
        verdicts: None,
        score_card: None,
    }
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_to_end_record_and_persist() {
    // 1. Build a recorder (NO longer owns the recap generator).
    let mut recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
    let started_at = recorder.started_at();

    // 2. Record 2 phrases and 2 tips.
    recorder.record_phrase(phrase_at(0)).unwrap();
    recorder.record_phrase(phrase_at(1)).unwrap();
    recorder
        .record_tip(
            0,
            "Nice tone at the start.".to_owned(),
            "encouragement".to_owned(),
            "tone".to_owned(),
        )
        .unwrap();
    recorder
        .record_tip(
            1,
            "Watch the intonation on the final note.".to_owned(),
            "suggestion".to_owned(),
            "intonation".to_owned(),
        )
        .unwrap();
    assert_eq!(recorder.phrase_count(), 2);
    assert_eq!(recorder.tip_count(), 2);

    // 3. Finalise the session — authoritative timestamps come from the
    //    recorder, not reconstructed from a recap.
    let completed = recorder.complete().expect("complete");
    assert_eq!(completed.primary_instrument(), "trumpet");
    assert_eq!(completed.phrase_count(), 2);
    assert_eq!(completed.started_at, started_at);
    assert!(completed.ended_at >= completed.started_at);

    // 4. Generate recap separately. Authoritative fields from
    //    CompletedSession must override anything the generator emitted.
    let generator = MockRecapGenerator::new();
    let recap = completed.generate_recap(&generator).await.expect("recap");
    assert_eq!(recap.phrase_count, 2);
    assert_eq!(recap.instrument, "trumpet");
    assert!((recap.duration_secs - completed.duration_secs).abs() < 1e-9);

    // 5. Persist to an in-memory store.
    let store = SessionStore::in_memory().unwrap();
    store
        .save(
            completed.id,
            completed.started_at,
            completed.ended_at,
            &recap,
        )
        .unwrap();

    // 6. Load back — StoredSession now returns the persisted timestamps
    //    so callers don't have to reconstruct them.
    let loaded = store.load(completed.id).unwrap();
    assert_eq!(loaded.id, completed.id);
    assert_eq!(loaded.started_at, completed.started_at);
    assert_eq!(loaded.ended_at, completed.ended_at);

    // Field-by-field equality on the recap body.
    let r = &loaded.recap;
    assert_eq!(r.overall_assessment, recap.overall_assessment);
    assert_eq!(r.strengths, recap.strengths);
    assert_eq!(r.areas_to_improve, recap.areas_to_improve);
    assert_eq!(r.next_session_suggestions, recap.next_session_suggestions);
    assert_eq!(r.phrase_count, recap.phrase_count);
    assert_eq!(r.instrument, recap.instrument);
    assert!(
        (r.duration_secs - recap.duration_secs).abs() < 1e-9,
        "duration_secs must round-trip exactly"
    );
}

#[tokio::test]
async fn multiple_sessions_coexist_in_store() {
    let store = SessionStore::in_memory().unwrap();
    let base = Utc::now();

    // Save 3 sessions with different timestamps and instruments.
    let mut ids = Vec::new();
    for (i, instrument) in ["trumpet", "violin", "voice"].iter().enumerate() {
        let mut recorder = SessionRecorder::new((*instrument).to_owned(), PracticeMode::default());
        recorder.record_phrase(phrase_at(0)).unwrap();
        let completed = recorder.complete().unwrap();

        let generator = MockRecapGenerator::new();
        let recap = completed.generate_recap(&generator).await.unwrap();

        let started = base - Duration::hours(i64::try_from(i).unwrap());
        let ended = started + Duration::seconds(30);
        store.save(completed.id, started, ended, &recap).unwrap();
        ids.push(completed.id);
    }

    // list_recent returns all 3.
    let recent = store.list_recent(10).unwrap();
    assert_eq!(recent.len(), 3);

    // Each one is loadable via id, and timestamps survive.
    for id in &ids {
        let loaded = store.load(*id).unwrap();
        assert!(!loaded.recap.instrument.is_empty());
        assert_eq!(loaded.recap.phrase_count, 1);
        assert!(loaded.ended_at > loaded.started_at);
    }

    // And the recent list's ids are exactly the ones we saved.
    let mut recent_ids: Vec<_> = recent.iter().map(|s| s.id).collect();
    recent_ids.sort_by_key(|id| id.as_str());
    let mut expected_ids = ids.clone();
    expected_ids.sort_by_key(|id| id.as_str());
    assert_eq!(recent_ids, expected_ids);
}

#[test]
fn empty_session_cannot_be_completed() {
    // Proves the whole pipeline refuses to produce a zero-phrase recap.
    let recorder = SessionRecorder::new("trumpet".to_owned(), PracticeMode::default());
    let err = recorder.complete().unwrap_err();
    assert!(
        matches!(err, SessionError::Empty),
        "zero-phrase session must fail fast: {err:?}"
    );
}

#[test]
fn completed_session_persists_without_recap_when_generator_fails() {
    // Regression for the CR #1 observation: if the LLM fails, the session
    // data must still be persistable. We simulate a transient LLM failure
    // by constructing a CompletedSession but never generating a recap —
    // callers can synthesize a minimal recap and still save the timing.
    let mut recorder = SessionRecorder::new("voice".to_owned(), PracticeMode::default());
    recorder.record_phrase(phrase_at(0)).unwrap();
    recorder.record_phrase(phrase_at(1)).unwrap();

    let completed = recorder.complete().unwrap();

    // Pretend the LLM failed. Caller builds a minimal fallback recap to
    // persist session metadata anyway (this is what a real app would do
    // rather than dropping the session wholesale).
    let fallback_recap = SessionRecap {
        score_summary: None,
        overall_assessment: "(recap unavailable)".to_owned(),
        strengths: Vec::new(),
        areas_to_improve: Vec::new(),
        next_session_suggestions: Vec::new(),
        duration_secs: completed.duration_secs,
        phrase_count: completed.phrase_count(),
        instrument: completed.primary_instrument().to_owned(),
        fingerprint: None,
        intonation_display: None,
        groove_display: None,
        flavour: None,
        idiom_notes: Vec::new(),
        connections: Vec::new(),
    };

    let store = SessionStore::in_memory().unwrap();
    store
        .save(
            completed.id,
            completed.started_at,
            completed.ended_at,
            &fallback_recap,
        )
        .unwrap();

    let loaded = store.load(completed.id).unwrap();
    assert_eq!(loaded.recap.phrase_count, 2);
    assert_eq!(loaded.recap.instrument, "voice");
}
