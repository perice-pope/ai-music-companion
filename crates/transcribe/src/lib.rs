//! Audio-to-MIDI transcription for AI Music Companion.
//!
//! Wraps Spotify's **basic-pitch** model (monophonic-first, Apache-2.0): decode
//! → resample to 22.05 kHz → ONNX inference → note creation → standard MIDI
//! bytes. The MIDI output is intentionally the same shape a user-dropped `.mid`
//! has, so the Tauri import command can feed it straight into the existing
//! `brain::score::midi` → MusicXML → library path.
//!
//! ## Runtime dependency
//!
//! Inference uses ONNX Runtime via `ort`'s `load-dynamic` backend: the native
//! `libonnxruntime` is `dlopen`ed at run time from `ORT_DYLIB_PATH` (or the
//! system library path). We use `load-dynamic` because the default
//! `download-binaries` backend fetches from a CDN that is blocked under our CI
//! network policy. CI vendors the official Microsoft ONNX Runtime release and
//! sets `ORT_DYLIB_PATH`; see `.github/workflows/ci.yml`. The model itself is
//! embedded in the binary (`models/nmp.onnx`, ~225 KB), so no model file needs
//! resolving at run time.

mod constants;
mod decode;
mod error;
mod inference;
mod midi_out;
mod notes;
mod resample;
pub mod runner;
pub mod stream;

pub use decode::decode_audio;
pub use error::TranscribeError;
pub use notes::NoteEvent;
pub use runner::PolyRunner;
pub use stream::{PolyEngine, PolyNote, StreamingBasicPitch};

use inference::infer;
use midi_out::notes_to_midi;
use notes::output_to_notes;
use resample::resample_to_model_rate;

/// Above this fraction of time-overlapping notes a transcription stops
/// reading as [`PolyphonyVerdict::Mono`]. Calibrated on real inference:
/// clean single lines (sine scales, the VA kit's sample recording) read 0.0;
/// a melody with one occasional held second voice reads ~0.33.
pub const BORDERLINE_POLYPHONY: f32 = 0.15;
/// At or above this fraction of overlapping notes the input is called a full
/// mix. Calibrated: sustained triads read 1.0, an occasional double-stop
/// ~0.33 — 0.5 splits "a second voice sometimes" from "chords are the
/// texture".
pub const FULL_MIX_POLYPHONY: f32 = 0.5;
/// Below this mean note activation the transcription as a whole is weak.
/// Combined with overlapping notes it escalates the verdict to
/// [`PolyphonyVerdict::FullMix`] — both signals saying "not one clear line".
pub const LOW_CONFIDENCE: f32 = 0.4;
/// A note whose own activation falls below this counts as "uncertain" in the
/// honesty counts ("caught N notes, M uncertain"). Sits at the model's onset
/// threshold: below it the note came from the leftover-energy mop-up pass or
/// barely fired.
pub const UNCERTAIN_NOTE_CONFIDENCE: f32 = 0.5;

/// The polyphony honesty verdict (#331): is this transcription the single
/// line the model is built for, or a full mix presented as one?
///
/// basic-pitch is monophonic-first — a whole-band recording still yields
/// *notes*, silently wrong ones. This verdict is what lets the UI say so
/// calmly instead of presenting a full-mix transcription as clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolyphonyVerdict {
    /// One clear line — no warning warranted.
    Mono,
    /// Some simultaneous notes (double stops, bleed, a second voice) — the
    /// transcription is usable but worth a calm heads-up.
    Borderline,
    /// The input sounds like a full mix — the notes may be approximate.
    FullMix,
}

/// A calm quality signal for a transcription — never a fake "accuracy score".
///
/// Audio transcription is approximate; these aggregates let the UI warn the
/// user when a recording looks polyphonic or weak, without pretending to a
/// precision we don't have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TranscriptionQuality {
    /// Number of notes detected.
    pub note_count: usize,
    /// Mean note activation (basic-pitch amplitude) in `[0, 1]` — a confidence proxy.
    pub mean_confidence: f32,
    /// Fraction of notes that overlap another note in time, in `[0, 1]`.
    /// basic-pitch is monophonic-first, so a high value means the input was
    /// likely polyphonic and the transcription is unreliable.
    pub polyphony: f32,
    /// Notes whose own activation fell below [`UNCERTAIN_NOTE_CONFIDENCE`] —
    /// the "M" of "caught N notes, M uncertain".
    pub uncertain_count: usize,
    /// The honesty-gate classification of this transcription (#331).
    pub verdict: PolyphonyVerdict,
}

/// Transcribe mono PCM `samples` (any `sample_rate`) into standard MIDI bytes.
///
/// Returns [`TranscribeError::Empty`] when no notes are detected (silence or no
/// clear pitch), and [`TranscribeError::Runtime`] when ONNX Runtime is
/// unavailable. The returned bytes parse with `midly` / the brain MIDI importer.
pub fn audio_to_midi(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, TranscribeError> {
    audio_to_midi_with_quality(samples, sample_rate).map(|(bytes, _)| bytes)
}

/// Like [`audio_to_midi`], but also returns a [`TranscriptionQuality`] signal.
pub fn audio_to_midi_with_quality(
    samples: &[f32],
    sample_rate: u32,
) -> Result<(Vec<u8>, TranscriptionQuality), TranscribeError> {
    let resampled = resample_to_model_rate(samples, sample_rate);
    let activations = infer(&resampled)?;
    let notes = output_to_notes(&activations.frames, &activations.onsets);
    if notes.is_empty() {
        return Err(TranscribeError::Empty);
    }
    let quality = quality_of(&notes);
    Ok((notes_to_midi(&notes), quality))
}

/// Compute the quality aggregates from decoded note events.
fn quality_of(notes: &[NoteEvent]) -> TranscriptionQuality {
    let note_count = notes.len();
    let mean_confidence = notes.iter().map(|n| n.amplitude).sum::<f32>() / note_count.max(1) as f32;

    // A note is "overlapping" if its span intersects any other note's span.
    let mut overlapping = 0usize;
    for (i, a) in notes.iter().enumerate() {
        let hit = notes
            .iter()
            .enumerate()
            .any(|(j, b)| i != j && a.start_frame < b.end_frame && b.start_frame < a.end_frame);
        if hit {
            overlapping += 1;
        }
    }
    let polyphony = overlapping as f32 / note_count as f32;

    let uncertain_count = notes
        .iter()
        .filter(|n| n.amplitude < UNCERTAIN_NOTE_CONFIDENCE)
        .count();

    TranscriptionQuality {
        note_count,
        mean_confidence,
        polyphony,
        uncertain_count,
        verdict: classify_polyphony(polyphony, mean_confidence),
    }
}

/// The honesty-gate rule (#331): simultaneous-note density bands the verdict,
/// and a weak overall confidence escalates an already-overlapping input to a
/// full mix — dense chords alone, or overlap plus weakness, both mean the
/// notes may be approximate.
fn classify_polyphony(polyphony: f32, mean_confidence: f32) -> PolyphonyVerdict {
    if polyphony >= FULL_MIX_POLYPHONY
        || (polyphony > BORDERLINE_POLYPHONY && mean_confidence < LOW_CONFIDENCE)
    {
        PolyphonyVerdict::FullMix
    } else if polyphony > BORDERLINE_POLYPHONY {
        PolyphonyVerdict::Borderline
    } else {
        PolyphonyVerdict::Mono
    }
}

/// Decode `bytes` to mono samples, then transcribe to MIDI in one step.
///
/// Convenience for the import command path (decode + [`audio_to_midi`]).
pub fn transcribe_audio_bytes(
    bytes: Vec<u8>,
    extension: Option<&str>,
) -> Result<Vec<u8>, TranscribeError> {
    transcribe_audio_bytes_with_quality(bytes, extension).map(|(bytes, _)| bytes)
}

/// Like [`transcribe_audio_bytes`], but also returns a [`TranscriptionQuality`].
pub fn transcribe_audio_bytes_with_quality(
    bytes: Vec<u8>,
    extension: Option<&str>,
) -> Result<(Vec<u8>, TranscriptionQuality), TranscribeError> {
    let (samples, sample_rate) = decode_audio(bytes, extension)?;
    audio_to_midi_with_quality(&samples, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start_frame: usize, end_frame: usize, amplitude: f32) -> NoteEvent {
        NoteEvent {
            start_frame,
            end_frame,
            midi: 60,
            amplitude,
        }
    }

    /// `n` fully sequential (never overlapping) notes at `amplitude`.
    fn sequential(n: usize, amplitude: f32) -> Vec<NoteEvent> {
        (0..n).map(|i| ev(i * 20, i * 20 + 15, amplitude)).collect()
    }

    #[test]
    fn clean_sequential_line_reads_mono_with_no_uncertain_notes() {
        let q = quality_of(&sequential(8, 0.8));
        assert_eq!(q.verdict, PolyphonyVerdict::Mono);
        assert_eq!(q.note_count, 8);
        assert_eq!(q.uncertain_count, 0);
        assert_eq!(q.polyphony, 0.0);
    }

    /// An overlap chain A-B-C (A and C don't touch) = 3 overlapping notes,
    /// followed by `extra` sequential filler notes far to the right.
    fn chain_plus(extra: usize, amp: f32) -> Vec<NoteEvent> {
        let mut notes = vec![ev(0, 10, amp), ev(5, 20, amp), ev(15, 25, amp)];
        notes.extend((0..extra).map(|i| ev(1000 + i * 20, 1000 + i * 20 + 15, amp)));
        notes
    }

    /// Exactly AT the borderline threshold stays Mono (the gate is strict-
    /// greater, matching the pre-#331 `polyphonic` flag's semantics); the
    /// next fixture just above it (3/19 ≈ 0.158) tips to Borderline. Pins
    /// `BORDERLINE_POLYPHONY` from BOTH sides: nudging the constant down
    /// fails the first assert, up fails the second.
    #[test]
    fn borderline_threshold_is_pinned_at_0_15() {
        // 3 of 20 overlapping = 0.15 exactly.
        let q = quality_of(&chain_plus(17, 0.8));
        assert!((q.polyphony - 0.15).abs() < 1e-6, "fixture drifted: {q:?}");
        assert_eq!(q.verdict, PolyphonyVerdict::Mono);

        // 3 of 19 overlapping ≈ 0.158 — just past the gate.
        let q = quality_of(&chain_plus(16, 0.8));
        assert!(
            (q.polyphony - 3.0 / 19.0).abs() < 1e-6,
            "fixture drifted: {q:?}"
        );
        assert_eq!(q.verdict, PolyphonyVerdict::Borderline);
    }

    /// Exactly AT the full-mix threshold reads FullMix (`>=`); just below
    /// (6/13 ≈ 0.46) stays Borderline. Pins `FULL_MIX_POLYPHONY` from BOTH
    /// sides: nudging the constant up fails the first assert, down fails
    /// the second.
    #[test]
    fn full_mix_threshold_is_pinned_at_0_5() {
        // 3 of 6 overlapping = 0.5 → FullMix.
        let q = quality_of(&chain_plus(3, 0.8));
        assert!((q.polyphony - 0.5).abs() < 1e-6, "fixture drifted: {q:?}");
        assert_eq!(q.verdict, PolyphonyVerdict::FullMix);

        // Two chains = 6 of 13 overlapping ≈ 0.46 → still Borderline at
        // high confidence.
        let mut notes = chain_plus(7, 0.8);
        notes.extend([ev(500, 510, 0.8), ev(505, 520, 0.8), ev(515, 525, 0.8)]);
        let q = quality_of(&notes);
        assert!(
            (q.polyphony - 6.0 / 13.0).abs() < 1e-6,
            "fixture drifted: {q:?}"
        );
        assert_eq!(q.verdict, PolyphonyVerdict::Borderline);
    }

    /// Overlap + weak overall confidence escalates Borderline to FullMix —
    /// and the same overlap at healthy confidence does not. The means sit
    /// tight (0.38 / 0.42) around `LOW_CONFIDENCE`, pinning the escalation
    /// gate from BOTH sides.
    #[test]
    fn weak_confidence_escalates_borderline_overlap_to_full_mix() {
        let weak = quality_of(&chain_plus(5, 0.38));
        assert!(weak.mean_confidence < LOW_CONFIDENCE);
        assert_eq!(weak.verdict, PolyphonyVerdict::FullMix);

        let healthy = quality_of(&chain_plus(5, 0.42));
        assert!(healthy.mean_confidence >= LOW_CONFIDENCE);
        assert_eq!(healthy.verdict, PolyphonyVerdict::Borderline);
    }

    /// Both boundary operators of the escalation clause, at exact equality:
    /// polyphony exactly AT the borderline threshold never escalates however
    /// weak the notes (the clause is strict-greater), and a mean confidence
    /// exactly AT `LOW_CONFIDENCE` is not "below" it (strict-less). Each
    /// kills the `>`→`>=` / `<`→`<=` mutation on the escalation clause.
    /// (Tested on the classifier directly: `quality_of`'s float
    /// accumulation can't hit these boundaries bit-exactly.)
    #[test]
    fn escalation_clause_boundaries_are_exclusive() {
        // Weak notes at exactly the borderline overlap: still Mono.
        assert_eq!(
            classify_polyphony(BORDERLINE_POLYPHONY, 0.2),
            PolyphonyVerdict::Mono
        );
        // Borderline overlap at exactly LOW_CONFIDENCE: not escalated.
        assert_eq!(
            classify_polyphony(0.2, LOW_CONFIDENCE),
            PolyphonyVerdict::Borderline
        );
    }

    /// Weak confidence alone (no overlapping notes) is NOT a full mix — a
    /// quiet, distant, but clearly single-line recording keeps its Mono
    /// verdict; `low_confidence` is the separate signal for that.
    #[test]
    fn weak_confidence_without_overlap_stays_mono() {
        let q = quality_of(&sequential(8, 0.3));
        assert!(q.mean_confidence < LOW_CONFIDENCE);
        assert_eq!(q.verdict, PolyphonyVerdict::Mono);
    }

    /// Uncertain notes are counted strictly below the constant: 0.49 counts,
    /// 0.50 does not. Pins `UNCERTAIN_NOTE_CONFIDENCE`.
    #[test]
    fn uncertain_count_is_pinned_to_the_note_confidence_threshold() {
        let notes = vec![
            ev(0, 15, 0.49),
            ev(20, 35, UNCERTAIN_NOTE_CONFIDENCE),
            ev(40, 55, 0.8),
        ];
        let q = quality_of(&notes);
        assert_eq!(q.uncertain_count, 1);
    }
}
