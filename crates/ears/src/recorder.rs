//! # Recorder — opt-in WAV session recording
//!
//! `AudioRecorder` records captured audio samples to a WAV file. It is
//! **disabled by default** and only becomes active after an explicit
//! [`AudioRecorder::start`] call. No background recording ever happens.
//!
//! Samples are expected to arrive on the processing thread (not the audio
//! thread). The recorder uses a [`BufWriter`] and pre-allocated scratch
//! buffers to keep per-call overhead low.
//!
//! The WAV file is written manually — no `hound` dependency. The header is
//! patched on [`AudioRecorder::stop`] with the final RIFF and data sizes.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// WAV header size in bytes — `RIFF` + `fmt ` + `data` chunks (canonical PCM).
const WAV_HEADER_SIZE: u64 = 44;

/// Offset of the RIFF chunk size field (file_size - 8).
const WAV_RIFF_SIZE_OFFSET: u64 = 4;

/// Offset of the `data` subchunk size field.
const WAV_DATA_SIZE_OFFSET: u64 = 40;

/// Configuration for an [`AudioRecorder`].
///
/// Values are validated on [`AudioRecorder::start`]; invalid combinations
/// surface as [`RecorderError::InvalidConfig`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderConfig {
    /// Directory that WAV files are written into. Must exist.
    pub output_dir: PathBuf,
    /// Safety cap on a single recording. When exceeded, [`AudioRecorder::write_samples`]
    /// returns [`RecorderError::SizeLimitReached`] and refuses further writes.
    ///
    /// **WAV format limit:** classic RIFF WAV stores its `RIFF` and `data`
    /// chunk sizes as unsigned 32-bit integers, so a single WAV file cannot
    /// legally exceed ~4 GiB. `RecorderConfig::validate()` enforces
    /// `max_bytes_per_file <= u32::MAX` and rejects anything larger —
    /// without this, `stop()` would `as u32`-truncate the patched size
    /// fields and produce a structurally invalid header.
    pub max_bytes_per_file: u64,
    /// Number of audio channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Sample rate in Hz (e.g. 44100, 48000).
    pub sample_rate: u32,
    /// Bit depth per sample. Only 16 and 24 are supported.
    pub bits_per_sample: u16,
}

impl RecorderConfig {
    fn validate(&self) -> Result<(), RecorderError> {
        if self.channels == 0 {
            return Err(RecorderError::InvalidConfig("channels must be >= 1".into()));
        }
        if self.sample_rate == 0 {
            return Err(RecorderError::InvalidConfig(
                "sample_rate must be > 0".into(),
            ));
        }
        if self.bits_per_sample != 16 && self.bits_per_sample != 24 {
            return Err(RecorderError::InvalidConfig(format!(
                "bits_per_sample must be 16 or 24, got {}",
                self.bits_per_sample
            )));
        }
        if self.max_bytes_per_file < WAV_HEADER_SIZE {
            return Err(RecorderError::InvalidConfig(format!(
                "max_bytes_per_file must be at least {WAV_HEADER_SIZE} (WAV header size)"
            )));
        }
        // WAV stores `RIFF` and `data` chunk sizes as u32. Values beyond
        // `u32::MAX` would silently truncate in `stop()` when the header
        // is patched, yielding a structurally invalid file. Reject at
        // config time rather than corrupting bytes at session end.
        if self.max_bytes_per_file > u64::from(u32::MAX) {
            return Err(RecorderError::InvalidConfig(format!(
                "max_bytes_per_file {} exceeds the WAV 4 GiB (u32::MAX) size limit",
                self.max_bytes_per_file
            )));
        }
        if !self.output_dir.exists() {
            return Err(RecorderError::OutputDirMissing(self.output_dir.clone()));
        }
        Ok(())
    }
}

/// A timestamped "best take" bookmark recorded mid-session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BestTakeMark {
    /// Seconds elapsed from the start of the recording.
    pub seconds_from_start: f64,
    /// Optional human-readable label.
    pub label: Option<String>,
}

/// Metadata returned by [`AudioRecorder::stop`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMetadata {
    pub session_id: String,
    pub instrument: String,
    pub file_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub duration_secs: f64,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub samples_written: u64,
    pub best_take_marks: Vec<BestTakeMark>,
}

/// Errors surfaced by the recorder and retention layers.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("recorder is already recording")]
    AlreadyRecording,
    #[error("recorder is not recording")]
    NotRecording,
    #[error("output dir does not exist: {0}")]
    OutputDirMissing(PathBuf),
    #[error("file size limit reached ({0} bytes)")]
    SizeLimitReached(u64),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

struct ActiveRecording {
    session_id: String,
    instrument: String,
    path: PathBuf,
    file: BufWriter<File>,
    started_at: DateTime<Utc>,
    samples_written: u64,
    data_bytes_written: u64,
    best_take_marks: Vec<BestTakeMark>,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    max_bytes_per_file: u64,
}

enum RecordingState {
    Idle,
    Recording(ActiveRecording),
}

/// Opt-in session audio recorder.
///
/// Construct with [`AudioRecorder::new`]; start/stop a recording with
/// [`AudioRecorder::start`] / [`AudioRecorder::stop`]. The recorder is Idle
/// until `start` is called, and returns to Idle after `stop`.
pub struct AudioRecorder {
    config: RecorderConfig,
    state: RecordingState,
}

impl AudioRecorder {
    /// Create a new, idle recorder. No files are opened.
    pub fn new(config: RecorderConfig) -> Self {
        Self {
            config,
            state: RecordingState::Idle,
        }
    }

    /// Whether a recording is currently in progress.
    pub fn is_recording(&self) -> bool {
        matches!(self.state, RecordingState::Recording(_))
    }

    /// Seconds elapsed since the current recording started, or `None` when idle.
    pub fn current_duration_secs(&self) -> Option<f64> {
        match &self.state {
            RecordingState::Recording(r) => Some(samples_to_secs(
                r.samples_written,
                r.sample_rate,
                r.channels,
            )),
            RecordingState::Idle => None,
        }
    }

    /// Begin a new recording. Creates the target WAV file with a placeholder
    /// header and transitions the recorder to the Recording state.
    ///
    /// Returns the path of the WAV file that was created.
    pub fn start(&mut self, session_id: &str, instrument: &str) -> Result<PathBuf, RecorderError> {
        if matches!(self.state, RecordingState::Recording(_)) {
            return Err(RecorderError::AlreadyRecording);
        }
        self.config.validate()?;

        let started_at = Utc::now();
        let filename = build_filename(started_at, instrument, session_id);
        let path = self.config.output_dir.join(filename);

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);

        write_wav_placeholder_header(
            &mut writer,
            self.config.channels,
            self.config.sample_rate,
            self.config.bits_per_sample,
        )?;

        self.state = RecordingState::Recording(ActiveRecording {
            session_id: session_id.to_string(),
            instrument: instrument.to_string(),
            path: path.clone(),
            file: writer,
            started_at,
            samples_written: 0,
            data_bytes_written: 0,
            best_take_marks: Vec::new(),
            channels: self.config.channels,
            sample_rate: self.config.sample_rate,
            bits_per_sample: self.config.bits_per_sample,
            max_bytes_per_file: self.config.max_bytes_per_file,
        });

        Ok(path)
    }

    /// Append PCM samples (f32 in `[-1.0, 1.0]`) to the current recording.
    ///
    /// Samples are clamped to the `[-1.0, 1.0]` range and converted to i16
    /// or i24 depending on the configured bit depth. If the write would push
    /// the file past `max_bytes_per_file`, recording is torn down and
    /// [`RecorderError::SizeLimitReached`] is returned.
    pub fn write_samples(&mut self, samples: &[f32]) -> Result<(), RecorderError> {
        let active = match &mut self.state {
            RecordingState::Recording(a) => a,
            RecordingState::Idle => return Err(RecorderError::NotRecording),
        };

        let bytes_per_sample = (active.bits_per_sample / 8) as u64;
        let incoming_bytes = samples.len() as u64 * bytes_per_sample;
        let projected = WAV_HEADER_SIZE + active.data_bytes_written + incoming_bytes;
        if projected > active.max_bytes_per_file {
            return Err(RecorderError::SizeLimitReached(active.max_bytes_per_file));
        }

        match active.bits_per_sample {
            16 => write_samples_i16(&mut active.file, samples)?,
            24 => write_samples_i24(&mut active.file, samples)?,
            other => {
                return Err(RecorderError::InvalidConfig(format!(
                    "unsupported bits_per_sample {other}"
                )))
            }
        }

        active.samples_written += samples.len() as u64;
        active.data_bytes_written += incoming_bytes;
        Ok(())
    }

    /// Record a best-take marker at the current timestamp.
    pub fn mark_best_take(&mut self, label: Option<String>) -> Result<(), RecorderError> {
        let active = match &mut self.state {
            RecordingState::Recording(a) => a,
            RecordingState::Idle => return Err(RecorderError::NotRecording),
        };
        let seconds_from_start =
            samples_to_secs(active.samples_written, active.sample_rate, active.channels);
        active.best_take_marks.push(BestTakeMark {
            seconds_from_start,
            label,
        });
        Ok(())
    }

    /// Finish the current recording. Patches the WAV header with real sizes,
    /// flushes to disk, returns full metadata, and moves back to Idle.
    pub fn stop(&mut self) -> Result<RecordingMetadata, RecorderError> {
        let state = std::mem::replace(&mut self.state, RecordingState::Idle);
        let mut active = match state {
            RecordingState::Recording(a) => a,
            RecordingState::Idle => return Err(RecorderError::NotRecording),
        };

        // Patch the header with real sizes.
        active.file.flush()?;
        let data_size = active.data_bytes_written as u32;
        // RIFF size = (overall file size) - 8 = (header size - 8) + data size
        // Header is 44 bytes total, RIFF field spans bytes 0..8 so 36 + data_size.
        let riff_size = 36u32 + data_size;

        active.file.seek(SeekFrom::Start(WAV_RIFF_SIZE_OFFSET))?;
        active.file.write_all(&riff_size.to_le_bytes())?;
        active.file.seek(SeekFrom::Start(WAV_DATA_SIZE_OFFSET))?;
        active.file.write_all(&data_size.to_le_bytes())?;
        active.file.flush()?;

        // Drop the BufWriter/File to release the handle.
        drop(active.file);

        let duration_secs =
            samples_to_secs(active.samples_written, active.sample_rate, active.channels);

        Ok(RecordingMetadata {
            session_id: active.session_id,
            instrument: active.instrument,
            file_path: active.path,
            started_at: active.started_at,
            duration_secs,
            channels: active.channels,
            sample_rate: active.sample_rate,
            bits_per_sample: active.bits_per_sample,
            samples_written: active.samples_written,
            best_take_marks: active.best_take_marks,
        })
    }

    /// Rename a finalized recording with a `_best` suffix so the retention
    /// sweeper will preserve it.
    ///
    /// This is a stateless helper — call it after [`AudioRecorder::stop`]
    /// returns a [`RecordingMetadata`] whose `best_take_marks` you want to
    /// preserve on disk.
    pub fn rename_with_best_marker(path: &Path) -> Result<PathBuf, RecorderError> {
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
            RecorderError::InvalidConfig(format!("path has no file stem: {}", path.display()))
        })?;
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("wav");
        let parent = path.parent().ok_or_else(|| {
            RecorderError::InvalidConfig(format!("path has no parent: {}", path.display()))
        })?;
        let new_path = parent.join(format!("{stem}_best.{ext}"));
        std::fs::rename(path, &new_path)?;
        Ok(new_path)
    }
}

#[allow(clippy::cast_precision_loss)]
fn samples_to_secs(samples: u64, sample_rate: u32, channels: u16) -> f64 {
    let ch = u64::from(channels.max(1));
    let frames = samples / ch;
    let sr = f64::from(sample_rate.max(1));
    frames as f64 / sr
}

fn build_filename(when: DateTime<Utc>, instrument: &str, session_id: &str) -> String {
    // YYYY-MM-DD_HH-MM-SS_instrument_sessionid.wav
    format!(
        "{}_{}_{}.wav",
        when.format("%Y-%m-%d_%H-%M-%S"),
        sanitize(instrument),
        sanitize(session_id),
    )
}

fn sanitize(component: &str) -> String {
    component
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Write a 44-byte canonical PCM WAV header with zero sizes — sizes are
/// patched in [`AudioRecorder::stop`].
fn write_wav_placeholder_header(
    w: &mut BufWriter<File>,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
) -> std::io::Result<()> {
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;

    w.write_all(b"RIFF")?;
    w.write_all(&0u32.to_le_bytes())?; // placeholder for RIFF chunk size
    w.write_all(b"WAVE")?;

    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // fmt subchunk size
    w.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())?;

    w.write_all(b"data")?;
    w.write_all(&0u32.to_le_bytes())?; // placeholder for data size

    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn write_samples_i16(w: &mut BufWriter<File>, samples: &[f32]) -> std::io::Result<()> {
    // Symmetric scaling: map 1.0 -> i16::MAX, -1.0 -> i16::MIN.
    let pos_scale = f64::from(i16::MAX);
    let neg_scale = -f64::from(i16::MIN);
    let min_i32 = i32::from(i16::MIN);
    let max_i32 = i32::from(i16::MAX);
    for &s in samples {
        let clamped = f64::from(s.clamp(-1.0, 1.0));
        let scaled = if clamped >= 0.0 {
            (clamped * pos_scale).round() as i32
        } else {
            (clamped * neg_scale).round() as i32
        };
        // Post-clamp guarantees the cast to i16 is lossless.
        let value = scaled.clamp(min_i32, max_i32) as i16;
        w.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn write_samples_i24(w: &mut BufWriter<File>, samples: &[f32]) -> std::io::Result<()> {
    // 24-bit signed range.
    const I24_MAX: i32 = 8_388_607; // 2^23 - 1
    const I24_MIN: i32 = -8_388_608; // -(2^23)
    let pos_scale = f64::from(I24_MAX);
    let neg_scale = -f64::from(I24_MIN);
    for &s in samples {
        let clamped = f64::from(s.clamp(-1.0, 1.0));
        let scaled = if clamped >= 0.0 {
            (clamped * pos_scale).round() as i32
        } else {
            (clamped * neg_scale).round() as i32
        };
        let value = scaled.clamp(I24_MIN, I24_MAX);
        let bytes = value.to_le_bytes();
        // Write three little-endian bytes.
        w.write_all(&bytes[0..3])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const EPSILON: f64 = 1e-9;

    fn base_config(dir: &Path) -> RecorderConfig {
        RecorderConfig {
            output_dir: dir.to_path_buf(),
            max_bytes_per_file: 10 * 1024 * 1024,
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 16,
        }
    }

    #[test]
    fn recorder_starts_idle() {
        let tmp = TempDir::new().unwrap();
        let rec = AudioRecorder::new(base_config(tmp.path()));
        assert!(!rec.is_recording());
        assert!(rec.current_duration_secs().is_none());
    }

    #[test]
    fn start_creates_file_at_correct_path() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("sess-abc", "trumpet").unwrap();
        assert!(path.exists(), "expected file to be created at {path:?}");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.ends_with(".wav"),
            "filename should end in .wav: {name}"
        );
        assert!(
            name.contains("trumpet"),
            "filename should contain instrument: {name}"
        );
        assert!(
            name.contains("sess-abc"),
            "filename should contain session id: {name}"
        );
        // Parent must match output dir.
        assert_eq!(path.parent().unwrap(), tmp.path());
    }

    #[test]
    fn start_twice_errors() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        rec.start("s1", "trumpet").unwrap();
        let err = rec.start("s2", "trumpet").unwrap_err();
        assert!(matches!(err, RecorderError::AlreadyRecording));
    }

    #[test]
    fn stop_without_start_errors() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let err = rec.stop().unwrap_err();
        assert!(matches!(err, RecorderError::NotRecording));
    }

    #[test]
    fn write_samples_without_start_errors() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let err = rec.write_samples(&[0.0; 64]).unwrap_err();
        assert!(matches!(err, RecorderError::NotRecording));
    }

    #[test]
    fn config_rejects_zero_channels() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.channels = 0;
        let mut rec = AudioRecorder::new(cfg);
        let err = rec.start("s", "t").unwrap_err();
        match err {
            RecorderError::InvalidConfig(_) => {}
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn config_rejects_unsupported_bits_per_sample() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.bits_per_sample = 12;
        let mut rec = AudioRecorder::new(cfg);
        let err = rec.start("s", "t").unwrap_err();
        match err {
            RecorderError::InvalidConfig(msg) => {
                assert!(msg.contains("bits_per_sample"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn config_rejects_max_bytes_above_u32_max() {
        // Regression: WAV's RIFF/data chunk sizes are 32-bit unsigned. A
        // `max_bytes_per_file` larger than `u32::MAX` would silently
        // truncate in `stop()` when patching the header, producing a
        // structurally invalid file. Reject at config validation time.
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.max_bytes_per_file = u64::from(u32::MAX) + 1;
        let mut rec = AudioRecorder::new(cfg);
        let err = rec.start("s", "t").unwrap_err();
        match err {
            RecorderError::InvalidConfig(msg) => {
                assert!(
                    msg.contains("4 GiB") || msg.contains("u32::MAX"),
                    "error should mention the WAV 4 GiB limit, got: {msg}"
                );
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn config_accepts_max_bytes_exactly_at_u32_max() {
        // Boundary: exactly u32::MAX is valid (though impractical).
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.max_bytes_per_file = u64::from(u32::MAX);
        let mut rec = AudioRecorder::new(cfg);
        // Start validates config and should succeed at the boundary.
        let _path = rec
            .start("s", "t")
            .expect("u32::MAX exactly should be accepted");
        // We can stop cleanly without writing any samples.
        rec.stop().unwrap();
    }

    #[test]
    fn config_rejects_zero_sample_rate() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.sample_rate = 0;
        let mut rec = AudioRecorder::new(cfg);
        let err = rec.start("s", "t").unwrap_err();
        match err {
            RecorderError::InvalidConfig(_) => {}
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn config_rejects_missing_output_dir() {
        let tmp = TempDir::new().unwrap();
        let ghost = tmp.path().join("does-not-exist");
        let cfg = RecorderConfig {
            output_dir: ghost.clone(),
            ..base_config(tmp.path())
        };
        let mut rec = AudioRecorder::new(cfg);
        let err = rec.start("s", "t").unwrap_err();
        match err {
            RecorderError::OutputDirMissing(p) => assert_eq!(p, ghost),
            other => panic!("expected OutputDirMissing, got {other:?}"),
        }
    }

    fn read_all(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    #[test]
    fn wav_header_has_correct_magic_bytes() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("s", "trumpet").unwrap();
        rec.write_samples(&[0.0; 1024]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
    }

    #[test]
    fn wav_header_sample_rate_matches_config() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.sample_rate = 44100;
        let mut rec = AudioRecorder::new(cfg);
        let path = rec.start("s", "t").unwrap();
        rec.write_samples(&[0.0; 8]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let sr = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        assert_eq!(sr, 44100);
    }

    #[test]
    fn wav_header_num_channels_matches_config() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.channels = 1;
        let mut rec = AudioRecorder::new(cfg);
        let path = rec.start("s", "t").unwrap();
        rec.write_samples(&[0.0; 8]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let ch = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
        assert_eq!(ch, 1);
    }

    #[test]
    fn wav_header_bits_per_sample_matches_config() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.bits_per_sample = 16;
        let mut rec = AudioRecorder::new(cfg);
        let path = rec.start("s", "t").unwrap();
        rec.write_samples(&[0.0; 8]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let bps = u16::from_le_bytes(bytes[34..36].try_into().unwrap());
        assert_eq!(bps, 16);
    }

    #[test]
    fn wav_file_size_field_patched_on_stop() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("s", "t").unwrap();
        let samples = vec![0.0_f32; 100];
        rec.write_samples(&samples).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u64;
        // Full file size on disk should match riff_size + 8.
        let file_size = bytes.len() as u64;
        assert_eq!(
            riff_size + 8,
            file_size,
            "RIFF size field should equal total file size minus 8"
        );
    }

    #[test]
    fn wav_data_size_field_patched_on_stop() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("s", "t").unwrap();
        let sample_count: u32 = 100;
        let samples = vec![0.25_f32; sample_count as usize];
        rec.write_samples(&samples).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        // 16-bit mono: bytes = samples * 2.
        assert_eq!(data_size, sample_count * 2);
    }

    #[test]
    fn samples_are_correctly_encoded_to_i16() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("s", "t").unwrap();
        let samples = [0.0f32, 1.0, -1.0, 0.5];
        rec.write_samples(&samples).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let data = &bytes[44..];
        let v0 = i16::from_le_bytes(data[0..2].try_into().unwrap());
        let v1 = i16::from_le_bytes(data[2..4].try_into().unwrap());
        let v2 = i16::from_le_bytes(data[4..6].try_into().unwrap());
        let v3 = i16::from_le_bytes(data[6..8].try_into().unwrap());
        assert_eq!(v0, 0);
        assert_eq!(v1, i16::MAX);
        assert_eq!(v2, i16::MIN);
        // 0.5 -> ~i16::MAX/2. Allow 2 counts of rounding slack.
        let expected = (0.5f32 * f32::from(i16::MAX)).round() as i16;
        assert!(
            (v3 - expected).abs() <= 1,
            "0.5 should encode near {expected}, got {v3}"
        );
    }

    #[test]
    fn samples_clamp_on_overflow() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("s", "t").unwrap();
        rec.write_samples(&[1.5f32, -1.5f32]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let data = &bytes[44..];
        let hi = i16::from_le_bytes(data[0..2].try_into().unwrap());
        let lo = i16::from_le_bytes(data[2..4].try_into().unwrap());
        assert_eq!(hi, i16::MAX, "1.5 should clamp to i16::MAX");
        assert_eq!(lo, i16::MIN, "-1.5 should clamp to i16::MIN");
    }

    #[test]
    fn size_limit_reached_stops_recording() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        // 44 (header) + 2*100 = 244 bytes for 100 i16 samples.
        cfg.max_bytes_per_file = 100;
        let mut rec = AudioRecorder::new(cfg);
        rec.start("s", "t").unwrap();
        let err = rec.write_samples(&[0.0; 100]).unwrap_err();
        assert!(matches!(err, RecorderError::SizeLimitReached(100)));
    }

    #[test]
    fn best_take_mark_records_current_timestamp() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        rec.start("s", "t").unwrap();
        // 1 second at 44100 Hz mono.
        rec.write_samples(&[0.0; 44100]).unwrap();
        rec.mark_best_take(Some("nailed it".into())).unwrap();
        let meta = rec.stop().unwrap();
        assert_eq!(meta.best_take_marks.len(), 1);
        let mark = &meta.best_take_marks[0];
        assert_eq!(mark.label.as_deref(), Some("nailed it"));
        assert!(
            (mark.seconds_from_start - 1.0).abs() < 1e-6,
            "expected ~1.0s, got {}",
            mark.seconds_from_start
        );
    }

    #[test]
    fn stop_returns_correct_duration_secs() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        rec.start("s", "t").unwrap();
        // 2 seconds at 44100 mono.
        rec.write_samples(&[0.0; 88_200]).unwrap();
        let meta = rec.stop().unwrap();
        assert!(
            (meta.duration_secs - 2.0).abs() < EPSILON,
            "expected 2.0s, got {}",
            meta.duration_secs
        );
        assert_eq!(meta.samples_written, 88_200);
    }

    #[test]
    fn stop_resets_to_idle() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        rec.start("s1", "t").unwrap();
        rec.write_samples(&[0.0; 16]).unwrap();
        rec.stop().unwrap();
        assert!(!rec.is_recording());
        // Can start again.
        rec.start("s2", "t").unwrap();
        assert!(rec.is_recording());
        rec.stop().unwrap();
    }

    #[test]
    fn current_duration_tracks_samples_written() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        rec.start("s", "t").unwrap();
        rec.write_samples(&[0.0; 22_050]).unwrap(); // 0.5s
        let dur = rec.current_duration_secs().unwrap();
        assert!((dur - 0.5).abs() < 1e-6, "expected 0.5s, got {dur}");
        rec.stop().unwrap();
    }

    #[test]
    fn filename_contains_date_time_instrument_and_session() {
        let when = DateTime::parse_from_rfc3339("2026-04-17T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let name = build_filename(when, "trumpet", "abc123");
        assert_eq!(name, "2026-04-17_10-30-00_trumpet_abc123.wav");
    }

    #[test]
    fn rename_with_best_marker_appends_suffix() {
        let tmp = TempDir::new().unwrap();
        let mut rec = AudioRecorder::new(base_config(tmp.path()));
        let path = rec.start("abc", "trumpet").unwrap();
        rec.write_samples(&[0.0; 16]).unwrap();
        rec.stop().unwrap();
        let new_path = AudioRecorder::rename_with_best_marker(&path).unwrap();
        let name = new_path.file_name().unwrap().to_string_lossy();
        assert!(name.ends_with("_best.wav"), "got {name}");
        assert!(!path.exists(), "original path should be gone after rename");
        assert!(new_path.exists(), "renamed path should exist");
    }

    #[test]
    fn samples_encoded_to_i24_uses_three_bytes_per_sample() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = base_config(tmp.path());
        cfg.bits_per_sample = 24;
        let mut rec = AudioRecorder::new(cfg);
        let path = rec.start("s", "t").unwrap();
        rec.write_samples(&[0.0f32, 1.0, -1.0]).unwrap();
        rec.stop().unwrap();
        let bytes = read_all(&path);
        let data = &bytes[44..];
        assert_eq!(data.len(), 9, "3 samples * 3 bytes = 9 data bytes");
        // i24 max and min in LE.
        // sample 1 (1.0) -> 0x7F_FF_FF LE = [0xFF, 0xFF, 0x7F]
        assert_eq!(&data[3..6], &[0xFF, 0xFF, 0x7F]);
        // sample 2 (-1.0) -> -8_388_608 = 0xFF_80_00_00 (i32 LE); first 3 bytes = [0x00, 0x00, 0x80]
        assert_eq!(&data[6..9], &[0x00, 0x00, 0x80]);
    }
}
