//! #349 T2b — chord drills judged by the T1 chord engine.
//!
//! The melodic follower judges lines; a BLOCK-CHORD drill is judged here
//! instead: the drill's [`ChordTarget`]s (what the T1 engine should hear,
//! straight from `variations::generate`) against the stable chord readings
//! the perception tracker actually promoted. Verdicts ride the same
//! [`NoteVerdict`] shape as melodic judging, so the strip, phrase cards,
//! and recap plumbing stay unchanged.
//!
//! Verdict rules (spec §6 T2):
//! - **Hit** — same root and quality; when the drill DEMANDS an inversion
//!   (`target.bass_pc = Some`), the heard bass must match too.
//! - **Near** — right root, wrong quality (C7 played for Cmaj7), or the
//!   right chord with the demanded bass wrong/missing. (Broader than the
//!   spec's "wrong extension": ANY right-root quality earns Near, because
//!   a quiet 7th genuinely reads as a triad at the detector — grading that
//!   as a full miss would punish the microphone, not the player.)
//! - **Missed** — the cell was never matched by anything the player did.
//!
//! **Settle-on-evidence (the follower's "silence beats false accusations"
//! discipline):** a reading settles the current target only when it MATCHES
//! it (Hit/Near), or when it is a clean HIT on the next target (skip
//! credit: the fumbled cell closes Missed, the nailed one credits Hit — an
//! exact next-hit also outranks a Near on current). Anything else — a
//! noodle, a stray wrong chord, detector mush — settles NOTHING: it can't
//! accuse the player of missing a cell they may be about to play. Cells
//! that were truly never played become honest Misses at [`finish`].
//! Consequence pinned by tests: noodle one wrong chord, then play the
//! whole drill perfectly → perfect score, not a cascade of misses.
//!
//! Pure and deterministic: no clock, no audio. **Session contract:** feed
//! it chord CHANGES (edge-triggered, exactly as the session's chord buffer
//! records them — one entry per promoted identity change, with a release
//! reset so a deliberate re-strike of the same chord arrives again). The
//! judge itself does not dedup: two identical consecutive entries mean the
//! player really struck the chord twice.
//!
//! [`finish`]: ChordDrillJudge::finish

use serde::{Deserialize, Serialize};

use crate::follower::{NoteVerdict, Verdict};
use theory::ChordQuality;
use variations::ChordTarget;

/// What the session layer heard — a stable promoted chord reading, reduced
/// to the fields grading needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeardChord {
    pub root_pc: u8,
    pub quality: ChordQuality,
    /// The sounding bass pitch class, when one was confidently named.
    pub bass_pc: Option<u8>,
}

/// How a heard chord grades against one target. The `% 12` normalizations
/// are defensive — both sides are documented 0–11.
fn classify(heard: &HeardChord, target: &ChordTarget) -> Verdict {
    if heard.root_pc % 12 != target.root_pc % 12 {
        return Verdict::Missed;
    }
    if heard.quality != target.quality {
        return Verdict::Near; // right root, wrong quality/extension
    }
    match target.bass_pc {
        // Inversion demanded: the heard bass must name it.
        Some(want) => match heard.bass_pc {
            Some(b) if b % 12 == want % 12 => Verdict::Hit,
            _ => Verdict::Near, // right chord, demanded bass wrong/missing
        },
        None => Verdict::Hit, // bass ignored unless the drill demands it
    }
}

/// How many cells ahead a clean Hit can rescue alignment (the melodic
/// follower's MAX_MISS_RUN discipline): dropping two cells then playing on
/// settles the dropped ones Missed and credits the rest — never a zeroed
/// remainder.
const MAX_SKIP_WINDOW: usize = 4;

/// Sequential settle-on-evidence judge over a stacked drill's targets.
#[derive(Debug, Clone)]
pub struct ChordDrillJudge {
    targets: Vec<ChordTarget>,
    /// Next unjudged target index.
    next: usize,
}

impl ChordDrillJudge {
    pub fn new(targets: Vec<ChordTarget>) -> Self {
        Self { targets, next: 0 }
    }

    /// True once every target has been judged.
    pub fn is_done(&self) -> bool {
        self.next >= self.targets.len()
    }

    /// Fold in one heard chord CHANGE (see the module docs for the session
    /// contract). Returns the verdicts it settles: one on a match, two when
    /// skip credit closes a fumbled cell — and NONE for a reading that
    /// matches neither the current nor the next target (held, never an
    /// accusation).
    pub fn observe(&mut self, heard: HeardChord) -> Vec<NoteVerdict> {
        if self.is_done() {
            return Vec::new();
        }
        let on_current = classify(&heard, &self.targets[self.next]);
        // A clean Hit within the skip window means the player has MOVED ON —
        // it outranks anything short of a Hit on the current cell; holding
        // the fumbled cells open would misattribute everything after them.
        let window_hit = (self.next + 1..=self.next + MAX_SKIP_WINDOW)
            .take_while(|&i| i < self.targets.len())
            .find(|&i| classify(&heard, &self.targets[i]) == Verdict::Hit);
        match (on_current, window_hit) {
            (Verdict::Hit, _) => vec![self.settle(Verdict::Hit)],
            (_, Some(hit_at)) => {
                let mut out = Vec::new();
                while self.next < hit_at {
                    out.push(self.settle(Verdict::Missed));
                }
                out.push(self.settle(Verdict::Hit));
                out
            }
            (Verdict::Near, None) => vec![self.settle(Verdict::Near)],
            // No evidence about the current cell: hold. finish() will close
            // it honestly if nothing ever matches.
            (Verdict::Missed, None) => Vec::new(),
        }
    }

    /// Close the drill: every unjudged target is an honest Missed — a cell
    /// nobody played is not a cell survived.
    pub fn finish(&mut self) -> Vec<NoteVerdict> {
        let mut out = Vec::new();
        while !self.is_done() {
            out.push(self.settle(Verdict::Missed));
        }
        out
    }

    /// Judged / total, for progress display.
    pub fn progress(&self) -> (usize, usize) {
        (self.next, self.targets.len())
    }

    fn settle(&mut self, verdict: Verdict) -> NoteVerdict {
        let target = &self.targets[self.next];
        self.next += 1;
        NoteVerdict {
            // One stacked cell per measure (RV grid): segment k = measure
            // k+1, judged on its downbeat.
            measure_number: target.segment as usize + 1,
            beat: 0.0,
            verdict,
        }
    }
}

/// The session's chord-evidence recorder (#349 T2b), extracted from the
/// audio-worker closure so the edge-trigger contract the judge depends on
/// is TESTED, not glue. Feed it the tracker's current stable chord every
/// reading tick:
/// - a NEW (root, quality) identity appends an entry;
/// - the SAME identity refreshes the last entry's slash in place (a late
///   bass correction on an inversion drill must be able to upgrade the
///   cell — judging is submit-time, so in-place is safe) — and never
///   inflates the count the timing proxy reads;
/// - `None` (label cleared) arms a release, so a deliberate re-strike of
///   the same chord lands as a fresh entry.
#[derive(Debug, Default)]
pub struct ChordChangeBuffer {
    entries: Vec<HeardChord>,
    /// The last entry is still ringing (no release since it was pushed).
    live: bool,
}

impl ChordChangeBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in one reading tick's promoted chord (or its absence).
    pub fn observe(&mut self, current: Option<HeardChord>) {
        let Some(h) = current else {
            self.live = false;
            return;
        };
        let sustains = self.live
            && self
                .entries
                .last()
                .is_some_and(|l| l.root_pc == h.root_pc && l.quality == h.quality);
        if sustains {
            // Slash refresh: the freshest NAMED bass wins; a bass that
            // merely faded to unnamed must not erase a real reading.
            if h.bass_pc.is_some() {
                if let Some(last) = self.entries.last_mut() {
                    last.bass_pc = h.bass_pc;
                }
            }
        } else {
            self.entries.push(h);
            self.live = true;
        }
    }

    /// Everything heard so far, in change order.
    pub fn heard(&self) -> &[HeardChord] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forget everything (new session).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.live = false;
    }
}

/// Tally a drill into the lesson's 0..1 accuracy signal: Hit = 1, Near = ½
/// (right root is real knowledge — the RV ramp shouldn't treat
/// C7-for-Cmaj7 like silence), Missed = 0. Divides by `total_targets`, not
/// by verdict count, so a caller that forgets [`ChordDrillJudge::finish`]
/// under-counts instead of silently inflating a one-hit drill to 100%.
pub fn chord_drill_accuracy(verdicts: &[NoteVerdict], total_targets: usize) -> f32 {
    if total_targets == 0 {
        return 0.0;
    }
    let score: f32 = verdicts
        .iter()
        .map(|v| match v.verdict {
            Verdict::Hit => 1.0,
            Verdict::Near => 0.5,
            Verdict::Missed => 0.0,
        })
        .sum();
    score / total_targets as f32
}

/// Grade a finished chord drill for the lesson ladder (#349 T2b): run the
/// buffered chord changes through the judge, close unplayed cells as
/// Misses, and shape the result as the [`DrillScore`] the ramp/recap/log
/// already consume. `per_note.correct` = Hit (a Near still shows as a miss
/// in the per-cell view but earns its half credit in `accuracy`).
///
/// [`DrillScore`]: crate::coach::DrillScore
pub fn score_chord_drill(
    targets: &[ChordTarget],
    heard: &[HeardChord],
) -> crate::coach::DrillScore {
    let mut judge = ChordDrillJudge::new(targets.to_vec());
    let mut verdicts = Vec::new();
    for &h in heard {
        verdicts.extend(judge.observe(h));
    }
    verdicts.extend(judge.finish());
    let accuracy = chord_drill_accuracy(&verdicts, targets.len());
    // Verdicts settle strictly in target order, so zip attributes correctly.
    let per_note = targets
        .iter()
        .zip(&verdicts)
        .map(|(t, v)| crate::coach::NoteGrade {
            // Representative pitch for display: the expected root near C4.
            target_midi: 60 + (t.root_pc % 12),
            played_hz: None,
            cents_deviation: None,
            correct: v.verdict == Verdict::Hit,
        })
        .collect();
    // Steadiness proxy, mirroring the melodic drill's onset-count proxy:
    // how closely the number of distinct chords heard tracks the number of
    // cells asked for (noodling many chords or freezing both read as loose).
    let timing_accuracy = if targets.is_empty() {
        0.0
    } else {
        let heard_n = heard.len() as f32;
        let want_n = targets.len() as f32;
        (heard_n.min(want_n) / heard_n.max(want_n)).clamp(0.0, 1.0)
    };
    crate::coach::DrillScore {
        per_note,
        accuracy,
        pitch_accuracy: accuracy,
        timing_accuracy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(segment: u32, root_pc: u8, quality: ChordQuality) -> ChordTarget {
        ChordTarget {
            segment,
            root_pc,
            quality,
            bass_pc: None,
        }
    }

    fn heard(root_pc: u8, quality: ChordQuality) -> HeardChord {
        HeardChord {
            root_pc,
            quality,
            bass_pc: None,
        }
    }

    /// #349 T2 AC2, the canonical ladder: right chord → Hit; C7 for Cmaj7
    /// → Near (right root, wrong extension); a wrong-root chord (Am for C)
    /// settles nothing live and the never-matched cell closes Missed.
    #[test]
    fn hit_near_missed_follow_the_spec_ladder() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Maj7),
            target(1, 0, ChordQuality::Maj7),
            target(2, 0, ChordQuality::Maj),
        ]);
        let v = j.observe(heard(0, ChordQuality::Maj7));
        assert_eq!((v[0].measure_number, v[0].verdict), (1, Verdict::Hit));
        let v = j.observe(heard(0, ChordQuality::Dom7)); // C7 for Cmaj7
        assert_eq!(v[0].verdict, Verdict::Near);
        // Am for C: no accusation live — the honest Missed lands at finish.
        assert!(j.observe(heard(9, ChordQuality::Min)).is_empty());
        let v = j.finish();
        assert_eq!((v[0].measure_number, v[0].verdict), (3, Verdict::Missed));
        assert!(j.is_done());
    }

    /// THE review-round probe, now the spec: noodle one wrong chord, then
    /// play the whole drill perfectly → perfect score. Fails if the judge
    /// ever goes back to consuming a target on a non-matching reading (the
    /// cascade that graded a near-perfect run 0.0).
    #[test]
    fn a_noodle_before_a_perfect_run_costs_nothing() {
        let targets = vec![
            target(0, 0, ChordQuality::Dom7),
            target(1, 5, ChordQuality::Dom7),
            target(2, 10, ChordQuality::Dom7),
        ];
        let mut j = ChordDrillJudge::new(targets.clone());
        assert!(j.observe(heard(9, ChordQuality::Min)).is_empty()); // noodle
        let mut verdicts = Vec::new();
        for t in &targets {
            verdicts.extend(j.observe(heard(t.root_pc, t.quality)));
        }
        verdicts.extend(j.finish());
        assert!(
            verdicts.iter().all(|v| v.verdict == Verdict::Hit),
            "noodle must not cascade: {verdicts:?}"
        );
        assert!((chord_drill_accuracy(&verdicts, 3) - 1.0).abs() < 1e-6);
    }

    /// #349 T2 AC2 (inversions): a drill that demands the bass judges it —
    /// right chord with the demanded bass = Hit; right chord, wrong or
    /// missing bass = Near. A no-demand drill ignores the bass entirely.
    #[test]
    fn a_demanded_inversion_judges_the_bass() {
        let demanded = ChordTarget {
            segment: 0,
            root_pc: 0,
            quality: ChordQuality::Maj,
            bass_pc: Some(4), // C/E demanded
        };
        let mut j = ChordDrillJudge::new(vec![demanded, demanded, demanded]);
        let with_bass = |b: Option<u8>| HeardChord {
            root_pc: 0,
            quality: ChordQuality::Maj,
            bass_pc: b,
        };
        assert_eq!(j.observe(with_bass(Some(4)))[0].verdict, Verdict::Hit);
        assert_eq!(j.observe(with_bass(Some(7)))[0].verdict, Verdict::Near);
        assert_eq!(j.observe(with_bass(None))[0].verdict, Verdict::Near);

        // Same voicing heard, but the drill did NOT demand a bass → Hit
        // even played root position or any inversion.
        let mut j = ChordDrillJudge::new(vec![target(0, 0, ChordQuality::Maj)]);
        assert_eq!(j.observe(with_bass(Some(7)))[0].verdict, Verdict::Hit);
    }

    /// Skip credit: fumble one key, nail the next — the fumbled cell closes
    /// Missed, the nailed one credits Hit, and alignment continues. An
    /// exact hit on next also outranks a Near on current (documented
    /// decision). Fails if the lookahead is removed.
    #[test]
    fn a_fumbled_cell_takes_skip_credit_from_a_clean_next_hit() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Dom7),  // C7 — fumbled
            target(1, 5, ChordQuality::Dom7),  // F7 — played instead
            target(2, 10, ChordQuality::Dom7), // Bb7
        ]);
        let v = j.observe(heard(5, ChordQuality::Dom7));
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].measure_number, v[0].verdict), (1, Verdict::Missed));
        assert_eq!((v[1].measure_number, v[1].verdict), (2, Verdict::Hit));
        let v = j.observe(heard(10, ChordQuality::Dom7));
        assert_eq!((v[0].measure_number, v[0].verdict), (3, Verdict::Hit));
        assert!(j.is_done());

        // Near-on-current vs exact-Hit-on-next: the hit wins.
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Maj7), // heard chord is Near here…
            target(1, 0, ChordQuality::Dom7), // …but an exact hit here
        ]);
        let v = j.observe(heard(0, ChordQuality::Dom7));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].verdict, Verdict::Missed);
        assert_eq!(v[1].verdict, Verdict::Hit);
    }

    /// Repeated identical targets: the session buffer is edge-triggered
    /// with a release reset, so a deliberate re-strike arrives as a second
    /// entry and judges the second cell — no dedup deadlock (review-round
    /// probe). Also covers the skip-credit-then-same-reading case.
    #[test]
    fn a_restrike_judges_a_repeated_target() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Maj),
            target(1, 0, ChordQuality::Maj),
        ]);
        assert_eq!(
            j.observe(heard(0, ChordQuality::Maj))[0].verdict,
            Verdict::Hit
        );
        assert_eq!(
            j.observe(heard(0, ChordQuality::Maj))[0].verdict,
            Verdict::Hit
        );
        assert!(j.is_done());

        // Skip credit into F7, then F7 again for the repeated third target.
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Dom7),
            target(1, 5, ChordQuality::Dom7),
            target(2, 5, ChordQuality::Dom7),
        ]);
        let v = j.observe(heard(5, ChordQuality::Dom7));
        assert_eq!(v.len(), 2, "skip credit fires");
        let v = j.observe(heard(5, ChordQuality::Dom7));
        assert_eq!(
            (v[0].measure_number, v[0].verdict),
            (3, Verdict::Hit),
            "the re-strike judges the repeated target"
        );
    }

    /// Edges: empty targets judge nothing; a single-target drill with a
    /// wrong reading holds (lookahead at the end is safely absent) and
    /// closes Missed at finish.
    #[test]
    fn empty_and_single_target_edges_hold_honestly() {
        let mut j = ChordDrillJudge::new(Vec::new());
        assert!(j.is_done());
        assert!(j.observe(heard(0, ChordQuality::Maj)).is_empty());
        assert!(j.finish().is_empty());

        let mut j = ChordDrillJudge::new(vec![target(0, 0, ChordQuality::Maj)]);
        assert!(j.observe(heard(6, ChordQuality::Maj)).is_empty());
        assert_eq!(j.progress(), (0, 1));
        assert_eq!(j.finish()[0].verdict, Verdict::Missed);
    }

    /// finish() closes every unplayed cell as an honest Missed, and the
    /// accuracy tally divides by TOTAL targets — a caller that forgets
    /// finish() under-counts instead of inflating one hit to 100%.
    #[test]
    fn finish_misses_the_unplayed_and_accuracy_divides_by_total() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Dom7),
            target(1, 5, ChordQuality::Dom7),
            target(2, 10, ChordQuality::Dom7),
            target(3, 3, ChordQuality::Dom7),
        ]);
        let mut verdicts = j.observe(heard(0, ChordQuality::Dom7)); // Hit
        verdicts.extend(j.observe(heard(5, ChordQuality::Maj7))); // Near
                                                                  // Forgotten finish: 1.5/4, never 1.5/2.
        assert!((chord_drill_accuracy(&verdicts, 4) - 0.375).abs() < 1e-6);
        verdicts.extend(j.finish()); // 2 unplayed → Missed
        assert_eq!(verdicts.len(), 4);
        assert!((chord_drill_accuracy(&verdicts, 4) - 0.375).abs() < 1e-6);
        assert_eq!(chord_drill_accuracy(&[], 0), 0.0);
    }

    /// score_chord_drill: the lesson-facing grade — a half-played 12-key
    /// drill earns exactly its hits, unplayed cells score zero, and the
    /// count proxy reflects the shortfall.
    #[test]
    fn score_chord_drill_grades_a_half_played_drill_honestly() {
        let targets: Vec<ChordTarget> = (0..4)
            .map(|i| target(i, (i as u8 * 5) % 12, ChordQuality::Dom7))
            .collect();
        // Player nails cells 1 and 2, then stops.
        let played = vec![heard(0, ChordQuality::Dom7), heard(5, ChordQuality::Dom7)];
        let s = score_chord_drill(&targets, &played);
        assert_eq!(s.per_note.len(), 4);
        assert_eq!(s.per_note.iter().filter(|g| g.correct).count(), 2);
        assert!((s.accuracy - 0.5).abs() < 1e-6);
        assert!((s.timing_accuracy - 0.5).abs() < 1e-6);
        // Empty drill: honest zero, no panic.
        let empty = score_chord_drill(&[], &played);
        assert_eq!(empty.per_note.len(), 0);
        assert_eq!(empty.accuracy, 0.0);
    }

    /// The round-2 review probe, now the spec: drop TWO cells, play the
    /// rest perfectly → the dropped cells grade Missed and the remainder
    /// grades Hit (0.6), never a zeroed drill. Fails if the skip window
    /// shrinks back to one cell.
    #[test]
    fn dropping_two_cells_never_zeroes_the_remainder() {
        let targets: Vec<ChordTarget> = [0u8, 5, 10, 3, 8]
            .iter()
            .enumerate()
            .map(|(i, &r)| target(i as u32, r, ChordQuality::Dom7))
            .collect();
        let mut j = ChordDrillJudge::new(targets.clone());
        let mut verdicts = Vec::new();
        for t in &targets[2..] {
            verdicts.extend(j.observe(heard(t.root_pc, t.quality)));
        }
        verdicts.extend(j.finish());
        assert_eq!(verdicts.len(), 5);
        assert_eq!(
            verdicts
                .iter()
                .filter(|v| v.verdict == Verdict::Hit)
                .count(),
            3,
            "the played remainder must credit: {verdicts:?}"
        );
        assert!((chord_drill_accuracy(&verdicts, 5) - 0.6).abs() < 1e-6);
    }

    /// ChordChangeBuffer (round-2 review): the edge-trigger contract the
    /// judge depends on, tested as a pure struct instead of closure glue.
    /// Sustain → one entry; release + re-strike → a fresh entry; a slash
    /// CORRECTION refreshes the last entry in place (so a late bass read
    /// can still upgrade a demanded-inversion cell) and a bass that fades
    /// to unnamed doesn't erase a real reading.
    #[test]
    fn the_change_buffer_records_changes_not_frames() {
        let c = |b: Option<u8>| HeardChord {
            root_pc: 0,
            quality: ChordQuality::Maj,
            bass_pc: b,
        };
        let mut buf = ChordChangeBuffer::new();
        // Sustained chord over many ticks: one entry.
        for _ in 0..5 {
            buf.observe(Some(c(None)));
        }
        assert_eq!(buf.len(), 1);
        // Wrong first slash read, corrected while ringing: refreshed in
        // place — count unchanged, bass upgraded.
        buf.observe(Some(c(Some(7))));
        buf.observe(Some(c(Some(4))));
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.heard()[0].bass_pc, Some(4));
        // Bass fading to unnamed keeps the named reading.
        buf.observe(Some(c(None)));
        assert_eq!(buf.heard()[0].bass_pc, Some(4));
        // Release, then re-strike the SAME chord: a fresh entry.
        buf.observe(None);
        buf.observe(Some(c(None)));
        assert_eq!(buf.len(), 2);
        // A different chord while ringing: a fresh entry too.
        buf.observe(Some(HeardChord {
            root_pc: 7,
            quality: ChordQuality::Maj,
            bass_pc: None,
        }));
        assert_eq!(buf.len(), 3);
        buf.clear();
        assert!(buf.is_empty());
    }

    /// Verdicts land on the RV grid: segment k = measure k+1, downbeat —
    /// so the strip and recap attribute cells correctly even after a
    /// root shuffle.
    #[test]
    fn verdicts_carry_the_segment_measure() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 2, ChordQuality::Min7),
            target(1, 9, ChordQuality::Min7),
        ]);
        let v = j.observe(heard(2, ChordQuality::Min7));
        assert_eq!((v[0].measure_number, v[0].beat), (1, 0.0));
        let v = j.observe(heard(9, ChordQuality::Min7));
        assert_eq!(v[0].measure_number, 2);
    }
}
