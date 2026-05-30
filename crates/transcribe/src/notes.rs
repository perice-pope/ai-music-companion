//! Port of basic-pitch's `note_creation.output_to_notes_polyphonic`.
//!
//! Faithful translation of the Python reference: infer extra onsets from frame
//! deltas, peak-pick onsets, track each note forward through the frame matrix,
//! then run the "melodia trick" over the leftover energy. Index/threshold
//! semantics match upstream so results line up with the Python pipeline.

use crate::constants::{
    DEFAULT_FRAME_THRESH, DEFAULT_MIN_NOTE_LEN, DEFAULT_ONSET_THRESH, ENERGY_TOL, MAX_FREQ_IDX,
    MIDI_OFFSET, N_FREQ_BINS_NOTES,
};

/// A decoded note event in model-frame units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    /// Inclusive start frame index.
    pub start_frame: usize,
    /// Exclusive end frame index.
    pub end_frame: usize,
    /// MIDI note number.
    pub midi: u8,
    /// Mean frame activation over the note's span, in `[0, 1]`.
    pub amplitude: f32,
}

/// Row-major `[n_frames, n_freqs]` activation matrix.
pub struct Matrix {
    pub data: Vec<f32>,
    pub n_frames: usize,
    pub n_freqs: usize,
}

impl Matrix {
    pub fn new(data: Vec<f32>, n_frames: usize, n_freqs: usize) -> Self {
        debug_assert_eq!(data.len(), n_frames * n_freqs);
        Self {
            data,
            n_frames,
            n_freqs,
        }
    }

    #[inline]
    fn idx(&self, t: usize, f: usize) -> usize {
        t * self.n_freqs + f
    }

    #[inline]
    fn get(&self, t: usize, f: usize) -> f32 {
        self.data[self.idx(t, f)]
    }

    #[inline]
    fn set(&mut self, t: usize, f: usize, v: f32) {
        let i = self.idx(t, f);
        self.data[i] = v;
    }
}

/// `get_infered_onsets`: augment predicted onsets with rescaled frame deltas.
fn infer_onsets(onsets: &Matrix, frames: &Matrix, n_diff: usize) -> Vec<f32> {
    let (nt, nf) = (frames.n_frames, frames.n_freqs);
    // frame_diff[t,f] = min over n in 1..=n_diff of (frames[t,f] - frames[t-n,f]),
    // where frames[t-n] is treated as 0 for t < n. Negatives clamped to 0.
    let mut frame_diff = vec![f32::INFINITY; nt * nf];
    for n in 1..=n_diff {
        for t in 0..nt {
            for f in 0..nf {
                let prev = if t < n { 0.0 } else { frames.get(t - n, f) };
                let d = frames.get(t, f) - prev;
                let cell = &mut frame_diff[t * nf + f];
                if d < *cell {
                    *cell = d;
                }
            }
        }
    }
    for v in frame_diff.iter_mut() {
        if *v < 0.0 {
            *v = 0.0;
        }
    }
    // Zero the first n_diff rows (no valid history).
    for t in 0..n_diff.min(nt) {
        for f in 0..nf {
            frame_diff[t * nf + f] = 0.0;
        }
    }

    let max_onset = onsets.data.iter().copied().fold(0.0_f32, f32::max);
    let max_diff = frame_diff.iter().copied().fold(0.0_f32, f32::max);
    // Rescale frame_diff to share the onset matrix's peak (guard against /0 on silence).
    if max_diff > 0.0 {
        let scale = max_onset / max_diff;
        for v in frame_diff.iter_mut() {
            *v *= scale;
        }
    }

    // Element-wise max of predicted onsets and inferred deltas.
    let mut out = vec![0.0_f32; nt * nf];
    for i in 0..nt * nf {
        out[i] = onsets.data[i].max(frame_diff[i]);
    }
    out
}

/// `output_to_notes_polyphonic` with basic-pitch defaults.
pub fn output_to_notes(frames: &Matrix, onsets: &Matrix) -> Vec<NoteEvent> {
    output_to_notes_with(
        frames,
        onsets,
        DEFAULT_ONSET_THRESH,
        DEFAULT_FRAME_THRESH,
        DEFAULT_MIN_NOTE_LEN,
        ENERGY_TOL,
    )
}

fn output_to_notes_with(
    frames: &Matrix,
    onsets: &Matrix,
    onset_thresh: f32,
    frame_thresh: f32,
    min_note_len: usize,
    energy_tol: usize,
) -> Vec<NoteEvent> {
    let n_frames = frames.n_frames;
    let nf = frames.n_freqs;
    debug_assert_eq!(nf, N_FREQ_BINS_NOTES);

    let inferred = infer_onsets(onsets, frames, 2);

    // peak_thresh_mat: keep onset values that are strict local maxima in time.
    // Mirrors scipy.signal.argrelmax(axis=0) followed by a >= threshold mask,
    // collected in row-major (t asc, f asc) order then walked backwards in time.
    let mut peaks: Vec<(usize, usize)> = Vec::new();
    for t in 1..n_frames.saturating_sub(1) {
        for f in 0..nf {
            let v = inferred[t * nf + f];
            if v > inferred[(t - 1) * nf + f] && v > inferred[(t + 1) * nf + f] && v >= onset_thresh
            {
                peaks.push((t, f));
            }
        }
    }
    peaks.reverse();

    // remaining_energy starts as a copy of the frame activations.
    let mut remaining = Matrix::new(frames.data.clone(), n_frames, nf);
    let mut note_events: Vec<NoteEvent> = Vec::new();

    for (note_start, freq_idx) in peaks {
        if note_start >= n_frames - 1 {
            continue;
        }
        // Walk forward until the band stays below threshold for energy_tol frames.
        let mut i = note_start + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < energy_tol {
            if remaining.get(i, freq_idx) < frame_thresh {
                k += 1;
            } else {
                k = 0;
            }
            i += 1;
        }
        i -= k; // back up to the last above-threshold frame

        if i.saturating_sub(note_start) <= min_note_len {
            continue;
        }

        for t in note_start..i {
            remaining.set(t, freq_idx, 0.0);
            if freq_idx < MAX_FREQ_IDX {
                remaining.set(t, freq_idx + 1, 0.0);
            }
            if freq_idx > 0 {
                remaining.set(t, freq_idx - 1, 0.0);
            }
        }

        let amplitude = mean_band(frames, note_start, i, freq_idx);
        note_events.push(NoteEvent {
            start_frame: note_start,
            end_frame: i,
            midi: freq_idx as u8 + MIDI_OFFSET,
            amplitude,
        });
    }

    // Melodia trick: mop up leftover sustained energy with no detected onset.
    melodia_trick(
        frames,
        &mut remaining,
        &mut note_events,
        frame_thresh,
        min_note_len,
        energy_tol,
    );

    note_events
}

fn melodia_trick(
    frames: &Matrix,
    remaining: &mut Matrix,
    note_events: &mut Vec<NoteEvent>,
    frame_thresh: f32,
    min_note_len: usize,
    energy_tol: usize,
) {
    let n_frames = frames.n_frames;
    loop {
        // argmax over the whole remaining-energy matrix.
        let mut best = (0usize, 0usize);
        let mut best_v = f32::NEG_INFINITY;
        for t in 0..n_frames {
            for f in 0..remaining.n_freqs {
                let v = remaining.get(t, f);
                if v > best_v {
                    best_v = v;
                    best = (t, f);
                }
            }
        }
        if best_v <= frame_thresh {
            break;
        }
        let (i_mid, freq_idx) = best;
        remaining.set(i_mid, freq_idx, 0.0);

        // Forward pass.
        let mut i = i_mid + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < energy_tol {
            if remaining.get(i, freq_idx) < frame_thresh {
                k += 1;
            } else {
                k = 0;
            }
            zero_band(remaining, i, freq_idx);
            i += 1;
        }
        let i_end = (i.saturating_sub(1)).saturating_sub(k);

        // Backward pass.
        let mut i_back: isize = i_mid as isize - 1;
        let mut k = 0usize;
        while i_back > 0 && k < energy_tol {
            let ti = i_back as usize;
            if remaining.get(ti, freq_idx) < frame_thresh {
                k += 1;
            } else {
                k = 0;
            }
            zero_band(remaining, ti, freq_idx);
            i_back -= 1;
        }
        let i_start = (i_back + 1 + k as isize).max(0) as usize;

        if i_end.saturating_sub(i_start) <= min_note_len {
            continue;
        }
        let amplitude = mean_band(frames, i_start, i_end, freq_idx);
        note_events.push(NoteEvent {
            start_frame: i_start,
            end_frame: i_end,
            midi: freq_idx as u8 + MIDI_OFFSET,
            amplitude,
        });
    }
}

#[inline]
fn zero_band(m: &mut Matrix, t: usize, freq_idx: usize) {
    m.set(t, freq_idx, 0.0);
    if freq_idx < MAX_FREQ_IDX {
        m.set(t, freq_idx + 1, 0.0);
    }
    if freq_idx > 0 {
        m.set(t, freq_idx - 1, 0.0);
    }
}

#[inline]
fn mean_band(frames: &Matrix, start: usize, end: usize, freq_idx: usize) -> f32 {
    if end <= start {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for t in start..end {
        sum += frames.get(t, freq_idx);
    }
    sum / (end - start) as f32
}
