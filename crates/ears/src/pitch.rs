//! Pitch detection using the YIN algorithm (pure Rust).
//!
//! YIN is the algorithm family that Aubio's `yinfft` is based on.
//! This implementation is suitable for real-time use with ~5-6ms hop sizes.

use crate::AudioEvent;

/// Errors from pitch detector construction.
#[derive(Debug, thiserror::Error)]
pub enum PitchError {
    #[error("sample_rate must be > 0, got {0}")]
    InvalidSampleRate(u32),
    #[error("freq_min_hz must be > 0 and < freq_max_hz (got min={min}, max={max})")]
    InvalidFreqRange { min: f64, max: f64 },
    #[error("threshold must be in (0.0, 1.0), got {0}")]
    InvalidThreshold(f64),
}

/// Configuration for the pitch detector.
#[derive(Debug, Clone)]
pub struct PitchConfig {
    /// Sample rate of the audio input.
    pub sample_rate: u32,
    /// YIN threshold — lower = stricter pitch confidence. Typical: 0.10–0.20.
    pub threshold: f64,
    /// Minimum detectable frequency in Hz (sets max lag).
    pub freq_min_hz: f64,
    /// Maximum detectable frequency in Hz (sets min lag).
    pub freq_max_hz: f64,
}

impl Default for PitchConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            threshold: 0.15,
            freq_min_hz: 60.0,
            freq_max_hz: 2000.0,
        }
    }
}

/// Real-time pitch detector using the YIN algorithm.
///
/// Scratch buffers are pre-allocated once at construction — `detect()` does
/// **zero heap allocations**, making it safe for the audio thread.
pub struct PitchDetector {
    config: PitchConfig,
    /// Running timestamp in seconds.
    timestamp: f64,
    /// Analysis window size (samples). Must contain ≥2 periods of freq_min.
    window_size: usize,
    /// Whether the previous frame was voiced (for onset edge detection).
    prev_voiced: bool,
    // --- Pre-allocated scratch buffers (no alloc in detect) ---
    tau_min: usize,
    tau_max: usize,
    diff: Vec<f64>,
    cmnd: Vec<f64>,
}

impl PitchDetector {
    /// Create a new pitch detector, pre-allocating all scratch buffers.
    ///
    /// Returns an error if the config contains invalid values.
    pub fn new(config: PitchConfig) -> Result<Self, PitchError> {
        if config.sample_rate == 0 {
            return Err(PitchError::InvalidSampleRate(config.sample_rate));
        }
        if config.freq_min_hz <= 0.0 || config.freq_max_hz <= config.freq_min_hz {
            return Err(PitchError::InvalidFreqRange {
                min: config.freq_min_hz,
                max: config.freq_max_hz,
            });
        }
        if config.threshold <= 0.0 || config.threshold >= 1.0 {
            return Err(PitchError::InvalidThreshold(config.threshold));
        }

        // Window needs at least 2 periods of the lowest frequency
        let window_size =
            (2.0 * config.sample_rate as f64 / config.freq_min_hz) as usize;
        let tau_min = (config.sample_rate as f64 / config.freq_max_hz) as usize;
        let tau_max = (config.sample_rate as f64 / config.freq_min_hz).ceil() as usize;

        // Pre-allocate scratch buffers
        let diff = vec![0.0f64; tau_max + 1];
        let cmnd = vec![0.0f64; tau_max + 1];

        Ok(Self {
            config,
            timestamp: 0.0,
            window_size,
            prev_voiced: false,
            tau_min,
            tau_max,
            diff,
            cmnd,
        })
    }

    /// Required number of samples per analysis window.
    pub fn window_size(&self) -> usize {
        self.window_size
    }

    /// Detect pitch from a buffer of f32 samples.
    ///
    /// The buffer should contain at least `window_size()` samples.
    /// Returns an `AudioEvent` with pitch, confidence, amplitude, and onset info.
    ///
    /// **Zero heap allocations** — all scratch buffers are pre-allocated.
    pub fn detect(&mut self, samples: &[f32]) -> AudioEvent {
        let amplitude = rms(samples);
        let timestamp_secs = self.timestamp;
        self.timestamp += samples.len() as f64 / self.config.sample_rate as f64;

        // Silence gate — don't bother detecting pitch in silence
        if amplitude < 0.01 {
            self.prev_voiced = false;
            return AudioEvent {
                pitch_hz: None,
                confidence: 0.0,
                amplitude,
                timestamp_secs,
                is_onset: false,
            };
        }

        let (pitch_hz, confidence) = self.yin_pitch(samples);

        // Onset edge detection: fire only on the transition from unvoiced → voiced
        let currently_voiced = confidence > 0.8 && amplitude > 0.05;
        let is_onset = currently_voiced && !self.prev_voiced;
        self.prev_voiced = currently_voiced;

        AudioEvent {
            pitch_hz,
            confidence,
            amplitude,
            timestamp_secs,
            is_onset,
        }
    }

    /// YIN pitch detection using pre-allocated scratch buffers.
    ///
    /// Returns `(Some(hz), confidence)` if a pitch is found, `(None, 0.0)` if not.
    fn yin_pitch(&mut self, samples: &[f32]) -> (Option<f64>, f64) {
        let n = samples.len();
        let half = n / 2;

        let tau_max = self.tau_max.min(half);
        let tau_min = self.tau_min;

        if tau_max <= tau_min || tau_max > half {
            return (None, 0.0);
        }

        // Step 1 & 2: Difference function (reuse pre-allocated buffer)
        for val in self.diff[..=tau_max].iter_mut() {
            *val = 0.0;
        }
        for tau in 1..=tau_max {
            let mut sum = 0.0f64;
            for j in 0..half {
                let d = samples[j] as f64 - samples[j + tau] as f64;
                sum += d * d;
            }
            self.diff[tau] = sum;
        }

        // Cumulative mean normalized difference function (reuse pre-allocated buffer)
        self.cmnd[0] = 1.0;
        let mut running_sum = 0.0;
        for tau in 1..=tau_max {
            running_sum += self.diff[tau];
            if running_sum > 0.0 {
                self.cmnd[tau] = self.diff[tau] * tau as f64 / running_sum;
            } else {
                self.cmnd[tau] = 1.0;
            }
        }

        // Step 3: Absolute threshold — find first dip below threshold
        let mut best_tau = None;
        for tau in tau_min..=tau_max {
            if self.cmnd[tau] < self.config.threshold {
                let mut min_tau = tau;
                while min_tau + 1 <= tau_max && self.cmnd[min_tau + 1] < self.cmnd[min_tau] {
                    min_tau += 1;
                }
                best_tau = Some(min_tau);
                break;
            }
        }

        // Fallback: global minimum
        let best_tau = best_tau.unwrap_or_else(|| {
            (tau_min..=tau_max)
                .min_by(|&a, &b| self.cmnd[a].partial_cmp(&self.cmnd[b]).unwrap())
                .unwrap_or(tau_min)
        });

        let confidence = 1.0 - self.cmnd[best_tau];

        if confidence < 0.5 {
            return (None, confidence);
        }

        // Step 4: Parabolic interpolation for sub-sample accuracy
        let tau_refined = if best_tau > 0 && best_tau < tau_max {
            let alpha = self.cmnd[best_tau - 1];
            let beta = self.cmnd[best_tau];
            let gamma = self.cmnd[best_tau + 1];
            let adjustment = (alpha - gamma) / (2.0 * (alpha - 2.0 * beta + gamma));
            best_tau as f64 + adjustment
        } else {
            best_tau as f64
        };

        let freq = self.config.sample_rate as f64 / tau_refined;

        if freq >= self.config.freq_min_hz && freq <= self.config.freq_max_hz {
            (Some(freq), confidence)
        } else {
            (None, confidence)
        }
    }
}

/// Calculate RMS (root mean square) amplitude of a sample buffer.
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// Generate a sine wave for testing.
pub fn generate_sine(freq_hz: f64, sample_rate: u32, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            (2.0 * std::f64::consts::PI * freq_hz * t).sin() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_a4_440hz() {
        let config = PitchConfig {
            sample_rate: 44100,
            ..Default::default()
        };
        let mut detector = PitchDetector::new(config).unwrap();
        let samples = generate_sine(440.0, 44100, detector.window_size());

        let event = detector.detect(&samples);
        assert!(event.pitch_hz.is_some(), "Should detect pitch");
        let hz = event.pitch_hz.unwrap();
        assert!((hz - 440.0).abs() < 5.0, "Expected ~440 Hz, got {hz:.1} Hz");
        assert!(event.confidence > 0.8, "Confidence should be high");
    }

    #[test]
    fn detect_middle_c_261hz() {
        let config = PitchConfig::default();
        let mut detector = PitchDetector::new(config).unwrap();
        let samples = generate_sine(261.63, 44100, detector.window_size());

        let event = detector.detect(&samples);
        assert!(event.pitch_hz.is_some());
        let hz = event.pitch_hz.unwrap();
        assert!(
            (hz - 261.63).abs() < 5.0,
            "Expected ~261.63 Hz, got {hz:.1} Hz"
        );
    }

    #[test]
    fn silence_returns_none() {
        let config = PitchConfig::default();
        let mut detector = PitchDetector::new(config).unwrap();
        let samples = vec![0.0f32; detector.window_size()];

        let event = detector.detect(&samples);
        assert!(event.pitch_hz.is_none(), "Silence should not detect pitch");
        assert!(event.amplitude < 0.01);
    }

    #[test]
    fn rms_of_known_signal() {
        let samples = [1.0f32, -1.0, 1.0, -1.0];
        let r = rms(&samples);
        assert!((r - 1.0).abs() < 0.001);
    }

    #[test]
    fn rms_of_silence() {
        let samples = [0.0f32; 100];
        assert!(rms(&samples) < 0.001);
    }

    #[test]
    fn timestamp_advances() {
        let config = PitchConfig {
            sample_rate: 44100,
            ..Default::default()
        };
        let mut detector = PitchDetector::new(config).unwrap();
        let samples = generate_sine(440.0, 44100, detector.window_size());

        let event1 = detector.detect(&samples);
        assert!((event1.timestamp_secs - 0.0).abs() < f64::EPSILON);

        let event2 = detector.detect(&samples);
        assert!(event2.timestamp_secs > 0.0);
    }

    #[test]
    fn generate_sine_correct_length() {
        let samples = generate_sine(440.0, 44100, 1024);
        assert_eq!(samples.len(), 1024);
    }

    #[test]
    fn invalid_sample_rate_rejected() {
        let config = PitchConfig {
            sample_rate: 0,
            ..Default::default()
        };
        assert!(PitchDetector::new(config).is_err());
    }

    #[test]
    fn invalid_freq_range_rejected() {
        let config = PitchConfig {
            freq_min_hz: 1000.0,
            freq_max_hz: 500.0,
            ..Default::default()
        };
        assert!(PitchDetector::new(config).is_err());
    }

    #[test]
    fn onset_only_fires_on_transition() {
        let config = PitchConfig::default();
        let mut detector = PitchDetector::new(config).unwrap();
        let samples = generate_sine(440.0, 44100, detector.window_size());

        // First voiced frame → onset
        let e1 = detector.detect(&samples);
        assert!(e1.is_onset, "First voiced frame should be an onset");

        // Second voiced frame → NOT an onset (sustained)
        let e2 = detector.detect(&samples);
        assert!(!e2.is_onset, "Sustained frame should not be an onset");

        // Silence → no onset
        let silence = vec![0.0f32; detector.window_size()];
        let e3 = detector.detect(&silence);
        assert!(!e3.is_onset);

        // Voice again → onset (transition from silence)
        let e4 = detector.detect(&samples);
        assert!(e4.is_onset, "Re-entry after silence should be an onset");
    }
}
