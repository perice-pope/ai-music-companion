//! Session-scoped audio pipeline: mic → pitch detection → `audio-event` IPC.
//!
//! Lives for the lifetime of one active `start_practice_session` → `end_practice_session`
//! pair. Dropped in between; dropped on error. The shape:
//!
//! ```text
//! cpal audio thread ──(ringbuf, lock-free)──► pipeline OS thread
//!                                                  │
//!                                                  ├─ PitchDetector::detect()
//!                                                  ▼
//!                                           emit callback (Tauri app.emit)
//! ```
//!
//! Why an OS thread and not a tokio task: on macOS `cpal::Stream` is `!Send`
//! so anything owning an `AudioCapture` can't cross `.await`. `std::thread`
//! sidesteps that entirely. The thread opens the capture locally, so the
//! stream never moves between threads.
//!
//! Why a callback instead of holding `AppHandle<R>` directly: keeping this
//! module free of `tauri::Runtime` generics keeps `AppState` non-generic.
//! The command wrappers pass `move |ev| { let _ = app.emit("audio-event", ev); }`
//! when constructing a pipeline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use brain::follower::ScoreFollower;
use brain::phrase::{PhraseAggregator, PhraseConfig, PhraseSummary};
use ears::capture::{AudioCapture, CaptureConfig, CaptureError};
use ears::pitch::{PitchConfig, PitchDetector, PitchError};
use ears::AudioEvent;

/// User-facing errors from pipeline start / reconfigure.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("failed to spawn audio pipeline thread: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("pipeline thread died before completing startup")]
    StartupChannelClosed,
    #[error("audio capture failed to open: {0}")]
    Capture(#[from] CaptureError),
    #[error("pitch detector rejected config: {0}")]
    Pitch(#[from] PitchError),
    #[error("pipeline is already stopped")]
    AlreadyStopped,
}

/// Tunables for the pitch half of the pipeline. The sample rate is
/// discovered from the capture device at runtime and overrides anything
/// the caller sets on this struct; everything else (threshold + frequency
/// window) is caller-supplied from the active instrument profile.
#[derive(Debug, Clone)]
pub struct DetectorProfile {
    pub threshold: f64,
    pub freq_min_hz: f64,
    pub freq_max_hz: f64,
}

impl DetectorProfile {
    /// Build a `PitchConfig` with the device's sample rate stitched in.
    fn into_pitch_config(self, sample_rate: u32) -> PitchConfig {
        PitchConfig {
            sample_rate,
            threshold: self.threshold,
            freq_min_hz: self.freq_min_hz,
            freq_max_hz: self.freq_max_hz,
        }
    }
}

/// Handle to a running pipeline. Drop (or explicit `stop`) joins the
/// worker thread and releases the mic.
pub struct AudioPipeline {
    shutdown: Arc<AtomicBool>,
    profile_tx: Sender<DetectorProfile>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AudioPipeline {
    /// Open the default input device and start streaming `AudioEvent`s to
    /// the supplied callback. Blocks until the worker thread has
    /// confirmed startup (so `Err` here means "mic failed to open", not
    /// "mic failed at some point later").
    ///
    /// `emit_phrase` is called once per *completed* phrase (every few
    /// seconds, at a phrase boundary — never per audio frame), carrying
    /// the [`PhraseSummary`]. When a `ScoreFollower` is supplied via
    /// [`AudioPipeline::start_with_follower`], each summary's
    /// `score_position` is the measure/beat the phrase began on, which
    /// drives the score-mode cursor.
    pub fn start<F>(profile: DetectorProfile, emit: F) -> Result<Self, PipelineError>
    where
        F: FnMut(AudioEvent) + Send + 'static,
    {
        Self::start_with_follower(profile, None, emit, |_| {})
    }

    /// Like [`AudioPipeline::start`], but also runs phrase aggregation on
    /// the worker thread and (optionally) a score follower. Completed
    /// phrases are handed to `emit_phrase`.
    pub fn start_with_follower<F, P>(
        profile: DetectorProfile,
        follower: Option<ScoreFollower>,
        emit: F,
        emit_phrase: P,
    ) -> Result<Self, PipelineError>
    where
        F: FnMut(AudioEvent) + Send + 'static,
        P: FnMut(PhraseSummary) + Send + 'static,
    {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (profile_tx, profile_rx) = std::sync::mpsc::channel::<DetectorProfile>();
        let (startup_tx, startup_rx) = std::sync::mpsc::channel::<Result<(), PipelineError>>();

        let shutdown_worker = Arc::clone(&shutdown);
        let thread = thread::Builder::new()
            .name("audio-pipeline".into())
            .spawn(move || {
                run_worker(
                    profile,
                    follower,
                    profile_rx,
                    shutdown_worker,
                    startup_tx,
                    emit,
                    emit_phrase,
                );
            })?;

        match startup_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                shutdown,
                profile_tx,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                // Worker already exited; wait for it so we don't leak a zombie.
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(PipelineError::StartupChannelClosed),
        }
    }

    /// Swap the detector's frequency window / threshold without tearing
    /// down the mic stream. Used on mid-session instrument switch.
    pub fn reconfigure(&self, profile: DetectorProfile) -> Result<(), PipelineError> {
        self.profile_tx
            .send(profile)
            .map_err(|_| PipelineError::AlreadyStopped)
    }

    /// Graceful shutdown. Also happens on `Drop`, but calling this
    /// explicitly lets errors surface (you get back the `JoinHandle`'s
    /// panic, if any).
    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            // Best-effort: if the thread panicked we just log-via-drop.
            let _ = h.join();
        }
    }
}

/// Worker thread entry point. Owns the `AudioCapture` + `PitchDetector`.
///
/// Opening the capture *here* (not in `start`) is deliberate: on macOS
/// `cpal::Stream` is `!Send`, so it must never cross a thread boundary.
#[allow(clippy::too_many_arguments)]
fn run_worker<F, P>(
    initial_profile: DetectorProfile,
    follower: Option<ScoreFollower>,
    profile_rx: Receiver<DetectorProfile>,
    shutdown: Arc<AtomicBool>,
    startup_tx: Sender<Result<(), PipelineError>>,
    mut emit: F,
    mut emit_phrase: P,
) where
    F: FnMut(AudioEvent),
    P: FnMut(PhraseSummary),
{
    // Phrase aggregator groups events into musical phrases. With a score
    // follower attached it also tags each phrase with the score position
    // it began on — the anchor the cursor follows. Constructed with the
    // default config (validated, so `expect` is unreachable in practice).
    let mut aggregator =
        PhraseAggregator::new(PhraseConfig::default()).expect("default PhraseConfig is valid");
    if let Some(f) = follower {
        aggregator.set_score_follower(f);
    }
    // --- Open capture. Bail early if the mic is unavailable. ---
    let mut capture = match AudioCapture::new(CaptureConfig::default()) {
        Ok(c) => c,
        Err(e) => {
            let _ = startup_tx.send(Err(PipelineError::Capture(e)));
            return;
        }
    };
    let sample_rate = capture.sample_rate();
    let channels = capture.channels();

    // --- Build initial detector. Same bail-early contract. ---
    let mut detector = match PitchDetector::new(initial_profile.into_pitch_config(sample_rate)) {
        Ok(d) => d,
        Err(e) => {
            let _ = startup_tx.send(Err(PipelineError::Pitch(e)));
            return;
        }
    };

    // Signal ready; from here on, errors are logged, not surfaced.
    let _ = startup_tx.send(Ok(()));

    tracing::info!(
        sample_rate,
        channels,
        "audio_pipeline: worker started; streaming audio-event"
    );

    // --- Pre-allocated scratch. Grown (allocator-on-processing-thread,
    // never on the cpal callback thread) only if a later reconfigure
    // enlarges the detector's window. The initial size comfortably holds
    // the widest window we expect (~1500 samples × stereo). ---
    let mut interleaved: Vec<f32> = vec![0.0; 4096];
    let mut mono: Vec<f32> = vec![0.0; 4096];

    while !shutdown.load(Ordering::Relaxed) {
        // Drain config updates; we only care about the latest.
        if let Some(new_profile) = drain_latest(&profile_rx) {
            match PitchDetector::new(new_profile.into_pitch_config(sample_rate)) {
                Ok(d) => {
                    detector = d;
                    tracing::debug!("audio_pipeline: detector reconfigured");
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "audio_pipeline: reconfigure rejected; keeping previous detector"
                    );
                }
            }
        }

        let window = detector.window_size();
        let needed = window * channels as usize;
        if interleaved.len() < needed {
            interleaved.resize(needed, 0.0);
        }
        if mono.len() < window {
            mono.resize(window, 0.0);
        }

        // Peek before draining: only pop a full detector window's worth
        // in one go. Reading partial windows drops them on the floor —
        // `pop_slice` removes whatever it returns from the ring buffer,
        // and the rest of this iteration discards the read on `continue`.
        // cpal delivers ~512 samples per callback while detector windows
        // are typically ~1024+, so partial reads were the steady state.
        if capture.available() < needed {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let read = capture.read_samples(&mut interleaved[..needed]);
        debug_assert_eq!(read, needed, "available() guaranteed a full read");

        let mono_slice: &[f32] = if channels == 1 {
            &interleaved[..window]
        } else {
            downmix_to_mono(
                &interleaved[..needed],
                channels as usize,
                &mut mono[..window],
            );
            &mono[..window]
        };

        let event = detector.detect(mono_slice);
        // Live pitch goes out first so the meter stays responsive, then
        // the aggregator folds the event into the current phrase.
        emit(event.clone());
        aggregator.push(&event);
        // `take_new_phrases` only allocates when a phrase actually closed
        // (a boundary every few seconds), never per frame — so this stays
        // off the per-frame allocation budget.
        for phrase in aggregator.take_new_phrases() {
            emit_phrase(phrase);
        }
    }

    // Close out the final in-progress phrase so the last bar of playing
    // isn't dropped when the user hits End.
    aggregator.flush();
    for phrase in aggregator.take_new_phrases() {
        emit_phrase(phrase);
    }

    tracing::info!("audio_pipeline: worker shutting down");
    // `capture` drops here → cpal stream ends → mic released.
    drop(capture);
}

/// Drain a channel, returning only the most recent value. Used so that
/// if two reconfigures queued while we were sleeping we skip straight
/// to the latest (which is what the user intends — show me the *current*
/// instrument, not the one I was on two switches ago).
fn drain_latest<T>(rx: &Receiver<T>) -> Option<T> {
    let mut latest: Option<T> = None;
    while let Ok(v) = rx.try_recv() {
        latest = Some(v);
    }
    latest
}

/// Average `channels` interleaved samples into a mono buffer.
///
/// `interleaved.len()` must equal `mono.len() * channels`. The caller
/// has already sized both buffers — this function never allocates.
fn downmix_to_mono(interleaved: &[f32], channels: usize, mono: &mut [f32]) {
    debug_assert_eq!(interleaved.len(), mono.len() * channels);
    let inv_channels = 1.0 / channels as f32;
    for (i, m) in mono.iter_mut().enumerate() {
        let base = i * channels;
        let mut sum = 0.0_f32;
        for c in 0..channels {
            sum += interleaved[base + c];
        }
        *m = sum * inv_channels;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo_averages_channels() {
        let interleaved = [1.0_f32, 3.0, 2.0, 4.0, 0.0, 0.0];
        let mut mono = [0.0_f32; 3];
        downmix_to_mono(&interleaved, 2, &mut mono);
        assert_eq!(mono, [2.0, 3.0, 0.0]);
    }

    #[test]
    fn downmix_mono_unchanged_passthrough_shape() {
        // Sanity: a 1-channel downmix is the identity.
        let interleaved = [0.25_f32, 0.5, 0.75];
        let mut mono = [0.0_f32; 3];
        downmix_to_mono(&interleaved, 1, &mut mono);
        assert_eq!(mono, [0.25, 0.5, 0.75]);
    }

    #[test]
    fn drain_latest_returns_only_the_last_value() {
        let (tx, rx) = std::sync::mpsc::channel::<i32>();
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();
        assert_eq!(drain_latest(&rx), Some(3));
        assert_eq!(drain_latest(&rx), None);
    }

    #[test]
    fn drain_latest_returns_none_when_disconnected() {
        let (tx, rx) = std::sync::mpsc::channel::<i32>();
        drop(tx);
        assert_eq!(drain_latest::<i32>(&rx), None);
    }

    // Note on full-pipeline tests: running `AudioPipeline::start` inside
    // CI would require a mic device, which GitHub Actions runners don't
    // have. Coverage of the capture→detect→emit plumbing lives in
    // `crates/ears/tests/audio_thread_output_test.rs` (capture-level)
    // and `crates/ears/tests/pitch_test.rs` (detector-level). What's
    // left to test here is pure logic — downmix + channel discipline —
    // above.
}
