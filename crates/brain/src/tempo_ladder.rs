//! The Pocket's graduated tempo ladder (#421 S5a): score practice's belay
//! partner. Start at 70% of the score's tempo; each clean pass — judged from
//! the same per-note verdict tally the player watches — earns +5%, up to the
//! score's own tempo. A dirty pass holds; the ladder never steps down.
//!
//! Pure state + rules only. Pass boundaries (when a run-through ended) and
//! the click/UI wiring are S5b's job, so these rules stay pinned while the
//! wiring iterates.

use serde::{Deserialize, Serialize};

use crate::follower::{NoteVerdict, Verdict};

/// The Pocket click's supported range — the same bounds as the desktop
/// shell's `clamp_pocket_params`. Wiring (S5b) should import these rather
/// than keep its own literals. (One deliberate divergence: the ladder sends
/// EVERY non-finite tempo to the floor, where the shell's clamp maps +∞ to
/// the ceiling — a garbage score tempo should slow the click, never max it.)
pub const POCKET_MIN_BPM: f64 = 40.0;
pub const POCKET_MAX_BPM: f64 = 220.0;

/// Ladder rules. 70 / +5 / top-at-score-tempo are the founder's numbers
/// (#421); the cleanliness fractions are tuning defaults in the same spirit
/// as `MomentConfig` (#259) — adjustable without touching the rules.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LadderConfig {
    /// Where the ladder starts, as a percent of score tempo.
    pub start_percent: u8,
    /// Earned per clean pass.
    pub step_percent: u8,
    /// The top of the ladder (100 = the score's own tempo).
    pub max_percent: u8,
    /// Judged notes / score notes required for a run to count as a pass.
    pub min_coverage: f32,
    /// Misses / judged at or below this can still be clean.
    pub max_miss_frac: f32,
    /// Hits / judged at or above this required for clean.
    pub min_hit_frac: f32,
}

impl Default for LadderConfig {
    fn default() -> Self {
        Self {
            start_percent: 70,
            step_percent: 5,
            max_percent: 100,
            min_coverage: 0.90,
            max_miss_frac: 0.02,
            min_hit_frac: 0.90,
        }
    }
}

/// One finished pass's verdict counts — the tally, aggregated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PassTally {
    pub hits: usize,
    pub nears: usize,
    pub misses: usize,
}

impl PassTally {
    pub fn from_verdicts(verdicts: &[NoteVerdict]) -> Self {
        let mut tally = Self::default();
        for v in verdicts {
            match v.verdict {
                Verdict::Hit => tally.hits += 1,
                Verdict::Near => tally.nears += 1,
                Verdict::Missed => tally.misses += 1,
            }
        }
        tally
    }

    pub fn judged(&self) -> usize {
        self.hits + self.nears + self.misses
    }
}

/// Why a pass held the ladder in place instead of stepping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    /// Nothing judged, or the score has no notes.
    NoNotes,
    /// Too little of the score was judged for the run to count as a pass.
    LowCoverage,
    /// Miss fraction above the clean bar.
    TooManyMisses,
    /// Hit fraction below the clean bar (e.g. an all-Near, sour run).
    LowHitRate,
}

/// What a finished pass did to the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum PassOutcome {
    /// Clean pass — the ladder climbed ("You earned 85%").
    Stepped { from_percent: u8, to_percent: u8 },
    /// Clean pass with the ladder already at the top.
    AtTop,
    /// Percent unchanged. Never a step down — a belay partner holds.
    Held { reason: HoldReason },
}

/// The ladder itself: current rung + rules.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TempoLadder {
    percent: u8,
    config: LadderConfig,
}

impl TempoLadder {
    /// Degenerate configs are sanitized rather than trusted: zero step/max
    /// are raised to 1, the start is clamped into the ladder, and the
    /// cleanliness fractions are forced into [0, 1] with NaN falling back
    /// to the default (NaN thresholds would make every comparison false and
    /// grade an all-miss run "clean" — the exact inversion of "we KNOW when
    /// it was clean").
    pub fn new(config: LadderConfig) -> Self {
        fn frac(value: f32, default: f32) -> f32 {
            if value.is_nan() {
                default
            } else {
                value.clamp(0.0, 1.0)
            }
        }
        let defaults = LadderConfig::default();
        let mut config = config;
        config.max_percent = config.max_percent.max(1);
        config.step_percent = config.step_percent.max(1);
        config.start_percent = config.start_percent.clamp(1, config.max_percent);
        config.min_coverage = frac(config.min_coverage, defaults.min_coverage);
        config.max_miss_frac = frac(config.max_miss_frac, defaults.max_miss_frac);
        config.min_hit_frac = frac(config.min_hit_frac, defaults.min_hit_frac);
        Self {
            percent: config.start_percent,
            config,
        }
    }

    /// Current rung, as a percent of score tempo.
    pub fn percent(&self) -> u8 {
        self.percent
    }

    /// The BPM the click should play: the rung applied to the score's tempo,
    /// clamped into the Pocket's range. Wild imported tempi (NaN, ±∞, zero,
    /// negative) all produce the floor.
    pub fn practice_bpm(&self, score_tempo_bpm: f64) -> f64 {
        let raw = score_tempo_bpm * f64::from(self.percent) / 100.0;
        if raw.is_finite() {
            raw.clamp(POCKET_MIN_BPM, POCKET_MAX_BPM)
        } else {
            POCKET_MIN_BPM
        }
    }

    /// Fold one finished run-through into the ladder. `score_note_count` is
    /// the score's total judgeable notes (coverage denominator).
    pub fn complete_pass(&mut self, tally: &PassTally, score_note_count: usize) -> PassOutcome {
        let judged = tally.judged();
        if judged == 0 || score_note_count == 0 {
            return PassOutcome::Held {
                reason: HoldReason::NoNotes,
            };
        }
        let coverage = judged as f32 / score_note_count as f32;
        if coverage < self.config.min_coverage {
            return PassOutcome::Held {
                reason: HoldReason::LowCoverage,
            };
        }
        let miss_frac = tally.misses as f32 / judged as f32;
        if miss_frac > self.config.max_miss_frac {
            return PassOutcome::Held {
                reason: HoldReason::TooManyMisses,
            };
        }
        let hit_frac = tally.hits as f32 / judged as f32;
        if hit_frac < self.config.min_hit_frac {
            return PassOutcome::Held {
                reason: HoldReason::LowHitRate,
            };
        }
        if self.percent >= self.config.max_percent {
            return PassOutcome::AtTop;
        }
        let from = self.percent;
        self.percent = self
            .percent
            .saturating_add(self.config.step_percent)
            .min(self.config.max_percent);
        PassOutcome::Stepped {
            from_percent: from,
            to_percent: self.percent,
        }
    }
}

impl Default for TempoLadder {
    fn default() -> Self {
        Self::new(LadderConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(v: Verdict) -> NoteVerdict {
        NoteVerdict {
            measure_number: 1,
            beat: 0.0,
            verdict: v,
        }
    }

    /// A tally that is clean under the default config: `hits` hits and
    /// nothing else, judged over the whole score.
    fn all_hits(n: usize) -> PassTally {
        PassTally {
            hits: n,
            nears: 0,
            misses: 0,
        }
    }

    #[test]
    fn clean_passes_climb_from_seventy_by_five() {
        let mut ladder = TempoLadder::default();
        assert_eq!(ladder.percent(), 70);
        assert_eq!(
            ladder.complete_pass(&all_hits(100), 100),
            PassOutcome::Stepped {
                from_percent: 70,
                to_percent: 75
            }
        );
        assert_eq!(
            ladder.complete_pass(&all_hits(100), 100),
            PassOutcome::Stepped {
                from_percent: 75,
                to_percent: 80
            }
        );
        assert_eq!(ladder.percent(), 80);
    }

    #[test]
    fn ladder_clamps_at_top_and_reports_at_top() {
        // Start one short of the top with an overshooting step: the climb
        // must land exactly on max, never past it.
        let mut ladder = TempoLadder::new(LadderConfig {
            start_percent: 98,
            ..LadderConfig::default()
        });
        assert_eq!(
            ladder.complete_pass(&all_hits(50), 50),
            PassOutcome::Stepped {
                from_percent: 98,
                to_percent: 100
            }
        );
        // Clean at the top: acknowledged, not climbed.
        assert_eq!(ladder.complete_pass(&all_hits(50), 50), PassOutcome::AtTop);
        assert_eq!(ladder.percent(), 100);
        // AtTop is an EARNED acknowledgment: a garbage run at the top must
        // still read as held, never as a pat on the back.
        let all_misses = PassTally {
            hits: 0,
            nears: 0,
            misses: 50,
        };
        assert_eq!(
            ladder.complete_pass(&all_misses, 50),
            PassOutcome::Held {
                reason: HoldReason::TooManyMisses
            }
        );
    }

    #[test]
    fn dirty_pass_holds_with_specific_reason() {
        let mut ladder = TempoLadder::default();
        // 5 misses in 100 judged = 5% > the 2% allowance.
        let missy = PassTally {
            hits: 95,
            nears: 0,
            misses: 5,
        };
        assert_eq!(
            ladder.complete_pass(&missy, 100),
            PassOutcome::Held {
                reason: HoldReason::TooManyMisses
            }
        );
        // Right notes, sour intonation: 20% Near leaves hits at 80% < 90%.
        let sour = PassTally {
            hits: 80,
            nears: 20,
            misses: 0,
        };
        assert_eq!(
            ladder.complete_pass(&sour, 100),
            PassOutcome::Held {
                reason: HoldReason::LowHitRate
            }
        );
        assert_eq!(ladder.percent(), 70, "held passes must not move the rung");
    }

    #[test]
    fn partial_run_holds_on_low_coverage() {
        let mut ladder = TempoLadder::default();
        // A flawless half-score run is not a pass.
        assert_eq!(
            ladder.complete_pass(&all_hits(50), 100),
            PassOutcome::Held {
                reason: HoldReason::LowCoverage
            }
        );
        assert_eq!(ladder.percent(), 70);
    }

    #[test]
    fn empty_pass_and_empty_score_hold_no_notes() {
        let mut ladder = TempoLadder::default();
        assert_eq!(
            ladder.complete_pass(&PassTally::default(), 100),
            PassOutcome::Held {
                reason: HoldReason::NoNotes
            }
        );
        // A zero-note score must not divide by zero (coverage denominator).
        assert_eq!(
            ladder.complete_pass(&all_hits(10), 0),
            PassOutcome::Held {
                reason: HoldReason::NoNotes
            }
        );
        assert_eq!(ladder.percent(), 70);
    }

    #[test]
    fn practice_bpm_scales_and_clamps() {
        let ladder = TempoLadder::default(); // 70%
        assert!((ladder.practice_bpm(100.0) - 70.0).abs() < 1e-9);
        // 40 BPM score at 70% = 28 raw → the click floor.
        assert_eq!(ladder.practice_bpm(40.0), POCKET_MIN_BPM);
        // At the top of the ladder a fast score still clamps to the ceiling.
        let mut topped = TempoLadder::default();
        for _ in 0..7 {
            topped.complete_pass(&all_hits(10), 10);
        }
        assert_eq!(topped.percent(), 100, "70 + 6×5 + a capped step reach 100");
        assert_eq!(topped.practice_bpm(300.0), POCKET_MAX_BPM);
        // Wild imported tempi never leak NaN or nonsense into the click —
        // and +∞ goes to the FLOOR, not the ceiling (a garbage tempo must
        // slow the click, never max it).
        assert_eq!(ladder.practice_bpm(f64::NAN), POCKET_MIN_BPM);
        assert_eq!(ladder.practice_bpm(f64::INFINITY), POCKET_MIN_BPM);
        assert_eq!(ladder.practice_bpm(f64::NEG_INFINITY), POCKET_MIN_BPM);
        assert_eq!(ladder.practice_bpm(0.0), POCKET_MIN_BPM);
        assert_eq!(ladder.practice_bpm(-60.0), POCKET_MIN_BPM);
    }

    #[test]
    fn threshold_boundaries_are_inclusive() {
        // Exactly at every bar: coverage 90/100, misses 2% of judged,
        // hits 90% of judged. 90 judged: 81 hits (90%), 7 nears, 2 misses
        // (2.2% > 2%) would fail — build exact 2%: 100 judged of a
        // 100-note score with 90 hits, 8 nears, 2 misses.
        let mut ladder = TempoLadder::default();
        let boundary = PassTally {
            hits: 90,
            nears: 8,
            misses: 2,
        };
        assert_eq!(
            ladder.complete_pass(&boundary, 100),
            PassOutcome::Stepped {
                from_percent: 70,
                to_percent: 75
            }
        );
        // Coverage exactly at 90% also steps.
        let mut ladder2 = TempoLadder::default();
        assert_eq!(
            ladder2.complete_pass(&all_hits(90), 100),
            PassOutcome::Stepped {
                from_percent: 70,
                to_percent: 75
            }
        );
        // One more miss tips the miss bar.
        let mut ladder3 = TempoLadder::default();
        let over = PassTally {
            hits: 90,
            nears: 7,
            misses: 3,
        };
        assert_eq!(
            ladder3.complete_pass(&over, 100),
            PassOutcome::Held {
                reason: HoldReason::TooManyMisses
            }
        );
    }

    #[test]
    fn tally_counts_each_verdict_bucket() {
        // Distinct counts per bucket, so swapping any two arms of the
        // `from_verdicts` match cannot survive this test.
        let verdicts = vec![
            verdict(Verdict::Hit),
            verdict(Verdict::Near),
            verdict(Verdict::Hit),
            verdict(Verdict::Missed),
            verdict(Verdict::Hit),
            verdict(Verdict::Near),
        ];
        let tally = PassTally::from_verdicts(&verdicts);
        assert_eq!(
            tally,
            PassTally {
                hits: 3,
                nears: 2,
                misses: 1
            }
        );
        assert_eq!(tally.judged(), 6);
    }

    #[test]
    fn degenerate_config_is_sanitized() {
        // Start above max clamps down to it (and reads AtTop on a clean pass).
        let mut high = TempoLadder::new(LadderConfig {
            start_percent: 120,
            max_percent: 100,
            ..LadderConfig::default()
        });
        assert_eq!(high.percent(), 100);
        assert_eq!(high.complete_pass(&all_hits(10), 10), PassOutcome::AtTop);
        // Zero step and zero max are raised to 1 — the ladder still functions.
        let mut zeroed = TempoLadder::new(LadderConfig {
            start_percent: 0,
            step_percent: 0,
            max_percent: 0,
            ..LadderConfig::default()
        });
        assert_eq!(zeroed.percent(), 1);
        assert_eq!(zeroed.complete_pass(&all_hits(10), 10), PassOutcome::AtTop);
        // A sanitized nonzero step still climbs.
        let mut stepless = TempoLadder::new(LadderConfig {
            step_percent: 0,
            ..LadderConfig::default()
        });
        assert_eq!(
            stepless.complete_pass(&all_hits(10), 10),
            PassOutcome::Stepped {
                from_percent: 70,
                to_percent: 71
            }
        );
    }

    #[test]
    fn nan_and_wild_thresholds_cannot_bless_a_dirty_pass() {
        // NaN thresholds make every guard comparison false — unsanitized,
        // an all-miss run would grade "clean" and step the ladder.
        let mut nan_cfg = TempoLadder::new(LadderConfig {
            min_coverage: f32::NAN,
            max_miss_frac: f32::NAN,
            min_hit_frac: f32::NAN,
            ..LadderConfig::default()
        });
        let all_misses = PassTally {
            hits: 0,
            nears: 0,
            misses: 100,
        };
        assert_eq!(
            nan_cfg.complete_pass(&all_misses, 100),
            PassOutcome::Held {
                reason: HoldReason::TooManyMisses
            }
        );
        assert_eq!(nan_cfg.percent(), 70);
        // Out-of-range fractions clamp into [0, 1]: a >1 coverage bar would
        // otherwise hold a full flawless run forever.
        let mut strict = TempoLadder::new(LadderConfig {
            min_coverage: 2.0,
            min_hit_frac: 5.0,
            ..LadderConfig::default()
        });
        assert_eq!(
            strict.complete_pass(&all_hits(100), 100),
            PassOutcome::Stepped {
                from_percent: 70,
                to_percent: 75
            }
        );
    }

    #[test]
    fn pass_outcome_wire_shape_is_pinned() {
        // S5b's TypeScript will match on this exact tagged shape; renaming
        // the tag or a variant must go red here, not in the app.
        let stepped = serde_json::to_value(PassOutcome::Stepped {
            from_percent: 80,
            to_percent: 85,
        })
        .expect("serializes");
        assert_eq!(
            stepped,
            serde_json::json!({
                "outcome": "stepped",
                "from_percent": 80,
                "to_percent": 85
            })
        );
        let held = serde_json::to_value(PassOutcome::Held {
            reason: HoldReason::LowCoverage,
        })
        .expect("serializes");
        assert_eq!(
            held,
            serde_json::json!({ "outcome": "held", "reason": "low_coverage" })
        );
        // And it round-trips.
        let back: PassOutcome = serde_json::from_value(held).expect("deserializes");
        assert_eq!(
            back,
            PassOutcome::Held {
                reason: HoldReason::LowCoverage
            }
        );
    }
}
