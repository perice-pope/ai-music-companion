//! Phrase-level scoring engine.
//!
//! Evaluates completed musical phrases on three dimensions:
//! - Intonation tendency: how stable and well-centered the pitch is
//! - Rhythmic stability: consistency of timing and pitch evenness
//! - Dynamic arc: whether the phrase had directional shape in dynamics
//!
//! Architecture v2 ("coach, don't judge") replaces per-note green/yellow/red
//! verdicts with phrase-level assessment. See `CLAUDE.md` and `docs/architecture/architecture-v2.md`.

use crate::phrase::PhraseSummary;
use serde::{Deserialize, Serialize};

/// Score for a completed musical phrase across three dimensions.
///
/// All scores are normalized to 0.0-1.0, where 1.0 is best.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhraseScore {
    /// Pitch stability and centering (0.0-1.0).
    ///
    /// High value: pitch is stable and well-centered.
    /// Low value: pitch wavers or drifts significantly.
    pub intonation_tendency: f64,

    /// Consistency of timing and performance evenness (0.0-1.0).
    ///
    /// High value: notes are evenly spaced and performance is smooth.
    /// Low value: timing is irregular or pitch unstable.
    ///
    /// Currently not computed — will be populated once the score follower
    /// provides per-note timing alignment data. Until then, None enforces
    /// silence over lies: coaching will never feed a fake rhythm signal.
    /// See hotspot #135 and decisions-log "Coaching-off-with-banner".
    pub rhythmic_stability: Option<f64>,

    /// Presence of dynamic shape and direction (0.0-1.0).
    ///
    /// High value: phrase has clear dynamic arc (crescendo/diminuendo).
    /// Low value: phrase is dynamically flat.
    pub dynamic_arc: f64,
}

/// Score a completed phrase.
///
/// Takes the phrase summary (computed during playback) and returns
/// a multi-dimensional score suitable for coaching feedback.
///
/// # Examples
///
/// ```ignore
/// let phrase = /* from PhraseAggregator */;
/// let score = score_phrase(&phrase);
/// if score.intonation_tendency > 0.8 {
///     println!("Nice centered pitch!");
/// }
/// ```
pub fn score_phrase(phrase: &PhraseSummary) -> PhraseScore {
    // Intonation: use the existing pitch stability metric.
    // The aggregator's `stability` score is 0.0-1.0 based on coefficient of variation.
    let intonation_tendency = phrase.stability;

    // Rhythmic stability: not yet computed. Until the score follower's
    // per-note timing alignment is wired into PhraseSummary, this is None.
    // This enforces the "silence > lies" principle: we will not feed a fake
    // rhythm signal to coaching. See hotspot #135 and decisions-log entry
    // "2026-04-20 — Coaching-off-with-banner, not rule-based filler tips".
    let rhythmic_stability = None;

    // Dynamic arc: based on whether the phrase has significant dynamic range.
    // Normalize the dynamic range to 0-1. A range of 0 = flat (0.0), larger ranges score higher.
    // Use 0.3 (30% of max amplitude) as the threshold for "has arc".
    let max_possible_amplitude = 1.0; // Audio amplitudes are normalized to ~0-1 range
    let normalized_range = (phrase.dynamics.dynamic_range / max_possible_amplitude).min(1.0);

    // Map range to arc score: 0 range → 0.0, max range → 1.0
    let dynamic_arc = normalized_range;

    PhraseScore {
        intonation_tendency,
        rhythmic_stability,
        dynamic_arc,
    }
}

/// Deprecated: per-note verdicts violate the "coach, don't judge" philosophy.
///
/// Use [`score_phrase`] instead for phrase-level assessment.
#[allow(deprecated)]
#[deprecated(
    since = "1.12.0",
    note = "Use `score_phrase(&PhraseSummary)` instead. Per-note verdicts violate the 'coach, don't judge' design principle."
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Green,
    Yellow,
    Red,
}

/// Deprecated: per-note scoring is replaced with phrase-level assessment.
///
/// Use [`score_phrase`] instead.
#[allow(deprecated)]
#[deprecated(
    since = "1.12.0",
    note = "Use `score_phrase(&PhraseSummary)` instead. Per-note verdicts violate the 'coach, don't judge' design principle."
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteScore {
    /// Index of the note in the score
    pub note_index: usize,
    /// Pitch deviation in cents (positive = sharp, negative = flat)
    pub cents_deviation: f64,
    /// Timing error in milliseconds (positive = late, negative = early)
    pub timing_error_ms: f64,
    /// Overall verdict (deprecated)
    pub verdict: Verdict,
}

/// Deprecated: use phrase-level scoring instead.
#[allow(deprecated)]
#[deprecated(
    since = "1.12.0",
    note = "Thresholds are per-note and violate the 'coach, don't judge' design principle."
)]
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

#[allow(deprecated)]
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

#[allow(deprecated)]
/// Deprecated: use `score_phrase(&PhraseSummary)` instead.
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
    use crate::phrase::{DynamicsStats, PitchStats};

    fn make_phrase_summary(stability: f64, dynamic_range: f64) -> PhraseSummary {
        PhraseSummary {
            phrase_index: 0,
            start_time: 0.0,
            end_time: 1.0,
            duration_secs: 1.0,
            note_count: 10,
            pitch_stats: PitchStats {
                mean_hz: 440.0,
                min_hz: 435.0,
                max_hz: 445.0,
                range_cents: 50.0,
                pitches: vec![440.0; 10],
            },
            dynamics: DynamicsStats {
                mean_amplitude: 0.5,
                min_amplitude: 0.2,
                max_amplitude: 0.2 + dynamic_range,
                dynamic_range,
            },
            stability,
            score_position: None,
            tone: None,
            key: None,
            onsets_secs: Vec::new(),
            score_span: None,
            verdicts: None,
            score_card: None,
        }
    }

    #[test]
    fn in_tune_phrase_has_high_intonation_tendency() {
        let phrase = make_phrase_summary(0.95, 0.1); // Very stable pitch
        let score = score_phrase(&phrase);
        assert!(
            score.intonation_tendency > 0.9,
            "stable pitch should give high intonation_tendency"
        );
    }

    #[test]
    fn rhythmic_stability_not_yet_computed() {
        // Until the score follower's per-note timing alignment reaches PhraseSummary,
        // `rhythmic_stability` is None. This enforces "silence > lies": we will not
        // feed a fake rhythm signal to coaching (hotspot #135).
        let unstable = score_phrase(&make_phrase_summary(0.2, 0.1));
        let stable = score_phrase(&make_phrase_summary(0.95, 0.1));
        assert!(unstable.rhythmic_stability.is_none());
        assert!(stable.rhythmic_stability.is_none());
    }

    #[test]
    fn loud_to_soft_phrase_has_dynamic_arc() {
        let phrase = make_phrase_summary(0.9, 0.6); // High dynamic range
        let score = score_phrase(&phrase);
        assert!(
            score.dynamic_arc > 0.5,
            "high dynamic range should give detectable dynamic_arc"
        );
    }

    #[test]
    fn flat_dynamics_has_no_arc() {
        let phrase = make_phrase_summary(0.9, 0.0); // No dynamic range
        let score = score_phrase(&phrase);
        assert!(
            score.dynamic_arc < 0.1,
            "flat dynamics should give low dynamic_arc"
        );
    }

    #[test]
    fn phrase_score_serialization_roundtrip() {
        let original = PhraseScore {
            intonation_tendency: 0.85,
            rhythmic_stability: Some(0.75),
            dynamic_arc: 0.60,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: PhraseScore = serde_json::from_str(&json).unwrap();
        assert!((deserialized.intonation_tendency - 0.85).abs() < f64::EPSILON);
        assert_eq!(deserialized.rhythmic_stability, Some(0.75));
        assert!((deserialized.dynamic_arc - 0.60).abs() < f64::EPSILON);
    }

    #[test]
    fn moderate_phrase_scores_in_middle_range() {
        let phrase = make_phrase_summary(0.65, 0.3);
        let score = score_phrase(&phrase);
        assert!(score.intonation_tendency > 0.6 && score.intonation_tendency < 0.7);
        // rhythmic_stability is not yet computed (hotspot #135).
        assert!(score.rhythmic_stability.is_none());
        assert!(score.dynamic_arc > 0.2 && score.dynamic_arc < 0.4);
    }
}
