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
    advance, apply_explore_delta, build_first, finish_lesson, played_notes_from_pitch_track,
    score_drill, sequence_to_score_model, start_explore, ChipSpec, Drill, DrillScore, ExploreState,
    LessonRecap, LessonSpec, VariationDelta,
};
use brain::coaching::{
    grounded_offline_recap, CoachingCategory, CoachingConfig, CoachingEngine, CoachingSeverity,
    CoachingTip, NetworkPolicy, ReqwestClient, SessionContext,
};
use brain::connections::{
    apply_enriched_why, reveal_on_phrase, MusicalContext, Reveal, DEFAULT_REVEAL_CADENCE,
};
use brain::follower::ScorePosition;
use brain::perception::PerceptionTracker;
use brain::phrase::PhraseSummary;
use brain::session::{
    CompletedSession, PracticeMode, RecapGenerator, RecapInput, ScoreId, SessionError,
    SessionRecap, SessionRecorder,
};
use brain::stats::PracticeStats;
use brain::store::{
    ScoreLibraryEntry, ScoreStore, SessionStore, SessionSummary, StoredSession, TasteProfile,
    LOCAL_TASTE_PROFILE_USER_ID,
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

        // Open the on-disk stores, degrading to in-memory (with a clear warning)
        // rather than crashing if the data directory is unusable (#137).
        let (session_store, score_store, persisted) = open_stores(&db_path);

        let coaching_svc = LlmCoachingService::new();
        let coaching_available = coaching_svc.coaching_available();
        let recap_gen = LlmRecapGenerator::new();

        Self {
            active_session: Mutex::new(None),
            coaching_service: Arc::new(coaching_svc),
            recap_generator: Arc::new(recap_gen),
            session_store: std::sync::Mutex::new(session_store),
            score_store: std::sync::Mutex::new(score_store),
            coaching_available,
            persistence_degraded: !persisted,
            omr_enabled: pdf_omr_enabled_from_env(),
            instruments: Arc::new(load_instrument_catalog(app_handle)),
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
            accompaniment_cmd_lock: Mutex::new(()),
            key_override: std::sync::Mutex::new(None),
            active_lesson: std::sync::Mutex::new(None),
            active_explore: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Wire entirely with mocks and in-memory store. Used by tests.
    pub fn with_mocks() -> Self {
        Self {
            active_session: Mutex::new(None),
            coaching_service: Arc::new(MockCoachingService::new()),
            recap_generator: Arc::new(MockRecapGenerator),
            session_store: std::sync::Mutex::new(
                SessionStore::in_memory().expect("in-memory store must succeed"),
            ),
            score_store: std::sync::Mutex::new(
                ScoreStore::in_memory().expect("in-memory store must succeed"),
            ),
            coaching_available: false,
            // In-memory by design here — that's the test default, not a
            // degradation.
            persistence_degraded: false,
            omr_enabled: false,
            instruments: Arc::new(test_instrument_catalog()),
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
            accompaniment_cmd_lock: Mutex::new(()),
            key_override: std::sync::Mutex::new(None),
            active_lesson: std::sync::Mutex::new(None),
            active_explore: Arc::new(std::sync::Mutex::new(None)),
        }
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
    pub(crate) fn teardown_accompaniment(&self) {
        let taken = self.accompaniment.lock_or_recover().take();
        if let Some(accompaniment) = taken {
            accompaniment.output.stop();
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
        // Voice uses a lower gate so quiet singing registers (#185); others
        // keep the 0.5 default.
        voiced_confidence_threshold: if name == "Voice" { 0.3 } else { 0.5 },
        polyphonic: family == "Keyboard",
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

/// Emit the follower's live score position (~10 Hz) so the cursor glides
/// between phrase boundaries. Only fires in score mode. Non-fatal on
/// error: a dropped tick just means one skipped frame of cursor motion.
fn emit_score_position_updated<R: Runtime>(app: &tauri::AppHandle<R>, position: ScorePosition) {
    // One breadcrumb per process on the first cursor tick (#277 diagnostics):
    // separates "follower never aligned" from "events not reaching the UI".
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| tracing::info!(?position, "first score-position emitted"));
    let _ = app.emit("score-position-updated", position);
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
            let recap = build_recap(
                &completed,
                &*generator,
                taste_profile,
                idiom_notes,
                note_verdicts,
            )
            .await?;
            // Persist the completed session so practice history, the stats
            // surface, and (opt-in) cloud sync all have something to read.
            // The store can degrade to in-memory at startup (see `open_stores`),
            // and a recap the user is waiting on must never be sunk by a
            // persistence failure — so we log and carry on rather than erroring.
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
                }
            }
            Ok(recap)
        }
        Err(SessionError::Empty) => Ok(empty_state_recap(elapsed_secs, instrument)),
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
    taste_profile: Option<TasteProfile>,
    idiom_notes: Vec<brain::idiom_recap::IdiomMatch>,
    note_verdicts: Vec<brain::follower::NoteVerdict>,
) -> Result<SessionRecap, CommandError> {
    completed
        .generate_recap_with_context(generator, taste_profile, idiom_notes, note_verdicts)
        .await
        .map_err(CommandError::from)
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
                    move |phrase| {
                        // Buffer a copy for the recap (drained into the recorder
                        // at session end), then emit to the UI for live display.
                        phrase_buffer.lock_or_recover().push(phrase.clone());
                        // Retune the band when this phrase carries a confident key.
                        if let Some(key) = phrase.key.as_ref() {
                            if let Some(accompaniment) =
                                accomp_for_phrase.lock_or_recover().as_mut()
                            {
                                accompaniment.driver.observe_key(key);
                            }
                        }
                        emit_phrase_detected(&app_for_phrase, phrase);
                    },
                    move |position| {
                        if explore_gate_position.lock_or_recover().is_some() {
                            return; // exploration on stage — the cursor rests
                        }
                        emit_score_position_updated(&app_for_position, position);
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
        state.teardown_accompaniment();
    }
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
/// audio worker feeds. The band stays silent until the live clock locks onto
/// the player's pulse, so it's safe to call before or during play. Fully
/// offline — no network.
#[tauri::command]
pub async fn start_accompaniment<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Serialize start/stop/teardown so two overlapping commands can't race the
    // device handoff (Tauri runs each command as its own task).
    let _cmd = state.accompaniment_cmd_lock.lock().await;

    // Tear the previous band down FIRST — fully (device released + threads
    // joined) and off the accompaniment lock — so we never (a) open a second
    // output device while the old one is still live, nor (b) drop-join an
    // `AudioOutput` while holding the std mutex the audio worker locks per frame.
    state.teardown_accompaniment();

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
    // Carry over a key the user pinned earlier this session so the band starts
    // in it rather than re-running auto-detection.
    if let Some((tonic, minor)) = state.current_key_override() {
        accompaniment.driver.set_key_override(tonic, minor);
    }
    // The slot is guaranteed empty (we just tore down under the cmd lock), so
    // this assignment never drops a live `AudioOutput` while holding the lock.
    *state.accompaniment.lock_or_recover() = Some(accompaniment);

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
    state.teardown_accompaniment();
    emit_accompaniment_status(&app, false);
    Ok(())
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
    state
        .get_coaching_tip(&phrase, &session_ctx)
        .await
        .map_err(|e| e.to_string())
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
}

/// Assemble the ExploreDto every explore command returns.
fn explore_dto(
    explore: &ExploreState,
    seq: &brain::coach::GeneratedSequence,
    model: &brain::learner::LearnerModel,
) -> ExploreDto {
    let chips = brain::coach::suggest_chips(explore, model);
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
        staff: brain::score::cellstaff::cell_staff_view(seq, key),
        can_undo: !explore.history.is_empty(),
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

/// The active explore key signature (for the edit engine's staff-step math).
fn explore_key(explore: &ExploreState) -> brain::score::KeySignature {
    // The signature follows the FIGURE the row actually deals (#335): a
    // scale explore engraves in its scale's family; a chord explore (the
    // jam bridge, #349 T4a) in its chord family — an Am7 row must not
    // engrave in A MAJOR and drown every stack in accidentals (review S1).
    let material = explore
        .spec
        .scale
        .map(|m| m.scale.label().to_lowercase())
        .or_else(|| explore.spec.chord.map(|c| c.chord.label().to_lowercase()))
        .or_else(|| {
            // #349 T3c: a lifted progression engraves in its ANCHOR chord's
            // family — a Dm7-anchored row must not read in D MAJOR (the
            // T4a M4 split-brain class, pre-empted this time).
            explore
                .spec
                .progression
                .as_ref()
                .and_then(|p| p.first())
                .map(|st| st.chord.label().to_lowercase())
        })
        .unwrap_or_else(|| "major".to_owned());
    brain::coach::key_signature_for(explore.tonic, &material)
}

/// Start (or restart) a free-play exploration from the live key. Reads the
/// Learner Model for the difficulty; the variation renders on the free-play
/// surface with its mutation chips.
pub fn start_explore_variation_impl(
    state: &AppState,
    tonic: u8,
    mode: &str,
    seed: u64,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) = start_explore(tonic, mode, &model, seed);
    let dto = explore_dto(&explore, &seq, &model);
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
pub fn start_explore_variation(
    state: State<'_, AppState>,
    tonic: u8,
    mode: String,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    start_explore_variation_impl(&state, tonic, &mode, seed)
}

/// Apply a tapped chip's delta to the in-flight exploration and return the
/// next rep. Calm error when nothing is being explored.
pub fn apply_variation_delta_impl(
    state: &AppState,
    delta: VariationDelta,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    let (next, seq) = apply_explore_delta(current, &delta);
    let dto = explore_dto(&next, &seq, &model);
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
pub fn apply_variation_delta(
    state: State<'_, AppState>,
    delta: VariationDelta,
) -> Result<ExploreDto, String> {
    apply_variation_delta_impl(&state, delta)
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
pub fn explore_last_phrase_impl(state: &AppState, seed: u64) -> Result<ExploreDto, String> {
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
    let (explore, seq) = brain::coach::start_explore_cell(cell, first_midi % 12, &model, seed);
    let dto = explore_dto(&explore, &seq, &model);
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
            while off > 36 {
                off -= 12;
            }
            while off < -36 {
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
    let (explore, seq) = brain::coach::start_explore_cell(cell, first % 12, &learner, seed);
    let dto = explore_dto(&explore, &seq, &learner);
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

/// #337 S5: row one measure of a stored score through 12 keys.
#[tauri::command]
pub fn explore_measure(
    state: State<'_, AppState>,
    score_id: String,
    measure_number: usize,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42);
    explore_measure_impl(&state, &score_id, measure_number, seed)
}

/// #349 T4a — the jam lane's RV bridge: row a chord the room played
/// through 12 keys as stacked block cells. Same explore engine (and same
/// live view swap) as "work on my last lick".
pub fn explore_chord_impl(
    state: &AppState,
    root_pc: u8,
    quality: brain::theory::ChordQuality,
    seed: u64,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let (explore, seq) = brain::coach::start_explore_chord(root_pc % 12, quality, &model, seed);
    let dto = explore_dto(&explore, &seq, &model);
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
pub fn explore_chord(
    state: State<'_, AppState>,
    root_pc: u8,
    quality: brain::theory::ChordQuality,
) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(7);
    explore_chord_impl(&state, root_pc, quality, seed)
}

/// #349 T3c — "work on my last progression": lift the chart's trailing
/// chord sequence (consecutive duplicates collapsed, unresolved stretches
/// skipped) and row it through 12 keys as stacked cells. Same live view
/// swap as the lick lift; refuses calmly under two distinct chords.
pub fn explore_progression_impl(state: &AppState, seed: u64) -> Result<ExploreDto, String> {
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
    let (explore, seq) = brain::coach::start_explore_progression(&chords, &model, seed);
    let dto = explore_dto(&explore, &seq, &model);
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
pub fn explore_progression(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(11);
    explore_progression_impl(&state, seed)
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
pub fn explore_last_phrase(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    explore_last_phrase_impl(&state, seed)
}

/// Apply a semantic note edit (#292 slice 3) to the in-flight exploration —
/// the edit bakes the CELL, so it lands in every key; the row never
/// reshuffles under the player's hands.
pub fn edit_explore_note_impl(
    state: &AppState,
    index: usize,
    edit: brain::coach::NoteEdit,
) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    let key = explore_key(current);
    let (next, seq) = brain::coach::edit_explore_note(current, index, &edit, &key)?;
    let dto = explore_dto(&next, &seq, &model);
    *guard = Some(next);
    Ok(dto)
}

#[tauri::command]
pub fn edit_explore_note(
    state: State<'_, AppState>,
    index: usize,
    edit: brain::coach::NoteEdit,
) -> Result<ExploreDto, String> {
    edit_explore_note_impl(&state, index, edit)
}

/// Undo the most recent explore edit — restores the exact prior rep.
pub fn undo_explore_edit_impl(state: &AppState) -> Result<ExploreDto, String> {
    let model = state
        .session_store
        .lock_or_recover()
        .get_learner_model(LOCAL_TASTE_PROFILE_USER_ID)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut guard = state.active_explore.lock_or_recover();
    let current = guard
        .as_ref()
        .ok_or_else(|| "nothing is being explored — start a variation first".to_owned())?;
    let (next, seq) = brain::coach::undo_explore_edit(current)?;
    let dto = explore_dto(&next, &seq, &model);
    *guard = Some(next);
    Ok(dto)
}

#[tauri::command]
pub fn undo_explore_edit(state: State<'_, AppState>) -> Result<ExploreDto, String> {
    undo_explore_edit_impl(&state)
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

fn drill_dto(drill: &Drill, drill_count: u8) -> DrillDto {
    let key = brain::coach::key_signature_for(drill.tonic, &drill.mode);
    let fifths = key.fifths;
    let model = sequence_to_score_model(&drill.sequence, &drill.sequence.label, key);
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
    };
    let first = build_first(&spec, &model);
    let dto = drill_dto(&first, spec.drill_count);
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

    match advance(&lesson.current, &score, &lesson.spec) {
        Some(next) => {
            let dto = drill_dto(&next, lesson.spec.drill_count);
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
    let polyphonic = match state.active_session_instrument().await {
        Some(name) => state
            .instruments
            .iter()
            .find(|i| i.name == name)
            .is_some_and(|i| i.polyphonic),
        None => false,
    };
    start_lesson_impl(&state, seed, polyphonic).map_err(|e| e.to_frontend())
}

/// Grade the just-played drill and step the lesson.
#[tauri::command]
pub fn submit_drill(state: State<'_, AppState>) -> Result<LessonStepDto, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    submit_drill_impl(&state, now).map_err(|e| e.to_frontend())
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
    Ok(list_instruments_impl(state.inner()))
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
    let score_store = state.score_store.lock_or_recover();
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

/// Above this fraction of time-overlapping notes we call the input polyphonic.
const POLYPHONIC_THRESHOLD: f32 = 0.15;
/// Below this mean note activation we flag the transcription as low-confidence.
const LOW_CONFIDENCE_THRESHOLD: f32 = 0.4;

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
}

impl ImportedAudioDto {
    fn new(entry: ScoreLibraryEntryDto, quality: transcribe::TranscriptionQuality) -> Self {
        Self {
            entry,
            note_count: quality.note_count,
            mean_confidence: quality.mean_confidence,
            polyphony: quality.polyphony,
            polyphonic: quality.polyphony > POLYPHONIC_THRESHOLD,
            low_confidence: quality.mean_confidence < LOW_CONFIDENCE_THRESHOLD,
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
    let score_store = state.score_store.lock_or_recover();
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
    let score_store = state.score_store.lock_or_recover();
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
        let recap = build_recap(&completed, &generator, None, Vec::new(), verdicts)
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
        let dto = explore_measure_impl(&s, &entry.id.to_string(), 1, 7).expect("measure 1 rows");
        assert!(
            !dto.staff.notes.is_empty(),
            "the exploration renders on the staff"
        );
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
        let err = explore_measure_impl(&s, &entry.id.to_string(), 99, 7).unwrap_err();
        assert!(err.contains("isn't in this piece"), "got: {err}");
        let err = explore_measure_impl(&s, "not-a-real-id", 1, 7).unwrap_err();
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

    /// #349 T2b end-to-end at the command layer: a POLYPHONIC lesson deals
    /// the chord drill as block chords and grades it from the chord buffer
    /// via the T1-engine judge — while melodic drills in the same lesson
    /// keep the phrase-buffer path. Fails if the grading fork, the marks,
    /// or the polyphonic dealing regresses.
    #[test]
    fn a_polyphonic_lesson_deals_and_grades_chord_drills() {
        let s = state();
        let mut last = start_lesson_impl(&s, 42, true).expect("lesson starts");
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
                        submit_drill_impl(&s, 1_000),
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
            last = submit_drill_impl(&s, 1_000).expect("submit succeeds");
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
        let step0 = start_lesson_impl(&s, 42, false).expect("lesson starts");
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

            last = submit_drill_impl(&s, 1_000).expect("submit succeeds");
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
        let again = start_lesson_impl(&s, 43, false).unwrap();
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
        assert!(submit_drill_impl(&s, 0).is_err());
        start_lesson_impl(&s, 1, false).unwrap();
        end_lesson_impl(&s, 10);
        assert!(
            submit_drill_impl(&s, 0).is_err(),
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
        start_lesson_impl(&s, 2, false).unwrap();
        play_current_drill_perfectly(&s);
        let step = submit_drill_impl(&s, 100).unwrap();
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
        start_lesson_impl(&s, 3, false).unwrap();
        play_current_drill_perfectly(&s);
        let step1 = submit_drill_impl(&s, 100).unwrap();
        assert!(step1.score.unwrap().accuracy > 0.99);

        // Drill 1: play only clearly WRONG material (a cluster far from any
        // target class run). If drill 0's perfect notes leaked into this
        // window (a dropped phrase_mark), the grade would be inflated.
        let mut wrong = sample_phrase();
        wrong.pitch_stats.pitches = std::iter::repeat_n(8_000.0, 40).collect();
        wrong.onsets_secs = vec![0.0; 4];
        s.phrase_buffer.lock().unwrap().push(wrong);
        let step2 = submit_drill_impl(&s, 200).unwrap();
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
        start_lesson_impl(&s, 12, false).unwrap();
        // Unpitched noise WITH onsets: grades (0%), never DrillNotHeard.
        let mut claps = sample_phrase();
        claps.pitch_stats.pitches = Vec::new();
        claps.onsets_secs = vec![0.1, 0.4, 0.8];
        s.phrase_buffer.lock().unwrap().push(claps);
        let step = submit_drill_impl(&s, 100).unwrap();
        assert_eq!(step.score.unwrap().accuracy, 0.0, "heard noise grades 0");

        // Pitched legato with ZERO detected onsets: also grades.
        let mut legato = sample_phrase();
        legato.pitch_stats.pitches = std::iter::repeat_n(440.0, 40).collect();
        legato.onsets_secs = Vec::new();
        s.phrase_buffer.lock().unwrap().push(legato);
        assert!(
            submit_drill_impl(&s, 200).is_ok(),
            "a legato singer must not be trapped in not-yet"
        );
    }

    /// #277 hardening: a tap before ANY phrase has closed for the drill is a
    /// calm "not yet" error, never a lying 0% grade — the drill stays live for
    /// a retry.
    #[test]
    fn eager_submit_before_any_phrase_is_a_calm_not_yet() {
        let s = state();
        start_lesson_impl(&s, 12, false).unwrap();
        assert!(matches!(
            submit_drill_impl(&s, 100),
            Err(CommandError::DrillNotHeard)
        ));
        // Still live: playing and resubmitting works.
        play_current_drill_perfectly(&s);
        assert!(submit_drill_impl(&s, 200).unwrap().score.unwrap().accuracy > 0.99);
    }

    /// Self-improvement (#252): every dealt exercise leaves EVIDENCE — a
    /// graded drill logs with its accuracy, explore/lift deals log ungraded
    /// (the "they bailed" signal), and the insights analyzer reads it all
    /// back per shape. Fails if any recording hook is dropped.
    #[test]
    fn exercises_leave_evidence_in_the_log() {
        let s = state();
        // A graded lesson drill…
        start_lesson_impl(&s, 12, false).unwrap();
        play_current_drill_perfectly(&s);
        submit_drill_impl(&s, 100).unwrap();
        // …an explore deal + a chip…
        start_explore_variation_impl(&s, 7, "dorian", 42).unwrap();
        apply_variation_delta_impl(&s, VariationDelta::ToggleDirection).unwrap();

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
            explore_last_phrase_impl(&s, 42).is_err(),
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

        let dto = explore_last_phrase_impl(&s, 42).unwrap();
        assert!(dto.label.contains("5-note cell"), "got {}", dto.label);
        assert!(!dto.root_pitch_classes.is_empty());
        assert!(!dto.staff.notes.is_empty());
        // And it's immediately editable (#292): the correction UX this loop
        // was built for.
        let edited =
            edit_explore_note_impl(&s, 0, brain::coach::NoteEdit::Octaves { by: 1 }).unwrap();
        assert!(edited.can_undo);
    }

    /// #349 T3 AC3 at the command layer: the chart's trailing chords lift
    /// into a rowed progression (consecutive dupes collapsed, unresolved
    /// skipped, capped at 4), logged as progression_lift; fewer than two
    /// distinct chords refuses calmly.
    #[test]
    fn the_charts_trailing_chords_lift_as_a_progression() {
        let s = state();
        assert!(
            explore_progression_impl(&s, 42)
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
        let dto = explore_progression_impl(&s, 42).unwrap();
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
        let dto = explore_progression_impl(&s, 42).unwrap();
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
        explore_progression_impl(&s, 42).unwrap();
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
        let dto = explore_progression_impl(&s, 42).unwrap();
        assert!(dto.label.contains("Ebm7"), "lift label: {}", dto.label);
        let bumped =
            apply_variation_delta_impl(&s, VariationDelta::BumpDifficulty { by: 1 }).unwrap();
        assert!(
            bumped.label.contains("Ebm7") && !bumped.label.contains('#'),
            "the label stays flat through a chip tap: {}",
            bumped.label
        );
        let shuffled = apply_variation_delta_impl(&s, VariationDelta::ReshuffleRoots).unwrap();
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
            explore_progression_impl(&s, 42)
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
        let dto = explore_chord_impl(&s, 10, brain::theory::ChordQuality::Dom13, 42).unwrap();
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
        let dto = explore_chord_impl(&s, 0, brain::theory::ChordQuality::Min7, 42).unwrap();
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
            apply_variation_delta_impl(&s, VariationDelta::ReshuffleRoots).is_err(),
            "no exploration yet → calm error"
        );

        let dto = start_explore_variation_impl(&s, 7, "Dorian", 42).unwrap();
        assert!(dto.label.contains("Dorian"), "got {}", dto.label);
        assert!(dto.music_xml.contains("<score-partwise"));
        assert!(!dto.chips.is_empty() && dto.chips.len() <= 3);
        assert!(!dto.root_pitch_classes.is_empty());

        let next = apply_variation_delta_impl(&s, VariationDelta::ToggleDirection).unwrap();
        assert_ne!(next.music_xml, dto.music_xml, "a delta produces a new rep");
        assert!(next.chips.len() <= 3);

        // #292 slice 3: chips and edits are both undo-able steps; an edit
        // bakes the cell and undo restores the exact prior rep.
        assert!(next.can_undo, "the chip itself is an undo-able step");
        let edited =
            edit_explore_note_impl(&s, 0, brain::coach::NoteEdit::Octaves { by: 1 }).unwrap();
        assert!(edited.can_undo);
        assert_ne!(edited.staff, next.staff, "the edit changes the staff");
        let undone = undo_explore_edit_impl(&s).unwrap();
        assert_eq!(undone.staff, next.staff, "undo restores the prior rep");
        // One more undo steps back over the CHIP to the very first rep…
        let back_to_start = undo_explore_edit_impl(&s).unwrap();
        assert_eq!(back_to_start.staff, dto.staff, "chips are undo-able too");
        // …and only then is history exhausted.
        assert!(undo_explore_edit_impl(&s).is_err(), "history exhausted");
    }

    /// #335 at the explore surface: a C#-major exploration engraves 5 flats,
    /// so its label must open with the same "Db" the first root cell shows —
    /// the header and the cells may never spell the root differently.
    #[test]
    fn explore_label_speaks_the_signature_spelling() {
        let s = state();
        let dto = start_explore_variation_impl(&s, 1, "major", 42).unwrap();
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
        let dto = drill_dto(&drill, 4);
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
        start_lesson_impl(&s, 4, false).unwrap();
        assert!(matches!(
            start_lesson_impl(&s, 5, false),
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
        start_lesson_impl(&s, 6, false).unwrap();
        play_current_drill_perfectly(&s);
        submit_drill_impl(&s, 100).unwrap();

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
        assert!(
            quality.polyphony <= POLYPHONIC_THRESHOLD,
            "monophonic input flagged polyphonic: {}",
            quality.polyphony
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
            duration_secs: 300.0,
            practice_mode: PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
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
}
