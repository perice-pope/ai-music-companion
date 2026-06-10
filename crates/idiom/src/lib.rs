//! Idiom / musical-language recognition (audio-embedding track) for AI Music
//! Companion.
//!
//! This is the **audio-embedding retrieval** half of the Phase 4 idiom
//! sub-track (`docs/design/story-phase4-musical-relevance.md` §3B): embed a
//! played phrase, then find the nearest known idioms/styles in a reference
//! corpus. It is the honest *"reminds me of / shares a neighbourhood with"*
//! layer — it **never asserts a hard fact**, and it stays **silent** below a
//! configurable confidence threshold rather than guessing.
//!
//! ## Engine only — no wiring
//!
//! This crate is deliberately self-contained. It does **not** depend on
//! `brain`, `ears`, the Tauri shell, or the frontend, and nothing here is
//! wired into them; that integration is a later step. Keeping it isolated is
//! what lets it land in parallel.
//!
//! ## Offline, not real-time
//!
//! Idiom recognition is reflective, end-of-phrase / on-demand analysis, **not**
//! on the <25 ms mic-to-screen hot path. Allocation is therefore fine
//! throughout this crate (unlike `crates/ears`). See [`embed`] for detail.
//!
//! ## The pieces
//!
//! - [`IdiomEmbedder`] — the pluggable embedding trait. A MERT-class
//!   transformer is the intended future drop-in; [`BaselineEmbedder`] is a
//!   real, deterministic classic-DSP implementation that runs today.
//! - [`Corpus`] — a validated, seed reference corpus loaded from
//!   `data/corpus.json`.
//! - [`match_idiom`] — cosine top-k retrieval with the confidence gate.
//!
//! ## Example
//!
//! ```no_run
//! use idiom::{BaselineEmbedder, Corpus, IdiomEmbedder, match_idiom};
//!
//! # fn demo(samples: &[f32]) -> Result<(), idiom::IdiomError> {
//! let embedder = BaselineEmbedder::new();
//! let corpus = Corpus::seed()?;
//! let query = embedder.embed(samples, 44_100)?;
//! // Only returns matches above 0.6 cosine similarity; silent otherwise.
//! let matches = match_idiom(&query, &corpus, 3, 0.6)?;
//! for m in matches {
//!     println!("reminds me of {} ({}, {})", m.label, m.genre, m.exemplar_artist);
//! }
//! # Ok(())
//! # }
//! ```

mod corpus;
mod embed;
mod error;
mod retrieve;

pub use corpus::{Corpus, CorpusEntry};
pub use embed::{BaselineEmbedder, IdiomEmbedder, BASELINE_DIM};
pub use error::IdiomError;
pub use retrieve::{cosine_similarity, match_idiom, IdiomMatch};
