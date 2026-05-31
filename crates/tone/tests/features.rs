//! Feature-extraction tests on synthesised tones — deterministic, no model,
//! no copyrighted audio. They assert the *ordering* the descriptors will rely
//! on (bright > dark, noisy > tonal, high-band for high tones).

use tone::{features, N_MELS, N_MFCC};

const SR: u32 = 44_100;
const N: usize = 22_050; // 0.5 s

fn sine(freq: f64) -> Vec<f32> {
    (0..N)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * 0.7
        })
        .collect()
}

/// Deterministic white-ish noise via a small LCG (no rand dependency).
fn noise() -> Vec<f32> {
    let mut state: u32 = 0x1234_5678;
    (0..N)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        })
        .map(|x| x * 0.5)
        .collect()
}

#[test]
fn brighter_tone_has_higher_centroid() {
    let dark = features(&sine(300.0), SR);
    let bright = features(&sine(3000.0), SR);
    assert!(
        bright.spectral_centroid_hz > dark.spectral_centroid_hz,
        "bright {} should exceed dark {}",
        bright.spectral_centroid_hz,
        dark.spectral_centroid_hz
    );
    // Centroid of a pure tone sits near its frequency.
    assert!((dark.spectral_centroid_hz - 300.0).abs() < 200.0);
    assert!((bright.spectral_centroid_hz - 3000.0).abs() < 400.0);
}

#[test]
fn noise_is_flatter_than_a_pure_tone() {
    let tonal = features(&sine(440.0), SR);
    let noisy = features(&noise(), SR);
    assert!(
        noisy.spectral_flatness > tonal.spectral_flatness,
        "noise flatness {} should exceed tone flatness {}",
        noisy.spectral_flatness,
        tonal.spectral_flatness
    );
}

#[test]
fn high_tone_has_more_high_band_energy() {
    let low = features(&sine(300.0), SR);
    let high = features(&sine(6000.0), SR);
    assert!(
        low.high_band_energy_ratio < 0.2,
        "300Hz tone is not high-band"
    );
    assert!(
        high.high_band_energy_ratio > 0.8,
        "6kHz tone should be mostly high-band, got {}",
        high.high_band_energy_ratio
    );
}

#[test]
fn steady_tone_has_lower_flux_than_noise() {
    let steady = features(&sine(440.0), SR);
    let noisy = features(&noise(), SR);
    assert!(
        noisy.spectral_flux > steady.spectral_flux,
        "noise flux {} should exceed steady-tone flux {}",
        noisy.spectral_flux,
        steady.spectral_flux
    );
}

#[test]
fn feature_vectors_have_expected_shape() {
    let f = features(&sine(440.0), SR);
    assert_eq!(f.mel_log_energies.len(), N_MELS);
    assert_eq!(f.mfcc.len(), N_MFCC);
    assert!(f.spectral_rolloff_hz > 0.0);
}

#[test]
fn empty_and_silent_input_are_handled() {
    let empty = features(&[], SR);
    assert_eq!(empty.spectral_centroid_hz, 0.0);
    assert_eq!(empty.mel_log_energies.len(), N_MELS);

    // Digital silence: no panics, centroid stays zero (no energy to weight).
    let silent = features(&vec![0.0; N], SR);
    assert_eq!(silent.spectral_centroid_hz, 0.0);
}
