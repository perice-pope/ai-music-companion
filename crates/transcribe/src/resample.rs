//! Self-contained, anti-aliased sample-rate conversion to the model rate.
//!
//! A windowed-sinc (Blackman) kernel with the cutoff clamped to the lower of
//! the input/output Nyquist frequencies, so down-sampling does not alias.
//! Deterministic and dependency-free — quality is adequate for monophonic
//! transcription and the whole path stays pure Rust.

use crate::constants::AUDIO_SAMPLE_RATE;

/// Half-width of the sinc kernel, in input samples (→ 2*HALF+1 taps).
pub(crate) const KERNEL_HALF: isize = 32;

/// Resample mono `samples` from `sr_in` Hz to the model's 22.05 kHz.
pub fn resample_to_model_rate(samples: &[f32], sr_in: u32) -> Vec<f32> {
    let sr_out = AUDIO_SAMPLE_RATE as u32;
    if sr_in == sr_out || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = sr_out as f64 / sr_in as f64;
    // Normalised cutoff (cycles/input-sample). For down-sampling, limit to the
    // output Nyquist to suppress aliasing; never exceed the input Nyquist.
    let cutoff = 0.5 * ratio.min(1.0);

    let out_len = ((samples.len() as f64) * ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);

    for n in 0..out_len {
        let center = n as f64 / ratio; // position in input-sample units
        let i0 = center.floor() as isize;
        let mut acc = 0.0_f64;
        let mut norm = 0.0_f64;
        for j in (i0 - KERNEL_HALF)..=(i0 + KERNEL_HALF) {
            if j < 0 || j as usize >= samples.len() {
                continue;
            }
            let x = center - j as f64;
            let w = sinc(2.0 * cutoff * x) * blackman(x, KERNEL_HALF as f64);
            acc += samples[j as usize] as f64 * w;
            norm += w;
        }
        out.push(if norm.abs() > 1e-12 {
            (acc / norm) as f32
        } else {
            0.0
        });
    }
    out
}

#[inline]
pub(crate) fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let pix = std::f64::consts::PI * x;
        pix.sin() / pix
    }
}

/// Blackman window over `[-half, half]`; zero outside.
#[inline]
pub(crate) fn blackman(x: f64, half: f64) -> f64 {
    if x.abs() > half {
        return 0.0;
    }
    // Standard Blackman, phase mapped so x=0 → peak, x=±half → edge.
    let t = (x + half) / (2.0 * half); // 0..1
    let two_pi = 2.0 * std::f64::consts::PI;
    0.42 - 0.5 * (two_pi * t).cos() + 0.08 * (2.0 * two_pi * t).cos()
}
