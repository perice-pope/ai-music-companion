# Spec: Streak + Daily Warmup Roulette (#257)

> Part of epic #252. Builds the daily ritual on the two foundations: F1 (RV generator engine)
> throws the challenge, F2 (Learner Model) remembers the streak. The streak transition is a pure,
> clock-free function so the timezone-correct math is fully unit-testable without a wall clock.

## 1. Summary
A practice **streak** plus a 60-second **Daily Warmup Roulette**: one tap throws a random key+scale
(via F1), the user plays it, gets a quick 0–1 score graded against the generated target, and the
streak advances. A small streak badge on the home surface makes the ritual visible.

## 2. Problem / why
The epic compounds sessions into a Learner Model, but nothing yet gives a player a reason to *return
tomorrow*. There is no daily hook, no "don't break the chain" loop, and no single-tap way to start a
short, structured warmup. Free play hears you but never throws you a quick, scored target. #257 adds
the lightweight daily ritual that drives retention; the streak number is the reward. F1 already
generates deterministic, gradable material (`generate(spec, seed) -> GeneratedSequence` with
`target_notes`) and F2 already owns `streak { count, last_completed_local_day }` and the pure
`apply_daily_completion(m, day, score)` transition — this feature wires them to a one-tap surface.

## 3. Non-goals
- No new pitch/scale detection or audio-thread work — grading reuses existing perception/scoring.
- No difficulty adaptation of the warmup, no leaderboards, no streak-freeze / repair purchases.
- No push notifications or background scheduling (no "remind me at 8am").
- No multi-day history chart — only the current streak count + today's done/not-done is surfaced.
- Does **not** redefine F1's `generate` or F2's `apply_daily_completion` signatures; it consumes them.

## 4. Contract / interface

### Local day (timezone correctness lives at the boundary)
```rust
/// A calendar day in the user's local timezone, expressed as the count of days
/// since the Unix epoch (1970-01-01) *in that timezone*. The instant → LocalDay
/// conversion happens exactly once, at the IPC/command boundary, using the OS
/// local offset. The pure core only ever sees this ordinal — so every streak
/// transition is deterministic and needs no clock and no tz database.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocalDay(pub i64);
```
The Learner Model already holds (F2):
```rust
pub struct Streak {
    pub count: u32,
    pub last_completed_local_day: Option<LocalDay>, // None until the first-ever completion
}
```
This feature adds, additively (forward-compatible blob), the most-recent daily result so the home
surface and idempotency rule can read it:
```rust
pub struct DailyWarmup {
    pub day: LocalDay,  // the day this result is for
    pub score: f32,     // best score recorded for that day, clamped to 0.0..=1.0
}
// new nullable field on LearnerModel: pub last_warmup: Option<DailyWarmup>  (serde default None)
```

### Streak transition (F2, pure — the heart of this spec)
```rust
/// Apply a completed warmup for local day `day` with grade `score` (clamped to
/// 0.0..=1.0). Pure + deterministic: same (model, day, score) → identical output.
pub fn apply_daily_completion(m: &LearnerModel, day: LocalDay, score: f32) -> LearnerModel;
```
**Exact streak math.** Let `prev = m.streak.last_completed_local_day`, `c = m.streak.count`,
`s = score.clamp(0.0, 1.0)`. The result's `streak.count` (`c'`), `last_completed_local_day` (`prev'`),
and `last_warmup` are:

| case | condition | `c'` | `prev'` | `last_warmup` |
|---|---|---|---|---|
| first ever | `prev == None` | `1` | `Some(day)` | `Some{day, s}` |
| same day (idempotent) | `Some(l)`, `day.0 == l.0` | `c` (unchanged) | `Some(l)` | `Some{day, max(s, prior_score_for_day)}` |
| consecutive | `Some(l)`, `day.0 == l.0 + 1` | `c.saturating_add(1)` | `Some(day)` | `Some{day, s}` |
| missed ≥1 day | `Some(l)`, `day.0 > l.0 + 1` | `1` (today restarts it) | `Some(day)` | `Some{day, s}` |
| stale / backward | `Some(l)`, `day.0 < l.0` | `c` (unchanged) | `Some(l)` | unchanged |

Notes that make it testable:
- **Reset rule:** a gap of a *full* calendar day (`day.0 > l.0 + 1`) resets the count to `1`, not `0` —
  today's completion is the first day of the new streak.
- **Consecutive** is *exactly* `l + 1`; **same day** is `== l`; nothing else advances.
- **Idempotent:** a second completion on the same `LocalDay` never changes `count` and never advances
  `prev'`; it only raises the recorded daily `score` to the max of attempts that day.
- **Backward day** (clock rollback / DST / out-of-order replay) is ignored: it neither decrements the
  count nor moves `prev'` backward — protects the streak from a wrong-direction clock.
- `count` uses `saturating_add` so it never overflows `u32`.
- The streak measures **showing up, not performance**: any completion counts regardless of `score`
  (even `0.0`), so a bad day still keeps the chain. `score` only feeds `last_warmup` / mastery.
- All other Learner Model fields are preserved unchanged (forward-compatible blob, F2 invariant).

### Roulette + grading (F1)
```rust
pub struct WarmupChallenge {
    pub spec: VariationSpec,         // a single random root (key) + one scale — one calm up-down
                                     // pass; the ~60s is the RITUAL's budget (throw → play →
                                     // score, S4 paces it), not the dealt material's length
                                     // (documented drift, S2 review)
    pub seed: u64,                   // echoed back so the grade is reproducible
    pub sequence: GeneratedSequence, // F1 output; `target_notes` is the grading target, `label` the UI string
}
/// Pure: deterministically derive one key+scale challenge from `seed`, then F1-`generate` it.
pub fn roulette(seed: u64) -> WarmupChallenge;
/// Grade played pitches against the generated target. 0.0..=1.0; tolerant of octave per existing scoring.
// Documented drift (S2): no `Pitch` type exists in the leaf crate and the DTOs are
// integer MIDI, so grading takes MIDI numbers — the target is F1's `target_midi`.
pub fn score_warmup(target_midi: &[u8], played_midi: &[u8]) -> f32;
```
`roulette` chooses the root + scale from a bounded catalog using only `seed` (no RNG, no I/O), so the
same seed always yields the same challenge. The command layer supplies a fresh seed per throw.

### IPC (command layer, `apps/desktop/src-tauri/src/commands.rs`)
- `start_daily_warmup() -> WarmupChallengeDto` — calls `roulette(fresh_seed)`; returns label + target
  for the score view. Read-only on the model.
- `complete_daily_warmup(seed, played_notes) -> WarmupResultDto` — grades via `score_warmup`,
  computes the **LocalDay at this instant from the OS local offset** (the only place a clock is
  read), then calls `apply_daily_completion`, persists the new Learner Model, and returns
  `{ score, streak: { count, completed_today } }`. Documented drift (S4): the spec originally
  returned `StreakDto` alone, but the score view must show the take's 0–1 grade ("gets a quick
  score", §1) — the badge DTO can't carry it (`last_warmup.score` is best-of-day, not this take)
  and re-deriving it client-side would put scoring in the frontend.
- `get_streak() -> StreakDto` — for the badge; `completed_today` = `last_warmup.day == today_localday`.

### Frontend (`apps/desktop/src/types/brain.ts` + components)
```ts
export interface StreakDto { count: number; completed_today: boolean }
export interface WarmupChallengeDto { seed: number; label: string; target_notes: number[] /* MIDI */ }
export interface WarmupResultDto { score: number; streak: StreakDto } // S4 drift, see above
```
- A small **StreakBadge** (flame + count) on the home surface; greyed when `completed_today` is false,
  lit when true.
- A one-tap **"Daily warmup"** entry that runs the ~60s flow (throw → play target → score → updated
  badge). Minimal UI; no settings.

## 5. Acceptance criteria (numbered, testable)
1. **First ever.** Given a Learner Model with `streak.last_completed_local_day == None`,
   `apply_daily_completion(m, LocalDay(d), s)` returns `count == 1` and `last_completed_local_day == Some(LocalDay(d))`.
2. **Consecutive day +1.** Given `last == LocalDay(d)` and `count == n`, completing on `LocalDay(d+1)`
   returns `count == n + 1` and `last == LocalDay(d+1)`.
3. **Idempotent same day.** Given `last == LocalDay(d)` and `count == n`, completing again on
   `LocalDay(d)` returns `count == n` (unchanged) and `last == LocalDay(d)` — completing twice in one
   day does not double-count.
4. **Reset after a missed day.** Given `last == LocalDay(d)` and `count == n`, completing on
   `LocalDay(d+2)` (one full day skipped) returns `count == 1` and `last == LocalDay(d+2)`. The same
   holds for any gap `> 1`.
5. **Backward day ignored.** Given `last == LocalDay(d)` and `count == n`, completing on
   `LocalDay(d-1)` returns `count == n` (unchanged) and `last == LocalDay(d)` (not moved backward).
6. **Determinism / purity.** `apply_daily_completion` called twice with identical `(m, day, score)`
   returns byte-identical models; it reads no clock and performs no I/O.
7. **Roulette is a real random key+scale via F1.** `roulette(seed)` returns a `WarmupChallenge` whose
   `sequence.target_notes` is non-empty and matches `generate(&spec, seed).target_notes`; two
   different seeds can yield different (root, scale) pairs, and a fixed seed always yields the same one.
8. **Scoring grades against the generated target.** `score_warmup(target, played)` returns `1.0` for a
   played sequence equal to `target_notes`, a strictly lower value for a partially-wrong sequence, and
   is in `0.0..=1.0` for any input.
9. **Completion writes both score and streak.** `complete_daily_warmup` updates the persisted Learner
   Model so that `last_warmup == Some{ day: today, score: graded }` **and** the streak reflects the
   AC1–5 transition for `today`; `get_streak` then reports the new `count` and `completed_today == true`.
10. **Score does not gate the streak.** A completion with `score == 0.0` still advances/initialises the
    streak exactly as a high score would (only `last_warmup.score` differs).
11. **Score clamped + best-of-day.** A `score` outside `0.0..=1.0` is clamped into range before storage;
    a second same-day completion stores `max(previous_day_score, new_score)`.
12. **Badge reflects state.** Given `StreakDto { count: 5, completed_today: true }` the StreakBadge shows
    "5" lit; given `completed_today: false` it renders greyed; tapping "Daily warmup" invokes
    `start_daily_warmup`.

## 6. Edge cases & failure modes
- **First run / empty model:** `None` last-day → `count == 1` (AC1); badge shows 0/greyed until first
  completion, never crashes.
- **Twice in one day:** idempotent (AC3); only best-of-day score updated (AC11).
- **Missed one day vs. many:** any gap `> 1` resets to `1` (AC4) — there is no partial credit.
- **Clock rollback / DST fall-back (a day appears to go backward):** ignored (AC5); the streak is never
  decremented and `last` never moves backward.
- **DST / travel across timezones:** the `LocalDay` is computed once at completion time from the OS
  local offset; a warmup finished after local midnight counts toward the new local day. The pure core
  is unaffected — it only sees the ordinal.
- **`u32` overflow:** `saturating_add` caps `count` at `u32::MAX` (defensive; ~11.7M years of streak).
- **Abandoned warmup (no notes played / quit early):** no `complete_daily_warmup` call → no write → no
  streak change. Only a finished warmup completes.
- **Empty `played` notes on completion:** grades to a low/zero score but still counts as a completion
  (AC10) — showing up is the ritual.
- **Offline:** entirely offline — `roulette`, `score_warmup`, and `apply_daily_completion` are pure
  Rust core; no network call anywhere in this feature.
- **Forward-compat:** a Learner Model saved before `last_warmup` existed loads with it `None` (serde
  default); roundtrip preserves unknown fields (F2 invariant).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::learner::tests::first_completion_starts_streak_at_one` | `None` → count 1, last set |
| AC2 | `brain::learner::tests::consecutive_day_increments` | `l+1` → count n+1 |
| AC3 | `brain::learner::tests::same_day_is_idempotent` | `== l` → count unchanged, no advance |
| AC4 | `brain::learner::tests::missed_day_resets_to_one` | gap >1 → count 1, last = today |
| AC5 | `brain::learner::tests::backward_day_ignored` | `< l` → count + last unchanged |
| AC6 | `brain::learner::tests::apply_daily_completion_is_pure_deterministic` | two calls byte-identical |
| AC7 | `variations::tests::roulette_matches_generate_and_is_seed_stable` | target ∈ F1 output; seed-stable; varies across seeds |
| AC8 | `variations::tests::score_warmup_perfect_partial_bounds` | 1.0 exact, lower partial, ∈[0,1] |
| AC9 | integration `commands::tests::complete_warmup_writes_score_and_streak` | model `last_warmup` + streak; `get_streak` reports it |
| AC10 | `brain::learner::tests::zero_score_still_advances_streak` | score 0 advances like any other |
| AC11 | `brain::learner::tests::score_clamped_and_best_of_day` | clamp to [0,1]; same-day keeps max |
| AC12 | `StreakBadge.test.tsx` | lit vs greyed by `completed_today`; tap invokes `start_daily_warmup` |
| edge: localday boundary | `commands::tests::localday_from_instant_uses_local_offset` | instant→LocalDay at the boundary only |
| edge: overflow | `brain::learner::tests::count_saturates_at_u32_max` | no panic/overflow |
| edge: forward-compat | `brain::learner::tests::model_without_last_warmup_loads` | absent field → None, roundtrip preserves |

## 8. Architecture / approach
All logic stays in Rust core per CLAUDE.md. The streak transition and `LocalDay` live in
`crates/brain/src/learner` alongside F2's existing `Streak` and `apply_daily_completion` (F2 declared
the signature; this slice fills in the exact math + the additive `last_warmup` field). The roulette +
grading live with F1 in `crates/variations` (a leaf crate), reusing `generate`/`target_notes` and the
existing scoring approach — no new music theory. The **only** clock read is at the command boundary,
which converts "now" to a `LocalDay` using the OS local offset and hands the ordinal to the pure core;
this keeps timezone correctness in one place and the transition wall-clock-free and unit-testable. The
new Learner Model field is nullable + serde-default (no migration; forward-compatible blob, persisted
locally and synced via the existing `profiles.learner_model jsonb`). Frontend is read-model + one tap:
a `StreakBadge` and a "Daily warmup" entry, both thin renderers over the three IPC commands. Fully
offline; nothing here is networked, so no new disclosure is required.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | `LocalDay` + exact `apply_daily_completion` streak math + additive `last_warmup` field (pure, fully unit-tested per AC1–6,10,11) | `crates/brain/src/learner*` | F2 | no |
| S2 | `roulette(seed)` + `score_warmup` over F1 (`target_notes`), seed-deterministic (AC7,8) | `crates/variations/**` | F1 | no |
| S3 | IPC: `start_daily_warmup` / `complete_daily_warmup` / `get_streak` + instant→`LocalDay` at boundary + persist (AC9, localday boundary test) | `apps/desktop/src-tauri/src/commands.rs`, `crates/brain` store glue | S1, S2 | no |
| S4 | Frontend StreakBadge + one-tap "Daily warmup" 60s flow + DTO types (AC12) | `apps/desktop/src/components/StreakBadge.tsx`, `DailyWarmup*.tsx`, `apps/desktop/src/types/brain.ts` | S3 | no |

Interface/seam slice = S1 (the `LocalDay`/transition contract) and S2 (the challenge/score contract);
S3 and S4 build behind them. S1 and S2 have disjoint footprints (brain vs variations) and merged deps
(F2, F1) → same wave; S3 then S4 serialize on the IPC shape.

## 10. Risks / open questions
- **Reset semantics:** chosen rule is "any missed full day resets to 1, today restarts the chain."
  No grace day / streak-freeze in this slice (a possible later kindness feature) — confirm product wants
  the strict rule for kids (strictness can feel punishing; flagged for #252 product review).
- **What counts as "completed":** finishing the warmup, independent of score (AC10). If product later
  wants a minimum-score gate, it's a one-line threshold in the command — but the streak stays a
  showing-up metric by design.
- **`LocalDay` at the boundary:** relies on the OS local offset at completion time; a user who manually
  changes their clock can game the streak. Accepted (low-stakes, single-user, offline); the backward-day
  guard (AC5) blocks the most common accidental case.
- **Roulette catalog scope:** start with the 12 roots × the common scales F1 already ships; expand later.

## 11. References
- Epic + foundations: `docs/specs/252-rv-practice-coach.md` (F1 `generate(spec, seed)` →
  `GeneratedSequence.target_notes`; F2 `Streak { count, last_completed_local_day }` +
  `apply_daily_completion(m, day, score)`; `profiles.learner_model jsonb`).
- Style sibling: `docs/specs/253-reveal-loop.md`.
- Existing: `crates/brain/src/store.rs` (forward-compatible JSON blob + local/Supabase persistence
  pattern), `apps/desktop/src/components/PracticeSession.tsx` (home/session surface for the badge +
  entry), `apps/desktop/src/types/brain.ts` (hand-maintained IPC type mirrors).
- Issue: #257.
