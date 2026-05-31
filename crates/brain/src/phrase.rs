//! Phrase aggregation — groups AudioEvents into musical phrases.
//!
//! A phrase boundary is detected when:
//! 1. Silence gap > 300ms (no voiced audio)
//! 2. A new measure boundary is reached (if a score is loaded)
//! 3. The aggregator is explicitly flushed (end of session)
//!
//! This module runs on the processing thread, NOT the audio thread,
//! so heap allocation (Vec, String, etc.) is allowed.

use crate::follower::{ScoreFollower, ScorePosition};
use ears::AudioEvent;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur in phrase aggregation.
#[derive(Debug, Error)]
pub enum PhraseError {
    #[error("silence_gap_secs must be positive, got {0}")]
    InvalidSilenceGap(f64),
    #[error("min_phrase_events must be >= 1, got {0}")]
    InvalidMinEvents(usize),
}

/// Configuration for phrase boundary detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseConfig {
    /// Minimum silence duration (seconds) to split phrases. Default: 0.3 (300ms).
    pub silence_gap_secs: f64,
    /// Minimum number of voiced events to form a valid phrase. Default: 3.
    pub min_phrase_events: usize,
}

impl Default for PhraseConfig {
    fn default() -> Self {
        Self {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
        }
    }
}

impl PhraseConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), PhraseError> {
        if !self.silence_gap_secs.is_finite() || self.silence_gap_secs <= 0.0 {
            return Err(PhraseError::InvalidSilenceGap(self.silence_gap_secs));
        }
        if self.min_phrase_events == 0 {
            return Err(PhraseError::InvalidMinEvents(self.min_phrase_events));
        }
        Ok(())
    }
}

/// Summary statistics for pitches within a phrase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PitchStats {
    /// Mean pitch in Hz across all voiced events.
    pub mean_hz: f64,
    /// Minimum pitch in Hz.
    pub min_hz: f64,
    /// Maximum pitch in Hz.
    pub max_hz: f64,
    /// Range of pitches in cents (from min to max).
    pub range_cents: f64,
    /// Individual detected pitches in Hz.
    pub pitches: Vec<f64>,
}

/// Summary statistics for dynamics (amplitude) within a phrase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicsStats {
    /// Mean amplitude across all events in the phrase.
    pub mean_amplitude: f64,
    /// Minimum amplitude.
    pub min_amplitude: f64,
    /// Maximum amplitude.
    pub max_amplitude: f64,
    /// Dynamic range (max - min).
    pub dynamic_range: f64,
}

/// Summary of a completed musical phrase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseSummary {
    /// Index of this phrase within the session (0-based).
    pub phrase_index: usize,
    /// Timestamp (seconds) when the phrase started.
    pub start_time: f64,
    /// Timestamp (seconds) when the phrase ended.
    pub end_time: f64,
    /// Duration of the phrase in seconds.
    pub duration_secs: f64,
    /// Number of voiced (pitched) events in the phrase.
    pub note_count: usize,
    /// Pitch statistics for the phrase.
    pub pitch_stats: PitchStats,
    /// Dynamics statistics for the phrase.
    pub dynamics: DynamicsStats,
    /// Pitch stability score (0.0 = unstable, 1.0 = perfectly stable).
    pub stability: f64,
    /// Score position at the moment this phrase began, when a score is loaded.
    /// `None` in free-play mode (no score) or before the follower has aligned
    /// any event in this phrase. Downstream consumers (LLM coaching, OSMD
    /// cursor) use this to anchor feedback to a specific measure / beat.
    pub score_position: Option<ScorePosition>,
    /// Tone-quality descriptor for the phrase, when tone analysis ran over the
    /// phrase audio. `None` when no audio was available (e.g. event-only paths)
    /// or tone analysis is disabled. Additive + `serde(default)` so older
    /// persisted phrases deserialize cleanly.
    #[serde(default)]
    pub tone: Option<tone::ToneDescriptor>,
}

/// Groups [`AudioEvent`]s into musical phrases based on silence gaps and score positions.
///
/// This aggregator runs on the processing thread (not the audio thread),
/// so `Vec` allocation is safe and expected.
pub struct PhraseAggregator {
    config: PhraseConfig,
    /// Events in the phrase currently being assembled.
    current_phrase_events: Vec<AudioEvent>,
    /// All completed phrases.
    phrases: Vec<PhraseSummary>,
    /// Timestamp of the first event in the current phrase.
    phrase_start_time: Option<f64>,
    /// Timestamp of the most recent voiced event.
    last_voiced_time: Option<f64>,
    /// Count of phrases produced so far (for indexing).
    next_phrase_index: usize,
    /// Index in `phrases` of the first "new" (un-drained) phrase.
    new_phrases_start: usize,
    /// Optional score follower for score-based phrase boundaries.
    score_follower: Option<ScoreFollower>,
    /// Last known position in the score (for detecting measure changes).
    last_score_measure: Option<usize>,
    /// Score position captured at the first aligned event of the current
    /// phrase. Carried into the emitted [`PhraseSummary::score_position`] so
    /// the LLM and OSMD cursor can anchor feedback to where the phrase began.
    current_phrase_start_position: Option<ScorePosition>,
}

impl PhraseAggregator {
    /// Create a new phrase aggregator with the given configuration.
    ///
    /// Returns an error if the config is invalid.
    pub fn new(config: PhraseConfig) -> Result<Self, PhraseError> {
        config.validate()?;
        Ok(Self {
            config,
            current_phrase_events: Vec::new(),
            phrases: Vec::new(),
            phrase_start_time: None,
            last_voiced_time: None,
            next_phrase_index: 0,
            new_phrases_start: 0,
            score_follower: None,
            last_score_measure: None,
            current_phrase_start_position: None,
        })
    }

    /// Set a score follower to enable score-based phrase boundaries.
    ///
    /// When a score is loaded, phrase boundaries are also triggered at measure
    /// boundaries, in addition to silence gaps. This improves phrase segmentation
    /// when the user is following a score.
    pub fn set_score_follower(&mut self, follower: ScoreFollower) {
        self.score_follower = Some(follower);
        self.last_score_measure = None;
        self.current_phrase_start_position = None;
    }

    /// Clear the score follower (when the session ends or score is unloaded).
    pub fn clear_score_follower(&mut self) {
        self.score_follower = None;
        self.last_score_measure = None;
        self.current_phrase_start_position = None;
    }

    /// The follower's current position in the score, if a score is loaded.
    ///
    /// Reflects the most recent alignment from [`push`](Self::push) — i.e.
    /// where the player is *right now*, not the position a phrase began on
    /// (that's carried by [`PhraseSummary::score_position`]). Returns `None`
    /// when no score follower is attached (free play). Used to drive a live
    /// cursor between phrase boundaries.
    pub fn current_score_position(&self) -> Option<ScorePosition> {
        self.score_follower.as_ref().map(|f| f.current_position())
    }

    /// Push an audio event into the aggregator.
    ///
    /// The aggregator checks for phrase boundaries (silence gaps and score measure
    /// boundaries) and automatically closes the current phrase when detected.
    pub fn push(&mut self, event: &AudioEvent) {
        /// Tolerance for floating-point timestamp comparison (1 µs).
        /// Prevents false phrase splits from IEEE-754 rounding,
        /// e.g. `0.4 - 0.1 = 0.30000000000000004 > 0.3`.
        const GAP_EPSILON: f64 = 1e-6;

        let is_voiced = event.pitch_hz.is_some() && event.confidence > 0.5;

        if is_voiced {
            // Check for score measure boundaries if a score is loaded
            let aligned_position = if let Some(follower) = &mut self.score_follower {
                let pos = follower.align(event);
                if let Some(last_measure) = self.last_score_measure {
                    if pos.measure_number > last_measure && !self.current_phrase_events.is_empty() {
                        self.close_current_phrase();
                    }
                }
                self.last_score_measure = Some(pos.measure_number);
                Some(pos)
            } else {
                None
            };

            // Check if there's been a silence gap since the last voiced event
            if let Some(last_time) = self.last_voiced_time {
                let gap = event.timestamp_secs - last_time;
                if gap - self.config.silence_gap_secs > GAP_EPSILON
                    && !self.current_phrase_events.is_empty()
                {
                    self.close_current_phrase();
                }
            }

            // Start a new phrase if needed. Capture the score position once
            // per phrase, at the first aligned event — that's the anchor we
            // surface in PhraseSummary::score_position.
            if self.phrase_start_time.is_none() {
                self.phrase_start_time = Some(event.timestamp_secs);
                self.current_phrase_start_position = aligned_position;
            }

            self.last_voiced_time = Some(event.timestamp_secs);
            self.current_phrase_events.push(event.clone());
        }
    }

    /// Flush the aggregator, closing the current phrase if one is in progress.
    ///
    /// Call this at the end of a practice session to ensure the final
    /// phrase is captured.
    pub fn flush(&mut self) {
        if !self.current_phrase_events.is_empty() {
            self.close_current_phrase();
        }
        self.clear_score_follower();
    }

    /// Get all completed phrases.
    pub fn phrases(&self) -> &[PhraseSummary] {
        &self.phrases
    }

    /// Drain newly completed phrases since the last call to this method.
    ///
    /// Useful for streaming phrase summaries to the UI as they complete.
    pub fn take_new_phrases(&mut self) -> Vec<PhraseSummary> {
        let start = self.new_phrases_start;
        self.new_phrases_start = self.phrases.len();
        self.phrases[start..].to_vec()
    }

    /// Close the current phrase, computing its summary.
    fn close_current_phrase(&mut self) {
        let events = &self.current_phrase_events;

        // Discard phrases that are too short
        if events.len() < self.config.min_phrase_events {
            self.current_phrase_events.clear();
            self.phrase_start_time = None;
            self.current_phrase_start_position = None;
            return;
        }

        let start_time = self.phrase_start_time.unwrap_or(0.0);
        let end_time = events
            .last()
            .map(|e| e.timestamp_secs)
            .unwrap_or(start_time);

        let pitch_stats = compute_pitch_stats(events);
        let dynamics = compute_dynamics_stats(events);
        let stability = compute_stability(events);

        let summary = PhraseSummary {
            phrase_index: self.next_phrase_index,
            start_time,
            end_time,
            duration_secs: end_time - start_time,
            note_count: events.len(),
            pitch_stats,
            dynamics,
            stability,
            score_position: self.current_phrase_start_position.take(),
            // Tone is attached downstream (the aggregator has no raw audio).
            tone: None,
        };

        self.phrases.push(summary);
        self.next_phrase_index += 1;
        self.current_phrase_events.clear();
        self.phrase_start_time = None;
    }
}

/// Compute pitch statistics from a slice of audio events.
fn compute_pitch_stats(events: &[AudioEvent]) -> PitchStats {
    let pitches: Vec<f64> = events.iter().filter_map(|e| e.pitch_hz).collect();

    if pitches.is_empty() {
        return PitchStats {
            mean_hz: 0.0,
            min_hz: 0.0,
            max_hz: 0.0,
            range_cents: 0.0,
            pitches,
        };
    }

    let sum: f64 = pitches.iter().sum();
    let mean_hz = sum / pitches.len() as f64;
    let min_hz = pitches.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_hz = pitches.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range_cents = hz_to_cents(min_hz, max_hz).abs();

    PitchStats {
        mean_hz,
        min_hz,
        max_hz,
        range_cents,
        pitches,
    }
}

/// Compute dynamics (amplitude) statistics from a slice of audio events.
fn compute_dynamics_stats(events: &[AudioEvent]) -> DynamicsStats {
    if events.is_empty() {
        return DynamicsStats {
            mean_amplitude: 0.0,
            min_amplitude: 0.0,
            max_amplitude: 0.0,
            dynamic_range: 0.0,
        };
    }

    let amplitudes: Vec<f64> = events.iter().map(|e| e.amplitude).collect();
    let sum: f64 = amplitudes.iter().sum();
    let mean_amplitude = sum / amplitudes.len() as f64;
    let min_amplitude = amplitudes.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_amplitude = amplitudes.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    DynamicsStats {
        mean_amplitude,
        min_amplitude,
        max_amplitude,
        dynamic_range: max_amplitude - min_amplitude,
    }
}

/// Compute pitch stability as a 0.0-1.0 score.
///
/// Stability is measured as the inverse of the coefficient of variation
/// of the pitch values. A perfectly constant pitch yields 1.0.
fn compute_stability(events: &[AudioEvent]) -> f64 {
    let pitches: Vec<f64> = events.iter().filter_map(|e| e.pitch_hz).collect();

    if pitches.len() < 2 {
        return 1.0; // Single note is perfectly "stable"
    }

    let mean: f64 = pitches.iter().sum::<f64>() / pitches.len() as f64;
    if mean <= 0.0 {
        return 0.0;
    }

    let variance: f64 =
        pitches.iter().map(|&p| (p - mean).powi(2)).sum::<f64>() / pitches.len() as f64;
    let std_dev = variance.sqrt();

    // Coefficient of variation (ratio of std_dev to mean)
    let cv = std_dev / mean;

    // Map CV to a 0.0-1.0 score. A CV of 0 = perfect stability (1.0).
    // A CV of 0.1 (~170 cents) or more = very unstable (0.0).
    (1.0 - (cv / 0.1)).clamp(0.0, 1.0)
}

/// Convert the interval between two frequencies to cents.
///
/// Returns positive if `b > a`, negative if `b < a`.
/// One semitone = 100 cents, one octave = 1200 cents.
pub fn hz_to_cents(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    1200.0 * (b / a).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a voiced AudioEvent.
    fn voiced_event(pitch_hz: f64, amplitude: f64, timestamp_secs: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: Some(pitch_hz),
            confidence: 0.95,
            amplitude,
            timestamp_secs,
            is_onset: false,
        }
    }

    /// Helper to create a silence AudioEvent.
    fn silence_event(timestamp_secs: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: None,
            confidence: 0.0,
            amplitude: 0.001,
            timestamp_secs,
            is_onset: false,
        }
    }

    // --- hz_to_cents tests ---

    #[test]
    fn hz_to_cents_octave() {
        let cents = hz_to_cents(220.0, 440.0);
        assert!(
            (cents - 1200.0).abs() < 0.01,
            "Octave should be 1200 cents, got {cents}"
        );
    }

    #[test]
    fn hz_to_cents_unison() {
        let cents = hz_to_cents(440.0, 440.0);
        assert!(cents.abs() < 0.01, "Unison should be 0 cents, got {cents}");
    }

    #[test]
    fn hz_to_cents_semitone() {
        // A4 to A#4
        let a4 = 440.0;
        let a_sharp4 = 440.0 * 2.0_f64.powf(1.0 / 12.0);
        let cents = hz_to_cents(a4, a_sharp4);
        assert!(
            (cents - 100.0).abs() < 0.01,
            "Semitone should be 100 cents, got {cents}"
        );
    }

    #[test]
    fn hz_to_cents_negative() {
        let cents = hz_to_cents(440.0, 220.0);
        assert!(
            (cents - (-1200.0)).abs() < 0.01,
            "Downward octave should be -1200 cents"
        );
    }

    #[test]
    fn hz_to_cents_zero_input() {
        assert_eq!(hz_to_cents(0.0, 440.0), 0.0);
        assert_eq!(hz_to_cents(440.0, 0.0), 0.0);
        assert_eq!(hz_to_cents(0.0, 0.0), 0.0);
    }

    // --- Silence gap splits phrases ---

    #[test]
    fn silence_gap_splits_phrases() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // Phrase 1: events at t=0.0, 0.05, 0.10
        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.push(&voiced_event(440.0, 0.8, 0.10));

        // Gap of 0.5s (> 0.3s threshold)
        // Phrase 2: events at t=0.60, 0.65
        agg.push(&voiced_event(330.0, 0.6, 0.60));
        agg.push(&voiced_event(330.0, 0.6, 0.65));

        agg.flush();

        let phrases = agg.phrases();
        assert_eq!(
            phrases.len(),
            2,
            "Should have 2 phrases, got {}",
            phrases.len()
        );
        assert_eq!(phrases[0].phrase_index, 0);
        assert_eq!(phrases[1].phrase_index, 1);
        assert_eq!(phrases[0].note_count, 3);
        assert_eq!(phrases[1].note_count, 2);
    }

    // --- Continuous events form a single phrase ---

    #[test]
    fn continuous_events_form_single_phrase() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // All events within 0.3s of each other
        for i in 0..10 {
            agg.push(&voiced_event(440.0, 0.8, i as f64 * 0.05));
        }
        agg.flush();

        let phrases = agg.phrases();
        assert_eq!(phrases.len(), 1, "Continuous events should form 1 phrase");
        assert_eq!(phrases[0].note_count, 10);
    }

    // --- Flush closes current phrase ---

    #[test]
    fn flush_closes_current_phrase() {
        let config = PhraseConfig::default();
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.push(&voiced_event(440.0, 0.8, 0.10));

        // Before flush, no phrases are completed
        assert_eq!(agg.phrases().len(), 0);

        agg.flush();

        // After flush, the phrase is completed
        assert_eq!(agg.phrases().len(), 1);
    }

    // --- Pitch stats calculation ---

    #[test]
    fn pitch_stats_calculation() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(220.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.push(&voiced_event(330.0, 0.8, 0.10));

        agg.flush();

        let phrase = &agg.phrases()[0];
        let stats = &phrase.pitch_stats;

        assert!((stats.min_hz - 220.0).abs() < 0.01);
        assert!((stats.max_hz - 440.0).abs() < 0.01);
        assert!((stats.mean_hz - 330.0).abs() < 0.01);
        assert!(stats.range_cents > 0.0);
        assert_eq!(stats.pitches.len(), 3);

        // 220 -> 440 = 1200 cents
        assert!((stats.range_cents - 1200.0).abs() < 1.0);
    }

    // --- Dynamics stats calculation ---

    #[test]
    fn dynamics_stats_calculation() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(440.0, 0.2, 0.0));
        agg.push(&voiced_event(440.0, 0.6, 0.05));
        agg.push(&voiced_event(440.0, 1.0, 0.10));

        agg.flush();

        let phrase = &agg.phrases()[0];
        let dyn_stats = &phrase.dynamics;

        assert!((dyn_stats.min_amplitude - 0.2).abs() < 0.001);
        assert!((dyn_stats.max_amplitude - 1.0).abs() < 0.001);
        assert!((dyn_stats.mean_amplitude - 0.6).abs() < 0.001);
        assert!((dyn_stats.dynamic_range - 0.8).abs() < 0.001);
    }

    // --- Short phrases are discarded ---

    #[test]
    fn short_phrases_discarded() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // Only 2 events, but min_phrase_events is 3
        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.flush();

        assert_eq!(
            agg.phrases().len(),
            0,
            "2-event phrase should be discarded with min=3"
        );
    }

    // --- take_new_phrases drains correctly ---

    #[test]
    fn take_new_phrases_returns_only_new() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // Create first phrase
        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.push(&voiced_event(440.0, 0.8, 0.10));

        // Gap + second phrase
        agg.push(&voiced_event(330.0, 0.6, 0.60));
        agg.push(&voiced_event(330.0, 0.6, 0.65));
        agg.flush();

        // First drain: should get 2 phrases
        let new1 = agg.take_new_phrases();
        assert_eq!(new1.len(), 2);

        // Second drain: should get 0 phrases
        let new2 = agg.take_new_phrases();
        assert_eq!(new2.len(), 0);

        // Create a third phrase
        agg.push(&voiced_event(550.0, 0.7, 2.0));
        agg.push(&voiced_event(550.0, 0.7, 2.05));
        agg.flush();

        // Third drain: should get 1 phrase
        let new3 = agg.take_new_phrases();
        assert_eq!(new3.len(), 1);
        assert_eq!(new3[0].phrase_index, 2);
    }

    // --- Stability calculation ---

    #[test]
    fn stable_pitch_has_high_stability() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // All events at the same pitch
        for i in 0..5 {
            agg.push(&voiced_event(440.0, 0.8, i as f64 * 0.05));
        }
        agg.flush();

        let phrase = &agg.phrases()[0];
        assert!(
            phrase.stability > 0.99,
            "Constant pitch should have near-perfect stability, got {}",
            phrase.stability
        );
    }

    #[test]
    fn varying_pitch_has_lower_stability() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        // Very different pitches
        agg.push(&voiced_event(200.0, 0.8, 0.0));
        agg.push(&voiced_event(400.0, 0.8, 0.05));
        agg.push(&voiced_event(200.0, 0.8, 0.10));
        agg.push(&voiced_event(400.0, 0.8, 0.15));
        agg.flush();

        let phrase = &agg.phrases()[0];
        assert!(
            phrase.stability < 0.5,
            "Widely varying pitch should have low stability, got {}",
            phrase.stability
        );
    }

    // --- Silence events are ignored ---

    #[test]
    fn silence_events_are_ignored() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&silence_event(0.05));
        agg.push(&silence_event(0.10));
        agg.push(&voiced_event(440.0, 0.8, 0.15));
        agg.flush();

        let phrases = agg.phrases();
        assert_eq!(phrases.len(), 1);
        // Only voiced events counted
        assert_eq!(phrases[0].note_count, 2);
    }

    // --- Config validation ---

    #[test]
    fn invalid_silence_gap_rejected() {
        let config = PhraseConfig {
            silence_gap_secs: -1.0,
            min_phrase_events: 3,
        };
        assert!(PhraseAggregator::new(config).is_err());
    }

    #[test]
    fn zero_min_events_rejected() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 0,
        };
        assert!(PhraseAggregator::new(config).is_err());
    }

    // --- Phrase timing ---

    #[test]
    fn phrase_timing_is_correct() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(440.0, 0.8, 1.0));
        agg.push(&voiced_event(440.0, 0.8, 1.1));
        agg.push(&voiced_event(440.0, 0.8, 1.2));
        agg.flush();

        let phrase = &agg.phrases()[0];
        assert!((phrase.start_time - 1.0).abs() < 0.001);
        assert!((phrase.end_time - 1.2).abs() < 0.001);
        assert!((phrase.duration_secs - 0.2).abs() < 0.001);
    }

    // --- PhraseSummary serialization ---

    #[test]
    fn phrase_summary_serialization_roundtrip() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        agg.push(&voiced_event(440.0, 0.8, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05));
        agg.push(&voiced_event(440.0, 0.8, 0.10));
        agg.flush();

        let phrase = &agg.phrases()[0];
        let json = serde_json::to_string(phrase).unwrap();
        let parsed: PhraseSummary = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.phrase_index, phrase.phrase_index);
        assert!((parsed.start_time - phrase.start_time).abs() < f64::EPSILON);
        assert_eq!(parsed.note_count, phrase.note_count);
    }

    // --- current_score_position tests ---

    #[test]
    fn current_score_position_is_none_without_follower() {
        let agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        assert!(
            agg.current_score_position().is_none(),
            "free play (no follower) must report no position"
        );
    }

    #[test]
    fn current_score_position_tracks_latest_alignment() {
        // Attach a follower built from the real C-major-scale fixture, then
        // push a voiced C4. The live position must reflect that alignment —
        // this is what drives the cursor between phrase boundaries.
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let follower = ScoreFollower::from_musicxml_str(xml, 0).expect("fixture parses");

        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        agg.set_score_follower(follower);

        // Before any event, position exists but sits at the start.
        assert!(agg.current_score_position().is_some());

        agg.push(&voiced_event(261.63, 0.5, 0.0)); // C4
        let pos = agg
            .current_score_position()
            .expect("a follower is attached");
        assert_eq!(
            pos.measure_number, 1,
            "first aligned note should put the live cursor in measure 1"
        );
    }

    #[test]
    fn current_score_position_cleared_with_follower() {
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let follower = ScoreFollower::from_musicxml_str(xml, 0).unwrap();
        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        agg.set_score_follower(follower);
        assert!(agg.current_score_position().is_some());

        agg.clear_score_follower();
        assert!(
            agg.current_score_position().is_none(),
            "clearing the follower must drop the live position"
        );
    }

    #[test]
    fn phrase_summary_tone_is_additive_and_backward_compatible() {
        // A phrase JSON serialised before `tone` existed (field absent) must
        // still deserialize, with `tone` defaulting to None.
        let legacy = r#"{
            "phrase_index": 0, "start_time": 0.0, "end_time": 1.0,
            "duration_secs": 1.0, "note_count": 3,
            "pitch_stats": {"mean_hz":440.0,"min_hz":435.0,"max_hz":445.0,"range_cents":40.0,"pitches":[440.0]},
            "dynamics": {"mean_amplitude":0.5,"min_amplitude":0.4,"max_amplitude":0.6,"dynamic_range":0.2},
            "stability": 0.8, "score_position": null
        }"#;
        let p: PhraseSummary = serde_json::from_str(legacy).expect("legacy phrase deserializes");
        assert!(p.tone.is_none(), "absent tone defaults to None");

        // And a phrase carrying tone round-trips.
        let with_tone = PhraseSummary {
            tone: Some(tone::ToneDescriptor {
                brightness: 0.6,
                warmth: 0.5,
                air_noise: 0.2,
                core_clarity: 0.8,
                vibrato_quality: 0.5,
            }),
            ..p
        };
        let json = serde_json::to_string(&with_tone).unwrap();
        let back: PhraseSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(with_tone, back);
    }
}
