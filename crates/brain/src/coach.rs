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
pub use variations::GeneratedSequence;
pub use variations::VariationSpec;

use variations::{
    generate, ArpeggioPattern, ChordModifier, ChordType, DirectionMode, Enclosure,
    IntervalModifier, RhythmSpec, ScaleModifier, ScalePattern, ScaleType,
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
    /// 0..1 — the graded signal the ramp runs on: recall (correct/target)
    /// scaled down when far more was played than asked (anti-noodling).
    pub accuracy: f32,
    /// Pure recall (correct / target) before the extras penalty — `accuracy`
    /// additionally scales down when far more was played than asked (anti-
    /// noodling; see `score_drill`).
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
        // Inert since the RV one-cell-per-measure rule: the grid ignores
        // rests (the breath is the remainder of each cell's measure). Kept
        // at 0.0 so nobody mistakes it for a live pedagogical knob.
        rest_beats_between_roots: 0.0,
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
                cell: None,
                degrees: None,
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
                cell: None,
                degrees: None,
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
                cell: None,
                degrees: None,
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
                cell: None,
                degrees: None,
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
    // Recall alone is gameable: because the alignment skips extras for free, a
    // slow chromatic walk contains every target as a subsequence and would
    // grade 100%. Cap the free extras at PLAYED_SLACK× the target length and
    // scale the grade down past it, so noodling collapses while a genuine take
    // with a few flubs is untouched.
    const PLAYED_SLACK: f32 = 1.5;
    let recall = if n == 0 {
        0.0
    } else {
        correct as f32 / n as f32
    };
    let precision_factor = if m == 0 || n == 0 {
        1.0
    } else {
        ((n as f32 * PLAYED_SLACK) / m as f32).min(1.0)
    };
    let accuracy = recall * precision_factor;
    let timing_accuracy = if n == 0 {
        0.0
    } else {
        1.0 - ((onset_count as f32 - n as f32).abs() / n as f32).min(1.0)
    };
    DrillScore {
        per_note,
        accuracy,
        pitch_accuracy: recall,
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

/// Key signature for a drill: `fifths` on the circle plus major/minor family,
/// derived from the tonic pitch class and the drill's material label (#277
/// follow-up: drills used to render keyless — an A# figure showed a wall of
/// accidentals over an implied C major).
///
/// Modes map to their conventional signatures relative to the tonic's major
/// (Dorian −2, Mixolydian −1, Lydian +1, Phrygian −4, minor family −3,
/// Locrian −5); chord/interval material uses the tonic's plain major or minor.
/// The result is clamped into the engravable −7..=7.
/// Display name for a tonic pitch class under a key signature: flat
/// signatures name flats — the "C# major" lesson engraves as Db (5 flats),
/// so its header and colored cells must say "Db", never "C#" (#335; the
/// #277 self-consistency family: surfaces must not visibly contradict each
/// other).
pub fn tonic_display_name(pc: u8, fifths: i8) -> &'static str {
    const SHARP: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    const FLAT: [&str; 12] = [
        "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
    ];
    if fifths < 0 {
        FLAT[usize::from(pc % 12)]
    } else {
        SHARP[usize::from(pc % 12)]
    }
}

pub fn key_signature_for(tonic: u8, mode_label: &str) -> KeySignature {
    /// fifths of the MAJOR key for each tonic pitch class, favoring the flat
    /// spelling where conventional (Db over C#, Eb, Ab, Bb; F# kept for pc 6).
    const MAJOR_FIFTHS: [i8; 12] = [0, -5, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5];
    let label = mode_label.trim().to_lowercase();
    let (offset, minor_family) = if label.contains("dorian") {
        (-2, true)
    } else if label.contains("mixolydian") || label.contains("dominant") {
        // A dominant-7 arpeggio carries the b7 — engrave it mixolydian-style so
        // C7's Bb reads as Bb, not A# (the founder's original report).
        (-1, false)
    } else if label.contains("lydian") {
        (1, false)
    } else if label.contains("phrygian") {
        (-4, true)
    } else if label.contains("locrian") || label.contains("diminished") {
        // Diminished / half-diminished material is flat-heavy — locrian's
        // signature spells it most readably.
        (-5, true)
    } else if label.contains("blues") || label.contains("minor") {
        // Natural / harmonic / melodic minor, minor pentatonic, minor chords:
        // engrave with the natural-minor (relative-major) signature.
        (-3, true)
    } else {
        (0, false)
    };
    // Wrap enharmonically instead of clamping: a raw -8 (Db-minor material) is
    // C# minor (+4), not a wrong Ab-minor signature; and prefer <=6 accidentals
    // so e.g. Db Dorian engraves as C# Dorian (5 sharps) rather than 7 flats.
    // Raw magnitude never exceeds 10, so one wrap suffices.
    let mut fifths = MAJOR_FIFTHS[usize::from(tonic % 12)] + offset;
    if fifths < -6 {
        fifths += 12;
    } else if fifths > 6 {
        fifths -= 12;
    }
    KeySignature {
        fifths,
        mode: if minor_family {
            KeyMode::Minor
        } else {
            KeyMode::Major
        },
    }
}

/// Adapt a generated drill to the app's `ScoreModel` so the existing
/// MusicXML emitter, ScoreView, and follower consume it unchanged. Gaps on the
/// beat grid become explicit rests; notes never cross barlines (a note is
/// clipped at the measure boundary — drill figures are grid-aligned so this
/// only trims the final sustain). `key` engraves the drill's key signature so
/// the notation reads like real music instead of a wall of accidentals.
pub fn sequence_to_score_model(
    seq: &GeneratedSequence,
    title: &str,
    key: KeySignature,
) -> ScoreModel {
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
        key_signature: key,
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

// ---------------------------------------------------------------------------
// Free-play exploration (#255): the ambient suggester. A reveal names a sound;
// these turn it into material — an RV variation seeded from the live key at
// the learner's difficulty, mutated one tapped chip at a time.
// ---------------------------------------------------------------------------

/// A concrete, named change to the active variation. The frontend never
/// constructs these — it echoes back the exact delta attached to a tapped chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VariationDelta {
    /// RV's signature: same material, new shuffled key order.
    ReshuffleRoots,
    /// `[Make it spicy]` = +1 / `[Simpler]` = −1, clamped to the ladder.
    BumpDifficulty { by: i8 },
    /// Swap to a different scale colour (seeded pick, never the current one).
    DifferentScale,
    /// Pull a 4-note pattern from RV's pattern database (#289) — seeded pick,
    /// never the current one; ties to whatever scale is active.
    TryPattern,
    /// Forward ↔ reversed figures.
    ToggleDirection,
}

/// One tappable chip: a label and the exact change it applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChipSpec {
    pub label: String,
    pub delta: VariationDelta,
}

/// The in-flight exploration: the spec that produced the current rep plus the
/// knobs deltas mutate. Seed advances every rep so "again" is always fresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreState {
    pub spec: VariationSpec,
    pub difficulty: u8,
    pub tonic: u8,
    pub seed: u64,
    /// Undo stack (#292 slice 3): every edit AND every chip pushes the full
    /// snapshot it replaced (spec + seed + difficulty), so undo is a universal
    /// "back one step" that restores the EXACT rep the player saw — never a
    /// third state stitched from a stale spec under a new seed. Bounded.
    #[serde(default)]
    pub history: Vec<ExploreSnapshot>,
}

/// One undo point: everything that determines a rep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreSnapshot {
    pub spec: VariationSpec,
    pub seed: u64,
    pub difficulty: u8,
}

fn push_history(next: &mut ExploreState, prev: &ExploreState) {
    next.history.push(ExploreSnapshot {
        spec: prev.spec.clone(),
        seed: prev.seed,
        difficulty: prev.difficulty,
    });
    if next.history.len() > MAX_HISTORY {
        next.history.remove(0);
    }
}

/// Map a live-detected mode label onto the generator's scale space; `None`
/// when the label names no scale we generate (caller falls back to the
/// difficulty ladder's scale).
fn scale_for_mode_label(label: &str) -> Option<ScaleType> {
    let l = label.trim().to_lowercase();
    Some(match l.as_str() {
        "major" | "ionian" => ScaleType::Major,
        "minor" | "aeolian" => ScaleType::NaturalMinor,
        "dorian" => ScaleType::Dorian,
        "mixolydian" => ScaleType::Mixolydian,
        "lydian" => ScaleType::Lydian,
        "phrygian" => ScaleType::Phrygian,
        "locrian" => ScaleType::Locrian,
        _ => return None,
    })
}

/// Seed an exploration from the live key at the learner's difficulty and
/// generate its first rep. Deterministic for a fixed `(tonic, mode, model,
/// seed)`.
pub fn start_explore(
    tonic: u8,
    mode: &str,
    model: &LearnerModel,
    seed: u64,
) -> (ExploreState, GeneratedSequence) {
    let difficulty = model.difficulty.min(MAX_DIFFICULTY);
    let (mut spec, _) = spec_for(DrillKind::WarmupScale, difficulty, tonic);
    if let (Some(scale), Some(m)) = (scale_for_mode_label(mode), spec.scale.as_mut()) {
        // Explore the sound the player is actually in, not the ladder default.
        m.scale = scale;
    }
    let sequence = generate(&spec, seed);
    (
        ExploreState {
            spec,
            difficulty,
            tonic,
            seed,
            history: Vec::new(),
        },
        sequence,
    )
}

/// Struggling threshold for offering `[Simpler]` instead of `[Spicy]`.
const STRUGGLING_EWMA: f32 = 0.6;

/// The ≤3 chips for the current exploration — pure and stable-ordered:
/// 1. `[New keys 🎲]` whenever there is more than one root to shuffle;
/// 2. `[Simpler]` when the learner is struggling on this key (or at the top of
///    the ladder), otherwise `[Make it spicy]` — never a raise at MAX, never a
///    lower at 0;
/// 3. `[Different scale]` always;
/// 4. `[Reverse it]` fills the row only when fewer than 3 chips gathered.
pub fn suggest_chips(state: &ExploreState, model: &LearnerModel) -> Vec<ChipSpec> {
    let mut chips = Vec::new();
    if state.spec.roots.len() > 1 {
        chips.push(ChipSpec {
            label: "New keys 🎲".to_owned(),
            delta: VariationDelta::ReshuffleRoots,
        });
    }
    // Struggling = ANY practiced mode on THIS tonic below the bar (mastery
    // keys are "tonic:mode"; judging by an unrelated key's struggle would
    // gate [Simpler] arbitrarily).
    let tonic_prefix = format!("{}:", state.tonic % 12);
    let struggling = model.key_mastery.iter().any(|(k, m)| {
        k.starts_with(&tonic_prefix) && m.attempts > 0 && m.accuracy_ewma < STRUGGLING_EWMA
    });
    if (struggling || state.difficulty >= MAX_DIFFICULTY) && state.difficulty > 0 {
        chips.push(ChipSpec {
            label: "Simpler".to_owned(),
            delta: VariationDelta::BumpDifficulty { by: -1 },
        });
    } else if state.difficulty < MAX_DIFFICULTY {
        chips.push(ChipSpec {
            label: "Make it spicy".to_owned(),
            delta: VariationDelta::BumpDifficulty { by: 1 },
        });
    }
    // Slot 3 alternates by seed parity (the seed advances every rep), so both
    // the scale palette and the pattern database stay reachable in <=3 chips.
    if state.seed.is_multiple_of(2) {
        chips.push(ChipSpec {
            label: "Different scale".to_owned(),
            delta: VariationDelta::DifferentScale,
        });
    } else {
        chips.push(ChipSpec {
            label: "Try a pattern 🎲".to_owned(),
            delta: VariationDelta::TryPattern,
        });
    }
    if chips.len() < 3 {
        chips.push(ChipSpec {
            label: "Reverse it".to_owned(),
            delta: VariationDelta::ToggleDirection,
        });
    }
    chips.truncate(3);
    chips
}

/// RV's pattern database (#289): classic 4-note degree patterns, named,
/// ordered easy → hard. Each ties to ANY scale (degrees, not pitches).
pub const DEGREE_PATTERNS: [(&str, [u8; 4]); 8] = [
    ("1-2-3-5", [1, 2, 3, 5]),
    ("1-2-3-4", [1, 2, 3, 4]),
    ("5-3-2-1", [5, 3, 2, 1]),
    ("1-3-2-4", [1, 3, 2, 4]),
    ("1-3-5-3", [1, 3, 5, 3]),
    ("3-2-1-5", [3, 2, 1, 5]),
    ("1-4-3-2", [1, 4, 3, 2]),
    ("5-6-5-3", [5, 6, 5, 3]),
];

/// Scales the `[Different scale]` chip cycles through, easy → exotic.
const EXPLORE_SCALES: [ScaleType; 6] = [
    ScaleType::Major,
    ScaleType::MajorPentatonic,
    ScaleType::Mixolydian,
    ScaleType::Dorian,
    ScaleType::Blues,
    ScaleType::HarmonicMinor,
];

/// Apply a tapped delta and generate the next rep. Pure; the seed always
/// advances so every rep is fresh (that's what makes `ReshuffleRoots` — an
/// otherwise spec-identical delta — actually reshuffle).
pub fn apply_explore_delta(
    state: &ExploreState,
    delta: &VariationDelta,
) -> (ExploreState, GeneratedSequence) {
    let mut next = state.clone();
    // Chips are undo-able steps too (#292 review M5): snapshot before acting.
    push_history(&mut next, state);
    next.seed = next
        .seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(1);
    match delta {
        VariationDelta::ReshuffleRoots => {
            next.spec.randomize_roots = true;
        }
        VariationDelta::BumpDifficulty { by } => {
            let d = i16::from(next.difficulty) + i16::from(*by);
            next.difficulty = d.clamp(0, i16::from(MAX_DIFFICULTY)) as u8;
            // Rebuild the knobs at the new step, preserving the explored
            // scale AND any hand-edited cell — a difficulty chip must never
            // silently destroy the player's own material (review M4); the
            // tempo/roots knobs still ramp meaningfully around a cell.
            let scale = next.spec.scale;
            let cell = next.spec.cell.take();
            let degrees = next.spec.degrees.take();
            let (spec, _) = spec_for(DrillKind::WarmupScale, next.difficulty, next.tonic);
            next.spec = spec;
            if let (Some(prev), Some(m)) = (scale, next.spec.scale.as_mut()) {
                m.scale = prev.scale;
            }
            next.spec.cell = cell;
            // Degree patterns are the player's material too (#289) — they
            // survive the ladder rebuild exactly like a hand-edited cell.
            next.spec.degrees = degrees;
        }
        VariationDelta::DifferentScale => {
            // With a hand-edited cell the scale figure is shadowed (cell has
            // precedence) — this chip is then explicitly "fresh material":
            // discard the cell, back to the catalog; undo recovers it
            // (review M3).
            next.spec.cell = None;
            if let Some(m) = next.spec.scale.as_mut() {
                let current = m.scale;
                let idx = (next.seed as usize) % EXPLORE_SCALES.len();
                let pick = EXPLORE_SCALES
                    .iter()
                    .cycle()
                    .skip(idx)
                    .find(|&&sc| sc != current)
                    .copied()
                    .unwrap_or(ScaleType::Major);
                m.scale = pick;
            }
        }
        VariationDelta::TryPattern => {
            // Like DifferentScale, a pattern is "fresh material" vs a hand-
            // edited cell — discard the cell (undo recovers it) and pull a
            // pattern from the database, never the one already playing.
            next.spec.cell = None;
            // Degrees need a scale to map through; give a default rather than
            // silently no-op if explore ever seeds from scale-less material.
            if next.spec.scale.is_none() {
                next.spec.scale = Some(ScaleModifier {
                    scale: ScaleType::Major,
                    pattern: ScalePattern::Up,
                });
            }
            let current = next.spec.degrees.clone();
            let idx = (next.seed as usize) % DEGREE_PATTERNS.len();
            let pick = DEGREE_PATTERNS
                .iter()
                .cycle()
                .skip(idx)
                .map(|(_, d)| d.to_vec())
                .find(|d| Some(d) != current.as_ref())
                .expect("database has >1 pattern");
            next.spec.degrees = Some(pick);
        }
        VariationDelta::ToggleDirection => {
            next.spec.direction = match next.spec.direction {
                DirectionMode::Reversed => DirectionMode::Forward,
                _ => DirectionMode::Reversed,
            };
        }
    }
    let sequence = generate(&next.spec, next.seed);
    (next, sequence)
}

// ---------------------------------------------------------------------------
// Phrase seeding (#285): the flagship RV loop — hear a phrase worth working
// on, lift it as a CELL, row it through the 12 keys. "The player practices
// their own music in every key."
// ---------------------------------------------------------------------------

/// Founder cap: a lifted phrase becomes a cell of at most 17 notes
/// ("more than enough") — longer takes keep their most recent tail.
pub const LIFT_MAX_NOTES: usize = 17;
/// Below this many notes there is nothing worth rowing.
pub const LIFT_MIN_NOTES: usize = 4;
/// Collapse threshold for LIFTING (stricter than grading's): tuned against
/// real voice/trumpet phrases — at 3 samples, pitch-track jitter lifts as
/// chromatic wiggle; at 5, only clearly held notes survive.
pub const LIFT_MIN_RUN: usize = 5;

/// Lift a played pitch track into a cell: collapse to notes, keep the last
/// ≤17, express as semitone offsets from the first note, folding any offset
/// past the ±3-octave range back by octaves (a wild leap reads as its
/// in-range shape, never a refusal — the player can edit from there).
/// `None` when fewer than [`LIFT_MIN_NOTES`] clear notes were heard.
/// Returns the cell plus the first note's MIDI (the cell's home root).
pub fn lift_cell_from_pitch_track(pitches: &[f64], min_run: usize) -> Option<(Vec<i8>, u8)> {
    let collapsed = played_notes_from_pitch_track(pitches, min_run);
    // Merge re-articulated repeats (tuned on real data): a note re-struck at
    // the same pitch adds rhythm, not melody — an RV cell is a pitch pattern.
    let mut notes: Vec<PlayedNote> = Vec::with_capacity(collapsed.len());
    let mut last_midi: Option<u8> = None;
    for n in collapsed {
        let midi = hz_to_midi(n.hz);
        if midi.is_some() && midi == last_midi {
            continue;
        }
        last_midi = midi;
        notes.push(n);
    }
    if notes.len() < LIFT_MIN_NOTES {
        return None;
    }
    let tail = &notes[notes.len().saturating_sub(LIFT_MAX_NOTES)..];
    let first = hz_to_midi(tail[0].hz)?;
    let offsets: Vec<i8> = tail
        .iter()
        .filter_map(|n| hz_to_midi(n.hz))
        .map(|m| {
            let mut off = i16::from(m) - i16::from(first);
            while off > 36 {
                off -= 12;
            }
            while off < -36 {
                off += 12;
            }
            off as i8
        })
        .collect();
    if offsets.len() < LIFT_MIN_NOTES {
        return None;
    }
    // A single pitch repeated is not a melodic cell — nothing to row.
    if offsets
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        < 2
    {
        return None;
    }
    Some((offsets, first))
}

fn hz_to_midi(hz: f64) -> Option<u8> {
    if hz <= 0.0 {
        return None;
    }
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
    if (0.0..=127.0).contains(&midi) {
        Some(midi as u8)
    } else {
        None
    }
}

/// Start an exploration from a LIFTED cell (#285): the player's own phrase,
/// rowed through the keys at their difficulty. The cell plays exactly as
/// played (no enclosure/direction modifiers until they stack chips).
/// A lifted lick always rows through at least this many keys — the method IS
/// the transposition ("transpose it to 12 keys"); one key would demo nothing,
/// and a fresh learner starts at difficulty 0 = 1 root.
pub const LIFT_MIN_ROOTS: usize = 3;

pub fn start_explore_cell(
    cell: Vec<i8>,
    tonic: u8,
    model: &LearnerModel,
    seed: u64,
) -> (ExploreState, GeneratedSequence) {
    let (mut state, _) = start_explore(tonic, "major", model, seed);
    state.spec.cell = Some(cell);
    state.spec.enclosure = None;
    state.spec.direction = DirectionMode::Forward;
    if state.spec.roots.len() < LIFT_MIN_ROOTS {
        state.spec.roots = roots_for(tonic, LIFT_MIN_ROOTS);
        state.spec.randomize_roots = true; // the RV shuffle, from the start
    }
    let sequence = generate(&state.spec, state.seed);
    (state, sequence)
}

// ---------------------------------------------------------------------------
// Cell editing (#292 slice 3): the player edits the CELL — one gesture fixes
// the note in every key, because the row re-derives from the edited cell.
// The frontend sends semantic gestures only; all pitch math lives here.
// ---------------------------------------------------------------------------

/// One semantic edit gesture on a note of the current rep. The frontend
/// constructs nothing but these; every resulting pitch is computed here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoteEdit {
    /// Vertical drag: move by diatonic staff positions (Rust picks the pitch
    /// on that line/space, preferring the in-key spelling).
    StaffSteps {
        by: i8,
    },
    /// Chromatic nudge — together with the drag this reaches ANY of the 12
    /// notes (founder: 12 notes × 3 octaves).
    Semitones {
        by: i8,
    },
    Octaves {
        by: i8,
    },
    Remove,
}

/// The founder's edit range: a note may move at most ±3 octaves from its root.
const MAX_CELL_OFFSET: i16 = 36;
/// Undo depth — enough for a whole editing session, bounded for the blob.
const MAX_HISTORY: usize = 20;

/// How many notes each root's segment contributes (figures are uniform-length
/// across roots by construction). `None` if the shape is unexpectedly ragged.
fn segment_len(seq: &GeneratedSequence, roots: usize) -> Option<usize> {
    if roots == 0 || !seq.target_midi.len().is_multiple_of(roots) {
        return None;
    }
    Some(seq.target_midi.len() / roots)
}

/// Move `midi` by `by` diatonic staff positions under `key`, preferring the
/// pitch the key signature already covers (no accidental), then the nearest.
/// `Err` when the target line/space is out of reach — the caller must NOT
/// mutate anything on a refused gesture.
fn midi_at_staff_steps(midi: u8, by: i8, key: &crate::score::KeySignature) -> Result<u8, String> {
    let step_of = |m: u8| crate::score::cellstaff::staff_step(m, key);
    let target = step_of(midi) + i16::from(by);
    let mut best: Option<(u8, bool, u8)> = None; // (midi, has_accidental, distance)
                                                 // ±40 semitones comfortably covers the founder's ±3-octave range
                                                 // (±21 staff steps ≈ ±36 semitones) in one drag.
    for m in midi.saturating_sub(40)..=midi.saturating_add(40).min(127) {
        if step_of(m) != target {
            continue;
        }
        let has_acc = crate::score::cellstaff::accidental_for(m, key).is_some();
        let dist = m.abs_diff(midi);
        let better = match best {
            None => true,
            Some((_, best_acc, best_dist)) => {
                (!has_acc && best_acc) || (has_acc == best_acc && dist < best_dist)
            }
        };
        if better {
            best = Some((m, has_acc, dist));
        }
    }
    best.map(|(m, _, _)| m)
        .ok_or_else(|| "that's further than a note can move".to_owned())
}

/// Apply one edit gesture to note `index` of the current rep and regenerate.
/// The edited segment's realized figure is BAKED into `spec.cell` (this is
/// the moment "you edit the cell, not a note" becomes literal), direction and
/// enclosure fold into the baked shape, and the seed is kept so the row order
/// does NOT reshuffle under the player's hands. Errors are calm strings.
pub fn edit_explore_note(
    state: &ExploreState,
    index: usize,
    edit: &NoteEdit,
    key: &crate::score::KeySignature,
) -> Result<(ExploreState, GeneratedSequence), String> {
    let seq = generate(&state.spec, state.seed);
    if index >= seq.target_midi.len() {
        return Err("that note is no longer on the staff — try again".to_owned());
    }
    let seg_len = segment_len(&seq, seq.root_order.len())
        .ok_or_else(|| "this variation can't be edited yet".to_owned())?;
    let seg = index / seg_len;
    let root = i16::from(seq.root_order[seg]);
    let pos = index - seg * seg_len;
    // Once a cell exists (and no un-baked modifiers sit on top), edit the CELL
    // itself — never round-trip through the generated output, whose per-
    // segment octave fold would corrupt other segments' offsets (review M2).
    // First edit: bake from the visible segment (fold included — that's what
    // the player is pointing at).
    let direct = state.spec.cell.as_ref().is_some_and(|c| !c.is_empty())
        && state.spec.enclosure.is_none()
        && state.spec.direction == DirectionMode::Forward;
    let mut offsets: Vec<i8> = if direct {
        state.spec.cell.clone().expect("checked above")
    } else {
        seq.target_midi[seg * seg_len..(seg + 1) * seg_len]
            .iter()
            .map(|&m| (i16::from(m) - root).clamp(-MAX_CELL_OFFSET, MAX_CELL_OFFSET) as i8)
            .collect()
    };
    if pos >= offsets.len() {
        return Err("that note is no longer on the staff — try again".to_owned());
    }
    match edit {
        NoteEdit::Remove => {
            if offsets.len() <= 1 {
                return Err("a cell needs at least one note".to_owned());
            }
            offsets.remove(pos);
        }
        _ => {
            // The gesture applies to the UNFOLDED pitch when editing the cell
            // directly; on first bake it applies to the visible note.
            let current = (root + i16::from(offsets[pos])).clamp(0, 127) as u8;
            let new_midi = match edit {
                NoteEdit::StaffSteps { by } => midi_at_staff_steps(current, *by, key)?,
                NoteEdit::Semitones { by } => {
                    (i16::from(current) + i16::from(*by)).clamp(0, 127) as u8
                }
                NoteEdit::Octaves { by } => {
                    (i16::from(current) + 12 * i16::from(*by)).clamp(0, 127) as u8
                }
                NoteEdit::Remove => unreachable!(),
            };
            let off = i16::from(new_midi) - root;
            // Refuse past the founder's ±3-octave range rather than landing
            // on a pitch the gesture never asked for (review nice-to-have).
            if !(-MAX_CELL_OFFSET..=MAX_CELL_OFFSET).contains(&off) {
                return Err("that's as far as this note can go".to_owned());
            }
            offsets[pos] = off as i8;
        }
    }
    let mut next = state.clone();
    push_history(&mut next, state);
    // Bake: the cell IS the figure now; direction/enclosure are folded in.
    next.spec.cell = Some(offsets);
    next.spec.direction = DirectionMode::Forward;
    next.spec.enclosure = None;
    let seq = generate(&next.spec, next.seed);
    Ok((next, seq))
}

/// Undo the most recent step — edit OR chip — restoring the exact rep the
/// player saw (spec, seed, and difficulty together). Calm error when there is
/// nothing to undo.
pub fn undo_explore_edit(
    state: &ExploreState,
) -> Result<(ExploreState, GeneratedSequence), String> {
    let mut next = state.clone();
    let snap = next
        .history
        .pop()
        .ok_or_else(|| "nothing to undo".to_owned())?;
    next.spec = snap.spec;
    next.seed = snap.seed;
    next.difficulty = snap.difficulty;
    let seq = generate(&next.spec, next.seed);
    Ok((next, seq))
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

    /// The ladder is monotonic where it must be: tempo strictly rises with
    /// every step, roots never decrease and grow overall — a constant table
    /// (a "difficulty" that changes nothing) fails this.
    #[test]
    fn ladder_is_monotonic_in_roots_and_tempo() {
        for d in 0..MAX_DIFFICULTY {
            assert!(ROOTS_BY_DIFFICULTY[d as usize] <= ROOTS_BY_DIFFICULTY[d as usize + 1]);
            assert!(
                tempo_for(d) + 1e-9 < tempo_for(d + 1),
                "tempo must STRICTLY increase at step {d}"
            );
        }
        assert!(
            ROOTS_BY_DIFFICULTY[0] < ROOTS_BY_DIFFICULTY[MAX_DIFFICULTY as usize],
            "roots must grow across the ladder overall"
        );
    }

    /// #254 AC6: a difficulty change alters the generated content — a
    /// down-ramp after a failed drill produces a spec with a slower tempo and
    /// no more roots than the harder step would have used. Fails if the ramp
    /// stops feeding the ladder.
    #[test]
    fn ramp_changes_generated_content() {
        let spec = LessonSpec {
            seed: 8,
            drill_count: 4,
            start_difficulty: 3,
        };
        let model = LearnerModel::default();
        let d0 = build_first(&spec, &model);
        assert_eq!(d0.difficulty, 3);
        // Bomb the drill → next is one step easier with easier knobs.
        let zero = score_drill(&d0.sequence.target_midi, &[], 0);
        let d1 = advance(&d0, &zero, &spec).unwrap();
        assert_eq!(d1.difficulty, 2);
        assert!(
            d1.spec.rhythm.tempo_bpm < d0.spec.rhythm.tempo_bpm,
            "easier drill must be slower"
        );
        assert!(d1.spec.roots.len() <= d0.spec.roots.len());
    }

    /// drill_count is clamped to 3..=4 — degenerate requests still produce a
    /// complete, terminating routine.
    #[test]
    fn drill_count_is_clamped() {
        let model = LearnerModel::default();
        for (requested, expected_drills) in [(0u8, 3usize), (1, 3), (9, 4)] {
            let spec = LessonSpec {
                seed: 1,
                drill_count: requested,
                start_difficulty: 0,
            };
            let mut n = 1;
            let mut drill = build_first(&spec, &model);
            while let Some(next) = advance(&drill, &perfect_score(&drill), &spec) {
                drill = next;
                n += 1;
                assert!(n <= 4, "must terminate");
            }
            assert_eq!(n, expected_drills, "requested {requested}");
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
        // Extras are skipped by the alignment (recall stays perfect) but past
        // the 1.5× slack they start costing: 5 played vs 3 asked → ×0.9.
        assert_eq!(s3.pitch_accuracy, 1.0, "alignment skips extras for recall");
        assert!(
            (s3.accuracy - 0.9).abs() < 1e-6,
            "extras beyond slack cost, got {}",
            s3.accuracy
        );
    }

    /// Anti-gaming: a slow chromatic walk contains any target as a
    /// subsequence — recall alone would grade it 100%. The extras penalty
    /// collapses it, while a genuine take with a couple of stray notes is
    /// untouched. Fails if the penalty is dropped (noodling grades ~1.0).
    #[test]
    fn chromatic_noodling_cannot_score_high() {
        let target = [60u8, 64, 67, 72];
        // 36 notes of chromatic wandering that necessarily embed the target.
        let noodle: Vec<PlayedNote> = (48u8..84)
            .map(|m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        let s = score_drill(&target, &noodle, noodle.len());
        assert!(
            s.accuracy < 0.5,
            "a 36-note noodle against a 4-note target must collapse, got {}",
            s.accuracy
        );
        assert!(s.pitch_accuracy > 0.9, "recall itself may stay high");

        // A genuine take with two stray notes keeps its full grade (within
        // the slack allowance).
        let honest: Vec<PlayedNote> = [60u8, 61, 64, 67, 66, 72]
            .iter()
            .map(|&m| PlayedNote { hz: midi_to_hz(m) })
            .collect();
        let s2 = score_drill(&target, &honest, honest.len());
        assert_eq!(s2.accuracy, 1.0, "a few flubs within slack cost nothing");
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
            cell: None,
            degrees: None,
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
        let model = sequence_to_score_model(&seq, "Warmup", key_signature_for(0, "major"));
        assert_eq!(model.tempo_bpm, 90.0);
        assert_eq!(model.key_signature.fifths, 0);
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

        // RV grid rule, pinned where lessons actually consume it: each
        // root's figure sits at the TOP of its own measure — the C figure
        // opens measure 1, the D figure opens measure 2 (8 notes × 0.5
        // beats fill a bar exactly). Fails if generator spacing drifts off
        // barlines or the adapter re-flows figures across measures.
        assert!(model.measures.len() >= 2, "two figures → two measures");
        let first_sounding = |mi: usize| {
            model.measures[mi]
                .notes
                .iter()
                .find(|n| !n.is_rest)
                .expect("measure has notes")
        };
        let c_open = first_sounding(0);
        assert_eq!((c_open.midi_number, c_open.start_beat), (60, 0.0));
        let d_open = first_sounding(1);
        assert_eq!((d_open.midi_number, d_open.start_beat), (62, 0.0));
    }

    /// #335 — the VA's C#-major lesson drew a FLAT signature under a header
    /// saying "C#". The display name must follow the SIGNATURE's spelling:
    /// a sharp name may never sit over a flat signature or vice versa, for
    /// every tonic × mode family the coach actually deals.
    #[test]
    fn tonic_names_agree_with_their_key_signatures() {
        for label in ["major", "minor", "dorian", "mixolydian", "lydian"] {
            for pc in 0u8..12 {
                let fifths = key_signature_for(pc, label).fifths;
                let name = tonic_display_name(pc, fifths);
                assert!(
                    !(name.contains('#') && fifths < 0),
                    "{name} ({label}) over a {fifths}-fifths (flat) signature"
                );
                assert!(
                    !(name.contains('b') && fifths >= 0),
                    "{name} ({label}) over a {fifths}-fifths (sharp) signature"
                );
            }
        }
        // The exact #324/#335 report: pc 1 major engraves flat → names "Db".
        assert_eq!(
            tonic_display_name(1, key_signature_for(1, "major").fifths),
            "Db"
        );
        // pc 1 dorian wraps to a SHARP signature (+5) → names "C#".
        assert_eq!(
            tonic_display_name(1, key_signature_for(1, "dorian").fifths),
            "C#"
        );
        assert_eq!(
            tonic_display_name(6, key_signature_for(6, "major").fifths),
            "F#"
        );
    }

    /// #277 must-fix: the key-signature mapping over the REAL drill-label
    /// space. Any wrong LUT entry, mode offset, family flag, or the
    /// mixolydian-before-lydian ordering trap fails here.
    #[test]
    fn key_signature_for_maps_the_real_label_space() {
        let ks = |t: u8, l: &str| {
            let k = key_signature_for(t, l);
            (k.fifths, k.mode)
        };
        use crate::score::KeyMode::{Major, Minor};
        assert_eq!(ks(0, "major"), (0, Major));
        assert_eq!(ks(7, "major"), (1, Major)); // G
        assert_eq!(ks(10, "major"), (-2, Major)); // Bb — the founder's case
        assert_eq!(ks(0, "mixolydian"), (-1, Major)); // ordering trap vs lydian
        assert_eq!(ks(0, "lydian"), (1, Major));
        assert_eq!(ks(0, "dorian"), (-2, Minor));
        assert_eq!(ks(0, "harmonic minor"), (-3, Minor));
        assert_eq!(ks(0, "melodic minor"), (-3, Minor));
        assert_eq!(ks(0, "minor pentatonic"), (-3, Minor));
        assert_eq!(ks(0, "blues"), (-3, Minor));
        assert_eq!(ks(0, "minor triad"), (-3, Minor));
        // Dominant material engraves the b7 (mixolydian), not tonic major —
        // C7's Bb must not spell as A# (#277 must-fix 1).
        assert_eq!(ks(0, "dominant 7"), (-1, Major));
        assert_eq!(ks(0, "half-diminished 7"), (-5, Minor));
        // Chord/interval material with no mode word → tonic major.
        assert_eq!(ks(0, "major triad"), (0, Major));
        assert_eq!(ks(0, "interval 7"), (0, Major));
    }

    /// #277 must-fix 2: out-of-range signatures wrap ENHARMONICALLY, never
    /// clamp to a wrong neighbor. Db-minor material (raw -8) is C# minor (+4);
    /// Db Dorian (raw -7) prefers C# Dorian (+5, 5 sharps) over 7 flats.
    #[test]
    fn key_signature_wraps_enharmonically_instead_of_clamping() {
        assert_eq!(key_signature_for(1, "minor triad").fifths, 4); // C# minor
        assert_eq!(key_signature_for(1, "dorian").fifths, 5); // C# dorian
        assert_eq!(key_signature_for(1, "phrygian").fifths, 3);
        // Everything stays in the friendly window.
        for tonic in 0..12u8 {
            for label in [
                "major",
                "dorian",
                "phrygian",
                "locrian",
                "minor 7",
                "dominant 7",
            ] {
                let f = key_signature_for(tonic, label).fifths;
                assert!((-6..=6).contains(&f), "{tonic} {label} -> {f}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // #285 — phrase seeding
    // -----------------------------------------------------------------------

    fn track_of(midis: &[u8]) -> Vec<f64> {
        midis
            .iter()
            .flat_map(|&m| {
                let hz = 440.0 * 2f64.powf((f64::from(m) - 69.0) / 12.0);
                std::iter::repeat_n(hz, 5)
            })
            .collect()
    }

    /// #285 AC (the lift): a clearly-played lick becomes offsets from its
    /// first note; too few clear notes lift nothing.
    #[test]
    fn lifting_extracts_the_played_shape() {
        let (cell, first) = lift_cell_from_pitch_track(&track_of(&[62, 65, 64, 69]), 3).unwrap();
        assert_eq!(cell, vec![0, 3, 2, 7], "offsets from the first note");
        assert_eq!(first, 62, "the lick's home root rides along");
        assert!(
            lift_cell_from_pitch_track(&track_of(&[60, 62, 64]), 3).is_none(),
            "3 notes is not a lick worth rowing"
        );
        assert!(lift_cell_from_pitch_track(&[], 3).is_none());
        // Tuned on real data: re-struck repeats merge (rhythm, not melody)…
        let (dedup, _) =
            lift_cell_from_pitch_track(&track_of(&[62, 62, 65, 65, 64, 69]), 3).unwrap();
        assert_eq!(dedup, vec![0, 3, 2, 7], "consecutive repeats merge");
        // …and a monotone drone is not a melodic cell at all.
        assert!(
            lift_cell_from_pitch_track(&track_of(&[60, 60, 60, 60, 60, 60]), 3).is_none(),
            "one pitch repeated has nothing to row"
        );
    }

    /// #285 AC (founder cap): a long take keeps its most recent 17 notes; a
    /// wild leap folds back into the ±3-octave range instead of refusing.
    #[test]
    fn lifting_caps_at_17_and_folds_wild_leaps() {
        let long: Vec<u8> = (0..25).map(|i| 55 + (i % 12) as u8).collect();
        let (cell, _) = lift_cell_from_pitch_track(&track_of(&long), 3).unwrap();
        assert_eq!(cell.len(), LIFT_MAX_NOTES);
        let (leap, _) = lift_cell_from_pitch_track(&track_of(&[40, 42, 44, 90]), 3).unwrap();
        assert_eq!(leap[3], 38 % 12 + 24, "50 semitones folds to 26 (in range)");
        assert!(leap.iter().all(|&o| (-36..=36).contains(&o)));
    }

    /// #285 AC (the row): the lifted cell rows through EVERY key of the
    /// exploration — each root's segment carries the exact played shape.
    #[test]
    fn a_lifted_cell_rows_through_the_keys() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let cell = vec![0i8, 3, 2, 7];
        let (state, seq) = start_explore_cell(cell.clone(), 2, &model, 11);
        assert_eq!(state.spec.cell.as_ref(), Some(&cell));
        let roots = seq.root_order.len();
        assert!(roots >= 2);
        let seg_len = seq.target_midi.len() / roots;
        assert_eq!(seg_len, cell.len(), "the cell IS the figure, unmodified");
        for seg in 0..roots {
            let root = i16::from(seq.root_order[seg]);
            let shape: Vec<i16> = seq.target_midi[seg * seg_len..(seg + 1) * seg_len]
                .iter()
                .map(|&m| i16::from(m) - root)
                .collect();
            let expected: Vec<i16> = cell.iter().map(|&o| i16::from(o)).collect();
            assert_eq!(shape, expected, "segment {seg} plays the player's lick");
        }
        assert!(seq.label.contains("4-note cell"), "got {}", seq.label);
    }

    // -----------------------------------------------------------------------
    // #289 — the pattern database
    // -----------------------------------------------------------------------

    /// #289: the pattern chip is reachable (slot 3 alternates by seed parity
    /// with the scale chip), and applying it pulls a database pattern that
    /// ties to the active scale — never the one already playing.
    #[test]
    fn pattern_chip_alternates_and_applies_from_the_database() {
        let model = LearnerModel::default();
        let (mut state, _) = start_explore(0, "major", &model, 2); // even seed
        let labels = |st: &ExploreState| -> Vec<String> {
            suggest_chips(st, &model)
                .iter()
                .map(|c| c.label.clone())
                .collect()
        };
        assert!(labels(&state).iter().any(|l| l.contains("Different scale")));
        state.seed = 3; // odd
        assert!(
            labels(&state).iter().any(|l| l.contains("Try a pattern")),
            "odd seeds offer the pattern chip: {:?}",
            labels(&state)
        );

        let (with_pat, seq) = apply_explore_delta(&state, &VariationDelta::TryPattern);
        let degrees = with_pat.spec.degrees.clone().expect("a pattern landed");
        assert!(
            DEGREE_PATTERNS.iter().any(|(_, d)| d.to_vec() == degrees),
            "the pick comes from the database"
        );
        assert!(seq.label.contains("pattern"), "got {}", seq.label);
        // Again: never the same pattern twice in a row.
        let (with_pat2, _) = apply_explore_delta(&with_pat, &VariationDelta::TryPattern);
        assert_ne!(with_pat2.spec.degrees, with_pat.spec.degrees);
    }

    /// #289 + #292: a pattern is "fresh material" vs a hand-edited cell (the
    /// cell is discarded, undo recovers it), while [Different scale] KEEPS
    /// the pattern — degrees ride any scale, that's the whole point.
    #[test]
    fn patterns_respect_cells_and_survive_scale_swaps() {
        let model = LearnerModel::default();
        let (state, _) = start_explore(0, "major", &model, 3);
        let (edited, _) =
            edit_explore_note(&state, 0, &NoteEdit::Octaves { by: 1 }, &c_major_key()).unwrap();
        let cell = edited.spec.cell.clone().unwrap();
        let (with_pat, _) = apply_explore_delta(&edited, &VariationDelta::TryPattern);
        assert!(with_pat.spec.cell.is_none(), "pattern = fresh material");
        let (back, _) = undo_explore_edit(&with_pat).unwrap();
        assert_eq!(back.spec.cell, Some(cell), "undo recovers the cell");

        let (swapped, _) = apply_explore_delta(&with_pat, &VariationDelta::DifferentScale);
        assert_eq!(
            swapped.spec.degrees, with_pat.spec.degrees,
            "a scale swap keeps the degree pattern — it re-colors it"
        );
    }

    /// Review must-fix regression: [Make it spicy] preserves a degree
    /// PATTERN exactly like a hand-edited cell — the player's material never
    /// silently vanishes on a difficulty chip.
    #[test]
    fn degrees_survive_a_difficulty_bump() {
        let model = LearnerModel::default();
        let (state, _) = start_explore(0, "major", &model, 3); // odd: pattern chip
        let (with_pat, _) = apply_explore_delta(&state, &VariationDelta::TryPattern);
        let degrees = with_pat.spec.degrees.clone().expect("pattern landed");
        let (harder, seq) =
            apply_explore_delta(&with_pat, &VariationDelta::BumpDifficulty { by: 1 });
        assert_eq!(
            harder.spec.degrees,
            Some(degrees),
            "a difficulty chip must never destroy the pattern"
        );
        assert!(seq.label.contains("pattern"), "got {}", seq.label);
    }

    /// Review must-fix regression (the flagship demo): a lifted lick rows
    /// through SEVERAL keys even for a brand-new learner (difficulty 0 = one
    /// root), because the method IS the transposition.
    #[test]
    fn a_fresh_learners_lick_still_rows_through_keys() {
        let model = LearnerModel::default(); // difficulty 0
        let (state, seq) = start_explore_cell(vec![0, 3, 2, 7], 2, &model, 11);
        assert!(
            seq.root_order.len() >= LIFT_MIN_ROOTS,
            "got {} roots",
            seq.root_order.len()
        );
        // And the reshuffle chip is on offer (roots > 1).
        assert!(suggest_chips(&state, &model)
            .iter()
            .any(|c| c.delta == VariationDelta::ReshuffleRoots));
    }

    /// Mutation M1: a one-root row must NOT offer [New keys] — the chip would
    /// do nothing.
    #[test]
    fn one_root_rows_offer_no_reshuffle_chip() {
        let model = LearnerModel::default(); // difficulty 0 → 1 root
        let (state, seq) = start_explore(0, "major", &model, 2);
        assert_eq!(seq.root_order.len(), 1, "difficulty 0 is a one-root row");
        assert!(
            !suggest_chips(&state, &model)
                .iter()
                .any(|c| c.delta == VariationDelta::ReshuffleRoots),
            "no reshuffle chip on a single root"
        );
    }

    // -----------------------------------------------------------------------
    // #292 slice 3 — cell editing
    // -----------------------------------------------------------------------

    fn c_major_key() -> crate::score::KeySignature {
        key_signature_for(0, "major")
    }

    /// #292 AC (the superpower): editing ONE note bakes the cell, and the fix
    /// appears in EVERY root's segment — same relative change everywhere.
    /// Fails if editing degrades to a single-note patch.
    #[test]
    fn editing_one_note_fixes_every_key() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, before) = start_explore(0, "major", &model, 9);
        let roots = before.root_order.len();
        assert!(roots >= 2, "need a real row for this test");
        let seg_len = before.target_midi.len() / roots;

        // Raise the SECOND note of the first segment an octave.
        let (next, after) =
            edit_explore_note(&state, 1, &NoteEdit::Octaves { by: 1 }, &c_major_key()).unwrap();
        assert!(next.spec.cell.is_some(), "the edit bakes a cell");
        for seg in 0..roots {
            let root_before = i16::from(before.root_order[seg]);
            let root_after = i16::from(after.root_order[seg]);
            let b = i16::from(before.target_midi[seg * seg_len + 1]) - root_before;
            let a = i16::from(after.target_midi[seg * seg_len + 1]) - root_after;
            assert_eq!(a, b + 12, "segment {seg} must carry the same fix");
        }
        // The row did NOT reshuffle under the player's hands.
        assert_eq!(after.root_order, before.root_order);
    }

    /// Staff-step drags land on the line/space the player dropped the note on,
    /// preferring the in-key pitch: from E up two staff steps in C major = G;
    /// a semitone nudge then reaches the chromatic note (G#) — together the
    /// drag + nudge reach any of the 12 notes (founder's edit range).
    #[test]
    fn drags_are_diatonic_and_nudges_are_chromatic() {
        let model = LearnerModel::default(); // difficulty 0: scale run from C
        let (state, seq) = start_explore(0, "major", &model, 9);
        // Note 2 of a C-major run is E (64).
        assert_eq!(seq.target_midi[2] % 12, 4);
        let (st2, after) =
            edit_explore_note(&state, 2, &NoteEdit::StaffSteps { by: 2 }, &c_major_key()).unwrap();
        assert_eq!(
            after.target_midi[2] % 12,
            7,
            "E dragged +2 steps lands on G"
        );
        let (_, after2) =
            edit_explore_note(&st2, 2, &NoteEdit::Semitones { by: 1 }, &c_major_key()).unwrap();
        assert_eq!(after2.target_midi[2] % 12, 8, "then a nudge reaches G#");
    }

    /// Remove deletes the note from the cell everywhere; the last note can't
    /// be removed; out-of-range indices err calmly.
    #[test]
    fn remove_and_guards() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, before) = start_explore(0, "major", &model, 9);
        let roots = before.root_order.len();
        let (next, after) =
            edit_explore_note(&state, 0, &NoteEdit::Remove, &c_major_key()).unwrap();
        assert_eq!(after.target_midi.len(), before.target_midi.len() - roots);
        assert!(
            edit_explore_note(&next, 10_000, &NoteEdit::Remove, &c_major_key()).is_err(),
            "stale index errs calmly"
        );
        // Shrink to a single-note cell, then removal must refuse.
        let mut one = next.clone();
        one.spec.cell = Some(vec![0]);
        assert!(edit_explore_note(&one, 0, &NoteEdit::Remove, &c_major_key()).is_err());
    }

    /// Review M1/B/E regressions: a full 3-octave drag WORKS (the founder's
    /// range), a further one refuses WITHOUT mutating state, and octave
    /// gestures refuse at the ±36 boundary instead of landing on a pitch the
    /// gesture never asked for.
    #[test]
    fn founder_range_reachable_and_boundaries_refuse() {
        let model = LearnerModel::default();
        let (state, seq) = start_explore(0, "major", &model, 9);
        let start = seq.target_midi[0];
        // 3 octaves up = 21 staff steps: reachable in ONE drag.
        let (st, after) =
            edit_explore_note(&state, 0, &NoteEdit::StaffSteps { by: 21 }, &c_major_key()).unwrap();
        assert_eq!(
            i16::from(after.target_midi[0]),
            i16::from(start) + 36,
            "21 staff steps in C major = exactly 3 octaves"
        );
        // Further than the range: calm refusal, NOTHING mutated.
        let before = st.clone();
        assert!(
            edit_explore_note(&st, 0, &NoteEdit::Octaves { by: 1 }, &c_major_key()).is_err(),
            "past +36 from the root must refuse"
        );
        assert_eq!(st, before, "a refused gesture must not mutate state");
        assert!(
            edit_explore_note(&state, 0, &NoteEdit::StaffSteps { by: 50 }, &c_major_key()).is_err(),
            "an absurd drag refuses instead of silently no-opping"
        );
    }

    /// Review M2 regression: editing an already-edited cell operates on the
    /// CELL, not the octave-folded output — a net-zero pair of nudges leaves
    /// the whole row EXACTLY as it was, even with a segment folded at the
    /// register ceiling.
    #[test]
    fn net_zero_nudges_never_corrupt_the_row() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, _) = start_explore(0, "major", &model, 9);
        // Push the last note high so high roots' segments fold.
        let (st1, seq1) =
            edit_explore_note(&state, 5, &NoteEdit::Octaves { by: 2 }, &c_major_key()).unwrap();
        // Net-zero: sharp then flat on a note of a LATER (possibly folded) segment.
        let idx = seq1.target_midi.len() - 2;
        let (st2, _) =
            edit_explore_note(&st1, idx, &NoteEdit::Semitones { by: 1 }, &c_major_key()).unwrap();
        let (st3, seq3) =
            edit_explore_note(&st2, idx, &NoteEdit::Semitones { by: -1 }, &c_major_key()).unwrap();
        assert_eq!(
            seq3.target_midi, seq1.target_midi,
            "sharp-then-flat must be a perfect no-op on every segment"
        );
        assert_eq!(st3.spec.cell, st1.spec.cell);
    }

    /// Review M3/M4 regression: chips are cell-aware — a difficulty bump
    /// PRESERVES the player's hand-edited cell (knobs still ramp), and
    /// "different scale" explicitly discards it (fresh material) with undo
    /// able to bring it back.
    #[test]
    fn chips_respect_the_edited_cell() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, _) = start_explore(0, "major", &model, 9);
        let (edited, _) =
            edit_explore_note(&state, 1, &NoteEdit::Octaves { by: 1 }, &c_major_key()).unwrap();
        let cell = edited.spec.cell.clone().expect("edit bakes a cell");

        let (harder, _) = apply_explore_delta(&edited, &VariationDelta::BumpDifficulty { by: 1 });
        assert_eq!(
            harder.spec.cell.as_ref(),
            Some(&cell),
            "a difficulty chip must never destroy the player's material"
        );
        assert!(harder.spec.rhythm.tempo_bpm > edited.spec.rhythm.tempo_bpm);

        let (fresh, fresh_seq) = apply_explore_delta(&edited, &VariationDelta::DifferentScale);
        assert!(
            fresh.spec.cell.is_none(),
            "different scale = fresh material"
        );
        assert!(!fresh_seq.label.contains("cell"), "the label reflects it");
        let (back, _) = undo_explore_edit(&fresh).unwrap();
        assert_eq!(back.spec.cell, Some(cell), "undo recovers the cell");
    }

    /// Review M5 regression: undo is a universal back-one-step — after
    /// [edit, chip], one undo restores the EXACT rep the player saw after the
    /// edit (spec AND seed AND difficulty), never a third stitched state.
    #[test]
    fn undo_steps_back_through_chips_exactly() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, _) = start_explore(0, "major", &model, 9);
        let (edited, seen_after_edit) =
            edit_explore_note(&state, 0, &NoteEdit::Octaves { by: 1 }, &c_major_key()).unwrap();
        let (chipped, _) = apply_explore_delta(&edited, &VariationDelta::ReshuffleRoots);
        let (undone, rep) = undo_explore_edit(&chipped).unwrap();
        assert_eq!(rep, seen_after_edit, "exact rep, same seed and shuffle");
        assert_eq!(undone.seed, edited.seed);
        assert_eq!(undone.difficulty, edited.difficulty);
    }

    /// Review F: the history bound drops the OLDEST snapshot — after many
    /// edits, undo still walks back through the most recent ones.
    #[test]
    fn history_bound_keeps_the_newest() {
        let model = LearnerModel::default();
        let (mut state, _) = start_explore(0, "major", &model, 9);
        let mut reps = Vec::new();
        for i in 0..25 {
            let by = if i % 2 == 0 { 1 } else { -1 };
            let (next, rep) =
                edit_explore_note(&state, 0, &NoteEdit::Semitones { by }, &c_major_key()).unwrap();
            reps.push(rep);
            state = next;
        }
        assert_eq!(state.history.len(), 20, "bounded");
        let (after_undo, rep) = undo_explore_edit(&state).unwrap();
        assert_eq!(rep, reps[reps.len() - 2], "undo = the previous rep");
        assert_eq!(after_undo.history.len(), 19);
    }

    /// Review C (flat-key case): dragging onto the B line in F major picks
    /// Bb (the signature's pitch, no glyph) over B natural.
    #[test]
    fn flat_key_drags_prefer_the_signature_pitch() {
        let model = LearnerModel::default();
        let (state, seq) = start_explore(5, "major", &model, 9); // F major
                                                                 // First note is F (65); up 3 staff steps = the B line.
        assert_eq!(seq.target_midi[0] % 12, 5);
        let key = key_signature_for(5, "major");
        let (_, after) =
            edit_explore_note(&state, 0, &NoteEdit::StaffSteps { by: 3 }, &key).unwrap();
        assert_eq!(
            after.target_midi[0] % 12,
            10,
            "the B line in F major is Bb, not B natural"
        );
    }

    /// #292 AC: undo restores the EXACT prior rep (spec and seed), stacking
    /// through multiple edits; empty history errs calmly.
    #[test]
    fn undo_restores_the_exact_prior_rep() {
        let model = LearnerModel::default();
        let (state, original) = start_explore(0, "major", &model, 9);
        assert!(undo_explore_edit(&state).is_err(), "nothing to undo yet");
        let (st1, _) =
            edit_explore_note(&state, 0, &NoteEdit::Octaves { by: 1 }, &c_major_key()).unwrap();
        let (st2, _) =
            edit_explore_note(&st1, 1, &NoteEdit::Semitones { by: -1 }, &c_major_key()).unwrap();
        let (st3, undone_once) = undo_explore_edit(&st2).unwrap();
        assert_eq!(undone_once, generate(&st1.spec, st1.seed));
        let (_, undone_twice) = undo_explore_edit(&st3).unwrap();
        assert_eq!(undone_twice, original, "two undos = the untouched rep");
    }

    // -----------------------------------------------------------------------
    // #255 — free-play exploration
    // -----------------------------------------------------------------------

    /// #255: chips are ≤3, stable-ordered, and carry concrete deltas; the
    /// difficulty chip is gated — spicy below MAX, simpler at MAX or when
    /// struggling, nothing below 0. Fails if the gating rules regress.
    #[test]
    fn suggest_chips_gates_by_difficulty_and_struggle() {
        let model = LearnerModel::default(); // difficulty 0, no mastery
        let (state, _) = start_explore(7, "dorian", &model, 1);
        let chips = suggest_chips(&state, &model);
        assert!(chips.len() <= 3);
        assert!(
            chips.iter().any(|c| c.label.contains("spicy")),
            "fresh learner below MAX gets a spicy chip: {chips:?}"
        );
        assert!(
            !chips
                .iter()
                .any(|c| c.delta == VariationDelta::BumpDifficulty { by: -1 }),
            "difficulty 0 must not offer Simpler"
        );

        // At the top of the ladder: never a raise, offer simpler instead.
        let mut top = crate::learner::apply_difficulty(&model, MAX_DIFFICULTY, 1);
        top.difficulty = MAX_DIFFICULTY;
        let (state_top, _) = start_explore(7, "dorian", &top, 1);
        let chips_top = suggest_chips(&state_top, &top);
        assert!(
            !chips_top
                .iter()
                .any(|c| c.delta == VariationDelta::BumpDifficulty { by: 1 }),
            "MAX difficulty must not offer a raise: {chips_top:?}"
        );
        assert!(chips_top
            .iter()
            .any(|c| c.delta == VariationDelta::BumpDifficulty { by: -1 }));

        // Struggling on the key → Simpler replaces Spicy (needs difficulty > 0).
        let mut struggling = crate::learner::apply_difficulty(&model, 3, 1);
        for t in 0..3 {
            struggling = crate::learner::apply_drill_result(
                &struggling,
                &crate::learner::DrillResult {
                    tonic: 7,
                    mode: "major".to_owned(),
                    accuracy: 0.2,
                },
                t,
            );
        }
        let (state_s, _) = start_explore(7, "dorian", &struggling, 1);
        let chips_s = suggest_chips(&state_s, &struggling);
        assert!(
            chips_s
                .iter()
                .any(|c| c.delta == VariationDelta::BumpDifficulty { by: -1 }),
            "a struggling learner gets Simpler: {chips_s:?}"
        );
    }

    /// #255: start_explore seeds the variation from the LIVE key — a Dorian
    /// context explores Dorian, not the ladder default — and is deterministic.
    #[test]
    fn start_explore_uses_the_live_mode_and_is_deterministic() {
        let model = LearnerModel::default();
        let (state, seq) = start_explore(7, "Dorian", &model, 42);
        assert_eq!(state.spec.scale.unwrap().scale, ScaleType::Dorian);
        assert!(seq.label.contains("Dorian"), "got {}", seq.label);
        assert_eq!(start_explore(7, "Dorian", &model, 42).1, seq);
        // Unknown mode label falls back to the ladder scale, never panics.
        let (fallback, _) = start_explore(7, "super-locrian bebop", &model, 42);
        assert!(fallback.spec.scale.is_some());
    }

    /// #255: each delta changes exactly its mapped knobs.
    #[test]
    fn deltas_change_exactly_their_knobs() {
        let model = crate::learner::apply_difficulty(&LearnerModel::default(), 3, 1);
        let (state, first) = start_explore(0, "major", &model, 5);

        // Spicy: one step harder → faster tempo (the ladder is strict there).
        let (harder, _) = apply_explore_delta(&state, &VariationDelta::BumpDifficulty { by: 1 });
        assert_eq!(harder.difficulty, 4);
        assert!(harder.spec.rhythm.tempo_bpm > state.spec.rhythm.tempo_bpm);
        assert_eq!(
            harder.spec.scale.unwrap().scale,
            ScaleType::Major,
            "the explored scale survives a difficulty rebuild"
        );

        // Different scale: never the current one.
        let (swapped, _) = apply_explore_delta(&state, &VariationDelta::DifferentScale);
        assert_ne!(swapped.spec.scale.unwrap().scale, ScaleType::Major);

        // Toggle: forward <-> reversed.
        let (rev, _) = apply_explore_delta(&state, &VariationDelta::ToggleDirection);
        assert_eq!(rev.spec.direction, DirectionMode::Reversed);
        let (fwd, _) = apply_explore_delta(&rev, &VariationDelta::ToggleDirection);
        assert_eq!(fwd.spec.direction, DirectionMode::Forward);

        // Reshuffle: same material, fresh rep (seed advanced → new sequence).
        let (reshuffled, again) = apply_explore_delta(&state, &VariationDelta::ReshuffleRoots);
        assert!(reshuffled.spec.randomize_roots);
        assert_ne!(again, first, "a reshuffle must produce a fresh rep");
    }
}
