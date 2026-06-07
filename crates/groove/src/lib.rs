//! Rhythm & groove analysis — Phase 4, Track A2 (`crates/groove`).
//!
//! This crate sits on the **processing thread** (not the real-time audio
//! thread), so heap allocation, sorting, and floating-point arithmetic are
//! all fine.  It consumes a slice of onset timestamps (in seconds) and
//! produces a [`GrooveDescriptor`] — descriptors of tempo, swing, and timing
//! consistency — that will flow into the `MusicalFingerprint` assembled in a
//! later PR.
//!
//! # Design rationale (from `story-phase4-musical-relevance.md` §2 "A2")
//!
//! The current codebase hard-codes `rhythmic_stability = 1.0` in
//! `crates/brain/src/scoring.rs:67`.  This crate replaces that placeholder
//! with honest measurement.  The honesty contract is strict:
//!
//! - Return `None` / low-confidence when there are not enough onsets to form
//!   a judgement.  Never emit a made-up number.
//! - All fields that require more evidence than the input can supply are
//!   `Option<f32>` and will be `None` rather than a guess.
//! - The `timing_consistency` field is normalized, so callers can gate on it
//!   without knowing the units of IOI variance.
//!
//! # Entry points
//!
//! ```rust
//! use groove::{analyze_groove, onsets_from_events};
//! ```
//!
//! `onsets_from_events` extracts onset timestamps from a `&[ears::AudioEvent]`
//! slice.  `analyze_groove` accepts a `&[f64]` of onset times so it can be
//! tested with plain timestamps independently of the audio pipeline.
//!
//! # Lay-back / feel vs. a beat grid
//!
//! Measuring "lay-back" (playing behind the beat) requires an external beat
//! grid or metronome reference — without one we can only measure *internal*
//! consistency of the performer's own timing.  That external-grid comparison
//! is a follow-up: when a score or click track is loaded, the brain crate can
//! compute per-onset offsets (in ms) against the expected grid beat and pass
//! them here.  For now, `GrooveDescriptor` leaves a doc note marking the field
//! as a future extension.

mod analyze;

pub use analyze::{analyze_groove, onsets_from_events, GrooveDescriptor};
