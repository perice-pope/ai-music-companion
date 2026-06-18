//! Library crate for the AI Music Companion Tauri shell.
//!
//! Splitting the command surface into a `lib.rs` alongside `main.rs` lets
//! unit tests compile (and run) without triggering the bin target's
//! `tauri::generate_context!` macro — which needs the packaged icons and
//! a fully-resolved Tauri config at compile time. Tests exercise the
//! command *implementations* (see [`commands::start_practice_session_impl`]
//! and friends), not the `#[tauri::command]` wrappers themselves.

pub mod audio_pipeline;
pub mod commands;
pub mod runtime;

pub use commands::{
    clear_accompaniment_key, end_practice_session, list_instruments, set_accompaniment_key,
    start_accompaniment, start_practice_session, stop_accompaniment, switch_instrument, AppState,
    CommandError, InstrumentInfo, MockCoachingService, MockRecapGenerator, SegmentChangedPayload,
    SessionPhase, SessionStatusPayload,
};
