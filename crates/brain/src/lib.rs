//! # Brain — Score following, scoring, and coaching
//!
//! This crate handles:
//! - Phrase aggregation (grouping audio events into musical phrases)
//! - Per-note scoring (pitch, timing, dynamics, articulation)
//! - Score following (aligning played notes to sheet music)
//! - Adaptive practice planning (spaced repetition)

pub mod coaching;
pub mod phrase;
pub mod score;
pub mod scoring;

#[cfg(test)]
mod tests {
    use crate::phrase::{PhraseAggregator, PhraseConfig};
    use crate::scoring::{score_note, ScoringThresholds, Verdict};

    #[test]
    fn phrase_and_scoring_modules_are_accessible() {
        let agg = PhraseAggregator::new(PhraseConfig::default());
        assert!(agg.is_ok());
        let score = score_note(0, 0.0, 0.0, &ScoringThresholds::default());
        assert_eq!(score.verdict, Verdict::Green);
    }
}
