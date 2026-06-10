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

use crate::fingerprint::MusicalFingerprint;
use crate::phrase::PhraseSummary;
use crate::session::{RecapGenerator, RecapInput, SessionError, SessionRecap};

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
        _ => "claude-opus-4-7".to_owned(),
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

        match response {
            Ok(body) => Self::parse_tip_from_response(&body).or_else(|_| Ok(Self::fallback_tip())),
            Err(_) => Ok(Self::fallback_tip()),
        }
    }

    // -----------------------------------------------------------------------
    // Prompt construction
    // -----------------------------------------------------------------------

    /// Build a generic system prompt that shapes the LLM's coaching personality.
    ///
    /// This is the fallback when instrument-specific prompts are not needed.
    /// For real coaching, use `build_system_prompt_for_instrument`.
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
        let instrument_lower = instrument.to_lowercase();
        let instrument_guidance = match instrument_lower.as_str() {
            // Brass family (trumpet, french horn, trombone, tuba)
            _ if instrument_lower.contains("trumpet")
                || instrument_lower.contains("horn")
                || instrument_lower.contains("trombone")
                || instrument_lower.contains("tuba")
                || instrument_lower.contains("brass") =>
            {
                "You are coaching a brass player. Focus on: embouchure consistency, breath support, \
                resonance and tone projection, clean articulation (tonguing), range extensions, and \
                intonation stability in the upper register. Reference these technical terms naturally \
                when appropriate. Emphasize that a strong embouchure comes from relaxation and \
                air pressure, not tension."
            }

            // Voice
            _ if instrument_lower.contains("voice")
                || instrument_lower.contains("vocal")
                || instrument_lower.contains("singer")
                || instrument_lower.contains("soprano")
                || instrument_lower.contains("alto")
                || instrument_lower.contains("tenor")
                || instrument_lower.contains("bass") =>
            {
                "You are coaching a vocalist. Focus on: breath management and phrasing, resonance \
                and projection (not pushing), vowel placement and consistency, vibrato control and \
                speed, legit versus belted production, and register transitions. Use the language \
                a voice teacher would use: open throat, supported breath, resonant space, etc. \
                Emphasize that good tone comes from efficient use of airflow, not muscular tension."
            }

            // Strings (violin, viola, cello, bass)
            _ if instrument_lower.contains("violin")
                || instrument_lower.contains("viola")
                || instrument_lower.contains("cello")
                || instrument_lower.contains("bass")
                || instrument_lower.contains("string") =>
            {
                "You are coaching a string player. Focus on: bow control and balance, intonation \
                stability (especially double stops), vibrato quality and width, clean articulation \
                and bow changes, position shifts and accuracy, and tone color variation. Reference \
                bow techniques, string crossing, and left-hand position naturally. Emphasize that \
                good intonation comes from listening and micro-adjustments, not from tension."
            }

            // Woodwinds (flute, clarinet, oboe, saxophone)
            _ if instrument_lower.contains("flute")
                || instrument_lower.contains("clarinet")
                || instrument_lower.contains("oboe")
                || instrument_lower.contains("saxophone")
                || instrument_lower.contains("bassoon")
                || instrument_lower.contains("woodwind") =>
            {
                "You are coaching a woodwind player. Focus on: embouchure flexibility and tone \
                centering, breath support and phrasing, tone color and articulation clarity, vibrato \
                control (for appropriate instruments), and register transitions. Use woodwind-specific \
                language: air stream, voicing, response. Emphasize that tone comes from the air \
                moving efficiently through an open, flexible embouchure."
            }

            // Piano
            _ if instrument_lower.contains("piano") || instrument_lower.contains("keyboard") => {
                "You are coaching a pianist. Focus on: hand position and relaxation, even touch \
                across registers, voicing and balance in chords, pedal timing and clarity, runs \
                and passages with rhythmic precision, and legato/staccato articulation. Reference \
                weight distribution, finger independence, and arm rotation naturally. Emphasize \
                that technical fluency comes from relaxed efficiency and musical listening, not speed."
            }

            // Unknown instrument: use generic prompt
            _ => {
                return Self::build_system_prompt();
            }
        };

        format!(
            "You are a warm, experienced music teacher providing real-time coaching \
            during a practice session. Your role is to be an encouraging mentor \
            who helps the student improve through positive, actionable feedback.\n\n\
            INSTRUMENT-SPECIFIC GUIDANCE:\n\
            {}\n\n\
            IMPORTANT RULES:\n\
            - NEVER give letter grades (A, B, C, D, F) or percentage scores.\n\
            - NEVER say things like \"you scored 85%\" or \"that was a B+\".\n\
            - NEVER use judgmental language like \"poor\", \"bad\", or \"failing\".\n\
            - Focus on ONE actionable improvement at a time.\n\
            - Be encouraging FIRST, then constructive.\n\
            - Reference specific musical aspects you observe in the data.\n\
            - Use warm, conversational language as if speaking to the student in person.\n\
            - Keep tips concise — one to three sentences maximum.\n\n\
            Respond with valid JSON in this exact format:\n\
            {{\n\
              \"text\": \"Your coaching tip here\",\n\
              \"severity\": \"encouragement\" | \"suggestion\" | \"focus\",\n\
              \"category\": \"tone\" | \"intonation\" | \"rhythm\" | \"dynamics\" | \"expression\" | \"technique\"\n\
            }}\n\n\
            Choose severity based on the data:\n\
            - \"encouragement\" when the student is doing well in an area\n\
            - \"suggestion\" for gentle improvements\n\
            - \"focus\" when an area clearly needs attention\n\n\
            Choose the category that best matches the most notable aspect of the phrase data.",
            instrument_guidance
        )
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

        // Tone quality, when analysis ran over the phrase audio. Lets a live
        // tip speak to *how* it sounded, not just pitch/rhythm/dynamics.
        if let Some(t) = &phrase.tone {
            prompt.push_str(&format!("- Tone: {}\n", describe_tone(t)));
        }

        if !context.previous_tips.is_empty() {
            prompt.push_str("\nPrevious tips already given (avoid repeating these):\n");
            for tip in &context.previous_tips {
                prompt.push_str(&format!("- {tip}\n"));
            }
        }

        // In Score Mode, tell the coach what piece this is so the tip can
        // speak to the music, not just the instrument.
        if let Some(title) = &context.score_title {
            prompt.push_str(&format!("\nThe student is playing \"{title}\".\n"));
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
// RecapGenerator implementation
// ===========================================================================

#[async_trait]
impl RecapGenerator for CoachingEngine {
    async fn generate_recap(&self, input: &RecapInput) -> Result<SessionRecap, SessionError> {
        let system_prompt = Self::build_recap_system_prompt();
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
            Ok(body) => Self::parse_recap_from_response(&body, input)
                .or_else(|_| Ok(Self::fallback_recap(input))),
            Err(_) => Ok(Self::fallback_recap(input)),
        }
    }
}

// ===========================================================================
// Recap-specific prompt and parsing
// ===========================================================================

impl CoachingEngine {
    /// Build a system prompt for session recap generation.
    fn build_recap_system_prompt() -> String {
        "\
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
- Use warm, conversational language.

Respond with valid JSON in this exact format:
{
  \"overall_assessment\": \"One paragraph capturing the overall arc of the session\",
  \"strengths\": [\"specific strength 1\", \"specific strength 2\"],
  \"areas_to_improve\": [\"area 1\", \"area 2\"],
  \"next_session_suggestions\": [\"focus 1\", \"focus 2\"]
}

All text should be written as a teacher would speak — warm, specific, and actionable."
            .to_owned()
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
        let key_line = match &fingerprint.key {
            Some(k) => format!(
                "- Key / mode: {} (confidence {:.2})\n",
                k.name(),
                k.confidence
            ),
            None => String::new(),
        };

        // Intonation over the session, when enough notes were observed. These
        // are *computed* cents figures — the model must not invent numbers, only
        // phrase the facts we hand it.
        let intonation_line = match &fingerprint.intonation {
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

        format!(
            "Please write end-of-session notes for a student who just finished practicing {}. \
            They played {} phrases over approximately {} minutes.\n\n\
            Phrase data summary:\n\
            - Phrase count: {}\n\
            - Average intonation tendency: {:.2}\n\
            - Average dynamic control: {:.2}\n\
            {}{}{}{}\n\
            {}{}{}\n\n\
            Based on this practice session, write encouraging, specific, handwritten-style notes \
            that celebrate what went well and identify clear next steps.{}",
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
            if input.score_title.is_some() {
                " Where it helps, refer to specific measures by number so the \
                student knows exactly which passage you mean."
            } else {
                ""
            },
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

        let recap = SessionRecap {
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
            fingerprint: fingerprint_for_recap(&input.phrases),
            // Carry the gated, offline idiom matches straight through. They are
            // grounded facts the recorder computed, not LLM output — so we
            // persist them verbatim regardless of what the model returned.
            idiom_notes: input.idiom_notes.clone(),
        };

        Ok(recap)
    }

    /// Fallback recap returned when the API fails.
    fn fallback_recap(input: &RecapInput) -> SessionRecap {
        SessionRecap {
            overall_assessment: format!(
                "You completed a {} minute practice session on {}. \
                 That's excellent dedication! You played {} phrases and showed consistent engagement.",
                (input.duration_secs / 60.0).round() as i32,
                input.instrument,
                input.phrases.len()
            ),
            strengths: vec![
                "Consistent practice and focus.".to_owned(),
                "You showed up and played — that's what matters most.".to_owned(),
            ],
            areas_to_improve: vec![
                "Every session builds on the last one.".to_owned(),
                "Keep recording yourself to track progress.".to_owned(),
            ],
            next_session_suggestions: vec![
                "Work on the passages that felt most challenging.".to_owned(),
                "Try breaking difficult sections into smaller chunks.".to_owned(),
            ],
            duration_secs: input.duration_secs,
            phrase_count: input.phrases.len(),
            instrument: input.instrument.clone(),
            fingerprint: fingerprint_for_recap(&input.phrases),
            // Even when the LLM call fails, the offline idiom matches stand on
            // their own — surface them so the fallback recap still gets the
            // grounded "reminds me of" note. The frontend hedges the phrasing.
            idiom_notes: input.idiom_notes.clone(),
        }
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

/// Build the session's [`MusicalFingerprint`] from the per-dimension
/// aggregation. Each dimension reuses its existing evidence gate (see the
/// `aggregate_*` functions), so a dimension is `Some` only when the session
/// produced enough evidence to report it honestly. This is the single place
/// the four measurements are assembled — the recap prompt and the persisted
/// recap both source their grounded facts from the result.
fn build_fingerprint(phrases: &[PhraseSummary]) -> MusicalFingerprint {
    MusicalFingerprint {
        tone: aggregate_tone(phrases),
        key: aggregate_key(phrases),
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

/// Session-level key/mode over every phrase's detected pitches. Returns `None`
/// unless the fit is confident enough to state plainly — we'd rather show no key
/// than a shaky one (a trained ear would catch a wrong call instantly, and the
/// cultural-relevance layer must never build on an invented key).
fn aggregate_key(phrases: &[PhraseSummary]) -> Option<theory::KeyEstimate> {
    /// Don't surface a key below this correlation — hedge instead.
    const MIN_CONFIDENCE: f32 = 0.5;
    /// Or below this many distinct pitch classes — you can't name a key from a
    /// note or two, however well a lone spike happens to correlate.
    const MIN_DISTINCT: usize = 4;
    let mut profile = theory::PitchClassProfile::new();
    for p in phrases {
        for &hz in &p.pitch_stats.pitches {
            profile.add_hz(hz as f32, 1.0);
        }
    }
    let est = theory::estimate_key(&profile)?;
    (profile.distinct() >= MIN_DISTINCT && est.confidence >= MIN_CONFIDENCE).then_some(est)
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

    // Reuse the gated session key (if any) to anchor per-degree tendencies.
    let tonic = aggregate_key(phrases).map(|k| k.tonic);

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
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![
                sample_phrase_at_measure(0, 1),
                sample_phrase_at_measure(1, 5),
            ],
            tips: vec![],
            score_title: Some("Haydn Trumpet Concerto".to_owned()),
            idiom_notes: Vec::new(),
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
            duration_secs: 600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: vec![],
            score_title: None,
            idiom_notes: Vec::new(),
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
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![toned],
            tips: Vec::new(),
            score_title: None,
            idiom_notes: Vec::new(),
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
        // No phrases → no key.
        assert!(aggregate_key(&[]).is_none());

        // A phrase whose pitches spell a C-major scale with the tonic (C) and
        // fifth (G) emphasised — as real playing does — → confident C major.
        let mut p = sample_phrase();
        p.pitch_stats.pitches = vec![
            261.63, 261.63, 261.63, // C ×3 (tonic)
            293.66, 329.63, 349.23, // D E F
            392.0, 392.0, // G ×2 (fifth)
            440.0, 493.88, // A B
        ];
        let key = aggregate_key(std::slice::from_ref(&p)).expect("a clear key");
        assert_eq!(key.name(), "C major", "got {}", key.name());

        // A single repeated note is too thin → hedge to None.
        let mut thin = sample_phrase();
        thin.pitch_stats.pitches = vec![440.0; 6];
        assert!(
            aggregate_key(std::slice::from_ref(&thin)).is_none(),
            "a single pitch class must not yield a confident key"
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
        let summary =
            aggregate_intonation(std::slice::from_ref(&p)).expect("enough notes to report");
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
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![p],
            tips: Vec::new(),
            score_title: None,
            idiom_notes: Vec::new(),
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

    #[test]
    fn recap_prompt_includes_idiom_block_as_grounded_input_when_present() {
        // A session with a gated idiom match should surface it in the prompt as
        // GROUNDED INPUT the model may hedge around — never as a hard fact.
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            idiom_notes: vec![sample_idiom_match()],
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
    fn recap_prompt_omits_idiom_block_when_silent() {
        // No gated matches → the recap stays silent on idiom (no block).
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            idiom_notes: Vec::new(),
        };
        let prompt = CoachingEngine::build_recap_user_prompt(&input);
        assert!(
            !prompt.contains("Idiom proximity"),
            "no idiom block when nothing cleared the gate, got:\n{prompt}"
        );
    }

    #[test]
    fn fallback_recap_carries_idiom_notes() {
        // The offline matches stand alone — they survive an LLM failure into the
        // fallback recap so the grounded "reminds me of" note still shows.
        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            duration_secs: 120.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: Vec::new(),
            score_title: None,
            idiom_notes: vec![sample_idiom_match()],
        };
        let recap = CoachingEngine::fallback_recap(&input);
        assert_eq!(
            recap.idiom_notes,
            vec![sample_idiom_match()],
            "fallback recap must carry the gated idiom matches verbatim"
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
        assert_eq!(resolve_model("", None), "claude-opus-4-7");
        assert_eq!(resolve_model("", Some("")), "claude-opus-4-7");
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
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 10.0,
            },
            Box::new(mock),
        )
        .unwrap();

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
        assert_eq!(
            tip2.severity,
            CoachingSeverity::Encouragement,
            "Rate-limited response should be encouragement"
        );
        assert!(
            tip2.text.contains("momentum"),
            "Rate-limited tip should have characteristic wording"
        );
    }

    #[tokio::test]
    async fn api_failure_returns_fallback_tip() {
        let mock = MockHttpClient::failing("Service unavailable");
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let phrase = sample_phrase();
        let context = sample_context();

        let result = engine.get_tip(&phrase, &context).await;
        assert!(result.is_ok(), "API failure should not propagate error");

        let tip = result.unwrap();
        assert!(
            !tip.text.is_empty(),
            "Fallback tip should have non-empty text"
        );
        assert_eq!(
            tip.severity,
            CoachingSeverity::Encouragement,
            "Fallback tip should be encouraging"
        );
    }

    #[tokio::test]
    async fn malformed_response_returns_fallback() {
        let mock = MockHttpClient::succeeding("{\"invalid\": \"json\"}");
        let mut engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let phrase = sample_phrase();
        let context = sample_context();

        let result = engine.get_tip(&phrase, &context).await;
        assert!(
            result.is_ok(),
            "Malformed response should not propagate error"
        );
        let tip = result.unwrap();
        assert!(
            !tip.text.is_empty(),
            "Fallback should be provided for malformed response"
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
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let input = RecapInput {
            instrument: "trumpet".to_owned(),
            duration_secs: 1800.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase()],
            tips: vec![],
            score_title: None,
            idiom_notes: Vec::new(),
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "trumpet");
        assert_eq!(recap.duration_secs, 1800.0);
        assert_eq!(recap.phrase_count, 1);
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
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "gpt-4".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let input = RecapInput {
            instrument: "violin".to_owned(),
            duration_secs: 2400.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase(); 3],
            tips: vec![],
            score_title: None,
            idiom_notes: Vec::new(),
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
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let input = RecapInput {
            instrument: "voice".to_owned(),
            duration_secs: 1500.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase(); 5],
            tips: vec![],
            score_title: None,
            idiom_notes: Vec::new(),
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "voice");
        assert_eq!(recap.phrase_count, 5);
        assert!(recap.duration_secs > 0.0);
        assert!(!recap.overall_assessment.is_empty());
        assert!(!recap.strengths.is_empty());
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
        let engine = CoachingEngine::new(
            CoachingConfig {
                api_key: "test".to_owned(),
                model: "claude-3-5-sonnet".to_owned(),
                rate_limit_secs: 0.0,
            },
            Box::new(mock),
        )
        .unwrap();

        let input = RecapInput {
            instrument: "piano".to_owned(),
            duration_secs: 3600.0,
            practice_mode: crate::session::PracticeMode::default(),
            phrases: vec![sample_phrase(); 2],
            tips: vec![],
            score_title: None,
            idiom_notes: Vec::new(),
        };

        let recap = engine.generate_recap(&input).await.unwrap();

        assert_eq!(recap.instrument, "piano");
        assert!(!recap.overall_assessment.is_empty());
        assert!(recap.strengths.is_empty() || !recap.strengths[0].is_empty());
    }
}
