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

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::phrase::PhraseSummary;

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
    /// Model identifier (e.g. "claude-3-5-sonnet", "gpt-4").
    /// Falls back to `MUSIC_COMPANION_LLM_MODEL` env var, then to
    /// `"claude-3-5-sonnet"`.
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
        _ => "claude-3-5-sonnet".to_owned(),
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
    /// 3. `"claude-3-5-sonnet"` default
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
        })
    }

    /// Request a coaching tip for the given phrase and session context.
    ///
    /// Respects rate limiting: if the last call was fewer than
    /// `config.rate_limit_secs` seconds ago, returns a rate-limited
    /// generic tip without calling the API.
    ///
    /// On API failure, returns a generic encouraging tip instead of an error
    /// (graceful degradation).
    pub async fn get_tip(
        &mut self,
        phrase: &PhraseSummary,
        context: &SessionContext,
    ) -> Result<CoachingTip, CoachingError> {
        // Rate limiting
        if let Some(last) = self.last_call_time {
            let elapsed = last.elapsed().as_secs_f64();
            if elapsed < self.config.rate_limit_secs {
                return Ok(Self::rate_limited_tip());
            }
        }

        let system_prompt = Self::build_system_prompt();
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

        match response {
            Ok(body) => Self::parse_tip_from_response(&body).or_else(|_| Ok(Self::fallback_tip())),
            Err(_) => Ok(Self::fallback_tip()),
        }
    }

    // -----------------------------------------------------------------------
    // Prompt construction
    // -----------------------------------------------------------------------

    /// Build the system prompt that shapes the LLM's coaching personality.
    ///
    /// This is public for testing so we can verify it contains required
    /// anti-grading language.
    pub fn build_system_prompt() -> String {
        "\
You are a warm, experienced music teacher providing real-time coaching \
during a practice session. Your role is to be an encouraging mentor \
who helps the student improve through positive, actionable feedback.

IMPORTANT RULES:
- NEVER give letter grades (A, B, C, D, F) or percentage scores.
- NEVER say things like \"you scored 85%\" or \"that was a B+\".
- NEVER use judgmental language like \"poor\", \"bad\", or \"failing\".
- Focus on ONE actionable improvement at a time.
- Be encouraging FIRST, then constructive.
- Reference specific musical aspects you observe in the data.
- Vary your feedback category based on what the data shows needs attention.
- Use warm, conversational language as if speaking to the student in person.
- Keep tips concise — one to three sentences maximum.

Respond with valid JSON in this exact format:
{
  \"text\": \"Your coaching tip here\",
  \"severity\": \"encouragement\" | \"suggestion\" | \"focus\",
  \"category\": \"tone\" | \"intonation\" | \"rhythm\" | \"dynamics\" | \"expression\" | \"technique\"
}

Choose severity based on the data:
- \"encouragement\" when the student is doing well in an area
- \"suggestion\" for gentle improvements
- \"focus\" when an area clearly needs attention

Choose the category that best matches the most notable aspect of the phrase data."
            .to_owned()
    }

    /// Build the user prompt from phrase data and session context.
    ///
    /// Public for testing so we can verify context influences the prompt.
    pub fn build_user_prompt(phrase: &PhraseSummary, context: &SessionContext) -> String {
        let mut prompt = format!(
            "Instrument: {instrument}\n\
             Session duration: {duration:.0} seconds\n\
             Phrases played so far: {phrases}\n\
             \n\
             Current phrase analysis:\n\
             - Duration: {phrase_dur:.2}s\n\
             - Notes played: {notes}\n\
             - Pitch: mean {mean_hz:.1} Hz, range {range_cents:.0} cents\n\
             - Pitch stability: {stability:.2} (0 = unstable, 1 = perfectly stable)\n\
             - Dynamics: mean amplitude {mean_amp:.3}, range {dyn_range:.3}\n",
            instrument = context.instrument,
            duration = context.session_duration_secs,
            phrases = context.phrases_played,
            phrase_dur = phrase.duration_secs,
            notes = phrase.note_count,
            mean_hz = phrase.pitch_stats.mean_hz,
            range_cents = phrase.pitch_stats.range_cents,
            stability = phrase.stability,
            mean_amp = phrase.dynamics.mean_amplitude,
            dyn_range = phrase.dynamics.dynamic_range,
        );

        if !context.previous_tips.is_empty() {
            prompt.push_str("\nPrevious tips already given (avoid repeating these):\n");
            for tip in &context.previous_tips {
                prompt.push_str(&format!("- {tip}\n"));
            }
        }

        prompt.push_str("\nPlease provide a coaching tip based on this data.");
        prompt
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

    // -----------------------------------------------------------------------
    // Fallback tips
    // -----------------------------------------------------------------------

    /// Generic encouraging tip returned when the API fails.
    fn fallback_tip() -> CoachingTip {
        CoachingTip {
            text: "Great work keeping at it! Consistent practice is the key to improvement. \
                   Try focusing on the passages that feel most comfortable and gradually \
                   push your boundaries."
                .to_owned(),
            severity: CoachingSeverity::Encouragement,
            category: CoachingCategory::Expression,
        }
    }

    /// Generic tip returned when rate-limited.
    fn rate_limited_tip() -> CoachingTip {
        CoachingTip {
            text: "Keep up the momentum! Take a moment to listen back to what you just \
                   played and notice what felt natural."
                .to_owned(),
            severity: CoachingSeverity::Encouragement,
            category: CoachingCategory::Expression,
        }
    }
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
        }
    }

    fn sample_context() -> SessionContext {
        SessionContext {
            instrument: "trumpet".to_owned(),
            session_duration_secs: 120.0,
            phrases_played: 5,
            previous_tips: vec!["Try relaxing your embouchure on the high notes.".to_owned()],
        }
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
            model: "claude-3-5-sonnet".to_owned(),
            rate_limit_secs: 3.0,
        };
        CoachingEngine::new(config, Box::new(mock)).unwrap()
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
            .unwrap();

        assert_eq!(tip.severity, CoachingSeverity::Suggestion);
        assert_eq!(tip.category, CoachingCategory::Expression);
        assert!(
            tip.text.contains("phrase breathe"),
            "Expected LLM tip text, got: {}",
            tip.text
        );
    }

    #[tokio::test]
    async fn get_tip_gracefully_degrades_on_api_failure() {
        let mock = MockHttpClient::failing("connection refused");
        let mut engine = make_engine(mock);

        // Should NOT return Err — should return a fallback tip
        let result = engine.get_tip(&sample_phrase(), &sample_context()).await;

        assert!(result.is_ok(), "API failure should degrade gracefully");
        let tip = result.unwrap();
        assert_eq!(tip.severity, CoachingSeverity::Encouragement);
        assert!(
            tip.text.contains("Consistent practice"),
            "Fallback tip should encourage practice, got: {}",
            tip.text
        );
    }

    #[tokio::test]
    async fn rate_limiter_skips_call_when_too_soon() {
        let mock = MockHttpClient::succeeding(&mock_anthropic_response());
        let call_count = Arc::clone(&mock.call_count);
        let mut engine = make_engine(mock);

        // First call — should hit the API
        let _tip1 = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        // Second call immediately — should be rate-limited
        let tip2 = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "Rate limiter should have prevented the second API call"
        );
        // The rate-limited tip has specific text
        assert!(
            tip2.text.contains("Keep up the momentum"),
            "Rate-limited tip should be the generic one, got: {}",
            tip2.text
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
        let mut engine = CoachingEngine::new(config, Box::new(mock)).unwrap();

        let tip = engine
            .get_tip(&sample_phrase(), &sample_context())
            .await
            .unwrap();

        assert_eq!(tip.category, CoachingCategory::Dynamics);
        assert_eq!(tip.severity, CoachingSeverity::Encouragement);
    }

    #[tokio::test]
    async fn malformed_llm_response_triggers_fallback() {
        // The LLM returns text that isn't valid JSON
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

        // Should get the fallback, not an error
        assert_eq!(tip.severity, CoachingSeverity::Encouragement);
        assert!(tip.text.contains("Consistent practice"));
    }

    #[test]
    fn missing_api_key_returns_error() {
        // Use with_env + explicit None so we never touch process env.
        // This is the deterministic, parallel-safe equivalent of the old test
        // that called env::remove_var — that approach would race against
        // other tests reading MUSIC_COMPANION_LLM_API_KEY.
        let config = CoachingConfig {
            api_key: String::new(),
            model: "claude-3-5-sonnet".to_owned(),
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
        assert_eq!(resolve_model("", None), "claude-3-5-sonnet");
        assert_eq!(resolve_model("", Some("")), "claude-3-5-sonnet");
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
}
