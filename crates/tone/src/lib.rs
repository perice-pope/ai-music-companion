//! On-device tone-quality (timbre) feature extraction for AI Music Companion.
//!
//! This is PR 1 of the tone-quality story (`docs/design/story-phase3-tone-quality.md`):
//! the deterministic DSP front end that turns a phrase-length audio buffer into
//! a [`ToneFeatures`] vector. Later slices map these features to a
//! `ToneDescriptor` (heuristic, then a learned ONNX model) — but the feature
//! contract lives here and is fully testable without any model.
//!
//! Everything runs off the real-time path (per phrase, not per sample), so a
//! dedicated crate keeps the eventual ONNX dependency out of `crates/ears`'
//! real-time build — mirroring the `crates/transcribe` boundary.

mod baseline;
mod descriptor;
mod frames;
mod mel;
mod room;
mod spectral;
mod vibrato;

pub use baseline::{ToneBaseline, ToneDelta};
pub use descriptor::{assess, ToneDescriptor};
pub use mel::{N_MELS, N_MFCC};
pub use room::RoomProfile;
pub use vibrato::vibrato_regularity;

use frames::magnitude_frames;
use mel::mel_features;
use spectral::spectral_stats;

/// Deterministic timbre features extracted from a phrase-length mono buffer.
///
/// All values are averaged over the analysis frames, so one phrase yields one
/// [`ToneFeatures`]. Frequencies are in Hz; `mel_log_energies` has [`N_MELS`]
/// entries and `mfcc` has [`N_MFCC`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToneFeatures {
    /// Spectral centroid — the "brightness" cue. Higher = brighter.
    pub spectral_centroid_hz: f32,
    /// Spectral spread (bandwidth) about the centroid.
    pub spectral_spread_hz: f32,
    /// Frequency below which 85% of the energy lies.
    pub spectral_rolloff_hz: f32,
    /// Spectral flatness in `[0, 1]` — high for noise-like (airy) spectra.
    pub spectral_flatness: f32,
    /// Spectral flux — mean positive frame-to-frame change (steadiness cue).
    pub spectral_flux: f32,
    /// Fraction of energy above ~4 kHz — an air/noise proxy.
    pub high_band_energy_ratio: f32,
    /// Frame-averaged log mel-band energies (`N_MELS`).
    pub mel_log_energies: Vec<f32>,
    /// Frame-averaged MFCCs (`N_MFCC`).
    pub mfcc: Vec<f32>,
}

impl ToneFeatures {
    /// A zeroed feature vector (used for empty/silent input).
    fn zeroed() -> Self {
        Self {
            spectral_centroid_hz: 0.0,
            spectral_spread_hz: 0.0,
            spectral_rolloff_hz: 0.0,
            spectral_flatness: 0.0,
            spectral_flux: 0.0,
            high_band_energy_ratio: 0.0,
            mel_log_energies: vec![0.0; N_MELS],
            mfcc: vec![0.0; N_MFCC],
        }
    }
}

/// Extract timbre features from a phrase-length mono buffer.
///
/// `samples` is mono PCM at `sample_rate`; no resampling is required (mel scale
/// adapts to the rate). Empty input returns a zeroed [`ToneFeatures`] rather
/// than erroring — silence is a valid, if uninformative, phrase.
pub fn features(samples: &[f32], sample_rate: u32) -> ToneFeatures {
    if samples.is_empty() || sample_rate == 0 {
        return ToneFeatures::zeroed();
    }
    let spectra = magnitude_frames(samples, sample_rate);
    let s = spectral_stats(&spectra);
    let m = mel_features(&spectra);
    ToneFeatures {
        spectral_centroid_hz: s.centroid_hz,
        spectral_spread_hz: s.spread_hz,
        spectral_rolloff_hz: s.rolloff_hz,
        spectral_flatness: s.flatness,
        spectral_flux: s.flux,
        high_band_energy_ratio: s.high_band_energy_ratio,
        mel_log_energies: m.mel_log_energies,
        mfcc: m.mfcc,
    }
}
