//! basic-pitch model + post-processing constants.
//!
//! Values mirror `basic_pitch/constants.py` and `note_creation.py` exactly so
//! our Rust port produces output identical to the upstream Python pipeline.

/// Sample rate the model expects (Hz).
pub const AUDIO_SAMPLE_RATE: usize = 22_050;
/// STFT hop used by the model's front end.
pub const FFT_HOP: usize = 256;
/// Model input window length in seconds.
pub const AUDIO_WINDOW_LENGTH: usize = 2;
/// Samples per model input window: `SR * WINDOW_LENGTH - FFT_HOP`.
pub const AUDIO_N_SAMPLES: usize = AUDIO_SAMPLE_RATE * AUDIO_WINDOW_LENGTH - FFT_HOP; // 43_844
/// Frame rate of the model output: `SR // FFT_HOP`.
pub const ANNOTATIONS_FPS: usize = AUDIO_SAMPLE_RATE / FFT_HOP; // 86
/// Output frames per window: `FPS * WINDOW_LENGTH`.
pub const ANNOT_N_FRAMES: usize = ANNOTATIONS_FPS * AUDIO_WINDOW_LENGTH; // 172
/// Piano-key resolution of the note/onset heads.
pub const N_FREQ_BINS_NOTES: usize = 88;

/// Overlap between consecutive windows, in output frames.
pub const N_OVERLAPPING_FRAMES: usize = 30;
/// Overlap in samples: `N_OVERLAPPING_FRAMES * FFT_HOP`.
pub const OVERLAP_LEN: usize = N_OVERLAPPING_FRAMES * FFT_HOP; // 7_680
/// Stride between windows in samples: `AUDIO_N_SAMPLES - OVERLAP_LEN`.
pub const HOP_SIZE: usize = AUDIO_N_SAMPLES - OVERLAP_LEN; // 36_164

/// MIDI note number of the lowest model bin (bin 0 = A0).
pub const MIDI_OFFSET: u8 = 21;
/// Highest valid frequency bin index (`N_FREQ_BINS_NOTES - 1`).
pub const MAX_FREQ_IDX: usize = 87;

/// Default note-creation thresholds (basic-pitch defaults).
pub const DEFAULT_ONSET_THRESH: f32 = 0.5;
pub const DEFAULT_FRAME_THRESH: f32 = 0.3;
/// Minimum note length in frames (`round(127.70ms/1000 * SR/FFT_HOP)`).
pub const DEFAULT_MIN_NOTE_LEN: usize = 11;
/// Number of below-threshold frames tolerated before a note is closed.
pub const ENERGY_TOL: usize = 11;
/// Empirical per-window time correction from `note_creation.py`.
pub const MAGIC_ALIGNMENT_OFFSET: f64 = 0.0018;

/// Map a model output-frame index to a time in seconds.
///
/// Port of `note_creation.model_frames_to_time`. The per-window offset
/// compensates for the window framing + overlap trimming.
pub fn frame_to_time(frame: usize) -> f64 {
    let original = (frame * FFT_HOP) as f64 / AUDIO_SAMPLE_RATE as f64;
    let window_number = (frame / ANNOT_N_FRAMES) as f64;
    let window_offset = (FFT_HOP as f64 / AUDIO_SAMPLE_RATE as f64)
        * (ANNOT_N_FRAMES as f64 - (AUDIO_N_SAMPLES as f64 / FFT_HOP as f64))
        + MAGIC_ALIGNMENT_OFFSET;
    original - window_offset * window_number
}
