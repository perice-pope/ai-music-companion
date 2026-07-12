//! #349 T2b — chord drills judged by the T1 chord engine.
//!
//! The melodic follower judges lines; a BLOCK-CHORD drill is judged here
//! instead: the drill's [`ChordTarget`]s (what the T1 engine should hear,
//! straight from `variations::generate`) against the stable
//! [`ChordReading`]s the perception tracker actually promoted. Verdicts
//! ride the same [`NoteVerdict`] shape as melodic judging, so the strip,
//! phrase cards, and recap plumbing stay unchanged.
//!
//! Verdict rules (spec §6 T2):
//! - **Hit** — same root and quality; when the drill DEMANDS an inversion
//!   (`target.bass_pc = Some`), the heard bass must match too.
//! - **Near** — right root, wrong quality (C7 played for Cmaj7), or the
//!   right chord with the demanded bass wrong/missing.
//! - **Missed** — different root, or the cell was skipped over entirely.
//!
//! Progress is sequential with one cell of lookahead (the same skip-credit
//! discipline as the melodic follower): a reading that misses the current
//! target but is a clean HIT on the next one closes the current cell as
//! Missed and credits the next — a player who fumbles one key and moves on
//! is never dragged permanently out of alignment.
//!
//! Pure and deterministic: no clock, no audio — feed it readings, get
//! verdicts. The session layer owns cadence (readings arrive only when the
//! chord tracker PROMOTES a stable label, ~chord rate, never per frame).

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

/// How a heard chord grades against one target.
fn classify(heard: &HeardChord, target: &ChordTarget) -> Verdict {
    if heard.root_pc % 12 != target.root_pc % 12 {
        return Verdict::Missed;
    }
    if heard.quality != target.quality {
        return Verdict::Near; // right root, wrong extension/quality
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

/// Sequential judge over a stacked drill's chord targets.
#[derive(Debug, Clone)]
pub struct ChordDrillJudge {
    targets: Vec<ChordTarget>,
    /// Next unjudged target index.
    next: usize,
    /// The last reading judged, so a sustained chord (the tracker refreshes
    /// its confidence every ~100 ms) is judged ONCE, not per reading.
    last_heard: Option<HeardChord>,
}

impl ChordDrillJudge {
    pub fn new(targets: Vec<ChordTarget>) -> Self {
        Self {
            targets,
            next: 0,
            last_heard: None,
        }
    }

    /// True once every target has been judged.
    pub fn is_done(&self) -> bool {
        self.next >= self.targets.len()
    }

    /// Fold in one stable chord reading. Returns the verdicts it settles
    /// (usually one; two when skip-credit closes a fumbled cell). A repeat
    /// of the previous reading returns nothing — one chord, one verdict.
    pub fn observe(&mut self, heard: HeardChord) -> Vec<NoteVerdict> {
        if self.is_done() || self.last_heard == Some(heard) {
            return Vec::new();
        }
        self.last_heard = Some(heard);

        let mut out = Vec::new();
        let current = &self.targets[self.next];
        match classify(&heard, current) {
            Verdict::Missed => {
                // One cell of lookahead: a clean HIT on the next target
                // closes this one as Missed and credits the next.
                let skip_hit = self
                    .targets
                    .get(self.next + 1)
                    .is_some_and(|t| classify(&heard, t) == Verdict::Hit);
                if skip_hit {
                    out.push(self.settle(Verdict::Missed));
                    out.push(self.settle(Verdict::Hit));
                } else {
                    out.push(self.settle(Verdict::Missed));
                }
            }
            verdict => out.push(self.settle(verdict)),
        }
        out
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

/// Tally a finished chord drill into the lesson's 0..1 accuracy signal:
/// Hit = 1, Near = ½ (right root is real knowledge — the RV ramp shouldn't
/// treat C7-for-Cmaj7 like silence), Missed = 0.
pub fn chord_drill_accuracy(verdicts: &[NoteVerdict]) -> f32 {
    if verdicts.is_empty() {
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
    score / verdicts.len() as f32
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
    /// → Near (right root, wrong extension); Am for C → Missed.
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
        let v = j.observe(heard(9, ChordQuality::Min)); // Am for C
        assert_eq!(v[0].verdict, Verdict::Missed);
        assert!(j.is_done());
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
    /// Missed, the nailed one credits Hit, and alignment continues. Fails
    /// if the lookahead is removed (the player would be judged against the
    /// wrong target forever after).
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
    }

    /// One chord, one verdict: the tracker re-promotes the same reading for
    /// as long as the chord rings; repeats must not consume targets.
    #[test]
    fn a_sustained_chord_is_judged_once() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Maj),
            target(1, 7, ChordQuality::Maj),
        ]);
        assert_eq!(j.observe(heard(0, ChordQuality::Maj)).len(), 1);
        assert!(j.observe(heard(0, ChordQuality::Maj)).is_empty());
        assert!(j.observe(heard(0, ChordQuality::Maj)).is_empty());
        // A NEW chord judges the next target.
        assert_eq!(
            j.observe(heard(7, ChordQuality::Maj))[0].verdict,
            Verdict::Hit
        );
    }

    /// finish() closes every unplayed cell as an honest Missed, and the
    /// accuracy tally weighs Hit=1 / Near=0.5 / Missed=0.
    #[test]
    fn finish_misses_the_unplayed_and_accuracy_tallies() {
        let mut j = ChordDrillJudge::new(vec![
            target(0, 0, ChordQuality::Dom7),
            target(1, 5, ChordQuality::Dom7),
            target(2, 10, ChordQuality::Dom7),
            target(3, 3, ChordQuality::Dom7),
        ]);
        let mut verdicts = j.observe(heard(0, ChordQuality::Dom7)); // Hit
        verdicts.extend(j.observe(heard(5, ChordQuality::Maj7))); // Near
        verdicts.extend(j.finish()); // 2 unplayed → Missed
        assert_eq!(verdicts.len(), 4);
        assert!(j.is_done());
        // (1 + 0.5 + 0 + 0) / 4
        assert!((chord_drill_accuracy(&verdicts) - 0.375).abs() < 1e-6);
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
