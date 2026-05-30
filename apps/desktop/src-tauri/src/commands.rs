//! Tauri command surface for Story #14 — free-play practice mode.
//!
//! Holds an [`AppState`] that tracks one active session at a time and
//! enforces a `Idle → Starting → Listening → Ending → Idle` state
//! machine. Exposes `start_practice_session`, `switch_instrument`,
//! `end_practice_session`, and `list_instruments` to the frontend.
//!
//! PR 1 is pure scaffolding: a [`MockCoachingService`] and
//! [`MockRecapGenerator`] return canned data so the full UI→backend
//! →recap loop can be driven without a network call or a live mic.
//! PR 3 replaces the mocks with real implementations behind the same
//! trait (`CoachingService`) and [`brain::session::RecapGenerator`].

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use brain::coaching::{
    CoachingCategory, CoachingConfig, CoachingEngine, CoachingSeverity, CoachingTip, ReqwestClient,
    SessionContext,
};
use brain::follower::ScorePosition;
use brain::phrase::PhraseSummary;
use brain::session::{
    CompletedSession, PracticeMode, RecapGenerator, RecapInput, ScoreId, SessionError,
    SessionRecap, SessionRecorder,
};
use brain::stats::PracticeStats;
use brain::store::{ScoreLibraryEntry, ScoreStore, SessionStore, SessionSummary, StoredSession};
use chrono::{DateTime, Utc};
use ears::profile::{InstrumentProfile, ProfileLoader};

use crate::audio_pipeline::{AudioPipeline, DetectorProfile, PipelineError};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, Runtime, State};
use thiserror::Error;
use tokio::sync::Mutex;

/// Realtime-safe lower bound on the YIN detector's frequency window.
///
/// YIN's window scales as `2 × sample_rate / freq_min_hz`; at 44.1 kHz
/// a 60 Hz floor is ~1470 samples (~33 ms), which fits alongside the
/// rest of the pipeline inside the project's latency budget. Lower
/// floors (e.g. Piano's 28 Hz) would explode the window and miss the
/// budget on exactly the instruments that need the widest UI range.
/// Matches the `PitchConfig::default()` floor shipped by the `ears`
/// crate — keep them in sync.
const DETECTOR_MIN_HZ: f64 = 60.0;

// ---------------------------------------------------------------------------
// IPC types
// ---------------------------------------------------------------------------

/// UI-facing instrument descriptor. Matches the TS `InstrumentInfo` in
/// `apps/desktop/src/types/brain.ts`.
///
/// Sourced from `profiles/*.json` at startup — see `AppState::new`.
/// Every field reflects what the `profiles/` files contain, so adding
/// an instrument stays a one-file change: drop a JSON in `profiles/`,
/// the UI picks it up on next launch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentInfo {
    pub name: String,
    pub family: String,
    pub freq_min_hz: f64,
    pub freq_max_hz: f64,
    pub vibrato_tolerance_cents: f64,
    pub emoji: String,
}

/// Payload of the `session-status` Tauri event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionStatusPayload {
    pub status: &'static str,
}

/// Payload of the `segment-changed` Tauri event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentChangedPayload {
    pub segment_id: String,
    pub instrument: String,
    pub started_at: DateTime<Utc>,
}

/// IPC representation of a session summary for history listing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummaryDto {
    pub id: String,
    pub instrument: String,
    pub started_at: DateTime<Utc>,
    pub duration_secs: f64,
    pub phrase_count: usize,
}

impl From<SessionSummary> for SessionSummaryDto {
    fn from(s: SessionSummary) -> Self {
        Self {
            id: s.id.as_str().to_owned(),
            instrument: s.instrument,
            started_at: s.started_at,
            duration_secs: s.duration_secs,
            phrase_count: s.phrase_count,
        }
    }
}

/// IPC representation of a full session with recap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSessionDto {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub recap: SessionRecap,
}

impl From<StoredSession> for StoredSessionDto {
    fn from(s: StoredSession) -> Self {
        Self {
            id: s.id.as_str().to_owned(),
            started_at: s.started_at,
            ended_at: s.ended_at,
            recap: s.recap,
        }
    }
}

/// IPC representation of practice statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PracticeStatsDto {
    pub total_sessions: usize,
    pub total_time_secs: f64,
    pub sessions_this_week: usize,
    pub avg_session_length_secs: f64,
    pub trend: String,
}

impl From<PracticeStats> for PracticeStatsDto {
    fn from(s: PracticeStats) -> Self {
        let trend = match s.trend {
            brain::stats::Trend::Up => "up",
            brain::stats::Trend::Down => "down",
            brain::stats::Trend::Stable => "stable",
        };
        Self {
            total_sessions: s.total_sessions,
            total_time_secs: s.total_time_secs,
            sessions_this_week: s.sessions_this_week,
            avg_session_length_secs: s.avg_session_length_secs,
            trend: trend.to_owned(),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// All error paths from the command surface. Serialised to the
/// frontend as strings (`Result<_, String>`) to keep the IPC contract
/// narrow.
#[derive(Debug, Error)]
pub enum CommandError {
    #[error("a practice session is already active — end it before starting a new one")]
    AlreadyActive,
    #[error("no active practice session — start one first")]
    NotActive,
    #[error("session is already ending — wait for it to finish")]
    AlreadyEnding,
    #[error("instrument name cannot be empty")]
    EmptyInstrument,
    #[error("unknown instrument: {0}")]
    UnknownInstrument(String),
    #[error("recorder error: {0}")]
    Recorder(#[from] SessionError),
    #[error("session store error: {0}")]
    Store(#[from] brain::store::StoreError),
}

impl CommandError {
    fn to_frontend(&self) -> String {
        self.to_string()
    }
}

// ---------------------------------------------------------------------------
// CoachingService trait + mock
// ---------------------------------------------------------------------------

/// Trait over "produce a coaching tip for a phrase". PR 1 uses only
/// [`MockCoachingService`]; PR 3 wires the real `CoachingEngine`
/// behind this same trait.
#[async_trait]
pub trait CoachingService: Send + Sync {
    /// May return `None` to indicate "no tip worth showing" (e.g.
    /// rate-limited, empty phrase).
    async fn get_tip(
        &self,
        phrase: &PhraseSummary,
        context: &SessionContext,
    ) -> Option<CoachingTip>;
}

/// Real coaching service backed by the Claude API.
pub struct LlmCoachingService {
    engine: Option<Arc<Mutex<CoachingEngine>>>,
}

impl LlmCoachingService {
    /// Create a new LLM coaching service, attempting to initialize a real
    /// coaching engine. If no API key is configured, coaching tips will
    /// not be generated (coaching will be disabled).
    pub fn new() -> Self {
        let engine =
            match CoachingEngine::new(CoachingConfig::default(), Box::new(ReqwestClient::new())) {
                Ok(e) => Some(Arc::new(Mutex::new(e))),
                Err(_) => None,
            };
        Self { engine }
    }

    /// Check if coaching is available. When false, get_tip will return None.
    pub fn coaching_available(&self) -> bool {
        self.engine.is_some()
    }
}

impl Default for LlmCoachingService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CoachingService for LlmCoachingService {
    async fn get_tip(
        &self,
        phrase: &PhraseSummary,
        context: &SessionContext,
    ) -> Option<CoachingTip> {
        match &self.engine {
            Some(engine_arc) => {
                let mut engine = engine_arc.lock().await;
                engine.get_tip(phrase, context).await.ok()
            }
            None => None,
        }
    }
}

/// Deterministic stub. Rotates through a small set of canned tips.
pub struct MockCoachingService {
    tips: Vec<CoachingTip>,
}

impl MockCoachingService {
    pub fn new() -> Self {
        Self {
            tips: vec![
                CoachingTip {
                    text: "Nice steady tone. Try letting the end of the phrase breathe.".to_owned(),
                    severity: CoachingSeverity::Encouragement,
                    category: CoachingCategory::Tone,
                },
                CoachingTip {
                    text: "Watch the intonation on the top note — a touch sharp there.".to_owned(),
                    severity: CoachingSeverity::Suggestion,
                    category: CoachingCategory::Intonation,
                },
                CoachingTip {
                    text: "Good dynamic shape. Keep that forward motion through the line."
                        .to_owned(),
                    severity: CoachingSeverity::Encouragement,
                    category: CoachingCategory::Expression,
                },
            ],
        }
    }
}

impl Default for MockCoachingService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CoachingService for MockCoachingService {
    async fn get_tip(
        &self,
        phrase: &PhraseSummary,
        _context: &SessionContext,
    ) -> Option<CoachingTip> {
        self.tips
            .get(phrase.phrase_index % self.tips.len())
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// LLM Recap Generator (real implementation for PR 3)
// ---------------------------------------------------------------------------

/// Production recap generator that calls the Claude API for natural-language
/// session summaries. Respects API key configuration and gracefully
/// degrades to fallback text if the API is unavailable.
pub struct LlmRecapGenerator {
    engine: Option<Arc<Mutex<CoachingEngine>>>,
}

impl LlmRecapGenerator {
    /// Create a new LLM recap generator, attempting to initialize a real
    /// coaching engine. If no API key is configured, returns a generator
    /// with no engine (coaching_available() will return false).
    pub fn new() -> Self {
        let engine =
            match CoachingEngine::new(CoachingConfig::default(), Box::new(ReqwestClient::new())) {
                Ok(e) => Some(Arc::new(Mutex::new(e))),
                Err(_) => None,
            };
        Self { engine }
    }

    /// Check if a real coaching engine is available. When false, the
    /// generator will use fallback text instead of calling the API.
    pub fn coaching_available(&self) -> bool {
        self.engine.is_some()
    }
}

impl Default for LlmRecapGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RecapGenerator for LlmRecapGenerator {
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        let Some(engine_arc) = &self.engine else {
            // No API key: return fallback recap with canned text.
            return Ok(SessionRecap {
                overall_assessment: format!(
                    "Nice {}-minute session. You kept the tone centered and stayed with the music.",
                    (input.duration_secs / 60.0).round().max(1.0) as u32,
                ),
                strengths: vec![
                    "Consistent, focused tone throughout the session.".to_owned(),
                    "Good breath support and phrasing.".to_owned(),
                ],
                areas_to_improve: vec![
                    "Intonation wandered slightly on the upper register.".to_owned()
                ],
                next_session_suggestions: vec![
                    "Open with long tones in the key you ended on.".to_owned(),
                    "Try a slow scale with a drone to tune up the top of the range.".to_owned(),
                ],
                duration_secs: 0.0,
                phrase_count: 0,
                instrument: String::new(),
            });
        };

        // Call the LLM coaching engine for the full recap.
        let engine = engine_arc.lock().await;
        engine.generate_recap(input).await
    }
}

// ---------------------------------------------------------------------------
// MockRecapGenerator
// ---------------------------------------------------------------------------

/// Stub recap generator. Authoritative fields (`instrument`,
/// `phrase_count`, `duration_secs`) are overwritten by
/// [`CompletedSession::generate_recap`], so the canned text here is
/// only the narrative shell.
pub struct MockRecapGenerator;

#[async_trait]
impl RecapGenerator for MockRecapGenerator {
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        // Call the sync version for now - in real code this would be async
        self.generate_recap_impl(input)
    }
}

impl MockRecapGenerator {
    fn generate_recap_impl(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        Ok(SessionRecap {
            overall_assessment: format!(
                "Nice {}-minute session. You kept the tone centered and stayed with the music.",
                (input.duration_secs / 60.0).round().max(1.0) as u32,
            ),
            strengths: vec![
                "Consistent, focused tone throughout the session.".to_owned(),
                "Good breath support and phrasing.".to_owned(),
            ],
            areas_to_improve: vec!["Intonation wandered slightly on the upper register.".to_owned()],
            next_session_suggestions: vec![
                "Open with long tones in the key you ended on.".to_owned(),
                "Try a slow scale with a drone to tune up the top of the range.".to_owned(),
            ],
            duration_secs: 0.0,
            phrase_count: 0,
            instrument: String::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Backend-visible lifecycle phases. The frontend's `Recap` screen is
/// UI-only — the backend goes straight back to `Idle` when
/// `end_practice_session` resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Idle,
    Starting,
    Listening,
    Ending,
}

impl SessionPhase {
    fn status_label(self) -> Option<&'static str> {
        match self {
            SessionPhase::Idle => None,
            SessionPhase::Starting => Some("starting"),
            SessionPhase::Listening => Some("listening"),
            SessionPhase::Ending => Some("ending"),
        }
    }
}

/// In-flight session state.
pub struct ActiveSession {
    phase: SessionPhase,
    recorder: SessionRecorder,
    practice_mode: PracticeMode,
    /// Held so PR 2 can spawn tokio tasks off a phrase completion.
    /// Unused directly in PR 1 — hence the `allow(dead_code)`.
    #[allow(dead_code)]
    coaching: Arc<dyn CoachingService>,
}

impl ActiveSession {
    fn new(
        instrument: String,
        practice_mode: PracticeMode,
        coaching: Arc<dyn CoachingService>,
    ) -> Self {
        Self {
            phase: SessionPhase::Starting,
            recorder: SessionRecorder::new(instrument, practice_mode),
            practice_mode,
            coaching,
        }
    }
}

/// Global backend state held via `tauri::Manager`.
///
/// `Mutex<Option<ActiveSession>>` guarantees at most one session exists
/// at a time — the double-start test verifies this.
pub struct AppState {
    active_session: Mutex<Option<ActiveSession>>,
    coaching_factory: Arc<dyn Fn() -> Arc<dyn CoachingService> + Send + Sync>,
    recap_generator: Arc<dyn RecapGenerator>,
    /// `std::sync::Mutex` because rusqlite's `Connection` wraps a
    /// `RefCell` internally, making `SessionStore: !Sync`. Tauri's
    /// managed `State<T>` requires `T: Sync`, so we interpose a mutex.
    /// Critical sections are short (single SQL query) and we never hold
    /// this lock across an `.await`.
    session_store: std::sync::Mutex<SessionStore>,
    score_store: std::sync::Mutex<ScoreStore>,
    coaching_available: bool,
    /// Instrument catalog loaded once at construction, shared by the
    /// `list_instruments` command and by session-validation paths.
    /// Held behind Arc so clones into IPC responses are cheap.
    instruments: Arc<Vec<InstrumentInfo>>,
    /// Live mic → pitch-detector → `audio-event` pipeline. `Some` only
    /// between `start_practice_session` and `end_practice_session`;
    /// swapped in place by `switch_instrument`. Held in a separate
    /// mutex from `active_session` because the pipeline only needs the
    /// instrument profile (not the recorder), and we prefer to hand the
    /// profile in from the command wrapper without briefly holding two
    /// locks at once.
    audio_pipeline: Mutex<Option<AudioPipeline>>,
}

impl AppState {
    /// Production constructor — opens the SessionStore and ScoreStore at the
    /// platform default location, wires the real coaching engine, and loads
    /// the instrument catalog from `profiles/*.json`.
    ///
    /// Resolves profiles via env override → workspace walk (see
    /// [`locate_profiles_dir`]). For a *packaged* build, prefer
    /// [`AppState::new_with_app_handle`], which also checks the bundled
    /// resource directory — a bare `new()` in an installed app would fail
    /// to find profiles and panic (the bug behind #112).
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Production constructor for packaged builds. Identical to [`new`](Self::new)
    /// except it can resolve `profiles/` from the app bundle's resource
    /// directory via the `AppHandle`, which is the only location that
    /// exists in an installed app. Wire this from `Builder::setup` so the
    /// handle is available.
    pub fn new_with_app_handle(app_handle: &tauri::AppHandle) -> Self {
        Self::build(Some(app_handle))
    }

    /// Shared constructor body. `app_handle` is `Some` only for packaged
    /// builds (enables bundled-resource profile resolution).
    fn build(app_handle: Option<&tauri::AppHandle>) -> Self {
        let db_path = SessionStore::default_path().unwrap_or_else(|_| {
            // Fallback: use in-memory if default path unavailable
            // (extremely rare — headless/no data dir).
            std::path::PathBuf::from(":memory:")
        });

        let session_store = SessionStore::open(&db_path)
            .unwrap_or_else(|_| SessionStore::in_memory().expect("in-memory store must succeed"));

        let score_store = ScoreStore::open(&db_path)
            .unwrap_or_else(|_| ScoreStore::in_memory().expect("in-memory store must succeed"));

        let coaching_svc = LlmCoachingService::new();
        let coaching_available = coaching_svc.coaching_available();
        let recap_gen = LlmRecapGenerator::new();

        Self {
            active_session: Mutex::new(None),
            coaching_factory: Arc::new(move || Arc::new(LlmCoachingService::new())),
            recap_generator: Arc::new(recap_gen),
            session_store: std::sync::Mutex::new(session_store),
            score_store: std::sync::Mutex::new(score_store),
            coaching_available,
            instruments: Arc::new(load_instrument_catalog(app_handle)),
            audio_pipeline: Mutex::new(None),
        }
    }

    /// Wire entirely with mocks and in-memory store. Used by tests.
    pub fn with_mocks() -> Self {
        Self {
            active_session: Mutex::new(None),
            coaching_factory: Arc::new(|| Arc::new(MockCoachingService::new())),
            recap_generator: Arc::new(MockRecapGenerator),
            session_store: std::sync::Mutex::new(
                SessionStore::in_memory().expect("in-memory store must succeed"),
            ),
            score_store: std::sync::Mutex::new(
                ScoreStore::in_memory().expect("in-memory store must succeed"),
            ),
            coaching_available: false,
            instruments: Arc::new(test_instrument_catalog()),
            audio_pipeline: Mutex::new(None),
        }
    }

    /// Check whether a given instrument name is present in the catalog.
    /// Used by `start_practice_session` and `switch_instrument` to
    /// reject unknown names before touching the session recorder.
    pub fn is_known_instrument(&self, name: &str) -> bool {
        self.instruments.iter().any(|i| i.name == name)
    }

    /// Look up the frequency window for the named instrument. Returns
    /// `None` for unknown names — callers should validate via
    /// `is_known_instrument` first. Used by the audio pipeline to size
    /// the pitch detector's window.
    ///
    /// The detector's `freq_min_hz` is *clamped up* from the catalog's
    /// absolute floor to a realtime-safe minimum. Rationale: YIN's
    /// `window_size ≈ 2 × sample_rate / freq_min_hz`, so letting
    /// Piano's 28 Hz floor through would give a 3150-sample window
    /// (~71 ms at 44.1 kHz) per detect — blowing past the project's
    /// <25 ms latency budget for exactly the instruments with the
    /// widest ranges. The UI catalog still shows the full instrument
    /// range; only the detector gets the clamped floor. Notes below
    /// the floor arrive as `pitch_hz: None` (below-range), which the UI
    /// already handles.
    pub fn detector_profile_for(&self, name: &str) -> Option<DetectorProfile> {
        self.instruments
            .iter()
            .find(|i| i.name == name)
            .map(|i| DetectorProfile {
                // YIN threshold of 0.15 is the ears-crate default and
                // works across brass/strings/voice. Per-instrument
                // overrides belong in `profiles/*.json` later — for now
                // the detector config is purely frequency-windowed.
                threshold: 0.15,
                freq_min_hz: i.freq_min_hz.max(DETECTOR_MIN_HZ),
                freq_max_hz: i.freq_max_hz,
            })
    }

    /// Install the audio pipeline for the currently-active session,
    /// but only if the session is still live.
    ///
    /// Returns the pipeline back to the caller if the session was torn
    /// down while mic startup was in flight — the caller is expected
    /// to drop it (which joins the worker + releases the mic), so we
    /// don't leak a hot pipeline into an idle app.
    ///
    /// Replaces any previous pipeline in place when installed, so
    /// edge cases like a failed mid-session reconfigure followed by a
    /// retry stay safe — `AudioPipeline::Drop` joins the old worker
    /// before the new one takes over.
    pub(crate) async fn install_audio_pipeline(
        &self,
        pipeline: AudioPipeline,
    ) -> Result<(), AudioPipeline> {
        // Taking the session lock briefly keeps this check atomic with
        // respect to `end_practice_session_impl`, which drains the
        // session under the same lock before calling
        // `stop_audio_pipeline`.
        let session_guard = self.active_session.lock().await;
        if !matches!(
            session_guard.as_ref().map(|s| s.phase),
            Some(SessionPhase::Listening)
        ) {
            return Err(pipeline);
        }
        drop(session_guard);
        let mut guard = self.audio_pipeline.lock().await;
        *guard = Some(pipeline);
        Ok(())
    }

    /// Swap the detector profile on the currently-running pipeline
    /// without tearing down the mic stream. No-op if no pipeline is
    /// running (e.g. if capture init failed at session start — we don't
    /// want a mid-session switch to fail loudly because of that).
    pub(crate) async fn reconfigure_audio_pipeline(
        &self,
        profile: DetectorProfile,
    ) -> Result<(), PipelineError> {
        let guard = self.audio_pipeline.lock().await;
        if let Some(p) = guard.as_ref() {
            p.reconfigure(profile)?;
        }
        Ok(())
    }

    /// Tear down the audio pipeline (stops mic, joins worker thread).
    pub(crate) async fn stop_audio_pipeline(&self) {
        let mut guard = self.audio_pipeline.lock().await;
        if let Some(pipeline) = guard.take() {
            pipeline.stop();
        }
    }

    /// Build a [`ScoreFollower`] for the given score id by loading its
    /// stored MusicXML and parsing the part it was imported under.
    ///
    /// Returns `None` (and logs) on any failure — an unknown id, a bad
    /// uuid, or unparseable MusicXML. Score following is an enhancement,
    /// not a precondition: if it can't be built the session still runs,
    /// just without a moving cursor. This keeps "couldn't follow the
    /// score" from ever looking like "couldn't start practising".
    fn build_follower(&self, score_id: &str) -> Option<brain::follower::ScoreFollower> {
        let id: ScoreId = match score_id.parse() {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, score_id, "invalid score id; starting without follower");
                return None;
            }
        };
        let (music_xml, part_index) = {
            let store = match self.score_store.lock() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "score store lock poisoned; no follower");
                    return None;
                }
            };
            match store.get(id) {
                Ok(entry) => (entry.music_xml, entry.part_index),
                Err(e) => {
                    tracing::warn!(error = %e, score_id, "score not found; no follower");
                    return None;
                }
            }
        };
        match brain::follower::ScoreFollower::from_musicxml_str(&music_xml, part_index) {
            Ok(f) => Some(f),
            Err(e) => {
                tracing::warn!(error = %e, score_id, "score MusicXML did not parse; no follower");
                None
            }
        }
    }

    /// Look up a score's title by id for recap context. Returns `None` on
    /// a bad id, a lock failure, or an unknown score — all non-fatal: the
    /// session simply falls back to free-play recap framing.
    fn score_title_for(&self, score_id: &str) -> Option<String> {
        let id: ScoreId = score_id.parse().ok()?;
        let store = self.score_store.lock().ok()?;
        store.get(id).ok().map(|entry| entry.title)
    }

    /// Clone the full instrument catalog for an IPC response.
    pub fn list_instruments(&self) -> Vec<InstrumentInfo> {
        (*self.instruments).clone()
    }

    /// Count of instruments in the catalog. Tests use this to assert
    /// the catalog was loaded.
    #[cfg(test)]
    fn instrument_count(&self) -> usize {
        self.instruments.len()
    }

    /// Check if coaching (LLM tips and recap) is available.
    /// Returns false if no API key is configured.
    pub fn coaching_available(&self) -> bool {
        self.coaching_available
    }

    /// Peek at the current phase. Returns `Idle` when no session
    /// exists. Exposed so tests can assert on the state machine
    /// without holding the internal lock themselves.
    pub async fn current_phase(&self) -> SessionPhase {
        match &*self.active_session.lock().await {
            Some(s) => s.phase,
            None => SessionPhase::Idle,
        }
    }

    /// Get the current session's instrument name. Returns None if no session is active.
    pub async fn active_session_instrument(&self) -> Option<String> {
        self.active_session
            .lock()
            .await
            .as_ref()
            .and_then(|s| s.recorder.current_instrument().map(|i| i.to_owned()))
    }

    /// Title of the active session's score, if score-backed. Used to give
    /// the live coach piece context. `None` in free play or when idle.
    pub async fn active_session_score_title(&self) -> Option<String> {
        self.active_session
            .lock()
            .await
            .as_ref()
            .and_then(|s| s.recorder.score_title().map(|t| t.to_owned()))
    }

    /// Request a coaching tip from the coaching service for the given phrase.
    pub async fn get_coaching_tip(
        &self,
        phrase: &PhraseSummary,
        context: &SessionContext,
    ) -> Result<Option<CoachingTip>, CommandError> {
        let coaching = (self.coaching_factory)();
        Ok(coaching.get_tip(phrase, context).await)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Instrument catalog (loaded from `profiles/*.json` at startup)
// ---------------------------------------------------------------------------

/// Convert an [`InstrumentProfile`] into the UI-facing [`InstrumentInfo`].
///
/// The profile's `family` is serde-snake_case on disk; the IPC surface
/// uses title-case display names (matches the TS expectation and the
/// existing family-badge coloring in `InstrumentSelector.tsx`).
fn profile_to_info(profile: &InstrumentProfile) -> InstrumentInfo {
    InstrumentInfo {
        name: profile.name.clone(),
        family: profile.family.display_name().to_owned(),
        freq_min_hz: profile.freq_min_hz,
        freq_max_hz: profile.freq_max_hz,
        vibrato_tolerance_cents: profile.vibrato_tolerance_cents,
        emoji: profile.emoji.clone(),
    }
}

/// Locate the `profiles/` directory on disk.
///
/// Resolution order:
/// 1. `$AI_MUSIC_COMPANION_PROFILES_DIR` — explicit override hook.
/// 2. Bundled app resources (`AppHandle::path().resource_dir()/profiles`) —
///    only available when `app_handle` is `Some`, i.e. a packaged build
///    from `cargo tauri build`. This is the only location that exists in
///    an installed app, so resolving it here is what fixes #112 (the app
///    panicking on every bundled install).
/// 3. `CARGO_MANIFEST_DIR/../../../profiles` — dev/workspace fallback.
///    `CARGO_MANIFEST_DIR` is baked in at compile time for cargo builds
///    (including `cargo tauri dev`) and points at this crate
///    (`apps/desktop/src-tauri`); three hops up reach the workspace root.
fn locate_profiles_dir(app_handle: Option<&tauri::AppHandle>) -> std::path::PathBuf {
    if let Ok(explicit) = std::env::var("AI_MUSIC_COMPANION_PROFILES_DIR") {
        return std::path::PathBuf::from(explicit);
    }
    // Packaged builds: profiles ship under the bundle's resource dir
    // (declared in `tauri.conf.json` `bundle.resources`). Only take this
    // path when the dir actually exists, so a dev build with a handle
    // still falls through to the workspace walk.
    if let Some(handle) = app_handle {
        if let Ok(resource_dir) = handle.path().resource_dir() {
            let bundled = resource_dir.join("profiles");
            if bundled.is_dir() {
                return bundled;
            }
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("workspace root should exist three levels above this crate")
        .join("profiles")
}

/// Load the canonical instrument catalog from `profiles/*.json`.
///
/// `app_handle` is `Some` for packaged builds (enables bundled-resource
/// resolution) and `None` for dev/test (workspace walk).
///
/// Panics with a clear message on failure — the app is unusable without
/// instruments, and silent degradation to an empty catalog would leave
/// the user staring at an empty selector with no feedback about why.
fn load_instrument_catalog(app_handle: Option<&tauri::AppHandle>) -> Vec<InstrumentInfo> {
    let dir = locate_profiles_dir(app_handle);
    let profiles = ProfileLoader::load_all(&dir).unwrap_or_else(|e| {
        panic!(
            "failed to load instrument profiles from {}: {}. \
             Set AI_MUSIC_COMPANION_PROFILES_DIR to override the location.",
            dir.display(),
            e
        )
    });
    if profiles.is_empty() {
        panic!(
            "no instrument profiles found in {}. Check that the profiles/ \
             directory is populated; set AI_MUSIC_COMPANION_PROFILES_DIR to override.",
            dir.display()
        );
    }
    profiles.iter().map(profile_to_info).collect()
}

/// Deterministic catalog used by `AppState::with_mocks` so tests don't
/// touch the filesystem. Covers every instrument any test in this
/// module refers to (Trumpet, Piano, Voice, Violin, Flute, Cello) plus
/// the rest of the prod catalog for coverage. Must stay in sync with
/// the names used in `profiles/*.json`.
fn test_instrument_catalog() -> Vec<InstrumentInfo> {
    // Keep shapes similar to the real profiles so tests that round-trip
    // through IPC payloads don't surprise us later. Emoji is cosmetic
    // and doesn't need to match the real catalog exactly.
    [
        ("Trumpet", "Brass", 165.0, 1047.0),
        ("Trombone", "Brass", 58.0, 587.0),
        ("French Horn", "Brass", 87.0, 880.0),
        ("Violin", "Strings", 196.0, 3136.0),
        ("Cello", "Strings", 65.0, 988.0),
        ("Flute", "Woodwind", 262.0, 2093.0),
        ("Clarinet", "Woodwind", 147.0, 1568.0),
        ("Voice", "Voice", 82.0, 1047.0),
        ("Piano", "Keyboard", 28.0, 4186.0),
    ]
    .into_iter()
    .map(|(name, family, lo, hi)| InstrumentInfo {
        name: name.to_owned(),
        family: family.to_owned(),
        freq_min_hz: lo,
        freq_max_hz: hi,
        vibrato_tolerance_cents: 25.0,
        emoji: String::new(),
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Emit helpers
// ---------------------------------------------------------------------------

fn emit_session_status<R: Runtime>(app: &tauri::AppHandle<R>, phase: SessionPhase) {
    if let Some(status) = phase.status_label() {
        // Event emission errors are non-fatal — the UI just misses a
        // status beat. Swallow and continue.
        let _ = app.emit("session-status", SessionStatusPayload { status });
    }
}

fn emit_segment_changed<R: Runtime>(app: &tauri::AppHandle<R>, payload: SegmentChangedPayload) {
    let _ = app.emit("segment-changed", payload);
}

/// Emit a completed phrase to the frontend. Carries `score_position`
/// (measure/beat) when a score follower is attached — that's what the
/// score-mode cursor advances to. Non-fatal on error: a dropped event
/// just means the cursor misses one hop.
fn emit_phrase_detected<R: Runtime>(app: &tauri::AppHandle<R>, phrase: PhraseSummary) {
    let _ = app.emit("phrase-detected", phrase);
}

/// Emit the follower's live score position (~10 Hz) so the cursor glides
/// between phrase boundaries. Only fires in score mode. Non-fatal on
/// error: a dropped tick just means one skipped frame of cursor motion.
fn emit_score_position_updated<R: Runtime>(app: &tauri::AppHandle<R>, position: ScorePosition) {
    let _ = app.emit("score-position-updated", position);
}

// ---------------------------------------------------------------------------
// Command handlers — pure (testable) implementations
// ---------------------------------------------------------------------------

/// Pure implementation of `start_practice_session`, separated from the
/// `#[tauri::command]` wrapper so tests can drive it without a Tauri
/// runtime.
pub async fn start_practice_session_impl(
    state: &AppState,
    instrument: String,
    practice_mode: PracticeMode,
    _coaching_enabled: bool,
    score_title: Option<String>,
) -> Result<String, CommandError> {
    if instrument.trim().is_empty() {
        return Err(CommandError::EmptyInstrument);
    }
    if !state.is_known_instrument(&instrument) {
        return Err(CommandError::UnknownInstrument(instrument));
    }

    let mut guard = state.active_session.lock().await;
    if guard.is_some() {
        return Err(CommandError::AlreadyActive);
    }

    let coaching = (state.coaching_factory)();
    let mut session = ActiveSession::new(instrument, practice_mode, coaching);
    // Tag the recorder with the score title (if score-backed) so the
    // end-of-session recap can name the piece and cite measures.
    session.recorder.set_score_title(score_title);
    let session_id = session.recorder.session_id().as_str();
    // Starting → Listening is synchronous in PR 1. PR 2 inserts a real
    // pause once audio capture startup is async.
    session.phase = SessionPhase::Listening;

    *guard = Some(session);
    Ok(session_id)
}

/// Pure implementation of `switch_instrument`. Returns the new segment
/// id plus the wall-clock timestamp reported to the UI.
pub async fn switch_instrument_impl(
    state: &AppState,
    instrument: String,
    practice_mode: PracticeMode,
) -> Result<(String, DateTime<Utc>), CommandError> {
    if instrument.trim().is_empty() {
        return Err(CommandError::EmptyInstrument);
    }
    if !state.is_known_instrument(&instrument) {
        return Err(CommandError::UnknownInstrument(instrument));
    }

    let mut guard = state.active_session.lock().await;
    let session = guard.as_mut().ok_or(CommandError::NotActive)?;
    if session.phase != SessionPhase::Listening {
        return Err(CommandError::AlreadyEnding);
    }

    session.practice_mode = practice_mode;
    let new_segment_id = session
        .recorder
        .switch_instrument(instrument, practice_mode)?;
    Ok((new_segment_id.as_str(), Utc::now()))
}

/// Pure implementation of `end_practice_session`.
///
/// Per design doc §8 q3, a session with zero phrases returns a calm
/// empty-state recap rather than erroring — the recorder would
/// normally return `SessionError::Empty`, which we intercept.
pub async fn end_practice_session_impl(state: &AppState) -> Result<SessionRecap, CommandError> {
    let taken = {
        let mut guard = state.active_session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Err(CommandError::NotActive);
        };
        session.phase = SessionPhase::Ending;
        guard.take()
    };
    let session = taken.expect("session was Some under the lock we just took");
    let generator = Arc::clone(&state.recap_generator);

    match session.recorder.complete() {
        Ok(completed) => build_recap(&completed, &*generator).await,
        Err(SessionError::Empty) => Ok(empty_state_recap()),
        Err(other) => Err(CommandError::Recorder(other)),
    }
}

/// Pure implementation of `list_instruments`. Returns a clone of the
/// catalog cached on `AppState` — catalog loading happens once in
/// `AppState::new` and is shared across IPC calls.
pub fn list_instruments_impl(state: &AppState) -> Vec<InstrumentInfo> {
    state.list_instruments()
}

async fn build_recap(
    completed: &CompletedSession,
    generator: &dyn RecapGenerator,
) -> Result<SessionRecap, CommandError> {
    completed
        .generate_recap(generator)
        .await
        .map_err(CommandError::from)
}

/// Minimal recap for the zero-phrase case. Tone is intentionally
/// "yoga teacher not gym coach" — we don't shame the user for
/// not playing.
fn empty_state_recap() -> SessionRecap {
    SessionRecap {
        overall_assessment:
            "Looks like you didn't get to play this time — come back when you're ready.".to_owned(),
        strengths: Vec::new(),
        areas_to_improve: Vec::new(),
        next_session_suggestions: vec![
            "Open the app a few minutes before you want to play — just having it running can help."
                .to_owned(),
        ],
        duration_secs: 0.0,
        phrase_count: 0,
        instrument: String::new(),
    }
}

// ---------------------------------------------------------------------------
// History command implementations (Story #17)
// ---------------------------------------------------------------------------

/// Pure implementation of `get_session_history`.
///
/// Returns filtered session summaries. If `instrument_filter` is None,
/// all sessions are returned. If date range is None, no date filtering.
pub fn get_session_history_impl(
    state: &AppState,
    instrument_filter: Option<String>,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
) -> Result<Vec<SessionSummaryDto>, CommandError> {
    let store = state
        .session_store
        .lock()
        .expect("session store mutex poisoned");
    let sessions = if let Some(instrument) = instrument_filter {
        store.list_by_instrument(Some(&instrument))?
    } else if start_date.is_some() || end_date.is_some() {
        store.list_by_date_range(start_date, end_date)?
    } else {
        // No filters — return all sessions, reasonable limit for UI
        store.list_recent(1000)?
    };

    Ok(sessions.into_iter().map(SessionSummaryDto::from).collect())
}

/// Pure implementation of `get_session_detail`.
pub fn get_session_detail_impl(
    state: &AppState,
    session_id: String,
) -> Result<StoredSessionDto, CommandError> {
    use brain::session::SessionId;
    let id = SessionId::from_str(&session_id)
        .map_err(|_| CommandError::Store(brain::store::StoreError::NotFound(session_id)))?;
    let session = state
        .session_store
        .lock()
        .expect("session store mutex poisoned")
        .load(id)?;
    Ok(StoredSessionDto::from(session))
}

/// Pure implementation of `get_practice_stats`.
pub fn get_practice_stats_impl(state: &AppState) -> Result<PracticeStatsDto, CommandError> {
    let all_sessions = state
        .session_store
        .lock()
        .expect("session store mutex poisoned")
        .list_recent(10000)?;
    let stats = PracticeStats::calculate(&all_sessions, Utc::now());
    Ok(PracticeStatsDto::from(stats))
}

// ---------------------------------------------------------------------------
// Tauri command wrappers
// ---------------------------------------------------------------------------

/// Start a new practice session. Emits `session-status` as
/// `starting` then `listening`, and spins up the mic → pitch →
/// `audio-event` pipeline so the UI's pitch display comes alive.
///
/// Mic failure is non-fatal by design: the session still starts (so
/// the user can practise "silently" and still get an end-of-session
/// recap) and the failure is logged via `tracing`. We consciously
/// reject the alternative of failing the whole session start —
/// "lost audio pipeline" shouldn't look like "lost session" to the
/// user mid-practice.
#[tauri::command]
pub async fn start_practice_session<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    instrument: String,
    practice_mode: PracticeMode,
    coaching_enabled: bool,
    score_id: Option<String>,
) -> Result<String, String> {
    emit_session_status(&app, SessionPhase::Starting);
    let detector_profile = state.detector_profile_for(&instrument);
    // Build the score follower (if a score was chosen) before committing
    // the session, so a missing/bad score degrades to "no cursor" rather
    // than failing the start. `None` when no score, or when the lookup or
    // MusicXML parse failed — `build_follower` logs the reason.
    let follower = score_id.as_deref().and_then(|id| state.build_follower(id));
    // Look up the score's title (cheap metadata read) so the recap can name
    // the piece. Independent of follower success: a score that parsed for
    // metadata but failed to build a follower still names itself in the recap.
    let score_title = score_id.as_deref().and_then(|id| state.score_title_for(id));
    match start_practice_session_impl(
        state.inner(),
        instrument.clone(),
        practice_mode,
        coaching_enabled,
        score_title,
    )
    .await
    {
        Ok(id) => {
            // Spin up the pipeline only after state-machine commit —
            // if we can't open the mic we at least want the recorder
            // in a consistent state.
            if let Some(profile) = detector_profile {
                let app_for_emit = app.clone();
                let app_for_phrase = app.clone();
                let app_for_position = app.clone();
                match AudioPipeline::start_with_follower(
                    profile,
                    follower,
                    move |event| {
                        let _ = app_for_emit.emit("audio-event", event);
                    },
                    move |phrase| {
                        emit_phrase_detected(&app_for_phrase, phrase);
                    },
                    move |position| {
                        emit_score_position_updated(&app_for_position, position);
                    },
                ) {
                    Ok(pipeline) => {
                        // Install only if the session is still live —
                        // a concurrent `end_practice_session` could
                        // have fired while `AudioPipeline::start`
                        // blocked on mic init, and we must not leave a
                        // hot mic open on an idle `AppState`. If the
                        // session is gone, the pipeline is dropped
                        // here, which joins the worker and releases
                        // the mic.
                        if let Err(pipeline) = state.install_audio_pipeline(pipeline).await {
                            tracing::info!(
                                instrument = %instrument,
                                "session ended before mic startup completed; stopping pipeline"
                            );
                            pipeline.stop();
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            instrument = %instrument,
                            "audio pipeline failed to start; session continues without live pitch"
                        );
                    }
                }
            }
            emit_session_status(&app, SessionPhase::Listening);
            Ok(id)
        }
        Err(e) => Err(e.to_frontend()),
    }
}

/// Close the current instrument segment and open a new one. Emits
/// `segment-changed` and hot-swaps the pitch detector's frequency
/// window on the running audio pipeline so the UI stops filtering out
/// notes that are in-range for the *new* instrument but were out-of-range
/// for the old one.
///
/// Pipeline reconfigure failures are logged but non-fatal — the segment
/// switch itself still succeeds. If the pipeline isn't running (mic
/// failed to open at session start) the reconfigure is a silent no-op.
#[tauri::command]
pub async fn switch_instrument<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    instrument: String,
    practice_mode: PracticeMode,
) -> Result<String, String> {
    // Look up the new profile before touching the state machine so a
    // successful `_impl` result and a reconfigurable pipeline move in
    // lockstep; validation of the instrument name itself is the
    // recorder's job.
    let new_profile = state.detector_profile_for(&instrument);
    match switch_instrument_impl(state.inner(), instrument.clone(), practice_mode).await {
        Ok((segment_id, started_at)) => {
            if let Some(profile) = new_profile {
                if let Err(e) = state.reconfigure_audio_pipeline(profile).await {
                    tracing::warn!(
                        error = %e,
                        instrument = %instrument,
                        "audio pipeline reconfigure failed; segment switch proceeds"
                    );
                }
            }
            emit_segment_changed(
                &app,
                SegmentChangedPayload {
                    segment_id: segment_id.clone(),
                    instrument,
                    started_at,
                },
            );
            Ok(segment_id)
        }
        Err(e) => Err(e.to_frontend()),
    }
}

/// Finalise the active session and return its recap. Always leaves
/// the backend in `Idle`, even on failure.
///
/// Audio pipeline is torn down *first* so the mic is released
/// immediately when the user clicks "End" — the recap generation that
/// follows may take a beat (LLM round-trip), and we'd rather not hold
/// the input device for that whole time.
#[tauri::command]
pub async fn end_practice_session<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<SessionRecap, String> {
    emit_session_status(&app, SessionPhase::Ending);
    state.stop_audio_pipeline().await;
    end_practice_session_impl(state.inner())
        .await
        .map_err(|e| e.to_frontend())
}

/// Request a real-time coaching tip for a phrase. Called between phrases
/// during a practice session. Returns None if coaching is disabled or
/// rate-limited. Always succeeds operationally; graceful degradation for
/// API failures.
#[tauri::command]
pub async fn get_coaching_tip(
    state: State<'_, AppState>,
    phrase: PhraseSummary,
    session_duration_secs: f64,
    phrases_played: usize,
) -> Result<Option<CoachingTip>, String> {
    let session_ctx = SessionContext {
        instrument: state.active_session_instrument().await.unwrap_or_default(),
        session_duration_secs,
        phrases_played,
        previous_tips: Vec::new(),
        score_title: state.active_session_score_title().await,
    };
    state
        .get_coaching_tip(&phrase, &session_ctx)
        .await
        .map_err(|e| e.to_string())
}

/// Return the instrument catalog for the selector grid.
#[tauri::command]
pub fn list_instruments(state: State<'_, AppState>) -> Result<Vec<InstrumentInfo>, String> {
    Ok(list_instruments_impl(state.inner()))
}

/// Check app capabilities (coaching availability, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCapabilities {
    pub coaching_available: bool,
}

#[tauri::command]
pub fn get_app_capabilities(state: State<'_, AppState>) -> Result<AppCapabilities, String> {
    Ok(AppCapabilities {
        coaching_available: state.coaching_available(),
    })
}

/// Get filtered session history for the history page.
///
/// If `instrument_filter` is provided, only sessions with that instrument
/// are returned. If `start_date` and `end_date` are provided, only sessions
/// within that range are returned. Returns in reverse chronological order.
#[tauri::command]
pub fn get_session_history(
    state: State<'_, AppState>,
    instrument_filter: Option<String>,
    start_date: Option<DateTime<Utc>>,
    end_date: Option<DateTime<Utc>>,
) -> Result<Vec<SessionSummaryDto>, String> {
    get_session_history_impl(state.inner(), instrument_filter, start_date, end_date)
        .map_err(|e| e.to_frontend())
}

/// Get full details (recap) for a specific session.
#[tauri::command]
pub fn get_session_detail(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<StoredSessionDto, String> {
    get_session_detail_impl(state.inner(), session_id).map_err(|e| e.to_frontend())
}

/// Get practice statistics (total sessions, total time, trends, etc.).
#[tauri::command]
pub fn get_practice_stats(state: State<'_, AppState>) -> Result<PracticeStatsDto, String> {
    get_practice_stats_impl(state.inner()).map_err(|e| e.to_frontend())
}

// ---------------------------------------------------------------------------
// Score management commands (Story: Score Mode)
// ---------------------------------------------------------------------------

/// IPC representation of a score library entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreLibraryEntryDto {
    pub id: String,
    pub title: String,
    pub composer: Option<String>,
    pub source_filename: String,
    pub added_at: DateTime<Utc>,
    pub last_practiced_at: Option<DateTime<Utc>>,
    pub part_index: usize,
    pub duration_measures: usize,
}

impl From<ScoreLibraryEntry> for ScoreLibraryEntryDto {
    fn from(s: ScoreLibraryEntry) -> Self {
        Self {
            id: s.id.as_str().to_owned(),
            title: s.title,
            composer: s.composer,
            source_filename: s.source_filename,
            added_at: s.added_at,
            last_practiced_at: s.last_practiced_at,
            part_index: s.part_index,
            duration_measures: s.duration_measures,
        }
    }
}

/// IPC representation of a fully-loaded score (entry + MusicXML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedScoreDto {
    pub entry: ScoreLibraryEntryDto,
    pub music_xml: String,
}

/// Import a score from a file or raw MusicXML.
///
/// Persists the score to the library and returns its entry + ID.
/// `title`, `composer`, `music_xml`, `part_index`, and `duration_measures`
/// come from the frontend after parsing the file.
#[tauri::command]
pub fn import_score(
    state: State<'_, AppState>,
    title: String,
    composer: Option<String>,
    source_filename: String,
    music_xml: String,
    part_index: usize,
    duration_measures: usize,
) -> Result<ScoreLibraryEntryDto, String> {
    let score_store = state
        .score_store
        .lock()
        .map_err(|e| format!("lock error: {e}"))?;
    let entry = score_store
        .import(
            title,
            composer,
            source_filename,
            music_xml,
            part_index,
            duration_measures,
        )
        .map_err(|e| e.to_string())?;
    Ok(entry.into())
}

/// List all scores in the library.
#[tauri::command]
pub fn list_scores(state: State<'_, AppState>) -> Result<Vec<ScoreLibraryEntryDto>, String> {
    let score_store = state
        .score_store
        .lock()
        .map_err(|e| format!("lock error: {e}"))?;
    let entries = score_store.list().map_err(|e| e.to_string())?;
    Ok(entries.into_iter().map(|e| e.into()).collect())
}

/// Load a score by id (returns entry + MusicXML).
#[tauri::command]
pub fn get_score(state: State<'_, AppState>, id: String) -> Result<LoadedScoreDto, String> {
    // Turbofish (not a `uuid::Error` annotation) pins the parse target:
    // `uuid` isn't a direct dependency of this crate, so naming its error
    // type won't resolve, but the inferred error still impls `Display`.
    let score_id: ScoreId = id.parse::<ScoreId>().map_err(|e| e.to_string())?;
    let score_store = state
        .score_store
        .lock()
        .map_err(|e| format!("lock error: {e}"))?;
    let entry = score_store.get(score_id).map_err(|e| e.to_string())?;
    Ok(LoadedScoreDto {
        music_xml: entry.music_xml.clone(),
        entry: entry.into(),
    })
}

/// Delete a score from the library.
#[tauri::command]
pub fn delete_score(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // See `get_score`: turbofish pins the parse target without naming the
    // non-direct-dependency `uuid::Error` type.
    let score_id: ScoreId = id.parse::<ScoreId>().map_err(|e| e.to_string())?;
    let score_store = state
        .score_store
        .lock()
        .map_err(|e| format!("lock error: {e}"))?;
    score_store.delete(score_id).map_err(|e| e.to_string())?;
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use brain::coaching::SessionContext;
    use brain::phrase::{DynamicsStats, PhraseSummary, PitchStats};

    fn state() -> AppState {
        AppState::with_mocks()
    }

    fn sample_phrase() -> PhraseSummary {
        PhraseSummary {
            phrase_index: 0,
            start_time: 0.0,
            end_time: 2.0,
            duration_secs: 2.0,
            note_count: 6,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 430.0,
                max_hz: 450.0,
                range_cents: 80.0,
                pitches: vec![440.0; 6],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.5,
                min_amplitude: 0.3,
                max_amplitude: 0.8,
                dynamic_range: 0.5,
            },
            stability: 0.8,
            score_position: None,
        }
    }

    #[tokio::test]
    async fn start_session_then_end_session_produces_recap() {
        // Happy path with phrases exercises the rich-recap branch.
        let s = state();
        let session_id = start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        assert!(!session_id.is_empty(), "session id must be non-empty");
        assert_eq!(s.current_phase().await, SessionPhase::Listening);

        // Drop a phrase so the recorder isn't empty — PR 2 wires this
        // via the aggregator, PR 1 simulates directly.
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }

        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(recap.phrase_count, 1);
        assert_eq!(recap.instrument, "Trumpet");
        assert!(!recap.strengths.is_empty());
        assert!(!recap.areas_to_improve.is_empty());
        assert_eq!(s.current_phase().await, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn double_start_is_rejected_with_clear_error() {
        let s = state();
        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .unwrap();
        let err = start_practice_session_impl(
            &s,
            "Piano".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::AlreadyActive), "{err:?}");
        // Error message must be human-readable — frontend surfaces it
        // directly.
        assert!(err.to_frontend().contains("already active"));
    }

    #[tokio::test]
    async fn end_without_start_is_rejected() {
        let s = state();
        let err = end_practice_session_impl(&s).await.unwrap_err();
        assert!(matches!(err, CommandError::NotActive), "{err:?}");
        assert_eq!(s.current_phase().await, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn switch_without_start_is_rejected() {
        let s = state();
        let err = switch_instrument_impl(&s, "Piano".to_owned(), PracticeMode::Practice)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::NotActive), "{err:?}");
    }

    #[tokio::test]
    async fn switch_instrument_closes_old_segment_opens_new() {
        let s = state();
        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .unwrap();
        let (new_segment_id, _) =
            switch_instrument_impl(&s, "Piano".to_owned(), PracticeMode::Practice)
                .await
                .unwrap();
        assert!(!new_segment_id.is_empty());

        let guard = s.active_session.lock().await;
        let session = guard.as_ref().unwrap();
        // The currently-open segment must be the new one.
        assert_eq!(session.recorder.current_instrument(), Some("Piano"));
    }

    #[tokio::test]
    async fn end_session_with_zero_phrases_returns_empty_state_recap() {
        // Per design doc §8 q3: zero phrases = calm empty-state recap,
        // NOT an error.
        let s = state();
        start_practice_session_impl(&s, "Voice".to_owned(), PracticeMode::Practice, false, None)
            .await
            .unwrap();
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(recap.phrase_count, 0);
        assert!(recap.strengths.is_empty());
        assert!(recap.areas_to_improve.is_empty());
        assert!(recap
            .overall_assessment
            .to_lowercase()
            .contains("didn't get to play"));
        // Still invite the user back.
        assert!(!recap.next_session_suggestions.is_empty());
    }

    #[tokio::test]
    async fn state_machine_transitions_idle_starting_listening_ending_idle() {
        let s = state();
        assert_eq!(s.current_phase().await, SessionPhase::Idle);

        start_practice_session_impl(&s, "Trumpet".to_owned(), PracticeMode::Practice, true, None)
            .await
            .unwrap();
        // PR 1 collapses Starting → Listening synchronously (no
        // audio-stream wait yet); PR 2 will insert the real pause.
        assert_eq!(s.current_phase().await, SessionPhase::Listening);

        switch_instrument_impl(&s, "Piano".to_owned(), PracticeMode::Practice)
            .await
            .unwrap();
        assert_eq!(s.current_phase().await, SessionPhase::Listening);

        end_practice_session_impl(&s).await.unwrap();
        assert_eq!(s.current_phase().await, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn start_rejects_empty_or_unknown_instrument() {
        let s = state();
        let empty =
            start_practice_session_impl(&s, "  ".to_owned(), PracticeMode::Practice, false, None)
                .await
                .unwrap_err();
        assert!(matches!(empty, CommandError::EmptyInstrument));

        let unknown = start_practice_session_impl(
            &s,
            "Kazoo".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .unwrap_err();
        match unknown {
            CommandError::UnknownInstrument(name) => assert_eq!(name, "Kazoo"),
            other => panic!("expected UnknownInstrument, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn switch_rejects_unknown_instrument() {
        let s = state();
        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .unwrap();
        let err = switch_instrument_impl(&s, "Kazoo".to_owned(), PracticeMode::Practice)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::UnknownInstrument(_)), "{err:?}");
    }

    #[test]
    fn list_instruments_returns_expected_catalog() {
        let s = state();
        let list = list_instruments_impl(&s);
        assert_eq!(
            list.len(),
            s.instrument_count(),
            "list_instruments must return every cached entry"
        );
        let by_name: std::collections::HashMap<_, _> = list
            .iter()
            .map(|i| (i.name.as_str(), i.family.as_str()))
            .collect();
        assert_eq!(by_name.get("Trumpet"), Some(&"Brass"));
        assert_eq!(by_name.get("Piano"), Some(&"Keyboard"));
        assert_eq!(by_name.get("Voice"), Some(&"Voice"));

        // Every returned entry must have a real frequency range. Defends
        // against a future regression where the mock catalog drifts from
        // the IPC shape (zeroes would pass a naive check but break the UI
        // range labels).
        for info in &list {
            assert!(
                info.freq_max_hz > info.freq_min_hz,
                "catalog entry {:?} has invalid freq range [{}, {}]",
                info.name,
                info.freq_min_hz,
                info.freq_max_hz,
            );
        }
    }

    /// Directly exercises the production catalog loader. The regular
    /// tests use `with_mocks`, which short-circuits the disk read — this
    /// one asserts the real `profiles/*.json` → `InstrumentInfo` path
    /// stays working end-to-end so a profile-schema change can't silently
    /// break the desktop catalog.
    #[test]
    fn production_catalog_loads_from_workspace_profiles() {
        // CARGO_MANIFEST_DIR is apps/desktop/src-tauri — workspace root
        // is three hops up, matching the locate_profiles_dir() default.
        // `None` handle → workspace-walk resolution (the dev/test path).
        let loaded = load_instrument_catalog(None);
        assert!(
            !loaded.is_empty(),
            "load_instrument_catalog must return every profile in profiles/"
        );
        let names: Vec<&str> = loaded.iter().map(|i| i.name.as_str()).collect();
        for expected in ["Trumpet", "Violin", "Piano", "Voice"] {
            assert!(
                names.contains(&expected),
                "expected {expected} in loaded catalog, got {names:?}"
            );
        }
        // Emoji + frequency range come from profile JSON — assert at
        // least one non-empty emoji to prove the field wires through.
        assert!(
            loaded.iter().any(|i| !i.emoji.is_empty()),
            "at least one profile should ship with a non-empty emoji"
        );
    }

    #[tokio::test]
    async fn mock_coaching_rotates_through_tips() {
        let svc = MockCoachingService::new();
        let ctx = SessionContext {
            instrument: "Trumpet".to_owned(),
            session_duration_secs: 0.0,
            phrases_played: 0,
            previous_tips: Vec::new(),
            score_title: None,
        };
        let phrase_0 = PhraseSummary {
            phrase_index: 0,
            start_time: 0.0,
            end_time: 1.0,
            duration_secs: 1.0,
            note_count: 8,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 430.0,
                max_hz: 450.0,
                range_cents: 80.0,
                pitches: vec![440.0; 8],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.6,
                min_amplitude: 0.4,
                max_amplitude: 0.8,
                dynamic_range: 0.4,
            },
            stability: 0.85,
            score_position: None,
        };
        let phrase_1 = PhraseSummary {
            phrase_index: 1,
            ..phrase_0.clone()
        };
        let phrase_3 = PhraseSummary {
            phrase_index: 3,
            ..phrase_0.clone()
        };
        let first = svc.get_tip(&phrase_0, &ctx).await.unwrap();
        let second = svc.get_tip(&phrase_1, &ctx).await.unwrap();
        let wrap = svc.get_tip(&phrase_3, &ctx).await.unwrap();
        assert_ne!(first.text, second.text);
        // Rotation wraps at len boundary.
        assert_eq!(first.text, wrap.text);
    }

    // ---------------------------------------------------------------
    // Audio pipeline wiring
    //
    // We can't open a real mic under `cargo test` — CI has no input
    // device. So these cover the surface that *doesn't* require the
    // pipeline to actually be started: profile lookup + the no-op
    // reconfigure/stop paths when no pipeline is installed.
    // ---------------------------------------------------------------

    #[test]
    fn detector_profile_for_known_instrument_mirrors_catalog_range() {
        let s = state();
        let profile = s
            .detector_profile_for("Trumpet")
            .expect("Trumpet must resolve to a profile");
        // Mock catalog has Trumpet at 165–1047 Hz. Trumpet's floor is
        // already above DETECTOR_MIN_HZ, so the clamp is a no-op here
        // and the detector freq window round-trips the catalog exactly.
        assert_eq!(profile.freq_min_hz, 165.0);
        assert_eq!(profile.freq_max_hz, 1047.0);
        // Threshold is defaulted for now (per-instrument overrides are a
        // future-profile-schema concern) — regression-assert the default
        // so a silent change here is caught.
        assert!(
            (profile.threshold - 0.15).abs() < f64::EPSILON,
            "threshold default must stay 0.15 until per-profile overrides ship"
        );
    }

    #[test]
    fn detector_profile_clamps_low_floor_instruments_to_realtime_minimum() {
        // Piano's 28 Hz catalog floor would give the YIN detector a
        // ~71 ms window at 44.1 kHz — blowing the project's 25 ms
        // latency budget. The clamp pulls the *detector* floor up to
        // DETECTOR_MIN_HZ without narrowing the UI catalog range.
        let s = state();
        let profile = s
            .detector_profile_for("Piano")
            .expect("Piano must resolve to a profile");
        assert_eq!(
            profile.freq_min_hz, DETECTOR_MIN_HZ,
            "detector floor must clamp up to DETECTOR_MIN_HZ for low-range instruments"
        );
        // Upper bound stays authoritative — we want the detector to
        // cover the full top of Piano's range.
        assert_eq!(profile.freq_max_hz, 4186.0);
    }

    #[test]
    fn detector_profile_for_unknown_instrument_returns_none() {
        let s = state();
        assert!(s.detector_profile_for("Kazoo").is_none());
    }

    // Note on `install_audio_pipeline` coverage: the race-guard
    // (reject install when session was drained during mic init) can't
    // be exercised here without a real mic to build an `AudioPipeline`
    // from. The wrapper's integration path is exercised manually on
    // hardware; capture-level coverage lives in the `ears` crate.

    #[tokio::test]
    async fn reconfigure_audio_pipeline_without_running_pipeline_is_a_noop() {
        // Invariant: if the mic failed to open at session start, a later
        // instrument switch must not fail just because the pipeline is
        // absent — the segment-level state still moves forward.
        let s = state();
        let profile = s
            .detector_profile_for("Trumpet")
            .expect("Trumpet must resolve");
        s.reconfigure_audio_pipeline(profile)
            .await
            .expect("reconfigure with no pipeline should be a silent no-op");
    }

    #[tokio::test]
    async fn stop_audio_pipeline_without_running_pipeline_is_a_noop() {
        // Symmetrical invariant for end_practice_session: tearing down
        // the pipeline must never explode when there's nothing to tear
        // down.
        let s = state();
        s.stop_audio_pipeline().await;
        s.stop_audio_pipeline().await; // Double-stop, still fine.
    }
}
