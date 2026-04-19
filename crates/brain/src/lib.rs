//! # Brain — Score following, scoring, and coaching
//!
//! This crate handles:
//! - Phrase aggregation (grouping audio events into musical phrases)
//! - Per-note scoring (pitch, timing, dynamics, articulation)
//! - Score following (aligning played notes to sheet music)
//! - Adaptive practice planning (spaced repetition)
//! - Session recording and persistence (SQLite-backed)

pub mod coaching;
pub mod phrase;
pub mod score;
pub mod scoring;
pub mod session;
pub mod store;

#[cfg(test)]
mod tests {
    use crate::phrase::{PhraseAggregator, PhraseConfig};
    use crate::scoring::{score_note, ScoringThresholds, Verdict};
    use crate::session::SessionId;
    use crate::store::SessionStore;

    #[test]
    fn phrase_and_scoring_modules_are_accessible() {
        let agg = PhraseAggregator::new(PhraseConfig::default());
        assert!(agg.is_ok());
        let score = score_note(0, 0.0, 0.0, &ScoringThresholds::default());
        assert_eq!(score.verdict, Verdict::Green);
    }

    #[test]
    fn session_and_store_modules_are_accessible() {
        // Forcing the types to resolve means lib.rs must actually re-export
        // session and store. A dropped `pub mod` would break this at compile
        // time.
        let _id = SessionId::new();
        let store = SessionStore::in_memory();
        assert!(store.is_ok(), "in-memory SQLite must always open");
    }
}
