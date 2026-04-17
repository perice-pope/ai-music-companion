//! Session recorder — accumulates phrase summaries and coaching tips
//! during a practice session, then delegates to a [`RecapGenerator`] to
//! produce a teacher-quality [`SessionRecap`].
//!
//! This module runs on the processing thread, NOT the audio thread, so
//! heap allocation (Vec, String, etc.) is allowed.
//!
//! # Decoupling from the coaching module
//!
//! The recap is generated through the [`RecapGenerator`] trait rather
//! than by calling the coaching module directly. That keeps this module
//! compilable ahead of the coaching LLM engine landing on `main`: tests
//! use a mock implementation, and production wiring (Story #14, free
//! play) plugs in the real coaching engine.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::phrase::PhraseSummary;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while running a practice session.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The underlying [`RecapGenerator`] returned an error.
    #[error("recap generator failed: {0}")]
    RecapFailed(String),
    /// The session ended without a single recorded phrase. We refuse to
    /// synthesize a recap out of nothing, because that would mask a bug
    /// in the caller (e.g. pushing phrases to the wrong recorder).
    #[error("no phrases recorded — cannot generate recap")]
    Empty,
}

// ---------------------------------------------------------------------------
// SessionId
// ---------------------------------------------------------------------------

/// Unique identifier for a practice session.
///
/// Wraps a [`Uuid`] (v4) so callers can persist it as a string and hand
/// it to the [`SessionStore`](crate::store::SessionStore) without
/// depending on `uuid` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Uuid);

impl SessionId {
    /// Generate a fresh random session id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Hyphenated string representation.
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }

    /// The inner UUID, if a caller needs it.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for SessionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

// ---------------------------------------------------------------------------
// Recorded tip
// ---------------------------------------------------------------------------

/// A coaching tip captured against a specific phrase during a session.
///
/// `severity` and `category` are stored as strings so this module does
/// not need to depend on the coaching module's enum types. The
/// coaching enums serialize in snake_case (e.g. `"suggestion"`), so
/// callers should pass the same lower-case string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedTip {
    /// Index of the phrase in [`SessionRecorder::phrases`] that this
    /// tip was generated for.
    pub phrase_index: usize,
    /// Human-readable coaching text.
    pub text: String,
    /// Severity string (e.g. `"encouragement"`, `"suggestion"`, `"focus"`).
    pub severity: String,
    /// Category string (e.g. `"tone"`, `"intonation"`, ...).
    pub category: String,
    /// When the tip was recorded.
    pub recorded_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Recap types
// ---------------------------------------------------------------------------

/// Input passed to a [`RecapGenerator`] to produce a [`SessionRecap`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapInput {
    /// The instrument being played.
    pub instrument: String,
    /// Total session duration in seconds.
    pub duration_secs: f64,
    /// All phrase summaries recorded during the session.
    pub phrases: Vec<PhraseSummary>,
    /// All coaching tips recorded during the session.
    pub tips: Vec<RecordedTip>,
}

/// The post-session recap shown to the student.
///
/// Rendered from a carefully constructed LLM prompt (see the coaching
/// module). All text fields are teacher-quality natural language:
/// no letter grades, no percentages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecap {
    /// Overall qualitative assessment of the session.
    pub overall_assessment: String,
    /// 2–4 specific strengths the student demonstrated.
    pub strengths: Vec<String>,
    /// 2–4 concrete areas to improve.
    pub areas_to_improve: Vec<String>,
    /// Suggestions for what to focus on in the next session.
    pub next_session_suggestions: Vec<String>,
    /// Total session duration in seconds (copied from `RecapInput`).
    pub duration_secs: f64,
    /// Number of phrases played (copied from `RecapInput.phrases.len()`).
    pub phrase_count: usize,
    /// Instrument played (copied from `RecapInput.instrument`).
    pub instrument: String,
}

// ---------------------------------------------------------------------------
// RecapGenerator trait
// ---------------------------------------------------------------------------

/// Abstraction over "turn a [`RecapInput`] into a [`SessionRecap`]".
///
/// Production implementations will call an LLM. Tests use a canned
/// mock. Using a trait here decouples [`SessionRecorder`] from the
/// coaching module so these can land as independent PRs.
pub trait RecapGenerator: Send + Sync {
    /// Produce a recap from accumulated session data.
    fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError>;
}

// ---------------------------------------------------------------------------
// SessionRecorder
// ---------------------------------------------------------------------------

/// Records a single practice session: phrases, coaching tips, and
/// wall-clock timing. On `end_session`, delegates to a
/// [`RecapGenerator`] to synthesize a teacher-quality [`SessionRecap`].
pub struct SessionRecorder {
    session_id: SessionId,
    instrument: String,
    started_at: DateTime<Utc>,
    phrases: Vec<PhraseSummary>,
    tips: Vec<RecordedTip>,
    recap_generator: Box<dyn RecapGenerator>,
}

impl SessionRecorder {
    /// Create a new recorder for the given instrument, using `recap_gen`
    /// to produce the recap when the session ends. The start time is
    /// captured as `Utc::now()` at construction.
    pub fn new(instrument: String, recap_gen: Box<dyn RecapGenerator>) -> Self {
        Self {
            session_id: SessionId::new(),
            instrument,
            started_at: Utc::now(),
            phrases: Vec::new(),
            tips: Vec::new(),
            recap_generator: recap_gen,
        }
    }

    /// Append a completed phrase to the session.
    pub fn record_phrase(&mut self, phrase: PhraseSummary) {
        self.phrases.push(phrase);
    }

    /// Append a coaching tip to the session.
    ///
    /// `phrase_index` should point into `self.phrases` — it is stored
    /// verbatim and not validated, because phrases may be buffered in
    /// the UI before the recorder sees them and out-of-range values
    /// are still meaningful metadata for retrospective analysis.
    pub fn record_tip(
        &mut self,
        phrase_index: usize,
        text: String,
        severity: String,
        category: String,
    ) {
        self.tips.push(RecordedTip {
            phrase_index,
            text,
            severity,
            category,
            recorded_at: Utc::now(),
        });
    }

    /// Finalise the session: compute duration, call the recap
    /// generator, and return the session id together with the recap.
    ///
    /// Consumes `self`: a recorder is single-use.
    ///
    /// Returns [`SessionError::Empty`] if no phrases were recorded, or
    /// [`SessionError::RecapFailed`] if the underlying generator
    /// failed.
    pub fn end_session(self) -> Result<(SessionId, SessionRecap), SessionError> {
        if self.phrases.is_empty() {
            return Err(SessionError::Empty);
        }

        let ended_at = Utc::now();
        let duration_secs = (ended_at - self.started_at).num_milliseconds() as f64 / 1000.0;

        let input = RecapInput {
            instrument: self.instrument.clone(),
            duration_secs,
            phrases: self.phrases,
            tips: self.tips,
        };

        let recap = self.recap_generator.generate_recap(&input)?;
        Ok((self.session_id, recap))
    }

    /// The session's unique identifier. Stable across the recorder's
    /// lifetime.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Number of phrases recorded so far.
    pub fn phrase_count(&self) -> usize {
        self.phrases.len()
    }

    /// Number of tips recorded so far (primarily for assertions in
    /// tests / diagnostics).
    pub fn tip_count(&self) -> usize {
        self.tips.len()
    }

    /// When the session started.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// Instrument label this recorder was constructed with.
    pub fn instrument(&self) -> &str {
        &self.instrument
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrase::{DynamicsStats, PhraseSummary, PitchStats};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // -----------------------------------------------------------------------
    // Mock recap generator
    // -----------------------------------------------------------------------

    /// Records every call it receives so the test can assert on what
    /// was passed through.
    struct RecordingMockGenerator {
        /// The last input the generator saw. Wrapped in Mutex because
        /// `generate_recap` takes `&self`.
        last_input: Arc<Mutex<Option<RecapInput>>>,
        /// Number of times the generator has been called.
        call_count: Arc<AtomicUsize>,
        /// Canned recap to return.
        canned: SessionRecap,
    }

    impl RecordingMockGenerator {
        fn new(canned: SessionRecap) -> Self {
            Self {
                last_input: Arc::new(Mutex::new(None)),
                call_count: Arc::new(AtomicUsize::new(0)),
                canned,
            }
        }

        fn last_input_handle(&self) -> Arc<Mutex<Option<RecapInput>>> {
            Arc::clone(&self.last_input)
        }

        fn call_count_handle(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.call_count)
        }
    }

    impl RecapGenerator for RecordingMockGenerator {
        fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            *self.last_input.lock().unwrap() = Some(input.clone());
            Ok(self.canned.clone())
        }
    }

    /// A generator that always fails. Used to verify error propagation.
    struct FailingGenerator;

    impl RecapGenerator for FailingGenerator {
        fn generate_recap(&self, _input: &RecapInput) -> Result<SessionRecap, SessionError> {
            Err(SessionError::RecapFailed("llm blew up".to_owned()))
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn canned_recap() -> SessionRecap {
        SessionRecap {
            overall_assessment: "Solid warm-up with room to grow.".to_owned(),
            strengths: vec![
                "Consistent tone on middle-register long tones.".to_owned(),
                "Good breath support throughout.".to_owned(),
            ],
            areas_to_improve: vec!["Intonation drifts sharp in the upper register.".to_owned()],
            next_session_suggestions: vec!["Play the C major scale slowly with a drone.".to_owned()],
            duration_secs: 0.0,
            phrase_count: 0,
            instrument: "trumpet".to_owned(),
        }
    }

    fn phrase(idx: usize) -> PhraseSummary {
        PhraseSummary {
            phrase_index: idx,
            start_time: idx as f64,
            end_time: idx as f64 + 1.0,
            duration_secs: 1.0,
            note_count: 8,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 40.0,
                pitches: vec![440.0; 8],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.6,
                min_amplitude: 0.4,
                max_amplitude: 0.8,
                dynamic_range: 0.4,
            },
            stability: 0.9,
        }
    }

    fn make_recorder(canned: SessionRecap) -> (SessionRecorder, Arc<Mutex<Option<RecapInput>>>) {
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let recorder = SessionRecorder::new("trumpet".to_owned(), Box::new(mock));
        (recorder, input_handle)
    }

    // -----------------------------------------------------------------------
    // Behaviour tests
    // -----------------------------------------------------------------------

    #[test]
    fn recorder_starts_empty() {
        let (recorder, _) = make_recorder(canned_recap());
        assert_eq!(recorder.phrase_count(), 0);
        assert_eq!(recorder.tip_count(), 0);
        assert_eq!(recorder.instrument(), "trumpet");
    }

    #[test]
    fn record_phrase_accumulates() {
        let (mut recorder, _) = make_recorder(canned_recap());
        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));
        recorder.record_phrase(phrase(2));
        assert_eq!(
            recorder.phrase_count(),
            3,
            "three record_phrase calls should yield count == 3"
        );
    }

    #[test]
    fn record_tip_attaches_to_phrase_index() {
        let canned = canned_recap();
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), Box::new(mock));

        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));
        recorder.record_tip(
            1,
            "Breathe before this entrance.".to_owned(),
            "suggestion".to_owned(),
            "expression".to_owned(),
        );

        assert_eq!(recorder.tip_count(), 1);

        // End the session so the mock captures the full input.
        recorder.end_session().unwrap();
        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert_eq!(captured.tips.len(), 1);
        assert_eq!(captured.tips[0].phrase_index, 1);
        assert_eq!(captured.tips[0].text, "Breathe before this entrance.");
        assert_eq!(captured.tips[0].severity, "suggestion");
        assert_eq!(captured.tips[0].category, "expression");
    }

    #[test]
    fn end_session_calls_recap_generator_with_all_data() {
        let canned = canned_recap();
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let call_count = mock.call_count_handle();
        let mut recorder = SessionRecorder::new("violin".to_owned(), Box::new(mock));

        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));
        recorder.record_tip(
            0,
            "Nice bow control.".to_owned(),
            "encouragement".to_owned(),
            "technique".to_owned(),
        );
        recorder.record_tip(
            1,
            "Watch the intonation on your shifts.".to_owned(),
            "focus".to_owned(),
            "intonation".to_owned(),
        );

        recorder.end_session().unwrap();

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "recap generator should be called exactly once per session end"
        );
        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert_eq!(captured.instrument, "violin");
        assert_eq!(captured.phrases.len(), 2);
        assert_eq!(captured.phrases[0].phrase_index, 0);
        assert_eq!(captured.phrases[1].phrase_index, 1);
        assert_eq!(captured.tips.len(), 2);
        assert_eq!(captured.tips[0].category, "technique");
        assert_eq!(captured.tips[1].category, "intonation");
    }

    #[test]
    fn end_session_returns_recap_and_session_id() {
        let canned = canned_recap();
        let expected_assessment = canned.overall_assessment.clone();
        let expected_strengths = canned.strengths.clone();

        let (mut recorder, _) = make_recorder(canned);
        let expected_id = recorder.session_id();
        recorder.record_phrase(phrase(0));

        let (id, recap) = recorder.end_session().unwrap();
        assert_eq!(
            id, expected_id,
            "returned SessionId must match the recorder's id"
        );
        assert_eq!(recap.overall_assessment, expected_assessment);
        assert_eq!(recap.strengths, expected_strengths);
        assert_eq!(
            recap.instrument, "trumpet",
            "canned recap instrument field must survive unchanged"
        );
    }

    #[test]
    fn end_session_errors_on_empty() {
        let (recorder, _) = make_recorder(canned_recap());
        let err = recorder.end_session().unwrap_err();
        assert!(
            matches!(err, SessionError::Empty),
            "empty session should return SessionError::Empty, got {err:?}"
        );
    }

    #[test]
    fn end_session_calculates_duration_from_start_to_now() {
        let (mut recorder, input_handle) = make_recorder(canned_recap());
        recorder.record_phrase(phrase(0));

        // Sleep a small but measurable amount so duration > 0.
        std::thread::sleep(std::time::Duration::from_millis(15));

        recorder.end_session().unwrap();

        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert!(
            captured.duration_secs > 0.0,
            "duration must be > 0 after real wall-clock time elapses, got {}",
            captured.duration_secs
        );
        assert!(
            captured.duration_secs < 60.0,
            "duration must be plausible (<60s) for a fast unit test, got {}",
            captured.duration_secs
        );
    }

    #[test]
    fn session_id_is_unique_across_recorders() {
        let a = SessionRecorder::new(
            "trumpet".to_owned(),
            Box::new(RecordingMockGenerator::new(canned_recap())),
        );
        let b = SessionRecorder::new(
            "trumpet".to_owned(),
            Box::new(RecordingMockGenerator::new(canned_recap())),
        );
        let c = SessionRecorder::new(
            "trumpet".to_owned(),
            Box::new(RecordingMockGenerator::new(canned_recap())),
        );

        let ids = [a.session_id(), b.session_id(), c.session_id()];
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert_ne!(ids[1], ids[2]);
    }

    #[test]
    fn recap_generator_failure_propagates_as_session_error() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned(), Box::new(FailingGenerator));
        recorder.record_phrase(phrase(0));

        let err = recorder.end_session().unwrap_err();
        match err {
            SessionError::RecapFailed(msg) => assert_eq!(msg, "llm blew up"),
            other => panic!("expected RecapFailed, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // SessionId behaviour
    // -----------------------------------------------------------------------

    #[test]
    fn session_id_roundtrip_via_from_str() {
        let id = SessionId::new();
        let parsed: SessionId = id.as_str().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn session_id_display_matches_as_str() {
        let id = SessionId::new();
        assert_eq!(format!("{id}"), id.as_str());
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let parsed: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn recap_serde_preserves_vectors() {
        let recap = canned_recap();
        let json = serde_json::to_string(&recap).unwrap();
        let parsed: SessionRecap = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, recap, "SessionRecap must roundtrip byte-for-byte");
    }
}
