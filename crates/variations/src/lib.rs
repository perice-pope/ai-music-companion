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
    pub fn label(self) -> &'static str {
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
    /// time — RV's "rests"). Treated as a **minimum**: the next figure always
    /// starts on a whole beat, so the actual gap is rounded up to beat
    /// alignment (a 0.5-beat request after a triplet figure becomes ≥1 beat).
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
/// The figure comes from the first of these that is set, in precedence order
/// **cell > scale > chord > interval** (the rest are ignored); with none set
/// the figure is the bare root. `enclosure` composes with any of them,
/// approaching each figure's first note chromatically.
///
/// `cell` is the RV method's deepest primitive (see
/// `docs/architecture/rv-methodology.md`): ANY note sequence — most powerfully
/// a phrase the player just played — expressed as semitone offsets from its
/// first note, rowed through the 12 keys exactly like catalog material.
///
/// Deferred from the epic-spec sketch (documented drift, #252 §4): instrument
/// `transpose` (C/Bb/A/G/F/Eb views — lands with the notation adapter in #254,
/// where spelling/transposition belong), `stacked` chord/interval rendering
/// (needs chord rendering in the score path), and per-*note* random direction
/// (per-root is the RV behavior the coach wants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariationSpec {
    /// Root notes as MIDI numbers (e.g. the 12 chromatic roots from C4).
    pub roots: Vec<u8>,
    /// A custom cell: semitone offsets from the cell's first note (so a lifted
    /// phrase C4-E4-D4 is `[0, 4, 2]`). Takes precedence over every catalog
    /// modifier. Empty = ignored. Additive (`serde(default)`), so specs from
    /// before this field existed still parse.
    #[serde(default)]
    pub cell: Option<Vec<i8>>,
    /// RV's pattern database (#289): 1-based scale-DEGREE indices applied
    /// through the active scale (so `[1,2,3,5]` over C major = C D E G, over
    /// C Dorian = C D Eb G). Degrees past the scale length extend into the
    /// next octave (degree 9 of a 7-note scale = the 2nd, an octave up).
    /// Precedence: cell > degrees > the scale's own pattern. Empty = ignored.
    #[serde(default)]
    pub degrees: Option<Vec<u8>>,
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
    /// The roots in PLAY order (post-shuffle) — RV's signature randomized key
    /// sequence, surfaced so the UI can render it as the brand's colored cells.
    pub root_order: Vec<u8>,
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
        // Scramble the seed through one splitmix64 step so adjacent seeds
        // (0/1/2…) start from decorrelated states, and so seed 0 is distinct
        // from seed 1 (a bare xorshift locks at zero forever).
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)).max(1))
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
        root_order: roots,
        label,
        tempo_bpm: spec.rhythm.tempo_bpm,
        beats_per_measure: 4,
    }
}

/// Expand one root into its figure (before direction/enclosure), as i16 so
/// intermediate math can't wrap.
fn figure_for(spec: &VariationSpec, root: u8) -> Vec<i16> {
    let root = i16::from(root);

    // The player's own material rows through the keys before any catalog
    // figure — the RV method's core move.
    if let Some(cell) = spec.cell.as_ref().filter(|c| !c.is_empty()) {
        return cell.iter().map(|&off| root + i16::from(off)).collect();
    }

    if let Some(s) = spec.scale {
        // Degree pattern (#289): map each 1-based degree through the scale,
        // octave-extending past its length. Shadows the scale's own pattern.
        if let Some(pat) = spec.degrees.as_ref().filter(|d| !d.is_empty()) {
            let ivs = s.scale.semitones();
            return pat
                .iter()
                .map(|&d| {
                    let z = usize::from(d.max(1)) - 1;
                    let oct = (z / ivs.len()) as i16;
                    root + i16::from(ivs[z % ivs.len()]) + 12 * oct
                })
                .collect();
        }
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

    let figure = if spec.cell.as_ref().is_some_and(|c| !c.is_empty()) {
        let n = spec.cell.as_ref().map(Vec::len).unwrap_or(0);
        format!("{first_root} · your {n}-note cell")
    } else if let (Some(pat), Some(s)) = (
        spec.degrees.as_ref().filter(|d| !d.is_empty()),
        spec.scale.as_ref(),
    ) {
        let digits: Vec<String> = pat.iter().map(u8::to_string).collect();
        format!(
            "{first_root} {} · {} pattern",
            s.scale.label(),
            digits.join("-")
        )
    } else if let Some(s) = spec.scale {
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

    /// Extract the root each figure starts on (base-spec scale figures are 8
    /// notes long, so figure starts are every 8th target note).
    fn figure_roots(seq: &GeneratedSequence, figure_len: usize) -> Vec<u8> {
        seq.target_midi.chunks(figure_len).map(|c| c[0]).collect()
    }

    /// RV's signature rule: the shuffle keeps the FIRST root fixed and permutes
    /// only the rest — same multiset, nothing lost or duplicated. Fails if
    /// index 0 ever moves, if a swap bug drops/duplicates a root, or if the
    /// shuffle stops reordering at all.
    #[test]
    fn shuffle_keeps_first_root_and_permutes_the_rest() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;

        let mut any_reordered = false;
        for seed in 0..40 {
            let seq = generate(&spec, seed);
            let roots = figure_roots(&seq, 8);
            assert_eq!(roots[0], 60, "first root must stay fixed (seed {seed})");
            assert_eq!(
                seq.root_order, roots,
                "root_order must report the PLAYED order (seed {seed})"
            );
            // Multiset preserved: sorted roots equal the sorted input.
            let mut sorted = roots.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                chromatic_roots(),
                "shuffle must permute, not lose/duplicate (seed {seed})"
            );
            if roots != chromatic_roots() {
                any_reordered = true;
            }
        }
        assert!(any_reordered, "randomize_roots must actually reorder");
    }

    /// The tail shuffle is a real permutation generator: with 3 movable roots
    /// over many seeds every tail arrangement occurs. A Sattolo-style
    /// off-by-one (never self-swapping) makes some arrangements impossible and
    /// fails this.
    #[test]
    fn shuffle_reaches_every_tail_permutation() {
        let mut spec = base_spec();
        spec.roots = vec![60, 62, 64, 65]; // 1 fixed + 3 movable → 6 arrangements
        spec.randomize_roots = true;
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..200 {
            seen.insert(figure_roots(&generate(&spec, seed), 8));
        }
        assert_eq!(seen.len(), 6, "all 3! tail permutations must be reachable");
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

    // -----------------------------------------------------------------------
    // Catalog pins — the tables are product data a musician kid plays from;
    // a wrong semitone anywhere is a real bug. Pin the hard cases through
    // `generate` so a table edit can't slip past.
    // -----------------------------------------------------------------------

    fn scale_up(scale: ScaleType) -> Vec<u8> {
        let mut spec = base_spec();
        spec.scale = Some(ScaleModifier {
            scale,
            pattern: ScalePattern::Up,
        });
        generate(&spec, 0).target_midi
    }

    /// Pins the tricky scale tables from C4. Fails on any wrong semitone.
    #[test]
    fn scale_tables_are_pinned() {
        assert_eq!(
            scale_up(ScaleType::Dorian),
            vec![60, 62, 63, 65, 67, 69, 70, 72]
        );
        assert_eq!(
            scale_up(ScaleType::MelodicMinor),
            vec![60, 62, 63, 65, 67, 69, 71, 72]
        );
        assert_eq!(
            scale_up(ScaleType::HarmonicMinor),
            vec![60, 62, 63, 65, 67, 68, 71, 72]
        );
        assert_eq!(scale_up(ScaleType::Blues), vec![60, 63, 65, 66, 67, 70, 72]);
        assert_eq!(
            scale_up(ScaleType::Phrygian),
            vec![60, 61, 63, 65, 67, 68, 70, 72]
        );
        assert_eq!(
            scale_up(ScaleType::Lydian),
            vec![60, 62, 64, 66, 67, 69, 71, 72]
        );
        assert_eq!(
            scale_up(ScaleType::Mixolydian),
            vec![60, 62, 64, 65, 67, 69, 70, 72]
        );
        assert_eq!(
            scale_up(ScaleType::Locrian),
            vec![60, 61, 63, 65, 66, 68, 70, 72]
        );
        assert_eq!(
            scale_up(ScaleType::MinorPentatonic),
            vec![60, 63, 65, 67, 70, 72]
        );
    }

    /// Pins the seventh-chord tables from C4. Fails on any wrong chord tone.
    #[test]
    fn chord_tables_are_pinned() {
        let arp = |chord: ChordType| {
            let mut spec = base_spec();
            spec.scale = None;
            spec.chord = Some(ChordModifier {
                chord,
                pattern: ArpeggioPattern::Ascending,
                inversion: 0,
            });
            generate(&spec, 0).target_midi
        };
        assert_eq!(arp(ChordType::Dominant7), vec![60, 64, 67, 70]);
        assert_eq!(arp(ChordType::HalfDiminished7), vec![60, 63, 66, 70]);
        assert_eq!(arp(ChordType::Major7), vec![60, 64, 67, 71]);
        assert_eq!(arp(ChordType::Minor7), vec![60, 63, 67, 70]);
        assert_eq!(arp(ChordType::DiminishedTriad), vec![60, 63, 66]);
        assert_eq!(arp(ChordType::AugmentedTriad), vec![60, 64, 68]);
        assert_eq!(arp(ChordType::Sus4Triad), vec![60, 65, 67]);
    }

    /// Pins the remaining enclosure patterns (the approach notes precede the
    /// target, in table order).
    #[test]
    fn enclosure_tables_are_pinned() {
        let enc = |e: Enclosure| {
            let mut spec = base_spec();
            spec.scale = None; // bare root: figure is just C4
            spec.enclosure = Some(e);
            generate(&spec, 0).target_midi
        };
        assert_eq!(enc(Enclosure::OneUp), vec![61, 60]);
        assert_eq!(enc(Enclosure::TwoUp), vec![62, 61, 60]);
        assert_eq!(enc(Enclosure::TwoDown), vec![58, 59, 60]);
        assert_eq!(enc(Enclosure::OneUpOneDown), vec![61, 59, 60]);
    }

    /// Structural invariant over every scale table: starts at 0, strictly
    /// increasing, all offsets within the octave. Catches a disordered or
    /// out-of-range edit to any future table.
    #[test]
    fn all_scale_tables_are_well_formed() {
        let all = [
            ScaleType::Major,
            ScaleType::NaturalMinor,
            ScaleType::MajorPentatonic,
            ScaleType::MinorPentatonic,
            ScaleType::Blues,
            ScaleType::Dorian,
            ScaleType::Mixolydian,
            ScaleType::Lydian,
            ScaleType::Phrygian,
            ScaleType::HarmonicMinor,
            ScaleType::MelodicMinor,
            ScaleType::Locrian,
            ScaleType::Chromatic,
        ];
        for scale in all {
            let t = scale.semitones();
            assert_eq!(t[0], 0, "{scale:?} must start on the root");
            assert!(
                t.windows(2).all(|w| w[0] < w[1]),
                "{scale:?} must be strictly increasing"
            );
            assert!(
                t.iter().all(|&s| s < 12),
                "{scale:?} must stay in the octave"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Edge cases & the IPC contract
    // -----------------------------------------------------------------------

    /// Empty roots → an empty (but well-formed) sequence, never a panic.
    #[test]
    fn empty_roots_yield_an_empty_sequence() {
        let mut spec = base_spec();
        spec.roots = vec![];
        spec.randomize_roots = true;
        let seq = generate(&spec, 1);
        assert!(seq.notes.is_empty());
        assert!(seq.target_midi.is_empty());
    }

    /// notes_per_beat = 0 is guarded (treated as 1), not a division by zero.
    #[test]
    fn zero_notes_per_beat_is_guarded() {
        let mut spec = base_spec();
        spec.rhythm.notes_per_beat = 0;
        let seq = generate(&spec, 0);
        assert!(seq.notes[1].start_beat.is_finite());
        assert_eq!(seq.notes[1].start_beat, 1.0);
    }

    /// An inversion ≥ the chord size wraps (mod) instead of panicking or
    /// mis-shifting: inv 3 of a triad is root position again (an octave up
    /// is NOT applied to all three).
    #[test]
    fn oversized_inversion_wraps() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.chord = Some(ChordModifier {
            chord: ChordType::MajorTriad,
            pattern: ArpeggioPattern::Ascending,
            inversion: 3,
        });
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 64, 67]);
    }

    /// A negative rest is clamped to zero: the grid stays monotonic.
    #[test]
    fn negative_rest_keeps_the_grid_monotonic() {
        let mut spec = base_spec();
        spec.roots = vec![60, 62];
        spec.rhythm.rest_beats_between_roots = -3.0;
        let seq = generate(&spec, 0);
        for w in seq.notes.windows(2) {
            assert!(w[1].start_beat > w[0].start_beat);
        }
    }

    /// With two roots, "shuffle all but the first" is a no-op — stable order,
    /// no degenerate RNG path.
    #[test]
    fn two_roots_with_randomize_keep_their_order() {
        let mut spec = base_spec();
        spec.roots = vec![60, 67];
        spec.randomize_roots = true;
        for seed in 0..10 {
            assert_eq!(figure_roots(&generate(&spec, seed), 8), vec![60, 67]);
        }
    }

    /// #289: a degree pattern maps through the ACTIVE scale — `[1,2,3,5]`
    /// spells major thirds in major and minor thirds in dorian — and degrees
    /// past the scale length extend into the next octave. Fails if the
    /// mapping goes chromatic or the octave extension breaks.
    #[test]
    fn degree_patterns_map_through_the_scale() {
        let mut spec = base_spec();
        spec.degrees = Some(vec![1, 2, 3, 5]);
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 62, 64, 67]);
        spec.scale = Some(ScaleModifier {
            scale: ScaleType::Dorian,
            pattern: ScalePattern::Up,
        });
        assert_eq!(
            generate(&spec, 0).target_midi,
            vec![60, 62, 63, 67],
            "the same pattern through Dorian flats the 3rd"
        );
        spec.degrees = Some(vec![1, 8, 9]); // octave + the 2nd above it
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 72, 74]);
        assert!(generate(&spec, 0).label.contains("1-8-9 pattern"));
    }

    /// Precedence: a cell shadows degrees; empty degrees fall through to the
    /// scale's own pattern.
    #[test]
    fn cell_shadows_degrees_and_empty_degrees_fall_through() {
        let mut spec = base_spec();
        spec.degrees = Some(vec![1, 2, 3, 5]);
        spec.cell = Some(vec![0, 1]);
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 61]);
        spec.cell = None;
        spec.degrees = Some(vec![]);
        assert_eq!(
            generate(&spec, 0).target_midi,
            vec![60, 62, 64, 65, 67, 69, 71, 72],
            "empty degrees = the plain scale run"
        );
    }

    /// The RV method's deepest primitive (#285): a custom cell — e.g. a lifted
    /// player phrase as semitone offsets — rows through the keys exactly like
    /// catalog material, preserving its shape at every root. Fails if the cell
    /// path is dropped or the offsets are misapplied.
    #[test]
    fn a_custom_cell_rows_through_the_keys() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.cell = Some(vec![0, 4, 2, 7]); // C-E-D-G shaped lick
        spec.roots = vec![60, 62];
        let seq = generate(&spec, 0);
        assert_eq!(seq.target_midi, vec![60, 64, 62, 67, 62, 66, 64, 69]);
        assert!(seq.label.contains("4-note cell"), "got {}", seq.label);
    }

    /// Cell precedence: when a cell is present, catalog modifiers are ignored —
    /// the player's material wins. An empty cell is ignored (falls through).
    #[test]
    fn cell_takes_precedence_and_empty_cell_falls_through() {
        let mut spec = base_spec(); // has a Major scale set
        spec.cell = Some(vec![0, 3]);
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 63]);

        spec.cell = Some(vec![]);
        assert_eq!(
            generate(&spec, 0).target_midi,
            vec![60, 62, 64, 65, 67, 69, 71, 72],
            "an empty cell must fall through to the scale figure"
        );
    }

    /// Cells compose with the RV modifiers: direction reversal flips the cell,
    /// and an enclosure approaches its (post-direction) first note. Negative
    /// offsets (a descending lick) work.
    #[test]
    fn cells_compose_with_direction_and_enclosure() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.cell = Some(vec![0, -3, 5]); // down then up lick
        spec.direction = DirectionMode::Reversed;
        spec.enclosure = Some(Enclosure::OneDown);
        let seq = generate(&spec, 0);
        // Reversed cell from C4: 65, 57, 60; enclosure approaches 65 from below.
        assert_eq!(seq.target_midi, vec![64, 65, 57, 60]);
    }

    /// Additive schema: a spec serialized before `cell` existed still parses
    /// (serde default), so stored/replayed drills survive the upgrade.
    #[test]
    fn pre_cell_specs_still_parse() {
        let json = r#"{
            "roots": [60],
            "scale": { "scale": "major", "pattern": "up" },
            "chord": null,
            "interval": null,
            "enclosure": null,
            "direction": "forward",
            "rhythm": { "notes_per_beat": 2, "tempo_bpm": 80.0, "rest_beats_between_roots": 1.0 },
            "randomize_roots": false
        }"#;
        let spec: VariationSpec = serde_json::from_str(json).expect("old spec parses");
        assert!(spec.cell.is_none());
        assert_eq!(
            generate(&spec, 0).target_midi,
            vec![60, 62, 64, 65, 67, 69, 71, 72]
        );
    }

    /// The IPC contract: spec + sequence roundtrip through JSON with the
    /// snake_case enum encoding intact. Fails on any serde rename/derive drift
    /// that would break the Tauri boundary in #254.
    #[test]
    fn spec_and_sequence_roundtrip_through_json() {
        let mut spec = base_spec();
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;
        spec.direction = DirectionMode::RandomPerRoot;
        spec.enclosure = Some(Enclosure::OneDownOneUp);
        // Carry a real cell so the IPC boundary for #285 is actually pinned —
        // a serde regression that drops `cell` must fail here, not ship a
        // silent degrade to catalog material.
        spec.cell = Some(vec![0, 4, -3]);

        let spec_json = serde_json::to_string(&spec).unwrap();
        assert!(
            spec_json.contains("\"random_per_root\""),
            "enum must serialize snake_case, got: {spec_json}"
        );
        assert!(
            spec_json.contains("\"cell\":[0,4,-3]"),
            "the cell must cross the wire, got: {spec_json}"
        );
        assert!(spec_json.contains("\"one_down_one_up\""));
        let spec_back: VariationSpec = serde_json::from_str(&spec_json).unwrap();
        assert_eq!(spec_back, spec);

        let seq = generate(&spec, 5);
        let seq_back: GeneratedSequence =
            serde_json::from_str(&serde_json::to_string(&seq).unwrap()).unwrap();
        assert_eq!(seq_back, seq);
    }
}
