//! The tone descriptor and the heuristic that maps DSP features to it.
//!
//! This is the bootstrap path from `docs/design/story-phase3-tone-quality.md`:
//! useful, honest descriptors computed from spectral features today, behind the
//! same `assess()` interface a learned model will later implement. Descriptors,
//! never a single grade.

use serde::{Deserialize, Serialize};

use crate::room::RoomProfile;
use crate::vibrato::vibrato_regularity;
use crate::ToneFeatures;

/// Lowest / highest centroid (Hz) mapped to brightness 0 / 1 (log scale).
const BRIGHT_LO_HZ: f32 = 200.0;
const BRIGHT_HI_HZ: f32 = 4000.0;
/// Anchor the room normalisation around a canonical centroid.
const NEUTRAL_REF_HZ: f32 = 1000.0;
/// Returned for `vibrato_quality` when there is no clear vibrato to assess.
pub const VIBRATO_NEUTRAL: f32 = 0.5;

/// A multi-dimensional tone descriptor — each axis in `[0, 1]`.
///
/// Deliberately *not* a single score (architecture-v2 §5). These are the
/// qualities a teacher would name; the UI shows them as gentle labels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToneDescriptor {
    /// Bright vs dark (spectral centroid, room-normalised).
    pub brightness: f32,
    /// Warm vs thin (low-mid emphasis).
    pub warmth: f32,
    /// Air / breath / noise in the sound.
    pub air_noise: f32,
    /// Clarity / focus of the tonal core.
    pub core_clarity: f32,
    /// Vibrato regularity, or `VIBRATO_NEUTRAL` when no vibrato is present.
    pub vibrato_quality: f32,
}

#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Map timbre [`ToneFeatures`] (+ optional f0 contour) to a [`ToneDescriptor`],
/// conditioned on the [`RoomProfile`] so the same tone reads consistently
/// across rooms.
///
/// `f0_contour` is an optional per-frame fundamental-frequency track (Hz, 0 for
/// unvoiced) used only for `vibrato_quality`; pass `None` to leave it neutral.
pub fn assess(
    features: &ToneFeatures,
    f0_contour: Option<&[f32]>,
    room: &RoomProfile,
) -> ToneDescriptor {
    // Brightness: divide out the room's spectral colouring, then map the
    // effective centroid onto a log scale.
    let reference = if room.reference_centroid_hz > 0.0 {
        room.reference_centroid_hz
    } else {
        NEUTRAL_REF_HZ
    };
    let effective_centroid = features.spectral_centroid_hz * (NEUTRAL_REF_HZ / reference);
    let brightness = clamp01(
        (effective_centroid.max(1.0).log2() - BRIGHT_LO_HZ.log2())
            / (BRIGHT_HI_HZ.log2() - BRIGHT_LO_HZ.log2()),
    );

    // Warmth: the low-mid counterweight to brightness + high-band air.
    let warmth = clamp01(0.6 * (1.0 - brightness) + 0.4 * (1.0 - features.high_band_energy_ratio));

    // Air/noise: flatness + high-band energy, minus the room's own noise floor.
    let air_noise = clamp01(
        0.6 * features.spectral_flatness + 0.4 * features.high_band_energy_ratio - room.noise_floor,
    );

    // Core clarity: a tonal (peaked) spectrum has a clear core; noise muddies it.
    let core_clarity = clamp01((1.0 - features.spectral_flatness) - 0.3 * air_noise);

    let vibrato_quality = match f0_contour {
        Some(f0) => vibrato_regularity(f0),
        None => VIBRATO_NEUTRAL,
    };

    ToneDescriptor {
        brightness,
        warmth,
        air_noise,
        core_clarity,
        vibrato_quality,
    }
}
