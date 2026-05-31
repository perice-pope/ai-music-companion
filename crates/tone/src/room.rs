//! Room acoustic profile — the conditioning input that lets the same tone read
//! consistently across rooms (architecture-v2 §5, mitigation #1).
//!
//! Captured from a short calibration: the musician plays a sustained tone (for
//! the room's spectral colouring) and we sample ambient silence (for the noise
//! floor). The profile is then divided out / subtracted in [`assess`].
//!
//! [`assess`]: crate::assess

use serde::{Deserialize, Serialize};

use crate::features;

/// A room's acoustic signature, used to normalise tone descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RoomProfile {
    /// Spectral centroid (Hz) the room imparts to a reference tone. `0.0`
    /// means "uncalibrated" — [`assess`] then uses a neutral anchor.
    ///
    /// [`assess`]: crate::assess
    pub reference_centroid_hz: f32,
    /// Ambient noisiness baseline in `[0, 1]` (flatness + high-band of silence),
    /// subtracted from the measured air/noise.
    pub noise_floor: f32,
}

impl RoomProfile {
    /// A neutral, uncalibrated profile — descriptors fall back to absolute
    /// (un-normalised) mappings. Relative-to-baseline tracking still works.
    pub fn neutral() -> Self {
        Self {
            reference_centroid_hz: 0.0,
            noise_floor: 0.0,
        }
    }

    /// Build a profile from calibration audio: a `reference_tone` (a sustained
    /// note) supplies the room's spectral colouring, and optional `ambient`
    /// (near-silence) supplies the noise floor.
    pub fn calibrate(reference_tone: &[f32], ambient: Option<&[f32]>, sample_rate: u32) -> Self {
        let tone = features(reference_tone, sample_rate);
        let noise_floor = match ambient {
            Some(a) if !a.is_empty() => {
                let amb = features(a, sample_rate);
                // Ambient "noisiness" — what the room contributes with nothing played.
                (0.6 * amb.spectral_flatness + 0.4 * amb.high_band_energy_ratio).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        Self {
            reference_centroid_hz: tone.spectral_centroid_hz,
            noise_floor,
        }
    }
}

impl Default for RoomProfile {
    fn default() -> Self {
        Self::neutral()
    }
}
