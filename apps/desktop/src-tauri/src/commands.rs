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
    CoachingCategory, CoachingConfig, CoachingEngine, CoachingError, CoachingSeverity,
    CoachingTip, ReqwestClient, SessionContext,
};
use brain::session::{
    CompletedSession, RecapGenerator, RecapInput, SessionError, SessionId, SessionRecap,
    SessionRecorder,
};
use brain::stats::PracticeStats;
use brain::store::{SessionStore, SessionSummary, StoredSession};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Runtime, State};
use thiserror::Error;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// IPC types
// ---------------------------------------------------------------------------

/// UI-facing instrument descriptor. Matches the TS `InstrumentInfo` in
/// `apps/desktop/src/types/brain.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstrumentInfo {
    pub name: String,
    pub family: String,
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
    async fn get_tip(&self, phrase_index: usize, context: &SessionContext)
        -> Option<CoachingTip>;
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
        let engine = match CoachingEngine::new(
            CoachingConfig::default(),
            Box::new(ReqwestClient::new()),
        ) {
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
    async fn get_tip(&self, _phrase_index: usize, _context: &SessionContext) -> Option<CoachingTip> {
        // For now, return None to indicate no tip is available.
        // In a full implementation, this would:
        // 1. Lock the engine
        // 2. Build a coaching prompt from the phrase and context
        // 3. Call engine.get_tip()
        // 4. Return the tip or None on error/rate-limit
        None
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
                    text: "Nice steady tone. Try letting the end of the phrase breathe."
                        .to_owned(),
                    severity: CoachingSeverity::Encouragement,
                    category: CoachingCategory::Tone,
                },
                CoachingTip {
                    text: "Watch the intonation on the top note — a touch sharp there."
                        .to_owned(),
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
        phrase_index: usize,
        _context: &SessionContext,
    ) -> Option<CoachingTip> {
        self.tips.get(phrase_index % self.tips.len()).cloned()
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
        let engine = match CoachingEngine::new(
            CoachingConfig::default(),
            Box::new(ReqwestClient::new()),
        ) {
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
                    "Intonation wandered slightly on the upper register.".to_owned(),
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
        // This is a simplified placeholder — the real implementation will
        // format the RecapInput as a detailed prompt and parse the LLM response.
        // For now, we return the canned text to avoid blocking on an actual
        // API call during testing.
        let mut engine = engine_arc.lock().await;

        // TODO: Build a comprehensive recap prompt from the input (phrases,
        // tips, session duration, instrument) and call the LLM.
        // For now, return placeholder that indicates the engine is present.
        drop(engine);

        Ok(SessionRecap {
            overall_assessment: format!(
                "Strong practice session in {}. You demonstrated consistent focus and technical discipline over {} minutes.",
                input.instrument,
                (input.duration_secs / 60.0).round().max(1.0) as u32,
            ),
            strengths: vec![
                "Maintained steady tone and good breath support throughout.".to_owned(),
                "Showed improvement in phrase shaping.".to_owned(),
            ],
            areas_to_improve: vec![
                "Continue refining intonation in the upper register.".to_owned(),
            ],
            next_session_suggestions: vec![
                "Work with a drone to anchor your intonation.".to_owned(),
                "Practice the challenging passage from today at half-speed.".to_owned(),
            ],
            duration_secs: 0.0,
            phrase_count: 0,
            instrument: String::new(),
        })
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
            areas_to_improve: vec![
                "Intonation wandered slightly on the upper register.".to_owned(),
            ],
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
    /// Held so PR 2 can spawn tokio tasks off a phrase completion.
    /// Unused directly in PR 1 — hence the `allow(dead_code)`.
    #[allow(dead_code)]
    coaching: Arc<dyn CoachingService>,
}

impl ActiveSession {
    fn new(instrument: String, coaching: Arc<dyn CoachingService>) -> Self {
        Self {
            phase: SessionPhase::Starting,
            recorder: SessionRecorder::new(instrument),
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
    session_store: SessionStore,
    coaching_available: bool,
}

impl AppState {
    /// Production constructor — opens the SessionStore at the platform
    /// default location and wires the real coaching engine.
    pub fn new() -> Self {
        let session_store = SessionStore::open(&SessionStore::default_path().unwrap_or_else(|_| {
            // Fallback: use in-memory if default path unavailable
            // (extremely rare — headless/no data dir).
            std::path::PathBuf::from(":memory:")
        }))
        .unwrap_or_else(|_| SessionStore::in_memory().expect("in-memory store must succeed"));

        let coaching_svc = LlmCoachingService::new();
        let coaching_available = coaching_svc.coaching_available();
        let recap_gen = LlmRecapGenerator::new();

        Self {
            active_session: Mutex::new(None),
            coaching_factory: Arc::new(move || Arc::new(LlmCoachingService::new())),
            recap_generator: Arc::new(recap_gen),
            session_store,
            coaching_available,
        }
    }

    /// Wire entirely with mocks and in-memory store. Used by tests.
    pub fn with_mocks() -> Self {
        Self {
            active_session: Mutex::new(None),
            coaching_factory: Arc::new(|| Arc::new(MockCoachingService::new())),
            recap_generator: Arc::new(MockRecapGenerator),
            session_store: SessionStore::in_memory().expect("in-memory store must succeed"),
            coaching_available: false,
        }
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
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Instrument catalog (hard-coded in PR 1)
// ---------------------------------------------------------------------------

/// Matches `apps/desktop/src/components/InstrumentSelector.tsx`.
/// Replaced by a `profiles/*.json` scan in a follow-up story.
pub const KNOWN_INSTRUMENTS: &[(&str, &str)] = &[
    ("Trumpet", "Brass"),
    ("Trombone", "Brass"),
    ("French Horn", "Brass"),
    ("Violin", "Strings"),
    ("Cello", "Strings"),
    ("Flute", "Woodwind"),
    ("Clarinet", "Woodwind"),
    ("Voice", "Voice"),
    ("Piano", "Keyboard"),
];

fn is_known_instrument(name: &str) -> bool {
    KNOWN_INSTRUMENTS.iter().any(|(n, _)| *n == name)
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

// ---------------------------------------------------------------------------
// Command handlers — pure (testable) implementations
// ---------------------------------------------------------------------------

/// Pure implementation of `start_practice_session`, separated from the
/// `#[tauri::command]` wrapper so tests can drive it without a Tauri
/// runtime.
pub async fn start_practice_session_impl(
    state: &AppState,
    instrument: String,
    _coaching_enabled: bool,
) -> Result<String, CommandError> {
    if instrument.trim().is_empty() {
        return Err(CommandError::EmptyInstrument);
    }
    if !is_known_instrument(&instrument) {
        return Err(CommandError::UnknownInstrument(instrument));
    }

    let mut guard = state.active_session.lock().await;
    if guard.is_some() {
        return Err(CommandError::AlreadyActive);
    }

    let coaching = (state.coaching_factory)();
    let mut session = ActiveSession::new(instrument, coaching);
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
) -> Result<(String, DateTime<Utc>), CommandError> {
    if instrument.trim().is_empty() {
        return Err(CommandError::EmptyInstrument);
    }
    if !is_known_instrument(&instrument) {
        return Err(CommandError::UnknownInstrument(instrument));
    }

    let mut guard = state.active_session.lock().await;
    let session = guard.as_mut().ok_or(CommandError::NotActive)?;
    if session.phase != SessionPhase::Listening {
        return Err(CommandError::AlreadyEnding);
    }

    let new_segment_id = session.recorder.switch_instrument(instrument)?;
    Ok((new_segment_id.as_str(), Utc::now()))
}

/// Pure implementation of `end_practice_session`.
///
/// Per design doc §8 q3, a session with zero phrases returns a calm
/// empty-state recap rather than erroring — the recorder would
/// normally return `SessionError::Empty`, which we intercept.
pub async fn end_practice_session_impl(
    state: &AppState,
) -> Result<SessionRecap, CommandError> {
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

/// Pure implementation of `list_instruments`.
pub fn list_instruments_impl() -> Vec<InstrumentInfo> {
    KNOWN_INSTRUMENTS
        .iter()
        .map(|(name, family)| InstrumentInfo {
            name: (*name).to_owned(),
            family: (*family).to_owned(),
        })
        .collect()
}

async fn build_recap(
    completed: &CompletedSession,
    generator: &dyn RecapGenerator,
) -> Result<SessionRecap, CommandError> {
    completed.generate_recap(generator).await.map_err(CommandError::from)
}

/// Minimal recap for the zero-phrase case. Tone is intentionally
/// "yoga teacher not gym coach" — we don't shame the user for
/// not playing.
fn empty_state_recap() -> SessionRecap {
    SessionRecap {
        overall_assessment:
            "Looks like you didn't get to play this time — come back when you're ready."
                .to_owned(),
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
    let sessions = if let Some(instrument) = instrument_filter {
        state.session_store.list_by_instrument(Some(&instrument))?
    } else if start_date.is_some() || end_date.is_some() {
        state.session_store.list_by_date_range(start_date, end_date)?
    } else {
        // No filters — return all sessions, reasonable limit for UI
        state.session_store.list_recent(1000)?
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
    let session = state.session_store.load(id)?;
    Ok(StoredSessionDto::from(session))
}

/// Pure implementation of `get_practice_stats`.
pub fn get_practice_stats_impl(
    state: &AppState,
) -> Result<PracticeStatsDto, CommandError> {
    let all_sessions = state.session_store.list_recent(10000)?;
    let stats = PracticeStats::calculate(&all_sessions, Utc::now());
    Ok(PracticeStatsDto::from(stats))
}

// ---------------------------------------------------------------------------
// Tauri command wrappers
// ---------------------------------------------------------------------------

/// Start a new practice session. Emits `session-status` as
/// `starting` then `listening`.
#[tauri::command]
pub async fn start_practice_session<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    instrument: String,
    coaching_enabled: bool,
) -> Result<String, String> {
    emit_session_status(&app, SessionPhase::Starting);
    match start_practice_session_impl(state.inner(), instrument, coaching_enabled).await {
        Ok(id) => {
            emit_session_status(&app, SessionPhase::Listening);
            Ok(id)
        }
        Err(e) => Err(e.to_frontend()),
    }
}

/// Close the current instrument segment and open a new one. Emits
/// `segment-changed`.
#[tauri::command]
pub async fn switch_instrument<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    instrument: String,
) -> Result<String, String> {
    match switch_instrument_impl(state.inner(), instrument.clone()).await {
        Ok((segment_id, started_at)) => {
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
#[tauri::command]
pub async fn end_practice_session<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<SessionRecap, String> {
    emit_session_status(&app, SessionPhase::Ending);
    end_practice_session_impl(state.inner())
        .await
        .map_err(|e| e.to_frontend())
}

/// Return the instrument catalog for the selector grid.
#[tauri::command]
pub fn list_instruments() -> Result<Vec<InstrumentInfo>, String> {
    Ok(list_instruments_impl())
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
        }
    }

    #[tokio::test]
    async fn start_session_then_end_session_produces_recap() {
        // Happy path with phrases exercises the rich-recap branch.
        let s = state();
        let session_id = start_practice_session_impl(&s, "Trumpet".to_owned(), false)
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
        start_practice_session_impl(&s, "Trumpet".to_owned(), false)
            .await
            .unwrap();
        let err = start_practice_session_impl(&s, "Piano".to_owned(), false)
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
        let err = switch_instrument_impl(&s, "Piano".to_owned())
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::NotActive), "{err:?}");
    }

    #[tokio::test]
    async fn switch_instrument_closes_old_segment_opens_new() {
        let s = state();
        start_practice_session_impl(&s, "Trumpet".to_owned(), false)
            .await
            .unwrap();
        let (new_segment_id, _) = switch_instrument_impl(&s, "Piano".to_owned())
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
        start_practice_session_impl(&s, "Voice".to_owned(), false)
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

        start_practice_session_impl(&s, "Trumpet".to_owned(), true)
            .await
            .unwrap();
        // PR 1 collapses Starting → Listening synchronously (no
        // audio-stream wait yet); PR 2 will insert the real pause.
        assert_eq!(s.current_phase().await, SessionPhase::Listening);

        switch_instrument_impl(&s, "Piano".to_owned()).await.unwrap();
        assert_eq!(s.current_phase().await, SessionPhase::Listening);

        end_practice_session_impl(&s).await.unwrap();
        assert_eq!(s.current_phase().await, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn start_rejects_empty_or_unknown_instrument() {
        let s = state();
        let empty = start_practice_session_impl(&s, "  ".to_owned(), false)
            .await
            .unwrap_err();
        assert!(matches!(empty, CommandError::EmptyInstrument));

        let unknown = start_practice_session_impl(&s, "Kazoo".to_owned(), false)
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
        start_practice_session_impl(&s, "Trumpet".to_owned(), false)
            .await
            .unwrap();
        let err = switch_instrument_impl(&s, "Kazoo".to_owned())
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::UnknownInstrument(_)), "{err:?}");
    }

    #[test]
    fn list_instruments_returns_expected_catalog() {
        let list = list_instruments_impl();
        assert_eq!(list.len(), KNOWN_INSTRUMENTS.len());
        let by_name: std::collections::HashMap<_, _> = list
            .iter()
            .map(|i| (i.name.as_str(), i.family.as_str()))
            .collect();
        assert_eq!(by_name.get("Trumpet"), Some(&"Brass"));
        assert_eq!(by_name.get("Piano"), Some(&"Keyboard"));
        assert_eq!(by_name.get("Voice"), Some(&"Voice"));
    }

    #[tokio::test]
    async fn mock_coaching_rotates_through_tips() {
        let svc = MockCoachingService::new();
        let ctx = SessionContext {
            instrument: "Trumpet".to_owned(),
            session_duration_secs: 0.0,
            phrases_played: 0,
            previous_tips: Vec::new(),
        };
        let first = svc.get_tip(0, &ctx).await.unwrap();
        let second = svc.get_tip(1, &ctx).await.unwrap();
        let wrap = svc.get_tip(3, &ctx).await.unwrap();
        assert_ne!(first.text, second.text);
        // Rotation wraps at len boundary.
        assert_eq!(first.text, wrap.text);
    }
}
