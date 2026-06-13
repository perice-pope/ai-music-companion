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
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use brain::follower::{ScoreFollower, ScorePosition};
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

// ---------------------------------------------------------------------------
// Session-scoped idiom capture buffer
// ---------------------------------------------------------------------------

/// Target sample rate (Hz) the idiom buffer downsamples to. The idiom
/// embedder is a coarse chroma/MFCC feature extractor over ~46 ms windows;
/// it captures harmonic/timbral colour, which survives decimation to ~22 kHz
/// comfortably (Nyquist ~11 kHz still covers the spectral content the baseline
/// embedder uses). Halving the rate halves the memory footprint of the
/// session-scope buffer for free.
pub const IDIOM_TARGET_SAMPLE_RATE: u32 = 22_050;

/// Hard cap on retained idiom samples (~120 s at the target rate). Idiom
/// matching needs a representative slice of the session, not every sample —
/// once we've buffered this much we stop appending so a marathon session can't
/// grow the buffer without bound. This is the documented tradeoff: we trade
/// total-session fidelity for a fixed memory ceiling, off the hot path.
pub const IDIOM_MAX_SAMPLES: usize = (IDIOM_TARGET_SAMPLE_RATE as usize) * 120;

/// Mono PCM accumulated for end-of-session idiom analysis, plus the rate it
/// was captured at. Bounded and downsampled (see the consts above).
#[derive(Debug, Default)]
pub struct IdiomCapture {
    /// Downsampled mono samples, capped at [`IDIOM_MAX_SAMPLES`].
    pub samples: Vec<f32>,
    /// Effective sample rate of `samples` (the rate idiom analysis must use).
    pub sample_rate: u32,
}

/// A session-scoped, **off-hot-path** handle to the idiom capture buffer.
///
/// Shared between the Tauri shell (which holds it on `AppState` and reads it
/// when building the recap) and the audio-pipeline **worker thread** (which
/// appends downsampled mono audio as phrases stream by). Crucially this is
/// touched only on the processing/worker side and the command side — **never**
/// inside the realtime cpal callback — so the `Mutex` lock and `Vec` growth
/// here do not violate the no-allocation-in-audio-thread rule.
///
/// Fully offline: this just retains samples in memory for on-device analysis;
/// nothing here touches the network.
#[derive(Clone, Default)]
pub struct SharedIdiomBuffer(Arc<Mutex<IdiomCapture>>);

impl SharedIdiomBuffer {
    /// A fresh, empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `src` (mono at `src_rate` Hz) into the buffer, decimating toward
    /// [`IDIOM_TARGET_SAMPLE_RATE`] and stopping once [`IDIOM_MAX_SAMPLES`] is
    /// reached. Called on the worker thread, never the cpal callback.
    ///
    /// `pub(crate)` so the command-layer tests can seed a buffer without a live
    /// mic; production code only calls it from the worker thread.
    pub(crate) fn append_downsampled(&self, src: &[f32], src_rate: u32) {
        if src.is_empty() || src_rate == 0 {
            return;
        }
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return, // poisoned: drop the chunk rather than panic.
        };
        if guard.samples.len() >= IDIOM_MAX_SAMPLES {
            return;
        }
        // Integer decimation factor toward the target rate (>= 1). At 44.1 kHz
        // source this is 2 → ~22.05 kHz; at 22.05 kHz it's 1 → passthrough.
        let factor = (src_rate / IDIOM_TARGET_SAMPLE_RATE).max(1) as usize;
        guard.sample_rate = src_rate / factor as u32;
        for chunk in src.chunks(factor) {
            if guard.samples.len() >= IDIOM_MAX_SAMPLES {
                break;
            }
            // Take the first sample of each decimation window. A crude
            // anti-alias (no low-pass) is acceptable here: the baseline
            // embedder's features are robust to it, and the alternative
            // (a proper filter) isn't worth the cost off the hot path for a
            // "reminds me of" proximity signal.
            guard.samples.push(chunk[0]);
        }
    }

    /// Take a snapshot of the captured samples + their rate, leaving the buffer
    /// in place. Called from the recap path at session end.
    pub fn snapshot(&self) -> (Vec<f32>, u32) {
        match self.0.lock() {
            Ok(g) => (g.samples.clone(), g.sample_rate),
            Err(_) => (Vec::new(), 0),
        }
    }

    /// Clear the buffer (e.g. at the start of a new session).
    pub fn clear(&self) {
        if let Ok(mut g) = self.0.lock() {
            g.samples.clear();
            g.sample_rate = 0;
        }
    }
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
    /// Per-instrument voiced-confidence gate fed to the phrase aggregator so
    /// quiet/breathy playing (notably Voice) still counts as practice (#185).
    pub voiced_confidence_threshold: f64,
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
        Self::start_with_follower(profile, None, None, emit, |_| {}, |_| {})
    }

    /// Like [`AudioPipeline::start`], but also runs phrase aggregation on
    /// the worker thread and (optionally) a score follower.
    ///
    /// - `emit_phrase` fires once per completed phrase (boundary cadence,
    ///   seconds apart), carrying a [`PhraseSummary`].
    /// - `emit_position` fires at ~10 Hz with the follower's live
    ///   [`ScorePosition`] — fine-grained enough for a smoothly gliding
    ///   cursor *within* a measure, while staying well off the per-frame
    ///   path. It never fires in free play (no follower attached).
    ///
    /// `idiom_buffer`, when supplied, receives the session's downsampled mono
    /// audio for **offline, end-of-session** idiom analysis. It is filled on
    /// the worker thread (allocation there is fine), never in the realtime
    /// callback, and read by the recap path after the session ends.
    pub fn start_with_follower<F, P, S>(
        profile: DetectorProfile,
        follower: Option<ScoreFollower>,
        idiom_buffer: Option<SharedIdiomBuffer>,
        emit: F,
        emit_phrase: P,
        emit_position: S,
    ) -> Result<Self, PipelineError>
    where
        F: FnMut(AudioEvent) + Send + 'static,
        P: FnMut(PhraseSummary) + Send + 'static,
        S: FnMut(ScorePosition) + Send + 'static,
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
                    idiom_buffer,
                    profile_rx,
                    shutdown_worker,
                    startup_tx,
                    emit,
                    emit_phrase,
                    emit_position,
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
fn run_worker<F, P, S>(
    initial_profile: DetectorProfile,
    follower: Option<ScoreFollower>,
    idiom_buffer: Option<SharedIdiomBuffer>,
    profile_rx: Receiver<DetectorProfile>,
    shutdown: Arc<AtomicBool>,
    startup_tx: Sender<Result<(), PipelineError>>,
    mut emit: F,
    mut emit_phrase: P,
    mut emit_position: S,
) where
    F: FnMut(AudioEvent),
    P: FnMut(PhraseSummary),
    S: FnMut(ScorePosition),
{
    /// Minimum spacing between live score-position emits. ~10 Hz is smooth
    /// enough for the cursor to glide within a measure without flooding IPC
    /// (the detect loop runs ~40–50 Hz). Driven off event timestamps, not
    /// wall-clock, so it tracks audio time exactly.
    const POSITION_EMIT_INTERVAL_SECS: f64 = 0.1;

    // Phrase aggregator groups events into musical phrases. With a score
    // follower attached it also tags each phrase with the score position
    // it began on — the anchor the cursor follows. The voiced-confidence gate
    // comes from the active instrument profile so quiet, breathy singing still
    // forms phrases (#185); the rest of the config is the validated default.
    let mut aggregator = PhraseAggregator::new(PhraseConfig {
        voiced_confidence_threshold: initial_profile.voiced_confidence_threshold,
        ..PhraseConfig::default()
    })
    .expect("PhraseConfig derived from the instrument profile is valid");
    let has_follower = follower.is_some();
    if let Some(f) = follower {
        aggregator.set_score_follower(f);
    }
    // Timestamp of the last position emit, for 10 Hz downsampling. `None`
    // until the first emit so the cursor moves immediately on first audio.
    let mut last_position_emit_secs: Option<f64> = None;
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

    // Tone accumulation. We gather the current phrase's mono audio and its
    // per-window pitch contour on the processing thread (Vec growth is expected
    // here, never on the cpal callback), and compute a tone descriptor when the
    // phrase closes. Capped so a never-ending sound can't grow unbounded.
    const MAX_TONE_SAMPLES: usize = 22_050 * 30; // ~30 s
    let mut phrase_audio: Vec<f32> = Vec::new();
    let mut phrase_pitch: Vec<f32> = Vec::new();

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
        let event_time = event.timestamp_secs;
        aggregator.push(&event);

        // Accumulate this window's audio + pitch for the in-progress phrase.
        phrase_audio.extend_from_slice(mono_slice);
        phrase_pitch.push(event.pitch_hz.unwrap_or(0.0) as f32);

        // Feed the session-scoped idiom buffer (offline, end-of-session
        // analysis). This is on the *worker* thread — the lock + Vec growth
        // are deliberately off the realtime cpal callback, so the
        // no-allocation-in-audio-thread rule is upheld. The buffer downsamples
        // and self-caps, so this stays cheap and bounded.
        if let Some(buf) = &idiom_buffer {
            buf.append_downsampled(mono_slice, sample_rate);
        }
        if phrase_audio.len() > MAX_TONE_SAMPLES {
            let overflow = phrase_audio.len() - MAX_TONE_SAMPLES;
            phrase_audio.drain(..overflow);
        }

        // `take_new_phrases` only allocates when a phrase actually closed
        // (a boundary every few seconds), never per frame — so this stays
        // off the per-frame allocation budget.
        for mut phrase in aggregator.take_new_phrases() {
            // Attach a tone descriptor computed from the phrase's audio. The
            // accumulated buffer ≈ this phrase (plus any trailing silence,
            // which only dilutes the average slightly).
            phrase.tone = phrase_tone(&phrase_audio, sample_rate, &phrase_pitch);
            phrase_audio.clear();
            phrase_pitch.clear();
            emit_phrase(phrase);
        }

        // Live cursor: emit the follower's current position at ~10 Hz so it
        // glides between phrase boundaries. Skipped entirely in free play
        // (no follower → nothing to report, and `current_score_position`
        // returns `None`).
        if has_follower {
            let due = match last_position_emit_secs {
                None => true,
                Some(last) => event_time - last >= POSITION_EMIT_INTERVAL_SECS,
            };
            if due {
                if let Some(pos) = aggregator.current_score_position() {
                    emit_position(pos);
                    last_position_emit_secs = Some(event_time);
                }
            }
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

/// Minimum phrase audio (samples) for a meaningful tone descriptor — below
/// ~2 analysis windows the spectral features are too noisy to trust.
const MIN_TONE_SAMPLES: usize = 4096;

/// Compute a tone descriptor for a completed phrase from its accumulated mono
/// audio and per-window pitch contour. Returns `None` for phrases too short to
/// analyse. Uses a neutral room profile — room calibration is a later slice.
fn phrase_tone(audio: &[f32], sample_rate: u32, pitch_hz: &[f32]) -> Option<tone::ToneDescriptor> {
    if audio.len() < MIN_TONE_SAMPLES {
        return None;
    }
    let features = tone::features(audio, sample_rate);
    let contour: Vec<f32> = pitch_hz.iter().copied().filter(|&f| f > 0.0).collect();
    let f0 = if contour.is_empty() {
        None
    } else {
        Some(contour.as_slice())
    };
    Some(tone::assess(&features, f0, &tone::RoomProfile::neutral()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32) -> Vec<f32> {
        (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin() * 0.7)
            .collect()
    }

    #[test]
    fn phrase_tone_reads_bright_audio_as_brighter() {
        let sr = 44_100;
        let bright = phrase_tone(&sine(3000.0, sr), sr, &[]).expect("enough audio");
        let dark = phrase_tone(&sine(250.0, sr), sr, &[]).expect("enough audio");
        assert!(
            bright.brightness > dark.brightness,
            "bright {} should exceed dark {}",
            bright.brightness,
            dark.brightness
        );
    }

    #[test]
    fn phrase_tone_is_none_for_short_audio() {
        assert!(phrase_tone(&[0.1_f32; 100], 44_100, &[]).is_none());
    }

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

    #[test]
    fn idiom_buffer_downsamples_44k_to_target_rate() {
        let buf = SharedIdiomBuffer::new();
        // 44.1 kHz → integer factor 2 → 22.05 kHz, half the samples.
        let src: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        buf.append_downsampled(&src, 44_100);
        let (samples, rate) = buf.snapshot();
        assert_eq!(rate, 22_050, "factor-2 decimation reports the halved rate");
        assert_eq!(samples.len(), 500, "factor-2 decimation keeps every 2nd");
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[1], 2.0, "keeps the first sample of each window");
    }

    #[test]
    fn idiom_buffer_passthrough_at_target_rate() {
        let buf = SharedIdiomBuffer::new();
        // Already at the target rate → factor 1 → passthrough.
        let src: Vec<f32> = (0..100).map(|i| i as f32).collect();
        buf.append_downsampled(&src, IDIOM_TARGET_SAMPLE_RATE);
        let (samples, rate) = buf.snapshot();
        assert_eq!(rate, IDIOM_TARGET_SAMPLE_RATE);
        assert_eq!(samples.len(), 100);
    }

    #[test]
    fn idiom_buffer_caps_at_max_samples() {
        let buf = SharedIdiomBuffer::new();
        // Push more than the cap (at the target rate so factor == 1).
        let src = vec![0.5_f32; IDIOM_MAX_SAMPLES + 5_000];
        buf.append_downsampled(&src, IDIOM_TARGET_SAMPLE_RATE);
        let (samples, _) = buf.snapshot();
        assert_eq!(
            samples.len(),
            IDIOM_MAX_SAMPLES,
            "buffer must self-cap at IDIOM_MAX_SAMPLES"
        );
        // A further append is a no-op once full.
        buf.append_downsampled(&[1.0; 10], IDIOM_TARGET_SAMPLE_RATE);
        assert_eq!(buf.snapshot().0.len(), IDIOM_MAX_SAMPLES);
    }

    #[test]
    fn idiom_buffer_clear_resets() {
        let buf = SharedIdiomBuffer::new();
        buf.append_downsampled(&[1.0, 2.0, 3.0, 4.0], IDIOM_TARGET_SAMPLE_RATE);
        assert!(!buf.snapshot().0.is_empty());
        buf.clear();
        let (samples, rate) = buf.snapshot();
        assert!(samples.is_empty());
        assert_eq!(rate, 0);
    }

    #[test]
    fn idiom_buffer_ignores_empty_or_zero_rate() {
        let buf = SharedIdiomBuffer::new();
        buf.append_downsampled(&[], 44_100);
        buf.append_downsampled(&[1.0, 2.0], 0);
        assert!(buf.snapshot().0.is_empty());
    }

    // Note on full-pipeline tests: running `AudioPipeline::start` inside
    // CI would require a mic device, which GitHub Actions runners don't
    // have. Coverage of the capture→detect→emit plumbing lives in
    // `crates/ears/tests/audio_thread_output_test.rs` (capture-level)
    // and `crates/ears/tests/pitch_test.rs` (detector-level). What's
    // left to test here is pure logic — downmix + channel discipline —
    // above.
}
