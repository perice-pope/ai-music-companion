//! Short-time analysis: window the signal and produce per-frame magnitude
//! spectra. The shared front end for every spectral feature.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Analysis window length in samples (~46 ms at 44.1 kHz).
pub const WINDOW_SIZE: usize = 2048;
/// Hop between consecutive frames (75% overlap).
pub const HOP_SIZE: usize = 512;

/// Per-frame magnitude spectra plus the sample rate they were computed at.
pub struct Spectra {
    /// One magnitude row per frame; each row has `WINDOW_SIZE / 2 + 1` bins.
    pub frames: Vec<Vec<f32>>,
    /// Sample rate of the source audio (Hz).
    pub sample_rate: u32,
}

impl Spectra {
    /// Number of frequency bins per frame (`WINDOW_SIZE / 2 + 1`).
    pub fn n_bins(&self) -> usize {
        WINDOW_SIZE / 2 + 1
    }

    /// Centre frequency (Hz) of bin `k`.
    pub fn bin_hz(&self, k: usize) -> f32 {
        k as f32 * self.sample_rate as f32 / WINDOW_SIZE as f32
    }
}

/// Compute windowed FFT magnitude frames for `samples`.
///
/// Signals shorter than one window yield a single zero-padded frame, so the
/// caller always gets at least one frame for non-empty input.
pub fn magnitude_frames(samples: &[f32], sample_rate: u32) -> Spectra {
    let window = hann_window(WINDOW_SIZE);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);

    let mut frames: Vec<Vec<f32>> = Vec::new();
    if samples.is_empty() {
        return Spectra {
            frames,
            sample_rate,
        };
    }

    let mut start = 0usize;
    let n_bins = WINDOW_SIZE / 2 + 1;
    let mut scratch: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); WINDOW_SIZE];
    loop {
        // Fill the (windowed, zero-padded) frame.
        for i in 0..WINDOW_SIZE {
            let s = samples.get(start + i).copied().unwrap_or(0.0);
            scratch[i] = Complex::new(s * window[i], 0.0);
        }
        fft.process(&mut scratch);
        let mags: Vec<f32> = scratch[..n_bins].iter().map(|c| c.norm()).collect();
        frames.push(mags);

        if start + WINDOW_SIZE >= samples.len() {
            break;
        }
        start += HOP_SIZE;
    }

    Spectra {
        frames,
        sample_rate,
    }
}

/// Periodic Hann window of length `n`.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = (std::f32::consts::PI * i as f32 / n as f32).sin();
            x * x // sin^2(πi/n) == 0.5(1 - cos(2πi/n))
        })
        .collect()
}
