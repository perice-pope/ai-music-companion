//! Rolling key tracker: a decaying pitch-class profile + hysteresis, so a live
//! reading follows modulation without flickering note-to-note.

use crate::key::{correlation_for, estimate_key, KeyEstimate, PitchClassProfile};

/// Tuning for [`KeyTracker`].
#[derive(Debug, Clone, Copy)]
pub struct KeyTrackerConfig {
    /// Per-observation decay applied before each note is added. `< 1.0` makes it
    /// a rolling window (recent notes dominate); `1.0` accumulates forever.
    pub decay: f32,
    /// Minimum confidence to report a *new* key. Below this we keep the last
    /// stable key (if any) rather than guessing.
    pub min_confidence: f32,
    /// A candidate key must beat the currently-held key (on the current
    /// profile) by at least this much to take over — the anti-flicker margin.
    pub switch_margin: f32,
    /// Minimum distinct pitch classes before we'll commit to a key at all —
    /// you can't name a key from one or two notes.
    pub min_pitch_classes: u8,
}

impl Default for KeyTrackerConfig {
    fn default() -> Self {
        Self {
            // ~0.97/note ⇒ the most recent ~30–60 notes carry the estimate,
            // enough context to be stable, short enough to follow a modulation.
            decay: 0.97,
            min_confidence: 0.4,
            switch_margin: 0.05,
            min_pitch_classes: 4,
        }
    }
}

/// Tracks the current key over a stream of detected notes.
///
/// Feed notes with [`observe_hz`](Self::observe_hz) / [`observe_pc`](Self::observe_pc)
/// (e.g. one call per detected note, weighted by duration), then read
/// [`current`](Self::current). The held estimate only changes when a different
/// key clears `switch_margin`, so a passing chromatic note won't flip it.
#[derive(Debug, Clone)]
pub struct KeyTracker {
    config: KeyTrackerConfig,
    profile: PitchClassProfile,
    held: Option<KeyEstimate>,
}

impl Default for KeyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyTracker {
    /// A tracker with default tuning.
    pub fn new() -> Self {
        Self::with_config(KeyTrackerConfig::default())
    }

    /// A tracker with explicit tuning.
    pub fn with_config(config: KeyTrackerConfig) -> Self {
        Self {
            config,
            profile: PitchClassProfile::new(),
            held: None,
        }
    }

    /// Observe a note by frequency (Hz), weighted by e.g. its duration.
    pub fn observe_hz(&mut self, hz: f32, weight: f32) {
        self.profile.decay(self.config.decay);
        self.profile.add_hz(hz, weight);
        self.update();
    }

    /// Observe a note by pitch class (0–11), weighted by e.g. its duration.
    pub fn observe_pc(&mut self, pc: u8, weight: f32) {
        self.profile.decay(self.config.decay);
        self.profile.add_pc(pc, weight);
        self.update();
    }

    /// The current held key, or `None` until enough evidence accumulates.
    pub fn current(&self) -> Option<KeyEstimate> {
        self.held
    }

    /// Forget all accumulated pitch history and the held key.
    pub fn reset(&mut self) {
        self.profile = PitchClassProfile::new();
        self.held = None;
    }

    /// Re-evaluate the held key against the current profile, applying the
    /// confidence floor and the anti-flicker switch margin.
    fn update(&mut self) {
        let Some(candidate) = estimate_key(&self.profile) else {
            return;
        };
        // Enough harmonic evidence and a confident-enough fit to act on.
        let committable = self.profile.distinct() >= self.config.min_pitch_classes as usize
            && candidate.confidence >= self.config.min_confidence;

        match self.held {
            None => {
                if committable {
                    self.held = Some(candidate);
                }
            }
            Some(held) => {
                if held.tonic == candidate.tonic && held.mode == candidate.mode {
                    // Same key — refresh confidence/margin.
                    self.held = Some(candidate);
                } else if committable {
                    // Different key: only switch if it clearly beats the held
                    // key on the *current* profile (anti-flicker hysteresis).
                    let held_r = correlation_for(&self.profile, held.tonic, held.mode);
                    if candidate.confidence - held_r > self.config.switch_margin {
                        self.held = Some(candidate);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mode;

    /// Feed a scale (tonic emphasised) `reps` times into the tracker.
    fn feed_scale(t: &mut KeyTracker, tonic: u8, mode: Mode, reps: usize) {
        for _ in 0..reps {
            for (i, &iv) in mode.intervals().iter().enumerate() {
                let w = if i == 0 { 3.0 } else { 1.0 };
                t.observe_pc(tonic + iv, w);
            }
        }
    }

    #[test]
    fn settles_on_a_clear_key() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 6);
        let est = t.current().expect("should have settled");
        assert_eq!(est.name(), "C major", "got {}", est.name());
    }

    #[test]
    fn stays_quiet_until_confident() {
        // A single note is not enough to commit to a key.
        let mut t = KeyTracker::new();
        t.observe_pc(0, 1.0);
        assert!(t.current().is_none(), "one note should not pin a key");
    }

    #[test]
    fn follows_a_modulation() {
        // Establish C major, then play lots of F# major; the tracker should
        // move off C once the new key dominates the rolling window.
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 6);
        assert_eq!(t.current().unwrap().tonic, 0);

        feed_scale(&mut t, 6, Mode::Ionian, 12);
        let est = t.current().unwrap();
        assert_eq!(
            est.tonic,
            6,
            "should have modulated to F#, got {}",
            est.name()
        );
    }

    #[test]
    fn a_passing_note_does_not_flip_the_key() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 8);
        let before = t.current().unwrap();
        // One out-of-key note (F#) shouldn't change the held key.
        t.observe_pc(6, 1.0);
        let after = t.current().unwrap();
        assert_eq!(
            (before.tonic, before.mode),
            (after.tonic, after.mode),
            "a single chromatic note must not flip the key"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 6);
        assert!(t.current().is_some());
        t.reset();
        assert!(t.current().is_none());
    }
}
