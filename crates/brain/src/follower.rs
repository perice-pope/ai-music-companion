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
    #[error("failed to parse score: {0}")]
    Parse(#[from] crate::score::ScoreError),
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
    /// The MIDI note number we expect to hear at this position, if any.
    pub expected_note: Option<u8>,
}

impl ScorePosition {
    /// Create a position at the start of the score.
    fn start() -> Self {
        Self {
            measure_number: 1,
            beat: 0.0,
            expected_note: None,
        }
    }
}

/// How firmly a played note matched the score note the follower aligned it
/// to (#337 S2). Emitted per SCORE NOTE as the alignment advances — never
/// per audio frame — so the UI can light passed notes hit/near/missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Same pitch, within the instrument's cents tolerance.
    Hit,
    /// Same pitch but out of tune, or one semitone off.
    Near,
    /// Wrong pitch, or the score note was skipped over entirely.
    Missed,
}

/// One judged score note: where it lives in the score and how it went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteVerdict {
    pub measure_number: usize,
    pub beat: f64,
    pub verdict: Verdict,
}

/// Tolerances for judging (#337 S2). Profile-driven (founder decision
/// 2026-07-10): the app derives `hit_cents` from the instrument profile's
/// vibrato tolerance; this default covers profiles without one.
#[derive(Debug, Clone, Copy)]
pub struct VerdictTolerances {
    /// Same-pitch deviation (cents) still counted as a clean hit.
    pub hit_cents: f64,
}

impl Default for VerdictTolerances {
    fn default() -> Self {
        Self { hit_cents: 30.0 }
    }
}

/// Alignment jumps LARGER than this emit no Missed verdicts for the skipped
/// notes: a big jump is the follower relocating (repeat, lost-and-found),
/// not the player skipping — silence beats false accusations.
const MAX_MISS_RUN: usize = 4;

/// Extra cost (cents units) to arrive at a score note from the note AFTER it —
/// i.e. to step the alignment backward. Playing through a score is forward
/// motion; a free backward step lets a descending line re-trace its ascending
/// twin earlier in the score (VA #347: the cursor walked measures 3–4 back
/// through 2→1). Backward correction stays possible under sustained evidence
/// (~2 frames of a cheaper match outweigh it); relocation over a distance is
/// the silence-gap reset's job.
const BACKWARD_STEP_PENALTY: f64 = 300.0;

/// Cumulative costs within this of the minimum count as tied; ties resolve
/// by proximity to the current position (see `update_alignment`).
const COST_TIE_EPSILON: f64 = 1e-6;

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
    pitch_hz: f64,
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

/// Online DTW state tracker.
#[derive(Debug)]
struct OnlineDtw {
    /// Window of recent played events (up to 100 events).
    played_window: VecDeque<PlayedEvent>,
    /// Previous row of cost matrix (for space efficiency).
    prev_cost_row: Vec<DtwCost>,
    /// Current row of cost matrix (being computed).
    curr_cost_row: Vec<DtwCost>,
    /// Index in the score notes sequence.
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
            prev_cost_row: Vec::with_capacity(num_score_notes),
            curr_cost_row: Vec::with_capacity(num_score_notes),
            score_index: 0,
            tempo_ratio: 1.0,
            // A rehearsal break, not a breath: 0.3 s reset the alignment on
            // every detached quarter note, wiping the verdict watermark so a
            // cleanly sung pass judged almost nothing (VA #347). Phrase
            // segmentation keeps its own tighter gap; relocation needs a gap
            // no articulation produces.
            silence_threshold_secs: 1.5,
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

    /// Reset alignment state (on silence gap or user restart). Clears the
    /// cost rows too: a reset that keeps the cumulative matrix isn't a
    /// re-alignment at all — the next entry would just continue the old
    /// path instead of re-anchoring by pure pitch match.
    fn reset(&mut self) {
        self.played_window.clear();
        self.prev_cost_row.clear();
        self.curr_cost_row.clear();
        self.score_index = 0;
        self.tempo_ratio = 1.0;
    }

    /// Check if a silence gap has occurred.
    /// Returns `true` when the gap reset the DTW state, so the caller can
    /// reset dependent state (the verdict judge, #337 S2 review S4).
    fn check_silence_gap(&mut self, current_time: f64, is_voiced: bool) -> bool {
        let mut did_reset = false;
        if is_voiced {
            if current_time - self.last_voiced_time > self.silence_threshold_secs {
                self.reset();
                did_reset = true;
            }
            self.last_voiced_time = current_time;
        }
        did_reset
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
    /// Verdict engine (#337 S2): last score index judged, verdicts awaiting
    /// [`Self::take_verdicts`], and the per-instrument tolerances.
    last_judged_index: Option<usize>,
    /// A one-frame confirmation gate (#337 S2 review MF3): an advance is
    /// judged only when the NEXT voiced frame lands on the same score note.
    /// A lost follower's argmin wanders frame to frame and rarely confirms,
    /// so playing a different piece stays (mostly) unjudged — the closest
    /// thing to "locked" an argmin follower has.
    judge_candidate: Option<usize>,
    pending_verdicts: Vec<NoteVerdict>,
    tolerances: VerdictTolerances,
    /// Timestamp of the last judged advance — the horizon behind which the
    /// played window can't be used as evidence for a skipped note (see
    /// [`Self::skipped_note_verdict`]).
    last_advance_time: f64,
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
    /// Build a follower directly from an in-memory MusicXML string.
    ///
    /// Convenience for callers that hold the raw MusicXML (e.g. the
    /// desktop app, which stores it inline in the score library) rather
    /// than a file path. `part_index` selects the `<part>` to follow,
    /// matching the part the score was imported under.
    pub fn from_musicxml_str(xml: &str, part_index: usize) -> Result<Self, FollowerError> {
        let score = crate::score::musicxml::parse_musicxml_str_part(xml, part_index)?;
        Self::new(score)
    }

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
            last_judged_index: None,
            judge_candidate: None,
            pending_verdicts: Vec::new(),
            tolerances: VerdictTolerances::default(),
            last_advance_time: 0.0,
        })
    }

    /// Set the per-instrument judging tolerances (#337 S2) — derived from
    /// the active instrument profile by the app layer.
    pub fn set_verdict_tolerances(&mut self, tolerances: VerdictTolerances) {
        self.tolerances = tolerances;
    }

    /// Drain the note verdicts produced since the last call (#337 S2).
    /// One entry per JUDGED SCORE NOTE, in score order — never per frame.
    pub fn take_verdicts(&mut self) -> Vec<NoteVerdict> {
        std::mem::take(&mut self.pending_verdicts)
    }

    /// Process a single audio event and return the updated score position.
    ///
    /// This is the main entry point: it aligns the incoming audio to the score
    /// and updates the position tracker. Runs in <3ms (within latency budget).
    pub fn align(&mut self, event: &AudioEvent) -> ScorePosition {
        let is_voiced = event.pitch_hz.is_some() && event.confidence > 0.5;

        // Check for silence gaps and reset if needed. A reset also clears
        // the verdict judge: after a pause the alignment restarts, and the
        // stale watermark would otherwise silence replayed notes and then
        // emit phantom Missed runs when it re-locked (#337 S2 review).
        if self.dtw.check_silence_gap(event.timestamp_secs, is_voiced) {
            self.last_judged_index = None;
            self.judge_candidate = None;
        }

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

            // Cumulative cost: take the best path from previous state.
            // Playing a score is forward motion — advancing and staying are
            // free transitions, stepping backward pays BACKWARD_STEP_PENALTY
            // so a descending line can't re-trace its ascending twin earlier
            // in the score for free (VA #347).
            let cumulative_cost = if self.dtw.prev_cost_row.is_empty() {
                note_cost
            } else {
                let advance_cost = if score_idx > 0 {
                    self.dtw.prev_cost_row[score_idx - 1].cumulative
                } else {
                    f64::INFINITY
                };
                let same_cost = self
                    .dtw
                    .prev_cost_row
                    .get(score_idx)
                    .map(|c| c.cumulative)
                    .unwrap_or(f64::INFINITY);
                let backward_cost = self
                    .dtw
                    .prev_cost_row
                    .get(score_idx + 1)
                    .map(|c| c.cumulative + BACKWARD_STEP_PENALTY)
                    .unwrap_or(f64::INFINITY);

                note_cost + advance_cost.min(same_cost).min(backward_cost)
            };

            self.dtw.curr_cost_row.push(DtwCost {
                pitch_distance: note_cost,
                cumulative: cumulative_cost,
            });
        }

        // Find the best score position (minimum cumulative cost). Ties go to
        // the index nearest the current position, preferring forward — the
        // old first-minimum rule was a systematic backward bias: any repeated
        // pitch (a scale's descent, a da-capo) tied with its earlier twin and
        // lost to it, pinning the cursor at the top of the score (VA #347).
        let current = self.dtw.score_index;
        let mut best: Option<(usize, f64)> = None;
        for (idx, cost) in self.dtw.curr_cost_row.iter().enumerate() {
            let cumulative = cost.cumulative;
            let wins = match best {
                None => true,
                Some((best_idx, best_cost)) => {
                    if cumulative + COST_TIE_EPSILON < best_cost {
                        true
                    } else if (cumulative - best_cost).abs() <= COST_TIE_EPSILON {
                        let dist = idx.abs_diff(current);
                        let best_dist = best_idx.abs_diff(current);
                        dist < best_dist || (dist == best_dist && idx > best_idx)
                    } else {
                        false
                    }
                }
            };
            if wins {
                best = Some((idx, cumulative));
            }
        }
        if let Some((best_idx, best_cost)) = best {
            // Update position based on the best-aligned score note.
            if best_cost < f64::INFINITY {
                let score_note = &self.score_notes[best_idx];
                self.position.measure_number = score_note.measure_number;
                self.position.beat = score_note.beat_in_measure;
                self.position.expected_note = Some(score_note.midi_number);
                self.dtw.score_index = best_idx;
                self.judge_advance(best_idx, played_event);
            }
        }
    }

    /// "Near" reaches one semitone plus a quarter-tone of slack: a note a
    /// full tone off is not near on any instrument (#337 S2 review S6; the
    /// spec's "within 1 semitone" plus real-world detector jitter).
    const NEAR_MAX_SEMITONES: f64 = 1.25;

    /// Judge the alignment advance (#337 S2): each score note the follower
    /// LANDS on — once the next frame CONFIRMS the landing — is judged
    /// against what was played; notes skipped in a small forward jump are
    /// Missed. Regressions, big jumps (the follower relocating), unconfirmed
    /// one-frame advances (a lost follower's wandering argmin), and a
    /// mid-piece first alignment judge nothing: silence beats false
    /// accusations.
    ///
    /// Semantics, stated honestly: this grades the note the FOLLOWER LANDED
    /// ON, not "the next expected note" — a wrong-but-diatonic note can
    /// align elsewhere and read better than it was played. The counts mean
    /// "as the app followed along" (documented deviation, spec §4).
    fn judge_advance(&mut self, best_idx: usize, played: &PlayedEvent) {
        let from = self.last_judged_index;
        let advanced = from.is_none_or(|last| best_idx > last);
        if !advanced {
            self.judge_candidate = None;
            return;
        }
        // Confirmation gate: judge only when two consecutive voiced frames
        // agree on the landing (#337 S2 review MF3).
        if self.judge_candidate != Some(best_idx) {
            self.judge_candidate = Some(best_idx);
            return;
        }
        self.judge_candidate = None;
        // First alignment of the session: only a start at the top is
        // judged. A mid-piece entry (or a wrong opening note aligning to
        // whatever matches it) sets the watermark silently.
        if from.is_none() && best_idx != 0 {
            self.last_judged_index = Some(best_idx);
            self.last_advance_time = played.timestamp_secs;
            return;
        }
        let gap = from.map_or(0, |last| best_idx - last - 1);
        if from.is_some() && gap > 0 && gap <= MAX_MISS_RUN {
            for idx in (best_idx - gap)..best_idx {
                let n = &self.score_notes[idx];
                let (measure_number, beat) = (n.measure_number, n.beat_in_measure);
                // Alignment doesn't stop on every note of a repeated or fast
                // figure (a cost tie stays put, then "skips" the twin). If
                // this note's pitch is in the played window since the last
                // advance, it WAS played — grade that evidence instead of
                // accusing (VA #347's phantom misses).
                let verdict = self.skipped_note_verdict(n.midi_number);
                self.pending_verdicts.push(NoteVerdict {
                    measure_number,
                    beat,
                    verdict,
                });
            }
        }
        // Judge the landed note only on small advances: a relocation jump
        // says nothing about how THIS note was played.
        if gap <= MAX_MISS_RUN {
            let expected = &self.score_notes[best_idx];
            let played_fractional = 12.0 * (played.pitch_hz / 440.0).log2() + 69.0;
            let semis = played_fractional - f64::from(expected.midi_number);
            let verdict = if semis.abs() * 100.0 <= self.tolerances.hit_cents {
                Verdict::Hit
            } else if semis.abs() <= Self::NEAR_MAX_SEMITONES {
                Verdict::Near
            } else {
                Verdict::Missed
            };
            self.pending_verdicts.push(NoteVerdict {
                measure_number: expected.measure_number,
                beat: expected.beat_in_measure,
                verdict,
            });
        }
        self.last_judged_index = Some(best_idx);
        self.last_advance_time = played.timestamp_secs;
    }

    /// Verdict for a score note the alignment skipped over: Hit/Near when
    /// the played window since the last advance holds its exact rounded
    /// pitch (graded by that evidence's cents), Missed when nothing near it
    /// sounded. A neighbouring pitch is NOT evidence — that's the landed
    /// note's own audio, and crediting it would soften real skips. Known
    /// generosity: with no onset/duration evidence, one long C and two
    /// repeated Cs are indistinguishable here, so a genuinely dropped
    /// repeat of a pitch that did sound is credited rather than accused —
    /// the same silence-beats-false-accusations trade as `MAX_MISS_RUN`.
    fn skipped_note_verdict(&self, expected_midi: u8) -> Verdict {
        let mut best_cents = f64::INFINITY;
        for e in &self.dtw.played_window {
            if e.timestamp_secs <= self.last_advance_time
                || e.midi_number != expected_midi
                || e.pitch_hz <= 0.0
            {
                continue;
            }
            let played_fractional = 12.0 * (e.pitch_hz / 440.0).log2() + 69.0;
            let cents = (played_fractional - f64::from(expected_midi)).abs() * 100.0;
            best_cents = best_cents.min(cents);
        }
        if best_cents <= self.tolerances.hit_cents {
            Verdict::Hit
        } else if best_cents <= Self::NEAR_MAX_SEMITONES * 100.0 {
            Verdict::Near
        } else {
            Verdict::Missed
        }
    }

    /// Reset the follower (e.g., when the user stops and restarts).
    pub fn reset(&mut self) {
        self.dtw.reset();
        self.position = ScorePosition::start();
        self.last_judged_index = None;
        self.judge_candidate = None;
        self.pending_verdicts.clear();
        self.last_advance_time = 0.0;
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
            grand_staff: false,
        }
    }

    fn create_audio_event(pitch_hz: f64, confidence: f64, timestamp_secs: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: Some(pitch_hz),
            confidence,
            amplitude: 0.5,
            timestamp_secs,
            is_onset: true,
            note_info: None,
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
            grand_staff: false,
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
            grand_staff: false,
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

    /// #337 S2 AC3: a scripted pitch stream against a known score produces
    /// the expected hit/near/missed sequence — in-tune C hits, a 60-cents-
    /// sharp D is near, an E played as G# misses. Fails if judging drifts
    /// off the profile tolerance or stops firing per score note.
    /// Feed one played note as several consecutive frames (real detection
    /// is ~45 Hz), satisfying the confirmation gate the way live audio does.
    fn play_note(f: &mut ScoreFollower, hz: f64, t0: f64) {
        for i in 0..4 {
            f.align(&create_audio_event(hz, 0.9, t0 + f64::from(i) * 0.02));
        }
    }

    #[test]
    fn scripted_stream_yields_the_expected_verdicts() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        // C4 in tune (261.63 Hz).
        play_note(&mut f, 261.63, 0.0);
        // D4 ~60 cents sharp: 293.66 * 2^(0.6/12).
        play_note(&mut f, 293.66 * 1.0353, 0.2);
        // E4 played 2 semitones sharp (F#4, 369.99 Hz): nothing in the
        // 3-note C-D-E score is nearer, so the alignment lands on E and
        // judges the wrong pitch Missed (review MF1: the whole ladder,
        // asserted exactly).
        play_note(&mut f, 369.99, 0.4);

        let verdicts = f.take_verdicts();
        let kinds: Vec<Verdict> = verdicts.iter().map(|v| v.verdict).collect();
        assert_eq!(
            kinds,
            vec![Verdict::Hit, Verdict::Near, Verdict::Missed],
            "the full ladder, in order: {verdicts:?}"
        );
        assert_eq!(
            verdicts[0].measure_number, 1,
            "score coordinates ride along"
        );
        // Drained means drained.
        assert!(f.take_verdicts().is_empty(), "second drain is empty");
    }

    /// Review MF2: a big alignment jump is the follower RELOCATING, not the
    /// player skipping — no Missed run, no landed verdict.
    #[test]
    fn a_relocation_jump_judges_nothing() {
        let mut f = ScoreFollower::new(create_two_measure_score()).unwrap();
        play_note(&mut f, 261.63, 0.0); // C4 → judged at the top
        let _ = f.take_verdicts();
        // Leap to the last note (C5, idx 7): gap of 6 > MAX_MISS_RUN.
        play_note(&mut f, 523.25, 0.2);
        let after = f.take_verdicts();
        assert!(
            after.is_empty(),
            "a relocation must accuse nobody: {after:?}"
        );
    }

    /// Review MF1 (second half): a small forward jump marks the skipped
    /// notes Missed and still judges the landed note.
    #[test]
    fn a_small_skip_marks_the_skipped_notes_missed() {
        let mut f = ScoreFollower::new(create_two_measure_score()).unwrap();
        play_note(&mut f, 261.63, 0.0); // C4 (idx 0)
        let _ = f.take_verdicts();
        // Jump to F4 (idx 3): skips D4, E4 — gap 2 ≤ MAX_MISS_RUN.
        play_note(&mut f, 349.23, 0.2);
        let verdicts = f.take_verdicts();
        let kinds: Vec<Verdict> = verdicts.iter().map(|v| v.verdict).collect();
        assert_eq!(
            kinds,
            vec![Verdict::Missed, Verdict::Missed, Verdict::Hit],
            "two skipped + one landed: {verdicts:?}"
        );
    }

    /// Review MF3: a single unconfirmed frame — the wandering argmin of a
    /// lost follower — judges nothing. Only two consecutive frames on the
    /// same score note do.
    #[test]
    fn an_unconfirmed_single_frame_judges_nothing() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        f.align(&create_audio_event(261.63, 0.9, 0.0));
        assert!(
            f.take_verdicts().is_empty(),
            "one frame is a candidate, not a verdict"
        );
        f.align(&create_audio_event(261.63, 0.9, 0.02));
        assert_eq!(f.take_verdicts().len(), 1, "the second frame confirms");
    }

    /// Review S4: a silence gap resets the judge — replaying from the top
    /// after a pause judges again and emits no phantom Missed run.
    #[test]
    fn a_pause_and_restart_judges_cleanly_from_the_top() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        play_note(&mut f, 261.63, 0.0);
        play_note(&mut f, 293.66, 0.2);
        let _ = f.take_verdicts();
        // Long silence, then start over from C4.
        play_note(&mut f, 261.63, 10.0);
        play_note(&mut f, 293.66, 10.2);
        let verdicts = f.take_verdicts();
        let kinds: Vec<Verdict> = verdicts.iter().map(|v| v.verdict).collect();
        assert_eq!(
            kinds,
            vec![Verdict::Hit, Verdict::Hit],
            "a restart re-judges without phantom misses: {verdicts:?}"
        );
    }

    /// Skip-credit evidence must postdate the last judged advance: a pitch
    /// played EARLIER in the piece (still sitting in the played window) is
    /// not proof the skipped copy of it sounded. Score D-C-D-F, played
    /// D, C, F: the skipped second D's only window evidence is the opening
    /// D, which predates the C advance — it must read Missed, not Hit.
    #[test]
    fn stale_window_evidence_does_not_credit_a_skipped_note() {
        let midis = [62u8, 60, 62, 65]; // D4 C4 D4 F4
        let hz = [293.66, 261.63, 293.66, 349.23];
        let note = |i: usize| ScoreNote {
            pitch_hz: hz[i],
            midi_number: midis[i],
            duration_beats: 1.0,
            start_beat: i as f64,
            dynamic: None,
            is_rest: false,
        };
        let score = ScoreModel {
            title: "Skip Evidence".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: (0..4).map(note).collect(),
            }],
        };
        let mut f = ScoreFollower::new(score).unwrap();
        play_note(&mut f, 293.66, 0.0); // D4 → idx 0
        play_note(&mut f, 261.63, 0.2); // C4 → idx 1
        play_note(&mut f, 349.23, 0.4); // F4 → idx 3, skipping idx 2 (D4)
        let verdicts = f.take_verdicts();
        let kinds: Vec<Verdict> = verdicts.iter().map(|v| v.verdict).collect();
        assert_eq!(
            kinds,
            vec![Verdict::Hit, Verdict::Hit, Verdict::Missed, Verdict::Hit],
            "the skipped D was never re-played — stale frames from the \
             opening D must not credit it: {verdicts:?}"
        );
    }

    /// Cost ties resolve to the index nearest the current position. The
    /// reachable tie: one frame of the PREVIOUS note (a common detector
    /// artifact at a note release) when the interval back to it is exactly
    /// 3 semitones — staying on the current note (300 cents wrong) and
    /// stepping backward (BACKWARD_STEP_PENALTY + perfect match) then cost
    /// the same. The cursor must hold its ground; the old first-minimum
    /// rule flicked it backward.
    #[test]
    fn a_release_artifact_frame_does_not_flick_the_cursor_back() {
        // Two notes, 3 semitones apart: C4 then Eb4.
        let score = ScoreModel {
            title: "Release".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![Measure {
                number: 1,
                notes: vec![
                    ScoreNote {
                        pitch_hz: 261.63,
                        midi_number: 60,
                        duration_beats: 1.0,
                        start_beat: 0.0,
                        dynamic: None,
                        is_rest: false,
                    },
                    ScoreNote {
                        pitch_hz: 311.13,
                        midi_number: 63,
                        duration_beats: 1.0,
                        start_beat: 1.0,
                        dynamic: None,
                        is_rest: false,
                    },
                ],
            }],
        };
        let mut f = ScoreFollower::new(score).unwrap();
        play_note(&mut f, 261.63, 0.0); // C4 → idx 0
        play_note(&mut f, 311.13, 0.2); // Eb4 → idx 1
        assert_eq!(f.current_position().expected_note, Some(63));
        // One stray C4 frame at the release.
        let pos = f.align(&create_audio_event(261.63, 0.9, 0.4));
        assert_eq!(
            pos.expected_note,
            Some(63),
            "a single relapsed frame ties the costs and must not move the cursor"
        );
    }

    /// An 8-note two-measure C major scale, for jump/skip scenarios.
    fn create_two_measure_score() -> ScoreModel {
        let midis = [60u8, 62, 64, 65, 67, 69, 71, 72];
        let hz = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];
        let note = |i: usize, start: f64| ScoreNote {
            pitch_hz: hz[i],
            midi_number: midis[i],
            duration_beats: 1.0,
            start_beat: start,
            dynamic: None,
            is_rest: false,
        };
        ScoreModel {
            title: "Two Measures".to_string(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 120.0,
            grand_staff: false,
            measures: vec![
                Measure {
                    number: 1,
                    notes: (0..4).map(|i| note(i, i as f64)).collect(),
                },
                Measure {
                    number: 2,
                    notes: (4..8).map(|i| note(i, (i - 4) as f64)).collect(),
                },
            ],
        }
    }

    /// #337 S2: verdicts are per SCORE NOTE, never per frame — holding one
    /// note across many frames judges it once.
    #[test]
    fn a_held_note_is_judged_once_not_per_frame() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        for i in 0..20 {
            f.align(&create_audio_event(261.63, 0.9, f64::from(i) * 0.02));
        }
        let verdicts = f.take_verdicts();
        assert_eq!(
            verdicts.len(),
            1,
            "one verdict for one held C: {verdicts:?}"
        );
        assert_eq!(verdicts[0].verdict, Verdict::Hit);
    }

    /// #337 S2: reset clears the judge — no verdicts leak across sessions,
    /// and judging restarts cleanly.
    #[test]
    fn reset_clears_the_verdict_engine() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        play_note(&mut f, 261.63, 0.0);
        let _ = f.take_verdicts(); // drain whatever fired
        play_note(&mut f, 293.66, 0.2);
        f.reset();
        assert!(
            f.take_verdicts().is_empty(),
            "reset must drop undelivered verdicts"
        );
        play_note(&mut f, 261.63, 10.0);
        let after = f.take_verdicts();
        assert_eq!(after.len(), 1, "judging restarts after reset: {after:?}");
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
            note_info: None,
        };

        let pos_before = follower.current_position();
        let pos_after = follower.align(&event);

        assert_eq!(pos_before, pos_after);
    }

    #[test]
    fn align_resets_on_silence_gap() {
        let mut follower = ScoreFollower::new(create_two_measure_score()).unwrap();

        // Play up to A4 (idx 5, measure 2) …
        for (i, hz) in [261.63, 293.66, 329.63, 349.23, 392.0, 440.0]
            .iter()
            .enumerate()
        {
            play_note(&mut follower, *hz, i as f64 * 0.1);
        }
        assert_eq!(follower.current_position().measure_number, 2);

        // … then a real break (> 1.5 s) and a restart from the top. The
        // reset must clear the cost matrix so the entry re-anchors by pure
        // pitch match at the start, not continue the old path.
        let pos = follower.align(&create_audio_event(261.63, 0.9, 10.0));
        assert_eq!(pos.measure_number, 1);
        assert_eq!(pos.expected_note, Some(60));
    }

    /// VA #347: a breath between detached notes (≤ ~1 s) is articulation,
    /// not a rehearsal break — it must NOT reset the alignment or the
    /// verdict judge. Under the old 0.3 s threshold every detached quarter
    /// note wiped the watermark and a cleanly sung pass judged almost
    /// nothing.
    #[test]
    fn a_breath_between_notes_does_not_reset_the_judge() {
        let mut f = ScoreFollower::new(create_simple_score()).unwrap();
        play_note(&mut f, 261.63, 0.0); // C4
        let _ = f.take_verdicts();
        // 1.0 s of silence — a breath, under the 1.5 s relocation threshold.
        play_note(&mut f, 293.66, 1.1);
        let verdicts = f.take_verdicts();
        assert_eq!(
            verdicts.iter().map(|v| v.verdict).collect::<Vec<_>>(),
            vec![Verdict::Hit],
            "the D after a breath is judged, not silently re-anchored: {verdicts:?}"
        );
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
            expected_note: Some(64),
        };

        let json = serde_json::to_string(&pos).expect("serialize ScorePosition");
        let roundtrip: ScorePosition =
            serde_json::from_str(&json).expect("deserialize ScorePosition");

        assert_eq!(roundtrip.measure_number, 5);
        assert_eq!(roundtrip.beat, 2.5);
        assert_eq!(roundtrip.expected_note, Some(64));
    }

    /// Recap blobs persisted before the dead `section_name` field was
    /// removed (#496) still carry it; loading them must keep working, so
    /// `ScorePosition` must tolerate the unknown field. Historical blobs
    /// only ever hold `null` (the follower always wrote `None`); the
    /// string case is the stricter tolerance check.
    #[test]
    fn score_position_loads_legacy_blobs_with_section_name() {
        let historical =
            r#"{"measure_number":5,"beat":2.5,"section_name":null,"expected_note":64}"#;
        let pos: ScorePosition = serde_json::from_str(historical).expect("historical blob loads");
        assert_eq!(pos.measure_number, 5);
        assert_eq!(pos.beat, 2.5);
        assert_eq!(pos.expected_note, Some(64));

        let stricter =
            r#"{"measure_number":5,"beat":2.5,"section_name":"Chorus","expected_note":64}"#;
        let pos: ScorePosition = serde_json::from_str(stricter).expect("legacy blob loads");
        assert_eq!(pos.measure_number, 5);
        assert_eq!(pos.beat, 2.5);
        assert_eq!(pos.expected_note, Some(64));
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

    #[test]
    fn from_musicxml_str_parses_and_follows() {
        // Build a follower straight from the raw MusicXML string the desktop
        // app stores inline — no temp file, no path. The fixture is a real
        // C-major scale, so aligning a C4 must land us on its measure.
        let xml = include_str!("../tests/fixtures/simple_scale.musicxml");
        let mut follower =
            ScoreFollower::from_musicxml_str(xml, 0).expect("fixture is valid MusicXML");

        let c4 = create_audio_event(261.63, 0.9, 0.0);
        let pos = follower.align(&c4);
        assert_eq!(
            pos.measure_number, 1,
            "first aligned note should sit in measure 1"
        );
    }

    #[test]
    fn from_musicxml_str_rejects_garbage() {
        let err = ScoreFollower::from_musicxml_str("not xml at all", 0).unwrap_err();
        assert!(
            matches!(err, FollowerError::Parse(_)),
            "garbage input must surface as a Parse error, got {err:?}"
        );
    }
}
