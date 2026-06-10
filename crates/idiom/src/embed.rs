//! Audio embedding: the pluggable [`IdiomEmbedder`] trait and a working,
//! deterministic [`BaselineEmbedder`].
//!
//! ## Off the hot path — allocation is fine here
//!
//! Idiom recognition is **offline, reflective analysis** (end-of-phrase recap,
//! the "what was that?" moment), *not* part of the <25 ms mic-to-screen audio
//! path described in `CLAUDE.md`. So unlike `crates/ears`, this code is free to
//! allocate (`Vec`, FFT planners, etc.); the audio-thread no-allocation rule
//! does not apply.
//!
//! ## Baseline now, MERT-class model later
//!
//! [`IdiomEmbedder`] is the seam. Today the only implementation is
//! [`BaselineEmbedder`], which computes a real, deterministic feature vector
//! (chroma + MFCC-style spectral statistics) directly from audio with classic
//! DSP. It is honestly *not* state of the art — it captures harmonic/timbral
//! colour, not the swing-feel-and-articulation nuance the design doc wants.
//!
//! The **intended future drop-in** is a music foundation model such as
//! **MERT** (`docs/design/story-phase4-musical-relevance.md` §3B.1), run
//! offline through ONNX Runtime exactly like `crates/transcribe` runs
//! basic-pitch. When that lands it implements the *same* [`IdiomEmbedder`]
//! trait and the retrieval code in [`crate::retrieve`] is unchanged — only the
//! corpus embeddings get regenerated with the new encoder. The baseline exists
//! so the engine, the corpus format, and the confidence-gated retrieval are
//! real and testable today, without waiting on the model.

use crate::error::IdiomError;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Analysis window length in samples (~46 ms at 44.1 kHz). Matches the
/// convention in `crates/tone` so features are comparable across the workspace.
const WINDOW_SIZE: usize = 2048;
/// Hop between consecutive frames (75% overlap).
const HOP_SIZE: usize = 512;
/// Number of pitch classes in 12-tone-equal-temperament chroma.
const N_CHROMA: usize = 12;
/// Number of mel bands for the MFCC front end.
const N_MELS: usize = 40;
/// Number of MFCC coefficients retained (DC term dropped, see below).
const N_MFCC: usize = 12;
/// Reference frequency (Hz) for chroma binning: A4 = 440 Hz.
const A4_HZ: f32 = 440.0;

/// Dimension of every [`BaselineEmbedder`] output vector.
///
/// `N_CHROMA` chroma + `N_MFCC` MFCC + 1 spectral-centroid scalar.
pub const BASELINE_DIM: usize = N_CHROMA + N_MFCC + 1;

/// Turns audio into a fixed-length embedding vector for similarity search.
///
/// Implementations must be **deterministic**: the same samples and sample rate
/// always yield the same vector (the retrieval tests and the corpus depend on
/// this). The vector's dimension must be stable for a given embedder so that
/// query and corpus vectors are comparable.
///
/// A MERT-class transformer model is the intended future implementation behind
/// this trait; see the module docs.
pub trait IdiomEmbedder {
    /// Embed mono PCM `samples` captured at `sample_rate` Hz.
    fn embed(&self, samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, IdiomError>;

    /// The fixed dimension of vectors produced by [`Self::embed`].
    fn dim(&self) -> usize;
}

/// A real, deterministic baseline embedder using classic DSP.
///
/// The vector is the L2-normalised concatenation of:
/// 1. **Chroma (12)** — energy folded onto the 12 pitch classes, capturing
///    harmonic / tonal colour (which idiom *key area* a phrase lives in).
/// 2. **MFCC-style coefficients (12)** — frame-averaged log-mel energies
///    through a DCT-II, capturing timbre / articulation colour.
/// 3. **Spectral centroid (1)** — brightness, normalised to `[0, 1]`.
///
/// This is intentionally modest: it is a grounded stand-in that *actually
/// runs*, not a learned music encoder. See module docs for the MERT roadmap.
#[derive(Debug, Default, Clone, Copy)]
pub struct BaselineEmbedder;

impl BaselineEmbedder {
    /// Construct the baseline embedder (it holds no state).
    pub fn new() -> Self {
        Self
    }
}

impl IdiomEmbedder for BaselineEmbedder {
    fn embed(&self, samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, IdiomError> {
        if samples.is_empty() {
            return Err(IdiomError::EmptyInput);
        }
        if sample_rate == 0 {
            return Err(IdiomError::InvalidSampleRate(sample_rate));
        }

        let frames = magnitude_frames(samples);
        // `samples` is non-empty, so `magnitude_frames` yields >= 1 frame.

        let chroma = chroma_vector(&frames, sample_rate);
        let mfcc = mfcc_vector(&frames, sample_rate);
        let centroid = mean_spectral_centroid(&frames, sample_rate);

        let mut v = Vec::with_capacity(BASELINE_DIM);
        v.extend_from_slice(&chroma);
        v.extend_from_slice(&mfcc);
        v.push(centroid);

        Ok(l2_normalize(v))
    }

    fn dim(&self) -> usize {
        BASELINE_DIM
    }
}

/// Windowed FFT magnitude frames for `samples`.
///
/// Non-empty input always yields at least one (zero-padded) frame.
fn magnitude_frames(samples: &[f32]) -> Vec<Vec<f32>> {
    let window = hann_window(WINDOW_SIZE);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);

    let n_bins = WINDOW_SIZE / 2 + 1;
    let mut frames: Vec<Vec<f32>> = Vec::new();
    let mut scratch: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); WINDOW_SIZE];

    let mut start = 0usize;
    loop {
        for (i, slot) in scratch.iter_mut().enumerate() {
            let s = samples.get(start + i).copied().unwrap_or(0.0);
            *slot = Complex::new(s * window[i], 0.0);
        }
        fft.process(&mut scratch);
        frames.push(scratch[..n_bins].iter().map(|c| c.norm()).collect());

        if start + WINDOW_SIZE >= samples.len() {
            break;
        }
        start += HOP_SIZE;
    }
    frames
}

/// Centre frequency (Hz) of FFT bin `k` at `sample_rate`.
fn bin_hz(k: usize, sample_rate: u32) -> f32 {
    k as f32 * sample_rate as f32 / WINDOW_SIZE as f32
}

/// Frame-averaged, peak-normalised 12-bin chroma.
fn chroma_vector(frames: &[Vec<f32>], sample_rate: u32) -> [f32; N_CHROMA] {
    let mut acc = [0.0f64; N_CHROMA];
    let n_bins = frames.first().map(|f| f.len()).unwrap_or(0);

    // Skip bin 0 (DC) and ignore sub-audible bins below ~20 Hz.
    for mags in frames {
        for (k, &m) in mags.iter().enumerate().take(n_bins).skip(1) {
            let f = bin_hz(k, sample_rate);
            if f < 20.0 {
                continue;
            }
            let pc = pitch_class(f);
            // Use power so louder partials weigh more, matching chroma practice.
            acc[pc] += (m * m) as f64;
        }
    }

    let max = acc.iter().cloned().fold(0.0f64, f64::max);
    let mut out = [0.0f32; N_CHROMA];
    if max > 0.0 {
        for (o, a) in out.iter_mut().zip(acc.iter()) {
            *o = (a / max) as f32;
        }
    }
    out
}

/// The pitch class (0 = C, … 9 = A, … 11 = B) nearest to frequency `f`.
fn pitch_class(f: f32) -> usize {
    // MIDI note number relative to A4 (= MIDI 69). MIDI % 12 already gives the
    // pitch class with C = 0 (C4 = MIDI 60, 60 % 12 = 0; A4 = 69, 69 % 12 = 9).
    let midi = 69.0 + 12.0 * (f / A4_HZ).log2();
    let note = midi.round() as i64;
    note.rem_euclid(N_CHROMA as i64) as usize
}

/// Frame-averaged MFCC-style coefficients (DC coefficient dropped).
fn mfcc_vector(frames: &[Vec<f32>], sample_rate: u32) -> [f32; N_MFCC] {
    let n_bins = frames.first().map(|f| f.len()).unwrap_or(0);
    let filterbank = build_filterbank(n_bins, sample_rate);

    let mut acc = vec![0.0f64; N_MELS];
    for mags in frames {
        for (m, filter) in filterbank.iter().enumerate() {
            let mut energy = 0.0f32;
            for &(bin, weight) in filter {
                energy += mags[bin] * mags[bin] * weight;
            }
            acc[m] += (energy as f64 + 1e-10).ln();
        }
    }
    let n = frames.len().max(1) as f64;
    let log_mel: Vec<f32> = acc.iter().map(|v| (v / n) as f32).collect();

    // DCT-II keeping coefficients 1..=N_MFCC (drop coefficient 0, the overall
    // log-energy / loudness term, so the embedding ignores absolute level).
    let mut out = [0.0f32; N_MFCC];
    for (i, slot) in out.iter_mut().enumerate() {
        let c = i + 1;
        let mut sum = 0.0f32;
        for (m, &x) in log_mel.iter().enumerate() {
            sum += x * (std::f32::consts::PI * c as f32 * (m as f32 + 0.5) / N_MELS as f32).cos();
        }
        *slot = sum;
    }
    out
}

/// Mean spectral centroid across frames, normalised to `[0, 1]` by Nyquist.
fn mean_spectral_centroid(frames: &[Vec<f32>], sample_rate: u32) -> f32 {
    let nyquist = sample_rate as f32 / 2.0;
    let mut sum = 0.0f64;
    let mut counted = 0usize;
    for mags in frames {
        let mut weighted = 0.0f32;
        let mut total = 0.0f32;
        for (k, &m) in mags.iter().enumerate() {
            weighted += bin_hz(k, sample_rate) * m;
            total += m;
        }
        if total > 0.0 {
            sum += (weighted / total) as f64;
            counted += 1;
        }
    }
    if counted == 0 || nyquist <= 0.0 {
        return 0.0;
    }
    ((sum / counted as f64) as f32 / nyquist).clamp(0.0, 1.0)
}

/// Triangular mel filterbank as a sparse `[(bin, weight)]` list per filter.
fn build_filterbank(n_bins: usize, sample_rate: u32) -> Vec<Vec<(usize, f32)>> {
    let f_max = sample_rate as f32 / 2.0;
    let mel_max = hz_to_mel(f_max);
    let edges_hz: Vec<f32> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(mel_max * i as f32 / (N_MELS + 1) as f32))
        .collect();

    let bin_freq = |k: usize| k as f32 * sample_rate as f32 / WINDOW_SIZE as f32;

    let mut filters = Vec::with_capacity(N_MELS);
    for m in 0..N_MELS {
        let (lo, mid, hi) = (edges_hz[m], edges_hz[m + 1], edges_hz[m + 2]);
        let mut filter = Vec::new();
        for k in 0..n_bins {
            let f = bin_freq(k);
            let w = if f >= lo && f <= mid && mid > lo {
                (f - lo) / (mid - lo)
            } else if f > mid && f <= hi && hi > mid {
                (hi - f) / (hi - mid)
            } else {
                0.0
            };
            if w > 0.0 {
                filter.push((k, w));
            }
        }
        filters.push(filter);
    }
    filters
}

/// L2-normalise a vector so cosine similarity is a plain dot product.
///
/// A zero vector (silence with no spectral content) is returned unchanged.
fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Periodic Hann window of length `n`.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = (std::f32::consts::PI * i as f32 / n as f32).sin();
            x * x
        })
        .collect()
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure sine tone at `freq` Hz, `secs` long, at `sr` Hz sample rate.
    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn embeds_to_fixed_dimension() {
        let e = BaselineEmbedder::new();
        let v = e.embed(&sine(440.0, 0.5, 44_100), 44_100).unwrap();
        assert_eq!(v.len(), BASELINE_DIM);
        assert_eq!(v.len(), e.dim());
    }

    #[test]
    fn is_deterministic() {
        let e = BaselineEmbedder::new();
        let s = sine(261.63, 0.5, 44_100); // middle C
        let a = e.embed(&s, 44_100).unwrap();
        let b = e.embed(&s, 44_100).unwrap();
        assert_eq!(a, b, "same input must yield byte-identical embedding");
    }

    #[test]
    fn output_is_l2_normalized_and_finite() {
        let e = BaselineEmbedder::new();
        let v = e.embed(&sine(440.0, 0.5, 44_100), 44_100).unwrap();
        assert!(v.iter().all(|x| x.is_finite()));
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "expected unit norm, got {norm}");
    }

    #[test]
    fn different_pitches_give_different_embeddings() {
        let e = BaselineEmbedder::new();
        let c = e.embed(&sine(261.63, 0.5, 44_100), 44_100).unwrap();
        let g = e.embed(&sine(392.00, 0.5, 44_100), 44_100).unwrap();
        assert_ne!(c, g);
    }

    #[test]
    fn empty_input_errors() {
        let e = BaselineEmbedder::new();
        assert!(matches!(e.embed(&[], 44_100), Err(IdiomError::EmptyInput)));
    }

    #[test]
    fn zero_sample_rate_errors() {
        let e = BaselineEmbedder::new();
        assert!(matches!(
            e.embed(&sine(440.0, 0.1, 44_100), 0),
            Err(IdiomError::InvalidSampleRate(0))
        ));
    }

    #[test]
    fn chroma_peaks_at_played_pitch_class() {
        // A 440 Hz tone is pitch class A (index 9 in C=0 numbering).
        let frames = magnitude_frames(&sine(440.0, 0.5, 44_100));
        let chroma = chroma_vector(&frames, 44_100);
        let (peak_idx, _) = chroma
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert_eq!(peak_idx, 9, "A4 should peak on pitch class A");
    }

    #[test]
    fn pitch_class_maps_known_notes() {
        assert_eq!(pitch_class(440.0), 9); // A
        assert_eq!(pitch_class(261.63), 0); // C
        assert_eq!(pitch_class(329.63), 4); // E
    }
}
