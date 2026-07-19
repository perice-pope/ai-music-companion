# Spec: the history analyzer — evidence-cited practice suggestions (#453 S1)

## 1. Summary
The first slice of the coach-learns-from-your-history loop: pure,
deterministic functions in `crates/brain` that read what already
accumulates locally (the exercise log + `key_mastery` EWMAs) and
return suggestions that CITE their own evidence. No LLM, no frontend —
a thin `practice_suggestions` command exposes the list so S2 (recap)
and S3 (the coaching box) can consume it.

## 2. Contract
- `brain::insights::PracticeSuggestion { kind: SuggestionKind, text:
  String, evidence: String }` with `SuggestionKind::{Trend, Neglect,
  Momentum}` (serde lowercase). `text` is the human sentence and
  EMBEDS its numbers (counts, days, percentages); `evidence` is the
  compact citation (source rows/keys, dates, raw values).
- `brain::insights::practice_suggestions(log, key_mastery, now) ->
  Vec<PracticeSuggestion>` — pure over inputs, deterministic order
  (Trend by mastery key, Neglect in fixed group order, Momentum by
  first appearance in the log).
- `brain::store::TimedExerciseLogEntry { logged_at: String, entry:
  ExerciseLogEntry }` + `Store::list_exercise_log_timed()` — the log
  WITH its RFC3339 write stamps (the untimed reader stays untouched).
  `logged_at` is treated as unparsed input: rows whose stamp fails
  `DateTime::parse_from_rfc3339` are skipped from all time-based
  analysis (defensive — a corrupt row must never invent a claim).
- Command `practice_suggestions` → `Vec<PracticeSuggestionDto { kind:
  String, text, evidence }>` (impl + wrapper + main.rs registration,
  `my_patterns` pattern: store failure → empty list, never errors).

## 3. Rules and their bars (silence > lies — the #445-6b discipline:
below any bar the rule returns NOTHING)

**Trend** (a key you keep playing and it isn't landing) — from
`key_mastery` alone:
- `TREND_MIN_ATTEMPTS = 5`: below 5 graded drills the EWMA (alpha
  0.3) is mostly priming noise; also stricter than the wheel's own
  `OWNED_MIN_ATTEMPTS` (3), so we never claim a trend on thinner
  evidence than "owned" needs.
- `TREND_ACCURACY_BAR = 0.6`: the founder's number ("below 60%");
  far under `OWNED_ACCURACY_THRESHOLD` (0.85), leaving a quiet middle
  where neither praise nor concern fires.
- `TREND_RECENT_DAYS = 14`: `last_epoch_secs` must be within 14 days
  of now — a weak key you stopped playing is history, not a trend.
- Mastery keys that don't parse as `tonic:mode`, or non-finite EWMAs,
  are skipped (can't cite what can't be named).

**Neglect** (a whole tonic side gone dark while you practiced) — from
the timed log:
- Groups: `the flat keys` [1,3,5,8,10] (Db,Eb,F,Ab,Bb) and `the sharp
  keys` [2,4,7,9,11] (D,E,G,A,B). C (0) and F#/Gb (6) sit on the seam
  of both sides and are claimed by neither — either claim about them
  would be contestable.
- `NEGLECT_MIN_DAYS = 14`: the founder's "two weeks".
- `NEGLECT_CONTRAST_MIN_ROWS = 8`: while the group sat idle, at least
  8 rows landed in OTHER pitch classes within those 14 days — without
  the contrast the whole log is idle and silence wins (an absent
  player owes no flat keys).
- Two citable forms: the group has older rows → "last one N days
  ago"; the group has NO rows ever but the oldest parseable row is
  ≥14 days old → "not once in your N-day log". A log younger than 14
  days can never fire neglect.

**Momentum** (a cell of yours that jumped) — from the timed log:
- Cell identity = `spec.cell` offsets with ≥2 notes (the same line My
  Patterns draws: catalog drills and single notes aren't "your"
  cells). Graded rows only.
- `MOMENTUM_WINDOW_ROWS = 12`: the delta is computed over the cell's
  most recent ≤12 graded rows — the claim is about now, not the
  cell's lifetime.
- `MOMENTUM_MIN_GRADED = 6` (K): newer-half vs older-half means need
  ≥3 rows each so neither half rests on one lucky take (the same
  halving discipline as `ShapeInsight::accuracy_trend`, which
  refuses below 4).
- `MOMENTUM_MIN_DELTA = +0.15` (X): newer-half mean minus older-half
  mean; 15 points clears normal grade jitter (the issue's 55%→80%
  example clears it three times over).
- `MOMENTUM_RECENT_DAYS = 14`: the newest graded row must be within
  14 days — no celebrating ancient history.

## 4. Non-goals
Recap integration (S2), the coaching box (S3), habit-shape analysis,
taste/goal tie-in (S4), any frontend, any LLM narration, any cap or
per-session cadence (S3 owns cadence).

## 5. ACs
1. Trend fires only above ALL bars: ≥5 attempts AND ewma <0.60 AND
   last attempt ≤14 days — each bar individually silences it; the
   text carries attempts, percentage, and days-ago.
2. Neglect needs the contrast: group absent ≥14 days AND ≥8 recent
   rows elsewhere; either missing → silence; a recently-practiced
   group → silence; text carries days and the contrast count. The
   never-practiced form fires only when the log itself spans ≥14 days.
3. Momentum needs K: ≥6 graded rows for one cell, half-delta ≥+0.15,
   newest ≤14 days old; fewer rows, smaller delta, or a stale newest
   row → silence; text carries both half-percentages and the row count.
4. Empty log + empty mastery → empty vec. Rows with garbage
   `logged_at` or unparseable spec_json feed no rule (a neglect/
   momentum that would fire off garbage stamps stays silent).
5. Deterministic: identical inputs → identical output, pinned order.
6. `list_exercise_log_timed` returns the same rows as
   `list_exercise_log` plus an RFC3339-parseable `logged_at`.
7. The command returns DTOs (kind as lowercase string) and never
   errors: empty state → empty list; a seeded momentum history +
   below-bar mastery surfaces exactly the earned suggestions.

## 6. Test map
| AC | Test |
|---|---|
| 1 | `insights::tests::trend_fires_only_above_every_bar` |
| 2 | `insights::tests::neglect_needs_absence_and_the_contrast` |
| 3 | `insights::tests::momentum_needs_k_rows_delta_and_recency` |
| 4 | `insights::tests::empty_and_garbage_history_stay_silent` |
| 5 | `insights::tests::suggestions_are_deterministic_and_ordered` |
| 6 | `store::tests::timed_exercise_log_carries_parseable_stamps` |
| 7 | `commands::tests::practice_suggestions_command_cites_or_stays_silent` |

## 7. Architecture
Extends `crates/brain/src/insights.rs` (the existing pure-analysis
module over `ExerciseLogEntry`) — same crate as `learner::Mastery`,
so the fn takes `&BTreeMap<String, Mastery>` directly. `now` is a
parameter (purity/determinism); the command injects `Utc::now()`.
Offline-first: reads sessions.db only, no network. References:
issue #453, docs/specs/445-thin-recap.md (the evidence-bar
discipline), #419 S3 (`my_patterns_impl` — the command pattern and
the cell line), #252 (`insights.rs`, the log).
