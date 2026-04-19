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

/// A finalised practice session, with authoritative timestamps and all
/// recorded data. Produced by [`SessionRecorder::complete`].
///
/// This is the unit of persistence: once you have a `CompletedSession`,
/// you can store it, generate a recap from it (once or multiple times),
/// or discard it — independent of whether the LLM recap call succeeds.
/// Previously `end_session` coupled finalisation with recap generation,
/// which meant a transient LLM failure would destroy the session data
/// along with the consumed recorder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedSession {
    pub id: SessionId,
    pub instrument: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_secs: f64,
    pub phrases: Vec<PhraseSummary>,
    pub tips: Vec<RecordedTip>,
}

impl CompletedSession {
    /// How many phrases were recorded.
    pub fn phrase_count(&self) -> usize {
        self.phrases.len()
    }

    /// Build a [`RecapInput`] from this completed session. Keeps the
    /// LLM prompt inputs consistent with what actually happened.
    pub fn to_recap_input(&self) -> RecapInput {
        RecapInput {
            instrument: self.instrument.clone(),
            duration_secs: self.duration_secs,
            phrases: self.phrases.clone(),
            tips: self.tips.clone(),
        }
    }

    /// Generate a recap via the given [`RecapGenerator`].
    ///
    /// **Authoritative-fields guarantee:** regardless of what the
    /// generator emits, the returned recap's `duration_secs`,
    /// `phrase_count`, and `instrument` are overwritten with the values
    /// from this [`CompletedSession`]. An LLM can hallucinate or
    /// miscalculate these; the recorder is the source of truth.
    pub fn generate_recap(
        &self,
        generator: &dyn RecapGenerator,
    ) -> Result<SessionRecap, SessionError> {
        let input = self.to_recap_input();
        let mut recap = generator.generate_recap(&input)?;
        recap.duration_secs = self.duration_secs;
        recap.phrase_count = self.phrases.len();
        recap.instrument = self.instrument.clone();
        Ok(recap)
    }
}

/// Records a single practice session: phrases, coaching tips, and
/// wall-clock timing.
///
/// A recorder does NOT own a [`RecapGenerator`]; recap generation is a
/// separate step on the returned [`CompletedSession`]. This keeps
/// session data safe from LLM failures and lets callers choose whether
/// to generate a recap at all.
pub struct SessionRecorder {
    session_id: SessionId,
    instrument: String,
    started_at: DateTime<Utc>,
    phrases: Vec<PhraseSummary>,
    tips: Vec<RecordedTip>,
}

impl SessionRecorder {
    /// Create a new recorder for the given instrument. The start time
    /// is captured as `Utc::now()` at construction.
    pub fn new(instrument: String) -> Self {
        Self {
            session_id: SessionId::new(),
            instrument,
            started_at: Utc::now(),
            phrases: Vec::new(),
            tips: Vec::new(),
        }
    }

    /// Append a completed phrase to the session.
    pub fn record_phrase(&mut self, phrase: PhraseSummary) {
        self.phrases.push(phrase);
    }

    /// Append a coaching tip to the session.
    ///
    /// `phrase_index` should point into the phrases already recorded —
    /// it is stored verbatim and not validated, because phrases may be
    /// buffered in the UI before the recorder sees them and out-of-range
    /// values are still meaningful metadata for retrospective analysis.
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

    /// Finalise the session and return a [`CompletedSession`] with
    /// authoritative timestamps. Consumes `self` — a recorder is
    /// single-use.
    ///
    /// Returns [`SessionError::Empty`] if no phrases were recorded.
    ///
    /// Note: this no longer calls any recap generator. Call
    /// [`CompletedSession::generate_recap`] separately if/when you
    /// want a recap — that way a failed LLM call doesn't destroy the
    /// session data.
    pub fn complete(self) -> Result<CompletedSession, SessionError> {
        if self.phrases.is_empty() {
            return Err(SessionError::Empty);
        }
        let ended_at = Utc::now();
        let duration_secs = (ended_at - self.started_at).num_milliseconds() as f64 / 1000.0;
        Ok(CompletedSession {
            id: self.session_id,
            instrument: self.instrument,
            started_at: self.started_at,
            ended_at,
            duration_secs,
            phrases: self.phrases,
            tips: self.tips,
        })
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

    /// Convenience factory: returns a recorder plus a mock generator
    /// (kept separately, since the recorder no longer owns it) and a
    /// handle for inspecting the mock's last captured input after
    /// `generate_recap` is called.
    fn make_recorder(
        canned: SessionRecap,
    ) -> (
        SessionRecorder,
        RecordingMockGenerator,
        Arc<Mutex<Option<RecapInput>>>,
    ) {
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let recorder = SessionRecorder::new("trumpet".to_owned());
        (recorder, mock, input_handle)
    }

    // -----------------------------------------------------------------------
    // Behaviour tests
    // -----------------------------------------------------------------------

    #[test]
    fn recorder_starts_empty() {
        let (recorder, _, _) = make_recorder(canned_recap());
        assert_eq!(recorder.phrase_count(), 0);
        assert_eq!(recorder.tip_count(), 0);
        assert_eq!(recorder.instrument(), "trumpet");
    }

    #[test]
    fn record_phrase_accumulates() {
        let (mut recorder, _, _) = make_recorder(canned_recap());
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
        let (mut recorder, mock, input_handle) = make_recorder(canned_recap());

        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));
        recorder.record_tip(
            1,
            "Breathe before this entrance.".to_owned(),
            "suggestion".to_owned(),
            "expression".to_owned(),
        );

        assert_eq!(recorder.tip_count(), 1);

        // Complete + generate recap so the mock captures the input.
        let completed = recorder.complete().unwrap();
        completed.generate_recap(&mock).unwrap();
        let captured = input_handle.lock().unwrap().clone().unwrap();
        assert_eq!(captured.tips.len(), 1);
        assert_eq!(captured.tips[0].phrase_index, 1);
        assert_eq!(captured.tips[0].text, "Breathe before this entrance.");
        assert_eq!(captured.tips[0].severity, "suggestion");
        assert_eq!(captured.tips[0].category, "expression");
    }

    #[test]
    fn generate_recap_calls_generator_with_all_data() {
        let canned = canned_recap();
        let mock = RecordingMockGenerator::new(canned);
        let input_handle = mock.last_input_handle();
        let call_count = mock.call_count_handle();
        let mut recorder = SessionRecorder::new("violin".to_owned());

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

        let completed = recorder.complete().unwrap();
        completed.generate_recap(&mock).unwrap();

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "recap generator should be called exactly once per generate_recap"
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
    fn complete_returns_completed_session_with_authoritative_timestamps() {
        let (mut recorder, _mock, _) = make_recorder(canned_recap());
        let expected_id = recorder.session_id();
        let expected_started = recorder.started_at();
        recorder.record_phrase(phrase(0));

        let completed = recorder.complete().unwrap();
        assert_eq!(completed.id, expected_id);
        assert_eq!(completed.started_at, expected_started);
        assert!(
            completed.ended_at >= completed.started_at,
            "ended_at must not be earlier than started_at"
        );
        assert_eq!(completed.instrument, "trumpet");
        assert_eq!(completed.phrases.len(), 1);
    }

    #[test]
    fn complete_errors_on_empty_session() {
        let (recorder, _, _) = make_recorder(canned_recap());
        let err = recorder.complete().unwrap_err();
        assert!(
            matches!(err, SessionError::Empty),
            "empty session should return SessionError::Empty, got {err:?}"
        );
    }

    #[test]
    fn complete_calculates_duration_from_start_to_now() {
        let (mut recorder, _mock, _) = make_recorder(canned_recap());
        recorder.record_phrase(phrase(0));

        // Sleep a small but measurable amount so duration > 0.
        std::thread::sleep(std::time::Duration::from_millis(15));

        let completed = recorder.complete().unwrap();
        assert!(
            completed.duration_secs > 0.0,
            "duration must be > 0 after real wall-clock time elapses, got {}",
            completed.duration_secs,
        );
        assert!(
            completed.duration_secs < 60.0,
            "duration must be plausible (<60s) for a fast unit test, got {}",
            completed.duration_secs,
        );
    }

    #[test]
    fn session_id_is_unique_across_recorders() {
        let a = SessionRecorder::new("trumpet".to_owned());
        let b = SessionRecorder::new("trumpet".to_owned());
        let c = SessionRecorder::new("trumpet".to_owned());

        let ids = [a.session_id(), b.session_id(), c.session_id()];
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert_ne!(ids[1], ids[2]);
    }

    #[test]
    fn recap_generator_failure_propagates_as_session_error() {
        let mut recorder = SessionRecorder::new("trumpet".to_owned());
        recorder.record_phrase(phrase(0));
        let completed = recorder.complete().unwrap();

        let err = completed.generate_recap(&FailingGenerator).unwrap_err();
        match err {
            SessionError::RecapFailed(msg) => assert_eq!(msg, "llm blew up"),
            other => panic!("expected RecapFailed, got {other:?}"),
        }
    }

    #[test]
    fn generate_recap_overwrites_authoritative_fields() {
        // An LLM could hallucinate wrong values for duration / phrase_count /
        // instrument. The recap we return to callers must reflect what the
        // recorder actually observed, not whatever the generator emitted.
        let mut lying_recap = canned_recap();
        lying_recap.duration_secs = 999.0;
        lying_recap.phrase_count = 42;
        lying_recap.instrument = "tuba".to_owned(); // NB: recorder is trumpet

        let mock = RecordingMockGenerator::new(lying_recap);
        let mut recorder = SessionRecorder::new("trumpet".to_owned());
        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));
        std::thread::sleep(std::time::Duration::from_millis(5));

        let completed = recorder.complete().unwrap();
        let recap = completed.generate_recap(&mock).unwrap();

        assert_eq!(
            recap.instrument, "trumpet",
            "recorder instrument must override generator output"
        );
        assert_eq!(
            recap.phrase_count, 2,
            "phrase_count must be authoritative (actually-recorded count)"
        );
        assert!(
            (recap.duration_secs - completed.duration_secs).abs() < 1e-9,
            "duration_secs must match CompletedSession, not the generator's claim"
        );
        // And the fields the generator owns ARE preserved:
        assert!(!recap.overall_assessment.is_empty());
    }

    #[test]
    fn session_data_survives_recap_generator_failure() {
        // Regression for CR comment #1: a failing LLM must not destroy the
        // CompletedSession. Callers can retry or persist without a recap.
        let mut recorder = SessionRecorder::new("flute".to_owned());
        recorder.record_phrase(phrase(0));
        recorder.record_phrase(phrase(1));

        let completed = recorder.complete().unwrap();
        let saved_id = completed.id;
        let saved_count = completed.phrases.len();

        // First attempt fails.
        let err = completed.generate_recap(&FailingGenerator).unwrap_err();
        assert!(matches!(err, SessionError::RecapFailed(_)));

        // CompletedSession is still intact and usable.
        assert_eq!(completed.id, saved_id);
        assert_eq!(completed.phrase_count(), saved_count);

        // And a retry with a working generator succeeds.
        let retry_mock = RecordingMockGenerator::new(canned_recap());
        let recap = completed.generate_recap(&retry_mock).unwrap();
        assert_eq!(recap.phrase_count, saved_count);
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
