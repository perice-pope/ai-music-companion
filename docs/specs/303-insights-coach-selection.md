# Spec: Close the loop — exercise insights reach coach selection (#303)

> Part of epic #252. The exercise log + `brain::insights` (shipped) record which material
> shapes teach. This spec wires the verdicts back: the coach prefers what's working, retires
> what's repeatedly dealt-and-bailed, and the recap says so in one line.

## 1. Summary

A pure verdict layer over the exercise log — per catalog shape, "this is WORKING for the
player" or "this keeps getting dealt and BAILED on" — plus the founder's recap line
("the 1-2-3-5 pattern is working for you"). Selection wiring (explore deals, guided-coach
ladder) builds behind the verdict contract in follow-up slices.

## 2. Problem / why

`insights::exercise_insights` answers "which exercises are good?" but nothing consumes the
answer (`insights.rs` says so itself: "wiring the verdicts back into drill selection is the
follow-up issue"). The coach deals patterns and scales by pure seed; a shape the player
grades higher every week is dealt no more often than one they abandon every time. Issue #303
is the founder's directive to close the loop.

## 3. Non-goals

- No change to WHICH chips exist or their order — #445-4 pins the stable five.
- No change to the drill-kind ladder (`kind_at`) or difficulty ramp semantics.
- S1 does not yet bias the seeded picks (`deal_explore_pattern`, `swap_explore_scale`) —
  that's S2, behind the verdict contract this slice lands.
- No new UI. The line rides the existing #453 plumbing (recap prompt block + coaching box).
- No network, no schema changes, no new IPC command.

## 4. Contract / interface

New in `crates/brain/src/insights.rs` (pure, deterministic, `now` injected):

```rust
pub enum ShapeCall {
    /// Grades rising inside the window — deal it again.
    Working { early: f32, late: f32, graded: u32 },
    /// Dealt repeatedly, rarely played to a grade — selection should rest it.
    Bailing { dealt: u32, graded: u32 },
}
pub struct ShapeVerdict { pub shape: String, pub call: ShapeCall, pub newest: DateTime<FixedOffset> }
pub fn shape_verdicts(log: &[TimedExerciseLogEntry], now: DateTime<Utc>) -> Vec<ShapeVerdict>
```

`SuggestionKind` gains a fourth variant `Working`; `practice_suggestions` appends its lines
last (pinned order becomes Trend → Neglect → Momentum → Working). The DTO maps it to
`"working"`. Additive: no existing signature changes.

Verdicts cover **catalog shapes only** — player material (a lifted cell, which is the
Momentum rule's beat, or a lifted progression) earns no verdict (one material, one claim,
and S2's seeded catalog dealer couldn't re-deal it), and score-reference / unparseable rows
feed no verdict (a shape we can't name honestly earns no call).

Bars, mirroring the shipped #453 discipline:

- Window: the shape's most recent ≤ `SHAPE_WINDOW_ROWS` (12) log rows — a verdict is about
  the shape's recent life, not its lifetime.
- Working: ≥ `WORKING_MIN_GRADED` (6) graded rows in the window (each half rests on ≥3),
  newer-half − older-half ≥ `WORKING_MIN_DELTA` (0.15), newest **graded** row ≤
  `SHAPE_RECENT_DAYS` (14) days old.
- Bailing: ≥ `BAILING_MIN_DEALT` (6) rows in the window, graded × 3 ≤ dealt, newest row ≤
  14 days old. Mutually exclusive with Working by arithmetic (6 graded × 3 > 12-row window).

## 5. Acceptance criteria (numbered, testable)

1. A catalog shape with ≥6 graded rows in its last ≤12 rows, half-delta ≥ +0.15, and a
   graded row ≤14 days old yields exactly one `Working` suggestion whose text embeds the
   shape name, both percentages, and the graded count, and whose evidence cites the halves
   and newest date. All three bars are inclusive: exactly 6 graded rows, a delta of exactly
   +0.15, and a newest grade exactly 14 days old still fire.
2. Each Working bar individually silences: 5 graded rows, delta 0.14, newest grade 15 days
   old, and ungraded rows never counting toward the graded bar.
3. Rows outside the 12-row window never fabricate a climb: ancient lows before a recent
   flat plateau stay silent.
4. Player material never earns `Working`: a rising player-cell history still earns Momentum
   (one voice), a rising lifted-progression history earns no verdict, and score-reference /
   unparseable rows produce no verdict of either call.
5. A shape dealt ≥6 times in its window with graded×3 ≤ dealt and a row ≤14 days old yields
   a `Bailing` verdict carrying both window counts. The ratio bar is inclusive (2-of-6 and
   4-of-12 fire; 3-of-6 and 5-of-12 stay silent); 5 deals or a stale window silence it;
   recency reads the window's NEWEST row (old deals with a fresh bail still call). No shape
   ever carries both calls.
6. `practice_suggestions` order is pinned Trend → Neglect → Momentum → Working and is
   deterministic (identical inputs → identical output).
7. The command layer surfaces a seeded rising catalog history as one `"working"` DTO whose
   text and evidence carry numbers (and nothing else fires on that fixture).

## 6. Edge cases & failure modes

- Empty log → no verdicts, no suggestions (existing AC holds).
- Corrupt `logged_at` stamps → the row feeds no verdict (same defensive parse as #453).
- Garbage `spec_json` → no verdict (catalog gate returns `None`).
- NaN accuracy → comparisons are false, no Working claim fires (NaN rows still count as
  graded, so they suppress Bailing too — a graded row is a graded row).
- A shape with ≥6 grades but a flat trend → NEITHER call (grading a lot isn't bailing).

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `insights::tests::working_fires_on_a_rising_catalog_shape` | one Working line, numbers in text + evidence |
| AC1 (inclusive bars) | `insights::tests::working_bars_are_inclusive` | 6 grades / +0.15 exactly / 14 days still fire |
| AC2 | `insights::tests::working_needs_k_delta_and_recency` | each bar individually silences; NaN silent |
| AC3 | `insights::tests::working_window_excludes_lifetime` | plateau after ancient lows stays silent |
| AC4 | `insights::tests::cells_scores_and_junk_never_earn_working` | Momentum fires, Working doesn't; progression/score/junk silent |
| AC5 | `insights::tests::bailing_fires_on_dealt_and_abandoned` | counts carried; both ratio boundaries; newest-row recency; exclusivity |
| AC6 | `insights::tests::suggestions_are_deterministic_and_ordered` (extended) | 4-kind pinned order |
| AC7 | `commands::tests::working_suggestion_reaches_the_wire` | `"working"` DTO with cited numbers |
| edges | `insights::tests::empty_and_garbage_history_stay_silent` (existing) + AC2/AC4 tests | silence on empty/corrupt |

## 8. Architecture / approach

All logic in the Rust core (`insights.rs`), consumed through the existing #453 read
(`practice_suggestions_core`): the LLM recap gets the line via `history_prompt_block`
(grounded input), the offline recap appends the first suggestion, the coaching box shows
the analyzer's first line. Offline-first: pure local reads, zero network. Nothing touches
the audio thread.

## 9. Slice breakdown (ordered, each a shippable PR)

| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | Verdict contract (`shape_verdicts`) + Working recap line through #453 plumbing | `crates/brain/src/insights.rs`, `commands.rs` DTO arm | — | no |
| S2 | Explore deals consume verdicts: `TryPattern`/`DifferentScale` seeded picks prefer Working shapes, rest Bailing ones | `crates/brain/src/coach.rs`, `commands.rs` threading | S1 | no |
| S3 | Guided-coach ladder reads verdicts (`build_first`/`advance` material preference) — needs a design pass on how preference meets the fixed kind ladder | `crates/brain/src/coach.rs` | S1, S2 | no |

## 10. Risks / open questions

- Bars are tuning values in the shipped #453 style; they may need felt adjustment once the
  founder sees real lines. All are named constants.
- S3's interaction with the fixed drill-kind ladder (`kind_at`) is a design question —
  deferred to its own slice, not guessed here.

## 11. References

- Issue #303; epic #252; `crates/brain/src/insights.rs` (#453 S1); #453 S2+S3 plumbing
  (`practice_suggestions_core`, `history_prompt_block`); #445-4 stable chips.
