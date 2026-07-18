//! Audio output: metronome, tuning drone, and mixer.
//!
//! This module generates audio output samples for practice aids:
//! - [`Metronome`] — click track with configurable BPM, time signature, and accent.
//! - [`TuningDrone`] — continuous reference tone in sine / triangle / sawtooth.
//! - [`OutputMixer`] — sums both into a single mono stream, clipped to [-1.0, 1.0].
//!
//! All `next_sample` methods are **audio-thread safe** — they never allocate
//! and rely only on pre-initialized state. Configuration changes (which may
//! allocate or do floating-point division) go through `update_config`, which
//! is intended to be called from the UI / control thread.

use std::f64::consts::PI;

/// Errors produced by output construction and configuration updates.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OutputError {
    #[error("bpm {0} out of range (30-300)")]
    BpmOutOfRange(f64),
    #[error("frequency {0} out of range (20-2000)")]
    FrequencyOutOfRange(f64),
    #[error("invalid time signature ({0}/{1})")]
    InvalidTimeSignature(u8, u8),
    #[error("volume {0} must be in 0.0..=1.0")]
    VolumeOutOfRange(f32),
}

// ---------------------------------------------------------------------------
// Metronome
// ---------------------------------------------------------------------------

/// Configuration for a [`Metronome`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetronomeConfig {
    /// Beats per minute. Must be in 30.0..=300.0.
    pub bpm: f64,
    /// Time signature `(beats_per_measure, beat_type)`. `beat_type` must be a
    /// power of two (1, 2, 4, 8, 16, 32). Examples: `(4, 4)`, `(6, 8)`.
    pub time_signature: (u8, u8),
    /// When true, the first beat of each measure produces a higher-pitched
    /// "accent" click.
    pub accent_first_beat: bool,
    /// Output volume, 0.0..=1.0.
    pub volume: f32,
}

/// Duration of a single click in milliseconds.
const CLICK_DURATION_MS: f64 = 30.0;
/// Frequency of a normal click in Hz.
const CLICK_FREQ_HZ: f64 = 1000.0;
/// Frequency of an accent (downbeat) click in Hz.
const ACCENT_FREQ_HZ: f64 = 1500.0;
/// Exponential decay rate for the click envelope.
const CLICK_DECAY: f64 = 50.0;

/// A tempo click generator.
///
/// Emits a short decaying sine "tick" at each beat. `next_sample` is hot-path
/// safe and must be called at the audio sample rate.
#[derive(Debug, Clone)]
pub struct Metronome {
    config: MetronomeConfig,
    sample_rate: u32,
    /// Index of the *next* beat to fire within the current measure (0-indexed).
    /// `current_beat()` derives the just-fired beat from this.
    beat_index: usize,
    /// Samples remaining until the *next* beat fires. When this reaches 0 a
    /// new click is triggered and the counter reloaded.
    samples_until_next_beat: usize,
    /// Number of samples in one click window (click duration * sample_rate).
    samples_in_click: usize,
    /// Samples elapsed since the most recent click started. When
    /// `click_cursor >= samples_in_click`, silence is emitted.
    click_cursor: usize,
    /// True if the currently-playing click is an accent (downbeat).
    current_is_accent: bool,
    /// Set to true the first time a beat fires. Used so `current_beat()` can
    /// report 0 before the first click rather than `measure_len - 1`.
    any_beat_fired: bool,
    /// #421 S1: count-in beats still to play. While > 0, EVERY click uses
    /// the accent voice (the classic "1-2-3-4!" call-off); afterwards the
    /// normal accent-on-downbeat rule applies. Set via [`Self::with_count_in`].
    count_in_beats_remaining: usize,
}

impl Metronome {
    /// Create a new metronome. Validates the full config.
    ///
    /// The first click fires on the first call to [`Self::next_sample`].
    /// Subsequent clicks fire every `samples_per_beat` samples thereafter.
    pub fn new(config: MetronomeConfig, sample_rate: u32) -> Result<Self, OutputError> {
        validate_metronome_config(&config)?;
        let samples_in_click = ((CLICK_DURATION_MS / 1000.0) * sample_rate as f64) as usize;
        Ok(Self {
            config,
            sample_rate,
            beat_index: 0,
            // `0` means "fire on the next next_sample call".
            samples_until_next_beat: 0,
            samples_in_click,
            // Start past the click window so no stray click plays at
            // construction time (before the first sample is requested).
            click_cursor: samples_in_click,
            current_is_accent: false,
            any_beat_fired: false,
            count_in_beats_remaining: 0,
        })
    }

    /// #421 S1: prepend `bars` count-in bars — every click in them is the
    /// ACCENT voice, so the call-off is unmistakable. Builder-style so no
    /// existing constructor changes.
    #[must_use]
    pub fn with_count_in(mut self, bars: u8) -> Self {
        self.count_in_beats_remaining =
            usize::from(bars) * usize::from(self.config.time_signature.0);
        self
    }

    /// Number of samples between consecutive beats at the current BPM.
    ///
    /// Useful for the test suite and for UI widgets that want to display a
    /// pulsing beat indicator at the correct rate.
    pub fn samples_per_beat(&self) -> usize {
        samples_per_beat(self.config.bpm, self.sample_rate)
    }

    /// Replace the configuration. Preserves phase (click_cursor / beat_index)
    /// so changes don't pop. The time to the next beat is rescaled
    /// proportionally to the new BPM so the beat grid shifts smoothly rather
    /// than stalling or double-firing.
    pub fn update_config(&mut self, config: MetronomeConfig) -> Result<(), OutputError> {
        validate_metronome_config(&config)?;
        let old_period = samples_per_beat(self.config.bpm, self.sample_rate);
        let new_period = samples_per_beat(config.bpm, self.sample_rate);
        if old_period > 0 && self.samples_until_next_beat > 0 {
            let remaining_frac = self.samples_until_next_beat as f64 / old_period as f64;
            self.samples_until_next_beat = (remaining_frac * new_period as f64) as usize;
        }
        // If the new time signature has fewer beats, clamp beat_index.
        if self.beat_index >= config.time_signature.0 as usize {
            self.beat_index = 0;
        }
        self.config = config;
        Ok(())
    }

    /// Generate the next sample. **Hot path — no allocation.**
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.samples_until_next_beat == 0 {
            // Fire a new click.
            self.click_cursor = 0;
            self.samples_until_next_beat = samples_per_beat(self.config.bpm, self.sample_rate);
            self.current_is_accent = if self.count_in_beats_remaining > 0 {
                self.count_in_beats_remaining -= 1;
                true // every count-in click calls off in the accent voice
            } else {
                self.config.accent_first_beat && self.beat_index == 0
            };
            self.beat_index = (self.beat_index + 1) % self.config.time_signature.0 as usize;
            self.any_beat_fired = true;
        }

        let sample = if self.click_cursor < self.samples_in_click {
            let t = self.click_cursor as f64 / self.sample_rate as f64;
            let freq = if self.current_is_accent {
                ACCENT_FREQ_HZ
            } else {
                CLICK_FREQ_HZ
            };
            let envelope = (-CLICK_DECAY * t).exp() as f32;
            let tone = (2.0 * PI * freq * t).sin() as f32;
            envelope * tone * self.config.volume
        } else {
            0.0
        };

        // Wrapping_sub-style guard in case the counter was 0 above after a
        // click fire without decrement; after setting samples_until_next_beat
        // we still decrement to advance one sample.
        self.click_cursor = self.click_cursor.saturating_add(1);
        self.samples_until_next_beat = self.samples_until_next_beat.saturating_sub(1);
        sample
    }

    /// The most recently started beat within the measure (0-indexed).
    ///
    /// After a beat has fired, this returns the beat index that just played
    /// (the one the UI should highlight). In the very brief window between
    /// construction and the first sample, this returns 0 — the upcoming
    /// first beat is already known to be beat 0.
    pub fn current_beat(&self) -> usize {
        let beats = self.config.time_signature.0 as usize;
        if beats == 0 {
            return 0;
        }
        if !self.any_beat_fired {
            return 0;
        }
        // `beat_index` tracks the *next* beat to fire. After a beat fires,
        // beat_index points to the beat after it; report beat_index - 1.
        (self.beat_index + beats - 1) % beats
    }

    /// True while a click is currently being emitted (envelope is nonzero).
    pub fn is_clicking(&self) -> bool {
        self.click_cursor < self.samples_in_click
    }
}

fn samples_per_beat(bpm: f64, sample_rate: u32) -> usize {
    // Quarter-note beats: sample_rate samples per second / (bpm beats / 60s)
    // = 60 * sample_rate / bpm.
    ((60.0 * sample_rate as f64) / bpm).round() as usize
}

fn validate_metronome_config(c: &MetronomeConfig) -> Result<(), OutputError> {
    if !(30.0..=300.0).contains(&c.bpm) {
        return Err(OutputError::BpmOutOfRange(c.bpm));
    }
    let (beats, beat_type) = c.time_signature;
    if beats == 0 || beat_type == 0 {
        return Err(OutputError::InvalidTimeSignature(beats, beat_type));
    }
    if !beat_type.is_power_of_two() {
        return Err(OutputError::InvalidTimeSignature(beats, beat_type));
    }
    if !(0.0..=1.0).contains(&c.volume) {
        return Err(OutputError::VolumeOutOfRange(c.volume));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TuningDrone
// ---------------------------------------------------------------------------

/// Waveform for the tuning drone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Triangle,
    Sawtooth,
}

/// Configuration for a [`TuningDrone`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroneConfig {
    /// Fundamental frequency in Hz. Must be in 20.0..=2000.0.
    pub frequency_hz: f64,
    pub waveform: Waveform,
    /// Output volume, 0.0..=1.0.
    pub volume: f32,
}

/// A continuous-tone drone for tuning against.
#[derive(Debug, Clone)]
pub struct TuningDrone {
    config: DroneConfig,
    sample_rate: u32,
    /// Phase accumulator in [0.0, 1.0). Not reset on `update_config` so that
    /// frequency changes don't introduce pops.
    phase: f64,
}

impl TuningDrone {
    pub fn new(config: DroneConfig, sample_rate: u32) -> Result<Self, OutputError> {
        validate_drone_config(&config)?;
        Ok(Self {
            config,
            sample_rate,
            phase: 0.0,
        })
    }

    /// Replace the configuration. Phase is preserved to avoid audible pops.
    pub fn update_config(&mut self, config: DroneConfig) -> Result<(), OutputError> {
        validate_drone_config(&config)?;
        self.config = config;
        Ok(())
    }

    /// Generate the next sample. **Hot path — no allocation.**
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let phase = self.phase;
        let raw = match self.config.waveform {
            Waveform::Sine => (2.0 * PI * phase).sin(),
            Waveform::Triangle => {
                // 2 * |2 * (phase - floor(phase + 0.5))| - 1
                2.0 * (2.0 * (phase - (phase + 0.5).floor())).abs() - 1.0
            }
            Waveform::Sawtooth => 2.0 * (phase - (phase + 0.5).floor()),
        };

        // Advance phase (keep in [0, 1) to avoid float precision drift).
        let increment = self.config.frequency_hz / self.sample_rate as f64;
        self.phase += increment;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
        }

        (raw as f32) * self.config.volume
    }
}

fn validate_drone_config(c: &DroneConfig) -> Result<(), OutputError> {
    if !(20.0..=2000.0).contains(&c.frequency_hz) {
        return Err(OutputError::FrequencyOutOfRange(c.frequency_hz));
    }
    if !(0.0..=1.0).contains(&c.volume) {
        return Err(OutputError::VolumeOutOfRange(c.volume));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Concert pitch presets
// ---------------------------------------------------------------------------

/// Preset fundamental frequencies for common tuning references.
///
/// These correspond to the concert pitch (i.e., pitch in C) for transposing
/// instruments. For example, a Bb trumpet reading "C" sounds concert Bb.
pub struct ConcertPitch;

impl ConcertPitch {
    /// Bb3 = 233.08 Hz (concert pitch for Bb trumpet, tenor sax, trombone).
    pub fn bb_concert() -> f64 {
        233.08
    }

    /// A4 = 440.0 Hz (standard concert pitch).
    pub fn a_concert() -> f64 {
        440.0
    }

    /// F4 = 349.23 Hz (concert pitch for French horn).
    pub fn f_concert() -> f64 {
        349.23
    }

    /// C4 = 261.63 Hz (middle C).
    pub fn c_concert() -> f64 {
        261.63
    }

    /// D4 = 293.66 Hz.
    pub fn d_concert() -> f64 {
        293.66
    }

    /// Eb4 = 311.13 Hz (concert pitch for Eb alto sax, Eb clarinet).
    pub fn eb_concert() -> f64 {
        311.13
    }

    /// A4 with an offset in cents. `from_a4(0.0) == 440.0`.
    ///
    /// Formula: `440 * 2^(cents / 1200)`.
    pub fn from_a4(offset_cents: f64) -> f64 {
        440.0 * 2f64.powf(offset_cents / 1200.0)
    }
}

// ---------------------------------------------------------------------------
// OutputMixer
// ---------------------------------------------------------------------------

/// Mixes [`Metronome`] and [`TuningDrone`] into a single mono stream.
///
/// Sources are summed and clipped to [-1.0, 1.0]. The mixer has a master
/// volume applied after summing but before clipping.
#[derive(Debug, Clone)]
pub struct OutputMixer {
    metronome: Option<Metronome>,
    drone: Option<TuningDrone>,
    master_volume: f32,
}

impl OutputMixer {
    pub fn new() -> Self {
        Self {
            metronome: None,
            drone: None,
            master_volume: 1.0,
        }
    }

    pub fn set_metronome(&mut self, m: Option<Metronome>) {
        self.metronome = m;
    }

    pub fn set_drone(&mut self, d: Option<TuningDrone>) {
        self.drone = d;
    }

    /// Set the master volume. Returns `Err` if not in 0.0..=1.0.
    pub fn set_master_volume(&mut self, v: f32) -> Result<(), OutputError> {
        if !(0.0..=1.0).contains(&v) {
            return Err(OutputError::VolumeOutOfRange(v));
        }
        self.master_volume = v;
        Ok(())
    }

    /// Generate the next mixed sample. **Hot path — no allocation.**
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let mut sum = 0.0_f32;
        if let Some(m) = self.metronome.as_mut() {
            sum += m.next_sample();
        }
        if let Some(d) = self.drone.as_mut() {
            sum += d.next_sample();
        }
        sum *= self.master_volume;
        sum.clamp(-1.0, 1.0)
    }

    /// True if at least one source is attached.
    pub fn has_active_source(&self) -> bool {
        self.metronome.is_some() || self.drone.is_some()
    }
}

impl Default for OutputMixer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;
    const EPS: f32 = 1e-6;

    fn metro(bpm: f64) -> MetronomeConfig {
        MetronomeConfig {
            bpm,
            time_signature: (4, 4),
            accent_first_beat: true,
            volume: 0.8,
        }
    }

    fn drone(freq: f64, wf: Waveform) -> DroneConfig {
        DroneConfig {
            frequency_hz: freq,
            waveform: wf,
            volume: 0.5,
        }
    }

    // ---- Metronome ----

    #[test]
    fn metronome_rejects_bpm_below_range() {
        let err = Metronome::new(metro(29.0), SR).unwrap_err();
        assert_eq!(err, OutputError::BpmOutOfRange(29.0));
    }

    #[test]
    fn metronome_rejects_bpm_above_range() {
        let err = Metronome::new(metro(301.0), SR).unwrap_err();
        assert_eq!(err, OutputError::BpmOutOfRange(301.0));
    }

    #[test]
    fn metronome_rejects_zero_bpm() {
        let err = Metronome::new(metro(0.0), SR).unwrap_err();
        assert!(matches!(err, OutputError::BpmOutOfRange(_)));
    }

    #[test]
    fn metronome_accepts_boundary_bpm() {
        assert!(Metronome::new(metro(30.0), SR).is_ok());
        assert!(Metronome::new(metro(300.0), SR).is_ok());
    }

    #[test]
    fn metronome_beat_interval_matches_bpm() {
        // 120 BPM at 44100 sample rate: 60 * 44100 / 120 = 22050 samples per beat.
        let m = Metronome::new(metro(120.0), 44_100).unwrap();
        assert_eq!(m.samples_per_beat(), 22_050);

        // At 60 BPM: 60 * 44100 / 60 = 44100 samples per beat.
        let m60 = Metronome::new(metro(60.0), 44_100).unwrap();
        assert_eq!(m60.samples_per_beat(), 44_100);
    }

    #[test]
    fn metronome_produces_nonzero_clicks() {
        // 120 BPM = 2 beats/sec; 2 seconds = 4 beats -> 4 clicks.
        let mut m = Metronome::new(metro(120.0), 44_100).unwrap();
        let total_samples = 2 * 44_100;
        let mut nonzero = 0usize;
        for _ in 0..total_samples {
            if m.next_sample().abs() > 1e-5 {
                nonzero += 1;
            }
        }
        // Each click is ~30 ms = ~1323 samples. 4 clicks -> ~5292 samples.
        // Allow a wide band because the tail of the envelope is small-but-nonzero.
        assert!(
            (4 * 800..=4 * 1500).contains(&nonzero),
            "expected ~4 click windows of nonzero audio, got {nonzero} nonzero samples",
        );
    }

    #[test]
    fn metronome_zero_volume_produces_silence() {
        let mut cfg = metro(120.0);
        cfg.volume = 0.0;
        let mut m = Metronome::new(cfg, SR).unwrap();
        for _ in 0..SR {
            assert_eq!(m.next_sample(), 0.0);
        }
    }

    #[test]
    fn metronome_current_beat_is_zero_before_first_sample() {
        let m = Metronome::new(metro(120.0), SR).unwrap();
        assert_eq!(m.current_beat(), 0);
        assert!(!m.is_clicking());
    }

    #[test]
    fn metronome_beat_index_cycles_with_signature() {
        // Run an entire 4/4 measure at 120 BPM. After each beat fires,
        // `current_beat()` should report the just-fired beat index.
        let mut m = Metronome::new(metro(120.0), 44_100).unwrap();
        let samples_per_beat = 22_050;
        let mut observed = Vec::new();
        // Advance through 4 full beats; record current_beat immediately after
        // each beat fires (one sample in).
        for beat in 0..4 {
            // Step to exactly one sample after the beat fires.
            for _ in 0..samples_per_beat {
                let _ = m.next_sample();
            }
            observed.push((beat, m.current_beat()));
        }
        // current_beat reports the most-recently-started beat: after beats 0..4,
        // the most-recent indices should be 0, 1, 2, 3.
        let expected: Vec<(usize, usize)> = (0..4).map(|i| (i, i)).collect();
        assert_eq!(observed, expected);
    }

    #[test]
    fn metronome_time_signature_658_handles_six_beats() {
        let cfg = MetronomeConfig {
            bpm: 120.0,
            time_signature: (6, 8),
            accent_first_beat: true,
            volume: 0.5,
        };
        let mut m = Metronome::new(cfg, 44_100).unwrap();
        let samples_per_beat = 22_050;
        // Advance through 7 beats; beat_index should cycle 0..6 and wrap.
        let mut indices = Vec::new();
        for _ in 0..7 {
            for _ in 0..samples_per_beat {
                let _ = m.next_sample();
            }
            indices.push(m.current_beat());
        }
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5, 0]);
    }

    #[test]
    fn metronome_invalid_time_signature_rejected() {
        let cfg_zero = MetronomeConfig {
            bpm: 120.0,
            time_signature: (0, 4),
            accent_first_beat: true,
            volume: 0.5,
        };
        assert!(matches!(
            Metronome::new(cfg_zero, SR),
            Err(OutputError::InvalidTimeSignature(0, 4))
        ));

        // 4/3 — beat_type is not a power of 2.
        let cfg_non_pow2 = MetronomeConfig {
            bpm: 120.0,
            time_signature: (4, 3),
            accent_first_beat: true,
            volume: 0.5,
        };
        assert!(matches!(
            Metronome::new(cfg_non_pow2, SR),
            Err(OutputError::InvalidTimeSignature(4, 3))
        ));
    }

    #[test]
    fn metronome_update_config_validates_new_bpm() {
        let mut m = Metronome::new(metro(120.0), SR).unwrap();
        let err = m.update_config(metro(1000.0)).unwrap_err();
        assert_eq!(err, OutputError::BpmOutOfRange(1000.0));
    }

    // ---- TuningDrone ----

    #[test]
    fn drone_rejects_frequency_below_range() {
        let err = TuningDrone::new(drone(19.0, Waveform::Sine), SR).unwrap_err();
        assert_eq!(err, OutputError::FrequencyOutOfRange(19.0));
    }

    #[test]
    fn drone_rejects_frequency_above_range() {
        let err = TuningDrone::new(drone(2001.0, Waveform::Sine), SR).unwrap_err();
        assert_eq!(err, OutputError::FrequencyOutOfRange(2001.0));
    }

    #[test]
    fn drone_sine_at_440hz_crosses_zero_440_times_per_second() {
        // Two zero crossings per cycle, 440 cycles -> 880 crossings (±2 tol).
        let cfg = DroneConfig {
            frequency_hz: 440.0,
            waveform: Waveform::Sine,
            volume: 1.0,
        };
        let mut d = TuningDrone::new(cfg, 44_100).unwrap();
        let mut prev = d.next_sample();
        let mut crossings = 0usize;
        for _ in 1..44_100 {
            let s = d.next_sample();
            if (prev >= 0.0 && s < 0.0) || (prev < 0.0 && s >= 0.0) {
                crossings += 1;
            }
            prev = s;
        }
        assert!(
            crossings.abs_diff(880) <= 2,
            "expected ~880 zero crossings for 440Hz sine, got {}",
            crossings
        );
    }

    #[test]
    fn drone_triangle_waveform_stays_in_minus1_to_1() {
        let cfg = DroneConfig {
            frequency_hz: 220.0,
            waveform: Waveform::Triangle,
            volume: 1.0,
        };
        let mut d = TuningDrone::new(cfg, 44_100).unwrap();
        for _ in 0..10_000 {
            let s = d.next_sample();
            assert!(
                (-1.0..=1.0).contains(&s),
                "triangle sample out of range: {}",
                s
            );
        }
    }

    #[test]
    fn drone_sawtooth_waveform_stays_in_minus1_to_1() {
        let cfg = DroneConfig {
            frequency_hz: 220.0,
            waveform: Waveform::Sawtooth,
            volume: 1.0,
        };
        let mut d = TuningDrone::new(cfg, 44_100).unwrap();
        for _ in 0..10_000 {
            let s = d.next_sample();
            assert!(
                (-1.0..=1.0).contains(&s),
                "sawtooth sample out of range: {}",
                s
            );
        }
    }

    #[test]
    fn drone_zero_volume_silent() {
        let cfg = DroneConfig {
            frequency_hz: 440.0,
            waveform: Waveform::Sine,
            volume: 0.0,
        };
        let mut d = TuningDrone::new(cfg, SR).unwrap();
        for _ in 0..SR {
            assert_eq!(d.next_sample(), 0.0);
        }
    }

    #[test]
    fn drone_update_config_changes_frequency_smoothly() {
        // Advance a quarter cycle at 440 Hz, then swap to 220 Hz. The phase
        // should NOT reset — continuity means the next sample is close to the
        // previous one (no discontinuity pop).
        let mut d = TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap();
        // Advance some samples to move phase away from 0.
        for _ in 0..100 {
            let _ = d.next_sample();
        }
        let before = d.phase;
        d.update_config(drone(220.0, Waveform::Sine)).unwrap();
        let after = d.phase;
        assert!(
            (before - after).abs() < f64::EPSILON,
            "phase should be preserved across update_config ({} -> {})",
            before,
            after
        );
    }

    #[test]
    fn drone_update_config_validates() {
        let mut d = TuningDrone::new(drone(440.0, Waveform::Sine), SR).unwrap();
        let err = d
            .update_config(drone(10_000.0, Waveform::Sine))
            .unwrap_err();
        assert_eq!(err, OutputError::FrequencyOutOfRange(10_000.0));
    }

    // ---- ConcertPitch ----

    #[test]
    fn bb_concert_is_2330_hz() {
        assert!((ConcertPitch::bb_concert() - 233.08).abs() < 0.1);
    }

    #[test]
    fn a_concert_is_440() {
        assert_eq!(ConcertPitch::a_concert(), 440.0);
    }

    #[test]
    fn from_a4_zero_cents_is_440() {
        assert!((ConcertPitch::from_a4(0.0) - 440.0).abs() < 1e-9);
    }

    #[test]
    fn from_a4_plus_100_cents_is_a_sharp4() {
        // A#4 ≈ 466.16 Hz.
        assert!((ConcertPitch::from_a4(100.0) - 466.16).abs() < 0.1);
    }

    #[test]
    fn from_a4_minus_100_cents_is_ab4() {
        // Ab4 ≈ 415.30 Hz.
        assert!((ConcertPitch::from_a4(-100.0) - 415.30).abs() < 0.1);
    }

    // ---- OutputMixer ----

    #[test]
    fn mixer_with_nothing_produces_silence() {
        let mut mixer = OutputMixer::new();
        for _ in 0..1000 {
            assert_eq!(mixer.next_sample(), 0.0);
        }
    }

    #[test]
    fn mixer_sums_metronome_and_drone() {
        // Construct a mixer with only a drone at 440 Hz and a metronome at
        // 120 BPM. Construct separate instances to compare.
        let mut reference_drone = TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap();
        let mut reference_metro = Metronome::new(metro(120.0), 44_100).unwrap();

        let mut mixer = OutputMixer::new();
        mixer.set_drone(Some(
            TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap(),
        ));
        mixer.set_metronome(Some(Metronome::new(metro(120.0), 44_100).unwrap()));

        // Check across many samples including a click.
        for _ in 0..5000 {
            let expected_raw = reference_drone.next_sample() + reference_metro.next_sample();
            let expected = expected_raw.clamp(-1.0, 1.0);
            let actual = mixer.next_sample();
            assert!(
                (expected - actual).abs() < EPS,
                "mixer sum mismatch: expected {}, got {}",
                expected,
                actual
            );
        }
    }

    #[test]
    fn mixer_clips_to_range() {
        // Use two loud drones (via direct construction impossible — only one
        // drone slot). Instead, crank master volume to push a full-scale
        // sample (drone at vol 1.0 sine = up to 1.0) past 1.0 with a
        // metronome click stacked on top.
        let mut mixer = OutputMixer::new();
        mixer.set_drone(Some(
            TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap(),
        ));
        // Use a loud metronome volume so clicks push above 1.0.
        let mut loud = metro(120.0);
        loud.volume = 1.0;
        mixer.set_metronome(Some(Metronome::new(loud, 44_100).unwrap()));
        // Master volume at 1.0 — summed inputs already can exceed 1.0.
        for _ in 0..10_000 {
            let s = mixer.next_sample();
            assert!(
                (-1.0..=1.0).contains(&s),
                "mixer output clipped out of [-1, 1]: {}",
                s
            );
        }
    }

    #[test]
    fn mixer_master_volume_scales_output() {
        // Full-vol vs half-vol drone — RMS should halve.
        let drone_full = TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap();
        let drone_half = TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap();

        let mut full = OutputMixer::new();
        full.set_drone(Some(drone_full));
        let mut half = OutputMixer::new();
        half.set_drone(Some(drone_half));
        half.set_master_volume(0.5).unwrap();

        let mut sum_sq_full = 0.0_f64;
        let mut sum_sq_half = 0.0_f64;
        let n = 44_100;
        for _ in 0..n {
            let a = full.next_sample() as f64;
            let b = half.next_sample() as f64;
            sum_sq_full += a * a;
            sum_sq_half += b * b;
        }
        let rms_full = (sum_sq_full / n as f64).sqrt();
        let rms_half = (sum_sq_half / n as f64).sqrt();
        // Halved master volume -> halved amplitude -> halved RMS (tolerance 2%).
        assert!(
            (rms_half / rms_full - 0.5).abs() < 0.02,
            "expected rms ratio 0.5, got {}",
            rms_half / rms_full
        );
    }

    #[test]
    fn mixer_master_volume_zero_silences_everything() {
        let mut mixer = OutputMixer::new();
        mixer.set_drone(Some(
            TuningDrone::new(drone(440.0, Waveform::Sine), 44_100).unwrap(),
        ));
        mixer.set_metronome(Some(Metronome::new(metro(120.0), 44_100).unwrap()));
        mixer.set_master_volume(0.0).unwrap();
        for _ in 0..SR {
            assert_eq!(mixer.next_sample(), 0.0);
        }
    }

    #[test]
    fn mixer_rejects_invalid_volume() {
        let mut mixer = OutputMixer::new();
        assert_eq!(
            mixer.set_master_volume(-0.1),
            Err(OutputError::VolumeOutOfRange(-0.1))
        );
        assert_eq!(
            mixer.set_master_volume(1.1),
            Err(OutputError::VolumeOutOfRange(1.1))
        );
    }

    #[test]
    fn mixer_has_active_source_reflects_state() {
        let mut mixer = OutputMixer::new();
        assert!(!mixer.has_active_source());
        mixer.set_metronome(Some(Metronome::new(metro(120.0), SR).unwrap()));
        assert!(mixer.has_active_source());
        mixer.set_metronome(None);
        assert!(!mixer.has_active_source());
        mixer.set_drone(Some(
            TuningDrone::new(drone(440.0, Waveform::Sine), SR).unwrap(),
        ));
        assert!(mixer.has_active_source());
    }

    // ---- Errors ----

    #[test]
    fn output_error_volume_out_of_range_displayed() {
        let err = OutputError::VolumeOutOfRange(1.5);
        let msg = format!("{}", err);
        assert!(msg.contains("1.5"), "error message missing value: {}", msg);
        assert!(
            msg.contains("0.0..=1.0") || msg.contains("0.0") && msg.contains("1.0"),
            "error message missing range: {}",
            msg
        );
    }

    // ── #421 S1: count-in ───────────────────────────────────────────────

    /// Walk one beat's worth of samples and report whether the click that
    /// fired at its start was the accent voice (detected by frequency: the
    /// accent is 1500 Hz, normal is 1000 Hz — compare early-sample phase).
    fn fired_accent(m: &mut Metronome) -> bool {
        let period = m.samples_per_beat();
        // The click fires on the first sample of the beat. A sine goes
        // negative after its half-period: 1500 Hz (accent) at 48 kHz →
        // 16 samples; 1000 Hz (normal) → 24. The first negative sample
        // index cleanly separates the two voices.
        let mut first_negative = None;
        for i in 0..period {
            let s = m.next_sample();
            if first_negative.is_none() && s < 0.0 {
                first_negative = Some(i);
            }
        }
        // Threshold derived from the voices themselves: midway between the
        // two half-periods, so a legitimate retuning of either constant
        // moves the boundary instead of silently breaking the detector.
        let accent_half = (48_000.0 / ACCENT_FREQ_HZ / 2.0) as usize;
        let normal_half = (48_000.0 / CLICK_FREQ_HZ / 2.0) as usize;
        let threshold = (accent_half + normal_half) / 2;
        first_negative.expect("click produced a negative half-cycle") < threshold
    }

    #[test]
    fn count_in_bars_call_off_in_the_accent_voice() {
        let config = MetronomeConfig {
            bpm: 120.0,
            time_signature: (4, 4),
            accent_first_beat: true,
            volume: 1.0,
        };
        let mut m = Metronome::new(config, 48_000).unwrap().with_count_in(1);
        // Bar 1 (count-in): all four clicks accent.
        for beat in 0..4 {
            assert!(fired_accent(&mut m), "count-in beat {beat} must accent");
        }
        // Bar 2 (live): only beat 1 accents.
        assert!(fired_accent(&mut m), "live downbeat accents");
        for beat in 1..4 {
            assert!(
                !fired_accent(&mut m),
                "live beat {beat} must be the normal click"
            );
        }
    }

    #[test]
    fn zero_count_in_is_todays_behavior() {
        let config = MetronomeConfig {
            bpm: 120.0,
            time_signature: (4, 4),
            accent_first_beat: true,
            volume: 1.0,
        };
        let mut plain = Metronome::new(config, 48_000).unwrap();
        let mut with_zero = Metronome::new(config, 48_000).unwrap().with_count_in(0);
        for _ in 0..48_000 {
            assert_eq!(plain.next_sample(), with_zero.next_sample());
        }
    }
}
