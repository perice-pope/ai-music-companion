//! On-device key & mode detection (Phase 4, Track A — `crates/theory`).
//!
//! This is the honest measurement substrate the Cultural Relevance Engine
//! stands on (see `docs/design/story-phase4-musical-relevance.md`): the LLM may
//! only assert a key/mode the DSP actually measured, each carrying a
//! **confidence**, so a trained ear never catches it inventing one.
//!
//! The approach is profile-correlation (Krumhansl–Schmuckler family): accumulate
//! a duration/energy-weighted **pitch-class profile**, then correlate it against
//! a template for every (tonic, mode) pair and take the best fit. Templates
//! encode the tonal hierarchy (tonic > fifth > third > other scale tones >
//! chromatic), which is what lets us tell relative modes apart — C major, A
//! minor and D dorian share seven pitch classes; only the *emphasis* differs.
//!
//! Two entry points:
//! - [`estimate_key`] — one-shot over a whole [`PitchClassProfile`] (e.g. a
//!   session aggregate for the recap).
//! - [`KeyTracker`] — a rolling, decaying estimate with hysteresis, so a live
//!   reading tracks modulation without flickering note-to-note.
//!
//! Nothing here runs on the audio thread: it consumes detected notes on the
//! processing thread, so heap/array work is fine.

use serde::{Deserialize, Serialize};

mod intonation;
mod key;
mod tracker;

pub use intonation::{
    cents_off_equal_temperament, summarize_intonation, DegreeTendency, IntonationSummary,
    DEFAULT_IN_TUNE_TOLERANCE_CENTS,
};
pub use key::{correlation_for, estimate_key, KeyEstimate, PitchClassProfile};
pub use tracker::{KeyTracker, KeyTrackerConfig};

/// Pitch-class names, index 0 = C, using sharps.
pub const PITCH_CLASS_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Name a pitch class (0–11, C = 0). Out-of-range inputs wrap mod 12.
pub fn pitch_class_name(pc: u8) -> &'static str {
    PITCH_CLASS_NAMES[(pc % 12) as usize]
}

/// The seven diatonic (church) modes. `Ionian` is the major scale, `Aeolian`
/// the natural minor; the other five carry the colour that drives a lot of
/// genre identity (mixolydian → blues/rock/Celtic, dorian → modal jazz/funk…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ionian,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Aeolian,
    Locrian,
}

impl Mode {
    /// All seven modes, for iteration during estimation.
    pub const ALL: [Mode; 7] = [
        Mode::Ionian,
        Mode::Dorian,
        Mode::Phrygian,
        Mode::Lydian,
        Mode::Mixolydian,
        Mode::Aeolian,
        Mode::Locrian,
    ];

    /// Semitone offsets of the mode's scale degrees from its tonic.
    pub fn intervals(self) -> [u8; 7] {
        match self {
            Mode::Ionian => [0, 2, 4, 5, 7, 9, 11],
            Mode::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Mode::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Mode::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Mode::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Mode::Aeolian => [0, 2, 3, 5, 7, 8, 10],
            Mode::Locrian => [0, 1, 3, 5, 6, 8, 10],
        }
    }

    /// Human label, using the familiar "major"/"minor" for Ionian/Aeolian and
    /// the mode name otherwise. Drives copy like "G Mixolydian" / "C major".
    pub fn label(self) -> &'static str {
        match self {
            Mode::Ionian => "major",
            Mode::Aeolian => "minor",
            Mode::Dorian => "Dorian",
            Mode::Phrygian => "Phrygian",
            Mode::Lydian => "Lydian",
            Mode::Mixolydian => "Mixolydian",
            Mode::Locrian => "Locrian",
        }
    }
}
