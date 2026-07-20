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
    /// Consecutive observations the held key's LIVE fit must fail the commit
    /// bar (`min_confidence` / `min_pitch_classes`) before the reading stops
    /// being settled (#404 finding 2). Key-less material — long-tone warm-ups,
    /// chromatic wandering — keeps an early tentative commit on the strip
    /// forever otherwise; once unsettled, the display's honest state is
    /// "finding the key…", not a name the profile no longer supports. The
    /// streak requirement is the hysteresis: one thin observation never
    /// blanks the display.
    pub unsettle_dwell: u8,
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
            // ~4 notes of sustained sub-bar fit before the name goes quiet —
            // long enough to ride out one odd note, short enough that a
            // warm-up doesn't wear a wrong name for a whole exercise.
            unsettle_dwell: 4,
        }
    }
}

/// Clear-fit observations needed to re-settle after the reading went quiet —
/// shorter than `unsettle_dwell` (real material re-earns its name in a few
/// notes) but more than one, so a fit that hovers at the bar can't flap the
/// display between a name and "finding the key…".
const RESETTLE_DWELL: u8 = 3;

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
    /// Whether the held reading is settled enough to display as a name
    /// (#404 finding 2). Both directions are dwelled (see `bump_settling`),
    /// so the display can't flap between a name and "finding the key…".
    settled: bool,
    /// Consecutive observations pushing AGAINST the current `settled` state:
    /// thin-fit ones while settled, clear-fit ones while unsettled.
    settle_streak: u8,
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
            settled: true,
            settle_streak: 0,
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

    /// Whether the held reading is settled enough to display as a name.
    /// `false` once the held key's LIVE fit has failed the commit bar for
    /// `unsettle_dwell` consecutive observations (#404 finding 2) — key-less
    /// material (long-tone warm-ups, chromatic wandering) keeps whatever the
    /// tracker tentatively committed, and the honest strip state for it is
    /// "finding the key…", not the name. With no held key there is nothing
    /// to disclaim: vacuously `true`.
    pub fn is_settled(&self) -> bool {
        self.held.is_none() || self.settled
    }

    /// Forget all accumulated pitch history and the held key.
    pub fn reset(&mut self) {
        self.profile = PitchClassProfile::new();
        self.held = None;
        self.challenger = None;
        self.settled = true;
        self.settle_streak = 0;
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
                    self.settled = true;
                    self.settle_streak = 0;
                }
            }
            Some(held) => {
                if held.tonic == candidate.tonic && held.mode == candidate.mode {
                    // Same key — refresh confidence/margin, stand down any
                    // challenger.
                    self.held = Some(candidate);
                    self.challenger = None;
                    self.bump_settling(candidate.confidence);
                    return;
                }
                // The top candidate sits elsewhere: the held key's honest fit
                // is its correlation against the CURRENT profile — a frozen
                // commit-time confidence outlives its evidence on wandering
                // material (#404 finding 2). Margin is 0: it is not the best
                // fit right now.
                let held_r = correlation_for(&self.profile, held.tonic, held.mode);
                self.held = Some(KeyEstimate {
                    confidence: held_r.clamp(0.0, 1.0),
                    margin: 0.0,
                    ..held
                });
                self.bump_settling(held_r);
                if committable {
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
                        // Dwell 3, not 2: real vocabulary alternates in PAIRS
                        // (b7/natural-7 blues licks read as Mixolydian/Ionian
                        // pairs), and a dwell of 2 demonstrably flapped on
                        // them. Pairs never sustain 3 consecutive wins; a
                        // genuine refinement does.
                        (self.config.switch_margin * 0.2, 3)
                    } else {
                        (self.config.switch_margin, self.config.switch_dwell)
                    };
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
                            // A switch is a fresh, dwell-earned reading —
                            // settling starts over on the new key.
                            self.settled = true;
                            self.settle_streak = 0;
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

    /// Advance the settling state from the held key's live fit. The bar is
    /// the commit bar — what it takes to earn a display is what it takes to
    /// keep it — and BOTH transitions are dwelled: `unsettle_dwell` thin
    /// observations to go quiet, [`RESETTLE_DWELL`] clear ones to name again,
    /// so the strip can't flap between a name and "finding the key…" when the
    /// fit hovers at the bar.
    fn bump_settling(&mut self, live_fit: f32) {
        let thin = live_fit < self.config.min_confidence
            || self.profile.distinct() < self.config.min_pitch_classes as usize;
        if thin == self.settled {
            // Pushing against the current state — count toward flipping it.
            self.settle_streak = self.settle_streak.saturating_add(1);
            let dwell = if self.settled {
                self.config.unsettle_dwell
            } else {
                RESETTLE_DWELL
            };
            if self.settle_streak >= dwell {
                self.settled = !self.settled;
                self.settle_streak = 0;
            }
        } else {
            self.settle_streak = 0;
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

    /// #277 follow-up (review probe): alternating b7/natural-7 PAIRS — normal
    /// blues/mixolydian vocabulary over one tonic — must not flap the held
    /// mode. With a same-tonic dwell of 2 this flipped 5 times in 10
    /// observations; pairs can never sustain a 3-streak. Fails if the
    /// same-tonic dwell drops below 3.
    #[test]
    fn alternating_seventh_pairs_do_not_flap_the_mode() {
        let mut t = KeyTracker::new();
        // A LIGHT C-major establishment (2 reps), then heavy alternating
        // b7/natural-7 pairs — the volatile-profile case that demonstrably
        // flaps at a same-tonic dwell below 3.
        feed_scale(&mut t, 0, Mode::Ionian, 2);
        let start = t.current().unwrap();
        let mut flips = 0;
        let mut last = (start.tonic, start.mode);
        for _ in 0..8 {
            for pc in [10u8, 10, 11, 11] {
                t.observe_pc(pc, 4.0);
                // Keep some tonal context so the reading stays committable.
                t.observe_pc(0, 1.0);
                let cur = t.current().unwrap();
                if (cur.tonic, cur.mode) != last {
                    flips += 1;
                    last = (cur.tonic, cur.mode);
                }
            }
        }
        assert!(flips <= 1, "seventh-pair vocabulary flapped {flips} times");
    }

    /// Same-tonic refinement stays cheap: material that first reads C
    /// Mixolydian sharpens into C major once the leading tone lands — inside
    /// the theory crate, so the rule isn't guarded only by a brain-crate test.
    #[test]
    fn same_tonic_mode_refinement_is_fast() {
        let mut t = KeyTracker::new();
        // Mixolydian-ish start: C scale with b7 emphasized.
        for _ in 0..4 {
            for &pc in &[0u8, 0, 0, 2, 4, 5, 7, 7, 9, 10] {
                t.observe_pc(pc, 1.0);
            }
        }
        // Now sustained leading-tone (B natural) major material.
        for _ in 0..6 {
            for &pc in &[0u8, 0, 0, 2, 4, 5, 7, 7, 9, 11, 11] {
                t.observe_pc(pc, 1.0);
            }
        }
        let est = t.current().unwrap();
        assert_eq!(est.tonic, 0);
        assert_eq!(est.mode, Mode::Ionian, "got {}", est.name());
    }

    #[test]
    fn reset_clears_state() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 6);
        assert!(t.current().is_some());
        t.reset();
        assert!(t.current().is_none());
        // Note on the settling flag: with `held` cleared, `is_settled()` is
        // vacuously true and every re-commit path resets the flag, so a
        // forgotten flag reset is unobservable — nothing real to assert here.
    }

    /// Chromatic long tones, each held (the VA's quiet warm-up shape, #404
    /// finding 2). The tracker tentatively commits a key a few notes in
    /// (margin ~0) — the display must go UNSETTLED once the window wanders
    /// on, and stay quiet through the rest of the key-less material instead
    /// of wearing a confident-looking wrong name. Fails on code that never
    /// re-examines the held key's fit against the live profile.
    #[test]
    fn keyless_warmup_reads_unsettled_not_a_confident_name() {
        let mut t = KeyTracker::new();
        let mut unsettled_tail = 0;
        for i in 0..24 {
            let pc = ((12 - (i % 12)) % 12) as u8;
            t.observe_pc(pc, 2.0);
            // The second full chromatic pass is unambiguously key-less.
            if i >= 12 && !t.is_settled() {
                unsettled_tail += 1;
            }
        }
        assert!(
            t.current().is_some(),
            "the tracker still holds its tentative estimate internally"
        );
        assert!(
            unsettled_tail >= 10,
            "a chromatic warm-up must read unsettled through its tail; \
             only {unsettled_tail}/12 observations were"
        );
    }

    /// #404 finding 2, the stale-fit half: the held estimate's confidence
    /// must track the LIVE profile. After C major is established, wandering
    /// material that never dethrones C (near-ties don't clear the switch
    /// margin) must still drag the displayed confidence down. Fails on code
    /// that only refreshes confidence when the top candidate is the held key.
    #[test]
    fn held_confidence_tracks_the_live_profile() {
        // Chromatic long tones: the tracker commits a tentative key a few
        // notes in (fit ~0.4, margin ~0), then the window wanders on and the
        // top candidate moves elsewhere while near-ties never clear the
        // switch margin — exactly the case where a frozen commit-time
        // confidence lies to the display.
        let mut t = KeyTracker::new();
        let mut committed: Option<KeyEstimate> = None;
        let mut min_fit = f32::MAX;
        for i in 0..24 {
            let pc = ((12 - (i % 12)) % 12) as u8;
            t.observe_pc(pc, 2.0);
            match (committed, t.current()) {
                (None, Some(est)) => committed = Some(est),
                (Some(first), Some(now)) => {
                    assert_eq!(
                        (now.tonic, now.mode),
                        (first.tonic, first.mode),
                        "near-tie mush must not flip the held identity"
                    );
                    min_fit = min_fit.min(now.confidence);
                }
                _ => {}
            }
        }
        let first = committed.expect("the warm-up shape commits a tentative key");
        assert!(
            min_fit < first.confidence.min(0.2),
            "held confidence must fall with the live profile, not freeze at \
             its commit-time value {:.2}; lowest seen {:.2}",
            first.confidence,
            min_fit
        );
    }

    /// Steady one-key material must never flash "finding the key…": settled
    /// at every observation from first commit on. Guards against the
    /// unsettle rule firing on honest material (the false-quiet failure).
    #[test]
    fn steady_material_never_flashes_unsettled() {
        let mut t = KeyTracker::new();
        let mut committed = false;
        for _ in 0..6 {
            for (i, &iv) in Mode::Ionian.intervals().iter().enumerate() {
                t.observe_pc(iv, if i == 0 { 3.0 } else { 1.0 });
                committed |= t.current().is_some();
                assert!(
                    t.is_settled(),
                    "steady C-major material must stay settled at every note"
                );
            }
        }
        assert!(committed, "the material must actually commit a key");
    }

    /// A sustained modulation still lands and asserts: after C major then
    /// heavy F# major, the held key is F#-tonic and SETTLED — the settling
    /// gate must never leave a real modulation stuck in "finding the key…".
    #[test]
    fn a_modulation_lands_settled() {
        let mut t = KeyTracker::new();
        feed_scale(&mut t, 0, Mode::Ionian, 6);
        feed_scale(&mut t, 6, Mode::Ionian, 12);
        let est = t.current().unwrap();
        assert_eq!(est.tonic, 6, "should end on F#; got {}", est.name());
        assert!(t.is_settled(), "a landed modulation must read settled");
    }

    /// The unsettle hysteresis itself (#404, spec §6): brief thin dips —
    /// fewer than `unsettle_dwell` CONSECUTIVE sub-bar observations — must
    /// never blank the display, however many accumulate across a session,
    /// while a sustained sub-bar stretch goes quiet at exactly the dwell.
    /// The commit bar is raised so light off-key notes read "thin" without
    /// genuinely re-keying the profile — the state machine in isolation.
    /// Fails if the dwell is shortened (a single odd note would flicker the
    /// strip to "finding the key…") or if the consecutiveness reset is
    /// dropped (dips would accumulate forever and go quiet mid-tune).
    #[test]
    fn brief_thin_dips_never_blank_the_display_sustained_thinness_does() {
        let mut t = KeyTracker::with_config(KeyTrackerConfig {
            min_confidence: 0.8,
            ..Default::default()
        });
        feed_scale(&mut t, 0, Mode::Ionian, 8);
        assert!(t.is_settled(), "setup: established C major reads settled");

        // Three rounds of a brief off-key dip (the fit falls under the bar
        // for under-dwell observations) followed by in-key recovery: settled
        // at EVERY observation, even as the dips accumulate.
        for round in 0..3 {
            for _ in 0..3 {
                t.observe_pc(6, 2.0);
                assert!(
                    t.is_settled(),
                    "round {round}: a brief dip must not blank the display"
                );
            }
            for _ in 0..10 {
                for (j, &iv) in Mode::Ionian.intervals().iter().enumerate() {
                    t.observe_pc(iv, if j == 0 { 3.0 } else { 1.0 });
                    assert!(
                        t.is_settled(),
                        "round {round}: recovery material must stay settled"
                    );
                }
            }
        }

        // A SUSTAINED sub-bar stretch: quiet after `unsettle_dwell`
        // consecutive thin observations (the fit falls under 0.8 from the
        // second observation on), and the eventually dwell-earned switch to
        // the new key re-asserts a settled name.
        let mut states = Vec::new();
        for _ in 0..10 {
            t.observe_pc(6, 4.0);
            states.push((t.is_settled(), t.current().unwrap().tonic));
        }
        assert!(
            states[..4].iter().all(|&(s, _)| s),
            "under-dwell thinness must not yet blank the display; got {states:?}"
        );
        assert!(
            !states[4].0 && !states[5].0,
            "the dwell-th consecutive thin observation must unsettle; got {states:?}"
        );
        let last = *states.last().unwrap();
        assert_eq!(
            last,
            (true, 6),
            "a dwell-earned switch must re-assert a settled name; got {states:?}"
        );
    }

    /// The other half of the hysteresis (#404, spec §6): while unsettled,
    /// re-earning the name takes CONSECUTIVE clear observations. Interrupted
    /// recovery — two in-key notes, then a heavy off-key one, repeated — must
    /// stay quiet however many clear observations accumulate, while sustained
    /// in-key material re-earns a settled name. Fails if the streak's
    /// consecutiveness reset is dropped (accumulated clears would flash the
    /// name back mid-wander — the flap this state machine exists to kill).
    #[test]
    fn interrupted_recovery_stays_quiet_sustained_recovery_names_again() {
        let mut t = KeyTracker::new();
        // The chromatic long-tone warm-up until the reading goes quiet.
        let mut i = 0;
        while t.current().is_none() || t.is_settled() {
            t.observe_pc(((12 - (i % 12)) % 12) as u8, 2.0);
            i += 1;
            assert!(i < 60, "the warm-up shape must unsettle the reading");
        }
        let tonic = t.current().unwrap().tonic;

        // Interrupted recovery: the held key's fit clears the bar for two
        // notes at a time (never RESETTLE_DWELL consecutively), so the
        // display must stay at "finding the key…" throughout.
        for cycle in 0..2 {
            for off in [0u8, 7] {
                t.observe_pc((tonic + off) % 12, 2.0);
                assert!(
                    !t.is_settled(),
                    "cycle {cycle}: interrupted recovery must not flash the name back"
                );
            }
            t.observe_pc((tonic + 6) % 12, 5.0);
            assert!(!t.is_settled(), "cycle {cycle}: still wandering");
        }

        // Sustained tonal material re-earns a settled name (whichever key
        // wins the accumulated evidence) within a realistic stretch.
        let mode = t.current().unwrap().mode;
        let mut resettled = false;
        for k in 0..20 {
            let ivs = mode.intervals();
            t.observe_pc(
                (tonic + ivs[k % 7]) % 12,
                if k % 7 == 0 { 3.0 } else { 1.5 },
            );
            if t.is_settled() {
                resettled = true;
                break;
            }
        }
        assert!(
            resettled,
            "sustained tonal material must re-earn a settled name"
        );
    }
}
