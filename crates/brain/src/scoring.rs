//! Per-note scoring engine.
//!
//! Evaluates each played note against the expected note from the score
//! and produces a verdict: green (good), yellow (acceptable), red (needs work).

use serde::{Deserialize, Serialize};

/// Verdict for a single played note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Green,
    Yellow,
    Red,
}

/// Detailed score for a single note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteScore {
    /// Index of the note in the score
    pub note_index: usize,
    /// Pitch deviation in cents (positive = sharp, negative = flat)
    pub cents_deviation: f64,
    /// Timing error in milliseconds (positive = late, negative = early)
    pub timing_error_ms: f64,
    /// Overall verdict
    pub verdict: Verdict,
}

/// Thresholds for scoring verdicts.
#[derive(Debug, Clone)]
pub struct ScoringThresholds {
    /// Max cents deviation for green
    pub green_cents: f64,
    /// Max cents deviation for yellow (above this = red)
    pub yellow_cents: f64,
    /// Max timing error (ms) for green
    pub green_timing_ms: f64,
    /// Max timing error (ms) for yellow
    pub yellow_timing_ms: f64,
}

impl Default for ScoringThresholds {
    fn default() -> Self {
        Self {
            green_cents: 15.0,
            yellow_cents: 30.0,
            green_timing_ms: 50.0,
            yellow_timing_ms: 100.0,
        }
    }
}

/// Score a single note against expected values.
pub fn score_note(
    note_index: usize,
    cents_deviation: f64,
    timing_error_ms: f64,
    thresholds: &ScoringThresholds,
) -> NoteScore {
    let cents_abs = cents_deviation.abs();
    let timing_abs = timing_error_ms.abs();

    let verdict = if cents_abs <= thresholds.green_cents && timing_abs <= thresholds.green_timing_ms
    {
        Verdict::Green
    } else if cents_abs <= thresholds.yellow_cents && timing_abs <= thresholds.yellow_timing_ms {
        Verdict::Yellow
    } else {
        Verdict::Red
    };

    NoteScore {
        note_index,
        cents_deviation,
        timing_error_ms,
        verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_note_is_green() {
        let score = score_note(0, 2.0, 10.0, &ScoringThresholds::default());
        assert_eq!(score.verdict, Verdict::Green);
    }

    #[test]
    fn slightly_off_note_is_yellow() {
        let score = score_note(1, 20.0, 70.0, &ScoringThresholds::default());
        assert_eq!(score.verdict, Verdict::Yellow);
    }

    #[test]
    fn way_off_note_is_red() {
        let score = score_note(2, 50.0, 150.0, &ScoringThresholds::default());
        assert_eq!(score.verdict, Verdict::Red);
    }

    #[test]
    fn note_score_serialization() {
        let score = score_note(0, -5.0, 15.0, &ScoringThresholds::default());
        let json = serde_json::to_string(&score).unwrap();
        assert!(json.contains("\"verdict\":\"green\""));
    }
}
