//! # Brain — Score following, scoring, and coaching
//!
//! This crate handles:
//! - Phrase aggregation (grouping audio events into musical phrases)
//! - Per-note scoring (pitch, timing, dynamics, articulation)
//! - Score following (aligning played notes to sheet music)
//! - Adaptive practice planning (spaced repetition)
//! - Session recording and persistence (SQLite-backed)

pub mod accompaniment;
/// Re-exported so the desktop shell can name chord qualities (#349 T4a)
/// without a direct `theory` dependency.
pub use theory;

pub mod chord_chart;
pub mod chord_judge;
pub mod coach;
pub mod coaching;
pub mod connections;
pub mod fingerprint;
pub mod follower;
pub mod idiom_recap;
pub mod insights;
pub mod learner;
pub mod mirror;
pub mod perception;
pub mod phrase;
pub mod score;
pub mod scoring;
pub mod session;
pub mod stats;
pub mod store;
pub mod wheel;

#[cfg(test)]
mod tests {
    use crate::phrase::{DynamicsStats, PhraseAggregator, PhraseConfig, PhraseSummary, PitchStats};
    use crate::scoring::score_phrase;
    use crate::session::{ScoreId, SessionId};
    use crate::store::SessionStore;

    #[test]
    fn phrase_and_scoring_modules_are_accessible() {
        let agg = PhraseAggregator::new(PhraseConfig::default());
        assert!(agg.is_ok());

        // Test phrase-level scoring with a minimal phrase summary.
        let phrase = PhraseSummary {
            phrase_index: 0,
            start_time: 0.0,
            end_time: 1.0,
            duration_secs: 1.0,
            note_count: 5,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 50.0,
                pitches: vec![440.0; 5],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.5,
                min_amplitude: 0.4,
                max_amplitude: 0.6,
                dynamic_range: 0.2,
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

        let score = score_phrase(&phrase);
        assert!(
            score.intonation_tendency > 0.8,
            "stable phrase should score high"
        );
    }

    #[test]
    fn session_and_store_modules_are_accessible() {
        // Forcing the types to resolve means lib.rs must actually re-export
        // session and store. A dropped `pub mod` would break this at compile
        // time.
        let _session_id = SessionId::new();
        let _score_id = ScoreId::new();
        let store = SessionStore::in_memory();
        assert!(store.is_ok(), "in-memory SQLite must always open");
    }
}
