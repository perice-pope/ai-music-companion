//! Aggregate spectral descriptors computed from per-frame magnitude spectra.
//!
//! Each value is averaged over frames so a phrase yields one number. These are
//! the DSP timbre cues a tone descriptor leans on: brightness (centroid),
//! spread, rolloff, noisiness (flatness, high-band energy), and change (flux).

use crate::frames::Spectra;

/// Frequency above which energy counts as "high band" (air/noise proxy), Hz.
const HIGH_BAND_HZ: f32 = 4000.0;
/// Energy fraction for the spectral rolloff point.
const ROLLOFF_FRACTION: f32 = 0.85;

/// Frame-averaged spectral descriptors.
pub struct SpectralStats {
    pub centroid_hz: f32,
    pub spread_hz: f32,
    pub rolloff_hz: f32,
    pub flatness: f32,
    pub flux: f32,
    pub high_band_energy_ratio: f32,
}

pub fn spectral_stats(spectra: &Spectra) -> SpectralStats {
    let n_frames = spectra.frames.len();
    if n_frames == 0 {
        return SpectralStats {
            centroid_hz: 0.0,
            spread_hz: 0.0,
            rolloff_hz: 0.0,
            flatness: 0.0,
            flux: 0.0,
            high_band_energy_ratio: 0.0,
        };
    }

    let n_bins = spectra.n_bins();
    let freqs: Vec<f32> = (0..n_bins).map(|k| spectra.bin_hz(k)).collect();

    let mut centroid = 0.0f64;
    let mut spread = 0.0f64;
    let mut rolloff = 0.0f64;
    let mut flatness = 0.0f64;
    let mut high_ratio = 0.0f64;
    let mut flux = 0.0f64;

    for (fi, mags) in spectra.frames.iter().enumerate() {
        let total: f32 = mags.iter().sum();
        if total <= f32::EPSILON {
            continue;
        }

        // Centroid + spread (magnitude-weighted mean / std of frequency).
        let c: f32 = freqs.iter().zip(mags).map(|(f, m)| f * m).sum::<f32>() / total;
        let var: f32 = freqs
            .iter()
            .zip(mags)
            .map(|(f, m)| (f - c).powi(2) * m)
            .sum::<f32>()
            / total;
        centroid += c as f64;
        spread += var.max(0.0).sqrt() as f64;

        // Rolloff: lowest frequency below which ROLLOFF_FRACTION of energy lies.
        let threshold = total * ROLLOFF_FRACTION;
        let mut acc = 0.0f32;
        let mut roll = freqs[n_bins - 1];
        for (k, m) in mags.iter().enumerate() {
            acc += *m;
            if acc >= threshold {
                roll = freqs[k];
                break;
            }
        }
        rolloff += roll as f64;

        // Flatness: geometric / arithmetic mean of the power spectrum (0..1),
        // high for noise-like spectra, low for tonal ones.
        let mut log_sum = 0.0f64;
        let mut arith = 0.0f64;
        for m in mags {
            let p = (m * m) as f64 + 1e-12;
            log_sum += p.ln();
            arith += p;
        }
        let geo = (log_sum / n_bins as f64).exp();
        let ari = arith / n_bins as f64;
        flatness += if ari > 0.0 { geo / ari } else { 0.0 };

        // High-band energy ratio.
        let high: f32 = freqs
            .iter()
            .zip(mags)
            .filter(|(f, _)| **f >= HIGH_BAND_HZ)
            .map(|(_, m)| m)
            .sum();
        high_ratio += (high / total) as f64;

        // Spectral flux: positive L2 change from the previous frame.
        if fi > 0 {
            let prev = &spectra.frames[fi - 1];
            let mut sq = 0.0f32;
            for (m, p) in mags.iter().zip(prev) {
                let d = m - p;
                if d > 0.0 {
                    sq += d * d;
                }
            }
            flux += (sq.sqrt() / total) as f64;
        }
    }

    let nf = n_frames as f64;
    let flux_div = (n_frames.saturating_sub(1)).max(1) as f64;
    SpectralStats {
        centroid_hz: (centroid / nf) as f32,
        spread_hz: (spread / nf) as f32,
        rolloff_hz: (rolloff / nf) as f32,
        flatness: (flatness / nf) as f32,
        flux: (flux / flux_div) as f32,
        high_band_energy_ratio: (high_ratio / nf) as f32,
    }
}
