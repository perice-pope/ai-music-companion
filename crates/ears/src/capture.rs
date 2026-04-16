//! Audio capture from the default input device via cpal.
//!
//! Samples are written into a lock-free SPSC ring buffer so a consumer
//! thread can read them without blocking the audio callback.
//! **No heap allocations occur in the audio callback.**

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use thiserror::Error;

/// Errors from audio capture.
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("no input device available")]
    NoInputDevice,
    #[error("no supported input config: {0}")]
    NoSupportedConfig(String),
    #[error("failed to build input stream: {0}")]
    BuildStream(String),
    #[error("failed to start stream: {0}")]
    PlayStream(String),
}

/// Configuration for audio capture.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Ring buffer capacity in samples. Larger = more latency tolerance,
    /// smaller = lower latency but risk of overrun.
    pub buffer_capacity: usize,
    /// Preferred sample rate in Hz. Falls back to device default if unavailable.
    pub preferred_sample_rate: Option<u32>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            buffer_capacity: 4096,
            preferred_sample_rate: Some(44100),
        }
    }
}

/// Handle to an active audio capture session.
///
/// Holds the cpal stream (which runs in its own thread) and provides
/// a consumer handle for reading captured samples.
pub struct AudioCapture {
    /// Keep the stream alive — audio stops when this is dropped.
    _stream: Stream,
    /// Consumer side of the ring buffer. Read samples from here.
    consumer: ringbuf::HeapCons<f32>,
    /// The actual sample rate negotiated with the device.
    sample_rate: u32,
    /// Number of channels being captured.
    channels: u16,
}

impl AudioCapture {
    /// Open the default input device and start capturing audio.
    ///
    /// Returns a handle with a consumer for reading samples from another thread.
    pub fn new(config: CaptureConfig) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(CaptureError::NoInputDevice)?;

        let supported_config = device
            .default_input_config()
            .map_err(|e| CaptureError::NoSupportedConfig(e.to_string()))?;

        let sample_rate = config
            .preferred_sample_rate
            .unwrap_or(supported_config.sample_rate().0);
        let channels = supported_config.channels();

        let stream_config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let rb = HeapRb::<f32>::new(config.buffer_capacity);
        let (mut producer, consumer) = rb.split();

        let err_fn = |err: cpal::StreamError| {
            eprintln!("audio stream error: {err}");
        };

        // Build the stream based on the device's native sample format.
        // The callback does NO heap allocation — it only writes to the
        // pre-allocated ring buffer via the producer handle.
        let stream = match supported_config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Write as many samples as the ring buffer can accept.
                    // If the consumer is slow, excess samples are silently dropped
                    // (better than blocking the audio thread).
                    producer.push_slice(data);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    for &sample in data {
                        let f = sample as f32 / i16::MAX as f32;
                        let _ = producer.try_push(f);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    for &sample in data {
                        let f = (sample as f32 / u16::MAX as f32) * 2.0 - 1.0;
                        let _ = producer.try_push(f);
                    }
                },
                err_fn,
                None,
            ),
            format => {
                return Err(CaptureError::NoSupportedConfig(format!(
                    "unsupported sample format: {format:?}"
                )));
            }
        }
        .map_err(|e| CaptureError::BuildStream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| CaptureError::PlayStream(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            consumer,
            sample_rate,
            channels,
        })
    }

    /// Read available samples into the provided buffer.
    /// Returns the number of samples actually read.
    pub fn read_samples(&mut self, buf: &mut [f32]) -> usize {
        self.consumer.pop_slice(buf)
    }

    /// Number of samples currently available to read.
    pub fn available(&self) -> usize {
        self.consumer.occupied_len()
    }

    /// The sample rate the device is running at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Number of audio channels being captured.
    pub fn channels(&self) -> u16 {
        self.channels
    }
}

/// Stand-alone ring buffer pair for testing or custom pipelines.
/// Returns (writer, reader) — writer goes to the audio thread,
/// reader goes to the processing thread.
pub fn create_sample_buffer(capacity: usize) -> (SampleProducer, SampleConsumer) {
    let rb = HeapRb::<f32>::new(capacity);
    let (producer, consumer) = rb.split();
    (SampleProducer(producer), SampleConsumer(consumer))
}

/// Producer side of a sample ring buffer (audio thread side).
pub struct SampleProducer(ringbuf::HeapProd<f32>);

impl SampleProducer {
    /// Push samples into the buffer. Returns how many were written.
    /// Never blocks — excess samples are silently dropped.
    pub fn push_samples(&mut self, samples: &[f32]) -> usize {
        self.0.push_slice(samples)
    }
}

/// Consumer side of a sample ring buffer (processing thread side).
pub struct SampleConsumer(ringbuf::HeapCons<f32>);

impl SampleConsumer {
    /// Read samples from the buffer into `buf`. Returns how many were read.
    pub fn read_samples(&mut self, buf: &mut [f32]) -> usize {
        self.0.pop_slice(buf)
    }

    /// Number of samples available to read.
    pub fn available(&self) -> usize {
        self.0.occupied_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_roundtrip() {
        let (mut producer, mut consumer) = create_sample_buffer(1024);

        let input: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        let written = producer.push_samples(&input);
        assert_eq!(written, 512);

        let mut output = vec![0.0f32; 512];
        let read = consumer.read_samples(&mut output);
        assert_eq!(read, 512);
        assert_eq!(input, output);
    }

    #[test]
    fn ring_buffer_overflow_drops_excess() {
        let (mut producer, mut consumer) = create_sample_buffer(64);

        // Write more than capacity
        let input: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let written = producer.push_samples(&input);
        assert_eq!(written, 64); // Only 64 fit

        let mut output = vec![0.0f32; 64];
        let read = consumer.read_samples(&mut output);
        assert_eq!(read, 64);
    }

    #[test]
    fn ring_buffer_empty_read_returns_zero() {
        let (_producer, mut consumer) = create_sample_buffer(64);
        let mut output = vec![0.0f32; 32];
        let read = consumer.read_samples(&mut output);
        assert_eq!(read, 0);
    }

    #[test]
    fn ring_buffer_available_count() {
        let (mut producer, consumer) = create_sample_buffer(256);
        assert_eq!(consumer.available(), 0);

        let samples = [1.0f32; 100];
        producer.push_samples(&samples);
        assert_eq!(consumer.available(), 100);
    }

    #[test]
    fn default_capture_config() {
        let config = CaptureConfig::default();
        assert_eq!(config.buffer_capacity, 4096);
        assert_eq!(config.preferred_sample_rate, Some(44100));
    }
}
