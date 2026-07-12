//! #349 T3a — streaming polyphony: basic-pitch over a live stream.
//!
//! The batch pipeline ([`crate::audio_to_midi`]) transcribes a finished
//! recording; this runs the SAME model over a rolling window — 2 s of
//! context, advanced 1 s at a time — so a session can consume polyphonic
//! note events at ~1–2 s latency (spec §Tier 3: voicing-true labels, honest
//! polyphonic phrase evidence, the progression lift).
//!
//! ## The seam
//! [`PolyEngine`] is the swap point (spec §3): consumers hold a
//! `Box<dyn PolyEngine>` and never name basic-pitch. If a better
//! Apache/MIT model lands, it implements the trait and nothing above
//! changes.
//!
//! ## Kill-switch honesty (spec T3 AC4)
//! Construction is FALLIBLE: no ONNX Runtime (or a broken model) returns a
//! calm error and the caller ships without polyphony — T1/T2 keep working,
//! nothing panics, nothing lies. Inference errors after construction
//! surface per-poll the same way.
//!
//! ## Windowing
//! Notes are emitted when their onset falls in a hop that has a full
//! window of context behind it: each 1 s hop is inferred inside its 2 s
//! window, and only onsets in the hop's own span are emitted — the
//! overlap exists to give every emitted onset ≥1 s of following context,
//! and no note is ever emitted twice.

use ort::session::Session;

use crate::constants::{ANNOTATIONS_FPS, AUDIO_SAMPLE_RATE};
use crate::error::TranscribeError;
use crate::inference::{build_session, infer_with_session};
use crate::notes::output_to_notes;
use crate::resample::resample_to_model_rate;

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

/// The polyphonic-engine seam (spec §3): feed mono audio, poll note events.
pub trait PolyEngine {
    /// Push a chunk of mono samples at `sample_rate`.
    fn feed(&mut self, samples: &[f32], sample_rate: u32);
    /// Run inference if a new hop is ready; returns newly emitted notes
    /// (stream-absolute times), empty when no hop is due. Errors are calm
    /// and non-fatal — the next poll may succeed.
    fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError>;
}

/// Window length in seconds (context the model sees per inference).
const WINDOW_SECS: usize = 2;
/// Hop length in seconds (how far the window advances per inference).
const HOP_SECS: usize = 1;
const WINDOW_SAMPLES: usize = AUDIO_SAMPLE_RATE * WINDOW_SECS;
const HOP_SAMPLES: usize = AUDIO_SAMPLE_RATE * HOP_SECS;

/// basic-pitch as a [`PolyEngine`]: one ONNX session reused across hops.
pub struct StreamingBasicPitch {
    session: Session,
    /// Stream audio at the model rate (22.05 kHz), from stream start.
    /// Bounded: consumed audio is drained once a hop is emitted, keeping at
    /// most one window + one unprocessed hop in memory.
    buf: Vec<f32>,
    /// Model-rate samples drained from the front of `buf` so far — the
    /// stream-absolute position of `buf[0]`.
    drained: usize,
    /// Stream-absolute sample index up to which notes have been emitted.
    emitted_until: usize,
}

impl StreamingBasicPitch {
    /// Build the engine. Errors calmly when ONNX Runtime is unavailable —
    /// the kill switch (T3 AC4): callers degrade to T1/T2 and carry on.
    ///
    /// ort's `load-dynamic` backend PANICS on a missing dylib instead of
    /// returning an error; the #267 guard lives here at the seam so no
    /// caller ever has to remember `catch_unwind` — a missing runtime is a
    /// calm `Err`, never a crash.
    pub fn new() -> Result<Self, TranscribeError> {
        let session = std::panic::catch_unwind(build_session).map_err(|_| {
            TranscribeError::Runtime(
                "ONNX Runtime unavailable — polyphonic hearing is off; everything else works"
                    .to_owned(),
            )
        })??;
        Ok(Self {
            session,
            buf: Vec::new(),
            drained: 0,
            emitted_until: 0,
        })
    }
}

impl PolyEngine for StreamingBasicPitch {
    fn feed(&mut self, samples: &[f32], sample_rate: u32) {
        // Chunk-wise resample: chunks arrive at analysis-window size
        // (~1024+ samples), plenty for the polyphase resampler; boundary
        // error is far below the model's own tolerance.
        let at_model_rate = resample_to_model_rate(samples, sample_rate);
        self.buf.extend_from_slice(&at_model_rate);
    }

    fn poll(&mut self) -> Result<Vec<PolyNote>, TranscribeError> {
        // A hop is due when the buffer holds a full window AND the hop at
        // its head hasn't been emitted yet.
        if self.buf.len() < WINDOW_SAMPLES {
            return Ok(Vec::new());
        }
        let window_start = self.drained; // stream-absolute
        let window = &self.buf[..WINDOW_SAMPLES];
        let activations = infer_with_session(&mut self.session, window)?;
        let notes = output_to_notes(&activations.frames, &activations.onsets);

        // Emit notes whose ONSET lies in this window's first hop — each has
        // a full second of following context, and the next window starts
        // where this hop ends, so nothing repeats and nothing is skipped.
        let fps = ANNOTATIONS_FPS as f64;
        let hop_end_frame = (HOP_SECS as f64 * fps) as usize;
        let mut out = Vec::new();
        for n in notes {
            if n.start_frame >= hop_end_frame {
                continue; // onsets in the second half wait for their hop
            }
            let on_abs =
                window_start as f64 / AUDIO_SAMPLE_RATE as f64 + n.start_frame as f64 / fps;
            let on_sample = window_start + n.start_frame * AUDIO_SAMPLE_RATE / ANNOTATIONS_FPS;
            if on_sample < self.emitted_until {
                continue; // already emitted by a previous hop
            }
            out.push(PolyNote {
                midi: n.midi,
                on_secs: on_abs,
                off_secs: window_start as f64 / AUDIO_SAMPLE_RATE as f64 + n.end_frame as f64 / fps,
                amplitude: n.amplitude,
            });
        }
        // Advance one hop: drain consumed audio, mark the hop emitted.
        self.buf.drain(..HOP_SAMPLES);
        self.drained += HOP_SAMPLES;
        self.emitted_until = self.drained;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seam is object-safe — consumers can hold `Box<dyn PolyEngine>`
    /// (the swap point the spec demands). A compile-time contract.
    #[test]
    fn the_seam_is_object_safe() {
        fn _takes(_: &mut dyn PolyEngine) {}
    }
}
