//! LLM coaching engine — turns phrase analysis into teacher-quality feedback.
//!
//! This module runs on the processing thread (NOT the audio thread),
//! so heap allocation is allowed.
//!
//! The engine calls an LLM API (Claude or GPT-4) with a carefully crafted
//! system prompt that enforces a "coach, don't judge" philosophy: no letter
//! grades, no percentages, warm and encouraging tone.

use std::env;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fingerprint::{KeyClaimStrength, MusicalFingerprint};
use crate::phrase::PhraseSummary;
use crate::session::{RecapGenerator, RecapInput, SessionError, SessionRecap};
use crate::store::TasteProfile;

mod prompts;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur in the coaching engine.
#[derive(Debug, Error)]
pub enum CoachingError {
    #[error("no API key provided and MUSIC_COMPANION_LLM_API_KEY is not set")]
    MissingApiKey,
    #[error("HTTP request failed: {0}")]
    HttpError(String),
    #[error("failed to parse LLM response: {0}")]
    ParseError(String),
}

// ---------------------------------------------------------------------------
// Network policy — the Rust-core "airplane switch"
// ---------------------------------------------------------------------------

/// Whether the coaching engine is permitted to make an outbound network call.
///
/// This is the **hard, Rust-core enforcement** of the offline-first principle
/// (see `docs/architecture/offline-first-and-network-transparency.md`). It is
/// not a UI toggle and not a hint: when [`NetworkPolicy::Offline`], the
/// recap/tip generation path **never constructs an outbound request and never
/// invokes the [`HttpClient`]**. The on-device fallback is used instead.
///
/// The frontend `coachingEnabled` preference is mirrored into this policy at
/// the command layer (see `apps/desktop/src-tauri/src/commands.rs`). Because
/// the guarantee lives here — below the IPC boundary — a bug, a malformed IPC
/// payload, or a future caller that forgets the FE toggle still cannot cause a
/// silent outbound call: an `Offline` engine is structurally incapable of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// No outbound calls. The `HttpClient` is never invoked; tips and recaps
    /// are served entirely by the on-device fallback. This is the default —
    /// offline by default, the internet is never required.
    #[default]
    Offline,
    /// Outbound LLM narration is permitted. The user has opted in.
    Online,
}

impl NetworkPolicy {
    /// True when the engine may reach the network. The single predicate every
    /// outbound path must consult before touching the [`HttpClient`].
    #[inline]
    pub fn allows_network(self) -> bool {
        matches!(self, NetworkPolicy::Online)
    }

    /// Build a policy from a plain opt-in flag (the FE `coachingEnabled` pref).
    /// `true` → [`NetworkPolicy::Online`], `false` → [`NetworkPolicy::Offline`].
    pub fn from_opt_in(enabled: bool) -> Self {
        if enabled {
            NetworkPolicy::Online
        } else {
            NetworkPolicy::Offline
        }
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Severity of the coaching feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoachingSeverity {
    /// Positive reinforcement for things going well.
    Encouragement,
    /// A gentle nudge toward improvement.
    Suggestion,
    /// A specific area that needs attention.
    Focus,
}

/// Category of musical feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoachingCategory {
    Tone,
    Intonation,
    Rhythm,
    Dynamics,
    Expression,
    Technique,
}

// ---------------------------------------------------------------------------
// Coaching tip
// ---------------------------------------------------------------------------

/// A coaching tip returned by the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachingTip {
    /// Human-readable coaching text.
    pub text: String,
    /// How urgent / positive the feedback is.
    pub severity: CoachingSeverity,
    /// Which musical dimension this tip addresses.
    pub category: CoachingCategory,
}

// ---------------------------------------------------------------------------
// Session context
// ---------------------------------------------------------------------------

/// Contextual information about the current practice session.
///
/// Passed alongside a [`PhraseSummary`] to give the LLM richer context
/// for generating relevant coaching tips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    /// The instrument being played (e.g. "trumpet", "violin").
    pub instrument: String,
    /// How long the session has been running, in seconds.
    pub session_duration_secs: f64,
    /// Number of phrases played so far.
    pub phrases_played: usize,
    /// Previous coaching tips already shown (to avoid repetition).
    pub previous_tips: Vec<String>,
    /// Title of the score being practised, when in Score Mode. Lets the
    /// live coach name the piece ("on this passage of the Haydn…") rather
    /// than speaking generically. `None` in free play.
    pub score_title: Option<String>,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the coaching engine.
#[derive(Debug, Clone)]
pub struct CoachingConfig {
    /// API key for the LLM provider. If empty, the engine will attempt to
    /// read `MUSIC_COMPANION_LLM_API_KEY` from the environment.
    pub api_key: String,
    /// Model identifier (e.g. "claude-opus-4-8", "gpt-4").
    /// Falls back to `MUSIC_COMPANION_LLM_MODEL` env var, then to
    /// `"claude-opus-4-8"`.
    pub model: String,
    /// Minimum seconds between consecutive API calls. Default: 3.0.
    pub rate_limit_secs: f64,
}

impl Default for CoachingConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: String::new(),
            rate_limit_secs: 3.0,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client trait
// ---------------------------------------------------------------------------

/// Abstraction over HTTP so the engine can be tested with a mock client.
#[async_trait::async_trait]
pub trait HttpClient: Send + Sync {
    /// Send a POST request with a JSON body and return the response body.
    async fn post_json(
        &self,
        url: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Result<String, CoachingError>;
}

/// Production HTTP client backed by `reqwest`.
pub struct ReqwestClient {
    inner: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self {
            inner: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl HttpClient for ReqwestClient {
    async fn post_json(
        &self,
        url: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Result<String, CoachingError> {
        let mut builder = self
            .inner
            .post(url)
            .header("Content-Type", "application/json");

        for &(key, value) in headers {
            builder = builder.header(key, value);
        }

        let response = builder
            .body(body.to_owned())
            .send()
            .await
            .map_err(|e| CoachingError::HttpError(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| CoachingError::HttpError(e.to_string()))?;

        if !status.is_success() {
            return Err(CoachingError::HttpError(format!("HTTP {status}: {text}")));
        }

        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// Pure resolution helpers (testable without env mutation)
// ---------------------------------------------------------------------------

/// Resolve the API key from an explicit config value and an optional env value.
///
/// Order: config (if non-empty) → env (if Some and non-empty) → error.
fn resolve_api_key(config_key: &str, env_value: Option<&str>) -> Result<String, CoachingError> {
    if !config_key.is_empty() {
        return Ok(config_key.to_owned());
    }
    match env_value {
        Some(v) if !v.is_empty() => Ok(v.to_owned()),
        _ => Err(CoachingError::MissingApiKey),
    }
}

/// Resolve the model from an explicit config value and an optional env value.
///
/// Order: config (if non-empty) → env (if Some and non-empty) → default.
fn resolve_model(config_model: &str, env_value: Option<&str>) -> String {
    if !config_model.is_empty() {
        return config_model.to_owned();
    }
    match env_value {
        Some(v) if !v.is_empty() => v.to_owned(),
        _ => "claude-opus-4-8".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Coaching engine
// ---------------------------------------------------------------------------

/// The coaching engine orchestrates LLM calls to produce musical coaching tips.
pub struct CoachingEngine {
    config: CoachingConfig,
    http_client: Box<dyn HttpClient>,
    last_call_time: Option<Instant>,
    resolved_api_key: String,
    resolved_model: String,
    /// The airplane switch. When [`NetworkPolicy::Offline`], no outbound
    /// request is ever built and [`Self::http_client`] is never invoked.
    /// Defaults to [`NetworkPolicy::Offline`] — offline by default.
    policy: NetworkPolicy,
    /// #449 T1: whether the LAST `generate_recap` produced its recap from a
    /// parsed LLM response (vs. the offline/thin/failure fallbacks). Atomic
    /// because the `RecapGenerator` impl takes `&self`. Read via
    /// [`Self::recap_used_llm`] to journal `narration_used {"kind":"recap"}`
    /// — and only when a narration genuinely fired.
    recap_llm_fired: std::sync::atomic::AtomicBool,
}

impl CoachingEngine {
    /// Create a new coaching engine.
    ///
    /// The API key is resolved in this order:
    /// 1. `config.api_key` (if non-empty)
    /// 2. `MUSIC_COMPANION_LLM_API_KEY` environment variable
    ///
    /// The model is resolved in this order:
    /// 1. `config.model` (if non-empty)
    /// 2. `MUSIC_COMPANION_LLM_MODEL` environment variable
    /// 3. `"claude-opus-4-8"` default
    pub fn new(
        config: CoachingConfig,
        http_client: Box<dyn HttpClient>,
    ) -> Result<Self, CoachingError> {
        let env_api_key = env::var("MUSIC_COMPANION_LLM_API_KEY").ok();
        let env_model = env::var("MUSIC_COMPANION_LLM_MODEL").ok();
        Self::with_env(
            config,
            http_client,
            env_api_key.as_deref(),
            env_model.as_deref(),
        )
    }

    /// Construct an engine with explicit environment values.
    ///
    /// This is the pure-function core of [`Self::new`] — it accepts the env
    /// values as arguments instead of reading globals, which makes it
    /// deterministic and safe to exercise from tests without mutating
    /// process-wide state.
    pub fn with_env(
        config: CoachingConfig,
        http_client: Box<dyn HttpClient>,
        env_api_key: Option<&str>,
        env_model: Option<&str>,
    ) -> Result<Self, CoachingError> {
        let resolved_api_key = resolve_api_key(&config.api_key, env_api_key)?;
        let resolved_model = resolve_model(&config.model, env_model);

        Ok(Self {
            config,
            http_client,
            last_call_time: None,
            resolved_api_key,
            resolved_model,
            // Offline by default. The internet is never required; callers
            // opt in explicitly via `set_network_policy`.
            policy: NetworkPolicy::Offline,
            recap_llm_fired: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// #449 T1: did the last [`RecapGenerator::generate_recap`] call on this
    /// engine produce its recap from a parsed LLM response? `false` for the
    /// offline-policy, thin-session, API-failure, and parse-failure paths —
    /// all of which serve on-device text.
    pub fn recap_used_llm(&self) -> bool {
        self.recap_llm_fired
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the [`NetworkPolicy`] — the airplane switch. The command layer
    /// calls this from the persisted `coachingEnabled` preference at session
    /// start, so the engine's network behavior tracks the user's choice.
    ///
    /// While [`NetworkPolicy::Offline`], `get_tip` and `generate_recap` return
    /// the on-device fallback without ever touching the [`HttpClient`].
    pub fn set_network_policy(&mut self, policy: NetworkPolicy) {
        self.policy = policy;
    }

    /// The engine's current [`NetworkPolicy`].
    pub fn network_policy(&self) -> NetworkPolicy {
        self.policy
    }

    /// Request a coaching tip for the given phrase and session context.
    ///
    /// Returns `Ok(None)` — **no tip at all** — whenever a genuine LLM tip can't
    /// be produced: the engine is [`NetworkPolicy::Offline`], the call is
    /// rate-limited, the API request fails, or the response can't be parsed.
    /// This is deliberate: for *live* tips, **silence beats a lie**. We never
    /// substitute canned encouragement for a real observation, because a
    /// fabricated "great work!" the model never actually said is dishonest in a
    /// way an empty panel is not. (The session *recap* keeps its grounded
    /// on-device fallback — that is built from measured facts, so it stays
    /// honest; see [`Self::fallback_recap`].)
    ///
    /// On success, returns `Ok(Some(tip))` with the parsed LLM tip.
    pub async fn get_tip(
        &mut self,
        phrase: &PhraseSummary,
        context: &SessionContext,
    ) -> Result<Option<CoachingTip>, CoachingError> {
        // Airplane switch (hard, Rust-core). When offline we return *no tip*
        // before building any prompt, URL, headers, or request body — and
        // crucially before touching `http_client`. There is no code path from
        // `Offline` to an outbound call, and we never invent encouragement to
        // fill the silence.
        if !self.policy.allows_network() {
            return Ok(None);
        }

        // Rate limiting. Too soon since the last call → stay silent rather than
        // emit a canned filler tip.
        if let Some(last) = self.last_call_time {
            let elapsed = last.elapsed().as_secs_f64();
            if elapsed < self.config.rate_limit_secs {
                return Ok(None);
            }
        }

        let system_prompt = Self::build_system_prompt_for_instrument(&context.instrument);
        let user_prompt = Self::build_user_prompt(phrase, context);

        let request_body = self.build_request_body(&system_prompt, &user_prompt);

        let url = self.api_url();
        let headers = self.api_headers();
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        self.last_call_time = Some(Instant::now());

        let response = self
            .http_client
            .post_json(&url, &request_body, &header_refs)
            .await;

        // On API failure or an unparseable response, return no tip rather than
        // fabricating one. A genuine observation or nothing.
        match response {
            Ok(body) => Ok(Self::parse_tip_from_response(&body).ok()),
            Err(_) => Ok(None),
        }
    }

    /// Reword the `why` line for a grounded reveal (#253 S2). Mirrors
    /// [`get_tip`](Self::get_tip)'s airplane switch: returns `None` (→ the caller
    /// keeps the curated `why`) when offline or on any failure, and there is **no
    /// code path from `Offline` to an outbound call**. The model is given the
    /// concept and the FIXED real-world `connection` and asked only for one short
    /// "why" sentence — it never picks or changes the artist/piece.
    pub async fn enrich_reveal_why(
        &mut self,
        concept: &str,
        connection: &str,
        curated_why: &str,
    ) -> Option<String> {
        if !self.policy.allows_network() {
            return None;
        }
        let system_prompt = "\
You reword one short line for a music-practice app that just told a young player \
what real-world music lives in the scale they are playing. You are given the \
CONCEPT (a scale/mode sound) and the CONNECTION (a real, already-verified artist \
or piece). Write ONE warm, engaging sentence (max ~140 characters) about why that \
music lives in that sound, for a curious beginner. Do NOT change, question, or add \
any artist or piece — the connection is fixed and correct. No hype, no emojis. \
Respond with valid JSON in this exact form: { \"why\": \"...\" }";
        let user_prompt =
            format!("CONCEPT: {concept}\nCONNECTION: {connection}\nCurrent line: {curated_why}");
        let request_body = self.build_request_body(system_prompt, &user_prompt);
        let url = self.api_url();
        let headers = self.api_headers();
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let body = self
            .http_client
            .post_json(&url, &request_body, &header_refs)
            .await
            .ok()?;
        let text = Self::extract_content_text(&body)?;
        let why = serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("why")
            .and_then(|w| w.as_str())
            .map(|s| s.trim().to_owned())?;
        // An empty rewrite is a failure — keep the curated line.
        (!why.is_empty()).then_some(why)
    }

    /// Pull the model's text content out of an Anthropic- or OpenAI-shaped
    /// response body. `None` if the shape isn't recognized.
    fn extract_content_text(response_body: &str) -> Option<String> {
        let v = serde_json::from_str::<serde_json::Value>(response_body).ok()?;
        if let Some(text) = v
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
        {
            return Some(text.to_owned());
        }
        v.get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|ch| ch.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .map(str::to_owned)
    }

    // -----------------------------------------------------------------------
    // Prompt construction (wording lives in the `prompts` submodule)
    // -----------------------------------------------------------------------

    /// Build a generic system prompt that shapes the LLM's coaching personality.
    ///
    /// This is the fallback when instrument-specific prompts are not needed.
    /// For real coaching, use `build_system_prompt_for_instrument`.
    pub fn build_system_prompt() -> String {
        prompts::build_system_prompt()
    }

    /// Build an instrument-specific system prompt for coaching.
    ///
    /// Different instruments have different pedagogical priorities:
    /// - Brass: embouchure, breath support, resonance, tonguing, range extension
    /// - Voice: breath management, resonance, vowel placement, vibrato control, projection
    /// - Strings: bow control, intonation stability, vibrato quality, articulation, shifting
    /// - Woodwinds: embouchure flexibility, tone centering, articulation clarity, vibrato control
    /// - Piano: hand position, voicing clarity, pedal timing, evenness across registers
    ///
    /// Each prompt includes instrument-specific vocabulary and emphasis while maintaining
    /// the "coach, don't judge" philosophy.
    pub fn build_system_prompt_for_instrument(instrument: &str) -> String {
        prompts::build_system_prompt_for_instrument(instrument)
    }

    /// Build the user prompt from phrase data and session context.
    ///
    /// Public for testing so we can verify context influences the prompt.
    pub fn build_user_prompt(phrase: &PhraseSummary, context: &SessionContext) -> String {
        prompts::build_user_prompt(phrase, context)
    }

    // -----------------------------------------------------------------------
    // API specifics
    // -----------------------------------------------------------------------

    fn api_url(&self) -> String {
        if self.resolved_model.starts_with("claude") {
            "https://api.anthropic.com/v1/messages".to_owned()
        } else {
            "https://api.openai.com/v1/chat/completions".to_owned()
        }
    }

    fn api_headers(&self) -> Vec<(String, String)> {
        if self.resolved_model.starts_with("claude") {
            vec![
                ("x-api-key".to_owned(), self.resolved_api_key.clone()),
                ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ]
        } else {
            vec![(
                "Authorization".to_owned(),
                format!("Bearer {}", self.resolved_api_key),
            )]
        }
    }

    fn build_request_body(&self, system_prompt: &str, user_prompt: &str) -> String {
        if self.resolved_model.starts_with("claude") {
            serde_json::json!({
                "model": self.resolved_model,
                "max_tokens": 256,
                "system": system_prompt,
                "messages": [
                    { "role": "user", "content": user_prompt }
                ]
            })
            .to_string()
        } else {
            serde_json::json!({
                "model": self.resolved_model,
                "max_tokens": 256,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ]
            })
            .to_string()
        }
    }

    // -----------------------------------------------------------------------
    // Response parsing
    // -----------------------------------------------------------------------

    /// Parse a coaching tip from the raw LLM response JSON.
    fn parse_tip_from_response(response_body: &str) -> Result<CoachingTip, CoachingError> {
        // Try Anthropic response format first
        if let Ok(anthropic) = serde_json::from_str::<serde_json::Value>(response_body) {
            // Anthropic: { "content": [ { "type": "text", "text": "..." } ] }
            if let Some(text) = anthropic
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|block| block.get("text"))
                .and_then(|t| t.as_str())
            {
                return Self::parse_tip_json(text);
            }

            // OpenAI: { "choices": [ { "message": { "content": "..." } } ] }
            if let Some(text) = anthropic
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|t| t.as_str())
            {
                return Self::parse_tip_json(text);
            }
        }

        Err(CoachingError::ParseError(
            "unrecognized response format".to_owned(),
        ))
    }

    /// Parse the inner JSON tip from the LLM's text output.
    fn parse_tip_json(text: &str) -> Result<CoachingTip, CoachingError> {
        // The LLM may include markdown fences; strip them.
        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str::<CoachingTip>(cleaned)
            .map_err(|e| CoachingError::ParseError(e.to_string()))
    }
}

// ===========================================================================
// RecapGenerator implementation
// ===========================================================================

#[async_trait]
impl RecapGenerator for CoachingEngine {
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        // #449 T1: pessimistic until a network response actually parses —
        // every early return below serves on-device text, and the telemetry
        // journal must never claim a narration that didn't fire.
        self.recap_llm_fired
            .store(false, std::sync::atomic::Ordering::Relaxed);
        // Airplane switch (hard, Rust-core). When offline we return the
        // grounded on-device fallback recap *before* building any prompt or
        // request and before touching `http_client`. The fallback is sourced
        // only from locally measured facts (see `fallback_recap`), so the
        // offline recap is still honest — it just reads more generally. All
        // existing grounding behavior (the `aggregate_*` evidence gates) is
        // preserved on this path.
        if !self.policy.allows_network() {
            return Ok(Self::fallback_recap(input));
        }

        // #445-6b: a thin session never reaches the model — the LLM must
        // not inflate twenty seconds of noodling into an essay. Same bar,
        // same short form as the offline path (one choke point per path).
        if is_thin_session(input) {
            return Ok(thin_session_recap(input));
        }

        // Cross-genre connections are only in play when a taste profile exists
        // AND the session produced enough measured signal to ground one — both
        // the prompt's grounding instructions and the response parsing key off
        // this single gate, so they can never drift apart.
        let connections_enabled = connections_gate_open(input);
        let system_prompt =
            Self::build_recap_system_prompt(connections_enabled, &input.instrument_family);
        let user_prompt = Self::build_recap_user_prompt(input);

        let request_body = self.build_request_body(&system_prompt, &user_prompt);

        let url = self.api_url();
        let headers = self.api_headers();
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let response = self
            .http_client
            .post_json(&url, &request_body, &header_refs)
            .await;

        match response {
            Ok(body) => match Self::parse_recap_from_response(&body, input) {
                Ok(recap) => {
                    // The one branch where the narration genuinely fired:
                    // network response received AND parsed into the recap.
                    self.recap_llm_fired
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(recap)
                }
                Err(_) => Ok(Self::fallback_recap(input)),
            },
            Err(_) => Ok(Self::fallback_recap(input)),
        }
    }

    async fn recap_used_llm(&self) -> bool {
        CoachingEngine::recap_used_llm(self)
    }
}

// ===========================================================================
// Recap-specific prompt and parsing
// ===========================================================================

impl CoachingEngine {
    /// Build a system prompt for session recap generation.
    ///
    /// When `connections_enabled` is true (a taste profile is present *and* the
    /// session produced enough measured signal — see [`connections_gate_open`]),
    /// the prompt gains a `connections` output field plus the cross-genre
    /// grounding contract. When false, the prompt is byte-for-byte the existing
    /// recap prompt: no `connections` field, no cross-genre framing — the coach
    /// behaves exactly as before (cold-start / thin-signal fallback).
    fn build_recap_system_prompt(connections_enabled: bool, instrument_family: &str) -> String {
        let base = "\
You are a warm, experienced music teacher writing end-of-session notes for a student.
Your role is to provide honest, encouraging feedback that celebrates progress and
identifies clear next steps for improvement.

IMPORTANT RULES:
- NEVER give letter grades (A, B, C, D, F) or percentage scores.
- NEVER say things like \"you scored 85%\" or \"that was a B+\".
- NEVER use judgmental language like \"poor\", \"bad\", or \"failing\".
- Write as if you're leaving handwritten notes on a practice journal.
- Be specific and reference actual things you heard in their performance.
- Celebrate genuine progress and specific strengths.
- For areas to improve, be constructive and give concrete next steps.
- Use warm, conversational language.";

        // #417-4/#389: on a fixed-pitch instrument the player cannot bend
        // pitch — the model must never critique intonation or reach for the
        // wind-instrument practice bag. Same voice the TIP path already has
        // (prompts.rs piano branch); recaps finally match it.
        let base = if fixed_pitch_family(instrument_family) {
            format!(
                "{base}\n\n\
FIXED-PITCH INSTRUMENT RULES (this student plays one, e.g. piano):\n\
- The player CANNOT alter intonation. Never critique the player's tuning; never \
suggest tuner drones, drone work, or long-tone pitch practice.\n\
- If the notes read sharp or flat overall, that is the INSTRUMENT's tuning — \
mention it at most once, phrased as the instrument (\"your piano reads a touch \
flat\"), never as the player's skill.\n\
- Never use breath, air, or embouchure vocabulary. Speak this instrument's own \
practice language: evenness between the hands, chord voicing and balance, \
articulation consistency, pedal clarity, and steady tempo through position \
shifts."
            )
        } else {
            base.to_owned()
        };

        if !connections_enabled {
            return format!(
                "{base}\n\n\
Respond with valid JSON in this exact format:\n\
{{\n\
  \"overall_assessment\": \"One paragraph capturing the overall arc of the session\",\n\
  \"strengths\": [\"specific strength 1\", \"specific strength 2\"],\n\
  \"areas_to_improve\": [\"area 1\", \"area 2\"],\n\
  \"next_session_suggestions\": [\"focus 1\", \"focus 2\"]\n\
}}\n\n\
All text should be written as a teacher would speak — warm, specific, and actionable."
            );
        }

        // Connections are in play: add the field + the grounding contract.
        // The contract is the trust-critical part — a trained ear catches a
        // fabricated reference instantly, so the model may only relate at the
        // level of style/feel/technique and must hedge or stay silent.
        format!(
            "{base}\n\n\
CROSS-GENRE CONNECTIONS:\n\
This student told us about the music they love (their genres, artists, and goals are \
below). Where — and ONLY where — the measured musical facts of THIS session genuinely \
relate to that world, you may add one or two short \"connections\" that bridge what they \
just played to the music they care about. This is encouragement and context, never a \
score.\n\n\
GROUNDING CONTRACT (read carefully — this protects the student's trust):\n\
- HEDGE every connection. Say things like \"the way you're shaping that line reminds me \
of how horn players phrase in [a genre they like]\" — NOT \"this is exactly the lick from \
[song]\".\n\
- Relate only at the level of STYLE, FEEL, or TECHNIQUE (phrasing, groove, intonation \
colour, tone). Never assert that a specific recording, performance, or artist did a \
specific thing.\n\
- Do NOT invent track names, album names, timestamps, lyrics, quotes, or any specific \
musical fact you were not given. If you are not sure, stay general (genre-level) or omit \
the connection entirely.\n\
- Base connections on the measured session facts above (key/mode, feel, intonation, \
tone) joined to the student's stated taste — never on guesses about what they played.\n\
- Prefer SILENCE over a hollow or forced connection. An empty \"connections\" list is \
completely fine and often the right answer.\n\n\
Respond with valid JSON in this exact format:\n\
{{\n\
  \"overall_assessment\": \"One paragraph capturing the overall arc of the session\",\n\
  \"strengths\": [\"specific strength 1\", \"specific strength 2\"],\n\
  \"areas_to_improve\": [\"area 1\", \"area 2\"],\n\
  \"next_session_suggestions\": [\"focus 1\", \"focus 2\"],\n\
  \"connections\": [\"a hedged, style-level bridge to their world (or omit / leave empty)\"]\n\
}}\n\n\
All text should be written as a teacher would speak — warm, specific, and actionable."
        )
    }

    /// Build a user prompt for session recap generation from session input.
    fn build_recap_user_prompt(input: &RecapInput) -> String {
        let tip_summary = if input.tips.is_empty() {
            "No tips were recorded during this session.".to_owned()
        } else {
            let tip_texts: Vec<String> = input
                .tips
                .iter()
                .take(5)
                .map(|tip| format!("- {}", tip.text))
                .collect();
            format!(
                "Coaching tips from this session (sample of {} total):\n{}",
                input.tips.len(),
                tip_texts.join("\n")
            )
        };

        let phrase_count = input.phrases.len();
        let duration_mins = (input.duration_secs / 60.0).round();

        // Name the piece when the session was score-backed, so the recap
        // can talk about *the music*, not just "your instrument".
        let practicing_what = match &input.score_title {
            Some(title) => format!("{} (playing \"{title}\")", input.instrument),
            None => input.instrument.clone(),
        };
        let score_block = Self::build_score_block(input);

        // The session's musical fingerprint — one unified read of tone, key,
        // intonation, and groove. Every grounded line below is pulled from it,
        // so the prompt and the persisted recap are sourced from the same place.
        let fingerprint = build_fingerprint(&input.phrases);

        // Aggregate tone across the session's phrases, when available.
        let tone_line = match &fingerprint.tone {
            Some(t) => format!("- Tone quality: {}\n", describe_tone(t)),
            None => String::new(),
        };

        // Detected key/mode over the session, when confident — a grounded fact
        // the recap (and later the cultural-relevance layer) can lean on.
        // The claim strength (#316) travels with it: a hedged key must be
        // phrased tentatively, and a session that never settled instructs the
        // model NOT to name one — silence beats a fabricated key.
        let key_line = match (&fingerprint.key, fingerprint.key_claim) {
            (Some(k), Some(KeyClaimStrength::Leaning)) => format!(
                "- Key / mode: leaning {} toward the end — the reading settled late; \
                 if you mention key at all, phrase it tentatively, never as a fact\n",
                k.name()
            ),
            // #404: the key carried the session but the live reading wandered
            // off it by the close — a whole-session claim. Saying it was the
            // key "toward the end" would contradict the strip the player
            // watched.
            (Some(k), Some(KeyClaimStrength::Drifted)) => format!(
                "- Key / mode: mostly {} — the reading wandered off it by the close; \
                 if you mention key at all, say it carried most of the session, \
                 never that it was the key at the end\n",
                k.name()
            ),
            (Some(k), _) => format!(
                "- Key / mode: {} (confidence {:.2})\n",
                k.name(),
                k.confidence
            ),
            // Tonal readings existed but never firmed into a claim — forbid
            // naming a key instead of staying silent (an absent line lets
            // the model guess one).
            (None, Some(KeyClaimStrength::Unsettled)) => {
                "- Key / mode: kept moving — never settled on one key; do not name a key\n"
                    .to_owned()
            }
            // No tonal readings at all (percussive / silent material): say
            // nothing about key.
            (None, _) => String::new(),
        };

        // Intonation over the session, when enough notes were observed. These
        // are *computed* cents figures — the model must not invent numbers, only
        // phrase the facts we hand it.
        let intonation_line = match &fingerprint.intonation {
            // #417-4/#389: hand the model the honest framing, not raw player
            // critique — on fixed pitch the cents belong to the instrument.
            Some(s) if fixed_pitch_family(&input.instrument_family) => {
                if s.mean_cents.abs() >= 10.0 {
                    format!(
                        "- Instrument tuning read (NOT player-controllable): about \
                         {:.0} cents {} of center\n",
                        s.mean_cents.abs(),
                        if s.mean_cents > 0.0 { "sharp" } else { "flat" },
                    )
                } else {
                    String::new()
                }
            }
            Some(s) => format!("- Intonation: {}\n", describe_intonation(s)),
            None => String::new(),
        };

        // Rhythmic feel over the session, when enough onsets were observed.
        let groove_line = match &fingerprint.groove {
            Some(g) => format!("- Feel: {}\n", describe_groove(g)),
            None => String::new(),
        };

        // Offline idiom-proximity block, when anything cleared the confidence
        // gate. Handed to the model as GROUNDED INPUT it may hedge around but
        // must never assert as fact or invent. Empty string when silent.
        let idiom_block = crate::idiom_recap::idiom_prompt_block(&input.idiom_notes);

        // #453 S2: history-grounded suggestions — evidence-cited facts the
        // local analyzer computed over the student's own log. Same GROUNDED
        // INPUT posture as the idiom block: the model narrates, the facts come
        // from local analysis. Empty string when the history earned nothing.
        let history_block = crate::insights::history_prompt_block(&input.history_suggestions);

        // #454 S3: the method-book tip this session's measured evidence
        // earned — same GROUNDED INPUT posture, plus the attribution the
        // model must keep visible (the #454 copyright posture: attributed
        // paraphrase, never an unattributed or invented book claim). Empty
        // string when no evidence bar was crossed.
        let pedagogy_block = crate::pedagogy::tip_prompt_block(input.method_book_tip.as_ref());

        // The student's stated taste, as *context* for framing — never as a
        // performance fact. Joined here at coaching time only (the measured
        // fingerprint above stays the source of truth). Empty string at cold
        // start or when the profile carries nothing to frame with.
        let taste_block = input
            .taste_profile
            .as_ref()
            .map(describe_taste_profile)
            .unwrap_or_default();

        // Whether to nudge for grounded connections — same gate the system
        // prompt and parser use, so the instruction only appears when a
        // grounded connection is actually possible.
        let connection_instruction = if connections_gate_open(input) {
            " Then, if — and only if — what they played genuinely relates to the music \
                they love above, you may add one or two short, hedged connections to their \
                world (style/feel/technique level only). Prefer leaving connections empty \
                over forcing one."
        } else {
            ""
        };

        format!(
            "Please write end-of-session notes for a student who just finished practicing {}. \
            They played {} phrases over approximately {} minutes.\n\n\
            Phrase data summary:\n\
            - Phrase count: {}\n\
            - Average intonation tendency: {:.2}\n\
            - Average dynamic control: {:.2}\n\
            {}{}{}{}\n\
            {}{}{}{}{}{}\n\n\
            Based on this practice session, write encouraging, specific, handwritten-style notes \
            that celebrate what went well and identify clear next steps.{}{}",
            practicing_what,
            phrase_count,
            duration_mins as i32,
            phrase_count,
            Self::average_metric(&input.phrases, |p| p.stability),
            Self::average_metric(&input.phrases, |p| p.dynamics.mean_amplitude),
            tone_line,
            key_line,
            intonation_line,
            groove_line,
            tip_summary,
            score_block,
            idiom_block,
            history_block,
            pedagogy_block,
            taste_block,
            if input.score_title.is_some() {
                " Where it helps, refer to specific measures by number so the \
                student knows exactly which passage you mean."
            } else {
                ""
            },
            connection_instruction,
        )
    }

    /// Build the optional "measure map" block for score-backed sessions:
    /// one line per phrase with the measure it began on, so the LLM can
    /// anchor feedback to real bar numbers ("measure 5 lost direction").
    /// Returns an empty string in free play (no score positions to show).
    fn build_score_block(input: &RecapInput) -> String {
        if input.score_title.is_none() {
            return String::new();
        }
        let lines: Vec<String> = input
            .phrases
            .iter()
            .filter_map(|p| {
                p.score_position.as_ref().map(|pos| {
                    format!(
                        "- Phrase {}: began at measure {}",
                        p.phrase_index + 1,
                        pos.measure_number
                    )
                })
            })
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        format!(
            "\n\nWhere each phrase sat in the score:\n{}",
            lines.join("\n")
        )
    }

    /// Helper to compute average of a metric across phrases.
    fn average_metric<F>(phrases: &[PhraseSummary], f: F) -> f64
    where
        F: Fn(&PhraseSummary) -> f64,
    {
        if phrases.is_empty() {
            0.0
        } else {
            phrases.iter().map(f).sum::<f64>() / phrases.len() as f64
        }
    }

    /// Parse a session recap from the raw LLM response JSON.
    fn parse_recap_from_response(
        response_body: &str,
        input: &RecapInput,
    ) -> Result<SessionRecap, SessionError> {
        // Try Anthropic response format first
        if let Ok(anthropic) = serde_json::from_str::<serde_json::Value>(response_body) {
            // Anthropic: { "content": [ { "type": "text", "text": "..." } ] }
            if let Some(text) = anthropic
                .get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|block| block.get("text"))
                .and_then(|t| t.as_str())
            {
                return Self::parse_recap_json(text, input);
            }

            // OpenAI: { "choices": [ { "message": { "content": "..." } } ] }
            if let Some(text) = anthropic
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|t| t.as_str())
            {
                return Self::parse_recap_json(text, input);
            }
        }

        Err(SessionError::RecapFailed(
            "unrecognized response format".to_owned(),
        ))
    }

    /// Parse the inner JSON recap from the LLM's text output.
    fn parse_recap_json(text: &str, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        // The LLM may include markdown fences; strip them.
        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .map_err(|e| SessionError::RecapFailed(format!("JSON parse error: {}", e)))?;

        // Build the fingerprint once so the persisted fingerprint and the
        // theory-grounded flavour (#209) read from the same measured signal.
        let fingerprint = build_fingerprint(&input.phrases);
        let flavour = theory_flavour(&fingerprint);

        let recap = SessionRecap {
            score_summary: input
                .score_title
                .as_deref()
                .and_then(|t| score_practice_summary(t, &input.note_verdicts)),
            overall_assessment: parsed
                .get("overall_assessment")
                .and_then(|v| v.as_str())
                .unwrap_or("Great session! Keep building on your progress.")
                .to_owned(),
            strengths: parsed
                .get("strengths")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_owned()))
                        .collect()
                })
                .unwrap_or_else(|| vec!["Consistent focus during practice.".to_owned()]),
            areas_to_improve: parsed
                .get("areas_to_improve")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_owned()))
                        .collect()
                })
                .unwrap_or_else(|| {
                    vec!["Consider recording yourself to hear progress over time.".to_owned()]
                }),
            next_session_suggestions: parsed
                .get("next_session_suggestions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_owned()))
                        .collect()
                })
                .unwrap_or_else(|| vec!["Focus on one phrase at a time.".to_owned()]),
            duration_secs: input.duration_secs,
            phrase_count: input.phrases.len(),
            instrument: input.instrument.clone(),
            fingerprint: (!fingerprint.is_empty()).then_some(fingerprint),
            // Theory-grounded flavour from mode + swing (#209), shared with the
            // offline path. Hedged, and `None` when there's no clear signal.
            flavour,
            // Carry the gated, offline idiom matches straight through. They are
            // grounded facts the recorder computed, not LLM output — so we
            // persist them verbatim regardless of what the model returned.
            idiom_notes: input.idiom_notes.clone(),
            // Honor the model's connections ONLY when the gate was open (a
            // profile existed AND the signal was groundable). If the gate is
            // closed, the prompt never asked for connections, so anything that
            // slipped through is dropped — the data model stays honest about
            // when a connection was actually grounded.
            connections: if connections_gate_open(input) {
                parsed
                    .get("connections")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str())
                            .map(|s| s.trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        };

        Ok(recap)
    }

    /// Fallback recap returned when the API fails or no key is configured.
    ///
    /// Delegates to [`grounded_offline_recap`] so both offline paths — the
    /// engine's network-failure fallback and the command layer's no-API-key
    /// branch — share one grounded generator that builds prose from the
    /// measured fingerprint instead of canned text.
    fn fallback_recap(input: &RecapInput) -> SessionRecap {
        grounded_offline_recap(input)
    }
}

/// A short, **hedged** "flavour" line grounded in the two signals the app
/// measures reliably — the key's **mode** and the groove's **swing ratio** —
/// or `None` when there is no clear signal to stand on ("silence > lies").
///
/// This deliberately replaces the placeholder offline idiom engine (#208) as
/// the *displayed* flavour: that corpus is synthetic, so it would report
/// "Chopin" for a G-Dorian swung scat. Mode + swing, by contrast, are computed
/// from the audio and trustworthy. The phrasing is always a "… feel" / "…
/// colour" — a hedge, never an asserted fact.
///
/// # Thresholds (tuned, documented)
///
/// - **swung**: `swing_ratio` present and `>= SWING_MIN` (1.4). Triplet swing
///   is ≈ 2.0, light swing ≈ 1.5; 1.4 keeps a clearly-swung feel in while
///   leaving a dead band before the straight cutoff so borderline grooves go
///   silent rather than mislabelled.
/// - **straight**: `swing_ratio` present and `< STRAIGHT_MAX` (1.25). Even
///   eighths come back ≈ 1.0; 1.25 absorbs measurement jitter.
/// - The `[1.25, 1.4)` band is intentionally ambiguous → no swing verdict.
/// - **mode trust**: the `key` is only consulted when its `confidence` is
///   `>= KEY_MIN_CONFIDENCE` (0.5), matching the recap's existing key gate.
///   *modal* = Dorian/Phrygian/Lydian/Mixolydian/Locrian; *diatonic* =
///   Ionian/Aeolian (plain major/minor — not distinctive on its own).
///
/// # Mapping
///
/// | swing    | mode     | flavour                                              |
/// |----------|----------|------------------------------------------------------|
/// | swung    | modal    | modal-jazz feel — modal lines over a swung pulse     |
/// | swung    | diatonic | swung, jazz-leaning feel                             |
/// | straight | modal    | modal colour                                         |
/// | straight | diatonic | `None` (plain major/minor + straight ⇒ no signal)    |
/// | (other)  | (other)  | `None`                                               |
///
/// `key` and `groove` are independent gates: a usable mode with no swing read,
/// or a swing read with no usable mode, still produce a line where the table
/// allows; only the all-ambiguous / no-signal case returns `None`.
pub fn theory_flavour(fp: &MusicalFingerprint) -> Option<String> {
    /// At/above this swing ratio we call the feel swung (triplet ≈ 2.0).
    const SWING_MIN: f32 = 1.4;
    /// Below this swing ratio we call the feel straight (even eighths ≈ 1.0).
    const STRAIGHT_MAX: f32 = 1.25;
    /// Don't trust the mode below this key confidence (matches the recap gate).
    const KEY_MIN_CONFIDENCE: f32 = 0.5;

    // Swing verdict: Some(true) swung, Some(false) straight, None ambiguous/absent.
    let swung = fp
        .groove
        .as_ref()
        .and_then(|g| g.swing_ratio)
        .and_then(|r| {
            if r >= SWING_MIN {
                Some(true)
            } else if r < STRAIGHT_MAX {
                Some(false)
            } else {
                None
            }
        });

    // Mode verdict: only when the key cleared its confidence gate AND the
    // session earned a flat assertion (#316). A hedged (Leaning/Drifted) key
    // must not drive a mode-named flavour line — the swing legs still fire,
    // so a hedged session degrades to the key-free variants rather than
    // silence. `key_claim == None` with a key present is a legacy blob:
    // treat as asserted, matching the behavior those recaps shipped with.
    let key_asserted = !fp.key_claim.is_some_and(|c| c.hedged());
    let modal = fp
        .key
        .as_ref()
        .filter(|k| k.confidence >= KEY_MIN_CONFIDENCE && key_asserted)
        .map(|k| {
            matches!(
                k.mode,
                theory::Mode::Dorian
                    | theory::Mode::Phrygian
                    | theory::Mode::Lydian
                    | theory::Mode::Mixolydian
                    | theory::Mode::Locrian
            )
        });

    // The measured specifics that make the line visibly derived from THIS
    // session (#277: a fixed string across sessions reads as hardcoded).
    let mode_name = fp
        .key
        .as_ref()
        .filter(|k| k.confidence >= KEY_MIN_CONFIDENCE && key_asserted)
        .map(|k| k.name());
    let swing_ratio = fp.groove.as_ref().and_then(|g| g.swing_ratio);
    let tempo = fp.groove.as_ref().and_then(|g| g.tempo_bpm);

    let pulse = match (swing_ratio, tempo) {
        (Some(r), Some(bpm)) => format!(" (swing ~{r:.1}:1 at ~{bpm:.0} BPM)"),
        (Some(r), None) => format!(" (swing ~{r:.1}:1)"),
        (None, Some(bpm)) => format!(" (~{bpm:.0} BPM)"),
        (None, None) => String::new(),
    };
    // On a straight/absent-swing verdict, quoting a swing ratio reads oddly —
    // carry only the tempo.
    let pulse_no_swing = tempo
        .map(|bpm| format!(" (~{bpm:.0} BPM)"))
        .unwrap_or_default();

    match (swung, modal) {
        (Some(true), Some(true)) => Some(format!(
            "a modal-jazz feel — {} lines over a swung pulse{pulse} (think Miles Davis)",
            mode_name.unwrap_or_else(|| "modal".to_owned()),
        )),
        // Swung but the mode is diatonic or untrusted → jazz-leaning, no modal claim.
        (Some(true), Some(false)) | (Some(true), None) => {
            Some(format!("a swung, jazz-leaning feel{pulse} (think bebop)"))
        }
        // Trusted modal mode with a straight or absent swing read → modal colour.
        (Some(false), Some(true)) | (None, Some(true)) => Some(format!(
            "a modal colour — {} shapes{pulse_no_swing}",
            mode_name.unwrap_or_else(|| "modal".to_owned()),
        )),
        // Straight + diatonic, or no usable signal at all → silence > lies.
        _ => None,
    }
}

/// Build a grounded, fully offline [`SessionRecap`] from the session's measured
/// [`MusicalFingerprint`], reusing the same `describe_*` helpers the online
/// prompt uses. Deterministic and network-free — the prose varies with the
/// numbers, never with a clock or RNG.
///
/// The contract is honesty over filler: every claim is sourced from a
/// fingerprint dimension that passed its evidence gate. A dimension that is
/// absent (its gate failed) contributes nothing — we never invent a tempo, a
/// cents figure, or a key that wasn't measured. A quiet session with no signal
/// therefore degrades to engagement-level encouragement with **no fabricated
/// numeric claims**, and `fingerprint` is carried through (`None` only when
/// nothing was measured), never thrown away.
/// #417-4/#389: families whose pitch the PLAYER cannot bend. Intonation
/// critique and tuner/drone advice are meaningless on these — measured
/// pitch deviation is the INSTRUMENT's tuning and is phrased as such,
/// and practice vocabulary speaks hands/voicing/evenness, never breath.
pub(crate) fn fixed_pitch_family(family: &str) -> bool {
    matches!(family, "Keyboard" | "Percussion")
}

/// #445-6b: the evidence bar for a FULL recap. Below it the session was a
/// quick touch, and the recap must say LESS, plainly — silence > lies
/// applies to word count too: a paragraph of coaching over twenty seconds
/// of noodling reads as fabrication even when every clause is gated.
pub const THIN_SESSION_MIN_PHRASES: usize = 3;
pub const THIN_SESSION_MIN_PLAYED_SECS: f64 = 20.0;

/// A session that produced SOME phrases but not enough to earn the full
/// recap. Zero phrases is NOT thin — the empty-state path (the copy the
/// founder singled out as the voice reference) stays untouched.
pub fn is_thin_session(input: &RecapInput) -> bool {
    // Review MF2: a score session where the follower JUDGED notes carries
    // measured accuracy with its own denominator — phrase count is the
    // wrong gate for it. Judged notes exempt the session from the thin
    // bar so the #337 S4 accuracy panel is never suppressed.
    if !input.note_verdicts.is_empty() {
        return false;
    }
    let played: f64 = input.phrases.iter().map(|p| p.duration_secs).sum();
    !input.phrases.is_empty()
        && (input.phrases.len() < THIN_SESSION_MIN_PHRASES || played < THIN_SESSION_MIN_PLAYED_SECS)
}

/// #445-6b: the honest short form — technical, warm, no filler (voice
/// reference: the can't-hear-you copy, #445 pt 7). Names exactly what
/// happened, says plainly there isn't enough to read, offers ONE next
/// step. A fingerprint dimension that genuinely cleared its gate still
/// surfaces its single strongest fact — measured truth is never
/// suppressed, only padding is.
pub fn thin_session_recap(input: &RecapInput) -> SessionRecap {
    let fingerprint = build_fingerprint(&input.phrases);
    let phrase_count = input.phrases.len();
    // Review MF4: the copy speaks the SAME clock the bar judged — summed
    // PLAYED seconds, never the wall clock (a 10-minute session holding
    // 60s of noodling is still a quick touch, and quoting "10 minutes"
    // while judging 60s is two clocks in one sentence).
    let played_secs: f64 = input.phrases.iter().map(|p| p.duration_secs).sum();
    let played_str = if played_secs < 120.0 {
        let s = played_secs.round().max(1.0) as i64;
        format!("{s} second{}", if s == 1 { "" } else { "s" })
    } else {
        format!("{} minutes", (played_secs / 60.0).round() as i64)
    };
    let mut overall = format!(
        "A quick touch — {phrase_count} phrase{}, about {played_str} of actual playing, on {}.",
        if phrase_count == 1 { "" } else { "s" },
        input.instrument,
    );
    // Review MF4: never claim we couldn't read what the fingerprint DID
    // read — the recap surface renders those rows right below this line.
    if fingerprint.is_empty() {
        overall.push_str(
            " That's not enough for me to read tone, key, or feel honestly, so I'll keep this short.",
        );
    } else {
        overall.push_str(" Not enough playing for a full read, so I'll keep this short.");
    }
    if let Some(s) = &fingerprint.intonation {
        if !fixed_pitch_family(&input.instrument_family) && s.note_count >= 4 {
            overall.push_str(&format!(
                " One thing I did catch: {:.0}% of the {} notes I heard landed in tune.",
                s.in_tune_ratio * 100.0,
                s.note_count,
            ));
        }
    }
    SessionRecap {
        overall_assessment: overall,
        strengths: Vec::new(),
        areas_to_improve: Vec::new(),
        next_session_suggestions: vec![
            "Settle in for a few minutes of continuous playing — a handful of full phrases and I'll have something real to say."
                .to_owned(),
        ],
        duration_secs: input.duration_secs,
        phrase_count,
        instrument: input.instrument.clone(),
        fingerprint: if fingerprint.is_empty() {
            None
        } else {
            Some(fingerprint)
        },
        // A thin session earns no flavour line, no idiom notes, no
        // cross-genre connection — those are full-recap privileges.
        flavour: None,
        idiom_notes: Vec::new(),
        connections: Vec::new(),
        score_summary: None,
    }
}

pub fn grounded_offline_recap(input: &RecapInput) -> SessionRecap {
    // #445-6b: a quick touch earns the short form, not the full essay.
    if is_thin_session(input) {
        return thin_session_recap(input);
    }
    let fixed_pitch = fixed_pitch_family(&input.instrument_family);
    let fingerprint = build_fingerprint(&input.phrases);
    let flavour = theory_flavour(&fingerprint);
    let duration_mins = (input.duration_secs / 60.0).round().max(1.0) as i32;
    let phrase_count = input.phrases.len();

    // --- Overall assessment ------------------------------------------------
    // Open with the session frame (always honest), then weave in the single
    // strongest measured read so the line reflects *this* session.
    let mut overall = format!(
        "You practiced for about {duration_mins} minute{} on {} across {phrase_count} phrase{}.",
        if duration_mins == 1 { "" } else { "s" },
        input.instrument,
        if phrase_count == 1 { "" } else { "s" },
    );
    if let Some(s) = &fingerprint.intonation {
        let centered = s.mean_cents.abs() <= 1.0;
        if fixed_pitch {
            // #389: the player cannot alter this — mention only a clear
            // instrument-level tendency, phrased as the instrument.
            if s.mean_cents.abs() >= 10.0 {
                overall.push_str(&format!(
                    " Your {} reads about {:.0} cents {} overall — that's the \
                     instrument's tuning, not your playing.",
                    input.instrument.to_lowercase(),
                    s.mean_cents.abs(),
                    if s.mean_cents > 0.0 { "sharp" } else { "flat" },
                ));
            }
        } else {
            overall.push_str(&format!(
                " Intonation: {} — {}.",
                if centered {
                    "centered overall".to_owned()
                } else if s.mean_cents > 0.0 {
                    "running a touch sharp".to_owned()
                } else {
                    "running a touch flat".to_owned()
                },
                describe_intonation(s),
            ));
        }
    }
    if let Some(g) = &fingerprint.groove {
        overall.push_str(&format!(" Feel held {}.", describe_groove(g)));
    }
    if fingerprint.is_empty() {
        // No measured signal at all — stay honest, claim no numbers.
        overall.push_str(
            " We didn't capture enough clear signal to read tone, key, intonation, or feel this \
             time, but showing up and playing is what moves you forward.",
        );
    }

    // --- Strengths ---------------------------------------------------------
    // Each is gated on a dimension that actually cleared its evidence bar.
    let mut strengths: Vec<String> = Vec::new();
    if let Some(s) = &fingerprint.intonation {
        // #389: an in-tune ratio on a fixed-pitch instrument measures the
        // instrument (and our polyphonic cent estimates), not the player.
        if !fixed_pitch && s.in_tune_ratio >= 0.7 {
            strengths.push(format!(
                "Solid intonation — {:.0}% of {} notes landed in tune.",
                s.in_tune_ratio * 100.0,
                s.note_count,
            ));
        }
    }
    if let Some(g) = &fingerprint.groove {
        if g.timing_consistency >= 0.85 {
            let tempo = g
                .tempo_bpm
                .map(|bpm| format!(" around {bpm:.0} BPM"))
                .unwrap_or_default();
            strengths.push(format!("Steady time{tempo} — your pulse stayed locked in."));
        }
    }
    if let Some(k) = &fingerprint.key {
        // "Sat firmly" is exactly the claim a hedged (Leaning/Drifted) key
        // didn't earn (#316, #404): the strength only renders on an asserted
        // key.
        if k.confidence >= 0.6 && !fingerprint.key_claim.is_some_and(|c| c.hedged()) {
            strengths.push(format!(
                "Clear tonal center — the session sat firmly in {}.",
                k.name(),
            ));
        }
    }
    if let Some(t) = &fingerprint.tone {
        if t.core_clarity >= 0.7 {
            strengths.push("Focused, present tone with a clear core.".to_owned());
        }
    }
    if strengths.is_empty() {
        // Nothing cleared a "strength" threshold — stay encouraging without
        // inventing a measured win.
        strengths.push("You showed up and played — that consistency is what builds.".to_owned());
    }

    // --- Areas to improve --------------------------------------------------
    let mut areas: Vec<String> = Vec::new();
    // #389 acceptance: a fixed-pitch recap contains NO player-intonation
    // critique — the whole block is the player-controllable path.
    if let Some(s) = fingerprint.intonation.as_ref().filter(|_| !fixed_pitch) {
        if s.in_tune_ratio < 0.7 {
            areas.push(format!(
                "Intonation drifted — only {:.0}% of notes sat in tune.",
                s.in_tune_ratio * 100.0,
            ));
        }
        if let Some(worst) = s
            .tendencies
            .iter()
            .filter(|t| t.count >= 2 && t.mean_cents.abs() >= 5.0)
            .max_by(|a, b| a.mean_cents.abs().total_cmp(&b.mean_cents.abs()))
        {
            let degree = degree_name(worst.semitones_from_tonic);
            let dir = if worst.mean_cents >= 0.0 {
                "sharp"
            } else {
                "flat"
            };
            areas.push(format!(
                "Your {degree} ran {:+.0} cents {dir} — worth a tuner check.",
                worst.mean_cents,
            ));
        }
    }
    if let Some(g) = &fingerprint.groove {
        if g.timing_consistency < 0.7 {
            areas.push("Timing wandered — the pulse drifted between phrases.".to_owned());
        }
    }
    if let Some(t) = &fingerprint.tone {
        // "Air in the tone" is breath vocabulary — meaningless on keys.
        if !fixed_pitch && t.air_noise >= 0.5 {
            areas.push(
                "A bit of air in the tone — tighten the core for a cleaner sound.".to_owned(),
            );
        }
    }
    // A score session the follower never judged (§6: player may have been
    // on a different piece) says so instead of silently showing nothing.
    if input.score_title.is_some() && input.note_verdicts.is_empty() && !input.phrases.is_empty() {
        areas.push(
            "I couldn't follow along with the score this time — was this the right piece?"
                .to_owned(),
        );
    }
    if areas.is_empty() {
        areas.push(
            "Keep recording yourself — each session builds on the last and tracks your progress."
                .to_owned(),
        );
    }
    // #454 S3: at most ONE method-book line — the entry this session's
    // measured fingerprint earned (evidence-gated in `pedagogy`). The
    // guidance ships verbatim (already the founder's attributed-paraphrase
    // voice); the parenthesized source line guarantees the attribution is in
    // the copy for every entry. It lands in areas_to_improve because the tip
    // is a deepened diagnosis of THIS session's measured deficit — the
    // history voice (trajectory, what to do next time) keeps
    // next_session_suggestions. Thin sessions returned the short form above,
    // so a quick touch never gains a book line (#445-6b).
    if let Some(tip) = &input.method_book_tip {
        areas.push(format!("{} ({})", tip.guidance, tip.source_line()));
    }

    // --- Next-session suggestions ------------------------------------------
    // Targeted to the weakest measured read, with safe defaults when quiet.
    let mut suggestions: Vec<String> = Vec::new();
    if let Some(k) = &fingerprint.key {
        if k.confidence >= 0.5 {
            // A hedged key is still a fine practice anchor — the suggestion
            // just says what it is (#316): the key the session drifted
            // toward, not the key it "was".
            suggestions.push(match (fixed_pitch, fingerprint.key_claim) {
                // #417-4: keyboard vocabulary — hands and evenness, not breath.
                (true, Some(KeyClaimStrength::Leaning)) => format!(
                    "Open with a slow scale in {} — the key you were leaning toward — \
                     hands together, listening for evenness between them.",
                    k.name()
                ),
                // #404: a drifted key was NOT the key at the end — anchor the
                // suggestion to the session, never to "you ended on".
                (true, Some(KeyClaimStrength::Drifted)) => format!(
                    "Open with a slow scale in {} — the key that carried most of the \
                     session — hands together, listening for evenness between them.",
                    k.name()
                ),
                (true, _) => format!(
                    "Open with a slow scale in {}, hands together — even touch, one \
                     steady tempo.",
                    k.name()
                ),
                (false, Some(KeyClaimStrength::Leaning)) => format!(
                    "Open with long tones in {} — the key you were leaning toward at the end.",
                    k.name()
                ),
                (false, Some(KeyClaimStrength::Drifted)) => format!(
                    "Open with long tones in {} — the key that carried most of the session.",
                    k.name()
                ),
                (false, _) => format!(
                    "Open with long tones in {}, the key you ended on.",
                    k.name()
                ),
            });
        }
    }
    if let Some(s) = &fingerprint.intonation {
        if fixed_pitch {
            // #389: never tuner/drone advice — but a strong instrument-level
            // tendency earns the honest note, phrased as the instrument.
            if s.mean_cents.abs() >= 10.0 {
                suggestions.push(format!(
                    "Your {} reads about {:.0} cents {} of center — a tuning visit \
                     would make everything you practice sound truer.",
                    input.instrument.to_lowercase(),
                    s.mean_cents.abs(),
                    if s.mean_cents > 0.0 { "sharp" } else { "flat" },
                ));
            }
        } else if let Some(worst) = s
            .tendencies
            .iter()
            .filter(|t| t.count >= 2 && t.mean_cents.abs() >= 5.0)
            .max_by(|a, b| a.mean_cents.abs().total_cmp(&b.mean_cents.abs()))
        {
            suggestions.push(format!(
                "Play a slow scale against a drone, listening for the {}.",
                degree_name(worst.semitones_from_tonic),
            ));
        } else if s.in_tune_ratio < 0.7 {
            suggestions
                .push("Spend a few minutes with a tuner drone to settle your pitch.".to_owned());
        }
    }
    if let Some(g) = &fingerprint.groove {
        if g.timing_consistency < 0.85 {
            let tempo = g
                .tempo_bpm
                .map(|bpm| format!(" at {bpm:.0} BPM"))
                .unwrap_or_default();
            suggestions.push(format!("Run the tricky passages with a metronome{tempo}."));
        }
    }
    if suggestions.is_empty() {
        suggestions.push(if fixed_pitch {
            "Warm up with a slow scale, hands together, then revisit what felt \
             hardest today."
                .to_owned()
        } else {
            "Warm up with long tones, then revisit what felt hardest today.".to_owned()
        });
    }
    // #453 S2: at most ONE history-grounded line — the analyzer's first by
    // its pinned order; the text already embeds its citation numbers, so it
    // ships verbatim. Thin sessions returned the short form above, so a
    // quick touch never stacks history onto its single suggestion (#445-6b).
    if let Some(h) = input.history_suggestions.first() {
        suggestions.push(h.text.clone());
    }

    SessionRecap {
        overall_assessment: overall,
        strengths,
        areas_to_improve: areas,
        next_session_suggestions: suggestions,
        score_summary: input
            .score_title
            .as_deref()
            .and_then(|t| score_practice_summary(t, &input.note_verdicts)),
        duration_secs: input.duration_secs,
        phrase_count,
        instrument: input.instrument.clone(),
        // Persist the measured fingerprint instead of throwing it away —
        // `None` only when nothing cleared a gate, so "nothing measured" stays
        // distinct from "some dimensions measured".
        fingerprint: (!fingerprint.is_empty()).then_some(fingerprint),
        // Theory-grounded flavour from mode + swing (#209) — hedged, and `None`
        // when there's no clear signal. Replaces the placeholder idiom corpus
        // (#208) as the *displayed* flavour.
        flavour,
        // Offline idiom matches are computed independently of any LLM, so carry
        // them through — the frontend hedges the "reminds me of" phrasing.
        idiom_notes: input.idiom_notes.clone(),
        // The offline path never reached the model, so there is no grounded
        // cross-genre reference to surface — empty, by design.
        connections: Vec::new(),
    }
}

/// Render a tone descriptor as a compact, label-led line for the LLM prompt.
/// We hand the model the labelled 0–1 values and let it phrase them warmly —
/// consistent with "coaching text, not a traffic-light display".
fn describe_tone(t: &tone::ToneDescriptor) -> String {
    format!(
        "brightness {:.2}, warmth {:.2}, air/noise {:.2}, core clarity {:.2}, vibrato {:.2} (each 0–1)",
        t.brightness, t.warmth, t.air_noise, t.core_clarity, t.vibrato_quality
    )
}

/// Render an intonation summary as a compact, grounded line for the LLM prompt.
/// We feed the model the *computed* numbers (mean cents, in-tune ratio, and the
/// single most out-of-tune scale degree) so it can phrase them warmly without
/// inventing any figures.
fn describe_intonation(s: &theory::IntonationSummary) -> String {
    let direction = if s.mean_cents > 1.0 {
        "tends sharp"
    } else if s.mean_cents < -1.0 {
        "tends flat"
    } else {
        "centered"
    };
    let mut line = format!(
        "mean {:+.0} cents ({direction}), {:.0}% within tolerance over {} notes",
        s.mean_cents,
        s.in_tune_ratio * 100.0,
        s.note_count
    );
    // Surface the single worst degree, when we have per-degree tendencies — a
    // concrete, teacher-style observation ("the 3rd ran sharp").
    if let Some(worst) = s
        .tendencies
        .iter()
        .filter(|t| t.count >= 2)
        .max_by(|a, b| a.mean_cents.abs().total_cmp(&b.mean_cents.abs()))
    {
        if worst.mean_cents.abs() >= 5.0 {
            let degree = degree_name(worst.semitones_from_tonic);
            let dir = if worst.mean_cents >= 0.0 {
                "sharp"
            } else {
                "flat"
            };
            line.push_str(&format!(
                "; the {degree} ran {:+.0} cents ({dir})",
                worst.mean_cents
            ));
        }
    }
    line
}

/// Human name for a scale degree given its distance in semitones from the
/// tonic. Used only for grounded recap phrasing.
fn degree_name(semitones_from_tonic: u8) -> &'static str {
    match semitones_from_tonic {
        0 => "tonic",
        1 => "flat 2nd",
        2 => "2nd",
        3 => "minor 3rd",
        4 => "major 3rd",
        5 => "4th",
        6 => "tritone",
        7 => "5th",
        8 => "minor 6th",
        9 => "major 6th",
        10 => "minor 7th",
        11 => "leading tone",
        _ => "degree",
    }
}

/// Render a groove descriptor as a compact, grounded line for the LLM prompt.
/// Only the computed facts are surfaced — tempo, swing ratio, and a plain-word
/// steadiness read derived from `timing_consistency`.
fn describe_groove(g: &groove::GrooveDescriptor) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(bpm) = g.tempo_bpm {
        parts.push(format!("~{:.0} BPM", bpm));
    }
    if let Some(swing) = g.swing_ratio {
        if swing >= 1.25 {
            parts.push(format!("swung ~{swing:.1}:1"));
        } else {
            parts.push("straight feel".to_owned());
        }
    }
    let steadiness = if g.timing_consistency >= 0.9 {
        "steady"
    } else if g.timing_consistency >= 0.7 {
        "mostly steady"
    } else {
        "uneven"
    };
    parts.push(steadiness.to_owned());
    parts.push(format!("{} onsets", g.onset_count));
    parts.join(", ")
}

/// Render the student's stated [`TasteProfile`] as a labelled context block for
/// the recap prompt. This is **stated preference**, not measured fact — it is
/// the one place preferences join the fingerprint (at coaching time), and the
/// prompt is careful to frame it as context the coach may relate to, never as
/// something the coach may assert was played.
///
/// Returns an empty string when the profile carries nothing to frame with (no
/// genres, artists, or goals), so a default-empty profile reads the same as no
/// profile at all — the data model stays honest about cold start.
fn describe_taste_profile(profile: &TasteProfile) -> String {
    if profile.genres.is_empty() && profile.artists.is_empty() && profile.goals.is_empty() {
        return String::new();
    }
    let mut block = String::from(
        "\n\nThe student also told us about the music they love (use this only to FRAME \
         connections — it is their taste, NOT something they played):\n",
    );
    if !profile.genres.is_empty() {
        block.push_str(&format!(
            "- Genres they like: {}\n",
            profile.genres.join(", ")
        ));
    }
    if !profile.artists.is_empty() {
        block.push_str(&format!(
            "- Artists they love: {}\n",
            profile.artists.join(", ")
        ));
    }
    if !profile.goals.is_empty() {
        block.push_str(&format!(
            "- Why they're here: {}\n",
            profile.goals.join(", ")
        ));
    }
    block.push_str(&format!(
        "- Experience level: {}",
        profile.experience.as_str()
    ));
    block
}

/// Whether grounded cross-genre connections may be attempted for this session.
///
/// Connections are gated on BOTH halves of the join being present and honest:
/// 1. a [`TasteProfile`] exists with something to frame with (a non-empty
///    genre/artist/goal list — a default-empty profile is treated as cold
///    start), AND
/// 2. the session produced enough measured signal to ground a connection —
///    i.e. at least one fingerprint dimension passed its evidence gate.
///
/// If either half is missing, the coach falls back to its existing behavior: no
/// forced cross-genre reference. This single gate is consulted by the system
/// prompt, the user prompt, and the response parser, so the prompt never asks
/// for something the parser would discard, and the parser never surfaces a
/// connection the prompt didn't ground.
fn connections_gate_open(input: &RecapInput) -> bool {
    let has_profile = input
        .taste_profile
        .as_ref()
        .is_some_and(|p| !describe_taste_profile(p).is_empty());
    let has_signal = fingerprint_for_recap(&input.phrases).is_some();
    has_profile && has_signal
}

/// Per-measure verdict counts for the score recap (#337 S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasureVerdicts {
    pub measure_number: usize,
    pub hit: usize,
    pub near: usize,
    pub missed: usize,
}

/// What a score-practice session amounted to (#337 S4): honest accuracy
/// over the notes the follower actually JUDGED, and the measures that most
/// need work. Rides `SessionRecap.score_summary` (additive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorePracticeSummary {
    pub score_title: String,
    /// Notes the follower judged this session — the denominator.
    pub judged: usize,
    /// Clean hits as a percentage of judged notes, 0..=100.
    pub accuracy_pct: f32,
    /// Up to [`WORST_MEASURES_CAP`] measures ranked worst-first (most
    /// missed, then most rough). Only measures with a non-clean note appear.
    pub worst_measures: Vec<MeasureVerdicts>,
}

/// How many worst measures the recap names — enough to practice, not a wall.
const WORST_MEASURES_CAP: usize = 3;

/// Aggregate a session's note verdicts into the score recap summary
/// (#337 S4). `None` when nothing was judged — a session where the follower
/// never locked says nothing about the piece (silence > lies).
pub fn score_practice_summary(
    score_title: &str,
    verdicts: &[crate::follower::NoteVerdict],
) -> Option<ScorePracticeSummary> {
    use crate::follower::Verdict;
    if verdicts.is_empty() {
        return None;
    }
    let mut per_measure: std::collections::BTreeMap<usize, MeasureVerdicts> =
        std::collections::BTreeMap::new();
    let mut hits = 0usize;
    for v in verdicts {
        let m = per_measure
            .entry(v.measure_number)
            .or_insert(MeasureVerdicts {
                measure_number: v.measure_number,
                hit: 0,
                near: 0,
                missed: 0,
            });
        match v.verdict {
            Verdict::Hit => {
                m.hit += 1;
                hits += 1;
            }
            Verdict::Near => m.near += 1,
            Verdict::Missed => m.missed += 1,
        }
    }
    let judged = verdicts.len();
    let mut worst: Vec<MeasureVerdicts> = per_measure
        .into_values()
        .filter(|m| m.near + m.missed > 0)
        .collect();
    worst.sort_by(|a, b| {
        (b.missed, b.near)
            .cmp(&(a.missed, a.near))
            .then(a.measure_number.cmp(&b.measure_number))
    });
    worst.truncate(WORST_MEASURES_CAP);
    Some(ScorePracticeSummary {
        score_title: score_title.to_owned(),
        judged,
        accuracy_pct: (hits as f32 / judged as f32) * 100.0,
        worst_measures: worst,
    })
}

/// Build the session's [`MusicalFingerprint`] from the per-dimension
/// aggregation. Each dimension reuses its existing evidence gate (see the
/// `aggregate_*` functions), so a dimension is `Some` only when the session
/// produced enough evidence to report it honestly. This is the single place
/// the four measurements are assembled — the recap prompt and the persisted
/// recap both source their grounded facts from the result.
///
/// `pub` since #454 S3: the command layer's live end-session path runs the
/// pedagogy selection engine over THIS session's evidence, and it must read
/// the same assembly (same evidence gates) the recap generators read — never
/// a re-implementation, never a store read-back.
pub fn build_fingerprint(phrases: &[PhraseSummary]) -> MusicalFingerprint {
    let (key, key_claim) = match aggregate_key(phrases) {
        KeyVerdict::Claimed(est, strength) => (Some(est), Some(strength)),
        KeyVerdict::Unsettled => (None, Some(KeyClaimStrength::Unsettled)),
        KeyVerdict::Silent => (None, None),
    };
    MusicalFingerprint {
        tone: aggregate_tone(phrases),
        key,
        key_claim,
        intonation: aggregate_intonation(phrases),
        groove: aggregate_groove(phrases),
    }
}

/// The fingerprint to persist on a [`SessionRecap`]: `None` when nothing was
/// measured (every gate failed), otherwise `Some` with whatever dimensions
/// passed. Collapsing the all-`None` case to `None` keeps "nothing measured"
/// distinct from "some dimensions measured" at the recap level.
fn fingerprint_for_recap(phrases: &[PhraseSummary]) -> Option<MusicalFingerprint> {
    let fingerprint = build_fingerprint(phrases);
    (!fingerprint.is_empty()).then_some(fingerprint)
}

/// Session-level key/mode for the recap — derived from the **same live
/// tracking the player watched during the session**, never a separate
/// re-estimation.
///
/// Each phrase carries the session-long `theory::KeyTracker` reading at the
/// moment it closed (`PhraseSummary::key`) — the same tracker family behind
/// the live "I hear" header, fed per NOTE through the same gate (#324: fed
/// per frame instead, the phrase tracker wandered where the strip stayed
/// calm, and the vote below hedged genuinely steady sessions). The recap
/// takes a confidence-weighted vote over
/// those tracked readings (ties → the later call wins, since the tracker had
/// more evidence by then). A pooled whole-session re-estimate was used before
/// and could confidently name a key the header **never showed** (#277) — the
/// recap must never contradict what the player watched, so it now votes only
/// over keys that were actually tracked live, or stays silent.
///
/// The session-level key verdict (#316): what key, if any, the recap may talk
/// about, and how firmly. Internal to the aggregation — the persisted shape
/// is [`MusicalFingerprint::key`] + [`MusicalFingerprint::key_claim`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum KeyVerdict {
    /// A key the recap may name, at the given strength.
    Claimed(theory::KeyEstimate, KeyClaimStrength),
    /// Tonal readings existed but never firmed into a claimable key — the
    /// recap says the key kept moving, and names nothing.
    Unsettled,
    /// No tonal readings at all (percussive / silent material) — the recap
    /// says nothing about key whatsoever.
    Silent,
}

/// The returned [`KeyVerdict`] is the honesty contract of #316 — the recap
/// must not state a key more firmly than the live tracking earned:
///
/// - **Every** displayed reading participates. The live tracker commits keys
///   from confidence 0.4 and holds them as confidence decays, so the strip
///   can end on a reading below the 0.5 claim bar — the aggregation must
///   still see it, or the recap can flatly contradict the strip (the review
///   probe on the first cut of this code). Sub-bar readings contribute to
///   the denominator and to the final-reading check; only claim-*eligible*
///   keys (some sighting ≥ [`MIN_CONFIDENCE`]) can be named.
/// - `Asserted` needs all three: the winner is where the strip **ended**
///   (its trailing run), it carried ≥ [`ASSERT_SHARE`] of the total tracked
///   confidence mass (wandering time counts against it), and it was seen in
///   at least two phrases (one reading is never "sat firmly").
/// - Winner ≠ final reading — the VA's half-step case (#313): if the closing
///   key held ≥ 2 phrases, defer to it (what the player last watched),
///   `Leaning`; if the session ended on a single-phrase blip, claim the
///   dominant key as `Drifted` instead — a blip may not hijack the recap,
///   but the dominant key is not where the strip ended, so it must never be
///   phrased as an end-state (#404). Either way, neither key is ever
///   asserted flatly.
/// - Readings but no eligible key → `Unsettled`. No readings → `Silent`.
fn aggregate_key(phrases: &[PhraseSummary]) -> KeyVerdict {
    /// A key must have at least one sighting at/above this confidence to be
    /// *claimable* — below it, the tracking never firmed up enough to name.
    const MIN_CONFIDENCE: f32 = 0.5;
    /// The winning key must carry at least this share of the total tracked
    /// confidence mass (including sub-bar wandering) to be asserted flatly.
    /// A session that wandered for most of its length and settled only near
    /// the end (#313: "a coin flip") lands below this and *leans*.
    const ASSERT_SHARE: f32 = 0.6;

    type KeyId = (u8, &'static str);
    let id = |k: &theory::KeyEstimate| -> KeyId { (k.tonic % 12, k.mode.label()) };

    // (mass, last idx, exemplar, eligible, phrase count) per key — over ALL
    // non-NaN readings, so displayed-but-shaky time is never invisible.
    let mut votes: std::collections::BTreeMap<KeyId, (f32, usize, theory::KeyEstimate, bool, u32)> =
        std::collections::BTreeMap::new();
    let mut total_mass = 0.0f32;
    let mut readings: Vec<theory::KeyEstimate> = Vec::new();
    for (idx, p) in phrases.iter().enumerate() {
        let Some(key) = p.key else { continue };
        if key.confidence.is_nan() {
            continue;
        }
        total_mass += key.confidence;
        readings.push(key);
        let entry = votes.entry(id(&key)).or_insert((0.0, idx, key, false, 0));
        entry.0 += key.confidence;
        entry.1 = idx; // most recent phrase carrying this key
        if key.confidence > entry.2.confidence {
            entry.2 = key; // keep the most confident sighting as the exemplar
        }
        entry.3 |= key.confidence >= MIN_CONFIDENCE;
        entry.4 += 1;
    }
    if readings.is_empty() {
        return KeyVerdict::Silent;
    }
    let Some((winner_mass, _, winner, _, winner_count)) =
        votes.into_values().filter(|v| v.3).max_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        })
    else {
        return KeyVerdict::Unsettled;
    };
    // What the live "I hear" strip showed as the session closed, and for how
    // many closing phrases it held.
    let final_reading = *readings.last().expect("readings is non-empty");
    let final_run = readings
        .iter()
        .rev()
        .take_while(|k| id(k) == id(&final_reading))
        .count();
    if id(&winner) != id(&final_reading) {
        // The vote and the strip's end state disagree — the exact
        // contradiction #313 caught. Never assert either key: defer to the
        // closing key when it genuinely held (≥ 2 phrases), otherwise claim
        // the dominant key — a single-phrase closing blip may not hijack
        // the recap. The dominant key is `Drifted`, not `Leaning` (#404):
        // it is NOT where the strip ended, so every "toward the end"
        // phrasing downstream would name a key the player never watched
        // there (the VA's "leaning F Phrygian" over a cycling
        // E Locrian / F major close).
        return if final_run >= 2 {
            KeyVerdict::Claimed(final_reading, KeyClaimStrength::Leaning)
        } else {
            KeyVerdict::Claimed(winner, KeyClaimStrength::Drifted)
        };
    }
    let strength = if winner_mass / total_mass >= ASSERT_SHARE && winner_count >= 2 {
        KeyClaimStrength::Asserted
    } else {
        KeyClaimStrength::Leaning
    };
    KeyVerdict::Claimed(winner, strength)
}

/// Session-level intonation over every phrase's detected pitches. Returns
/// `None` until enough notes accumulate to report honestly — a handful of
/// pitches makes for a noisy, untrustworthy cents figure, and the recap should
/// stay silent rather than assert a shaky tendency.
///
/// When a confident session key is available, its tonic is passed through so
/// the summary can attribute tendencies to specific scale degrees ("the 3rd
/// ran sharp"). Without a key, only the overall cents statistics are reported.
fn aggregate_intonation(phrases: &[PhraseSummary]) -> Option<theory::IntonationSummary> {
    /// Below this many usable pitch observations, don't report intonation —
    /// the mean would swing wildly on a couple of notes.
    const MIN_NOTES: u32 = 12;

    // Degree names ("the 3rd ran sharp") are key-relative claims stated as
    // fact downstream, so they anchor only to an ASSERTED key (#316 review):
    // with a hedged or absent key the attribution would be a coin flip, and
    // the summary reports raw cents statistics with no degree tendencies.
    let tonic = match aggregate_key(phrases) {
        KeyVerdict::Claimed(k, KeyClaimStrength::Asserted) => Some(k.tonic),
        _ => None,
    };

    let pitches: Vec<f32> = phrases
        .iter()
        .flat_map(|p| p.pitch_stats.pitches.iter().map(|&hz| hz as f32))
        .collect();

    let summary =
        theory::summarize_intonation(&pitches, tonic, theory::DEFAULT_IN_TUNE_TOLERANCE_CENTS)?;
    (summary.note_count >= MIN_NOTES).then_some(summary)
}

/// Session-level groove (tempo / swing / timing) over every phrase's retained
/// onset timestamps. Returns `None` until enough onsets accumulate — tempo and
/// swing are meaningless from one or two onsets, so the recap stays silent
/// rather than print a fabricated BPM.
fn aggregate_groove(phrases: &[PhraseSummary]) -> Option<groove::GrooveDescriptor> {
    /// Below this many onsets, don't report groove. Swing estimation already
    /// needs ≥ 4 onsets; we ask for a few more so the tempo/timing figures are
    /// stable rather than a fluke of two intervals.
    const MIN_ONSETS: u32 = 6;

    let onsets: Vec<f64> = phrases
        .iter()
        .flat_map(|p| p.onsets_secs.iter().copied())
        .collect();

    let descriptor = groove::analyze_groove(&onsets)?;
    (descriptor.onset_count >= MIN_ONSETS).then_some(descriptor)
}

/// Mean tone descriptor across the phrases that carry one. `None` when no
/// phrase had tone analysis (e.g. tone disabled or all phrases too short).
fn aggregate_tone(phrases: &[PhraseSummary]) -> Option<tone::ToneDescriptor> {
    let toned: Vec<&tone::ToneDescriptor> =
        phrases.iter().filter_map(|p| p.tone.as_ref()).collect();
    if toned.is_empty() {
        return None;
    }
    let n = toned.len() as f32;
    let mean = |f: fn(&tone::ToneDescriptor) -> f32| toned.iter().map(|t| f(t)).sum::<f32>() / n;
    Some(tone::ToneDescriptor {
        brightness: mean(|t| t.brightness),
        warmth: mean(|t| t.warmth),
        air_noise: mean(|t| t.air_noise),
        core_clarity: mean(|t| t.core_clarity),
        vibrato_quality: mean(|t| t.vibrato_quality),
    })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrase::{DynamicsStats, PitchStats};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // Mock HTTP client
    // -----------------------------------------------------------------------

    /// A mock HTTP client that records calls and returns a canned response.
    struct MockHttpClient {
        /// The response body to return.
        response: Result<String, String>,
        /// How many times `post_json` was called.
        call_count: Arc<AtomicUsize>,
    }

    impl MockHttpClient {
        fn succeeding(response_body: &str) -> Self {
            Self {
                response: Ok(response_body.to_owned()),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn failing(error_msg: &str) -> Self {
            Self {
                response: Err(error_msg.to_owned()),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl HttpClient for MockHttpClient {
        async fn post_json(
            &self,
            _url: &str,
            _body: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, CoachingError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);

            match &self.response {
                Ok(resp) => Ok(resp.clone()),
                Err(msg) => Err(CoachingError::HttpError(msg.clone())),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn sample_phrase() -> PhraseSummary {
        PhraseSummary {
            phrase_index: 0,
            start_time: 0.0,
            end_time: 1.5,
            duration_secs: 1.5,
            note_count: 12,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 39.5,
                pitches: vec![440.0; 12],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.65,
                min_amplitude: 0.4,
                max_amplitude: 0.9,
                dynamic_range: 0.5,
            },
            stability: 0.92,
            score_position: None,
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
            score_span: None,
            verdicts: None,
            score_card: None,
        }
    }

    /// A phrase tagged with a score position (began at the given measure).
    fn sample_phrase_at_measure(phrase_index: usize, measure: usize) -> PhraseSummary {
        let mut p = sample_phrase();
        p.phrase_index = phrase_index;
        p.score_position = Some(crate::follower::ScorePosition {
            measure_number: measure,
            beat: 0.0,
            section_name: None,
            expected_note: None,
        });
        p
    }

    #[test]
    fn recap_prompt_names_the_score_and_cites_measures() {
        // A score-backed session: the prompt must name the piece and list
        // the measure each phrase began on, so the LLM can anchor feedback
        // to real bars.
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![
                sample_phrase_at_measure(0, 1),
                sample_phrase_at_measure(1, 5),
            ],
            tips: vec![],
            score_title: Some("Haydn Trumpet Concerto".to_owned()),
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let prompt = CoachingEngine::build_recap_user_prompt(&input);

        assert!(
            prompt.contains("Haydn Trumpet Concerto"),
            "prompt should name the piece, got:\n{prompt}"
        );
        assert!(
            prompt.contains("measure 5"),
            "prompt should cite the measure a phrase began on, got:\n{prompt}"
        );
        assert!(
            prompt.contains("refer to specific measures"),
            "score-backed prompt should instruct the LLM to cite measures, got:\n{prompt}"
        );
    }

    #[test]
    fn recap_prompt_omits_score_block_in_free_play() {
        // No score → no piece name, no measure map, no measure instruction.
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let prompt = CoachingEngine::build_recap_user_prompt(&input);

        assert!(
            !prompt.contains("Where each phrase sat"),
            "free play must not include the measure map"
        );
        assert!(
            !prompt.contains("refer to specific measures"),
            "free play must not ask the LLM to cite measures"
        );
        // Still names the instrument plainly.
        assert!(prompt.contains("trumpet"));
    }

    fn sample_context() -> SessionContext {
        SessionContext {
            instrument: "trumpet".to_owned(),
            session_duration_secs: 120.0,
            phrases_played: 5,
            previous_tips: vec!["Try relaxing your embouchure on the high notes.".to_owned()],
            score_title: None,
        }
    }

    fn sample_tone() -> tone::ToneDescriptor {
        tone::ToneDescriptor {
            brightness: 0.62,
            warmth: 0.48,
            air_noise: 0.18,
            core_clarity: 0.80,
            vibrato_quality: 0.55,
        }
    }

    #[test]
    fn live_tip_prompt_includes_tone_only_when_present() {
        let ctx = sample_context();

        let without = CoachingEngine::build_user_prompt(&sample_phrase(), &ctx);
        assert!(!without.contains("Tone:"), "no tone line when tone is None");

        let toned = PhraseSummary {
            tone: Some(sample_tone()),
            ..sample_phrase()
        };
        let with = CoachingEngine::build_user_prompt(&toned, &ctx);
        assert!(
            with.contains("- Tone:"),
            "tone line present when tone is Some"
        );
        assert!(with.contains("brightness 0.62"), "tone values rendered");
    }

    #[test]
    fn recap_prompt_includes_tone_summary_when_present() {
        let toned = PhraseSummary {
            tone: Some(sample_tone()),
            ..sample_phrase()
        };
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![toned],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(prompt.contains("Tone quality:"), "recap prompt names tone");

        // The generated recap also carries the session tone aggregate for
        // persistence + trends, now via the unified fingerprint.
        let recap = CoachingEngine::fallback_recap(&input);
        assert!(
            recap
                .fingerprint
                .as_ref()
                .and_then(|f| f.tone.as_ref())
                .is_some(),
            "recap fingerprint should carry the session tone aggregate"
        );
    }

    #[test]
    fn aggregate_key_names_a_clear_key_and_hedges_otherwise() {
        // No phrases → no tonal readings → silence, not "kept moving".
        assert_eq!(aggregate_key(&[]), KeyVerdict::Silent);

        let tracked = |tonic: u8, mode: theory::Mode, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode,
                confidence,
                margin: 0.2,
            });
            p
        };

        // The recap votes over the LIVE-TRACKED per-phrase keys (#277): the
        // confidence-weighted winner is what the player actually watched.
        // The wander sits mid-session so the vote and the final reading
        // agree — the dominant key is asserted flatly.
        let phrases = vec![
            tracked(0, theory::Mode::Ionian, 0.8),
            tracked(7, theory::Mode::Mixolydian, 0.6), // brief mid-session wander
            tracked(0, theory::Mode::Ionian, 0.8),
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a clear key must be claimed");
        };
        assert_eq!(key.name(), "C major", "got {}", key.name());
        assert_eq!(strength, KeyClaimStrength::Asserted, "dominant + final");

        // Low-confidence tracked readings never firm into a claim — but the
        // session isn't silent either: the honest verdict is "kept moving".
        let shaky = vec![tracked(3, theory::Mode::Dorian, 0.3)];
        assert_eq!(aggregate_key(&shaky), KeyVerdict::Unsettled);

        // Phrases with no tracked key at all (thin / percussive material) →
        // Silent: "kept moving" would claim motion nothing observed.
        assert_eq!(
            aggregate_key(std::slice::from_ref(&sample_phrase())),
            KeyVerdict::Silent,
            "untracked phrases must not yield a key"
        );
    }

    /// #277: the recap can never name a key the live tracker never showed —
    /// every candidate comes from `PhraseSummary::key` (the tracked stream),
    /// so a pooled re-estimation naming an unseen key is structurally
    /// impossible. This pins that the winner is always one of the tracked
    /// readings.
    #[test]
    fn aggregate_key_only_names_live_tracked_keys() {
        let mut a = sample_phrase();
        a.key = Some(theory::KeyEstimate {
            tonic: 6,
            mode: theory::Mode::Phrygian,
            confidence: 0.9,
            margin: 0.3,
        });
        // Raw pitches that a pooled estimator would read as something else
        // entirely (a C-major scale) — they must be ignored.
        a.pitch_stats.pitches = vec![261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(std::slice::from_ref(&a)) else {
            panic!("tracked key wins");
        };
        assert_eq!(key.name(), "F# Phrygian", "got {}", key.name());
        // A single reading can be named, but never "sat firmly" (#316).
        assert_eq!(strength, KeyClaimStrength::Leaning);
    }

    /// The vote is CONFIDENCE-weighted, not a head count: two 0.9 readings of
    /// one key outvote three 0.5 readings of another. Fails if the weight
    /// regresses to counting.
    #[test]
    fn aggregate_key_weights_by_confidence_not_count() {
        let tracked = |tonic: u8, mode: theory::Mode, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode,
                confidence,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(0, theory::Mode::Ionian, 0.5),
            tracked(0, theory::Mode::Ionian, 0.5),
            tracked(0, theory::Mode::Ionian, 0.5), // 3 votes, weight 1.5
            tracked(7, theory::Mode::Mixolydian, 0.9),
            tracked(7, theory::Mode::Mixolydian, 0.9), // 2 votes, weight 1.8
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(key.name(), "G Mixolydian", "got {}", key.name());
        // 1.8 of 3.3 total mass = a contested vote → the claim hedges (#316).
        assert_eq!(strength, KeyClaimStrength::Leaning);
    }

    /// On an exact weight tie, the key the tracker saw LATER wins (it had more
    /// evidence by then). Fails if the recency tiebreak is reversed or lost.
    #[test]
    fn aggregate_key_breaks_ties_by_recency() {
        let tracked = |tonic: u8, mode: theory::Mode| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode,
                confidence: 0.7,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(0, theory::Mode::Ionian),
            tracked(9, theory::Mode::Aeolian), // same weight, seen later
        ];
        let KeyVerdict::Claimed(key, _) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(
            key.name(),
            "A minor",
            "later key wins ties, got {}",
            key.name()
        );
    }

    /// A NaN-confidence tracked reading is discarded, never poisoning a vote.
    #[test]
    fn aggregate_key_ignores_nan_confidence() {
        let mut good = sample_phrase();
        good.key = Some(theory::KeyEstimate {
            tonic: 0,
            mode: theory::Mode::Ionian,
            confidence: 0.7,
            margin: 0.2,
        });
        let mut bad = sample_phrase();
        bad.key = Some(theory::KeyEstimate {
            tonic: 5,
            mode: theory::Mode::Lydian,
            confidence: f32::NAN,
            margin: 0.2,
        });
        let KeyVerdict::Claimed(key, _) = aggregate_key(&[good, bad]) else {
            panic!("the good reading claims");
        };
        assert_eq!(key.name(), "C major", "got {}", key.name());
    }

    /// #316 AC1 — the VA's half-step case (#313): G major wins the
    /// confidence vote, but the live strip ENDED on G# major. The recap must
    /// defer to the final live reading, hedged — flatly asserting either key
    /// would contradict what the player watched. Fails on pre-#316 code,
    /// which asserted the vote winner ("G major") outright.
    #[test]
    fn recap_defers_to_the_final_live_reading_on_contradiction() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(7, 0.8), // G major dominates the vote…
            tracked(7, 0.8),
            tracked(8, 0.7), // …but the tracker settled on G# at the end
            tracked(8, 0.7), // (held ≥ 2 phrases — genuinely where it ended)
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(key.name(), "G# major", "must follow the strip's end state");
        assert_eq!(
            strength,
            KeyClaimStrength::Leaning,
            "a contradicted key is never asserted flatly"
        );
    }

    /// #316 review MUST-FIX 1 — the live tracker commits keys from
    /// confidence 0.4 and holds them as confidence decays, so the strip can
    /// END on a reading below the 0.5 claim bar. The aggregation must still
    /// see it: a session that closed on sub-bar G readings must not flatly
    /// assert the earlier G# — that's the literal #313 bug in the
    /// [0.4, 0.5) band. Fails if sub-bar readings are filtered before the
    /// final-reading check.
    #[test]
    fn a_sub_bar_closing_key_still_counts_as_the_strips_end_state() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(8, 0.7), // G# major, confidently…
            tracked(8, 0.7),
            tracked(7, 0.45), // …then the strip drifts to G and HOLDS it,
            tracked(7, 0.45), // displayed below the claim bar
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(
            key.name(),
            "G major",
            "the recap must follow the strip's displayed end state"
        );
        assert_eq!(strength, KeyClaimStrength::Leaning);
    }

    /// #316 review MUST-FIX 2 — sub-bar wandering counts AGAINST assertion:
    /// the canonical #313 session (wanders shakily for most of its length,
    /// settles late) must lean even though only the settled key clears the
    /// claim bar. Also pins the two-sighting floor: a single counted reading
    /// alone is never "sat firmly". Fails if the assert-share denominator
    /// drops sub-bar readings, or the count floor is removed.
    #[test]
    fn sub_bar_wandering_counts_against_assertion() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p
        };
        // Four shaky wandering keys (displayed live at ≥0.4), then G# settles.
        let phrases = vec![
            tracked(2, 0.45),
            tracked(4, 0.45),
            tracked(9, 0.45),
            tracked(5, 0.45),
            tracked(8, 0.7),
            tracked(8, 0.7),
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(key.name(), "G# major");
        assert_eq!(
            strength,
            KeyClaimStrength::Leaning,
            "most of the session wandered — settling late must not read as 'sat firmly'"
        );

        // A single counted reading with nothing else: claimable, never firm.
        let KeyVerdict::Claimed(_, strength) = aggregate_key(&[tracked(8, 0.7)]) else {
            panic!("a key was tracked");
        };
        assert_eq!(strength, KeyClaimStrength::Leaning);
    }

    /// #324 — the VA's "one thing to change": a session that sits steadily
    /// in ONE key must earn the FLAT assertion, end-to-end through the real
    /// phrase aggregation. Fails when the aggregator feeds its key tracker
    /// per frame instead of per note: the collapsed rolling window snapshots
    /// wandering relative-mode readings across phrases, the vote's winner
    /// share drops below the assert bar, and the recap hedges ("leaning G#
    /// major toward the end") a session the strip showed as rock-steady.
    #[test]
    fn a_steady_session_earns_the_flat_assertion() {
        let mut agg = crate::phrase::PhraseAggregator::new(crate::phrase::PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .expect("valid config");
        let frame = |hz: f64, t: f64| ears::AudioEvent {
            pitch_hz: Some(hz),
            confidence: 0.9,
            amplitude: 0.6,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        };
        // G# major, tonic and fifth emphasized, at the pipeline's real
        // ~45 Hz frame rate — several phrases separated by breaths.
        let gs_major = [415.30, 466.16, 523.25, 554.37, 622.25, 698.46, 783.99];
        let mut t = 0.0;
        for _ in 0..6 {
            for (i, &hz) in gs_major.iter().enumerate() {
                let dur = match i {
                    0 => 0.5,
                    4 => 0.35,
                    _ => 0.2,
                };
                let end = t + dur;
                while t < end {
                    agg.push(&frame(hz, t));
                    t += 0.022;
                }
            }
            t += 0.5; // a breath → phrase boundary
        }
        agg.flush();
        let phrases = agg.phrases();
        assert!(phrases.len() >= 4, "expected several phrases");

        let KeyVerdict::Claimed(key, strength) = aggregate_key(phrases) else {
            panic!("a steady session must claim its key");
        };
        assert_eq!(key.name(), "G# major", "got {}", key.name());
        assert_eq!(
            strength,
            KeyClaimStrength::Asserted,
            "a session that sat in one key the whole time earned the flat claim"
        );
    }

    /// #316 review SHOULD 3 + #404 — a single-phrase blip at session end
    /// must not hijack the recap key: with C major dominant and one closing
    /// G-Mixolydian phrase, the recap claims C (hedged), not the blip. And
    /// the claim is `Drifted`, not `Leaning`: C is NOT where the strip
    /// ended, so "leaning C toward the end" would name an end-state the
    /// player never watched. Fails if the contradiction branch defers to any
    /// final reading regardless of how briefly it held, or if it anchors the
    /// dominant key to the session's close.
    #[test]
    fn a_single_phrase_closing_blip_does_not_hijack_the_recap_key() {
        let tracked = |tonic: u8, mode: theory::Mode, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode,
                confidence,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(0, theory::Mode::Ionian, 0.8),
            tracked(0, theory::Mode::Ionian, 0.8),
            tracked(0, theory::Mode::Ionian, 0.8),
            tracked(7, theory::Mode::Mixolydian, 0.6), // one-phrase closing blip
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(
            key.name(),
            "C major",
            "the blip must not win: {}",
            key.name()
        );
        assert_eq!(
            strength,
            KeyClaimStrength::Drifted,
            "the dominant key was not the strip's end state — whole-session claim only"
        );
    }

    /// #404 AC1 — the VA's literal shape: one key carries the session, then
    /// the strip cycles between two other keys at the close (every closing
    /// run is a single phrase). The recap must claim the dominant key as
    /// `Drifted` — "mostly F Phrygian" — never `Leaning`, whose every
    /// rendering says "toward the end" and contradicts the cycling strip the
    /// player watched. Fails on pre-#404 code, which returned `Leaning` from
    /// the blip branch.
    #[test]
    fn a_cycling_session_end_reads_mostly_the_dominant_key() {
        let tracked = |tonic: u8, mode: theory::Mode, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode,
                confidence,
                margin: 0.2,
            });
            p
        };
        let phrases = vec![
            tracked(5, theory::Mode::Phrygian, 0.7), // F Phrygian carries…
            tracked(5, theory::Mode::Phrygian, 0.7),
            tracked(5, theory::Mode::Phrygian, 0.7),
            tracked(4, theory::Mode::Locrian, 0.5), // …then the close cycles
            tracked(5, theory::Mode::Ionian, 0.5),  // E Locrian / F major
            tracked(4, theory::Mode::Locrian, 0.5),
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(key.name(), "F Phrygian", "the session's carrier is named");
        assert_eq!(
            strength,
            KeyClaimStrength::Drifted,
            "a cycling close means the carrier was not the end state"
        );
    }

    /// #316 review SHOULD 4 — degree names ("the 3rd ran sharp") are
    /// key-relative claims: they anchor only to an ASSERTED key. A hedged
    /// session reports raw cents statistics with no degree tendencies, so
    /// the recap can't state a coin-flip attribution as fact.
    #[test]
    fn degree_tendencies_require_an_asserted_key() {
        let sharp = |hz: f64| hz * 2f64.powf(20.0 / 1200.0);
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p.pitch_stats.pitches = (0..16).map(|_| sharp(440.0)).collect();
            p
        };

        // Asserted key → tendencies are attributed to degrees.
        let stable = vec![tracked(8, 0.8), tracked(8, 0.8)];
        let summary = aggregate_intonation(&stable).expect("enough notes");
        assert!(
            !summary.tendencies.is_empty(),
            "an asserted key anchors degree tendencies"
        );

        // The #313 contradiction shape → hedged → no degree attribution,
        // but the overall cents statistics still report.
        let hedged = vec![
            tracked(7, 0.8),
            tracked(7, 0.8),
            tracked(8, 0.7),
            tracked(8, 0.7),
        ];
        let summary = aggregate_intonation(&hedged).expect("enough notes");
        assert!(
            summary.tendencies.is_empty(),
            "a hedged key must not attribute tendencies to degrees"
        );
        assert!(summary.note_count >= 12, "raw cents stats still report");
    }

    /// #316 AC2 — a session that wanders through keys and settles only near
    /// the end carries too little mass on the winner to assert: the claim
    /// leans. Fails if the ASSERT_SHARE gate is dropped (everything would
    /// assert) or inverted.
    #[test]
    fn a_late_settling_key_leans_instead_of_asserting() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p
        };
        // Most of the session wandered (D, A, E…), G# established only late:
        // G# wins the vote (1.4 of 3.2) but under the 0.6 assert share.
        let phrases = vec![
            tracked(2, 0.6),
            tracked(9, 0.6),
            tracked(4, 0.6),
            tracked(8, 0.7),
            tracked(8, 0.7),
        ];
        let KeyVerdict::Claimed(key, strength) = aggregate_key(&phrases) else {
            panic!("a key was tracked");
        };
        assert_eq!(key.name(), "G# major");
        assert_eq!(strength, KeyClaimStrength::Leaning, "settled late = hedge");
    }

    /// #316 AC3+AC5 — downstream copy degrades with the claim. An asserted
    /// key keeps today's flat lines; a hedged one renders no "sat firmly"
    /// strength and hedges the long-tones suggestion. The persisted
    /// fingerprint carries the strength so the UI can hedge the same way.
    #[test]
    fn grounded_recap_copy_degrades_with_a_hedged_key() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            // #445-6b: each tracked phrase carries enough played time that
            // every session below clears the thin-session bar.
            p.duration_secs = 7.0;
            p
        };

        // Stable session: G# major throughout → asserted, flat copy.
        let stable = recap_input_with(
            vec![tracked(8, 0.8), tracked(8, 0.8), tracked(8, 0.8)],
            None,
        );
        let recap = grounded_offline_recap(&stable);
        let fp = recap.fingerprint.as_ref().expect("key was measured");
        assert_eq!(fp.key_claim, Some(KeyClaimStrength::Asserted));
        assert!(
            recap.strengths.join(" ").contains("sat firmly in G# major"),
            "an asserted key keeps the tonal-center strength: {:?}",
            recap.strengths
        );
        assert!(
            recap
                .next_session_suggestions
                .join(" ")
                .contains("the key you ended on"),
            "asserted key keeps the flat suggestion: {:?}",
            recap.next_session_suggestions
        );

        // The #313 shape: G dominates, G# holds at the end → leaning, hedged
        // copy.
        let hedged = recap_input_with(
            vec![
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(8, 0.7),
                tracked(8, 0.7),
            ],
            None,
        );
        let recap = grounded_offline_recap(&hedged);
        let fp = recap.fingerprint.as_ref().expect("key was measured");
        assert_eq!(fp.key_claim, Some(KeyClaimStrength::Leaning));
        assert!(
            !recap.strengths.join(" ").contains("sat firmly"),
            "a hedged key must not claim a firm tonal center: {:?}",
            recap.strengths
        );
        let suggestions = recap.next_session_suggestions.join(" ");
        assert!(
            suggestions.contains("leaning toward at the end") && suggestions.contains("G# major"),
            "the suggestion hedges toward the final reading: {suggestions}"
        );

        // #404: G carries the session, a single-phrase G# close knocks the
        // strip off it → drifted. No "sat firmly", and the suggestion anchors
        // to the session — "ended on" or "at the end" would name an
        // end-state the strip contradicted.
        let drifted = recap_input_with(
            vec![
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(8, 0.6),
            ],
            None,
        );
        let recap = grounded_offline_recap(&drifted);
        let fp = recap.fingerprint.as_ref().expect("key was measured");
        assert_eq!(fp.key_claim, Some(KeyClaimStrength::Drifted));
        assert!(
            !recap.strengths.join(" ").contains("sat firmly"),
            "a drifted key must not claim a firm tonal center: {:?}",
            recap.strengths
        );
        let suggestions = recap.next_session_suggestions.join(" ");
        assert!(
            suggestions.contains("the key that carried most of the session")
                && suggestions.contains("G major"),
            "the suggestion anchors a drifted key to the session: {suggestions}"
        );
        assert!(
            !suggestions.contains("ended on") && !suggestions.contains("at the end"),
            "a drifted key must never be phrased as an end-state: {suggestions}"
        );
    }

    /// #316 AC4 — the LLM prompt states the claim honestly at every strength:
    /// hedged keys are marked tentative, and a session that never settled
    /// tells the model NOT to name a key (silence beats fabrication).
    #[test]
    fn recap_prompt_marks_hedged_and_unsettled_keys() {
        let tracked = |tonic: u8, confidence: f32| {
            let mut p = sample_phrase();
            p.key = Some(theory::KeyEstimate {
                tonic,
                mode: theory::Mode::Ionian,
                confidence,
                margin: 0.2,
            });
            p
        };

        // Hedged (#313 shape): the prompt says "leaning" and "tentatively".
        let hedged = recap_input_with(
            vec![
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(8, 0.7),
                tracked(8, 0.7),
            ],
            None,
        );
        let prompt = CoachingEngine::build_recap_user_prompt(&hedged);
        assert!(
            prompt.contains("leaning G# major toward the end") && prompt.contains("tentatively"),
            "hedged key must read as tentative in the prompt: {prompt}"
        );
        assert!(
            !prompt.contains("confidence"),
            "a hedged key must not also print a flat confidence figure"
        );

        // Drifted (#404, the VA's cycling-close shape): the dominant key is
        // named as a whole-session fact, with an explicit instruction never
        // to call it the key at the end — "toward the end" here would
        // contradict the strip the player watched close on other keys.
        let drifted = recap_input_with(
            vec![
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(7, 0.8),
                tracked(8, 0.6), // single-phrase close off the carrier
            ],
            None,
        );
        let prompt = CoachingEngine::build_recap_user_prompt(&drifted);
        assert!(
            prompt.contains("mostly G major")
                && prompt.contains("never that it was the key at the end"),
            "a drifted key must read as a whole-session claim: {prompt}"
        );
        assert!(
            !prompt.contains("toward the end"),
            "a drifted key must not be anchored to the session's close: {prompt}"
        );

        // Unsettled: tonal readings existed but never firmed into a claim →
        // the prompt forbids naming a key instead of staying silent (an
        // absent line lets the model guess).
        let unsettled = recap_input_with(vec![tracked(3, 0.3)], None);
        let prompt = CoachingEngine::build_recap_user_prompt(&unsettled);
        assert!(
            prompt.contains("do not name a key"),
            "an unsettled session must instruct against naming a key: {prompt}"
        );

        // Silent: no tonal readings at all (percussive material) → no key
        // line of any kind; "kept moving" would claim motion nothing saw.
        let silent = recap_input_with(vec![sample_phrase()], None);
        let prompt = CoachingEngine::build_recap_user_prompt(&silent);
        assert!(
            !prompt.contains("Key / mode"),
            "a session with no tonal readings says nothing about key: {prompt}"
        );
    }

    /// #316 AC5 — a hedged key must not drive a mode-named flavour line: the
    /// swing leg still fires (key-free variant), and a legacy fingerprint
    /// (key present, claim absent) keeps its original mode-named line.
    #[test]
    fn a_hedged_key_does_not_drive_a_mode_named_flavour() {
        let mut fp = fp_with(Some((theory::Mode::Dorian, 0.8)), Some(2.0));
        fp.key_claim = Some(KeyClaimStrength::Leaning);
        let line = theory_flavour(&fp).expect("swing leg still produces a line");
        assert!(
            line.contains("jazz-leaning") && !line.contains("Dorian"),
            "hedged mode must degrade to the key-free variant: {line}"
        );

        // #404: a drifted key is hedged the same way — the strip contradicted
        // its end, so a mode-named flavour would state it as fact.
        fp.key_claim = Some(KeyClaimStrength::Drifted);
        let line = theory_flavour(&fp).expect("swing leg still produces a line");
        assert!(
            line.contains("jazz-leaning") && !line.contains("Dorian"),
            "a drifted mode must degrade to the key-free variant: {line}"
        );

        // Legacy blob: key present, claim field absent → original behavior.
        fp.key_claim = None;
        let line = theory_flavour(&fp).expect("legacy flavour");
        assert!(
            line.contains("Dorian"),
            "legacy fingerprints must not retroactively hedge: {line}"
        );
    }

    #[test]
    fn aggregate_tone_means_present_descriptors_or_none() {
        assert!(
            aggregate_tone(&[sample_phrase()]).is_none(),
            "no tone → None"
        );

        let a = PhraseSummary {
            tone: Some(tone::ToneDescriptor {
                brightness: 0.2,
                ..sample_tone()
            }),
            ..sample_phrase()
        };
        let b = PhraseSummary {
            tone: Some(tone::ToneDescriptor {
                brightness: 0.8,
                ..sample_tone()
            }),
            ..sample_phrase()
        };
        let agg = aggregate_tone(&[a, b]).expect("two toned phrases");
        assert!((agg.brightness - 0.5).abs() < 1e-6, "brightness averaged");
    }

    #[test]
    fn aggregate_intonation_reports_only_with_enough_notes() {
        // No phrases → no intonation.
        assert!(aggregate_intonation(&[]).is_none());

        // Too few notes to trust → hedge to None (gate boundary: < 12 notes).
        let mut thin = sample_phrase();
        thin.pitch_stats.pitches = vec![440.0; 11];
        assert!(
            aggregate_intonation(std::slice::from_ref(&thin)).is_none(),
            "11 notes is below the gate → None"
        );

        // At the gate (12 notes), a perfectly-tuned A4 train reports a summary
        // centered near zero cents.
        let mut ok = sample_phrase();
        ok.pitch_stats.pitches = vec![440.0; 12];
        let summary =
            aggregate_intonation(std::slice::from_ref(&ok)).expect("12 in-tune notes report");
        assert_eq!(summary.note_count, 12);
        assert!(
            summary.mean_abs_cents < 1.0,
            "in-tune A4 should be near 0 cents, got {}",
            summary.mean_abs_cents
        );
    }

    #[test]
    fn aggregate_intonation_attributes_degree_tendencies_with_a_key() {
        // A C-major scale with the 3rd (E) consistently sharp. With a confident
        // key, the summary should carry per-degree tendencies including the
        // sharp major 3rd.
        let mut p = sample_phrase();
        // The degree anchor comes from the live-tracked session key (#277).
        p.key = Some(theory::KeyEstimate {
            tonic: 0,
            mode: theory::Mode::Ionian,
            confidence: 0.8,
            margin: 0.3,
        });
        // E4 is 329.63 Hz; push it ~20 cents sharp.
        let e_sharp = 329.63_f64 * 2f64.powf(20.0 / 1200.0);
        p.pitch_stats.pitches = vec![
            261.63, 261.63, 261.63, // C ×3 (tonic)
            293.66, // D
            e_sharp, e_sharp, e_sharp, // E ×3, sharp
            349.23,  // F
            392.0, 392.0, // G ×2
            440.0, 493.88, // A B
        ];
        // Two phrases in the same key: degree anchoring requires an ASSERTED
        // key (#316), and a single tracked reading is never asserted.
        let phrases = vec![p.clone(), p];
        let summary = aggregate_intonation(&phrases).expect("enough notes to report");
        let third = summary
            .tendencies
            .iter()
            .find(|t| t.semitones_from_tonic == 4)
            .expect("a major-3rd tendency once a key anchors the degrees");
        assert!(
            third.mean_cents > 10.0,
            "the sharp 3rd should read clearly sharp, got {}",
            third.mean_cents
        );
    }

    #[test]
    fn aggregate_groove_reports_only_with_enough_onsets() {
        // No phrases / no onsets → None.
        assert!(aggregate_groove(&[]).is_none());
        assert!(
            aggregate_groove(&[sample_phrase()]).is_none(),
            "a phrase with no retained onsets → None"
        );

        // Gate boundary: 5 onsets is below MIN_ONSETS (6) → None.
        let mut thin = sample_phrase();
        thin.onsets_secs = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        assert!(
            aggregate_groove(std::slice::from_ref(&thin)).is_none(),
            "5 onsets is below the gate → None"
        );

        // 6 steady onsets at 120 BPM → a groove descriptor with ~120 BPM and
        // high timing consistency.
        let mut ok = sample_phrase();
        ok.onsets_secs = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5];
        let g = aggregate_groove(std::slice::from_ref(&ok)).expect("6 onsets report");
        assert_eq!(g.onset_count, 6);
        let bpm = g.tempo_bpm.expect("tempo from steady onsets");
        assert!((bpm - 120.0).abs() < 1.0, "expected ~120 BPM, got {bpm}");
        assert!(g.timing_consistency > 0.99, "steady train is consistent");
    }

    #[test]
    fn aggregate_groove_spans_phrases() {
        // Onsets are retained per phrase; the session groove must pool them.
        let mut a = sample_phrase();
        a.onsets_secs = vec![0.0, 0.5, 1.0];
        let mut b = sample_phrase();
        b.onsets_secs = vec![1.5, 2.0, 2.5];
        let g = aggregate_groove(&[a, b]).expect("pooled onsets clear the gate");
        assert_eq!(g.onset_count, 6, "onsets from both phrases are pooled");
    }

    #[test]
    fn recap_prompt_includes_intonation_and_groove_when_present() {
        // A free-play session with enough notes and onsets to clear both gates
        // should surface grounded intonation and feel lines in the prompt, and
        // carry the aggregates on the recap for persistence.
        let mut p = sample_phrase();
        p.pitch_stats.pitches = vec![440.0; 16];
        p.onsets_secs = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5];
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            prompt.contains("Intonation:"),
            "recap prompt names intonation"
        );
        assert!(prompt.contains("Feel:"), "recap prompt names rhythmic feel");

        let recap = CoachingEngine::fallback_recap(&input);
        let fingerprint = recap
            .fingerprint
            .as_ref()
            .expect("recap carries a fingerprint when dimensions were measured");
        assert!(
            fingerprint.intonation.is_some(),
            "recap fingerprint carries the intonation aggregate"
        );
        assert!(
            fingerprint.groove.is_some(),
            "recap fingerprint carries the groove aggregate"
        );
    }

    fn sample_idiom_match() -> crate::idiom_recap::IdiomMatch {
        crate::idiom_recap::IdiomMatch {
            label: "Bebop line".to_owned(),
            genre: "jazz".to_owned(),
            exemplar_artist: "Charlie Parker".to_owned(),
            era: "1940s-50s".to_owned(),
            similarity: 0.78,
        }
    }

    // -----------------------------------------------------------------------
    // Cross-genre contextual coaching (Phase 4)
    // -----------------------------------------------------------------------

    /// A phrase with enough notes + onsets to clear the fingerprint gates, so
    /// `connections_gate_open` sees real measured signal.
    fn groundable_phrase() -> PhraseSummary {
        let mut p = sample_phrase();
        p.pitch_stats.pitches = vec![440.0; 16];
        p.onsets_secs = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5];
        p
    }

    fn sample_taste_profile() -> crate::store::TasteProfile {
        crate::store::TasteProfile {
            genres: vec!["hip-hop".to_owned(), "gospel".to_owned()],
            artists: vec!["D'Angelo".to_owned()],
            goals: vec!["play in church band".to_owned()],
            experience: crate::store::ExperienceLevel::Intermediate,
            is_under_13: false,
        }
    }

    fn recap_input_with(
        phrases: Vec<PhraseSummary>,
        taste_profile: Option<crate::store::TasteProfile>,
    ) -> RecapInput {
        RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases,
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        }
    }

    #[test]
    fn recap_prompt_includes_idiom_block_as_grounded_input_when_present() {
        // A session with a gated idiom match should surface it in the prompt as
        // GROUNDED INPUT the model may hedge around — never as a hard fact.
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: vec![sample_idiom_match()],
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            prompt.contains("GROUNDED INPUT"),
            "idiom block must be marked as grounded input, got:\n{prompt}"
        );
        assert!(
            prompt.contains("Bebop line") && prompt.contains("Charlie Parker"),
            "idiom block must name the matched idiom + exemplar, got:\n{prompt}"
        );
        assert!(
            prompt.contains("NEVER invent"),
            "idiom block must forbid the model inventing idioms, got:\n{prompt}"
        );
    }

    #[test]
    fn recap_prompt_injects_taste_profile_as_context_when_present() {
        // Profile present + groundable signal → the prompt carries the
        // student's genres/artists/goals as framing context, and instructs the
        // model to add hedged, style-level connections without fabricating.
        let input = recap_input_with(vec![groundable_phrase()], Some(sample_taste_profile()));
        let user = CoachingEngine::build_recap_user_prompt(&input);
        assert!(user.contains("hip-hop"), "genres appear as context");
        assert!(user.contains("D'Angelo"), "artists appear as context");
        assert!(
            user.contains("play in church band"),
            "goals appear as context"
        );
        assert!(
            user.contains("hedged connections") || user.contains("connections to their world"),
            "user prompt nudges for hedged connections, got:\n{user}"
        );

        // The system prompt, with connections enabled, must carry the
        // anti-hallucination grounding contract.
        let system = CoachingEngine::build_recap_system_prompt(true, "");
        assert!(
            system.contains("GROUNDING CONTRACT"),
            "system prompt states the grounding contract"
        );
        assert!(
            system.contains("Do NOT invent track names"),
            "system prompt forbids inventing track names/quotes"
        );
        assert!(
            system.contains("HEDGE"),
            "system prompt instructs the model to hedge"
        );
        assert!(
            system.contains("\"connections\""),
            "system prompt's JSON schema includes the connections field"
        );
    }

    #[test]
    fn recap_prompt_omits_idiom_block_when_silent() {
        // No gated matches → the recap stays silent on idiom (no block).
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            !prompt.contains("Idiom proximity"),
            "no idiom block when nothing cleared the gate, got:\n{prompt}"
        );
    }

    // -----------------------------------------------------------------
    // #453 S2: history-grounded suggestions in the recap.
    // -----------------------------------------------------------------

    fn history_suggestion(text: &str, evidence: &str) -> crate::insights::PracticeSuggestion {
        crate::insights::PracticeSuggestion {
            kind: crate::insights::SuggestionKind::Trend,
            text: text.to_owned(),
            evidence: evidence.to_owned(),
        }
    }

    /// Three settled phrases — clears the #445-6b thin bar so these tests
    /// pin the FULL recap's history behavior.
    fn settled_sample_phrases() -> Vec<PhraseSummary> {
        (0..3)
            .map(|_| {
                let mut p = sample_phrase();
                p.duration_secs = 7.0;
                p
            })
            .collect()
    }

    /// #453 S2 AC3: the LLM user prompt carries history suggestions as
    /// GROUNDED INPUT — marked, text AND evidence present, further invention
    /// forbidden — and no block at all when the history earned nothing.
    /// Fails if the block is dropped, unmarked, loses its citation, or
    /// starts rendering on empty history.
    #[test]
    fn recap_prompt_carries_history_as_grounded_input() {
        let mut input = recap_input_with(vec![sample_phrase()], None);
        input.history_suggestions = vec![history_suggestion(
            "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
            "key_mastery 3:major: 6 attempts, accuracy EWMA 0.54, last attempt 2d ago",
        )];
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            prompt.contains("Practice-history suggestions") && prompt.contains("GROUNDED INPUT"),
            "history rides in as marked grounded input, got:\n{prompt}"
        );
        assert!(
            prompt.contains("54% over 6 attempts") && prompt.contains("key_mastery 3:major"),
            "text AND evidence travel into the prompt, got:\n{prompt}"
        );
        assert!(
            prompt.contains("NEVER invent further history claims"),
            "the prompt forbids inventing history, got:\n{prompt}"
        );

        // Silence: no suggestions → the block is absent entirely.
        let quiet = recap_input_with(vec![sample_phrase()], None);
        assert!(
            !CoachingEngine::build_recap_user_prompt(&quiet).contains("Practice-history"),
            "no history block when the analyzer said nothing"
        );
    }

    /// #453 S2 AC1: the offline full recap appends exactly ONE history line
    /// — the FIRST by the analyzer's pinned order, verbatim (citation
    /// numbers intact) — and adds nothing when history is silent. Fails if
    /// a second suggestion leaks in, the text gets rephrased, or the append
    /// point disappears.
    #[test]
    fn offline_recap_appends_at_most_one_history_suggestion() {
        let mut input = recap_input_with(settled_sample_phrases(), None);
        input.history_suggestions = vec![
            history_suggestion(
                "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
                "e1",
            ),
            history_suggestion("SECOND — must never surface.", "e2"),
        ];
        let recap = grounded_offline_recap(&input);
        let tail = recap
            .next_session_suggestions
            .last()
            .expect("a full recap always suggests something");
        assert!(
            tail.contains("54% over 6 attempts (last one 2 days ago)"),
            "the first suggestion's text ships verbatim as the tail line: {tail}"
        );
        assert!(
            !recap
                .next_session_suggestions
                .iter()
                .any(|s| s.contains("SECOND")),
            "AT MOST ONE — the second pinned suggestion never surfaces: {:?}",
            recap.next_session_suggestions
        );

        // No history → exactly the measured suggestions, one fewer line.
        let quiet = recap_input_with(settled_sample_phrases(), None);
        let base = grounded_offline_recap(&quiet);
        assert_eq!(
            base.next_session_suggestions.len() + 1,
            recap.next_session_suggestions.len(),
            "history adds exactly one line to the same session"
        );
    }

    /// #453 S2 AC2: a thin session keeps its single #445-6b suggestion —
    /// history NEVER stacks onto the short form (the thin gate fires before
    /// the append). Fails if the append moves above the thin early-return.
    #[test]
    fn thin_recap_gains_no_history_suggestion() {
        // One 1.5s phrase = thin (below both #445-6b bars).
        let mut input = recap_input_with(vec![sample_phrase()], None);
        input.history_suggestions = vec![history_suggestion(
            "Your Eb major rows are sitting at 54% over 6 attempts (last one 2 days ago) — worth a slow pass.",
            "e1",
        )];
        assert!(is_thin_session(&input), "fixture must actually be thin");
        let recap = grounded_offline_recap(&input);
        assert_eq!(
            recap.next_session_suggestions.len(),
            1,
            "the thin short form keeps its single suggestion: {:?}",
            recap.next_session_suggestions
        );
        assert!(
            !recap.next_session_suggestions[0].contains("Eb major"),
            "no history line on a thin recap: {}",
            recap.next_session_suggestions[0]
        );
    }

    // -----------------------------------------------------------------
    // #454 S3: the method-book tip in the recap.
    // -----------------------------------------------------------------

    /// A resolved tip in the shape the command layer threads in — a
    /// Schlossberg-style paraphrase-only entry, so the assertions pin the
    /// attribution machinery, not the shipped corpus prose.
    fn sample_method_book_tip() -> crate::pedagogy::PedagogyEntry {
        crate::pedagogy::PedagogyEntry {
            id: "brass-schlossberg-long-tones".to_owned(),
            family: crate::pedagogy::Family::Brass,
            topic: "Long tones and pitch stability".to_owned(),
            source: crate::pedagogy::SourceRef {
                title: "Daily Drills and Technical Studies".to_owned(),
                author: "Max Schlossberg".to_owned(),
                year: 1937,
                status: crate::pedagogy::SourceStatus::ParaphraseOnly,
                section: "Long-tone drills (opening section)".to_owned(),
            },
            guidance: "There are drills for exactly this in Schlossberg's Daily Drills — \
                       start the note softly, let it grow, and keep the pitch absolutely level."
                .to_owned(),
            quote: None,
            note: None,
            triggers: vec!["pitch-sag-sustain".to_owned()],
        }
    }

    /// #454 S3 AC1: the offline full recap appends exactly ONE method-book
    /// line — the guidance verbatim with the formal attribution in the copy
    /// — to `areas_to_improve` (the deepened-diagnosis home; history keeps
    /// `next_session_suggestions`), and adds nothing without a tip. Fails
    /// if the append point disappears, the attribution is dropped, the line
    /// lands in the wrong list, or a tipless recap grows a book line.
    #[test]
    fn offline_recap_appends_attributed_method_book_line() {
        let mut input = recap_input_with(settled_sample_phrases(), None);
        input.method_book_tip = Some(sample_method_book_tip());
        let recap = grounded_offline_recap(&input);
        let tail = recap
            .areas_to_improve
            .last()
            .expect("a full recap always has an area line");
        assert!(
            tail.contains("keep the pitch absolutely level"),
            "the guidance ships verbatim: {tail}"
        );
        assert!(
            tail.contains("(Max Schlossberg, Daily Drills and Technical Studies)"),
            "the attribution is IN the copy — non-negotiable (#454): {tail}"
        );
        assert_eq!(
            recap
                .areas_to_improve
                .iter()
                .filter(|a| a.contains("Schlossberg"))
                .count(),
            1,
            "AT MOST ONE book line: {:?}",
            recap.areas_to_improve
        );

        // No tip → the same session's areas are exactly one line shorter,
        // and the suggestions list (the history voice's home) is untouched.
        let quiet = recap_input_with(settled_sample_phrases(), None);
        let base = grounded_offline_recap(&quiet);
        assert_eq!(
            base.areas_to_improve.len() + 1,
            recap.areas_to_improve.len(),
            "the tip adds exactly one area line"
        );
        assert_eq!(
            base.next_session_suggestions, recap.next_session_suggestions,
            "the book tip never leaks into next_session_suggestions"
        );
    }

    /// #454 S3 AC2: a thin session's short form gains NO book line even when
    /// the command layer resolved a tip — the thin gate fires before any
    /// weaving (#445-6b). Fails if the append moves above the thin
    /// early-return.
    #[test]
    fn thin_recap_gains_no_method_book_line() {
        // One 1.5s phrase = thin (below both #445-6b bars).
        let mut input = recap_input_with(vec![sample_phrase()], None);
        input.method_book_tip = Some(sample_method_book_tip());
        assert!(is_thin_session(&input), "fixture must actually be thin");
        let recap = grounded_offline_recap(&input);
        let rendered = serde_json::to_string(&recap).expect("recap serializes");
        assert!(
            !rendered.contains("Schlossberg"),
            "no book line anywhere on a thin recap: {rendered}"
        );
    }

    /// #454 S3 AC3: the LLM user prompt carries the tip as GROUNDED INPUT —
    /// marked, guidance AND attribution present, invention of further book
    /// content forbidden — and no block at all without a tip. Fails if the
    /// block is dropped, unmarked, loses the attribution, or renders on a
    /// tipless input.
    #[test]
    fn recap_prompt_carries_method_book_tip_as_grounded_input() {
        let mut input = recap_input_with(vec![sample_phrase()], None);
        input.method_book_tip = Some(sample_method_book_tip());
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            prompt.contains("Method-book guidance") && prompt.contains("GROUNDED INPUT"),
            "the tip rides in as marked grounded input, got:\n{prompt}"
        );
        assert!(
            prompt.contains("Max Schlossberg, Daily Drills and Technical Studies")
                && prompt.contains("keep the pitch absolutely level"),
            "attribution AND guidance travel into the prompt, got:\n{prompt}"
        );
        assert!(
            prompt.contains("NEVER invent further book claims")
                && prompt.contains("NEVER cite any book not named here"),
            "the prompt forbids inventing book content or citations, got:\n{prompt}"
        );

        // Silence: no tip → the block is absent entirely.
        let quiet = recap_input_with(vec![sample_phrase()], None);
        assert!(
            !CoachingEngine::build_recap_user_prompt(&quiet).contains("Method-book"),
            "no pedagogy block when no evidence bar was crossed"
        );
    }

    #[test]
    fn recap_prompt_omits_taste_and_connections_when_no_profile() {
        // Cold start: groundable signal but NO profile → no taste context, no
        // connection nudge, and the system prompt is the existing one (no
        // connections field, no grounding contract).
        let input = recap_input_with(vec![groundable_phrase()], None);
        let user = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            !user.contains("the music they love"),
            "no taste context block at cold start, got:\n{user}"
        );
        assert!(
            !user.contains("connections to their world"),
            "no connection nudge at cold start"
        );

        let system = CoachingEngine::build_recap_system_prompt(false, "");
        assert!(
            !system.contains("GROUNDING CONTRACT"),
            "no grounding contract when connections are disabled"
        );
        assert!(
            !system.contains("\"connections\""),
            "no connections field in the JSON schema when disabled"
        );
        // Still the warm, no-grades recap prompt.
        assert!(system.contains("NEVER give letter grades"));
    }

    #[test]
    fn connections_gate_closed_on_thin_signal_even_with_profile() {
        // Profile present but the session is too thin to measure anything
        // (a couple of repeated notes, no onsets) → every fingerprint gate
        // fails, the gate closes, and there is no forced cross-genre reference.
        // Silence over a hollow connection.
        let mut thin = sample_phrase();
        thin.pitch_stats.pitches = vec![440.0; 2];
        thin.onsets_secs = Vec::new();
        thin.tone = None;
        let input = recap_input_with(vec![thin], Some(sample_taste_profile()));
        assert!(
            !connections_gate_open(&input),
            "thin signal must close the connections gate"
        );
        let user = CoachingEngine::build_recap_user_prompt(&input);
        // Taste context may still appear (it's harmless framing), but the
        // connection nudge must not — there's nothing to ground.
        assert!(
            !user.contains("connections to their world"),
            "thin signal must not nudge for connections, got:\n{user}"
        );
    }

    #[test]
    fn fallback_recap_carries_idiom_notes() {
        // The offline matches stand alone — they survive an LLM failure into the
        // fallback recap so the grounded "reminds me of" note still shows.
        // #445-6b: three settled phrases clear the thin-session bar — the
        // thin recap deliberately drops idiom notes, and this test pins the
        // FULL fallback.
        let mut p = sample_phrase();
        p.duration_secs = 7.0;
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p; 3],
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: vec![sample_idiom_match()],
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };
        let recap = CoachingEngine::fallback_recap(&input);
        assert_eq!(
            recap.idiom_notes,
            vec![sample_idiom_match()],
            "fallback recap must carry the gated idiom matches verbatim"
        );
    }

    #[test]
    fn connections_gate_closed_on_empty_profile() {
        // A default (empty) profile is treated as cold start, even though it's
        // technically `Some` — there's nothing to frame with.
        let empty = crate::store::TasteProfile::default();
        let input = recap_input_with(vec![groundable_phrase()], Some(empty));
        assert!(
            !connections_gate_open(&input),
            "an empty profile must not open the connections gate"
        );
    }

    #[tokio::test]
    async fn parse_recap_surfaces_connections_only_when_gate_open() {
        // The model returned a grounded connection. With a profile + signal,
        // the parsed recap carries it; with no profile, it's dropped.
        // #445-6b: three settled groundable phrases (7s each, onsets kept
        // monotone) clear the thin-session bar so the engine actually calls
        // the model.
        let settled = || {
            (0..3)
                .map(|i| {
                    let mut p = groundable_phrase();
                    p.phrase_index = i;
                    p.duration_secs = 7.0;
                    p.onsets_secs = p.onsets_secs.iter().map(|t| t + i as f64 * 4.0).collect();
                    p
                })
                .collect::<Vec<_>>()
        };
        let recap_content = serde_json::json!({
            "overall_assessment": "Lovely, grounded session.",
            "strengths": ["Steady time"],
            "areas_to_improve": ["Open the sound up top"],
            "next_session_suggestions": ["Long tones"],
            "connections": [
                "the way you're laying back on the beat reminds me of the pocket in a lot of the hip-hop you love",
                "   "
            ]
        });
        let response = serde_json::json!({
            "content": [{ "type": "text", "text": recap_content.to_string() }]
        })
        .to_string();

        let engine_with = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(MockHttpClient::succeeding(&response)),
        );

        // Gate open: profile + groundable signal.
        let with = engine_with
            .generate_recap(&recap_input_with(settled(), Some(sample_taste_profile())))
            .await
            .unwrap();
        assert_eq!(
            with.connections.len(),
            1,
            "exactly the one non-blank connection is surfaced (blank dropped), got {:?}",
            with.connections
        );
        assert!(with.connections[0].contains("laying back on the beat"));

        // Gate closed (no profile): the model's connections are discarded, the
        // data model stays honest about when a connection was grounded.
        let engine_without = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(MockHttpClient::succeeding(&response)),
        );
        let without = engine_without
            .generate_recap(&recap_input_with(settled(), None))
            .await
            .unwrap();
        assert!(
            without.connections.is_empty(),
            "no profile → connections dropped, got {:?}",
            without.connections
        );
    }

    #[test]
    fn fallback_recap_never_carries_connections() {
        // The offline fallback never reached the model, so it must never
        // fabricate a connection — even with a profile and signal.
        let input = recap_input_with(vec![groundable_phrase()], Some(sample_taste_profile()));
        let recap = CoachingEngine::fallback_recap(&input);
        assert!(
            recap.connections.is_empty(),
            "offline fallback must carry no connections"
        );
    }

    fn mock_anthropic_response() -> String {
        serde_json::json!({
            "content": [{
                "type": "text",
                "text": "{\"text\": \"Nice tone on that passage! Try letting the phrase breathe a little more at the end.\", \"severity\": \"suggestion\", \"category\": \"expression\"}"
            }]
        })
        .to_string()
    }

    fn make_engine(mock: MockHttpClient) -> CoachingEngine {
        let config = CoachingConfig {
            api_key: "test-key-12345".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            rate_limit_secs: 3.0,
        };
        online_engine(config, Box::new(mock))
    }

    /// Construct an engine and opt it into [`NetworkPolicy::Online`], the way
    /// the command layer does when the user has enabled coaching. Engines
    /// default to `Offline` (offline by default), so any test that exercises
    /// the *network* path must opt in explicitly — exactly as production does.
    fn online_engine(config: CoachingConfig, client: Box<dyn HttpClient>) -> CoachingEngine {
        let mut engine = CoachingEngine::new(config, client).unwrap();
        engine.set_network_policy(NetworkPolicy::Online);
        engine
    }

    /// A mock that **panics** if any HTTP method is invoked. Used by the
    /// airplane-switch tests to prove that an `Offline` engine is structurally
    /// incapable of an outbound call: if the policy ever leaks, the test fails
    /// loudly instead of silently passing.
    struct PanickingHttpClient;

    #[async_trait::async_trait]
    impl HttpClient for PanickingHttpClient {
        async fn post_json(
            &self,
            _url: &str,
            _body: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, CoachingError> {
            panic!(
                "HttpClient::post_json was called while NetworkPolicy::Offline — \
                 the airplane switch must prevent every outbound call"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn config_reads_api_key_from_field() {
        let config = CoachingConfig {
            api_key: "explicit-key-abc".to_owned(),
            model: "gpt-4".to_owned(),
            rate_limit_secs: 5.0,
        };
        let mock = MockHttpClient::succeeding("{}");
        let engine = CoachingEngine::new(config, Box::new(mock)).unwrap();

        assert_eq!(engine.resolved_api_key, "explicit-key-abc");
        assert_eq!(engine.resolved_model, "gpt-4");
    }

    #[tokio::test]
    async fn get_tip_returns_coaching_tip_from_mock() {
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let mut engine = make_engine(mock);

        let tip = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap()
            .expect("a successful API call yields Some(tip)");

        assert_eq!(tip.severity, CoachingSeverity::Suggestion);
        assert_eq!(tip.category, CoachingCategory::Expression);
        assert!(
            tip.text.contains("phrase breathe"),
            "Expected LLM tip text, got: {}",
            tip.text
        );
    }

    #[tokio::test]
    async fn get_tip_returns_none_on_api_failure() {
        // Silence beats a lie: on API failure the live-tip path returns NO tip
        // (None), never canned encouragement.
        let mock = MockHttpClient::failing("connection refused");
        let mut engine = make_engine(mock);

        let result = engine.get_tip(&sample_phrase(), &sample_context()).await;

        assert!(result.is_ok(), "API failure should not surface as Err");
        assert!(
            result.unwrap().is_none(),
            "API failure must yield no tip, not a fabricated one"
        );
    }

    /// An Anthropic-shaped response whose text is `{"why": <why>}` (what the S2
    /// reveal-enrichment prompt asks the model to return).
    fn reveal_response(why: &str) -> String {
        let inner = serde_json::json!({ "why": why }).to_string();
        serde_json::json!({ "content": [{ "type": "text", "text": inner }] }).to_string()
    }

    // #253 S2 AC2: online, a valid `{"why": ...}` yields that rewritten line.
    #[tokio::test]
    async fn enrich_reveal_why_online_returns_why() {
        let mock = MockHttpClient::succeeding(&reveal_response(
            "It is the cool, modal-jazz minor Miles made famous.",
        ));
        let mut engine = make_engine(mock);
        engine.set_network_policy(NetworkPolicy::Online);
        let why = engine
            .enrich_reveal_why("G Dorian", "Miles Davis — \"So What\"", "curated line")
            .await;
        assert_eq!(
            why.as_deref(),
            Some("It is the cool, modal-jazz minor Miles made famous.")
        );
    }

    // #253 S2 AC1: offline makes NO outbound call (airplane switch) → None.
    #[tokio::test]
    async fn enrich_reveal_why_offline_makes_no_call() {
        let mock = MockHttpClient::succeeding(&reveal_response("should never be seen"));
        let call_count = Arc::clone(&mock.call_count);
        let mut engine = make_engine(mock);
        engine.set_network_policy(NetworkPolicy::Offline);
        let why = engine
            .enrich_reveal_why("G Dorian", "Miles Davis", "curated")
            .await;
        assert!(why.is_none(), "offline must not enrich");
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            0,
            "offline must make no outbound call"
        );
    }

    // #253 S2 AC3: an API error (or unparseable body) falls back to None.
    #[tokio::test]
    async fn enrich_reveal_why_api_error_returns_none() {
        let mock = MockHttpClient::failing("connection refused");
        let mut engine = make_engine(mock);
        engine.set_network_policy(NetworkPolicy::Online);
        assert!(engine
            .enrich_reveal_why("G Dorian", "Miles Davis", "curated")
            .await
            .is_none());
    }

    // An empty rewrite is treated as failure — keep the curated line.
    #[tokio::test]
    async fn enrich_reveal_why_empty_rewrite_returns_none() {
        let mock = MockHttpClient::succeeding(&reveal_response("   "));
        let mut engine = make_engine(mock);
        engine.set_network_policy(NetworkPolicy::Online);
        assert!(engine
            .enrich_reveal_why("G Dorian", "Miles Davis", "curated")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn rate_limiter_skips_call_and_returns_none_when_too_soon() {
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let call_count = Arc::clone(&mock.call_count);
        let mut engine = make_engine(mock);

        // First call — should hit the API and produce a real tip.
        let tip1 = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();
        assert!(tip1.is_some(), "first call returns a genuine tip");

        // Second call immediately — should be rate-limited and return no tip.
        let tip2 = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "Rate limiter should have prevented the second API call"
        );
        assert!(
            tip2.is_none(),
            "rate-limited live tip must be silent, not a canned filler"
        );
    }

    #[test]
    fn system_prompt_contains_no_grades_instruction() {
        let prompt = CoachingEngine::build_system_prompt();

        assert!(
            prompt.contains("NEVER give letter grades"),
            "System prompt must forbid letter grades"
        );
        assert!(
            prompt.contains("NEVER say things like"),
            "System prompt must forbid percentage scores"
        );
        assert!(
            prompt.contains("encouraging"),
            "System prompt must emphasize encouragement"
        );
        assert!(
            prompt.contains("ONE actionable improvement"),
            "System prompt must enforce single-focus tips"
        );
    }

    #[test]
    fn session_context_influences_prompt() {
        let phrase = sample_phrase();
        let context = SessionContext {
            instrument: "violin".to_owned(),
            session_duration_secs: 300.0,
            phrases_played: 10,
            previous_tips: vec![
                "Work on bow control near the frog.".to_owned(),
                "Your vibrato is coming along nicely.".to_owned(),
            ],
            score_title: None,
        };

        let prompt = CoachingEngine::build_user_prompt(&phrase, &context);

        assert!(
            prompt.contains("violin"),
            "User prompt must include the instrument name"
        );
        assert!(
            prompt.contains("300"),
            "User prompt must include session duration"
        );
        assert!(
            prompt.contains("bow control"),
            "User prompt must include previous tips to avoid repetition"
        );
        assert!(
            prompt.contains("vibrato"),
            "User prompt must include all previous tips"
        );
    }

    #[test]
    fn user_prompt_names_the_score_in_score_mode() {
        let mut context = sample_context();
        context.score_title = Some("Bach Cello Suite No. 1".to_owned());

        let prompt = CoachingEngine::build_user_prompt(&sample_phrase(), &context);

        assert!(
            prompt.contains("Bach Cello Suite No. 1"),
            "score-mode live tip prompt should name the piece, got:\n{prompt}"
        );
    }

    #[test]
    fn user_prompt_omits_score_line_in_free_play() {
        // sample_context() has score_title: None.
        let prompt = CoachingEngine::build_user_prompt(&sample_phrase(), &sample_context());

        assert!(
            !prompt.contains("The student is playing"),
            "free-play live tip prompt must not mention a piece, got:\n{prompt}"
        );
    }

    #[test]
    fn all_severity_levels_are_serializable() {
        let severities = [
            CoachingSeverity::Encouragement,
            CoachingSeverity::Suggestion,
            CoachingSeverity::Focus,
        ];

        for severity in &severities {
            let json = serde_json::to_string(severity).unwrap();
            let roundtripped: CoachingSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(
                *severity, roundtripped,
                "Severity {json} did not roundtrip correctly"
            );
        }

        // Verify the specific JSON representations
        assert_eq!(
            serde_json::to_string(&CoachingSeverity::Encouragement).unwrap(),
            "\"encouragement\""
        );
        assert_eq!(
            serde_json::to_string(&CoachingSeverity::Suggestion).unwrap(),
            "\"suggestion\""
        );
        assert_eq!(
            serde_json::to_string(&CoachingSeverity::Focus).unwrap(),
            "\"focus\""
        );
    }

    #[test]
    fn all_categories_are_serializable() {
        let categories = [
            CoachingCategory::Tone,
            CoachingCategory::Intonation,
            CoachingCategory::Rhythm,
            CoachingCategory::Dynamics,
            CoachingCategory::Expression,
            CoachingCategory::Technique,
        ];

        for category in &categories {
            let json = serde_json::to_string(category).unwrap();
            let roundtripped: CoachingCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(
                *category, roundtripped,
                "Category {json} did not roundtrip correctly"
            );
        }

        // Verify the specific JSON representations
        assert_eq!(
            serde_json::to_string(&CoachingCategory::Tone).unwrap(),
            "\"tone\""
        );
        assert_eq!(
            serde_json::to_string(&CoachingCategory::Technique).unwrap(),
            "\"technique\""
        );
    }

    #[tokio::test]
    async fn openai_response_format_is_parsed_correctly() {
        let openai_response = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "{\"text\": \"Your dynamics are lovely here.\", \"severity\": \"encouragement\", \"category\": \"dynamics\"}"
                }
            }]
        })
        .to_string();

        let mock = MockHttpClient::succeeding(&openai_response);
        let config = CoachingConfig {
            api_key: "test-key".to_owned(),
            model: "gpt-4".to_owned(),
            rate_limit_secs: 0.0,
        };
        let mut engine = online_engine(config, Box::new(mock));

        let tip = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap()
            .expect("a successful OpenAI response yields Some(tip)");

        assert_eq!(tip.category, CoachingCategory::Dynamics);
        assert_eq!(tip.severity, CoachingSeverity::Encouragement);
    }

    #[tokio::test]
    async fn malformed_llm_response_returns_none() {
        // The LLM returns text that isn't valid tip JSON. Rather than fabricate
        // a canned tip, the live-tip path stays silent (None).
        let bad_response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "Sorry, I can't help with that."
            }]
        })
        .to_string();

        let mock = MockHttpClient::succeeding(&bad_response);
        let mut engine = make_engine(mock);

        let tip = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        // No parseable tip → no tip, not a fabricated one.
        assert!(
            tip.is_none(),
            "an unparseable response must yield no tip, got {tip:?}"
        );
    }

    #[test]
    fn missing_api_key_returns_error() {
        // Use with_env + explicit None so we never touch process env.
        // This is the deterministic, parallel-safe equivalent of the old test
        // that called env::remove_var — that approach would race against
        // other tests reading MUSIC_COMPANION_LLM_API_KEY.
        let config = CoachingConfig {
            api_key: String::new(),
            model: "claude-opus-4-8".to_owned(),
            rate_limit_secs: 3.0,
        };
        let mock = MockHttpClient::succeeding("{}");
        let result = CoachingEngine::with_env(config, Box::new(mock), None, None);

        // CoachingEngine doesn't impl Debug (it holds a `Box<dyn HttpClient>`
        // which can't require Debug without poisoning the trait object), so we
        // pattern-match instead of calling `.unwrap_err()`.
        match result {
            Err(CoachingError::MissingApiKey) => {}
            Err(other) => panic!("Expected MissingApiKey, got: {other}"),
            Ok(_) => panic!("Expected MissingApiKey error, got Ok"),
        }
    }

    #[test]
    fn resolve_api_key_prefers_config_over_env() {
        let result = resolve_api_key("config-key", Some("env-key")).unwrap();
        assert_eq!(result, "config-key");
    }

    #[test]
    fn resolve_api_key_falls_back_to_env_when_config_empty() {
        let result = resolve_api_key("", Some("env-key")).unwrap();
        assert_eq!(result, "env-key");
    }

    #[test]
    fn resolve_api_key_errors_when_both_empty() {
        let err = resolve_api_key("", None).unwrap_err();
        assert!(matches!(err, CoachingError::MissingApiKey));

        // Empty string in env counts as missing, not a valid key
        let err = resolve_api_key("", Some("")).unwrap_err();
        assert!(matches!(err, CoachingError::MissingApiKey));
    }

    #[test]
    fn resolve_model_prefers_config_then_env_then_default() {
        // Config wins
        assert_eq!(resolve_model("gpt-4", Some("claude-3-opus")), "gpt-4");
        // Env wins when config empty
        assert_eq!(resolve_model("", Some("claude-3-opus")), "claude-3-opus");
        // Default when both empty
        assert_eq!(resolve_model("", None), "claude-opus-4-8");
        assert_eq!(resolve_model("", Some("")), "claude-opus-4-8");
    }

    #[test]
    fn with_env_uses_explicit_env_values() {
        let config = CoachingConfig {
            api_key: String::new(),
            model: String::new(),
            rate_limit_secs: 3.0,
        };
        let mock = MockHttpClient::succeeding("{}");
        let engine = CoachingEngine::with_env(
            config,
            Box::new(mock),
            Some("injected-key"),
            Some("gpt-4-turbo"),
        )
        .unwrap();
        assert_eq!(engine.resolved_api_key, "injected-key");
        assert_eq!(engine.resolved_model, "gpt-4-turbo");
    }

    #[test]
    fn claude_model_uses_anthropic_url() {
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let engine = make_engine(mock);

        let url = engine.api_url();
        assert!(
            url.contains("anthropic.com"),
            "Claude model should use Anthropic API, got: {url}"
        );
    }

    #[test]
    fn gpt_model_uses_openai_url() {
        let config = CoachingConfig {
            api_key: "test-key".to_owned(),
            model: "gpt-4".to_owned(),
            rate_limit_secs: 3.0,
        };
        let mock = MockHttpClient::succeeding("{}");
        let engine = CoachingEngine::new(config, Box::new(mock)).unwrap();

        let url = engine.api_url();
        assert!(
            url.contains("openai.com"),
            "GPT model should use OpenAI API, got: {url}"
        );
    }

    #[test]
    fn user_prompt_includes_phrase_data() {
        let user_prompt = CoachingEngine::build_user_prompt(&sample_phrase(), &sample_context());
        assert!(
            user_prompt.contains("440.0"),
            "User prompt should include mean pitch"
        );
        assert!(
            user_prompt.contains("trumpet"),
            "User prompt should include instrument"
        );
        assert!(
            user_prompt.contains("0.92"),
            "User prompt should include stability"
        );
        assert!(
            user_prompt.contains("12"),
            "User prompt should include note count"
        );
    }

    #[test]
    fn trumpet_uses_brass_specific_prompt() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("trumpet");
        assert!(
            prompt.contains("embouchure"),
            "Trumpet prompt should mention embouchure"
        );
        assert!(
            prompt.contains("breath support"),
            "Trumpet prompt should mention breath support"
        );
        assert!(
            prompt.contains("resonance"),
            "Trumpet prompt should mention resonance"
        );
        assert!(
            prompt.contains("tonguing"),
            "Trumpet prompt should mention tonguing/articulation"
        );
    }

    #[test]
    fn voice_uses_vocal_specific_prompt() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("Voice");
        assert!(
            prompt.contains("breath management"),
            "Voice prompt should mention breath management"
        );
        assert!(
            prompt.contains("resonance"),
            "Voice prompt should mention resonance"
        );
        assert!(
            prompt.contains("projection"),
            "Voice prompt should mention projection"
        );
        assert!(
            prompt.contains("vibrato"),
            "Voice prompt should mention vibrato control"
        );
    }

    #[test]
    fn violin_uses_string_specific_prompt() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("Violin");
        assert!(
            prompt.contains("bow control"),
            "String prompt should mention bow control"
        );
        assert!(
            prompt.contains("intonation"),
            "String prompt should mention intonation"
        );
        assert!(
            prompt.contains("vibrato"),
            "String prompt should mention vibrato quality"
        );
        assert!(
            prompt.contains("position shift"),
            "String prompt should mention position shifts"
        );
    }

    #[test]
    fn flute_uses_woodwind_specific_prompt() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("flute");
        assert!(
            prompt.contains("embouchure"),
            "Woodwind prompt should mention embouchure"
        );
        assert!(
            prompt.contains("articulation"),
            "Woodwind prompt should mention articulation clarity"
        );
        assert!(
            prompt.contains("register"),
            "Woodwind prompt should mention register transitions"
        );
        // "air stream" appears only in the woodwind guidance and "tonguing"
        // only in brass, so a rerouted match arm can't hide behind vocabulary
        // ("embouchure", "articulation", "register") the two families share.
        assert!(
            prompt.contains("air stream"),
            "Woodwind prompt should use woodwind-specific language (air stream)"
        );
        assert!(
            !prompt.contains("tonguing"),
            "Woodwind prompt must not carry the brass guidance"
        );
    }

    #[test]
    fn piano_uses_keyboard_specific_prompt() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("piano");
        assert!(
            prompt.contains("hand position"),
            "Piano prompt should mention hand position"
        );
        assert!(
            prompt.contains("pedal"),
            "Piano prompt should mention pedal timing"
        );
        assert!(
            prompt.contains("voicing"),
            "Piano prompt should mention voicing"
        );
    }

    #[test]
    fn unknown_instrument_falls_back_to_generic() {
        let prompt = CoachingEngine::build_system_prompt_for_instrument("theremin");
        let generic = CoachingEngine::build_system_prompt();
        assert_eq!(
            prompt, generic,
            "Unknown instrument should use generic prompt"
        );
    }

    #[test]
    fn tip_system_prompts_carry_the_json_output_contract() {
        // `parse_tip_from_response` deserializes the model's text into
        // `CoachingTip`, so every tip system prompt must demand JSON with the
        // exact severity/category vocabularies the serde enums accept. If this
        // block is dropped or drifts, tips silently stop parsing.
        for (label, prompt) in [
            ("generic", CoachingEngine::build_system_prompt()),
            (
                "instrument",
                CoachingEngine::build_system_prompt_for_instrument("trumpet"),
            ),
        ] {
            assert!(
                prompt.contains("Respond with valid JSON in this exact format"),
                "{label} prompt must demand JSON output"
            );
            assert!(
                prompt.contains("\"severity\": \"encouragement\" | \"suggestion\" | \"focus\""),
                "{label} prompt must enumerate the CoachingSeverity variants"
            );
            assert!(
                prompt.contains(
                    "\"category\": \"tone\" | \"intonation\" | \"rhythm\" | \"dynamics\" | \
                     \"expression\" | \"technique\""
                ),
                "{label} prompt must enumerate the CoachingCategory variants"
            );
        }
    }

    #[test]
    fn all_instrument_prompts_forbid_grading() {
        let instruments = vec!["trumpet", "Voice", "violin", "Flute", "piano", "Saxophone"];
        for instrument in instruments {
            let prompt = CoachingEngine::build_system_prompt_for_instrument(instrument);
            assert!(
                prompt.contains("NEVER give letter grades"),
                "Prompt for {} must forbid letter grades",
                instrument
            );
            assert!(
                prompt.contains("NEVER say things like"),
                "Prompt for {} must forbid percentage scores",
                instrument
            );
        }
    }

    #[tokio::test]
    async fn rate_limiting_blocks_rapid_calls() {
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let call_count = mock.call_count.clone();
        let mut engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 10.0,
            },
            Box::new(mock),
        );

        let phrase = sample_phrase();
        let context = sample_context();

        let _tip1 = engine.get_tip(&phrase, &context).await.unwrap();
        let initial_calls = call_count.load(Ordering::Acquire);

        let tip2 = engine.get_tip(&phrase, &context).await.unwrap();
        let after_rapid_calls = call_count.load(Ordering::Acquire);

        assert_eq!(
            initial_calls, after_rapid_calls,
            "Rate limiting should prevent second API call within window"
        );
        assert!(
            tip2.is_none(),
            "rate-limited live tip must be silent, not a canned filler"
        );
    }

    #[tokio::test]
    async fn api_failure_returns_none_tip() {
        // Silence beats a lie: the live-tip path returns no tip on API failure.
        let mock = MockHttpClient::failing("Service unavailable");
        let mut engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        let phrase = sample_phrase();
        let context = sample_context();

        let result = engine.get_tip(&phrase, &context).await;
        assert!(result.is_ok(), "API failure should not propagate error");
        assert!(
            result.unwrap().is_none(),
            "API failure must yield no tip, not a fabricated one"
        );
    }

    #[tokio::test]
    async fn malformed_response_returns_none() {
        let mock = MockHttpClient::succeeding("{\"invalid\": \"json\"}");
        let mut engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        let phrase = sample_phrase();
        let context = sample_context();

        let result = engine.get_tip(&phrase, &context).await;
        assert!(
            result.is_ok(),
            "Malformed response should not propagate error"
        );
        assert!(
            result.unwrap().is_none(),
            "an unparseable response must yield no tip"
        );
    }

    // -----------------------------------------------------------------------
    // Airplane-switch tests (NetworkPolicy)
    //
    // These prove the hard, Rust-core guarantee: when the policy is Offline,
    // NO HttpClient method is ever called, and the on-device fallback is
    // returned. The mock client panics if hit, so a policy leak fails loudly.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // #445-6b — the thin-session recap: words scale to evidence
    // -----------------------------------------------------------------------

    /// A phrase with a chosen played duration (clear pitch so the
    /// fingerprint CAN gate in when the test wants it to).
    fn phrase_lasting(secs: f64) -> PhraseSummary {
        let mut p = groundable_phrase();
        p.duration_secs = secs;
        p
    }

    /// AC1: two phrases — however long — earn the short form: brief
    /// assessment, NO strengths/areas padding, exactly one suggestion.
    #[test]
    fn a_two_phrase_session_gets_the_short_form() {
        let input = recap_input_with(vec![phrase_lasting(30.0), phrase_lasting(30.0)], None);
        assert!(is_thin_session(&input));
        let recap = grounded_offline_recap(&input);
        assert!(
            recap.overall_assessment.starts_with("A quick touch"),
            "got: {}",
            recap.overall_assessment
        );
        assert!(
            recap
                .overall_assessment
                .contains("about 60 seconds of actual playing"),
            "the copy speaks the played clock, not the wall clock: {}",
            recap.overall_assessment
        );
        assert!(recap.strengths.is_empty(), "no padding lists");
        assert!(recap.areas_to_improve.is_empty());
        assert_eq!(recap.next_session_suggestions.len(), 1);
        assert_eq!(recap.phrase_count, 2);
        assert!(recap.flavour.is_none() && recap.connections.is_empty());
    }

    /// AC2: many tiny phrases under 20s of playing are thin; three
    /// phrases at ≥20s cross the bar and keep the FULL recap (boundary).
    #[test]
    fn the_thin_bar_is_count_and_played_seconds() {
        let confetti = recap_input_with(vec![phrase_lasting(2.0); 6], None);
        assert!(is_thin_session(&confetti), "12s of confetti is thin");
        assert!(grounded_offline_recap(&confetti)
            .overall_assessment
            .starts_with("A quick touch"));

        let settled = recap_input_with(vec![phrase_lasting(7.0); 3], None);
        assert!(!is_thin_session(&settled), "3 phrases × 7s clears the bar");
        assert!(
            grounded_offline_recap(&settled)
                .overall_assessment
                .starts_with("You practiced for about"),
            "at the bar the full recap speaks"
        );
    }

    /// AC3: ZERO phrases is not thin — the empty-state path (the voice
    /// the founder praised) is byte-identical to before this change.
    #[test]
    fn an_empty_session_keeps_the_empty_state_path() {
        let input = recap_input_with(Vec::new(), None);
        assert!(!is_thin_session(&input));
        let recap = grounded_offline_recap(&input);
        assert!(
            recap
                .overall_assessment
                .starts_with("You practiced for about"),
            "the 0-phrase recap still opens with the session frame"
        );
    }

    /// AC4: the ONLINE engine never lets the model inflate a thin
    /// session — the panicking client proves no HTTP happens.
    #[tokio::test]
    async fn a_thin_session_never_reaches_the_model() {
        let engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(PanickingHttpClient),
        );
        let input = recap_input_with(vec![phrase_lasting(30.0)], None);
        let recap = engine.generate_recap(&input).await.unwrap();
        assert!(recap.overall_assessment.starts_with("A quick touch"));
        assert_eq!(recap.next_session_suggestions.len(), 1);
    }

    /// AC5: a measured fact that cleared its gate rides along even in
    /// the short form — only padding is suppressed, never truth.
    #[test]
    fn a_thin_session_still_states_a_cleared_fact() {
        // One long, clearly-pitched phrase: thin by count, but the
        // intonation gate has 16 in-tune pitches to read.
        let input = recap_input_with(vec![phrase_lasting(30.0)], None);
        let recap = grounded_offline_recap(&input);
        assert!(
            recap.overall_assessment.contains("landed in tune"),
            "the cleared intonation fact speaks: {}",
            recap.overall_assessment
        );
        assert!(
            recap
                .overall_assessment
                .contains("Not enough playing for a full read"),
            "a read fingerprint gets the non-contradicting line: {}",
            recap.overall_assessment
        );
        // And on a fixed-pitch instrument the same fact stays QUIET
        // (#389: that measures the instrument, not the player).
        let mut piano = recap_input_with(vec![phrase_lasting(30.0)], None);
        piano.instrument_family = "Keyboard".to_owned();
        assert!(!grounded_offline_recap(&piano)
            .overall_assessment
            .contains("landed in tune"));
    }

    /// #445-6b review MF2: a score session where the follower JUDGED
    /// notes is NEVER thin — measured accuracy has its own denominator,
    /// and the #337 S4 panel must not vanish behind a phrase count.
    #[test]
    fn a_judged_score_session_is_never_thin() {
        let mut input = recap_input_with(vec![phrase_lasting(5.0)], None);
        input.score_title = Some("Für Elise".to_owned());
        input.note_verdicts = vec![crate::follower::NoteVerdict {
            measure_number: 1,
            beat: 0.0,
            verdict: crate::follower::Verdict::Hit,
        }];
        assert!(!is_thin_session(&input), "judged notes exempt the bar");
        let recap = grounded_offline_recap(&input);
        assert!(
            recap
                .overall_assessment
                .starts_with("You practiced for about"),
            "the full recap speaks: {}",
            recap.overall_assessment
        );
        assert!(
            recap.score_summary.is_some(),
            "the judged-note accuracy panel survives"
        );
    }

    /// #445-6b review SF5: the FULL path's empty-fingerprint degradation
    /// stayed covered when the quiet-session test moved to the thin
    /// contract — a settled session (3×7s) that produced no readable
    /// signal claims no numbers and still encourages.
    #[test]
    fn a_settled_but_silent_session_still_degrades_gracefully() {
        let silent = || {
            let mut p = phrase_lasting(7.0);
            p.pitch_stats.pitches = Vec::new();
            p.onsets_secs = Vec::new();
            p
        };
        let input = recap_input_with(vec![silent(), silent(), silent()], None);
        assert!(!is_thin_session(&input), "3×7s clears the bar");
        let recap = grounded_offline_recap(&input);
        assert!(recap
            .overall_assessment
            .starts_with("You practiced for about"));
        assert!(
            recap
                .overall_assessment
                .contains("didn't capture enough clear signal"),
            "the honest no-signal line: {}",
            recap.overall_assessment
        );
        assert!(recap.fingerprint.is_none(), "no fabricated fingerprint");
        assert!(
            !recap.strengths.is_empty(),
            "default encouragement survives on the full path"
        );
        assert!(!recap.next_session_suggestions.is_empty());
    }

    #[test]
    fn engine_defaults_to_offline() {
        // Offline by default — the internet is never required. A freshly
        // constructed engine must not be in a network-permitting state.
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(PanickingHttpClient),
        )
        .unwrap();
        assert_eq!(engine.network_policy(), NetworkPolicy::Offline);
        assert!(!engine.network_policy().allows_network());
    }

    #[test]
    fn network_policy_from_opt_in_maps_correctly() {
        assert_eq!(NetworkPolicy::from_opt_in(true), NetworkPolicy::Online);
        assert_eq!(NetworkPolicy::from_opt_in(false), NetworkPolicy::Offline);
        assert!(NetworkPolicy::Online.allows_network());
        assert!(!NetworkPolicy::Offline.allows_network());
    }

    #[tokio::test]
    async fn offline_get_tip_never_calls_http_client_and_returns_none() {
        // The mock panics if `post_json` is ever reached. An Offline engine
        // must return NO tip (silence) without touching the client — it never
        // fabricates canned encouragement to fill the gap.
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(PanickingHttpClient),
        )
        .unwrap();
        // Default is already Offline, but set it explicitly to document intent.
        engine.set_network_policy(NetworkPolicy::Offline);

        let tip = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .expect("offline tip must succeed without network");

        assert!(
            tip.is_none(),
            "offline live tip must be silent (None), never a canned fallback"
        );
    }

    #[tokio::test]
    async fn offline_generate_recap_never_calls_http_client_and_returns_grounded_fallback() {
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(PanickingHttpClient),
        )
        .unwrap();
        assert_eq!(engine.network_policy(), NetworkPolicy::Offline);

        // A session with enough measured signal to ground intonation + groove.
        let mut p = sample_phrase();
        p.pitch_stats.pitches = vec![440.0; 16];
        p.onsets_secs = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5];
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        // Must NOT panic (no HttpClient call) and must return the fallback.
        let recap = engine
            .generate_recap(&input)
            .await
            .expect("offline recap must succeed without network");

        assert_eq!(recap.instrument, "trumpet");
        assert!(!recap.overall_assessment.is_empty());
        // Grounding intact: the offline fallback still carries the measured
        // fingerprint (intonation + groove cleared their gates).
        let fingerprint = recap
            .fingerprint
            .as_ref()
            .expect("offline fallback still carries the measured fingerprint");
        assert!(
            fingerprint.intonation.is_some(),
            "offline recap preserves grounded intonation"
        );
        assert!(
            fingerprint.groove.is_some(),
            "offline recap preserves grounded groove"
        );
        // Offline never fabricates cross-genre connections.
        assert!(
            recap.connections.is_empty(),
            "offline recap must not carry LLM connections"
        );
    }

    #[tokio::test]
    async fn online_get_tip_does_call_http_client() {
        // Counterpart to the offline test: with Online policy the recording
        // mock IS hit, proving the policy is what gates the call (not an
        // unrelated short-circuit).
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let call_count = Arc::clone(&mock.call_count);
        let mut engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        let _ = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "online policy must permit exactly one outbound call"
        );
    }

    // -----------------------------------------------------------------------
    // RecapGenerator tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn generate_recap_parses_anthropic_response() {
        use crate::session::RecapInput;

        let recap_content = serde_json::json!({
            "overall_assessment": "Strong session with good focus.",
            "strengths": ["Consistent tone", "Great rhythm"],
            "areas_to_improve": ["High register confidence", "Dynamic control"],
            "next_session_suggestions": ["Work on high passages", "Focus on dynamics"]
        });

        let anthropic_response = serde_json::json!({
            "content": [{
                "type": "text",
                "text": recap_content.to_string()
            }]
        });

        let mock = MockHttpClient::succeeding(&anthropic_response.to_string());
        let engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        // #445-6b: three settled phrases clear the thin-session bar so the
        // mocked response actually reaches the parser.
        let mut p = sample_phrase();
        p.duration_secs = 7.0;
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 1800.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p; 3],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "trumpet");
        assert_eq!(recap.duration_secs, 1800.0);
        assert_eq!(recap.phrase_count, 3);
        assert_eq!(recap.strengths.len(), 2);
        assert_eq!(recap.areas_to_improve.len(), 2);
        assert_eq!(recap.next_session_suggestions.len(), 2);
        assert!(recap.overall_assessment.contains("Strong"));
    }

    #[tokio::test]
    async fn generate_recap_parses_openai_response() {
        use crate::session::RecapInput;

        let recap_content = serde_json::json!({
            "overall_assessment": "Great progress today!",
            "strengths": ["Good intonation"],
            "areas_to_improve": ["Articulation clarity"],
            "next_session_suggestions": ["Focus on tonguing"]
        });

        let openai_response = serde_json::json!({
            "choices": [{
                "message": {
                    "content": recap_content.to_string()
                }
            }]
        });

        let mock = MockHttpClient::succeeding(&openai_response.to_string());
        let engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "gpt-4".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        // #445-6b: 7s per phrase clears the thin-session bar (3 × 1.5s did
        // not), so the mocked response reaches the parser.
        let mut p = sample_phrase();
        p.duration_secs = 7.0;
        let input = RecapInput {
            instrument: "violin".to_owned(),
            instrument_family: String::new(),
            duration_secs: 2400.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p; 3],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "violin");
        assert_eq!(recap.phrase_count, 3);
        assert!(recap.overall_assessment.contains("progress"));
    }

    #[tokio::test]
    async fn generate_recap_gracefully_handles_api_failure() {
        use crate::session::RecapInput;

        let mock = MockHttpClient::failing("Service unavailable");
        let engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        // #445-6b: 7s per phrase clears the thin-session bar so the failure
        // path exercises the FULL fallback recap (thin has empty strengths).
        let mut p = sample_phrase();
        p.duration_secs = 7.0;
        let input = RecapInput {
            instrument: "voice".to_owned(),
            instrument_family: String::new(),
            duration_secs: 1500.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p; 5],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "voice");
        assert_eq!(recap.phrase_count, 5);
        assert!(recap.duration_secs > 0.0);
        assert!(!recap.overall_assessment.is_empty());
        assert!(!recap.strengths.is_empty());
    }

    /// #449 T1 AC10: `recap_used_llm` is true ONLY after a network response
    /// actually parsed into the recap. Offline policy resets a previous
    /// `true` (the flag speaks for the LAST call), and an API failure's
    /// fallback reports false — so the practice_events journal can never
    /// claim a narration that didn't fire.
    #[tokio::test]
    async fn recap_used_llm_true_only_after_parsed_network_recap() {
        use crate::session::RecapInput;

        let full_input = || {
            // 7 s per phrase clears the thin-session bar so the mocked
            // response actually reaches the parser.
            let mut p = sample_phrase();
            p.duration_secs = 7.0;
            RecapInput {
                instrument: "trumpet".to_owned(),
                instrument_family: String::new(),
                duration_secs: 1800.0,
                practice_mode: crate::session::PracticeMode::default(),
                phrases: vec![p; 3],
                tips: vec![],
                score_title: None,
                note_verdicts: Vec::new(),
                idiom_notes: Vec::new(),
                taste_profile: None,
                history_suggestions: Vec::new(),
                method_book_tip: None,
            }
        };
        let config = || CoachingConfig {
            api_key: "test".to_owned(),
            model: "claude-opus-4-8".to_owned(),
            rate_limit_secs: 0.0,
        };

        let recap_content = serde_json::json!({
            "overall_assessment": "Strong session.",
            "strengths": ["Tone"],
            "areas_to_improve": ["Time"],
            "next_session_suggestions": ["Drone work"]
        });
        let anthropic_response = serde_json::json!({
            "content": [{ "type": "text", "text": recap_content.to_string() }]
        });
        let mut engine = online_engine(
            config(),
            Box::new(MockHttpClient::succeeding(&anthropic_response.to_string())),
        );
        assert!(
            !engine.recap_used_llm(),
            "a fresh engine has narrated nothing"
        );
        engine.generate_recap(&full_input()).await.unwrap();
        assert!(
            engine.recap_used_llm(),
            "a parsed network recap IS a fired narration"
        );

        // The flag speaks for the LAST call: going offline (the airplane
        // switch mid-app-life) resets it on the next recap.
        engine.set_network_policy(NetworkPolicy::Offline);
        engine.generate_recap(&full_input()).await.unwrap();
        assert!(
            !engine.recap_used_llm(),
            "an offline fallback recap must not inherit the previous call's flag"
        );

        // API failure → the fallback recap is served → no narration fired.
        let failing = online_engine(
            config(),
            Box::new(MockHttpClient::failing("Service unavailable")),
        );
        failing.generate_recap(&full_input()).await.unwrap();
        assert!(
            !failing.recap_used_llm(),
            "a failure fallback is not a narration"
        );
    }

    #[tokio::test]
    async fn generate_recap_handles_malformed_response() {
        use crate::session::RecapInput;

        let malformed = r#"{"invalid": "json", "structure": true}"#;
        let anthropic_response = format!(
            r#"{{"content": [{{"type": "text", "text": "{}"}}]}}"#,
            malformed.replace('"', "\\\"")
        );

        let mock = MockHttpClient::succeeding(&anthropic_response);
        let engine = online_engine(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-opus-4-8".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        );

        let input = RecapInput {
            instrument: "piano".to_owned(),
            instrument_family: String::new(),
            duration_secs: 3600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase(); 2],
            tips: vec![],
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "piano");
        assert!(!recap.overall_assessment.is_empty());
        assert!(recap.strengths.is_empty() || !recap.strengths[0].is_empty());
    }

    // -----------------------------------------------------------------------
    // grounded_offline_recap — the no-key / network-failure offline path
    // -----------------------------------------------------------------------

    /// A phrase whose pitches and onsets are caller-controlled, so a test can
    /// produce a deliberately distinct fingerprint (key / intonation / groove).
    fn phrase_from(pitches: Vec<f64>, onsets_secs: Vec<f64>) -> PhraseSummary {
        let mut p = sample_phrase();
        p.note_count = pitches.len();
        p.pitch_stats.pitches = pitches;
        p.onsets_secs = onsets_secs;
        p
    }

    fn offline_input(phrases: Vec<PhraseSummary>) -> RecapInput {
        RecapInput {
            instrument: "trumpet".to_owned(),
            instrument_family: String::new(),
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases,
            tips: Vec::new(),
            score_title: None,
            note_verdicts: Vec::new(),
            idiom_notes: Vec::new(),
            taste_profile: None,
            history_suggestions: Vec::new(),
            method_book_tip: None,
        }
    }

    /// #445-6b: three copies of the phrase (7 s each, onsets shifted by
    /// `phrase_shift_secs` per copy so the pulse stays monotone and even) —
    /// the same fingerprint content as one phrase, but enough phrases and
    /// summed seconds to clear the thin-session bar and exercise the FULL
    /// recap path.
    fn settled_phrases(
        pitches: Vec<f64>,
        onsets_secs: Vec<f64>,
        phrase_shift_secs: f64,
    ) -> Vec<PhraseSummary> {
        (0..3)
            .map(|i| {
                let mut p = phrase_from(
                    pitches.clone(),
                    onsets_secs
                        .iter()
                        .map(|t| t + i as f64 * phrase_shift_secs)
                        .collect(),
                );
                p.phrase_index = i;
                p.duration_secs = 7.0;
                p
            })
            .collect()
    }

    /// Core regression: two sessions with different intonation/groove/key
    /// fingerprints must produce *different* recap prose. This is what proves
    /// the offline recap is no longer canned.
    #[test]
    fn grounded_offline_recap_differs_across_distinct_fingerprints() {
        // Session A: C-major content, dead-on 440-region tuning, slow steady
        // pulse (~120 BPM).
        let a = offline_input(settled_phrases(
            vec![
                261.63, 293.66, 329.63, 349.23, 392.00, 440.00, 493.88, 261.63, 329.63, 392.00,
                440.00, 261.63,
            ],
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            4.0,
        ));

        // Session B: D-major content, audibly sharp tuning, faster pulse
        // (~150 BPM). Different key, different intonation, different groove.
        let sharp = |hz: f64| hz * 2f64.powf(30.0 / 1200.0); // +30 cents
        let b = offline_input(settled_phrases(
            vec![
                sharp(293.66),
                sharp(329.63),
                sharp(369.99),
                sharp(440.00),
                sharp(493.88),
                sharp(277.18),
                sharp(293.66),
                sharp(369.99),
                sharp(440.00),
                sharp(293.66),
                sharp(369.99),
                sharp(440.00),
            ],
            vec![0.0, 0.4, 0.8, 1.2, 1.6, 2.0, 2.4, 2.8],
            3.2,
        ));

        let recap_a = grounded_offline_recap(&a);
        let recap_b = grounded_offline_recap(&b);

        assert_ne!(
            recap_a.overall_assessment, recap_b.overall_assessment,
            "distinct fingerprints must yield distinct overall prose"
        );
        assert!(
            recap_a.strengths != recap_b.strengths
                || recap_a.areas_to_improve != recap_b.areas_to_improve
                || recap_a.next_session_suggestions != recap_b.next_session_suggestions,
            "distinct fingerprints must yield distinct strengths/areas/suggestions"
        );
    }

    // ── theory_flavour: mode + swing → hedged flavour, or silence ───────────

    /// Build a fingerprint carrying only a key (at `confidence`) and/or a swing
    /// ratio — the two signals `theory_flavour` consults. `None` for either
    /// arg omits that dimension.
    fn fp_with(mode_conf: Option<(theory::Mode, f32)>, swing: Option<f32>) -> MusicalFingerprint {
        MusicalFingerprint {
            tone: None,
            key: mode_conf.map(|(mode, confidence)| theory::KeyEstimate {
                tonic: 2, // D — the tonic now names the mode in the line ("D Dorian").
                mode,
                confidence,
                margin: 0.1,
            }),
            key_claim: mode_conf.map(|_| KeyClaimStrength::Asserted),
            intonation: None,
            groove: swing.map(|swing_ratio| groove::GrooveDescriptor {
                tempo_bpm: Some(120.0),
                swing_ratio: Some(swing_ratio),
                mean_ioi_secs: 0.5,
                timing_consistency: 0.9,
                onset_count: 16,
            }),
        }
    }

    /// #277: the flavour line embeds THIS session's measured specifics (mode
    /// name, swing, tempo), so different sessions can't produce the same
    /// word-for-word line — it visibly reacts to what was played. Fails if the
    /// strings regress to fixed copy.
    #[test]
    fn theory_flavour_varies_with_the_measured_session() {
        let a = theory_flavour(&fp_with(Some((theory::Mode::Dorian, 0.8)), Some(1.8)))
            .expect("flavour for a swung Dorian session");
        let b = theory_flavour(&fp_with(Some((theory::Mode::Mixolydian, 0.8)), Some(1.5)))
            .expect("flavour for a swung Mixolydian session");
        assert_ne!(a, b, "different sessions must read differently");
        assert!(a.contains("Dorian"), "names the tracked mode, got: {a}");
        assert!(a.contains("1.8"), "carries the measured swing, got: {a}");
        assert!(a.contains("120"), "carries the measured tempo, got: {a}");

        // The bebop (swung-diatonic) arm must carry the measured pulse too —
        // otherwise every swung-diatonic session reads word-for-word the same,
        // the exact #277 complaint.
        let c = theory_flavour(&fp_with(None, Some(1.8))).expect("swung, no trusted mode");
        let d = theory_flavour(&fp_with(None, Some(1.5))).expect("swung, no trusted mode");
        assert_ne!(c, d, "bebop arm must vary with the measured swing");
        assert!(c.contains("1.8"), "got: {c}");

        // The straight modal-colour arm names tempo but never a swing ratio.
        let e = theory_flavour(&fp_with(Some((theory::Mode::Lydian, 0.8)), Some(1.0)))
            .expect("straight modal");
        assert!(e.contains("120"), "modal colour carries tempo, got: {e}");
        assert!(
            !e.contains(":1"),
            "no swing ratio on a straight verdict, got: {e}"
        );
    }

    #[test]
    fn theory_flavour_swung_modal_is_modal_jazz() {
        // G-Dorian swung scat — the motivating case (was "Chopin").
        let f = theory_flavour(&fp_with(Some((theory::Mode::Dorian, 0.8)), Some(1.8)))
            .expect("swung + modal → Some");
        assert!(f.contains("modal-jazz"), "got: {f}");
        assert!(f.contains("Miles Davis"), "names an exemplar, got: {f}");
        // Hedged, never asserted as fact.
        assert!(f.contains("feel"), "must be hedged, got: {f}");
    }

    #[test]
    fn theory_flavour_swung_diatonic_is_jazz_leaning() {
        let f = theory_flavour(&fp_with(Some((theory::Mode::Ionian, 0.8)), Some(1.8)))
            .expect("swung + diatonic → Some");
        assert!(f.contains("jazz-leaning"), "got: {f}");
        assert!(f.contains("feel"), "must be hedged, got: {f}");
    }

    #[test]
    fn theory_flavour_straight_diatonic_is_none() {
        // Plain major + straight feel ⇒ no distinctive signal → silence.
        assert!(theory_flavour(&fp_with(Some((theory::Mode::Ionian, 0.8)), Some(1.0))).is_none());
    }

    #[test]
    fn theory_flavour_straight_modal_is_modal_colour() {
        let f = theory_flavour(&fp_with(Some((theory::Mode::Mixolydian, 0.8)), Some(1.0)))
            .expect("straight + modal → Some");
        assert!(f.contains("modal colour"), "got: {f}");
    }

    #[test]
    fn theory_flavour_low_confidence_key_is_not_trusted() {
        // Modal but below the 0.5 confidence gate, straight feel → no signal.
        assert!(theory_flavour(&fp_with(Some((theory::Mode::Dorian, 0.3)), Some(1.0))).is_none());
        // Even swung, a low-confidence modal key falls back to the jazz-leaning
        // (mode untrusted) line, not the modal-jazz one.
        let f = theory_flavour(&fp_with(Some((theory::Mode::Dorian, 0.3)), Some(1.8)))
            .expect("swung with untrusted mode → jazz-leaning");
        assert!(f.contains("jazz-leaning"), "got: {f}");
        assert!(!f.contains("modal-jazz"), "must not claim modal, got: {f}");
    }

    #[test]
    fn theory_flavour_ambiguous_swing_band_is_silent() {
        // Swing ratio in the [1.25, 1.4) dead band, plain major → no verdict.
        assert!(theory_flavour(&fp_with(Some((theory::Mode::Ionian, 0.8)), Some(1.3))).is_none());
    }

    #[test]
    fn theory_flavour_no_signal_is_none() {
        assert!(theory_flavour(&fp_with(None, None)).is_none());
    }

    #[test]
    fn theory_flavour_modal_only_no_swing_is_modal_colour() {
        // A trusted modal key with no usable swing read still earns a hedged
        // modal colour (key and groove are independent gates).
        let f = theory_flavour(&fp_with(Some((theory::Mode::Lydian, 0.8)), None))
            .expect("modal key, no swing → Some");
        assert!(f.contains("modal colour"), "got: {f}");
    }

    /// #337 S4 AC5: with scripted errors in measures 3 and 7, the summary
    /// names exactly those measures worst-first and the accuracy matches
    /// the scripted hit rate. Empty input → None (silence > lies).
    #[test]
    fn score_summary_ranks_worst_measures_and_reports_honest_accuracy() {
        use crate::follower::{NoteVerdict, Verdict};
        let v = |m: usize, verdict: Verdict| NoteVerdict {
            measure_number: m,
            beat: 0.0,
            verdict,
        };
        let verdicts = vec![
            v(1, Verdict::Hit),
            v(1, Verdict::Hit),
            v(2, Verdict::Hit),
            v(3, Verdict::Missed),
            v(3, Verdict::Near),
            v(4, Verdict::Hit),
            v(5, Verdict::Hit),
            v(7, Verdict::Missed),
            v(7, Verdict::Missed),
            v(8, Verdict::Hit),
        ];
        let s = score_practice_summary("Haydn", &verdicts).expect("judged notes");
        assert_eq!(s.judged, 10);
        assert!(
            (s.accuracy_pct - 60.0).abs() < 0.01,
            "6/10 = 60%: {}",
            s.accuracy_pct
        );
        let worst: Vec<usize> = s.worst_measures.iter().map(|m| m.measure_number).collect();
        assert_eq!(worst, vec![7, 3], "worst-first, clean measures omitted");
        assert_eq!(s.worst_measures[0].missed, 2);

        assert!(
            score_practice_summary("Haydn", &[]).is_none(),
            "nothing judged says nothing"
        );
    }

    /// #337 S4: the exercise log's score entries group by piece in insights.
    #[test]
    fn insights_shape_names_score_practice() {
        let shape = crate::insights::exercise_insights(&[crate::store::ExerciseLogEntry {
            source: "score_practice".to_owned(),
            label: "Haydn".to_owned(),
            spec_json: r#"{"score_title":"Haydn"}"#.to_owned(),
            seed: 0,
            difficulty: 0,
            tonic: 0,
            accuracy: Some(0.6),
        }]);
        assert_eq!(shape[0].shape, "score: Haydn");
    }

    /// The offline recap must persist the measured fingerprint, not throw it
    /// away (`fingerprint: None` was the original bug).
    #[test]
    fn grounded_offline_recap_populates_fingerprint() {
        let input = offline_input(vec![phrase_from(
            vec![440.0; 16],
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
        )]);
        let recap = grounded_offline_recap(&input);
        let fp = recap
            .fingerprint
            .as_ref()
            .expect("offline recap must carry the measured fingerprint");
        assert!(
            fp.intonation.is_some() || fp.groove.is_some(),
            "a session with clear pitch + onsets must surface at least one dimension"
        );
    }

    /// The grounded prose must reflect the measured numbers: a sharp session
    /// reads as sharp and names a cents figure somewhere.
    #[test]
    fn grounded_offline_recap_reflects_measured_intonation() {
        let sharp = |hz: f64| hz * 2f64.powf(25.0 / 1200.0);
        let input = offline_input(vec![phrase_from(
            (0..16).map(|_| sharp(440.0)).collect(),
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
        )]);
        let recap = grounded_offline_recap(&input);
        let prose = format!(
            "{} {} {} {}",
            recap.overall_assessment,
            recap.strengths.join(" "),
            recap.areas_to_improve.join(" "),
            recap.next_session_suggestions.join(" "),
        );
        assert!(
            prose.contains("cents") || prose.contains("sharp") || prose.contains("tune"),
            "grounded prose must surface the measured intonation, got: {prose}"
        );
    }

    /// A quiet/empty session (no clear pitch, no onsets) must degrade
    /// gracefully: no fingerprint, and no fabricated numeric claims.
    /// #445-6b: a single near-silent phrase is by design a THIN session now,
    /// so the graceful degradation is the thin recap — short assessment, no
    /// strengths/areas padding, exactly one suggestion.
    #[test]
    fn grounded_offline_recap_quiet_session_degrades_gracefully() {
        // Empty pitch + no onsets → no dimension clears its gate.
        let input = offline_input(vec![phrase_from(Vec::new(), Vec::new())]);
        let recap = grounded_offline_recap(&input);

        assert!(
            recap.fingerprint.is_none(),
            "a quiet session measured nothing, so it must carry no fingerprint"
        );
        let prose = format!(
            "{} {} {} {}",
            recap.overall_assessment,
            recap.strengths.join(" "),
            recap.areas_to_improve.join(" "),
            recap.next_session_suggestions.join(" "),
        );
        // No fabricated measurements: no cents figures, no BPM, no in-tune
        // percentages invented out of an empty session.
        assert!(
            !prose.contains("cents") && !prose.contains("BPM") && !prose.contains('%'),
            "a quiet session must not fabricate numeric claims, got: {prose}"
        );
        // Still honest — and, per the #445-6b thin contract, short: a plain
        // "quick touch" assessment, no padded lists, exactly one suggestion.
        assert!(
            recap.overall_assessment.starts_with("A quick touch"),
            "a thin quiet session opens by naming the quick touch: {}",
            recap.overall_assessment
        );
        assert!(recap.strengths.is_empty());
        assert!(recap.areas_to_improve.is_empty());
        assert_eq!(recap.next_session_suggestions.len(), 1);
    }

    // -----------------------------------------------------------------------
    // #417-4 / #389 — family-aware recap vocabulary
    // -----------------------------------------------------------------------

    /// A deliberately detuned C-major session (~20 cents flat everywhere):
    /// low in-tune ratio + a strong mean tendency — exactly the stats that
    /// trigger every intonation phrase bank.
    fn detuned_session_input(instrument: &str, family: &str) -> RecapInput {
        let flat = 2f64.powf(-20.0 / 1200.0);
        // #445-6b: three settled phrases clear the thin-session bar — the
        // family-vocabulary tests pin FULL-recap copy.
        let mut input = offline_input(settled_phrases(
            vec![
                261.63, 293.66, 329.63, 349.23, 392.00, 440.00, 493.88, 261.63, 329.63, 392.00,
                440.00, 261.63,
            ]
            .into_iter()
            .map(|f| f * flat)
            .collect(),
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            4.0,
        ));
        input.instrument = instrument.to_owned();
        input.instrument_family = family.to_owned();
        input
    }

    fn recap_text(r: &SessionRecap) -> String {
        let mut all = vec![r.overall_assessment.clone()];
        all.extend(r.strengths.clone());
        all.extend(r.areas_to_improve.clone());
        all.extend(r.next_session_suggestions.clone());
        all.join(" | ").to_lowercase()
    }

    /// #389 acceptance, offline path: a PIANO recap contains no player-
    /// intonation critique and no tuner/drone/long-tone advice — and the
    /// strong flat tendency surfaces as the INSTRUMENT's tuning instead.
    #[test]
    fn piano_offline_recap_never_critiques_player_intonation() {
        let recap = grounded_offline_recap(&detuned_session_input("Piano", "Keyboard"));
        let text = recap_text(&recap);
        for forbidden in [
            "tuner",
            "drone",
            "long tones",
            "intonation drifted",
            "sat in tune",
            "air in the tone",
        ] {
            assert!(
                !text.contains(forbidden),
                "piano recap must not say {forbidden:?}: {text}"
            );
        }
        // The honest instrument-level note (phrased as the instrument).
        assert!(
            text.contains("your piano reads about") && text.contains("instrument"),
            "the strong flat read surfaces as the instrument's tuning: {text}"
        );
        // Review MF2 (AC2): the SUGGESTION bank fires too — the overall
        // line shares the "reads about" prefix, so pin the tuning-visit
        // line where it lives.
        assert!(
            recap
                .next_session_suggestions
                .iter()
                .any(|s| s.to_lowercase().contains("a tuning visit")),
            "the >=10-cent tendency earns the tuning-visit suggestion: {:?}",
            recap.next_session_suggestions
        );
    }

    /// #389 acceptance, other half: the same detuned session on TRUMPET
    /// keeps the continuous-pitch bank — tuner/drone advice is correct there.
    #[test]
    fn trumpet_offline_recap_keeps_the_continuous_pitch_bank() {
        let recap = grounded_offline_recap(&detuned_session_input("trumpet", "Brass"));
        let text = recap_text(&recap);
        assert!(
            text.contains("drone") || text.contains("tuner"),
            "continuous-pitch instruments keep intonation practice advice: {text}"
        );
        assert!(
            !text.contains("your trumpet reads about"),
            "the instrument-tuning note is fixed-pitch only: {text}"
        );
    }

    /// #417-4: the key-anchored opener suggestion speaks each family's
    /// practice language — hands/evenness for keyboard, long tones for
    /// continuous pitch. Uses the in-tune C-major session (the key asserts,
    /// so the opener fires).
    #[test]
    fn opener_suggestion_speaks_the_familys_language() {
        // #445-6b: settled_phrases clears the thin-session bar — this test
        // pins FULL-recap opener copy.
        let in_tune = || {
            offline_input(settled_phrases(
                vec![
                    261.63, 293.66, 329.63, 349.23, 392.00, 440.00, 493.88, 261.63, 329.63, 392.00,
                    440.00, 261.63,
                ],
                vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
                4.0,
            ))
        };
        let mut piano = in_tune();
        piano.instrument = "Piano".to_owned();
        piano.instrument_family = "Keyboard".to_owned();
        let piano_text = recap_text(&grounded_offline_recap(&piano));
        assert!(
            !piano_text.contains("long tones"),
            "keyboard opener must not breathe: {piano_text}"
        );
        assert!(
            piano_text.contains("hands together") || piano_text.contains("slow scale"),
            "keyboard opener speaks hands/evenness: {piano_text}"
        );

        let trumpet_text = recap_text(&grounded_offline_recap(&in_tune()));
        assert!(
            trumpet_text.contains("long tones"),
            "continuous-pitch opener keeps long tones: {trumpet_text}"
        );
    }

    /// #417-4: an UNKNOWN family (old stored inputs, serde default) behaves
    /// exactly like continuous pitch — no silent behavior change.
    #[test]
    fn unknown_family_defaults_to_continuous_pitch_behavior() {
        let recap = grounded_offline_recap(&detuned_session_input("trumpet", ""));
        let text = recap_text(&recap);
        assert!(text.contains("drone") || text.contains("tuner"), "{text}");
    }

    /// #417-4: the LLM system prompt carries the fixed-pitch guardrails for
    /// keyboard family — and does NOT for continuous-pitch instruments.
    #[test]
    fn recap_system_prompt_gates_fixed_pitch_rules_by_family() {
        let keyboard = CoachingEngine::build_recap_system_prompt(true, "Keyboard");
        assert!(keyboard.contains("FIXED-PITCH INSTRUMENT RULES"));
        assert!(keyboard.contains("evenness between the hands"));
        assert!(keyboard.contains("Never critique the player's tuning"));
        let brass = CoachingEngine::build_recap_system_prompt(true, "Brass");
        assert!(!brass.contains("FIXED-PITCH"));
        let unknown = CoachingEngine::build_recap_system_prompt(true, "");
        assert!(!unknown.contains("FIXED-PITCH"));
    }

    /// #417-4: the LLM user prompt reframes the intonation FACT for fixed
    /// pitch — instrument tuning, explicitly not player-controllable — and
    /// keeps the player framing for continuous pitch.
    #[test]
    fn recap_user_prompt_reframes_intonation_for_fixed_pitch() {
        let piano =
            CoachingEngine::build_recap_user_prompt(&detuned_session_input("Piano", "Keyboard"));
        assert!(
            piano.contains("NOT player-controllable"),
            "piano prompt frames cents as instrument tuning: {piano}"
        );
        assert!(!piano.contains("- Intonation:"));
        let trumpet =
            CoachingEngine::build_recap_user_prompt(&detuned_session_input("trumpet", "Brass"));
        assert!(trumpet.contains("- Intonation:"));
        assert!(!trumpet.contains("NOT player-controllable"));
    }
}
