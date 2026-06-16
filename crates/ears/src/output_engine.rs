//! Audio output engine: a cpal **output** stream fed by a lock-free SPSC ring
//! buffer.
//!
//! This is the playback counterpart to [`crate::capture`]. It mirrors the input
//! side exactly:
//! - A dedicated **device thread** owns the `cpal::Stream` (which is `!Send` on
//!   macOS, so it must never cross a thread boundary). Its callback only *drains*
//!   the ring buffer into the device buffer — **no heap allocation, no locks**.
//! - A **render thread** owns a [`RenderSource`] (e.g. an [`OutputMixer`]),
//!   renders blocks of mono samples into a pre-allocated scratch buffer, and
//!   pushes them into the ring. All allocation happens here at construction, not
//!   on the audio callback.
//!
//! The samples in the ring are **mono**; the callback fans each mono sample out
//! across the device's channels. On underrun (render thread fell behind) the
//! callback emits silence rather than blocking or repeating.
//!
//! [`OutputMixer`]: crate::output::OutputMixer

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use thiserror::Error;

use crate::output::OutputMixer;

/// Errors from the audio output engine.
#[derive(Debug, Error)]
pub enum OutputEngineError {
    /// No default output device is available.
    #[error("no output device available")]
    NoOutputDevice,
    /// The device reported no usable output configuration.
    #[error("no supported output config: {0}")]
    NoSupportedConfig(String),
    /// cpal could not build the output stream.
    #[error("failed to build output stream: {0}")]
    BuildStream(String),
    /// cpal could not start the output stream.
    #[error("failed to start stream: {0}")]
    PlayStream(String),
    /// The device thread exited before reporting startup status.
    #[error("output device thread closed before startup")]
    StartupChannelClosed,
}

/// Ring buffer capacity in **mono** samples. ~186 ms at 44.1 kHz — enough that a
/// briefly-scheduled render thread won't underrun, small enough that output
/// latency stays modest. The callback never blocks regardless of this value.
const RING_CAPACITY: usize = 8192;

/// Mono samples rendered per render-thread iteration. A few ms of audio; keeps
/// the render thread responsive to the stop flag.
const RENDER_BLOCK: usize = 512;

/// How long the render thread sleeps when the ring has no room for a full block.
const RENDER_BACKOFF: Duration = Duration::from_millis(3);

/// A source of mono audio samples for the output engine.
///
/// Implementors fill `out` with one mono sample per slot. **Hot-path-ish:** this
/// runs on the render thread (not the realtime callback), but it must still avoid
/// allocation so it can later move onto a tighter budget — pre-allocate at
/// construction.
///
/// **Range contract:** samples *should* lie in `[-1.0, 1.0]`. The engine clamps
/// defensively before they reach the device (see [`run_render`]), so a source
/// that transiently overshoots will not blast the speaker — but a well-behaved
/// source stays in range so the clamp never has to engage.
pub trait RenderSource: Send {
    /// Fill `out` with mono samples. Must write every slot.
    fn render(&mut self, out: &mut [f32]);
}

/// An [`OutputMixer`] is a render source: each slot is one mixed sample.
impl RenderSource for OutputMixer {
    fn render(&mut self, out: &mut [f32]) {
        for slot in out.iter_mut() {
            *slot = self.next_sample();
        }
    }
}

/// Producer side of the output ring buffer (render-thread side).
pub struct OutputProducer(HeapProd<f32>);

impl OutputProducer {
    /// Push mono samples into the ring. Returns how many were written. Never
    /// blocks — if the ring is full, the excess is not written (caller retries).
    pub fn push_samples(&mut self, samples: &[f32]) -> usize {
        self.0.push_slice(samples)
    }

    /// Number of free mono-sample slots currently in the ring.
    pub fn vacant(&self) -> usize {
        self.0.vacant_len()
    }
}

/// Consumer side of the output ring buffer (audio-callback side).
pub struct OutputConsumer(HeapCons<f32>);

impl OutputConsumer {
    /// Shared drain core for every device format. Drains one mono sample per
    /// output frame, converts it via `convert`, and fans the converted value
    /// across the frame's `channels`. On underrun the remaining frames — and any
    /// `out.len() % channels` remainder slots — are filled with `silence`.
    /// **No allocation.**
    ///
    /// Returns the number of frames filled from real audio, in
    /// `0..=out.len() / channels`.
    fn fill_with<T: Copy>(
        &mut self,
        out: &mut [T],
        channels: usize,
        silence: T,
        convert: impl Fn(f32) -> T,
    ) -> usize {
        if channels == 0 {
            for slot in out.iter_mut() {
                *slot = silence;
            }
            return 0;
        }
        let frames = out.len() / channels;
        let mut frames_filled = 0usize;
        let mut underrun = false;
        for i in 0..frames {
            // Once the ring runs dry, every remaining frame is silence.
            let value = if underrun {
                silence
            } else {
                match self.0.try_pop() {
                    Some(s) => {
                        frames_filled += 1;
                        convert(s)
                    }
                    None => {
                        underrun = true;
                        silence
                    }
                }
            };
            let base = i * channels;
            for slot in &mut out[base..base + channels] {
                *slot = value;
            }
        }
        // Silence any trailing remainder when out.len() isn't a channel multiple.
        for slot in &mut out[frames * channels..] {
            *slot = silence;
        }
        frames_filled
    }

    /// Drain into an `f32` interleaved buffer. Samples are already in device
    /// range (the render thread clamps before they enter the ring), so this is a
    /// transparent fan-out. Used by the F32 device callback. **No allocation.**
    pub fn fill(&mut self, out: &mut [f32], channels: usize) -> usize {
        self.fill_with(out, channels, 0.0, |s| s)
    }

    /// Drain into an `i16` interleaved buffer, converting `[-1.0, 1.0]` to the
    /// full `i16` range. Used by the I16 device callback. **No allocation.**
    pub fn fill_i16(&mut self, out: &mut [i16], channels: usize) -> usize {
        self.fill_with(out, channels, 0, |s| {
            (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        })
    }

    /// Drain into a `u16` interleaved buffer, mapping `[-1.0, 1.0]` to
    /// `[0, u16::MAX]`. Used by the U16 device callback. **No allocation.**
    pub fn fill_u16(&mut self, out: &mut [u16], channels: usize) -> usize {
        self.fill_with(out, channels, 0, |s| {
            (((s.clamp(-1.0, 1.0) + 1.0) * 0.5) * u16::MAX as f32) as u16
        })
    }

    /// Number of mono samples currently buffered.
    pub fn occupied(&self) -> usize {
        self.0.occupied_len()
    }
}

/// Stand-alone output ring buffer pair for testing or custom pipelines.
/// `producer` is pushed by the render thread; `consumer` is drained by the audio
/// callback.
pub fn create_output_buffer(capacity: usize) -> (OutputProducer, OutputConsumer) {
    let rb = HeapRb::<f32>::new(capacity);
    let (producer, consumer) = rb.split();
    (OutputProducer(producer), OutputConsumer(consumer))
}

/// Handle to a running audio output engine.
///
/// Owns the device thread (cpal stream) and the render thread. Dropping or
/// calling [`AudioOutput::stop`] tears both down and releases the device.
pub struct AudioOutput {
    stop: Arc<AtomicBool>,
    device_thread: Option<JoinHandle<()>>,
    render_thread: Option<JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
}

impl AudioOutput {
    /// Open the default output device and start playing samples from a
    /// [`RenderSource`].
    ///
    /// `make_source` is called **once the device sample rate is known** so the
    /// source can be constructed at the device rate (e.g. a metronome needs the
    /// rate to time its clicks). It runs on the render thread.
    pub fn start<F, S>(make_source: F) -> Result<Self, OutputEngineError>
    where
        F: FnOnce(u32) -> S + Send + 'static,
        S: RenderSource + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let (producer, consumer) = create_output_buffer(RING_CAPACITY);
        let (startup_tx, startup_rx) = mpsc::channel::<Result<(u32, u16), OutputEngineError>>();

        // --- Device thread: owns the cpal output Stream (it is `!Send`). ---
        let stop_device = Arc::clone(&stop);
        let device_thread = thread::Builder::new()
            .name("audio-output-device".into())
            .spawn(move || run_device(consumer, stop_device, startup_tx))
            .map_err(|e| OutputEngineError::BuildStream(e.to_string()))?;

        // Wait for the device to negotiate a config (or fail).
        let (sample_rate, channels) = match startup_rx.recv() {
            Ok(Ok(cfg)) => cfg,
            Ok(Err(e)) => {
                let _ = device_thread.join();
                return Err(e);
            }
            Err(_) => {
                let _ = device_thread.join();
                return Err(OutputEngineError::StartupChannelClosed);
            }
        };

        // --- Render thread: owns the source + producer. ---
        let stop_render = Arc::clone(&stop);
        let render_thread = match thread::Builder::new()
            .name("audio-output-render".into())
            .spawn(move || run_render(make_source(sample_rate), producer, stop_render))
        {
            Ok(h) => h,
            Err(e) => {
                // Device is up but render failed to spawn — tear the device down.
                stop.store(true, Ordering::Relaxed);
                let _ = device_thread.join();
                return Err(OutputEngineError::BuildStream(e.to_string()));
            }
        };

        Ok(Self {
            stop,
            device_thread: Some(device_thread),
            render_thread: Some(render_thread),
            sample_rate,
            channels,
        })
    }

    /// The negotiated output sample rate, in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The number of output channels the device is running.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Stop playback, join both threads, and release the device. Also runs on
    /// `Drop`; calling it explicitly lets a thread panic surface here.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Join the render thread first so it stops pushing, then the device.
        if let Some(h) = self.render_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.device_thread.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Device-thread entry point: builds and plays the cpal output stream whose
/// callback drains `consumer`, then parks until told to stop.
fn run_device(
    consumer: OutputConsumer,
    stop: Arc<AtomicBool>,
    startup_tx: mpsc::Sender<Result<(u32, u16), OutputEngineError>>,
) {
    let stream = match build_output_stream(consumer) {
        Ok((stream, sample_rate, channels)) => {
            if let Err(e) = stream.play() {
                let _ = startup_tx.send(Err(OutputEngineError::PlayStream(e.to_string())));
                return;
            }
            let _ = startup_tx.send(Ok((sample_rate, channels)));
            stream
        }
        Err(e) => {
            let _ = startup_tx.send(Err(e));
            return;
        }
    };

    // Keep the stream alive on this thread until shutdown. The poll is short so
    // the device (and the audio device handle) is released promptly on stop —
    // important for the future "Play with me" start/stop cycling in Slice 4.
    // Note: a brief startup underrun is expected and harmless — the device
    // begins draining the instant `play()` succeeds, before the render thread
    // has pushed its first block, so the first callback(s) emit silence (§6).
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(5));
    }
    drop(stream);
}

/// Build the cpal output stream and the callback that drains `consumer`.
/// Returns the stream plus the negotiated sample rate and channel count.
fn build_output_stream(
    mut consumer: OutputConsumer,
) -> Result<(cpal::Stream, u32, u16), OutputEngineError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(OutputEngineError::NoOutputDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|e| OutputEngineError::NoSupportedConfig(e.to_string()))?;

    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    let config = StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };
    let chan = channels as usize;

    let err_fn = |err: cpal::StreamError| {
        tracing::warn!(error = %err, "audio output stream error");
    };

    // The callback performs NO heap allocation — it only drains the
    // pre-allocated ring buffer into the device buffer.
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                consumer.fill(data, chan);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                consumer.fill_i16(data, chan);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                consumer.fill_u16(data, chan);
            },
            err_fn,
            None,
        ),
        other => {
            return Err(OutputEngineError::NoSupportedConfig(format!(
                "unsupported sample format: {other:?}"
            )));
        }
    }
    .map_err(|e| OutputEngineError::BuildStream(e.to_string()))?;

    Ok((stream, sample_rate, channels))
}

/// Render-thread entry point: render blocks of mono audio and push them into the
/// ring until told to stop. All buffers are pre-allocated here.
fn run_render<S: RenderSource>(mut source: S, mut producer: OutputProducer, stop: Arc<AtomicBool>) {
    let mut scratch = vec![0.0f32; RENDER_BLOCK];
    while !stop.load(Ordering::Relaxed) {
        if producer.vacant() >= RENDER_BLOCK {
            source.render(&mut scratch);
            // Clamp to device range here (render thread, not the realtime
            // callback) so every device format — including the transparent F32
            // path — is protected from a source that overshoots [-1, 1].
            for s in scratch.iter_mut() {
                *s = s.clamp(-1.0, 1.0);
            }
            let mut written = 0;
            // push_samples never blocks; loop in case of a partial write.
            while written < scratch.len() && !stop.load(Ordering::Relaxed) {
                let n = producer.push_samples(&scratch[written..]);
                written += n;
                if n == 0 {
                    thread::sleep(RENDER_BACKOFF);
                }
            }
        } else {
            thread::sleep(RENDER_BACKOFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{Metronome, MetronomeConfig, TuningDrone};

    #[test]
    fn fill_drains_in_order_mono() {
        let (mut p, mut c) = create_output_buffer(16);
        assert_eq!(p.push_samples(&[1.0, 2.0, 3.0]), 3);
        let mut out = [0.0f32; 3];
        let frames = c.fill(&mut out, 1);
        assert_eq!(frames, 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn fill_fans_each_sample_across_channels() {
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[0.5, -0.5]);
        let mut out = [0.0f32; 4]; // 2 frames x 2 channels
        let frames = c.fill(&mut out, 2);
        assert_eq!(frames, 2);
        // Each mono sample duplicated to both channels of its frame.
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn fill_zero_fills_on_full_underrun() {
        let (_p, mut c) = create_output_buffer(16);
        let mut out = [9.9f32; 4];
        let frames = c.fill(&mut out, 2);
        assert_eq!(frames, 0);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn fill_partial_then_silence() {
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[0.7]);
        let mut out = [9.9f32; 4]; // 2 frames x 2 channels, only 1 frame of audio
        let frames = c.fill(&mut out, 2);
        assert_eq!(frames, 1);
        assert_eq!(out, [0.7, 0.7, 0.0, 0.0]);
    }

    #[test]
    fn fill_zero_channels_is_silence() {
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[1.0, 2.0]);
        let mut out = [9.9f32; 3];
        let frames = c.fill(&mut out, 0);
        assert_eq!(frames, 0);
        assert_eq!(out, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn push_over_capacity_drops_tail_and_keeps_fifo_order() {
        // "Lossless until full": push more than the ring holds. The contract is
        // (a) excess is dropped (not all written, no panic/block), (b) the ring
        // ends full, and (c) the RETAINED samples are the *oldest* (FIFO) and
        // drain back in order — a ring that kept the wrong end would fail (c).
        let (mut p, mut c) = create_output_buffer(8);
        let written = p.push_samples(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        assert!(written < 10, "excess past capacity must be dropped");
        assert!(written > 0, "some samples must be written");
        assert_eq!(
            p.vacant(),
            0,
            "ring must be full after an over-capacity push"
        );

        // Drain everything and confirm it's exactly the first `written` samples
        // in order — proves the dropped samples were the tail, not the head.
        let mut out = vec![0.0f32; written];
        let frames = c.fill(&mut out, 1);
        assert_eq!(frames, written);
        let expected: Vec<f32> = (1..=written).map(|i| i as f32).collect();
        assert_eq!(
            out, expected,
            "retained samples must be the oldest, in order"
        );
    }

    #[test]
    fn push_into_full_ring_returns_zero_and_never_blocks() {
        // The render loop relies on push_samples returning 0 on a full ring to
        // trigger its backoff. A bug returning the input length (pretending to
        // write) would silently lose audio and spin the render thread.
        let (mut p, _c) = create_output_buffer(8);
        // Fill the ring completely.
        while p.vacant() > 0 {
            assert_eq!(p.push_samples(&[0.1]), 1);
        }
        assert_eq!(p.vacant(), 0);
        // Further pushes write nothing and return immediately.
        assert_eq!(p.push_samples(&[0.2]), 0);
        assert_eq!(p.push_samples(&[0.2, 0.3, 0.4]), 0);
    }

    #[test]
    fn fill_i16_converts_full_scale_and_silences_underrun() {
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[1.0, -1.0, 0.0]);
        let mut out = [123i16; 4]; // 4 frames mono; only 3 have audio
        let frames = c.fill_i16(&mut out, 1);
        assert_eq!(frames, 3);
        // 1.0 -> i16::MAX, -1.0 -> -i16::MAX, 0.0 -> 0, underrun frame -> 0.
        assert_eq!(out, [i16::MAX, -i16::MAX, 0, 0]);
    }

    #[test]
    fn fill_i16_clamps_out_of_range_input() {
        // Defense in depth: even if an out-of-range sample reaches the ring, the
        // i16 conversion must not wrap around to a huge negative value.
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[2.0, -2.0]);
        let mut out = [0i16; 2];
        c.fill_i16(&mut out, 1);
        assert_eq!(out, [i16::MAX, -i16::MAX]);
    }

    #[test]
    fn fill_u16_maps_range_to_unsigned() {
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[1.0, -1.0, 0.0]);
        let mut out = [7u16; 3];
        let frames = c.fill_u16(&mut out, 1);
        assert_eq!(frames, 3);
        // 1.0 -> u16::MAX, -1.0 -> 0, 0.0 -> midpoint (32767 after truncation).
        assert_eq!(out, [u16::MAX, 0, 32_767]);
    }

    #[test]
    fn fill_silences_remainder_when_len_not_channel_multiple() {
        // out.len() (5) is not a multiple of channels (2): 2 full frames + a
        // stray slot. The stray must be silence, and frames_filled must not
        // exceed out.len() / channels.
        let (mut p, mut c) = create_output_buffer(16);
        p.push_samples(&[0.5, 0.5, 0.5]);
        let mut out = [9.9f32; 5];
        let frames = c.fill(&mut out, 2);
        assert_eq!(frames, 2, "only out.len()/channels frames are produced");
        assert_eq!(out, [0.5, 0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn output_mixer_render_fills_block_with_audio() {
        let mut mixer = OutputMixer::new();
        mixer.set_drone(Some(
            TuningDrone::new(
                crate::output::DroneConfig {
                    frequency_hz: 440.0,
                    waveform: crate::output::Waveform::Sine,
                    volume: 1.0,
                },
                44_100,
            )
            .unwrap(),
        ));
        let mut out = [0.0f32; 256];
        mixer.render(&mut out);
        // A 440 Hz sine over 256 samples must produce nonzero, in-range audio.
        assert!(
            out.iter().any(|s| s.abs() > 0.01),
            "render produced silence for an active drone"
        );
        assert!(
            out.iter().all(|s| (-1.0..=1.0).contains(s)),
            "render produced out-of-range samples"
        );
    }

    #[test]
    fn metronome_render_source_round_trips_through_ring() {
        // Render a metronome through the ring and confirm a click reaches the
        // consumer in order (nonzero audio appears).
        let m = Metronome::new(
            MetronomeConfig {
                bpm: 120.0,
                time_signature: (4, 4),
                accent_first_beat: true,
                volume: 0.9,
            },
            44_100,
        )
        .unwrap();
        let mut mixer = OutputMixer::new();
        mixer.set_metronome(Some(m));

        let (mut p, mut c) = create_output_buffer(4096);
        let mut scratch = [0.0f32; 2048];
        mixer.render(&mut scratch);
        let pushed = p.push_samples(&scratch);
        assert!(pushed > 0);

        let mut out = vec![0.0f32; pushed];
        let frames = c.fill(&mut out, 1);
        assert_eq!(frames, pushed);
        assert!(
            out.iter().any(|s| s.abs() > 1e-4),
            "no click audio drained from the ring"
        );
    }

    /// A render source that emits a constant value — easy to assert on.
    struct ConstSource(f32);
    impl RenderSource for ConstSource {
        fn render(&mut self, out: &mut [f32]) {
            for slot in out.iter_mut() {
                *slot = self.0;
            }
        }
    }

    #[test]
    fn run_render_fills_ring_and_halts_on_stop_flag() {
        // Device-free coverage of the render thread: it must (a) push rendered
        // audio into the ring, and (b) stop and let the thread join when the
        // stop flag flips — proving the shutdown handshake terminates the thread
        // rather than spinning or deadlocking.
        let (producer, mut consumer) = create_output_buffer(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let handle = thread::spawn(move || run_render(ConstSource(0.5), producer, stop_worker));

        // Wait (bounded) for the render thread to produce audio.
        let mut produced = false;
        for _ in 0..200 {
            if consumer.occupied() > 0 {
                produced = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            produced,
            "render thread never pushed any audio into the ring"
        );

        // The rendered (and clamped) value reaches the consumer in order.
        let mut out = [0.0f32; 64];
        let frames = consumer.fill(&mut out, 1);
        assert!(frames > 0);
        assert!(
            out[..frames].iter().all(|s| (*s - 0.5).abs() < 1e-6),
            "render thread produced the wrong samples"
        );

        // Flip the stop flag and confirm the thread actually exits (join returns).
        stop.store(true, Ordering::Relaxed);
        handle
            .join()
            .expect("render thread should exit cleanly on stop");
    }

    #[test]
    fn run_render_clamps_out_of_range_source() {
        // A misbehaving source that overshoots [-1, 1] must be clamped before
        // its samples reach the ring (and thus the F32 device path).
        let (producer, mut consumer) = create_output_buffer(4096);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let handle = thread::spawn(move || run_render(ConstSource(3.0), producer, stop_worker));

        let mut got = false;
        let mut out = [0.0f32; 64];
        for _ in 0..200 {
            if consumer.occupied() >= out.len() {
                got = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        assert!(got, "render thread never filled the ring");
        let frames = consumer.fill(&mut out, 1);
        assert!(frames > 0);
        assert!(
            out[..frames].iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "out-of-range source was not clamped to 1.0 before the ring"
        );
    }
}
