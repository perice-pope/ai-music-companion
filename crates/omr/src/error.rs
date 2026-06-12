//! Error type for the OMR (Optical Music Recognition) pipeline.

use thiserror::Error;

/// Failures from PDF validation or the recognition engine.
#[derive(Debug, Error)]
pub enum OmrError {
    /// The provided bytes are not a PDF document.
    #[error("that doesn't look like a PDF file")]
    NotPdf,

    /// The recognition engine could not be found or launched. This is the
    /// honest signal when the bundled sidecar is absent (e.g. a dev build
    /// without the engine fetched) — never a pretend result.
    #[error("the sheet-music reader isn't available in this build: {0}")]
    EngineUnavailable(String),

    /// The recognition engine ran but failed.
    #[error("the sheet-music reader failed: {0}")]
    Engine(String),

    /// The engine produced output that isn't usable MusicXML — the scan didn't
    /// yield readable sheet music.
    #[error("couldn't read any sheet music from that scan")]
    Empty,
}
