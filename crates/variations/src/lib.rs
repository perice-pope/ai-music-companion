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

/// Comfort window the generator folds figures toward (C2..=C7). This is a
/// register PREFERENCE, not truth (#471-2): a figure wider than the window
/// pokes past it rather than ever bending an interval — the only hard pitch
/// bound is physical MIDI 0..=127. #471-4 (H4) will narrow this per
/// instrument through [`fold_into_window`], under the same rule.
pub const MIDI_MIN: u8 = 36;
/// See [`MIDI_MIN`].
pub const MIDI_MAX: u8 = 96;
/// The RV grid's fixed meter: cells are placed ONE PER MEASURE of this size
/// (founder rule, 2026-07-08) — every root's figure starts on a barline.
pub const BEATS_PER_MEASURE: u8 = 4;

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
    /// #349 T2a: emit the chord as a BLOCK — all tones struck together on
    /// the measure's downbeat and held for the measure (one stacked cell
    /// per measure, the RV grid rule) — instead of an arpeggio walk.
    /// `pattern`, `direction`, and `enclosure` describe melodic order and
    /// are ignored for a simultaneity; `inversion` still voices the stack.
    #[serde(default)]
    pub stacked: bool,
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
    /// Historical "thinking time" between figures. **Inert since the RV grid
    /// rule (founder, 2026-07-08):** every figure now starts on a measure
    /// boundary and the breath is whatever remains of the measure, so this
    /// field no longer moves the grid. Kept in the wire format so persisted
    /// specs (exercise log, assignments) keep parsing.
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
/// where spelling/transposition belong) and per-*note* random direction
/// (per-root is the RV behavior the coach wants). `stacked` chord rendering
/// landed with #349 T2a ([`ChordModifier::stacked`]).
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
    /// #349 T3c: a lifted chord PROGRESSION — each step deals as a stacked
    /// block cell (one per measure), re-rooted per key. The player's own
    /// material, like `cell`: precedence cell > progression > scale >
    /// chord. Empty = ignored; additive on the wire.
    #[serde(default)]
    pub progression: Option<Vec<ProgressionStep>>,
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
    /// This note IS the root of the cell it belongs to (same pitch class as
    /// its own segment's root — any octave). RV color rule (founder,
    /// 2026-07-08): renderers color root notes and draw everything else
    /// white. Set per SEGMENT, not globally: in a 12-root row every pitch
    /// class is somebody's root, but inside C's cell only the C's are roots.
    #[serde(default)]
    pub is_root: bool,
    /// #349 T2a: notes sharing a group sound TOGETHER (a block chord).
    /// Renderers draw them as one vertical stack; MusicXML marks the 2nd+
    /// with `<chord/>`. `None` = ordinary melodic note. Group ids are the
    /// segment's index in play order — unique per stacked cell.
    #[serde(default)]
    pub chord_group: Option<u32>,
}

/// #349 T3c: one chord of a lifted PROGRESSION, relative to the row's key
/// root — "the ii chord, minor 7" is `offset: 2, chord: Minor7`, and it
/// re-roots in every key exactly like a cell's semitone offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressionStep {
    /// Semitones above the key root, 0..12. Register note: offsets always
    /// realize ABOVE the key root, so descending root motion (C → Am)
    /// renders its A a 6th up rather than a 3rd down — the harmonic
    /// identity is what one-cell-per-measure drills practice; nearest-
    /// realization (signed) offsets are a possible later refinement.
    pub offset: u8,
    /// The practice template for this chord.
    pub chord: ChordType,
}

/// #349 T2b: the grading target for one STACKED cell — what the T1 chord
/// engine should hear when the cell is played right. Segment == the cell
/// tones' `chord_group`, so verdicts land on the right measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChordTarget {
    /// Play-order segment index (mirrors `GeneratedNote::chord_group`).
    pub segment: u32,
    /// Expected root pitch class, 0-11 (C = 0).
    pub root_pc: u8,
    /// Expected quality in the T1 engine's vocabulary.
    pub quality: theory::ChordQuality,
    /// `Some(pc)` when the drill DEMANDS this bass (inversion drills) —
    /// grading then judges the sounding bass too. `None` = bass ignored.
    pub bass_pc: Option<u8>,
}

/// The generated drill: playable notes, the exact grading target, and a human
/// label describing what was asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedSequence {
    pub notes: Vec<GeneratedNote>,
    /// The grading target: `notes` in order, as MIDI numbers.
    pub target_midi: Vec<u8>,
    /// #349 T2b: per-stacked-cell chord targets, in play order. Empty for
    /// melodic material (additive on the wire — old sequences parse).
    #[serde(default)]
    pub chord_targets: Vec<ChordTarget>,
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
    let roots = shuffled_roots(spec, &mut rng);

    // 2. Per root: figure + direction + enclosure, folded into range.
    let mut notes: Vec<GeneratedNote> = Vec::new();
    let step = 1.0 / f64::from(spec.rhythm.notes_per_beat.max(1));
    let mut cursor_beat = 0.0_f64;

    let stacked_chord = active_stacked_chord(spec);
    let progression = active_progression(spec);

    let mut chord_targets: Vec<ChordTarget> = Vec::new();
    for (segment, &root) in roots.iter().enumerate() {
        // Progression row (#349 T3c): every step deals as a stacked block
        // cell — one chord per measure, re-rooted from THIS key's root by
        // its offset. chord_group stays unique per dealt cell so renderers
        // and grading attribute stacks correctly.
        if let Some(steps) = progression {
            for (step_idx, step) in steps.iter().enumerate() {
                let step_root = i16::from(root) + i16::from(step.offset % 12);
                let mut tones = chord_tones(
                    ChordModifier {
                        chord: step.chord,
                        pattern: ArpeggioPattern::Ascending,
                        inversion: 0,
                        stacked: true,
                    },
                    step_root,
                );
                fold_into_range(&mut tones);
                tones.sort_unstable();
                tones.dedup();
                let group = (segment * steps.len() + step_idx) as u32;
                chord_targets.push(ChordTarget {
                    segment: group,
                    root_pc: (step_root.rem_euclid(12)) as u8,
                    quality: step.chord.quality(),
                    bass_pc: None,
                });
                for &m in &tones {
                    notes.push(GeneratedNote {
                        midi: m as u8,
                        start_beat: cursor_beat,
                        duration_beats: f64::from(BEATS_PER_MEASURE),
                        // RV color rule: THIS cell's root pc colors; the
                        // other chord tones draw white.
                        is_root: m.rem_euclid(12) == step_root.rem_euclid(12),
                        chord_group: Some(group),
                    });
                }
                cursor_beat += f64::from(BEATS_PER_MEASURE);
            }
            continue;
        }
        // Block chord: every tone struck on the measure's downbeat, held for
        // the measure — one stacked cell per measure. Direction and
        // enclosure describe melodic order; a simultaneity has none.
        if let Some(c) = stacked_chord {
            let mut tones = chord_tones(c, i16::from(root));
            fold_into_range(&mut tones);
            tones.sort_unstable();
            tones.dedup(); // guard duplicate template tones (fold no longer
                           // collides tones — it never clamps, #471-2)
                           // The grading target (#349 T2b): what the T1 engine should hear.
                           // An inversion drill demands its voicing's lowest tone as the
                           // bass; root-position drills leave the bass unjudged.
                           // The EFFECTIVE inversion (chord_tones wraps oversize values):
                           // inversion 3 of a triad voices root position and must not
                           // silently demand a bass (review nit — harsher grading).
            let effective_inversion = usize::from(c.inversion) % c.chord.semitones().len().max(1);
            chord_targets.push(ChordTarget {
                segment: segment as u32,
                root_pc: root % 12,
                quality: c.chord.quality(),
                bass_pc: (effective_inversion > 0)
                    .then(|| tones.first().map(|&m| (m.rem_euclid(12)) as u8))
                    .flatten(),
            });
            for &m in &tones {
                notes.push(GeneratedNote {
                    midi: m as u8,
                    start_beat: cursor_beat,
                    duration_beats: f64::from(BEATS_PER_MEASURE),
                    is_root: m.rem_euclid(12) == i16::from(root).rem_euclid(12),
                    chord_group: Some(segment as u32),
                });
            }
            cursor_beat += f64::from(BEATS_PER_MEASURE);
            continue;
        }

        let reversed = segment_reversed(spec, &mut rng);
        let mut figure = realized_figure(spec, root, reversed);

        fold_into_range(&mut figure);

        for m in &figure {
            notes.push(GeneratedNote {
                midi: *m as u8,
                start_beat: cursor_beat,
                duration_beats: step,
                // Root of THIS cell only (octave-insensitive). Enclosure
                // approach notes and scale neighbors stay non-root even when
                // some other cell in the row is anchored on their pitch class.
                is_root: (*m).rem_euclid(12) == i16::from(root).rem_euclid(12),
                chord_group: None,
            });
            cursor_beat += step;
        }
        // RV grid rule (founder, 2026-07-08): ONE CELL PER MEASURE. Every
        // root's figure starts on a measure boundary — a 4-note cell reads as
        // four notes to the bar, an exact-measure scale run flows measure to
        // measure, and a longer figure owns whole measures before the next
        // root begins. The breath between cells is whatever remains of the
        // measure; `rest_beats_between_roots` stays in the wire format for
        // compat but no longer moves the grid.
        cursor_beat =
            (cursor_beat / f64::from(BEATS_PER_MEASURE)).ceil() * f64::from(BEATS_PER_MEASURE);
    }

    let target_midi = notes.iter().map(|n| n.midi).collect();
    let label = label_for(spec, &roots);

    GeneratedSequence {
        notes,
        target_midi,
        chord_targets,
        root_order: roots,
        label,
        tempo_bpm: spec.rhythm.tempo_bpm,
        beats_per_measure: BEATS_PER_MEASURE,
    }
}

/// RV's shuffle: keeps the first root fixed and permutes the rest (seeded
/// Fisher–Yates). Extracted so [`unfolded_figures`] consumes the seed
/// identically to [`generate`].
fn shuffled_roots(spec: &VariationSpec, rng: &mut Xorshift64) -> Vec<u8> {
    let mut roots = spec.roots.clone();
    if spec.randomize_roots && roots.len() > 2 {
        for i in (2..roots.len()).rev() {
            let j = 1 + rng.below(i); // 1..=i — never index 0
            roots.swap(i, j);
        }
    }
    roots
}

/// #349 T2a: a stacked chord is the effective figure only when the chord
/// modifier IS the figure (cell and scale take precedence, same order as
/// `figure_for`).
fn active_stacked_chord(spec: &VariationSpec) -> Option<ChordModifier> {
    if spec.cell.as_ref().is_none_or(|c| c.is_empty()) && spec.scale.is_none() {
        spec.chord.filter(|c| c.stacked)
    } else {
        None
    }
}

/// #349 T3c: a lifted progression is the player's material — it outranks
/// catalog figures (cell still wins: cell > progression > scale > chord).
fn active_progression(spec: &VariationSpec) -> Option<&[ProgressionStep]> {
    if spec.cell.as_ref().is_none_or(|c| c.is_empty()) {
        spec.progression.as_deref().filter(|p| !p.is_empty())
    } else {
        None
    }
}

/// One melodic segment's direction draw. RandomPerRoot consumes exactly one
/// coin per segment — [`generate`] and [`unfolded_figures`] MUST both go
/// through here so their RNG streams stay in lockstep.
fn segment_reversed(spec: &VariationSpec, rng: &mut Xorshift64) -> bool {
    match spec.direction {
        DirectionMode::Forward => false,
        DirectionMode::Reversed => true,
        DirectionMode::RandomPerRoot => rng.coin(),
    }
}

/// One root's fully-realized melodic figure — figure + direction + enclosure —
/// BEFORE any register fold. This is the truth of the cell's shape.
fn realized_figure(spec: &VariationSpec, root: u8, reversed: bool) -> Vec<i16> {
    let mut figure = figure_for(spec, root);
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
    figure
}

/// The exact PRE-FOLD melodic figures [`generate`] deals, per segment in play
/// order (#471-2 F2): same shuffle, same per-root direction draws, same
/// enclosure — only the register fold is left out. This is what the edit
/// engine bakes cell offsets from, so a fold can never smuggle a register
/// shift (or, historically, a clamp) into the player's material.
/// Empty for stacked-chord and progression rows — they deal no melodic cell.
pub fn unfolded_figures(spec: &VariationSpec, seed: u64) -> Vec<Vec<i16>> {
    let mut rng = Xorshift64::new(seed);
    let roots = shuffled_roots(spec, &mut rng);
    if active_stacked_chord(spec).is_some() || active_progression(spec).is_some() {
        return Vec::new();
    }
    roots
        .iter()
        .map(|&root| {
            let reversed = segment_reversed(spec, &mut rng);
            realized_figure(spec, root, reversed)
        })
        .collect()
}

/// Expand one root into its figure (before direction/enclosure), as i16 so
/// intermediate math can't wrap.
/// The voiced chord tones for a root: template intervals with `inversion`
/// tones rotated up an octave, in ascending playing order.
fn chord_tones(c: ChordModifier, root: i16) -> Vec<i16> {
    let mut tones: Vec<i16> = c
        .chord
        .semitones()
        .iter()
        .map(|&t| root + i16::from(t))
        .collect();
    let n = usize::from(c.inversion) % tones.len().max(1);
    for tone in tones.iter_mut().take(n) {
        *tone += 12;
    }
    tones.rotate_left(n);
    tones
}

/// Expand one root into its melodic figure (cell > scale > chord >
/// interval > bare root), as MIDI values before range folding.
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
        let mut tones = chord_tones(c, root);
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

/// Shift a whole figure by ONE whole-octave amount toward the given register
/// window — never per note, never a clamp (#471-2 F1: a wrong interval is the
/// one thing this generator must never emit; RV is "the same cell, exactly,
/// through 12 keys").
///
/// Placement criterion (deterministic, pinned by tests):
/// 1. minimize the figure's worst poke past the window — the larger of the
///    two per-end overflows, 0 when the figure fits. For any figure that FITS
///    the window this reproduces the legacy fold's placement exactly, so
///    stored-seed replays of never-broken rows stay bit-identical; figures
///    wider than the window get their overflow balanced around it instead of
///    (as before) silently clamped flat;
/// 2. ties break to the smallest |shift| (stay nearest the as-built register);
/// 3. a remaining tie breaks to the lower octave.
///
/// If the placed figure would poke past physical MIDI (0..=127), the shift
/// steps back inside — register beats centering; intervals never bend. A
/// figure no whole-octave shift can fit into 0..=127 (span > 127 semitones —
/// hostile wire only; every UI-constructible cell spans ≤ 72 and is proven in
/// tests to always fit) is a loud validation panic, never a clamp.
///
/// `lo..=hi` is the comfort window — `MIDI_MIN..=MIDI_MAX` today; #471-4 (H4)
/// plugs per-instrument windows in here.
pub fn fold_into_window(figure: &mut [i16], lo: u8, hi: u8) {
    if figure.is_empty() {
        return;
    }
    let (lo, hi) = (i16::from(lo), i16::from(hi.max(lo)));
    let (min, max) = figure
        .iter()
        .fold((i16::MAX, i16::MIN), |(a, b), &m| (a.min(m), b.max(m)));
    // Scan every octave shift that could possibly matter: from "top of the
    // figure at the window floor" to "bottom of the figure at the window
    // ceiling", one octave of slack each side.
    let k_lo = (lo - max).div_euclid(12) - 1;
    let k_hi = (hi - min).div_euclid(12) + 1;
    let mut shift = (k_lo..=k_hi)
        .map(|k| k * 12)
        .min_by_key(|&s| {
            let poke_low = (lo - (min + s)).max(0);
            let poke_high = ((max + s) - hi).max(0);
            // Lexicographic: worst poke, then |shift|, then the lower octave.
            (poke_low.max(poke_high), s.abs(), s)
        })
        .expect("candidate range is non-empty");
    // Physical MIDI is the only hard bound. Step the WHOLE figure back inside
    // if the comfort placement pokes past it (still a whole-octave move).
    while max + shift > 127 {
        shift -= 12;
    }
    while min + shift < 0 {
        shift += 12;
    }
    assert!(
        min + shift >= 0 && max + shift <= 127,
        "figure spans {} semitones — no whole-octave shift fits MIDI 0..=127; \
         refusing to clamp (a lied interval is worse than a loud failure, #471-2)",
        max - min
    );
    for m in figure.iter_mut() {
        *m += shift;
    }
}

/// [`fold_into_window`] with the default comfort window.
fn fold_into_range(figure: &mut [i16]) {
    fold_into_window(figure, MIDI_MIN, MIDI_MAX);
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
    } else if let Some(steps) = spec.progression.as_ref().filter(|p| !p.is_empty()) {
        // Chord names spelled sharp here; the coach respells the whole
        // label to the engraved signature (#335), same as every row.
        let names: Vec<String> = steps
            .iter()
            .map(|st| {
                let root = roots.first().copied().unwrap_or(60);
                let pc = (root + st.offset) % 12;
                format!(
                    "{}{}",
                    theory::pitch_class_name(pc),
                    st.chord.quality().suffix()
                )
            })
            .collect();
        format!("your progression · {}", names.join(" → "))
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
        // A stack has no melodic order — say "block chords", not a pattern.
        let motion = if c.stacked {
            "block chords"
        } else {
            c.pattern.label()
        };
        format!("{first_root} {}{inv} · {}", c.chord.label(), motion)
    } else if let Some(iv) = spec.interval {
        let dir = if iv.ascending { "up" } else { "down" };
        format!("{first_root} · interval of {} {dir}", iv.semitones)
    } else {
        format!("{first_root} · single notes")
    };

    let mut parts = vec![figure];
    // One root has no order or directions to shuffle — the knobs may stay on
    // internally, but the label must not brag about randomizing a single item (#391).
    let order = match (roots.len(), spec.randomize_roots) {
        (1, _) => "1 root".to_owned(),
        (n, true) => format!("{n} roots, random order"),
        (n, false) => format!("{n} roots"),
    };
    parts.push(order);
    if spec.direction == DirectionMode::RandomPerRoot && roots.len() > 1 {
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
            progression: None,
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
            stacked: false,
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

    /// The beat grid: notes step by 1/notes_per_beat and every figure starts
    /// on a MEASURE boundary (the RV one-cell-per-measure rule) — an
    /// exact-measure scale run flows measure to measure with no dead bar.
    #[test]
    fn rhythm_grid_flows_measure_to_measure_on_exact_fills() {
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
        // The first figure fills measure 1 exactly (8 notes × 0.5 = 4 beats),
        // so the second starts right at the next measure boundary — the rest
        // field no longer pushes it into a dead bar.
        assert_eq!(seq.notes[8].start_beat, 4.0);
        // Monotonic non-overlapping starts throughout.
        for w in seq.notes.windows(2) {
            assert!(w[1].start_beat > w[0].start_beat);
        }
    }

    /// The RV grid rule (founder, 2026-07-08): one cell per measure. A 4-note
    /// cell at one note per beat owns exactly one bar per root; a figure that
    /// spills past a barline owns whole measures and the next root starts on
    /// the next boundary. Fails if cell starts drift off measure boundaries.
    #[test]
    fn one_cell_per_measure_anchors_every_root_on_a_barline() {
        // 4-note cell, 1 note/beat → roots at beats 0, 4, 8.
        let mut spec = base_spec();
        spec.scale = None;
        spec.cell = Some(vec![0, 3, 5, 7]);
        spec.roots = vec![60, 62, 64];
        spec.rhythm = RhythmSpec {
            notes_per_beat: 1,
            tempo_bpm: 100.0,
            rest_beats_between_roots: 1.0,
        };
        let seq = generate(&spec, 0);
        let starts: Vec<f64> = seq.notes.iter().map(|n| n.start_beat).collect();
        assert_eq!(&starts[0..4], &[0.0, 1.0, 2.0, 3.0], "cell 1 = measure 1");
        assert_eq!(&starts[4..8], &[4.0, 5.0, 6.0, 7.0], "cell 2 = measure 2");
        assert_eq!(starts[8], 8.0, "cell 3 = measure 3");

        // A 6-note cell at 2 notes/beat ends mid-measure (3 beats); the next
        // root still waits for the barline.
        spec.cell = Some(vec![0, 2, 3, 5, 7, 8]);
        spec.rhythm.notes_per_beat = 2;
        let seq = generate(&spec, 0);
        assert_eq!(
            seq.notes[6].start_beat, 4.0,
            "mid-measure cell end still snaps"
        );

        // A figure longer than a measure (9 notes at 2/beat = 4.5 beats) owns
        // two measures; the next root starts at beat 8.
        spec.cell = Some(vec![0, 2, 3, 5, 7, 8, 10, 12, 14]);
        let seq = generate(&spec, 0);
        assert_eq!(
            seq.notes[9].start_beat, 8.0,
            "long cells own whole measures"
        );
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

    // -----------------------------------------------------------------------
    // #471-2 F1 — transposition NEVER loses the pattern. The investigation's
    // exact repros, pinned: the old fold's per-note clamp flattened intervals
    // on any figure wider than 49 semitones, in exactly the keys whose octave
    // shift overflowed ("broken keys ≈ span − 49").
    // -----------------------------------------------------------------------

    fn delta_seq(midis: &[u8]) -> Vec<i16> {
        midis
            .windows(2)
            .map(|w| i16::from(w[1]) - i16::from(w[0]))
            .collect()
    }

    /// The true delta sequence of a cell after direction + enclosure,
    /// computed from first principles (independently of `realized_figure`,
    /// so a bug shared with production can't hide).
    fn expected_deltas(cell: &[i8], reversed: bool, enclosure: Option<Enclosure>) -> Vec<i16> {
        let mut offs: Vec<i16> = cell.iter().map(|&o| i16::from(o)).collect();
        if reversed {
            offs.reverse();
        }
        if let Some(enc) = enclosure {
            let first = offs[0];
            let mut with: Vec<i16> = enc
                .approach_semitones()
                .iter()
                .map(|&s| first + i16::from(s))
                .collect();
            with.extend_from_slice(&offs);
            offs = with;
        }
        offs.windows(2).map(|w| w[1] - w[0]).collect()
    }

    /// AC1: the investigation's repro cells — span 50 (the root-61 break),
    /// span 55 ("6 of 12 keys wrong, up to 6-semitone errors"), and a span-72
    /// lifted-style cell (the legal maximum: ±MAX_CELL_OFFSET) — play the
    /// EXACT interval sequence in all 12 keys, Forward AND Reversed AND with
    /// an enclosure. Fails if any fold ever bends an interval again.
    #[test]
    fn wide_cells_transpose_exactly_in_every_key() {
        let repros: [&[i8]; 3] = [
            &[0, 25, -25],                    // span 50
            &[0, 16, 28, 36, 26, 14, 2, -19], // span 55
            &[0, 36, -36, 24, -24, 12, -12],  // span 72, lifted-style extremes
        ];
        for cell in repros {
            for direction in [DirectionMode::Forward, DirectionMode::Reversed] {
                for enclosure in [None, Some(Enclosure::OneDownOneUp)] {
                    let mut spec = base_spec();
                    spec.scale = None;
                    spec.cell = Some(cell.to_vec());
                    spec.roots = chromatic_roots(); // 60..72 — root 61 included
                    spec.direction = direction;
                    spec.enclosure = enclosure;
                    let seq = generate(&spec, 3);
                    let seg_len = seq.target_midi.len() / 12;
                    let expected =
                        expected_deltas(cell, direction == DirectionMode::Reversed, enclosure);
                    for (root, seg) in seq.root_order.iter().zip(seq.target_midi.chunks(seg_len)) {
                        assert_eq!(
                            delta_seq(seg),
                            expected,
                            "root {root} must play the cell's exact interval sequence \
                             (cell {cell:?}, {direction:?}, enclosure {enclosure:?})"
                        );
                    }
                }
            }
        }
    }

    /// AC2 — the physical-fit proof: a span-72 figure (the widest a ±36-offset
    /// cell can be) fits MIDI 0..=127 under the chosen shift at EVERY
    /// placement and every 12-key root, span preserved exactly. This is why
    /// the loud-failure path is unreachable for UI-constructible material.
    #[test]
    fn span_72_always_fits_physical_midi() {
        for lo_off in -72..=0i16 {
            let cell = vec![lo_off as i8, (lo_off + 72) as i8];
            for root in 60..=71u8 {
                let mut spec = base_spec();
                spec.scale = None;
                spec.cell = Some(cell.clone());
                spec.roots = vec![root];
                let seq = generate(&spec, 0);
                // All notes physical (u8 already ≤ 255; assert the real bound)…
                assert!(
                    seq.target_midi.iter().all(|&m| m <= 127),
                    "root {root}, placement {lo_off}: {:?}",
                    seq.target_midi
                );
                // …and the 72-semitone interval is EXACT, never clamped.
                assert_eq!(
                    delta_seq(&seq.target_midi),
                    vec![72],
                    "root {root}, placement {lo_off}"
                );
            }
        }
    }

    /// AC3: a figure that cannot fit 0..=127 under ANY whole-octave shift
    /// (span > 127 — hostile wire only) fails LOUDLY. A silent clamp here
    /// would lie an interval; that is the one forbidden outcome.
    #[test]
    #[should_panic(expected = "refusing to clamp")]
    fn an_unfittable_figure_fails_loudly_instead_of_lying() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.cell = Some(vec![-128, 127]); // span 255 > 127: no shift fits
        generate(&spec, 0);
    }

    /// AC4 — replay stability: any figure the old fold placed WITHOUT
    /// clamping keeps its exact legacy placement (smallest fitting |shift|),
    /// so stored-seed replays of never-broken rows are bit-identical. Pinned
    /// on the root-95 major run the fold suite has always used.
    #[test]
    fn fitting_figures_keep_their_legacy_placement() {
        let mut spec = base_spec();
        spec.roots = vec![95];
        let seq = generate(&spec, 0);
        assert_eq!(
            seq.target_midi,
            vec![83, 85, 87, 88, 90, 92, 94, 95],
            "the legacy -12 placement, not a deeper re-centering"
        );
    }

    /// F2's seam: `unfolded_figures` deals the SAME segments as `generate` —
    /// same shuffle, same per-root direction coins, same enclosure — differing
    /// only by the whole-octave register shift. Fails if the two RNG streams
    /// ever drift apart (which would bake the wrong segment's shape).
    #[test]
    fn unfolded_figures_mirror_generates_segments() {
        let mut spec = base_spec();
        spec.scale = None;
        spec.cell = Some(vec![0, 16, 28, 36, 26, 14, 2, -19]);
        spec.roots = chromatic_roots();
        spec.randomize_roots = true;
        spec.direction = DirectionMode::RandomPerRoot;
        spec.enclosure = Some(Enclosure::TwoDown);
        let seq = generate(&spec, 11);
        let figs = unfolded_figures(&spec, 11);
        assert_eq!(figs.len(), 12);
        let seg_len = seq.target_midi.len() / 12;
        for (i, fig) in figs.iter().enumerate() {
            let mut folded = fig.clone();
            fold_into_window(&mut folded, MIDI_MIN, MIDI_MAX);
            let dealt: Vec<i16> = seq.target_midi[i * seg_len..(i + 1) * seg_len]
                .iter()
                .map(|&m| i16::from(m))
                .collect();
            assert_eq!(folded, dealt, "segment {i} must be the folded truth");
        }
        // Stacked/progression rows deal no melodic cell to bake.
        let stacked = stacked_spec(ChordType::Dominant7, 0);
        assert!(unfolded_figures(&stacked, 7).is_empty());
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

    /// #391: a one-root drill says "1 root" and never claims to shuffle order
    /// or directions — there is nothing to randomize. Fails if the singular
    /// form regresses to "1 roots" or the meaningless modifiers reappear.
    #[test]
    fn single_root_label_drops_meaningless_modifiers() {
        let mut spec = base_spec();
        spec.randomize_roots = true;
        spec.direction = DirectionMode::RandomPerRoot;
        let label = generate(&spec, 0).label;
        assert!(label.contains("1 root"), "got: {label}");
        assert!(!label.contains("roots"), "got: {label}");
        assert!(!label.contains("random order"), "got: {label}");
        assert!(!label.contains("random directions"), "got: {label}");
    }

    /// #391 guard-rail: the suppression is strictly the one-root case — two
    /// roots still declare their shuffle and random directions.
    #[test]
    fn two_root_label_keeps_modifiers() {
        let mut spec = base_spec();
        spec.roots = vec![60, 67];
        spec.randomize_roots = true;
        spec.direction = DirectionMode::RandomPerRoot;
        let label = generate(&spec, 0).label;
        assert!(label.contains("2 roots, random order"), "got: {label}");
        assert!(label.contains("random directions"), "got: {label}");
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
                stacked: false,
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
            stacked: false,
        });
        assert_eq!(generate(&spec, 0).target_midi, vec![60, 64, 67]);
    }

    /// The rest field is inert under the RV grid rule: ANY value (negative,
    /// zero, huge) yields the identical measure-aligned grid, and the grid
    /// stays monotonic. Fails if rests start moving cell placement again
    /// without a deliberate decision.
    #[test]
    fn rest_field_is_inert_and_the_grid_stays_monotonic() {
        let mut spec = base_spec();
        spec.roots = vec![60, 62];
        let grids: Vec<Vec<f64>> = [-3.0, 0.0, 1.0, 7.5]
            .into_iter()
            .map(|rest| {
                spec.rhythm.rest_beats_between_roots = rest;
                generate(&spec, 0)
                    .notes
                    .iter()
                    .map(|n| n.start_beat)
                    .collect()
            })
            .collect();
        assert!(
            grids.windows(2).all(|w| w[0] == w[1]),
            "the grid must not depend on the rest field"
        );
        for w in grids[0].windows(2) {
            assert!(w[1] > w[0], "grid stays monotonic");
        }
    }

    /// RV color rule: `is_root` marks each cell's OWN root only (any octave).
    /// A pitch class that is some OTHER cell's root — G inside C's figure
    /// when G is also a dealt root — stays unmarked, and a 12-root row still
    /// has plenty of unmarked (white) notes. Fails if the flag regresses to a
    /// global pitch-class check (which colors everything at 12 roots).
    #[test]
    fn is_root_marks_only_each_cells_own_root() {
        let mut spec = base_spec(); // C major scale, Up: 8 notes per figure
        spec.roots = vec![60, 67]; // C and G — G appears inside C's figure
        let seq = generate(&spec, 0);
        let c_figure = &seq.notes[0..8];
        let flags: Vec<bool> = c_figure.iter().map(|n| n.is_root).collect();
        assert_eq!(
            flags,
            vec![true, false, false, false, false, false, false, true],
            "in C's cell only the C's are roots — the G inside it is not, \
             even though G anchors the next cell"
        );
        let g_figure = &seq.notes[8..16];
        assert!(g_figure[0].is_root && g_figure[7].is_root);
        assert!(!g_figure[1].is_root);

        // The flagship 12-root row: every pitch class is somebody's root,
        // but most notes are still white.
        spec.roots = (60..72).collect();
        let seq = generate(&spec, 0);
        let white = seq.notes.iter().filter(|n| !n.is_root).count();
        assert_eq!(
            white,
            12 * 6,
            "each 8-note figure keeps 6 non-root (white) notes"
        );
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

    fn stacked_spec(chord: ChordType, inversion: u8) -> VariationSpec {
        VariationSpec {
            roots: chromatic_roots(),
            progression: None,
            scale: None,
            chord: Some(ChordModifier {
                chord,
                pattern: ArpeggioPattern::Ascending,
                inversion,
                stacked: true,
            }),
            ..base_spec()
        }
    }

    /// #349 T2a AC1: a "C7 in all 12 keys" stacked drill deals 12 block
    /// cells — one measure each, every tone struck on the downbeat, held
    /// for the measure, sharing that segment's chord_group. Fails if the
    /// stack path walks tones melodically or breaks the one-cell-per-
    /// measure grid.
    #[test]
    fn a_stacked_drill_deals_one_block_chord_per_measure() {
        let seq = generate(&stacked_spec(ChordType::Dominant7, 0), 7);
        assert_eq!(seq.notes.len(), 48, "12 roots x 4 tones");
        for (segment, chunk) in seq.notes.chunks(4).enumerate() {
            let downbeat = (segment * usize::from(BEATS_PER_MEASURE)) as f64;
            for n in chunk {
                assert_eq!(n.start_beat, downbeat, "struck together: {n:?}");
                assert_eq!(n.duration_beats, f64::from(BEATS_PER_MEASURE));
                assert_eq!(n.chord_group, Some(segment as u32));
            }
            // Ascending voicing, root position: the root is the lowest tone
            // and the only is_root in its cell.
            assert!(chunk.windows(2).all(|w| w[0].midi < w[1].midi));
            assert_eq!(
                chunk.iter().filter(|n| n.is_root).count(),
                1,
                "one root per dominant-7 cell"
            );
            assert!(chunk[0].is_root, "root position: lowest tone is the root");
        }
        assert!(
            seq.label.contains("block chords"),
            "label must say what it deals: {}",
            seq.label
        );
    }

    /// Inversion still voices the stack: first inversion puts the 3rd in
    /// the bass (root no longer lowest). Fails if `stacked` ignores the
    /// inversion.
    #[test]
    fn a_stacked_inversion_revoices_the_block() {
        let spec = VariationSpec {
            roots: vec![60],
            ..stacked_spec(ChordType::MajorTriad, 1)
        };
        let seq = generate(&spec, 1);
        assert_eq!(seq.notes.len(), 3);
        assert!(
            !seq.notes[0].is_root,
            "first inversion: the bass is the 3rd, not the root: {:?}",
            seq.notes
        );
        assert_eq!(seq.notes.iter().filter(|n| n.is_root).count(), 1);
    }

    /// Melodic figures are untouched by T2a: an ordinary arpeggio spec
    /// (stacked = false) carries no chord groups anywhere. Guards AC3
    /// (free play and melodic drills bit-identical).
    #[test]
    fn melodic_figures_carry_no_chord_groups() {
        let spec = VariationSpec {
            roots: chromatic_roots(),
            progression: None,
            scale: None,
            chord: Some(ChordModifier {
                chord: ChordType::Dominant7,
                pattern: ArpeggioPattern::UpDown,
                inversion: 0,
                stacked: false,
            }),
            ..base_spec()
        };
        let seq = generate(&spec, 7);
        assert!(seq.notes.iter().all(|n| n.chord_group.is_none()));
        // And the walk is still melodic: consecutive distinct beats.
        assert!(seq
            .notes
            .windows(2)
            .all(|w| w[0].start_beat < w[1].start_beat));
    }

    /// Wire compat: specs and sequences persisted BEFORE T2a (no `stacked`,
    /// no `chord_group`) still parse — the exercise log must keep replaying.
    #[test]
    fn t2a_fields_are_additive_on_the_wire() {
        let old_modifier = r#"{"chord":"dominant7","pattern":"ascending","inversion":0}"#;
        let m: ChordModifier = serde_json::from_str(old_modifier).expect("old modifier parses");
        assert!(!m.stacked);
        let old_note = r#"{"midi":60,"start_beat":0.0,"duration_beats":1.0,"is_root":true}"#;
        let n: GeneratedNote = serde_json::from_str(old_note).expect("old note parses");
        assert_eq!(n.chord_group, None);
    }

    /// Precedence holds: a cell (the RV primitive) shadows a stacked chord
    /// modifier entirely — no stray groups from the ignored modifier.
    #[test]
    fn a_cell_shadows_a_stacked_chord() {
        let spec = VariationSpec {
            cell: Some(vec![0, 4, 2]),
            ..stacked_spec(ChordType::Dominant7, 0)
        };
        let seq = generate(&spec, 3);
        assert!(seq.notes.iter().all(|n| n.chord_group.is_none()));
        assert!(seq.label.contains("cell"), "label: {}", seq.label);
    }

    /// The other half of the precedence order (cell > SCALE > chord): a
    /// scale modifier shadows a stacked chord too — the drill deals the
    /// scale run, melodically, with no stray groups. Fails if the stacked
    /// guard stops checking `spec.scale`.
    #[test]
    fn a_scale_shadows_a_stacked_chord() {
        let spec = VariationSpec {
            progression: None,
            scale: Some(ScaleModifier {
                scale: ScaleType::Major,
                pattern: ScalePattern::Up,
            }),
            ..stacked_spec(ChordType::Dominant7, 0)
        };
        let seq = generate(&spec, 3);
        assert!(seq.notes.iter().all(|n| n.chord_group.is_none()));
        assert!(
            seq.notes
                .windows(2)
                .all(|w| w[0].start_beat < w[1].start_beat),
            "the scale run plays melodically, not as a block"
        );
        assert!(seq.label.contains("Major"), "label: {}", seq.label);
    }

    /// #349 T2b AC1: the grading target carries expected ChordReadings —
    /// a 12-key stacked C7 drill deals 12 chord targets in play order,
    /// segments mirroring the tones' chord_group, bass undemanded at root
    /// position. Fails if targets drift from the dealt cells.
    #[test]
    fn a_stacked_drill_carries_its_chord_targets() {
        let seq = generate(&stacked_spec(ChordType::Dominant7, 0), 7);
        assert_eq!(seq.chord_targets.len(), 12);
        for (i, t) in seq.chord_targets.iter().enumerate() {
            assert_eq!(t.segment, i as u32);
            assert_eq!(t.root_pc, seq.root_order[i] % 12, "target {i}");
            assert_eq!(t.quality, theory::ChordQuality::Dom7);
            assert_eq!(t.bass_pc, None, "root position demands no bass");
            // The target names exactly the cell dealt at this segment.
            let cell: Vec<_> = seq
                .notes
                .iter()
                .filter(|n| n.chord_group == Some(t.segment))
                .collect();
            assert_eq!(cell.len(), 4);
            assert!(cell.iter().any(|n| n.midi % 12 == t.root_pc));
        }
    }

    /// An INVERSION drill demands its voicing's bass: the target's bass_pc
    /// is the stack's lowest tone (the 3rd, for first inversion). Fails if
    /// inversion drills stop judging the bass (spec AC2 tail).
    #[test]
    fn an_inversion_drill_demands_the_voiced_bass() {
        let spec = VariationSpec {
            roots: vec![60],
            ..stacked_spec(ChordType::MajorTriad, 1)
        };
        let seq = generate(&spec, 1);
        assert_eq!(seq.chord_targets.len(), 1);
        let t = seq.chord_targets[0];
        let lowest = seq.notes.iter().map(|n| n.midi).min().unwrap();
        assert_eq!(t.bass_pc, Some(lowest % 12));
        assert_ne!(t.bass_pc, Some(t.root_pc), "first inversion: bass != root");
    }

    /// An OVERSIZE inversion wraps to root position (pinned generator
    /// behavior) — and must therefore demand NO bass: a nominally
    /// "inverted" drill that actually deals root position must not grade
    /// harsher than it plays (review-round nit).
    #[test]
    fn a_wrapped_inversion_demands_no_bass() {
        let spec = VariationSpec {
            roots: vec![60],
            ..stacked_spec(ChordType::MajorTriad, 3) // 3 % 3 = root position
        };
        let seq = generate(&spec, 1);
        assert_eq!(seq.chord_targets[0].bass_pc, None);
        // And the voicing really is root position.
        assert!(seq.notes[0].is_root);
    }

    /// #349 T3c AC: a lifted ii–V–I rows through 12 keys — each key deals
    /// its three chords as consecutive stacked cells (one per measure),
    /// re-rooted by the step offsets, with grading targets to match. Fails
    /// if the re-rooting, grid, grouping, or targets drift.
    #[test]
    fn a_progression_rows_through_twelve_keys() {
        let spec = VariationSpec {
            progression: Some(vec![
                ProgressionStep {
                    offset: 2,
                    chord: ChordType::Minor7,
                }, // ii
                ProgressionStep {
                    offset: 7,
                    chord: ChordType::Dominant7,
                }, // V
                ProgressionStep {
                    offset: 0,
                    chord: ChordType::Major7,
                }, // I
            ]),
            roots: chromatic_roots(),
            ..base_spec_no_scale()
        };
        let seq = generate(&spec, 7);
        assert_eq!(seq.chord_targets.len(), 36, "12 keys x 3 chords");
        assert_eq!(seq.notes.len(), 36 * 4, "4 tones per 7th chord");
        for (k, &key_root) in seq.root_order.iter().enumerate() {
            for (j, (off, q)) in [
                (2u8, theory::ChordQuality::Min7),
                (7, theory::ChordQuality::Dom7),
                (0, theory::ChordQuality::Maj7),
            ]
            .iter()
            .enumerate()
            {
                let t = seq.chord_targets[k * 3 + j];
                assert_eq!(t.root_pc, (key_root + off) % 12, "key {k} step {j}");
                assert_eq!(t.quality, *q);
                assert_eq!(t.segment as usize, k * 3 + j);
                // The dealt cell sits in its own measure on the RV grid.
                let cell: Vec<_> = seq
                    .notes
                    .iter()
                    .filter(|n| n.chord_group == Some(t.segment))
                    .collect();
                assert_eq!(cell.len(), 4);
                let downbeat = (t.segment as usize * 4) as f64;
                assert!(cell.iter().all(|n| n.start_beat == downbeat));
                assert_eq!(cell.iter().filter(|n| n.is_root).count(), 1);
            }
        }
        assert!(
            seq.label.contains("your progression"),
            "label: {}",
            seq.label
        );
    }

    /// Precedence: the player's CELL still shadows a progression, and a
    /// progression shadows scale/chord catalog figures. Wire-additive: old
    /// specs without the field parse.
    #[test]
    fn progression_precedence_and_wire_compat() {
        let steps = vec![ProgressionStep {
            offset: 0,
            chord: ChordType::MajorTriad,
        }];
        let spec = VariationSpec {
            cell: Some(vec![0, 4]),
            progression: Some(steps.clone()),
            ..base_spec_no_scale()
        };
        let seq = generate(&spec, 1);
        assert!(seq.chord_targets.is_empty(), "cell wins");

        let spec = VariationSpec {
            progression: Some(steps),
            scale: Some(ScaleModifier {
                scale: ScaleType::Major,
                pattern: ScalePattern::Up,
            }),
            ..base_spec_no_scale()
        };
        let seq = generate(&spec, 1);
        assert!(
            !seq.chord_targets.is_empty(),
            "progression outranks the scale"
        );

        let old_wire = r#"{"roots":[60],"scale":null,"chord":null,"interval":null,"enclosure":null,"direction":"forward","rhythm":{"notes_per_beat":2,"tempo_bpm":80.0,"rest_beats_between_roots":1.0},"randomize_roots":false}"#;
        let parsed: VariationSpec = serde_json::from_str(old_wire).expect("old wire parses");
        assert!(parsed.progression.is_none());
    }

    fn base_spec_no_scale() -> VariationSpec {
        VariationSpec {
            scale: None,
            ..base_spec()
        }
    }

    /// Melodic material carries NO chord targets, and pre-T2b sequences
    /// (no chord_targets on the wire) still parse.
    #[test]
    fn melodic_sequences_have_no_targets_and_old_wire_parses() {
        let seq = generate(&base_spec(), 1);
        assert!(seq.chord_targets.is_empty());
        let mut json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&seq).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("chord_targets");
        let back: GeneratedSequence = serde_json::from_value(json).expect("old wire parses");
        assert!(back.chord_targets.is_empty());
    }

    /// Stacks respect the playable range: a root at the very top of the
    /// range (MIDI 96 is legal) folds its voicing down instead of emitting
    /// tones above C7. Fails if the stacked branch skips fold_into_range.
    #[test]
    fn a_stacked_voicing_at_the_range_top_folds_down() {
        let spec = VariationSpec {
            roots: vec![MIDI_MAX],
            ..stacked_spec(ChordType::Dominant7, 0)
        };
        let seq = generate(&spec, 1);
        assert_eq!(seq.notes.len(), 4);
        for n in &seq.notes {
            assert!(
                (MIDI_MIN..=MIDI_MAX).contains(&n.midi),
                "tone {} escapes the playable range",
                n.midi
            );
        }
    }
}
