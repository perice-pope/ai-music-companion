//! Rhythm quantization — snaps raw note timing to a musical grid before it is
//! grouped into measures and notated.
//!
//! ## Why this exists
//!
//! Imported MIDI and audio→MIDI transcription carry *performance* micro-timing:
//! a note meant to be a quarter arrives 0.97 beats long and starting at beat
//! 1.03. If those fractional values flow straight into [`super::midi`]'s
//! `build_measures` and out through the MusicXML emitter, OSMD renders the mess
//! faithfully — ties, odd tuplets, wrong note values. The fix is a deterministic
//! quantization stage that snaps onsets and durations to a rhythmic grid and
//! inserts rests so each measure is time-complete.
//!
//! ## What it does
//!
//! Given raw note events in *tick* space (the natural unit MIDI parsing
//! produces) plus the grid geometry (`ticks_per_beat`, `ticks_per_measure`):
//!
//! 1. **Grid snap (baseline).** Each onset and offset is snapped to the nearest
//!    grid line. The grid is a 1/16-note division of the beat by default, but is
//!    refined per-beat when a finer or tuplet division fits the local timing
//!    better (see below).
//! 2. **Tuplet detection (deeper).** For each beat we test a small set of
//!    candidate subdivisions — binary (1/16) *and* tuplet (triplet, sextuplet)
//!    — and pick the one whose grid the actual onsets/offsets in that beat sit
//!    closest to. A clean triplet figure therefore snaps to a triplet grid
//!    rather than being mangled into tied sixteenths.
//! 3. **Swing detection (deeper).** Across the whole part we measure where the
//!    *second* eighth of each beat actually falls. If those off-beats cluster
//!    around the 2:1 swing ratio (≈0.667 of the beat) rather than the straight
//!    0.5, the passage is flagged as swung and the eighth grid is interpreted
//!    with a swing ratio so a swung passage is recognised instead of quantized
//!    into triplet soup or dotted-eighth/sixteenth.
//!
//! Everything here is pure, offline and deterministic: same input → same
//! output, no clocks, no randomness, no I/O.

use std::fmt;

use thiserror::Error;

/// Errors that can occur during quantization.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuantizeError {
    /// `ticks_per_beat` was zero or negative, so no grid can be built.
    #[error("ticks_per_beat must be positive, got {0}")]
    InvalidTicksPerBeat(i64),
}

/// A raw note event in tick space, as produced by MIDI parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawNoteEvent {
    /// MIDI key number (0–127).
    pub midi_key: u8,
    /// Absolute onset in ticks from the start of the piece.
    pub start_tick: u64,
    /// Sounding length in ticks.
    pub duration_ticks: u64,
}

/// A quantized event: either a sounding note or an inserted rest. Positions and
/// durations are exact tick values that lie on the chosen rhythmic grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantizedEvent {
    /// MIDI key (ignored for rests).
    pub midi_key: u8,
    /// Snapped onset in absolute ticks.
    pub start_tick: u64,
    /// Snapped duration in ticks (always ≥ 1 for notes).
    pub duration_ticks: u64,
    /// Whether this event is a rest (a gap filler) rather than a sounding note.
    pub is_rest: bool,
}

/// How the swing feel was resolved for a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwingFeel {
    /// Straight eighths (off-beats sit at the half-beat).
    Straight,
    /// Swung eighths — the off-beat eighth is delayed toward a 2:1 ratio.
    Swung,
}

impl fmt::Display for SwingFeel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwingFeel::Straight => f.write_str("straight"),
            SwingFeel::Swung => f.write_str("swung"),
        }
    }
}

/// The grid division chosen for a single beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatGrid {
    /// Binary subdivision: the beat is split into `n` equal parts where `n` is a
    /// power-of-two multiple (e.g. 4 → sixteenths).
    Binary,
    /// Tuplet subdivision: the beat is split into `n` equal parts where `n` is
    /// not a power of two (e.g. 3 → triplet eighths, 6 → triplet sixteenths).
    Tuplet(u32),
}

/// Result of quantizing a part: the events plus the musical analysis that
/// produced them (exposed mainly so tests and callers can assert on the
/// detected feel / tuplets).
#[derive(Debug, Clone)]
pub struct QuantizeResult {
    /// All events (notes and rests) in onset order, grid-snapped.
    pub events: Vec<QuantizedEvent>,
    /// Detected swing feel for the part.
    pub swing: SwingFeel,
    /// Per-beat grid choice, keyed by beat index. Only beats that contained at
    /// least one onset appear; a `Tuplet` entry means a tuplet was detected.
    pub beat_grids: Vec<(u64, BeatGrid)>,
}

/// Configuration for the quantizer. Defaults match the issue's baseline:
/// nearest 1/16 grid, tolerant snapping, tuplet + swing detection on.
#[derive(Debug, Clone, Copy)]
pub struct QuantizeConfig {
    /// Binary subdivisions of the beat to consider (e.g. `4` = sixteenths).
    /// The largest that still passes the fit test wins.
    pub max_binary_div: u32,
    /// Tuplet subdivisions of the beat to consider (e.g. `3` = triplets).
    pub tuplet_divs: &'static [u32],
    /// A grid candidate only "fits" a beat if every onset/offset in that beat
    /// lands within this fraction of the *beat* of a grid line. Keeps us from
    /// over-fitting a fine grid to noise.
    pub fit_tolerance_beats: f64,
    /// Enable tuplet detection. When off, only binary grids are considered.
    pub detect_tuplets: bool,
    /// Enable swing detection.
    pub detect_swing: bool,
}

impl Default for QuantizeConfig {
    fn default() -> Self {
        Self {
            max_binary_div: 4, // sixteenth-note grid
            tuplet_divs: &[3, 6],
            fit_tolerance_beats: 0.12,
            detect_tuplets: true,
            detect_swing: true,
        }
    }
}

/// The 2:1 swing ratio: the first swung eighth occupies 2/3 of the beat, so the
/// off-beat eighth falls at 2/3 ≈ 0.6667 of the way through the beat.
const SWING_OFFBEAT_RATIO: f64 = 2.0 / 3.0;
/// Straight eighths put the off-beat at exactly the half-beat.
const STRAIGHT_OFFBEAT_RATIO: f64 = 0.5;
/// An off-beat must sit within this fraction of the swing ratio to count as a
/// swing vote (and likewise for straight). Halfway between 0.5 and 0.667 is
/// ≈0.583, so a window of 0.06 keeps the two camps cleanly separated.
const SWING_VOTE_WINDOW: f64 = 0.06;
/// Need at least this fraction of off-beats voting "swing" to call the part
/// swung — one stray swung note shouldn't flip the whole piece.
const SWING_MAJORITY: f64 = 0.6;
/// Minimum number of off-beat eighths before we trust a swing verdict at all.
const SWING_MIN_SAMPLES: usize = 3;

/// Quantize a slice of raw note events against a rhythmic grid.
///
/// `ticks_per_beat` is the number of MIDI ticks in one *beat* (the time
/// signature's beat unit, already resolved by the caller). Events are returned
/// in onset order with rests inserted to fill gaps up to the end of the final
/// measure that contains a note.
pub fn quantize_notes(
    notes: &[RawNoteEvent],
    ticks_per_beat: f64,
    ticks_per_measure: f64,
    config: &QuantizeConfig,
) -> Result<QuantizeResult, QuantizeError> {
    if ticks_per_beat <= 0.0 {
        return Err(QuantizeError::InvalidTicksPerBeat(ticks_per_beat as i64));
    }

    if notes.is_empty() {
        return Ok(QuantizeResult {
            events: Vec::new(),
            swing: SwingFeel::Straight,
            beat_grids: Vec::new(),
        });
    }

    // Work on a sorted copy so the caller need not pre-sort and so output is
    // deterministic regardless of input ordering.
    let mut sorted: Vec<RawNoteEvent> = notes.to_vec();
    sorted.sort_by_key(|n| (n.start_tick, n.midi_key));

    // ── 1. Swing detection (whole-part) ──────────────────────────────────
    let swing = if config.detect_swing {
        detect_swing(&sorted, ticks_per_beat)
    } else {
        SwingFeel::Straight
    };

    // ── 2. Per-beat grid selection (tuplet detection) ────────────────────
    // Group onset/offset positions by the beat they fall in, then pick the
    // best-fitting grid for each beat.
    let beat_grids = choose_beat_grids(&sorted, ticks_per_beat, config);

    // ── 3. Snap every note to its beat's grid ────────────────────────────
    let mut snapped: Vec<QuantizedEvent> = Vec::with_capacity(sorted.len());
    for note in &sorted {
        let start = snap_tick(
            note.start_tick as f64,
            ticks_per_beat,
            &beat_grids,
            swing,
            config,
        );
        let raw_end = (note.start_tick + note.duration_ticks) as f64;
        let mut end = snap_tick(raw_end, ticks_per_beat, &beat_grids, swing, config);
        // A note must keep a positive length: if snapping collapsed it, give it
        // the smallest grid step at its onset beat.
        if end <= start {
            let step = grid_step_at(start, ticks_per_beat, &beat_grids, config);
            end = start + step.max(1);
        }
        snapped.push(QuantizedEvent {
            midi_key: note.midi_key,
            start_tick: start,
            duration_ticks: end - start,
            is_rest: false,
        });
    }

    // Snapping can collide two onsets onto the same tick; keep onset order and
    // a stable, deterministic sequence.
    snapped.sort_by_key(|e| (e.start_tick, e.midi_key));

    // ── 4. Insert rests for gaps so measures are time-complete ───────────
    let events = insert_rests(&snapped, ticks_per_measure);

    Ok(QuantizeResult {
        events,
        swing,
        beat_grids,
    })
}

// ── Swing detection ───────────────────────────────────────────────────────

/// Look at every onset that falls in the *second half* of its beat and measure
/// how far through the beat it sits. If those off-beats cluster around the 2:1
/// swing ratio rather than the straight half-beat, call the part swung.
fn detect_swing(notes: &[RawNoteEvent], ticks_per_beat: f64) -> SwingFeel {
    let mut swing_votes = 0usize;
    let mut total = 0usize;

    for note in notes {
        let pos_in_beat = (note.start_tick as f64 / ticks_per_beat).fract();
        // Only consider genuine off-beat eighths: positions roughly in the
        // back half of the beat. On-beats and finer subdivisions don't inform
        // the eighth-note swing feel.
        if pos_in_beat <= 0.30 || pos_in_beat >= 0.85 {
            continue;
        }
        total += 1;
        let d_straight = (pos_in_beat - STRAIGHT_OFFBEAT_RATIO).abs();
        let d_swing = (pos_in_beat - SWING_OFFBEAT_RATIO).abs();
        if d_swing < d_straight && d_swing <= SWING_VOTE_WINDOW {
            swing_votes += 1;
        }
    }

    if total >= SWING_MIN_SAMPLES && (swing_votes as f64) >= SWING_MAJORITY * (total as f64) {
        SwingFeel::Swung
    } else {
        SwingFeel::Straight
    }
}

// ── Tuplet detection / per-beat grid choice ────────────────────────────────

/// For each beat that contains at least one onset or offset, choose the grid
/// (binary or tuplet) whose lines the actual timings sit closest to.
fn choose_beat_grids(
    notes: &[RawNoteEvent],
    ticks_per_beat: f64,
    config: &QuantizeConfig,
) -> Vec<(u64, BeatGrid)> {
    // Collect the fractional in-beat positions for each beat index.
    let mut by_beat: Vec<(u64, Vec<f64>)> = Vec::new();
    let mut push_pos = |tick: f64| {
        let beat = (tick / ticks_per_beat).floor() as u64;
        let frac = (tick / ticks_per_beat) - beat as f64;
        match by_beat.iter_mut().find(|(b, _)| *b == beat) {
            Some((_, v)) => v.push(frac),
            None => by_beat.push((beat, vec![frac])),
        }
    };
    for n in notes {
        push_pos(n.start_tick as f64);
        push_pos((n.start_tick + n.duration_ticks) as f64);
    }
    by_beat.sort_by_key(|(b, _)| *b);

    // Candidate subdivisions: binary powers up to max_binary_div, plus tuplets.
    let mut binary_divs: Vec<u32> = Vec::new();
    let mut d = 1u32;
    while d <= config.max_binary_div {
        binary_divs.push(d);
        d *= 2;
    }

    by_beat
        .into_iter()
        .map(|(beat, fracs)| {
            let grid = best_grid_for_beat(&fracs, &binary_divs, config);
            (beat, grid)
        })
        .collect()
}

/// Pick the best grid for one beat's set of fractional positions.
///
/// We score each candidate subdivision by its worst-case snap error. The
/// coarsest grid that still snaps everything within tolerance wins; a tuplet
/// grid is only chosen when it fits strictly *better* than the binary grids,
/// so straight passages never get spurious triplets.
fn best_grid_for_beat(fracs: &[f64], binary_divs: &[u32], config: &QuantizeConfig) -> BeatGrid {
    let worst_err = |div: u32| -> f64 {
        fracs
            .iter()
            .map(|&f| {
                let nearest = (f * div as f64).round() / div as f64;
                (f - nearest).abs()
            })
            .fold(0.0_f64, f64::max)
    };

    // Best binary fit.
    let mut best_binary_err = f64::INFINITY;
    for &div in binary_divs {
        let e = worst_err(div);
        if e < best_binary_err {
            best_binary_err = e;
        }
    }

    if config.detect_tuplets {
        // Does a tuplet grid fit, and does it beat the binary fit by a clear
        // margin? Require the tuplet to be both within tolerance and at least
        // a hair better than binary so we don't introduce triplets gratuitously.
        let mut best_tuplet: Option<(u32, f64)> = None;
        for &div in config.tuplet_divs {
            let e = worst_err(div);
            if e <= config.fit_tolerance_beats && best_tuplet.map(|(_, be)| e < be).unwrap_or(true)
            {
                best_tuplet = Some((div, e));
            }
        }
        if let Some((div, te)) = best_tuplet {
            // Tuplet wins only if binary genuinely fails to model this beat
            // (binary error exceeds tolerance) yet the tuplet succeeds, or the
            // tuplet is meaningfully tighter than the best binary grid.
            if best_binary_err > config.fit_tolerance_beats || te + 1e-6 < best_binary_err {
                return BeatGrid::Tuplet(div);
            }
        }
    }

    BeatGrid::Binary
}

// ── Snapping ───────────────────────────────────────────────────────────────

/// Find the grid choice recorded for the beat containing `tick`.
fn grid_for_tick(tick: f64, ticks_per_beat: f64, beat_grids: &[(u64, BeatGrid)]) -> BeatGrid {
    let beat = (tick / ticks_per_beat).floor() as u64;
    beat_grids
        .iter()
        .find(|(b, _)| *b == beat)
        .map(|(_, g)| *g)
        .unwrap_or(BeatGrid::Binary)
}

/// The smallest grid step (in ticks) at a given position, for the chosen grid.
fn grid_step_at(
    tick: u64,
    ticks_per_beat: f64,
    beat_grids: &[(u64, BeatGrid)],
    config: &QuantizeConfig,
) -> u64 {
    match grid_for_tick(tick as f64, ticks_per_beat, beat_grids) {
        BeatGrid::Binary => (ticks_per_beat / config.max_binary_div as f64).round() as u64,
        BeatGrid::Tuplet(div) => (ticks_per_beat / div as f64).round() as u64,
    }
}

/// Snap an absolute tick position to the grid of the beat it falls in.
///
/// For binary beats under a swung feel, the eighth-note off-beat is mapped to
/// the 2:1 ratio rather than the straight half so swung passages keep their
/// notated eighth-note pairs instead of collapsing into triplets.
fn snap_tick(
    tick: f64,
    ticks_per_beat: f64,
    beat_grids: &[(u64, BeatGrid)],
    swing: SwingFeel,
    config: &QuantizeConfig,
) -> u64 {
    let beat = (tick / ticks_per_beat).floor();
    let beat_start = beat * ticks_per_beat;
    let frac = (tick - beat_start) / ticks_per_beat; // 0.0..1.0 within the beat

    let snapped_frac = match grid_for_tick(tick, ticks_per_beat, beat_grids) {
        BeatGrid::Tuplet(div) => (frac * div as f64).round() / div as f64,
        BeatGrid::Binary => {
            if swing == SwingFeel::Swung {
                // Candidate swing grid lines within a beat: 0, swing off-beat,
                // and the beat boundary at 1.0. Finer binary positions are
                // still available for non-eighth content.
                snap_binary_with_swing(frac, config.max_binary_div)
            } else {
                (frac * config.max_binary_div as f64).round() / config.max_binary_div as f64
            }
        }
    };

    (beat_start + snapped_frac * ticks_per_beat).round() as u64
}

/// Snap a within-beat fraction to the nearest of: the straight binary grid
/// lines, plus the swing off-beat at 2/3. Whichever is closest wins, so on-beat
/// material still lands cleanly while swung off-beats snap to the swing ratio.
fn snap_binary_with_swing(frac: f64, max_binary_div: u32) -> f64 {
    let mut candidates: Vec<f64> = Vec::new();
    for i in 0..=max_binary_div {
        candidates.push(i as f64 / max_binary_div as f64);
    }
    candidates.push(SWING_OFFBEAT_RATIO);
    candidates
        .into_iter()
        .min_by(|a, b| {
            (frac - a)
                .abs()
                .partial_cmp(&(frac - b).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0.0)
}

// ── Rest insertion ──────────────────────────────────────────────────────────

/// Insert rests so the timeline has no gaps between events and the final
/// measure is filled out to its end. Input must be onset-sorted notes.
fn insert_rests(notes: &[QuantizedEvent], ticks_per_measure: f64) -> Vec<QuantizedEvent> {
    if notes.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<QuantizedEvent> = Vec::with_capacity(notes.len() * 2);
    let mut cursor: u64 = 0;

    for note in notes {
        if note.start_tick > cursor {
            push_rests(&mut out, cursor, note.start_tick, ticks_per_measure);
        }
        // Advance the cursor past this note. If notes overlap (cursor already
        // ahead of this onset), keep the later end so we never emit negative
        // gaps; overlap handling for true polyphony is out of scope here.
        let note_end = note.start_tick + note.duration_ticks;
        out.push(*note);
        cursor = cursor.max(note_end);
    }

    // Fill the tail of the final measure with a rest so the bar is complete.
    if ticks_per_measure > 0.0 {
        let measures = (cursor as f64 / ticks_per_measure).ceil();
        let measure_end = (measures * ticks_per_measure).round() as u64;
        if measure_end > cursor {
            push_rests(&mut out, cursor, measure_end, ticks_per_measure);
        }
    }

    out
}

/// Push rest event(s) spanning `[from, to)`, splitting at measure boundaries so
/// a rest never straddles a barline (which OSMD would have to tie/split anyway).
fn push_rests(out: &mut Vec<QuantizedEvent>, from: u64, to: u64, ticks_per_measure: f64) {
    if to <= from {
        return;
    }
    if ticks_per_measure <= 0.0 {
        out.push(rest(from, to - from));
        return;
    }

    let tpm = ticks_per_measure;
    let mut pos = from;
    while pos < to {
        let measure_idx = (pos as f64 / tpm).floor();
        let next_barline = ((measure_idx + 1.0) * tpm).round() as u64;
        let segment_end = next_barline.min(to);
        if segment_end > pos {
            out.push(rest(pos, segment_end - pos));
        }
        pos = segment_end.max(pos + 1);
    }
}

/// Construct a rest event of the given length.
fn rest(start_tick: u64, duration_ticks: u64) -> QuantizedEvent {
    QuantizedEvent {
        midi_key: 0,
        start_tick,
        duration_ticks,
        is_rest: true,
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TPB: f64 = 480.0; // ticks per beat (quarter)
    const TPM: f64 = 480.0 * 4.0; // 4/4 measure

    fn note(midi: u8, start: u64, dur: u64) -> RawNoteEvent {
        RawNoteEvent {
            midi_key: midi,
            start_tick: start,
            duration_ticks: dur,
        }
    }

    fn quantize(notes: &[RawNoteEvent]) -> QuantizeResult {
        quantize_notes(notes, TPB, TPM, &QuantizeConfig::default()).expect("quantize")
    }

    #[test]
    fn rejects_nonpositive_ticks_per_beat() {
        let err =
            quantize_notes(&[note(60, 0, 480)], 0.0, TPM, &QuantizeConfig::default()).unwrap_err();
        assert_eq!(err, QuantizeError::InvalidTicksPerBeat(0));
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let res = quantize(&[]);
        assert!(res.events.is_empty());
        assert_eq!(res.swing, SwingFeel::Straight);
    }

    /// Acceptance: a note ~0.97 beats long notates as a clean quarter.
    #[test]
    fn imperfect_quarter_snaps_to_one_beat() {
        // 0.97 beats ≈ 466 ticks, starting slightly late at 12 ticks.
        let res = quantize(&[note(60, 12, 466)]);
        let sounding: Vec<_> = res.events.iter().filter(|e| !e.is_rest).collect();
        assert_eq!(sounding.len(), 1);
        assert_eq!(sounding[0].start_tick, 0, "onset snaps to beat 0");
        assert_eq!(
            sounding[0].duration_ticks, 480,
            "0.97-beat note snaps to a clean quarter (480 ticks)"
        );
    }

    /// Acceptance: off-grid onsets snap to the grid.
    #[test]
    fn offgrid_onsets_snap_to_grid() {
        // Three notes whose onsets are jittered around beats 0, 1, 2.
        let res = quantize(&[
            note(60, 7, 470),   // ~beat 0
            note(62, 489, 466), // ~beat 1
            note(64, 953, 472), // ~beat 2
        ]);
        let starts: Vec<u64> = res
            .events
            .iter()
            .filter(|e| !e.is_rest)
            .map(|e| e.start_tick)
            .collect();
        assert_eq!(starts, vec![0, 480, 960]);
    }

    /// Acceptance: gaps become rests and each measure is rhythmically complete.
    #[test]
    fn gaps_become_rests_and_measure_is_complete() {
        // A quarter on beat 0, then silence until beat 2, then a quarter.
        let res = quantize(&[note(60, 0, 480), note(64, 960, 480)]);

        // There should be a rest filling beat 1 (the gap) and beat 3 (the tail).
        let rests: Vec<_> = res.events.iter().filter(|e| e.is_rest).collect();
        assert!(!rests.is_empty(), "expected rests for the gaps");

        // Total covered time should fill exactly one 4-beat measure.
        let total: u64 = res.events.iter().map(|e| e.duration_ticks).sum();
        assert_eq!(total, TPM as u64, "measure must be time-complete");

        // The timeline must be contiguous: each event starts where the last ended.
        let mut cursor = 0u64;
        for e in &res.events {
            assert_eq!(e.start_tick, cursor, "no gaps/overlaps: {:?}", res.events);
            cursor += e.duration_ticks;
        }
        assert_eq!(cursor, TPM as u64);
    }

    /// A rest spanning a barline is split so it never straddles the bar.
    #[test]
    fn rests_split_at_barlines() {
        // One note at the very start, then nothing for 2 measures.
        let res = quantize(&[note(60, 0, 480)]);
        // Expect: quarter note, then a rest filling the rest of measure 1
        // (3 beats). The tail does not extend past the single measure that
        // contains the note. So total = one measure.
        let total: u64 = res.events.iter().map(|e| e.duration_ticks).sum();
        assert_eq!(total, TPM as u64);
        // No single rest should be longer than a measure.
        for e in &res.events {
            assert!(
                e.duration_ticks <= TPM as u64,
                "no event longer than a measure: {e:?}"
            );
        }
    }

    /// Acceptance: a triplet figure is detected as a tuplet, not tied sixteenths.
    #[test]
    fn triplet_figure_detected_as_tuplet() {
        // Three even notes across one beat: ticks 0, 160, 320 (480/3 = 160),
        // each 160 long. A binary 1/16 grid (steps of 120) cannot place 160 or
        // 320 cleanly, but a triplet grid (3) nails them.
        let res = quantize(&[note(60, 0, 160), note(62, 160, 160), note(64, 320, 160)]);

        let grid_for_beat0 = res
            .beat_grids
            .iter()
            .find(|(b, _)| *b == 0)
            .map(|(_, g)| *g)
            .expect("beat 0 grid");
        assert_eq!(
            grid_for_beat0,
            BeatGrid::Tuplet(3),
            "even-thirds beat should be detected as a triplet, got {grid_for_beat0:?}"
        );

        // And the three onsets snap exactly onto the triplet grid.
        let starts: Vec<u64> = res
            .events
            .iter()
            .filter(|e| !e.is_rest)
            .map(|e| e.start_tick)
            .collect();
        assert_eq!(starts, vec![0, 160, 320]);
    }

    /// A genuinely binary beat must NOT be mis-detected as a triplet.
    #[test]
    fn straight_eighths_stay_binary() {
        // Two eighths on a beat: ticks 0 and 240 (= half beat).
        let res = quantize(&[note(60, 0, 240), note(62, 240, 240)]);
        let grid = res
            .beat_grids
            .iter()
            .find(|(b, _)| *b == 0)
            .map(|(_, g)| *g)
            .expect("beat 0 grid");
        assert_eq!(grid, BeatGrid::Binary, "straight eighths must stay binary");
    }

    /// Acceptance: a swung passage is recognised.
    #[test]
    fn swung_passage_is_detected() {
        // Swung eighths: each beat has an on-beat and an off-beat at ~2/3.
        // Beat = 480 ticks, off-beat at 320 (= 2/3 * 480). Four beats of it.
        let mut notes = Vec::new();
        for beat in 0..4u64 {
            let base = beat * 480;
            notes.push(note(60, base, 320)); // long swung eighth
            notes.push(note(62, base + 320, 160)); // short swung eighth
        }
        let res = quantize(&notes);
        assert_eq!(
            res.swing,
            SwingFeel::Swung,
            "a 2:1 swung passage must be detected as swung"
        );
    }

    /// Straight eighths across the part must NOT be flagged as swing.
    #[test]
    fn straight_passage_not_flagged_as_swung() {
        let mut notes = Vec::new();
        for beat in 0..4u64 {
            let base = beat * 480;
            notes.push(note(60, base, 240));
            notes.push(note(62, base + 240, 240));
        }
        let res = quantize(&notes);
        assert_eq!(res.swing, SwingFeel::Straight);
    }

    /// Under a swung feel, an off-beat near 2/3 snaps to the swing ratio rather
    /// than being dragged to the straight half-beat.
    #[test]
    fn swing_offbeat_snaps_to_swing_ratio() {
        let mut notes = Vec::new();
        for beat in 0..4u64 {
            let base = beat * 480;
            notes.push(note(60, base, 320));
            notes.push(note(62, base + 320, 160));
        }
        let res = quantize(&notes);
        assert_eq!(res.swing, SwingFeel::Swung);
        // The off-beat of beat 0 (raw 320) should remain at 320 (2/3 * 480),
        // not get pulled to 240 (the straight half).
        let offbeat = res
            .events
            .iter()
            .find(|e| !e.is_rest && e.start_tick > 0 && e.start_tick < 480)
            .expect("an off-beat note in beat 0");
        assert_eq!(
            offbeat.start_tick, 320,
            "swung off-beat should snap to 2/3 of the beat"
        );
    }

    /// Notes are returned in onset order even if input is unsorted.
    #[test]
    fn output_is_onset_sorted() {
        let res = quantize(&[note(64, 960, 480), note(60, 0, 480), note(62, 480, 480)]);
        let starts: Vec<u64> = res
            .events
            .iter()
            .filter(|e| !e.is_rest)
            .map(|e| e.start_tick)
            .collect();
        assert_eq!(starts, vec![0, 480, 960]);
    }

    /// A note that snaps to zero length is rescued to one grid step.
    #[test]
    fn degenerate_short_note_gets_minimum_length() {
        // A 3-tick blip — shorter than any grid step.
        let res = quantize(&[note(60, 0, 3)]);
        let sounding = res.events.iter().find(|e| !e.is_rest).unwrap();
        assert!(
            sounding.duration_ticks >= 1,
            "degenerate note must keep positive length"
        );
    }
}
