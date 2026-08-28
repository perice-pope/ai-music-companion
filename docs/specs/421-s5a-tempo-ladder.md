# Spec: The Pocket S5a — graduated tempo ladder core (#421)

> Part of #421 (The Pocket). S5 in the story's slice list: "Graduated tempo ladder in score
> practice; subdivision journey". This slice is the ladder's pure Rust core; the subdivision
> journey is a separate later slice.

## 1. Summary
The deterministic heart of score practice's "belay partner" metronome: a ladder that starts at
70% of the score's tempo and earns +5% per clean pass — "You earned 85%." Pure `crates/brain`
logic (state + judgment + BPM derivation), consumed by the IPC/UI wiring slice (S5b).

## 2. Problem / why
#421's mode table promises score practice a graduated tempo ladder: "start at 70% of score
tempo; each clean pass (tally-driven — we KNOW when it was clean) steps +5%." Nothing implements
it. The tally (per-note `follower::Verdict`s) and the Pocket click (#421 S1/S2) both exist, but
no component turns a finished run-through into an earned tempo step. Business logic belongs in
the Rust core (CLAUDE.md), so the ladder's rules land there first, fully testable, before any
IPC or UI.

## 3. Non-goals
- No IPC commands/events, no frontend, no `practiceStore` changes — that's S5b (deliberately,
  four open PRs currently touch `commands.rs`/`practiceStore.ts`; this slice stays off them).
- No pass-boundary detection: deciding *when* a run-through ended (follower end-reached, player
  restart, silence reset) is wiring-time follower work. The core consumes a finished pass's
  verdicts.
- No subdivision journey (the S5 second half; its own slice).
- No persistence — ladder state is per-session; S5b decides if it outlives one.
- No change to the Pocket click, band, or follower.

## 4. Contract / interface
New module `crates/brain/src/tempo_ladder.rs` (`pub mod tempo_ladder` in `lib.rs`).

```rust
/// Same click law as the Pocket's `clamp_pocket_params` (commands.rs);
/// S5b should import these instead of keeping its own literals.
pub const POCKET_MIN_BPM: f64 = 40.0;
pub const POCKET_MAX_BPM: f64 = 220.0;

pub struct LadderConfig {
    pub start_percent: u8,  // 70
    pub step_percent: u8,   // 5
    pub max_percent: u8,    // 100 — the ladder tops at the score's own tempo
    pub min_coverage: f32,  // 0.90 — judged/score notes for a run to count as a pass
    pub max_miss_frac: f32, // 0.02 — misses/judged at/below this can still be clean
    pub min_hit_frac: f32,  // 0.90 — hits/judged at/above this required for clean
}

pub struct PassTally { pub hits: usize, pub nears: usize, pub misses: usize }
impl PassTally {
    pub fn from_verdicts(verdicts: &[follower::NoteVerdict]) -> Self;
    pub fn judged(&self) -> usize;
}

pub struct TempoLadder { /* percent + config */ }
impl TempoLadder {
    pub fn new(config: LadderConfig) -> Self; // sanitizes degenerate configs
    pub fn percent(&self) -> u8;
    pub fn practice_bpm(&self, score_tempo_bpm: f64) -> f64; // percent applied, clamped 40–220
    pub fn complete_pass(&mut self, tally: &PassTally, score_note_count: usize) -> PassOutcome;
}

pub enum PassOutcome {
    Stepped { from_percent: u8, to_percent: u8 }, // clean pass, ladder climbed
    AtTop,                                        // clean pass already at max_percent
    Held { reason: HoldReason },                  // percent unchanged — never a step down
}
pub enum HoldReason { NoNotes, LowCoverage, TooManyMisses, LowHitRate }
```

A pass is **clean** iff `judged/score_note_count >= min_coverage` AND
`misses/judged <= max_miss_frac` AND `hits/judged >= min_hit_frac`. A dirty pass **holds** —
the belay partner never drops the climber. All types `Serialize`/`Deserialize` so S5b can emit
them over IPC unchanged. Thresholds are tuning values (same stance as #259's `MomentConfig`);
the founder-fixed numbers are 70 / +5 / tally-driven.

## 5. Acceptance criteria (numbered, testable)
1. A new ladder starts at `start_percent` (70) and a clean pass steps exactly `step_percent`
   up, reporting `Stepped { from, to }` (70 → 75, then 75 → 80, …).
2. The ladder never exceeds `max_percent`: a step that would overshoot clamps to it, and a
   clean pass at the top returns `AtTop` with the percent unchanged.
3. A pass whose miss fraction exceeds `max_miss_frac` or whose hit fraction is below
   `min_hit_frac` returns `Held` with the specific reason and leaves the percent unchanged.
4. A pass judging fewer than `min_coverage` of the score's notes returns
   `Held { LowCoverage }` even if every judged note was a hit.
5. Zero judged notes or a zero-note score returns `Held { NoNotes }` — no panic, no division
   by zero, no step.
6. `practice_bpm` is `score_tempo × percent/100`, clamped into 40–220 BPM; non-finite or
   non-positive score tempi produce the 40 BPM floor (the `clamp_pocket_params` law).
7. Threshold boundaries are inclusive as specified: coverage exactly at `min_coverage`, miss
   fraction exactly at `max_miss_frac`, and hit fraction exactly at `min_hit_frac` all count
   toward clean.
8. `PassTally::from_verdicts` counts each `Verdict` variant into the right bucket.
9. A degenerate config is sanitized at construction: `start_percent` above `max_percent`
   clamps to it, and zero `step_percent`/`max_percent` are raised to 1 (the ladder still
   functions, it never divides by zero or steps by nothing forever at 0%).

## 6. Edge cases & failure modes
- Empty verdict slice / empty score → AC5 (`NoNotes`).
- All-`Near` run (right notes, sour intonation) → `LowHitRate`, not a step (a ladder step is
  *earned*).
- Follower relocation already suppresses false `Missed` runs (`MAX_MISS_RUN`), so the 2%
  miss allowance is against honest misses; no extra handling here.
- Slow score (40 BPM) at 70% → 28 BPM raw → clamps to the 40 floor: the ladder can read 70%
  while the click plays the floor. `practice_bpm` is the single truth the wiring must use.
- Score tempo `NaN`/`0`/negative (imported scores are wild) → 40 BPM floor, never `NaN` out.
- Overshoot config (start 98, step 5) → clean pass lands exactly on `max_percent`.

## 7. Test plan
Inline `#[cfg(test)]` in `tempo_ladder.rs` (pure unit logic, repo convention).
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `clean_passes_climb_from_seventy_by_five` | 70→75→80 with exact `Stepped` payloads |
| AC2 | `ladder_clamps_at_top_and_reports_at_top` | overshoot lands on 100; next clean pass `AtTop`, still 100 |
| AC3 | `dirty_pass_holds_with_specific_reason` | miss-heavy → `TooManyMisses`; near-heavy → `LowHitRate`; percent frozen |
| AC4 | `partial_run_holds_on_low_coverage` | perfect-but-partial → `LowCoverage` |
| AC5 | `empty_pass_and_empty_score_hold_no_notes` | `NoNotes` for judged==0 and score_note_count==0 |
| AC6 | `practice_bpm_scales_and_clamps` | 100→70 @70%; 40→40; 300→220 @100%; NaN/0/-5 → 40 |
| AC7 | `threshold_boundaries_are_inclusive` | exact-boundary tallies step; one miss past holds |
| AC8 | `tally_counts_each_verdict_bucket` | mixed verdict slice → exact hit/near/miss counts |
| AC9 | `degenerate_config_is_sanitized` | start>max clamps; zero step/max raised to 1 |

## 8. Architecture / approach
Pure state machine beside its consumers' peers (`wheel.rs`, `chord_judge.rs` shape): no I/O,
no clock, no allocation concerns (runs at pass boundaries, far off the audio thread). Consumes
`follower::NoteVerdict` — the same tally the score UI already renders — so "clean" can never
drift from what the player saw. Fully offline; no network surface.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S5a | ladder core (this spec) | `crates/brain/src/tempo_ladder.rs`, `lib.rs` | follower verdicts (shipped) | no |
| S5b | wiring: pass boundary from the follower, ladder IPC + score-practice UI chip, Pocket click driven by `practice_bpm` | `commands.rs`, score-practice components/store | S5a, S1 click | no |
| S5c | subdivision journey (the delighter) | output engine schedule + Pocket UI | S1/S2 | no |

## 10. Risks / open questions
- Cleanliness thresholds (0.90/0.02/0.90) are engineering defaults pending real-piano feel;
  they live in `LadderConfig`, so S5b can tune without touching the rules. Same stance the
  founder accepted for `MomentConfig` (#259).
- Pass-boundary detection (S5b) is the genuinely fuzzy part (follower resets, partial
  restarts); kept out of the core on purpose so the rules stay pinned while wiring iterates.

## 11. References
- #421 (story; the mode table's "Graduated tempo ladder" row), S1 #434, S2 #441.
- `crates/brain/src/follower.rs` (`Verdict`, `NoteVerdict`, `MAX_MISS_RUN`).
- `apps/desktop/src-tauri/src/commands.rs` (`clamp_pocket_params` — the 40–220 click law).
- `docs/specs/259-boss-moments.md` (core-first slicing + tuning-config precedent).
