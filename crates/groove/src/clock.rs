//! Continuous live clock — a beat/tempo/swing tracker for the accompanist.
//!
//! [`analyze::analyze_groove`] gives a one-shot, per-phrase summary. The
//! follow-me accompaniment instead needs a *continuous* estimate it can read
//! every render block: the current tempo, where we are within the beat, and
//! whether the player is swinging — updated live as onsets arrive.
//!
//! [`LiveClock`] consumes onset timestamps (seconds) on the **processing
//! thread** (never the realtime audio callback — it allocates and sorts) and
//! exposes a [`ClockState`] snapshot via [`LiveClock::tick`].
//!
//! This is the seam `analyze.rs` deliberately left open ("lay-back vs a beat
//! grid"): the clock *is* the beat grid the accompanist plays against.
//!
//! [`analyze::analyze_groove`]: crate::analyze_groove

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::analyze::{ioi_consistency, median_f64, swing_ratio_from_iois};

/// Lowest tempo the clock will lock to, in BPM. Below this we assume the onset
/// spacing is rests/phrases, not a pulse.
const TEMPO_MIN_BPM: f32 = 40.0;
/// Highest tempo the clock will lock to, in BPM. Above this the spacing is a
/// subdivision (e.g. sixteenths) and is folded down an octave.
const TEMPO_MAX_BPM: f32 = 220.0;
/// Most recent onsets kept for estimation. Bounds latency-to-converge on a
/// tempo change (older onsets age out of the window) and caps memory.
const MAX_ONSETS: usize = 12;
/// Onsets older than this (relative to the latest known time) are dropped, so a
/// pause in playing winds the clock down to "no tempo" instead of freezing.
const MAX_AGE_SECS: f64 = 4.0;
/// Onsets needed before the clock will report a locked tempo. Two onsets give a
/// tempo number but not confidence; a short run is required to trust it.
const MIN_LOCK_ONSETS: usize = 4;
/// Confidence at or above which [`ClockState::is_locked`] is true.
const LOCK_CONFIDENCE: f32 = 0.5;

/// A live snapshot of the performer's pulse.
///
/// `tempo_bpm` / `swing_ratio` are `None` when there isn't enough evidence —
/// the clock never fabricates a pulse (same honesty contract as
/// [`GrooveDescriptor`](crate::GrooveDescriptor)).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClockState {
    /// Estimated tempo in BPM, folded into `[TEMPO_MIN_BPM, TEMPO_MAX_BPM]`.
    /// `None` until at least two onsets have been observed.
    pub tempo_bpm: Option<f32>,
    /// Swing ratio (long:short IOI). `≈1.0` straight, `>1.4` swung. `None`
    /// until there are enough onsets to judge (≥4).
    pub swing_ratio: Option<f32>,
    /// Position within the current beat, in `[0.0, 1.0)`. `0.0` when unlocked.
    /// Advances with wall-clock time between onsets and wraps at each beat.
    pub beat_phase: f32,
    /// Confidence the pulse is real, in `[0.0, 1.0]`. Combines how many onsets
    /// are in the window with how steady they are.
    pub confidence: f32,
}

impl ClockState {
    /// An unlocked, silent state — no tempo, no confidence.
    pub const SILENT: ClockState = ClockState {
        tempo_bpm: None,
        swing_ratio: None,
        beat_phase: 0.0,
        confidence: 0.0,
    };

    /// True when the clock has a tempo and enough confidence for the
    /// accompanist to play along.
    pub fn is_locked(&self) -> bool {
        self.tempo_bpm.is_some() && self.confidence >= LOCK_CONFIDENCE
    }
}

/// A continuous tempo/beat/swing tracker.
///
/// Feed it onset timestamps with [`observe_onset`](Self::observe_onset) and read
/// the live estimate with [`tick`](Self::tick). Processing-thread only.
#[derive(Debug, Clone, Default)]
pub struct LiveClock {
    /// Recent onset times (seconds), oldest first, bounded by `MAX_ONSETS`.
    onsets: VecDeque<f64>,
    /// The last tempo we reported, used to resolve octave ambiguity in favour
    /// of continuity (avoid sudden double/half jumps).
    last_tempo_bpm: Option<f32>,
}

impl LiveClock {
    /// Create an empty clock with no pulse.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an onset at `t_secs` (seconds from session start). Onsets older
    /// than `MAX_AGE_SECS` relative to `t_secs`, and any beyond the most recent
    /// `MAX_ONSETS`, are dropped.
    pub fn observe_onset(&mut self, t_secs: f64) {
        // Ignore non-monotonic timestamps (a late/out-of-order event): keeping
        // the window sorted lets us compute IOIs without re-sorting every tick.
        if let Some(&last) = self.onsets.back() {
            if t_secs < last {
                return;
            }
        }
        self.onsets.push_back(t_secs);
        self.prune(t_secs);
    }

    /// Produce the live [`ClockState`] as of `now_secs`. Ages out stale onsets
    /// first, so a long pause yields [`ClockState::SILENT`].
    pub fn tick(&mut self, now_secs: f64) -> ClockState {
        self.prune(now_secs);

        let iois = self.iois();
        let tempo_bpm = self.estimate_tempo(&iois);
        self.last_tempo_bpm = tempo_bpm;

        let swing_ratio = swing_ratio_from_iois(&iois);
        // Confidence describes a *real pulse*: if there's no tempo (too few
        // onsets, or degenerate all-equal timestamps whose IOIs collapse to
        // zero), confidence is zero — never a fabricated high value.
        let confidence = if tempo_bpm.is_some() {
            self.confidence(&iois)
        } else {
            0.0
        };
        let beat_phase = self.beat_phase(now_secs, tempo_bpm, confidence);

        ClockState {
            tempo_bpm,
            swing_ratio,
            beat_phase,
            confidence,
        }
    }

    /// Forget all history. The next `tick` reports [`ClockState::SILENT`].
    pub fn reset(&mut self) {
        self.onsets.clear();
        self.last_tempo_bpm = None;
    }

    // ── internals ──────────────────────────────────────────────────────────

    /// Drop onsets older than `MAX_AGE_SECS` relative to `reference`, then cap
    /// the window to the most recent `MAX_ONSETS`.
    fn prune(&mut self, reference: f64) {
        let cutoff = reference - MAX_AGE_SECS;
        while let Some(&front) = self.onsets.front() {
            if front < cutoff {
                self.onsets.pop_front();
            } else {
                break;
            }
        }
        while self.onsets.len() > MAX_ONSETS {
            self.onsets.pop_front();
        }
    }

    /// Inter-onset intervals across the current window (already time-ordered).
    fn iois(&self) -> Vec<f64> {
        self.onsets
            .iter()
            .zip(self.onsets.iter().skip(1))
            .map(|(a, b)| b - a)
            .collect()
    }

    /// Tempo from the median IOI, folded into the valid BPM band with a
    /// preference for staying near the previously-reported tempo.
    fn estimate_tempo(&self, iois: &[f64]) -> Option<f32> {
        let med = median_f64(iois)?;
        if med < 1e-9 {
            return None;
        }
        Some(fold_tempo((60.0 / med) as f32, self.last_tempo_bpm))
    }

    /// Confidence in `[0, 1]` that there is a real pulse.
    ///
    /// Once the window holds at least `MIN_LOCK_ONSETS` onsets this is the
    /// *pulse regularity* (see [`pulse_regularity`]); below that it is held
    /// strictly under [`LOCK_CONFIDENCE`] so the clock cannot lock on too little
    /// evidence. Erratic timing keeps regularity — and therefore confidence —
    /// low even with many onsets.
    fn confidence(&self, iois: &[f64]) -> f32 {
        let n = self.onsets.len();
        if n < 2 {
            return 0.0;
        }
        let regularity = pulse_regularity(iois);
        if n < MIN_LOCK_ONSETS {
            // Ramp up toward — but never reach — the lock threshold.
            return LOCK_CONFIDENCE * 0.9 * regularity * (n as f32 / MIN_LOCK_ONSETS as f32);
        }
        regularity
    }

    /// Phase within the current beat, in `[0.0, 1.0)`. `0.0` unless locked.
    ///
    /// The grid is **anchored on the most recent onset**: phase is the
    /// fractional number of beats elapsed since it. This deliberately phase-locks
    /// the accompanist to the player — onsets land roughly on beats, so
    /// re-anchoring keeps the band aligned with what the musician just played.
    /// Between onsets phase advances monotonically and wraps each beat; at a new
    /// onset it re-aligns to the player (the "follow me" behaviour).
    ///
    /// `now_secs` is clamped to the anchor so a slightly-stale render clock
    /// (`now < anchor`) can't produce a spurious large phase.
    fn beat_phase(&self, now_secs: f64, tempo_bpm: Option<f32>, confidence: f32) -> f32 {
        let (Some(bpm), Some(&anchor)) = (tempo_bpm, self.onsets.back()) else {
            return 0.0;
        };
        if confidence < LOCK_CONFIDENCE || bpm <= 0.0 {
            return 0.0;
        }
        let period = 60.0 / bpm as f64;
        let elapsed = (now_secs - anchor).max(0.0);
        ((elapsed / period) % 1.0) as f32
    }
}

/// How regular the underlying pulse is, in `[0, 1]` — swing-tolerant.
///
/// Raw IOI consistency would punish a swung feel (alternating long/short IOIs
/// look "inconsistent" even though the pulse is rock-steady). Instead we measure
/// the consistency of **sliding sums of consecutive IOIs**: a swung pair is
/// `long + short`, the next is `short + long` — both equal one beat — so swung
/// and straight playing are both regular, while genuinely erratic timing is not.
///
/// Falls back to raw IOI consistency when there are too few IOIs to form sums.
fn pulse_regularity(iois: &[f64]) -> f32 {
    if iois.len() < 2 {
        return ioi_consistency(iois);
    }
    let beat_sums: Vec<f64> = iois.windows(2).map(|w| w[0] + w[1]).collect();
    ioi_consistency(&beat_sums)
}

/// Fold a raw tempo into `[TEMPO_MIN_BPM, TEMPO_MAX_BPM]`.
///
/// When the raw estimate is **already in band**, it is trusted as-is: the
/// windowed median that produced it is our hysteresis against single noisy
/// onsets, so we must not second-guess sustained evidence. (An earlier version
/// snapped in-band values toward the previous tempo, which made an early
/// half/double misdetection *stick forever* — once it reported, say, 200 BPM, a
/// genuine 100 BPM read folded back up to 200 because that matched the
/// reference. Trusting in-band raw lets the estimate correct itself.)
///
/// Continuity only resolves the octave when folding is **forced** — i.e. the raw
/// spacing is a subdivision/super-beat outside the band (e.g. sixteenths →
/// 600 BPM). Then, among the in-band octaves, we pick the one nearest the
/// previous tempo so subdivision changes don't flip-flop the reported beat.
fn fold_tempo(raw: f32, reference: Option<f32>) -> f32 {
    if (TEMPO_MIN_BPM..=TEMPO_MAX_BPM).contains(&raw) {
        return raw;
    }
    let mut t = raw;
    while t > TEMPO_MAX_BPM {
        t /= 2.0;
    }
    while t < TEMPO_MIN_BPM {
        t *= 2.0;
    }
    let Some(r) = reference else {
        return t;
    };
    // Forced fold: choose the in-band octave of `t` nearest the previous tempo.
    let mut best = t;
    for cand in [t * 0.5, t, t * 2.0] {
        if (TEMPO_MIN_BPM..=TEMPO_MAX_BPM).contains(&cand) && (cand - r).abs() < (best - r).abs() {
            best = cand;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `n` onsets spaced `interval` apart starting at `t0` into `clock`,
    /// returning the time of the last onset.
    fn feed(clock: &mut LiveClock, t0: f64, interval: f64, n: usize) -> f64 {
        let mut t = t0;
        for i in 0..n {
            t = t0 + i as f64 * interval;
            clock.observe_onset(t);
        }
        t
    }

    #[test]
    fn empty_clock_is_silent() {
        let mut clock = LiveClock::new();
        let s = clock.tick(0.0);
        assert_eq!(s, ClockState::SILENT);
        assert!(!s.is_locked());
    }

    #[test]
    fn locks_to_fixed_tempo_within_tolerance() {
        // 120 BPM → 0.5 s IOI. After ≥4 onsets the clock locks within ±3%.
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, 6);
        let s = clock.tick(last);
        let bpm = s.tempo_bpm.expect("tempo should be Some after 6 onsets");
        assert!(
            (bpm - 120.0).abs() / 120.0 <= 0.03,
            "expected ~120 BPM (±3%), got {bpm:.2}"
        );
        assert!(
            s.is_locked(),
            "6 steady onsets should lock; conf={}",
            s.confidence
        );
    }

    #[test]
    fn does_not_lock_before_min_onsets() {
        // The lock boundary is MIN_LOCK_ONSETS (=4): fewer than that must report
        // a tempo but never lock — even with perfectly steady timing. We test
        // n=2 AND n=3 (the n=3 case is the boundary an off-by-one in the
        // confidence ramp would flip).
        for n in [2usize, 3] {
            let mut clock = LiveClock::new();
            let last = feed(&mut clock, 0.0, 0.5, n);
            let s = clock.tick(last);
            assert!(
                s.tempo_bpm.is_some(),
                "tempo should be reported from {n} steady onsets"
            );
            assert!(
                !s.is_locked(),
                "must not lock on {n} onsets (< {MIN_LOCK_ONSETS}); conf {} should be < {LOCK_CONFIDENCE}",
                s.confidence
            );
        }
        // ...and at exactly MIN_LOCK_ONSETS, steady timing *does* lock — proving
        // the boundary is where the constant says, not one off.
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, MIN_LOCK_ONSETS);
        assert!(
            clock.tick(last).is_locked(),
            "exactly {MIN_LOCK_ONSETS} steady onsets should lock"
        );
    }

    #[test]
    fn follows_tempo_change_from_90_to_120() {
        let mut clock = LiveClock::new();
        // Establish 90 BPM (IOI ≈ 0.667 s).
        let mut t = feed(&mut clock, 0.0, 60.0 / 90.0, 8);
        let early = clock.tick(t).tempo_bpm.unwrap();
        assert!(
            (early - 90.0).abs() / 90.0 <= 0.03,
            "should start ~90, got {early}"
        );

        // Now play 120 BPM (IOI = 0.5 s). AC4 requires convergence within a
        // *bounded* number of onsets — assert it has tracked up after only 7 new
        // onsets (well inside the 12-onset window turning fully over), so a
        // regression that converged slowly would fail rather than be masked by
        // waiting a full window.
        let step = 0.5;
        for _ in 0..7 {
            t += step;
            clock.observe_onset(t);
        }
        let converged = clock.tick(t).tempo_bpm.unwrap();
        assert!(
            (converged - 120.0).abs() / 120.0 <= 0.05,
            "expected convergence to ~120 BPM within 7 onsets, got {converged:.2}"
        );
    }

    #[test]
    fn detects_swing() {
        // Triplet swing: long:short = 2:1 over a 0.5 s beat.
        let long = 0.5 * 2.0 / 3.0;
        let short = 0.5 / 3.0;
        let mut clock = LiveClock::new();
        let mut t = 0.0;
        clock.observe_onset(t);
        for _ in 0..8 {
            t += long;
            clock.observe_onset(t);
            t += short;
            clock.observe_onset(t);
        }
        let s = clock.tick(t);
        let sr = s.swing_ratio.expect("swing_ratio should be Some");
        assert!(
            sr > 1.4,
            "swung pattern should report swing > 1.4, got {sr:.2}"
        );
        // A swung groove has a clear pulse and MUST still lock — confidence is
        // measured on beat-level regularity, not raw IOI uniformity.
        assert!(
            s.is_locked(),
            "swung groove should lock (conf {:.2}); the band must swing if you swing",
            s.confidence
        );
    }

    #[test]
    fn erratic_timing_does_not_lock() {
        // Aperiodic onsets with no steady pulse even at the beat level (runs of
        // the same gap make the sliding beat-sums lurch between 0.2 and 2.0), so
        // confidence must stay low and the clock must refuse to lock (vs. the
        // steady case which locks at ~1.0). Note: a *swung* feel would lock —
        // see `detects_swing` — because its beat-sums are regular; this is not.
        let gaps = [0.10, 0.10, 1.00, 1.00, 0.10, 0.10, 1.00, 1.00, 0.10, 0.10];
        let mut clock = LiveClock::new();
        let mut t = 0.0;
        clock.observe_onset(t);
        for g in gaps {
            t += g;
            clock.observe_onset(t);
        }
        let erratic = clock.tick(t);

        let mut steady = LiveClock::new();
        let last = feed(&mut steady, 0.0, 0.5, 11);
        let steady_state = steady.tick(last);

        assert!(
            erratic.confidence < steady_state.confidence,
            "erratic ({:.2}) must be less confident than steady ({:.2})",
            erratic.confidence,
            steady_state.confidence
        );
        assert!(
            !erratic.is_locked(),
            "erratic timing must not lock, got confidence {:.2}",
            erratic.confidence
        );
    }

    #[test]
    fn even_eighths_are_not_swung() {
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.25, 12);
        let sr = clock.tick(last).swing_ratio.expect("swing ratio Some");
        // Noiseless even eighths → ratio is essentially exactly 1.0; a tight
        // bound ensures this can fail for a real swing-detection regression
        // (well clear of the 1.4 swung threshold).
        assert!(
            (sr - 1.0).abs() < 0.05,
            "even eighths should be ≈1.0 (not swung), got {sr:.2}"
        );
    }

    #[test]
    fn beat_phase_advances_and_wraps() {
        // Lock 120 BPM (period 0.5 s) with the last onset as the anchor.
        let mut clock = LiveClock::new();
        let anchor = feed(&mut clock, 0.0, 0.5, 8);

        let p0 = clock.tick(anchor).beat_phase;
        let p_quarter = clock.tick(anchor + 0.125).beat_phase; // 1/4 beat
        let p_half = clock.tick(anchor + 0.25).beat_phase; // 1/2 beat
        let p_three_q = clock.tick(anchor + 0.375).beat_phase; // 3/4 beat
        let p_wrap = clock.tick(anchor + 0.5).beat_phase; // full beat → wraps

        assert!(
            (p0 - 0.0).abs() < 1e-3,
            "phase at anchor should be 0, got {p0}"
        );
        assert!((p_quarter - 0.25).abs() < 1e-3, "got {p_quarter}");
        assert!((p_half - 0.5).abs() < 1e-3, "got {p_half}");
        assert!((p_three_q - 0.75).abs() < 1e-3, "got {p_three_q}");
        // Monotonic increase within the beat.
        assert!(p0 < p_quarter && p_quarter < p_half && p_half < p_three_q);
        // Wraps back near 0 at the beat boundary.
        assert!(
            p_wrap < 0.05,
            "phase should wrap to ~0 at the beat, got {p_wrap}"
        );
    }

    #[test]
    fn beat_phase_is_zero_when_unlocked() {
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, 2); // not enough to lock
        let s = clock.tick(last + 0.3);
        assert!(!s.is_locked());
        assert_eq!(s.beat_phase, 0.0);
    }

    #[test]
    fn silence_winds_down_to_no_tempo() {
        // Lock, then go quiet: after MAX_AGE_SECS all onsets age out → SILENT.
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, 8);
        assert!(clock.tick(last).is_locked());

        let s = clock.tick(last + MAX_AGE_SECS + 1.0);
        assert_eq!(s.tempo_bpm, None, "stale onsets should yield no tempo");
        assert_eq!(s.confidence, 0.0);
        assert!(!s.is_locked());
    }

    #[test]
    fn folds_subdivision_into_tempo_band() {
        // Sixteenth notes at 0.1 s IOI → raw 600 BPM, folded to 150 BPM
        // (600 → 300 → 150), which is within the band.
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.1, 10);
        let bpm = clock.tick(last).tempo_bpm.unwrap();
        assert!(
            (TEMPO_MIN_BPM..=TEMPO_MAX_BPM).contains(&bpm),
            "tempo must be folded into [{TEMPO_MIN_BPM}, {TEMPO_MAX_BPM}], got {bpm}"
        );
        assert!(
            (bpm - 150.0).abs() < 1.0,
            "600 BPM should fold to 150, got {bpm}"
        );
    }

    #[test]
    fn out_of_order_onset_is_ignored() {
        let mut clock = LiveClock::new();
        feed(&mut clock, 0.0, 0.5, 6);
        let before = clock.tick(2.5).tempo_bpm.unwrap();
        let len_before = clock.onsets.len();
        clock.observe_onset(1.0); // earlier than the last onset → ignored
                                  // The onset must be dropped entirely, not just have its value ignored.
        assert_eq!(
            clock.onsets.len(),
            len_before,
            "out-of-order onset must not be pushed into the window"
        );
        let after = clock.tick(2.5).tempo_bpm.unwrap();
        assert!(
            (before - after).abs() < 1e-3,
            "an out-of-order onset must not perturb the estimate"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, 8);
        assert!(clock.tick(last).is_locked());
        clock.reset();
        assert_eq!(clock.tick(last), ClockState::SILENT);
    }

    #[test]
    fn window_is_bounded() {
        // Far more onsets than the window — memory stays bounded.
        let mut clock = LiveClock::new();
        let last = feed(&mut clock, 0.0, 0.5, 100);
        assert!(clock.onsets.len() <= MAX_ONSETS);
        // Still tracks the steady tempo.
        let bpm = clock.tick(last).tempo_bpm.unwrap();
        assert!((bpm - 120.0).abs() < 1.0);
    }

    #[test]
    fn fold_tempo_trusts_in_band_and_resolves_octave_only_when_forced() {
        // In-band raw is trusted as-is, regardless of the reference — this is
        // what stops an early half/double error from sticking.
        assert!((fold_tempo(100.0, Some(200.0)) - 100.0).abs() < 1e-3);
        assert!((fold_tempo(60.0, Some(118.0)) - 60.0).abs() < 1e-3);
        assert!((fold_tempo(60.0, None) - 60.0).abs() < 1e-3);
        // Forced fold (raw out of band): 600 BPM (sixteenths) folds to an in-band
        // octave, and the reference picks WHICH one — 140 → 150 (600→300→150).
        assert!((fold_tempo(600.0, Some(140.0)) - 150.0).abs() < 1e-3);
        // Same raw, reference near 75 → folds down to 75 instead.
        assert!((fold_tempo(600.0, Some(72.0)) - 75.0).abs() < 1e-3);
    }

    #[test]
    fn corrects_an_early_octave_error_once_real_tempo_dominates() {
        // Regression guard for the "sticky octave" bug: if the clock first reads
        // a doubled tempo, a sustained true (half) tempo must pull it back —
        // continuity is hysteresis, not a permanent attractor.
        let mut clock = LiveClock::new();
        // Establish 200 BPM (0.3 s IOI).
        let mut t = feed(&mut clock, 0.0, 0.3, 8);
        let doubled = clock.tick(t).tempo_bpm.unwrap();
        assert!(
            (doubled - 200.0).abs() < 2.0,
            "should start at 200, got {doubled}"
        );

        // Player settles to 100 BPM (0.6 s IOI) — both 100 and its double 200 are
        // in band, so a sticky implementation would stay at 200 forever.
        for _ in 0..12 {
            t += 0.6;
            clock.observe_onset(t);
        }
        let corrected = clock.tick(t).tempo_bpm.unwrap();
        assert!(
            (corrected - 100.0).abs() / 100.0 <= 0.05,
            "tempo must correct down to ~100, not stick at the doubled 200; got {corrected:.1}"
        );
    }

    #[test]
    fn beat_phase_clamps_when_now_precedes_anchor() {
        // A render clock slightly behind the last onset must not yield a spurious
        // large phase (rem_euclid on a negative would): clamp to 0.
        let mut clock = LiveClock::new();
        let anchor = feed(&mut clock, 0.0, 0.5, 8);
        let s = clock.tick(anchor - 0.1);
        assert_eq!(s.beat_phase, 0.0, "phase must clamp to 0 when now < anchor");
    }

    #[test]
    fn degenerate_identical_timestamps_report_no_pulse() {
        // A stuck/duplicated onset source: all IOIs are zero. The clock must not
        // fabricate confidence — tempo None and confidence 0 (not 1.0 from a
        // "perfectly consistent" zero-variance reading).
        let mut clock = LiveClock::new();
        for _ in 0..8 {
            clock.observe_onset(1.0);
        }
        let s = clock.tick(1.0);
        assert_eq!(s.tempo_bpm, None, "identical timestamps have no tempo");
        assert_eq!(
            s.confidence, 0.0,
            "must not report confidence without a pulse"
        );
        assert!(!s.is_locked());
    }
}
