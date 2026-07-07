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

/// A voiced gap longer than this ends the current note.
const NOTE_GAP_SECS: f64 = 0.25;
/// Trailing duration credited past a note's last frame (~one analysis hop,
/// upper bound) — a note seen in one frame still lasted about a hop.
const MAX_FRAME_DT_SECS: f64 = 0.1;
/// Evidence shorter than this isn't a note — it's a single-frame detector
/// glitch (octave errors, attack transients) and must not feed the tracker.
const MIN_NOTE_SECS: f64 = 0.04;
/// Duration-weight cap so one long drone can't own the whole profile.
const MAX_NOTE_WEIGHT_SECS: f64 = 2.0;

/// Segments confident pitch frames into notes.
///
/// The analysis loop produces pitch frames at ~40–50 Hz, but [`KeyTracker`]'s
/// decay and anti-flap dwell are tuned per NOTE ("one call per detected note,
/// weighted by its duration"). Fed raw frames, its ~30–60-note rolling window
/// collapses to under a second of audio and the switch dwell is satisfied
/// inside a single held note — the #313 strip that flapped through a dozen
/// keys per session. This gate merges consecutive same-pitch-class frames
/// into one duration-weighted observation, so the tracker sees what it was
/// tuned for.
#[derive(Debug, Default)]
struct NoteGate {
    pending: Option<PendingNote>,
}

#[derive(Debug, Clone, Copy)]
struct PendingNote {
    pc: u8,
    start_secs: f64,
    last_seen_secs: f64,
}

impl NoteGate {
    /// Fold in one confident pitched frame. Returns the completed note
    /// `(pitch class, duration weight)` when this frame ends the previous one
    /// (pitch-class change, or a re-attack after a voiced gap).
    fn observe(&mut self, pc: u8, t: f64) -> Option<(u8, f32)> {
        match self.pending {
            Some(cur) if cur.pc == pc && t - cur.last_seen_secs <= NOTE_GAP_SECS => {
                self.pending = Some(PendingNote {
                    last_seen_secs: t,
                    ..cur
                });
                None
            }
            prev => {
                self.pending = Some(PendingNote {
                    pc,
                    start_secs: t,
                    last_seen_secs: t,
                });
                prev.and_then(|n| Self::close(n, t))
            }
        }
    }

    /// End the pending note once `now` is clearly past the voiced gap, so a
    /// note followed by silence still lands (snapshots call this ~8 Hz).
    fn flush(&mut self, now: f64) -> Option<(u8, f32)> {
        match self.pending {
            Some(cur) if now - cur.last_seen_secs > NOTE_GAP_SECS => {
                self.pending = None;
                Self::close(cur, now)
            }
            _ => None,
        }
    }

    fn reset(&mut self) {
        self.pending = None;
    }

    /// A note ends when the next one starts (or, bounded by one hop of
    /// trailing credit, when its frames stop). Sub-frame evidence is a glitch,
    /// not a note.
    fn close(note: PendingNote, end_hint: f64) -> Option<(u8, f32)> {
        let end = end_hint.min(note.last_seen_secs + MAX_FRAME_DT_SECS);
        let dur = end - note.start_secs;
        (dur >= MIN_NOTE_SECS).then_some((note.pc, dur.min(MAX_NOTE_WEIGHT_SECS) as f32))
    }
}

/// Nearest equal-tempered pitch class for a frequency (A4 = 440) — the same
/// rounding [`theory::PitchClassProfile::add_hz`] applies.
fn pitch_class_of(hz: f32) -> Option<u8> {
    if hz <= 0.0 {
        return None;
    }
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
    midi.is_finite()
        .then_some((midi as i64).rem_euclid(12) as u8)
}

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
    /// Frame→note segmentation in front of the key tracker (see [`NoteGate`]).
    notes: NoteGate,
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
    /// accumulate into notes that feed the key tracker (one duration-weighted
    /// observation per note, the tracker's contract — see [`NoteGate`]).
    pub fn observe(&mut self, event: &AudioEvent) {
        if event.is_onset {
            self.clock.observe_onset(event.timestamp_secs);
        }
        if let Some(hz) = event.pitch_hz {
            if event.confidence >= MIN_PITCH_CONFIDENCE && hz > 0.0 {
                if let Some(pc) = pitch_class_of(hz as f32) {
                    if let Some((note_pc, weight)) = self.notes.observe(pc, event.timestamp_secs) {
                        self.key.observe_pc(note_pc, weight);
                    }
                }
            }
        }
    }

    /// Snapshot what's perceived as of `now_secs`.
    pub fn snapshot(&mut self, now_secs: f64) -> PerceptionSnapshot {
        // A note that ended into silence lands here (~8 Hz), not only when the
        // next note begins.
        if let Some((pc, weight)) = self.notes.flush(now_secs) {
            self.key.observe_pc(pc, weight);
        }
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
        self.notes.reset();
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

    /// Feed one sustained note as realistic ~45 Hz analysis frames (the rate
    /// the pipeline's detect loop actually produces). Returns the time after
    /// the note.
    fn feed_note_frames(p: &mut PerceptionTracker, hz: f64, start: f64, dur: f64) -> f64 {
        let mut t = start;
        while t < start + dur {
            p.observe(&pitched(hz, t));
            t += 0.022;
        }
        t
    }

    /// C-major establishment material at frame rate: the scale with the tonic
    /// held longest, `passes` times over. Returns the time after the material.
    fn feed_c_major_frames(p: &mut PerceptionTracker, start: f64, passes: usize) -> f64 {
        // C D E F G A B, tonic double-length.
        let scale = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88];
        let mut t = start;
        for _ in 0..passes {
            t = feed_note_frames(p, scale[0], t, 0.5);
            for &hz in &scale[1..] {
                t = feed_note_frames(p, hz, t, 0.25);
            }
        }
        t
    }

    /// The #313 strip flap: at the pipeline's real frame rate, a brief foreign
    /// excursion (under a second — three F#-major notes) must NOT flip the
    /// displayed key, while a genuinely sustained new key still takes over.
    /// Fails when the key tracker is fed per FRAME instead of per note: at
    /// ~45 Hz its rolling window and anti-flap dwell collapse to fractions of
    /// a second, so the sub-second excursion flips the display.
    #[test]
    fn a_brief_excursion_at_frame_rate_does_not_flip_the_live_key() {
        let mut p = PerceptionTracker::new();
        let mut t = feed_c_major_frames(&mut p, 0.0, 4);
        let held = p.snapshot(t).key.expect("C material should read a key");
        assert_eq!(held.tonic, 0, "setup should read a C key; got {held:?}");

        // Three quick F#-major-triad notes — a lick, not a modulation.
        for &hz in &[369.99, 466.16, 554.37] {
            t = feed_note_frames(&mut p, hz, t, 0.25);
        }
        let after = p.snapshot(t).key.expect("a key should still be displayed");
        assert_eq!(
            after.tonic, 0,
            "a sub-second excursion must not flip the strip; got {after:?}"
        );

        // A real modulation — seconds of F#-major material — still wins.
        let fs_scale = [369.99, 415.30, 466.16, 493.88, 554.37, 622.25, 698.46];
        for _ in 0..10 {
            t = feed_note_frames(&mut p, fs_scale[0], t, 0.5);
            for &hz in &fs_scale[1..] {
                t = feed_note_frames(&mut p, hz, t, 0.25);
            }
        }
        let modulated = p.snapshot(t).key.expect("key after modulation");
        assert_eq!(
            modulated.tonic, 6,
            "a sustained new key must still take over; got {modulated:?}"
        );
    }

    /// Single-frame detector glitches (one ~22 ms foreign reading between real
    /// notes) are not notes and must not move the displayed key.
    #[test]
    fn single_frame_glitches_do_not_move_the_key() {
        let mut p = PerceptionTracker::new();
        let mut t = feed_c_major_frames(&mut p, 0.0, 4);
        let held = p.snapshot(t).key.expect("C material should read a key");
        assert_eq!(held.tonic, 0);

        // C-major notes with an isolated F# glitch frame between each.
        for _ in 0..12 {
            p.observe(&pitched(369.99, t));
            t += 0.022;
            t = feed_note_frames(&mut p, 261.63, t, 0.2);
            t = feed_note_frames(&mut p, 329.63, t, 0.2);
        }
        let after = p.snapshot(t).key.expect("key should persist");
        assert_eq!(
            (after.tonic, after.name.clone()),
            (held.tonic, held.name),
            "glitch frames must not move the reading; got {after:?}"
        );
    }

    /// A note followed by silence still lands: the snapshot flushes a pending
    /// note once the voiced gap has clearly passed, so material that ENDS on
    /// the decisive pitch (here the 4th distinct pitch class the tracker needs
    /// before it will commit) is not stuck waiting for a next note.
    #[test]
    fn snapshot_flushes_the_final_note_after_a_gap() {
        let mut p = PerceptionTracker::new();
        // C E G, then B as the final note — 4 distinct pitch classes only once
        // the last note counts.
        let mut t = 0.0;
        for _ in 0..3 {
            for &hz in &[261.63, 329.63, 392.0] {
                t = feed_note_frames(&mut p, hz, t, 0.4);
            }
        }
        let end = feed_note_frames(&mut p, 493.88, t, 0.8);

        // Well past any voiced gap, the held final note must have landed.
        let settled = p.snapshot(end + 1.0).key;
        assert!(
            settled.is_some(),
            "the final held note must land after the gap; got {settled:?}"
        );
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
