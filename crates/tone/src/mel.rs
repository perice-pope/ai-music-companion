//! Mel filterbank energies and MFCCs from per-frame magnitude spectra.
//!
//! Standard triangular mel filterbank → log energies → DCT-II for MFCCs.
//! Both are frame-averaged so a phrase yields one vector each.

use crate::frames::Spectra;

/// Number of mel bands.
pub const N_MELS: usize = 40;
/// Number of MFCC coefficients retained.
pub const N_MFCC: usize = 13;

/// Frame-averaged log-mel band energies and MFCCs.
pub struct MelFeatures {
    pub mel_log_energies: Vec<f32>,
    pub mfcc: Vec<f32>,
}

pub fn mel_features(spectra: &Spectra) -> MelFeatures {
    if spectra.frames.is_empty() {
        return MelFeatures {
            mel_log_energies: vec![0.0; N_MELS],
            mfcc: vec![0.0; N_MFCC],
        };
    }

    let filterbank = build_filterbank(spectra.n_bins(), spectra.sample_rate);

    // Average log-mel energies across frames.
    let mut acc = vec![0.0f64; N_MELS];
    for mags in &spectra.frames {
        for (m, filter) in filterbank.iter().enumerate() {
            let mut energy = 0.0f32;
            for &(bin, weight) in filter {
                energy += mags[bin] * mags[bin] * weight;
            }
            acc[m] += (energy as f64 + 1e-10).ln();
        }
    }
    let n = spectra.frames.len() as f64;
    let mel_log_energies: Vec<f32> = acc.iter().map(|v| (v / n) as f32).collect();

    MelFeatures {
        mfcc: dct_ii(&mel_log_energies, N_MFCC),
        mel_log_energies,
    }
}

/// Triangular mel filterbank as a sparse `[(bin, weight)]` list per filter.
fn build_filterbank(n_bins: usize, sample_rate: u32) -> Vec<Vec<(usize, f32)>> {
    let f_max = sample_rate as f32 / 2.0;
    let mel_max = hz_to_mel(f_max);
    // N_MELS filters need N_MELS + 2 band edges in mel space.
    let edges_hz: Vec<f32> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(mel_max * i as f32 / (N_MELS + 1) as f32))
        .collect();

    let bin_hz = |k: usize| k as f32 * sample_rate as f32 / (2.0 * (n_bins - 1) as f32);

    let mut filters = Vec::with_capacity(N_MELS);
    for m in 0..N_MELS {
        let (lo, mid, hi) = (edges_hz[m], edges_hz[m + 1], edges_hz[m + 2]);
        let mut filter = Vec::new();
        for k in 0..n_bins {
            let f = bin_hz(k);
            let w = if f >= lo && f <= mid && mid > lo {
                (f - lo) / (mid - lo)
            } else if f > mid && f <= hi && hi > mid {
                (hi - f) / (hi - mid)
            } else {
                0.0
            };
            if w > 0.0 {
                filter.push((k, w));
            }
        }
        filters.push(filter);
    }
    filters
}

/// DCT-II of `input`, keeping the first `n_out` coefficients.
fn dct_ii(input: &[f32], n_out: usize) -> Vec<f32> {
    let n = input.len();
    (0..n_out)
        .map(|i| {
            let mut sum = 0.0f32;
            for (m, &x) in input.iter().enumerate() {
                sum += x * (std::f32::consts::PI * i as f32 * (m as f32 + 0.5) / n as f32).cos();
            }
            sum
        })
        .collect()
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}
