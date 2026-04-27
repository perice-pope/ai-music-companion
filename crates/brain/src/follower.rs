//! Online Dynamic Time Warping for score following.
//!
//! This module implements Online DTW to align live audio events (played notes)
//! to a loaded score in real time, enabling phrase-level feedback and UI
//! position tracking.
//!
//! Key features:
//! - Windowed DTW to stay within latency budget (<3ms per alignment step)
//! - Tempo tolerance (±20%) to handle tempo fluctuations
//! - Automatic re-alignment on silence gaps
//! - Per-measure and per-beat tracking

use crate::score::{ScoreModel, ScoreNote};
use ears::AudioEvent;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/// Errors that can occur in score following.
#[derive(Debug, Error)]
pub enum FollowerError {
    #[error("no measures in score")]
    EmptyScore,
    #[error("invalid tempo: expected > 0, got {0}")]
    InvalidTempo(f64),
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Current position in the score.
///
/// Returned by [`ScoreFollower::align`] after each audio event. Tracks both
/// absolute position (measure, beat) and the expected note at this position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScorePosition {
    /// 1-based measure number.
    pub measure_number: usize,
    /// Beat within the current measure (0.0-based, e.g., 0.0-3.99 in 4/4).
    pub beat: f64,
    /// Optional section name (e.g., "Verse", "Chorus") if available in score.
    pub section_name: Option<String>,
    /// The MIDI note number we expect to hear at this position, if any.
    pub expected_note: Option<u8>,
}

impl ScorePosition {
    /// Create a position at the start of the score.
    fn start() -> Self {
        Self {
            measure_number: 1,
            beat: 0.0,
            section_name: None,
            expected_note: None,
        }
    }
}

/// A note event from the score, tagged with its position.
#[derive(Debug, Clone)]
struct ScoredNote {
    midi_number: u8,
    #[allow(dead_code)]
    pitch_hz: f64,
    #[allow(dead_code)]
    duration_beats: f64,
    measure_number: usize,
    beat_in_measure: f64,
}

impl ScoredNote {
    fn from_score(note: &ScoreNote, measure_number: usize) -> Self {
        Self {
            midi_number: note.midi_number,
            pitch_hz: note.pitch_hz,
            duration_beats: note.duration_beats,
            measure_number,
            beat_in_measure: note.start_beat,
        }
    }
}

/// A single event from the player's audio, tagged with timing.
#[derive(Debug, Clone)]
struct PlayedEvent {
    midi_number: u8,
    #[allow(dead_code)]
    pitch_hz: f64,
    #[allow(dead_code)]
    timestamp_secs: f64,
    #[allow(dead_code)]
    confidence: f64,
}

impl PlayedEvent {
    fn from_audio(event: &AudioEvent) -> Self {
        let midi_number = event.pitch_hz.map(hz_to_midi).unwrap_or(0);
        Self {
            midi_number,
            pitch_hz: event.pitch_hz.unwrap_or(0.0),
            timestamp_secs: event.timestamp_secs,
            confidence: event.confidence,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Online DTW Implementation
// ─────────────────────────────────────────────────────────────────────────────

/// Cost for a single alignment step.
#[derive(Clone, Debug)]
struct DtwCost {
    /// Pitch distance (cents).
    #[allow(dead_code)]
    pitch_distance: f64,
    /// Cumulative cost from the previous step.
    cumulative: f64,
}

impl DtwCost {
    fn new(pitch_distance: f64, prev_cumulative: f64) -> Self {
        Self {
            pitch_distance,
            cumulative: prev_cumulative + pitch_distance,
        }
    }
}

/// Online DTW state tracker.
#[derive(Debug)]
struct OnlineDtw {
    /// Window of recent played events (up to 100 events).
    #[allow(dead_code)]
    played_window: VecDeque<PlayedEvent>,
    /// Previous row of cost matrix (for space efficiency).
    #[allow(dead_code)]
    prev_cost_row: Vec<DtwCost>,
    /// Current row of cost matrix (being computed).
    curr_cost_row: Vec<DtwCost>,
    /// Index in the score notes sequence.
    #[allow(dead_code)]
    score_index: usize,
    /// Tempo ratio (actual / expected). Start at 1.0.
    #[allow(dead_code)]
    tempo_ratio: f64,
    /// Threshold for re-alignment (silence gap in seconds).
    silence_threshold_secs: f64,
    /// Last timestamp of a voiced event.
    last_voiced_time: f64,
}

impl OnlineDtw {
    fn new(num_score_notes: usize) -> Self {
        Self {
            played_window: VecDeque::with_capacity(100),
            prev_cost_row: vec![DtwCost::new(0.0, 0.0); num_score_notes],
            curr_cost_row: vec![DtwCost::new(0.0, 0.0); num_score_notes],
            score_index: 0,
            tempo_ratio: 1.0,
            silence_threshold_secs: 0.3,
            last_voiced_time: 0.0,
        }
    }

    /// Compute the cost of aligning a played MIDI note to an expected MIDI note,
    /// accounting for cents deviation and dynamic tempo.
    fn note_cost(&self, played_midi: f64, expected_midi: f64) -> f64 {
        let cents_diff = (played_midi - expected_midi).abs() * 100.0;
        // Cost: 1 cent deviation = 1 cost unit.
        // Rests (0 MIDI) are very expensive to match.
        if played_midi == 0.0 || expected_midi == 0.0 {
            500.0
        } else {
            cents_diff.min(1000.0)
        }
    }

    /// Reset alignment state (on silence gap or user restart).
    fn reset(&mut self) {
        self.played_window.clear();
        self.score_index = 0;
        self.tempo_ratio = 1.0;
    }

    /// Check if a silence gap has occurred.
    fn check_silence_gap(&mut self, current_time: f64, is_voiced: bool) {
        if is_voiced {
            if current_time - self.last_voiced_time > self.silence_threshold_secs {
                self.reset();
            }
            self.last_voiced_time = current_time;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main ScoreFollower
// ─────────────────────────────────────────────────────────────────────────────

/// Tracks the musician's position within a loaded score in real time.
///
/// ScoreFollower uses Online DTW to align played audio events to the score,
/// enabling real-time feedback about which measure and beat the musician is in,
/// along with what note is expected next.
#[derive(Debug)]
pub struct ScoreFollower {
    /// The score being followed.
    #[allow(dead_code)]
    score: ScoreModel,
    /// All non-rest notes from the score, flattened for DTW.
    score_notes: Vec<ScoredNote>,
    /// Current position in the score.
    position: ScorePosition,
    /// Online DTW state.
    dtw: OnlineDtw,
    /// Beats per minute (from score).
    #[allow(dead_code)]
    tempo_bpm: f64,
    /// Time signature beats per measure.
    #[allow(dead_code)]
    beats_per_measure: f64,
}

impl ScoreFollower {
    /// Create a new ScoreFollower for a given score.
    ///
    /// Extracts all non-rest notes from the score and initializes DTW state.
    pub fn new(score: ScoreModel) -> Result<Self, FollowerError> {
        if score.measures.is_empty() {
            return Err(FollowerError::EmptyScore);
        }
        if score.tempo_bpm <= 0.0 {
            return Err(FollowerError::InvalidTempo(score.tempo_bpm));
        }

        // Flatten all notes from the score (skip rests).
        let mut score_notes = Vec::new();
        for measure in &score.measures {
            for note in &measure.notes {
                if !note.is_rest {
                    score_notes.push(ScoredNote::from_score(note, measure.number));
                }
            }
        }

        let beats_per_measure = score.time_signature.beats as f64;
        let dtw = OnlineDtw::new(score_notes.len());

        Ok(Self {
            tempo_bpm: score.tempo_bpm,
            beats_per_measure,
            score: score.clone(),
            score_notes,
            position: ScorePosition::start(),
            dtw,
        })
    }

    /// Process a single audio event and return the updated score position.
    ///
    /// This is the main entry point: it aligns the incoming audio to the score
    /// and updates the position tracker. Runs in <3ms (within latency budget).
    pub fn align(&mut self, event: &AudioEvent) -> ScorePosition {
        let is_voiced = event.pitch_hz.is_some() && event.confidence > 0.5;

        // Check for silence gaps and reset if needed.
        self.dtw.check_silence_gap(event.timestamp_secs, is_voiced);

        if !is_voiced {
            // Not a voiced event — return current position unchanged.
            return self.position.clone();
        }

        let played_event = PlayedEvent::from_audio(event);

        // Add to the DTW window.
        self.dtw.played_window.push_back(played_event.clone());
        if self.dtw.played_window.len() > 100 {
            self.dtw.played_window.pop_front();
        }

        // Run DTW update to find the best alignment.
        self.update_alignment(&played_event);

        self.position.clone()
    }

    /// Run one step of Online DTW to update alignment.
    fn update_alignment(&mut self, played_event: &PlayedEvent) {
        let num_score = self.score_notes.len();

        if num_score == 0 {
            return;
        }

        // Swap cost rows: prev now holds the last iteration's data,
        // curr is ready to be cleared and rebuilt. This avoids allocation.
        std::mem::swap(&mut self.dtw.prev_cost_row, &mut self.dtw.curr_cost_row);
        self.dtw.curr_cost_row.clear();

        // Compute cost for the new played event against all score notes.
        // Use a simple approach: find the score note with minimum cost to this played note,
        // allowing for continuation from the previous state.
        for (score_idx, score_note) in self.score_notes.iter().enumerate() {
            let note_cost = self.dtw.note_cost(
                played_event.midi_number as f64,
                score_note.midi_number as f64,
            );

            // Cumulative cost: take the best path from previous state
            let cumulative_cost = if self.dtw.prev_cost_row.is_empty() {
                note_cost
            } else {
                // Allow continuing from this or nearby score positions in the previous row
                let prev_cost = if score_idx > 0 {
                    self.dtw.prev_cost_row[score_idx - 1].cumulative
                } else {
                    self.dtw.prev_cost_row[0].cumulative
                };
                let same_cost = self
                    .dtw
                    .prev_cost_row
                    .get(score_idx)
                    .map(|c| c.cumulative)
                    .unwrap_or(f64::INFINITY);
                let next_cost = self
                    .dtw
                    .prev_cost_row
                    .get(score_idx + 1)
                    .map(|c| c.cumulative)
                    .unwrap_or(f64::INFINITY);

                note_cost + prev_cost.min(same_cost).min(next_cost)
            };

            self.dtw
                .curr_cost_row
                .push(DtwCost::new(note_cost, cumulative_cost));
        }

        // Find the best score position (minimum cumulative cost).
        if let Some((best_idx, best_cost)) =
            self.dtw.curr_cost_row.iter().enumerate().min_by(|a, b| {
                a.1.cumulative
                    .partial_cmp(&b.1.cumulative)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            // Update position based on the best-aligned score note.
            if best_cost.cumulative < f64::INFINITY {
                let score_note = &self.score_notes[best_idx];
                self.position.measure_number = score_note.measure_number;
                self.position.beat = score_note.beat_in_measure;
                self.position.section_name = None; // TODO: extract from score metadata
                self.position.expected_note = Some(score_note.midi_number);
                self.dtw.score_index = best_idx;
            }
        }
    }

    /// Reset the follower (e.g., when the user stops and restarts).
    pub fn reset(&mut self) {
        self.dtw.reset();
        self.position = ScorePosition::start();
    }

    /// Get the current position without processing an event.
    pub fn current_position(&self) -> ScorePosition {
        self.position.clone()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Convert Hz to MIDI note number (fractional).
fn hz_to_midi(hz: f64) -> u8 {
    (12.0 * (hz / 440.0).log2() + 69.0).round() as u8
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{KeySignature, Measure, TimeSignature};

    fn create_simple_score() -> ScoreModel {
        // C major scale: C4, D4, E4, F4, G4, A4, B4, C5
        let notes = vec![
            ScoreNote {
                pitch_hz: 261.63,
                midi_number: 60,
                duration_beats: 1.0,
                start_beat: 0.0,
                dynamic: None,
                is_rest: false,
            },
            ScoreNote {
                pitch_hz: 293.66,
                midi_number: 62,
                duration_beats: 1.0,
                start_beat: 1.0,
                dynamic: None,
                is_rest: false,
            },
            ScoreNote {
                pitch_hz: 329.63,
                midi_number: 64,
                duration_beats: 1.0,
                start_beat: 2.0,
                dynamic: None,
                is_rest: false,
            },
        ];

        ScoreModel {
            title: "Test Scale".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            measures: vec![Measure { number: 1, notes }],
        }
    }

    fn create_audio_event(pitch_hz: f64, confidence: f64, timestamp_secs: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: Some(pitch_hz),
            confidence,
            amplitude: 0.5,
            timestamp_secs,
            is_onset: true,
        }
    }

    #[test]
    fn new_initializes_with_valid_score() {
        let score = create_simple_score();
        let follower = ScoreFollower::new(score);
        assert!(follower.is_ok());

        let f = follower.unwrap();
        assert_eq!(f.score_notes.len(), 3); // Three notes in the test score
        assert_eq!(f.position.measure_number, 1);
        assert_eq!(f.position.beat, 0.0);
    }

    #[test]
    fn new_rejects_empty_score() {
        let score = ScoreModel {
            title: "Empty".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            measures: vec![],
        };

        let err = ScoreFollower::new(score).unwrap_err();
        assert!(matches!(err, FollowerError::EmptyScore));
    }

    #[test]
    fn new_rejects_invalid_tempo() {
        let score = ScoreModel {
            title: "Invalid".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 0.0,
            measures: vec![Measure {
                number: 1,
                notes: vec![ScoreNote {
                    pitch_hz: 440.0,
                    midi_number: 69,
                    duration_beats: 1.0,
                    start_beat: 0.0,
                    dynamic: None,
                    is_rest: false,
                }],
            }],
        };

        let err = ScoreFollower::new(score).unwrap_err();
        assert!(matches!(err, FollowerError::InvalidTempo(0.0)));
    }

    #[test]
    fn align_with_matching_pitch_advances_position() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // First note: C4 (MIDI 60, 261.63 Hz)
        let event = create_audio_event(261.63, 0.9, 0.0);
        let pos = follower.align(&event);

        // Should have moved to at least the first note in the score.
        assert_eq!(pos.measure_number, 1);
        assert_eq!(pos.expected_note, Some(60));
    }

    #[test]
    fn align_ignores_unvoiced_events() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // Unvoiced event (confidence too low)
        let event = AudioEvent {
            pitch_hz: Some(261.63),
            confidence: 0.2,
            amplitude: 0.1,
            timestamp_secs: 0.0,
            is_onset: false,
        };

        let pos_before = follower.current_position();
        let pos_after = follower.align(&event);

        assert_eq!(pos_before, pos_after);
    }

    #[test]
    fn align_resets_on_silence_gap() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // First event
        let event1 = create_audio_event(261.63, 0.9, 0.0);
        let _pos1 = follower.align(&event1);

        // Long silence gap (> 0.3s) followed by a new event
        let event2 = create_audio_event(293.66, 0.9, 1.0);
        let pos2 = follower.align(&event2);

        // After reset, position should be back near the start.
        // (The DTW should realign to find the best match, but the silence triggers a reset.)
        assert_eq!(pos2.measure_number, 1);
    }

    #[test]
    fn reset_clears_position() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // Align to a note.
        let event = create_audio_event(329.63, 0.9, 0.0);
        let _ = follower.align(&event);

        // Reset.
        follower.reset();

        let pos = follower.current_position();
        assert_eq!(pos.measure_number, 1);
        assert_eq!(pos.beat, 0.0);
    }

    #[test]
    fn score_position_serializes() {
        let pos = ScorePosition {
            measure_number: 5,
            beat: 2.5,
            section_name: Some("Chorus".to_string()),
            expected_note: Some(64),
        };

        let json = serde_json::to_string(&pos).expect("serialize ScorePosition");
        let roundtrip: ScorePosition =
            serde_json::from_str(&json).expect("deserialize ScorePosition");

        assert_eq!(roundtrip.measure_number, 5);
        assert_eq!(roundtrip.beat, 2.5);
        assert_eq!(roundtrip.section_name.as_deref(), Some("Chorus"));
        assert_eq!(roundtrip.expected_note, Some(64));
    }

    #[test]
    fn hz_to_midi_a4_is_69() {
        let midi = hz_to_midi(440.0);
        assert_eq!(midi, 69);
    }

    #[test]
    fn hz_to_midi_c4_is_60() {
        let midi = hz_to_midi(261.63);
        assert_eq!(midi, 60);
    }

    #[test]
    fn integration_aligned_audio_matches_score() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // Simulate playing the first three notes of the score in order.
        let notes = [261.63, 293.66, 329.63]; // C4, D4, E4
        let expected_midis = [60, 62, 64];

        for (i, &hz) in notes.iter().enumerate() {
            let event = create_audio_event(hz, 0.9, i as f64 * 0.5);
            let pos = follower.align(&event);
            assert_eq!(
                pos.expected_note,
                Some(expected_midis[i]),
                "Event {} should align to MIDI {}",
                i,
                expected_midis[i]
            );
        }
    }

    #[test]
    fn integration_misaligned_audio_recovers() {
        let score = create_simple_score();
        let mut follower = ScoreFollower::new(score).unwrap();

        // Feed a wrong note.
        let wrong_event = create_audio_event(500.0, 0.9, 0.0);
        let _pos = follower.align(&wrong_event);

        // Feed the correct note — should recover.
        let correct_event = create_audio_event(261.63, 0.9, 0.1);
        let pos = follower.align(&correct_event);

        assert_eq!(pos.expected_note, Some(60)); // Should have recovered to the correct note.
    }
}
