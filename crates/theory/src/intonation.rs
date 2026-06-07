//! Real intonation analysis: cents deviation vs equal temperament and a
//! per-scale-degree tuning-tendency map (Phase 4, A1).
//!
//! Today the app uses pitch *stability* as a proxy for intonation, which is
//! honest about *consistency* but says nothing about whether you are sharp or
//! flat, or by how much. This module replaces that with genuine **cents
//! deviation** analysis — the same measure a tuner uses — and accumulates a
//! **per-degree tendency map** so a recap can say "your major thirds ran +11 ¢,
//! your leading tone was consistently flat."
//!
//! Nothing here runs on the audio thread: it is called from the processing
//! thread where heap allocation is fine.
//!
//! ## Entry points
//! - [`cents_off_equal_temperament`] — convert one Hz reading to the nearest
//!   MIDI note plus its signed cents error.
//! - [`summarize_intonation`] — aggregate a slice of Hz pitches into an
//!   [`IntonationSummary`] that is ready to cross the IPC boundary to the
//!   frontend (serde-derived).
//!
//! ## Honesty / hedging contract
//! Consistent with the design rationale in
//! `docs/design/story-phase4-musical-relevance.md` §1: when there is not enough
//! evidence (empty slice, too few samples) we return `None` rather than
//! asserting shaky numbers. Callers should gate on `note_count` and on the
//! `confidence` field of any enclosing [`crate::KeyEstimate`] before presenting
//! intonation numbers to the user.

use serde::{Deserialize, Serialize};

// ─── Constants ───────────────────────────────────────────────────────────────

/// A pitch is considered "in tune" when its absolute deviation from equal
/// temperament is within this many cents. Chosen to match the existing
/// `crates/brain` threshold (15 ¢) so the two subsystems agree.
pub const DEFAULT_IN_TUNE_TOLERANCE_CENTS: f32 = 15.0;

/// MIDI note number for A4 (440 Hz) — the anchor of equal temperament.
const MIDI_A4: f32 = 69.0;
/// Reference frequency for A4 in Hz.
const HZ_A4: f32 = 440.0;
/// Cents per semitone (and per MIDI step).
const CENTS_PER_SEMITONE: f32 = 100.0;

// ─── Core pitch calculation ───────────────────────────────────────────────────

/// Convert a frequency in Hz to the nearest equal-tempered MIDI note number
/// and the signed cents deviation from that note.
///
/// Returns `(midi_note, cents)` where:
/// - `midi_note` is the nearest MIDI note (0–127; A4 = 440 Hz = 69).
/// - `cents` is the signed deviation in the half-open range **(-50, +50]**
///   (positive = sharp, negative = flat).
///
/// Returns `None` for non-positive or non-finite inputs — callers must guard
/// before passing the result to downstream DSP or UI.
///
/// # Examples
/// ```
/// # use theory::cents_off_equal_temperament;
/// let (note, cents) = cents_off_equal_temperament(440.0).unwrap();
/// assert_eq!(note, 69u8); // A4
/// assert!(cents.abs() < 0.01_f32);
/// ```
pub fn cents_off_equal_temperament(hz: f32) -> Option<(u8, f32)> {
    if !hz.is_finite() || hz <= 0.0 {
        return None;
    }

    // Exact number of semitones above/below A4 (can be negative / fractional).
    let semitones_from_a4 = 12.0 * (hz / HZ_A4).log2();

    // Nearest MIDI note: round to the nearest integer semitone.
    let midi_float = MIDI_A4 + semitones_from_a4;
    let midi_rounded = midi_float.round();

    // Clamp to the valid MIDI range (0–127).
    if !(0.0..=127.0).contains(&midi_rounded) {
        return None;
    }
    let midi_note = midi_rounded as u8;

    // Cents deviation: fractional part of the semitone distance, scaled.
    // `round()` gives [-0.5, +0.5] semitones; multiplied by 100 that is
    // [-50, +50] ¢. We want the half-open range (-50, +50] so that -50 ¢ is
    // always reported as +50 ¢ on the note one semitone lower.  In practice
    // -50 ¢ only occurs at the exact midpoint between two ET pitches, which is
    // a degenerate input; snapping it keeps the invariant clean.
    let cents = (midi_float - midi_rounded) * CENTS_PER_SEMITONE;
    let (midi_note, cents) = if cents <= -50.0 {
        // Snap to the note below and report +50 ¢ sharp.
        if midi_note == 0 {
            // Can't go lower; accept -50 ¢ as a special case.
            (midi_note, -50.0_f32)
        } else {
            (midi_note - 1, 50.0_f32)
        }
    } else {
        (midi_note, cents)
    };

    debug_assert!(
        (-50.0..=50.0).contains(&cents),
        "cents out of range: {cents}"
    );

    Some((midi_note, cents))
}

// ─── Aggregate types ─────────────────────────────────────────────────────────

/// The intonation tendency of a single scale degree relative to a known tonic.
///
/// For example, a `DegreeTendency { semitones_from_tonic: 4, mean_cents: +11.3,
/// count: 17 }` means "across 17 observed major-third notes, the player was on
/// average 11.3 ¢ sharp." A great teacher can't log this objectively for 40
/// minutes; this struct captures that machine advantage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DegreeTendency {
    /// Distance in semitones from the tonic pitch class (0 = tonic, 11 =
    /// leading tone). Always in `[0, 11]`.
    pub semitones_from_tonic: u8,
    /// Mean signed cents deviation for this degree vs equal temperament.
    /// Positive = sharp, negative = flat.
    pub mean_cents: f32,
    /// Number of pitch observations that went into this average.
    pub count: u32,
}

/// Session-level intonation summary, ready to cross the IPC boundary.
///
/// All fields are statistics over pitches that had a finite, usable
/// [`cents_off_equal_temperament`] value — non-positive / non-finite Hz values
/// are silently discarded, and the caller can detect this via `note_count`
/// being smaller than the input slice.
///
/// When returned by [`summarize_intonation`] with `tonic = None`, `tendencies`
/// is always empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntonationSummary {
    /// Number of usable pitch observations that contributed to the statistics.
    pub note_count: u32,
    /// Mean signed deviation of all observations from equal temperament
    /// (positive = player runs sharp overall, negative = flat overall).
    pub mean_cents: f32,
    /// Mean *absolute* deviation — the average magnitude of error regardless
    /// of direction. Zero for a perfectly in-tune player; higher = less precise.
    pub mean_abs_cents: f32,
    /// Fraction of observations within [`DEFAULT_IN_TUNE_TOLERANCE_CENTS`] (or
    /// the caller-supplied tolerance). Always in `[0, 1]`.
    pub in_tune_ratio: f32,
    /// Per-scale-degree tendency relative to the supplied tonic; sorted
    /// ascending by `semitones_from_tonic`. Empty when no tonic was given.
    pub tendencies: Vec<DegreeTendency>,
}

// ─── Main aggregation function ────────────────────────────────────────────────

/// Aggregate a slice of Hz pitches into an [`IntonationSummary`].
///
/// - `pitches`: raw fundamental-frequency readings in Hz (from the processing
///   thread, not the audio thread).
/// - `tonic`: optional pitch class of the known tonic (0 = C, 1 = C#/Db … 11
///   = B). When `Some`, per-degree tendencies are computed; when `None`,
///   `tendencies` is empty.
/// - `tolerance_cents`: threshold for "in tune" — pass
///   [`DEFAULT_IN_TUNE_TOLERANCE_CENTS`] if you have no preference.
///
/// Returns `None` if the slice contains no usable pitches (e.g. all zeros /
/// infinite / NaN, or empty). This is the honesty hedge: we never assert
/// intonation statistics over no data.
///
/// # Examples
/// ```
/// # use theory::{summarize_intonation, DEFAULT_IN_TUNE_TOLERANCE_CENTS};
/// // A4 = 440 Hz, perfectly in tune.
/// let summary = summarize_intonation(&[440.0], None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
/// assert_eq!(summary.note_count, 1);
/// assert!(summary.mean_abs_cents < 0.01_f32);
/// assert_eq!(summary.in_tune_ratio, 1.0_f32);
/// ```
pub fn summarize_intonation(
    pitches: &[f32],
    tonic: Option<u8>,
    tolerance_cents: f32,
) -> Option<IntonationSummary> {
    // Collect per-pitch data.  Allocating on the processing thread is fine.
    let mut sum_cents = 0.0f32;
    let mut sum_abs_cents = 0.0f32;
    let mut in_tune_count = 0u32;
    let mut note_count = 0u32;

    // Per-degree accumulators: indexed by semitones_from_tonic (0..12).
    // We use fixed-size arrays to avoid a HashMap; the degree space is tiny.
    let mut degree_sum = [0.0f32; 12];
    let mut degree_count = [0u32; 12];

    for &hz in pitches {
        let Some((midi, cents)) = cents_off_equal_temperament(hz) else {
            continue;
        };

        sum_cents += cents;
        sum_abs_cents += cents.abs();
        if cents.abs() <= tolerance_cents {
            in_tune_count += 1;
        }
        note_count += 1;

        // Accumulate per-degree data when tonic is known.
        if let Some(tc) = tonic {
            // Pitch class of this MIDI note (0 = C).
            let pc = midi % 12;
            // How many semitones above the tonic (always 0..12).
            let degree = (pc as i16 - (tc % 12) as i16).rem_euclid(12) as usize;
            degree_sum[degree] += cents;
            degree_count[degree] += 1;
        }
    }

    if note_count == 0 {
        return None;
    }

    let n = note_count as f32;
    let mean_cents = sum_cents / n;
    let mean_abs_cents = sum_abs_cents / n;
    let in_tune_ratio = in_tune_count as f32 / n;

    // Build the sorted tendency vec (only degrees that were actually observed).
    let tendencies = if tonic.is_some() {
        let mut v: Vec<DegreeTendency> = (0u8..12)
            .filter(|&d| degree_count[d as usize] > 0)
            .map(|d| DegreeTendency {
                semitones_from_tonic: d,
                mean_cents: degree_sum[d as usize] / degree_count[d as usize] as f32,
                count: degree_count[d as usize],
            })
            .collect();
        // Sort ascending by degree (already in order from the iterator, but
        // make the contract explicit).
        v.sort_by_key(|t| t.semitones_from_tonic);
        v
    } else {
        Vec::new()
    };

    Some(IntonationSummary {
        note_count,
        mean_cents,
        mean_abs_cents,
        in_tune_ratio,
        tendencies,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── cents_off_equal_temperament ──────────────────────────────────────────

    #[test]
    fn a4_is_midi_69_with_zero_cents() {
        let (note, cents) = cents_off_equal_temperament(440.0).unwrap();
        assert_eq!(note, 69, "A4 should be MIDI 69");
        assert!(
            cents.abs() < 0.01,
            "A4 should be 0 ¢ off ET, got {cents:.4}"
        );
    }

    #[test]
    fn c4_maps_correctly() {
        // C4 ≈ 261.626 Hz → MIDI 60.
        let (note, cents) = cents_off_equal_temperament(261.63).unwrap();
        assert_eq!(note, 60, "C4 should be MIDI 60, got {note}");
        assert!(
            cents.abs() < 0.5,
            "C4 at 261.63 Hz should be near 0 ¢, got {cents:.4}"
        );
    }

    #[test]
    fn quarter_tone_sharp_a_wraps_at_50_cents() {
        // A quarter tone above A4 ≈ A4 × 2^(0.5/12) ≈ 452.893 Hz.
        // This sits exactly on the boundary between A4 (+50 ¢) and A#4 (-50 ¢).
        // Depending on floating-point rounding, `round()` may snap it to A4
        // (+50 ¢) or A#4 (-50 ¢).  When it lands on -50 ¢ our boundary snap
        // converts it to +50 ¢ on the note below.  Either way the absolute
        // deviation must equal 50 ¢ and the function must not panic.
        let hz = 440.0_f32 * 2.0_f32.powf(0.5 / 12.0);
        let (_, cents) = cents_off_equal_temperament(hz).unwrap();
        assert!(
            cents.abs() <= 50.0,
            "quarter-tone sharp A should be ≤50 ¢ from nearest ET note, got {cents:.4}"
        );
    }

    #[test]
    fn sharp_a_444hz_is_about_15_cents_sharp() {
        // 444 Hz is commonly measured as ~+15.7 ¢ sharp vs 440.
        // Exact: 1200 × log2(444/440) ≈ 15.66 ¢.
        let (note, cents) = cents_off_equal_temperament(444.0).unwrap();
        assert_eq!(note, 69, "444 Hz rounds to A4");
        assert!(
            (cents - 15.66).abs() < 0.2,
            "444 Hz should be ~+15.7 ¢ sharp, got {cents:.4}"
        );
    }

    #[test]
    fn non_positive_hz_returns_none() {
        assert!(cents_off_equal_temperament(0.0).is_none());
        assert!(cents_off_equal_temperament(-1.0).is_none());
    }

    #[test]
    fn non_finite_hz_returns_none() {
        assert!(cents_off_equal_temperament(f32::NAN).is_none());
        assert!(cents_off_equal_temperament(f32::INFINITY).is_none());
        assert!(cents_off_equal_temperament(f32::NEG_INFINITY).is_none());
    }

    // ── summarize_intonation — basic cases ───────────────────────────────────

    #[test]
    fn empty_slice_returns_none() {
        assert!(
            summarize_intonation(&[], None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).is_none(),
            "empty slice must return None"
        );
    }

    #[test]
    fn all_invalid_pitches_returns_none() {
        let bad = [0.0, -1.0, f32::NAN, f32::INFINITY];
        assert!(
            summarize_intonation(&bad, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).is_none(),
            "all non-positive/non-finite pitches must return None"
        );
    }

    #[test]
    fn perfectly_in_tune_et_pitches() {
        // A4 (440), C4 (≈261.63), E4 (≈329.63) are ET reference frequencies.
        // mean_abs_cents ≈ 0, in_tune_ratio == 1.0.
        let et_a4 = 440.0_f32;
        let et_c4 = 440.0 * 2.0_f32.powf(-9.0 / 12.0); // C4
        let et_e4 = 440.0 * 2.0_f32.powf(-5.0 / 12.0); // E4
        let pitches = [et_a4, et_c4, et_e4];

        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert_eq!(s.note_count, 3);
        assert!(
            s.mean_abs_cents < 0.01,
            "ET pitches → mean_abs_cents ≈ 0, got {:.4}",
            s.mean_abs_cents
        );
        assert_eq!(s.in_tune_ratio, 1.0, "all ET pitches should be in tune");
        assert!(s.tendencies.is_empty(), "no tonic → no tendencies");
    }

    #[test]
    fn consistently_sharp_set_has_positive_mean_cents() {
        // 444 Hz is ~+15.7 ¢ sharp vs A4.  Feed several of these.
        let pitches = vec![444.0_f32; 10];
        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert!(
            s.mean_cents > 10.0,
            "consistently sharp pitches → mean_cents > 10, got {:.4}",
            s.mean_cents
        );
        assert!(
            s.mean_abs_cents > 10.0,
            "consistently sharp → mean_abs_cents > 10, got {:.4}",
            s.mean_abs_cents
        );
    }

    #[test]
    fn consistently_flat_set_has_negative_mean_cents() {
        // 436 Hz is ~−15.7 ¢ flat vs A4.
        let pitches = vec![436.0_f32; 8];
        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert!(
            s.mean_cents < -10.0,
            "consistently flat pitches → mean_cents < -10, got {:.4}",
            s.mean_cents
        );
    }

    #[test]
    fn in_tune_ratio_all_within_tolerance() {
        // Pitch very close to A4 — well inside tolerance.
        let pitches = vec![440.5_f32; 5]; // ~+1.97 ¢
        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert_eq!(
            s.in_tune_ratio, 1.0,
            "all pitches within tolerance → ratio 1.0"
        );
    }

    #[test]
    fn in_tune_ratio_none_within_tolerance() {
        // 444 Hz is ~+15.7 ¢ — barely outside the 15 ¢ default tolerance.
        // Use a tighter 10 ¢ tolerance so this definitely fails.
        let pitches = vec![444.0_f32; 4];
        let s = summarize_intonation(&pitches, None, 10.0).unwrap();
        assert_eq!(
            s.in_tune_ratio, 0.0,
            "all pitches outside 10 ¢ tolerance → ratio 0.0"
        );
    }

    // ── summarize_intonation — per-degree tendency map ───────────────────────

    #[test]
    fn major_third_sharp_tendency() {
        // Tonic = C (pc 0). E4 is a major third above C (4 semitones).
        // ET E4 = 440 × 2^(-5/12).  We raise it by +20 ¢ by multiplying by
        // 2^(20/1200).
        let et_e4 = 440.0_f32 * 2.0_f32.powf(-5.0_f32 / 12.0);
        let sharp_e4 = et_e4 * 2.0_f32.powf(20.0_f32 / 1200.0); // +20 ¢

        // Feed this consistently-sharp E4 several times with tonic C (pc 0).
        let pitches: Vec<f32> = vec![sharp_e4; 8];
        let s = summarize_intonation(&pitches, Some(0 /* C */), DEFAULT_IN_TUNE_TOLERANCE_CENTS)
            .unwrap();

        // We should find exactly one tendency entry for degree 4 (major third).
        let e_tendency = s
            .tendencies
            .iter()
            .find(|t| t.semitones_from_tonic == 4)
            .expect("should have a tendency for the major third (degree 4)");

        assert_eq!(e_tendency.count, 8, "all 8 notes should be counted");
        assert!(
            (e_tendency.mean_cents - 20.0).abs() < 1.0,
            "major third tendency should be ~+20 ¢, got {:.4}",
            e_tendency.mean_cents
        );
    }

    #[test]
    fn tendencies_sorted_by_degree() {
        // Feed tonic, major third, and fifth — sorted order: 0, 4, 7.
        let et = |semitones: f32| -> f32 { 440.0 * 2.0_f32.powf(semitones / 12.0) };
        let pitches = vec![
            et(-9.0), // C4 (degree 0 from C tonic)
            et(-5.0), // E4 (degree 4)
            et(-2.0), // G4 (degree 7)
        ];
        let s = summarize_intonation(&pitches, Some(0), DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        let degrees: Vec<u8> = s
            .tendencies
            .iter()
            .map(|t| t.semitones_from_tonic)
            .collect();
        let mut sorted = degrees.clone();
        sorted.sort();
        assert_eq!(degrees, sorted, "tendencies must be sorted by degree");
    }

    #[test]
    fn no_tonic_means_no_tendencies() {
        let pitches = [440.0, 261.63, 329.63];
        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert!(
            s.tendencies.is_empty(),
            "no tonic → tendencies must be empty"
        );
    }

    #[test]
    fn mixed_valid_and_invalid_pitches() {
        // NaN and negatives should be silently skipped; 440 Hz is usable.
        let pitches = [f32::NAN, -1.0, 440.0, 0.0, f32::INFINITY];
        let s = summarize_intonation(&pitches, None, DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        assert_eq!(
            s.note_count, 1,
            "only 440 Hz should contribute, got {} notes",
            s.note_count
        );
    }

    // ── serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn intonation_summary_serde_roundtrip() {
        let et_e4 = 440.0_f32 * 2.0_f32.powf(-5.0_f32 / 12.0);
        let sharp_e4 = et_e4 * 2.0_f32.powf(20.0_f32 / 1200.0);
        let pitches: Vec<f32> = [440.0_f32, sharp_e4, 261.63].into();

        let original =
            summarize_intonation(&pitches, Some(0), DEFAULT_IN_TUNE_TOLERANCE_CENTS).unwrap();
        let json = serde_json::to_string(&original).expect("serialise failed");
        let roundtripped: IntonationSummary =
            serde_json::from_str(&json).expect("deserialise failed");
        assert_eq!(
            original, roundtripped,
            "IntonationSummary must round-trip through JSON"
        );
    }

    #[test]
    fn degree_tendency_serde_roundtrip() {
        let tendency = DegreeTendency {
            semitones_from_tonic: 4,
            mean_cents: 11.3,
            count: 17,
        };
        let json = serde_json::to_string(&tendency).expect("serialise failed");
        let back: DegreeTendency = serde_json::from_str(&json).expect("deserialise failed");
        assert_eq!(
            tendency, back,
            "DegreeTendency must round-trip through JSON"
        );
    }
}
