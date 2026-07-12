//! ONNX inference: window audio, run basic-pitch, unwrap overlapping frames.
//!
//! Mirrors `basic_pitch/inference.py` (`window_audio_file`, `get_audio_input`,
//! `unwrap_output`). The model is embedded at compile time; ONNX Runtime is
//! loaded dynamically at run time (see crate docs / ci.yml).

use ndarray::Array3;
use ort::session::Session;

use crate::constants::{
    ANNOTATIONS_FPS, ANNOT_N_FRAMES, AUDIO_N_SAMPLES, AUDIO_WINDOW_LENGTH, HOP_SIZE,
    N_FREQ_BINS_NOTES, N_OVERLAPPING_FRAMES, OVERLAP_LEN,
};
use crate::error::TranscribeError;
use crate::notes::Matrix;

/// basic-pitch model (tf2onnx export), Apache-2.0. See `models/README.md`.
static MODEL_BYTES: &[u8] = include_bytes!("../models/nmp.onnx");

const INPUT_NAME: &str = "serving_default_input_2:0";
const OUT_NOTE: &str = "StatefulPartitionedCall:1";
const OUT_ONSET: &str = "StatefulPartitionedCall:2";

/// Frame + onset activation matrices for the whole signal (`22.05 kHz` mono in).
pub struct Activations {
    pub frames: Matrix,
    pub onsets: Matrix,
}

/// Run basic-pitch over mono 22.05 kHz samples and return unwrapped activations.
pub fn infer(samples_22k: &[f32]) -> Result<Activations, TranscribeError> {
    let original_length = samples_22k.len();
    // Empty input needs no model — return before touching ONNX Runtime.
    if original_length == 0 {
        return Ok(Activations {
            frames: Matrix::new(Vec::new(), 0, N_FREQ_BINS_NOTES),
            onsets: Matrix::new(Vec::new(), 0, N_FREQ_BINS_NOTES),
        });
    }

    let mut session = build_session()?;
    infer_with_session(&mut session, samples_22k)
}

/// Build the ONNX session once — the streaming engine (#349 T3a) reuses one
/// session across hops instead of paying model init per inference.
pub(crate) fn build_session() -> Result<Session, TranscribeError> {
    Session::builder()
        .map_err(|e| TranscribeError::Runtime(e.to_string()))?
        .commit_from_memory(MODEL_BYTES)
        .map_err(|e| TranscribeError::Runtime(e.to_string()))
}

/// Run basic-pitch through an EXISTING session (see [`build_session`]).
pub(crate) fn infer_with_session(
    session: &mut Session,
    samples_22k: &[f32],
) -> Result<Activations, TranscribeError> {
    let original_length = samples_22k.len();
    if original_length == 0 {
        return Ok(Activations {
            frames: Matrix::new(Vec::new(), 0, N_FREQ_BINS_NOTES),
            onsets: Matrix::new(Vec::new(), 0, N_FREQ_BINS_NOTES),
        });
    }

    // Front-pad with OVERLAP_LEN/2 zeros, then tile into AUDIO_N_SAMPLES windows
    // stepping by HOP_SIZE; the final window is zero-padded.
    let mut padded = vec![0.0_f32; OVERLAP_LEN / 2 + original_length];
    padded[OVERLAP_LEN / 2..].copy_from_slice(samples_22k);

    let mut windows: Vec<f32> = Vec::new();
    let mut n_windows = 0usize;
    let mut start = 0usize;
    while start < padded.len() {
        let end = (start + AUDIO_N_SAMPLES).min(padded.len());
        windows.extend_from_slice(&padded[start..end]);
        windows.resize(windows.len() + (AUDIO_N_SAMPLES - (end - start)), 0.0);
        n_windows += 1;
        start += HOP_SIZE;
    }

    // [n_windows, AUDIO_N_SAMPLES, 1]
    let input = Array3::from_shape_vec((n_windows, AUDIO_N_SAMPLES, 1), windows)
        .map_err(|e| TranscribeError::Runtime(e.to_string()))?;
    let input = ort::value::Tensor::from_array(input)
        .map_err(|e| TranscribeError::Runtime(e.to_string()))?;

    let outputs = session
        .run(ort::inputs![INPUT_NAME => input])
        .map_err(|e| TranscribeError::Runtime(e.to_string()))?;

    let frames = unwrap_head(&outputs, OUT_NOTE, n_windows, original_length)?;
    let onsets = unwrap_head(&outputs, OUT_ONSET, n_windows, original_length)?;
    Ok(Activations { frames, onsets })
}

/// Extract one `[n_windows, ANNOT_N_FRAMES, 88]` output head and unwrap it into a
/// continuous `[n_times, 88]` matrix (port of `unwrap_output`).
fn unwrap_head(
    outputs: &ort::session::SessionOutputs,
    name: &str,
    n_windows: usize,
    original_length: usize,
) -> Result<Matrix, TranscribeError> {
    let (shape, data) = outputs[name]
        .try_extract_tensor::<f32>()
        .map_err(|e| TranscribeError::Runtime(e.to_string()))?;
    let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
    if dims.len() != 3 || dims[1] != ANNOT_N_FRAMES || dims[2] != N_FREQ_BINS_NOTES {
        return Err(TranscribeError::Runtime(format!(
            "unexpected output shape for {name}: {dims:?}"
        )));
    }
    let nf = N_FREQ_BINS_NOTES;

    // Trim n_olap frames from each window's start and end, then concatenate.
    let n_olap = N_OVERLAPPING_FRAMES / 2; // 15
    let kept_per_window = ANNOT_N_FRAMES - 2 * n_olap; // 142
    let mut unwrapped: Vec<f32> = Vec::with_capacity(n_windows * kept_per_window * nf);
    for w in 0..n_windows {
        let base = w * ANNOT_N_FRAMES * nf;
        for t in n_olap..(ANNOT_N_FRAMES - n_olap) {
            let row = base + t * nf;
            unwrapped.extend_from_slice(&data[row..row + nf]);
        }
    }

    // Truncate to the number of frames the original (unpadded) audio spans.
    // NB: matches the reference's *float* division — `int(n_windows * fpw)` —
    // not integer division, which would drop the tail of the audio.
    let n_frames_per_window = AUDIO_WINDOW_LENGTH * ANNOTATIONS_FPS - N_OVERLAPPING_FRAMES; // 142
    let n_expected =
        ((original_length as f64 / HOP_SIZE as f64) * n_frames_per_window as f64) as usize;
    let n_frames = n_expected.min(unwrapped.len() / nf);
    unwrapped.truncate(n_frames * nf);

    Ok(Matrix::new(unwrapped, n_frames, nf))
}
