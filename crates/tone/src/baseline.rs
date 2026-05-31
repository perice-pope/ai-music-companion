//! Relative-to-baseline tone tracking — "warmer than Tuesday".
//!
//! Absolute tone grading is fragile (room acoustics, mic). Comparing a phrase
//! to the musician's *own* rolling baseline in the *same* room is both more
//! useful and more robust (architecture-v2 §5, mitigation #2). The baseline is
//! an exponential moving average per dimension; [`ToneBaseline::compare`]
//! returns the signed deltas.

use serde::{Deserialize, Serialize};

use crate::ToneDescriptor;

/// Default EMA smoothing factor (weight of each new observation).
const DEFAULT_ALPHA: f32 = 0.2;

/// Signed per-dimension difference of a descriptor from a baseline.
/// Positive `warmth` means "warmer than your baseline", and so on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToneDelta {
    pub brightness: f32,
    pub warmth: f32,
    pub air_noise: f32,
    pub core_clarity: f32,
    pub vibrato_quality: f32,
}

/// A rolling, per-dimension exponential-moving-average baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToneBaseline {
    mean: Option<ToneDescriptor>,
    alpha: f32,
}

impl ToneBaseline {
    /// New baseline with the default smoothing factor.
    pub fn new() -> Self {
        Self {
            mean: None,
            alpha: DEFAULT_ALPHA,
        }
    }

    /// New baseline with an explicit smoothing factor in `(0, 1]`.
    pub fn with_alpha(alpha: f32) -> Self {
        Self {
            mean: None,
            alpha: alpha.clamp(f32::EPSILON, 1.0),
        }
    }

    /// The current baseline descriptor, if any observations have been recorded.
    pub fn current(&self) -> Option<ToneDescriptor> {
        self.mean
    }

    /// Fold a new descriptor into the rolling baseline.
    pub fn update(&mut self, d: &ToneDescriptor) {
        self.mean = Some(match self.mean {
            None => *d,
            Some(m) => ToneDescriptor {
                brightness: ema(m.brightness, d.brightness, self.alpha),
                warmth: ema(m.warmth, d.warmth, self.alpha),
                air_noise: ema(m.air_noise, d.air_noise, self.alpha),
                core_clarity: ema(m.core_clarity, d.core_clarity, self.alpha),
                vibrato_quality: ema(m.vibrato_quality, d.vibrato_quality, self.alpha),
            },
        });
    }

    /// Signed deltas of `d` from the current baseline. `None` until at least one
    /// observation has been recorded.
    pub fn compare(&self, d: &ToneDescriptor) -> Option<ToneDelta> {
        self.mean.map(|m| ToneDelta {
            brightness: d.brightness - m.brightness,
            warmth: d.warmth - m.warmth,
            air_noise: d.air_noise - m.air_noise,
            core_clarity: d.core_clarity - m.core_clarity,
            vibrato_quality: d.vibrato_quality - m.vibrato_quality,
        })
    }
}

impl Default for ToneBaseline {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn ema(prev: f32, new: f32, alpha: f32) -> f32 {
    alpha * new + (1.0 - alpha) * prev
}
