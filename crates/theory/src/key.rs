//! Pitch-class profile, mode templates, and the one-shot key estimator.

use serde::{Deserialize, Serialize};

use crate::{pitch_class_name, Mode};

// Tonal-hierarchy weights used to build a mode template. The ordering (tonic >
// fifth > third > other scale tones > chromatic) is what distinguishes relative
// modes that share the same seven pitch classes.
const W_TONIC: f32 = 5.0;
const W_FIFTH: f32 = 3.5;
const W_THIRD: f32 = 3.0;
const W_SCALE: f32 = 2.0;
const W_CHROMATIC: f32 = 0.3;

/// A duration/energy-weighted histogram over the 12 pitch classes (C = 0).
///
/// Feed it the notes you detected (by Hz, MIDI, or pitch class) and hand it to
/// [`estimate_key`]. [`PitchClassProfile::decay`] lets a caller turn it into a
/// rolling window (see [`crate::KeyTracker`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PitchClassProfile {
    weights: [f32; 12],
}

impl PitchClassProfile {
    /// An empty profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `weight` (e.g. note duration in seconds, or energy) to pitch class
    /// `pc` (wraps mod 12). Non-positive weights are ignored.
    pub fn add_pc(&mut self, pc: u8, weight: f32) {
        if weight > 0.0 {
            self.weights[(pc % 12) as usize] += weight;
        }
    }

    /// Add a note by MIDI number (pitch class = `midi % 12`).
    pub fn add_midi(&mut self, midi: u8, weight: f32) {
        self.add_pc(midi % 12, weight);
    }

    /// Add a note by frequency in Hz (rounded to the nearest equal-tempered
    /// pitch class, A4 = 440). Non-positive / non-finite Hz are ignored.
    pub fn add_hz(&mut self, hz: f32, weight: f32) {
        if hz > 0.0 {
            let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
            if midi.is_finite() {
                let pc = (midi as i64).rem_euclid(12) as u8;
                self.add_pc(pc, weight);
            }
        }
    }

    /// Total accumulated weight.
    pub fn total(&self) -> f32 {
        self.weights.iter().sum()
    }

    /// True when nothing has been accumulated.
    pub fn is_empty(&self) -> bool {
        self.total() <= 0.0
    }

    /// How many distinct pitch classes carry (non-trivial) weight. Used to gate
    /// committing to a key — you can't name one from a note or two.
    pub fn distinct(&self) -> usize {
        self.weights.iter().filter(|&&w| w > 1e-6).count()
    }

    /// Multiply all weights by `factor` (in `[0, 1]`) — the rolling-window
    /// decay that lets recent notes dominate.
    pub fn decay(&mut self, factor: f32) {
        for w in &mut self.weights {
            *w *= factor.clamp(0.0, 1.0);
        }
    }

    /// Raw weights (C = index 0).
    pub fn weights(&self) -> &[f32; 12] {
        &self.weights
    }

    /// Weights normalised to sum to 1 (all zeros if empty).
    fn normalized(&self) -> [f32; 12] {
        let total = self.total();
        let mut out = [0.0f32; 12];
        if total > 0.0 {
            for (o, w) in out.iter_mut().zip(self.weights.iter()) {
                *o = w / total;
            }
        }
        out
    }
}

/// A detected key: tonic pitch class, mode, and how trustworthy the call is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KeyEstimate {
    /// Tonic pitch class, 0–11 (C = 0).
    pub tonic: u8,
    /// Detected mode.
    pub mode: Mode,
    /// Best-fit correlation, clamped to `[0, 1]`. Higher = more confident.
    pub confidence: f32,
    /// Separation from the runner-up key (`best − second_best`, ≥ 0). A small
    /// margin means the call is ambiguous even if `confidence` is high.
    pub margin: f32,
}

impl KeyEstimate {
    /// Human label, e.g. `"C major"`, `"A minor"`, `"G Mixolydian"`.
    pub fn name(&self) -> String {
        format!("{} {}", pitch_class_name(self.tonic), self.mode.label())
    }
}

/// Template for a (tonic, mode) pair: the tonal-hierarchy weights for `mode`,
/// rotated so the tonic lands on `tonic`.
fn template(tonic: u8, mode: Mode) -> [f32; 12] {
    let mut base = [W_CHROMATIC; 12];
    let ints = mode.intervals();
    for &iv in &ints {
        base[iv as usize] = W_SCALE;
    }
    // Emphasis: tonic, then the (perfect or diminished) fifth, then the third.
    base[0] = W_TONIC;
    base[ints[4] as usize] = W_FIFTH; // 5th scale degree
    base[ints[2] as usize] = W_THIRD; // 3rd scale degree

    let mut out = [0.0f32; 12];
    for (i, &b) in base.iter().enumerate() {
        out[(i + tonic as usize) % 12] = b;
    }
    out
}

/// Pearson correlation between two 12-bin vectors. Returns 0 when either side
/// is flat (no variance) — e.g. a uniform/atonal profile correlates with nothing.
fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let n = 12.0;
    let mean_a = a.iter().sum::<f32>() / n;
    let mean_b = b.iter().sum::<f32>() / n;
    let (mut num, mut da, mut db) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..12 {
        let x = a[i] - mean_a;
        let y = b[i] - mean_b;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da <= 0.0 || db <= 0.0 {
        return 0.0;
    }
    num / (da.sqrt() * db.sqrt())
}

/// Correlation of `profile` against one specific (tonic, mode) template. Used by
/// the tracker's hysteresis to compare a candidate key against the held one.
pub fn correlation_for(profile: &PitchClassProfile, tonic: u8, mode: Mode) -> f32 {
    pearson(&profile.normalized(), &template(tonic, mode))
}

/// Estimate the best-fit key/mode over the whole profile.
///
/// Returns `None` only for an empty profile. For a flat/atonal profile it
/// returns a low-`confidence`, low-`margin` estimate — callers should gate on
/// those rather than treating any `Some` as a firm key.
pub fn estimate_key(profile: &PitchClassProfile) -> Option<KeyEstimate> {
    if profile.is_empty() {
        return None;
    }
    let obs = profile.normalized();

    let mut best_r = f32::NEG_INFINITY;
    let mut best = (0u8, Mode::Ionian);
    let mut second_r = f32::NEG_INFINITY;

    for mode in Mode::ALL {
        for tonic in 0..12u8 {
            let r = pearson(&obs, &template(tonic, mode));
            if r > best_r {
                second_r = best_r;
                best_r = r;
                best = (tonic, mode);
            } else if r > second_r {
                second_r = r;
            }
        }
    }

    Some(KeyEstimate {
        tonic: best.0,
        mode: best.1,
        confidence: best_r.clamp(0.0, 1.0),
        margin: (best_r - second_r).max(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a profile from (pitch_class, weight) pairs.
    fn profile(pairs: &[(u8, f32)]) -> PitchClassProfile {
        let mut p = PitchClassProfile::new();
        for &(pc, w) in pairs {
            p.add_pc(pc, w);
        }
        p
    }

    /// A scale rooted at `tonic` with the given intervals, tonic & fifth
    /// emphasised the way real tonal playing emphasises them.
    fn scale_profile(tonic: u8, mode: Mode) -> PitchClassProfile {
        let ints = mode.intervals();
        let mut p = PitchClassProfile::new();
        for (i, &iv) in ints.iter().enumerate() {
            // tonic 3×, fifth 2×, everything else 1× — realistic emphasis.
            let w = if i == 0 {
                3.0
            } else if i == 4 {
                2.0
            } else {
                1.0
            };
            p.add_pc(tonic + iv, w);
        }
        p
    }

    #[test]
    fn detects_c_major() {
        let est = estimate_key(&scale_profile(0, Mode::Ionian)).unwrap();
        assert_eq!(est.tonic, 0, "tonic C");
        assert_eq!(est.mode, Mode::Ionian);
        assert_eq!(est.name(), "C major");
        assert!(est.confidence > 0.8, "confidence {}", est.confidence);
        assert!(est.margin > 0.0);
    }

    #[test]
    fn detects_a_minor_distinct_from_c_major() {
        // A natural minor shares C-major's pitch classes; only tonic/fifth
        // emphasis tells them apart. Tonic A=9.
        let est = estimate_key(&scale_profile(9, Mode::Aeolian)).unwrap();
        assert_eq!(est.tonic, 9, "tonic A, got {}", est.name());
        assert_eq!(est.mode, Mode::Aeolian);
        assert_eq!(est.name(), "A minor");
    }

    #[test]
    fn detects_d_dorian() {
        let est = estimate_key(&scale_profile(2, Mode::Dorian)).unwrap();
        assert_eq!(est.name(), "D Dorian", "got {}", est.name());
    }

    #[test]
    fn detects_g_mixolydian() {
        let est = estimate_key(&scale_profile(7, Mode::Mixolydian)).unwrap();
        assert_eq!(est.name(), "G Mixolydian", "got {}", est.name());
    }

    #[test]
    fn detects_f_lydian() {
        let est = estimate_key(&scale_profile(5, Mode::Lydian)).unwrap();
        assert_eq!(est.name(), "F Lydian", "got {}", est.name());
    }

    #[test]
    fn flat_profile_is_low_confidence() {
        // All twelve pitch classes equal → no tonal centre.
        let mut p = PitchClassProfile::new();
        for pc in 0..12 {
            p.add_pc(pc, 1.0);
        }
        let est = estimate_key(&p).unwrap();
        assert!(
            est.confidence < 0.2,
            "uniform profile should be low confidence, got {}",
            est.confidence
        );
    }

    #[test]
    fn empty_profile_is_none() {
        assert!(estimate_key(&PitchClassProfile::new()).is_none());
    }

    #[test]
    fn add_hz_maps_to_pitch_class() {
        let mut p = PitchClassProfile::new();
        p.add_hz(440.0, 1.0); // A4 → pc 9
        p.add_hz(261.63, 1.0); // C4 → pc 0
        assert_eq!(p.weights()[9], 1.0);
        assert_eq!(p.weights()[0], 1.0);
    }

    #[test]
    fn decay_shrinks_weights() {
        let mut p = profile(&[(0, 1.0)]);
        p.decay(0.5);
        assert!((p.total() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn estimate_roundtrips_through_serde() {
        let est = estimate_key(&scale_profile(7, Mode::Mixolydian)).unwrap();
        let json = serde_json::to_string(&est).unwrap();
        let back: KeyEstimate = serde_json::from_str(&json).unwrap();
        assert_eq!(est, back);
    }
}
