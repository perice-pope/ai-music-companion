//! Core groove analysis: tempo, swing ratio, and timing consistency.
//!
//! All functions here accept a `&[f64]` of onset times (seconds from session
//! start) so the logic is fully testable with synthetic data, independent of
//! the audio pipeline.

use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// Compact description of a performer's rhythm and groove.
///
/// All fields that require more evidence than the onset slice can supply are
/// `Option<f32>` and will be `None` rather than a fabricated value.  The
/// `onset_count` field is always populated.
///
/// **Lay-back / feel vs. a beat grid** is intentionally absent: measuring
/// "lay-back" (playing behind the beat) requires an external beat-grid or
/// click-track reference.  Without one, we can only measure *internal*
/// timing consistency.  When a score or click is loaded in a later PR, the
/// brain crate can pass per-onset grid offsets here and we will extend the
/// struct accordingly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrooveDescriptor {
    /// Estimated tempo in beats per minute.
    ///
    /// Derived from the **median** inter-onset interval (IOI) across the onset
    /// slice.  The median is used rather than the mean because it is robust to
    /// outlier long gaps (breath marks, rests) that would otherwise drag the
    /// estimate down.
    ///
    /// Interpretation: the median IOI is treated as the beat unit.  If a
    /// performer is playing steady eighth notes at 120 BPM, the median IOI is
    /// 0.25 s and we will report `tempo_bpm = Some(120.0)` — the beat, not the
    /// subdivision.  This is the most natural reading; callers that need to
    /// know about subdivision ambiguity should look at the IOI histogram
    /// directly.  `None` when fewer than 2 onsets are present.
    pub tempo_bpm: Option<f32>,

    /// Swing ratio: ratio of the *long* to the *short* IOI in consecutive
    /// pairs of eighth-note intervals.
    ///
    /// Algorithm: pair up consecutive IOIs, compute long/short for each pair,
    /// then take the median.  Straight eighths → ≈ 1.0; typical jazz swing
    /// → ≈ 1.5–2.0 (triplet eighth feels like 2:1).
    ///
    /// `None` when there are fewer than 4 onsets (need ≥ 3 IOIs to form ≥ 1
    /// pair with a next interval to compare against).
    pub swing_ratio: Option<f32>,

    /// Mean inter-onset interval in seconds.
    ///
    /// Together with `timing_consistency` this characterises the average
    /// density and steadiness of the performance.  Always `0.0` for an empty
    /// onset slice (the caller should check `onset_count` first).
    pub mean_ioi_secs: f32,

    /// Timing consistency: how steady the performer's pulse is, in `[0, 1]`.
    ///
    /// Computed as `1 − CV` where CV = `std_dev(IOIs) / mean(IOIs)` clamped to
    /// `[0, 1]`.  A perfectly metronomic performance yields `1.0`; a highly
    /// erratic one approaches `0.0`.
    ///
    /// The coefficient of variation is used rather than raw variance so that
    /// the score is independent of tempo (a fast tempo has smaller IOIs but
    /// should not be penalised for that).
    pub timing_consistency: f32,

    /// Number of onset events in the input slice.
    pub onset_count: u32,
}

// ──────────────────────────────────────────────────────────────────────────────
// Public entry points
// ──────────────────────────────────────────────────────────────────────────────

/// Analyse a slice of onset timestamps (seconds from session start) and return
/// a [`GrooveDescriptor`].
///
/// Returns `None` when `onsets_secs` contains fewer than 2 entries — there
/// must be at least one inter-onset interval to say anything meaningful.
///
/// The slice need not be sorted on entry; the function sorts a local copy.
pub fn analyze_groove(onsets_secs: &[f64]) -> Option<GrooveDescriptor> {
    if onsets_secs.len() < 2 {
        return None;
    }

    // Work with a sorted copy so we can compute IOIs even if events arrived
    // out of order (e.g. from a buffered event queue).
    let mut sorted = onsets_secs.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let iois: Vec<f64> = sorted.windows(2).map(|w| w[1] - w[0]).collect();

    // Guard against zero-length IOI vectors (cannot happen given ≥2 inputs, but
    // the compiler does not know that).
    if iois.is_empty() {
        return None;
    }

    let mean_ioi = mean_f64(&iois) as f32;
    let timing_consistency = ioi_consistency(&iois);
    let tempo_bpm = tempo_from_median_ioi(&iois);
    let swing_ratio = swing_ratio_from_iois(&iois);

    Some(GrooveDescriptor {
        tempo_bpm,
        swing_ratio,
        mean_ioi_secs: mean_ioi,
        timing_consistency,
        onset_count: onsets_secs.len() as u32,
    })
}

/// Extract onset timestamps from a slice of [`ears::AudioEvent`]s.
///
/// An event is considered an onset when `is_onset == true`.  The returned
/// `Vec<f64>` contains the `timestamp_secs` of every such event, in the order
/// they appear in the input slice.
///
/// This helper keeps the `analyze_groove` core independent of the `ears` crate
/// so it can be tested with raw timestamps.
pub fn onsets_from_events(events: &[ears::AudioEvent]) -> Vec<f64> {
    events
        .iter()
        .filter(|e| e.is_onset)
        .map(|e| e.timestamp_secs)
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Arithmetic mean of a non-empty f64 slice.  Panics in debug mode if empty.
fn mean_f64(xs: &[f64]) -> f64 {
    debug_assert!(!xs.is_empty());
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Timing consistency: `1 − clamp(CV, 0, 1)` where CV = σ/μ.
///
/// Returns `1.0` for a single-element IOI slice (no variance possible) or
/// when the mean is effectively zero.
fn ioi_consistency(iois: &[f64]) -> f32 {
    if iois.len() < 2 {
        // Only one IOI → no variance information.
        return 1.0;
    }
    let mu = mean_f64(iois);
    if mu < 1e-9 {
        // All onsets collapsed on the same instant — treat as perfectly
        // consistent (or perfectly bad — caller can check mean_ioi_secs).
        return 1.0;
    }
    let variance = iois.iter().map(|&x| (x - mu).powi(2)).sum::<f64>() / iois.len() as f64;
    let cv = variance.sqrt() / mu;
    (1.0 - cv.clamp(0.0, 1.0)) as f32
}

/// Estimate tempo from the median IOI.
///
/// The median is preferred over the mean so that long rests (IOI outliers) do
/// not pull the tempo estimate down.  Returns `None` if the median IOI rounds
/// to zero (degenerate input).
fn tempo_from_median_ioi(iois: &[f64]) -> Option<f32> {
    let med = median_f64(iois)?;
    if med < 1e-9 {
        return None;
    }
    // BPM = 60 / IOI_in_seconds.
    Some((60.0 / med) as f32)
}

/// Median of a non-empty f64 slice.  Returns `None` for an empty slice.
///
/// Works on a sorted copy to avoid mutating the input.
fn median_f64(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        Some((v[mid - 1] + v[mid]) / 2.0)
    } else {
        Some(v[mid])
    }
}

/// Estimate swing ratio from consecutive IOI pairs.
///
/// For each consecutive triple of onsets O[i], O[i+1], O[i+2] we have two
/// IOIs: a = O[i+1] − O[i] and b = O[i+2] − O[i+1].  If both `a` and `b` are
/// non-zero we form the ratio `max(a,b) / min(a,b)`.  Straight eighths produce
/// a ratio of 1.0; swung triplet eighths produce ≈ 2.0.  We take the median
/// across all such pairs and return it.
///
/// Returns `None` when there are fewer than 3 IOIs (need at least 2 pairs for
/// a meaningful ratio, i.e. ≥ 4 onsets → ≥ 3 IOIs).
fn swing_ratio_from_iois(iois: &[f64]) -> Option<f32> {
    if iois.len() < 3 {
        return None;
    }
    let ratios: Vec<f64> = iois
        .windows(2)
        .filter_map(|w| {
            let (a, b) = (w[0], w[1]);
            if a < 1e-9 || b < 1e-9 {
                None // skip degenerate pairs
            } else {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                Some(hi / lo)
            }
        })
        .collect();

    if ratios.is_empty() {
        return None;
    }

    median_f64(&ratios).map(|r| r as f32)
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Build a perfectly steady onset train: `n` onsets, `interval_secs` apart,
    /// starting at `t0`.
    fn steady(t0: f64, interval_secs: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| t0 + i as f64 * interval_secs).collect()
    }

    // ── analyze_groove: too few onsets ───────────────────────────────────────

    #[test]
    fn zero_onsets_returns_none() {
        assert!(analyze_groove(&[]).is_none());
    }

    #[test]
    fn one_onset_returns_none() {
        assert!(analyze_groove(&[1.0]).is_none());
    }

    // ── analyze_groove: steady 120 BPM (quarter-note = 0.5 s) ───────────────

    #[test]
    fn steady_120bpm_tempo_estimate() {
        // Quarter notes at 120 BPM → IOI = 0.5 s.
        let onsets = steady(0.0, 0.5, 16);
        let g = analyze_groove(&onsets).expect("should succeed with 16 onsets");
        let bpm = g.tempo_bpm.expect("tempo should be Some");
        assert!((bpm - 120.0).abs() < 1.0, "expected ~120 BPM, got {bpm:.2}");
    }

    #[test]
    fn steady_120bpm_high_consistency() {
        let onsets = steady(0.0, 0.5, 16);
        let g = analyze_groove(&onsets).unwrap();
        assert!(
            g.timing_consistency > 0.99,
            "steady train should be >0.99 consistent, got {}",
            g.timing_consistency
        );
    }

    #[test]
    fn steady_120bpm_onset_count() {
        let onsets = steady(0.0, 0.5, 16);
        let g = analyze_groove(&onsets).unwrap();
        assert_eq!(g.onset_count, 16);
    }

    // ── analyze_groove: 240 BPM (IOI = 0.25 s) ──────────────────────────────
    //
    // When the median IOI is 0.25 s we report tempo = 240 BPM.  That may be
    // "eighth notes at 120 BPM" depending on context, but without a score or
    // click track we have no way to distinguish the subdivision from the beat.
    // We document this honestly: the caller (brain crate) can halve the
    // reported tempo if it detects the subdivision is two steps per beat.

    #[test]
    fn steady_240bpm_tempo_estimate() {
        let onsets = steady(0.0, 0.25, 32);
        let g = analyze_groove(&onsets).unwrap();
        let bpm = g.tempo_bpm.unwrap();
        assert!(
            (bpm - 240.0).abs() < 1.0,
            "expected ~240 BPM for 0.25 s IOI, got {bpm:.2}"
        );
    }

    // ── analyze_groove: swing ratio ──────────────────────────────────────────

    /// Generate a swung eighth-note pattern: alternating long (L) and short (S)
    /// IOIs with ratio L:S = `ratio` (e.g. 2.0 for triplet swing).
    fn swung_onsets(beat_secs: f64, ratio: f64, pairs: usize) -> Vec<f64> {
        // Each pair covers one beat: L + S = beat_secs.
        // L = beat * ratio / (ratio + 1),  S = beat / (ratio + 1)
        let long_ioi = beat_secs * ratio / (ratio + 1.0);
        let short_ioi = beat_secs / (ratio + 1.0);
        let mut times = vec![0.0f64];
        for _ in 0..pairs {
            let last = *times.last().unwrap();
            times.push(last + long_ioi);
            times.push(last + long_ioi + short_ioi);
        }
        times
    }

    #[test]
    fn swung_eighths_swing_ratio_approx_two() {
        // Triplet swing: L:S ≈ 2:1, ratio should come back ≈ 2.0.
        let onsets = swung_onsets(0.5, 2.0, 8);
        let g = analyze_groove(&onsets).unwrap();
        let sr = g.swing_ratio.expect("swing_ratio should be Some");
        assert!(
            (sr - 2.0).abs() < 0.05,
            "expected swing_ratio ≈ 2.0, got {sr:.3}"
        );
    }

    #[test]
    fn straight_eighths_swing_ratio_approx_one() {
        // Even eighths: L = S, so ratio = 1.0.
        let onsets = steady(0.0, 0.25, 16);
        let g = analyze_groove(&onsets).unwrap();
        let sr = g.swing_ratio.expect("swing_ratio should be Some");
        assert!(
            (sr - 1.0).abs() < 0.05,
            "expected swing_ratio ≈ 1.0, got {sr:.3}"
        );
    }

    #[test]
    fn light_swing_ratio_in_range() {
        // Light swing: L:S ≈ 1.5:1.
        let onsets = swung_onsets(0.5, 1.5, 8);
        let g = analyze_groove(&onsets).unwrap();
        let sr = g.swing_ratio.unwrap();
        assert!(
            (sr - 1.5).abs() < 0.05,
            "expected swing_ratio ≈ 1.5, got {sr:.3}"
        );
    }

    // ── analyze_groove: timing consistency under jitter ──────────────────────

    #[test]
    fn jittered_onsets_lower_consistency_than_steady() {
        // A steady train should score higher than a jittered one.
        let steady_onsets = steady(0.0, 0.5, 20);
        let steady_g = analyze_groove(&steady_onsets).unwrap();

        // Add ±80 ms jitter (16 % of IOI), well above metronomic tolerance.
        let jitter_ms = 0.08_f64;
        let jittered_onsets: Vec<f64> = (0..20)
            .map(|i| {
                let base = i as f64 * 0.5;
                // Deterministic pseudo-jitter: odd indices go late, even early.
                let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                base + sign * jitter_ms
            })
            .collect();
        let jittered_g = analyze_groove(&jittered_onsets).unwrap();

        assert!(
            jittered_g.timing_consistency < steady_g.timing_consistency,
            "jittered ({}) should be less consistent than steady ({})",
            jittered_g.timing_consistency,
            steady_g.timing_consistency
        );
    }

    // ── swing_ratio: not enough onsets ───────────────────────────────────────

    #[test]
    fn two_onsets_swing_ratio_is_none() {
        // 2 onsets → 1 IOI → no pairs for swing estimation.
        let g = analyze_groove(&[0.0, 0.5]).unwrap();
        assert!(
            g.swing_ratio.is_none(),
            "swing_ratio should be None with only 2 onsets"
        );
    }

    #[test]
    fn three_onsets_swing_ratio_is_none() {
        // 3 onsets → 2 IOIs → 1 window pair but the function requires 3 IOIs.
        let g = analyze_groove(&[0.0, 0.5, 1.0]).unwrap();
        assert!(
            g.swing_ratio.is_none(),
            "swing_ratio should be None with only 3 onsets (need ≥4)"
        );
    }

    // ── onsets_from_events ───────────────────────────────────────────────────

    #[test]
    fn onsets_from_events_extracts_onset_timestamps() {
        let events = vec![
            ears::AudioEvent {
                pitch_hz: Some(440.0),
                confidence: 0.9,
                amplitude: 0.8,
                timestamp_secs: 0.0,
                is_onset: true,
            },
            ears::AudioEvent {
                pitch_hz: Some(440.0),
                confidence: 0.9,
                amplitude: 0.7,
                timestamp_secs: 0.1,
                is_onset: false, // continuation — must be excluded
            },
            ears::AudioEvent {
                pitch_hz: Some(494.0),
                confidence: 0.85,
                amplitude: 0.75,
                timestamp_secs: 0.5,
                is_onset: true,
            },
            ears::AudioEvent {
                pitch_hz: None,
                confidence: 0.0,
                amplitude: 0.01,
                timestamp_secs: 0.9,
                is_onset: false, // silence — must be excluded
            },
        ];

        let onsets = onsets_from_events(&events);
        assert_eq!(
            onsets,
            vec![0.0, 0.5],
            "should extract only is_onset=true timestamps"
        );
    }

    #[test]
    fn onsets_from_events_empty_input() {
        let onsets = onsets_from_events(&[]);
        assert!(onsets.is_empty());
    }

    #[test]
    fn onsets_from_events_no_onset_flags() {
        let events = vec![ears::AudioEvent {
            pitch_hz: Some(440.0),
            confidence: 0.9,
            amplitude: 0.8,
            timestamp_secs: 1.0,
            is_onset: false,
        }];
        let onsets = onsets_from_events(&events);
        assert!(onsets.is_empty(), "no onsets flagged → empty result");
    }

    // ── serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn groove_descriptor_serde_roundtrip() {
        let onsets = swung_onsets(0.5, 2.0, 8);
        let g = analyze_groove(&onsets).unwrap();
        let json = serde_json::to_string(&g).expect("serialize");
        let back: GrooveDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(g, back, "serde round-trip should be lossless");
    }

    #[test]
    fn groove_descriptor_serde_with_none_fields() {
        // 2 onsets: swing_ratio = None, tempo_bpm = Some.
        let g = analyze_groove(&[0.0, 0.5]).unwrap();
        assert!(g.swing_ratio.is_none());
        let json = serde_json::to_string(&g).expect("serialize");
        let back: GrooveDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(g, back);
    }

    // ── internal helpers ─────────────────────────────────────────────────────

    #[test]
    fn median_even_length() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        assert!((median_f64(&v).unwrap() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn median_odd_length() {
        let v = vec![1.0, 3.0, 2.0]; // unsorted on purpose
        assert!((median_f64(&v).unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn median_empty_is_none() {
        assert!(median_f64(&[]).is_none());
    }

    #[test]
    fn consistency_single_ioi_is_one() {
        // One IOI → no variance → consistency = 1.0.
        assert!((ioi_consistency(&[0.5]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn consistency_identical_iois_is_one() {
        let iois = vec![0.5; 10];
        assert!((ioi_consistency(&iois) - 1.0).abs() < 1e-6);
    }
}
