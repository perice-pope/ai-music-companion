//! Pitch detection using the YIN algorithm (pure Rust).
//!
//! YIN is the algorithm family that Aubio's `yinfft` is based on.
//! This implementation is suitable for real-time use with ~5-6ms hop sizes.

use crate::AudioEvent;

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
pub struct PitchDetector {
    config: PitchConfig,
    /// Running timestamp in seconds.
    timestamp: f64,
    /// Samples per analysis frame (hop size).
    hop_size: usize,
}

impl PitchDetector {
    pub fn new(config: PitchConfig) -> Self {
        // Hop size: need at least 2 periods of the lowest frequency
        let hop_size = (2.0 * config.sample_rate as f64 / config.freq_min_hz) as usize;
        Self {
            config,
            timestamp: 0.0,
            hop_size,
        }
    }

    /// Required number of samples per analysis frame.
    pub fn hop_size(&self) -> usize {
        self.hop_size
    }

    /// Detect pitch from a buffer of f32 samples.
    ///
    /// The buffer should contain at least `hop_size()` samples.
    /// Returns an `AudioEvent` with pitch, confidence, amplitude, and onset info.
    pub fn detect(&mut self, samples: &[f32]) -> AudioEvent {
        let amplitude = rms(samples);
        let timestamp_secs = self.timestamp;
        self.timestamp += samples.len() as f64 / self.config.sample_rate as f64;

        // Silence gate — don't bother detecting pitch in silence
        if amplitude < 0.01 {
            return AudioEvent {
                pitch_hz: None,
                confidence: 0.0,
                amplitude,
                timestamp_secs,
                is_onset: false,
            };
        }

        let (pitch_hz, confidence) = yin_pitch(samples, &self.config);

        // Simple onset detection: high confidence + sufficient amplitude
        // (will be refined in a dedicated onset module later)
        let is_onset = confidence > 0.8 && amplitude > 0.05;

        AudioEvent {
            pitch_hz,
            confidence,
            amplitude,
            timestamp_secs,
            is_onset,
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

/// YIN pitch detection algorithm.
///
/// Returns `(Some(hz), confidence)` if a pitch is found, or `(None, 0.0)` if not.
fn yin_pitch(samples: &[f32], config: &PitchConfig) -> (Option<f64>, f64) {
    let n = samples.len();
    let half = n / 2;

    let tau_min = (config.sample_rate as f64 / config.freq_max_hz) as usize;
    let tau_max = (config.sample_rate as f64 / config.freq_min_hz)
        .ceil()
        .min(half as f64) as usize;

    if tau_max <= tau_min || tau_max > half {
        return (None, 0.0);
    }

    // Step 1 & 2: Difference function + cumulative mean normalized difference
    let mut diff = vec![0.0f64; tau_max + 1];
    for tau in 1..=tau_max {
        let mut sum = 0.0f64;
        for j in 0..half {
            let d = samples[j] as f64 - samples[j + tau] as f64;
            sum += d * d;
        }
        diff[tau] = sum;
    }

    // Cumulative mean normalized difference function (CMND)
    let mut cmnd = vec![0.0f64; tau_max + 1];
    cmnd[0] = 1.0;
    let mut running_sum = 0.0;
    for tau in 1..=tau_max {
        running_sum += diff[tau];
        if running_sum > 0.0 {
            cmnd[tau] = diff[tau] * tau as f64 / running_sum;
        } else {
            cmnd[tau] = 1.0;
        }
    }

    // Step 3: Absolute threshold — find first dip below threshold
    let mut best_tau = None;
    for tau in tau_min..=tau_max {
        if cmnd[tau] < config.threshold {
            // Find the local minimum in this dip
            let mut min_tau = tau;
            while min_tau + 1 <= tau_max && cmnd[min_tau + 1] < cmnd[min_tau] {
                min_tau += 1;
            }
            best_tau = Some(min_tau);
            break;
        }
    }

    // If no dip below threshold, use the global minimum as fallback
    let best_tau = best_tau.unwrap_or_else(|| {
        (tau_min..=tau_max)
            .min_by(|&a, &b| cmnd[a].partial_cmp(&cmnd[b]).unwrap())
            .unwrap_or(tau_min)
    });

    let confidence = 1.0 - cmnd[best_tau];

    if confidence < 0.5 {
        return (None, confidence);
    }

    // Step 4: Parabolic interpolation for sub-sample accuracy
    let tau_refined = if best_tau > 0 && best_tau < tau_max {
        let alpha = cmnd[best_tau - 1];
        let beta = cmnd[best_tau];
        let gamma = cmnd[best_tau + 1];
        let adjustment = (alpha - gamma) / (2.0 * (alpha - 2.0 * beta + gamma));
        best_tau as f64 + adjustment
    } else {
        best_tau as f64
    };

    let freq = config.sample_rate as f64 / tau_refined;

    // Final range check
    if freq >= config.freq_min_hz && freq <= config.freq_max_hz {
        (Some(freq), confidence)
    } else {
        (None, confidence)
    }
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
        let mut detector = PitchDetector::new(config);
        let samples = generate_sine(440.0, 44100, detector.hop_size());

        let event = detector.detect(&samples);
        assert!(event.pitch_hz.is_some(), "Should detect pitch");
        let hz = event.pitch_hz.unwrap();
        assert!((hz - 440.0).abs() < 5.0, "Expected ~440 Hz, got {hz:.1} Hz");
        assert!(event.confidence > 0.8, "Confidence should be high");
    }

    #[test]
    fn detect_middle_c_261hz() {
        let config = PitchConfig::default();
        let mut detector = PitchDetector::new(config);
        let samples = generate_sine(261.63, 44100, detector.hop_size());

        let event = detector.detect(&samples);
        assert!(event.pitch_hz.is_some());
        let hz = event.pitch_hz.unwrap();
        assert!((hz - 261.63).abs() < 5.0, "Expected ~261.63 Hz, got {hz:.1} Hz");
    }

    #[test]
    fn silence_returns_none() {
        let config = PitchConfig::default();
        let mut detector = PitchDetector::new(config);
        let samples = vec![0.0f32; detector.hop_size()];

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
        let mut detector = PitchDetector::new(config);
        let samples = generate_sine(440.0, 44100, detector.hop_size());

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
}
