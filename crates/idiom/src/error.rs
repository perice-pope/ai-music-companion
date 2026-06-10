//! Error type for the idiom-recognition engine.

use thiserror::Error;

/// Failures from embedding, corpus loading, or retrieval.
#[derive(Debug, Error)]
pub enum IdiomError {
    /// The audio buffer handed to an embedder was empty.
    #[error("cannot embed empty audio")]
    EmptyInput,

    /// The sample rate was zero or otherwise unusable.
    #[error("invalid sample rate: {0} Hz")]
    InvalidSampleRate(u32),

    /// The corpus JSON could not be parsed.
    #[error("could not parse corpus: {0}")]
    CorpusParse(String),

    /// The corpus failed validation (e.g. mismatched embedding dimensions,
    /// non-finite values, or an empty corpus). Carries a human-readable reason.
    #[error("invalid corpus: {0}")]
    CorpusInvalid(String),

    /// A query vector's dimension did not match the corpus embedding dimension.
    #[error("dimension mismatch: query has {query} dims, corpus has {corpus}")]
    DimensionMismatch {
        /// Length of the query embedding.
        query: usize,
        /// Embedding dimension declared by the corpus.
        corpus: usize,
    },
}
