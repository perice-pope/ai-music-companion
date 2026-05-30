//! Decode compressed/uncompressed audio bytes to mono f32 samples via Symphonia.

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::TranscribeError;

/// Decode `bytes` (any Symphonia-supported container/codec) into
/// `(mono_samples, sample_rate)`. Multi-channel audio is downmixed by averaging.
/// `extension` (e.g. `"wav"`, `"mp3"`) is an optional probe hint.
pub fn decode_audio(
    bytes: Vec<u8>,
    extension: Option<&str>,
) -> Result<(Vec<f32>, u32), TranscribeError> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| TranscribeError::Decode(e.to_string()))?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| TranscribeError::Decode("no audio track".into()))?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| TranscribeError::Decode("unknown sample rate".into()))?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| TranscribeError::Decode(e.to_string()))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Clean EOF or a truncated stream end — stop reading.
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(TranscribeError::Decode(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let spec = *decoded.spec();
                    sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                }
                let buf = sample_buf.as_mut().unwrap();
                buf.copy_interleaved_ref(decoded);
                for frame in buf.samples().chunks(channels) {
                    mono.push(frame.iter().sum::<f32>() / channels as f32);
                }
            }
            // Skip recoverable per-packet decode errors, like the reference.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(TranscribeError::Decode(e.to_string())),
        }
    }

    if mono.is_empty() {
        return Err(TranscribeError::Decode("no audio samples decoded".into()));
    }
    Ok((mono, sample_rate))
}
