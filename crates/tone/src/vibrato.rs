//! Vibrato regularity from a fundamental-frequency contour.
//!
//! "Vibrato quality" ≈ how *consistent* the pitch oscillation is (architecture-v2
//! §5). We measure the regularity of the f0 modulation, rate-agnostic, so it
//! works regardless of the contour's frame rate. The contour itself comes from
//! the pitch tracker (`ears`) at integration time; here we only judge its shape.

use crate::descriptor::VIBRATO_NEUTRAL;

/// Minimum voiced frames needed to judge vibrato.
const MIN_VOICED: usize = 12;
/// Minimum modulation depth (cents) to count as vibrato rather than a steady tone.
const MIN_DEPTH_CENTS: f32 = 10.0;

/// Score the regularity of vibrato in an f0 contour, in `[0, 1]`.
///
/// `f0` is per-frame fundamental frequency in Hz (`0` or negative = unvoiced).
/// Returns [`VIBRATO_NEUTRAL`] when there is no clear vibrato to assess (too
/// little voiced data or a dead-straight tone). A regular oscillation scores
/// high; an erratic one scores low.
pub fn vibrato_regularity(f0: &[f32]) -> f32 {
    let voiced: Vec<f32> = f0.iter().copied().filter(|&f| f > 0.0).collect();
    if voiced.len() < MIN_VOICED {
        return VIBRATO_NEUTRAL;
    }

    // Work in cents relative to the mean so the measure is pitch-independent.
    let mean: f32 = voiced.iter().sum::<f32>() / voiced.len() as f32;
    if mean <= 0.0 {
        return VIBRATO_NEUTRAL;
    }
    let cents: Vec<f32> = voiced.iter().map(|&f| 1200.0 * (f / mean).log2()).collect();

    // Modulation depth — a straight tone has nothing to assess.
    let depth = cents.iter().cloned().fold(0.0_f32, |m, c| m.max(c.abs()));
    if depth < MIN_DEPTH_CENTS {
        return VIBRATO_NEUTRAL;
    }

    // Intervals between upward zero-crossings = vibrato cycle lengths.
    let mut crossings: Vec<usize> = Vec::new();
    for i in 1..cents.len() {
        if cents[i - 1] <= 0.0 && cents[i] > 0.0 {
            crossings.push(i);
        }
    }
    if crossings.len() < 3 {
        // Fewer than ~2 full cycles — not enough to judge regularity.
        return VIBRATO_NEUTRAL;
    }
    let periods: Vec<f32> = crossings.windows(2).map(|w| (w[1] - w[0]) as f32).collect();

    // Regularity = 1 - coefficient of variation of the cycle lengths.
    let pmean: f32 = periods.iter().sum::<f32>() / periods.len() as f32;
    if pmean <= 0.0 {
        return VIBRATO_NEUTRAL;
    }
    let var: f32 = periods.iter().map(|p| (p - pmean).powi(2)).sum::<f32>() / periods.len() as f32;
    let cv = var.sqrt() / pmean;
    (1.0 - cv).clamp(0.0, 1.0)
}
