//! Score Mode end-to-end: MusicXML import → score follower binding → phrase
//! detection. `score_test.rs` already covers MusicXML parsing and part
//! selection in isolation; this guards the *pipeline* — that a parsed score and
//! the live phrase aggregator actually work together to surface phrases.
//!
//! Salvaged and updated from the original #183 branch (which predated #185's
//! `voiced_confidence_threshold` and the fallible `ScoreFollower::new`).

use brain::follower::ScoreFollower;
use brain::phrase::{PhraseAggregator, PhraseConfig};
use brain::score::ScoreParser;
use ears::AudioEvent;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// A voiced event with confidence comfortably above the default gate (0.5).
fn voiced(pitch_hz: f64, amplitude: f64, timestamp_secs: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: Some(pitch_hz),
        confidence: 0.95,
        amplitude,
        timestamp_secs,
        is_onset: false,
    }
}

fn silence(timestamp_secs: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: None,
        confidence: 0.0,
        amplitude: 0.001,
        timestamp_secs,
        is_onset: false,
    }
}

#[test]
fn score_mode_parse_then_detect_phrase_end_to_end() {
    // GIVEN a loaded MusicXML score, bound to a score follower
    let path = fixture_path("simple_scale.musicxml");
    let score_model = ScoreParser::parse(&path).expect("parse MusicXML fixture");
    let _follower = ScoreFollower::new(score_model.clone()).expect("bind follower to score");

    // AND a phrase aggregator
    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
        voiced_confidence_threshold: 0.5,
    };
    let mut agg = PhraseAggregator::new(config).expect("create aggregator");

    // WHEN we play the opening of the scale (C D E F) and then fall silent
    agg.push(&voiced(261.63, 0.70, 0.00)); // C4
    agg.push(&voiced(293.66, 0.75, 0.05)); // D4
    agg.push(&voiced(329.63, 0.72, 0.10)); // E4
    agg.push(&voiced(349.23, 0.70, 0.15)); // F4
    agg.push(&silence(0.30));
    agg.push(&silence(0.40));
    agg.push(&silence(0.50));
    agg.flush();

    // THEN a phrase is detected, carrying the notes we played
    let phrases = agg.phrases();
    assert!(
        !phrases.is_empty(),
        "Score Mode should detect at least one phrase"
    );
    assert!(
        phrases[0].note_count > 0,
        "detected phrase should contain notes, got: {:?}",
        phrases[0]
    );

    // AND the parsed score has the measures/notes the follower needs for
    // position computation
    assert!(
        !score_model.measures.is_empty() && !score_model.measures[0].notes.is_empty(),
        "score should expose measures and notes for cursor tracking"
    );
}
