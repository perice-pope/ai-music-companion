//! #349 T3a — streaming polyphony: basic-pitch over a live stream.
//!
//! The batch pipeline ([`crate::audio_to_midi`]) transcribes a finished
//! recording; this runs the SAME model over a rolling window — ~2 s of
//! context, advanced 1 s at a time — so a session can consume polyphonic
//! note events at ~1–2 s latency (spec §Tier 3: voicing-true labels, honest
//! polyphonic phrase evidence, the progression lift).
//!
//! ## The seam
//! [`PolyEngine`] is the swap point (spec §3): consumers hold a
//! `Box<dyn PolyEngine>` (the trait is `Send` — it lives on the session
//! worker thread) and never name basic-pitch. If a better Apache/MIT model
//! lands, it implements the trait and nothing above changes.
//!
//! ## Kill-switch honesty (spec T3 AC4)
//! Construction is FALLIBLE: no ONNX Runtime (or a broken model) returns a
//! calm error and the caller ships without polyphony — T1/T2 keep working,
//! nothing panics, nothing lies. Inference errors after construction
//! surface per-poll the same way (and the input buffer stays bounded even
//! if every poll fails).
//!
//! ## Windowing & note continuity (review round 1)
//! Each window is inferred independently, and a chord that RINGS across a
//! hop boundary re-appears at the next window's start as an onset-less
//! note (the model reports energy, not memory). The engine therefore
//! tracks which midis were still sounding at each hop boundary and
//! suppresses their onset-less re-detections — a held chord emits each
//! note ONCE. A real re-strike (fresh onset later in the window) still
//! lands. An onset landing exactly on the seam emits in the window that
//! owns it (sample-space cutoff) and rings into the next, where the
//! continuity rule suppresses its re-detection — verified by mutation:
//! removing the ringing tracking fails both the held-chord and the
//! boundary-onset fixtures (no second guard exists to mask it).

use ort::session::Session;

use crate::constants::{AUDIO_N_SAMPLES, AUDIO_SAMPLE_RATE, FFT_HOP};
use crate::error::TranscribeError;
use crate::inference::{build_session, infer_with_session};
use crate::notes::output_to_notes;

/// One polyphonic note event on the STREAM clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyNote {
    pub midi: u8,
    /// Onset, seconds from the start of the stream.
    pub on_secs: f64,
    /// Offset, seconds from the start of the stream (may extend past the
    /// analyzed window for a still-ringing note).
    pub off_secs: f64,
    /// Mean activation in `[0, 1]` — a confidence proxy, never a grade.
    pub amplitude: f32,
}

/// The polyphonic-engine seam (spec §3): feed mono audio, poll note
/// events. `Send` so a `Box<dyn PolyEngine>` can move to the worker thread.
pub trait PolyEngine: Send {
    /// Push a chunk of mono samples at `sample_rate`.
    fn feed(&mut self, samples: &[f32], sample_rate: u32);
    /// Run inference if a new hop is ready; returns newly emitted notes
    /// (stream-absolute times), empty when no hop is due. Errors are calm
    /// and non-fatal — the next poll may succeed.
    fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError>;
    /// Flush the stream's tail: infer whatever remains (less than a full
    /// window) and emit its notes. Call once when the stream ends — a
    /// lifted take must not lose its final chord.
    fn finish(&mut self) -> Result<Vec<PolyNote>, TranscribeError>;
}

/// Window length: exactly one model input (~1.99 s) — feeding more would
/// silently pay for a second, mostly-padding inference window per hop.
const WINDOW_SAMPLES: usize = AUDIO_N_SAMPLES;
/// Hop length in samples (1 s at the model rate).
const HOP_SAMPLES: usize = AUDIO_SAMPLE_RATE;
/// A note starting within this many frames of a window's start, without a
/// preceding silence, is the continuation of a note ringing across the hop
/// boundary — energy, not a new attack.
const CONTINUATION_FRAMES: usize = 3;
/// Input-buffer hard cap: ~10 windows. A consumer that feeds but never
/// polls (or whose polls persistently error) stays bounded — the oldest
/// audio drops, honestly unanalyzed, instead of leaking.
const BUF_CAP_SAMPLES: usize = WINDOW_SAMPLES * 10;

/// basic-pitch as a [`PolyEngine`]: one ONNX session reused across hops.
pub struct StreamingBasicPitch {
    session: Session,
    /// Stream audio at the model rate, from the current window's start.
    buf: Vec<f32>,
    /// Model-rate samples consumed before `buf[0]` — the stream-absolute
    /// position of the buffer head.
    drained: usize,
    /// Midis still sounding at the last hop boundary (their onset-less
    /// re-detections in the next window are continuations, not notes).
    ringing: [bool; 128],
    /// Drift-free incremental resampler state (identity at 22.05 kHz).
    resampler: StreamResampler,
}

/// Chunk-wise resampling with GLOBAL rational accounting — output length
/// derives from total input consumed, never from per-chunk rounding, so a
/// 48 kHz stream cannot drift against the model clock (review round 1: the
/// naive per-chunk floor lost ~0.85 ms/s). A small input tail is carried
/// between chunks as kernel context, flushed at `finish`.
struct StreamResampler {
    sr_in: Option<u32>,
    /// Uncommitted input (kernel context + remainder).
    pending: Vec<f32>,
    /// Total input samples ever received.
    total_in: u64,
    /// Input samples already committed to output.
    committed_in: u64,
    /// Total output samples ever produced.
    total_out: u64,
}

/// Input samples held back as kernel context between chunks (the sinc in
/// `resample_to_model_rate` reaches ±64 taps).
const RESAMPLE_CONTEXT: usize = 128;

impl StreamResampler {
    fn new() -> Self {
        Self {
            sr_in: None,
            pending: Vec::new(),
            total_in: 0,
            committed_in: 0,
            total_out: 0,
        }
    }

    fn convert(&mut self, samples: &[f32], sr_in: u32, flush: bool) -> Vec<f32> {
        if sr_in == AUDIO_SAMPLE_RATE as u32 {
            return samples.to_vec();
        }
        self.sr_in = Some(sr_in);
        self.pending.extend_from_slice(samples);
        self.total_in += samples.len() as u64;
        let ratio = AUDIO_SAMPLE_RATE as f64 / f64::from(sr_in);

        // Keep kernel context unless flushing the stream end.
        let hold = if flush { 0 } else { RESAMPLE_CONTEXT };
        if self.pending.len() <= hold {
            return Vec::new();
        }
        let commit_to = self.total_in - hold as u64;
        // Global accounting: output owed up to `commit_to` input samples,
        // minus output already produced — no per-chunk floor drift.
        let owed = ((commit_to as f64 * ratio).floor() as u64).saturating_sub(self.total_out);
        if owed == 0 {
            self.trim_pending(commit_to);
            return Vec::new();
        }
        // Phase-continuous synthesis on the GLOBAL grid (round-2 must-fix:
        // re-anchoring the kernel per block jumped the phase up to one
        // input sample every commit, and at 48 kHz the discontinuities
        // read as phantom attacks). Output sample k lives at global input
        // position k/ratio, exactly as the batch resampler would place it
        // over the whole stream.
        let cutoff = 0.5 * ratio.min(1.0);
        let pending_start_in = self.total_in - self.pending.len() as u64;
        let mut out = Vec::with_capacity(owed as usize);
        for k in self.total_out..self.total_out + owed {
            let center = k as f64 / ratio - pending_start_in as f64;
            let i0 = center.floor() as isize;
            let (mut acc, mut norm) = (0.0_f64, 0.0_f64);
            for j in (i0 - crate::resample::KERNEL_HALF)..=(i0 + crate::resample::KERNEL_HALF) {
                if j < 0 || j as usize >= self.pending.len() {
                    continue;
                }
                let x = center - j as f64;
                let w = crate::resample::sinc(2.0 * cutoff * x)
                    * crate::resample::blackman(x, crate::resample::KERNEL_HALF as f64);
                acc += f64::from(self.pending[j as usize]) * w;
                norm += w;
            }
            out.push(if norm.abs() > 1e-12 {
                (acc / norm) as f32
            } else {
                0.0
            });
        }
        self.total_out += out.len() as u64;
        self.trim_pending(commit_to);
        out
    }

    /// Drop input before `commit_to`, keeping enough committed samples for
    /// kernel left-context on the next block.
    fn trim_pending(&mut self, commit_to: u64) {
        self.committed_in = commit_to;
        let pending_start_in = self.total_in - self.pending.len() as u64;
        let committed_in_pending = (commit_to - pending_start_in) as usize;
        let drop = committed_in_pending.saturating_sub(RESAMPLE_CONTEXT);
        if drop > 0 {
            self.pending.drain(..drop);
        }
    }
}

impl StreamingBasicPitch {
    /// Build the engine. Errors calmly when ONNX Runtime is unavailable —
    /// the kill switch (T3 AC4): callers degrade to T1/T2 and carry on.
    ///
    /// ort's `load-dynamic` backend PANICS on a missing dylib instead of
    /// returning an error; the #267 guard lives here at the seam (with the
    /// panic hook silenced for the attempt — a degrading startup must not
    /// spray a backtrace) so no caller ever has to remember
    /// `catch_unwind` — a missing runtime is a calm `Err`, never a crash.
    pub fn new() -> Result<Self, TranscribeError> {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let attempt = std::panic::catch_unwind(build_session);
        std::panic::set_hook(prev_hook);
        let session = attempt.map_err(|_| {
            TranscribeError::Runtime(
                "ONNX Runtime unavailable — polyphonic hearing is off; everything else works"
                    .to_owned(),
            )
        })??;
        Ok(Self {
            session,
            buf: Vec::new(),
            drained: 0,
            ringing: [false; 128],
            resampler: StreamResampler::new(),
        })
    }

    /// Infer over the buffer's first `window_len` samples, emitting notes
    /// whose onsets lie within `emit_span` of the window start, with
    /// continuity + re-attack guards. `boundary` (samples past window
    /// start) is where the NEXT window will begin — notes still sounding
    /// there ring into it.
    fn infer_window(
        &mut self,
        window_len: usize,
        emit_span: usize,
        boundary: usize,
    ) -> Result<Vec<PolyNote>, TranscribeError> {
        let window_start = self.drained;
        let activations = {
            let window = &self.buf[..window_len];
            infer_with_session(&mut self.session, window)?
        };
        let notes = output_to_notes(&activations.frames, &activations.onsets);

        let mut out = Vec::new();
        let mut next_ringing = [false; 128];
        for n in notes {
            let midi = usize::from(n.midi);
            let on_sample = n.start_frame * FFT_HOP;
            let end_sample = n.end_frame * FFT_HOP;
            let rings_past_boundary = end_sample >= boundary;
            // A continuation of a note already ringing across THIS window's
            // start: energy, not a new attack — never re-emitted (and it
            // keeps ringing forward if it still crosses the next boundary).
            if n.start_frame < CONTINUATION_FRAMES && self.ringing[midi] {
                if rings_past_boundary {
                    next_ringing[midi] = true;
                }
                continue;
            }
            if on_sample >= emit_span {
                // Deferred: this onset belongs to the NEXT window, where it
                // will re-detect near frame 0 — it must NOT be marked as
                // ringing here, or the continuation rule would swallow it
                // (round-2 must-fix: a ~25 ms dead band after every hop).
                continue;
            }
            if rings_past_boundary {
                next_ringing[midi] = true;
            }
            let abs_on = window_start + on_sample;
            out.push(PolyNote {
                midi: n.midi,
                on_secs: abs_on as f64 / AUDIO_SAMPLE_RATE as f64,
                off_secs: (window_start + end_sample) as f64 / AUDIO_SAMPLE_RATE as f64,
                amplitude: n.amplitude,
            });
        }
        self.ringing = next_ringing;
        Ok(out)
    }
}

impl PolyEngine for StreamingBasicPitch {
    fn feed(&mut self, samples: &[f32], sample_rate: u32) {
        let at_model_rate = self.resampler.convert(samples, sample_rate, false);
        self.buf.extend_from_slice(&at_model_rate);
        // Bounded even if poll() never runs or persistently errors: drop
        // the oldest audio (honestly unanalyzed) instead of leaking.
        if self.buf.len() > BUF_CAP_SAMPLES {
            let overflow = self.buf.len() - BUF_CAP_SAMPLES;
            self.buf.drain(..overflow);
            self.drained += overflow;
        }
    }

    fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
        if self.buf.len() < WINDOW_SAMPLES {
            return Ok(Vec::new());
        }
        let out = self.infer_window(WINDOW_SAMPLES, HOP_SAMPLES, HOP_SAMPLES)?;
        // Advance one hop (only on success — an erroring poll retries the
        // same window next time; feed()'s cap bounds the buffer meanwhile).
        self.buf.drain(..HOP_SAMPLES);
        self.drained += HOP_SAMPLES;
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
        // Flush the resampler's held-back tail into the buffer first.
        if let Some(sr) = self.resampler.sr_in {
            let tail = self.resampler.convert(&[], sr, true);
            self.buf.extend_from_slice(&tail);
        }
        let mut out = Vec::new();
        // Drain full hops first (a long backlog flushes correctly)…
        while self.buf.len() >= WINDOW_SAMPLES {
            out.extend(self.poll()?);
        }
        // …then the final partial window: everything left is emittable.
        if !self.buf.is_empty() {
            let len = self.buf.len();
            out.extend(self.infer_window(len, len, len)?);
            self.buf.clear();
            self.drained += len;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seam is object-safe AND Send — consumers can hold
    /// `Box<dyn PolyEngine>` and move it to the worker thread.
    #[test]
    fn the_seam_is_object_safe_and_send() {
        fn _takes(_: &mut dyn PolyEngine) {}
        fn _is_send<T: Send>() {}
        _is_send::<Box<dyn PolyEngine>>();
    }

    /// The drift-free resampler: after any chunking pattern, total output
    /// tracks floor(total_input × ratio) exactly — a 48 kHz stream cannot
    /// creep against the model clock (round-1 must-fix: the naive
    /// per-chunk floor lost a fraction of a sample per chunk, ~0.85 ms/s).
    #[test]
    fn chunked_resampling_never_drifts() {
        let sr_in = 48_000u32;
        let ratio = AUDIO_SAMPLE_RATE as f64 / f64::from(sr_in);
        let mut r = StreamResampler::new();
        let mut total_out = 0usize;
        let mut fed = 0u64;
        let chunk = vec![0.25f32; 1024];
        while fed < 30 * u64::from(sr_in) {
            total_out += r.convert(&chunk, sr_in, false).len();
            fed += chunk.len() as u64;
        }
        total_out += r.convert(&[], sr_in, true).len();
        let expected = (fed as f64 * ratio).floor() as usize;
        assert!(
            (total_out as i64 - expected as i64).abs() <= 1,
            "drift: produced {total_out}, expected {expected}"
        );
    }

    /// Phase continuity (round-2 must-fix): chunked resampling of a SINE
    /// matches the whole-buffer batch resampler sample-for-sample — no
    /// kernel re-anchoring jumps at chunk joins (which read as phantom
    /// attacks at 48 kHz). Count-only tests are structurally blind to this.
    #[test]
    fn chunked_resampling_is_phase_continuous_on_a_sine() {
        let sr_in = 48_000u32;
        let n = 3 * sr_in as usize;
        let sine: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / f64::from(sr_in);
                (2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32
            })
            .collect();
        let whole = crate::resample::resample_to_model_rate(&sine, sr_in);

        let mut r = StreamResampler::new();
        let mut chunked = Vec::new();
        for c in sine.chunks(1024) {
            chunked.extend(r.convert(c, sr_in, false));
        }
        chunked.extend(r.convert(&[], sr_in, true));

        let compare = whole.len().min(chunked.len());
        // Skip the first/last kernel widths (edge handling differs by
        // design: the batch has hard stream edges, the stream carries
        // context); the interior must agree to float noise.
        let skip = 64usize;
        let mut max_err = 0.0f32;
        for i in skip..compare - skip {
            max_err = max_err.max((whole[i] - chunked[i]).abs());
        }
        assert!(
            max_err < 1e-3,
            "chunk joins must be inaudible: max deviation {max_err}"
        );
    }

    /// The resampler stays bounded and correct across odd chunk sizes too
    /// (prime-sized chunks stress the global accounting).
    #[test]
    fn odd_chunk_sizes_account_exactly() {
        let sr_in = 44_100u32;
        let ratio = AUDIO_SAMPLE_RATE as f64 / f64::from(sr_in);
        let mut r = StreamResampler::new();
        let mut total_out = 0usize;
        let mut fed = 0u64;
        for size in [313usize, 997, 1024, 61, 2048].iter().cycle().take(200) {
            let chunk = vec![0.1f32; *size];
            total_out += r.convert(&chunk, sr_in, false).len();
            fed += *size as u64;
        }
        total_out += r.convert(&[], sr_in, true).len();
        let expected = (fed as f64 * ratio).floor() as usize;
        assert!(
            (total_out as i64 - expected as i64).abs() <= 1,
            "drift: produced {total_out}, expected {expected}"
        );
        assert!(r.pending.len() <= RESAMPLE_CONTEXT, "pending bounded");
    }
}
