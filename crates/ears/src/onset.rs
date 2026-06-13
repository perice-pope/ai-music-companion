//! SuperFlux onset detection (Böck & Widmer, 2013).
//!
//! A spectral-flux onset detector whose defining trick is a **max filter over
//! frequency** applied to the reference frame: it absorbs the small bin-to-bin
//! shifts a vibrato produces, so a steady vibrato no longer reads as a stream
//! of false onsets — the exact failure mode of the old voiced-edge heuristic
//! (#138, which fired whenever YIN confidence flickered across a note).
//!
//! Per analysis frame (one [`crate::pitch::PitchDetector`] window):
//! ```text
//!   Hann window → FFT → log-magnitude spectrum
//!     → flux = Σ_k max(0, mag[k] − maxfilt(prev_mag)[k])   (positive energy only)
//!     → online peak-pick: a rising flux above an adaptive (moving-average)
//!       threshold, debounced by a refractory gap
//! ```
//!
//! Runs on the processing thread (alongside pitch detection), **not** the
//! realtime audio callback. All scratch is pre-allocated, so [`process`] does
//! no heap allocation.
//!
//! [`process`]: SuperFluxOnset::process
//!
//! **Tuning caveat:** the thresholds and filter widths below are literature
//! defaults. Final tuning wants real-instrument audio on device — CI exercises
//! only synthetic fixtures (a transient fires, a vibrato doesn't), which proves
//! the *shape* of the behaviour, not production-grade sensitivity.

use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

/// Tunables for [`SuperFluxOnset`].
#[derive(Debug, Clone)]
pub struct OnsetConfig {
    /// Half-width (in frequency bins) of the max filter applied to the
    /// reference frame. Wider → more vibrato tolerance. SuperFlux uses ~3.
    pub max_filter_bins: usize,
    /// Number of recent ODF values averaged for the adaptive threshold.
    pub avg_window: usize,
    /// An onset requires `flux > mean(recent_odf) * mult + delta`.
    pub threshold_mult: f32,
    /// Additive margin on the adaptive threshold (absolute flux units).
    pub threshold_delta: f32,
    /// Scale-relative floor: the threshold is never below this fraction of the
    /// largest flux seen this session. Keeps a held tone's near-zero ODF ripple
    /// from tripping the (otherwise vanishing) adaptive threshold once the
    /// note-start spike rolls out of the moving average. Auto-scales to input
    /// level, so it needs no absolute calibration.
    pub floor_frac: f32,
    /// Minimum frames between consecutive onsets (debounce).
    pub refractory_frames: usize,
}

impl Default for OnsetConfig {
    fn default() -> Self {
        Self {
            max_filter_bins: 3,
            avg_window: 8,
            threshold_mult: 3.0,
            threshold_delta: 0.0,
            floor_frac: 0.15,
            refractory_frames: 2,
        }
    }
}

/// Online SuperFlux onset detector. Construct once per session (it carries
/// frame-to-frame state); feed each analysis window to [`process`].
///
/// [`process`]: SuperFluxOnset::process
pub struct SuperFluxOnset {
    fft: Arc<dyn Fft<f32>>,
    n_bins: usize,
    hann: Vec<f32>,

    // Pre-allocated scratch (no alloc in `process`).
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    cur_mag: Vec<f32>,
    /// Reference (previous) frame's log-magnitude. Initialised to zeros so the
    /// first audible frame after construction/reset reads as silence→sound,
    /// i.e. a strong flux → an onset on the note's first frame.
    prev_mag: Vec<f32>,

    // Peak-pick state. We detect a true local maximum of the ODF, which needs
    // the two preceding values — so onsets are reported with one frame of
    // latency (the peak is confirmed once the next, lower, frame arrives). A
    // consistent one-frame (~hop) offset doesn't affect inter-onset intervals,
    // which is what the groove analysis uses.
    odf_hist: Vec<f32>,
    odf_idx: usize,
    odf_filled: usize,
    /// ODF two frames ago, one frame ago, and the adaptive threshold that
    /// applied to the one-frame-ago value (so the peak is judged against the
    /// history that preceded it).
    odf_p2: f32,
    odf_p1: f32,
    thresh_p1: f32,
    /// Largest flux seen this session, for the scale-relative threshold floor.
    max_flux: f32,
    frames_since_onset: usize,

    config: OnsetConfig,
}

impl SuperFluxOnset {
    /// Build a detector for the given analysis-window size. The FFT runs at the
    /// next power of two ≥ `window_size` (zero-padded) for speed.
    pub fn new(window_size: usize, config: OnsetConfig) -> Self {
        let fft_size = window_size.next_power_of_two().max(2);
        let fft = FftPlanner::new().plan_fft_forward(fft_size);
        let scratch_len = fft.get_inplace_scratch_len();
        let n_bins = fft_size / 2 + 1;

        // Hann window: sin²(πi/(N−1)).
        let denom = (window_size.max(2) - 1) as f32;
        let hann: Vec<f32> = (0..window_size)
            .map(|i| (std::f32::consts::PI * i as f32 / denom).sin().powi(2))
            .collect();

        Self {
            fft,
            n_bins,
            hann,
            buf: vec![Complex::new(0.0, 0.0); fft_size],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            cur_mag: vec![0.0; n_bins],
            prev_mag: vec![0.0; n_bins],
            odf_hist: vec![0.0; config.avg_window.max(1)],
            odf_idx: 0,
            odf_filled: 0,
            odf_p2: 0.0,
            odf_p1: 0.0,
            thresh_p1: 0.0,
            max_flux: 0.0,
            // Large so the first frame is never suppressed by the refractory gap.
            frames_since_onset: usize::MAX / 2,
            config,
        }
    }

    /// Reset frame-to-frame state. Call at the start of a session and after a
    /// run of silence, so the next audible frame reads as a fresh onset.
    pub fn reset(&mut self) {
        for m in self.prev_mag.iter_mut() {
            *m = 0.0;
        }
        self.odf_idx = 0;
        self.odf_filled = 0;
        self.odf_p2 = 0.0;
        self.odf_p1 = 0.0;
        self.thresh_p1 = 0.0;
        self.max_flux = 0.0;
        self.frames_since_onset = usize::MAX / 2;
    }

    /// Feed one analysis frame; returns `true` when an onset is detected on it.
    /// **Zero heap allocations.**
    pub fn process(&mut self, samples: &[f32]) -> bool {
        // Windowed real input → FFT buffer, zero-padded to `fft_size`.
        let n = samples.len().min(self.hann.len());
        for (i, slot) in self.buf.iter_mut().enumerate() {
            let re = if i < n {
                samples[i] * self.hann[i]
            } else {
                0.0
            };
            *slot = Complex::new(re, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buf, &mut self.scratch);

        // Log-magnitude spectrum over the unique (real-input) bins.
        for k in 0..self.n_bins {
            self.cur_mag[k] = (1.0 + self.buf[k].norm()).ln();
        }

        // Max-filtered spectral flux: only energy that *appeared* relative to a
        // frequency-max-filtered previous frame counts. The max filter is what
        // makes a vibrato's small bin wobble not register as an onset.
        let w = self.config.max_filter_bins;
        let mut flux = 0.0f32;
        for k in 0..self.n_bins {
            let lo = k.saturating_sub(w);
            let hi = (k + w + 1).min(self.n_bins);
            let mut prev_max = f32::NEG_INFINITY;
            for &p in &self.prev_mag[lo..hi] {
                if p > prev_max {
                    prev_max = p;
                }
            }
            let d = self.cur_mag[k] - prev_max;
            if d > 0.0 {
                flux += d;
            }
        }

        std::mem::swap(&mut self.cur_mag, &mut self.prev_mag);
        self.frames_since_onset = self.frames_since_onset.saturating_add(1);

        // Adaptive threshold for *this* frame's flux, from the recent ODF
        // history (the past only — before this frame is folded in).
        let mean = if self.odf_filled == 0 {
            0.0
        } else {
            self.odf_hist[..self.odf_filled].iter().sum::<f32>() / self.odf_filled as f32
        };
        let floor = self.config.floor_frac * self.max_flux;
        let threshold =
            (mean * self.config.threshold_mult + self.config.threshold_delta).max(floor);

        // Peak-pick on a true local maximum: the previous frame is an onset when
        // it rose above its predecessor and the current frame falls back from it
        // (`odf_p2 < odf_p1 >= flux`), it cleared the threshold that applied to
        // it, and we're past the refractory gap. Requiring a real peak — rather
        // than any rising frame — is what stops a vibrato's low ODF ripple from
        // registering as a stream of onsets. Costs one frame of latency.
        //
        // The buffers start at 0, so frame 0 can't peak (0 > 0 is false) and the
        // note's first frame is confirmed as a peak on frame 1 — the intended
        // one-frame latency, with its threshold (`thresh_p1`) being the empty-
        // history 0 so a note start always clears it.
        let is_onset = self.odf_p1 > self.odf_p2
            && self.odf_p1 >= flux
            && self.odf_p1 > self.thresh_p1
            && self.frames_since_onset >= self.config.refractory_frames;

        // Advance the peak-pick window and fold this frame into the history.
        self.odf_p2 = self.odf_p1;
        self.odf_p1 = flux;
        self.thresh_p1 = threshold;
        self.max_flux = self.max_flux.max(flux);
        self.odf_hist[self.odf_idx] = flux;
        self.odf_idx = (self.odf_idx + 1) % self.odf_hist.len();
        if self.odf_filled < self.odf_hist.len() {
            self.odf_filled += 1;
        }

        if is_onset {
            self.frames_since_onset = 0;
        }
        is_onset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;
    const WIN: usize = 1470; // ~2 periods of 60 Hz, the detector's default window
    /// Onsets are reported with one frame of latency (local-max peak-pick), so
    /// "the start" means an onset within the first couple of frames.
    const START_FRAMES: usize = 2;

    /// A continuous buffer of `frames` windows whose instantaneous frequency is
    /// `freq_at(time_secs)`, with phase accumulated so consecutive windows join
    /// smoothly — i.e. real, artifact-free audio rather than independently
    /// generated frames whose phase jumps would themselves look like transients.
    fn continuous(frames: usize, mut freq_at: impl FnMut(f32) -> f32) -> Vec<f32> {
        let mut buf = vec![0.0f32; WIN * frames];
        let mut phase = 0.0f32;
        for (i, s) in buf.iter_mut().enumerate() {
            phase += 2.0 * std::f32::consts::PI * freq_at(i as f32 / SR) / SR;
            *s = phase.sin();
        }
        buf
    }

    /// Frame indices on which the detector reported an onset.
    fn onset_frames(od: &mut SuperFluxOnset, buf: &[f32]) -> Vec<usize> {
        buf.chunks(WIN)
            .enumerate()
            .filter(|(_, w)| w.len() == WIN)
            .filter_map(|(i, w)| od.process(w).then_some(i))
            .collect()
    }

    #[test]
    fn note_start_produces_an_onset() {
        let mut od = SuperFluxOnset::new(WIN, OnsetConfig::default());
        let onsets = onset_frames(&mut od, &continuous(6, |_| 440.0));
        assert!(
            onsets.iter().any(|&i| i <= START_FRAMES),
            "the note start should fire an onset, got {onsets:?}"
        );
    }

    #[test]
    fn steady_tone_fires_once_then_stays_quiet() {
        let mut od = SuperFluxOnset::new(WIN, OnsetConfig::default());
        let onsets = onset_frames(&mut od, &continuous(20, |_| 440.0));
        assert!(!onsets.is_empty(), "the start should fire");
        assert!(
            onsets.iter().all(|&i| i <= START_FRAMES),
            "a held tone must not re-fire after the start, got {onsets:?}"
        );
    }

    #[test]
    fn vibrato_does_not_re_fire() {
        // ±6 Hz, 5 Hz vibrato — the #138 regression. After the note start, the
        // local-max peak-pick plus the frequency max filter must yield no
        // further onsets (the old voiced-edge detector fired on every flicker).
        let mut od = SuperFluxOnset::new(WIN, OnsetConfig::default());
        let buf = continuous(30, |t| {
            440.0 + 6.0 * (2.0 * std::f32::consts::PI * 5.0 * t).sin()
        });
        let onsets = onset_frames(&mut od, &buf);
        assert!(
            onsets.iter().all(|&i| i <= START_FRAMES),
            "vibrato must not be heard as repeated onsets, got {onsets:?}"
        );
    }

    #[test]
    fn a_new_note_fires_a_second_onset() {
        // 440 for 10 frames, then 660: expect the start onset plus another near
        // the change (the new partials appearing produce a flux spike).
        let mut od = SuperFluxOnset::new(WIN, OnsetConfig::default());
        let switch_secs = (WIN * 10) as f32 / SR;
        let buf = continuous(20, |t| if t < switch_secs { 440.0 } else { 660.0 });
        let onsets = onset_frames(&mut od, &buf);
        assert!(
            onsets.iter().any(|&i| i <= START_FRAMES),
            "note start fires, got {onsets:?}"
        );
        assert!(
            onsets.iter().any(|&i| (10..=12).contains(&i)),
            "the new note should fire an onset around the change, got {onsets:?}"
        );
    }

    #[test]
    fn reset_lets_the_next_note_onset_again() {
        let mut od = SuperFluxOnset::new(WIN, OnsetConfig::default());
        let _ = onset_frames(&mut od, &continuous(10, |_| 440.0));
        od.reset(); // as the detector does after a run of silence
        let after = onset_frames(&mut od, &continuous(6, |_| 440.0));
        assert!(
            after.iter().any(|&i| i <= START_FRAMES),
            "after a reset the next note is a fresh onset, got {after:?}"
        );
    }
}
