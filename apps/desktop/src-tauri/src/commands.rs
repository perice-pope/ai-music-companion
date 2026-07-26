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
use brain::accompaniment::{
    accompaniment_control_channel, AccompanimentDriver, AccompanimentSynth,
    AccompanimentSynthSource,
};
use brain::coach::{
    advance_windowed, apply_explore_delta_windowed, build_first_windowed, finish_lesson,
    played_notes_from_pitch_track, score_drill, sequence_to_score_model, start_explore_windowed,
    ChipSpec, Drill, DrillScore, ExploreState, FoldWindow, LessonRecap, LessonSpec, VariationDelta,
};
use brain::coaching::{
    grounded_offline_recap, CoachingCategory, CoachingConfig, CoachingEngine, CoachingSeverity,
    CoachingTip, NetworkPolicy, ReqwestClient, SessionContext,
};
use brain::connections::{
    apply_enriched_why, reveal_on_phrase, MusicalContext, Reveal, DEFAULT_REVEAL_CADENCE,
};
use brain::follower::ScorePosition;

use crate::score_position_log::{PositionBreadcrumb, ScorePositionLog};
use brain::perception::PerceptionTracker;
use brain::phrase::PhraseSummary;
use brain::session::{
    CompletedSession, PracticeMode, RecapGenerator, RecapInput, ScoreId, SessionError,
    SessionRecap, SessionRecorder,
};
use brain::stats::PracticeStats;
use brain::store::{
    ExerciseFactRow, ScoreLibraryEntry, ScoreStore, SessionStore, SessionSummary, StoredSession,
    TasteProfile, LOCAL_TASTE_PROFILE_USER_ID,
};
use chrono::{DateTime, Utc};
use ears::profile::{InstrumentProfile, ProfileLoader};

use crate::audio_pipeline::{AudioPipeline, DetectorProfile, PipelineError, SharedIdiomBuffer};
use ears::output_engine::AudioOutput;
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
    /// Per-instrument voiced-confidence gate (see
    /// [`InstrumentProfile::voiced_confidence_threshold`]). Carried through so
    /// the phrase aggregator can count quiet singing as practice (#185).
    pub voiced_confidence_threshold: f64,
    /// #349 T2b: the instrument can sound simultaneous notes (struck or
    /// plucked attack — piano, guitar). Polyphonic lessons deal the chord
    /// drill as block chords judged by the T1 chord engine.
    #[serde(default)]
    pub polyphonic: bool,
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
    /// #449 T1 (§1b): played-clock seconds persisted at session close.
    /// `None` on rows that predate the columns — honest absence, so the
    /// History page can say "unknown" instead of flattering with zeros.
    pub played_secs: Option<f64>,
    /// #449 T1 (§1b): voiced events detected. `None` as above.
    pub note_count: Option<u64>,
    /// #449 T1 (§1b): 1 − played/wall, clamped. `None` as above.
    pub silence_ratio: Option<f64>,
}

impl From<SessionSummary> for SessionSummaryDto {
    fn from(s: SessionSummary) -> Self {
        Self {
            id: s.id.as_str().to_owned(),
            instrument: s.instrument,
            started_at: s.started_at,
            duration_secs: s.duration_secs,
            phrase_count: s.phrase_count,
            played_secs: s.played_secs,
            note_count: s.note_count,
            silence_ratio: s.silence_ratio,
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
    #[error("a lesson is already running — end it before starting a new one")]
    LessonActive,
    #[error(
        "I didn't catch that yet — give it a second after you finish playing, then grade again"
    )]
    DrillNotHeard,
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

    /// Mirror the persisted `coachingEnabled` preference onto the underlying
    /// engine's [`NetworkPolicy`] — the Rust-core airplane switch. Default is a
    /// no-op (mocks never touch the network). The real LLM service overrides it
    /// so that `Offline` makes an outbound tip request structurally impossible.
    async fn set_network_policy(&self, _policy: NetworkPolicy) {}

    /// Enrich a grounded reveal's `why` via the LLM when online (#253 S2);
    /// otherwise return it unchanged. Never alters `concept`/`connection`. The
    /// default impl is the identity — mocks and preview stay fully offline.
    async fn enrich_reveal(&self, reveal: Reveal) -> Reveal {
        reveal
    }
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

    /// Test-only constructor that injects a specific [`CoachingEngine`] (e.g.
    /// one backed by a panicking HTTP client). Lets the airplane-switch tests
    /// prove that an `Offline` policy prevents any outbound call at the command
    /// layer without needing a real network client or API key.
    #[cfg(test)]
    pub fn with_engine(engine: CoachingEngine) -> Self {
        Self {
            engine: Some(Arc::new(Mutex::new(engine))),
        }
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
                // `get_tip` returns `Result<Option<CoachingTip>>`: `Ok(None)`
                // when the engine is offline / rate-limited / the call failed
                // (silence beats a lie), `Err` only on construction-time issues.
                // Flatten both "no tip" cases to a single `None`.
                engine.get_tip(phrase, context).await.ok().flatten()
            }
            None => None,
        }
    }

    async fn set_network_policy(&self, policy: NetworkPolicy) {
        if let Some(engine_arc) = &self.engine {
            engine_arc.lock().await.set_network_policy(policy);
        }
    }

    async fn enrich_reveal(&self, reveal: Reveal) -> Reveal {
        let Some(engine_arc) = &self.engine else {
            return reveal;
        };
        // The engine's airplane switch makes this a no-op (no call) when the
        // coaching opt-in is off; a failed/blank rewrite keeps the curated line.
        let enriched = {
            let mut engine = engine_arc.lock().await;
            engine
                .enrich_reveal_why(&reveal.concept, &reveal.connection, &reveal.why)
                .await
        };
        apply_enriched_why(reveal, enriched)
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

/// Stable snake_case label for a [`CoachingSeverity`], matching the enum's
/// serde representation. Used when persisting a tip into the recorder, which
/// stores severity/category as plain strings.
fn coaching_severity_label(severity: CoachingSeverity) -> &'static str {
    match severity {
        CoachingSeverity::Encouragement => "encouragement",
        CoachingSeverity::Suggestion => "suggestion",
        CoachingSeverity::Focus => "focus",
    }
}

/// Stable snake_case label for a [`CoachingCategory`], matching the enum's
/// serde representation.
fn coaching_category_label(category: CoachingCategory) -> &'static str {
    match category {
        CoachingCategory::Tone => "tone",
        CoachingCategory::Intonation => "intonation",
        CoachingCategory::Rhythm => "rhythm",
        CoachingCategory::Dynamics => "dynamics",
        CoachingCategory::Expression => "expression",
        CoachingCategory::Technique => "technique",
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

    /// Test-only constructor that injects a specific [`CoachingEngine`]. See
    /// [`LlmCoachingService::with_engine`].
    #[cfg(test)]
    pub fn with_engine(engine: CoachingEngine) -> Self {
        Self {
            engine: Some(Arc::new(Mutex::new(engine))),
        }
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
            // No API key: build the recap from the fingerprint the app already
            // computes — grounded, deterministic, and fully offline (no network
            // call). This is the path users on the free/offline tier see, so it
            // must reflect *their* session rather than canned text.
            return Ok(grounded_offline_recap(input));
        };

        // Call the LLM coaching engine for the full recap.
        let engine = engine_arc.lock().await;
        engine.generate_recap(input).await
    }

    async fn apply_network_policy(&self, policy: NetworkPolicy) {
        if let Some(engine_arc) = &self.engine {
            engine_arc.lock().await.set_network_policy(policy);
        }
    }

    async fn recap_used_llm(&self) -> bool {
        // #449 T1: delegate to the engine's own flag — no engine (no API key)
        // means the grounded offline recap ran, which is never a narration.
        match &self.engine {
            Some(engine_arc) => engine_arc.lock().await.recap_used_llm(),
            None => false,
        }
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
            score_summary: input
                .score_title
                .as_deref()
                .and_then(|t| brain::coaching::score_practice_summary(t, &input.note_verdicts)),
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
    /// Score id this session was started against, retained so the persisted
    /// session row can record which piece was practised. `None` in free play.
    score_id: Option<String>,
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
            score_id: None,
            coaching,
        }
    }
}

/// #449 T1: the telemetry journal's view of the active session — identity
/// plus the one clock every `practice_events.at_secs` offsets from
/// (`SessionRecorder::started_at`, the same base `end_practice_session_impl`
/// uses for `elapsed_secs`; the audio worker's phrase clock starts at
/// pipeline spin-up a few ms later, so range joins against
/// `session_phrases` are sound).
///
/// Held in its own `std::sync::Mutex` (not the async `active_session` lock)
/// so *sync* commands (`set_pocket_tempo`) can journal without an await, and
/// so the writer never contends with the session lock. Lock order where both
/// std mutexes are needed: telemetry → session_store, never the reverse.
struct SessionTelemetry {
    session_id: brain::session::SessionId,
    started_at: DateTime<Utc>,
    /// Last effective (clamped) click tempo pushed — tracked on EVERY push
    /// so `pocket_stop` reports the true final tempo even though the
    /// journal itself is coalesced.
    pocket_bpm: Option<f64>,
    /// Tempo-coalescing state: the last *journaled* tempo and when
    /// (session-clock seconds). See [`tempo_log_due`].
    tempo_last_logged_bpm: Option<f64>,
    tempo_last_logged_at_secs: f64,
}

impl SessionTelemetry {
    fn new(session_id: brain::session::SessionId, started_at: DateTime<Utc>) -> Self {
        Self {
            session_id,
            started_at,
            pocket_bpm: None,
            tempo_last_logged_bpm: None,
            tempo_last_logged_at_secs: 0.0,
        }
    }

    /// Seconds since session start on the one journal clock.
    fn at_secs(&self) -> f64 {
        (Utc::now() - self.started_at).num_milliseconds() as f64 / 1000.0
    }
}

/// #449 T1 coalescing gates for `pocket_tempo` rows: a change must be both
/// big enough and settled long enough to be worth a row. A follow-mode
/// stream (≈1 Hz, ±2 BPM wobble around a locked pulse) journals nothing;
/// a genuine ramp journals at most one row per gap window.
const POCKET_TEMPO_LOG_MIN_DELTA_BPM: f64 = 5.0;
const POCKET_TEMPO_LOG_MIN_GAP_SECS: f64 = 5.0;

/// The pure coalescing decision (unit-tested in isolation): journal a
/// `pocket_tempo` row iff the effective tempo moved
/// ≥ [`POCKET_TEMPO_LOG_MIN_DELTA_BPM`] from the last *journaled* value AND
/// ≥ [`POCKET_TEMPO_LOG_MIN_GAP_SECS`] have passed since that row. With no
/// prior row this session (`last_bpm == None`) the first push journals — the
/// only way a click started before this session's baseline existed (not
/// reachable today, but the safe default).
fn tempo_log_due(last_bpm: Option<f64>, last_at_secs: f64, bpm: f64, at_secs: f64) -> bool {
    match last_bpm {
        None => true,
        Some(prev) => {
            (bpm - prev).abs() >= POCKET_TEMPO_LOG_MIN_DELTA_BPM
                && (at_secs - last_at_secs) >= POCKET_TEMPO_LOG_MIN_GAP_SECS
        }
    }
}

/// Lock a `std::sync::Mutex`, recovering the data if a previous holder
/// panicked instead of propagating the poison.
///
/// Every std mutex in [`AppState`] guards state that stays valid at any
/// commit point a panic could interrupt (a store handle, an `Option` of
/// session-scoped state, a buffer of complete phrases) — so the
/// last-written value is always safe to keep serving. Propagating the
/// poison instead would turn one panic under a lock into a permanent
/// crash of every later history/taste/lesson command (#246).
trait LockRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for std::sync::Mutex<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Global backend state held via `tauri::Manager`.
///
/// `Mutex<Option<ActiveSession>>` guarantees at most one session exists
/// at a time — the double-start test verifies this.
pub struct AppState {
    active_session: Mutex<Option<ActiveSession>>,
    coaching_service: Arc<dyn CoachingService>,
    recap_generator: Arc<dyn RecapGenerator>,
    /// `std::sync::Mutex` because rusqlite's `Connection` wraps a
    /// `RefCell` internally, making `SessionStore: !Sync`. Tauri's
    /// managed `State<T>` requires `T: Sync`, so we interpose a mutex.
    /// Critical sections are short (single SQL query) and we never hold
    /// this lock across an `.await`.
    session_store: std::sync::Mutex<SessionStore>,
    score_store: std::sync::Mutex<ScoreStore>,
    /// #214 S1b: the in-memory library-match index (rebuilt at startup,
    /// maintained by import/delete). Keys are hashes of the ScoreId's
    /// string form; `titles` maps them back to (id string, title).
    piece_matcher: std::sync::Mutex<PieceMatcher>,
    coaching_available: bool,
    /// On-disk persistence fell back to in-memory at startup (e.g. corrupt data
    /// dir, sandbox, or full disk): the practice loop still works, but this
    /// session's history and scores won't survive a restart. Surfaced so the UI
    /// can warn calmly — startup never crashes over it (#137).
    persistence_degraded: bool,
    /// PDF→sheet-music import (on-device OMR) is an experimental beta, **off by
    /// default**. Enabled when `AMC_ENABLE_PDF_OMR` is set in the environment.
    /// Gated here so a normal build never advertises an unverified read path;
    /// the founder flips it on to exercise it. See
    /// `docs/architecture/score-import-and-transcription.md`.
    omr_enabled: bool,
    /// Instrument catalog loaded once at construction, shared by the
    /// `list_instruments` command and by session-validation paths.
    /// Held behind Arc so clones into IPC responses are cheap.
    instruments: Arc<Vec<InstrumentInfo>>,
    /// Why the catalog is empty, when it is (missing/unreadable `profiles/`
    /// dir — e.g. a packaged build whose bundled resources are absent, the
    /// scenario behind #112). Startup no longer panics over it (#364);
    /// `list_instruments` returns this as its error so the selector screen
    /// shows the reason instead of an unexplained empty grid.
    catalog_error: Option<String>,
    /// Live mic → pitch-detector → `audio-event` pipeline. `Some` only
    /// between `start_practice_session` and `end_practice_session`;
    /// swapped in place by `switch_instrument`. Held in a separate
    /// mutex from `active_session` because the pipeline only needs the
    /// instrument profile (not the recorder), and we prefer to hand the
    /// profile in from the command wrapper without briefly holding two
    /// locks at once.
    audio_pipeline: Mutex<Option<AudioPipeline>>,
    /// Session-scoped, downsampled mono audio retained for **offline**
    /// end-of-session idiom analysis. Written by the audio-pipeline worker
    /// thread (never the realtime callback), read once when building the recap,
    /// and cleared at the start of each session. Shared (`Arc` inside) so the
    /// pipeline gets its own handle without holding the `AppState`.
    idiom_buffer: SharedIdiomBuffer,
    /// Phrases the audio worker detected this session. The worker emits each
    /// completed phrase to the UI **and** pushes a copy here (off the realtime
    /// callback); `end_practice_session_impl` drains it into the recorder after
    /// the worker has stopped and flushed. Without this wire the recorder stayed
    /// empty and **every** recap was the "you didn't play" empty state — the
    /// real root cause behind #185 (the confidence gate was only half of it).
    phrase_buffer: Arc<std::sync::Mutex<Vec<PhraseSummary>>>,
    /// Note verdicts the follower produced this session (#337 S4) —
    /// buffered exactly like `phrase_buffer` and drained into the recap
    /// input at session end for the score-practice summary.
    verdict_buffer: Arc<std::sync::Mutex<Vec<brain::follower::NoteVerdict>>>,
    /// Stable chord readings the perception tracker promoted this session
    /// (#349 T2b) — the edge-triggered recorder ([`ChordChangeBuffer`]:
    /// one entry per chord CHANGE, slash corrections refreshed in place,
    /// release-reset for re-strikes). Chord drills grade from the slice
    /// heard since the drill began, exactly like `phrase_buffer` for
    /// melodic drills.
    ///
    /// [`ChordChangeBuffer`]: brain::chord_judge::ChordChangeBuffer
    chord_buffer: Arc<std::sync::Mutex<brain::chord_judge::ChordChangeBuffer>>,
    /// The jam chord chart (#349 T4a): timed label sequence recorded from
    /// perception snapshots — the lane's source of truth and the recap's
    /// chord sketch. Labels + timestamps only; no audio retained.
    chord_chart: Arc<std::sync::Mutex<brain::chord_chart::ChartRecorder>>,
    /// Follow-me accompaniment ("Play with me"). `Some` while the band is
    /// playing. Held behind a `std::sync::Mutex` (not tokio) because the audio
    /// worker thread's emit closures feed its driver synchronously on every
    /// event/phrase; the start/stop commands also touch it but never across an
    /// `.await`. Shared (`Arc`) so the worker closures get their own handle
    /// without holding the whole `AppState`. The render-thread synth is driven
    /// entirely off-device via the lock-free control channel inside the driver —
    /// no network, no realtime-callback locking.
    accompaniment: Arc<std::sync::Mutex<Option<Accompaniment>>>,
    /// #421 S1/S2: The Pocket — the click output plus (S2) the live
    /// tempo producer Follow/Handoff feed. Same one-owner audio-device
    /// discipline as the band, serialized by the same cmd lock.
    pocket: Arc<std::sync::Mutex<Option<Pocket>>>,
    /// #445: the click gate — the Pocket's click-fire consumer + epoch,
    /// shared with the audio worker so it can ignore mic onsets that merely
    /// agree with the app's own click. Installed by `start_pocket`, cleared
    /// by `teardown_pocket`; `None` = nothing gated (fail-open).
    click_gate: crate::audio_pipeline::SharedClickGate,
    /// Serializes the `start`/`stop` accompaniment commands (and session-end
    /// teardown) so two overlapping Tauri command tasks can't race the audio
    /// device handoff — without it, an interleaved start could drop-join an
    /// `AudioOutput` while the per-frame worker holds the accompaniment lock.
    accompaniment_cmd_lock: Mutex<()>,
    /// The user's key override for the band, if any. Persists across band
    /// The user's pinned key for the band as `(tonic, minor)`, if any. Persists
    /// across band start/stop within a session (applied when a band starts) and
    /// is reset on session end. `None` = follow the auto-detected key. "Lock" and
    /// "use the alternative" are both just a concrete pinned key, so the UI can
    /// always show exactly what the band is playing.
    key_override: std::sync::Mutex<Option<(u8, bool)>>,
    /// The in-flight guided lesson (#254), if any. `std::sync::Mutex` — every
    /// access is a short synchronous read-modify-write, never held across an
    /// `.await`.
    active_lesson: std::sync::Mutex<Option<ActiveLesson>>,
    /// The in-flight free-play exploration (#255), if any. Same locking rules
    /// as `active_lesson`.
    active_explore: Arc<std::sync::Mutex<Option<ExploreState>>>,
    /// #449 T1: the telemetry journal's session context — `Some` exactly
    /// while a practice session is active (set on successful start, cleared
    /// on every end path). No context → no rows, calmly: the writer's
    /// no-session contract lives here.
    telemetry: std::sync::Mutex<Option<SessionTelemetry>>,
}

/// A running follow-me accompaniment.
struct Accompaniment {
    /// The audio output engine playing the synth. Kept alive here; tearing it
    /// down ([`AppState::teardown_accompaniment`]) stops playback and releases
    /// the output device.
    output: AudioOutput,
    /// Processing-thread driver fed by the audio worker: turns onset timing into
    /// live clock updates and phrase keys into key changes, pushing both to the
    /// render-thread synth over the lock-free control channel.
    driver: AccompanimentDriver,
}

/// Control-channel depth (messages). Comfortably more than the render thread can
/// fall behind between drains; excess is dropped (the next tick is fresher).
const ACCOMPANIMENT_CHANNEL_CAPACITY: usize = 64;

/// #445: click-fire channel depth. Clicks arrive < 4/s (220 BPM ceiling)
/// and the audio worker drains every ~23 ms window; when no session is
/// running nothing drains and the ring simply drops new fires — fine,
/// there is no mic to protect then.
const CLICK_FIRE_CHANNEL_CAPACITY: usize = 64;

/// Minimum spacing between `perception` event emits (~8 Hz). Smooth enough for a
/// live readout without flooding IPC; driven off event timestamps, not wall clock.
const PERCEPTION_EMIT_INTERVAL_SECS: f64 = 0.125;

/// Open the on-disk session + score stores, degrading to in-memory if the
/// on-disk database can't be opened (corrupt data dir, sandbox, full disk).
///
/// Returns the two stores plus whether **on-disk** persistence is active. A
/// `false` means this session's history and scores won't survive a restart — we
/// log that plainly and carry on, because the practice loop needs no
/// persistence. This replaces the back-to-back `.expect` that turned a bad data
/// directory into a hard startup crash (#137).
fn open_stores(db_path: &std::path::Path) -> (SessionStore, ScoreStore, bool) {
    match (SessionStore::open(db_path), ScoreStore::open(db_path)) {
        (Ok(session), Ok(score)) => (session, score, true),
        (session_res, score_res) => {
            let reason = session_res
                .err()
                .map(|e| e.to_string())
                .or_else(|| score_res.err().map(|e| e.to_string()))
                .unwrap_or_default();
            tracing::warn!(
                path = %db_path.display(),
                error = %reason,
                "could not open the on-disk store; practice still works but this session's \
                 history and scores won't be saved"
            );
            // In-memory SQLite touches no filesystem, so a failure here would
            // mean rusqlite itself is unusable (a broken build/link), which is
            // not a recoverable runtime condition — hence the single, documented
            // expect rather than the previous back-to-back pair.
            let session = SessionStore::in_memory()
                .expect("in-memory SQLite must open — a failure means rusqlite is unusable");
            let score = ScoreStore::in_memory()
                .expect("in-memory SQLite must open — a failure means rusqlite is unusable");
            (session, score, false)
        }
    }
}

impl AppState {
    /// Production constructor — opens the SessionStore and ScoreStore at the
    /// platform default location, wires the real coaching engine, and loads
    /// the instrument catalog from `profiles/*.json`.
    ///
    /// Resolves profiles via env override → workspace walk (see
    /// [`locate_profiles_dir`]). For a *packaged* build, prefer
    /// [`AppState::new_with_app_handle`], which also checks the bundled
    /// resource directory — a bare `new()` in an installed app finds no
    /// profiles and degrades to the explained-empty selector (#112, #364).
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

        // Open the on-disk stores, degrading to in-memory (with a clear warning)
        // rather than crashing if the data directory is unusable (#137).
        let (session_store, score_store, persisted) = open_stores(&db_path);

        let coaching_svc = LlmCoachingService::new();
        let coaching_available = coaching_svc.coaching_available();
        let recap_gen = LlmRecapGenerator::new();

        // A missing catalog degrades to an explained-empty selector rather
        // than a startup crash (#364) — same posture as `open_stores` above.
        let (instruments, catalog_error) = match load_instrument_catalog(app_handle) {
            Ok(list) => (list, None),
            Err(e) => {
                tracing::warn!(error = %e, "instrument catalog unavailable; selector will explain");
                (Vec::new(), Some(e))
            }
        };

        let state = Self {
            active_session: Mutex::new(None),
            coaching_service: Arc::new(coaching_svc),
            recap_generator: Arc::new(recap_gen),
            session_store: std::sync::Mutex::new(session_store),
            score_store: std::sync::Mutex::new(score_store),
            piece_matcher: std::sync::Mutex::new(PieceMatcher::default()),
            coaching_available,
            persistence_degraded: !persisted,
            omr_enabled: pdf_omr_enabled_from_env(),
            instruments: Arc::new(instruments),
            catalog_error,
            audio_pipeline: Mutex::new(None),
            idiom_buffer: SharedIdiomBuffer::new(),
            phrase_buffer: Arc::new(std::sync::Mutex::new(Vec::new())),
            verdict_buffer: Arc::new(std::sync::Mutex::new(Vec::new())),
            chord_buffer: Arc::new(std::sync::Mutex::new(
                brain::chord_judge::ChordChangeBuffer::new(),
            )),
            chord_chart: Arc::new(std::sync::Mutex::new(
                brain::chord_chart::ChartRecorder::new(),
            )),
            accompaniment: Arc::new(std::sync::Mutex::new(None)),
            pocket: Arc::new(std::sync::Mutex::new(None)),
            click_gate: crate::audio_pipeline::SharedClickGate::default(),
            accompaniment_cmd_lock: Mutex::new(()),
            key_override: std::sync::Mutex::new(None),
            active_lesson: std::sync::Mutex::new(None),
            active_explore: Arc::new(std::sync::Mutex::new(None)),
            telemetry: std::sync::Mutex::new(None),
        };
        state.indexed()
    }

    /// #214 S2: the shared constructor tail. EVERY `AppState` constructor
    /// must end here so the startup identification index can't silently
    /// fall out of one construction path (each entry parses in ms; a bad
    /// file is skipped).
    fn indexed(self) -> Self {
        self.rebuild_piece_index();
        self
    }

    /// Wire entirely with mocks and in-memory store. Used by tests.
    pub fn with_mocks() -> Self {
        Self::with_mocks_on(ScoreStore::in_memory().expect("in-memory store must succeed"))
    }

    /// Like `with_mocks`, but over a caller-provided score store — lets
    /// tests seed a library BEFORE construction and prove the shared
    /// constructor tail (`indexed`) makes it identifiable with no manual
    /// rebuild (#214 S2 pin).
    pub fn with_mocks_on(score_store: ScoreStore) -> Self {
        Self {
            active_session: Mutex::new(None),
            coaching_service: Arc::new(MockCoachingService::new()),
            recap_generator: Arc::new(MockRecapGenerator),
            session_store: std::sync::Mutex::new(
                SessionStore::in_memory().expect("in-memory store must succeed"),
            ),
            score_store: std::sync::Mutex::new(score_store),
            piece_matcher: std::sync::Mutex::new(PieceMatcher::default()),
            coaching_available: false,
            // In-memory by design here — that's the test default, not a
            // degradation.
            persistence_degraded: false,
            omr_enabled: false,
            instruments: Arc::new(test_instrument_catalog()),
            catalog_error: None,
            audio_pipeline: Mutex::new(None),
            idiom_buffer: SharedIdiomBuffer::new(),
            phrase_buffer: Arc::new(std::sync::Mutex::new(Vec::new())),
            verdict_buffer: Arc::new(std::sync::Mutex::new(Vec::new())),
            chord_buffer: Arc::new(std::sync::Mutex::new(
                brain::chord_judge::ChordChangeBuffer::new(),
            )),
            chord_chart: Arc::new(std::sync::Mutex::new(
                brain::chord_chart::ChartRecorder::new(),
            )),
            accompaniment: Arc::new(std::sync::Mutex::new(None)),
            pocket: Arc::new(std::sync::Mutex::new(None)),
            click_gate: crate::audio_pipeline::SharedClickGate::default(),
            accompaniment_cmd_lock: Mutex::new(()),
            key_override: std::sync::Mutex::new(None),
            active_lesson: std::sync::Mutex::new(None),
            active_explore: Arc::new(std::sync::Mutex::new(None)),
            telemetry: std::sync::Mutex::new(None),
        }
        .indexed()
    }

    /// Test-only: flip the PDF-OMR beta gate on without touching the
    /// environment, so `recognize_pdf` can be exercised deterministically.
    #[cfg(test)]
    fn with_omr_enabled(mut self) -> Self {
        self.omr_enabled = true;
        self
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
                // works across brass/strings/voice.
                threshold: 0.15,
                freq_min_hz: i.freq_min_hz.max(DETECTOR_MIN_HZ),
                freq_max_hz: i.freq_max_hz,
                // Per-instrument voiced gate from the profile — Voice is lower
                // so breathy singing still forms phrases (#185).
                voiced_confidence_threshold: i.voiced_confidence_threshold,
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

    /// Stop the follow-me accompaniment, if running: release the output device
    /// and join its threads. No-op when nothing is playing, and safe to call
    /// repeatedly (e.g. from both `stop_accompaniment` and session end). The
    /// handle is taken out from under the lock *before* joining so the lock isn't
    /// held across the thread join.
    /// Returns whether a band was actually running (#449 T1: the callers
    /// journal `band_stop` only for a real stop, so a no-op teardown can
    /// never fabricate an event).
    pub(crate) fn teardown_accompaniment(&self) -> bool {
        let taken = self.accompaniment.lock_or_recover().take();
        match taken {
            Some(accompaniment) => {
                accompaniment.output.stop();
                true
            }
            None => false,
        }
    }

    /// #421 S1: stop The Pocket's click if it is playing. Idempotent, like
    /// the band's teardown; both run at session end. Returns whether a click
    /// was actually running (#449 T1: same fabrication guard as the band's).
    pub(crate) fn teardown_pocket(&self) -> bool {
        // #445: clear the click gate FIRST so the audio worker stops
        // consulting a dying click's fires. Fail-open by construction:
        // an empty slot gates nothing.
        *self.click_gate.lock_or_recover() = None;
        let taken = self.pocket.lock_or_recover().take();
        match taken {
            Some(pocket) => {
                pocket.output.stop();
                true
            }
            None => false,
        }
    }

    /// Apply a key override to the live band (if any). Shared by the override
    /// commands and `start_accompaniment` (so a pre-set override takes effect
    /// when a new band starts).
    fn apply_key_override_to_live_band(&self, ov: Option<(u8, bool)>) {
        if let Some(band) = self.accompaniment.lock_or_recover().as_mut() {
            match ov {
                Some((tonic, minor)) => band.driver.set_key_override(tonic, minor),
                None => band.driver.clear_key_override(),
            }
        }
    }

    /// Pin the band to a specific key (the user correcting the auto-read, or
    /// "locking" the currently-shown key — both are a concrete key).
    pub(crate) fn set_key_override(&self, tonic: u8, minor: bool) {
        *self.key_override.lock_or_recover() = Some((tonic, minor));
        self.apply_key_override_to_live_band(Some((tonic, minor)));
    }

    /// Resume automatic key-following; also reset on session end.
    pub(crate) fn clear_key_override(&self) {
        *self.key_override.lock_or_recover() = None;
        self.apply_key_override_to_live_band(None);
    }

    /// The current key override, applied when a new band starts.
    fn current_key_override(&self) -> Option<(u8, bool)> {
        *self.key_override.lock_or_recover()
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
            let store = self.score_store.lock_or_recover();
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
    /// a bad id or an unknown score — both non-fatal: the session simply
    /// falls back to free-play recap framing.
    fn score_title_for(&self, score_id: &str) -> Option<String> {
        let id: ScoreId = score_id.parse().ok()?;
        let store = self.score_store.lock_or_recover();
        store.get(id).ok().map(|entry| entry.title)
    }

    /// #214 S1b: (re)index one library entry for identification. A score
    /// that fails to parse is skipped calmly — the library still works,
    /// it just can't be identified (startup must never break on one bad
    /// file).
    pub(crate) fn index_entry(&self, entry: &ScoreLibraryEntry) {
        let model = match brain::score::musicxml::parse_musicxml_str_part(
            &entry.music_xml,
            entry.part_index,
        ) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, title = %entry.title, "score not indexable; skipped");
                return;
            }
        };
        let id_str = entry.id.to_string();
        let key = piece_key(&id_str);
        let mut matcher = self.piece_matcher.lock_or_recover();
        matcher.index.index_score(key, &model);
        matcher.titles.insert(key, (id_str, entry.title.clone()));
    }

    /// #214 S1b: the one store-write + index seam the raw-import command
    /// delegates to (review MF7b — a hook the command can drop silently
    /// isn't a hook; this method is directly testable).
    pub(crate) fn import_raw_score(
        &self,
        title: String,
        composer: Option<String>,
        source_filename: String,
        music_xml: String,
        part_index: usize,
        duration_measures: usize,
    ) -> Result<ScoreLibraryEntry, String> {
        let entry = self
            .score_store
            .lock_or_recover()
            .import(
                title,
                composer,
                source_filename,
                music_xml,
                part_index,
                duration_measures,
            )
            .map_err(|e| e.to_string())?;
        self.index_entry(&entry);
        Ok(entry)
    }

    /// #214 S1b: delete + unindex as ONE seam (review MF7c).
    pub(crate) fn delete_score_by_id(&self, id: &str) -> Result<(), String> {
        let score_id: ScoreId = id.parse::<ScoreId>().map_err(|e| e.to_string())?;
        self.score_store
            .lock_or_recover()
            .delete(score_id)
            .map_err(|e| e.to_string())?;
        // A deleted score is silent immediately.
        self.unindex_score(id);
        Ok(())
    }

    /// #214 S1b: forget a deleted score.
    pub(crate) fn unindex_score(&self, id_str: &str) {
        let key = piece_key(id_str);
        let mut matcher = self.piece_matcher.lock_or_recover();
        matcher.index.remove_score(key);
        matcher.titles.remove(&key);
    }

    /// #214 S1b: build the identification index over the whole library —
    /// called once at startup.
    pub(crate) fn rebuild_piece_index(&self) {
        let entries = match self.score_store.lock_or_recover().list() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "library unlistable; identification idle");
                return;
            }
        };
        for entry in &entries {
            self.index_entry(entry);
        }
    }

    /// Import a MIDI file into the score library.
    ///
    /// Parses the raw MIDI bytes into a [`ScoreModel`], serialises it to
    /// canonical MusicXML (the single format the library and score follower
    /// speak — see `architecture-v2.md` §9), and persists it.
    ///
    /// Unlike [`import_score`], the metadata (title, composer, measure count)
    /// is derived *in the backend* from the parsed MIDI rather than passed in
    /// from the frontend: the backend has to parse the file to convert it
    /// anyway, so there is nothing for the frontend to compute. When the MIDI
    /// carries no `TrackName` (the parser yields `"Untitled"`), the title
    /// falls back to the file's name stem so the library entry is
    /// recognisable rather than a wall of "Untitled".
    fn import_midi(
        &self,
        source_filename: String,
        bytes: Vec<u8>,
    ) -> Result<ScoreLibraryEntry, String> {
        self.import_midi_track(source_filename, bytes, None)
    }

    /// Import one track of a multi-part MIDI file (#337 S1) — `track_index`
    /// comes from [`brain::score::midi::list_midi_parts`]; `None` reads every
    /// (non-percussion) track, the right call for single-part files.
    fn import_midi_track(
        &self,
        source_filename: String,
        bytes: Vec<u8>,
        track_index: Option<usize>,
    ) -> Result<ScoreLibraryEntry, String> {
        let model = brain::score::midi::parse_midi_bytes_track(&bytes, track_index)
            .map_err(|e| e.to_string())?;

        let title = if model.title == "Untitled" {
            filename_stem(&source_filename).unwrap_or_else(|| "Untitled".to_string())
        } else {
            model.title.clone()
        };
        let composer = model.composer.clone();
        let duration_measures = model.measures.len();
        let music_xml = brain::score::emit::score_model_to_musicxml(&model);

        let store = self.score_store.lock_or_recover();
        store
            .import(
                title,
                composer,
                source_filename,
                music_xml,
                0,
                duration_measures,
            )
            .map_err(|e| e.to_string())
            .inspect(|entry| self.index_entry(entry))
    }

    /// Import a MusicXML file into the score library.
    ///
    /// Parses the MusicXML to derive metadata (title, composer, measure count)
    /// *in the backend* — business logic stays in Rust (CLAUDE.md), so the
    /// frontend only ships raw bytes and a chosen part. `part_index` selects
    /// which `<part>` of a multi-instrument score to read and practice; the UI
    /// gets the choices from [`list_score_parts`]. The **original** MusicXML is
    /// stored unchanged so the score follower re-selects the same part by index
    /// at session start (see [`AppState::build_follower`]). Parsing the chosen
    /// part up front also means a malformed file fails *here*, at import, with a
    /// clear message — never silently later when the cursor won't move.
    ///
    /// Title falls back to the file's name stem when the score carries no
    /// `<work-title>` / `<movement-title>` (parser yields `"Untitled"`).
    fn import_musicxml(
        &self,
        source_filename: String,
        music_xml: String,
        part_index: usize,
    ) -> Result<ScoreLibraryEntry, String> {
        let model = brain::score::musicxml::parse_musicxml_str_part(&music_xml, part_index)
            .map_err(|e| e.to_string())?;

        let title = if model.title == "Untitled" {
            filename_stem(&source_filename).unwrap_or_else(|| "Untitled".to_string())
        } else {
            model.title.clone()
        };
        let composer = model.composer.clone();
        let duration_measures = model.measures.len();

        let store = self.score_store.lock_or_recover();
        store
            .import(
                title,
                composer,
                source_filename,
                music_xml,
                part_index,
                duration_measures,
            )
            .map_err(|e| e.to_string())
            .inspect(|entry| self.index_entry(entry))
    }

    /// Import an audio recording into the score library.
    ///
    /// Transcribes the recording to MIDI via basic-pitch (the [`transcribe`]
    /// crate), then reuses [`import_midi`](Self::import_midi) (parse → MusicXML
    /// → store) so the result behaves like any other library entry. Returns the
    /// new entry alongside a [`transcribe::TranscriptionQuality`] signal so the
    /// UI can warn calmly when the input looks polyphonic or weak. The title
    /// falls back to the file's name stem — transcribed MIDI carries no track
    /// name.
    fn import_audio(
        &self,
        source_filename: String,
        bytes: Vec<u8>,
        extension: Option<&str>,
    ) -> Result<(ScoreLibraryEntry, transcribe::TranscriptionQuality), String> {
        self.import_audio_with(source_filename, || {
            transcribe::transcribe_audio_bytes_with_quality(bytes, extension)
        })
    }

    /// Transcribe → import_midi, with `transcribe_fn` as a **seam** so the panic
    /// guard can be tested without a real ONNX failure: [`import_audio`] passes
    /// the real (native, panic-capable) transcription; a test can pass a
    /// panicking one and assert this returns a calm error instead of crashing
    /// the app (#267). The native call runs behind [`guard_transcription`], so a
    /// panic in it becomes a calm error instead of unwinding into the command.
    fn import_audio_with<F>(
        &self,
        source_filename: String,
        transcribe_fn: F,
    ) -> Result<(ScoreLibraryEntry, transcribe::TranscriptionQuality), String>
    where
        F: FnOnce() -> Result<
            (Vec<u8>, transcribe::TranscriptionQuality),
            transcribe::TranscribeError,
        >,
    {
        let (midi, quality) = guard_transcription(|| transcribe_fn().map_err(|e| e.to_string()))?;
        // The parser's no-notes refusal speaks MIDI ("drum, click, or marker
        // track") — for a recording the honest reason is that we couldn't
        // hear notes in the audio (review MUST-FIX 4).
        let entry = self
            .import_midi(source_filename, midi)
            .map_err(|e| {
                if e.contains("no playable notes") {
                    "we couldn't hear any notes in that recording — try a clearer,                      closer take"
                        .to_string()
                } else {
                    e
                }
            })?;
        Ok((entry, quality))
    }

    /// Recognize a sheet-music **PDF** into MusicXML using the given OMR
    /// `engine`, then read its parts — so the frontend can run the *same*
    /// "which part do you want to read?" picker as MusicXML import and feed the
    /// recognized MusicXML back through [`import_musicxml`](Self::import_musicxml).
    ///
    /// Nothing is stored here: OMR is purely a front-end that produces the
    /// canonical MusicXML, mirroring how audio transcription produces MIDI. The
    /// part-parsing lives in the backend (CLAUDE.md: no business logic in the
    /// frontend). Returns [`RecognizedScore`] with a calm "read from a scan"
    /// signal; the caller always surfaces it (OMR is approximate).
    ///
    /// Gated by [`omr_enabled`](AppState): when the beta flag is off this
    /// returns a plain, honest message rather than running an unverified path.
    fn recognize_pdf(
        &self,
        engine: &dyn omr::OmrEngine,
        bytes: &[u8],
    ) -> Result<RecognizedScore, String> {
        if !self.omr_enabled {
            return Err(
                "Reading sheet-music PDFs is an experimental feature that isn't \
                        enabled in this build yet."
                    .to_string(),
            );
        }
        let recognized = omr::pdf_to_musicxml(engine, bytes).map_err(|e| e.to_string())?;
        // Reuse the canonical MusicXML part reader — the exact seam MusicXML
        // import uses — so the picker UX and the stored part_index are shared.
        let parts =
            brain::score::musicxml::list_parts(&recognized.music_xml).map_err(|e| e.to_string())?;
        Ok(RecognizedScore {
            music_xml: recognized.music_xml,
            parts,
            low_content: recognized.quality.low_content,
        })
    }

    /// Clone the full instrument catalog for an IPC response, or the
    /// startup load failure when there is one (#364) — the selector
    /// screen renders the message where the grid would be.
    pub fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, String> {
        if let Some(reason) = &self.catalog_error {
            return Err(reason.clone());
        }
        // Structural guard, not just convention: an empty catalog must never
        // reach the UI as `Ok([])` — the selector renders that as a silent
        // empty grid, the exact no-feedback state the old panic existed to
        // prevent. Holds even if the degraded-load wiring in `build` regresses.
        if self.instruments.is_empty() {
            return Err("no instrument profiles are loaded. Reinstalling the app \
                 should restore them; set AI_MUSIC_COMPANION_PROFILES_DIR to override."
                .to_string());
        }
        Ok((*self.instruments).clone())
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

    /// Whether the experimental PDF→sheet-music (OMR) beta is enabled.
    pub fn omr_enabled(&self) -> bool {
        self.omr_enabled
    }

    /// Whether on-disk persistence degraded to in-memory at startup (#137).
    /// `true` means this session's history/scores won't survive a restart — the
    /// UI can surface a calm "not saving this session" note.
    pub fn persistence_degraded(&self) -> bool {
        self.persistence_degraded
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
        Ok(self.coaching_service.get_tip(phrase, context).await)
    }

    /// Enrich a grounded reveal's `why` via the coaching service (#253 S2). When
    /// the coaching opt-in is off, this returns the reveal unchanged with no
    /// network call.
    pub async fn enrich_reveal(&self, reveal: Reveal) -> Reveal {
        self.coaching_service.enrich_reveal(reveal).await
    }

    /// The texts of the most recent `limit` coaching tips recorded in the
    /// active session, oldest-first. Empty when there's no active session.
    /// Threaded into the live coach's `previous_tips` so it can avoid
    /// repeating recent advice.
    pub async fn recent_tip_texts(&self, limit: usize) -> Vec<String> {
        match &*self.active_session.lock().await {
            Some(s) => s.recorder.recent_tips(limit),
            None => Vec::new(),
        }
    }

    /// Persist a coaching tip into the active session's recorder so it lands in
    /// the session history and the end-of-session recap input. No-op (returns
    /// `NoSession`) when there's no active session.
    pub async fn record_coaching_tip(
        &self,
        phrase_index: usize,
        tip: &CoachingTip,
    ) -> Result<(), CommandError> {
        let mut guard = self.active_session.lock().await;
        let session = guard.as_mut().ok_or(CommandError::NotActive)?;
        session.recorder.record_tip(
            phrase_index,
            tip.text.clone(),
            coaching_severity_label(tip.severity).to_owned(),
            coaching_category_label(tip.category).to_owned(),
        )?;
        Ok(())
    }

    /// Mirror the user's persisted `coachingEnabled` preference onto the
    /// Rust-core airplane switch ([`NetworkPolicy`]) for **both** the live-tip
    /// engine and the recap engine.
    ///
    /// This is where the FE toggle becomes a hard, below-the-IPC guarantee:
    /// when `enabled == false`, both engines are set to [`NetworkPolicy::Offline`]
    /// and are then structurally incapable of an outbound call — the on-device
    /// fallback is used instead. Called at session start from the persisted
    /// preference; the engines default to `Offline` until then.
    pub async fn set_coaching_network_policy(&self, enabled: bool) {
        let policy = NetworkPolicy::from_opt_in(enabled);
        self.coaching_service.set_network_policy(policy).await;
        self.recap_generator.apply_network_policy(policy).await;
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
        voiced_confidence_threshold: profile.voiced_confidence_threshold,
        polyphonic: matches!(
            profile.attack_type,
            ears::profile::AttackType::Struck | ears::profile::AttackType::Plucked
        ),
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
/// Errors instead of panicking (#364): a packaged build with a missing or
/// empty resource dir must reach the UI so the selector can explain why
/// it's empty — a startup crash gives the user nothing to act on.
fn load_instrument_catalog(
    app_handle: Option<&tauri::AppHandle>,
) -> Result<Vec<InstrumentInfo>, String> {
    load_catalog_from(&locate_profiles_dir(app_handle))
}

/// Directory-explicit body of [`load_instrument_catalog`] — the seam the
/// error-path tests use, since the env-var override in `locate_profiles_dir`
/// is process-global and can't be varied safely across parallel tests.
fn load_catalog_from(dir: &std::path::Path) -> Result<Vec<InstrumentInfo>, String> {
    let profiles = ProfileLoader::load_all(dir).map_err(|e| {
        format!(
            "failed to load instrument profiles from {}: {}. \
             Set AI_MUSIC_COMPANION_PROFILES_DIR to override the location.",
            dir.display(),
            e
        )
    })?;
    if profiles.is_empty() {
        return Err(format!(
            "no instrument profiles found in {}. Check that the profiles/ \
             directory is populated; set AI_MUSIC_COMPANION_PROFILES_DIR to override.",
            dir.display()
        ));
    }
    Ok(profiles.iter().map(profile_to_info).collect())
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
        // Plucked → polyphonic like the real profiles (profile_to_info),
        // but Strings family: the case where polyphonic and grand-staff
        // genuinely diverge (review MF1).
        ("Guitar", "Strings", 82.0, 1319.0),
    ]
    .into_iter()
    .map(|(name, family, lo, hi)| InstrumentInfo {
        name: name.to_owned(),
        family: family.to_owned(),
        freq_min_hz: lo,
        freq_max_hz: hi,
        vibrato_tolerance_cents: 25.0,
        emoji: String::new(),
        // Voice uses a lower gate so quiet singing registers (#185); others
        // keep the 0.5 default.
        voiced_confidence_threshold: if name == "Voice" { 0.3 } else { 0.5 },
        polyphonic: family == "Keyboard" || name == "Guitar",
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Emit helpers
// ---------------------------------------------------------------------------

/// Emit an audio-import progress beat (`{ stage, pct }`). Non-fatal on error.
fn emit_import_progress<R: Runtime>(app: &tauri::AppHandle<R>, stage: &'static str, pct: u8) {
    let _ = app.emit("import-progress", ImportProgressPayload { stage, pct });
}

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

/// #370 (#341 review residual 2): while a tap-a-measure exploration is on
/// stage, the follower keeps aligning — the rowed cell is near-identical to
/// the score — so phrases closed mid-exploration carry score anchors for
/// music the player wasn't playing AT the score. Detach every score-anchored
/// field: the card never cites the exploration, the cursor never jumps to its
/// bogus measure (`phrase-detected` moves the cursor too), and the recap's
/// score summary never counts it. Everything else stays — the lift path
/// ("work on my last lick") lifts from `pitch_stats.pitches`, which must keep
/// hearing exploration playing.
fn scrub_score_anchors_if_exploring(mut phrase: PhraseSummary, exploring: bool) -> PhraseSummary {
    if exploring {
        phrase.score_position = None;
        phrase.score_span = None;
        phrase.verdicts = None;
        phrase.score_card = None;
    }
    phrase
}

/// The pipeline's phrase-close callback (#370): buffer for the recap, retune
/// the band, emit to the UI — with exploration phrases shedding their score
/// anchors first.
///
/// A phrase only closes when the NEXT voiced event arrives after a silence
/// gap, so the phrase overlaying an exploration typically closes AFTER
/// "Back to listening" clears the gate — a gate read at close time alone
/// leaks its bogus anchors (review MF1). Since the event that closes a
/// phrase is the first event of the next one, the callback latches the
/// gate's state at each close as "did the next phrase open mid-exploration"
/// and scrubs on either signal. The conservative half — the last honest
/// score phrase closing on the first ROWED note loses its anchors too — is
/// accepted: silence over lies.
fn make_phrase_closed_callback<R: Runtime>(
    app: tauri::AppHandle<R>,
    phrase_buffer: Arc<std::sync::Mutex<Vec<PhraseSummary>>>,
    accompaniment: Arc<std::sync::Mutex<Option<Accompaniment>>>,
    explore_gate: Arc<std::sync::Mutex<Option<ExploreState>>>,
) -> impl FnMut(PhraseSummary) + Send + 'static {
    // The first phrase opens with the session itself, and an exploration can
    // already be on stage (staged from a recap, practised in a fresh session).
    let mut opened_while_exploring = explore_gate.lock_or_recover().is_some();
    move |phrase| {
        let exploring_now = explore_gate.lock_or_recover().is_some();
        let phrase =
            scrub_score_anchors_if_exploring(phrase, exploring_now || opened_while_exploring);
        opened_while_exploring = exploring_now;
        // Buffer a copy for the recap (drained into the recorder at session
        // end), then emit to the UI for live display.
        phrase_buffer.lock_or_recover().push(phrase.clone());
        // Retune the band when this phrase carries a confident key.
        if let Some(key) = phrase.key.as_ref() {
            if let Some(accompaniment) = accompaniment.lock_or_recover().as_mut() {
                accompaniment.driver.observe_key(key);
            }
        }
        emit_phrase_detected(&app, phrase);
    }
}

/// Emit the follower's live score position (~10 Hz) so the cursor glides
/// between phrase boundaries. Only fires in score mode. Non-fatal on
/// error: a dropped tick just means one skipped frame of cursor motion.
///
/// `log` is the SESSION's breadcrumb state (#354). Its predecessor — a
/// process-wide `std::sync::Once` "first score-position emitted" line —
/// could never fire twice, so five VA runs of "no visual cursor" read as
/// "one emission then silence" whether or not emissions continued. The
/// per-session decisions (first / measure change / heartbeat / resume)
/// make the tester's log answer that directly.
fn emit_score_position_updated<R: Runtime>(
    app: &tauri::AppHandle<R>,
    log: &mut ScorePositionLog,
    position: ScorePosition,
) {
    for crumb in log.emitted(position.measure_number) {
        match crumb {
            PositionBreadcrumb::First { measure } => {
                tracing::info!(measure, ?position, "first score-position emitted");
            }
            PositionBreadcrumb::MeasureChanged { from, to, emitted } => {
                tracing::info!(from, to, emitted, "score-position measure changed");
            }
            PositionBreadcrumb::Heartbeat { emitted, measure } => {
                tracing::info!(emitted, measure, "score-position still emitting");
            }
            PositionBreadcrumb::Resumed { measure } => {
                tracing::info!(measure, "score-position emissions resumed");
            }
            // `emitted()` never yields this variant — suppression starts
            // are logged at the gate via `swallowed()`.
            PositionBreadcrumb::SuppressionStarted { emitted } => {
                tracing::info!(emitted, "score-position suppressed (exploration on stage)");
            }
        }
    }
    let _ = app.emit("score-position-updated", position);
}

/// Webview-side diagnostic breadcrumb (#354): lands frontend observations
/// (positions received, cursor shown, cursor DOM geometry) in the SAME
/// log file the tester already pulls, so one capture shows both sides of
/// the IPC boundary. Local IPC → local log only; nothing leaves the
/// device (offline-first: not a networked feature, nothing to disclose).
#[tauri::command]
pub fn frontend_breadcrumb(message: String) {
    tracing::info!(
        source = "webview",
        "{}",
        crate::score_position_log::clip_frontend_breadcrumb(&message)
    );
}

/// Payload for `accompaniment-status`. Reports whether the band is playing.
///
/// The richer "Band locked — G Mixolydian · 92 BPM" chip from the spec (live
/// tempo + key) is a tracked follow-up: it needs the worker to emit throttled
/// status updates carrying the current `ClockState` + key, a backend change
/// beyond the play/stop wiring. See the spec's AC10 deferral note.
#[derive(Clone, Serialize)]
struct AccompanimentStatusPayload {
    playing: bool,
}

/// Tell the UI whether the follow-me band is playing. Non-fatal on error.
/// #421 S2: the live Pocket — output device + the tempo channel's
/// control side.
pub(crate) struct Pocket {
    output: ears::output_engine::AudioOutput,
    tempo_tx: ringbuf::HeapProd<f64>,
}

#[derive(Clone, Serialize)]
struct PocketStatusPayload {
    playing: bool,
    tempo_bpm: f64,
}

/// #421 S1: the Pocket's status event — the frontend chip and pulse
/// follow this, exactly as the band follows accompaniment-status.
fn emit_pocket_status<R: Runtime>(app: &tauri::AppHandle<R>, playing: bool, tempo_bpm: f64) {
    let _ = app.emit("pocket-status", PocketStatusPayload { playing, tempo_bpm });
}

/// #214 S1b: the library-match state — S1a's engine plus the id/title map
/// the u64 index keys need (ScoreIds are UUIDs).
#[derive(Default)]
pub(crate) struct PieceMatcher {
    index: brain::piece_match::PieceIndex,
    titles: std::collections::HashMap<u64, (String, String)>,
}

fn piece_key(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

/// A gated library match, ready for the session chip.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PieceMatchDto {
    pub score_id: String,
    pub title: String,
    pub coherent_hits: usize,
}

fn emit_accompaniment_status<R: Runtime>(app: &tauri::AppHandle<R>, playing: bool) {
    let _ = app.emit(
        "accompaniment-status",
        AccompanimentStatusPayload { playing },
    );
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
    coaching_enabled: bool,
    score_title: Option<String>,
) -> Result<String, CommandError> {
    if instrument.trim().is_empty() {
        return Err(CommandError::EmptyInstrument);
    }
    if !state.is_known_instrument(&instrument) {
        return Err(CommandError::UnknownInstrument(instrument));
    }

    // Mirror the user's persisted opt-in onto the Rust-core airplane switch
    // before any phrase or recap can be generated this session. When coaching
    // is disabled, both engines go Offline and cannot make an outbound call —
    // the coach is served entirely by the on-device fallback (offline-first).
    state.set_coaching_network_policy(coaching_enabled).await;

    let mut guard = state.active_session.lock().await;
    if guard.is_some() {
        return Err(CommandError::AlreadyActive);
    }

    let coaching = Arc::clone(&state.coaching_service);
    let mut session = ActiveSession::new(instrument, practice_mode, coaching);
    // Tag the recorder with the score title (if score-backed) so the
    // end-of-session recap can name the piece and cite measures.
    session.recorder.set_score_title(score_title);
    let session_id = session.recorder.session_id().as_str();
    // #449 T1: open the telemetry journal for this session — its id plus the
    // recorder's `started_at`, the one clock every `at_secs` offsets from.
    // Set only on the success path (an AlreadyActive bounce above must never
    // clobber the live session's context).
    let telemetry =
        SessionTelemetry::new(session.recorder.session_id(), session.recorder.started_at());
    // Starting → Listening is synchronous in PR 1. PR 2 inserts a real
    // pause once audio capture startup is async.
    session.phase = SessionPhase::Listening;

    *guard = Some(session);
    *state.telemetry.lock_or_recover() = Some(telemetry);
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
    // #449 T1: the telemetry journal closes with the session — from here on,
    // tool commands are between-sessions and must journal nothing. Cleared
    // before any await so a slow recap can't leave a stale context journaling
    // events into an ended session. (The wrapper's band/click teardown runs
    // BEFORE this, so their stop events are already in; the recap narration
    // event below is written directly against the completed session.)
    *state.telemetry.lock_or_recover() = None;
    // A lesson can't outlive its session: the phrase buffer it grades from is
    // about to be drained, so a surviving lesson would silently mis-grade the
    // NEXT session (#254 review M1). Finalize it — completed drills keep their
    // mastery credit, the unfinished one is dropped.
    end_lesson_impl(
        state,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    // Exploration is session-scoped too (#255) — clear it with the session.
    *state.active_explore.lock_or_recover() = None;
    let mut session = taken.expect("session was Some under the lock we just took");
    let generator = Arc::clone(&state.recap_generator);

    // Record the phrases the audio worker detected this session into the
    // recorder *before* finalising it. The worker emits each phrase to the UI
    // for the live display and buffers a copy here; `end_practice_session` (our
    // wrapper) already stopped+joined the worker before calling us, so the
    // buffer now holds the complete set including the final flushed phrase.
    // Without this the recorder is empty and the recap is always the
    // "you didn't play" empty state regardless of how much was played (#185).
    let detected_phrases: Vec<PhraseSummary> =
        std::mem::take(&mut *state.phrase_buffer.lock_or_recover());
    for phrase in detected_phrases {
        if let Err(e) = session.recorder.record_phrase(phrase) {
            tracing::warn!(error = %e, "could not record a detected phrase into the session");
        }
    }

    // Run the offline idiom analysis once, at the session boundary, off the
    // realtime audio path. `analyze_idioms` is fully on-device (no network) and
    // confidence-gated — it returns an empty list when nothing clears the gate,
    // so quiet or non-idiomatic sessions surface nothing ("silence > lies").
    let (idiom_samples, idiom_rate) = state.idiom_buffer.snapshot();
    let idiom_notes = brain::idiom_recap::analyze_idioms(&idiom_samples, idiom_rate);
    // Don't retain a session's audio past its recap.
    state.idiom_buffer.clear();

    // Read the student's stated taste profile from the local store so the coach
    // can relate the measured musicianship to the music in their world. This is
    // the join point named in the personalization spine: the fingerprint
    // (facts) and the profile (preference) meet only here, at coaching time.
    // A missing or unreadable profile is cold start → `None`, and the coach
    // falls back to its existing genre-neutral behavior. Idiom notes (offline)
    // and connections (profile-driven) are complementary and both flow through.
    let taste_profile = state
        .session_store
        .lock_or_recover()
        .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)
        .ok()
        .flatten();

    // #453 S2: the history-grounded suggestions, read through the same
    // analyzer the `practice_suggestions` command exposes — evidence-cited
    // local facts the recap may weave in (offline: at most one appended;
    // LLM: prompt grounding). Store trouble → empty list, never an error.
    let history_suggestions = practice_suggestions_core(state);

    // Capture the real session length and instrument before `complete()`
    // consumes the recorder — the empty-state path needs them so a session with
    // genuine elapsed time never reads as "you didn't play" (#185).
    let elapsed_secs =
        (Utc::now() - session.recorder.started_at()).num_milliseconds() as f64 / 1000.0;
    let instrument = session
        .recorder
        .current_instrument()
        .unwrap_or_default()
        .to_owned();
    // Session-level debug metadata, captured before `complete()` consumes the
    // recorder, so the persisted row records how/what was practised (#201).
    let practice_mode_label = format!("{:?}", session.practice_mode);
    let session_score_id = session.score_id.take();

    // The session's note verdicts (#337 S4) — drained like the phrases so
    // the recap can rank the worst measures honestly.
    let note_verdicts: Vec<brain::follower::NoteVerdict> =
        std::mem::take(&mut *state.verdict_buffer.lock_or_recover());

    match session.recorder.complete() {
        Ok(completed) => {
            let family = instrument_family_for(state, completed.primary_instrument());
            // #454 S3: the method-book tip THIS session's measured evidence
            // earned — resolved on the live path from the same phrase set the
            // recap generators read (same `build_fingerprint`, same evidence
            // gates), never a store read-back (which would race the save and
            // speak to the PREVIOUS session). `None` is the calm, common
            // answer: no crossed bar, no catalog family, no matching entry.
            let method_book_tip = brain::pedagogy::select_pedagogy(
                &family,
                &brain::coaching::build_fingerprint(&completed.all_phrases()),
            );
            let recap = build_recap(
                &completed,
                &*generator,
                taste_profile,
                idiom_notes,
                note_verdicts,
                family,
                history_suggestions,
                method_book_tip,
            )
            .await?;
            // Persist the completed session so practice history, the stats
            // surface, and (opt-in) cloud sync all have something to read.
            // The store can degrade to in-memory at startup (see `open_stores`),
            // and a recap the user is waiting on must never be sunk by a
            // persistence failure — so we log and carry on rather than erroring.
            //
            // #449 T1 (§1b): the anti-fudge aggregates, computed ONCE, here,
            // in Rust, from the completed session's phrases. `wall_secs` uses
            // the exact derivation `SessionStore::save` uses for
            // `duration_secs`, so the ratio's denominator can never disagree
            // with the stored row.
            let wall_secs =
                (completed.ended_at - completed.started_at).num_milliseconds() as f64 / 1000.0;
            let integrity = brain::store::session_integrity(&completed.all_phrases(), wall_secs);
            {
                let store = state.session_store.lock_or_recover();
                if let Err(e) = store.save(
                    completed.id,
                    completed.started_at,
                    completed.ended_at,
                    &recap,
                ) {
                    tracing::warn!(error = %e, "could not persist the completed session");
                } else {
                    // Per-phrase rows depend on the session row's FK, so only
                    // after `save` succeeds — and never fail the recap over them.
                    if let Err(e) = store.save_phrases(completed.id, &completed.all_phrases()) {
                        tracing::warn!(error = %e, "could not persist the session's phrases");
                    }
                    // Score practice leaves exercise evidence too (#337 S4):
                    // repertoire work shows up in insights and, later, the
                    // teacher surfaces — same best-effort posture as every
                    // other log write.
                    if let Some(summary) = &recap.score_summary {
                        log_score_practice_best_effort(&store, summary);
                    }
                    // Session-level debug columns (#201), best-effort like the rest.
                    if let Err(e) = store.record_session_meta(
                        completed.id,
                        Some(env!("CARGO_PKG_VERSION")),
                        Some(practice_mode_label.as_str()),
                        session_score_id.as_deref(),
                    ) {
                        tracing::warn!(error = %e, "could not persist session metadata");
                    }
                    // #449 T1 (§1b): persist the integrity aggregates on the
                    // row we just saved — best-effort like everything here.
                    if let Err(e) = store.record_session_integrity(completed.id, &integrity) {
                        tracing::warn!(error = %e, "could not persist session integrity");
                    }
                }
            }
            // #449 T1: journal the recap narration only when the generator's
            // own flag says an LLM response actually produced it — offline,
            // thin-session, and failure fallbacks report false, so the
            // journal can't claim a narration that never fired. Written
            // directly against the completed session at its closing offset
            // (the live telemetry context is already closed above).
            if generator.recap_used_llm().await {
                write_practice_event(
                    state,
                    completed.id,
                    wall_secs,
                    "narration_used",
                    &serde_json::json!({ "kind": "recap" }),
                );
            }
            Ok(recap)
        }
        Err(SessionError::Empty) => Ok(empty_state_recap(elapsed_secs, instrument)),
        Err(other) => Err(CommandError::Recorder(other)),
    }
}

/// Pure implementation of `list_instruments`. Returns a clone of the
/// catalog cached on `AppState` — catalog loading happens once in
/// `AppState::new` and is shared across IPC calls. Errs with the load
/// failure when startup found no usable `profiles/` dir (#364).
pub fn list_instruments_impl(state: &AppState) -> Result<Vec<InstrumentInfo>, String> {
    state.list_instruments()
}

#[allow(clippy::too_many_arguments)]
async fn build_recap(
    completed: &CompletedSession,
    generator: &dyn RecapGenerator,
    taste_profile: Option<TasteProfile>,
    idiom_notes: Vec<brain::idiom_recap::IdiomMatch>,
    note_verdicts: Vec<brain::follower::NoteVerdict>,
    instrument_family: String,
    history_suggestions: Vec<brain::insights::PracticeSuggestion>,
    method_book_tip: Option<brain::pedagogy::PedagogyEntry>,
) -> Result<SessionRecap, CommandError> {
    completed
        .generate_recap_with_context(
            generator,
            taste_profile,
            idiom_notes,
            note_verdicts,
            instrument_family,
            history_suggestions,
            method_book_tip,
        )
        .await
        .map_err(CommandError::from)
}

/// #417-4/#389: the instrument catalog's family for a name ("Piano" →
/// "Keyboard"); empty when unknown, which the recap composer treats as
/// continuous-pitch (today's behavior).
fn instrument_family_for(state: &AppState, name: &str) -> String {
    state
        .instruments
        .iter()
        .find(|i| i.name == name)
        .map(|i| i.family.clone())
        .unwrap_or_default()
}

/// #471-4: an instrument profile's frequency range as a MIDI fold window.
/// `midi = 12·log2(hz/440) + 69`; each boundary snaps to the nearest note
/// when within 0.05 semitones (5 cents) — profiles store note frequencies
/// rounded to whole Hz, and blind inward rounding would steal real boundary
/// notes (165 Hz IS the trumpet's low E3, midi 52.02) — otherwise rounds
/// INWARD (lo ceils, hi floors: the window never claims a note the profile
/// doesn't cover). Intersected with physical MIDI 0..=127. `None` when the
/// range is degenerate or non-finite — callers fall back to the default
/// window. Full table: `docs/specs/471-h4-instrument-ranges.md` §3.
fn fold_window_from_hz(freq_min_hz: f64, freq_max_hz: f64) -> Option<FoldWindow> {
    const SNAP_SEMITONES: f64 = 0.05;
    let midi_of = |hz: f64| 12.0 * (hz / 440.0).log2() + 69.0;
    let bound = |hz: f64, inward_up: bool| -> Option<f64> {
        let m = midi_of(hz);
        if !m.is_finite() {
            return None;
        }
        let nearest = m.round();
        Some(if (m - nearest).abs() <= SNAP_SEMITONES {
            nearest
        } else if inward_up {
            m.ceil()
        } else {
            m.floor()
        })
    };
    let lo = bound(freq_min_hz, true)?.clamp(0.0, 127.0) as u8;
    let hi = bound(freq_max_hz, false)?.clamp(0.0, 127.0) as u8;
    (lo <= hi).then_some(FoldWindow { lo, hi })
}

/// #471-4: the fold window for a named instrument. **Voice-family
/// instruments are EXEMPT by founder rule** ("not vocals tho, leave that
/// be") and keep the default window, short-circuiting before any Hz math;
/// unknown names and degenerate ranges also resolve to the default.
fn fold_window_for(state: &AppState, name: &str) -> FoldWindow {
    state
        .instruments
        .iter()
        .find(|i| i.name == name)
        .filter(|i| i.family != "Voice")
        .and_then(|i| fold_window_from_hz(i.freq_min_hz, i.freq_max_hz))
        .unwrap_or_default()
}

/// #471-4: the ACTIVE session's fold window — the one every practice-material
/// `generate` folds toward. No session (or an unknown/Voice instrument) →
/// the default window, exactly today's behavior.
async fn session_fold_window(state: &AppState) -> FoldWindow {
    match state.active_session_instrument().await {
        Some(name) => fold_window_for(state, &name),
        None => FoldWindow::default(),
    }
}

/// A session can be in this state for ~this long before we stop assuming the
/// user simply didn't get started. Past it, a zero-phrase session more likely
/// means "we heard you but couldn't pick out phrases" than "you didn't play".
const HEARD_SOMETHING_SECS: f64 = 20.0;

/// Minimal recap for the zero-phrase case. Tone is intentionally "yoga teacher
/// not gym coach" — we never shame the user.
///
/// **Duration safety net (#185):** a session with real elapsed time must never
/// read as "you didn't play" — a vocalist who sang for a minute did play; the
/// gate just didn't form phrases. So when the session ran long enough, we show
/// a calm "couldn't quite pick out distinct phrases — check your mic" instead,
/// and always report the *real* `duration_secs` (so the header stops flooring a
/// zero-length session up to "1 minute").
fn empty_state_recap(duration_secs: f64, instrument: String) -> SessionRecap {
    let (overall_assessment, next_session_suggestions) = if duration_secs >= HEARD_SOMETHING_SECS {
        (
            "I couldn't quite pick out distinct phrases this time. If you were playing, \
             try moving a little closer to the mic or nudging your input level up — then \
             come back and I'll listen again."
                .to_owned(),
            vec![
                "Check your microphone input level, then play a few clear, sustained notes."
                    .to_owned(),
            ],
        )
    } else {
        (
            "Looks like you didn't get to play this time — come back when you're ready."
                .to_owned(),
            vec![
                "Open the app a few minutes before you want to play — just having it running can help."
                    .to_owned(),
            ],
        )
    };

    SessionRecap {
        score_summary: None,
        overall_assessment,
        strengths: Vec::new(),
        areas_to_improve: Vec::new(),
        next_session_suggestions,
        duration_secs,
        phrase_count: 0,
        instrument,
        fingerprint: None,
        intonation_display: None,
        groove_display: None,
        flavour: None,
        idiom_notes: Vec::new(),
        connections: Vec::new(),
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
    let store = state.session_store.lock_or_recover();
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
    let session = state.session_store.lock_or_recover().load(id)?;
    Ok(StoredSessionDto::from(session))
}

/// Pure implementation of `get_taste_profile`.
///
/// Returns the locally-captured taste profile, or [`TasteProfile::default`]
/// when none exists yet (cold start) — the onboarding UI treats a default-empty
/// profile as "not yet captured" without needing to special-case `null`.
pub fn get_taste_profile_impl(state: &AppState) -> Result<TasteProfile, CommandError> {
    let stored = state
        .session_store
        .lock_or_recover()
        .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)?;
    Ok(stored.unwrap_or_default())
}

/// Pure implementation of `set_taste_profile`.
pub fn set_taste_profile_impl(state: &AppState, profile: TasteProfile) -> Result<(), CommandError> {
    state
        .session_store
        .lock_or_recover()
        .upsert_taste_profile(LOCAL_TASTE_PROFILE_USER_ID, &profile)?;
    Ok(())
}

/// Fold a surfaced reveal into the Learner Model's collection (#253 S3) and
/// return the new **distinct** collection size. Load-or-default → the pure
/// `learner::apply_reveal` transition → write back; the whole read-modify-write
/// runs under the store lock so two rapid reveals can't lose an update. A
/// repeat of the same (concept, connection) leaves the size unchanged.
pub fn record_reveal_impl(
    state: &AppState,
    concept: &str,
    connection: &str,
    now_epoch_secs: i64,
) -> Result<usize, CommandError> {
    let store = state.session_store.lock_or_recover();
    let current = store
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    let next = brain::learner::apply_reveal(&current, concept, connection, now_epoch_secs);
    store.upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &next)?;
    Ok(next.collection_size())
}

/// Pure implementation of `get_practice_stats`.
pub fn get_practice_stats_impl(state: &AppState) -> Result<PracticeStatsDto, CommandError> {
    let all_sessions = state.session_store.lock_or_recover().list_recent(10000)?;
    let stats = PracticeStats::calculate(&all_sessions, Utc::now());
    Ok(PracticeStatsDto::from(stats))
}

// ---------------------------------------------------------------------------
// #449 T2: the dashboard sync projection, device → cloud (doc §2, P1–P4)
// ---------------------------------------------------------------------------
//
// These DTOs are THE shapes the frontend sync layer (`syncStore.ts`,
// `syncDashboard`) may read on the projection path. They are read-only,
// shaped here in Rust (house rule: no business logic in the frontend), and
// they carry exactly the columns of the cloud star schema —
// `supabase/migrations/0006_teacher_dashboard_star_schema.sql` — nothing
// more. Privacy is structural: what a type doesn't have, no caller can leak.

/// P1 — the local score practised in a session, for the `dim_material`
/// score row (0006 `dim_material`: `score_id`, `label`, kind `'score'`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreRefDto {
    pub score_id: String,
    /// The piece title — the teacher-facing label (doc §2 P1 "score_id→title").
    pub title: String,
}

/// P1 — one `fact_session` row's device-side fields (0006 `fact_session`:
/// `device_session_id`, `started_at`, `ended_at`, `duration_secs`,
/// `played_secs`, `note_count`, `silence_ratio`, `phrase_count`,
/// `instrument`, `practice_mode`, `score_material_id`←[`ScoreRefDto`],
/// `fingerprint`, `app_version`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFactDto {
    /// The `SessionRecorder` `SessionId` — the cloud idempotency key
    /// (`fact_session.device_session_id`, unique per student).
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Wall clock.
    pub duration_secs: f64,
    pub phrase_count: usize,
    pub instrument: String,
    /// Session-meta debug fields (`None` on older rows — honest absence).
    pub practice_mode: Option<String>,
    pub app_version: Option<String>,
    /// #449 T1 integrity aggregates — computed once, in Rust, at session
    /// close; projected verbatim, never re-derived (doc §1b/§2).
    pub played_secs: Option<f64>,
    pub note_count: Option<u64>,
    pub silence_ratio: Option<f64>,
    /// The evidence-gated fingerprint — already flows on the legacy push.
    pub fingerprint: Option<brain::fingerprint::MusicalFingerprint>,
    /// The score practised, when one was and it still exists locally.
    pub score: Option<ScoreRefDto>,
}

/// P2 — one THIN `fact_phrase` row (0006 `fact_phrase`: `phrase_index`,
/// `start_secs`, `end_secs`, `note_count`, `stability`, `tone`, `key_name`).
///
/// Deliberately not [`PhraseSummary`]: doc §2 P2, verbatim — "**Not** the
/// full `phrase_json` — no onsets vector, no pitch curves". This type has
/// no such fields, so the projection cannot send them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhraseFactDto {
    pub phrase_index: usize,
    /// One clock: seconds from session start (the #451 played-time clock).
    pub start_secs: f64,
    pub end_secs: f64,
    pub note_count: usize,
    pub stability: f64,
    /// Flat descriptor only (five bounded floats).
    pub tone: Option<tone::ToneDescriptor>,
    /// Key estimate NAME ("G Mixolydian"); `None` when the evidence gate
    /// failed — the dashboard must not out-claim the strip (#316).
    pub key_name: Option<String>,
}

/// P4 — one `fact_tool_event` row (0006 `fact_tool_event`:
/// `device_event_id`, `at_secs`, `kind`, `params`). `params_json` is
/// ids-and-numbers-only by the T1 vocabulary (doc §1a); no content.
///
/// SEMANTICS CAVEAT (#470, option b — documented at the projection site on
/// purpose): a `narration_used {"kind":"recap"}` event means **an LLM
/// response parsed**, NOT "the shown recap text was LLM-authored". The
/// recap parser is all-defaults-forgiving, so a valid-JSON-wrong-keys
/// response still counts as "used" while every user-visible field is a
/// canned default. Any dashboard reading of this event must say "narration
/// requested/parsed", never "AI wrote this recap". Tightening the parser is
/// #470's option (a), out of scope here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEventFactDto {
    /// Local `practice_events.id` — the cloud idempotency key
    /// (`fact_tool_event.device_event_id`, unique per session).
    pub device_event_id: i64,
    /// One clock: seconds from session start (doc §1a).
    pub at_secs: f64,
    pub kind: String,
    pub params_json: String,
}

/// Everything the sync layer needs to project ONE closed session up
/// (P1 + P2 + P4). P3 (exercises) is session-unlinked locally and rides
/// [`list_exercise_facts`] with its own watermark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProjectionDto {
    pub session: SessionFactDto,
    pub phrases: Vec<PhraseFactDto>,
    pub events: Vec<ToolEventFactDto>,
}

/// Pure implementation of `get_session_projection`.
pub fn get_session_projection_impl(
    state: &AppState,
    session_id: String,
) -> Result<SessionProjectionDto, CommandError> {
    use brain::session::SessionId;
    let id = SessionId::from_str(&session_id)
        .map_err(|_| CommandError::Store(brain::store::StoreError::NotFound(session_id)))?;
    let store = state.session_store.lock_or_recover();
    let stored = store.load(id)?;
    let meta = store.session_meta(id)?;

    // The score reference, when the session had one AND it still exists
    // (deleted score → no material link; honest absence). Prefer the recap's
    // own judged title, fall back to the library row.
    let score = meta.score_id.as_ref().and_then(|sid| {
        stored
            .recap
            .score_summary
            .as_ref()
            .map(|s| s.score_title.clone())
            .or_else(|| store.score_title(sid).ok().flatten())
            .map(|title| ScoreRefDto {
                score_id: sid.clone(),
                title,
            })
    });

    // Integrity columns come from the persisted sessions row (computed once
    // at close — doc §1b), via the same summary SELECT the History page uses.
    let summary = store
        .list_recent(10_000)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| CommandError::Store(brain::store::StoreError::NotFound(id.as_str())))?;

    let phrases = store
        .load_phrases(id)?
        .into_iter()
        .map(|p| PhraseFactDto {
            phrase_index: p.phrase_index,
            start_secs: p.start_time,
            end_secs: p.end_time,
            note_count: p.note_count,
            stability: p.stability,
            tone: p.tone,
            key_name: p.key.as_ref().map(brain::theory::KeyEstimate::name),
        })
        .collect();

    let events = store
        .list_practice_events(&id.as_str())?
        .into_iter()
        .map(|e| ToolEventFactDto {
            device_event_id: e.id,
            at_secs: e.at_secs,
            kind: e.kind,
            params_json: e.params_json,
        })
        .collect();

    Ok(SessionProjectionDto {
        session: SessionFactDto {
            id: id.as_str(),
            started_at: stored.started_at,
            ended_at: stored.ended_at,
            duration_secs: stored.recap.duration_secs,
            phrase_count: stored.recap.phrase_count,
            instrument: stored.recap.instrument.clone(),
            practice_mode: meta.practice_mode,
            app_version: meta.app_version,
            played_secs: summary.played_secs,
            note_count: summary.note_count,
            silence_ratio: summary.silence_ratio,
            fingerprint: stored.recap.fingerprint.clone(),
            score,
        },
        phrases,
        events,
    })
}

/// Pure implementation of `list_exercise_facts` — P3 rows past the
/// caller's watermark, `spec_json`/`seed` structurally absent (see
/// [`ExerciseFactRow`]).
pub fn list_exercise_facts_impl(
    state: &AppState,
    after_id: i64,
) -> Result<Vec<ExerciseFactRow>, CommandError> {
    Ok(state
        .session_store
        .lock_or_recover()
        .list_exercise_facts_after(after_id)?)
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
    let mut follower = score_id.as_deref().and_then(|id| state.build_follower(id));
    // Verdict tolerances are profile-driven (#337 S2, founder decision
    // 2026-07-10): the instrument's vibrato tolerance IS its in-tune slack —
    // voice gets more room than piano. Profiles without one keep the
    // follower's built-in default.
    if let (Some(f), Some(inst)) = (
        follower.as_mut(),
        state.instruments.iter().find(|i| i.name == instrument),
    ) {
        f.set_verdict_tolerances(brain::follower::VerdictTolerances {
            // The 20-cent floor keeps tight profiles (piano: 10¢) from
            // grading live mic detection stricter than the detector's own
            // jitter — a deliberate widening, not profile drift.
            hit_cents: inst.vibrato_tolerance_cents.max(20.0),
        });
    }
    // Cursor diagnostics (#277: "no cursor" reports were undebuggable): log
    // plainly whether this session has a follower at all.
    match (&score_id, follower.is_some()) {
        (Some(id), true) => tracing::info!(score_id = %id, "score follower installed"),
        (Some(id), false) => {
            tracing::warn!(score_id = %id, "score session started WITHOUT a follower")
        }
        (None, _) => {}
    }
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
            // Record the score id on the committed session so the persisted
            // row can name the piece practised (see `record_session_meta`).
            if let Some(active) = state.active_session.lock().await.as_mut() {
                active.score_id = score_id;
            }
            // The jam chart is served AFTER a session ends (recap sketch),
            // so it must clear at the next session's COMMIT — even one that
            // never opens a pipeline — or a no-mic session could recap the
            // previous session's chart (review nice-to-have).
            state.chord_chart.lock_or_recover().clear();
            // Spin up the pipeline only after state-machine commit —
            // if we can't open the mic we at least want the recorder
            // in a consistent state.
            if let Some(profile) = detector_profile {
                let app_for_emit = app.clone();
                let app_for_phrase = app.clone();
                let app_for_position = app.clone();
                let app_for_verdict = app.clone();
                // Fresh idiom buffer for this session — discard any leftovers
                // from a prior session, then hand the pipeline its own handle
                // so it can fill it (offline, off the realtime callback).
                state.idiom_buffer.clear();
                let idiom_buffer = state.idiom_buffer.clone();
                // Fresh phrase buffer too — the worker fills it as phrases close
                // so end_practice_session can record them into the recap (#185).
                state.phrase_buffer.lock_or_recover().clear();
                state.verdict_buffer.lock_or_recover().clear();
                state.chord_buffer.lock_or_recover().clear();
                let phrase_buffer = state.phrase_buffer.clone();
                let verdict_buffer = state.verdict_buffer.clone();
                let chord_buffer = state.chord_buffer.clone();
                let chord_chart = state.chord_chart.clone();
                // #341 review M2: while an exploration overlays the running
                // score session (tap-a-measure), the follower must not keep
                // judging — the rowed cell is near-identical to the score,
                // so it would align, advance, and pollute the recap with
                // verdicts about music the player wasn't playing AT the
                // score. Gate verdicts and cursor emits on the live
                // exploration; both resume at "Back to listening".
                let explore_gate_verdict = state.active_explore.clone();
                let explore_gate_position = state.active_explore.clone();
                let explore_gate_phrase = state.active_explore.clone();
                // #354: fresh per session — "first emitted" means first
                // for THIS session, unlike the process-wide Once it replaced.
                let mut position_log = ScorePositionLog::new();
                // Hand the worker closures their own handles to the (maybe-absent)
                // accompaniment so they can drive the follow-me band live. When
                // no band is playing these locks see `None` and do nothing.
                let accomp_for_event = state.accompaniment.clone();
                let accomp_for_phrase = state.accompaniment.clone();
                // Always-on perception (independent of whether the band is
                // playing) so the UI can show what the app hears the moment the
                // session starts. Owned by the closure → lives on the worker
                // thread; allocation here is fine (not the realtime callback).
                let mut perception = PerceptionTracker::new();
                let mut last_perception_secs: Option<f64> = None;
                // #349 T3b: polyphonic hearing for this session — kill-
                // switch honest: no ONNX runtime → None, everything else
                // runs exactly as before (voicing-true slashes just don't
                // upgrade). Arc-shared: the worker feeds it, this closure
                // reads it; the LAST drop joins the inference thread.
                let poly = transcribe::PolyRunner::spawn()
                    .map_err(|e| {
                        tracing::info!(error = %e, "session runs without polyphonic hearing");
                        e
                    })
                    .ok()
                    .map(std::sync::Arc::new);
                let poly_for_emit = poly.clone();
                match AudioPipeline::start_with_follower(
                    profile,
                    follower,
                    Some(idiom_buffer),
                    poly,
                    // #445: the shared click-gate slot — empty until the
                    // Pocket installs its gate, then the worker ignores
                    // onsets that agree with our own click.
                    Some(state.click_gate.clone()),
                    move |event, chroma| {
                        // Feed the accompaniment's clock from onset timing before
                        // the event is moved into the emit. Lock is uncontended
                        // (processing thread only) and never touches the realtime
                        // callback.
                        if let Some(accompaniment) = accomp_for_event.lock_or_recover().as_mut() {
                            accompaniment
                                .driver
                                .observe_event(event.is_onset, event.timestamp_secs);
                        }
                        // Update perception and emit a throttled snapshot so the
                        // UI shows live tempo/key/feel as the player plays.
                        perception.observe(&event);
                        // ~10 Hz chroma readings feed the chord tracker
                        // (#349 T1) — the "I hear Cmaj7" label.
                        if let Some(c) = chroma {
                            // #349 T3b: the voicing-true bass (the lowest
                            // note the poly engine hears sounding) refines
                            // the slash before the chroma reading lands.
                            if let Some(p) = &poly_for_emit {
                                if let Some(midi) = p.sounding_bass(event.timestamp_secs) {
                                    perception.observe_poly_bass(midi, event.timestamp_secs);
                                }
                            }
                            perception.observe_chroma(&c, event.timestamp_secs);
                            // Chord drills grade from the same stable
                            // readings the strip shows (#349 T2b). The
                            // buffer owns the edge-trigger/release/slash-
                            // refresh contract — see ChordChangeBuffer.
                            chord_buffer
                                .lock_or_recover()
                                .observe(perception.current_heard_chord());
                        }
                        let now = event.timestamp_secs;
                        let due = last_perception_secs
                            .is_none_or(|last| now - last >= PERCEPTION_EMIT_INTERVAL_SECS);
                        if due {
                            let snap = perception.snapshot(now);
                            // The jam chart records label CHANGES from the
                            // same snapshots the strip renders (#349 T4a).
                            chord_chart.lock_or_recover().observe(&snap, now);
                            let _ = app_for_emit.emit("perception", snap);
                            last_perception_secs = Some(now);
                        }
                        let _ = app_for_emit.emit("audio-event", event);
                    },
                    make_phrase_closed_callback(
                        app_for_phrase,
                        phrase_buffer,
                        accomp_for_phrase,
                        explore_gate_phrase,
                    ),
                    move |position| {
                        if explore_gate_position.lock_or_recover().is_some() {
                            // Exploration on stage — the cursor rests. One
                            // log line per suppressed stretch (#354), so a
                            // swallowed stream never reads as a dead one.
                            if let Some(PositionBreadcrumb::SuppressionStarted { emitted }) =
                                position_log.swallowed()
                            {
                                tracing::info!(
                                    emitted,
                                    "score-position suppressed (exploration on stage)"
                                );
                            }
                            return;
                        }
                        emit_score_position_updated(&app_for_position, &mut position_log, position);
                    },
                    move |verdict| {
                        // #341 M2: no score judging while the exploration is
                        // on stage — neither live strip nor recap buffer.
                        if explore_gate_verdict.lock_or_recover().is_some() {
                            return;
                        }
                        // Buffer a copy for the recap's score summary
                        // (#337 S4), then emit for the live strip.
                        verdict_buffer.lock_or_recover().push(verdict.clone());
                        let _ = app_for_verdict.emit("note-verdict", verdict);
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
    // Stop the band first so it doesn't keep playing once analysis stops. Hold
    // the command lock so this can't interleave with a concurrent start/stop.
    {
        let _cmd = state.accompaniment_cmd_lock.lock().await;
        // #449 T1: journal the stops the session-end teardown implies — the
        // telemetry context is still open here (the impl closes it), so the
        // tool-on spans close honestly at the session boundary.
        if state.teardown_accompaniment() {
            log_practice_event_best_effort(&state, "band_stop", serde_json::json!({}));
        }
        // #421 S1: the click stops with the session, like the band.
        if state.teardown_pocket() {
            note_pocket_stopped(&state);
        }
    }
    emit_pocket_status(&app, false, 0.0);
    // The key override is session-scoped — reset it so the next session
    // auto-detects fresh.
    state.clear_key_override();
    emit_accompaniment_status(&app, false);
    state.stop_audio_pipeline().await;
    end_practice_session_impl(state.inner())
        .await
        .map_err(|e| e.to_frontend())
}

/// Start the follow-me accompaniment ("Play with me"): open the audio output
/// engine, build the synth on the render thread, and install the driver the
/// audio worker feeds. Fully offline — no network.
///
/// #445 pt 9: the band carries a clock. `Some(tempo_bpm)` (solo mode — the
/// frontend passes the Pocket's set tempo) is clamped by the SAME
/// `clamp_pocket_params` the click uses and installed as the band's tempo,
/// so it plays immediately — starting the band silences the click (one
/// audio owner), so the band must be the clock the player locks to.
/// Follow/Handoff retimes then arrive via `set_band_tempo`, exactly as the
/// click's arrive via `set_pocket_tempo`.
///
/// Review MF2: `None` (room mode — "listen to the room") installs NO
/// override: the room's live players ARE the clock, so the band keeps the
/// legacy listen-and-join path — silent until the live clock locks onto
/// the room's pulse, aligned to its phase.
#[tauri::command]
pub async fn start_accompaniment<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    tempo_bpm: Option<f64>,
) -> Result<(), String> {
    // Serialize start/stop/teardown so two overlapping commands can't race the
    // device handoff (Tauri runs each command as its own task).
    let _cmd = state.accompaniment_cmd_lock.lock().await;

    // Tear the previous band down FIRST — fully (device released + threads
    // joined) and off the accompaniment lock — so we never (a) open a second
    // output device while the old one is still live, nor (b) drop-join an
    // `AudioOutput` while holding the std mutex the audio worker locks per frame.
    if state.teardown_accompaniment() {
        // #449 T1: a restart is a real stop of the previous band.
        log_practice_event_best_effort(&state, "band_stop", serde_json::json!({}));
    }
    // #421 S1: one audio-output owner — the band replaces the click.
    if state.teardown_pocket() {
        note_pocket_stopped(&state);
    }
    emit_pocket_status(&app, false, 0.0);

    // The receiver moves into the render-thread source; the sender lives in the
    // driver on the processing thread.
    let (sender, receiver) = accompaniment_control_channel(ACCOMPANIMENT_CHANNEL_CAPACITY);
    let output = AudioOutput::start(move |sample_rate| {
        AccompanimentSynthSource::new(AccompanimentSynth::new(sample_rate), receiver)
    })
    .map_err(|e| format!("could not start accompaniment audio output: {e}"))?;

    let mut accompaniment = Accompaniment {
        output,
        driver: AccompanimentDriver::new(sender),
    };
    // #445 pt 9: install the Pocket's set tempo as the band's clock (solo
    // mode) — the same clamp as the click, so the two carriers can never
    // disagree. Room mode (None) installs nothing: the room is the clock.
    install_band_clock(&mut accompaniment.driver, tempo_bpm);
    // Carry over a key the user pinned earlier this session so the band starts
    // in it rather than re-running auto-detection.
    if let Some((tonic, minor)) = state.current_key_override() {
        accompaniment.driver.set_key_override(tonic, minor);
    }
    // The slot is guaranteed empty (we just tore down under the cmd lock), so
    // this assignment never drops a live `AudioOutput` while holding the lock.
    *state.accompaniment.lock_or_recover() = Some(accompaniment);

    // #449 T1: journal the band's birth, noting whether it starts under a
    // pinned key (the pin itself is a `band_key_pin` event when set).
    log_practice_event_best_effort(
        &state,
        "band_start",
        serde_json::json!({ "key_pinned": state.current_key_override().is_some() }),
    );
    emit_accompaniment_status(&app, true);
    Ok(())
}

/// Stop the follow-me accompaniment. No-op if it isn't playing.
#[tauri::command]
pub async fn stop_accompaniment<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _cmd = state.accompaniment_cmd_lock.lock().await;
    if state.teardown_accompaniment() {
        // #449 T1: only a real stop is journaled (teardown reports it).
        log_practice_event_best_effort(&state, "band_stop", serde_json::json!({}));
    }
    emit_accompaniment_status(&app, false);
    Ok(())
}

/// #421 S1: start The Pocket — the strict Anchor click with an optional
/// one-bar count-in, at a clamped tempo. Replaces the band (one audio
/// owner per device).
#[tauri::command]
pub async fn start_pocket<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    tempo_bpm: f64,
    beats_per_bar: u8,
    count_in: bool,
) -> Result<(), String> {
    let _cmd = state.accompaniment_cmd_lock.lock().await;
    // #449 T1: journal what this start displaces — a real band stop and/or a
    // real stop of the previous click (restart), never a no-op teardown.
    if state.teardown_accompaniment() {
        log_practice_event_best_effort(&state, "band_stop", serde_json::json!({}));
    }
    if state.teardown_pocket() {
        note_pocket_stopped(&state);
    }
    emit_accompaniment_status(&app, false);

    let (tempo, beats) = clamp_pocket_params(tempo_bpm, beats_per_bar);
    // #421 S2: the tempo channel — Follow/Handoff push, render pops.
    let (tempo_tx, tempo_rx) = ears::output_engine::pocket_tempo_channel(16);
    // #445: the click-fire channel — the render-side metronome reports
    // every click's sample index; the audio worker's click gate consumes.
    let (fire_tx, fire_rx) = ears::output_engine::click_fire_channel(CLICK_FIRE_CHANNEL_CAPACITY);
    let output = ears::output_engine::AudioOutput::start(move |sample_rate| {
        let config = ears::output::MetronomeConfig {
            bpm: tempo,
            time_signature: (beats, 4),
            accent_first_beat: true,
            volume: 0.8,
        };
        // The params are clamped into the validated range, so construction
        // cannot fail.
        let metronome = ears::output::Metronome::new(config, sample_rate)
            .expect("clamped pocket config is always valid")
            .with_count_in(if count_in { 1 } else { 0 })
            .with_fire_channel(fire_tx);
        ears::output_engine::TempoFedMetronome::new(metronome, tempo_rx)
    })
    .map_err(|e| format!("could not start the click: {e}"))?;
    // #445: install the click gate. The epoch is recorded now — playback
    // of the metronome's sample 0 begins (device already running) the
    // moment `AudioOutput::start` returns; the few-ms slop is far inside
    // the gate window. The output rate is the unit fire indices count in.
    *state.click_gate.lock_or_recover() = Some(crate::audio_pipeline::ClickGate {
        fires: fire_rx,
        epoch: std::time::Instant::now(),
        output_sample_rate: output.sample_rate(),
    });
    *state.pocket.lock_or_recover() = Some(Pocket { output, tempo_tx });
    // #449 T1: journal the click's birth at its CLAMPED tempo (played ==
    // reported == journaled) and reset the tempo-coalescing baseline.
    note_pocket_started(&state, tempo, count_in);
    emit_pocket_status(&app, true, tempo);
    Ok(())
}

/// #421 S1: stop The Pocket. No-op if silent.
#[tauri::command]
pub async fn stop_pocket<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _cmd = state.accompaniment_cmd_lock.lock().await;
    if state.teardown_pocket() {
        // #449 T1: a real stop, with the final effective tempo.
        note_pocket_stopped(&state);
    }
    emit_pocket_status(&app, false, 0.0);
    Ok(())
}

/// #421 S2: re-time a playing click to the player's measured pulse.
/// Clamped by the SAME function start_pocket uses (played == reported);
/// a silent Pocket is a calm no-op — Follow policy lives in the
/// frontend, measurement in perception, and this seam just moves a
/// number onto the render thread without locks.
#[tauri::command]
pub fn set_pocket_tempo(state: State<'_, AppState>, tempo_bpm: f64) -> Result<(), String> {
    let retimed = if let Some(pocket) = state.pocket.lock_or_recover().as_mut() {
        push_clamped_tempo(&mut pocket.tempo_tx, tempo_bpm);
        true
    } else {
        false
    };
    // #449 T1: journal the retime — coalesced (≥5 BPM and ≥5 s from the last
    // journaled row), at the same CLAMPED value the render thread was fed,
    // and only when a click actually consumed the push. Off the pocket lock
    // so the telemetry/session-store locks never nest inside it.
    if retimed {
        let (effective, _) = clamp_pocket_params(tempo_bpm, 4);
        note_pocket_tempo(&state, effective);
    }
    Ok(())
}

/// #449 T1: record a Pocket personality change (anchor/follow/handoff) in
/// the practice_events journal. The mode itself lives in frontend state
/// (the follow policy is UI orchestration — #421 S2), so the backend can't
/// observe changes; this command is the journaling seam. It has no effect
/// on playback. No active session → no row, calmly. Frontend wiring rides
/// the T2/T4 work (this slice is backend-only; see the spec §4b).
#[tauri::command]
pub fn set_pocket_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    match mode.as_str() {
        "anchor" | "follow" | "handoff" => {}
        other => {
            return Err(format!(
                "pocket mode can be anchor, follow, or handoff — not {other:?}"
            ))
        }
    }
    log_practice_event_best_effort(&state, "pocket_mode", serde_json::json!({ "mode": mode }));
    Ok(())
}

/// #445 pt 9: re-time the playing band to the Pocket's effective clock —
/// the band's `set_pocket_tempo`. Clamped by the SAME function, a silent
/// band is a calm no-op, and the follow policy stays in the frontend; this
/// seam just moves a number onto the render thread over the existing SPSC
/// control channel (phase-preserving on the synth side).
#[tauri::command]
pub fn set_band_tempo(state: State<'_, AppState>, tempo_bpm: f64) -> Result<(), String> {
    if let Some(band) = state.accompaniment.lock_or_recover().as_mut() {
        set_clamped_band_tempo(&mut band.driver, tempo_bpm);
    }
    Ok(())
}

/// #445 pt 9 review MF1: the band's clamp+forward seam, testable with a
/// bare driver/channel pair — the mirror of `push_clamped_tempo` below.
/// This is what protects the 220..300 window on the band side: without it
/// a raw reading would be APPLIED verbatim by the synth. One function
/// under both `set_band_tempo` and the start-time install, so the band's
/// played values can never disagree with the click's.
fn set_clamped_band_tempo(driver: &mut AccompanimentDriver, tempo_bpm: f64) {
    let (tempo, _) = clamp_pocket_params(tempo_bpm, 4);
    driver.set_tempo(tempo);
}

/// #445 pt 9 review MF2: the band's start-time clock install. `Some` =
/// solo mode — the Pocket's set tempo carries (clamped). `None` = room
/// mode — the room's players are the clock, so nothing is installed and
/// the band keeps the legacy listen-and-join path.
fn install_band_clock(driver: &mut AccompanimentDriver, tempo_bpm: Option<f64>) {
    if let Some(bpm) = tempo_bpm {
        set_clamped_band_tempo(driver, bpm);
    }
}

/// #421 S2 review MF5: the clamp+push seam, testable with a bare
/// channel. This is what protects the 220..300 window where the
/// Metronome's own validation (30..=300) would happily APPLY a value
/// the product never plays — the frontend's same-range gate is UI
/// policy, this is the API guarantee.
fn push_clamped_tempo(tx: &mut ringbuf::HeapProd<f64>, tempo_bpm: f64) {
    use ringbuf::traits::Producer;
    let (tempo, _) = clamp_pocket_params(tempo_bpm, 4);
    let _ = tx.try_push(tempo); // full ring = drop, fine
}

/// #421 S1: the Pocket's parameter clamps — tempo 40..=220 (NaN → 40),
/// beats 2..=7. One function so the played values and the reported value
/// can never disagree.
fn clamp_pocket_params(tempo_bpm: f64, beats_per_bar: u8) -> (f64, u8) {
    let tempo = if tempo_bpm.is_nan() {
        40.0
    } else {
        tempo_bpm.clamp(40.0, 220.0)
    };
    (tempo, beats_per_bar.clamp(2, 7))
}

/// Pin the band to a specific key — the user correcting the auto-read (e.g.
/// "it's E minor, not G major"). `minor` selects Aeolian vs. Ionian. Takes effect
/// immediately on a playing band and on the next band start this session.
#[tauri::command]
pub async fn set_accompaniment_key(
    state: State<'_, AppState>,
    tonic: u8,
    minor: bool,
) -> Result<(), String> {
    // Serialize with start/stop so a pin during a band's device-init window
    // isn't dropped (it would otherwise see no band yet, then be overwritten).
    let _cmd = state.accompaniment_cmd_lock.lock().await;
    state.set_key_override(tonic, minor);
    // #449 T1: journal the pin as the user expressed it — the shipped
    // command speaks (tonic, minor), so that's what the row records
    // honestly; params_json is additive if the pin ever grows modes.
    log_practice_event_best_effort(
        &state,
        "band_key_pin",
        serde_json::json!({ "tonic": tonic, "minor": minor }),
    );
    Ok(())
}

/// Resume automatic key-following (undo a pin).
#[tauri::command]
pub async fn clear_accompaniment_key(state: State<'_, AppState>) -> Result<(), String> {
    let _cmd = state.accompaniment_cmd_lock.lock().await;
    state.clear_key_override();
    Ok(())
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
        // Thread the most recent tips from this session into the prompt so the
        // coach avoids repeating itself. Capped to keep the prompt tight.
        previous_tips: state.recent_tip_texts(PREVIOUS_TIPS_WINDOW).await,
        score_title: state.active_session_score_title().await,
    };
    let tip = state
        .get_coaching_tip(&phrase, &session_ctx)
        .await
        .map_err(|e| e.to_string())?;
    // #449 T1: `Some` here means a live LLM tip genuinely fired — under the
    // engine's silence-beats-a-lie contract, offline / rate-limited / failed
    // calls all return `None` (there is no canned-tip path). Journaled as a
    // usage fact only; no tip content leaves the coaching flow.
    if tip.is_some() {
        log_practice_event_best_effort(
            &state,
            "narration_used",
            serde_json::json!({ "kind": "tip" }),
        );
    }
    Ok(tip)
}

/// How many recent coaching tips to feed back to the live coach as
/// `previous_tips` (avoid-repetition context). A small window keeps the prompt
/// focused on what was *just* said without bloating it.
const PREVIOUS_TIPS_WINDOW: usize = 5;

/// Persist a coaching tip into the active session's recorder so it lands in the
/// session history and the end-of-session recap input. Called by the frontend
/// right after a tip is surfaced. No-ops (errors) when no session is active.
#[tauri::command]
pub async fn record_coaching_tip(
    state: State<'_, AppState>,
    phrase_index: usize,
    tip: CoachingTip,
) -> Result<(), String> {
    state
        .record_coaching_tip(phrase_index, &tip)
        .await
        .map_err(|e| e.to_string())
}

/// Offer a real-world music "reveal" for a just-completed phrase, from the live
/// perception reading (key + mode + confidence). Selection is curated + grounded
/// (`brain::connections`, Rust core): it never fabricates a connection and returns
/// `None` when confidence is low, the mode has no curated match, or the phrase
/// isn't on the reveal cadence. When the coaching opt-in is on (#253 S2), the
/// `why` line is reworded by the LLM (grounded — the artist/piece is unchanged);
/// offline it stays the curated line. See #253.
#[tauri::command]
pub async fn get_reveal(
    state: State<'_, AppState>,
    tonic: u8,
    mode: String,
    confidence: f32,
    phrase_index: usize,
) -> Result<Option<Reveal>, String> {
    let ctx = MusicalContext {
        tonic,
        mode,
        confidence,
    };
    match reveal_on_phrase(&ctx, phrase_index, DEFAULT_REVEAL_CADENCE) {
        Some(reveal) => Ok(Some(state.enrich_reveal(reveal).await)),
        None => Ok(None),
    }
}

/// Record a surfaced reveal into the Learner Model's collection (#253 S3) and
/// return the new distinct-collection size for the UI's count. Called by the
/// frontend right after a reveal is shown, mirroring `record_coaching_tip`.
/// Dedup lives in the pure `learner::apply_reveal` transition: a repeat of the
/// same (concept, connection) bumps its count but not the size.
#[tauri::command]
pub fn record_reveal(
    state: State<'_, AppState>,
    concept: String,
    connection: String,
) -> Result<usize, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    record_reveal_impl(&state, &concept, &connection, now).map_err(|e| e.to_frontend())
}

// ---------------------------------------------------------------------------
// Free-play exploration (#255): a reveal names a sound; these turn it into
// material with tappable mutation chips.
// ---------------------------------------------------------------------------

/// One exploration rep as the frontend renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreDto {
    pub label: String,
    pub music_xml: String,
    pub chips: Vec<ChipSpec>,
    /// Roots as pitch classes in PLAY order — the RV colored cells (#278).
    pub root_pitch_classes: Vec<u8>,
    /// Display names for those roots, spelled per the exploration's key
    /// signature (#335): flat signatures name flats so the cells can never
    /// contradict the staff. Same order/length as `root_pitch_classes`.
    pub root_names: Vec<String>,
    /// The dot-staff view (#292): all theory (steps, spelling, accidentals)
    /// computed here; the frontend renders geometry only.
    pub staff: brain::score::cellstaff::CellStaffView,
    /// Whether an edit can be undone (#292 slice 3).
    pub can_undo: bool,
    /// #471-4: a calm sentence when the dealt row was wider than the session
    /// instrument's range and fell back to the full window (never a clamp —
    /// the pattern stays exact). `None` when everything fits. Surfaces
    /// through the existing exploreNotice channel; additive on the wire.
    pub range_notice: Option<String>,
}

/// Assemble the ExploreDto every explore command returns.
fn explore_dto(explore: &ExploreState, seq: &brain::coach::GeneratedSequence) -> ExploreDto {
    let chips = brain::coach::suggest_chips(explore);
    // ONE key derivation for everything the player sees or edits —
    // explore_key follows the figure's family (scale, else the jam
    // chord, #349 T4a review M4-r2: a heard Cm7 must engrave Eb/Bb
    // under flats on the RENDERED staff, not just in the edit engine).
    let key = explore_key(explore);
    // Same rule as the root cells: the label speaks the engraved
    // signature's spelling, never "C#" over flats (#335). Progression rows
    // need more than respell_label's leading-token fix: every chip tap
    // regenerates the label through the sharp-only generator, so the
    // chord-name segment is REBUILT here from the steps + this key — the
    // one derivation every rep passes through (T3c review r2).
    let label = if let Some(steps) = explore.spec.progression.as_ref().filter(|p| !p.is_empty()) {
        let names: Vec<String> = steps
            .iter()
            .map(|st| {
                format!(
                    "{}{}",
                    brain::coach::tonic_display_name((explore.tonic + st.offset) % 12, key.fifths),
                    st.chord.quality().suffix()
                )
            })
            .collect();
        // label_for emits "your progression · <names> · <tail…>" — swap
        // the names part, keep the descriptive tail.
        let mut parts: Vec<String> = seq.label.split(" · ").map(str::to_owned).collect();
        if parts.len() >= 2 {
            parts[1] = names.join(" → ");
            parts.join(" · ")
        } else {
            format!("your progression · {}", names.join(" → "))
        }
    } else {
        brain::coach::respell_label(&seq.label, key.fifths)
    };
    let music_xml = brain::score::emit::score_model_to_musicxml(&sequence_to_score_model(
        seq,
        &label,
        key.clone(),
    ));
    ExploreDto {
        label,
        music_xml,
        chips,
        root_pitch_classes: seq.root_order.iter().map(|&r| r % 12).collect(),
        root_names: seq
            .root_order
            .iter()
            .map(|&r| brain::coach::tonic_display_name(r % 12, key.fifths).to_owned())
            .collect(),
        staff: brain::score::cellstaff::cell_staff_view(
            seq,
            key,
            // Per-segment spelling (#471-2 F3) derives from the same material
            // label the drawn signature does — one derivation, one voice.
            &brain::coach::explore_material(&explore.spec),
        ),
        can_undo: !explore.history.is_empty(),
        // #471-4 honesty: the row left the instrument's range rather than
        // bend the pattern — say so calmly, never silently.
        range_notice: seq.range_fallback.then(|| {
            "that pattern reaches past your instrument's range — dealing it in the full window \
             instead"
                .to_owned()
        }),
    }
}

/// Everything one exercise-log append needs (kept as a struct so call sites
/// stay readable).
struct ExerciseOutcome<'a> {
    source: &'a str,
    label: &'a str,
    spec: &'a brain::coach::VariationSpec,
    seed: u64,
    difficulty: u8,
    tonic: u8,
    accuracy: Option<f64>,
}

/// Append a score-practice session to the exercise log (#337 S4) —
/// best-effort like every log write. The spec_json is a score reference,
/// not a VariationSpec; `brain::insights::shape_of` knows the shape.
fn log_score_practice_best_effort(
    store: &SessionStore,
    summary: &brain::coaching::ScorePracticeSummary,
) {
    let spec_json = serde_json::json!({ "score_title": summary.score_title }).to_string();
    let entry = brain::store::ExerciseLogEntry {
        source: "score_practice".to_owned(),
        label: summary.score_title.clone(),
        spec_json,
        seed: 0,
        difficulty: 0,
        tonic: 0,
        accuracy: Some(f64::from(summary.accuracy_pct) / 100.0),
    };
    if let Err(e) = store.log_exercise(&entry) {
        tracing::warn!(error = %e, "could not log score practice to the exercise log");
    }
}

/// Append to the exercise log — best-effort, NEVER blocks the practice loop.
fn log_exercise_best_effort(store: &SessionStore, o: ExerciseOutcome<'_>) {
    let Ok(spec_json) = serde_json::to_string(o.spec) else {
        return;
    };
    let entry = brain::store::ExerciseLogEntry {
        source: o.source.to_owned(),
        label: o.label.to_owned(),
        spec_json,
        seed: o.seed,
        difficulty: o.difficulty,
        tonic: o.tonic,
        accuracy: o.accuracy,
    };
    if let Err(e) = store.log_exercise(&entry) {
        tracing::warn!(error = %e, "exercise log append failed (continuing)");
    }
}

// ---------------------------------------------------------------------------
// #449 T1: the practice_events writers. Best-effort, command-layer only
// (never the audio thread), one clock (seconds from session start). LOCAL
// ONLY: nothing here syncs until T2's enrollment opt-in + ConnectionsPrivacy
// rows land — see docs/specs/449-t1-local-telemetry.md.
// ---------------------------------------------------------------------------

/// The store write shared by every emitter below. One `tracing::warn` on
/// failure, nothing else — a telemetry failure must never break practice.
///
/// NOTE for callers: this takes the `session_store` lock, so it must be
/// called with that lock NOT held (std mutexes are not reentrant).
fn write_practice_event(
    state: &AppState,
    session_id: brain::session::SessionId,
    at_secs: f64,
    kind: &str,
    params: &serde_json::Value,
) {
    let store = state.session_store.lock_or_recover();
    if let Err(e) =
        store.log_practice_event(&session_id.as_str(), at_secs, kind, &params.to_string())
    {
        tracing::warn!(error = %e, kind, "practice-event append failed (continuing)");
    }
}

/// Append one tool-usage event to the journal — best-effort, NEVER blocks or
/// errors the practice loop (the `log_exercise_best_effort` posture, returns
/// `()` by construction). **No active session → no row, calmly** — tool use
/// outside a session (library browsing, opener preview) is not practice
/// evidence and must not fabricate any.
fn log_practice_event_best_effort(state: &AppState, kind: &str, params: serde_json::Value) {
    let (session_id, at_secs) = {
        let guard = state.telemetry.lock_or_recover();
        let Some(t) = guard.as_ref() else { return };
        (t.session_id, t.at_secs())
    };
    write_practice_event(state, session_id, at_secs, kind, &params);
}

/// `pocket_start`: journal the click's birth and reset the tempo-coalescing
/// baseline to its starting tempo. `mode` is `"anchor"` by backend contract —
/// every click starts as the strict Anchor; Follow/Handoff retimes arrive
/// later as `set_pocket_tempo` pushes (and mode *changes* as `pocket_mode`
/// events via `set_pocket_mode`).
fn note_pocket_started(state: &AppState, bpm: f64, count_in: bool) {
    let (session_id, at_secs) = {
        let mut guard = state.telemetry.lock_or_recover();
        let Some(t) = guard.as_mut() else { return };
        let at = t.at_secs();
        t.pocket_bpm = Some(bpm);
        t.tempo_last_logged_bpm = Some(bpm);
        t.tempo_last_logged_at_secs = at;
        (t.session_id, at)
    };
    write_practice_event(
        state,
        session_id,
        at_secs,
        "pocket_start",
        &serde_json::json!({ "bpm": bpm, "mode": "anchor", "count_in": count_in }),
    );
}

/// `pocket_tempo`, coalesced: the last-known effective tempo is tracked on
/// EVERY push (so `pocket_stop` reports the truth), but a journal row is
/// appended only when [`tempo_log_due`] says the change is big enough
/// (≥ 5 BPM from the last row) and settled enough (≥ 5 s since it). A
/// follow-mode stream therefore logs few rows — the doc's "coalesce: ≤ 1 row
/// per settled value".
fn note_pocket_tempo(state: &AppState, bpm: f64) {
    let due = {
        let mut guard = state.telemetry.lock_or_recover();
        let Some(t) = guard.as_mut() else { return };
        let at = t.at_secs();
        t.pocket_bpm = Some(bpm);
        if tempo_log_due(
            t.tempo_last_logged_bpm,
            t.tempo_last_logged_at_secs,
            bpm,
            at,
        ) {
            t.tempo_last_logged_bpm = Some(bpm);
            t.tempo_last_logged_at_secs = at;
            Some((t.session_id, at))
        } else {
            None
        }
    };
    if let Some((session_id, at_secs)) = due {
        write_practice_event(
            state,
            session_id,
            at_secs,
            "pocket_tempo",
            &serde_json::json!({ "bpm": bpm }),
        );
    }
}

/// `pocket_stop` with the final effective tempo (the last clamped value the
/// render thread was fed — start tempo if nothing ever retimed it). Callers
/// invoke this only when `teardown_pocket` reported a click was actually
/// running, so a no-op teardown can never fabricate a stop.
fn note_pocket_stopped(state: &AppState) {
    let (session_id, at_secs, bpm) = {
        let guard = state.telemetry.lock_or_recover();
        let Some(t) = guard.as_ref() else { return };
        (t.session_id, t.at_secs(), t.pocket_bpm)
    };
    let params = match bpm {
        Some(b) => serde_json::json!({ "bpm": b }),
        None => serde_json::json!({}),
    };
    write_practice_event(state, session_id, at_secs, "pocket_stop", &params);
}

/// The active explore key signature — the DRAWN (tonic's) signature. The
/// material derivation lives in `brain::coach::explore_material` (#471-2 F3)
/// so the edit engine and the per-segment staff spelling share it: the
/// signature follows the FIGURE the row actually deals (#335) — a scale
/// explore engraves in its scale's family; a chord explore (the jam bridge,
/// #349 T4a) in its chord family; a lifted progression in its ANCHOR chord's
/// family (#349 T3c).
fn explore_key(explore: &ExploreState) -> brain::score::KeySignature {
    brain::coach::key_signature_for(
        explore.tonic,
        &brain::coach::explore_material(&explore.spec),
    )
}

/// Start (or restart) a free-play exploration from the live key. Reads the
/// Learner Model for the difficulty; the variation renders on the free-play
/// surface with its mutation chips.
pub fn start_explore_variation_impl(
    state: &AppState,
    tonic: u8,
    mode: &str,
    seed: u64,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) = start_explore_windowed(tonic, mode, &model, seed, window);
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "explore",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

#[tauri::command]
pub async fn start_explore_variation(
    state: State<'_, AppState>,
    tonic: u8,
    mode: String,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    let window = session_fold_window(&state).await;
    start_explore_variation_impl(&state, tonic, &mode, seed, window)
}

/// Apply a tapped chip's delta to the in-flight exploration and return the
/// next rep. Calm error when nothing is being explored.
pub fn apply_variation_delta_impl(
    state: &AppState,
    delta: VariationDelta,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    let (next, seq) = apply_explore_delta_windowed(current, &delta, window);
    let dto = explore_dto(&next, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "explore_chip",
                label: &dto.label,
                spec: &next.spec,
                seed: next.seed,
                difficulty: next.difficulty,
                tonic: next.tonic,
                accuracy: None,
            },
        );
    }
    *guard = Some(next);
    Ok(dto)
}

#[tauri::command]
pub async fn apply_variation_delta(
    state: State<'_, AppState>,
    delta: VariationDelta,
) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    apply_variation_delta_impl(&state, delta, window)
}

/// The Learner Model as a JSON blob for cloud sync (`null` on cold start).
/// Read-only: sync pushes the local truth up; it never writes back down (the
/// local model is authoritative — last-writer-wins upsert keyed on the user).
#[tauri::command]
pub fn get_learner_model_blob(
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?;
    match model {
        Some(m) => serde_json::to_value(&m)
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// The 12-key mastery wheel (#256): a pure snapshot over the Learner Model +
/// recent fingerprint history. Read-only; fetched on screen mount.
pub fn get_mastery_wheel_impl(state: &AppState) -> Result<brain::wheel::WheelView, CommandError> {
    let store = state.session_store.lock_or_recover();
    let model = store
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    // Recent sessions' fingerprints, oldest → newest (list_recent is
    // newest-first), for the trend halves. Missing/legacy recaps contribute
    // nothing — trends stay honest with fewer points.
    const TREND_SESSIONS: usize = 12;
    let mut fingerprints: Vec<brain::fingerprint::MusicalFingerprint> = store
        .list_recent(TREND_SESSIONS)?
        .into_iter()
        .filter_map(|summary| store.load_recap(summary.id).ok())
        .filter_map(|recap| recap.fingerprint)
        .collect();
    fingerprints.reverse();
    Ok(brain::wheel::build_wheel(&model, &fingerprints))
}

#[tauri::command]
pub fn get_mastery_wheel(state: State<'_, AppState>) -> Result<brain::wheel::WheelView, String> {
    get_mastery_wheel_impl(&state).map_err(|e| e.to_frontend())
}

/// The "your sound" mirror (#258) as the frontend renders it: the derived
/// profile (None below brain::mirror::MIN_SESSIONS) plus how many measured
/// sessions exist, for the "N of K" empty-state copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundMirrorDto {
    pub profile: Option<brain::mirror::SoundProfile>,
    pub sessions_seen: usize,
}

/// Derive the sound mirror from stored fingerprints + taste, persist the
/// snapshot on the Learner Model (F2's reserved field), and return it.
pub fn get_sound_mirror_impl(state: &AppState, now: i64) -> Result<SoundMirrorDto, CommandError> {
    let store = state.session_store.lock_or_recover();
    // Recent measured sessions, oldest → newest (mirrors the wheel's trends).
    const MIRROR_SESSIONS: usize = 30;
    let mut fingerprints: Vec<brain::fingerprint::MusicalFingerprint> = store
        .list_recent(MIRROR_SESSIONS)?
        .into_iter()
        .filter_map(|summary| store.load_recap(summary.id).ok())
        .filter_map(|recap| recap.fingerprint)
        .collect();
    fingerprints.reverse();
    let taste = store
        .get_taste_profile(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    let profile = brain::mirror::derive_sound_profile(&fingerprints, &taste, now);
    if let Some(ref p) = profile {
        // Persist the snapshot on the blob so it syncs with the model.
        let mut model = store
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
            .unwrap_or_default();
        model.sound_profile = Some(p.clone());
        store.upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &model)?;
    }
    Ok(SoundMirrorDto {
        profile,
        sessions_seen: fingerprints.len(),
    })
}

#[tauri::command]
pub fn get_sound_mirror(state: State<'_, AppState>) -> Result<SoundMirrorDto, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    get_sound_mirror_impl(&state, now).map_err(|e| e.to_frontend())
}

/// #285 — the flagship RV loop: lift the player's most recent worth-lifting
/// phrase as a cell and row it through the keys at their difficulty. Errors
/// calmly when nothing liftable has been played yet.
pub fn explore_last_phrase_impl(
    state: &AppState,
    seed: u64,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let lifted = {
        let phrases = state.phrase_buffer.lock_or_recover();
        // Most recent phrase that yields a real cell wins.
        phrases.iter().rev().find_map(|p| {
            brain::coach::lift_cell_from_pitch_track(
                &p.pitch_stats.pitches,
                brain::coach::LIFT_MIN_RUN,
            )
        })
    };
    let (cell, first_midi) =
        lifted.ok_or_else(|| "play a little phrase first — then I can lift it".to_owned())?;
    let (explore, seq) = brain::coach::start_explore_cell_windowed(
        cell,
        first_midi % 12,
        &model,
        seed,
        brain::coach::DirectionMode::Forward,
        window,
    );
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "lift",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

/// #337 S5 — the RV bridge: lift a MEASURE of a stored score as a cell and
/// row it through 12 keys via the same explore engine as a lifted lick.
/// Refuses calmly on empty/rest-only measures or ones past the lift cap.
pub fn explore_measure_impl(
    state: &AppState,
    score_id: &str,
    measure_number: usize,
    seed: u64,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let id: ScoreId = score_id
        .parse()
        .map_err(|_| "that score isn't in the library anymore".to_owned())?;
    let (music_xml, part_index) = {
        let store = state.score_store.lock_or_recover();
        let entry = store
            .get(id)
            .map_err(|_| "that score isn't in the library anymore".to_owned())?;
        (entry.music_xml, entry.part_index)
    };
    let model = brain::score::musicxml::parse_musicxml_str_part(&music_xml, part_index)
        .map_err(|e| e.to_string())?;
    let measure = model
        .measures
        .iter()
        .find(|m| m.number == measure_number)
        .ok_or_else(|| format!("measure {measure_number} isn't in this piece"))?;
    let midis: Vec<u8> = measure
        .notes
        .iter()
        .filter(|n| !n.is_rest)
        .map(|n| n.midi_number)
        .collect();
    if midis.is_empty() {
        return Err(format!(
            "measure {measure_number} is all rests — nothing to row"
        ));
    }
    if midis.len() > brain::coach::LIFT_MAX_NOTES {
        return Err(format!(
            "measure {measure_number} is too busy to row ({} notes; {} is the ceiling)",
            midis.len(),
            brain::coach::LIFT_MAX_NOTES
        ));
    }
    let first = midis[0];
    // A cell is semitone offsets from its first note — same wire shape as a
    // lifted lick. Wide leaps octave-FOLD into range (matching
    // lift_cell_from_pitch_track's documented semantics): the pitch class is
    // the music; a clamp would quietly reshape it (S5 review finding 4).
    let cell: Vec<i8> = midis
        .iter()
        .map(|&m| {
            let mut off = i16::from(m) - i16::from(first);
            while off > brain::coach::MAX_CELL_OFFSET {
                off -= 12;
            }
            while off < -brain::coach::MAX_CELL_OFFSET {
                off += 12;
            }
            off as i8
        })
        .collect();
    let learner = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) = brain::coach::start_explore_cell_windowed(
        cell,
        first % 12,
        &learner,
        seed,
        brain::coach::DirectionMode::Forward,
        window,
    );
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "measure_bridge",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

/// #419 S1 — Session Starters: compile the Openers panel's items into one
/// composite cell and run it through the SAME explore engine as a lifted
/// lick. `commit: false` is the live preview (no state, no exercise log);
/// `commit: true` is Begin. The seed derives deterministically from the
/// items so the preview IS the exercise — what you saw is what you play.
pub fn opener_impl(
    state: &AppState,
    items: &[brain::starter::StarterItem],
    tonic: Option<u8>,
    direction: Option<&str>,
    commit: bool,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let cell = brain::starter::composite_cell(items, brain::coach::LIFT_MAX_NOTES)
        .map_err(|e| e.to_string())?;
    // #419 S2b: row from the key the room is in when the frontend heard
    // one confidently; C otherwise. Folded defensively — the wire must
    // never panic on a wild value.
    let tonic = tonic.unwrap_or(0) % 12;
    let direction = match direction.unwrap_or("forward") {
        "forward" => brain::coach::DirectionMode::Forward,
        "reversed" => brain::coach::DirectionMode::Reversed,
        "varied" => brain::coach::DirectionMode::RandomPerRoot,
        other => {
            return Err(format!(
                "direction can be forward, reversed, or varied — not {other:?}"
            ))
        }
    };
    // Deterministic per-recipe seed (session-local determinism only — the
    // hash need not be stable across releases, just between the preview
    // and the Begin that follows it).
    let seed = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cell.hash(&mut h);
        h.finish()
    };
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    // Openers speak in abstract degrees, so the row starts from C and
    // travels the 12 keys from there (S1 simplification, noted in #419).
    let (explore, seq) =
        brain::coach::start_explore_cell_windowed(cell, tonic, &model, seed, direction, window);
    let dto = explore_dto(&explore, &seq);
    if commit {
        {
            let store = state.session_store.lock_or_recover();
            log_exercise_best_effort(
                &store,
                ExerciseOutcome {
                    source: "opener",
                    label: &dto.label,
                    spec: &explore.spec,
                    seed: explore.seed,
                    difficulty: explore.difficulty,
                    tonic: explore.tonic,
                    accuracy: None,
                },
            );
        }
        // #449 T1: Begin is a tool moment, journaled alongside the exercise
        // row (outside the store-lock scope above — the writer takes that
        // lock itself). The recipe NAME never crosses the IPC boundary (the
        // frontend loads a recipe into the builder, then Begins), so it's
        // null until that wire exists; additive params absorb it later.
        log_practice_event_best_effort(
            state,
            "opener_begin",
            serde_json::json!({ "recipe": null }),
        );
        *state.active_explore.lock_or_recover() = Some(explore);
    }
    Ok(dto)
}

/// #419 S1: live preview of the opener being built — pure, no session state.
#[tauri::command]
pub async fn preview_opener(
    state: State<'_, AppState>,
    items: Vec<brain::starter::StarterItem>,
    tonic: Option<u8>,
    direction: Option<String>,
) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    opener_impl(&state, &items, tonic, direction.as_deref(), false, window)
}

/// #419 S1: Begin — the built opener becomes the session's exploration.
#[tauri::command]
pub async fn begin_opener(
    state: State<'_, AppState>,
    items: Vec<brain::starter::StarterItem>,
    tonic: Option<u8>,
    direction: Option<String>,
) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    opener_impl(&state, &items, tonic, direction.as_deref(), true, window)
}

/// #419 S4: one saved opener recipe on the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RecipeDto {
    pub id: i64,
    pub name: String,
    pub items: Vec<brain::starter::StarterItem>,
    pub direction: String,
}

/// The three direction words the Openers wire speaks (matches
/// `opener_impl`'s parse — a recipe must never store a word that Begin
/// would later refuse).
fn validate_direction(direction: &str) -> Result<(), String> {
    match direction {
        "forward" | "reversed" | "varied" => Ok(()),
        other => Err(format!(
            "direction can be forward, reversed, or varied — not {other:?}"
        )),
    }
}

/// #419 S4: keep the current builder as a named recipe.
pub fn save_opener_recipe_impl(
    state: &AppState,
    name: &str,
    items: &[brain::starter::StarterItem],
    direction: &str,
) -> Result<RecipeDto, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("give the recipe a name first".into());
    }
    if items.is_empty() {
        return Err("an empty opener isn't worth keeping — add something first".into());
    }
    validate_direction(direction)?;
    // Prove the items still compile into a playable cell BEFORE keeping
    // them — a saved recipe that can't preview is a door that won't open.
    brain::starter::composite_cell(items, brain::coach::LIFT_MAX_NOTES)
        .map_err(|e| e.to_string())?;
    let items_json = serde_json::to_string(items).map_err(|e| e.to_string())?;
    let id = state
        .session_store
        .lock_or_recover()
        .save_recipe(name, &items_json, direction)
        .map_err(|e| e.to_string())?;
    Ok(RecipeDto {
        id,
        name: name.into(),
        items: items.to_vec(),
        direction: direction.into(),
    })
}

/// #419 S4: saved recipes, most-recent-first. A row whose items no
/// longer parse is skipped calmly (the My Patterns garbage-tolerance
/// rule) — this list never errors the panel.
pub fn list_opener_recipes_impl(state: &AppState) -> Vec<RecipeDto> {
    let rows = match state.session_store.lock_or_recover().list_recipes() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "recipes unreadable; list empty");
            return Vec::new();
        }
    };
    rows.into_iter()
        .filter_map(|row| {
            let items =
                serde_json::from_str::<Vec<brain::starter::StarterItem>>(&row.items_json).ok()?;
            Some(RecipeDto {
                id: row.id,
                name: row.name,
                items,
                direction: row.direction,
            })
        })
        .collect()
}

/// #419 S4: yesterday's opener, as the chip announces it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LastOpenerDto {
    pub label: String,
    pub tonic: u8,
}

/// The stored-seed law (S4 spec §2): recall reads the newest begun
/// opener's SEED, CELL, TONIC, and DIRECTION straight from the log row —
/// never a recomputed hash, which is only promised stable within a
/// session. A row that no longer parses to a cell offers nothing.
fn last_opener_row(
    state: &AppState,
) -> Option<(brain::store::ExerciseLogEntry, brain::coach::VariationSpec)> {
    let row = state
        .session_store
        .lock_or_recover()
        .latest_exercise_for_source("opener")
        .ok()
        .flatten()?;
    let spec = serde_json::from_str::<brain::coach::VariationSpec>(&row.spec_json).ok()?;
    // The cell gate: a spec without one isn't an opener artifact.
    spec.cell.as_ref().filter(|c| c.len() >= 2)?;
    Some((row, spec))
}

/// #419 S4: what the recall chip shows — or None (no opener begun yet,
/// or the last one no longer parses): honest absence, not a guess.
pub fn recall_last_opener_impl(state: &AppState) -> Option<LastOpenerDto> {
    let (row, _) = last_opener_row(state)?;
    Some(LastOpenerDto {
        label: row.label.clone(),
        tonic: row.tonic,
    })
}

/// #419 S4: replay yesterday's opener EXACTLY — the stored spec (roots,
/// rhythm, cell, direction) re-rendered under the stored seed; today's
/// learner model touches nothing. Commits like `begin_opener` (fresh
/// log row carrying the same seed and spec, so recall chains).
pub fn begin_opener_recall_impl(
    state: &AppState,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let (row, spec) = last_opener_row(state)
        .ok_or_else(|| "no opener to recall yet — begin one first".to_string())?;
    // Review MF2: replay the STORED spec wholesale — roots, rhythm, and
    // all. Rebuilding through start_explore_cell would let today's
    // learner difficulty retune yesterday's opener (tempo, root count)
    // under a chip that promises "exactly". The fold window is the SESSION's
    // (#471-4): the stored artifact stays instrument-agnostic, so the same
    // instrument replays bit-identically and a different one re-registers.
    let (explore, seq) = brain::coach::resume_explore_spec_windowed(
        spec,
        row.tonic % 12,
        row.difficulty,
        row.seed,
        window,
    );
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "opener",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    // #449 T1: recall commits like Begin, so it journals like Begin (a
    // recalled opener has no recipe name either — it replays the last row).
    log_practice_event_best_effort(state, "opener_begin", serde_json::json!({ "recipe": null }));
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

/// #419 S4: keep the current builder as a named recipe.
#[tauri::command]
pub fn save_opener_recipe(
    state: State<'_, AppState>,
    name: String,
    items: Vec<brain::starter::StarterItem>,
    direction: Option<String>,
) -> Result<RecipeDto, String> {
    save_opener_recipe_impl(
        &state,
        &name,
        &items,
        direction.as_deref().unwrap_or("forward"),
    )
}

/// #419 S4: the saved-recipes strip.
#[tauri::command]
pub fn list_opener_recipes(state: State<'_, AppState>) -> Vec<RecipeDto> {
    list_opener_recipes_impl(&state)
}

/// #419 S4: forget a saved recipe.
#[tauri::command]
pub fn delete_opener_recipe(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state
        .session_store
        .lock_or_recover()
        .delete_recipe(id)
        .map_err(|e| e.to_string())
}

/// #419 S4: what the "yesterday's opener" chip shows.
#[tauri::command]
pub fn recall_last_opener(state: State<'_, AppState>) -> Option<LastOpenerDto> {
    recall_last_opener_impl(&state)
}

/// #419 S4: replay yesterday's opener exactly (stored seed — spec §2).
#[tauri::command]
pub async fn begin_opener_recall(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    begin_opener_recall_impl(&state, window)
}

/// #337 S5: row one measure of a stored score through 12 keys.
#[tauri::command]
pub async fn explore_measure(
    state: State<'_, AppState>,
    score_id: String,
    measure_number: usize,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42);
    let window = session_fold_window(&state).await;
    explore_measure_impl(&state, &score_id, measure_number, seed, window)
}

/// #349 T4a — the jam lane's RV bridge: row a chord the room played
/// through 12 keys as stacked block cells. Same explore engine (and same
/// live view swap) as "work on my last lick".
pub fn explore_chord_impl(
    state: &AppState,
    root_pc: u8,
    quality: brain::theory::ChordQuality,
    seed: u64,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) =
        brain::coach::start_explore_chord_windowed(root_pc % 12, quality, &model, seed, window);
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "jam_bridge",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

#[tauri::command]
pub async fn explore_chord(
    state: State<'_, AppState>,
    root_pc: u8,
    quality: brain::theory::ChordQuality,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(7);
    let window = session_fold_window(&state).await;
    explore_chord_impl(&state, root_pc, quality, seed, window)
}

/// #349 T3c — "work on my last progression": lift the chart's trailing
/// chord sequence (consecutive duplicates collapsed, unresolved stretches
/// skipped) and row it through 12 keys as stacked cells. Same live view
/// swap as the lick lift; refuses calmly under two distinct chords.
pub fn explore_progression_impl(
    state: &AppState,
    seed: u64,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    /// The most recent chords worth rowing — a phrase of harmony, not a set.
    const MAX_PROGRESSION_CHORDS: usize = 4;
    let chords: Vec<(u8, brain::theory::ChordQuality)> = {
        let chart = state.chord_chart.lock_or_recover();
        let mut seq: Vec<(u8, brain::theory::ChordQuality)> = Vec::new();
        for e in chart.entries().iter().filter(|e| !e.unresolved) {
            if let (Some(pc), Some(q)) = (e.root_pc, e.quality) {
                if seq.last() != Some(&(pc, q)) {
                    seq.push((pc, q));
                }
            }
        }
        let skip = seq.len().saturating_sub(MAX_PROGRESSION_CHORDS);
        seq.split_off(skip)
    };
    if chords.len() < 2 {
        return Err("play a couple of chords first — then I can lift the progression".to_owned());
    }
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) =
        brain::coach::start_explore_progression_windowed(&chords, &model, seed, window);
    let dto = explore_dto(&explore, &seq);
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "progression_lift",
                label: &dto.label,
                spec: &explore.spec,
                seed: explore.seed,
                difficulty: explore.difficulty,
                tonic: explore.tonic,
                accuracy: None,
            },
        );
    }
    *state.active_explore.lock_or_recover() = Some(explore);
    Ok(dto)
}

#[tauri::command]
pub async fn explore_progression(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(11);
    let window = session_fold_window(&state).await;
    explore_progression_impl(&state, seed, window)
}

/// #349 T4a — the session's chord chart so far: the timed label sequence
/// the jam recap sketches. Read-only; cleared at the next session start.
#[tauri::command]
pub fn session_chord_chart(
    state: State<'_, AppState>,
) -> Result<Vec<brain::chord_chart::ChartEntry>, String> {
    Ok(state.chord_chart.lock_or_recover().entries().to_vec())
}

#[tauri::command]
pub async fn explore_last_phrase(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    let window = session_fold_window(&state).await;
    explore_last_phrase_impl(&state, seed, window)
}

/// Apply a semantic note edit (#292 slice 3) to the in-flight exploration —
/// the edit bakes the CELL, so it lands in every key; the row never
/// reshuffles under the player's hands.
pub fn edit_explore_note_impl(
    state: &AppState,
    index: usize,
    edit: brain::coach::NoteEdit,
    window: FoldWindow,
) -> Result<ExploreDto, String> {
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    // The gesture's key derivation lives with the edit engine now (#471-2
    // F3): it speaks each edited segment's own signature — the one the
    // staff draws for that bar.
    let (next, seq) = brain::coach::edit_explore_note_windowed(current, index, &edit, window)?;
    let dto = explore_dto(&next, &seq);
    *guard = Some(next);
    Ok(dto)
}

#[tauri::command]
pub async fn edit_explore_note(
    state: State<'_, AppState>,
    index: usize,
    edit: brain::coach::NoteEdit,
) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    edit_explore_note_impl(&state, index, edit, window)
}

/// Undo the most recent explore edit — restores the exact prior rep.
pub fn undo_explore_edit_impl(state: &AppState, window: FoldWindow) -> Result<ExploreDto, String> {
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    let (next, seq) = brain::coach::undo_explore_edit_windowed(current, window)?;
    let dto = explore_dto(&next, &seq);
    *guard = Some(next);
    Ok(dto)
}

#[tauri::command]
pub async fn undo_explore_edit(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let window = session_fold_window(&state).await;
    undo_explore_edit_impl(&state, window)
}

/// Stop exploring (nothing persisted).
#[tauri::command]
pub fn end_explore(state: State<'_, AppState>) -> Result<(), String> {
    *state.active_explore.lock_or_recover() = None;
    Ok(())
}

// ---------------------------------------------------------------------------
// Guided lesson (#254): start → play → submit per drill → recap.
// ---------------------------------------------------------------------------

/// The in-flight lesson state held on [`AppState`].
struct ActiveLesson {
    spec: LessonSpec,
    current: Drill,
    completed: Vec<(Drill, DrillScore)>,
    /// Index into the session phrase buffer where the current drill started —
    /// everything after it is "what the player played for this drill".
    phrase_mark: usize,
    /// Same mark into the session CHORD buffer (#349 T2b) — the slice a
    /// stacked drill grades from.
    chord_mark: usize,
}

/// One drill as the frontend renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrillDto {
    pub index: u8,
    pub drill_count: u8,
    pub kind: String,
    pub label: String,
    pub tempo_bpm: f64,
    pub difficulty: u8,
    /// MusicXML for ScoreView (adapted from the generated sequence).
    pub music_xml: String,
    pub target_len: usize,
    /// The drill's roots as pitch classes (0–11), in PLAY order — RV's shuffled
    /// key sequence, rendered by the UI as the brand's colored cells (#278).
    pub root_pitch_classes: Vec<u8>,
    /// Display names for those roots, SPELLED PER THE DRILL'S KEY SIGNATURE
    /// (#335): a flat signature names flats (Db, not C#) so the cells can
    /// never contradict the engraved notation. Same order/length as
    /// `root_pitch_classes`.
    pub root_names: Vec<String>,
}

/// The grade of a just-submitted drill, trimmed for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrillScoreDto {
    pub accuracy: f32,
    pub pitch_accuracy: f32,
    pub timing_accuracy: f32,
    pub correct: usize,
    pub total: usize,
}

impl From<&DrillScore> for DrillScoreDto {
    fn from(s: &DrillScore) -> Self {
        Self {
            accuracy: s.accuracy,
            pitch_accuracy: s.pitch_accuracy,
            timing_accuracy: s.timing_accuracy,
            correct: s.per_note.iter().filter(|g| g.correct).count(),
            total: s.per_note.len(),
        }
    }
}

/// One step of the lesson state machine: the score for what was just played
/// (absent on start), then either the next drill or the final recap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonStepDto {
    pub seed: u64,
    pub score: Option<DrillScoreDto>,
    pub drill: Option<DrillDto>,
    pub recap: Option<LessonRecap>,
}

fn drill_dto(drill: &Drill, drill_count: u8, grand_staff: bool) -> DrillDto {
    let key = brain::coach::key_signature_for(drill.tonic, &drill.mode);
    let fifths = key.fifths;
    let mut model = sequence_to_score_model(&drill.sequence, &drill.sequence.label, key);
    // #417-3: a keyboard lesson engraves on a grand staff — the emitter
    // splits at middle C. All staff logic lives in Rust; OSMD just renders.
    model.grand_staff = grand_staff;
    DrillDto {
        index: drill.index,
        drill_count,
        kind: match drill.kind {
            brain::coach::DrillKind::WarmupScale => "warmup_scale",
            brain::coach::DrillKind::ArpeggioEnclosure => "arpeggio_enclosure",
            brain::coach::DrillKind::IntervalDrill => "interval_drill",
            brain::coach::DrillKind::RunThrough => "run_through",
        }
        .to_owned(),
        label: drill.sequence.label.clone(),
        tempo_bpm: drill.sequence.tempo_bpm,
        difficulty: drill.difficulty,
        music_xml: brain::score::emit::score_model_to_musicxml(&model),
        // A stacked drill's unit of work is the CELL (one chord), not the
        // tone — 12 chords, not 48 notes (#349 T2b).
        target_len: if drill.sequence.chord_targets.is_empty() {
            drill.sequence.target_midi.len()
        } else {
            drill.sequence.chord_targets.len()
        },
        root_pitch_classes: drill.sequence.root_order.iter().map(|&r| r % 12).collect(),
        root_names: drill
            .sequence
            .root_order
            .iter()
            .map(|&r| brain::coach::tonic_display_name(r % 12, fifths).to_owned())
            .collect(),
    }
}

/// Minimum consecutive pitch samples (~100 Hz stream) for the drill grader to
/// count a note — rejects single-sample flicker without eating fast notes.
const DRILL_MIN_PITCH_RUN: usize = 3;

/// Start a guided lesson: read the Learner Model for the starting difficulty,
/// build drill 0, and mark the phrase buffer. Pure logic lives in
/// `brain::coach`; this wires state + persistence.
pub fn start_lesson_impl(
    state: &AppState,
    seed: u64,
    polyphonic: bool,
    grand_staff: bool,
    window: FoldWindow,
) -> Result<LessonStepDto, CommandError> {
    if state.active_lesson.lock_or_recover().is_some() {
        return Err(CommandError::LessonActive);
    }
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
        .unwrap_or_default();
    let spec = LessonSpec {
        seed,
        // Clamped once here; every later consumer reads it verbatim.
        drill_count: 4u8.clamp(3, 4),
        start_difficulty: model.difficulty.min(brain::learner::MAX_DIFFICULTY),
        polyphonic,
        grand_staff,
    };
    let first = build_first_windowed(&spec, &model, window);
    let dto = drill_dto(&first, spec.drill_count, spec.grand_staff);
    let phrase_mark = state.phrase_buffer.lock_or_recover().len();
    let chord_mark = state.chord_buffer.lock_or_recover().len();
    *state.active_lesson.lock_or_recover() = Some(ActiveLesson {
        spec,
        current: first,
        completed: Vec::new(),
        phrase_mark,
        chord_mark,
    });
    Ok(LessonStepDto {
        seed,
        score: None,
        drill: Some(dto),
        recap: None,
    })
}

/// Grade the drill the player just performed (everything in the phrase buffer
/// since the drill started) and step the lesson: next drill, or finish +
/// persist the Learner Model and return the recap.
pub fn submit_drill_impl(
    state: &AppState,
    now_epoch_secs: i64,
    window: FoldWindow,
) -> Result<LessonStepDto, CommandError> {
    let mut lesson_guard = state.active_lesson.lock_or_recover();
    let lesson = lesson_guard.as_mut().ok_or(CommandError::NotActive)?;

    // What was played for this drill: the pitch track + onset count across the
    // phrases completed since the drill began.
    let (pitches, onsets) = {
        let buf = state.phrase_buffer.lock_or_recover();
        let slice = &buf[lesson.phrase_mark.min(buf.len())..];
        let pitches: Vec<f64> = slice
            .iter()
            .flat_map(|p| p.pitch_stats.pitches.iter().copied())
            .collect();
        let onsets: usize = slice.iter().map(|p| p.onsets_secs.len()).sum();
        (pitches, onsets)
    };
    let played = played_notes_from_pitch_track(&pitches, DRILL_MIN_PITCH_RUN);
    // #349 T2b: a STACKED drill is judged by the T1 chord engine against
    // the stable readings heard since the drill began — the monophonic
    // pitch track can't grade a simultaneity. Melodic drills keep the
    // existing path, bit-identical.
    let chord_targets = &lesson.current.sequence.chord_targets;
    let score = if chord_targets.is_empty() {
        // Eager-tap guard: phrases only close after a beat of silence, so a
        // tap the instant the last note ends can see an empty window — and
        // a take of sub-threshold pitch flickers collapses to zero NOTES
        // even with samples present. Grading either as 0% would be a lie
        // about the player — return a calm "not yet" instead. (Deliberately
        // failing a drill still works: play wrong notes. Unpitched noise
        // WITH onsets still grades — the app heard something, it just
        // wasn't the material.)
        if played.is_empty() && onsets == 0 {
            return Err(CommandError::DrillNotHeard);
        }
        score_drill(&lesson.current.sequence.target_midi, &played, onsets)
    } else {
        let heard: Vec<brain::chord_judge::HeardChord> = {
            let buf = state.chord_buffer.lock_or_recover();
            buf.heard()[lesson.chord_mark.min(buf.len())..].to_vec()
        };
        // Same calm "not yet" honesty: nothing heard at all → don't grade
        // silence as failure. Onsets WITHOUT a nameable chord still grade
        // (the app heard playing; it wasn't the asked-for chords).
        if heard.is_empty() && onsets == 0 {
            return Err(CommandError::DrillNotHeard);
        }
        brain::chord_judge::score_chord_drill(chord_targets, &heard)
    };
    let score_dto = DrillScoreDto::from(&score);
    let seed = lesson.spec.seed;
    // Exercise log (#252 self-improvement): the graded outcome for what the
    // engine dealt — the evidence "which exercises are good" feeds on.
    {
        let store = state.session_store.lock_or_recover();
        log_exercise_best_effort(
            &store,
            ExerciseOutcome {
                source: "lesson",
                label: &lesson.current.sequence.label,
                spec: &lesson.current.spec,
                seed,
                difficulty: lesson.current.difficulty,
                tonic: lesson.current.tonic,
                accuracy: Some(f64::from(score.accuracy)),
            },
        );
    }

    match advance_windowed(&lesson.current, &score, &lesson.spec, window) {
        Some(next) => {
            let dto = drill_dto(&next, lesson.spec.drill_count, lesson.spec.grand_staff);
            let completed = std::mem::replace(&mut lesson.current, next);
            lesson.completed.push((completed, score));
            lesson.phrase_mark = state.phrase_buffer.lock_or_recover().len();
            lesson.chord_mark = state.chord_buffer.lock_or_recover().len();
            Ok(LessonStepDto {
                seed,
                score: Some(score_dto),
                drill: Some(dto),
                recap: None,
            })
        }
        None => {
            // Persist FIRST; the lesson state is only cleared once the write
            // succeeds, so a store error leaves everything intact for a retry
            // instead of destroying four drills of results.
            let mut drills = lesson.completed.clone();
            drills.push((lesson.current.clone(), score));
            {
                let store = state.session_store.lock_or_recover();
                let model = store
                    .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)?
                    .unwrap_or_default();
                let (next_model, recap) = finish_lesson(&model, &drills, now_epoch_secs);
                store.upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &next_model)?;
                *lesson_guard = None;
                Ok(LessonStepDto {
                    seed,
                    score: Some(score_dto),
                    drill: None,
                    recap: Some(recap),
                })
            }
        }
    }
}

/// End the lesson early: per spec #254 §6 it finalizes with only the drills
/// that were actually completed and scored — a kid who nails 3 of 4 keeps the
/// credit. The unfinished current drill is dropped. With zero completed drills
/// nothing is persisted. Persistence at teardown is best-effort: a store error
/// is logged, never surfaced as a crash.
pub fn end_lesson_impl(state: &AppState, now_epoch_secs: i64) {
    let taken = state.active_lesson.lock_or_recover().take();
    let Some(lesson) = taken else { return };
    if lesson.completed.is_empty() {
        return;
    }
    let store = state.session_store.lock_or_recover();
    let persisted = store
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map(Option::unwrap_or_default)
        .and_then(|model| {
            let (next_model, _recap) = finish_lesson(&model, &lesson.completed, now_epoch_secs);
            store.upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &next_model)
        });
    if let Err(e) = persisted {
        tracing::warn!(error = %e, "could not persist the partial lesson at teardown");
    }
}

/// Start a guided lesson. `seed` optional — absent, one is derived from the
/// clock and echoed back so any lesson can be replayed exactly.
#[tauri::command]
pub async fn start_lesson(
    state: State<'_, AppState>,
    seed: Option<u64>,
) -> Result<LessonStepDto, String> {
    let seed = seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(1)
    });
    // #349 T2b: a lesson on a polyphonic instrument (piano, guitar) deals
    // the chord drill as block chords. Resolved from the LIVE session's
    // instrument; no session or unknown instrument → melodic ladder.
    // #417-3: keyboard-family sessions also engrave lessons on a grand
    // staff. Distinct from `polyphonic` (see LessonSpec) — resolution
    // lives in `lesson_instrument_traits` so it is testable (review MF1).
    let (polyphonic, grand_staff) = match state.active_session_instrument().await {
        Some(name) => lesson_instrument_traits(&state, &name),
        None => (false, false),
    };
    let window = session_fold_window(&state).await;
    start_lesson_impl(&state, seed, polyphonic, grand_staff, window).map_err(|e| e.to_frontend())
}

/// #417-3: how the session instrument shapes its lesson. Polyphonic dealing
/// and grand-staff engraving are SEPARATE facts — a guitar (Struck/Plucked,
/// family Strings) deals block chords but engraves on ONE staff; only the
/// keyboard family earns the grand staff. Unknown instrument → melodic,
/// single staff.
fn lesson_instrument_traits(state: &AppState, name: &str) -> (bool, bool) {
    state
        .instruments
        .iter()
        .find(|i| i.name == name)
        .map_or((false, false), |i| (i.polyphonic, i.family == "Keyboard"))
}

/// Grade the just-played drill and step the lesson.
#[tauri::command]
pub async fn submit_drill(state: State<'_, AppState>) -> Result<LessonStepDto, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let window = session_fold_window(&state).await;
    submit_drill_impl(&state, now, window).map_err(|e| e.to_frontend())
}

/// Abandon the in-flight lesson.
#[tauri::command]
pub fn end_lesson(state: State<'_, AppState>) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    end_lesson_impl(&state, now);
    Ok(())
}

/// Return the instrument catalog for the selector grid.
#[tauri::command]
pub fn list_instruments(state: State<'_, AppState>) -> Result<Vec<InstrumentInfo>, String> {
    list_instruments_impl(state.inner())
}

/// Check app capabilities (coaching availability, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCapabilities {
    pub coaching_available: bool,
    /// On-disk persistence fell back to in-memory at startup (e.g. corrupt
    /// data dir, sandbox, or full disk). The practice loop still works, but
    /// this session's history and scores won't survive a restart. Surfaced so
    /// the UI can warn the musician calmly instead of silently dropping their
    /// history (#137).
    pub storage_degraded: bool,
}

#[tauri::command]
pub fn get_app_capabilities(state: State<'_, AppState>) -> Result<AppCapabilities, String> {
    Ok(AppCapabilities {
        coaching_available: state.coaching_available(),
        storage_degraded: state.persistence_degraded(),
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

/// #449 T2: read ONE closed session shaped for the dashboard sync
/// projection (P1 session fact + P2 thin phrases + P4 tool events).
/// Read-only; the frontend `syncDashboard` is the only intended caller and
/// only ever calls it behind the signed-in + cloud-sync + dashboard-sync
/// gates (`connectionsStore`).
#[tauri::command]
pub fn get_session_projection(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionProjectionDto, String> {
    get_session_projection_impl(state.inner(), session_id).map_err(|e| e.to_frontend())
}

/// #449 T2: read exercise-log rows past `after_id`, shaped for the P3
/// projection — `spec_json`/`seed` structurally absent (see
/// [`brain::store::ExerciseFactRow`]).
#[tauri::command]
pub fn list_exercise_facts(
    state: State<'_, AppState>,
    after_id: i64,
) -> Result<Vec<ExerciseFactRow>, String> {
    list_exercise_facts_impl(state.inner(), after_id).map_err(|e| e.to_frontend())
}

/// Get the student's locally-stored taste profile (genres, artists, goals,
/// experience). Returns an empty default when nothing has been captured yet, so
/// the onboarding wizard can detect cold start. Local-first: no sign-in needed.
#[tauri::command]
pub fn get_taste_profile(state: State<'_, AppState>) -> Result<TasteProfile, String> {
    get_taste_profile_impl(state.inner()).map_err(|e| e.to_frontend())
}

/// Upsert the student's taste profile locally. Called by the onboarding wizard
/// and any later edit. Persistence only — relevance/coaching consumption of the
/// profile is owned by a separate layer.
#[tauri::command]
pub fn set_taste_profile(state: State<'_, AppState>, profile: TasteProfile) -> Result<(), String> {
    set_taste_profile_impl(state.inner(), profile).map_err(|e| e.to_frontend())
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
    let entry = state.import_raw_score(
        title,
        composer,
        source_filename,
        music_xml,
        part_index,
        duration_measures,
    )?;
    Ok(entry.into())
}

/// Extract a file's name stem (no directory, no extension) for use as a
/// fallback score title. Returns `None` for an empty stem so callers can
/// pick their own default rather than store a blank title.
fn filename_stem(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// Import a MIDI file into the score library.
///
/// Parses the MIDI, converts it to canonical MusicXML, and stores it — the
/// score then behaves like any other library entry (render, cursor-follow).
/// `bytes` is the raw file content read on the frontend; `source_filename`
/// is used for display and as a title fallback when the MIDI is unnamed.
#[tauri::command]
pub fn import_midi_file(
    state: State<'_, AppState>,
    source_filename: String,
    bytes: Vec<u8>,
    track_index: Option<usize>,
) -> Result<ScoreLibraryEntryDto, String> {
    state
        .import_midi_track(source_filename, bytes, track_index)
        .map(Into::into)
}

/// One playable track of a multi-part MIDI file — the picker's choices.
/// Mirrors `brain::score::midi::MidiPartInfo`.
#[derive(Debug, Clone, Serialize)]
pub struct MidiPartDto {
    pub track_index: usize,
    pub name: String,
    pub note_count: usize,
}

/// List the playable tracks of a MIDI file so the frontend can ask which
/// part to practice (#337 S1) — the MIDI twin of `list_score_parts`.
/// Conductor (meta-only) and percussion tracks are omitted.
#[tauri::command]
pub fn list_midi_parts(bytes: Vec<u8>) -> Result<Vec<MidiPartDto>, String> {
    brain::score::midi::list_midi_parts(&bytes)
        .map(|parts| {
            parts
                .into_iter()
                .map(|p| MidiPartDto {
                    track_index: p.track_index,
                    name: p.name,
                    note_count: p.note_count,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// Decode raw file bytes as a UTF-8 MusicXML string.
///
/// Uncompressed MusicXML (`.musicxml` / `.xml`) is plain text. Compressed
/// `.mxl` is a ZIP container, so it won't decode here — we surface a clear,
/// actionable message rather than a raw UTF-8 error, and point the user at the
/// uncompressed export their notation app can produce. (Adding a zip dep to
/// read `.mxl` directly is a deliberate later call, not silent scope creep.)
fn decode_musicxml_bytes(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| {
        "This doesn't look like uncompressed MusicXML. If it's a .mxl file, \
         re-export it as uncompressed .musicxml (or .xml) and try again."
            .to_string()
    })
}

/// List the instrument parts in a MusicXML file, in score order.
///
/// Lets the UI ask "which part do you want to read and practice?" before
/// import. A single-part score returns a one-element vec (the UI can skip the
/// picker and import part 0 directly). `bytes` is the raw file read on the
/// frontend. Read-only: nothing is stored.
#[tauri::command]
pub fn list_score_parts(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    let xml = decode_musicxml_bytes(bytes)?;
    brain::score::musicxml::list_parts(&xml).map_err(|e| e.to_string())
}

/// Import a MusicXML file into the score library, reading the chosen `part`.
///
/// The backend parses the MusicXML (metadata + the selected part) so the
/// frontend never carries score-parsing logic. `part_index` is the choice the
/// user made against [`list_score_parts`]; for a single-part score the UI
/// passes `0`. `bytes` is the raw file content read on the frontend.
#[tauri::command]
pub fn import_musicxml_file(
    state: State<'_, AppState>,
    source_filename: String,
    bytes: Vec<u8>,
    part_index: usize,
) -> Result<ScoreLibraryEntryDto, String> {
    let xml = decode_musicxml_bytes(bytes)?;
    state
        .import_musicxml(source_filename, xml, part_index)
        .map(Into::into)
}

/// Whether the PDF→sheet-music (OMR) beta is enabled, read from the
/// environment. Off unless `AMC_ENABLE_PDF_OMR` is set — the feature is
/// experimental and must never be advertised by a default build.
fn pdf_omr_enabled_from_env() -> bool {
    std::env::var_os("AMC_ENABLE_PDF_OMR").is_some()
}

/// Extract a lowercase file extension (no dot) for the audio decode hint.
fn filename_ext(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
}

/// `import-progress` event payload (audio import only).
#[derive(Clone, Serialize)]
struct ImportProgressPayload {
    stage: &'static str,
    pct: u8,
}

/// IPC result of an audio import: the new library entry plus a calm quality
/// signal. We surface approximate-ness; we never invent an accuracy score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedAudioDto {
    pub entry: ScoreLibraryEntryDto,
    pub note_count: usize,
    pub mean_confidence: f32,
    pub polyphony: f32,
    /// Input looks polyphonic — basic-pitch is monophonic-first.
    pub polyphonic: bool,
    /// Transcription confidence looks weak.
    pub low_confidence: bool,
    /// Notes the model itself was unsure of — the "M" of the honesty counts
    /// ("caught N notes, M uncertain") on the import result (#331).
    pub uncertain_count: usize,
    /// The honesty-gate verdict: `"mono"` | `"borderline"` | `"full_mix"`.
    /// Classified in `crates/transcribe` (`PolyphonyVerdict`), not here —
    /// the thresholds live with the model that produced the numbers.
    pub verdict: String,
}

impl ImportedAudioDto {
    fn new(entry: ScoreLibraryEntryDto, quality: transcribe::TranscriptionQuality) -> Self {
        Self {
            entry,
            note_count: quality.note_count,
            mean_confidence: quality.mean_confidence,
            polyphony: quality.polyphony,
            polyphonic: quality.verdict != transcribe::PolyphonyVerdict::Mono,
            low_confidence: quality.mean_confidence < transcribe::LOW_CONFIDENCE,
            uncertain_count: quality.uncertain_count,
            verdict: match quality.verdict {
                transcribe::PolyphonyVerdict::Mono => "mono",
                transcribe::PolyphonyVerdict::Borderline => "borderline",
                transcribe::PolyphonyVerdict::FullMix => "full_mix",
            }
            .to_string(),
        }
    }
}

/// The calm, non-fatal message shown when audio transcription can't run (the
/// ONNX Runtime is missing/unloadable, or it panicked). Score import (MusicXML /
/// MIDI) needs no engine, so we point there.
const AUDIO_ENGINE_UNAVAILABLE: &str =
    "Audio import isn't available right now — the audio engine couldn't start. You can still \
     practice with a MusicXML or MIDI score.";

/// Run the native transcription behind a panic guard so a failure in ONNX
/// Runtime or the audio decoder degrades to a calm error instead of unwinding
/// past the (synchronous) import command and crashing the whole app (#267). A
/// normal `Err` passes through unchanged; only an actual **panic** is converted
/// to [`AUDIO_ENGINE_UNAVAILABLE`]. (A hard C-level `abort()` can't be caught —
/// the real defense there is provisioning a compatible runtime.)
///
/// Relies on the workspace's default `panic = "unwind"`; a future `panic =
/// "abort"` would silently neuter this guard.
fn guard_transcription<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(inner) => inner,
        Err(_) => Err(AUDIO_ENGINE_UNAVAILABLE.to_string()),
    }
}

/// Import an audio recording: transcribe → MIDI → MusicXML → library.
///
/// Heavier than MIDI/MusicXML import, so it emits `import-progress` events
/// (`{ stage, pct }`) the UI shows as a live indicator. Returns the new entry
/// plus a quality signal for a dismissible "this may be approximate" banner.
/// `bytes` is the raw file content read on the frontend; `source_filename`
/// supplies the decode hint and the title fallback.
///
/// Must be `async`: a synchronous command runs on the main thread, which is
/// also the webview's event loop — the progress events would only be delivered
/// after the whole import finished, so the indicator never paints (#313).
#[tauri::command]
pub async fn import_audio_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    source_filename: String,
    bytes: Vec<u8>,
) -> Result<ImportedAudioDto, String> {
    import_audio_file_with(app, source_filename, bytes, |state, name, bytes, ext| {
        state.import_audio(name, bytes, ext)
    })
    .await
}

/// Body of [`import_audio_file`] with the import step as a **seam**, so tests
/// can observe the threading contract (heavy work never runs on the
/// dispatching thread) and the panic path without a real ONNX transcription.
async fn import_audio_file_with<R, F>(
    app: tauri::AppHandle<R>,
    source_filename: String,
    bytes: Vec<u8>,
    import_fn: F,
) -> Result<ImportedAudioDto, String>
where
    R: Runtime,
    F: FnOnce(
            &AppState,
            String,
            Vec<u8>,
            Option<&str>,
        ) -> Result<(ScoreLibraryEntry, transcribe::TranscriptionQuality), String>
        + Send
        + 'static,
{
    let ext = filename_ext(&source_filename);
    emit_import_progress(&app, "decoding", 15);
    emit_import_progress(&app, "transcribing", 45);
    let handle = app.clone();
    // A panic on the blocking thread surfaces as a join error — map it to the
    // same calm message as the #267 transcription guard (this also covers the
    // MIDI→MusicXML conversion the guard doesn't wrap) instead of leaving the
    // frontend's promise hanging.
    let (entry, quality) = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        import_fn(&state, source_filename, bytes, ext.as_deref())
    })
    .await
    .map_err(|_| AUDIO_ENGINE_UNAVAILABLE.to_string())??;
    emit_import_progress(&app, "converting", 85);
    emit_import_progress(&app, "done", 100);
    Ok(ImportedAudioDto::new(entry.into(), quality))
}

/// Backend result of recognizing a PDF: the canonical MusicXML plus the parts
/// found in it, so the frontend can run the shared "which part?" picker.
#[derive(Debug)]
struct RecognizedScore {
    music_xml: String,
    parts: Vec<String>,
    low_content: bool,
}

/// IPC result of `recognize_pdf_score`. The frontend feeds `music_xml` back
/// through `import_musicxml_file` with the chosen part — OMR reuses the exact
/// MusicXML import + part-picker path. `from_scan` is always true (it drives the
/// calm "read from a scan — check it" note); `low_content` warns when the scan
/// yielded almost nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognizedPdfDto {
    pub music_xml: String,
    pub parts: Vec<String>,
    pub from_scan: bool,
    pub low_content: bool,
}

impl From<RecognizedScore> for RecognizedPdfDto {
    fn from(r: RecognizedScore) -> Self {
        Self {
            music_xml: r.music_xml,
            parts: r.parts,
            from_scan: true,
            low_content: r.low_content,
        }
    }
}

/// Recognize a sheet-music **PDF** into MusicXML on-device (experimental beta).
///
/// Runs the bundled OMR engine (resolved via `OMR_ENGINE_PATH`, set at startup
/// from the app's resource dir — see `runtime::configure_omr_engine`). Heavier
/// than other imports, so it emits `import-progress` events the UI shows live.
/// Stores nothing: it returns the recognized MusicXML and its parts so the
/// frontend runs the same part picker as MusicXML import, then imports the
/// chosen part via [`import_musicxml_file`]. Errors calmly and specifically when
/// the beta is off, the engine isn't bundled, or the scan was unreadable —
/// never a fabricated score.
/// Like [`import_audio_file`], must be `async` so the OMR run happens off the
/// main thread and the progress events can actually paint (#313).
#[tauri::command]
pub async fn recognize_pdf_score<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    source_filename: String,
    bytes: Vec<u8>,
) -> Result<RecognizedPdfDto, String> {
    // `source_filename` is currently only a UX hint for the caller; the title
    // is derived when the chosen part is imported via `import_musicxml_file`.
    let _ = source_filename;
    // Check the beta gate first so the message is "not enabled" rather than a
    // confusing "engine missing" when the feature is simply off.
    if !state.omr_enabled() {
        return Err(
            "Reading sheet-music PDFs is an experimental feature that isn't \
                    enabled in this build yet."
                .to_string(),
        );
    }
    let engine_path = std::env::var_os("OMR_ENGINE_PATH").ok_or_else(|| {
        "The sheet-music reader isn't included in this build yet, so PDF import \
         isn't available."
            .to_string()
    })?;
    recognize_pdf_score_with(app, bytes, move |state, bytes| {
        let engine = omr::SidecarOmrEngine::new(std::path::PathBuf::from(engine_path));
        state.recognize_pdf(&engine, bytes)
    })
    .await
}

/// Calm message when the OMR run panics on the blocking thread — the join
/// error would otherwise leave the frontend's promise hanging.
const PDF_READER_STOPPED: &str =
    "The sheet-music reader stopped unexpectedly while reading that PDF.";

/// Body of [`recognize_pdf_score`] after its gates, with the OMR run as a
/// **seam** — the same testing story as [`import_audio_file_with`]: tests can
/// observe the threading contract and the panic path without a real engine.
async fn recognize_pdf_score_with<R, F>(
    app: tauri::AppHandle<R>,
    bytes: Vec<u8>,
    recognize_fn: F,
) -> Result<RecognizedPdfDto, String>
where
    R: Runtime,
    F: FnOnce(&AppState, &[u8]) -> Result<RecognizedScore, String> + Send + 'static,
{
    emit_import_progress(&app, "rasterizing", 20);
    emit_import_progress(&app, "reading-notes", 55);
    let handle = app.clone();
    let recognized = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        recognize_fn(&state, &bytes)
    })
    .await
    .map_err(|_| PDF_READER_STOPPED.to_string())??;
    emit_import_progress(&app, "done", 100);
    Ok(recognized.into())
}

/// List all scores in the library.
#[tauri::command]
pub fn list_scores(state: State<'_, AppState>) -> Result<Vec<ScoreLibraryEntryDto>, String> {
    let score_store = state.score_store.lock_or_recover();
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
    let entry = {
        let score_store = state.score_store.lock_or_recover();
        score_store.get(score_id).map_err(|e| e.to_string())?
    };
    // #449 T1: a successful load during a session is "a score is opened for
    // practice" — the id only, no content. Outside a session (library
    // browsing) the writer journals nothing, calmly.
    log_practice_event_best_effort(&state, "score_open", serde_json::json!({ "score_id": id }));
    Ok(LoadedScoreDto {
        music_xml: entry.music_xml.clone(),
        entry: entry.into(),
    })
}

/// #419 S3: one of "your patterns" — a cell your hands actually played,
/// ready to drop back into the Openers builder as a Notes item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MyPatternDto {
    pub label: String,
    pub offsets: Vec<i8>,
    pub times_practiced: usize,
    pub last_tonic: u8,
}

const MY_PATTERNS_CAP: usize = 6;

/// #419 S3: derive "My patterns" from the exercise log — rows carry the
/// full VariationSpec as replayable JSON, and any row whose spec has a
/// CELL is a pattern the player's hands actually produced. Dedup by
/// cell (identical offsets = one pattern practiced N times), most
/// recent first, capped. Store failures and unparseable rows are
/// skipped calmly — this list never errors the panel.
pub fn my_patterns_impl(state: &AppState) -> Vec<MyPatternDto> {
    let rows = match state.session_store.lock_or_recover().list_exercise_log() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "exercise log unreadable; My patterns empty");
            return Vec::new();
        }
    };
    // Most recent first; the log lists oldest-first.
    let mut patterns: Vec<MyPatternDto> = Vec::new();
    for row in rows.iter().rev() {
        let Ok(spec) = serde_json::from_str::<brain::coach::VariationSpec>(&row.spec_json) else {
            continue; // Old or foreign spec shape — skip calmly.
        };
        let Some(cell) = spec.cell.filter(|c| c.len() >= 2) else {
            // Catalog drills aren't "your" patterns, and a single note
            // (reachable via cell editing) isn't a pattern either —
            // there is nothing to row (the lift draws the same line).
            continue;
        };
        if let Some(existing) = patterns.iter_mut().find(|p| p.offsets == cell) {
            existing.times_practiced += 1;
            continue;
        }
        patterns.push(MyPatternDto {
            label: String::new(), // finalized below, once counts settle
            offsets: cell,
            times_practiced: 1,
            last_tonic: row.tonic % 12,
        });
    }
    patterns.truncate(MY_PATTERNS_CAP);
    for p in &mut patterns {
        let times = if p.times_practiced == 1 {
            "once".to_owned()
        } else {
            format!("{}×", p.times_practiced)
        };
        // #335 discipline: one spelling voice — signature-driven, the
        // same name the explore label would engrave (review MF3).
        let fifths = brain::coach::key_signature_for(p.last_tonic, "major").fifths;
        p.label = format!(
            "your {}-note cell · {times}, last in {}",
            p.offsets.len(),
            brain::coach::tonic_display_name(p.last_tonic, fifths)
        );
    }
    patterns
}

/// #419 S3: the My Patterns query for the Openers panel.
#[tauri::command]
pub fn my_patterns(state: State<'_, AppState>) -> Vec<MyPatternDto> {
    my_patterns_impl(&state)
}

/// #453 S1: one evidence-cited practice suggestion over the wire. `kind` is
/// the lowercase rule name ("trend" | "neglect" | "momentum"); `text` embeds
/// its numbers; `evidence` is the compact citation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PracticeSuggestionDto {
    pub kind: String,
    pub text: String,
    pub evidence: String,
}

impl From<brain::insights::PracticeSuggestion> for PracticeSuggestionDto {
    fn from(s: brain::insights::PracticeSuggestion) -> Self {
        let kind = match s.kind {
            brain::insights::SuggestionKind::Trend => "trend",
            brain::insights::SuggestionKind::Neglect => "neglect",
            brain::insights::SuggestionKind::Momentum => "momentum",
        };
        PracticeSuggestionDto {
            kind: kind.to_owned(),
            text: s.text,
            evidence: s.evidence,
        }
    }
}

/// #453 S1: the history analyzer over the local store — timed exercise log +
/// `key_mastery` EWMAs through `brain::insights::practice_suggestions`, with
/// `now` injected here (the analyzer stays pure). The my_patterns
/// discipline: store failures are skipped calmly with a warn — an empty list
/// is the honest answer, this never errors a surface. Silence > lies: no
/// history above the evidence bars means an EMPTY vec, not filler.
pub fn practice_suggestions_impl(state: &AppState) -> Vec<PracticeSuggestionDto> {
    practice_suggestions_core(state)
        .into_iter()
        .map(Into::into)
        .collect()
}

/// The store→analyzer read shared by the command (DTOs, S3's box) and the
/// recap path (#453 S2) — one read discipline, one silence discipline.
fn practice_suggestions_core(state: &AppState) -> Vec<brain::insights::PracticeSuggestion> {
    let store = state.session_store.lock_or_recover();
    let log = match store.list_exercise_log_timed() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "exercise log unreadable; no suggestions");
            return Vec::new();
        }
    };
    let model = match store.get_learner_model(LOCAL_TASTE_PROFILE_USER_ID) {
        Ok(model) => model.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "learner model unreadable; trends skipped");
            brain::learner::LearnerModel::default()
        }
    };
    brain::insights::practice_suggestions(&log, &model.key_mastery, Utc::now())
}

/// #453 S1: the practice-suggestions query (S2 recap + S3 coaching box
/// consume this; no frontend in this slice).
#[tauri::command]
pub fn practice_suggestions(state: State<'_, AppState>) -> Vec<PracticeSuggestionDto> {
    practice_suggestions_impl(&state)
}

/// #454 S2: one method-book tip over the wire — the pedagogy entry the last
/// session's measured fingerprint earned. `source_line` is
/// "{author}, {title}": the attribution the issue mandates, ALWAYS present.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PedagogyTipDto {
    pub topic: String,
    pub guidance: String,
    pub source_line: String,
}

/// #454 S2: resolve the LAST stored session's instrument family (the same
/// catalog resolution the recap uses — `instrument_family_for`) and run the
/// evidence-gated selection engine over that session's fingerprint.
///
/// `None` is the common, calm answer: no sessions yet, no measured
/// fingerprint on the latest session (the tip speaks to the last session,
/// never a stale one), an instrument outside the catalog, nothing above the
/// evidence bars, or a store failure (warn + `None` — the
/// `practice_suggestions` discipline: this never errors a surface).
/// Silence > lies: no measured trigger, no tip.
pub fn method_book_tip_impl(state: &AppState) -> Option<PedagogyTipDto> {
    let (instrument, fingerprint) = {
        let store = state.session_store.lock_or_recover();
        let latest = match store.list_recent(1) {
            Ok(mut sessions) => sessions.pop()?,
            Err(e) => {
                tracing::warn!(error = %e, "session history unreadable; no method-book tip");
                return None;
            }
        };
        let recap = match store.load_recap(latest.id) {
            Ok(recap) => recap,
            Err(e) => {
                tracing::warn!(error = %e, "latest recap unreadable; no method-book tip");
                return None;
            }
        };
        (recap.instrument, recap.fingerprint?)
    };
    let family = instrument_family_for(state, &instrument);
    let entry = brain::pedagogy::select_pedagogy(&family, &fingerprint)?;
    Some(PedagogyTipDto {
        source_line: entry.source_line(),
        topic: entry.topic,
        guidance: entry.guidance,
    })
}

/// #454 S2: the method-book-tip query (S3 wires it into the recap and the
/// #453 coaching box with the attribution visible; no frontend in this
/// slice).
#[tauri::command]
pub fn method_book_tip(state: State<'_, AppState>) -> Option<PedagogyTipDto> {
    method_book_tip_impl(&state)
}

/// #214 S1b: ambient identification — called by the frontend on each
/// phrase (the same cadence as coaching tips), reads the backend's own
/// phrase buffer, and answers through S1a's honesty gates. None is the
/// COMMON answer and never an error.
pub fn check_piece_match_impl(state: &AppState) -> Option<PieceMatchDto> {
    let recent: Vec<u8> = {
        let phrases = state.phrase_buffer.lock_or_recover();
        let tail: Vec<_> = phrases.iter().rev().take(3).collect();
        tail.into_iter()
            .rev()
            .flat_map(|p| {
                brain::coach::midi_track_from_pitch_track(
                    &p.pitch_stats.pitches,
                    brain::coach::LIFT_MIN_RUN,
                )
            })
            .collect()
    };
    let matcher = state.piece_matcher.lock_or_recover();
    let m = matcher.index.identify(&recent)?;
    let (score_id, title) = matcher.titles.get(&m.id)?.clone();
    Some(PieceMatchDto {
        score_id,
        title,
        coherent_hits: m.coherent_hits,
    })
}

/// #214 S1b: the identification query, frontend-triggered per phrase.
#[tauri::command]
pub fn check_piece_match(state: State<'_, AppState>) -> Option<PieceMatchDto> {
    check_piece_match_impl(&state)
}

/// Delete a score from the library.
#[tauri::command]
pub fn delete_score(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // See `get_score`: turbofish pins the parse target without naming the
    // non-direct-dependency `uuid::Error` type.
    state.delete_score_by_id(&id)
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

    /// #331: the IPC result carries the honesty verdict and counts exactly as
    /// the crate classified them — a full-mix quality maps to `"full_mix"` +
    /// `polyphonic: true` with the uncertain count intact, and a clean mono
    /// quality raises no flag at all. Fails if the DTO mapping drops, renames,
    /// or re-derives the verdict from its own thresholds.
    #[test]
    fn imported_audio_dto_carries_the_honesty_verdict_and_counts() {
        let entry = || ScoreLibraryEntryDto {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            title: "band".to_string(),
            composer: None,
            source_filename: "band.wav".to_string(),
            added_at: chrono::DateTime::UNIX_EPOCH,
            last_practiced_at: None,
            part_index: 0,
            duration_measures: 4,
        };

        let full_mix = ImportedAudioDto::new(
            entry(),
            transcribe::TranscriptionQuality {
                note_count: 12,
                mean_confidence: 0.6,
                polyphony: 1.0,
                uncertain_count: 5,
                verdict: transcribe::PolyphonyVerdict::FullMix,
            },
        );
        assert_eq!(full_mix.verdict, "full_mix");
        assert!(full_mix.polyphonic);
        assert!(!full_mix.low_confidence);
        assert_eq!(full_mix.note_count, 12);
        assert_eq!(full_mix.uncertain_count, 5);

        let mono = ImportedAudioDto::new(
            entry(),
            transcribe::TranscriptionQuality {
                note_count: 8,
                mean_confidence: 0.75,
                polyphony: 0.0,
                uncertain_count: 0,
                verdict: transcribe::PolyphonyVerdict::Mono,
            },
        );
        assert_eq!(mono.verdict, "mono");
        assert!(!mono.polyphonic, "a clean line must not warn");
        assert!(!mono.low_confidence);

        let weak_borderline = ImportedAudioDto::new(
            entry(),
            transcribe::TranscriptionQuality {
                note_count: 6,
                mean_confidence: 0.3,
                polyphony: 0.2,
                uncertain_count: 4,
                verdict: transcribe::PolyphonyVerdict::Borderline,
            },
        );
        assert_eq!(weak_borderline.verdict, "borderline");
        assert!(weak_borderline.polyphonic);
        assert!(
            weak_borderline.low_confidence,
            "mean confidence below the crate constant must flag"
        );
    }

    /// #267 AC1: a panic inside transcription (e.g. ONNX Runtime aborting a Rust
    /// panic, or a decoder unwrap) is converted to the calm, non-fatal message
    /// instead of unwinding into the command and crashing the app — while a
    /// normal `Ok`/`Err` passes through untouched. Fails if the guard ever
    /// re-panics, swallows a real error, or mangles a success.
    #[test]
    fn guard_transcription_converts_panic_and_passes_results_through() {
        let panicked: Result<(), String> = guard_transcription(|| panic!("onnxruntime blew up"));
        assert!(panicked.is_err());
        assert!(
            panicked
                .unwrap_err()
                .contains("Audio import isn't available"),
            "a panic must map to the friendly unavailable message"
        );

        assert_eq!(guard_transcription(|| Ok::<_, String>(7)).unwrap(), 7);

        let inner_err =
            guard_transcription(|| Err::<(), _>("could not decode audio: bad header".to_string()))
                .unwrap_err();
        assert_eq!(
            inner_err, "could not decode audio: bad header",
            "a real error must pass through unchanged, not be replaced by the panic message"
        );
    }

    /// #267 AC2: the guard is actually *wired into* the import path. A panic in
    /// the (injected) transcription surfaces as the friendly error from
    /// `import_audio_with` — not a crash. Fails if the guard is ever removed from
    /// the import wiring (the panic would unwind through the test body).
    #[test]
    fn import_audio_panic_degrades_to_error_not_crash() {
        let s = state();
        let result = s.import_audio_with(
            "recording.wav".to_string(),
            || -> Result<(Vec<u8>, transcribe::TranscriptionQuality), transcribe::TranscribeError> {
                panic!("onnxruntime aborted")
            },
        );
        assert!(
            result
                .as_ref()
                .err()
                .is_some_and(|e| e.contains("Audio import isn't available")),
            "a transcription panic must degrade to the friendly error, got {result:?}"
        );
    }

    /// #337 S4 end-to-end (review finding 7): verdicts flow
    /// verdict_buffer-shape → RecapInput.note_verdicts → score_summary →
    /// exercise_log. Fails if any join in the chain drops the data.
    #[tokio::test]
    async fn score_session_verdicts_reach_the_recap_and_the_log() {
        use brain::follower::{NoteVerdict, Verdict};
        let s = state();
        let mut recorder = brain::session::SessionRecorder::new(
            "trumpet".to_owned(),
            brain::session::PracticeMode::default(),
        );
        recorder.set_score_title(Some("Haydn".to_owned()));
        // One recorded phrase so the session isn't empty.
        recorder
            .record_phrase(sample_phrase())
            .expect("phrase records");
        let completed = recorder.complete().expect("session completes");
        let verdicts = vec![
            NoteVerdict {
                measure_number: 1,
                beat: 0.0,
                verdict: Verdict::Hit,
            },
            NoteVerdict {
                measure_number: 3,
                beat: 1.0,
                verdict: Verdict::Missed,
            },
        ];
        let generator = MockRecapGenerator;
        let recap = build_recap(
            &completed,
            &generator,
            None,
            Vec::new(),
            verdicts,
            String::new(),
            Vec::new(),
            None,
        )
        .await
        .expect("recap builds");
        let summary = recap
            .score_summary
            .as_ref()
            .expect("verdicts + a score title yield a summary");
        assert_eq!(summary.judged, 2);
        assert_eq!(summary.worst_measures[0].measure_number, 3);

        // And the summary leaves exercise evidence.
        {
            let store = s.session_store.lock().unwrap();
            log_score_practice_best_effort(&store, summary);
        }
        let log = s.session_store.lock().unwrap().list_exercise_log().unwrap();
        assert_eq!(log.last().unwrap().source, "score_practice");
        assert!((log.last().unwrap().accuracy.unwrap() - 0.5).abs() < 1e-6);
    }

    /// #337 S5 AC6: the RV bridge — a stored score's measure becomes a
    /// 12-key exploration through the real engine: the cell is the
    /// measure's notes as offsets, it rows through multiple roots, and the
    /// staff renders. Unknown and out-of-library cases refuse calmly.
    #[test]
    fn explore_measure_rows_a_stored_measure_through_keys() {
        let s = state();
        // A 4-note MIDI import gives us a library score with known content.
        let entry = s
            .import_midi("bridge.mid".to_string(), build_test_midi(Some("Bridge")))
            .expect("import fixture");
        let dto = explore_measure_impl(&s, &entry.id.to_string(), 1, 7, FoldWindow::default())
            .expect("measure 1 rows");
        assert!(
            !dto.staff.notes.is_empty(),
            "the exploration renders on the staff"
        );
        // #419 S2b round-3 MF3: the measure path stays FORWARD — the first
        // segment follows the stored measure's contour (C D E F). A
        // direction leak into this path goes red.
        let m1 = dto
            .music_xml
            .split("<measure number=\"2\">")
            .next()
            .unwrap();
        let steps: Vec<&str> = m1
            .match_indices("<step>")
            .map(|(i, _)| {
                let rest = &m1[i + 6..];
                &rest[..rest.find("</step>").unwrap()]
            })
            .collect();
        assert_eq!(steps, vec!["C", "D", "E", "F"], "measure contour intact");
        assert!(
            dto.root_pitch_classes.len() >= 3,
            "rowed through multiple keys: {:?}",
            dto.root_pitch_classes
        );
        // The engine's label names the player's own material.
        assert!(
            dto.label.contains("cell"),
            "a measure rows as a cell: {}",
            dto.label
        );
        // And it left exercise evidence with the bridge's own source tag.
        let log = s
            .session_store
            .lock()
            .unwrap()
            .list_exercise_log()
            .expect("log reads");
        assert_eq!(log.last().unwrap().source, "measure_bridge");

        // Calm refusals: a measure the piece doesn't have, and a bad id.
        let err = explore_measure_impl(&s, &entry.id.to_string(), 99, 7, FoldWindow::default())
            .unwrap_err();
        assert!(err.contains("isn't in this piece"), "got: {err}");
        let err =
            explore_measure_impl(&s, "not-a-real-id", 1, 7, FoldWindow::default()).unwrap_err();
        assert!(err.contains("isn't in the library"), "got: {err}");
    }

    /// Review MUST-FIX 4 (#337 S1): a recording whose transcription hears
    /// zero notes gets a RECORDING-flavored refusal — never the MIDI
    /// parser's "drum, click, or marker track" copy, which is a lie for a
    /// .wav. Fails if the audio seam stops mapping the parser's message.
    #[test]
    fn a_silent_recording_refuses_in_recording_terms() {
        let s = state();
        // A valid but empty MIDI transcription result (header + bare track).
        let mut midi = Vec::new();
        midi.extend_from_slice(b"MThd");
        midi.extend_from_slice(&6u32.to_be_bytes());
        midi.extend_from_slice(&0u16.to_be_bytes());
        midi.extend_from_slice(&1u16.to_be_bytes());
        midi.extend_from_slice(&480u16.to_be_bytes());
        midi.extend_from_slice(b"MTrk");
        midi.extend_from_slice(&4u32.to_be_bytes());
        midi.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let result = s.import_audio_with("silence.wav".to_string(), move || {
            Ok((
                midi,
                transcribe::TranscriptionQuality {
                    note_count: 0,
                    mean_confidence: 0.0,
                    polyphony: 0.0,
                    uncertain_count: 0,
                    verdict: transcribe::PolyphonyVerdict::Mono,
                },
            ))
        });
        let err = result.expect_err("empty transcription must refuse");
        assert!(
            err.contains("couldn't hear any notes in that recording"),
            "recording-flavored copy expected, got: {err}"
        );
        assert!(
            !err.contains("MIDI") && !err.contains("marker track"),
            "MIDI-parser copy must not leak to a .wav user: {err}"
        );
    }

    /// #253 S2 AC5: the mock coaching service (and the default impl) enrich a
    /// reveal to itself — no network, no change — so tests and the web preview
    /// stay fully offline. Fails if the default `enrich_reveal` ever mutated the
    /// reveal.
    #[tokio::test]
    async fn mock_service_enrich_reveal_is_identity() {
        let svc = MockCoachingService::new();
        let reveal = Reveal {
            concept: "G Dorian".to_owned(),
            connection: "Miles Davis — \"So What\"".to_owned(),
            why: "curated line".to_owned(),
            source: brain::connections::RevealSource::Grounded,
            tonic: 7,
            mode: "dorian".to_owned(),
        };
        assert_eq!(svc.enrich_reveal(reveal.clone()).await, reveal);
    }

    #[test]
    fn open_stores_degrades_to_in_memory_instead_of_panicking() {
        // Put a regular FILE where a directory component is expected, so the
        // on-disk open fails with ENOTDIR even though the store creates parent
        // dirs — and regardless of the test user's permissions (#137).
        let blocker = std::env::temp_dir().join(format!("amc_137_blocker_{}", std::process::id()));
        std::fs::write(&blocker, b"x").unwrap();
        let bad = blocker.join("sub").join("amc.db");

        let (session, score, persisted) = open_stores(&bad);
        let _ = std::fs::remove_file(&blocker);

        assert!(
            !persisted,
            "an unusable data dir must report degraded, not persisted"
        );
        // The degraded stores are still usable, so the session can run.
        assert!(
            session.list_recent(1).is_ok(),
            "degraded session store must still work"
        );
        assert!(score.list().is_ok(), "degraded score store must still work");
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
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
            score_span: None,
            verdicts: None,
            score_card: None,
        }
    }

    /// #417-3 review MF1: the trait resolution itself — polyphonic and
    /// grand-staff are separate facts. Guitar is the divergence case (deals
    /// chords, engraves on ONE staff); resolving grand staff from
    /// `polyphonic` instead of the family must fail here.
    #[test]
    fn lesson_traits_split_polyphonic_from_grand_staff() {
        let s = state();
        assert_eq!(lesson_instrument_traits(&s, "Piano"), (true, true));
        assert_eq!(lesson_instrument_traits(&s, "Guitar"), (true, false));
        assert_eq!(lesson_instrument_traits(&s, "Trumpet"), (false, false));
        assert_eq!(lesson_instrument_traits(&s, "Theremin"), (false, false));
    }

    /// #417-3 AC5 at the command layer: a KEYBOARD lesson's drills engrave
    /// on a grand staff — the first drill AND the next one after a submit
    /// (the flag must thread through `advance`, not just lesson start) —
    /// while a melodic lesson's XML carries no staff machinery at all.
    #[test]
    fn keyboard_lesson_drills_render_a_grand_staff() {
        let s = state();
        let step = start_lesson_impl(&s, 42, true, true, FoldWindow::default())
            .expect("keyboard lesson starts");
        let xml = &step.drill.as_ref().unwrap().music_xml;
        assert!(xml.contains("<staves>2</staves>"), "first drill: staves");
        assert!(xml.contains("<staff>"), "first drill: per-note staff");

        play_current_drill_perfectly(&s);
        let next = submit_drill_impl(&s, 1_000, FoldWindow::default()).expect("submit succeeds");
        let drill = next
            .drill
            .as_ref()
            .expect("a 4-drill lesson has a next drill after one submit");
        assert!(
            drill.music_xml.contains("<staves>2</staves>"),
            "the NEXT drill must stay grand staff — the flag threads \
             through advance, not just lesson start"
        );

        let s2 = state();
        let step2 = start_lesson_impl(&s2, 42, false, false, FoldWindow::default())
            .expect("melodic lesson starts");
        let xml2 = &step2.drill.as_ref().unwrap().music_xml;
        assert!(!xml2.contains("<staves>"), "melodic: no staves");
        assert!(!xml2.contains("<staff>"), "melodic: no staff elements");
        assert!(!xml2.contains("<clef"), "melodic: no clef (OSMD default)");
    }

    /// #349 T2b end-to-end at the command layer: a POLYPHONIC lesson deals
    /// the chord drill as block chords and grades it from the chord buffer
    /// via the T1-engine judge — while melodic drills in the same lesson
    /// keep the phrase-buffer path. Fails if the grading fork, the marks,
    /// or the polyphonic dealing regresses.
    #[test]
    fn a_polyphonic_lesson_deals_and_grades_chord_drills() {
        let s = state();
        let mut last =
            start_lesson_impl(&s, 42, true, true, FoldWindow::default()).expect("lesson starts");
        let mut saw_chord_drill = false;
        let mut steps = 0;
        while last.drill.is_some() {
            let (targets, target_midi) = {
                let guard = s.active_lesson.lock().unwrap();
                let cur = &guard.as_ref().unwrap().current;
                (
                    cur.sequence.chord_targets.clone(),
                    cur.sequence.target_midi.clone(),
                )
            };
            if targets.is_empty() {
                // Melodic drill: the existing phrase path.
                let mut phrase = sample_phrase();
                phrase.pitch_stats.pitches = target_midi
                    .iter()
                    .flat_map(|&m| {
                        let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                        std::iter::repeat_n(hz, 5)
                    })
                    .collect();
                phrase.onsets_secs = vec![0.0; target_midi.len()];
                s.phrase_buffer.lock().unwrap().push(phrase);
            } else {
                saw_chord_drill = true;
                // An empty room must not grade: no chords heard, no onsets.
                assert!(
                    matches!(
                        submit_drill_impl(&s, 1_000, FoldWindow::default()),
                        Err(CommandError::DrillNotHeard)
                    ),
                    "silence on a chord drill must be a calm 'not yet'"
                );
                // Play every cell right, through the REAL edge-trigger:
                // strike (observe Some), then release (observe None).
                let mut buf = s.chord_buffer.lock().unwrap();
                for t in &targets {
                    buf.observe(Some(brain::chord_judge::HeardChord {
                        root_pc: t.root_pc,
                        quality: t.quality,
                        bass_pc: t.bass_pc,
                    }));
                    buf.observe(None);
                }
            }
            last = submit_drill_impl(&s, 1_000, FoldWindow::default()).expect("submit succeeds");
            let score = last.score.as_ref().expect("score present");
            assert!(
                score.accuracy > 0.99,
                "perfect performance grades ~1.0, got {}",
                score.accuracy
            );
            steps += 1;
            assert!(steps <= 4, "routine must terminate");
        }
        assert!(
            saw_chord_drill,
            "a polyphonic lesson must deal at least one chord drill"
        );
        last.recap.expect("lesson ends in a recap");
    }

    /// #254 end-to-end at the command layer: start a lesson, play each drill
    /// perfectly (synthetic phrases matching the target), submit through all
    /// four drills, and confirm the recap + the persisted Learner Model
    /// (mastery recorded, difficulty ramped, next lesson starts harder). Fails
    /// if the state machine, grading wiring, or persistence hop breaks.
    #[test]
    fn lesson_lifecycle_grades_ramps_and_persists() {
        let s = state();
        let step0 =
            start_lesson_impl(&s, 42, false, false, FoldWindow::default()).expect("lesson starts");
        let drill0 = step0.drill.as_ref().expect("drill 0 present");
        assert_eq!(drill0.index, 0);
        assert!(drill0.music_xml.contains("<score-partwise"));
        assert!(step0.score.is_none() && step0.recap.is_none());

        // Perform each drill perfectly: synthesize a phrase whose pitch track
        // walks the exact target (each note held for several samples).
        let mut steps = 0;
        let mut last = step0;
        while let Some(drill_index) = last.drill.as_ref().map(|d| d.index) {
            // Rebuild the target's pitch track from the lesson state.
            let target: Vec<u8> = {
                let guard = s.active_lesson.lock().unwrap();
                guard.as_ref().unwrap().current.sequence.target_midi.clone()
            };
            let mut phrase = sample_phrase();
            phrase.pitch_stats.pitches = target
                .iter()
                .flat_map(|&m| {
                    let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                    std::iter::repeat_n(hz, 5)
                })
                .collect();
            phrase.onsets_secs = vec![0.0; target.len()];
            s.phrase_buffer.lock().unwrap().push(phrase);

            last = submit_drill_impl(&s, 1_000, FoldWindow::default()).expect("submit succeeds");
            let score = last.score.as_ref().expect("every submit carries a score");
            assert!(
                score.accuracy > 0.99,
                "perfect performance must grade ~1.0, got {} (drill {drill_index})",
                score.accuracy
            );
            steps += 1;
            assert!(steps <= 4, "routine must terminate");
        }

        let recap = last.recap.expect("lesson ends in a recap");
        assert_eq!(recap.drill_accuracies.len(), 4);
        assert_eq!(recap.start_difficulty, 0);
        assert!(recap.end_difficulty > 0, "perfect lesson ramps up");

        // Persisted: mastery written and the next lesson starts harder.
        let model = s
            .session_store
            .lock()
            .unwrap()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("model persisted");
        assert!(!model.key_mastery.is_empty());
        assert_eq!(model.difficulty, recap.end_difficulty);
        let again = start_lesson_impl(&s, 43, false, false, FoldWindow::default()).unwrap();
        assert_eq!(
            again.drill.unwrap().difficulty,
            recap.end_difficulty,
            "next lesson starts where the last one ended"
        );
    }

    /// Submitting with no active lesson is a calm error, and ending a lesson
    /// clears it (nothing persisted for an abandoned lesson).
    #[test]
    fn submit_without_lesson_errs_and_end_abandons() {
        let s = state();
        assert!(submit_drill_impl(&s, 0, FoldWindow::default()).is_err());
        start_lesson_impl(&s, 1, false, false, FoldWindow::default()).unwrap();
        end_lesson_impl(&s, 10);
        assert!(
            submit_drill_impl(&s, 0, FoldWindow::default()).is_err(),
            "abandoned lesson is gone"
        );
        // Zero completed drills → nothing persisted (spec #254 §6).
        assert!(s
            .session_store
            .lock()
            .unwrap()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .is_none());
    }

    /// Push one synthetic phrase whose pitch track perfectly walks the current
    /// drill's target.
    fn play_current_drill_perfectly(s: &AppState) {
        let target: Vec<u8> = {
            let guard = s.active_lesson.lock().unwrap();
            guard.as_ref().unwrap().current.sequence.target_midi.clone()
        };
        let mut phrase = sample_phrase();
        phrase.pitch_stats.pitches = target
            .iter()
            .flat_map(|&m| {
                let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                std::iter::repeat_n(hz, 5)
            })
            .collect();
        phrase.onsets_secs = vec![0.0; target.len()];
        s.phrase_buffer.lock().unwrap().push(phrase);
    }

    /// Spec #254 §6: ending EARLY keeps the credit for the drills that were
    /// completed and scored — mastery is persisted for them, while the
    /// unfinished drill is dropped. Fails if early-end regresses to
    /// abandon-everything.
    #[test]
    fn early_end_persists_completed_drills_only() {
        let s = state();
        start_lesson_impl(&s, 2, false, false, FoldWindow::default()).unwrap();
        play_current_drill_perfectly(&s);
        let step = submit_drill_impl(&s, 100, FoldWindow::default()).unwrap();
        assert!(step.drill.is_some(), "one drill done, lesson continues");

        end_lesson_impl(&s, 200);
        let model = s
            .session_store
            .lock()
            .unwrap()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("partial lesson persisted");
        assert_eq!(
            model.key_mastery.len(),
            1,
            "one scored drill = one mastery entry"
        );
        assert!(s.active_lesson.lock().unwrap().is_none());
    }

    /// #254 review (b): each drill is graded ONLY against what was played
    /// since it began. After a perfect drill 0, submitting drill 1 with no new
    /// phrases must grade 0 — a stale phrase_mark (or a dropped mark update)
    /// would let drill 0's notes leak in and inflate it.
    #[test]
    fn drill_grading_is_isolated_to_its_own_phrase_window() {
        let s = state();
        start_lesson_impl(&s, 3, false, false, FoldWindow::default()).unwrap();
        play_current_drill_perfectly(&s);
        let step1 = submit_drill_impl(&s, 100, FoldWindow::default()).unwrap();
        assert!(step1.score.unwrap().accuracy > 0.99);

        // Drill 1: play only clearly WRONG material (a cluster far from any
        // target class run). If drill 0's perfect notes leaked into this
        // window (a dropped phrase_mark), the grade would be inflated.
        let mut wrong = sample_phrase();
        wrong.pitch_stats.pitches = std::iter::repeat_n(8_000.0, 40).collect();
        wrong.onsets_secs = vec![0.0; 4];
        s.phrase_buffer.lock().unwrap().push(wrong);
        let step2 = submit_drill_impl(&s, 200, FoldWindow::default()).unwrap();
        assert!(
            step2.score.unwrap().accuracy < 0.2,
            "drill 1 must be graded ONLY on its own (wrong) take, not inherit drill 0's notes"
        );
    }

    /// Mutation M6 (review): the guard's boundary is AND, not OR — a take the
    /// app HEARD (onsets but nothing pitched, e.g. clapping; or pitched
    /// legato with zero detected onsets) still GRADES instead of trapping the
    /// player in "I didn't catch that yet" forever.
    #[test]
    fn heard_but_imperfect_takes_still_grade() {
        let s = state();
        start_lesson_impl(&s, 12, false, false, FoldWindow::default()).unwrap();
        // Unpitched noise WITH onsets: grades (0%), never DrillNotHeard.
        let mut claps = sample_phrase();
        claps.pitch_stats.pitches = Vec::new();
        claps.onsets_secs = vec![0.1, 0.4, 0.8];
        s.phrase_buffer.lock().unwrap().push(claps);
        let step = submit_drill_impl(&s, 100, FoldWindow::default()).unwrap();
        assert_eq!(step.score.unwrap().accuracy, 0.0, "heard noise grades 0");

        // Pitched legato with ZERO detected onsets: also grades.
        let mut legato = sample_phrase();
        legato.pitch_stats.pitches = std::iter::repeat_n(440.0, 40).collect();
        legato.onsets_secs = Vec::new();
        s.phrase_buffer.lock().unwrap().push(legato);
        assert!(
            submit_drill_impl(&s, 200, FoldWindow::default()).is_ok(),
            "a legato singer must not be trapped in not-yet"
        );
    }

    /// #277 hardening: a tap before ANY phrase has closed for the drill is a
    /// calm "not yet" error, never a lying 0% grade — the drill stays live for
    /// a retry.
    #[test]
    fn eager_submit_before_any_phrase_is_a_calm_not_yet() {
        let s = state();
        start_lesson_impl(&s, 12, false, false, FoldWindow::default()).unwrap();
        assert!(matches!(
            submit_drill_impl(&s, 100, FoldWindow::default()),
            Err(CommandError::DrillNotHeard)
        ));
        // Still live: playing and resubmitting works.
        play_current_drill_perfectly(&s);
        assert!(
            submit_drill_impl(&s, 200, FoldWindow::default())
                .unwrap()
                .score
                .unwrap()
                .accuracy
                > 0.99
        );
    }

    /// Self-improvement (#252): every dealt exercise leaves EVIDENCE — a
    /// graded drill logs with its accuracy, explore/lift deals log ungraded
    /// (the "they bailed" signal), and the insights analyzer reads it all
    /// back per shape. Fails if any recording hook is dropped.
    #[test]
    fn exercises_leave_evidence_in_the_log() {
        let s = state();
        // A graded lesson drill…
        start_lesson_impl(&s, 12, false, false, FoldWindow::default()).unwrap();
        play_current_drill_perfectly(&s);
        submit_drill_impl(&s, 100, FoldWindow::default()).unwrap();
        // …an explore deal + a chip…
        start_explore_variation_impl(&s, 7, "dorian", 42, FoldWindow::default()).unwrap();
        apply_variation_delta_impl(&s, VariationDelta::ToggleDirection, FoldWindow::default())
            .unwrap();

        let log = s.session_store.lock().unwrap().list_exercise_log().unwrap();
        let sources: Vec<&str> = log.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(sources, vec!["lesson", "explore", "explore_chip"]);
        assert!(
            log[0].accuracy.unwrap() > 0.99,
            "the graded drill logs its grade"
        );
        assert!(log[1].accuracy.is_none(), "explore deals log ungraded");
        // And the analyzer reads it per shape.
        let insights = brain::insights::exercise_insights(&log);
        assert!(!insights.is_empty());
        assert!(insights.iter().map(|i| i.generated).sum::<u32>() >= 3);
    }

    /// #258 command-layer lifecycle: below MIN_SESSIONS the mirror is dark
    /// with an honest count; at/after it, the profile derives from the stored
    /// fingerprints AND persists onto the Learner Model blob (F2's reserved
    /// field), so it rides the sync. Fails if the derivation plumbing or the
    /// persist-back breaks.
    #[test]
    fn sound_mirror_derives_counts_and_persists() {
        let s = state();

        // Two measured sessions: dark mirror, honest count.
        seed_measured_sessions(&s, 2);
        let dark = get_sound_mirror_impl(&s, 100).unwrap();
        assert!(dark.profile.is_none());
        assert_eq!(dark.sessions_seen, 2);

        // Six: the mirror resolves and lands on the model.
        seed_measured_sessions(&s, 4);
        let lit = get_sound_mirror_impl(&s, 200).unwrap();
        let p = lit.profile.expect("6 measured sessions light the mirror");
        assert_eq!(p.mode_lean, Some(brain::mirror::ModeLean::Minor));
        assert_eq!(p.feel, Some(brain::mirror::Feel::Swung));
        let stored = s
            .session_store
            .lock()
            .unwrap()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("model exists after mirror derivation");
        assert_eq!(
            stored.sound_profile.as_ref().map(|sp| sp.mode_lean),
            Some(Some(brain::mirror::ModeLean::Minor)),
            "the snapshot must persist onto the blob"
        );
    }

    /// Seed `n` stored sessions whose recaps carry a consistently dark, swung
    /// fingerprint (D dorian, swing 1.4).
    fn seed_measured_sessions(s: &AppState, n: usize) {
        use chrono::{Duration, TimeZone, Utc};
        let store = s.session_store.lock().unwrap();
        for _ in 0..n {
            let mut recap = empty_state_recap(60.0, "trumpet".to_owned());
            recap.fingerprint = Some(
                serde_json::from_value(serde_json::json!({
                    "key": { "tonic": 2, "mode": "dorian", "confidence": 0.8, "margin": 0.2 },
                    "groove": {
                        "tempo_bpm": 100.0, "swing_ratio": 1.4,
                        "timing_consistency": 0.8, "mean_ioi_secs": 0.3, "onset_count": 24
                    }
                }))
                .expect("fingerprint fixture parses"),
            );
            let id = brain::session::SessionId::new();
            let t0 = Utc.timestamp_opt(1_000_000, 0).unwrap();
            store
                .save(id, t0, t0 + Duration::seconds(60), &recap)
                .unwrap();
        }
    }

    /// #285 end-to-end at the command layer: a played lick in the phrase
    /// buffer lifts into an editable, rowed exploration; an empty buffer
    /// refuses calmly. Fails if the lift → explore → edit chain breaks.
    #[test]
    fn last_phrase_lifts_into_an_editable_row() {
        let s = state();
        assert!(
            explore_last_phrase_impl(&s, 42, FoldWindow::default()).is_err(),
            "nothing played yet → calm error"
        );
        // A clear 5-note lick: D F E A D (each held long enough to collapse).
        let mut phrase = sample_phrase();
        phrase.pitch_stats.pitches = [62u8, 65, 64, 69, 62]
            .iter()
            .flat_map(|&m| {
                let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                std::iter::repeat_n(hz, 6)
            })
            .collect();
        s.phrase_buffer.lock().unwrap().push(phrase);

        let dto = explore_last_phrase_impl(&s, 42, FoldWindow::default()).unwrap();
        assert!(dto.label.contains("5-note cell"), "got {}", dto.label);
        assert!(!dto.root_pitch_classes.is_empty());
        assert!(!dto.staff.notes.is_empty());
        // #419 S2b review MF7: the lift path stays FORWARD — the first
        // segment's steps follow the played lick's contour (D F E A D).
        // A direction leak into the lift path (the S2b param) breaks this.
        let m1 = dto
            .music_xml
            .split("<measure number=\"2\">")
            .next()
            .unwrap();
        let steps: Vec<&str> = m1
            .match_indices("<step>")
            .map(|(i, _)| {
                let rest = &m1[i + 6..];
                &rest[..rest.find("</step>").unwrap()]
            })
            .collect();
        assert_eq!(
            steps,
            vec!["D", "F", "E", "A", "D"],
            "lifted contour intact"
        );
        // And it's immediately editable (#292): the correction UX this loop
        // was built for.
        let edited = edit_explore_note_impl(
            &s,
            0,
            brain::coach::NoteEdit::Octaves { by: 1 },
            FoldWindow::default(),
        )
        .unwrap();
        assert!(edited.can_undo);
    }

    /// #370 (#341 review residual 2): a phrase closed while an exploration
    /// is on stage sheds every score anchor — so the score card, the
    /// cursor (`phrase-detected` moves it too), and the recap never cite
    /// the exploration — while the phrase itself stays liftable. With no
    /// exploration live, the phrase passes through untouched. Fails if the
    /// scrub drops the wrong fields, fires when it shouldn't, or breaks
    /// the lift path.
    #[test]
    fn exploration_phrases_shed_score_anchors_but_stay_liftable() {
        // A score-anchored phrase whose lick is liftable: D F E A D.
        let mut phrase = sample_phrase();
        phrase.pitch_stats.pitches = [62u8, 65, 64, 69, 62]
            .iter()
            .flat_map(|&m| {
                let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                std::iter::repeat_n(hz, 6)
            })
            .collect();
        phrase.score_position = Some(ScorePosition {
            measure_number: 3,
            beat: 1.0,
            section_name: None,
            expected_note: Some(62),
        });
        phrase.score_span = Some((3, 4));
        phrase.verdicts = Some(brain::phrase::PhraseVerdicts {
            hit: 2,
            near: 1,
            missed: 1,
        });
        phrase.score_card = Some("Measures 3–4 — 2 clean, 1 rough, 1 missed".to_owned());

        // No exploration live → untouched, anchors and all.
        let kept = scrub_score_anchors_if_exploring(phrase.clone(), false);
        assert_eq!(kept, phrase, "no exploration → the phrase passes through");

        // Exploration live → every score anchor gone, nothing else.
        let scrubbed = scrub_score_anchors_if_exploring(phrase.clone(), true);
        assert!(scrubbed.score_position.is_none(), "cursor anchor detached");
        assert!(scrubbed.score_span.is_none(), "span detached");
        assert!(scrubbed.verdicts.is_none(), "verdict tally detached");
        assert!(scrubbed.score_card.is_none(), "card line detached");
        assert_eq!(
            scrubbed.pitch_stats.pitches, phrase.pitch_stats.pitches,
            "the lick itself is untouched"
        );

        // …and the lift path still lifts it (the AC's second half).
        let s = state();
        s.phrase_buffer.lock().unwrap().push(scrubbed);
        let dto = explore_last_phrase_impl(&s, 42, FoldWindow::default()).unwrap();
        assert!(dto.label.contains("5-note cell"), "got {}", dto.label);
    }

    /// #370 review MF1+MF2 — the scrub WIRING at the "Back to listening"
    /// boundary, driven through the real pipeline callback. A phrase only
    /// closes when the NEXT event arrives, so the phrase overlaying an
    /// exploration closes AFTER the gate clears: a callback that reads the
    /// gate at close time alone leaks that phrase's bogus anchors into the
    /// buffer and the card. Fails if the scrub is unwired from the
    /// callback, or the opened-mid-exploration latch is dropped, or honest
    /// phrases get scrubbed.
    #[test]
    fn phrase_callback_scrubs_across_the_back_to_listening_boundary() {
        use tauri::test::mock_app;
        let app = mock_app();
        let buffer: Arc<std::sync::Mutex<Vec<PhraseSummary>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate: Arc<std::sync::Mutex<Option<ExploreState>>> =
            Arc::new(std::sync::Mutex::new(None));
        let mut on_phrase = make_phrase_closed_callback(
            app.handle().clone(),
            buffer.clone(),
            Arc::new(std::sync::Mutex::new(None)),
            gate.clone(),
        );
        let anchored = || {
            let mut p = sample_phrase();
            p.score_span = Some((3, 4));
            p.score_card = Some("Measures 3–4 — 2 clean".to_owned());
            p
        };
        let staged_explore = || {
            brain::coach::start_explore(0, "major", &brain::learner::LearnerModel::default(), 1).0
        };

        // 1: closes with no exploration anywhere near it → anchors kept.
        on_phrase(anchored());
        // 2: closes while the exploration is on stage → scrubbed.
        *gate.lock().unwrap() = Some(staged_explore());
        on_phrase(anchored());
        // 3: "Back to listening" clears the gate BEFORE the phrase that was
        // open during the exploration closes → still scrubbed (the leak).
        *gate.lock().unwrap() = None;
        on_phrase(anchored());
        // 4: opened and closed after the exploration ended → kept.
        on_phrase(anchored());

        let buf = buffer.lock().unwrap();
        let cards: Vec<bool> = buf.iter().map(|p| p.score_card.is_some()).collect();
        assert_eq!(
            cards,
            [true, false, false, true],
            "kept / mid-exploration / boundary leak / kept"
        );
        assert!(
            buf.iter().all(|p| p.note_count == 6),
            "the phrases themselves stay buffered for the lift path"
        );
    }

    /// #349 T3 AC3 at the command layer: the chart's trailing chords lift
    /// into a rowed progression (consecutive dupes collapsed, unresolved
    /// skipped, capped at 4), logged as progression_lift; fewer than two
    /// distinct chords refuses calmly.
    #[test]
    fn the_charts_trailing_chords_lift_as_a_progression() {
        let s = state();
        assert!(
            explore_progression_impl(&s, 42, FoldWindow::default())
                .unwrap_err()
                .contains("play a couple of chords"),
            "empty chart → calm refusal"
        );
        // The room played Dm7 (twice re-promoted), a messy stretch, G7,
        // Cmaj7 — the lift sees [Dm7, G7, Cmaj7].
        {
            let mut chart = s.chord_chart.lock().unwrap();
            let snap = |pc: u8, q: brain::theory::ChordQuality, label: &str| {
                brain::perception::PerceptionSnapshot {
                    chord: Some(brain::perception::ChordReading {
                        root_pc: pc,
                        quality: Some(q),
                        label: label.to_owned(),
                        bass_pc: None,
                        confidence: 0.8,
                    }),
                    ..brain::perception::PerceptionSnapshot::EMPTY
                }
            };
            let unresolved = brain::perception::PerceptionSnapshot {
                hearing_polyphony: true,
                ..brain::perception::PerceptionSnapshot::EMPTY
            };
            chart.observe(&snap(2, brain::theory::ChordQuality::Min7, "Dm7"), 0.0);
            chart.observe(&brain::perception::PerceptionSnapshot::EMPTY, 1.0);
            chart.observe(&snap(2, brain::theory::ChordQuality::Min7, "Dm7"), 1.5);
            chart.observe(&unresolved, 2.5);
            chart.observe(&snap(7, brain::theory::ChordQuality::Dom7, "G7"), 3.0);
            chart.observe(&snap(0, brain::theory::ChordQuality::Maj7, "Cmaj7"), 4.0);
        }
        let dto = explore_progression_impl(&s, 42, FoldWindow::default()).unwrap();
        assert!(
            dto.label.contains("Dm7") && dto.label.contains("G7") && dto.label.contains("Cmaj7"),
            "label: {}",
            dto.label
        );
        // The Dm7-anchored row engraves in the MINOR family — flats, never
        // D major's sharps (the T4a M4 split-brain class, pre-empted).
        assert!(
            dto.staff.fifths < 0,
            "Dm7 anchor must engrave flat-side, got fifths={}",
            dto.staff.fifths
        );
        // Stacked cells on the staff, three per key.
        let first_beat = dto.staff.notes[0].start_beat;
        assert!(
            dto.staff
                .notes
                .iter()
                .filter(|n| n.start_beat == first_beat)
                .count()
                >= 3,
            "stacked cells"
        );
        assert!(s.active_explore.lock().unwrap().is_some());
        let rows = s.session_store.lock().unwrap().list_exercise_log().unwrap();
        assert!(rows.iter().any(|r| r.source == "progression_lift"));
    }

    /// Review MF2: the chart-shaping contract, pinned against its three
    /// surviving mutations — A→B→A keeps its return chord (only
    /// CONSECUTIVE dupes collapse), a 5-chord chart lifts its TRAILING 4,
    /// and one repeated chord (dedup → 1 distinct) refuses calmly.
    #[test]
    fn chart_shaping_keeps_returns_trails_and_refuses() {
        let snap = |pc: u8, q: brain::theory::ChordQuality| brain::perception::PerceptionSnapshot {
            chord: Some(brain::perception::ChordReading {
                root_pc: pc,
                quality: Some(q),
                label: format!("pc{pc}"),
                bass_pc: None,
                confidence: 0.8,
            }),
            ..brain::perception::PerceptionSnapshot::EMPTY
        };
        use brain::theory::ChordQuality as Q;

        // A→B→A: the return chord survives.
        let s = state();
        {
            let mut chart = s.chord_chart.lock().unwrap();
            chart.observe(&snap(0, Q::Maj), 0.0);
            chart.observe(&snap(5, Q::Maj), 1.0);
            chart.observe(&snap(0, Q::Maj), 2.0);
        }
        let dto = explore_progression_impl(&s, 42, FoldWindow::default()).unwrap();
        let steps = s
            .active_explore
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .spec
            .progression
            .clone()
            .unwrap();
        assert_eq!(steps.len(), 3, "A-B-A keeps the return: {}", dto.label);
        assert_eq!(steps[2].offset, 0, "…back to the anchor");

        // 5 distinct chords: the TRAILING 4 lift (anchor = 2nd played).
        let s = state();
        {
            let mut chart = s.chord_chart.lock().unwrap();
            for (i, pc) in [0u8, 2, 4, 5, 7].iter().enumerate() {
                chart.observe(&snap(*pc, Q::Maj), i as f64);
            }
        }
        explore_progression_impl(&s, 42, FoldWindow::default()).unwrap();
        let steps = s
            .active_explore
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .spec
            .progression
            .clone()
            .unwrap();
        assert_eq!(steps.len(), 4, "capped at 4");
        assert_eq!(
            steps.iter().map(|st| st.offset).collect::<Vec<_>>(),
            [0, 2, 3, 5],
            "the trailing 4 (anchor D): D E F G"
        );

        // A FLAT progression's label survives a chip tap flat (review r2:
        // regeneration used the sharp-only label path — "Ebm7" re-read
        // "D#m7" one tap in while the staff stayed correctly flat).
        let s = state();
        {
            let mut chart = s.chord_chart.lock().unwrap();
            chart.observe(&snap(3, Q::Min7), 0.0); // Ebm7
            chart.observe(&snap(8, Q::Dom7), 1.0); // Ab7
            chart.observe(&snap(1, Q::Maj7), 2.0); // Dbmaj7
        }
        let dto = explore_progression_impl(&s, 42, FoldWindow::default()).unwrap();
        assert!(dto.label.contains("Ebm7"), "lift label: {}", dto.label);
        let bumped = apply_variation_delta_impl(
            &s,
            VariationDelta::BumpDifficulty { by: 1 },
            FoldWindow::default(),
        )
        .unwrap();
        assert!(
            bumped.label.contains("Ebm7") && !bumped.label.contains('#'),
            "the label stays flat through a chip tap: {}",
            bumped.label
        );
        let shuffled =
            apply_variation_delta_impl(&s, VariationDelta::ReshuffleRoots, FoldWindow::default())
                .unwrap();
        assert!(
            !shuffled.label.contains('#'),
            "…and through a reshuffle: {}",
            shuffled.label
        );

        // One chord re-struck around silence: 1 distinct → calm refusal.
        let s = state();
        {
            let mut chart = s.chord_chart.lock().unwrap();
            chart.observe(&snap(2, Q::Min7), 0.0);
            chart.observe(&brain::perception::PerceptionSnapshot::EMPTY, 1.0);
            chart.observe(&snap(2, Q::Min7), 2.0);
        }
        assert!(
            explore_progression_impl(&s, 42, FoldWindow::default())
                .unwrap_err()
                .contains("play a couple of chords"),
            "one distinct chord refuses"
        );
    }

    /// #349 T4a end-to-end at the command layer: tapping a heard chord in
    /// the jam lane rows it as stacked block cells through the SAME explore
    /// machinery as a lifted lick — active exploration installed, staff
    /// carries the stacks, exercise log gets a jam_bridge row. Fails if the
    /// bridge command or the stacked handoff breaks.
    #[test]
    fn a_tapped_jam_chord_rows_as_stacked_cells() {
        let s = state();
        let dto = explore_chord_impl(
            &s,
            10,
            brain::theory::ChordQuality::Dom13,
            42,
            FoldWindow::default(),
        )
        .unwrap();
        assert!(
            dto.label.contains("block chords"),
            "deals blocks: {}",
            dto.label
        );
        assert!(
            dto.label.starts_with("Bb"),
            "flat-family spelling: {}",
            dto.label
        );
        assert!(!dto.root_pitch_classes.is_empty());
        assert_eq!(dto.root_pitch_classes[0], 10, "rooted where it was heard");
        // The staff really carries simultaneities (stacked dots share beats).
        let first_beat = dto.staff.notes[0].start_beat;
        assert!(
            dto.staff
                .notes
                .iter()
                .filter(|n| n.start_beat == first_beat)
                .count()
                >= 3,
            "stacked cell on the staff"
        );
        assert!(
            s.active_explore.lock().unwrap().is_some(),
            "exploration is live — same view swap as lift"
        );
        // Logged for the self-improvement loop, honestly sourced.
        let rows = s.session_store.lock().unwrap().list_exercise_log().unwrap();
        assert!(
            rows.iter().any(|r| r.source == "jam_bridge"),
            "exercise log carries the bridge row"
        );
    }

    /// #349 T4a review M4-r2: the RENDERED staff follows the chord family —
    /// a heard Cm7 engraves flat-side (Eb/Bb under a flat signature), never
    /// D#/A# under fifths=0. The Bb-rooted test masks this (its plain major
    /// is already flat); C minor is the trap case.
    #[test]
    fn a_tapped_minor_chord_engraves_flat_side() {
        let s = state();
        let dto = explore_chord_impl(
            &s,
            0,
            brain::theory::ChordQuality::Min7,
            42,
            FoldWindow::default(),
        )
        .unwrap();
        assert!(
            dto.staff.fifths < 0,
            "C minor 7 must engrave under flats, got fifths={}",
            dto.staff.fifths
        );
        assert!(
            !dto.staff.notes.iter().any(|n| n.accidental == Some(1)),
            "no sharp accidentals on a C minor stack: {:?}",
            dto.staff.notes
        );
        assert!(dto.label.starts_with('C'), "label: {}", dto.label);
    }

    /// #255: the explore loop end-to-end at the command layer — start seeds a
    /// variation from the live key (chips + cells + engraved XML), a tapped
    /// delta mutates the rep, and apply-without-start errs calmly. Fails if
    /// the state machine or DTO assembly breaks.
    #[test]
    fn explore_lifecycle_starts_mutates_and_guards() {
        let s = state();
        assert!(
            apply_variation_delta_impl(&s, VariationDelta::ReshuffleRoots, FoldWindow::default())
                .is_err(),
            "no exploration yet → calm error"
        );

        let dto = start_explore_variation_impl(&s, 7, "Dorian", 42, FoldWindow::default()).unwrap();
        assert!(dto.label.contains("Dorian"), "got {}", dto.label);
        assert!(dto.music_xml.contains("<score-partwise"));
        assert_eq!(dto.chips.len(), 5, "the stable five (#445-4)");
        assert!(!dto.root_pitch_classes.is_empty());

        let next =
            apply_variation_delta_impl(&s, VariationDelta::ToggleDirection, FoldWindow::default())
                .unwrap();
        assert_ne!(next.music_xml, dto.music_xml, "a delta produces a new rep");
        assert_eq!(next.chips.len(), 5);

        // #292 slice 3: chips and edits are both undo-able steps; an edit
        // bakes the cell and undo restores the exact prior rep.
        assert!(next.can_undo, "the chip itself is an undo-able step");
        let edited = edit_explore_note_impl(
            &s,
            0,
            brain::coach::NoteEdit::Octaves { by: 1 },
            FoldWindow::default(),
        )
        .unwrap();
        assert!(edited.can_undo);
        assert_ne!(edited.staff, next.staff, "the edit changes the staff");
        let undone = undo_explore_edit_impl(&s, FoldWindow::default()).unwrap();
        assert_eq!(undone.staff, next.staff, "undo restores the prior rep");
        // One more undo steps back over the CHIP to the very first rep…
        let back_to_start = undo_explore_edit_impl(&s, FoldWindow::default()).unwrap();
        assert_eq!(back_to_start.staff, dto.staff, "chips are undo-able too");
        // …and only then is history exhausted.
        assert!(
            undo_explore_edit_impl(&s, FoldWindow::default()).is_err(),
            "history exhausted"
        );
    }

    /// #335 at the explore surface: a C#-major exploration engraves 5 flats,
    /// so its label must open with the same "Db" the first root cell shows —
    /// the header and the cells may never spell the root differently.
    #[test]
    fn explore_label_speaks_the_signature_spelling() {
        let s = state();
        let dto = start_explore_variation_impl(&s, 1, "major", 42, FoldWindow::default()).unwrap();
        assert!(
            dto.music_xml.contains("<fifths>-5</fifths>"),
            "pc-1 major engraves the conventional Db signature"
        );
        let head = dto.label.split(' ').next().unwrap();
        assert_eq!(
            head, dto.root_names[0],
            "label head and first root cell must agree, got {:?} vs {:?}",
            dto.label, dto.root_names
        );
        assert!(
            !dto.label.contains("C#"),
            "no C# over a flat signature, got {:?}",
            dto.label
        );
        // The engraved score title carries the same spelling…
        assert!(
            dto.music_xml.contains("<work-title>Db"),
            "score title must speak the flat spelling too"
        );
        // …and the exercise log records the label the player actually saw.
        let log = s
            .session_store
            .lock()
            .unwrap()
            .list_exercise_log()
            .expect("log reads");
        assert_eq!(
            log.last().unwrap().label,
            dto.label,
            "exercise log must record the displayed label"
        );
    }

    /// #277: the drill's MusicXML engraves the drill's real key signature —
    /// a Bb-major drill carries <fifths>-2</fifths> (and thus flat spelling),
    /// not the old hardcoded C major. Fails if drill_dto stops threading
    /// key_signature_for.
    #[test]
    fn drill_dto_engraves_the_drill_key_signature() {
        let drill = brain::coach::build_first(
            &brain::coach::LessonSpec {
                seed: 1,
                drill_count: 4,
                start_difficulty: 0,
                polyphonic: false,
                grand_staff: false,
            },
            &{
                // Practice every tonic except Bb so the picker trains Bb (10).
                let mut m = brain::learner::LearnerModel::default();
                for t in (0..12u8).filter(|&t| t != 10) {
                    m = brain::learner::apply_drill_result(
                        &m,
                        &brain::learner::DrillResult {
                            tonic: t,
                            mode: "major".to_owned(),
                            accuracy: 1.0,
                        },
                        i64::from(t),
                    );
                }
                m
            },
        );
        assert_eq!(drill.tonic, 10, "picker should choose the unpracticed Bb");
        let dto = drill_dto(&drill, 4, false);
        assert!(
            dto.music_xml.contains("<fifths>-2</fifths>"),
            "Bb-major drill must engrave 2 flats, got fifths line: {:?}",
            dto.music_xml.lines().find(|l| l.contains("<fifths>"))
        );
    }

    /// Starting a lesson while one is running is refused (a double-tap of the
    /// button must not silently discard the in-flight lesson).
    #[test]
    fn double_start_lesson_is_refused() {
        let s = state();
        start_lesson_impl(&s, 4, false, false, FoldWindow::default()).unwrap();
        assert!(matches!(
            start_lesson_impl(&s, 5, false, false, FoldWindow::default()),
            Err(CommandError::LessonActive)
        ));
    }

    /// #254 review M1: a lesson cannot outlive its practice session — ending
    /// the session finalizes the lesson (completed drills keep credit, state
    /// clears), so the next session can't be silently mis-graded by a stale
    /// phrase mark.
    #[tokio::test]
    async fn ending_the_session_finalizes_the_lesson() {
        let s = state();
        start_practice_session_impl(&s, "Trumpet".to_owned(), PracticeMode::Practice, true, None)
            .await
            .expect("session starts");
        start_lesson_impl(&s, 6, false, false, FoldWindow::default()).unwrap();
        play_current_drill_perfectly(&s);
        submit_drill_impl(&s, 100, FoldWindow::default()).unwrap();

        end_practice_session_impl(&s).await.expect("session ends");
        assert!(
            s.active_lesson.lock().unwrap().is_none(),
            "lesson must not survive the session"
        );
        let model = s
            .session_store
            .lock()
            .unwrap()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .expect("completed drill persisted at session end");
        assert_eq!(model.key_mastery.len(), 1);
    }

    #[test]
    fn taste_profile_cold_start_returns_empty_default() {
        let state = state();
        let got = get_taste_profile_impl(&state).expect("cold start must succeed");
        assert_eq!(
            got,
            TasteProfile::default(),
            "no captured profile yet must read back as the empty default"
        );
    }

    #[test]
    fn taste_profile_set_then_get_roundtrips_through_commands() {
        use brain::store::ExperienceLevel;
        let state = state();
        let profile = TasteProfile {
            genres: vec!["hip-hop".to_owned(), "film score".to_owned()],
            artists: vec!["Kendrick Lamar".to_owned()],
            goals: vec!["audition prep".to_owned()],
            experience: ExperienceLevel::Advanced,
            is_under_13: false,
        };
        set_taste_profile_impl(&state, profile.clone()).expect("set must succeed");

        let got = get_taste_profile_impl(&state).expect("get must succeed");
        assert_eq!(got, profile, "command round-trip must preserve every field");
    }

    #[test]
    fn taste_profile_set_overwrites_prior_via_command() {
        use brain::store::ExperienceLevel;
        let state = state();
        set_taste_profile_impl(
            &state,
            TasteProfile {
                genres: vec!["jazz".to_owned()],
                ..TasteProfile::default()
            },
        )
        .unwrap();
        let edited = TasteProfile {
            genres: vec!["classical".to_owned()],
            experience: ExperienceLevel::Intermediate,
            is_under_13: true,
            ..TasteProfile::default()
        };
        set_taste_profile_impl(&state, edited.clone()).unwrap();

        assert_eq!(get_taste_profile_impl(&state).unwrap(), edited);
    }

    /// Poison `m` the only way std allows: panic while holding the guard.
    fn poison<T>(m: &std::sync::Mutex<T>) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = m.lock().unwrap();
            panic!("deliberate poison (#246)");
        }));
    }

    /// #246: the helper must hand back the last-written data after a panic
    /// under the lock, not propagate the poison. Fails if `lock_or_recover`
    /// ever reverts to `.expect`/`?` semantics.
    #[test]
    fn lock_or_recover_yields_data_after_poison() {
        let m = std::sync::Mutex::new(41);
        *m.lock_or_recover() += 1;
        poison(&m);
        assert!(m.lock().is_err(), "the mutex must really be poisoned");
        assert_eq!(
            *m.lock_or_recover(),
            42,
            "recovery must return the last-written value"
        );
        *m.lock_or_recover() += 1;
        assert_eq!(*m.lock_or_recover(), 43, "the mutex must stay usable");
    }

    /// #246: a panic that poisons the session-store mutex must degrade the
    /// history/taste commands to their normal behavior — the store's data is
    /// still valid — instead of turning every later query into a crash.
    /// Fails (by panicking) if any of these paths reverts to
    /// `.lock().expect("… poisoned")`.
    #[test]
    fn poisoned_session_store_degrades_commands_instead_of_crashing() {
        let s = state();
        let profile = TasteProfile {
            genres: vec!["jazz".to_owned()],
            ..TasteProfile::default()
        };
        set_taste_profile_impl(&s, profile.clone()).unwrap();

        poison(&s.session_store);
        assert!(s.session_store.lock().is_err(), "store must be poisoned");

        assert_eq!(
            get_taste_profile_impl(&s).expect("read after poison must succeed"),
            profile,
            "the profile written before the poison must still be served"
        );
        let history = get_session_history_impl(&s, None, None, None)
            .expect("history after poison must succeed");
        assert!(history.is_empty(), "no sessions were recorded");
        let edited = TasteProfile {
            genres: vec!["salsa".to_owned()],
            ..TasteProfile::default()
        };
        set_taste_profile_impl(&s, edited.clone()).expect("write after poison must succeed");
        assert_eq!(
            get_taste_profile_impl(&s).unwrap(),
            edited,
            "writes after the poison must land, not be silently dropped"
        );
    }

    /// #246: a poisoned phrase buffer at session end must not erase the take.
    /// The pre-fix code (`.lock().map(…).unwrap_or_default()`) silently
    /// returned zero phrases here — the recap claimed "you didn't play" even
    /// though the buffer held the whole session.
    #[tokio::test]
    async fn end_session_keeps_worker_phrases_after_buffer_poison() {
        let s = state();
        start_practice_session_impl(&s, "Voice".to_owned(), PracticeMode::Practice, false, None)
            .await
            .unwrap();
        {
            let mut buf = s.phrase_buffer.lock().unwrap();
            buf.push(sample_phrase());
            buf.push(sample_phrase());
        }
        poison(&s.phrase_buffer);

        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(
            recap.phrase_count, 2,
            "phrases buffered before the poison must reach the recap"
        );
    }

    /// #417-4 review MF1: the REAL end-session path — session start with
    /// "Piano", offline policy, panicking HTTP client — must produce a
    /// keyboard-vocabulary recap. This is the threading pin: it fails if
    /// `end_practice_session_impl` stops resolving the family (commands
    /// call site), if `generate_recap_with_context` stops writing it into
    /// the input (session.rs), or if the resolver maps Piano wrongly.
    #[tokio::test]
    async fn ending_a_piano_session_threads_the_family_into_the_recap() {
        let mut s = state();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;
        start_practice_session_impl(&s, "Piano".to_owned(), PracticeMode::Practice, false, None)
            .await
            .expect("start should succeed");
        {
            // #445-6b: three settled phrases clear the thin bar — this
            // test pins FULL-recap family vocabulary through the real
            // end-session path.
            let mut guard = s.active_session.lock().await;
            for _ in 0..3 {
                let mut p = sample_phrase();
                p.duration_secs = 7.0;
                guard.as_mut().unwrap().recorder.record_phrase(p).unwrap();
            }
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(recap.instrument, "Piano");
        let text = format!(
            "{} {} {} {}",
            recap.overall_assessment,
            recap.strengths.join(" "),
            recap.areas_to_improve.join(" "),
            recap.next_session_suggestions.join(" ")
        )
        .to_lowercase();
        for forbidden in ["tuner", "drone", "long tones"] {
            assert!(
                !text.contains(forbidden),
                "the real end-session path must route the family: {text}"
            );
        }
        assert!(
            text.contains("slow scale"),
            "keyboard vocabulary through the real path: {text}"
        );
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
    async fn end_session_persists_the_completed_session() {
        // Regression guard: a completed (non-empty) session must be written to
        // the store so practice history, the stats surface, and opt-in cloud
        // sync have data. Before this was wired, `end_practice_session` built
        // the recap and dropped it — the `sessions` table stayed empty forever.
        let s = state();
        assert_eq!(
            s.session_store.lock().unwrap().count_sessions().unwrap(),
            0,
            "store must start empty"
        );

        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");

        // Feed the recorder a phrase so it completes via the rich-recap branch
        // (the empty-state path is a separate decision — see follow-up).
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

        let store = s.session_store.lock().unwrap();
        assert_eq!(
            store.count_sessions().unwrap(),
            1,
            "the completed session must be persisted"
        );
        let recent = store.list_recent(10).unwrap();
        assert_eq!(recent.len(), 1, "exactly one persisted session");
        assert_eq!(recent[0].instrument, recap.instrument);
        assert_eq!(recent[0].phrase_count, recap.phrase_count);
    }

    #[tokio::test]
    async fn end_session_persists_per_phrase_metrics() {
        // The session row only keeps a phrase *count*; the raw per-phrase data
        // (pitch/cents/timing) must also persist so a user-reported issue can
        // be debugged after the fact, not just summarized (#197).
        let s = state();
        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");

        let recorded = sample_phrase();
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(recorded.clone())
                .unwrap();
        }

        end_practice_session_impl(&s).await.unwrap();

        let store = s.session_store.lock().unwrap();
        let recent = store.list_recent(1).unwrap();
        assert_eq!(recent.len(), 1);
        let phrases = store.load_phrases(recent[0].id).unwrap();
        assert_eq!(phrases.len(), 1, "the session's phrase must be persisted");
        assert_eq!(phrases[0].note_count, recorded.note_count);
    }

    #[tokio::test]
    async fn end_session_records_session_debug_metadata() {
        // A completed session's row carries practice mode + app version. This
        // free-play path has no score, so score_id stays None (#201).
        let s = state();
        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }
        end_practice_session_impl(&s).await.unwrap();

        let store = s.session_store.lock().unwrap();
        let recent = store.list_recent(1).unwrap();
        let meta = store.session_meta(recent[0].id).unwrap();
        assert_eq!(meta.practice_mode.as_deref(), Some("Practice"));
        assert!(meta.app_version.is_some(), "app version must be recorded");
        assert!(meta.score_id.is_none(), "free-play session has no score id");
    }

    /// A pure sine tone — deterministic, embeddable audio for the offline
    /// idiom path. No mic, no network.
    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[tokio::test]
    async fn end_session_surfaces_offline_idiom_match_when_audio_is_relevant() {
        // End-to-end of the offline idiom wiring: seed the session-scoped idiom
        // buffer (as the worker thread would), end the session, and confirm a
        // confidence-gated match is surfaced on the recap — all on-device, with
        // no HttpClient (with_mocks wires no real LLM and no network).
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
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }

        // Seed the buffer with several exemplar-like tones; the bundled corpus
        // is built from baseline-embedded tones, so at least one clears the gate.
        for &f in &[196.0, 261.63, 329.63, 392.0, 440.0, 523.25] {
            s.idiom_buffer.clear();
            s.idiom_buffer
                .append_downsampled(&sine(f, 1.0, 44_100), 44_100);
            // Peek: does this tone produce a match offline?
            let (samples, rate) = s.idiom_buffer.snapshot();
            if !brain::idiom_recap::analyze_idioms(&samples, rate).is_empty() {
                break;
            }
        }

        let recap = end_practice_session_impl(&s).await.unwrap();
        assert!(
            !recap.idiom_notes.is_empty(),
            "an idiom-relevant session should surface a gated match"
        );
        for m in &recap.idiom_notes {
            assert!(
                m.similarity >= brain::idiom_recap::IDIOM_SIMILARITY_THRESHOLD,
                "surfaced matches must clear the confidence gate: {m:?}"
            );
        }
        // Buffer is cleared after the recap so audio isn't retained.
        assert!(s.idiom_buffer.snapshot().0.is_empty());
    }

    #[tokio::test]
    async fn end_session_with_quiet_audio_surfaces_no_idiom() {
        // Silence / no captured audio → no idiom notes ("silence > lies").
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
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }
        // Buffer left empty (no audio captured).
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert!(
            recap.idiom_notes.is_empty(),
            "no captured audio must yield no idiom notes"
        );
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

    #[test]
    fn empty_state_recap_is_duration_aware() {
        // A real session (heard time) must NOT say "you didn't play"; it shows a
        // calm mic-check note and reports the real duration + instrument (#185).
        let heard = empty_state_recap(75.0, "Voice".to_owned());
        assert_eq!(heard.phrase_count, 0);
        assert_eq!(heard.duration_secs, 75.0);
        assert_eq!(heard.instrument, "Voice");
        assert!(
            !heard.overall_assessment.contains("didn't get to play"),
            "a minute-long session must not read as 'didn't play': {}",
            heard.overall_assessment
        );

        // A blink-and-gone session keeps the gentle "come back" copy.
        let idle = empty_state_recap(2.0, String::new());
        assert!(idle.overall_assessment.contains("didn't get to play"));
        assert_eq!(idle.duration_secs, 2.0);
    }

    #[tokio::test]
    async fn end_session_records_worker_buffered_phrases_into_the_recap() {
        // #185 root cause: phrases the audio worker detects must reach the
        // recorder so the recap reflects them. They used to be emitted only to
        // the UI, so the recorder stayed empty and EVERY recap was the empty
        // "you didn't play" state. Simulate the worker buffering phrases (what
        // the live phrase callback does) and confirm they land in the recap.
        let s = state();
        start_practice_session_impl(&s, "Voice".to_owned(), PracticeMode::Practice, false, None)
            .await
            .unwrap();
        {
            let mut buf = s.phrase_buffer.lock().unwrap();
            buf.push(sample_phrase());
            buf.push(sample_phrase());
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(
            recap.phrase_count, 2,
            "worker-detected phrases must populate the recap, not be dropped on the floor"
        );
        assert_eq!(s.current_phase().await, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn end_session_with_no_phrases_carries_instrument_and_does_not_error() {
        let s = state();
        start_practice_session_impl(&s, "Voice".to_owned(), PracticeMode::Practice, false, None)
            .await
            .unwrap();
        // No phrase recorded — the recorder is empty.
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(recap.phrase_count, 0);
        // The empty path now carries the real instrument (was blank before).
        assert_eq!(recap.instrument, "Voice");
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

    /// #349 T4a (test-auditor gap): the chart survives session END — the
    /// store fetches it after end_practice_session returns, so a tidy-
    /// minded clear at session end would silently kill the recap sketch.
    /// Cleared only at the NEXT session start (glue in the pipeline block;
    /// the lifecycle contract is pinned here at the readable half).
    #[tokio::test]
    async fn the_chord_chart_survives_session_end() {
        let s = state();
        start_practice_session_impl(&s, "Piano".to_owned(), PracticeMode::Practice, false, None)
            .await
            .unwrap();
        {
            let mut chart = s.chord_chart.lock().unwrap();
            chart.observe(
                &brain::perception::PerceptionSnapshot {
                    chord: Some(brain::perception::ChordReading {
                        root_pc: 0,
                        quality: Some(brain::theory::ChordQuality::Maj),
                        label: "C".to_owned(),
                        bass_pc: None,
                        confidence: 0.8,
                    }),
                    ..brain::perception::PerceptionSnapshot::EMPTY
                },
                1.0,
            );
        }
        end_practice_session_impl(&s).await.unwrap();
        let entries = s.chord_chart.lock().unwrap().entries().to_vec();
        assert_eq!(entries.len(), 1, "the recap sketch's data must survive");
        assert_eq!(entries[0].label, "C");
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
        let list = list_instruments_impl(&s).expect("healthy state must list the catalog");
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
        let loaded = load_instrument_catalog(None).expect("workspace profiles/ must load");
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

    /// #364 AC: a missing profiles dir (the packaged-build failure of #112)
    /// is an `Err` naming the directory it tried — never a panic. The
    /// message is what the selector screen shows, so it must say where the
    /// app looked and how to override.
    #[test]
    fn catalog_load_errors_on_missing_dir() {
        let dir = std::path::Path::new("/nonexistent/amc-profiles-364");
        let err = load_catalog_from(dir).expect_err("missing dir must be an error, not a panic");
        assert!(
            err.contains("/nonexistent/amc-profiles-364"),
            "error must name the directory it tried: {err}"
        );
        assert!(
            err.contains("AI_MUSIC_COMPANION_PROFILES_DIR"),
            "error must mention the override hook: {err}"
        );
    }

    /// #364 AC: a present-but-empty profiles dir is also an `Err` (an empty
    /// selector grid with no explanation was the exact failure the old
    /// panic guarded against — the guard survives, the crash doesn't).
    #[test]
    fn catalog_load_errors_on_empty_dir() {
        let dir = std::env::temp_dir().join(format!("amc-empty-profiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let result = load_catalog_from(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let err = result.expect_err("empty dir must be an error, not a panic");
        assert!(
            err.contains("no instrument profiles found"),
            "error must say the dir was empty: {err}"
        );
    }

    /// #364 AC: when startup degraded (catalog missing), `list_instruments`
    /// surfaces the stored reason as its IPC error — the selector's
    /// existing catch renders it, so the user sees why the grid is empty.
    #[test]
    fn list_instruments_surfaces_catalog_error() {
        let mut s = AppState::with_mocks();
        s.instruments = Arc::new(Vec::new());
        s.catalog_error = Some("failed to load instrument profiles from /bundle/profiles".into());
        let err = list_instruments_impl(&s).expect_err("degraded catalog must err");
        assert!(
            err.contains("/bundle/profiles"),
            "IPC error must carry the load failure verbatim: {err}"
        );
    }

    /// #364 structural guard: an empty catalog errs even when no load
    /// failure was recorded. `Ok([])` renders as a silent empty grid — the
    /// exact no-feedback state the old startup panic existed to prevent —
    /// so it must be unreachable regardless of how the state got here
    /// (e.g. a future regression in `build`'s degraded-load wiring).
    #[test]
    fn empty_catalog_errs_even_without_recorded_failure() {
        let mut s = AppState::with_mocks();
        s.instruments = Arc::new(Vec::new());
        s.catalog_error = None;
        let err = list_instruments_impl(&s).expect_err("empty catalog must never be Ok([])");
        assert!(
            err.contains("no instrument profiles are loaded"),
            "error must explain the empty grid: {err}"
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
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
            score_span: None,
            verdicts: None,
            score_card: None,
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

    // -----------------------------------------------------------------------
    // #471-4 H4 — per-instrument fold windows (profile → MIDI derivation,
    // the voice exemption, and the wire's honesty about fallbacks). Table:
    // docs/specs/471-h4-instrument-ranges.md §3.
    // -----------------------------------------------------------------------

    /// H4 AC1: the trumpet window derives from its profile (165–1047 Hz) as
    /// exactly E3..C6 — the 5-cent snap recovers the low E that 165.0's
    /// integer rounding pushed to midi 52.02, and the top C holds. The rest
    /// of the catalog is pinned so a profile edit re-derives consciously.
    #[test]
    fn trumpet_window_derives_from_its_profile() {
        let s = state();
        assert_eq!(
            fold_window_for(&s, "Trumpet"),
            FoldWindow { lo: 52, hi: 84 }
        );
        // The full shipped-catalog table (voice excluded — its own pin below).
        for (name, lo, hi) in [
            ("Cello", 36, 83),
            ("Clarinet", 50, 91),
            ("Flute", 60, 96),
            ("French Horn", 41, 81),
            ("Guitar", 40, 88),
            ("Piano", 22, 108),
            ("Trombone", 34, 74),
            ("Violin", 55, 103),
        ] {
            assert_eq!(fold_window_for(&s, name), FoldWindow { lo, hi }, "{name}");
        }
    }

    /// H4 voice exemption (founder: "not vocals tho, leave that be"): Voice
    /// resolves to the DEFAULT window even though its profile carries a
    /// narrower frequency range — the family gate short-circuits before any
    /// Hz math. Fails if a refactor starts constraining singers.
    #[test]
    fn voice_is_exempt_from_range_windows() {
        let s = state();
        assert_eq!(fold_window_for(&s, "Voice"), FoldWindow::default());
        // The exemption is the FAMILY gate, not a degenerate range: the
        // same numbers under a non-voice family would derive a real window.
        assert_eq!(
            fold_window_from_hz(82.0, 1047.0),
            Some(FoldWindow { lo: 40, hi: 84 })
        );
    }

    /// H4: unknown instruments (and degenerate/hostile ranges) resolve to
    /// the default window — never a crash, never a made-up range.
    #[test]
    fn unknown_instrument_gets_the_default_window() {
        let s = state();
        assert_eq!(fold_window_for(&s, "Kazoo"), FoldWindow::default());
        assert_eq!(
            fold_window_from_hz(0.0, 440.0),
            None,
            "log2(0) is not a window"
        );
        assert_eq!(fold_window_from_hz(880.0, 440.0), None, "inverted range");
    }

    /// H4 AC4 (wire half): a row that can't fit the session instrument is
    /// dealt in the full window AND says so — the ExploreDto carries the
    /// calm range notice; a fitting row carries none. Fails if the fallback
    /// goes silent (display dishonesty) or a fitting row nags.
    #[test]
    fn cant_fit_fallback_surfaces_the_calm_range_notice() {
        let s = state();
        let trumpet = fold_window_for(&s, "Trumpet");
        let model = brain::learner::LearnerModel::default();
        // Span 40 > the trumpet's 32-wide window: no key can fit.
        let (wide_state, wide_seq) = brain::coach::start_explore_cell_windowed(
            vec![0, -20, 20],
            0,
            &model,
            5,
            brain::coach::DirectionMode::Forward,
            trumpet,
        );
        let dto = explore_dto(&wide_state, &wide_seq);
        let notice = dto.range_notice.expect("the fallback must be surfaced");
        assert!(notice.contains("range"), "got: {notice}");
        // A narrow cell fits every key: in-window notes, no notice.
        let (fit_state, fit_seq) = brain::coach::start_explore_cell_windowed(
            vec![0, 4, 7],
            0,
            &model,
            5,
            brain::coach::DirectionMode::Forward,
            trumpet,
        );
        assert!(fit_seq
            .target_midi
            .iter()
            .all(|&m| (trumpet.lo..=trumpet.hi).contains(&m)));
        assert_eq!(explore_dto(&fit_state, &fit_seq).range_notice, None);
    }

    /// H4 AC5: recall through the WIRE replays the stored opener
    /// bit-identically under the same instrument window, twice over — the
    /// stored artifact stays instrument-agnostic and the session window
    /// re-registers it deterministically at replay time.
    #[test]
    fn stored_seed_recall_replays_identically_under_the_same_window() {
        let s = state();
        let trumpet = fold_window_for(&s, "Trumpet");
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 3, 5, 8],
        }];
        let begun = opener_impl(&s, &items, Some(2), Some("forward"), true, trumpet)
            .expect("opener begins");
        let first = begin_opener_recall_impl(&s, trumpet).expect("recall replays");
        let second = begin_opener_recall_impl(&s, trumpet).expect("recall chains");
        assert_eq!(first.music_xml, begun.music_xml, "recall = the begun rep");
        assert_eq!(first.music_xml, second.music_xml, "recall is stable");
        assert_eq!(first.staff, second.staff);
    }

    /// H4 review round 1 MF1a: `session_fold_window` — the resolver every
    /// async command wrapper trusts — actually follows the live session.
    /// Idle → default; a Trumpet session → the trumpet's derived window.
    /// Fails if the resolver stops reading the active session's instrument.
    #[tokio::test]
    async fn session_fold_window_follows_the_active_session_instrument() {
        let s = state();
        assert_eq!(
            session_fold_window(&s).await,
            FoldWindow::default(),
            "no session → the default window"
        );
        start_practice_session_impl(&s, "Trumpet".to_owned(), PracticeMode::Practice, true, None)
            .await
            .expect("session starts");
        assert_eq!(
            session_fold_window(&s).await,
            FoldWindow { lo: 52, hi: 84 },
            "a live Trumpet session resolves the trumpet window"
        );
    }

    /// H4 review round 1 MF1a: the voice exemption holds through the LIVE
    /// resolver too — a Voice session resolves the default window, not a
    /// derived one. Fails if the family gate is bypassed on the session path.
    #[tokio::test]
    async fn session_fold_window_voice_session_keeps_the_default() {
        let s = state();
        start_practice_session_impl(&s, "Voice".to_owned(), PracticeMode::Practice, false, None)
            .await
            .expect("session starts");
        assert_eq!(session_fold_window(&s).await, FoldWindow::default());
    }

    /// H4 review round 1 MF1b: the threading is not trust-based — through
    /// the REAL `apply_variation_delta` command wrapper (mock Tauri app,
    /// managed AppState, live Trumpet session), a wide-cell exploration
    /// comes back with the range notice, proving the wrapper resolved the
    /// SESSION's window rather than defaulting. Kills the "wrapper passes
    /// FoldWindow::default()" mutant that the impl-level tests let survive.
    #[tokio::test]
    async fn an_active_trumpet_session_constrains_the_wrapper_path() {
        use tauri::Manager;
        let app = tauri::test::mock_app();
        app.manage(state());
        let s = app.state::<AppState>();
        start_practice_session_impl(
            s.inner(),
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            true,
            None,
        )
        .await
        .expect("session starts");
        // Seed a wide-cell exploration (span 40 > the trumpet's 32-wide
        // window — no key can fit).
        let (explore, _) = brain::coach::start_explore_cell(
            vec![0, -20, 20],
            0,
            &brain::learner::LearnerModel::default(),
            5,
            brain::coach::DirectionMode::Forward,
        );
        *s.active_explore.lock_or_recover() = Some(explore);
        // The genuine wire path: the wrapper must resolve the session window.
        let dto = apply_variation_delta(s.clone(), VariationDelta::ReshuffleRoots)
            .await
            .expect("chip applies");
        assert!(
            dto.range_notice.is_some(),
            "a Trumpet session must constrain the wrapper-path render \
             (fallback surfaced, not silently default-windowed)"
        );
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

    #[test]
    fn teardown_accompaniment_without_running_is_a_noop_and_idempotent() {
        // The band may be torn down by both `stop_accompaniment` and session
        // end, possibly when nothing is playing — it must never panic, and a
        // double teardown is safe. (Starting a real band needs an output device,
        // so the start path is exercised by the manual audible test, not here.)
        let s = state();
        assert!(
            s.accompaniment.lock().unwrap().is_none(),
            "no band should be running on a fresh state"
        );
        s.teardown_accompaniment();
        s.teardown_accompaniment(); // idempotent
        assert!(s.accompaniment.lock().unwrap().is_none());
    }

    #[test]
    fn key_override_persists_and_clears_without_a_running_band() {
        // The override is stored in AppState (applied when a band starts), so the
        // commands must work and persist even with no band playing, and never
        // panic. (Applying to a live band needs a device → covered by the driver
        // unit tests + manual verify.)
        let s = state();
        assert!(s.current_key_override().is_none());

        s.set_key_override(4, true); // E minor
        assert_eq!(s.current_key_override(), Some((4, true)));

        // Re-pin to a different key (e.g. the user picks again).
        s.set_key_override(7, false); // G major
        assert_eq!(s.current_key_override(), Some((7, false)));

        s.clear_key_override();
        assert!(s.current_key_override().is_none());
    }

    #[test]
    fn accompaniment_status_payload_serializes_to_playing_flag() {
        // The frontend reads `{ "playing": bool }`. Pin the field name + boolean
        // so a rename or flipped value fails here (AC10).
        assert_eq!(
            serde_json::to_string(&AccompanimentStatusPayload { playing: true }).unwrap(),
            r#"{"playing":true}"#
        );
        assert_eq!(
            serde_json::to_string(&AccompanimentStatusPayload { playing: false }).unwrap(),
            r#"{"playing":false}"#
        );
    }

    /// #421 S1 AC8 (review MF1): ending the session stops the click — the
    /// command's teardown + emit run BEFORE any device work, so the mock
    /// runtime exercises them. The command errs (no active session); the
    /// pocket event and the emptied slot are the assertions.
    #[tokio::test]
    async fn ending_the_session_stops_the_pocket() {
        use std::sync::Mutex as StdMutex;
        use tauri::test::mock_app;
        use tauri::{Listener, Manager};

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let captured = Arc::new(StdMutex::new(None::<String>));
        let sink = captured.clone();
        app.listen("pocket-status", move |event| {
            *sink.lock().unwrap() = Some(event.payload().to_string());
        });
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        // No active session → the command errs AFTER the teardown+emit.
        let _ = end_practice_session(handle, state.clone()).await;
        let payload = captured
            .lock()
            .unwrap()
            .clone()
            .expect("session end emits pocket-status");
        assert!(payload.contains("\"playing\":false"), "got: {payload}");
        assert!(state.pocket.lock_or_recover().is_none());
    }

    /// #421 S1 review MF2: the pocket-status wire shape — App.tsx reads
    /// payload.tempo_bpm; a serde rename ships a silently-dead pulse.
    #[test]
    fn pocket_status_payload_serializes_verbatim() {
        assert_eq!(
            serde_json::to_string(&PocketStatusPayload {
                playing: true,
                tempo_bpm: 96.0,
            })
            .unwrap(),
            r#"{"playing":true,"tempo_bpm":96.0}"#
        );
    }

    /// #421 S1 review MF4(a): the deterministic HALF of exclusivity — a
    /// pocket start silences the band's status and empties its slot BEFORE
    /// any device work, so this asserts on mock state regardless of whether
    /// the device open succeeds (it may, briefly, on a dev machine — the
    /// paired stop tears it down). Full audible exclusivity: manual verify.
    #[tokio::test]
    async fn starting_the_pocket_reports_the_band_stopped_first() {
        use std::sync::Mutex as StdMutex;
        use tauri::test::mock_app;
        use tauri::{Listener, Manager};

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let captured = Arc::new(StdMutex::new(None::<String>));
        let sink = captured.clone();
        app.listen("accompaniment-status", move |event| {
            *sink.lock().unwrap() = Some(event.payload().to_string());
        });
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        let _ = start_pocket(handle.clone(), state.clone(), 96.0, 4, true).await;
        let payload = captured
            .lock()
            .unwrap()
            .clone()
            .expect("pocket start reports the band stopped");
        assert!(payload.contains("\"playing\":false"), "got: {payload}");
        assert!(state.accompaniment.lock_or_recover().is_none());
        // If a real device opened, close it so the test leaves silence.
        let _ = stop_pocket(handle, state.clone()).await;
        assert!(state.pocket.lock_or_recover().is_none());
    }

    /// #421 S2 review MF5: the clamp seam — a 250 BPM push arrives at
    /// the consumer as 220 (the 220..300 window only THIS clamp covers;
    /// the Metronome itself would apply up to 300).
    #[test]
    fn pushed_tempos_arrive_clamped() {
        use ringbuf::traits::Consumer;
        let (mut tx, mut rx) = ears::output_engine::pocket_tempo_channel(4);
        push_clamped_tempo(&mut tx, 250.0);
        push_clamped_tempo(&mut tx, 20.0);
        push_clamped_tempo(&mut tx, f64::NAN);
        assert_eq!(rx.try_pop(), Some(220.0));
        assert_eq!(rx.try_pop(), Some(40.0));
        assert_eq!(rx.try_pop(), Some(40.0));
    }

    /// #421 S2 AC2: a silent Pocket makes set_pocket_tempo a calm no-op
    /// (no panic, no error) — the follow policy may outlive the click.
    #[tokio::test]
    async fn set_pocket_tempo_is_a_noop_when_silent() {
        use tauri::test::mock_app;
        use tauri::Manager;
        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();
        set_pocket_tempo(state, 96.0).expect("silent no-op");
    }

    /// #445 pt 9 AC2: a silent band makes set_band_tempo a calm no-op —
    /// the mirror of set_pocket_tempo's manners (the follow policy may
    /// outlive the band).
    #[tokio::test]
    async fn set_band_tempo_is_a_noop_when_silent() {
        use tauri::test::mock_app;
        use tauri::Manager;
        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();
        set_band_tempo(state, 96.0).expect("silent no-op");
    }

    /// #445 pt 9 review MF1: the band's clamp seam — a 250 BPM push
    /// arrives on the control channel as 220 (and 20 → 40, NaN → 40),
    /// exactly like `pushed_tempos_arrive_clamped` pins the click's. A
    /// mutant that drops `clamp_pocket_params` from the band path dies
    /// here: the synth would otherwise APPLY the raw value verbatim.
    #[test]
    fn band_tempos_arrive_clamped_at_the_channel() {
        use brain::accompaniment::AccompanimentControl;
        let (sender, mut rx) = accompaniment_control_channel(8);
        let mut driver = AccompanimentDriver::new(sender);
        set_clamped_band_tempo(&mut driver, 250.0);
        set_clamped_band_tempo(&mut driver, 20.0);
        set_clamped_band_tempo(&mut driver, f64::NAN);
        assert_eq!(rx.try_pop(), Some(AccompanimentControl::SetTempo(220.0)));
        assert_eq!(rx.try_pop(), Some(AccompanimentControl::SetTempo(40.0)));
        assert_eq!(rx.try_pop(), Some(AccompanimentControl::SetTempo(40.0)));
        assert_eq!(rx.try_pop(), None);
    }

    /// #445 pt 9 review MF2: the start-time install honours the two-clock
    /// rule — Some (solo) pushes the clamped Pocket tempo, None (room
    /// mode) pushes NOTHING, leaving the band on the legacy listen-and-
    /// join path where the room's players are the clock.
    #[test]
    fn install_band_clock_room_mode_installs_no_override() {
        use brain::accompaniment::AccompanimentControl;
        let (sender, mut rx) = accompaniment_control_channel(8);
        let mut driver = AccompanimentDriver::new(sender);
        install_band_clock(&mut driver, None);
        assert_eq!(rx.try_pop(), None, "room mode must not install a clock");
        install_band_clock(&mut driver, Some(300.0));
        assert_eq!(
            rx.try_pop(),
            Some(AccompanimentControl::SetTempo(220.0)),
            "solo mode installs the CLAMPED set tempo"
        );
    }

    /// #421 S1 AC3: the clamp table — played and reported values agree
    /// because one function produces both.
    #[test]
    fn pocket_params_clamp_to_the_documented_ranges() {
        assert_eq!(clamp_pocket_params(96.0, 4), (96.0, 4));
        assert_eq!(clamp_pocket_params(20.0, 4), (40.0, 4));
        assert_eq!(clamp_pocket_params(300.0, 4), (220.0, 4));
        assert_eq!(clamp_pocket_params(f64::NAN, 4), (40.0, 4));
        assert_eq!(clamp_pocket_params(90.0, 1), (90.0, 2));
        assert_eq!(clamp_pocket_params(90.0, 9), (90.0, 7));
    }

    /// #421 S1: teardown is idempotent and safe on a silent state — it runs
    /// from stop, from band start, and from session end.
    #[test]
    fn teardown_pocket_without_running_is_a_noop_and_idempotent() {
        let s = AppState::with_mocks();
        s.teardown_pocket();
        s.teardown_pocket();
    }

    /// #445 AC7: the click-gate lifecycle. A fresh state gates nothing;
    /// a (manually) installed gate is cleared by `teardown_pocket` — the
    /// same path stop_pocket, band start, and session end all take. And
    /// when `start_pocket` actually opens a device (dev machine), the
    /// gate slot is populated; either way stop leaves it empty.
    #[tokio::test]
    async fn pocket_click_gate_installed_on_start_and_cleared_on_teardown() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let s = AppState::with_mocks();
        assert!(
            s.click_gate.lock_or_recover().is_none(),
            "a fresh state must gate nothing (fail-open)"
        );
        // Seed a gate without a device, then tear down: the slot empties.
        let (_tx, rx) = ears::output_engine::click_fire_channel(8);
        *s.click_gate.lock_or_recover() = Some(crate::audio_pipeline::ClickGate {
            fires: rx,
            epoch: std::time::Instant::now(),
            output_sample_rate: 48_000,
        });
        s.teardown_pocket();
        assert!(
            s.click_gate.lock_or_recover().is_none(),
            "teardown_pocket must clear the click gate"
        );

        // Through the real commands (device-dependent start, like
        // `starting_the_pocket_reports_the_band_stopped_first`).
        let app = mock_app();
        app.manage(AppState::with_mocks());
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        let started = start_pocket(handle.clone(), state.clone(), 96.0, 4, false).await;
        if started.is_ok() {
            assert!(
                state.click_gate.lock_or_recover().is_some(),
                "a playing Pocket must install the click gate"
            );
        }
        let _ = stop_pocket(handle, state.clone()).await;
        assert!(state.click_gate.lock_or_recover().is_none());
    }

    /// #421 S1 AC2 (stop half): stop_pocket reports playing:false through
    /// the real command + mock runtime. (start needs a real output device —
    /// covered by `plays_the_pocket_click_with_count_in` in ears'
    /// output_engine_audible_test [ignored, manual] + the manual-verify
    /// checklist, the same boundary the band's tests drew.)
    #[tokio::test]
    async fn stop_pocket_emits_status_playing_false() {
        use std::sync::Mutex as StdMutex;
        use tauri::test::mock_app;
        use tauri::{Listener, Manager};

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let captured = Arc::new(StdMutex::new(None::<String>));
        let sink = captured.clone();
        app.listen("pocket-status", move |event| {
            *sink.lock().unwrap() = Some(event.payload().to_string());
        });
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        stop_pocket(handle, state)
            .await
            .expect("stop_pocket succeeds on a silent state");
        let payload = captured
            .lock()
            .unwrap()
            .clone()
            .expect("a pocket-status event should have been emitted");
        assert!(payload.contains("\"playing\":false"), "got: {payload}");
    }

    #[tokio::test]
    async fn stop_accompaniment_emits_status_playing_false() {
        // Drive the real command through Tauri's mock runtime and capture the
        // emitted event — proves stop reports `playing: false` (AC10), not just
        // that the payload type serializes. (start emits true but needs a real
        // output device, so it's covered by the manual audible test.)
        use std::sync::Mutex as StdMutex;
        use tauri::test::mock_app;
        use tauri::{Listener, Manager};

        let app = mock_app();
        app.manage(AppState::with_mocks());

        let captured = Arc::new(StdMutex::new(None::<String>));
        let sink = captured.clone();
        app.listen("accompaniment-status", move |event| {
            *sink.lock().unwrap() = Some(event.payload().to_string());
        });

        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        stop_accompaniment(handle, state)
            .await
            .expect("stop_accompaniment should succeed on a fresh state");

        let payload = captured
            .lock()
            .unwrap()
            .clone()
            .expect("an accompaniment-status event should have been emitted");
        assert!(
            payload.contains("\"playing\":false"),
            "stop should report playing=false, got: {payload}"
        );
    }

    // ── MIDI import (Story: Phase 2 Smart Import — PR 1) ───────────────

    fn write_variable_length(buf: &mut Vec<u8>, mut value: u32) {
        let mut bytes = vec![(value & 0x7F) as u8];
        value >>= 7;
        while value > 0 {
            bytes.push((value & 0x7F) as u8 | 0x80);
            value >>= 7;
        }
        bytes.reverse();
        buf.extend_from_slice(&bytes);
    }

    fn write_meta_event(buf: &mut Vec<u8>, delta: u16, meta_type: u8, data: &[u8]) {
        write_variable_length(buf, delta as u32);
        buf.push(0xFF);
        buf.push(meta_type);
        write_variable_length(buf, data.len() as u32);
        buf.extend_from_slice(data);
    }

    fn write_midi_event(buf: &mut Vec<u8>, delta: u16, status: u8, data1: u8, data2: u8) {
        write_variable_length(buf, delta as u32);
        buf.push(status);
        buf.push(data1);
        buf.push(data2);
    }

    /// Build a minimal valid Format-0 MIDI: a tempo, an optional TrackName,
    /// and one 4/4 measure of quarter notes (C-D-E-F). Mirrors the
    /// byte-level construction used by `brain`'s own MIDI parser tests so we
    /// need no MIDI-writing dependency in the desktop crate.
    fn build_test_midi(name: Option<&str>) -> Vec<u8> {
        let mut track = Vec::new();
        write_meta_event(&mut track, 0, 0x51, &[0x07, 0xA1, 0x20]); // 120 BPM
        if let Some(name) = name {
            write_meta_event(&mut track, 0, 0x03, name.as_bytes());
        }
        for pitch in [60_u8, 62, 64, 65] {
            write_midi_event(&mut track, 0, 0x90, pitch, 80); // note on
            write_midi_event(&mut track, 480, 0x80, pitch, 0); // note off, +1 beat
        }
        write_meta_event(&mut track, 0, 0x2F, &[]); // end of track

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // format 0
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // 1 track
        bytes.extend_from_slice(&480_u16.to_be_bytes()); // ticks per quarter
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&track);
        bytes
    }

    #[test]
    fn import_midi_stores_derived_metadata_and_roundtrips_to_musicxml() {
        let s = state();
        let entry = s
            .import_midi(
                "scales.mid".to_string(),
                build_test_midi(Some("C Major Scale")),
            )
            .expect("import valid MIDI");

        // Metadata is derived in the backend from the parsed MIDI.
        assert_eq!(entry.title, "C Major Scale");
        assert_eq!(entry.source_filename, "scales.mid");
        assert!(entry.duration_measures >= 1, "one measure of notes");

        // The stored payload must be real MusicXML the rest of the app can
        // parse — this is the canonical-format guarantee the emitter exists
        // to uphold.
        let reparsed = brain::score::musicxml::parse_musicxml_str_part(&entry.music_xml, 0)
            .expect("stored MusicXML must re-parse");
        assert!(!reparsed.measures.is_empty());

        // And it landed in the library.
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "C Major Scale");
    }

    /// #337 S1: a multi-track file imports ONE chosen part — the stored
    /// MusicXML carries only that track's notes, so the score view and
    /// follower practice the part the player picked, not a band mash-up.
    #[test]
    fn import_midi_track_stores_only_the_chosen_part() {
        // Format-1: track 0 = conductor (tempo only), track 1 = melody
        // (4 notes), track 2 = countermelody (2 notes).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(&480u16.to_be_bytes());
        let push = |events: Vec<u8>, buf: &mut Vec<u8>| {
            buf.extend_from_slice(b"MTrk");
            buf.extend_from_slice(&(events.len() as u32).to_be_bytes());
            buf.extend_from_slice(&events);
        };
        let mut t0 = Vec::new();
        write_meta_event(&mut t0, 0, 0x51, &[0x07, 0xA1, 0x20]);
        write_meta_event(&mut t0, 0, 0x2F, &[]);
        push(t0, &mut bytes);
        let mut t1 = Vec::new();
        write_meta_event(&mut t1, 0, 0x03, b"Melody");
        for pitch in [60_u8, 62, 64, 65] {
            write_midi_event(&mut t1, 0, 0x90, pitch, 80);
            write_midi_event(&mut t1, 480, 0x80, pitch, 0);
        }
        write_meta_event(&mut t1, 0, 0x2F, &[]);
        push(t1, &mut bytes);
        let mut t2 = Vec::new();
        write_meta_event(&mut t2, 0, 0x03, b"Counter");
        for pitch in [72_u8, 74] {
            write_midi_event(&mut t2, 0, 0x91, pitch, 80);
            write_midi_event(&mut t2, 480, 0x81, pitch, 0);
        }
        write_meta_event(&mut t2, 0, 0x2F, &[]);
        push(t2, &mut bytes);

        let s = state();
        let entry = s
            .import_midi_track("band.mid".to_string(), bytes, Some(2))
            .expect("import the Counter track");
        let reparsed = brain::score::musicxml::parse_musicxml_str_part(&entry.music_xml, 0)
            .expect("stored MusicXML re-parses");
        let midis: Vec<u8> = reparsed
            .measures
            .iter()
            .flat_map(|m| m.notes.iter().filter(|n| !n.is_rest).map(|n| n.midi_number))
            .collect();
        assert_eq!(
            midis,
            vec![72, 74],
            "only the chosen track's notes: {midis:?}"
        );
    }

    #[test]
    fn import_midi_falls_back_to_filename_when_untitled() {
        let s = state();
        // No TrackName → parser yields "Untitled" → we use the name stem,
        // stripped of directory and extension.
        let entry = s
            .import_midi("etudes/op10_no3.mid".to_string(), build_test_midi(None))
            .expect("import unnamed MIDI");
        assert_eq!(entry.title, "op10_no3");
    }

    #[test]
    fn import_midi_rejects_corrupt_bytes_without_persisting() {
        let s = state();
        let err = s
            .import_midi("garbage.mid".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF])
            .expect_err("corrupt MIDI must error");
        assert!(!err.is_empty());
        // Nothing should have been written to the library.
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert!(listed.is_empty(), "failed import must not persist a row");
    }

    /// Two-part MusicXML (Trumpet then Trombone) for the part-selection tests.
    const TWO_PART_MUSICXML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="3.1">
  <work><work-title>Little Duet</work-title></work>
  <identification><creator type="composer">Tester</creator></identification>
  <part-list>
    <score-part id="P1"><part-name>Trumpet</part-name></score-part>
    <score-part id="P2"><part-name>Trombone</part-name></score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
  <part id="P2">
    <measure number="1">
      <attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
      <note><pitch><step>G</step><octave>3</octave></pitch><duration>1</duration></note>
      <note><pitch><step>F</step><octave>3</octave></pitch><duration>1</duration></note>
    </measure>
  </part>
</score-partwise>"#;

    #[test]
    fn import_musicxml_stores_metadata_and_selected_part() {
        let s = state();
        // Choose the second part (Trombone) — the part the user picked must be
        // persisted so the follower re-selects it at session start.
        let entry = s
            .import_musicxml(
                "duet.musicxml".to_string(),
                TWO_PART_MUSICXML.to_string(),
                1,
            )
            .expect("import valid MusicXML");

        assert_eq!(entry.title, "Little Duet");
        assert_eq!(entry.composer.as_deref(), Some("Tester"));
        assert_eq!(entry.part_index, 1, "the chosen part must be stored");
        assert!(entry.duration_measures >= 1);

        // The stored MusicXML is the original (both parts), and re-parsing the
        // stored part_index yields the Trombone line (G3=55, F3=53).
        let reparsed =
            brain::score::musicxml::parse_musicxml_str_part(&entry.music_xml, entry.part_index)
                .expect("stored MusicXML must re-parse");
        let midi: Vec<u8> = reparsed.measures[0]
            .notes
            .iter()
            .map(|n| n.midi_number)
            .collect();
        assert_eq!(
            midi,
            vec![55, 53],
            "stored part_index should select Trombone"
        );

        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn import_musicxml_falls_back_to_filename_when_untitled() {
        let s = state();
        let xml =
            TWO_PART_MUSICXML.replace("<work><work-title>Little Duet</work-title></work>", "");
        let entry = s
            .import_musicxml("scores/my_etude.xml".to_string(), xml, 0)
            .expect("import untitled MusicXML");
        assert_eq!(entry.title, "my_etude");
    }

    #[test]
    fn import_musicxml_out_of_range_part_errors_without_persisting() {
        let s = state();
        let err = s
            .import_musicxml(
                "duet.musicxml".to_string(),
                TWO_PART_MUSICXML.to_string(),
                99,
            )
            .expect_err("out-of-range part must error");
        assert!(!err.is_empty());
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert!(listed.is_empty(), "failed import must not persist a row");
    }

    /// #385: dropping the same MIDI file twice (the back-to-back VA flow)
    /// must not stack a duplicate — the second import returns the first
    /// entry, and "My Scores" stays at one row.
    #[test]
    fn reimporting_the_same_midi_file_dedups_to_one_entry() {
        let s = state();
        let bytes = build_test_midi(Some("C Major Scale"));

        let first = s
            .import_midi("scales.mid".to_string(), bytes.clone())
            .expect("first import");
        let second = s
            .import_midi("scales.mid".to_string(), bytes)
            .expect("re-import");

        assert_eq!(second.id, first.id, "re-import must reuse the entry");
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert_eq!(listed.len(), 1, "no duplicate library row");
    }

    /// #385: re-importing the same MusicXML + same chosen part dedups, but
    /// picking the OTHER part of the same file is a genuinely new entry.
    #[test]
    fn reimporting_same_musicxml_dedups_per_chosen_part() {
        let s = state();

        let trombone = s
            .import_musicxml(
                "duet.musicxml".to_string(),
                TWO_PART_MUSICXML.to_string(),
                1,
            )
            .expect("first import");
        let again = s
            .import_musicxml(
                "duet.musicxml".to_string(),
                TWO_PART_MUSICXML.to_string(),
                1,
            )
            .expect("re-import same part");
        assert_eq!(again.id, trombone.id, "same file + same part must dedup");

        let trumpet = s
            .import_musicxml(
                "duet.musicxml".to_string(),
                TWO_PART_MUSICXML.to_string(),
                0,
            )
            .expect("import the other part");
        assert_ne!(trumpet.id, trombone.id, "the other part is its own entry");

        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert_eq!(listed.len(), 2, "one entry per (file, part)");
    }

    /// A minimal valid PDF header — enough to pass the OMR pipeline's front
    /// door. The `StaticOmrEngine` ignores the bytes and returns a fixed score.
    const FAKE_PDF: &[u8] = b"%PDF-1.7\nscan\n";

    #[test]
    fn recognize_pdf_yields_musicxml_and_parts_for_the_picker() {
        let s = state().with_omr_enabled();
        let engine = omr::StaticOmrEngine::new(TWO_PART_MUSICXML);
        let recognized = s
            .recognize_pdf(&engine, FAKE_PDF)
            .expect("recognition succeeds when the beta is on");

        // The recognized MusicXML is the canonical format and re-parses with the
        // same brain parser MusicXML import uses — proving it can flow straight
        // into import_musicxml_file with the chosen part.
        assert!(recognized.music_xml.contains("<score-partwise"));
        assert_eq!(recognized.parts, vec!["Trumpet", "Trombone"]);
        assert!(!recognized.low_content, "this score has measures");

        let reparsed = brain::score::musicxml::parse_musicxml_str_part(&recognized.music_xml, 1)
            .expect("recognized MusicXML must re-parse for the picked part");
        assert!(!reparsed.measures.is_empty());

        // OMR stores nothing itself — import happens later via the shared path.
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert!(listed.is_empty(), "recognition must not persist a row");
    }

    #[test]
    fn recognize_pdf_is_gated_off_by_default() {
        let s = state(); // beta flag off
        let engine = omr::StaticOmrEngine::new(TWO_PART_MUSICXML);
        let err = s
            .recognize_pdf(&engine, FAKE_PDF)
            .expect_err("disabled beta must refuse");
        assert!(err.contains("experimental"), "honest, calm message: {err}");
    }

    #[test]
    fn recognize_pdf_rejects_a_non_pdf_drop() {
        let s = state().with_omr_enabled();
        let engine = omr::StaticOmrEngine::new(TWO_PART_MUSICXML);
        let err = s
            .recognize_pdf(&engine, b"not a pdf at all")
            .expect_err("non-pdf must error before the engine runs");
        assert!(!err.is_empty());
    }

    #[test]
    fn recognize_pdf_flags_a_scan_that_read_almost_nothing() {
        let s = state().with_omr_enabled();
        // A structurally-valid score with one part but no measures.
        let empty = r#"<score-partwise version="4.0">
            <part-list><score-part id="P1"><part-name>Flute</part-name></score-part></part-list>
            <part id="P1"></part>
        </score-partwise>"#;
        let engine = omr::StaticOmrEngine::new(empty);
        let recognized = s
            .recognize_pdf(&engine, FAKE_PDF)
            .expect("recognizes structurally");
        assert!(
            recognized.low_content,
            "no measures → warn the read likely failed"
        );
    }

    #[test]
    fn decode_musicxml_bytes_rejects_compressed_or_binary() {
        // A .mxl (ZIP) starts with "PK\x03\x04" — not valid UTF-8 text. The
        // message must steer the user to re-export uncompressed, not leak a
        // raw decode error.
        let zip_magic = vec![0x50, 0x4B, 0x03, 0x04, 0xFF, 0xFE];
        let err = decode_musicxml_bytes(zip_magic).expect_err("binary must be rejected");
        assert!(err.contains(".mxl") && err.contains("uncompressed"));
    }

    #[test]
    fn list_score_parts_returns_names_in_order() {
        let parts = list_score_parts(TWO_PART_MUSICXML.as_bytes().to_vec())
            .expect("list parts from MusicXML");
        assert_eq!(parts, vec!["Trumpet".to_string(), "Trombone".to_string()]);
    }

    #[test]
    fn filename_ext_lowercases_and_strips_dot() {
        assert_eq!(filename_ext("Take 1.WAV").as_deref(), Some("wav"));
        assert_eq!(filename_ext("song.mp3").as_deref(), Some("mp3"));
        assert_eq!(filename_ext("noext"), None);
    }

    /// Build a minimal 16-bit PCM mono WAV in memory.
    fn wav_mono_16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&sample_rate.to_le_bytes());
        b.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    /// ONNX Runtime is required at run time for transcription; `ort`'s
    /// `load-dynamic` backend *panics* on a missing dylib, so gate before
    /// calling in. CI sets `TRANSCRIBE_REQUIRE_ORT=1` to make a missing
    /// runtime a hard failure rather than a silent skip.
    fn skip_without_ort() -> bool {
        let present = std::env::var("ORT_DYLIB_PATH")
            .ok()
            .map(|p| std::path::Path::new(&p).exists())
            .unwrap_or(false);
        if present {
            return false;
        }
        if std::env::var("TRANSCRIBE_REQUIRE_ORT").as_deref() == Ok("1") {
            panic!("ONNX Runtime required but ORT_DYLIB_PATH not set/found");
        }
        eprintln!("skipping audio-import test: ONNX Runtime unavailable");
        true
    }

    #[test]
    fn import_audio_transcribes_and_stores_with_quality() {
        if skip_without_ort() {
            return;
        }
        // A short monophonic sine scale at 22.05 kHz (the model rate).
        let sr = 22_050u32;
        let pitches = [60i32, 62, 64, 65, 67];
        let mut pcm: Vec<i16> = Vec::new();
        for &m in &pitches {
            let hz = 440.0 * 2f64.powf((m as f64 - 69.0) / 12.0);
            let n = (0.6 * sr as f64) as usize;
            pcm.extend((0..n).map(|i| {
                let t = i as f64 / sr as f64;
                ((2.0 * std::f64::consts::PI * hz * t).sin() * 16_000.0) as i16
            }));
        }
        let wav = wav_mono_16(&pcm, sr);

        let s = state();
        let (entry, quality) = s
            .import_audio("recording.wav".to_string(), wav, Some("wav"))
            .expect("audio import should succeed");

        // Title falls back to the filename stem (transcribed MIDI is unnamed).
        assert_eq!(entry.title, "recording");
        assert!(entry.duration_measures >= 1);
        assert!(quality.note_count > 0, "a clear scale yields notes");
        // A clean monophonic recording should not look polyphonic.
        assert_eq!(
            quality.verdict,
            transcribe::PolyphonyVerdict::Mono,
            "monophonic input flagged polyphonic: {quality:?}"
        );

        // Stored payload is real, re-parseable MusicXML.
        let reparsed = brain::score::musicxml::parse_musicxml_str_part(&entry.music_xml, 0)
            .expect("stored MusicXML must re-parse");
        assert!(!reparsed.measures.is_empty());
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn import_audio_rejects_garbage_without_persisting() {
        // Decode failure needs no ONNX Runtime — always runs.
        let s = state();
        let err = s
            .import_audio("noise.wav".to_string(), vec![0u8; 64], Some("wav"))
            .expect_err("garbage audio must error");
        assert!(!err.is_empty());
        let listed = s.score_store.lock().unwrap().list().expect("list scores");
        assert!(listed.is_empty(), "failed import must not persist a row");
    }

    /// Capture `import-progress` events emitted on `app` as `(stage, pct)`.
    fn capture_import_progress(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Arc<std::sync::Mutex<Vec<(String, u8)>>> {
        use tauri::Listener;
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        app.listen("import-progress", move |event| {
            let payload: serde_json::Value =
                serde_json::from_str(event.payload()).expect("progress payload is JSON");
            sink.lock().unwrap().push((
                payload["stage"].as_str().unwrap_or_default().to_owned(),
                payload["pct"].as_u64().unwrap_or_default() as u8,
            ));
        });
        events
    }

    /// #313 AC1: a successful audio import reports every progress beat in
    /// order AND runs the import off the dispatching thread. If the blocking
    /// hop is ever removed (the import inlined into the command body), the
    /// injected import runs on the caller's thread and the ThreadId assertion
    /// fails — which is exactly the bug: work on the dispatching (main) thread
    /// blocks the webview event loop and the progress UI never paints. (A full
    /// sync revert is pinned separately: the tests that `.await` the real
    /// commands stop compiling.)
    #[tokio::test]
    async fn import_audio_command_reports_progress_and_runs_off_the_dispatching_thread() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let events = capture_import_progress(&app);

        let import_thread = Arc::new(std::sync::Mutex::new(None));
        let seen = import_thread.clone();
        let dto = import_audio_file_with(
            app.handle().clone(),
            "recording.wav".to_string(),
            vec![0u8; 4],
            move |state, name, _bytes, _ext| {
                *seen.lock().unwrap() = Some(std::thread::current().id());
                let entry = state.import_midi(name, build_test_midi(None))?;
                Ok((
                    entry,
                    transcribe::TranscriptionQuality {
                        note_count: 4,
                        mean_confidence: 0.9,
                        polyphony: 0.0,
                        uncertain_count: 0,
                        verdict: transcribe::PolyphonyVerdict::Mono,
                    },
                ))
            },
        )
        .await
        .expect("import should succeed");

        assert_eq!(dto.entry.title, "recording");
        let stages = events.lock().unwrap().clone();
        assert_eq!(
            stages,
            vec![
                ("decoding".to_owned(), 15),
                ("transcribing".to_owned(), 45),
                ("converting".to_owned(), 85),
                ("done".to_owned(), 100),
            ],
            "the UI's stage labels depend on these exact beats, in order"
        );
        let import_thread = import_thread.lock().unwrap().expect("import ran");
        assert_ne!(
            import_thread,
            std::thread::current().id(),
            "heavy import work must never run on the dispatching thread — that \
             blocks the webview event loop and the progress UI never paints (#313)"
        );
    }

    /// #313 AC2: an undecodable recording makes the real command fail calmly
    /// and never claim `converting`/`done` — the progress trail stops where
    /// the work stopped.
    #[tokio::test]
    async fn import_audio_file_fails_calmly_without_claiming_completion() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let events = capture_import_progress(&app);

        let err = import_audio_file(app.handle().clone(), "noise.wav".to_string(), vec![0u8; 64])
            .await
            .expect_err("garbage audio must error");
        assert!(!err.is_empty());

        let stages: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .map(|(stage, _)| stage.clone())
            .collect();
        assert_eq!(
            stages,
            vec!["decoding".to_owned(), "transcribing".to_owned()],
            "a failed import must not report converting/done"
        );
    }

    /// #313 AC3: a panic anywhere in the off-thread import — including outside
    /// the #267 transcription guard (e.g. MIDI→MusicXML conversion) — degrades
    /// to the calm engine message via the join-error path, instead of crashing
    /// or leaving the frontend's promise hanging.
    #[tokio::test]
    async fn import_audio_command_converts_a_blocking_panic_to_the_calm_error() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks());

        let err = import_audio_file_with(
            app.handle().clone(),
            "recording.wav".to_string(),
            vec![0u8; 4],
            |_state,
             _name,
             _bytes,
             _ext|
             -> Result<(ScoreLibraryEntry, transcribe::TranscriptionQuality), String> {
                panic!("conversion blew up outside the guard")
            },
        )
        .await
        .expect_err("a panicking import must surface as an error");
        assert!(
            err.contains("Audio import isn't available"),
            "expected the calm engine-unavailable message, got: {err}"
        );
    }

    /// #313 AC4: the PDF recognizer reports its progress beats in order and
    /// runs the OMR work off the dispatching thread — the same contract as
    /// audio import, with the same failure mode if the blocking hop is removed.
    #[tokio::test]
    async fn recognize_pdf_command_reports_progress_and_runs_off_the_dispatching_thread() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks().with_omr_enabled());
        let events = capture_import_progress(&app);

        let omr_thread = Arc::new(std::sync::Mutex::new(None));
        let seen = omr_thread.clone();
        let dto = recognize_pdf_score_with(
            app.handle().clone(),
            FAKE_PDF.to_vec(),
            move |state, bytes| {
                *seen.lock().unwrap() = Some(std::thread::current().id());
                state.recognize_pdf(&omr::StaticOmrEngine::new(TWO_PART_MUSICXML), bytes)
            },
        )
        .await
        .expect("recognition should succeed");

        assert_eq!(dto.parts.len(), 2, "both parts reach the picker");
        assert!(dto.from_scan, "OMR results always carry the scan note");
        let stages = events.lock().unwrap().clone();
        assert_eq!(
            stages,
            vec![
                ("rasterizing".to_owned(), 20),
                ("reading-notes".to_owned(), 55),
                ("done".to_owned(), 100),
            ],
            "the UI's stage labels depend on these exact beats, in order"
        );
        let omr_thread = omr_thread.lock().unwrap().expect("OMR ran");
        assert_ne!(
            omr_thread,
            std::thread::current().id(),
            "OMR must never run on the dispatching thread — that blocks the \
             webview event loop and the progress UI never paints (#313)"
        );
    }

    /// #313 AC5: a panic in the blocking OMR section degrades to the calm
    /// reader-stopped message and never claims `done`.
    #[tokio::test]
    async fn recognize_pdf_command_converts_a_blocking_panic_to_the_calm_error() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks().with_omr_enabled());
        let events = capture_import_progress(&app);

        let err = recognize_pdf_score_with(
            app.handle().clone(),
            FAKE_PDF.to_vec(),
            |_state, _bytes| -> Result<RecognizedScore, String> {
                panic!("the OMR sidecar blew up")
            },
        )
        .await
        .expect_err("a panicking OMR run must surface as an error");
        assert_eq!(err, PDF_READER_STOPPED);

        let stages: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .map(|(stage, _)| stage.clone())
            .collect();
        assert_eq!(
            stages,
            vec!["rasterizing".to_owned(), "reading-notes".to_owned()],
            "a failed recognition must not report done"
        );
    }

    /// #313 AC6: the real command still refuses calmly when the beta is off —
    /// before any progress event. Awaiting the real command also pins its
    /// async signature: a revert to a sync command stops compiling here.
    #[tokio::test]
    async fn recognize_pdf_command_stays_gated_off_with_no_progress_events() {
        use tauri::test::mock_app;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let events = capture_import_progress(&app);

        let state = app.state::<AppState>();
        let err = recognize_pdf_score(
            app.handle().clone(),
            state,
            "score.pdf".to_string(),
            FAKE_PDF.to_vec(),
        )
        .await
        .expect_err("the beta gate must refuse");
        assert!(
            err.contains("experimental feature"),
            "expected the gate message, got: {err}"
        );
        assert!(
            events.lock().unwrap().is_empty(),
            "no progress beats may be emitted before the gate"
        );
    }

    // -----------------------------------------------------------------------
    // Airplane-switch threading (command layer → Rust-core NetworkPolicy)
    // -----------------------------------------------------------------------

    /// An HTTP client that panics if any outbound call is attempted. Used to
    /// prove the command-layer threading: when coaching is disabled, the
    /// engines are set Offline and never reach this client.
    struct PanickingHttpClient;

    #[async_trait]
    impl brain::coaching::HttpClient for PanickingHttpClient {
        async fn post_json(
            &self,
            _url: &str,
            _body: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, brain::coaching::CoachingError> {
            panic!("outbound HTTP attempted while coaching was disabled (Offline policy)");
        }
    }

    fn online_engine_with_panicking_client() -> CoachingEngine {
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test-key".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(PanickingHttpClient),
        )
        .expect("engine builds with an explicit api key");
        // Start Online so the test proves the *policy flip* — not a default —
        // is what prevents the call.
        engine.set_network_policy(NetworkPolicy::Online);
        engine
    }

    /// An HTTP client that always returns one canned response body.
    struct CannedHttpClient(String);

    #[async_trait]
    impl brain::coaching::HttpClient for CannedHttpClient {
        async fn post_json(
            &self,
            _url: &str,
            _body: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, brain::coaching::CoachingError> {
            Ok(self.0.clone())
        }
    }

    /// An Online engine whose LLM always answers with `{ "why": <why> }` in an
    /// Anthropic-shaped envelope — what the reveal-enrichment prompt requests.
    fn online_engine_answering_why(why: &str) -> CoachingEngine {
        let inner = serde_json::json!({ "why": why }).to_string();
        let body =
            serde_json::json!({ "content": [{ "type": "text", "text": inner }] }).to_string();
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test-key".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(CannedHttpClient(body)),
        )
        .expect("engine builds with an explicit api key");
        engine.set_network_policy(NetworkPolicy::Online);
        engine
    }

    fn grounded_reveal() -> Reveal {
        Reveal {
            concept: "G Dorian".to_owned(),
            connection: "Miles Davis — \"So What\"".to_owned(),
            why: "curated line".to_owned(),
            source: brain::connections::RevealSource::Grounded,
            tonic: 7,
            mode: "dorian".to_owned(),
        }
    }

    /// #253 S2 AC4 (service seam): online, the real service swaps in the LLM's
    /// `why` and flips the source — while `concept`/`connection` stay exactly
    /// the curated values. Fails if the override stops calling the engine or
    /// stops folding through `apply_enriched_why`.
    #[tokio::test]
    async fn enrich_reveal_online_replaces_why_keeps_connection() {
        let svc =
            LlmCoachingService::with_engine(online_engine_answering_why("A cooler, warmer line."));
        let base = grounded_reveal();
        let out = svc.enrich_reveal(base.clone()).await;
        assert_eq!(out.why, "A cooler, warmer line.");
        assert_eq!(out.source, brain::connections::RevealSource::LlmGrounded);
        assert_eq!(
            out.connection, base.connection,
            "connection must not change"
        );
        assert_eq!(out.concept, base.concept, "concept must not change");
    }

    /// #253 S2 AC4 (offline counterpart): with the coaching opt-in off, the real
    /// service returns the reveal unchanged and never touches the HTTP client
    /// (the client here panics on any call). Fails if the offline gate is lost.
    #[tokio::test]
    async fn enrich_reveal_offline_is_identity_and_makes_no_call() {
        let mut engine = online_engine_with_panicking_client();
        engine.set_network_policy(NetworkPolicy::Offline);
        let svc = LlmCoachingService::with_engine(engine);
        let base = grounded_reveal();
        assert_eq!(svc.enrich_reveal(base.clone()).await, base);
    }

    /// #253 S3: recording reveals through the command layer persists to the
    /// Learner Model with dedup — a novel reveal grows the distinct count by
    /// exactly 1, an exact repeat by 0, and the model survives (is re-read from)
    /// the store between calls. Fails if the load→apply→write path drops state
    /// or dedup regresses.
    #[test]
    fn record_reveal_impl_dedups_and_persists_across_calls() {
        let s = state();
        assert_eq!(
            record_reveal_impl(&s, "G Dorian", "Miles Davis — \"So What\"", 100).unwrap(),
            1,
            "first reveal unlocks one entry"
        );
        assert_eq!(
            record_reveal_impl(&s, "G Dorian", "Miles Davis — \"So What\"", 200).unwrap(),
            1,
            "an exact repeat must not grow the collection"
        );
        assert_eq!(
            record_reveal_impl(&s, "G Dorian", "Santana — \"Oye Como Va\"", 300).unwrap(),
            2,
            "a different connection is a new unlock"
        );
    }

    /// #214 S1b: the index lifecycle through the REAL import path —
    /// an imported score becomes identifiable in the same session, free
    /// noodling stays silent, deletion silences the score immediately,
    /// and a corrupt entry is skipped calmly (never a startup break).
    #[test]
    fn imported_scores_identify_and_deleted_ones_fall_silent() {
        let s = AppState::with_mocks();
        // A distinctive 16-note tune, emitted to MusicXML and imported.
        let melody: [u8; 16] = [
            64, 62, 60, 65, 64, 67, 65, 69, 71, 72, 69, 67, 71, 74, 72, 76,
        ];
        let mut measures = Vec::new();
        for (mi, chunk) in melody.chunks(4).enumerate() {
            measures.push(brain::score::Measure {
                number: mi + 1,
                notes: chunk
                    .iter()
                    .enumerate()
                    .map(|(i, &midi)| brain::score::ScoreNote {
                        pitch_hz: 440.0,
                        midi_number: midi,
                        duration_beats: 1.0,
                        start_beat: i as f64,
                        dynamic: None,
                        is_rest: false,
                    })
                    .collect(),
            });
        }
        let model = brain::score::ScoreModel {
            title: "The Lifecycle Tune".into(),
            composer: None,
            instrument: None,
            time_signature: brain::score::TimeSignature::default(),
            key_signature: brain::score::KeySignature::default(),
            tempo_bpm: 100.0,
            measures,
            grand_staff: false,
        };
        let xml = brain::score::emit::score_model_to_musicxml(&model);
        let entry = s
            .import_musicxml("tune.musicxml".into(), xml, 0)
            .expect("import succeeds");

        // Play 14 notes of it (as a phrase pitch track) → identified.
        let seed_phrases = |s: &AppState, midis: &[u8]| {
            let mut phrase = sample_phrase();
            phrase.pitch_stats.pitches = midis
                .iter()
                .flat_map(|&m| {
                    let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                    std::iter::repeat_n(hz, 6)
                })
                .collect();
            s.phrase_buffer.lock().unwrap().clear();
            s.phrase_buffer.lock().unwrap().push(phrase);
        };
        seed_phrases(&s, &melody[2..16]);
        let m = check_piece_match_impl(&s).expect("a real excerpt identifies");
        assert_eq!(m.title, "The Lifecycle Tune");
        assert_eq!(m.score_id, entry.id.to_string());

        // Free noodling: silence (the S1a gates, end to end).
        seed_phrases(
            &s,
            &[62, 65, 61, 70, 66, 59, 63, 71, 58, 67, 61, 73, 60, 68],
        );
        assert_eq!(check_piece_match_impl(&s), None, "noodling stays silent");

        // Deletion silences immediately — through the ONE seam the
        // delete_score command delegates to (review MF7c).
        seed_phrases(&s, &melody[2..16]);
        assert!(check_piece_match_impl(&s).is_some());
        s.delete_score_by_id(&entry.id.to_string())
            .expect("delete succeeds");
        assert_eq!(check_piece_match_impl(&s), None, "deleted → silent");

        // MIDI-path hook (review MF7b): the C-D-E-F test MIDI is too
        // short to identify, but its import must INDEX — prove the hook
        // ran by rebuilding-from-scratch equivalence: the entry appears
        // in the matcher's title map.
        let midi_entry = s
            .import_midi("hook.mid".to_string(), build_test_midi(Some("Hook")))
            .expect("midi import succeeds");
        assert!(
            s.piece_matcher
                .lock()
                .unwrap()
                .titles
                .values()
                .any(|(id, _)| *id == midi_entry.id.to_string()),
            "the MIDI import path indexes (hook present)"
        );

        // Raw-import seam (the command delegates here): identifiable too.
        let entry2 = s
            .import_raw_score(
                "Second Tune".into(),
                None,
                "t2.musicxml".into(),
                brain::score::emit::score_model_to_musicxml(&model),
                0,
                4,
            )
            .expect("raw import succeeds");
        seed_phrases(&s, &melody[2..16]);
        // The first copy was deleted above, so the raw-import seam's copy
        // is the sole owner — identifiable, proving the command's
        // delegate path indexes. (Duplicate-ambiguity itself is S1a's
        // margin test; the wiring needn't re-prove it.)
        let m2 = check_piece_match_impl(&s).expect("raw-import path identifies");
        assert_eq!(m2.title, "Second Tune");
        assert_eq!(m2.score_id, entry2.id.to_string());
        s.delete_score_by_id(&entry2.id.to_string()).unwrap();
        assert_eq!(check_piece_match_impl(&s), None, "second delete → silent");

        // A corrupt entry never breaks indexing (startup-calm path).
        let bogus = ScoreLibraryEntry {
            music_xml: "<not really xml".into(),
            ..entry
        };
        s.index_entry(&bogus); // must not panic
        assert_eq!(check_piece_match_impl(&s), None);
    }

    /// The 16-note melody + model both startup-index tests share.
    fn preexisting_melody_and_model() -> ([u8; 16], brain::score::ScoreModel) {
        let melody: [u8; 16] = [
            64, 62, 60, 65, 64, 67, 65, 69, 71, 72, 69, 67, 71, 74, 72, 76,
        ];
        let mut measures = Vec::new();
        for (mi, chunk) in melody.chunks(4).enumerate() {
            measures.push(brain::score::Measure {
                number: mi + 1,
                notes: chunk
                    .iter()
                    .enumerate()
                    .map(|(i, &midi)| brain::score::ScoreNote {
                        pitch_hz: 440.0,
                        midi_number: midi,
                        duration_beats: 1.0,
                        start_beat: i as f64,
                        dynamic: None,
                        is_rest: false,
                    })
                    .collect(),
            });
        }
        let model = brain::score::ScoreModel {
            title: "Preexisting".into(),
            composer: None,
            instrument: None,
            time_signature: brain::score::TimeSignature::default(),
            key_signature: brain::score::KeySignature::default(),
            tempo_bpm: 100.0,
            measures,
            grand_staff: false,
        };
        (melody, model)
    }

    /// Push a phrase whose pitch trail follows `melody[2..]` — enough
    /// coherent n-grams for identification.
    fn seed_preexisting_phrase(s: &AppState, melody: &[u8]) {
        let mut phrase = sample_phrase();
        phrase.pitch_stats.pitches = melody[2..16]
            .iter()
            .flat_map(|&m| {
                let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                std::iter::repeat_n(hz, 6)
            })
            .collect();
        s.phrase_buffer.lock().unwrap().push(phrase);
    }

    /// #214 S1b review MF7a: the STARTUP rebuild — a store that already
    /// holds scores (seeded around the hooks) identifies only after
    /// rebuild_piece_index() runs (the rebuild BEHAVIOR pin; the
    /// constructor-tail CALL SITE is pinned separately by
    /// `constructor_tail_indexes_a_preexisting_library`).
    #[test]
    fn startup_rebuild_indexes_a_preexisting_library() {
        let s = AppState::with_mocks();
        let (melody, model) = preexisting_melody_and_model();
        // Seed the STORE directly — around the import hooks, like a
        // library that predates this launch.
        s.score_store
            .lock()
            .unwrap()
            .import(
                "Preexisting".into(),
                None,
                "pre.musicxml".into(),
                brain::score::emit::score_model_to_musicxml(&model),
                0,
                4,
            )
            .expect("seed import");
        seed_preexisting_phrase(&s, &melody);
        assert_eq!(
            check_piece_match_impl(&s),
            None,
            "not indexed until the startup rebuild runs"
        );
        s.rebuild_piece_index();
        assert!(
            check_piece_match_impl(&s).is_some(),
            "the rebuild makes a preexisting library identifiable"
        );
    }

    /// #214 S2: the CONSTRUCTOR-TAIL pin — a library seeded BEFORE
    /// construction is identifiable with no manual rebuild call, because
    /// every constructor ends in the shared `indexed()` tail. Kills the
    /// mutant that drops `rebuild_piece_index` from a construction path.
    #[test]
    fn constructor_tail_indexes_a_preexisting_library() {
        let (melody, model) = preexisting_melody_and_model();
        let store = ScoreStore::in_memory().expect("in-memory store");
        store
            .import(
                "Preexisting".into(),
                None,
                "pre.musicxml".into(),
                brain::score::emit::score_model_to_musicxml(&model),
                0,
                4,
            )
            .expect("seed import");
        let s = AppState::with_mocks_on(store);
        seed_preexisting_phrase(&s, &melody);
        let m = check_piece_match_impl(&s)
            .expect("construction alone made the seeded library identifiable");
        assert_eq!(m.title, "Preexisting");
    }

    /// #419 S3: My Patterns derives from the exercise log — dedup by
    /// cell with counts, most-recent-first, capped, garbage skipped,
    /// store failure → empty (never errors the panel).
    #[test]
    fn my_patterns_dedup_count_cap_and_skip_garbage() {
        let s = AppState::with_mocks();
        let log = |cell: Option<Vec<i8>>, tonic: u8| {
            // Specs built the way the app builds them — through the real
            // explore constructors (VariationSpec has no Default).
            let model = brain::learner::LearnerModel::default();
            let spec = match cell {
                Some(c) => {
                    let (state, _) = brain::coach::start_explore_cell(
                        c,
                        tonic,
                        &model,
                        1,
                        brain::coach::DirectionMode::Forward,
                    );
                    state.spec
                }
                None => {
                    let (state, _) = brain::coach::start_explore(tonic, "major", &model, 1);
                    state.spec
                }
            };
            let store = s.session_store.lock_or_recover();
            log_exercise_best_effort(
                &store,
                ExerciseOutcome {
                    source: "opener",
                    label: "t",
                    spec: &spec,
                    seed: 1,
                    difficulty: 0,
                    tonic,
                    accuracy: None,
                },
            );
        };
        // Empty log → empty list, calm.
        assert!(my_patterns_impl(&s).is_empty());

        log(Some(vec![0, 4, 7]), 0); // pattern A in C
        log(None, 0); // catalog drill — not "yours"
        log(Some(vec![0, 4, 7]), 9); // pattern A again, in A
        log(Some(vec![0, 2, 4, 5]), 2); // pattern B in D
        let patterns = my_patterns_impl(&s);
        assert_eq!(patterns.len(), 2);
        // Most recent first: B (the last distinct arrival order reversed).
        assert_eq!(patterns[0].offsets, vec![0, 2, 4, 5]);
        assert_eq!(patterns[0].times_practiced, 1);
        assert_eq!(patterns[1].offsets, vec![0, 4, 7]);
        assert_eq!(patterns[1].times_practiced, 2);
        assert!(
            patterns[1].label.contains("2×") && patterns[1].label.contains("A"),
            "label carries count and last key: {}",
            patterns[1].label
        );

        // Direct pin: the most recent row's tonic wins (not contains()).
        assert_eq!(patterns[1].last_tonic, 9);

        // Review MF2: REAL garbage — a raw junk spec_json row and a
        // score-practice-shaped row (which every user with score history
        // has; it fails the VariationSpec parse) are skipped calmly.
        {
            let store = s.session_store.lock_or_recover();
            store
                .log_exercise(&brain::store::ExerciseLogEntry {
                    source: "opener".into(),
                    label: "junk".into(),
                    spec_json: "<not json>".into(),
                    seed: 1,
                    difficulty: 0,
                    tonic: 0,
                    accuracy: None,
                })
                .expect("junk row logs");
            store
                .log_exercise(&brain::store::ExerciseLogEntry {
                    source: "score_practice".into(),
                    label: "score row".into(),
                    spec_json: r#"{"score_title":"Sonata"}"#.into(),
                    seed: 1,
                    difficulty: 0,
                    tonic: 0,
                    accuracy: None,
                })
                .expect("score row logs");
        }
        // A single-note cell (reachable via cell editing) isn't a pattern.
        log(Some(vec![0]), 0);
        let after = my_patterns_impl(&s);
        assert_eq!(after.len(), 2, "garbage and single notes never surface");

        // Cap at 6 distinct patterns.
        for i in 0..8i8 {
            log(Some(vec![0, i + 1]), 0);
        }
        assert_eq!(my_patterns_impl(&s).len(), 6);
    }

    /// #453 S1 AC7: the command cites or stays silent. Empty state → empty
    /// list (silence > lies, never an error); a seeded momentum history +
    /// below-bar mastery surface exactly the earned kinds as lowercase DTOs
    /// whose text AND evidence carry numbers. A same-day log earns no
    /// neglect. Fails if the wire shape drifts, the store wiring breaks, or
    /// the command starts inventing filler.
    #[test]
    fn practice_suggestions_command_cites_or_stays_silent() {
        let s = AppState::with_mocks();
        assert!(
            practice_suggestions_impl(&s).is_empty(),
            "no history, no claims"
        );

        // Momentum: 8 graded rows of one cell, older half 0.5, newer 0.8
        // (all stamped now by the store — newest is recent, log spans 0
        // days, so neglect must NOT fire).
        let model = brain::learner::LearnerModel::default();
        let (explore, _) = brain::coach::start_explore_cell(
            vec![0, 4, 7],
            0,
            &model,
            1,
            brain::coach::DirectionMode::Forward,
        );
        {
            let store = s.session_store.lock_or_recover();
            for accuracy in [0.5, 0.5, 0.5, 0.5, 0.8, 0.8, 0.8, 0.8] {
                log_exercise_best_effort(
                    &store,
                    ExerciseOutcome {
                        source: "explore",
                        label: "t",
                        spec: &explore.spec,
                        seed: 1,
                        difficulty: 0,
                        tonic: 0,
                        accuracy: Some(accuracy),
                    },
                );
            }
            // Trend: Eb major below every bar (6 attempts, EWMA 0.50, now).
            let mut learner = brain::learner::LearnerModel::default();
            learner.key_mastery.insert(
                "3:major".to_owned(),
                brain::learner::Mastery {
                    attempts: 6,
                    accuracy_ewma: 0.5,
                    owned: false,
                    last_epoch_secs: Utc::now().timestamp(),
                    extra: Default::default(),
                },
            );
            store
                .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &learner)
                .expect("learner model upserts");
        }
        let out = practice_suggestions_impl(&s);
        let kinds: Vec<&str> = out.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["trend", "momentum"],
            "exactly the earned kinds, in analyzer order: {out:?}"
        );
        for dto in &out {
            assert!(
                dto.text.chars().any(|c| c.is_ascii_digit())
                    && dto.evidence.chars().any(|c| c.is_ascii_digit()),
                "every suggestion cites its numbers: {dto:?}"
            );
        }
        assert!(
            out[1].text.contains("50%") && out[1].text.contains("80%"),
            "momentum carries both halves: {}",
            out[1].text
        );
    }

    /// #453 S2: the S1 test's store fixture, shared with the recap tests —
    /// seeds a momentum cell history (50%→80%) AND a below-bar Eb-major
    /// mastery trend, so the analyzer's pinned order is trend first,
    /// momentum second.
    fn seed_history_fixture(s: &AppState) {
        let model = brain::learner::LearnerModel::default();
        let (explore, _) = brain::coach::start_explore_cell(
            vec![0, 4, 7],
            0,
            &model,
            1,
            brain::coach::DirectionMode::Forward,
        );
        let store = s.session_store.lock_or_recover();
        for accuracy in [0.5, 0.5, 0.5, 0.5, 0.8, 0.8, 0.8, 0.8] {
            log_exercise_best_effort(
                &store,
                ExerciseOutcome {
                    source: "explore",
                    label: "t",
                    spec: &explore.spec,
                    seed: 1,
                    difficulty: 0,
                    tonic: 0,
                    accuracy: Some(accuracy),
                },
            );
        }
        let mut learner = brain::learner::LearnerModel::default();
        learner.key_mastery.insert(
            "3:major".to_owned(),
            brain::learner::Mastery {
                attempts: 6,
                accuracy_ewma: 0.5,
                owned: false,
                last_epoch_secs: Utc::now().timestamp(),
                extra: Default::default(),
            },
        );
        store
            .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &learner)
            .expect("learner model upserts");
    }

    /// #453 S2 AC4: the REAL end-session path over a real store fixture
    /// weaves exactly ONE evidence-cited history line into the offline
    /// recap — the analyzer's FIRST (the Eb-major trend), never the second
    /// (momentum) — with its citation numbers intact. Fails if the command
    /// layer stops threading history into `build_recap`, more than one
    /// line is woven, or the pinned order is ignored.
    #[tokio::test]
    async fn end_session_weaves_one_cited_history_line() {
        let mut s = state();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;
        seed_history_fixture(&s);

        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        {
            // #445-6b: three settled phrases clear the thin bar.
            let mut guard = s.active_session.lock().await;
            for _ in 0..3 {
                let mut p = sample_phrase();
                p.duration_secs = 7.0;
                guard.as_mut().unwrap().recorder.record_phrase(p).unwrap();
            }
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        let history_lines: Vec<&String> = recap
            .next_session_suggestions
            .iter()
            .filter(|t| t.contains("Eb major") || t.contains("climbed from"))
            .collect();
        assert_eq!(
            history_lines.len(),
            1,
            "exactly one history line: {:?}",
            recap.next_session_suggestions
        );
        assert!(
            history_lines[0].contains("Eb major")
                && history_lines[0].contains("50%")
                && history_lines[0].contains("6 attempts"),
            "the FIRST by pinned order (the trend), citation intact: {}",
            history_lines[0]
        );
    }

    /// #453 S2 AC4 (thin): the SAME seeded history + a thin session (one
    /// short phrase) → the #445-6b short form keeps its single suggestion
    /// and gains no history line through the real path. Fails if history
    /// starts stacking onto thin recaps.
    #[tokio::test]
    async fn thin_end_session_weaves_no_history_line() {
        let mut s = state();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;
        seed_history_fixture(&s);

        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        {
            // One 1.5s phrase = thin (below both #445-6b bars).
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(
            recap.next_session_suggestions.len(),
            1,
            "the thin recap keeps its single suggestion: {:?}",
            recap.next_session_suggestions
        );
        assert!(
            !recap.next_session_suggestions[0].contains("Eb major")
                && !recap.next_session_suggestions[0].contains("climbed from"),
            "no history on a thin recap: {}",
            recap.next_session_suggestions[0]
        );
    }

    /// #454 S3 AC4: the REAL end-session path resolves the method-book tip
    /// from THIS session's live evidence — flat trumpet sustains (E1, every
    /// note ~20 cents under A4) select Schlossberg's long tones — and the
    /// offline recap carries exactly ONE attributed book line, in
    /// `areas_to_improve`. Fails if the command layer stops resolving or
    /// threading the tip, the attribution leaves the copy, the line moves
    /// lists, or a second book line appears.
    #[tokio::test]
    async fn end_session_weaves_attributed_method_book_line() {
        let mut s = state();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;

        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        {
            // Three settled phrases (clears #445-6b), every note ~20 cents
            // flat of A4 (434.95 Hz) — live E1 pitch-sag evidence.
            let mut guard = s.active_session.lock().await;
            for _ in 0..3 {
                let mut p = sample_phrase();
                p.duration_secs = 7.0;
                p.pitch_stats.pitches = vec![434.95; 12];
                guard.as_mut().unwrap().recorder.record_phrase(p).unwrap();
            }
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        let book_lines: Vec<&String> = recap
            .areas_to_improve
            .iter()
            .filter(|a| a.contains("Schlossberg"))
            .collect();
        assert_eq!(
            book_lines.len(),
            1,
            "exactly one book line: {:?}",
            recap.areas_to_improve
        );
        assert!(
            book_lines[0].contains("(Max Schlossberg, Daily Drills and Technical Studies)"),
            "the attribution is IN the copy — non-negotiable (#454): {}",
            book_lines[0]
        );
        assert!(
            !recap
                .next_session_suggestions
                .iter()
                .any(|t| t.contains("Schlossberg")),
            "the book line's home is areas_to_improve, never the history voice's list: {:?}",
            recap.next_session_suggestions
        );
    }

    /// #454 S3 AC4 (thin): the SAME live deficit on a thin session (one
    /// short flat phrase) → the #445-6b short form gains NO book line
    /// through the real path, even though the command layer resolved a tip.
    /// Fails if the thin gate stops shielding the recap from the append.
    #[tokio::test]
    async fn thin_end_session_weaves_no_method_book_line() {
        let mut s = state();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;

        start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::Practice,
            false,
            None,
        )
        .await
        .expect("start should succeed");
        {
            // One 1.5s phrase = thin, but its 12 flat notes still cross E1 —
            // the tip resolves; the thin recap must ignore it.
            let mut guard = s.active_session.lock().await;
            let mut p = sample_phrase();
            p.pitch_stats.pitches = vec![434.95; 12];
            guard.as_mut().unwrap().recorder.record_phrase(p).unwrap();
        }
        let recap = end_practice_session_impl(&s).await.unwrap();
        assert_eq!(
            recap.next_session_suggestions.len(),
            1,
            "fixture sanity: the thin short form, single suggestion: {:?}",
            recap.next_session_suggestions
        );
        let rendered = serde_json::to_string(&recap).expect("recap serializes");
        assert!(
            !rendered.contains("Schlossberg"),
            "no book line anywhere on a thin recap: {rendered}"
        );
    }

    /// #454 S2 AC7/AC8: the method-book-tip command cites its book or stays
    /// silent. Empty store → None; a measured Trumpet session whose only
    /// crossed bar is flat sustains (E1) → the Schlossberg long-tones entry
    /// with the attribution line always present; a NEWER unmeasured session
    /// silences it (the tip speaks to the last session, never a stale one);
    /// an instrument outside the catalog resolves to no family → None.
    /// Fails if the store wiring, family resolution, or the attribution
    /// formatting breaks, or if the command starts erroring/fabricating.
    #[test]
    fn method_book_tip_cites_book_or_stays_silent() {
        let s = AppState::with_mocks();
        assert!(method_book_tip_impl(&s).is_none(), "no sessions, no tip");

        // Save one session with the given instrument + optional fingerprint,
        // `secs` after the epoch base so list_recent ordering is explicit.
        let save = |instrument: &str, fingerprint: Option<&serde_json::Value>, secs: i64| {
            use chrono::{Duration, TimeZone, Utc};
            let store = s.session_store.lock_or_recover();
            let mut recap = empty_state_recap(60.0, instrument.to_owned());
            recap.fingerprint = fingerprint
                .map(|f| serde_json::from_value(f.clone()).expect("fixture fingerprint parses"));
            let t0 = Utc.timestamp_opt(1_000_000 + secs, 0).unwrap();
            store
                .save(
                    brain::session::SessionId::new(),
                    t0,
                    t0 + Duration::seconds(60),
                    &recap,
                )
                .unwrap();
        };
        // Internally consistent flat-only deficit: mean −20 ¢ trips E1 while
        // mean_abs (20 < 25) and in_tune_ratio (0.55) stay healthy-side.
        let flat = serde_json::json!({
            "intonation": {
                "note_count": 20, "mean_cents": -20.0, "mean_abs_cents": 20.0,
                "in_tune_ratio": 0.55, "tendencies": []
            }
        });

        save("Trumpet", Some(&flat), 0);
        let tip = method_book_tip_impl(&s).expect("measured brass deficit earns a tip");
        assert_eq!(tip.topic, "Long tones and pitch stability");
        assert_eq!(
            tip.source_line, "Max Schlossberg, Daily Drills and Technical Studies",
            "attribution is always present, author-comma-title"
        );
        assert!(!tip.guidance.is_empty());

        // A newer session with no fingerprint → silent, even though an older
        // measured session exists.
        save("Trumpet", None, 100);
        assert!(
            method_book_tip_impl(&s).is_none(),
            "the tip speaks to the LATEST session; unmeasured → silence"
        );

        // Unknown instrument → empty family → silent, never a borrowed tip.
        save("Theremin", Some(&flat), 200);
        assert!(
            method_book_tip_impl(&s).is_none(),
            "an instrument outside the catalog has no family pedagogy"
        );
    }

    /// #419 S1: preview is PURE — no active exploration, no exercise log.
    /// Begin commits both. Fails if preview grows side effects (a preview
    /// that hijacks the session on every keystroke) or Begin loses them.
    #[test]
    fn opener_preview_is_pure_and_begin_commits() {
        let state = AppState::with_mocks();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];

        let preview = opener_impl(&state, &items, None, None, false, FoldWindow::default())
            .expect("preview compiles");
        assert!(state.active_explore.lock_or_recover().is_none());
        // Review MF4: preview fires on EVERY tap and must not touch the
        // exercise log either — S3's My Patterns reads that log, and a
        // regression would flood it with half-built recipes.
        assert!(
            state
                .session_store
                .lock_or_recover()
                .list_exercise_log()
                .unwrap()
                .is_empty(),
            "a pure preview must not write the exercise log"
        );

        let begun = opener_impl(&state, &items, None, None, true, FoldWindow::default())
            .expect("begin compiles");
        assert!(state.active_explore.lock_or_recover().is_some());
        let log = state
            .session_store
            .lock_or_recover()
            .list_exercise_log()
            .unwrap();
        let sources: Vec<&str> = log.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(sources, vec!["opener"], "Begin logs exactly one opener row");
        // Deterministic seed: the preview IS the exercise.
        assert_eq!(preview.label, begun.label);
        assert_eq!(preview.music_xml, begun.music_xml);
    }

    /// #419 S2b AC1/AC4: the opener rows from the given live tonic (A=9)
    /// and defaults to C; preview and Begin stay deterministic with the
    /// same items+tonic+direction.
    #[test]
    fn opener_rows_from_the_live_tonic() {
        let state = AppState::with_mocks();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];
        let in_a =
            opener_impl(&state, &items, Some(9), None, false, FoldWindow::default()).unwrap();
        let in_c = opener_impl(&state, &items, None, None, false, FoldWindow::default()).unwrap();
        assert_ne!(in_a.music_xml, in_c.music_xml, "A row differs from C row");
        assert_eq!(in_a.root_pitch_classes.first(), Some(&9u8), "starts in A");
        assert_eq!(in_c.root_pitch_classes.first(), Some(&0u8), "defaults to C");
        // Wild wire tonic folds instead of panicking (AC edge).
        let folded = opener_impl(
            &state,
            &items,
            Some(120 + 9),
            None,
            false,
            FoldWindow::default(),
        )
        .unwrap();
        assert_eq!(folded.music_xml, in_a.music_xml, "120+9 folds to A");
        // Determinism: preview IS the exercise, with the new params too.
        // Round-3 review MF2: begin with the UNFOLDED value — the log row
        // is the fold's only observable seam (music_xml folds internally).
        let begun = opener_impl(
            &state,
            &items,
            Some(120 + 9),
            None,
            true,
            FoldWindow::default(),
        )
        .unwrap();
        assert_eq!(in_a.music_xml, begun.music_xml);
        // Review MF4: the exercise-log row records the LIVE tonic — the
        // % 12 fold's only observable seam, and what S4 recall will read.
        let log = state
            .session_store
            .lock_or_recover()
            .list_exercise_log()
            .unwrap();
        let last = log.last().expect("Begin logged a row");
        assert_eq!(last.source, "opener");
        assert_eq!(last.tonic, 9, "logged tonic is the folded live key");
    }

    /// #419 S2b AC3/AC7: directions re-voice the row — reversed differs
    /// from forward, varied is seed-stable — and junk refuses by name.
    #[test]
    fn opener_directions_revoice_and_refuse_calmly() {
        let state = AppState::with_mocks();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];
        let forward = opener_impl(
            &state,
            &items,
            None,
            Some("forward"),
            false,
            FoldWindow::default(),
        )
        .unwrap();
        let default =
            opener_impl(&state, &items, None, None, false, FoldWindow::default()).unwrap();
        assert_eq!(
            forward.music_xml, default.music_xml,
            "forward IS the default"
        );
        let reversed = opener_impl(
            &state,
            &items,
            None,
            Some("reversed"),
            false,
            FoldWindow::default(),
        )
        .unwrap();
        assert_ne!(reversed.music_xml, forward.music_xml);
        // Review MF3: "reversed" must BE the reversal — the first root
        // segment's pitch steps read backwards, not merely differently.
        let steps = |xml: &str| -> Vec<String> {
            let seg = xml.split("<measure number=\"2\">").next().unwrap();
            seg.match_indices("<step>")
                .map(|(i, _)| {
                    let rest = &seg[i + 6..];
                    rest[..rest.find("</step>").unwrap()].to_string()
                })
                .collect()
        };
        let fwd_steps = steps(&forward.music_xml);
        let mut rev_expected = fwd_steps.clone();
        rev_expected.reverse();
        assert_eq!(
            steps(&reversed.music_xml),
            rev_expected,
            "reversed is the reversal of forward's first segment"
        );
        let varied_a = opener_impl(
            &state,
            &items,
            None,
            Some("varied"),
            false,
            FoldWindow::default(),
        )
        .unwrap();
        let varied_b = opener_impl(
            &state,
            &items,
            None,
            Some("varied"),
            false,
            FoldWindow::default(),
        )
        .unwrap();
        assert_eq!(
            varied_a.music_xml, varied_b.music_xml,
            "varied is seed-stable"
        );
        assert_ne!(
            varied_a.music_xml, forward.music_xml,
            "varied differs from forward (AC3)"
        );
        let err = opener_impl(
            &state,
            &items,
            None,
            Some("sideways"),
            false,
            FoldWindow::default(),
        )
        .unwrap_err();
        assert!(err.contains("forward, reversed, or varied"), "got: {err}");
        assert!(
            state.active_explore.lock_or_recover().is_none(),
            "a refused direction commits nothing"
        );
    }

    /// The player-facing refusals surface verbatim through the command.
    #[test]
    fn opener_refuses_calmly_on_empty_and_bad_degrees() {
        let state = AppState::with_mocks();
        let err = opener_impl(&state, &[], None, None, false, FoldWindow::default()).unwrap_err();
        assert!(err.contains("add a note or two"), "got: {err}");
        let err = opener_impl(
            &state,
            &[brain::starter::StarterItem::NoteSequence { degrees: vec![13] }],
            None,
            None,
            false,
            FoldWindow::default(),
        )
        .unwrap_err();
        assert!(err.contains("1 to 12"), "got: {err}");
        assert!(state.active_explore.lock_or_recover().is_none());
    }

    /// #419 S4 AC1: recipes round-trip most-recent-first, delete removes,
    /// a garbage row is skipped calmly (never errors the panel), and the
    /// refusals speak by name.
    #[test]
    fn recipes_save_list_delete_and_skip_garbage() {
        let s = AppState::with_mocks();
        let notes = vec![brain::starter::StarterItem::Notes {
            offsets: vec![0, 4, 7],
        }];
        let seq = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];
        let first = save_opener_recipe_impl(&s, "Morning triad", &notes, "forward").unwrap();
        let second = save_opener_recipe_impl(&s, "The classic", &seq, "reversed").unwrap();
        let listed = list_opener_recipes_impl(&s);
        assert_eq!(
            listed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["The classic", "Morning triad"],
            "most recent first"
        );
        assert_eq!(listed[0].direction, "reversed");
        assert_eq!(listed[0].items, seq);
        s.session_store
            .lock_or_recover()
            .delete_recipe(second.id)
            .unwrap();
        let listed = list_opener_recipes_impl(&s);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, first.id);
        // A stale row whose items no longer parse is SKIPPED, calmly.
        s.session_store
            .lock_or_recover()
            .save_recipe("bad", "not json", "forward")
            .unwrap();
        assert_eq!(list_opener_recipes_impl(&s).len(), 1);
        // Refusals, by name.
        let err = save_opener_recipe_impl(&s, "   ", &notes, "forward").unwrap_err();
        assert!(err.contains("name"), "got: {err}");
        let err = save_opener_recipe_impl(&s, "Empty", &[], "forward").unwrap_err();
        assert!(err.contains("empty opener"), "got: {err}");
        let err = save_opener_recipe_impl(&s, "Bad dir", &notes, "sideways").unwrap_err();
        assert!(err.contains("forward, reversed, or varied"), "got: {err}");
        // The compile pre-flight: an uncompilable recipe refuses at SAVE
        // time with the backend's named message — never saved, never a
        // strip chip that won't open.
        let err = save_opener_recipe_impl(
            &s,
            "Bad degree",
            &[brain::starter::StarterItem::NoteSequence { degrees: vec![13] }],
            "forward",
        )
        .unwrap_err();
        assert!(err.contains("1 to 12"), "got: {err}");
        assert_eq!(list_opener_recipes_impl(&s).len(), 1, "nothing was saved");
    }

    /// #419 S4 AC3: recall's absences are honest — empty log, non-opener
    /// rows, and an opener row whose spec no longer parses all offer
    /// None, never a guess.
    #[test]
    fn recall_last_opener_absences_are_honest() {
        let s = AppState::with_mocks();
        assert_eq!(recall_last_opener_impl(&s), None, "empty log");
        let log = |source: &str, spec_json: &str| {
            s.session_store
                .lock_or_recover()
                .log_exercise(&brain::store::ExerciseLogEntry {
                    source: source.into(),
                    label: "a row".into(),
                    spec_json: spec_json.into(),
                    seed: 1,
                    difficulty: 1,
                    tonic: 0,
                    accuracy: None,
                })
                .unwrap();
        };
        // Review MF1: the lift row carries a fully PARSABLE spec WITH a
        // cell — the None below can only come from the SOURCE filter,
        // not from a convenient parse failure.
        let model = brain::learner::LearnerModel::default();
        let (lift_state, _) = brain::coach::start_explore_cell(
            vec![0, 4, 7],
            0,
            &model,
            1,
            brain::coach::DirectionMode::Forward,
        );
        let lift_spec = serde_json::to_string(&lift_state.spec).unwrap();
        log("lift", &lift_spec);
        assert_eq!(
            recall_last_opener_impl(&s),
            None,
            "non-opener rows are not yesterday's opener"
        );
        log("opener", "not json");
        assert_eq!(
            recall_last_opener_impl(&s),
            None,
            "an unparsable opener row offers nothing"
        );
        let err = begin_opener_recall_impl(&s, FoldWindow::default()).unwrap_err();
        assert!(err.contains("no opener to recall"), "got: {err}");
        assert!(s.active_explore.lock_or_recover().is_none());
    }

    /// #419 S4 AC4 — the stored-seed law: recall replays the log row's
    /// SEED, never a fresh cell hash. The row is written with a seed the
    /// hash could never have produced (simulating a hash function that
    /// drifted across releases); recall must follow the ROW. The
    /// recompute mutant dies on the spec comparison.
    #[test]
    fn recall_replays_the_stored_seed_not_a_rehash() {
        let s = AppState::with_mocks();
        let cell: Vec<i8> = vec![0, 4, 7, 12];
        let stored_seed: u64 = 777;
        let model = brain::learner::LearnerModel::default();
        // Varied direction makes the seed load-bearing (it drives the
        // per-root contour draws), so a wrong seed is VISIBLE in the
        // rendered sequence even though the spec is seed-independent.
        let (original, original_seq) = brain::coach::start_explore_cell(
            cell.clone(),
            0,
            &model,
            stored_seed,
            brain::coach::DirectionMode::RandomPerRoot,
        );
        let original_xml = explore_dto(&original, &original_seq).music_xml;
        // The seed opener_impl would hash today — and proof the pin has
        // teeth: that seed produces a DIFFERENT spec than the stored one.
        let rehash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            cell.hash(&mut h);
            h.finish()
        };
        assert_ne!(rehash, stored_seed);
        let (rehashed, rehashed_seq) = brain::coach::start_explore_cell(
            cell.clone(),
            0,
            &model,
            rehash,
            brain::coach::DirectionMode::RandomPerRoot,
        );
        assert_ne!(
            explore_dto(&rehashed, &rehashed_seq).music_xml,
            original_xml,
            "the two seeds must differ visibly or this pin is vacuous"
        );
        s.session_store
            .lock_or_recover()
            .log_exercise(&brain::store::ExerciseLogEntry {
                source: "opener".into(),
                label: "yesterday".into(),
                spec_json: serde_json::to_string(&original.spec).unwrap(),
                seed: stored_seed,
                difficulty: original.difficulty,
                tonic: 0,
                accuracy: None,
            })
            .unwrap();
        let replay = begin_opener_recall_impl(&s, FoldWindow::default()).expect("recall replays");
        assert_eq!(
            replay.music_xml, original_xml,
            "recall follows the STORED seed"
        );
        let guard = s.active_explore.lock_or_recover();
        let replayed = guard.as_ref().expect("recall commits an exploration");
        assert_eq!(replayed.seed, stored_seed);
        drop(guard);
        // Recall chains: the fresh log row carries the SAME seed, so
        // recalling a recall replays the same opener again.
        let last = s
            .session_store
            .lock_or_recover()
            .latest_exercise_for_source("opener")
            .unwrap()
            .expect("recall logged a fresh row");
        assert_eq!(last.seed, stored_seed);
    }

    /// #419 S4 review MF2: recall is exact ACROSS learner-model drift —
    /// the stored spec replays wholesale, so a difficulty that moved
    /// since the row was logged retunes nothing (tempo, root count). The
    /// rebuild-from-today's-model mutant dies here.
    #[test]
    fn recall_is_exact_across_learner_model_drift() {
        let s = AppState::with_mocks();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];
        let begun = opener_impl(&s, &items, None, None, true, FoldWindow::default()).unwrap();
        // The player got better overnight: the adaptive difficulty moves.
        let mut model = s
            .session_store
            .lock_or_recover()
            .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
            .unwrap()
            .unwrap_or_default();
        model.difficulty = brain::learner::MAX_DIFFICULTY;
        s.session_store
            .lock_or_recover()
            .upsert_learner_model(LOCAL_TASTE_PROFILE_USER_ID, &model)
            .unwrap();
        let replay = begin_opener_recall_impl(&s, FoldWindow::default()).expect("recall replays");
        assert_eq!(
            replay.music_xml, begun.music_xml,
            "a drifted difficulty must not retune the replay"
        );
    }

    /// #419 S4 AC5: a Reversed opener comes back Reversed — recall honors
    /// the stored direction. A Forward-pinning mutant changes the row
    /// order and dies on the XML comparison.
    #[test]
    fn reversed_opener_recalls_reversed() {
        let s = AppState::with_mocks();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];
        let begun = opener_impl(
            &s,
            &items,
            None,
            Some("reversed"),
            true,
            FoldWindow::default(),
        )
        .unwrap();
        let replay = begin_opener_recall_impl(&s, FoldWindow::default()).expect("recall replays");
        assert_eq!(
            replay.music_xml, begun.music_xml,
            "recall IS the begun opener, reversal included"
        );
        let guard = s.active_explore.lock_or_recover();
        assert_eq!(
            guard.as_ref().unwrap().spec.direction,
            brain::coach::DirectionMode::Reversed
        );
    }

    /// #253 S2 M2 (command wiring): `AppState::enrich_reveal` actually delegates
    /// to the coaching service — an online LLM-backed state returns the enriched
    /// reveal. Fails if `get_reveal`'s enrichment hop is bypassed at the state
    /// layer.
    #[tokio::test]
    async fn app_state_enrich_reveal_delegates_to_the_service() {
        let mut s = AppState::with_mocks();
        s.coaching_service = Arc::new(LlmCoachingService::with_engine(
            online_engine_answering_why("Enriched by the wire test."),
        ));
        let out = s.enrich_reveal(grounded_reveal()).await;
        assert_eq!(out.why, "Enriched by the wire test.");
        assert_eq!(out.source, brain::connections::RevealSource::LlmGrounded);
    }

    fn sample_context() -> SessionContext {
        SessionContext {
            instrument: "Trumpet".to_owned(),
            session_duration_secs: 60.0,
            phrases_played: 1,
            previous_tips: Vec::new(),
            score_title: None,
        }
    }

    #[tokio::test]
    async fn disabling_coaching_makes_get_tip_offline_no_http_call() {
        // Wire a real LLM service backed by a panicking client, then disable
        // coaching via the command-layer threading. The engine must go Offline
        // and return NO live tip — never touching the client, never fabricating
        // canned encouragement (silence beats a lie).
        let mut s = AppState::with_mocks();
        s.coaching_service = Arc::new(LlmCoachingService::with_engine(
            online_engine_with_panicking_client(),
        ));

        // The FE pref said OFF → Offline.
        s.set_coaching_network_policy(false).await;

        let tip = s
            .coaching_service
            .get_tip(&sample_phrase(), &sample_context())
            .await;
        assert!(
            tip.is_none(),
            "offline live tip must be silent (None), never a canned fallback"
        );
    }

    #[tokio::test]
    async fn disabling_coaching_makes_recap_offline_no_http_call() {
        let mut s = AppState::with_mocks();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));

        s.set_coaching_network_policy(false).await;

        let input = RecapInput {
            instrument: "Trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 300.0,
            practice_mode: PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        // Must not panic (no HTTP) and must return the grounded fallback recap.
        let recap = s
            .recap_generator
            .generate_recap(&input)
            .await
            .expect("offline recap still succeeds via on-device fallback");
        assert!(!recap.overall_assessment.is_empty());
        assert!(
            recap.connections.is_empty(),
            "offline recap must not carry LLM connections"
        );
    }

    /// #389 acceptance + #417-4 at the command layer: an OFFLINE piano
    /// recap — family resolved from the real catalog — contains no tuner/
    /// drone/long-tone advice and speaks keyboard practice language, while
    /// the same session on trumpet keeps the wind opener. Fails if the
    /// family stops reaching the recap composer.
    #[tokio::test]
    async fn offline_piano_recap_never_suggests_a_tuner() {
        let mut s = AppState::with_mocks();
        s.recap_generator = Arc::new(LlmRecapGenerator::with_engine(
            online_engine_with_panicking_client(),
        ));
        s.set_coaching_network_policy(false).await;
        assert_eq!(instrument_family_for(&s, "Piano"), "Keyboard");

        let input_for = |instrument: &str| RecapInput {
            instrument: instrument.to_owned(),
            instrument_family: instrument_family_for(&s, instrument),
            duration_secs: 300.0,
            practice_mode: PracticeMode::default(),
            // #445-6b: settled phrases — this pins the FULL recap's
            // family vocabulary, not the thin short form.
            phrases: (0..3)
                .map(|_| {
                    let mut p = sample_phrase();
                    p.duration_secs = 7.0;
                    p
                })
                .collect(),
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let text_of = |r: &brain::session::SessionRecap| {
            format!(
                "{} {} {} {}",
                r.overall_assessment,
                r.strengths.join(" "),
                r.areas_to_improve.join(" "),
                r.next_session_suggestions.join(" ")
            )
            .to_lowercase()
        };

        let piano = s
            .recap_generator
            .generate_recap(&input_for("Piano"))
            .await
            .expect("offline piano recap succeeds");
        let piano_text = text_of(&piano);
        for forbidden in ["tuner", "drone", "long tones"] {
            assert!(
                !piano_text.contains(forbidden),
                "piano recap must not say {forbidden:?}: {piano_text}"
            );
        }
        assert!(
            piano_text.contains("slow scale"),
            "keyboard warmup vocabulary expected: {piano_text}"
        );

        let trumpet = s
            .recap_generator
            .generate_recap(&input_for("Trumpet"))
            .await
            .expect("offline trumpet recap succeeds");
        assert!(
            text_of(&trumpet).contains("long tones"),
            "trumpet keeps the wind opener"
        );
    }

    #[tokio::test]
    async fn start_session_with_coaching_disabled_threads_offline_policy() {
        // End-to-end at the command boundary: starting a session with
        // coaching_enabled = false must put both engines Offline so that a
        // subsequent tip request never reaches the (panicking) client.
        let mut s = AppState::with_mocks();
        s.coaching_service = Arc::new(LlmCoachingService::with_engine(
            online_engine_with_panicking_client(),
        ));

        let id = start_practice_session_impl(
            &s,
            "Trumpet".to_owned(),
            PracticeMode::default(),
            false, // coaching disabled
            None,
        )
        .await
        .expect("session starts");
        assert!(!id.is_empty());

        // A tip request now would have panicked if the policy hadn't been
        // threaded to Offline. Offline yields no live tip (silence beats a lie).
        let tip = s
            .coaching_service
            .get_tip(&sample_phrase(), &sample_context())
            .await;
        assert!(
            tip.is_none(),
            "offline live tip must be silent (None), never a canned fallback"
        );
    }

    // -----------------------------------------------------------------------
    // Live coaching loop wiring (record_coaching_tip + previous_tips threading)
    // -----------------------------------------------------------------------

    fn focus_tip(text: &str) -> CoachingTip {
        CoachingTip {
            text: text.to_owned(),
            severity: CoachingSeverity::Suggestion,
            category: CoachingCategory::Tone,
        }
    }

    #[tokio::test]
    async fn record_coaching_tip_persists_into_active_session() {
        let s = state();
        start_practice_session_impl(&s, "Trumpet".to_owned(), PracticeMode::Practice, true, None)
            .await
            .expect("session starts");

        s.record_coaching_tip(0, &focus_tip("Let the phrase breathe at the top."))
            .await
            .expect("recording a tip into an active session succeeds");

        // It lands in the recorder, surfaced via the recent-tips accessor.
        let recent = s.recent_tip_texts(5).await;
        assert_eq!(
            recent,
            vec!["Let the phrase breathe at the top.".to_owned()]
        );
    }

    #[tokio::test]
    async fn record_coaching_tip_errors_with_no_active_session() {
        let s = state();
        let err = s
            .record_coaching_tip(0, &focus_tip("nope"))
            .await
            .expect_err("recording with no session must error");
        assert!(matches!(err, CommandError::NotActive));
    }

    #[tokio::test]
    async fn recent_tip_texts_threads_recent_tips_in_order_and_is_capped() {
        let s = state();
        start_practice_session_impl(&s, "Trumpet".to_owned(), PracticeMode::Practice, true, None)
            .await
            .expect("session starts");

        for i in 0..7 {
            s.record_coaching_tip(i, &focus_tip(&format!("tip {i}")))
                .await
                .expect("record tip");
        }

        // Only the most recent 5 are threaded, oldest-first within the window.
        let recent = s.recent_tip_texts(5).await;
        assert_eq!(
            recent,
            vec![
                "tip 2".to_owned(),
                "tip 3".to_owned(),
                "tip 4".to_owned(),
                "tip 5".to_owned(),
                "tip 6".to_owned(),
            ],
            "previous_tips window must carry the trailing tips, oldest-first"
        );
    }

    #[tokio::test]
    async fn recent_tip_texts_empty_with_no_active_session() {
        let s = state();
        assert!(s.recent_tip_texts(5).await.is_empty());
    }

    // -----------------------------------------------------------------------
    // #449 T1: practice_events emitters, coalescing, integrity columns
    // -----------------------------------------------------------------------

    /// The journal for one session as `(kind, params_json)` in clock order —
    /// the assertion surface every T1 emitter test reads.
    fn journal(s: &AppState, session_id: &str) -> Vec<(String, String)> {
        s.session_store
            .lock_or_recover()
            .list_practice_events(session_id)
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.params_json))
            .collect()
    }

    /// Total journal rows across all sessions — the "nothing was written
    /// anywhere" assertion.
    fn journal_len(s: &AppState) -> usize {
        s.session_store
            .lock_or_recover()
            .count_practice_events()
            .unwrap()
    }

    async fn start_session(s: &AppState) -> String {
        start_practice_session_impl(s, "Trumpet".to_owned(), PracticeMode::Practice, false, None)
            .await
            .expect("session starts")
    }

    /// AC5 (the pure gate): a `pocket_tempo` row needs BOTH a ≥5 BPM move
    /// from the last journaled value AND ≥5 s since it — either alone is a
    /// wobble, not a settled change. First-ever push logs (no baseline).
    #[test]
    fn tempo_log_due_gates_on_delta_and_gap() {
        assert!(tempo_log_due(None, 0.0, 90.0, 0.0), "no baseline → log");
        // Big jump, too soon: the follow stream's re-lock burst.
        assert!(!tempo_log_due(Some(90.0), 10.0, 130.0, 12.0));
        // Settled long, but a wobble: ±2 BPM around a locked pulse.
        assert!(!tempo_log_due(Some(90.0), 10.0, 92.0, 200.0));
        // Both gates pass — this is a real ramp step.
        assert!(tempo_log_due(Some(90.0), 10.0, 95.0, 15.0));
        assert!(tempo_log_due(Some(90.0), 10.0, 85.0, 15.0), "downward too");
        // Boundary: exactly 5 BPM and exactly 5 s both count as settled.
        assert!(tempo_log_due(Some(90.0), 10.0, 95.0, 15.0));
        assert!(!tempo_log_due(Some(90.0), 10.0, 94.9, 15.0));
        assert!(!tempo_log_due(Some(90.0), 10.0, 95.0, 14.9));
    }

    /// AC3/AC4 (the no-session contract): every writer is a calm no-op with
    /// no active session — tool use outside practice fabricates no evidence.
    #[test]
    fn no_session_emits_no_practice_events() {
        let s = state();
        log_practice_event_best_effort(&s, "score_open", serde_json::json!({"score_id": "x"}));
        note_pocket_started(&s, 90.0, true);
        note_pocket_tempo(&s, 120.0);
        note_pocket_stopped(&s);
        assert_eq!(journal_len(&s), 0, "no session → no rows, from any writer");
    }

    /// AC3: inside a session, rows land under the session's id with a
    /// non-negative session-clock offset; ending the session closes the
    /// journal (a straggler write after end journals nothing).
    #[tokio::test]
    async fn practice_events_carry_the_session_clock_and_close_with_it() {
        let s = state();
        let sid = start_session(&s).await;
        log_practice_event_best_effort(&s, "score_open", serde_json::json!({"score_id": "x"}));

        let events = s
            .session_store
            .lock_or_recover()
            .list_practice_events(&sid)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "score_open");
        assert!(
            (0.0..60.0).contains(&events[0].at_secs),
            "at_secs must be a small offset from session start, got {}",
            events[0].at_secs
        );

        end_practice_session_impl(&s).await.expect("session ends");
        assert!(
            s.telemetry.lock_or_recover().is_none(),
            "the telemetry context must close with the session"
        );
        log_practice_event_best_effort(&s, "score_open", serde_json::json!({"score_id": "y"}));
        assert_eq!(journal_len(&s), 1, "post-end writes journal nothing");
    }

    // -----------------------------------------------------------------------
    // #449 T2: the dashboard sync projection readers (spec AC10 / AC4)
    // -----------------------------------------------------------------------

    /// AC10: `get_session_projection` returns the P1 fact (with the
    /// close-time integrity aggregates and session meta), the P2 phrases,
    /// and the P4 events for exactly the closed session. Fails if the
    /// integrity columns stop flowing (the dashboard would re-derive and
    /// drift) or events lose their device ids (idempotency key).
    #[tokio::test]
    async fn session_projection_carries_integrity_meta_phrases_and_events() {
        let s = state();
        let sid = start_session(&s).await;
        log_practice_event_best_effort(&s, "score_open", serde_json::json!({"score_id": "x"}));
        {
            let mut guard = s.active_session.lock().await;
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(sample_phrase())
                .unwrap();
        }
        end_practice_session_impl(&s).await.expect("session ends");

        let proj = get_session_projection_impl(&s, sid.clone()).unwrap();
        assert_eq!(proj.session.id, sid, "device_session_id is the local id");
        assert_eq!(proj.session.instrument, "Trumpet");
        assert_eq!(proj.session.practice_mode.as_deref(), Some("Practice"));
        assert!(proj.session.app_version.is_some());
        // Integrity: computed once at close (T1), projected verbatim.
        let played = proj.session.played_secs.expect("played clock persisted");
        assert!((played - 2.0).abs() < 1e-9, "Σ phrase spans = 2.0");
        assert_eq!(proj.session.note_count, Some(6));
        assert!(proj.session.silence_ratio.is_some());
        assert!(proj.session.score.is_none(), "free play → no material link");

        assert_eq!(proj.phrases.len(), 1);
        let p = &proj.phrases[0];
        assert_eq!(
            (p.phrase_index, p.start_secs, p.end_secs, p.note_count),
            (0, 0.0, 2.0, 6)
        );
        assert!(p.key_name.is_none(), "no key evidence → no key claim");

        assert_eq!(proj.events.len(), 1);
        assert_eq!(proj.events[0].kind, "score_open");
        assert!(proj.events[0].device_event_id > 0, "cloud idempotency key");
    }

    /// AC4 (structural privacy pin, P2): the thin phrase DTO serializes
    /// EXACTLY the doc-§2 field list — no `phrase_json`, no `onsets`, no
    /// pitch curves. A field added to `PhraseFactDto` (or renamed) fails
    /// here before it can widen what crosses on the sync path.
    #[tokio::test]
    async fn phrase_fact_dto_is_structurally_thin() {
        let s = state();
        let sid = start_session(&s).await;
        {
            let mut guard = s.active_session.lock().await;
            let mut phrase = sample_phrase();
            phrase.onsets_secs = vec![0.1, 0.5, 0.9]; // present locally…
            guard
                .as_mut()
                .unwrap()
                .recorder
                .record_phrase(phrase)
                .unwrap();
        }
        end_practice_session_impl(&s).await.expect("session ends");

        let proj = get_session_projection_impl(&s, sid).unwrap();
        let json = serde_json::to_value(&proj.phrases[0]).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "end_secs",
                "key_name",
                "note_count",
                "phrase_index",
                "stability",
                "start_secs",
                "tone"
            ],
            "…but the projection must not carry onsets/pitch curves (doc §2 P2)"
        );
    }

    /// AC10: the P3 reader is watermark-incremental through the command
    /// layer too, and its rows are the spec_json/seed-free shape (the
    /// store-level pin covers serialization; this covers the wiring).
    #[test]
    fn list_exercise_facts_impl_respects_the_watermark() {
        let s = state();
        {
            let store = s.session_store.lock_or_recover();
            for tonic in [0u8, 7u8] {
                store
                    .log_exercise(&brain::store::ExerciseLogEntry {
                        source: "opener".to_owned(),
                        label: "Minor triad, enclosed".to_owned(),
                        spec_json: "{\"cell\":\"m\"}".to_owned(),
                        seed: 1,
                        difficulty: 2,
                        tonic,
                        accuracy: None,
                    })
                    .unwrap();
            }
        }
        let all = list_exercise_facts_impl(&s, 0).unwrap();
        assert_eq!(all.len(), 2);
        let rest = list_exercise_facts_impl(&s, all[0].id).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].tonic, 7);
    }

    /// AC5 (the stream): a follow-mode burst of pushes journals NO tempo
    /// rows (the 5 s gate holds even for big jumps); once the gap has
    /// passed, one settled change journals exactly one row; and the final
    /// effective tempo still reaches `pocket_stop` untruncated — coalescing
    /// thins the journal, never the truth.
    #[tokio::test]
    async fn pocket_tempo_stream_coalesces_to_few_rows() {
        let s = state();
        let sid = start_session(&s).await;
        note_pocket_started(&s, 90.0, true);

        // The follow stream: ~a dozen pushes in the same instant — wobbles
        // AND big re-lock jumps. All inside the 5 s gate → zero rows.
        for bpm in [
            90.5, 91.0, 89.0, 92.0, 130.0, 60.0, 91.5, 92.5, 93.0, 94.0, 118.0, 90.0,
        ] {
            note_pocket_tempo(&s, bpm);
        }
        let kinds = |s: &AppState| {
            journal(s, &sid)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(&s),
            vec!["pocket_start"],
            "a rapid stream must coalesce to zero tempo rows"
        );

        // Rewind the gate as if the last journaled row were long ago, then
        // one settled change journals exactly once.
        s.telemetry
            .lock_or_recover()
            .as_mut()
            .expect("session context")
            .tempo_last_logged_at_secs = -10.0;
        note_pocket_tempo(&s, 120.0);
        note_pocket_tempo(&s, 121.0); // immediately after → gated again
        assert_eq!(kinds(&s), vec!["pocket_start", "pocket_tempo"]);

        // Review round 1 MF1: the delta gate's baseline is the last
        // JOURNALED value (120), never the last PUSHED one (121). With the
        // time gate rewound, 125.5 must journal: |125.5 − 120| = 5.5 ≥ 5.
        // A mutant comparing against the last pushed value stays silent
        // (|125.5 − 121| = 4.5 < 5) and dies on this assert.
        s.telemetry
            .lock_or_recover()
            .as_mut()
            .expect("session context")
            .tempo_last_logged_at_secs = -10.0;
        note_pocket_tempo(&s, 125.5);
        assert_eq!(
            kinds(&s),
            vec!["pocket_start", "pocket_tempo", "pocket_tempo"],
            "a ≥5 BPM move from the last JOURNALED row must journal"
        );
        assert!(
            journal(&s, &sid)[2].1.contains("125.5"),
            "the third row carries the settled tempo: {}",
            journal(&s, &sid)[2].1
        );

        note_pocket_tempo(&s, 126.0); // gated wobble — tracked, not journaled
        note_pocket_stopped(&s);
        let events = journal(&s, &sid);
        let (stop_kind, stop_params) = events.last().unwrap();
        assert_eq!(stop_kind, "pocket_stop");
        assert!(
            stop_params.contains("126"),
            "pocket_stop must report the last EFFECTIVE tempo (126), \
             not the last journaled one (125.5): {stop_params}"
        );
        let start_params = &events[0].1;
        assert!(
            start_params.contains("\"mode\":\"anchor\""),
            "{start_params}"
        );
        assert!(start_params.contains("\"count_in\":true"), "{start_params}");
    }

    /// AC4: Begin journals `opener_begin` inside a session and nothing
    /// outside one — while the exercise-log row (material evidence) is
    /// written either way, exactly as before this slice.
    #[tokio::test]
    async fn begin_opener_journals_only_in_session() {
        let s = state();
        let items = vec![brain::starter::StarterItem::NoteSequence {
            degrees: vec![1, 2, 3, 5],
        }];

        opener_impl(&s, &items, None, None, true, FoldWindow::default())
            .expect("begin outside a session");
        assert_eq!(journal_len(&s), 0, "no session → no opener_begin row");
        assert_eq!(
            s.session_store
                .lock_or_recover()
                .list_exercise_log()
                .unwrap()
                .len(),
            1,
            "the exercise log keeps its row regardless (unchanged behavior)"
        );

        let sid = start_session(&s).await;
        opener_impl(&s, &items, None, None, true, FoldWindow::default())
            .expect("begin inside a session");
        assert_eq!(
            journal(&s, &sid),
            vec![("opener_begin".to_owned(), "{\"recipe\":null}".to_owned())]
        );
    }

    /// AC4: a successful `get_score` journals `score_open` with the id —
    /// only inside a session (library browsing journals nothing).
    #[tokio::test]
    async fn get_score_journals_score_open_only_in_session() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();
        let score_id = {
            let store = state.score_store.lock_or_recover();
            store
                .import(
                    "Etude".to_owned(),
                    None,
                    "etude.musicxml".to_owned(),
                    "<score-partwise/>".to_owned(),
                    0,
                    4,
                )
                .unwrap()
                .id
                .as_str()
        };

        get_score(state.clone(), score_id.clone()).expect("load outside a session");
        assert_eq!(journal_len(state.inner()), 0);

        let sid = start_session(state.inner()).await;
        get_score(state.clone(), score_id.clone()).expect("load inside a session");
        let events = journal(state.inner(), &sid);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "score_open");
        assert!(
            events[0].1.contains(&score_id),
            "params must carry the score id: {}",
            events[0].1
        );
    }

    /// AC4: pinning the band's key journals `band_key_pin` with the pin as
    /// the command speaks it — (tonic, minor) — inside a session only.
    #[tokio::test]
    async fn set_accompaniment_key_journals_band_key_pin() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();

        set_accompaniment_key(state.clone(), 4, true)
            .await
            .expect("pin outside a session");
        assert_eq!(journal_len(state.inner()), 0);

        let sid = start_session(state.inner()).await;
        set_accompaniment_key(state.clone(), 4, true)
            .await
            .expect("pin inside a session");
        assert_eq!(
            journal(state.inner(), &sid),
            vec![(
                "band_key_pin".to_owned(),
                "{\"minor\":true,\"tonic\":4}".to_owned()
            )]
        );
    }

    /// AC4 (`pocket_mode` seam): the command validates the vocabulary and
    /// journals inside a session only. (The frontend wire rides T2/T4 —
    /// this pins the backend contract it will call into.)
    #[tokio::test]
    async fn set_pocket_mode_validates_and_journals() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();

        assert!(
            set_pocket_mode(state.clone(), "swing".to_owned()).is_err(),
            "unknown modes are refused, not journaled"
        );
        set_pocket_mode(state.clone(), "follow".to_owned()).expect("valid mode, no session");
        assert_eq!(journal_len(state.inner()), 0);

        let sid = start_session(state.inner()).await;
        set_pocket_mode(state.clone(), "follow".to_owned()).expect("valid mode, in session");
        assert_eq!(
            journal(state.inner(), &sid),
            vec![("pocket_mode".to_owned(), "{\"mode\":\"follow\"}".to_owned())]
        );
    }

    /// AC4 (narration): a coaching tip that actually surfaced (`Some`)
    /// journals `narration_used {"kind":"tip"}` — usage fact only, no tip
    /// content in the row.
    #[tokio::test]
    async fn coaching_tip_some_journals_narration_used() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let state = app.state::<AppState>();
        let sid = start_session(state.inner()).await;

        let tip = get_coaching_tip(state.clone(), sample_phrase(), 60.0, 1)
            .await
            .expect("tip command succeeds");
        assert!(tip.is_some(), "the mock service serves a tip");
        let events = journal(state.inner(), &sid);
        assert_eq!(
            events,
            vec![("narration_used".to_owned(), "{\"kind\":\"tip\"}".to_owned())],
            "one usage row, no content"
        );
    }

    /// AC7: closing a session with phrases persists the integrity columns
    /// computed from those phrases — and the mock (non-LLM) recap journals
    /// NO `narration_used {"kind":"recap"}` row (a narration that never
    /// fired must never be claimed).
    #[tokio::test]
    async fn session_close_persists_integrity_columns() {
        let s = state();
        let sid = start_session(&s).await;

        // Two phrases: 30 s + 30 s of played time, 90 voiced notes.
        let mut a = sample_phrase();
        a.start_time = 0.0;
        a.end_time = 30.0;
        a.note_count = 40;
        let mut b = sample_phrase();
        b.phrase_index = 1;
        b.start_time = 100.0;
        b.end_time = 130.0;
        b.note_count = 50;
        s.phrase_buffer.lock_or_recover().extend([a, b]);

        end_practice_session_impl(&s).await.expect("session ends");

        let summaries = s.session_store.lock_or_recover().list_recent(10).unwrap();
        let row = summaries
            .iter()
            .find(|r| r.id.as_str() == sid)
            .expect("the closed session persisted");
        assert_eq!(row.played_secs, Some(60.0), "Σ phrase durations");
        assert_eq!(row.note_count, Some(90), "Σ voiced notes");
        // The test session's real wall clock is milliseconds (or exactly 0 —
        // both happen depending on scheduler timing), so assert the ratio
        // against the SAME documented rule applied to the persisted wall:
        // wall <= 0 → 1.0, else 1 − played/wall clamped (here played ≫ wall,
        // so 0.0). This is exactly what a re-derivation in a dashboard would
        // compute — the row must agree with it.
        let expected_ratio = if row.duration_secs <= 0.0 {
            1.0
        } else {
            (1.0 - 60.0 / row.duration_secs).clamp(0.0, 1.0)
        };
        assert_eq!(
            row.silence_ratio,
            Some(expected_ratio),
            "the stored ratio must follow the documented wall rule (wall = {})",
            row.duration_secs
        );
        assert!(
            journal(&s, &sid).iter().all(|(k, _)| k != "narration_used"),
            "a mock recap is not an LLM narration"
        );
    }

    /// Review round 1 MF2: a NO-OP teardown journals nothing. With a live
    /// session (so the writers COULD write) and neither click nor band
    /// running, the stop commands must not fabricate `pocket_stop` /
    /// `band_stop` rows — this pins the teardown bool-return guard. A
    /// mutant whose teardowns report `true` unconditionally dies here.
    #[tokio::test]
    async fn noop_teardown_journals_no_stop_events() {
        use tauri::test::mock_app;
        use tauri::Manager;

        let app = mock_app();
        app.manage(AppState::with_mocks());
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        let sid = start_session(state.inner()).await;

        stop_pocket(handle.clone(), state.clone())
            .await
            .expect("stopping a silent click is a calm no-op");
        stop_accompaniment(handle, state.clone())
            .await
            .expect("stopping a silent band is a calm no-op");

        let events = journal(state.inner(), &sid);
        assert!(
            events.is_empty(),
            "nothing was running — a no-op teardown must journal no stop: {events:?}"
        );
    }

    /// Review round 1 MF3: the best-effort writer survives a genuinely
    /// broken journal MID-SESSION — it returns calmly (a `.expect()` mutant
    /// panics here) and the session still closes normally afterwards. The
    /// store-level half (the `Err` actually exists) is pinned in
    /// `brain::store::tests::log_practice_event_err_is_surfaced_to_the_swallowing_caller`;
    /// this is the command-layer swallow, exercised for real.
    #[tokio::test]
    async fn best_effort_writer_survives_a_broken_journal() {
        let s = state();
        let _sid = start_session(&s).await;
        s.session_store
            .lock_or_recover()
            .break_practice_events_for_tests();

        // Under the mutant this panics; correct code shrugs (one warn).
        log_practice_event_best_effort(&s, "score_open", serde_json::json!({ "score_id": "x" }));

        let recap = end_practice_session_impl(&s).await;
        assert!(
            recap.is_ok(),
            "a broken journal must never sink the session close: {:?}",
            recap.err()
        );
    }
}
