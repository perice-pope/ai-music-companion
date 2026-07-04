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
    /// A challenging key must out-correlate the held key (by `switch_margin`)
    /// for this many CONSECUTIVE observations before taking over. The margin
    /// alone can't stop near-tie mode aliases (E Locrian / G# Locrian / D#
    /// Phrygian read almost identically over wandering material) from
    /// leapfrogging every few notes — the #277 header flapping. Time dwell
    /// kills the flicker; a real modulation sustains dominance and still wins.
    pub switch_dwell: u8,
}

impl Default for KeyTrackerConfig {
    fn default() -> Self {
        Self {
            // ~0.97/note ⇒ the most recent ~30–60 notes carry the estimate,
            // enough context to be stable, short enough to follow a modulation.
            decay: 0.97,
            min_confidence: 0.4,
            // Raised from 0.05 (#277): a challenger must clearly beat the
            // incumbent, not nose ahead.
            switch_margin: 0.1,
            min_pitch_classes: 4,
            switch_dwell: 6,
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
    /// The key currently out-correlating the held one, and for how many
    /// consecutive observations — the dwell counter behind the anti-flap rule.
    challenger: Option<(u8, crate::Mode, u8)>,
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
            challenger: None,
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
        self.challenger = None;
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
                    // Same key — refresh confidence/margin, stand down any
                    // challenger.
                    self.held = Some(candidate);
                    self.challenger = None;
                } else if committable {
                    // Two very different cases share this branch:
                    //  - SAME tonic, different mode — the tracker *refining*
                    //    its read as evidence sharpens (C Mixolydian → C major
                    //    once the leading tone lands). Cheap to accept: small
                    //    margin, short dwell.
                    //  - DIFFERENT tonic — the #277 flapping risk. Must clearly
                    //    beat the incumbent AND sustain it (full margin+dwell);
                    //    near-tie aliases leapfrogging never sustain, a real
                    //    modulation does.
                    let same_tonic = candidate.tonic == held.tonic;
                    let (margin, dwell) = if same_tonic {
                        (self.config.switch_margin * 0.2, 2)
                    } else {
                        (self.config.switch_margin, self.config.switch_dwell)
                    };
                    let held_r = correlation_for(&self.profile, held.tonic, held.mode);
                    if candidate.confidence - held_r > margin {
                        let streak = match self.challenger {
                            Some((t, m, n)) if t == candidate.tonic && m == candidate.mode => {
                                n.saturating_add(1)
                            }
                            _ => 1,
                        };
                        if streak >= dwell {
                            self.held = Some(candidate);
                            self.challenger = None;
                        } else {
                            self.challenger = Some((candidate.tonic, candidate.mode, streak));
                        }
                    } else {
                        self.challenger = None;
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

    /// #277 anti-flap: a challenger that out-correlates the held key must
    /// SUSTAIN that win for `switch_dwell` consecutive observations before the
    /// display flips — a brief excursion (fewer than dwell) leaves the held
    /// key alone, and an interrupted streak resets. Fails if the time-dwell
    /// rule is dropped (margin-only would flip immediately).
    #[test]
    fn a_brief_challenger_does_not_flip_but_a_sustained_one_does() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 8);
        assert_eq!(t.current().unwrap().tonic, 0);

        // A short, strong F#-major burst: challenger streak < dwell → no flip.
        for _ in 0..3 {
            t.observe_pc(6, 3.0);
        }
        assert_eq!(
            t.current().unwrap().tonic,
            0,
            "a brief excursion must not flip the held key"
        );

        // Returning to C-major material stands the challenger down…
        feed_scale(&mut t, 0, Mode::Ionian, 2);
        assert_eq!(t.current().unwrap().tonic, 0);

        // …while a genuinely sustained new key still takes over.
        feed_scale(&mut t, 6, Mode::Ionian, 12);
        assert_eq!(t.current().unwrap().tonic, 6);
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
