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
use crate::perception::{pitch_class_of, NoteGate, MIN_PITCH_CONFIDENCE};
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
    #[error("voiced_confidence_threshold must be in (0, 1], got {0}")]
    InvalidConfidenceThreshold(f64),
}

/// Configuration for phrase boundary detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseConfig {
    /// Minimum silence duration (seconds) to split phrases. Default: 0.3 (300ms).
    pub silence_gap_secs: f64,
    /// Minimum number of voiced events to form a valid phrase. Default: 3.
    pub min_phrase_events: usize,
    /// Minimum pitch-detection confidence for an event to count as **voiced**
    /// (and therefore as practice). Default: 0.5.
    ///
    /// This is **per-instrument**: breathy, vibrato-rich voice detects at lower
    /// confidence than, say, a piano, so a fixed 0.5 gate silently dropped whole
    /// sung sessions (they formed no phrases → "you didn't play", #185). The
    /// Tauri shell sets this from the active instrument profile
    /// (`voiced_confidence_threshold`); Voice uses a lower value.
    pub voiced_confidence_threshold: f64,
}

impl Default for PhraseConfig {
    fn default() -> Self {
        Self {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
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
        if !(self.voiced_confidence_threshold.is_finite()
            && self.voiced_confidence_threshold > 0.0
            && self.voiced_confidence_threshold <= 1.0)
        {
            return Err(PhraseError::InvalidConfidenceThreshold(
                self.voiced_confidence_threshold,
            ));
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

/// Per-phrase tally of the follower's note verdicts (#337 S3): how the
/// notes judged DURING this phrase went. `Default` = all zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PhraseVerdicts {
    pub hit: usize,
    pub near: usize,
    pub missed: usize,
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
    /// Rolling key/mode estimate as of this phrase (Phase 4). Carried from a
    /// session-long [`theory::KeyTracker`] fed per NOTE through the same
    /// [`crate::perception::NoteGate`] discipline as the live "I hear" strip,
    /// so it tracks modulation without flapping and the recap's key vote sees
    /// the calm readings the player watched (#316 / #324). `None` until
    /// enough pitch evidence accumulates.
    ///
    /// Key evidence gates at the strip's fixed confidence bar
    /// ([`crate::perception::MIN_PITCH_CONFIDENCE`]), NOT the per-instrument
    /// voiced threshold: a breathy sung session that detects below the bar
    /// still forms phrases (#185) but stays keyless — the strip never showed
    /// a key for it, and the recap must not out-claim the strip (#316).
    /// Additive + `serde(default)`.
    #[serde(default)]
    pub key: Option<theory::KeyEstimate>,
    /// Onset timestamps (seconds from session start) of the events in this
    /// phrase, extracted via [`groove::onsets_from_events`]. Retained so the
    /// session recap can compute a [`groove::GrooveDescriptor`] (tempo, swing,
    /// timing) over the whole session's onsets. Empty when no event in the
    /// phrase carried an onset flag. Additive + `serde(default)` so older
    /// persisted phrases deserialize cleanly.
    #[serde(default)]
    pub onsets_secs: Vec<f64>,
    /// Measures this phrase spanned — (first, last) from the follower's
    /// positions bracketing it (#337 S3). `None` in free play or when the
    /// follower never aligned during the phrase. Additive + `serde(default)`.
    #[serde(default)]
    pub score_span: Option<(usize, usize)>,
    /// Verdict tally for the notes judged during this phrase (#337 S3).
    /// `None` in free play. Additive + `serde(default)`.
    #[serde(default)]
    pub verdicts: Option<PhraseVerdicts>,
    /// Ready-to-show card line for score sessions (#337 S3, closes #210):
    /// "Measures 5–8 — 6 clean, 1 rough, 2 missed". Built HERE so the
    /// frontend renders text, never derives it (house rule: no business
    /// logic in the frontend). `None` when there's nothing honest to say
    /// (free play, or no notes were judged). Additive + `serde(default)`.
    #[serde(default)]
    pub score_card: Option<String>,
}

/// What a phrase-closing boundary means for the note pending in the key
/// gate: a silence gap (or session end) ended the note too — drain it into
/// the tracker so it counts in the closing phrase's snapshot; a measure
/// boundary may cut straight through a held note — keep it pending as ONE
/// note, exactly as the live strip hears it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NoteGateAtClose {
    Drain,
    Keep,
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
    /// Verdicts drained from the follower but not yet handed to the
    /// pipeline via [`Self::take_note_verdicts`] (#337 S2/S3).
    verdicts_out: Vec<crate::follower::NoteVerdict>,
    /// Verdict tally for the phrase currently being assembled (#337 S3).
    phrase_verdicts: PhraseVerdicts,
    /// Last measure the follower reported while THIS phrase was open — the
    /// end of its score span.
    phrase_end_measure: Option<usize>,
    /// Score position captured at the first aligned event of the current
    /// phrase. Carried into the emitted [`PhraseSummary::score_position`] so
    /// the LLM and OSMD cursor can anchor feedback to where the phrase began.
    current_phrase_start_position: Option<ScorePosition>,
    /// Session-long rolling key/mode tracker behind each phrase's `key`
    /// snapshot. Fed one duration-weighted observation per NOTE — the
    /// tracker's documented contract. Feeding it raw frames collapses its
    /// rolling window to under a second of audio, and the recap then votes
    /// over wandering readings the calm strip never showed, hedging even a
    /// rock-steady session (#316 / #324).
    key_tracker: theory::KeyTracker,
    /// Frame→note segmentation in front of `key_tracker` — the same gate the
    /// live strip runs (see [`NoteGate`]), so both trackers hear the session
    /// the same way.
    note_gate: NoteGate,
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
            verdicts_out: Vec::new(),
            phrase_verdicts: PhraseVerdicts::default(),
            phrase_end_measure: None,
            current_phrase_start_position: None,
            key_tracker: theory::KeyTracker::new(),
            note_gate: NoteGate::default(),
        })
    }

    /// Swap the voiced-confidence gate mid-session — the per-instrument half
    /// of a profile reconfigure (#521). The gate judges subsequent `push`es
    /// only; events already buffered in the open phrase keep the verdict they
    /// got. On an invalid value the previous gate stays in force, mirroring
    /// the pipeline's keep-previous-detector contract for bad profile data.
    pub fn set_voiced_confidence_threshold(&mut self, threshold: f64) -> Result<(), PhraseError> {
        let candidate = PhraseConfig {
            voiced_confidence_threshold: threshold,
            ..self.config.clone()
        };
        candidate.validate()?;
        self.config = candidate;
        Ok(())
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

    /// Drain the note verdicts the follower produced since the last call
    /// (#337 S2) — empty in free play or when the follower judged nothing.
    pub fn take_note_verdicts(&mut self) -> Vec<crate::follower::NoteVerdict> {
        std::mem::take(&mut self.verdicts_out)
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

        let is_voiced =
            event.pitch_hz.is_some() && event.confidence > self.config.voiced_confidence_threshold;

        if is_voiced {
            // Check for score measure boundaries if a score is loaded.
            // A measure boundary does NOT drain the note gate: the player may
            // be holding one note straight across it, and force-splitting a
            // tied pedal tone per measure would re-enter it as a fresh
            // (drone-capped) note every time — letting it outweigh the whole
            // rolling profile the cap protects.
            let aligned_position = if let Some(follower) = &mut self.score_follower {
                let pos = follower.align(event);
                let fresh = follower.take_verdicts();
                // Close the old phrase at a measure boundary BEFORE folding
                // this event's verdicts/measure in — otherwise every card
                // overstates its span by the next measure and boundary-frame
                // verdicts land in the phrase they didn't come from
                // (S3 review MUST-FIX 1).
                if let Some(last_measure) = self.last_score_measure {
                    if pos.measure_number > last_measure && !self.current_phrase_events.is_empty() {
                        self.close_current_phrase(NoteGateAtClose::Keep);
                    }
                }
                // Route fresh verdicts to the (possibly new) phrase's tally
                // AND the pipeline's out-buffer (#337 S3) — the phrase card
                // and the live strip must count the same judgments.
                for verdict in fresh {
                    match verdict.verdict {
                        crate::follower::Verdict::Hit => self.phrase_verdicts.hit += 1,
                        crate::follower::Verdict::Near => self.phrase_verdicts.near += 1,
                        crate::follower::Verdict::Missed => self.phrase_verdicts.missed += 1,
                    }
                    self.verdicts_out.push(verdict);
                }
                self.phrase_end_measure = Some(pos.measure_number);
                self.last_score_measure = Some(pos.measure_number);
                Some(pos)
            } else {
                None
            };

            // Check if there's been a silence gap since the last voiced event.
            // A silence gap means the previous phrase's final note truly
            // ended — drain it into the tracker so it counts in that
            // phrase's key snapshot.
            if let Some(last_time) = self.last_voiced_time {
                let gap = event.timestamp_secs - last_time;
                if gap - self.config.silence_gap_secs > GAP_EPSILON
                    && !self.current_phrase_events.is_empty()
                {
                    self.close_current_phrase(NoteGateAtClose::Drain);
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

        // Key evidence rides the strip's own gate: every confident pitched
        // frame, segmented into duration-weighted notes. Fed after the
        // boundary handling above so a gap-opening event starts its note in
        // the new phrase, not as a one-frame remnant drained into the old
        // one. Gated at the strip's fixed confidence bar (NOT the
        // per-instrument voiced threshold) deliberately — see the note on
        // `PhraseSummary::key`.
        if let Some(hz) = event.pitch_hz {
            if event.confidence >= MIN_PITCH_CONFIDENCE && hz > 0.0 {
                if let Some(pc) = pitch_class_of(hz as f32) {
                    if let Some((note_pc, weight)) =
                        self.note_gate.observe(pc, event.timestamp_secs)
                    {
                        self.key_tracker.observe_pc(note_pc, weight);
                    }
                }
            }
        }
    }

    /// Flush the aggregator, closing the current phrase if one is in progress.
    ///
    /// Call this at the end of a practice session to ensure the final
    /// phrase is captured.
    pub fn flush(&mut self) {
        if !self.current_phrase_events.is_empty() {
            self.close_current_phrase(NoteGateAtClose::Drain);
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

    /// Close the current phrase, computing its summary. `gate` says whether
    /// the closing boundary also ended the player's current note (a silence
    /// gap / session end did; a measure boundary may cut through a held one).
    fn close_current_phrase(&mut self, gate: NoteGateAtClose) {
        let events = &self.current_phrase_events;

        // Discard phrases that are too short
        if events.len() < self.config.min_phrase_events {
            self.current_phrase_events.clear();
            self.phrase_start_time = None;
            self.current_phrase_start_position = None;
            // A discarded fragment's tally goes with it: folding it into the
            // NEXT phrase would attribute its verdicts to measures they
            // didn't come from (S3 review MUST-FIX 2). The session recap
            // still counts them — it reads per-verdict measure numbers from
            // its own buffer, not the phrase tallies.
            self.phrase_verdicts = PhraseVerdicts::default();
            self.phrase_end_measure = None;
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

        // Retain onset timestamps so the session recap can analyse groove
        // (tempo / swing / timing) across the whole session. Only events
        // flagged `is_onset` contribute — `groove::onsets_from_events` does
        // the filtering, keeping the onset definition in one place.
        let onsets_secs = groove::onsets_from_events(events);

        // When the boundary ended the final note (silence / session end),
        // land it before reading the snapshot, or that note would miss this
        // phrase's key (and the session's last phrase would lose it
        // entirely). Notes were fed per event in `push`; see `note_gate`.
        if gate == NoteGateAtClose::Drain {
            if let Some((pc, weight)) = self.note_gate.drain() {
                self.key_tracker.observe_pc(pc, weight);
            }
        }
        let key = self.key_tracker.current();

        // Score anchoring (#337 S3): the span runs from the position the
        // phrase began on to the last measure the follower reported while it
        // was open. The card only speaks when at least one note was judged —
        // a phrase the follower never scored says nothing (silence > lies).
        let start_position = self.current_phrase_start_position.take();
        let verdicts = std::mem::take(&mut self.phrase_verdicts);
        let judged = verdicts.hit + verdicts.near + verdicts.missed;
        let score_span = start_position.as_ref().map(|p| {
            let start = p.measure_number;
            let end = self.phrase_end_measure.unwrap_or(start).max(start);
            (start, end)
        });
        self.phrase_end_measure = None;
        let score_card = match (score_span, judged) {
            (Some(span), j) if j > 0 => Some(score_phrase_card(span, verdicts)),
            _ => None,
        };

        let summary = PhraseSummary {
            phrase_index: self.next_phrase_index,
            start_time,
            end_time,
            duration_secs: end_time - start_time,
            note_count: events.len(),
            pitch_stats,
            dynamics,
            stability,
            score_position: start_position,
            // Tone is attached downstream (the aggregator has no raw audio).
            tone: None,
            key,
            onsets_secs,
            score_span,
            verdicts: score_span.map(|_| verdicts),
            score_card,
        };

        self.phrases.push(summary);
        self.next_phrase_index += 1;
        self.current_phrase_events.clear();
        self.phrase_start_time = None;
    }
}

/// The score-session phrase card line (#337 S3, closes #210): plain words,
/// measure-anchored, counting only what the follower actually judged.
fn score_phrase_card(span: (usize, usize), v: PhraseVerdicts) -> String {
    let measures = if span.0 == span.1 {
        format!("Measure {}", span.0)
    } else {
        format!("Measures {}–{}", span.0, span.1)
    };
    let mut parts: Vec<String> = Vec::new();
    if v.hit > 0 {
        parts.push(format!("{} clean", v.hit));
    }
    if v.near > 0 {
        parts.push(format!("{} rough", v.near));
    }
    if v.missed > 0 {
        parts.push(format!("{} missed", v.missed));
    }
    format!("{measures} — {}", parts.join(", "))
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
            note_info: None,
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
            note_info: None,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
        };
        assert!(PhraseAggregator::new(config).is_err());
    }

    #[test]
    fn zero_min_events_rejected() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 0,
            voiced_confidence_threshold: 0.5,
        };
        assert!(PhraseAggregator::new(config).is_err());
    }

    #[test]
    fn invalid_confidence_threshold_rejected() {
        for bad in [0.0, -0.1, 1.5, f64::NAN] {
            let config = PhraseConfig {
                silence_gap_secs: 0.3,
                min_phrase_events: 2,
                voiced_confidence_threshold: bad,
            };
            assert!(
                PhraseAggregator::new(config).is_err(),
                "threshold {bad} must be rejected"
            );
        }
    }

    #[test]
    fn lower_threshold_lets_low_confidence_voice_count_as_practice() {
        // Breathy / vibrato-rich singing often detects below the default 0.5
        // gate. With a fixed gate those events were silently dropped, so a sung
        // session formed no phrases and the recap said "you didn't play" (#185).
        let breathy = |hz: f64, t: f64| AudioEvent {
            pitch_hz: Some(hz),
            confidence: 0.4, // below default 0.5, above a voice gate of 0.3
            amplitude: 0.3,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        };

        // Default gate (0.5): nothing voiced → no phrases.
        let mut strict = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        for i in 0..5 {
            strict.push(&breathy(220.0, i as f64 * 0.05));
        }
        strict.flush();
        assert_eq!(
            strict.phrases().len(),
            0,
            "the default 0.5 gate drops a quietly-sung session"
        );

        // Voice gate (0.3): the same singing now registers as a phrase.
        let mut voice = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.3,
        })
        .unwrap();
        for i in 0..5 {
            voice.push(&breathy(220.0, i as f64 * 0.05));
        }
        voice.flush();
        assert_eq!(
            voice.phrases().len(),
            1,
            "a lower voice gate counts the singing as practice"
        );
        assert_eq!(voice.phrases()[0].note_count, 5);
    }

    #[test]
    fn loosened_gate_counts_previously_subvoiced_events() {
        // #521: a Trumpet→Voice switch mid-session. Same breathy 0.4-confidence
        // singing throughout; only the gate changes. Events before the switch
        // must NOT be retroactively re-judged — the Voice segment starts
        // counting, the Trumpet segment's verdicts stand.
        let breathy = |t: f64| AudioEvent {
            pitch_hz: Some(220.0),
            confidence: 0.4,
            amplitude: 0.3,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        };
        let mut agg = PhraseAggregator::new(PhraseConfig {
            voiced_confidence_threshold: 0.5,
            ..PhraseConfig::default()
        })
        .unwrap();
        for i in 0..5 {
            agg.push(&breathy(i as f64 * 0.05));
        }
        agg.set_voiced_confidence_threshold(0.3).unwrap();
        for i in 0..5 {
            agg.push(&breathy(1.0 + i as f64 * 0.05));
        }
        agg.flush();
        assert_eq!(
            agg.phrases().len(),
            1,
            "singing after the switch must form a phrase under the new gate"
        );
        assert_eq!(
            agg.phrases()[0].note_count,
            5,
            "only post-switch events count — the old gate's verdicts stand"
        );
        assert!(
            agg.phrases()[0].start_time >= 1.0,
            "the phrase must start at the first post-switch event, not be \
             back-dated to pre-switch audio"
        );
    }

    #[test]
    fn tightened_gate_stops_counting_borderline_events_as_voiced() {
        // The reverse switch (Voice→Trumpet, #521): the loose 0.3 gate must not
        // survive and keep counting low-confidence noise as practice.
        let breathy = |t: f64| AudioEvent {
            pitch_hz: Some(220.0),
            confidence: 0.4,
            amplitude: 0.3,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        };
        let mut agg = PhraseAggregator::new(PhraseConfig {
            voiced_confidence_threshold: 0.3,
            ..PhraseConfig::default()
        })
        .unwrap();
        for i in 0..5 {
            agg.push(&breathy(i as f64 * 0.05));
        }
        agg.set_voiced_confidence_threshold(0.5).unwrap();
        for i in 0..5 {
            agg.push(&breathy(1.0 + i as f64 * 0.05));
        }
        agg.flush();
        assert_eq!(agg.phrases().len(), 1, "only the pre-switch phrase exists");
        assert_eq!(
            agg.phrases()[0].note_count,
            5,
            "post-switch 0.4-confidence events must not extend the phrase \
             once the gate is 0.5"
        );
    }

    #[test]
    fn set_voiced_gate_rejects_invalid_values_and_keeps_the_old_gate() {
        let breathy = |t: f64| AudioEvent {
            pitch_hz: Some(220.0),
            confidence: 0.4,
            amplitude: 0.3,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        };
        let mut agg = PhraseAggregator::new(PhraseConfig {
            voiced_confidence_threshold: 0.3,
            ..PhraseConfig::default()
        })
        .unwrap();
        for bad in [0.0, -0.2, 1.5, f64::NAN] {
            match agg.set_voiced_confidence_threshold(bad) {
                Err(PhraseError::InvalidConfidenceThreshold(got)) => assert!(
                    got == bad || (got.is_nan() && bad.is_nan()),
                    "error must carry the offending value: got {got}, fed {bad}"
                ),
                other => panic!("expected typed rejection for {bad}, got {other:?}"),
            }
        }
        // The 0.3 gate is still in force: borderline singing still counts.
        for i in 0..3 {
            agg.push(&breathy(i as f64 * 0.05));
        }
        agg.flush();
        assert_eq!(
            agg.phrases().len(),
            1,
            "a rejected gate must leave the previous gate judging events"
        );
    }

    // --- Phrase timing ---

    #[test]
    fn phrase_timing_is_correct() {
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
            voiced_confidence_threshold: 0.5,
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
            voiced_confidence_threshold: 0.5,
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

    /// #337 S3 (closes #210): a score session's phrase carries its measure
    /// span, verdict tally, and a ready-to-show card naming the measures.
    /// Fails if the span/tally stop riding the phrase or the card copy
    /// loses its measure anchor.
    #[test]
    fn score_session_phrases_carry_span_verdicts_and_card() {
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let follower = ScoreFollower::from_musicxml_str(xml, 0).expect("fixture parses");
        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        agg.set_score_follower(follower);

        // Play the opening notes in tune, then a long silence closes the
        // phrase (drain path).
        for (i, hz) in [261.63, 293.66, 329.63, 349.23].iter().enumerate() {
            for f in 0..4 {
                agg.push(&voiced_event(*hz, 0.9, i as f64 * 0.4 + f64::from(f) * 0.1));
            }
        }
        agg.flush();

        let phrases = agg.phrases();
        assert!(!phrases.is_empty(), "the playing formed a phrase");
        let p = &phrases[0];
        let span = p.score_span.expect("score session phrases carry a span");
        assert!(span.0 >= 1 && span.1 >= span.0, "sane span: {span:?}");
        let v = p.verdicts.expect("verdict tally rides the phrase");
        assert!(
            v.hit + v.near + v.missed > 0,
            "in-tune playing judged something: {v:?}"
        );
        let card = p.score_card.as_ref().expect("a judged phrase has a card");
        assert!(
            card.starts_with("Measure"),
            "card anchors to measures: {card}"
        );
        assert!(
            card.contains("clean") || card.contains("rough") || card.contains("missed"),
            "card counts verdicts in plain words: {card}"
        );
    }

    /// S3 review MUST-FIX 1: a phrase closed at a measure boundary spans
    /// only the measures it CONTAINED — measure 1's card must not claim
    /// measure 2. Fails if the end-measure update runs before the close.
    #[test]
    fn a_boundary_closed_phrase_does_not_claim_the_next_measure() {
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let follower = ScoreFollower::from_musicxml_str(xml, 0).expect("fixture parses");
        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        agg.set_score_follower(follower);
        // Play straight through both measures of the fixture.
        for (i, hz) in [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25]
            .iter()
            .enumerate()
        {
            for f in 0..4 {
                agg.push(&voiced_event(*hz, 0.9, i as f64 * 0.4 + f64::from(f) * 0.1));
            }
        }
        agg.flush();
        let phrases = agg.phrases();
        let first_span = phrases[0].score_span.expect("scored phrase");
        assert_eq!(
            first_span,
            (1, 1),
            "phrase 1 holds only measure-1 events; card: {:?}",
            phrases[0].score_card
        );
    }

    /// S3 review MUST-FIX 2 (kills mutation a): a discarded short fragment
    /// takes its verdict tally with it, and a closed phrase RESETS the
    /// tally — the next phrase counts only its own notes.
    #[test]
    fn tallies_never_leak_across_phrases_or_discarded_fragments() {
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let follower = ScoreFollower::from_musicxml_str(xml, 0).expect("fixture parses");
        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        agg.set_score_follower(follower);
        // A judged 2-frame fragment (below min_phrase_events), then a long
        // silence discards it.
        agg.push(&voiced_event(261.63, 0.9, 0.0));
        agg.push(&voiced_event(261.63, 0.9, 0.02));
        // Real phrase later: 4 held notes.
        for (i, hz) in [261.63, 293.66, 329.63, 349.23].iter().enumerate() {
            for f in 0..4 {
                agg.push(&voiced_event(
                    *hz,
                    0.9,
                    10.0 + i as f64 * 0.4 + f64::from(f) * 0.1,
                ));
            }
        }
        agg.flush();
        let phrases = agg.phrases();
        let last = phrases.last().expect("the real phrase closed");
        let v = last.verdicts.expect("tally rides the phrase");
        let total = v.hit + v.near + v.missed;
        assert!(
            total <= 4,
            "the discarded fragment's verdicts must not leak in: {v:?}"
        );
    }

    /// #337 S3: free play is untouched — no span, no tally, no card.
    #[test]
    fn free_play_phrases_carry_no_score_card() {
        let mut agg = PhraseAggregator::new(PhraseConfig::default()).unwrap();
        for f in 0..8 {
            agg.push(&voiced_event(440.0, 0.9, f64::from(f) * 0.1));
        }
        agg.flush();
        let p = &agg.phrases()[0];
        assert!(p.score_span.is_none() && p.verdicts.is_none() && p.score_card.is_none());
    }

    /// #337 S3: the card copy itself — single-measure and range forms, only
    /// non-zero counts named. Fails if the wording drifts vague.
    #[test]
    fn card_copy_names_measures_and_counts() {
        let card = score_phrase_card(
            (5, 8),
            PhraseVerdicts {
                hit: 6,
                near: 1,
                missed: 2,
            },
        );
        assert_eq!(card, "Measures 5–8 — 6 clean, 1 rough, 2 missed");
        let solo = score_phrase_card(
            (3, 3),
            PhraseVerdicts {
                hit: 4,
                near: 0,
                missed: 0,
            },
        );
        assert_eq!(solo, "Measure 3 — 4 clean");
    }

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

    /// Feed one sustained note as realistic ~45 Hz analysis frames (the rate
    /// the pipeline's detect loop actually produces — the key tracker is fed
    /// per NOTE via the same gate as the live strip, and a real note spans
    /// many frames), at the given detection confidence. Returns the time
    /// after the note.
    fn feed_note_frames_at(
        agg: &mut PhraseAggregator,
        hz: f64,
        confidence: f64,
        start: f64,
        dur: f64,
    ) -> f64 {
        let mut t = start;
        while t < start + dur {
            let mut e = voiced_event(hz, 0.8, t);
            e.confidence = confidence;
            agg.push(&e);
            t += 0.022;
        }
        t
    }

    /// [`feed_note_frames_at`] at a comfortably confident 0.95.
    fn feed_note_frames(agg: &mut PhraseAggregator, hz: f64, start: f64, dur: f64) -> f64 {
        feed_note_frames_at(agg, hz, 0.95, start, dur)
    }

    /// One pass of tonic-emphasized scale material at frame rate: the scale
    /// with the tonic held longest and the fifth next (real tonal playing —
    /// that's what disambiguates a major key from its relative modes, which
    /// share all seven notes). Returns the time after the material.
    fn feed_scale_frames(agg: &mut PhraseAggregator, scale: &[f64; 7], start: f64) -> f64 {
        let mut t = feed_note_frames(agg, scale[0], start, 0.5);
        for (i, &hz) in scale[1..].iter().enumerate() {
            let dur = if i == 3 { 0.35 } else { 0.2 }; // the fifth, emphasized
            t = feed_note_frames(agg, hz, t, dur);
        }
        t
    }

    #[test]
    fn aggregator_detects_key_from_a_scale() {
        // A C-major scale played a few times within one phrase should surface
        // C major on that phrase via the rolling key tracker.
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        let c_major = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let mut t = 0.0;
        for _ in 0..6 {
            t = feed_scale_frames(&mut agg, &c_major, t);
        }
        agg.flush();

        let key = agg
            .phrases()
            .last()
            .unwrap()
            .key
            .expect("a key was detected");
        assert_eq!(key.name(), "C major", "got {}", key.name());
    }

    /// The #324 hedge-on-stable regression: a session that sits steadily in
    /// ONE key must read that key on EVERY phrase snapshot. Fed per frame
    /// (the pre-fix path), the session-long tracker's rolling window
    /// collapses to under a second of audio, each phrase snapshots whatever
    /// relative mode the last bar happened to emphasize, and the recap's
    /// vote — diluted across readings the calm strip never displayed —
    /// hedges a rock-steady session ("leaning G# major toward the end").
    #[test]
    fn steady_material_reads_one_key_on_every_phrase() {
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        // G# major (G# A# C C# D# F G), tonic-emphasized, as several phrases
        // separated by real silence gaps — the VA's steady singing session.
        let gs_major = [415.30, 466.16, 523.25, 554.37, 622.25, 698.46, 783.99];
        let mut t = 0.0;
        for _ in 0..6 {
            t = feed_scale_frames(&mut agg, &gs_major, t);
            t += 0.5; // rest between phrases → phrase boundary
        }
        agg.flush();

        let phrases = agg.phrases();
        assert!(phrases.len() >= 4, "expected several phrases");
        // The opening phrase may honestly still be refining (mode ambiguity
        // on one pass of material is real); every phrase after it must hold
        // the one steady key — that steadiness is what earns the recap's
        // flat assertion downstream (see coaching::aggregate_key).
        let names: Vec<Option<String>> = phrases.iter().map(|p| p.key.map(|k| k.name())).collect();
        assert!(
            names[1..].iter().all(|n| n.as_deref() == Some("G# major")),
            "after settling, every phrase snapshot must hold the steady key; got {names:?}"
        );
    }

    /// A phrase whose DECISIVE note is its last one (the 4th distinct pitch
    /// class the tracker needs before committing) still snapshots the key:
    /// the closing drain lands the final note before the snapshot is read.
    /// Fails if the drain is dropped — the last note would otherwise wait
    /// for a next phrase that never comes.
    #[test]
    fn a_phrases_final_note_lands_before_its_key_snapshot() {
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        // C E G repeatedly, then B as the very last note — only with it does
        // the tracker have the 4 distinct pitch classes it needs to commit.
        let mut t = 0.0;
        for _ in 0..3 {
            for &hz in &[261.63, 329.63, 392.0] {
                t = feed_note_frames(&mut agg, hz, t, 0.4);
            }
        }
        feed_note_frames(&mut agg, 493.88, t, 0.4);
        agg.flush();

        assert!(
            agg.phrases().last().unwrap().key.is_some(),
            "the final note must land in its own phrase's snapshot"
        );
    }

    /// Establish C major confidently, then feed `passes` full passes of an
    /// F#-major scale at the given detection confidence. Returns the final
    /// phrase's key name — the reading the recap would vote over.
    fn key_after_foreign_material(confidence: f64, passes: usize) -> String {
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.2,
        })
        .unwrap();
        let c_major = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let mut t = 0.0;
        for _ in 0..4 {
            t = feed_scale_frames(&mut agg, &c_major, t);
        }
        let fs_major = [369.99, 415.30, 466.16, 493.88, 554.37, 622.25, 698.46];
        for _ in 0..passes {
            for &hz in &fs_major {
                t = feed_note_frames_at(&mut agg, hz, confidence, t, 0.25);
            }
        }
        agg.flush();
        agg.phrases()
            .last()
            .unwrap()
            .key
            .expect("a key was detected")
            .name()
    }

    /// Low-confidence frames — breath noise, squeaks, shaky detection — never
    /// feed the key evidence, mirroring the strip's gate. The foreign
    /// material here is potent: the positive control proves the SAME notes
    /// above the confidence bar flip the key, so only the gate keeps the
    /// sub-bar variant from doing so. Fails if the key-evidence confidence
    /// gate is dropped or loosened to the per-instrument voiced threshold
    /// (0.2 here — voice-like).
    #[test]
    fn low_confidence_frames_do_not_move_the_key() {
        assert_ne!(
            key_after_foreign_material(0.9, 8),
            "C major",
            "positive control: this material, confidently detected, must flip the key"
        );
        assert_eq!(
            key_after_foreign_material(0.3, 8),
            "C major",
            "the same material below the key-evidence gate must not move the key"
        );
    }

    /// The deliberate strip-parity tradeoff (#316, "recap ≤ strip"): a
    /// breathy sung session whose frames sit between the instrument's voiced
    /// threshold and the strip's key-evidence bar still forms phrases (#185
    /// stays fixed) but stays KEYLESS — the strip never showed a key for it,
    /// and the recap must not out-claim the strip. Fails if the key gate is
    /// re-plumbed to the per-instrument threshold without deciding this
    /// on purpose.
    #[test]
    fn a_session_the_strip_never_keyed_stays_keyless() {
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.3, // the Voice profile's band
        })
        .unwrap();
        let c_major = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let mut t = 0.0;
        for _ in 0..4 {
            for &hz in &c_major {
                t = feed_note_frames_at(&mut agg, hz, 0.4, t, 0.25);
            }
            t += 0.5;
        }
        agg.flush();

        let phrases = agg.phrases();
        assert!(
            phrases.len() >= 3 && phrases.iter().all(|p| p.note_count > 0),
            "sub-bar voiced material must still form phrases (#185)"
        );
        assert!(
            phrases.iter().all(|p| p.key.is_none()),
            "no phrase may claim a key the strip never earned"
        );
    }

    /// A note held straight across measure-boundary closes is ONE
    /// drone-capped observation, not one re-entered (and re-capped) note per
    /// measure — that would let a tied pedal tone own the whole rolling
    /// profile, which is exactly what the drone cap exists to stop. Closes
    /// here use [`NoteGateAtClose::Keep`], the measure branch's mode; fails
    /// if measure closes drain the gate.
    #[test]
    fn a_note_held_across_measure_closes_cannot_own_the_key() {
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        let c_major = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let mut t = 0.0;
        for _ in 0..4 {
            t = feed_scale_frames(&mut agg, &c_major, t);
        }
        // An F# pedal tone tied across 8 "measures": the drone keeps
        // sounding while the phrase closes the way the measure branch
        // closes it.
        for _ in 0..8 {
            t = feed_note_frames(&mut agg, 369.99, t, 2.0);
            agg.close_current_phrase(NoteGateAtClose::Keep);
        }
        agg.flush();

        let key = agg
            .phrases()
            .last()
            .unwrap()
            .key
            .expect("a key was detected");
        assert_eq!(
            key.name(),
            "C major",
            "a tied pedal must weigh in once (capped), not once per measure; got {}",
            key.name()
        );
    }

    #[test]
    fn phrase_retains_onset_timestamps_for_groove() {
        // Onset-flagged events should have their timestamps retained in
        // `onsets_secs` so the session recap can analyse groove. Non-onset
        // (continuation) events must be excluded.
        let config = PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 2,
            voiced_confidence_threshold: 0.5,
        };
        let mut agg = PhraseAggregator::new(config).unwrap();

        let onset = |hz: f64, t: f64| AudioEvent {
            pitch_hz: Some(hz),
            confidence: 0.95,
            amplitude: 0.8,
            timestamp_secs: t,
            is_onset: true,
            note_info: None,
        };

        agg.push(&onset(440.0, 0.0));
        agg.push(&voiced_event(440.0, 0.8, 0.05)); // continuation, not an onset
        agg.push(&onset(440.0, 0.10));
        agg.flush();

        let phrase = &agg.phrases()[0];
        assert_eq!(
            phrase.onsets_secs,
            vec![0.0, 0.10],
            "only is_onset=true timestamps should be retained"
        );
    }

    #[test]
    fn phrase_summary_onsets_default_when_absent() {
        // A phrase JSON serialised before `onsets_secs` existed must still
        // deserialize, defaulting the field to an empty Vec.
        let legacy = r#"{
            "phrase_index": 0, "start_time": 0.0, "end_time": 1.0,
            "duration_secs": 1.0, "note_count": 3,
            "pitch_stats": {"mean_hz":440.0,"min_hz":435.0,"max_hz":445.0,"range_cents":40.0,"pitches":[440.0]},
            "dynamics": {"mean_amplitude":0.5,"min_amplitude":0.4,"max_amplitude":0.6,"dynamic_range":0.2},
            "stability": 0.8, "score_position": null
        }"#;
        let p: PhraseSummary = serde_json::from_str(legacy).expect("legacy phrase deserializes");
        assert!(p.onsets_secs.is_empty(), "absent onsets default to empty");
    }

    #[test]
    fn short_phrase_does_not_pin_a_key() {
        // Three repetitions of a single pitch class isn't enough harmonic
        // evidence to name a key — the tracker should stay quiet.
        let mut agg = PhraseAggregator::new(PhraseConfig {
            silence_gap_secs: 0.3,
            min_phrase_events: 3,
            voiced_confidence_threshold: 0.5,
        })
        .unwrap();
        for i in 0..3 {
            agg.push(&voiced_event(261.63, 0.8, i as f64 * 0.05));
        }
        agg.flush();

        assert!(
            agg.phrases()[0].key.is_none(),
            "one pitch class must not pin a key"
        );
    }
}
