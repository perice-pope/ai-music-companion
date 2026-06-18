//! Live perception snapshot — "here's what the app hears."
//!
//! Wraps the [`groove::LiveClock`] (tempo / feel / pulse-lock) and a
//! [`theory::KeyTracker`] (live key) into one thing the UI can show *as you
//! play*, so the adaptive engine stops being a black box. It also names the
//! honest **alternative** key (a G-major reading is also E-minor — the relative)
//! so the UI can ask rather than assert.
//!
//! Lives in `brain` so the Tauri app can drive it without depending on `groove`
//! or `theory` directly (same reason as [`crate::accompaniment`]). Runs on the
//! processing thread — allocation/sorting are fine here.

use ears::AudioEvent;
use groove::LiveClock;
use serde::{Deserialize, Serialize};
use theory::{KeyEstimate, KeyTracker, Mode};

/// Minimum pitch-detection confidence before a frame feeds the key tracker —
/// keeps breath noise and squeaks from skewing the key.
const MIN_PITCH_CONFIDENCE: f64 = 0.5;

/// A pickable key — enough for the UI to both display it and pin the band to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyOption {
    /// Tonic pitch class, 0–11 (C = 0).
    pub tonic: u8,
    /// Whether this option is minor (Aeolian) vs. major (Ionian).
    pub minor: bool,
    /// Full name, e.g. "E minor".
    pub name: String,
}

/// The detected key, named honestly with its relative alternative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySnapshot {
    /// Tonic pitch class, 0–11 (C = 0).
    pub tonic: u8,
    /// Mode label, e.g. "major" / "minor" / "Mixolydian".
    pub mode: String,
    /// Full name, e.g. "G major".
    pub name: String,
    /// Best-fit confidence, 0–1.
    pub confidence: f32,
    /// The relative-key reading the player might actually mean (e.g. "E minor"
    /// for a "G major" call) — structured so the UI can switch the band to it.
    pub alternative: Option<KeyOption>,
}

/// A live snapshot of what the app perceives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerceptionSnapshot {
    /// Live tempo (BPM), or `None` until a pulse is heard.
    pub tempo_bpm: Option<f32>,
    /// Swing ratio (long:short), or `None` when unsure.
    pub swing_ratio: Option<f32>,
    /// True when there's a confident, steady pulse.
    pub locked: bool,
    /// The current key reading, or `None` until enough pitched material.
    pub key: Option<KeySnapshot>,
}

impl PerceptionSnapshot {
    /// Nothing heard yet.
    pub const EMPTY: PerceptionSnapshot = PerceptionSnapshot {
        tempo_bpm: None,
        swing_ratio: None,
        locked: false,
        key: None,
    };
}

/// Builds [`PerceptionSnapshot`]s from the live analysis event stream.
#[derive(Debug, Default)]
pub struct PerceptionTracker {
    clock: LiveClock,
    key: KeyTracker,
    /// EMA-smoothed tempo for *display*, so the readout doesn't jump as the
    /// median IOI steps when onsets enter/leave the window. Does not affect the
    /// band (which runs its own clock).
    smoothed_tempo: Option<f32>,
}

/// EMA weight for the displayed tempo. Low enough to settle the jitter, high
/// enough to still track a real tempo change within a second or two.
const TEMPO_SMOOTHING: f32 = 0.25;

impl PerceptionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one analysis event in: onsets advance the clock, confident pitches
    /// feed the key tracker.
    pub fn observe(&mut self, event: &AudioEvent) {
        if event.is_onset {
            self.clock.observe_onset(event.timestamp_secs);
        }
        if let Some(hz) = event.pitch_hz {
            if event.confidence >= MIN_PITCH_CONFIDENCE && hz > 0.0 {
                self.key.observe_hz(hz as f32, event.confidence as f32);
            }
        }
    }

    /// Snapshot what's perceived as of `now_secs`.
    pub fn snapshot(&mut self, now_secs: f64) -> PerceptionSnapshot {
        let clock = self.clock.tick(now_secs);
        // Smooth the displayed tempo (EMA); drop the smoother when the pulse is
        // lost so a fresh start doesn't drift up from a stale value.
        let tempo_bpm = match clock.tempo_bpm {
            Some(raw) => {
                let next = ema(self.smoothed_tempo, raw, TEMPO_SMOOTHING);
                self.smoothed_tempo = Some(next);
                Some(next)
            }
            None => {
                self.smoothed_tempo = None;
                None
            }
        };
        PerceptionSnapshot {
            tempo_bpm,
            swing_ratio: clock.swing_ratio,
            locked: clock.is_locked(),
            key: self.key.current().map(key_snapshot),
        }
    }

    /// Forget all history (e.g. on a new session).
    pub fn reset(&mut self) {
        self.clock.reset();
        self.key.reset();
        self.smoothed_tempo = None;
    }
}

/// Exponential moving average: ease `prev` toward `raw` by `alpha`. With no
/// prior, start at `raw`.
fn ema(prev: Option<f32>, raw: f32, alpha: f32) -> f32 {
    match prev {
        Some(p) => p + alpha * (raw - p),
        None => raw,
    }
}

/// Build a [`KeySnapshot`] (name + relative alternative) from an estimate.
fn key_snapshot(est: KeyEstimate) -> KeySnapshot {
    let (alt_tonic, alt_mode) = relative_key(est.tonic, est.mode);
    let alternative = KeyOption {
        tonic: alt_tonic,
        minor: alt_mode == Mode::Aeolian,
        name: KeyEstimate {
            tonic: alt_tonic,
            mode: alt_mode,
            confidence: 0.0,
            margin: 0.0,
        }
        .name(),
    };
    KeySnapshot {
        tonic: est.tonic,
        mode: est.mode.label().to_string(),
        name: est.name(),
        confidence: est.confidence,
        alternative: Some(alternative),
    }
}

/// The relative-key reading: relative minor of a major key, relative major of a
/// minor key, and the parent major for the other modes — the most likely thing
/// the player "really means" when the call is ambiguous.
fn relative_key(tonic: u8, mode: Mode) -> (u8, Mode) {
    let down = |semitones: u8| (tonic + 12 - semitones) % 12;
    match mode {
        Mode::Ionian => ((tonic + 9) % 12, Mode::Aeolian), // relative minor (6th degree)
        Mode::Aeolian => ((tonic + 3) % 12, Mode::Ionian), // relative major (♭3)
        // Parent major (the Ionian whose key signature this mode shares).
        Mode::Dorian => (down(2), Mode::Ionian),
        Mode::Phrygian => (down(4), Mode::Ionian),
        Mode::Lydian => (down(5), Mode::Ionian),
        Mode::Mixolydian => (down(7), Mode::Ionian),
        Mode::Locrian => (down(11), Mode::Ionian),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ears::AudioEvent;

    fn onset(t: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: None,
            confidence: 0.0,
            amplitude: 0.5,
            timestamp_secs: t,
            is_onset: true,
            note_info: None,
        }
    }

    fn pitched(hz: f64, t: f64) -> AudioEvent {
        AudioEvent {
            pitch_hz: Some(hz),
            confidence: 0.9,
            amplitude: 0.5,
            timestamp_secs: t,
            is_onset: false,
            note_info: None,
        }
    }

    #[test]
    fn tempo_locks_on_steady_onsets() {
        let mut p = PerceptionTracker::new();
        let mut t = 0.0;
        for _ in 0..8 {
            p.observe(&onset(t));
            t += 0.5; // 120 BPM
        }
        let s = p.snapshot(t);
        let bpm = s.tempo_bpm.expect("tempo should be Some");
        assert!(
            (bpm - 120.0).abs() / 120.0 <= 0.05,
            "expected ~120 BPM, got {bpm}"
        );
        assert!(s.locked, "steady onsets should lock; got {s:?}");
    }

    #[test]
    fn silence_is_unlocked_no_tempo() {
        let mut p = PerceptionTracker::new();
        let s = p.snapshot(0.0);
        assert_eq!(s.tempo_bpm, None);
        assert!(!s.locked);
        assert!(s.key.is_none());
    }

    #[test]
    fn detects_tonic_and_offers_a_relative_alternative() {
        // Emphasize the C tonic (a uniform diatonic scale is genuinely ambiguous
        // across all 7 modes of C, so we lean on the tonic to disambiguate). We
        // assert the snapshot wires through a key whose tonic is C plus a
        // non-empty relative alternative; the exact relative mapping is pinned by
        // the `relative_of_*` unit tests.
        let mut p = PerceptionTracker::new();
        // Full C-major scale (≥4 distinct pitch classes are required before the
        // tracker reports a key) with the tonic triad emphasized so C wins.
        let stream = [
            261.63, 261.63, 329.63, 392.0, 293.66, 349.23, 440.0, 493.88, 261.63,
        ]; // C C E G D F A B C
        let mut t = 0.0;
        for _ in 0..12 {
            for &hz in &stream {
                p.observe(&pitched(hz, t));
                t += 0.1;
            }
        }
        let key = p.snapshot(t).key.expect("a key should be detected");
        assert_eq!(
            key.tonic, 0,
            "tonic-emphasized C should read tonic C; got {key:?}"
        );
        assert!(
            key.name.starts_with('C'),
            "name should be a C key; got {key:?}"
        );
        let alt = key
            .alternative
            .expect("a relative alternative should be offered");
        assert_ne!(
            alt.name, key.name,
            "the alternative must differ from the main reading"
        );
    }

    #[test]
    fn displayed_tempo_is_smoothed() {
        // EMA eases toward the raw reading instead of snapping, so the readout
        // doesn't jump as the median IOI steps.
        assert!(
            (ema(None, 120.0, 0.25) - 120.0).abs() < 1e-3,
            "no prior → raw"
        );
        // From 120 toward 160 by 25% of the 40 BPM gap → 130, not 160.
        assert!((ema(Some(120.0), 160.0, 0.25) - 130.0).abs() < 1e-3);
    }

    #[test]
    fn snapshot_routes_tempo_through_smoother_and_resets_on_silence() {
        // Integration: lock at 120, go silent (tempo → None, smoother reset),
        // then a fresh 90 BPM start must read ~90 — NOT an EMA blend of the stale
        // 120 (which would land ~112 and fail), catching a missing reset.
        let mut p = PerceptionTracker::new();
        let mut t = 0.0;
        for _ in 0..8 {
            p.observe(&onset(t));
            t += 0.5; // 120 BPM
        }
        let locked = p.snapshot(t).tempo_bpm.expect("locked tempo");
        assert!((locked - 120.0).abs() / 120.0 <= 0.05);

        // Long silence ages the window out → no tempo, smoother cleared.
        assert_eq!(p.snapshot(t + 10.0).tempo_bpm, None);

        // Fresh 90 BPM start.
        let mut t2 = t + 20.0;
        for _ in 0..8 {
            p.observe(&onset(t2));
            t2 += 60.0 / 90.0;
        }
        let fresh = p.snapshot(t2).tempo_bpm.expect("fresh tempo");
        assert!(
            (fresh - 90.0).abs() / 90.0 <= 0.05,
            "fresh start must read ~90, not drift from stale 120; got {fresh}"
        );
    }

    #[test]
    fn relative_key_covers_all_seven_modes() {
        // Pin the exact relative/parent reading for every mode (the constants
        // are easy to get off-by-N and can't be eyeballed).
        assert_eq!(relative_key(7, Mode::Ionian), (4, Mode::Aeolian)); // G major → E minor
        assert_eq!(relative_key(9, Mode::Aeolian), (0, Mode::Ionian)); // A minor → C major
        assert_eq!(relative_key(2, Mode::Dorian), (0, Mode::Ionian)); // D Dorian → C
        assert_eq!(relative_key(4, Mode::Phrygian), (0, Mode::Ionian)); // E Phrygian → C
        assert_eq!(relative_key(5, Mode::Lydian), (0, Mode::Ionian)); // F Lydian → C
        assert_eq!(relative_key(7, Mode::Mixolydian), (0, Mode::Ionian)); // G Mixo → C
        assert_eq!(relative_key(11, Mode::Locrian), (0, Mode::Ionian)); // B Locrian → C
    }

    #[test]
    fn key_snapshot_major_offers_relative_minor() {
        // The user's exact example: G major's honest alternative is E minor —
        // pinned end-to-end through key_snapshot (mapping + naming + wiring).
        let s = key_snapshot(KeyEstimate {
            tonic: 7,
            mode: Mode::Ionian,
            confidence: 0.8,
            margin: 0.2,
        });
        assert_eq!(s.name, "G major");
        let alt = s.alternative.expect("alternative");
        assert_eq!(alt.name, "E minor");
        assert_eq!(alt.tonic, 4);
        assert!(alt.minor);
    }

    #[test]
    fn key_snapshot_minor_offers_relative_major() {
        let s = key_snapshot(KeyEstimate {
            tonic: 9,
            mode: Mode::Aeolian,
            confidence: 0.7,
            margin: 0.2,
        });
        assert_eq!(s.name, "A minor");
        let alt = s.alternative.expect("alternative");
        assert_eq!(alt.name, "C major");
        assert_eq!(alt.tonic, 0);
        assert!(!alt.minor);
    }

    #[test]
    fn reset_clears_perception() {
        let mut p = PerceptionTracker::new();
        let mut t = 0.0;
        for _ in 0..8 {
            p.observe(&onset(t));
            t += 0.5;
        }
        assert!(p.snapshot(t).locked);
        p.reset();
        assert_eq!(p.snapshot(t + 0.01), PerceptionSnapshot::EMPTY);
    }
}
