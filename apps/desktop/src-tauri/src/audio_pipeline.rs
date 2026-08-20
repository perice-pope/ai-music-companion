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
use std::time::{Duration, Instant};

use brain::follower::{NoteVerdict, ScoreFollower, ScorePosition};
use brain::phrase::{PhraseAggregator, PhraseConfig, PhraseError, PhraseSummary};
use ears::capture::{AudioCapture, CaptureConfig, CaptureError};
use ears::output_engine::ClickFire;
use ears::pitch::{PitchConfig, PitchDetector, PitchError};
use ears::AudioEvent;
use ringbuf::traits::Consumer;
use ringbuf::HeapCons;

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
    #[error("phrase aggregator rejected profile-derived config: {0}")]
    Phrase(#[from] PhraseError),
    #[error("pipeline is already stopped")]
    AlreadyStopped,
}

// ---------------------------------------------------------------------------
// #445: time-gated click rejection
// ---------------------------------------------------------------------------

/// #445: the Pocket's click, as the audio worker sees it. Installed by
/// `start_pocket`, cleared by `teardown_pocket`, shared via
/// [`SharedClickGate`] on `AppState`. The worker drains `fires` each
/// window (PROCESSING thread — the `Mutex` lock is fine here, NEVER in a
/// render/capture callback) and converts each fire's sample index to wall
/// time so mic onsets that merely agree with our own click can be ignored.
pub struct ClickGate {
    /// Consumer half of `ears::output_engine::click_fire_channel` — the
    /// render-side metronome pushes, we pop.
    pub fires: HeapCons<ClickFire>,
    /// Wall-clock instant of the metronome's sample 0 (recorded when the
    /// Pocket's output started).
    pub epoch: Instant,
    /// The OUTPUT device's sample rate — the unit `ClickFire::sample_index`
    /// is counted in. (The input side has its own, possibly different, rate.)
    pub output_sample_rate: u32,
}

/// The shared slot the command layer installs the gate into and the audio
/// worker reads it from. `None` whenever no click is playing — the gate
/// fails OPEN (nothing is suppressed).
pub type SharedClickGate = Arc<Mutex<Option<ClickGate>>>;

/// #445: gate window before a click, in ms. Covers detector-window
/// quantization jitter (an onset is timestamped at its analysis window's
/// start, which can land just ahead of the click's wall time).
pub(crate) const CLICK_GATE_PRE_MS: f64 = 15.0;
/// #445: gate window after a click, in ms. The click itself is 30 ms
/// (`CLICK_DURATION_MS`) plus its exponential decay tail, plus
/// device/acoustic latency slop between "rendered" and "heard by the mic".
pub(crate) const CLICK_GATE_POST_MS: f64 = 90.0;
/// Max fires drained per detect window — bounds the per-window work. At
/// the Pocket's 220 BPM ceiling clicks arrive < 4/s while windows tick
/// ~40-50/s, so this is never the limiter in practice.
const CLICK_GATE_MAX_DRAIN: usize = 32;
/// Recent click wall-times retained (fixed-size ring, no allocation in
/// the loop). The gate window is ~105 ms wide and clicks are ≥ ~270 ms
/// apart, so only the nearest click can ever match; 8 is generous.
const RECENT_CLICKS: usize = 8;

/// #445: the pure gate check. `true` when the onset's wall time falls
/// within `[click − CLICK_GATE_PRE_MS, click + CLICK_GATE_POST_MS]` of any
/// recent click (both edges inclusive).
///
/// Honesty property: this can only SUPPRESS an onset that AGREES with the
/// click — a player right on the beat needs no tempo correction anyway —
/// and it PASSES disagreement, which is exactly what Follow mode needs to
/// hear. It never invents onsets, and pitch is deliberately untouched
/// (this slice scopes to tempo confusion; see the spec).
fn onset_gated(event_wall_secs: f64, recent_clicks: &[f64]) -> bool {
    let pre = CLICK_GATE_PRE_MS / 1000.0;
    let post = CLICK_GATE_POST_MS / 1000.0;
    recent_clicks.iter().any(|&click| {
        let delta = event_wall_secs - click;
        (-pre..=post).contains(&delta)
    })
}

/// Signed seconds from `b` to `a` (`a − b`), since `Instant` subtraction
/// alone can't go negative. The click epoch and the worker's session epoch
/// are recorded independently, in either order.
fn instant_offset_secs(a: Instant, b: Instant) -> f64 {
    if a >= b {
        (a - b).as_secs_f64()
    } else {
        -((b - a).as_secs_f64())
    }
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

/// The worker loop's view of an audio input: what the loop needs from a
/// capture and nothing more. Production is the cpal-backed [`AudioCapture`];
/// tests drive the loop with scripted PCM instead, so the emission contract
/// (positions/verdicts/phrases actually leave the loop — #354's VA logs
/// showed a follower installed but zero position events, and nothing below
/// the mic could prove which side was broken) is pinned without a device.
trait CaptureSource {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn available(&self) -> usize;
    fn read_samples(&mut self, out: &mut [f32]) -> usize;
}

impl CaptureSource for AudioCapture {
    fn sample_rate(&self) -> u32 {
        AudioCapture::sample_rate(self)
    }
    fn channels(&self) -> u16 {
        AudioCapture::channels(self)
    }
    fn available(&self) -> usize {
        AudioCapture::available(self)
    }
    fn read_samples(&mut self, out: &mut [f32]) -> usize {
        AudioCapture::read_samples(self, out)
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
        F: FnMut(AudioEvent, Option<[f32; 12]>) + Send + 'static,
    {
        Self::start_with_follower(
            profile,
            None,
            None,
            None,
            None,
            emit,
            |_| {},
            |_| {},
            |_| {},
        )
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
    ///
    /// `click_gate` (#445), when supplied, is the shared slot the Pocket
    /// installs its [`ClickGate`] into; the worker strips onset flags that
    /// coincide with the app's own click. `None`/empty slot → nothing gated.
    #[allow(clippy::too_many_arguments)]
    pub fn start_with_follower<F, P, S, V>(
        profile: DetectorProfile,
        follower: Option<ScoreFollower>,
        idiom_buffer: Option<SharedIdiomBuffer>,
        poly: Option<std::sync::Arc<transcribe::PolyRunner>>,
        click_gate: Option<SharedClickGate>,
        emit: F,
        emit_phrase: P,
        emit_position: S,
        emit_verdict: V,
    ) -> Result<Self, PipelineError>
    where
        F: FnMut(AudioEvent, Option<[f32; 12]>) + Send + 'static,
        P: FnMut(PhraseSummary) + Send + 'static,
        S: FnMut(ScorePosition) + Send + 'static,
        V: FnMut(NoteVerdict) + Send + 'static,
    {
        // Fail a doomed startup before spawning a thread or opening the mic
        // (which on macOS can raise the permission prompt): the phrase gate
        // needs nothing from the device to validate (#509). The worker
        // re-checks when it builds the aggregator, so direct `worker_loop`
        // callers get the same typed bail.
        PhraseConfig {
            voiced_confidence_threshold: profile.voiced_confidence_threshold,
            ..PhraseConfig::default()
        }
        .validate()?;

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
                    poly,
                    click_gate,
                    profile_rx,
                    shutdown_worker,
                    startup_tx,
                    emit,
                    emit_phrase,
                    emit_position,
                    emit_verdict,
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
fn run_worker<F, P, S, V>(
    initial_profile: DetectorProfile,
    follower: Option<ScoreFollower>,
    idiom_buffer: Option<SharedIdiomBuffer>,
    poly: Option<std::sync::Arc<transcribe::PolyRunner>>,
    click_gate: Option<SharedClickGate>,
    profile_rx: Receiver<DetectorProfile>,
    shutdown: Arc<AtomicBool>,
    startup_tx: Sender<Result<(), PipelineError>>,
    emit: F,
    emit_phrase: P,
    emit_position: S,
    emit_verdict: V,
) where
    F: FnMut(AudioEvent, Option<[f32; 12]>),
    P: FnMut(PhraseSummary),
    S: FnMut(ScorePosition),
    V: FnMut(NoteVerdict),
{
    // --- Open capture. Bail early if the mic is unavailable. ---
    let capture = match AudioCapture::new(CaptureConfig::default()) {
        Ok(c) => c,
        Err(e) => {
            let _ = startup_tx.send(Err(PipelineError::Capture(e)));
            return;
        }
    };
    worker_loop(
        capture,
        initial_profile,
        follower,
        idiom_buffer,
        poly,
        click_gate,
        profile_rx,
        shutdown,
        startup_tx,
        emit,
        emit_phrase,
        emit_position,
        emit_verdict,
    );
}

/// The worker's detect-and-emit loop, generic over the sample source so
/// tests can feed scripted PCM through the *real* path — detector →
/// aggregator → follower → emit gates. Everything observable about a
/// session (audio-events, phrases, positions, verdicts) leaves through
/// the four callbacks here.
#[allow(clippy::too_many_arguments)]
fn worker_loop<C, F, P, S, V>(
    mut capture: C,
    initial_profile: DetectorProfile,
    follower: Option<ScoreFollower>,
    idiom_buffer: Option<SharedIdiomBuffer>,
    poly: Option<std::sync::Arc<transcribe::PolyRunner>>,
    click_gate: Option<SharedClickGate>,
    profile_rx: Receiver<DetectorProfile>,
    shutdown: Arc<AtomicBool>,
    startup_tx: Sender<Result<(), PipelineError>>,
    mut emit: F,
    mut emit_phrase: P,
    mut emit_position: S,
    mut emit_verdict: V,
) where
    C: CaptureSource,
    F: FnMut(AudioEvent, Option<[f32; 12]>),
    P: FnMut(PhraseSummary),
    S: FnMut(ScorePosition),
    V: FnMut(NoteVerdict),
{
    /// Minimum spacing between live score-position emits. ~10 Hz is smooth
    /// enough for the cursor to glide within a measure without flooding IPC
    /// (the detect loop runs ~40–50 Hz). Driven off event timestamps, not
    /// wall-clock, so it tracks audio time exactly.
    const POSITION_EMIT_INTERVAL_SECS: f64 = 0.1;
    /// Samples between chroma readings (#349 T1) — ≈10.8 Hz at 44.1 kHz,
    /// matching the position cadence. The Goertzel bank costs well under a
    /// millisecond per reading; at this hop it is noise in the budget.
    const CHROMA_HOP_SAMPLES: usize = 4096;

    // Phrase aggregator groups events into musical phrases. With a score
    // follower attached it also tags each phrase with the score position
    // it began on — the anchor the cursor follows. The voiced-confidence gate
    // comes from the active instrument profile so quiet, breathy singing still
    // forms phrases (#185); the rest of the config is the validated default.
    // The gate is profile DATA, not an invariant: a bad value must bail
    // through the startup channel like the capture/detector failures below —
    // a panic here kills this thread and takes pitch detection with it (#509).
    let mut aggregator = match PhraseAggregator::new(PhraseConfig {
        voiced_confidence_threshold: initial_profile.voiced_confidence_threshold,
        ..PhraseConfig::default()
    }) {
        Ok(a) => a,
        Err(e) => {
            let _ = startup_tx.send(Err(PipelineError::Phrase(e)));
            return;
        }
    };
    let has_follower = follower.is_some();
    if let Some(f) = follower {
        aggregator.set_score_follower(f);
    }
    // Timestamp of the last position emit, for 10 Hz downsampling. `None`
    // until the first emit so the cursor moves immediately on first audio.
    let mut last_position_emit_secs: Option<f64> = None;
    let sample_rate = capture.sample_rate();
    let channels = capture.channels();

    // #445: the worker's wall-clock epoch — the instant event time 0 was
    // (approximately) captured. Click fires are converted into the same
    // frame (`instant_offset_secs(pocket_epoch, session_epoch) + index/rate`)
    // so onsets and clicks compare on one axis; the few-ms recording slop
    // is far inside the gate's 105 ms window.
    let session_epoch = Instant::now();
    // Recent click wall-times, a fixed ring — NO allocation in the loop.
    // NEG_INFINITY sentinels can never match a finite onset time.
    let mut recent_clicks = [f64::NEG_INFINITY; RECENT_CLICKS];
    let mut recent_click_idx: usize = 0;

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

    // Chroma front end for the live chord label (#349 T1). Fed every
    // window; computed every CHROMA_HOP_SAMPLES (~10 Hz) at the END of an
    // iteration and delivered with the NEXT event's emit, so its cost never
    // delays a live pitch emit. All buffers are pre-allocated inside —
    // nothing here allocates per frame.
    let mut chroma_extractor = ears::chroma::ChromaExtractor::new(sample_rate);
    let mut samples_since_chroma: usize = 0;
    let mut pending_chroma: Option<[f32; 12]> = None;
    // The session clock as of the last emitted event — carried into a
    // reconfigured detector so the clock never restarts mid-session.
    let mut session_clock: f64 = 0.0;

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
            let voiced_gate = new_profile.voiced_confidence_threshold;
            match PitchDetector::new(new_profile.into_pitch_config(sample_rate)) {
                Ok(mut d) => {
                    // #349 T3b review M1: ONE session clock. A fresh detector
                    // restarts at 0.0, which desyncs every consumer keyed on
                    // event time (poly bass lookups die, stale freshness
                    // checks pin wrong slashes, chart timestamps restart).
                    d.set_clock(session_clock);
                    detector = d;
                    // #521: the voiced gate is per-instrument profile data
                    // (#185) and must move with the detector — otherwise a
                    // Trumpet→Voice switch keeps judging breathy singing by
                    // the old 0.5 gate and the segment records no practice.
                    // A bad gate keeps the previous one, same contract as a
                    // rejected detector rebuild below.
                    if let Err(e) = aggregator.set_voiced_confidence_threshold(voiced_gate) {
                        tracing::warn!(
                            error = %e,
                            "audio_pipeline: voiced-gate update rejected; keeping previous gate"
                        );
                    }
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

        let mut event = detector.detect(mono_slice);
        // #445: time-gated click rejection. Drain this window's click
        // fires (bounded; drained every window so the SPSC ring never
        // backs up into staleness) into the recent ring, then strip the
        // onset flag if it merely agrees with our own click. The lock is
        // on the PROCESSING thread — fine here, never in a render
        // callback; a poisoned lock fails OPEN (onsets pass untouched).
        // This runs BEFORE emit/aggregator/perception so every consumer
        // sees one consistent story. Pitch is deliberately untouched.
        if let Some(gate_slot) = &click_gate {
            if let Ok(mut slot) = gate_slot.lock() {
                if let Some(gate) = slot.as_mut() {
                    for _ in 0..CLICK_GATE_MAX_DRAIN {
                        let Some(fire) = gate.fires.try_pop() else {
                            break;
                        };
                        recent_clicks[recent_click_idx] =
                            instant_offset_secs(gate.epoch, session_epoch)
                                + fire.sample_index as f64 / f64::from(gate.output_sample_rate);
                        recent_click_idx = (recent_click_idx + 1) % RECENT_CLICKS;
                    }
                }
            }
            if event.is_onset && onset_gated(event.timestamp_secs, &recent_clicks) {
                event.is_onset = false;
            }
        }
        // Live pitch goes out first so the meter stays responsive, then
        // the aggregator folds the event into the current phrase. The chroma
        // reading attached here was computed at the END of the *previous*
        // iteration (one window ≈ 23 ms of extra chord latency, invisible at
        // the ~10 Hz cadence) — deliberately, so the Goertzel bank can never
        // sit between detection and the live emit (#349 AC6).
        emit(event.clone(), pending_chroma.take());
        // Now (with the live emit already out the door) fold this window
        // into the chroma ring and, at the ~10 Hz hop, compute the reading
        // the next emit will carry.
        // #349 T3b: the polyphony runner gets the same window — feed()
        // NEVER blocks (bounded channel, drops on backpressure), so the
        // ~38 ms inference lives entirely on its own thread.
        if let Some(p) = &poly {
            p.feed(mono_slice, sample_rate);
        }
        chroma_extractor.feed(mono_slice);
        samples_since_chroma += mono_slice.len();
        if samples_since_chroma >= CHROMA_HOP_SAMPLES {
            samples_since_chroma = 0;
            pending_chroma = chroma_extractor.chroma();
        }
        let event_time = event.timestamp_secs;
        // The detector's INTERNAL clock has already advanced past this
        // window; seed a reconfigure with end-of-window so a switch never
        // back-steps the session clock (review r2 nit).
        session_clock = event_time + window as f64 / f64::from(sample_rate);
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

        // Note verdicts (#337 S2): drained per event but produced only when
        // the alignment ADVANCES to a new score note — boundary cadence,
        // never per-frame. Nothing in free play.
        if has_follower {
            for verdict in aggregator.take_note_verdicts() {
                emit_verdict(verdict);
            }
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

    // -----------------------------------------------------------------
    // Emission-contract tests (#354): drive the REAL worker loop —
    // detector → aggregator → follower → emit gates — with scripted PCM.
    // `AudioPipeline::start` still can't run in CI (no mic device on the
    // runners); the `CaptureSource` seam exists so everything below the
    // mic is pinned here anyway.
    // -----------------------------------------------------------------

    use brain::follower::ScoreFollower;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const TEST_SAMPLE_RATE: u32 = 44_100;

    /// A finite, mono, pre-recorded "mic". When drained it flips the
    /// worker's own shutdown flag, so the loop exits exactly like a real
    /// session ending — final flush included.
    struct ScriptedCapture {
        samples: Vec<f32>,
        cursor: usize,
        shutdown: Arc<AtomicBool>,
        /// Consecutive `available()` calls with samples left but nothing
        /// consumed — see the starvation guard below.
        stalls: Cell<u32>,
        last_cursor: Cell<usize>,
    }

    impl CaptureSource for ScriptedCapture {
        fn sample_rate(&self) -> u32 {
            TEST_SAMPLE_RATE
        }
        fn channels(&self) -> u16 {
            1
        }
        fn available(&self) -> usize {
            let left = self.samples.len() - self.cursor;
            if left == 0 {
                self.shutdown.store(true, Ordering::Relaxed);
            } else if self.last_cursor.get() == self.cursor {
                // Starvation guard: if the loop keeps asking without ever
                // consuming (a trailing partial window, or a future change
                // to the loop's read unit), fail loudly instead of hanging
                // the test on eternal 5 ms sleeps.
                let stalls = self.stalls.get() + 1;
                assert!(
                    stalls < 1_000,
                    "worker loop starved: {left} samples left but none consumed"
                );
                self.stalls.set(stalls);
            } else {
                self.last_cursor.set(self.cursor);
                self.stalls.set(0);
            }
            left
        }
        fn read_samples(&mut self, out: &mut [f32]) -> usize {
            let n = out.len().min(self.samples.len() - self.cursor);
            out[..n].copy_from_slice(&self.samples[self.cursor..self.cursor + n]);
            self.cursor += n;
            n
        }
    }

    fn test_profile() -> DetectorProfile {
        DetectorProfile {
            threshold: 0.15,
            freq_min_hz: 60.0,
            freq_max_hz: 2000.0,
            voiced_confidence_threshold: 0.5,
        }
    }

    /// The detector window the loop will run at for [`test_profile`], in
    /// samples. Scripted audio is built in whole windows so the capture
    /// drains to exactly zero (a trailing partial window would starve the
    /// loop forever instead of ending it).
    fn detector_window() -> usize {
        PitchDetector::new(test_profile().into_pitch_config(TEST_SAMPLE_RATE))
            .expect("test profile is valid")
            .window_size()
    }

    fn scale_follower() -> ScoreFollower {
        let xml = include_str!("../../../../crates/brain/tests/fixtures/simple_scale.musicxml");
        ScoreFollower::from_musicxml_str(xml, 0).expect("fixture parses")
    }

    /// Whole-window sine renderings of the fixture scale (C4..C5, the same
    /// notes `simple_scale.musicxml` engraves), ~0.35 s per note.
    fn played_scale(window: usize) -> Vec<f32> {
        const SCALE_HZ: [f64; 8] = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];
        let windows_per_note =
            ((0.35 * f64::from(TEST_SAMPLE_RATE)) / window as f64).ceil() as usize;
        let mut out = Vec::new();
        for hz in SCALE_HZ {
            out.extend(ears::pitch::generate_sine(
                hz,
                TEST_SAMPLE_RATE,
                windows_per_note * window,
            ));
        }
        out
    }

    /// Everything the loop emitted, in arrival order.
    #[derive(Default)]
    struct EmitLog {
        events: Cell<usize>,
        /// Timestamps of emitted events that carried `is_onset` (#445).
        onsets: RefCell<Vec<f64>>,
        /// Onset timestamps the AGGREGATOR retained — flattened
        /// `PhraseSummary::onsets_secs` across all phrases, i.e. the groove
        /// path's view of onsets. #445 one-story pin: the gate must run
        /// BEFORE the aggregator too, not just before the emit copy.
        phrase_onsets: RefCell<Vec<f64>>,
        /// (events seen when this position left the loop, the position).
        positions: RefCell<Vec<(usize, ScorePosition)>>,
        verdicts: Cell<usize>,
        phrases: Cell<usize>,
    }

    /// Run the real worker loop over `samples` and record what it emits.
    fn drive_loop(
        samples: Vec<f32>,
        follower: Option<ScoreFollower>,
        click_gate: Option<SharedClickGate>,
    ) -> Rc<EmitLog> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let capture = ScriptedCapture {
            samples,
            cursor: 0,
            shutdown: Arc::clone(&shutdown),
            stalls: Cell::new(0),
            last_cursor: Cell::new(0),
        };
        let (_profile_tx, profile_rx) = std::sync::mpsc::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::channel();
        let log = Rc::new(EmitLog::default());
        let (ev, pos, ver, phr) = (
            Rc::clone(&log),
            Rc::clone(&log),
            Rc::clone(&log),
            Rc::clone(&log),
        );
        worker_loop(
            capture,
            test_profile(),
            follower,
            None,
            None,
            click_gate,
            profile_rx,
            shutdown,
            startup_tx,
            move |event, _chroma| {
                ev.events.set(ev.events.get() + 1);
                if event.is_onset {
                    ev.onsets.borrow_mut().push(event.timestamp_secs);
                }
            },
            move |phrase| {
                phr.phrases.set(phr.phrases.get() + 1);
                phr.phrase_onsets
                    .borrow_mut()
                    .extend_from_slice(&phrase.onsets_secs);
            },
            move |p| pos.positions.borrow_mut().push((pos.events.get(), p)),
            move |_verdict| ver.verdicts.set(ver.verdicts.get() + 1),
        );
        assert!(
            matches!(startup_rx.try_recv(), Ok(Ok(()))),
            "worker loop must signal successful startup"
        );
        log
    }

    /// #509: a bad profile-derived voiced-confidence gate must fail startup
    /// through the typed channel, not panic the audio-pipeline thread. The
    /// old `.expect` unwound the worker before it signaled ready, so the
    /// caller saw only `StartupChannelClosed` with the root cause lost to
    /// the panic — a config/data bug turned into undiagnosable feature loss.
    #[test]
    fn invalid_voiced_gate_fails_startup_typed_instead_of_panicking() {
        for bad in [0.0, -0.5, 1.5, f64::NAN] {
            let shutdown = Arc::new(AtomicBool::new(false));
            let capture = ScriptedCapture {
                samples: vec![0.0; 4096],
                cursor: 0,
                shutdown: Arc::clone(&shutdown),
                stalls: Cell::new(0),
                last_cursor: Cell::new(0),
            };
            let (_profile_tx, profile_rx) = std::sync::mpsc::channel();
            let (startup_tx, startup_rx) = std::sync::mpsc::channel();
            let events = Rc::new(Cell::new(0_usize));
            let ev = Rc::clone(&events);
            worker_loop(
                capture,
                DetectorProfile {
                    voiced_confidence_threshold: bad,
                    ..test_profile()
                },
                None,
                None,
                None,
                None,
                profile_rx,
                shutdown,
                startup_tx,
                move |_event, _chroma| ev.set(ev.get() + 1),
                |_phrase| {},
                |_position| {},
                |_verdict| {},
            );
            match startup_rx.try_recv() {
                Ok(Err(PipelineError::Phrase(PhraseError::InvalidConfidenceThreshold(got)))) => {
                    assert!(
                        got == bad || (got.is_nan() && bad.is_nan()),
                        "error must carry the offending value: got {got}, fed {bad}"
                    );
                }
                other => {
                    panic!("expected typed phrase-config startup error for {bad}, got {other:?}")
                }
            }
            assert_eq!(events.get(), 0, "a doomed worker must not process audio");
        }
    }

    /// #509 review: the public start path rejects a bad profile before
    /// spawning a worker or opening the mic. CI-safe precisely because the
    /// bail happens ahead of `AudioCapture::new` (runners have no input
    /// device) — a `Capture` error here would mean validation moved back
    /// behind the device open.
    #[test]
    fn start_rejects_invalid_profile_before_touching_the_mic() {
        match AudioPipeline::start(
            DetectorProfile {
                voiced_confidence_threshold: f64::NAN,
                ..test_profile()
            },
            |_, _| {},
        ) {
            Err(PipelineError::Phrase(PhraseError::InvalidConfidenceThreshold(v))) => {
                assert!(v.is_nan(), "error must carry the offending value, got {v}");
            }
            Err(other) => panic!("expected the typed phrase-config error, got {other:?}"),
            Ok(_) => panic!("invalid gate must not start a pipeline"),
        }
    }

    /// The initial detector's per-window (voiced?, confidence) readings on
    /// `samples` — the same detector the loop builds for [`test_profile`], so
    /// gates derived from these can't silently drift from what the loop sees.
    fn probe_confidences(samples: &[f32]) -> Vec<(bool, f64)> {
        let mut d = PitchDetector::new(test_profile().into_pitch_config(TEST_SAMPLE_RATE))
            .expect("test profile is valid");
        let w = d.window_size();
        samples
            .chunks_exact(w)
            .map(|c| {
                let e = d.detect(c);
                (e.pitch_hz.is_some(), e.confidence)
            })
            .collect()
    }

    /// Run the real worker loop over `samples`, sending `swap` through the
    /// production profile channel once `swap_after` events have emitted
    /// (never, if `swap_after` exceeds the event count). Returns the emitted
    /// event count and each closed phrase's note count.
    fn drive_with_reconfigure(
        samples: Vec<f32>,
        initial: DetectorProfile,
        swap: DetectorProfile,
        swap_after: usize,
    ) -> (usize, Vec<usize>) {
        let shutdown = Arc::new(AtomicBool::new(false));
        let capture = ScriptedCapture {
            samples,
            cursor: 0,
            shutdown: Arc::clone(&shutdown),
            stalls: Cell::new(0),
            last_cursor: Cell::new(0),
        };
        let (profile_tx, profile_rx) = std::sync::mpsc::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::channel();
        let events = Rc::new(Cell::new(0_usize));
        let notes = Rc::new(RefCell::new(Vec::new()));
        let ev = Rc::clone(&events);
        let ph = Rc::clone(&notes);
        let mut pending_swap = Some(swap);
        worker_loop(
            capture,
            initial,
            None,
            None,
            None,
            None,
            profile_rx,
            shutdown,
            startup_tx,
            move |_event, _chroma| {
                ev.set(ev.get() + 1);
                if ev.get() == swap_after {
                    if let Some(p) = pending_swap.take() {
                        profile_tx.send(p).expect("worker holds the receiver");
                    }
                }
            },
            move |phrase| ph.borrow_mut().push(phrase.note_count),
            |_position| {},
            |_verdict| {},
        );
        assert!(
            matches!(startup_rx.try_recv(), Ok(Ok(()))),
            "worker loop must signal successful startup"
        );
        let counts = notes.borrow().clone();
        (events.get(), counts)
    }

    /// #521 AC1: a mid-session reconfigure must move the #185 voiced gate to
    /// the aggregator along with the detector. Before the fix the gate rode
    /// the channel and was dropped by `into_pitch_config` — a Trumpet→Voice
    /// switch kept judging singing by the old instrument's gate, so the Voice
    /// segment formed no phrases ("you didn't play", #185's exact failure).
    #[test]
    fn reconfigure_moves_the_voiced_gate_with_the_detector() {
        let window = detector_window();
        let n_windows = 40;
        let samples = ears::pitch::generate_sine(440.0, TEST_SAMPLE_RATE, n_windows * window);

        // Derive the two gates from measured confidence so the contrast can't
        // rot if detector tuning shifts: `strict` sits above every reading
        // (nothing voiced), `loose` below every voiced reading.
        let probe = probe_confidences(&samples);
        let voiced: Vec<f64> = probe
            .iter()
            .filter(|(v, c)| *v && *c > 0.0)
            .map(|(_, c)| *c)
            .collect();
        assert!(
            voiced.len() > n_windows / 2 + 4,
            "sine fixture must detect as voiced in most windows, got {}",
            voiced.len()
        );
        let cmax = probe.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
        let strict = (cmax + 1.0) / 2.0;
        let loose = voiced.iter().copied().fold(1.0_f64, f64::min) / 2.0;
        assert!(loose > 0.0 && strict <= 1.0 && loose < strict);

        // Control: without the reconfigure the strict gate keeps the whole
        // take unvoiced — proving the contrast below is the gate's doing.
        let (_, control_notes) = drive_with_reconfigure(
            samples.clone(),
            DetectorProfile {
                voiced_confidence_threshold: strict,
                ..test_profile()
            },
            DetectorProfile {
                voiced_confidence_threshold: loose,
                ..test_profile()
            },
            n_windows + 1,
        );
        assert!(
            control_notes.is_empty(),
            "control run: the strict gate must suppress every phrase"
        );

        // The switch: same audio, reconfigure to the loose gate mid-take.
        let (events, notes) = drive_with_reconfigure(
            samples,
            DetectorProfile {
                voiced_confidence_threshold: strict,
                ..test_profile()
            },
            DetectorProfile {
                voiced_confidence_threshold: loose,
                ..test_profile()
            },
            n_windows / 2,
        );
        assert_eq!(events, n_windows, "every window becomes an event");
        assert!(
            !notes.is_empty(),
            "after the switch, singing that clears the NEW gate must form \
             phrases — the voiced gate must move with the detector (#521)"
        );
        let total: usize = notes.iter().sum();
        assert!(
            total <= n_windows - n_windows / 2,
            "pre-switch events keep the old gate's verdict: {total} notes \
             counted from {} post-switch windows",
            n_windows - n_windows / 2
        );
    }

    /// #521 AC3: an invalid gate arriving via reconfigure (bad profile data)
    /// keeps the previous gate and the worker alive — the same warn-and-keep
    /// contract as a rejected detector rebuild, never a dead pipeline (#509).
    #[test]
    fn invalid_reconfigured_gate_keeps_the_previous_gate() {
        let window = detector_window();
        let n_windows = 40;
        let swap_after = n_windows / 2;
        let samples = ears::pitch::generate_sine(440.0, TEST_SAMPLE_RATE, n_windows * window);
        let probe = probe_confidences(&samples);
        let loose = probe
            .iter()
            .filter(|(v, c)| *v && *c > 0.0)
            .map(|(_, c)| *c)
            .fold(1.0_f64, f64::min)
            / 2.0;
        assert!(loose > 0.0);

        let (events, notes) = drive_with_reconfigure(
            samples,
            DetectorProfile {
                voiced_confidence_threshold: loose,
                ..test_profile()
            },
            DetectorProfile {
                voiced_confidence_threshold: f64::NAN,
                ..test_profile()
            },
            swap_after,
        );
        assert_eq!(events, n_windows, "a bad gate must not kill the worker");
        let total: usize = notes.iter().sum();
        assert!(
            total > swap_after + 2,
            "the previous gate must keep judging post-switch audio as voiced \
             (a NaN gate would leave only the ~{swap_after} pre-switch \
             events; got {total})"
        );
    }

    /// #521 AC4: a rejected detector rebuild keeps the WHOLE previous profile
    /// — gate included. Applying the gate from a profile whose pitch half was
    /// refused would leave a mixed half-applied profile from one bad message.
    #[test]
    fn rejected_detector_rebuild_keeps_the_previous_gate() {
        let window = detector_window();
        let n_windows = 40;
        let samples = ears::pitch::generate_sine(440.0, TEST_SAMPLE_RATE, n_windows * window);
        let probe = probe_confidences(&samples);
        let cmax = probe.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);
        let strict = (cmax + 1.0) / 2.0;
        let loose = probe
            .iter()
            .filter(|(v, c)| *v && *c > 0.0)
            .map(|(_, c)| *c)
            .fold(1.0_f64, f64::min)
            / 2.0;

        let (events, notes) = drive_with_reconfigure(
            samples,
            DetectorProfile {
                voiced_confidence_threshold: strict,
                ..test_profile()
            },
            // Inverted pitch range → PitchDetector::new refuses → the loose
            // gate riding the same message must be refused with it.
            DetectorProfile {
                freq_min_hz: 2000.0,
                freq_max_hz: 60.0,
                voiced_confidence_threshold: loose,
                ..test_profile()
            },
            n_windows / 2,
        );
        assert_eq!(
            events, n_windows,
            "a rejected rebuild must not kill the worker"
        );
        assert!(
            notes.is_empty(),
            "the strict gate must survive a rejected reconfigure whole — got \
             phrases with note counts {notes:?}"
        );
    }

    /// #354's exact symptom: 'score follower installed' with ZERO position
    /// emissions. The contract this pins: a score session ticks positions
    /// from the very first window and keeps ticking for the whole session,
    /// even if the player never makes a sound — a stuck-at-Measure-1
    /// follower still reports Measure 1. Total emission absence is a bug
    /// below the IPC layer, and this test is where it goes red.
    #[test]
    fn score_session_emits_positions_from_first_window_even_in_silence() {
        let window = detector_window();
        let n_windows = 40;
        let samples = vec![0.0_f32; window * n_windows];
        let duration_secs = (window * n_windows) as f64 / f64::from(TEST_SAMPLE_RATE);

        let log = drive_loop(samples, Some(scale_follower()), None);

        let positions = log.positions.borrow();
        assert_eq!(log.events.get(), n_windows, "every window becomes an event");
        assert!(
            !positions.is_empty(),
            "a session with a follower must emit score positions (#354: \
             installed follower, zero emissions)"
        );
        assert_eq!(
            positions[0].0, 1,
            "the cursor must move on the FIRST audio window, not wait for \
             alignment or voicing"
        );
        // Silence never advances alignment: honest Measure 1 throughout.
        assert!(
            positions.iter().all(|(_, p)| p.measure_number == 1),
            "silence must not advance the reported measure"
        );
        // The ~10 Hz cadence, asserted as time-based contract bounds (not a
        // re-derivation of the gate): at most one emit per 0.1 s of audio
        // (+1 for the immediate first tick), and no long dead stretches —
        // each tick lands at most one window late.
        let window_secs = window as f64 / f64::from(TEST_SAMPLE_RATE);
        let max_emits = (duration_secs / 0.1).floor() as usize + 1;
        let min_emits = (duration_secs / (0.1 + window_secs)).floor() as usize;
        assert!(
            positions.len() <= max_emits,
            "position emits must be throttled to ~10 Hz: {} > {}",
            positions.len(),
            max_emits
        );
        assert!(
            positions.len() >= min_emits.max(2),
            "positions must keep ticking across the session: {} < {}",
            positions.len(),
            min_emits.max(2)
        );
    }

    /// The #347 log shape — judged verdicts in the recap but a dead cursor —
    /// must be impossible from one loop run: when a played scale produces
    /// verdicts, positions flowed too, and they advanced with the playing.
    #[test]
    fn played_scale_advances_positions_and_verdicts_together() {
        let window = detector_window();
        let log = drive_loop(played_scale(window), Some(scale_follower()), None);

        let positions = log.positions.borrow();
        assert!(
            log.verdicts.get() > 0,
            "playing the score's own notes must produce note verdicts"
        );
        assert!(
            !positions.is_empty(),
            "verdicts without positions is the impossible #347 shape"
        );
        assert!(
            positions.iter().any(|(_, p)| p.measure_number >= 2),
            "playing through the two-measure scale must advance the live \
             cursor past measure 1; got measures {:?}",
            positions
                .iter()
                .map(|(_, p)| p.measure_number)
                .collect::<Vec<_>>()
        );
    }

    /// Free play (no follower) must stay silent on the score channels —
    /// no positions, no verdicts — while audio events still flow.
    #[test]
    fn free_play_emits_no_positions_or_verdicts() {
        let window = detector_window();
        let log = drive_loop(played_scale(window), None, None);

        assert!(log.events.get() > 0, "audio events must still flow");
        assert!(
            log.positions.borrow().is_empty(),
            "free play must never emit a score position"
        );
        assert_eq!(
            log.verdicts.get(),
            0,
            "free play must never emit a note verdict"
        );
        // The stream is voiced to its final sample with no silence gap and
        // (without a follower) no measure boundaries, so it forms exactly
        // one phrase — deliverable ONLY by the end-of-session flush. Zero
        // here means the last bar of a real session gets dropped.
        assert_eq!(
            log.phrases.get(),
            1,
            "the end-of-session flush must deliver the trailing phrase"
        );
    }

    // -----------------------------------------------------------------
    // #445: time-gated click rejection
    // -----------------------------------------------------------------

    /// #445 AC3: the pure gate window, boundaries pinned INCLUSIVE on
    /// both edges. A window that drifted (wrong sign, ms/seconds mixup,
    /// exclusive bounds) fails here.
    #[test]
    fn onset_gated_window_boundaries_are_pinned() {
        let clicks = [10.0];
        // Inside: the click instant itself, mid-window either side.
        assert!(onset_gated(10.0, &clicks));
        assert!(onset_gated(9.99, &clicks));
        assert!(onset_gated(10.05, &clicks));
        // Exact boundaries are gated (inclusive). A click at 0.0 keeps the
        // event−click delta exactly representable, so this pins the
        // comparison operators rather than f64 rounding noise.
        assert!(onset_gated(-CLICK_GATE_PRE_MS / 1000.0, &[0.0]));
        assert!(onset_gated(CLICK_GATE_POST_MS / 1000.0, &[0.0]));
        // Just outside either edge passes.
        assert!(!onset_gated(-CLICK_GATE_PRE_MS / 1000.0 - 0.001, &[0.0]));
        assert!(!onset_gated(CLICK_GATE_POST_MS / 1000.0 + 0.001, &[0.0]));
        // Far away passes; empty click list passes; the NEG_INFINITY
        // ring sentinels can never match.
        assert!(!onset_gated(9.5, &clicks));
        assert!(!onset_gated(10.2, &clicks));
        assert!(!onset_gated(10.0, &[]));
        assert!(!onset_gated(10.0, &[f64::NEG_INFINITY]));
        // Any recent click may match, not just the newest.
        assert!(onset_gated(10.0, &[3.0, 10.01, f64::NEG_INFINITY]));
    }

    #[test]
    fn instant_offset_is_signed_both_ways() {
        let a = Instant::now();
        let b = a + Duration::from_millis(250);
        assert!((instant_offset_secs(b, a) - 0.25).abs() < 1e-9);
        assert!((instant_offset_secs(a, b) + 0.25).abs() < 1e-9);
        assert_eq!(instant_offset_secs(a, a), 0.0);
    }

    /// #445 AC4: the REAL worker loop, scripted PCM. Run 1 (no gate)
    /// finds where the detector hears onsets; run 2 installs a gate whose
    /// clicks land exactly there → every one is stripped — from the emit
    /// stream AND from the aggregator's groove view (one story past the
    /// emit boundary). Window-magnitude runs pin the post window through
    /// the same path: a click 60 ms before the onset gates, 150 ms before
    /// passes. Run 3 shifts the clicks 0.5 s away → every onset survives
    /// on both sides (the gate passes DISAGREEMENT — the thing Follow
    /// mode must hear). Event counts stay identical throughout: the gate
    /// drops flags, never events.
    #[test]
    fn click_coincident_onsets_are_gated_and_off_beat_onsets_pass() {
        use ears::output_engine::click_fire_channel;
        use ringbuf::traits::Producer;

        let window = detector_window();
        // Silence, then a loud sine: a clean silence→sound attack the
        // SuperFlux onset detector flags.
        let mut samples = vec![0.0_f32; window * 10];
        samples.extend(ears::pitch::generate_sine(
            440.0,
            TEST_SAMPLE_RATE,
            window * 10,
        ));

        // Run 1: no gate — collect the detector's onset timestamps. The
        // aggregator's groove view must see them too (this also arms the
        // run-2 phrase_onsets assertion against vacuous passes).
        let baseline = drive_loop(samples.clone(), None, None);
        let onsets = baseline.onsets.borrow().clone();
        assert!(
            !onsets.is_empty(),
            "the silence→sine attack must register at least one onset"
        );
        assert!(
            !baseline.phrase_onsets.borrow().is_empty(),
            "ungated onsets must reach the aggregator's groove path"
        );

        // The metronome's output clock: any rate works — the gate only
        // uses sample_index / rate. The gate's epoch and the loop's
        // session_epoch are recorded µs apart, far inside the window.
        const OUTPUT_RATE: u32 = 48_000;
        let gate_at = |offset_secs: f64| {
            let (mut tx, rx) = click_fire_channel(64);
            for &t in &onsets {
                tx.try_push(ClickFire {
                    sample_index: ((t + offset_secs) * f64::from(OUTPUT_RATE)) as u64,
                    is_accent: false,
                })
                .expect("fire fits the ring");
            }
            Arc::new(Mutex::new(Some(ClickGate {
                fires: rx,
                epoch: Instant::now(),
                output_sample_rate: OUTPUT_RATE,
            })))
        };

        // Run 2: clicks exactly on the onsets → all gated, no event lost.
        let gated = drive_loop(samples.clone(), None, Some(gate_at(0.0)));
        assert_eq!(
            gated.events.get(),
            baseline.events.get(),
            "the gate must never drop events, only onset flags"
        );
        assert!(
            gated.onsets.borrow().is_empty(),
            "onsets agreeing with our own click must be stripped, got {:?}",
            gated.onsets.borrow()
        );
        // ONE story past the emit boundary: the aggregator (groove path,
        // PhraseSummary::onsets_secs) must see the SAME gated events — a
        // gate that stripped only the emitted copy passes the assertion
        // above but dies here.
        assert!(
            gated.phrase_onsets.borrow().is_empty(),
            "the aggregator must see the gated events too (one story), got {:?}",
            gated.phrase_onsets.borrow()
        );

        // Window magnitude, through the same worker-loop path: a click
        // 60 ms BEFORE the onset (onset at click + 60 ms, inside the
        // +90 ms post window: decay + latency slop) must gate…
        let post_window = drive_loop(samples.clone(), None, Some(gate_at(-0.06)));
        assert!(
            post_window.onsets.borrow().is_empty(),
            "an onset 60 ms after the click sits in the post window and must gate"
        );
        // …while a click 150 ms before (onset at click + 150 ms, past the
        // +90 ms edge) must pass.
        let past_window = drive_loop(samples.clone(), None, Some(gate_at(-0.15)));
        assert_eq!(
            *past_window.onsets.borrow(),
            onsets,
            "an onset 150 ms after the click is outside the window and must pass"
        );

        // Run 3: clicks 0.5 s off the onsets → disagreement passes, on
        // BOTH sides of the emit boundary.
        let passed = drive_loop(samples, None, Some(gate_at(0.5)));
        assert_eq!(
            *passed.onsets.borrow(),
            onsets,
            "onsets that DISAGREE with the click must pass untouched"
        );
        assert_eq!(
            *passed.phrase_onsets.borrow(),
            *baseline.phrase_onsets.borrow(),
            "passing onsets must reach the aggregator unchanged"
        );
    }
}
