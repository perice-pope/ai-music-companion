//! Error type for the transcription pipeline.

use thiserror::Error;

/// Failures from decoding, inference, or note creation.
#[derive(Debug, Error)]
pub enum TranscribeError {
    /// Audio could not be decoded from the provided bytes.
    #[error("could not decode audio: {0}")]
    Decode(String),

    /// ONNX Runtime / model error (e.g. the runtime library is unavailable).
    #[error("transcription runtime error: {0}")]
    Runtime(String),

    /// The input contained no detectable notes.
    #[error("no notes detected")]
    Empty,
}
