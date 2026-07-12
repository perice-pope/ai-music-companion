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

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert_eq!(
                ct.quality().intervals(),
                ct.semitones(),
                "{ct:?}: dealt tones and graded quality must be the same chord"
            );
        }
    }
}
