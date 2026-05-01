//! Integration tests for the phrase aggregation module.

use brain::phrase::{hz_to_cents, PhraseAggregator, PhraseConfig};
use ears::AudioEvent;

/// Helper to create a voiced AudioEvent.
fn voiced(pitch_hz: f64, amplitude: f64, timestamp_secs: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: Some(pitch_hz),
        confidence: 0.95,
        amplitude,
        timestamp_secs,
        is_onset: false,
    }
}

/// Helper to create a silence event.
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
fn end_to_end_session_with_multiple_phrases() {
    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
    };
    let mut agg = PhraseAggregator::new(config).unwrap();

    // Simulated practice session:
    // Phrase 1: Play A4 (440 Hz) for ~150ms
    agg.push(&voiced(440.0, 0.7, 0.0));
    agg.push(&voiced(442.0, 0.75, 0.05));
    agg.push(&voiced(438.0, 0.72, 0.10));

    // Silence gap (500ms)
    agg.push(&silence(0.20));
    agg.push(&silence(0.30));
    agg.push(&silence(0.40));
    agg.push(&silence(0.50));

    // Phrase 2: Play C4 (~262 Hz) for ~100ms
    agg.push(&voiced(261.0, 0.6, 0.65));
    agg.push(&voiced(263.0, 0.62, 0.70));
    agg.push(&voiced(262.0, 0.61, 0.75));

    // Silence gap (400ms)
    agg.push(&silence(0.80));
    agg.push(&silence(0.90));
    agg.push(&silence(1.00));
    agg.push(&silence(1.10));

    // Phrase 3: Short phrase (only 1 event — should be discarded with min=2)
    agg.push(&voiced(330.0, 0.5, 1.20));

    // End session
    agg.flush();

    let phrases = agg.phrases();

    // Should have 2 valid phrases (third too short)
    assert_eq!(
        phrases.len(),
        2,
        "Expected 2 phrases, got {}",
        phrases.len()
    );

    // Phrase 1 checks
    let p1 = &phrases[0];
    assert_eq!(p1.phrase_index, 0);
    assert_eq!(p1.note_count, 3);
    assert!(p1.pitch_stats.mean_hz > 430.0 && p1.pitch_stats.mean_hz < 450.0);
    assert!(p1.stability > 0.9, "A4 cluster should be very stable");
    assert!(p1.start_time < 0.01);
    assert!((p1.end_time - 0.10).abs() < 0.01);

    // Phrase 2 checks
    let p2 = &phrases[1];
    assert_eq!(p2.phrase_index, 1);
    assert_eq!(p2.note_count, 3);
    assert!(p2.pitch_stats.mean_hz > 255.0 && p2.pitch_stats.mean_hz < 270.0);
    assert!(p2.stability > 0.9, "C4 cluster should be very stable");
}

#[test]
fn phrases_are_serializable_to_json() {
    let config = PhraseConfig::default();
    let mut agg = PhraseAggregator::new(config).unwrap();

    agg.push(&voiced(440.0, 0.8, 0.0));
    agg.push(&voiced(440.0, 0.8, 0.05));
    agg.push(&voiced(440.0, 0.8, 0.10));
    agg.flush();

    let phrase = &agg.phrases()[0];
    let json = serde_json::to_string_pretty(phrase).unwrap();

    // Full round-trip: deserialize and verify all key fields
    let parsed: brain::phrase::PhraseSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.phrase_index, 0);
    assert_eq!(parsed.note_count, 3);
    assert!(
        (parsed.start_time - 0.0).abs() < 1e-9,
        "start_time mismatch"
    );
    assert!(
        (parsed.pitch_stats.mean_hz - 440.0).abs() < 1.0,
        "mean_hz should be ~440, got {}",
        parsed.pitch_stats.mean_hz
    );
    assert!(
        parsed.stability > 0.9,
        "stability should be high for identical pitches, got {}",
        parsed.stability
    );
}

#[test]
fn streaming_phrases_with_take_new() {
    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
    };
    let mut agg = PhraseAggregator::new(config).unwrap();

    // First phrase
    agg.push(&voiced(440.0, 0.8, 0.0));
    agg.push(&voiced(440.0, 0.8, 0.05));

    // Gap triggers phrase close on next voiced event
    agg.push(&voiced(330.0, 0.6, 0.60));

    // First phrase should be closed now
    let batch1 = agg.take_new_phrases();
    assert_eq!(batch1.len(), 1);
    assert_eq!(batch1[0].phrase_index, 0);

    // Continue second phrase
    agg.push(&voiced(330.0, 0.6, 0.65));
    agg.flush();

    let batch2 = agg.take_new_phrases();
    assert_eq!(batch2.len(), 1);
    assert_eq!(batch2[0].phrase_index, 1);

    // No more new phrases
    let batch3 = agg.take_new_phrases();
    assert!(batch3.is_empty());
}

#[test]
fn hz_to_cents_musical_intervals() {
    // Perfect fifth: ratio 3/2 ≈ 702 cents
    let fifth = hz_to_cents(440.0, 660.0);
    assert!(
        (fifth - 702.0).abs() < 1.0,
        "Perfect fifth should be ~702 cents, got {fifth}"
    );

    // Major third: ratio 5/4 ≈ 386 cents
    let third = hz_to_cents(440.0, 550.0);
    assert!(
        (third - 386.3).abs() < 1.0,
        "Major third should be ~386 cents, got {third}"
    );
}

#[test]
fn default_config_works() {
    let agg = PhraseAggregator::new(PhraseConfig::default());
    assert!(agg.is_ok(), "Default config should be valid");
}

#[test]
fn silence_gap_equal_to_threshold_does_not_split_phrase() {
    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
    };
    let mut agg = PhraseAggregator::new(config).unwrap();

    // First voiced events
    agg.push(&voiced(440.0, 0.7, 0.0));
    agg.push(&voiced(442.0, 0.7, 0.1));

    // Gap of exactly 0.3s (== threshold) — should NOT split
    agg.push(&voiced(438.0, 0.7, 0.4));
    agg.push(&voiced(440.0, 0.7, 0.45));

    agg.flush();

    let phrases = agg.phrases();
    assert_eq!(
        phrases.len(),
        1,
        "Gap exactly equal to threshold should NOT split — got {} phrases",
        phrases.len()
    );
    assert_eq!(phrases[0].note_count, 4);
}

#[test]
fn phrase_summary_carries_score_position_when_follower_is_set() {
    use brain::follower::ScoreFollower;
    use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};

    // Two-measure score so we can verify the captured measure_number is
    // tied to the *position the follower aligned to*, not a default.
    fn note(midi_number: u8, pitch_hz: f64, start_beat: f64) -> ScoreNote {
        ScoreNote {
            pitch_hz,
            midi_number,
            duration_beats: 0.5,
            start_beat,
            dynamic: None,
            is_rest: false,
        }
    }
    let score = ScoreModel {
        title: "Test scale".to_string(),
        composer: None,
        instrument: None,
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 120.0,
        measures: vec![
            Measure {
                number: 1,
                notes: vec![note(60, 261.63, 0.0), note(62, 293.66, 0.5)],
            },
            Measure {
                number: 2,
                notes: vec![note(64, 329.63, 0.0), note(65, 349.23, 0.5)],
            },
        ],
    };
    let follower = ScoreFollower::new(score).expect("build follower");

    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
    };
    let mut agg = PhraseAggregator::new(config).unwrap();
    agg.set_score_follower(follower);

    // Push two events for measure 1's notes; the follower should align to
    // measure 1, and the phrase that closes on flush carries that position.
    agg.push(&voiced(261.63, 0.6, 0.0));
    agg.push(&voiced(293.66, 0.6, 0.1));
    agg.flush();

    let phrases = agg.phrases();
    assert_eq!(phrases.len(), 1, "expected exactly one phrase");
    let pos = phrases[0]
        .score_position
        .as_ref()
        .expect("score_position should be Some when a follower is set");
    assert_eq!(
        pos.measure_number, 1,
        "phrase should anchor to the measure where it began"
    );
}

#[test]
fn phrase_summary_score_position_is_none_in_free_play_mode() {
    let config = PhraseConfig {
        silence_gap_secs: 0.3,
        min_phrase_events: 2,
    };
    let mut agg = PhraseAggregator::new(config).unwrap();

    agg.push(&voiced(440.0, 0.6, 0.0));
    agg.push(&voiced(440.0, 0.6, 0.1));
    agg.flush();

    let phrases = agg.phrases();
    assert_eq!(phrases.len(), 1);
    assert!(
        phrases[0].score_position.is_none(),
        "free-play mode (no follower) should leave score_position as None"
    );
}
