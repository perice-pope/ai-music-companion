//! # Brain — Score following, scoring, and coaching
//!
//! This crate handles:
//! - Phrase aggregation (grouping audio events into musical phrases)
//! - Per-note scoring (pitch, timing, dynamics, articulation)
//! - Score following (aligning played notes to sheet music)
//! - Adaptive practice planning (spaced repetition)

pub mod phrase;
pub mod scoring;

#[cfg(test)]
mod tests {
    #[test]
    fn brain_crate_loads() {
        assert!(true);
    }
}
