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

pub use decode::decode_audio;
pub use error::TranscribeError;
pub use notes::NoteEvent;

use inference::infer;
use midi_out::notes_to_midi;
use notes::output_to_notes;
use resample::resample_to_model_rate;

/// Transcribe mono PCM `samples` (any `sample_rate`) into standard MIDI bytes.
///
/// Returns [`TranscribeError::Empty`] when no notes are detected (silence or no
/// clear pitch), and [`TranscribeError::Runtime`] when ONNX Runtime is
/// unavailable. The returned bytes parse with `midly` / the brain MIDI importer.
pub fn audio_to_midi(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, TranscribeError> {
    let resampled = resample_to_model_rate(samples, sample_rate);
    let activations = infer(&resampled)?;
    let notes = output_to_notes(&activations.frames, &activations.onsets);
    if notes.is_empty() {
        return Err(TranscribeError::Empty);
    }
    Ok(notes_to_midi(&notes))
}

/// Decode `bytes` to mono samples, then transcribe to MIDI in one step.
///
/// Convenience for the import command path (decode + [`audio_to_midi`]).
pub fn transcribe_audio_bytes(
    bytes: Vec<u8>,
    extension: Option<&str>,
) -> Result<Vec<u8>, TranscribeError> {
    let (samples, sample_rate) = decode_audio(bytes, extension)?;
    audio_to_midi(&samples, sample_rate)
}
