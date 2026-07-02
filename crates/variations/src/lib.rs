//! # Variations — the Random Variations practice-pattern generator (F1, #252)
//!
//! Kris's Random Variations methodology, as an engine: take a set of root
//! notes, expand each into a musical figure (scale run, arpeggio, interval,
//! optionally approached by a jazz enclosure), and **randomize the order and
//! direction** so muscle memory can't carry the player — they only succeed if
//! they truly own the material in every key.
//!
//! Design rules (the reasons this is a leaf crate):
//! - **Pure + seed-deterministic.** No I/O, no wall clock, no process
//!   randomness: an explicit `seed` drives an internal xorshift PRNG, so
//!   `generate(spec, seed)` always returns the same sequence — reproducible
//!   drills, trivially testable, replayable from a recap.
//! - **RV's signature shuffle keeps the first root fixed** and permutes the
//!   rest, exactly like the original app.
//! - Output is renderer-agnostic: plain notes on a beat grid plus the exact
//!   `target_midi` grading list. The desktop app adapts it to a `ScoreModel`
//!   (→ MusicXML → ScoreView/follower) in `brain::coach`; this crate knows
//!   nothing about scores or scoring.

pub mod catalog;

pub use catalog::{ChordType, Enclosure, ScaleType};

use serde::{Deserialize, Serialize};

/// Playable MIDI range the generator folds figures into (C2..=C7) — beyond
/// this, real students can't follow.
pub const MIDI_MIN: u8 = 36;
/// See [`MIDI_MIN`].
pub const MIDI_MAX: u8 = 96;

/// How a scale figure walks its degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalePattern {
    /// Root up to the octave.
    Up,
    /// Octave down to the root.
    Down,
    /// Up then back down (octave and root not repeated).
    UpDown,
    /// Down then back up.
    DownUp,
    /// Ascending in diatonic thirds (1-3, 2-4, 3-5, …).
    ThirdsUp,
}

impl ScalePattern {
    fn label(self) -> &'static str {
        match self {
            ScalePattern::Up => "up",
            ScalePattern::Down => "down",
            ScalePattern::UpDown => "up-down",
            ScalePattern::DownUp => "down-up",
            ScalePattern::ThirdsUp => "in thirds",
        }
    }
}

/// How an arpeggio walks its chord tones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArpeggioPattern {
    Ascending,
    Descending,
    UpDown,
}

impl ArpeggioPattern {
    fn label(self) -> &'static str {
        match self {
            ArpeggioPattern::Ascending => "ascending",
            ArpeggioPattern::Descending => "descending",
            ArpeggioPattern::UpDown => "up-down",
        }
    }
}

/// Scale expansion for each root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleModifier {
    pub scale: ScaleType,
    pub pattern: ScalePattern,
}

/// Arpeggio expansion for each root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChordModifier {
    pub chord: ChordType,
    pub pattern: ArpeggioPattern,
    /// 0 = root position; n rotates n chord tones up an octave.
    pub inversion: u8,
}

/// Broken-interval expansion: each root becomes (root, root ± interval).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntervalModifier {
    /// Interval size in semitones, 1..=12.
    pub semitones: u8,
    /// `true` = the second note is above the root; `false` = below.
    pub ascending: bool,
}

/// Per-root figure direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionMode {
    /// Figures play as built.
    Forward,
    /// Every figure is reversed.
    Reversed,
    /// A seeded coin flip per root — RV's "random directions".
    RandomPerRoot,
}

/// The beat grid the figure notes land on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RhythmSpec {
    /// Notes per beat (1 = quarters, 2 = eighths, 3 = triplets, 4 = sixteenths).
    pub notes_per_beat: u8,
    /// Tempo the drill is meant to be played at.
    pub tempo_bpm: f64,
    /// Silent beats inserted between one root's figure and the next (thinking
    /// time — RV's "rests").
    pub rest_beats_between_roots: f64,
}

impl Default for RhythmSpec {
    fn default() -> Self {
        Self {
            notes_per_beat: 2,
            tempo_bpm: 80.0,
            rest_beats_between_roots: 1.0,
        }
    }
}

/// A full variation request: roots × modifiers × randomization.
///
/// Exactly one of `scale` / `chord` / `interval` is normally set (the figure);
/// with none set the figure is the bare root. `enclosure` composes with any of
/// them, approaching each figure's first note chromatically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariationSpec {
    /// Root notes as MIDI numbers (e.g. the 12 chromatic roots from C4).
    pub roots: Vec<u8>,
    pub scale: Option<ScaleModifier>,
    pub chord: Option<ChordModifier>,
    pub interval: Option<IntervalModifier>,
    pub enclosure: Option<Enclosure>,
    pub direction: DirectionMode,
    pub rhythm: RhythmSpec,
    /// RV's signature: shuffle the root order, keeping the first root fixed.
    pub randomize_roots: bool,
}

/// One generated note on the beat grid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeneratedNote {
    pub midi: u8,
    pub start_beat: f64,
    pub duration_beats: f64,
}

/// The generated drill: playable notes, the exact grading target, and a human
/// label describing what was asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedSequence {
    pub notes: Vec<GeneratedNote>,
    /// The grading target: `notes` in order, as MIDI numbers.
    pub target_midi: Vec<u8>,
    /// e.g. `"G Dorian · up-down · 12 roots, random order · 80 BPM"`.
    pub label: String,
    pub tempo_bpm: f64,
    /// Fixed 4/4 grid for the ScoreModel adapter.
    pub beats_per_measure: u8,
}

/// Deterministic xorshift64* PRNG — the crate's only randomness source, seeded
/// explicitly so generation is reproducible (no `rand`, no wall clock).
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // A zero state would lock xorshift at zero forever.
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform value in `0..n` (n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

/// Generate a variation. Pure and deterministic: the same `(spec, seed)` always
/// produces the same sequence.
pub fn generate(spec: &VariationSpec, seed: u64) -> GeneratedSequence {
    let mut rng = Xorshift64::new(seed);

    // 1. Root order — RV's shuffle keeps the first root fixed and permutes the
    //    rest (seeded Fisher–Yates).
    let mut roots = spec.roots.clone();
    if spec.randomize_roots && roots.len() > 2 {
        for i in (2..roots.len()).rev() {
            let j = 1 + rng.below(i); // 1..=i — never index 0
            roots.swap(i, j);
        }
    }

    // 2. Per root: figure + direction + enclosure, folded into range.
    let mut notes: Vec<GeneratedNote> = Vec::new();
    let step = 1.0 / f64::from(spec.rhythm.notes_per_beat.max(1));
    let mut cursor_beat = 0.0_f64;

    for &root in &roots {
        let mut figure = figure_for(spec, root);

        let reversed = match spec.direction {
            DirectionMode::Forward => false,
            DirectionMode::Reversed => true,
            DirectionMode::RandomPerRoot => rng.coin(),
        };
        if reversed {
            figure.reverse();
        }

        // Enclosure approaches the figure's (post-direction) first note.
        if let (Some(enc), Some(&first)) = (spec.enclosure, figure.first()) {
            let mut with_enclosure: Vec<i16> = enc
                .approach_semitones()
                .iter()
                .map(|&s| first + i16::from(s))
                .collect();
            with_enclosure.extend_from_slice(&figure);
            figure = with_enclosure;
        }

        fold_into_range(&mut figure);

        for m in &figure {
            notes.push(GeneratedNote {
                midi: *m as u8,
                start_beat: cursor_beat,
                duration_beats: step,
            });
            cursor_beat += step;
        }
        cursor_beat += spec.rhythm.rest_beats_between_roots.max(0.0);
        // Keep each root's figure starting on a whole beat so the grid stays
        // readable when rendered.
        cursor_beat = cursor_beat.ceil();
    }

    let target_midi = notes.iter().map(|n| n.midi).collect();
    let label = label_for(spec, &roots);

    GeneratedSequence {
        notes,
        target_midi,
        label,
        tempo_bpm: spec.rhythm.tempo_bpm,
        beats_per_measure: 4,
    }
}

/// Expand one root into its figure (before direction/enclosure), as i16 so
/// intermediate math can't wrap.
fn figure_for(spec: &VariationSpec, root: u8) -> Vec<i16> {
    let root = i16::from(root);

    if let Some(s) = spec.scale {
        let degrees: Vec<i16> = s
            .scale
            .semitones()
            .iter()
            .map(|&d| root + i16::from(d))
            .chain(std::iter::once(root + 12)) // top octave completes the run
            .collect();
        return match s.pattern {
            ScalePattern::Up => degrees,
            ScalePattern::Down => {
                let mut d = degrees;
                d.reverse();
                d
            }
            ScalePattern::UpDown => {
                let mut d = degrees.clone();
                d.extend(degrees.iter().rev().skip(1)); // don't repeat the top
                d
            }
            ScalePattern::DownUp => {
                let mut d: Vec<i16> = degrees.iter().rev().copied().collect();
                d.extend(degrees.iter().skip(1)); // don't repeat the bottom
                d
            }
            ScalePattern::ThirdsUp => {
                // 1-3, 2-4, 3-5 … over the one-octave degree list.
                let base: Vec<i16> = degrees;
                let mut out = Vec::with_capacity(base.len() * 2);
                for i in 0..base.len().saturating_sub(2) {
                    out.push(base[i]);
                    out.push(base[i + 2]);
                }
                out
            }
        };
    }

    if let Some(c) = spec.chord {
        let mut tones: Vec<i16> = c
            .chord
            .semitones()
            .iter()
            .map(|&t| root + i16::from(t))
            .collect();
        // Inversion: rotate n tones up an octave.
        let n = usize::from(c.inversion) % tones.len().max(1);
        for tone in tones.iter_mut().take(n) {
            *tone += 12;
        }
        tones.rotate_left(n);
        return match c.pattern {
            ArpeggioPattern::Ascending => tones,
            ArpeggioPattern::Descending => {
                tones.reverse();
                tones
            }
            ArpeggioPattern::UpDown => {
                let mut t = tones.clone();
                t.extend(tones.iter().rev().skip(1));
                t
            }
        };
    }

    if let Some(iv) = spec.interval {
        let delta = i16::from(iv.semitones.clamp(1, 12));
        let second = if iv.ascending {
            root + delta
        } else {
            root - delta
        };
        return vec![root, second];
    }

    vec![root]
}

/// Shift a whole figure by octaves until it fits `MIDI_MIN..=MIDI_MAX` (whole-
/// figure shifts preserve the contour); per-note clamp as a last resort for
/// figures wider than the range.
fn fold_into_range(figure: &mut [i16]) {
    if figure.is_empty() {
        return;
    }
    let (min, max) = figure
        .iter()
        .fold((i16::MAX, i16::MIN), |(lo, hi), &m| (lo.min(m), hi.max(m)));
    let mut shift = 0i16;
    while min + shift < i16::from(MIDI_MIN) {
        shift += 12;
    }
    while max + shift > i16::from(MIDI_MAX) {
        shift -= 12;
    }
    for m in figure.iter_mut() {
        *m = (*m + shift).clamp(i16::from(MIDI_MIN), i16::from(MIDI_MAX));
    }
}

/// Human description of the drill, e.g.
/// `"G Dorian · up-down · 12 roots, random order · enclosed (one down) · 80 BPM"`.
fn label_for(spec: &VariationSpec, roots: &[u8]) -> String {
    let first_root = roots
        .first()
        .map(|&m| theory::pitch_class_name(m % 12).to_owned())
        .unwrap_or_else(|| "—".to_owned());

    let figure = if let Some(s) = spec.scale {
        format!("{first_root} {} · {}", s.scale.label(), s.pattern.label())
    } else if let Some(c) = spec.chord {
        let inv = if c.inversion > 0 {
            format!(", inv {}", c.inversion)
        } else {
            String::new()
        };
        format!(
            "{first_root} {}{inv} · {}",
            c.chord.label(),
            c.pattern.label()
        )
    } else if let Some(iv) = spec.interval {
        let dir = if iv.ascending { "up" } else { "down" };
        format!("{first_root} · interval of {} {dir}", iv.semitones)
    } else {
        format!("{first_root} · single notes")
    };

    let mut parts = vec![figure];
    let order = if spec.randomize_roots {
        format!("{} roots, random order", roots.len())
    } else {
        format!("{} roots", roots.len())
    };
    parts.push(order);
    if spec.direction == DirectionMode::RandomPerRoot {
        parts.push("random directions".to_owned());
    }
    if let Some(enc) = spec.enclosure {
        parts.push(format!("enclosed ({})", enc.label()));
    }
    parts.push(format!("{:.0} BPM", spec.rhythm.tempo_bpm));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_spec() -> VariationSpec {
        VariationSpec {
            roots: vec![60], // C4
            scale: Some(ScaleModifier {
                scale: ScaleType::Major,
                pattern: ScalePattern::Up,
            }),
            chord: None,
            interval: None,
            enclosure: None,
            direction: DirectionMode::Forward,
            rhythm: RhythmSpec::default(),
            randomize_roots: false,
        }
    }

    fn chromatic_roots() -> Vec<u8> {
        (60..72).collect()
    }

    /// F1 invariant: identical (spec, seed) → identical output, and a different
    /// seed actually changes a randomized spec's output. Fails if any hidden
    /// randomness (wall clock, thread_rng) sneaks in, or the seed is ignored.
    #[test]
    fn generate_is_seed_deterministic() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;
        spec.direction = DirectionMode::RandomPerRoot;

        assert_eq!(generate(&spec, 42), generate(&spec, 42));
        assert_ne!(
            generate(&spec, 42).target_midi,
            generate(&spec, 43).target_midi,
            "a different seed should reorder a randomized 12-root drill"
        );
    }

    /// RV's signature rule: the shuffle keeps the FIRST root fixed and permutes
    /// only the rest (same multiset). Fails if index 0 ever moves or a root is
    /// lost/duplicated.
    #[test]
    fn shuffle_keeps_first_root_and_permutes_the_rest() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;

        for seed in 0..20 {
            let seq = generate(&spec, seed);
            // First figure starts on the first root (C major up from C4).
            assert_eq!(
                seq.notes[0].midi, 60,
                "first root must stay fixed (seed {seed})"
            );
        }
        // Some seed must actually permute the tail (i.e. the shuffle is real).
        let baseline = {
            let mut s = base_spec();
            s.roots = chromatic_roots();
            generate(&s, 7).target_midi
        };
        let shuffled_differs = (0..20).any(|seed| generate(&spec, seed).target_midi != baseline);
        assert!(shuffled_differs, "randomize_roots must actually reorder");
    }

    /// Scale expansion correctness: C major up from C4 is exactly
    /// C D E F G A B C. Fails on any interval-table or expansion bug.
    #[test]
    fn c_major_up_is_the_diatonic_octave_run() {
        let seq = generate(&base_spec(), 0);
        assert_eq!(seq.target_midi, vec![60, 62, 64, 65, 67, 69, 71, 72]);
    }

    /// UpDown doesn't repeat the top note; DownUp doesn't repeat the bottom.
    #[test]
    fn updown_and_downup_do_not_repeat_the_turnaround() {
        let mut spec = base_spec();
        spec.scale = Some(ScaleModifier {
            scale: ScaleType::MajorPentatonic,
            pattern: ScalePattern::UpDown,
        });
        let up_down = generate(&spec, 0).target_midi;
        // 6 up (5 degrees + octave), 5 back down = 11 notes, palindrome.
        assert_eq!(up_down.len(), 11);
        assert_eq!(up_down[5], 72, "turnaround is the octave");
        assert_eq!(up_down[4], up_down[6], "descent mirrors the ascent");
        let rev: Vec<u8> = up_down.iter().rev().copied().collect();
        assert_eq!(up_down, rev, "up-down run is a palindrome");
    }

    /// Chord inversion rotates tones up an octave: C major triad inv 1 from C4
    /// arpeggiates E4 G4 C5.
    #[test]
    fn chord_inversion_rotates_up_an_octave() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.chord = Some(ChordModifier {
            chord: ChordType::MajorTriad,
            pattern: ArpeggioPattern::Ascending,
            inversion: 1,
        });
        assert_eq!(generate(&spec, 0).target_midi, vec![64, 67, 72]);
    }

    /// Enclosure approach notes precede the figure's first note — including
    /// after a reversal (they approach whatever is played first).
    #[test]
    fn enclosure_approaches_the_first_played_note() {
        let mut spec = base_spec();
        spec.enclosure = Some(Enclosure::OneDownOneUp);
        let seq = generate(&spec, 0);
        assert_eq!(&seq.target_midi[..3], &[59, 61, 60], "approach then target");

        spec.direction = DirectionMode::Reversed;
        let rev = generate(&spec, 0);
        // Reversed C-major run starts on the octave (72); enclosure approaches it.
        assert_eq!(&rev.target_midi[..3], &[71, 73, 72]);
    }

    /// Interval modifier: broken pair, ascending and descending.
    #[test]
    fn interval_builds_broken_pairs() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.interval = Some(IntervalModifier {
            semitones: 7,
            ascending: true,
        });
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 67]);
        spec.interval = Some(IntervalModifier {
            semitones: 7,
            ascending: false,
        });
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 53]);
    }

    /// Reversed direction reverses every figure.
    #[test]
    fn reversed_direction_reverses_the_figure() {
        let mut spec = base_spec();
        spec.direction = DirectionMode::Reversed;
        assert_eq!(
            generate(&spec, 0).target_midi,
            vec![72, 71, 69, 67, 65, 64, 62, 60]
        );
    }

    /// The beat grid: notes step by 1/notes_per_beat, roots are separated by
    /// the rest gap, and every figure starts on a whole beat.
    #[test]
    fn rhythm_grid_is_monotonic_with_rests_between_roots() {
        let mut spec = base_spec();
        spec.roots = vec![60, 62];
        spec.rhythm = RhythmSpec {
            notes_per_beat: 2,
            tempo_bpm: 100.0,
            rest_beats_between_roots: 1.0,
        };
        let seq = generate(&spec, 0);
        // 8 notes per root figure.
        assert_eq!(seq.notes.len(), 16);
        assert_eq!(seq.notes[0].start_beat, 0.0);
        assert_eq!(seq.notes[1].start_beat, 0.5);
        // Second figure starts on a whole beat after the rest (4.0 + 1.0 = 5.0).
        assert_eq!(seq.notes[8].start_beat, 5.0);
        // Monotonic non-overlapping starts throughout.
        for w in seq.notes.windows(2) {
            assert!(w[1].start_beat > w[0].start_beat);
        }
    }

    /// Out-of-range figures fold back into the playable window by whole
    /// octaves (contour preserved). Fails if extreme roots emit unplayable or
    /// wrapped notes.
    #[test]
    fn figures_fold_into_the_playable_range() {
        let mut spec = base_spec();
        spec.roots = vec![95]; // near the top: a major run would exceed MIDI_MAX
        let seq = generate(&spec, 0);
        assert!(seq
            .target_midi
            .iter()
            .all(|&m| (MIDI_MIN..=MIDI_MAX).contains(&m)));
        // Still a major run shape (successive diatonic steps).
        let deltas: Vec<i16> = seq
            .target_midi
            .windows(2)
            .map(|w| i16::from(w[1]) - i16::from(w[0]))
            .collect();
        assert_eq!(deltas, vec![2, 2, 1, 2, 2, 2, 1]);
    }

    /// The label names the material honestly.
    #[test]
    fn label_describes_the_drill() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;
        spec.scale = Some(ScaleModifier {
            scale: ScaleType::Dorian,
            pattern: ScalePattern::UpDown,
        });
        spec.enclosure = Some(Enclosure::OneDown);
        let label = generate(&spec, 3).label;
        assert!(label.contains("Dorian"), "got: {label}");
        assert!(label.contains("12 roots, random order"), "got: {label}");
        assert!(label.contains("enclosed (one down)"), "got: {label}");
        assert!(label.contains("80 BPM"), "got: {label}");
    }

    /// target_midi always mirrors notes (the grading contract).
    #[test]
    fn target_matches_notes() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;
        let seq = generate(&spec, 9);
        assert_eq!(
            seq.target_midi,
            seq.notes.iter().map(|n| n.midi).collect::<Vec<_>>()
        );
    }
}
