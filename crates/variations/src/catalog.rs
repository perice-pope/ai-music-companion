//! The RV catalog: interval tables for scales, chords, and enclosures.
//!
//! Ported from Random Variations' data files (`scales.json`, `chords.json`,
//! `enclosures.json` in `perice-pope/random-variations`) — trimmed to the set
//! the practice coach draws from today. Each table is semitone offsets from the
//! root; expansion into playable figures lives in the generator. Adding an
//! entry here needs no generator changes.

use serde::{Deserialize, Serialize};

/// Scales the generator can expand. Ordered roughly easy → exotic; the coach's
/// difficulty ladder leans on that ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleType {
    Major,
    NaturalMinor,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    Dorian,
    Mixolydian,
    Lydian,
    Phrygian,
    HarmonicMinor,
    MelodicMinor,
    Locrian,
    Chromatic,
}

impl ScaleType {
    /// Semitone offsets from the root, one octave, root included.
    pub fn semitones(self) -> &'static [u8] {
        match self {
            ScaleType::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleType::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleType::MajorPentatonic => &[0, 2, 4, 7, 9],
            ScaleType::MinorPentatonic => &[0, 3, 5, 7, 10],
            ScaleType::Blues => &[0, 3, 5, 6, 7, 10],
            ScaleType::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            ScaleType::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            ScaleType::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            ScaleType::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            ScaleType::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleType::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            ScaleType::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            ScaleType::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    /// Human label, e.g. `"Dorian"`.
    pub fn label(self) -> &'static str {
        match self {
            ScaleType::Major => "Major",
            ScaleType::NaturalMinor => "Natural Minor",
            ScaleType::MajorPentatonic => "Major Pentatonic",
            ScaleType::MinorPentatonic => "Minor Pentatonic",
            ScaleType::Blues => "Blues",
            ScaleType::Dorian => "Dorian",
            ScaleType::Mixolydian => "Mixolydian",
            ScaleType::Lydian => "Lydian",
            ScaleType::Phrygian => "Phrygian",
            ScaleType::HarmonicMinor => "Harmonic Minor",
            ScaleType::MelodicMinor => "Melodic Minor",
            ScaleType::Locrian => "Locrian",
            ScaleType::Chromatic => "Chromatic",
        }
    }
}

/// Chords the generator can arpeggiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordType {
    MajorTriad,
    MinorTriad,
    DiminishedTriad,
    AugmentedTriad,
    Sus4Triad,
    Major7,
    Minor7,
    Dominant7,
    HalfDiminished7,
    /// #349 T4a: sus2 triad — heard by the T1 engine, rowable from the lane.
    Sus2Triad,
    /// #349 T4a: full diminished 7 — same reason.
    Diminished7,
    /// #349 T4a review S4: 7sus4 rows AS a sus voicing — reducing it to a
    /// plain dominant would reintroduce the major 3rd the voicing exists
    /// to avoid.
    Dominant7Sus4,
}

impl ChordType {
    /// #349 T2b: the `theory` quality the T1 chord engine reports when this
    /// chord is heard — the grading vocabulary bridge. Every variant maps;
    /// a new ChordType without a mapping is a compile error here, never a
    /// silent misgrade.
    pub fn quality(self) -> theory::ChordQuality {
        match self {
            ChordType::MajorTriad => theory::ChordQuality::Maj,
            ChordType::MinorTriad => theory::ChordQuality::Min,
            ChordType::DiminishedTriad => theory::ChordQuality::Dim,
            ChordType::AugmentedTriad => theory::ChordQuality::Aug,
            ChordType::Sus4Triad => theory::ChordQuality::Sus4,
            ChordType::Major7 => theory::ChordQuality::Maj7,
            ChordType::Minor7 => theory::ChordQuality::Min7,
            ChordType::Dominant7 => theory::ChordQuality::Dom7,
            ChordType::HalfDiminished7 => theory::ChordQuality::Min7b5,
            ChordType::Sus2Triad => theory::ChordQuality::Sus2,
            ChordType::Diminished7 => theory::ChordQuality::Dim7,
            ChordType::Dominant7Sus4 => theory::ChordQuality::Dom7Sus4,
        }
    }

    /// Semitone offsets from the root, root position.
    pub fn semitones(self) -> &'static [u8] {
        match self {
            ChordType::MajorTriad => &[0, 4, 7],
            ChordType::MinorTriad => &[0, 3, 7],
            ChordType::DiminishedTriad => &[0, 3, 6],
            ChordType::AugmentedTriad => &[0, 4, 8],
            ChordType::Sus4Triad => &[0, 5, 7],
            ChordType::Major7 => &[0, 4, 7, 11],
            ChordType::Minor7 => &[0, 3, 7, 10],
            ChordType::Dominant7 => &[0, 4, 7, 10],
            ChordType::HalfDiminished7 => &[0, 3, 6, 10],
            ChordType::Sus2Triad => &[0, 2, 7],
            ChordType::Diminished7 => &[0, 3, 6, 9],
            ChordType::Dominant7Sus4 => &[0, 5, 7, 10],
        }
    }

    /// Human label, e.g. `"Minor 7"`.
    pub fn label(self) -> &'static str {
        match self {
            ChordType::MajorTriad => "Major Triad",
            ChordType::MinorTriad => "Minor Triad",
            ChordType::DiminishedTriad => "Diminished Triad",
            ChordType::AugmentedTriad => "Augmented Triad",
            ChordType::Sus4Triad => "Sus4 Triad",
            ChordType::Major7 => "Major 7",
            ChordType::Minor7 => "Minor 7",
            ChordType::Dominant7 => "Dominant 7",
            ChordType::HalfDiminished7 => "Half-Diminished 7",
            ChordType::Sus2Triad => "Sus2 Triad",
            ChordType::Diminished7 => "Diminished 7",
            ChordType::Dominant7Sus4 => "Dominant 7 Sus4",
        }
    }
}

/// Jazz approach-note enclosures: chromatic offsets played *before* the target,
/// in order. Ported from RV's `enclosures.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enclosure {
    OneUp,
    OneDown,
    TwoUp,
    TwoDown,
    OneUpOneDown,
    OneDownOneUp,
}

impl Enclosure {
    /// Semitone offsets (relative to the target) of the approach notes, in
    /// playing order; the target itself follows.
    pub fn approach_semitones(self) -> &'static [i8] {
        match self {
            Enclosure::OneUp => &[1],
            Enclosure::OneDown => &[-1],
            Enclosure::TwoUp => &[2, 1],
            Enclosure::TwoDown => &[-2, -1],
            Enclosure::OneUpOneDown => &[1, -1],
            Enclosure::OneDownOneUp => &[-1, 1],
        }
    }

    /// Human label, e.g. `"one up, one down"`.
    pub fn label(self) -> &'static str {
        match self {
            Enclosure::OneUp => "one up",
            Enclosure::OneDown => "one down",
            Enclosure::TwoUp => "two up",
            Enclosure::TwoDown => "two down",
            Enclosure::OneUpOneDown => "one up, one down",
            Enclosure::OneDownOneUp => "one down, one up",
        }
    }
}

/// Rhythm cells (#421 S3): small repeating duration figures a melodic row can
/// be re-timed with — same pitches, new rhythm. The classic subdivision
/// figures from RV's meter/division language; each cycles per root so every
/// key drills the identical rhythm. Adding an entry here needs no generator
/// changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhythmCell {
    DottedEighthSixteenth,
    SixteenthDottedEighth,
    EighthTwoSixteenths,
    TwoSixteenthsEighth,
    SwungEighths,
}

/// Swung-eighth pair as exact-at-DIVISIONS f64 thirds (2/3 + 1/3 of a beat).
const SWUNG_PAIR: [f64; 2] = [2.0 / 3.0, 1.0 / 3.0];

impl RhythmCell {
    /// Note durations in beats, in playing order, cycled over the figure.
    /// Every duration is > 0; every cell sums to one beat.
    pub fn durations(self) -> &'static [f64] {
        match self {
            RhythmCell::DottedEighthSixteenth => &[0.75, 0.25],
            RhythmCell::SixteenthDottedEighth => &[0.25, 0.75],
            RhythmCell::EighthTwoSixteenths => &[0.5, 0.25, 0.25],
            RhythmCell::TwoSixteenthsEighth => &[0.25, 0.25, 0.5],
            RhythmCell::SwungEighths => &SWUNG_PAIR,
        }
    }

    /// Human label, e.g. `"dotted eighth + sixteenth"`.
    pub fn label(self) -> &'static str {
        match self {
            RhythmCell::DottedEighthSixteenth => "dotted eighth + sixteenth",
            RhythmCell::SixteenthDottedEighth => "sixteenth + dotted eighth",
            RhythmCell::EighthTwoSixteenths => "eighth + two sixteenths",
            RhythmCell::TwoSixteenthsEighth => "two sixteenths + eighth",
            RhythmCell::SwungEighths => "swung eighths",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #421 S3 (review should-fix): the generator's barline safety rides on
    /// every rhythm cell honoring three invariants — non-empty, all
    /// durations > 0, and every running prefix ≤ 1 with the full cycle
    /// summing to exactly one beat. "Adding an entry needs no generator
    /// changes" is only true under them: a cell like [1.5, 0.5] would deal a
    /// note across a barline, which `push_span` splits into two UNTIED notes
    /// — silently changing what the player is asked to play. Fails on the
    /// first catalog entry that breaks the contract.
    #[test]
    fn every_rhythm_cell_is_barline_safe() {
        const ALL: [RhythmCell; 5] = [
            RhythmCell::DottedEighthSixteenth,
            RhythmCell::SixteenthDottedEighth,
            RhythmCell::EighthTwoSixteenths,
            RhythmCell::TwoSixteenthsEighth,
            RhythmCell::SwungEighths,
        ];
        for cell in ALL {
            let d = cell.durations();
            assert!(!d.is_empty(), "{cell:?} has durations");
            let mut prefix = 0.0_f64;
            for &v in d {
                assert!(v > 0.0, "{cell:?} durations all positive");
                prefix += v;
                assert!(prefix <= 1.0 + 1e-9, "{cell:?} prefix crosses a beat");
            }
            assert!(
                (prefix - 1.0).abs() < 1e-9,
                "{cell:?} cycle sums to one beat, got {prefix}"
            );
            assert!(!cell.label().is_empty(), "{cell:?} has a label");
        }
    }

    /// #349 T2b (review must-fix): every ChordType's grading quality is
    /// interval-identical to the tones it deals — a divergent mapping
    /// systematically misgrades every drill of that type (each correct
    /// chord would read Near forever). Kills any HalfDim7→Min7-style
    /// mutation permanently.
    #[test]
    fn every_chord_type_grades_as_the_intervals_it_deals() {
        for ct in [
            ChordType::MajorTriad,
            ChordType::MinorTriad,
            ChordType::DiminishedTriad,
            ChordType::AugmentedTriad,
            ChordType::Sus4Triad,
            ChordType::Major7,
            ChordType::Minor7,
            ChordType::Dominant7,
            ChordType::HalfDiminished7,
            ChordType::Sus2Triad,
            ChordType::Diminished7,
            ChordType::Dominant7Sus4,
        ] {
            assert_eq!(
                ct.quality().intervals(),
                ct.semitones(),
                "{ct:?}: dealt tones and graded quality must be the same chord"
            );
        }
    }
}
