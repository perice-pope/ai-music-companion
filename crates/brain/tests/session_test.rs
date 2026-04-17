//! Integration tests for the session recorder + SQLite store pipeline.
//!
//! These tests exercise the public API only (nothing `pub(crate)`) to
//! guarantee the types remain usable from downstream crates such as the
//! Tauri shell.

use brain::phrase::{DynamicsStats, PhraseSummary, PitchStats};
use brain::session::{RecapGenerator, RecapInput, SessionError, SessionRecap, SessionRecorder};
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

impl RecapGenerator for MockRecapGenerator {
    fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(SessionRecap {
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
    }
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[test]
fn end_to_end_record_and_persist() {
    // 1. Build a recorder with our deterministic mock.
    let mut recorder =
        SessionRecorder::new("trumpet".to_owned(), Box::new(MockRecapGenerator::new()));
    let started_at = recorder.started_at();

    // 2. Record 2 phrases and 2 tips.
    recorder.record_phrase(phrase_at(0));
    recorder.record_phrase(phrase_at(1));
    recorder.record_tip(
        0,
        "Nice tone at the start.".to_owned(),
        "encouragement".to_owned(),
        "tone".to_owned(),
    );
    recorder.record_tip(
        1,
        "Watch the intonation on the final note.".to_owned(),
        "suggestion".to_owned(),
        "intonation".to_owned(),
    );
    assert_eq!(recorder.phrase_count(), 2);
    assert_eq!(recorder.tip_count(), 2);

    // 3. End the session.
    let (id, recap) = recorder.end_session().unwrap();

    // Recap echoed back our data faithfully.
    assert_eq!(recap.phrase_count, 2);
    assert_eq!(recap.instrument, "trumpet");
    assert!(recap.duration_secs >= 0.0);

    // 4. Persist to an in-memory store and read back.
    let store = SessionStore::in_memory().unwrap();
    let ended_at =
        started_at + Duration::milliseconds((recap.duration_secs * 1000.0).round() as i64);
    store.save(id, started_at, ended_at, &recap).unwrap();

    let loaded = store.load(id).unwrap();

    // Field-by-field equality — not JSON-contains, not string-match.
    assert_eq!(loaded.overall_assessment, recap.overall_assessment);
    assert_eq!(loaded.strengths, recap.strengths);
    assert_eq!(loaded.areas_to_improve, recap.areas_to_improve);
    assert_eq!(
        loaded.next_session_suggestions,
        recap.next_session_suggestions
    );
    assert_eq!(loaded.phrase_count, recap.phrase_count);
    assert_eq!(loaded.instrument, recap.instrument);
    assert!(
        (loaded.duration_secs - recap.duration_secs).abs() < 1e-9,
        "duration_secs must round-trip exactly"
    );
}

#[test]
fn multiple_sessions_coexist_in_store() {
    let store = SessionStore::in_memory().unwrap();
    let base = Utc::now();

    // Save 3 sessions with different timestamps and instruments.
    let mut ids = Vec::new();
    for (i, instrument) in ["trumpet", "violin", "voice"].iter().enumerate() {
        let mut recorder = SessionRecorder::new(
            (*instrument).to_owned(),
            Box::new(MockRecapGenerator::new()),
        );
        recorder.record_phrase(phrase_at(0));
        let (id, recap) = recorder.end_session().unwrap();

        let started = base - Duration::hours(i64::try_from(i).unwrap());
        let ended = started + Duration::seconds(30);
        store.save(id, started, ended, &recap).unwrap();
        ids.push(id);
    }

    // list_recent returns all 3.
    let recent = store.list_recent(10).unwrap();
    assert_eq!(recent.len(), 3);

    // Each one is loadable via id.
    for id in &ids {
        let loaded = store.load(*id).unwrap();
        assert!(!loaded.instrument.is_empty());
        assert_eq!(loaded.phrase_count, 1);
    }

    // And the recent list's ids are exactly the ones we saved.
    let mut recent_ids: Vec<_> = recent.iter().map(|s| s.id).collect();
    recent_ids.sort_by_key(|id| id.as_str());
    let mut expected_ids = ids.clone();
    expected_ids.sort_by_key(|id| id.as_str());
    assert_eq!(recent_ids, expected_ids);
}

#[test]
fn empty_session_cannot_be_persisted_because_end_session_errors() {
    // Proves the whole pipeline refuses to produce a zero-phrase recap.
    let recorder = SessionRecorder::new("trumpet".to_owned(), Box::new(MockRecapGenerator::new()));
    let err = recorder.end_session().unwrap_err();
    assert!(
        matches!(err, SessionError::Empty),
        "zero-phrase session must fail fast: {err:?}"
    );
}
