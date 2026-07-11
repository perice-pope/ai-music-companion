//! Jazz Ears T1b (#349 §5.2): the 12-bin chromagram front end.
//!
//! Turns the mono stream into a smoothed pitch-class energy vector the
//! chord matcher (`theory::chords`) can read. Runs on the *processing*
//! thread at ~10 Hz — but it is still built zero-alloc after construction
//! (pre-allocated ring buffer + Goertzel bank), because it sits in the same
//! loop as the real-time analysis path and must never cause a hiccup.
//!
//! Design (no FFT dependency):
//! - A fixed ring buffer holds the most recent ~186 ms of mono audio.
//! - One **Goertzel filter per semitone** C2..B6 (60 bins), each evaluated
//!   over its own window of up to [`ANALYSIS_PERIODS`] periods — a
//!   constant-Q bank, so bass bins get the longer windows they physically
//!   need for selectivity.
//! - **Harmonic subtraction**: energy at a semitone is discounted by the
//!   expected harmonic bleed of the fundamentals below it (2nd/3rd/4th/5th
//!   partials), so one rich trumpet note does not light up root+fifth+third
//!   and read as a major triad.
//! - **Log compression** for level robustness, then folding into 12 pitch
//!   classes and per-bin exponential smoothing (τ ≈ 250 ms) so the matcher
//!   sees a stable picture instead of frame noise.

/// Ring-buffer capacity in samples (~186 ms at 44.1 kHz). Also the hard cap
/// on any bin's analysis window.
const BUFFER_LEN: usize = 8192;
/// Ideal analysis window per bin, in periods of that bin's frequency
/// (clamped to the buffer). 24 periods ≈ half-semitone selectivity for
/// mid-range bins; bass bins clamp to the buffer and accept some smear —
/// their harmonics an octave up carry the discrimination.
const ANALYSIS_PERIODS: f32 = 24.0;
/// Lowest analyzed semitone: MIDI 36 = C2 (guitar low E is MIDI 40).
const MIDI_LO: usize = 36;
/// Number of semitone bins: C2..=B6, five full octaves.
const NUM_BINS: usize = 60;
/// Fraction of a lower semitone's energy subtracted from where its
/// 2nd/3rd/4th/5th partials land (+12, +19, +24, +28 semitones).
const HARMONIC_BLEED: [(usize, f32); 4] = [(12, 0.4), (19, 0.3), (24, 0.25), (28, 0.2)];
/// Smoothing time constant for the folded pitch-class bins.
const SMOOTHING_TAU_SECS: f32 = 0.25;
/// Gain inside the log compressor: `ln(1 + G·x) / ln(1 + G)`.
const LOG_COMPRESSION_GAIN: f32 = 20.0;

/// Zero-alloc (after `new`) 12-bin chroma extractor. Feed it every analysis
/// window; ask for [`ChromaExtractor::chroma`] at the reporting cadence.
pub struct ChromaExtractor {
    /// Mono ring buffer of the most recent samples.
    ring: Box<[f32; BUFFER_LEN]>,
    write_pos: usize,
    /// Total samples ever fed (saturating); gates warm-up.
    filled: usize,
    /// Per-bin Goertzel coefficient `2·cos(2π·f/sr)`.
    coeffs: [f32; NUM_BINS],
    /// Per-bin analysis window length in samples.
    window_len: [usize; NUM_BINS],
    /// Scratch: raw per-semitone power for the current compute.
    energy: [f32; NUM_BINS],
    /// Smoothed folded pitch-class bins (the output).
    smoothed: [f32; 12],
    /// Samples fed since the last `chroma()` call — drives the EMA alpha.
    fed_since_compute: usize,
    sample_rate: f32,
}

impl ChromaExtractor {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        let mut coeffs = [0.0f32; NUM_BINS];
        let mut window_len = [0usize; NUM_BINS];
        for (bin, (coeff, len)) in coeffs.iter_mut().zip(window_len.iter_mut()).enumerate() {
            let midi = (MIDI_LO + bin) as f32;
            let freq = 440.0 * ((midi - 69.0) / 12.0).exp2();
            *coeff = 2.0 * (std::f32::consts::TAU * freq / sr).cos();
            // Whole periods, so the rectangular window ends near a zero
            // crossing of the target frequency (less leakage).
            let periods = (ANALYSIS_PERIODS * sr / freq).round() as usize;
            *len = periods.clamp(64, BUFFER_LEN);
        }
        Self {
            ring: Box::new([0.0; BUFFER_LEN]),
            write_pos: 0,
            filled: 0,
            coeffs,
            window_len,
            energy: [0.0; NUM_BINS],
            smoothed: [0.0; 12],
            fed_since_compute: 0,
            sample_rate: sr,
        }
    }

    /// Push a window of mono samples. Zero-alloc; call every analysis loop.
    pub fn feed(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % BUFFER_LEN;
        }
        self.filled = self.filled.saturating_add(samples.len());
        self.fed_since_compute = self.fed_since_compute.saturating_add(samples.len());
    }

    /// True once the ring holds a full analysis buffer.
    pub fn ready(&self) -> bool {
        self.filled >= BUFFER_LEN
    }

    /// Compute the smoothed 12-bin chroma (C = index 0). Returns `None`
    /// during warm-up. Zero-alloc.
    pub fn chroma(&mut self) -> Option<[f32; 12]> {
        if !self.ready() {
            return None;
        }

        // 1. Per-semitone Goertzel power over that bin's own window. The
        // ring is walked as (at most) two contiguous slices — no per-sample
        // modulo in the hot loop.
        for bin in 0..NUM_BINS {
            let n = self.window_len[bin];
            let coeff = self.coeffs[bin];
            let start = (self.write_pos + BUFFER_LEN - n) % BUFFER_LEN;
            let (first, second) = if start + n <= BUFFER_LEN {
                (&self.ring[start..start + n], &self.ring[0..0])
            } else {
                (&self.ring[start..], &self.ring[..start + n - BUFFER_LEN])
            };
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for &x in first.iter().chain(second) {
                let s0 = x + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            // Normalize by window length² so long bass windows don't
            // dominate purely by integrating more samples.
            let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
            self.energy[bin] = power / (n as f32 * n as f32);
        }

        // 2. Whiten to the strongest bin (level invariance).
        let max = self.energy.iter().cloned().fold(0.0f32, f32::max);
        if max > 1e-12 {
            for e in self.energy.iter_mut() {
                *e /= max;
            }
        }

        // 3. Harmonic subtraction, bottom-up: discount each semitone by the
        // expected partial bleed of the fundamentals below it.
        for bin in 0..NUM_BINS {
            let mut bleed = 0.0f32;
            for &(offset, weight) in &HARMONIC_BLEED {
                if bin >= offset {
                    bleed += weight * self.energy[bin - offset];
                }
            }
            self.energy[bin] = (self.energy[bin] - bleed).max(0.0);
        }

        // 4. Log-compress and fold into pitch classes (C2 → pc 0).
        let mut folded = [0.0f32; 12];
        let norm = (1.0 + LOG_COMPRESSION_GAIN).ln();
        for bin in 0..NUM_BINS {
            let compressed = (1.0 + LOG_COMPRESSION_GAIN * self.energy[bin]).ln() / norm;
            folded[(MIDI_LO + bin) % 12] += compressed;
        }

        // 5. Exponential smoothing, τ ≈ 250 ms of *audio time* elapsed
        // since the previous reading.
        let dt = self.fed_since_compute as f32 / self.sample_rate;
        self.fed_since_compute = 0;
        let alpha = (1.0 - (-dt / SMOOTHING_TAU_SECS).exp()).clamp(0.0, 1.0);
        for (sm, f) in self.smoothed.iter_mut().zip(folded.iter()) {
            *sm += alpha * (f - *sm);
        }

        Some(self.smoothed)
    }

    /// Forget everything (session restart / long silence) so a stale chord
    /// picture can't bleed into the next sound.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.filled = 0;
        self.write_pos = 0;
        self.fed_since_compute = 0;
        self.smoothed = [0.0; 12];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    fn midi_freq(m: i32) -> f32 {
        440.0 * (((m - 69) as f32) / 12.0).exp2()
    }

    /// Sum of tones, each with a natural-ish harmonic rolloff (1/k
    /// amplitude, 5 partials) — the synthetic-additive fixture from the
    /// spec's ACs.
    fn render(midis: &[i32], secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        let mut out = vec![0.0f32; n];
        for &m in midis {
            let f = midi_freq(m);
            for k in 1..=5u32 {
                let amp = 1.0 / k as f32;
                let w = std::f32::consts::TAU * f * k as f32 / SR as f32;
                for (i, o) in out.iter_mut().enumerate() {
                    *o += amp * (w * i as f32).sin();
                }
            }
        }
        out
    }

    /// Feed audio in 1024-sample windows, calling chroma() every ~100 ms,
    /// and return the last reading.
    fn last_chroma(audio: &[f32]) -> Option<[f32; 12]> {
        let mut ex = ChromaExtractor::new(SR);
        let mut last = None;
        for (i, w) in audio.chunks(1024).enumerate() {
            ex.feed(w);
            if i % 4 == 3 {
                if let Some(c) = ex.chroma() {
                    last = Some(c);
                }
            }
        }
        last
    }

    fn top_pcs(c: &[f32; 12], n: usize) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..12).collect();
        idx.sort_by(|&a, &b| c[b].partial_cmp(&c[a]).unwrap());
        let mut top: Vec<usize> = idx.into_iter().take(n).collect();
        top.sort_unstable();
        top
    }

    /// #349 T1b AC: a rendered triad's three pitch classes are the three
    /// strongest chroma bins — at every root (12-root sweep, mixed
    /// registers). Fails if tuning, folding, or harmonic handling breaks.
    #[test]
    fn major_triads_at_all_twelve_roots_dominate_the_chroma() {
        for root in 0..12 {
            let base = 48 + root; // C3..B3 region
            let audio = render(&[base, base + 4, base + 7], 1.0);
            let c = last_chroma(&audio).expect("warm");
            let mut want = vec![
                (base % 12) as usize,
                ((base + 4) % 12) as usize,
                ((base + 7) % 12) as usize,
            ];
            want.sort_unstable();
            assert_eq!(top_pcs(&c, 3), want, "root midi {base}");
        }
    }

    /// End-to-end with the matcher: a rendered C7 names Dom7 at C; a single
    /// rich note (5 partials — its 3rd and 5th partials land on the fifth
    /// and major third!) must NOT read as a chord. This is the
    /// no-label-on-mono honesty AC; it fails if harmonic subtraction is
    /// removed.
    #[test]
    fn matcher_hears_a_c7_but_refuses_a_single_rich_note() {
        let chord = last_chroma(&render(&[48, 52, 55, 58], 1.0)).unwrap();
        let m = theory::best_match(&chord, None)
            .unwrap_or_else(|| panic!("C7 should match; chroma {chord:?}"));
        assert_eq!((m.root_pc, m.quality), (0, theory::ChordQuality::Dom7));

        let mono = last_chroma(&render(&[57], 1.0)).unwrap(); // lone A3
        assert!(
            theory::best_match(&mono, None).is_none(),
            "single note must not label a chord: {mono:?}"
        );
    }

    /// #349 T1b AC1 (extension fixtures): the spec's jazz ladder rendered
    /// as audio — harmonics, subtraction, compression and folding must not
    /// mangle extension tones. Where the pitch-class set is genuinely
    /// quality-ambiguous (Cm7 ≡ Eb6, dim7 rotations, m7b5 ≡ m6) we assert
    /// the matched template reproduces the exact fed set — enharmonic
    /// honesty, not a wrong answer; unambiguous sets assert (root, quality).
    #[test]
    fn the_rendered_jazz_ladder_survives_the_front_end() {
        use theory::ChordQuality as Q;
        // (midis, root pc, quality, exact-quality assertion?)
        let cases: &[(&[i32], u8, Q, bool)] = &[
            (&[48, 52, 55, 59], 0, Q::Maj7, true),       // Cmaj7
            (&[48, 51, 55, 58], 0, Q::Min7, false),      // Cm7 (≡ Eb6)
            (&[48, 51, 54, 57], 0, Q::Dim7, false),      // Cdim7 (rotations)
            (&[48, 52, 55, 58, 63], 0, Q::Dom7s9, true), // C7#9
            (&[54, 57, 60, 64], 6, Q::Min7b5, false),    // F#m7b5 (≡ Am6)
            (&[46, 50, 55, 56], 10, Q::Dom13, true),     // Bb13 shell, no 5th
        ];
        for &(midis, root, quality, exact) in cases {
            let c = last_chroma(&render(midis, 1.0)).expect("warm");
            let m = theory::best_match(&c, None)
                .unwrap_or_else(|| panic!("{quality:?}@{root} unmatched: {c:?}"));
            if exact {
                assert_eq!((m.root_pc, m.quality), (root, quality), "chroma {c:?}");
            } else {
                let fed: std::collections::BTreeSet<u8> =
                    midis.iter().map(|&x| (x % 12) as u8).collect();
                let matched: std::collections::BTreeSet<u8> = m
                    .quality
                    .intervals()
                    .iter()
                    .map(|&iv| (m.root_pc + iv) % 12)
                    .collect();
                assert_eq!(matched, fed, "{quality:?}@{root} matched {m:?}: {c:?}");
            }
        }
    }

    /// A rendered two-note dyad (C3+G3, 5 partials each — C's 5th partial
    /// lands ON the major third!) must not mint a phantom triad. Fails if
    /// harmonic subtraction stops covering the dyad case.
    #[test]
    fn a_rendered_dyad_never_becomes_a_phantom_triad() {
        let c = last_chroma(&render(&[48, 55], 1.0)).expect("warm");
        assert!(
            theory::best_match(&c, None).is_none(),
            "dyad must not read as a chord: {c:?}"
        );
    }

    /// #349 §7 "never sticks stale": after the chord stops, the smoothed
    /// picture must decay below the matcher's silence floor within ~1 s of
    /// silence — through the REAL extractor, pinning the τ/floor interplay
    /// (hand-zeroed chroma can't catch a floor or smoothing regression).
    #[test]
    fn a_stopped_chord_decays_out_of_the_matcher_within_a_second() {
        let mut ex = ChromaExtractor::new(SR);
        for w in render(&[48, 52, 55], 1.0).chunks(1024) {
            ex.feed(w);
        }
        assert!(
            theory::best_match(&ex.chroma().expect("ready"), None).is_some(),
            "chord readable while ringing"
        );
        // One second of silence, read at the production ~10 Hz cadence.
        let silence = vec![0.0f32; 1024];
        let mut cleared_at: Option<usize> = None;
        for i in 0..44 {
            ex.feed(&silence);
            if i % 4 == 3 {
                let c = ex.chroma().expect("ready");
                if theory::best_match(&c, None).is_none() {
                    cleared_at = Some(i);
                    break;
                }
            }
        }
        assert!(
            cleared_at.is_some(),
            "chord must decay out of the matcher within ~1 s of silence"
        );
    }

    /// Silence produces no reading during warm-up and a near-zero vector
    /// after — never a phantom chord.
    #[test]
    fn silence_is_silent() {
        let mut ex = ChromaExtractor::new(SR);
        assert!(ex.chroma().is_none(), "warm-up must return None");
        let zeros = vec![0.0f32; BUFFER_LEN * 2];
        ex.feed(&zeros);
        let c = ex.chroma().expect("ready");
        assert!(theory::best_match(&c, None).is_none(), "phantom: {c:?}");
    }

    /// reset() clears the smoothed picture entirely and re-enters warm-up
    /// (the gradual-decay path is pinned by
    /// `a_stopped_chord_decays_out_of_the_matcher_within_a_second`).
    #[test]
    fn reset_clears_the_smoothed_picture() {
        let audio = render(&[48, 52, 55], 1.0);
        let mut ex = ChromaExtractor::new(SR);
        for w in audio.chunks(1024) {
            ex.feed(w);
        }
        let before = ex.chroma().expect("ready");
        assert!(before.iter().any(|&v| v > 0.1));
        ex.reset();
        assert!(ex.chroma().is_none(), "reset must re-enter warm-up");
    }
}
