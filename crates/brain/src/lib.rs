//! # Brain — Score following, scoring, and coaching
//!
//! This crate handles:
//! - Score following (aligning played notes to sheet music)
//! - Per-note scoring (pitch, timing, dynamics, articulation)
//! - Adaptive practice planning (spaced repetition)

pub mod scoring;

#[cfg(test)]
mod tests {
    #[test]
    fn brain_crate_loads() {
        assert!(true);
    }
}
