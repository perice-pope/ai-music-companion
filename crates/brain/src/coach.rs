//! The guided lesson: an adaptive Random-Variations routine (#254, epic #252).
//!
//! Composes the two foundations — F1 (`variations::generate`, the deterministic
//! RV drill generator) and F2 (`learner`, per-key mastery + the difficulty
//! step) — into a taught lesson: build a drill, hear the player, grade what was
//! played against the drill's exact target, move the difficulty one bounded
//! step, repeat, then fold every result back into the Learner Model.
//!
//! Everything here is **pure and deterministic** given its inputs (the seed
//! comes from the lesson spec; time is injected at the persistence boundary).
//! Scoring note: free play deliberately avoids per-note verdicts ("coach,
//! don't judge") — but a drill has an explicit, user-accepted target, so
//! grading against `target_midi` is the honest signal here (spec #254 §2).

use serde::{Deserialize, Serialize};

use crate::learner::{
    apply_difficulty, apply_drill_result, DrillResult, LearnerModel, MAX_DIFFICULTY,
};
use crate::score::{KeyMode, KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};
use variations::{
    generate, ArpeggioPattern, ChordModifier, ChordType, DirectionMode, Enclosure,
    GeneratedSequence, IntervalModifier, RhythmSpec, ScaleModifier, ScalePattern, ScaleType,
    VariationSpec,
};

/// The canonical drill kinds, in play order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrillKind {
    WarmupScale,
    ArpeggioEnclosure,
    IntervalDrill,
    RunThrough,
}

/// The canonical routine order. `drill_count` truncates from the front, so a
/// 3-drill lesson is warmup → arpeggio → run-through.
const KIND_ORDER_4: [DrillKind; 4] = [
    DrillKind::WarmupScale,
    DrillKind::ArpeggioEnclosure,
    DrillKind::IntervalDrill,
    DrillKind::RunThrough,
];
const KIND_ORDER_3: [DrillKind; 3] = [
    DrillKind::WarmupScale,
    DrillKind::ArpeggioEnclosure,
    DrillKind::RunThrough,
];

/// What the user asked for. The seed makes the whole lesson reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LessonSpec {
    pub seed: u64,
    /// Number of drills, clamped to 3..=4.
    pub drill_count: u8,
    /// Taken from `LearnerModel.difficulty` at lesson start (clamped on read).
    pub start_difficulty: u8,
}

/// One drill: the F1 spec + its generated sequence, tagged with the key/scale
/// it trains (for F2 mastery) and the difficulty it was built at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drill {
    pub index: u8,
    pub kind: DrillKind,
    pub difficulty: u8,
    /// Tonic pitch class this drill trains (0–11).
    pub tonic: u8,
    /// Material label for the mastery key, e.g. `"dorian"` / `"major triad"`.
    pub mode: String,
    pub spec: VariationSpec,
    pub sequence: GeneratedSequence,
}

/// Per-note grade of one played execution against the drill's target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteGrade {
    /// The target note (MIDI).
    pub target_midi: u8,
    /// The played pitch matched to this target, if any (Hz).
    pub played_hz: Option<f64>,
    /// Cents deviation of the played pitch from the target's pitch class.
    pub cents_deviation: Option<f64>,
    /// Matched within tolerance.
    pub correct: bool,
}

/// The grade for one played drill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillScore {
    pub per_note: Vec<NoteGrade>,
    /// 0..1 — correct notes / target length. The signal the ramp runs on.
    pub accuracy: f32,
    /// Same basis as `accuracy` (pitch is the graded dimension in v1).
    pub pitch_accuracy: f32,
    /// 0..1 — steadiness proxy: how closely the detected onset count tracks
    /// the target count (a full per-note timing grade needs aligned onsets,
    /// which the v1 pitch-track heuristic doesn't provide — documented in §10).
    pub timing_accuracy: f32,
}

/// Ramp thresholds: ≥ high → +1 difficulty, ≤ low → −1, else unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RampThresholds {
    pub high: f32,
    pub low: f32,
}

impl Default for RampThresholds {
    fn default() -> Self {
        Self {
            high: 0.85,
            low: 0.60,
        }
    }
}

/// Recap of a finished lesson.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LessonRecap {
    pub drill_labels: Vec<String>,
    pub drill_accuracies: Vec<f32>,
    pub start_difficulty: u8,
    pub end_difficulty: u8,
}

// ---------------------------------------------------------------------------
// The difficulty ladder — difficulty step → concrete RV knobs. Pure tables.
// ---------------------------------------------------------------------------

/// Root count per difficulty step (more keys = less autopilot).
const ROOTS_BY_DIFFICULTY: [usize; 10] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 12];

/// Tempo per difficulty step.
fn tempo_for(difficulty: u8) -> f64 {
    60.0 + 4.0 * f64::from(difficulty.min(MAX_DIFFICULTY))
}

/// Scale hardness ladder for warmup/run-through drills.
fn scale_for(difficulty: u8) -> ScaleType {
    match difficulty {
        0..=2 => ScaleType::Major,
        3..=4 => ScaleType::Mixolydian,
        5..=6 => ScaleType::Dorian,
        7..=8 => ScaleType::MelodicMinor,
        _ => ScaleType::HarmonicMinor,
    }
}

/// Chord hardness ladder for arpeggio drills.
fn chord_for(difficulty: u8) -> ChordType {
    match difficulty {
        0..=2 => ChordType::MajorTriad,
        3..=4 => ChordType::MinorTriad,
        5..=6 => ChordType::Dominant7,
        7..=8 => ChordType::Minor7,
        _ => ChordType::HalfDiminished7,
    }
}

/// Interval ladder for interval drills (semitones).
fn interval_for(difficulty: u8) -> u8 {
    match difficulty {
        0..=2 => 4, // major third
        3..=4 => 5, // fourth
        5..=6 => 7, // fifth
        7..=8 => 9, // sixth
        _ => 12,    // octave
    }
}

/// The 12 chromatic roots starting from C4, rotated so `tonic` leads.
fn roots_for(tonic: u8, count: usize) -> Vec<u8> {
    let base = 60 + i16::from(tonic % 12);
    (0..count.clamp(1, 12))
        .map(|i| (base + (i as i16 * 7)) % 12 + 60) // walk the circle of fifths
        .map(|m| m as u8)
        .collect()
}

/// Map one drill kind at one difficulty to a concrete F1 spec.
fn spec_for(kind: DrillKind, difficulty: u8, tonic: u8) -> (VariationSpec, String) {
    let d = difficulty.min(MAX_DIFFICULTY);
    let roots = roots_for(tonic, ROOTS_BY_DIFFICULTY[d as usize]);
    let rhythm = RhythmSpec {
        notes_per_beat: if d >= 6 { 3 } else { 2 },
        tempo_bpm: tempo_for(d),
        rest_beats_between_roots: if d >= 4 { 1.0 } else { 2.0 },
    };
    let direction = if d >= 5 {
        DirectionMode::RandomPerRoot
    } else {
        DirectionMode::Forward
    };
    let randomize_roots = d >= 2;

    match kind {
        DrillKind::WarmupScale => {
            let scale = scale_for(d);
            let spec = VariationSpec {
                roots,
                scale: Some(ScaleModifier {
                    scale,
                    pattern: if d >= 3 {
                        ScalePattern::UpDown
                    } else {
                        ScalePattern::Up
                    },
                }),
                chord: None,
                interval: None,
                enclosure: None,
                direction,
                rhythm,
                randomize_roots,
            };
            (spec, scale.label().to_lowercase())
        }
        DrillKind::ArpeggioEnclosure => {
            let chord = chord_for(d);
            let spec = VariationSpec {
                roots,
                scale: None,
                chord: Some(ChordModifier {
                    chord,
                    pattern: if d >= 3 {
                        ArpeggioPattern::UpDown
                    } else {
                        ArpeggioPattern::Ascending
                    },
                    inversion: if d >= 7 { 1 } else { 0 },
                }),
                interval: None,
                enclosure: (d >= 5).then_some(Enclosure::OneDown),
                direction,
                rhythm,
                randomize_roots,
            };
            (spec, chord.label().to_lowercase())
        }
        DrillKind::IntervalDrill => {
            let semitones = interval_for(d);
            let spec = VariationSpec {
                roots,
                scale: None,
                chord: None,
                interval: Some(IntervalModifier {
                    semitones,
                    ascending: true,
                }),
                enclosure: None,
                direction,
                rhythm,
                randomize_roots,
            };
            (spec, format!("interval {semitones}"))
        }
        DrillKind::RunThrough => {
            // The performance pass: the warmup material with the randomization
            // fully on, regardless of step.
            let scale = scale_for(d);
            let spec = VariationSpec {
                roots,
                scale: Some(ScaleModifier {
                    scale,
                    pattern: ScalePattern::UpDown,
                }),
                chord: None,
                interval: None,
                enclosure: None,
                direction: DirectionMode::RandomPerRoot,
                rhythm,
                randomize_roots: true,
            };
            (spec, scale.label().to_lowercase())
        }
    }
}

/// Pick the tonic this lesson trains: the least-practiced key in the model
/// (fewest attempts, then lowest smoothed accuracy), tie-broken by a seeded
/// rotation so fresh models still vary lesson to lesson. Deterministic for a
/// fixed `(model, seed)`.
fn pick_tonic(model: &LearnerModel, seed: u64) -> u8 {
    let score_of = |tonic: u8| -> (u32, u32) {
        // Sum across modes for this tonic (mastery keys are "tonic:mode").
        let prefix = format!("{tonic}:");
        let (mut attempts, mut ewma_milli) = (0u32, 0u32);
        for (k, m) in &model.key_mastery {
            if k.starts_with(&prefix) {
                attempts = attempts.saturating_add(m.attempts);
                ewma_milli = ewma_milli.saturating_add((m.accuracy_ewma * 1000.0) as u32);
            }
        }
        (attempts, ewma_milli)
    };
    let rotation = (seed % 12) as u8;
    (0..12u8)
        .min_by_key(|&t| {
            let (attempts, ewma) = score_of(t);
            // Least attempts first, then lowest accuracy, then seeded rotation.
            (attempts, ewma, (t + 12 - rotation) % 12)
        })
        .unwrap_or(0)
}

fn kind_at(index: u8, drill_count: u8) -> Option<DrillKind> {
    if drill_count <= 3 {
        KIND_ORDER_3.get(index as usize).copied()
    } else {
        KIND_ORDER_4.get(index as usize).copied()
    }
}

/// Per-drill seed: decorrelated from the lesson seed so regenerating drill N
/// never depends on how many drills preceded it.
fn drill_seed(lesson_seed: u64, index: u8) -> u64 {
    lesson_seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(index))
}

fn build_drill(lesson: &LessonSpec, index: u8, difficulty: u8, tonic: u8) -> Option<Drill> {
    let count = lesson.drill_count.clamp(3, 4);
    let kind = kind_at(index, count)?;
    let d = difficulty.min(MAX_DIFFICULTY);
    let (spec, mode) = spec_for(kind, d, tonic);
    let sequence = generate(&spec, drill_seed(lesson.seed, index));
    Some(Drill {
        index,
        kind,
        difficulty: d,
        tonic,
        mode,
        spec,
        sequence,
    })
}

/// Build drill 0 from the lesson spec + current learner state. Deterministic
/// for a fixed `(LessonSpec, LearnerModel)`.
pub fn build_first(lesson: &LessonSpec, model: &LearnerModel) -> Drill {
    let tonic = pick_tonic(model, lesson.seed);
    build_drill(
        lesson,
        0,
        lesson.start_difficulty.min(MAX_DIFFICULTY),
        tonic,
    )
    .expect("index 0 always exists in a 3..=4 drill routine")
}

/// The single bounded ramp rule: ≥ high → +1, ≤ low → −1, else unchanged;
/// clamped to `0..=MAX_DIFFICULTY`. NaN accuracy is treated as 0 (ramps down).
pub fn next_difficulty(current: u8, accuracy: f32, t: &RampThresholds) -> u8 {
    let a = if accuracy.is_nan() { 0.0 } else { accuracy };
    if a >= t.high {
        current.saturating_add(1).min(MAX_DIFFICULTY)
    } else if a <= t.low {
        current.saturating_sub(1)
    } else {
        current
    }
}

/// Grade the completed drill and build the next one — or `None` when the
/// routine is done.
pub fn advance(prev: &Drill, score: &DrillScore, lesson: &LessonSpec) -> Option<Drill> {
    let difficulty = next_difficulty(prev.difficulty, score.accuracy, &RampThresholds::default());
    build_drill(lesson, prev.index + 1, difficulty, prev.tonic)
}

// ---------------------------------------------------------------------------
// Scoring — align what was played to the drill's exact target.
// ---------------------------------------------------------------------------

/// One played note (from the pitch-track collapse below).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayedNote {
    pub hz: f64,
}

fn hz_to_midi_f(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Pitch-class match (octave-agnostic): a voice hums the right scale degrees
/// wherever its range sits; demanding the exact octave would fail every
/// singer whose voice sits below the drill's written register.
fn class_matches(target_midi: u8, played_hz: f64) -> bool {
    let played = hz_to_midi_f(played_hz);
    let played_class = played.round().rem_euclid(12.0) as i32 % 12;
    i32::from(target_midi % 12) == played_class
}

/// Cents deviation of `played_hz` from the nearest octave of the target class.
fn cents_from_class(target_midi: u8, played_hz: f64) -> f64 {
    let played = hz_to_midi_f(played_hz);
    let diff = (played - f64::from(target_midi)).rem_euclid(12.0);
    let wrapped = if diff > 6.0 { diff - 12.0 } else { diff };
    wrapped * 100.0
}

/// Grade `played` against `target_midi` with an order-preserving alignment
/// (LCS on pitch class). Deterministic. Extra played notes are ignored;
/// unmatched targets are misses. `onset_count` feeds the timing proxy.
pub fn score_drill(target_midi: &[u8], played: &[PlayedNote], onset_count: usize) -> DrillScore {
    let n = target_midi.len();
    let m = played.len();
    // LCS table over pitch-class matches.
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if class_matches(target_midi[i], played[j].hz) {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    // Walk the alignment to grade each target note.
    let mut per_note = Vec::with_capacity(n);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n {
        if j < m && class_matches(target_midi[i], played[j].hz) && dp[i][j] == dp[i + 1][j + 1] + 1
        {
            per_note.push(NoteGrade {
                target_midi: target_midi[i],
                played_hz: Some(played[j].hz),
                cents_deviation: Some(cents_from_class(target_midi[i], played[j].hz)),
                correct: true,
            });
            i += 1;
            j += 1;
        } else if j < m && dp[i][j + 1] >= dp[i + 1][j] {
            j += 1; // skip an extra played note
        } else {
            per_note.push(NoteGrade {
                target_midi: target_midi[i],
                played_hz: None,
                cents_deviation: None,
                correct: false,
            });
            i += 1;
        }
    }

    let correct = per_note.iter().filter(|g| g.correct).count();
    let accuracy = if n == 0 {
        0.0
    } else {
        correct as f32 / n as f32
    };
    let timing_accuracy = if n == 0 {
        0.0
    } else {
        1.0 - ((onset_count as f32 - n as f32).abs() / n as f32).min(1.0)
    };
    DrillScore {
        per_note,
        accuracy,
        pitch_accuracy: accuracy,
        timing_accuracy,
    }
}

/// Collapse a raw sampled pitch track (per-audio-event Hz, ~100 Hz rate) into
/// discrete played notes: consecutive samples rounding to the same MIDI note
/// merge into one, and runs shorter than `min_run` samples are dropped as
/// flicker. v1 heuristic — honest about its limits (no per-note onsets).
pub fn played_notes_from_pitch_track(pitches: &[f64], min_run: usize) -> Vec<PlayedNote> {
    let mut notes = Vec::new();
    let mut run_start = 0usize;
    let mut run_midi: Option<i32> = None;
    let flush = |notes: &mut Vec<PlayedNote>, pitches: &[f64], start: usize, end: usize| {
        if end - start >= min_run.max(1) {
            let mean = pitches[start..end].iter().sum::<f64>() / (end - start) as f64;
            notes.push(PlayedNote { hz: mean });
        }
    };
    for (idx, &hz) in pitches.iter().enumerate() {
        if !(hz.is_finite() && hz > 0.0) {
            if run_midi.is_some() {
                flush(&mut notes, pitches, run_start, idx);
                run_midi = None;
            }
            continue;
        }
        let midi = hz_to_midi_f(hz).round() as i32;
        match run_midi {
            Some(current) if current == midi => {}
            Some(_) => {
                flush(&mut notes, pitches, run_start, idx);
                run_start = idx;
                run_midi = Some(midi);
            }
            None => {
                run_start = idx;
                run_midi = Some(midi);
            }
        }
    }
    if run_midi.is_some() {
        flush(&mut notes, pitches, run_start, pitches.len());
    }
    notes
}

// ---------------------------------------------------------------------------
// Lesson end — fold results into the Learner Model.
// ---------------------------------------------------------------------------

/// Fold every drill result through F2 and set the final difficulty. Pure given
/// inputs; `now_epoch_secs` is injected. Returns the new model + the recap.
pub fn finish_lesson(
    model: &LearnerModel,
    drills: &[(Drill, DrillScore)],
    now_epoch_secs: i64,
) -> (LearnerModel, LessonRecap) {
    let mut next = model.clone();
    for (drill, score) in drills {
        next = apply_drill_result(
            &next,
            &DrillResult {
                tonic: drill.tonic,
                mode: drill.mode.clone(),
                accuracy: score.accuracy,
            },
            now_epoch_secs,
        );
    }
    let start_difficulty = drills.first().map(|(d, _)| d.difficulty).unwrap_or(0);
    let end_difficulty = drills
        .last()
        .map(|(d, s)| next_difficulty(d.difficulty, s.accuracy, &RampThresholds::default()))
        .unwrap_or(start_difficulty);
    next = apply_difficulty(&next, end_difficulty, now_epoch_secs);
    let recap = LessonRecap {
        drill_labels: drills
            .iter()
            .map(|(d, _)| d.sequence.label.clone())
            .collect(),
        drill_accuracies: drills.iter().map(|(_, s)| s.accuracy).collect(),
        start_difficulty,
        end_difficulty,
    };
    (next, recap)
}

// ---------------------------------------------------------------------------
// Notation adapter — GeneratedSequence → ScoreModel (→ MusicXML → ScoreView).
// ---------------------------------------------------------------------------

fn midi_to_hz(midi: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0)
}

/// Adapt a generated drill to the app's `ScoreModel` so the existing
/// MusicXML emitter, ScoreView, and follower consume it unchanged. Gaps on the
/// beat grid become explicit rests; notes never cross barlines (a note is
/// clipped at the measure boundary — drill figures are grid-aligned so this
/// only trims the final sustain).
pub fn sequence_to_score_model(seq: &GeneratedSequence, title: &str) -> ScoreModel {
    let bpm = f64::from(seq.beats_per_measure.max(1));
    let total_beats = seq
        .notes
        .last()
        .map(|n| n.start_beat + n.duration_beats)
        .unwrap_or(0.0);
    let measure_count = (total_beats / bpm).ceil().max(1.0) as usize;
    let mut measures: Vec<Measure> = (0..measure_count)
        .map(|i| Measure {
            number: i + 1,
            notes: Vec::new(),
        })
        .collect();

    let mut cursor = 0.0_f64;
    for note in &seq.notes {
        // Gap before this note → rests, measure by measure.
        push_span(&mut measures, bpm, cursor, note.start_beat, None);
        let end = note.start_beat + note.duration_beats;
        push_span(&mut measures, bpm, note.start_beat, end, Some(note.midi));
        cursor = end;
    }
    // Pad the final measure with a rest so it adds up.
    let final_end = measure_count as f64 * bpm;
    push_span(&mut measures, bpm, cursor, final_end, None);

    ScoreModel {
        title: title.to_owned(),
        composer: None,
        instrument: None,
        time_signature: TimeSignature {
            beats: seq.beats_per_measure.max(1),
            beat_type: 4,
        },
        key_signature: KeySignature {
            fifths: 0,
            mode: KeyMode::Major,
        },
        tempo_bpm: seq.tempo_bpm,
        measures,
    }
}

/// Append one span (note or rest) to the measures, splitting at barlines.
fn push_span(
    measures: &mut [Measure],
    beats_per_measure: f64,
    start: f64,
    end: f64,
    midi: Option<u8>,
) {
    let mut at = start;
    while end - at > 1e-9 {
        let measure_idx = (at / beats_per_measure).floor() as usize;
        if measure_idx >= measures.len() {
            break;
        }
        let measure_end = (measure_idx as f64 + 1.0) * beats_per_measure;
        let span_end = end.min(measure_end);
        let duration = span_end - at;
        if duration > 1e-9 {
            measures[measure_idx].notes.push(ScoreNote {
                pitch_hz: midi.map(midi_to_hz).unwrap_or(0.0),
                midi_number: midi.unwrap_or(0),
                duration_beats: duration,
                start_beat: at - measure_idx as f64 * beats_per_measure,
                dynamic: None,
                is_rest: midi.is_none(),
            });
        }
        at = span_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learner::Mastery;

    fn lesson(seed: u64) -> LessonSpec {
        LessonSpec {
            seed,
            drill_count: 4,
            start_difficulty: 0,
        }
    }

    fn perfect_score(drill: &Drill) -> DrillScore {
        let played: Vec<PlayedNote> = drill
            .sequence
            .target_midi
            .iter()
            .map(|&m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        score_drill(
            &drill.sequence.target_midi,
            &played,
            drill.sequence.target_midi.len(),
        )
    }

    /// #254 AC1: a lesson yields the canonical kinds in order with concrete
    /// targets; deterministic for a fixed (spec, model).
    #[test]
    fn routine_is_canonical_and_deterministic() {
        let model = LearnerModel::default();
        let spec = lesson(11);
        let d0 = build_first(&spec, &model);
        assert_eq!(d0.kind, DrillKind::WarmupScale);
        assert!(!d0.sequence.target_midi.is_empty());
        assert_eq!(d0, build_first(&spec, &model), "must be deterministic");

        let d1 = advance(&d0, &perfect_score(&d0), &spec).unwrap();
        assert_eq!(d1.kind, DrillKind::ArpeggioEnclosure);
        let d2 = advance(&d1, &perfect_score(&d1), &spec).unwrap();
        assert_eq!(d2.kind, DrillKind::IntervalDrill);
        let d3 = advance(&d2, &perfect_score(&d2), &spec).unwrap();
        assert_eq!(d3.kind, DrillKind::RunThrough);
        assert!(
            advance(&d3, &perfect_score(&d3), &spec).is_none(),
            "routine ends after drill_count"
        );
    }

    /// A 3-drill lesson drops the interval drill, not the run-through.
    #[test]
    fn three_drill_lesson_keeps_the_run_through() {
        let mut spec = lesson(1);
        spec.drill_count = 3;
        let model = LearnerModel::default();
        let d0 = build_first(&spec, &model);
        let d1 = advance(&d0, &perfect_score(&d0), &spec).unwrap();
        let d2 = advance(&d1, &perfect_score(&d1), &spec).unwrap();
        assert_eq!(d2.kind, DrillKind::RunThrough);
        assert!(advance(&d2, &perfect_score(&d2), &spec).is_none());
    }

    /// #254 AC3: the ramp moves exactly one bounded step per drill.
    #[test]
    fn ramp_moves_one_bounded_step() {
        let t = RampThresholds::default();
        assert_eq!(next_difficulty(3, 0.9, &t), 4, "high accuracy → +1");
        assert_eq!(next_difficulty(3, 0.5, &t), 2, "low accuracy → -1");
        assert_eq!(next_difficulty(3, 0.7, &t), 3, "middling → unchanged");
        // AC5: bounded.
        assert_eq!(next_difficulty(MAX_DIFFICULTY, 1.0, &t), MAX_DIFFICULTY);
        assert_eq!(next_difficulty(0, 0.0, &t), 0);
        assert_eq!(next_difficulty(3, f32::NAN, &t), 2, "NaN ramps down");
    }

    /// The ladder is monotonic where it must be: higher difficulty never means
    /// fewer roots or a slower tempo.
    #[test]
    fn ladder_is_monotonic_in_roots_and_tempo() {
        for d in 0..MAX_DIFFICULTY {
            assert!(ROOTS_BY_DIFFICULTY[d as usize] <= ROOTS_BY_DIFFICULTY[d as usize + 1]);
            assert!(tempo_for(d) < tempo_for(d + 1) + 1e-9);
        }
    }

    /// #254 AC2 (scoring): a perfect performance scores 1.0 with per-note
    /// grades; a missed note is graded incorrect and lowers accuracy; extra
    /// played notes don't inflate the score.
    #[test]
    fn score_drill_grades_per_note_against_the_target() {
        let target = [60u8, 62, 64];
        let played: Vec<PlayedNote> = [60u8, 62, 64]
            .iter()
            .map(|&m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        let s = score_drill(&target, &played, 3);
        assert_eq!(s.accuracy, 1.0);
        assert!(s.per_note.iter().all(|g| g.correct));

        // Miss the middle note.
        let played_miss: Vec<PlayedNote> = [60u8, 64]
            .iter()
            .map(|&m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        let s2 = score_drill(&target, &played_miss, 2);
        assert!((s2.accuracy - 2.0 / 3.0).abs() < 1e-6);
        assert!(!s2.per_note[1].correct);
        assert!(s2.per_note[1].played_hz.is_none());

        // Extra wrong notes between correct ones don't help.
        let played_extra: Vec<PlayedNote> = [60u8, 61, 62, 63, 64]
            .iter()
            .map(|&m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        let s3 = score_drill(&target, &played_extra, 5);
        assert_eq!(s3.accuracy, 1.0, "alignment skips extras");
    }

    /// Octave-agnostic matching: a singer an octave below the written drill
    /// still matches, and the cents deviation is measured from the class.
    #[test]
    fn scoring_is_octave_agnostic_with_cents() {
        let target = [72u8];
        let played = [PlayedNote {
            hz: midi_to_hz(60) * 1.01, // C4, slightly sharp
        }];
        let s = score_drill(&target, &played, 1);
        assert!(s.per_note[0].correct);
        let cents = s.per_note[0].cents_deviation.unwrap();
        assert!(cents > 5.0 && cents < 30.0, "≈17 cents sharp, got {cents}");
    }

    /// Empty target → zero accuracy, no panic; empty played → all misses.
    #[test]
    fn scoring_edge_cases() {
        assert_eq!(score_drill(&[], &[], 0).accuracy, 0.0);
        let s = score_drill(&[60], &[], 0);
        assert_eq!(s.accuracy, 0.0);
        assert!(!s.per_note[0].correct);
    }

    /// The pitch-track collapse merges stable runs, drops flicker, and skips
    /// unvoiced (0/NaN) samples.
    #[test]
    fn pitch_track_collapses_to_notes() {
        let a440 = 440.0;
        let b494 = 493.88;
        let mut track = vec![a440; 10];
        track.push(1234.0); // 1-sample flicker
        track.extend(vec![0.0; 3]); // silence
        track.extend(vec![b494; 8]);
        track.push(f64::NAN);
        let notes = played_notes_from_pitch_track(&track, 3);
        assert_eq!(notes.len(), 2);
        assert!((hz_to_midi_f(notes[0].hz).round() - 69.0).abs() < 0.5);
        assert!((hz_to_midi_f(notes[1].hz).round() - 71.0).abs() < 0.5);
    }

    /// #254 AC4: finishing a lesson writes mastery deltas + the final
    /// difficulty into the Learner Model, and the recap reports both ends.
    #[test]
    fn finish_lesson_updates_the_learner_model() {
        let model = LearnerModel::default();
        let spec = lesson(5);
        let d0 = build_first(&spec, &model);
        let s0 = perfect_score(&d0);
        let d1 = advance(&d0, &s0, &spec).unwrap();
        let s1 = perfect_score(&d1);

        let (next, recap) = finish_lesson(&model, &[(d0.clone(), s0), (d1, s1)], 99);
        // Mastery recorded for the trained keys.
        assert!(!next.key_mastery.is_empty());
        assert!(next
            .key_mastery
            .values()
            .all(|m: &Mastery| m.attempts >= 1 && m.accuracy_ewma > 0.9));
        // Perfect drills ramp the difficulty up from 0 (d1 at difficulty 1,
        // perfect → end 2).
        assert_eq!(recap.start_difficulty, 0);
        assert_eq!(recap.end_difficulty, 2);
        assert_eq!(next.difficulty, 2);
        assert_eq!(next.updated_at_epoch_secs, 99);
        assert_eq!(recap.drill_accuracies.len(), 2);
    }

    /// pick_tonic prefers the least-practiced key and is seed-deterministic.
    #[test]
    fn pick_tonic_prefers_unpracticed_keys() {
        let model = LearnerModel::default();
        assert_eq!(pick_tonic(&model, 3), pick_tonic(&model, 3));

        // Practice tonic 0 heavily → a fresh lesson should pick something else.
        let mut m = model.clone();
        for t in 0..5 {
            m = apply_drill_result(
                &m,
                &DrillResult {
                    tonic: 0,
                    mode: "major".to_owned(),
                    accuracy: 1.0,
                },
                t,
            );
        }
        assert_ne!(pick_tonic(&m, 3), 0, "practiced key must not be re-picked");
    }

    /// The notation adapter: notes land in the right measures with in-measure
    /// start beats, gaps become rests, and every measure sums to the bar.
    #[test]
    fn sequence_adapts_to_a_well_formed_score_model() {
        let spec = VariationSpec {
            roots: vec![60, 62],
            scale: Some(ScaleModifier {
                scale: ScaleType::Major,
                pattern: ScalePattern::Up,
            }),
            chord: None,
            interval: None,
            enclosure: None,
            direction: DirectionMode::Forward,
            rhythm: RhythmSpec {
                notes_per_beat: 2,
                tempo_bpm: 90.0,
                rest_beats_between_roots: 1.0,
            },
            randomize_roots: false,
        };
        let seq = generate(&spec, 0);
        let model = sequence_to_score_model(&seq, "Warmup");
        assert_eq!(model.tempo_bpm, 90.0);
        assert!(!model.measures.is_empty());
        for measure in &model.measures {
            let total: f64 = measure.notes.iter().map(|n| n.duration_beats).sum();
            assert!(
                (total - 4.0).abs() < 1e-6,
                "measure {} sums to {total}, not the bar",
                measure.number
            );
            for note in &measure.notes {
                assert!(note.start_beat >= 0.0 && note.start_beat < 4.0);
                assert!(note.is_rest == (note.midi_number == 0));
            }
        }
        // The rendered MusicXML is consumable by the existing emitter.
        let xml = crate::score::emit::score_model_to_musicxml(&model);
        assert!(xml.contains("<score-partwise"));
    }
}
