//! Integration tests for the coaching engine.

use brain::coaching::{
    CoachingCategory, CoachingConfig, CoachingEngine, CoachingError, CoachingSeverity, CoachingTip,
    HttpClient, SessionContext,
};
use brain::phrase::{DynamicsStats, PhraseSummary, PitchStats};

// ---------------------------------------------------------------------------
// Mock HTTP client for integration tests
// ---------------------------------------------------------------------------

struct IntegrationMockClient {
    response: String,
}

impl IntegrationMockClient {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl HttpClient for IntegrationMockClient {
    async fn post_json(
        &self,
        _url: &str,
        _body: &str,
        _headers: &[(&str, &str)],
    ) -> Result<String, CoachingError> {
        Ok(self.response.clone())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_phrase(
    mean_hz: f64,
    stability: f64,
    mean_amplitude: f64,
    dynamic_range: f64,
    note_count: usize,
) -> PhraseSummary {
    PhraseSummary {
        phrase_index: 0,
        start_time: 0.0,
        end_time: 2.0,
        duration_secs: 2.0,
        note_count,
        pitch_stats: PitchStats {
            mean_hz,
            min_hz: mean_hz - 5.0,
            max_hz: mean_hz + 5.0,
            range_cents: 39.0,
            pitches: vec![mean_hz; note_count],
        },
        dynamics: DynamicsStats {
            mean_amplitude,
            min_amplitude: mean_amplitude - dynamic_range / 2.0,
            max_amplitude: mean_amplitude + dynamic_range / 2.0,
            dynamic_range,
        },
        stability,
        score_position: None,
    }
}

fn make_context(instrument: &str) -> SessionContext {
    SessionContext {
        instrument: instrument.to_owned(),
        session_duration_secs: 180.0,
        phrases_played: 8,
        previous_tips: vec!["Focus on breath support.".to_owned()],
        score_title: None,
    }
}

fn anthropic_response(text: &str, severity: &str, category: &str) -> String {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!(
                "{{\"text\": \"{text}\", \"severity\": \"{severity}\", \"category\": \"{category}\"}}"
            )
        }]
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_to_end_coaching_with_mock_api() {
    let response = anthropic_response(
        "Your intonation is beautifully centered. Try adding a slight crescendo through the phrase.",
        "suggestion",
        "dynamics",
    );

    let mock = IntegrationMockClient::new(&response);
    let config = CoachingConfig {
        api_key: "integration-test-key".to_owned(),
        model: "claude-3-5-sonnet".to_owned(),
        rate_limit_secs: 0.0, // disable for testing
    };

    let mut engine = CoachingEngine::new(config, Box::new(mock)).unwrap();

    // Build a realistic phrase: A4, high stability, moderate dynamics
    let phrase = make_phrase(440.0, 0.95, 0.7, 0.3, 15);
    let context = make_context("trumpet");

    let tip = engine.get_tip(&phrase, &context).await.unwrap();

    // Verify the returned tip has correct structure
    assert_eq!(tip.severity, CoachingSeverity::Suggestion);
    assert_eq!(tip.category, CoachingCategory::Dynamics);
    assert!(
        tip.text.contains("crescendo"),
        "Tip text should come from mock response, got: {}",
        tip.text
    );

    // Verify the tip is serializable (important for IPC to the frontend)
    let json = serde_json::to_string(&tip).unwrap();
    let roundtripped: CoachingTip = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtripped.text, tip.text);
    assert_eq!(roundtripped.severity, tip.severity);
    assert_eq!(roundtripped.category, tip.category);
}

#[tokio::test]
async fn multiple_phrases_get_different_prompts() {
    let response = anthropic_response("Keep it up!", "encouragement", "tone");

    let mock = IntegrationMockClient::new(&response);
    let config = CoachingConfig {
        api_key: "integration-test-key".to_owned(),
        model: "claude-3-5-sonnet".to_owned(),
        rate_limit_secs: 0.0, // disable rate limiting
    };

    let mut engine = CoachingEngine::new(config, Box::new(mock)).unwrap();

    // Phrase 1: High pitch, stable, soft
    let phrase1 = make_phrase(880.0, 0.98, 0.3, 0.1, 8);
    let context1 = make_context("flute");

    // Phrase 2: Low pitch, less stable, loud
    let phrase2 = make_phrase(220.0, 0.55, 0.85, 0.6, 20);
    let context2 = SessionContext {
        instrument: "trombone".to_owned(),
        session_duration_secs: 300.0,
        phrases_played: 15,
        previous_tips: vec![
            "Focus on breath support.".to_owned(),
            "Keep your slide positions precise.".to_owned(),
        ],
        score_title: None,
    };

    let _tip1 = engine.get_tip(&phrase1, &context1).await.unwrap();
    let _tip2 = engine.get_tip(&phrase2, &context2).await.unwrap();

    // Access the mock through the engine to check recorded bodies.
    // We verify that the user prompts are different by checking the
    // build_user_prompt function directly — this is more reliable than
    // reaching into the mock.
    let prompt1 = CoachingEngine::build_user_prompt(&phrase1, &context1);
    let prompt2 = CoachingEngine::build_user_prompt(&phrase2, &context2);

    // The prompts must differ because the phrase data differs
    assert_ne!(
        prompt1, prompt2,
        "Different phrases should produce different user prompts"
    );

    // Verify specific differences
    assert!(
        prompt1.contains("880.0"),
        "Prompt 1 should contain 880.0 Hz"
    );
    assert!(
        prompt2.contains("220.0"),
        "Prompt 2 should contain 220.0 Hz"
    );
    assert!(prompt1.contains("flute"), "Prompt 1 should mention flute");
    assert!(
        prompt2.contains("trombone"),
        "Prompt 2 should mention trombone"
    );
    assert!(
        prompt2.contains("slide positions"),
        "Prompt 2 should include trombone-specific previous tips"
    );
    assert!(
        prompt1.contains("0.98"),
        "Prompt 1 should reflect high stability"
    );
    assert!(
        prompt2.contains("0.55"),
        "Prompt 2 should reflect lower stability"
    );
}
