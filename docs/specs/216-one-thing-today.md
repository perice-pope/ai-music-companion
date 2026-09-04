# Spec: "Your one thing today" — the adaptive daily pick (#216)

## 1. Summary
On open, the app reads local practice history, names the **single
highest-leverage fix** with cited evidence, and deals a targeted
cell-through-the-row exercise for it — one card, one tap, fully
offline. Kills "what do I even practice?" paralysis. This spec covers
the epic's slice plan and pins S1 (the pure pick engine) in detail.

## 2. Problem / why
The longitudinal signal exists and is already mined for *sentences*
(#453's `insights::practice_suggestions` cites trends in the recap and
coaching box), but nothing turns the top finding into a *dealt
exercise*. The Daily Warmup (#257) deals by dice (`roulette(seed)`),
not by evidence. The founder's card (#216): "*Today: your 4th runs
flat — here's a 5-min fix.*"

## 3. Non-goals
- **No LLM anywhere in the substance.** Phrasing variety via the
  opt-in coaching path is a possible later polish; the pick, the
  evidence, and the exercise are deterministic local rules (v1 ships
  without any LLM involvement at all).
- **No groove/timing detector in v1.** Timing focus overlaps #421 S3
  "Coach's Rhythm" (in flight); a timing-weakness source joins as a
  later slice once that merges, sharing its insight reads.
- **No new persistence.** The pick is a pure read of what already
  accumulates (stored recaps' fingerprints + the Learner Model). No
  schema change, no new store writes.
- **No plan, no calendar.** One pick per day, recomputed from current
  history — never a stored multi-day program.

## 4. Contract / interface

New module `crates/brain/src/one_thing.rs` (+ `pub mod` line in
`lib.rs`). Pure and deterministic; no I/O, no clock reads.

```rust
/// One session's evidence row, extracted by the caller from
/// `Store::list_recent` + `load_recap` (fingerprint rides the recap).
pub struct SessionEvidence {
    pub started_at: DateTime<Utc>,
    pub instrument: String,
    pub intonation: Option<theory::IntonationSummary>,
}

pub enum FocusKind { DegreeTendency, KeyTrend }

pub struct DailyPick {
    pub kind: FocusKind,
    /// The card line: "Your 4th runs flat (−12¢ across 4 sessions)".
    pub headline: String,
    /// Compact citation (#453 style): sessions/dates/raw values.
    pub evidence: String,
    /// Final rank score — surfaced so tests and telemetry can assert
    /// the ranking, never shown to the user.
    pub leverage: f32,
    /// The dealt fix: cell × row, ready for `variations::generate`.
    /// Tempo rides where it already lives: `spec.rhythm.tempo_bpm`.
    pub spec: variations::VariationSpec,
}

pub fn daily_pick(
    history: &[SessionEvidence],
    key_mastery: &BTreeMap<String, learner::Mastery>,
    instrument: &str,
    fixed_pitch: bool,
    now: DateTime<Utc>,
    day_seed: u64,
) -> Option<DailyPick>
```

`None` means "no weakness clears its evidence bar" — the caller falls
back to the existing roulette warmup. Silence over lies (the
#453/#445-6b discipline): below any bar, the card must not fire.

### 4.1 Candidate sources and bars (all constants `pub` for tests)

**DegreeTendency** — the flagship ("your 4th runs flat"). From
`IntonationSummary.tendencies` across recent sessions:
- Consider `history` rows matching `instrument`, `started_at` within
  `EVIDENCE_WINDOW_DAYS = 21` of `now`, newest `MAX_SESSIONS = 20`,
  with `intonation: Some`.
- A session *testifies* for `(semitones_from_tonic, sign)` when that
  degree has `count >= DEGREE_MIN_COUNT (5)` and
  `|mean_cents| >= DEGREE_CENTS_BAR (10.0)` (bar sits below the 15¢
  in-tune tolerance on purpose: a *consistent* 10¢ lean is a tendency
  even when single notes pass).
- Candidate requires `>= DEGREE_MIN_SESSIONS (3)` same-sign testifying
  sessions AND zero opposite-sign testifying sessions — a degree that
  flips direction is not a stable tendency; say nothing.
- **Gated off entirely when `fixed_pitch`** (#389/#417-4: on piano the
  tendency is the instrument's tuning, not the player's ear —
  `coaching::fixed_pitch_family` is the caller's source of truth).
- Non-finite `mean_cents` rows are skipped defensively.

**KeyTrend** — a key the player keeps grading badly. From
`key_mastery`, reusing #453's shipped bars verbatim
(`insights::{TREND_MIN_ATTEMPTS, TREND_ACCURACY_BAR,
TREND_RECENT_DAYS}`): `attempts >= 5`, finite `accuracy_ewma < 0.6`,
`last_epoch_secs` within 14 days of `now`. Keys that don't parse as
`tonic:mode`, or whose mode has no `variations::ScaleType` mapping,
are skipped (can't deal what can't be named).

### 4.2 Leverage ranking (frequency × severity × recency — the
founder's formula, made exact)

- Degree: `frequency` = testifying / considered-with-intonation;
  `severity` = `min(1.0, mean of |mean_cents| over testifying / 25.0)`;
  `recency` = `0.5^(days since newest testifying session / 7.0)`.
- Key: `frequency` = `min(1.0, attempts as f32 / 10.0)`; `severity` =
  `(TREND_ACCURACY_BAR − accuracy_ewma) / TREND_ACCURACY_BAR`;
  `recency` = `0.5^(days since last_epoch_secs / 7.0)`.
- `leverage = frequency × severity × recency`; argmax wins.
- Ties (exact `f32` equality): DegreeTendency beats KeyTrend (more
  specific coaching); among degrees, lower `semitones_from_tonic`;
  among keys, `BTreeMap` (lexicographic) order. Fully deterministic.

### 4.3 The dealt exercise (cell-first — the RV north star)

The fix is a **cell rowed through 12 keys**, never a key-drill:
- DegreeTendency on semitone `s`: `cell = Some(vec![0, s, s, 0])` —
  approach, sit on the problem tone twice, resolve — over all 12
  chromatic roots, `randomize_roots: true` with `day_seed`, on the
  quarter grid (`notes_per_beat: 1`) at `DEGREE_TEMPO_BPM = 60.0`
  (slow enough to hear the lean).
- KeyTrend on `tonic:mode`: the mode's own scale pattern
  (`scale: Some(ScaleModifier)` from the mode's `ScaleType`), 12
  roots with the **weak tonic first** (RV
  keeps the first root fixed; the row still shuffles the rest via
  `day_seed`), `KEY_TEMPO_BPM = 80`. The weak key gets its reps *and*
  its eleven siblings — difficulty is row exposure, not "harder
  keys". Grid: `notes_per_beat: 1` at `KEY_TEMPO_BPM = 80.0`.

~5 minutes is a sizing intention, not a graded property: 12 roots × a
short cell at these tempos lands in the 3–6 minute band.

## 5. Acceptance criteria (numbered, testable)
1. Three recent same-instrument sessions each showing degree 5 with
   ≥5 observations at mean ≤ −10¢ (and no opposite-sign testimony) →
   `daily_pick` returns a `DegreeTendency` pick whose headline names
   the degree and direction, whose evidence cites the session count
   and mean cents, and whose spec is the `[0,5,5,0]` cell over 12
   randomized roots at 60 BPM.
2. Only two testifying sessions → `None` (below
   `DEGREE_MIN_SESSIONS`); the card stays silent rather than guess.
3. Three flat-testifying plus one sharp-testifying session on the same
   degree → that degree yields no candidate (direction flip = no
   stable tendency).
4. With no degree candidate and a `key_mastery` entry at
   `attempts = 6, accuracy_ewma = 0.4`, fresh `last_epoch_secs` →
   a `KeyTrend` pick whose spec's first root is the weak tonic and
   whose scale is the entry's mode.
5. Both candidate kinds present → the higher `leverage` wins; with
   equal leverage the degree wins (tie-break pinned by test).
6. `fixed_pitch = true` with AC1's history → no `DegreeTendency` pick
   (a qualifying KeyTrend may still fire).
7. Sessions of another instrument, sessions older than 21 days, and
   rows with `intonation: None` contribute nothing (AC1's evidence
   minus one qualifying row via each exclusion → `None`).
8. Same inputs and `day_seed` → byte-identical pick, including the
   generated root order; a different `day_seed` reorders the row but
   not the pick itself.
9. Empty history and empty `key_mastery` → `None`.
10. A non-finite `accuracy_ewma` or `mean_cents` never panics and
    never produces a candidate.

## 6. Edge cases & failure modes
- Empty/short history, no intonation ever measured → `None` (AC9/7).
- Corrupt stored recap (fingerprint absent) → row carries
  `intonation: None`, excluded, never invents a claim (AC7).
- NaN/∞ from legacy blobs → skipped (AC10).
- Direction-flipping degree → silence (AC3).
- All-strong player (nothing below any bar) → `None`; the surface
  keeps its dice — a player with no weakness gets roulette, not a
  fabricated concern.
- Mid-history instrument switch → only the current instrument's
  sessions count (AC7); `key_mastery` is instrument-agnostic today
  (F2's existing shape) and is used as-is.
- Clock skew (session `started_at` in the future) → treated as age 0,
  never negative-days panic.

## 7. Test plan
All in `crates/brain/src/one_thing.rs` `#[cfg(test)]` (pure module —
unit tests suffice; the IPC slice adds its own seam tests).
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `flat_fourth_across_sessions_fires_degree_pick` | kind, headline text, evidence numbers, exact cell/roots/tempo |
| AC2 | `two_sessions_stay_silent` | `None` below the session bar |
| AC3 | `direction_flip_disqualifies_degree` | `None` from mixed signs |
| AC4 | `weak_key_deals_its_mode_with_tonic_first` | first root pinned, scale = mode |
| AC5 | `higher_leverage_wins_and_tie_prefers_degree` | ranking + pinned tie-break |
| AC6 | `fixed_pitch_never_gets_intonation_critique` | degree gated, key still allowed |
| AC7 | `stale_other_instrument_and_absent_rows_excluded` | each exclusion flips AC1 to `None` |
| AC8 | `same_seed_identical_different_seed_reorders_row` | determinism both ways |
| AC9 | `empty_inputs_return_none` | `None`, no panic |
| AC10 | `non_finite_values_are_skipped` | no candidate, no panic |

## 8. Architecture / approach
Pure Rust in `crates/brain` (business logic never in the frontend).
S1 reads only types that already exist: `theory::IntonationSummary` /
`DegreeTendency`, `learner::Mastery`, `variations::VariationSpec`;
it writes nothing. Offline-first: zero network, nothing to add to the
allowlist or `ConnectionsPrivacy`. Not on the audio thread — no
allocation constraints beyond ordinary taste. Cell-first per
`docs/architecture/rv-methodology.md`: the deliverable is a cell × row
deal; key/degree evidence only *aims* it.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint | Depends on |
|---|---|---|---|
| S1 | `one_thing::daily_pick` — types, bars, ranking, dealt spec, the full test table above | `crates/brain/src/one_thing.rs`, `lib.rs` mod line | nothing in flight |
| S2 | IPC `get_daily_pick`: load evidence rows (`list_recent(20)` + `load_recap`), pass `fixed_pitch` via `coaching::fixed_pitch_family` (promoted from `pub(crate)` to `pub` here), DTO out; store failure → `None`, never an error | `commands.rs` (+ registration) | S1; **after** the in-flight `commands.rs` PRs (#490/#507/#519/#487) merge |
| S3 | The card on the Daily Warmup surface: pick present → "Today: …" + **Start today** (deals `spec` through the existing drill flow); pick absent or **Surprise me** → the #257 roulette, unchanged | `DailyWarmupPanel.tsx`, warmup store | S2, PR #490 merged |
| S4+ | Later: timing-weakness source (post-#421 S3), opt-in LLM phrasing variety | — | founder priority |

## 10. Risks / open questions
- **Surface placement** (S3): recommended — the pick *replaces* the
  roulette headline when it fires, "Surprise me" keeps the dice; the
  same evidence-over-dice-with-an-escape-hatch pattern the founder
  blessed for #421's Coach's Rhythm. Founder confirms at S3 review;
  S1 is placement-agnostic.
- **Streak interaction**: does completing the pick count as the daily
  warmup for #257's streak? Recommended yes (showing up is the
  ritual, whatever was dealt); decided at S2/S3, not load-bearing for
  S1.
- **Bar tuning**: `DEGREE_CENTS_BAR`/`DEGREE_MIN_SESSIONS` are
  reasoned starting values; real-history calibration may move them.
  They're `pub` constants with the test table pinned to them, so a
  retune is a one-line diff plus intentional test updates.

## 11. References
- Issue #216 (founder's card, UX, and leverage formula).
- `docs/specs/453-s1-history-analyzer.md` — the evidence-cited
  suggestion discipline and the trend bars S1 reuses.
- `docs/specs/257-streak-daily-roulette.md` — the surface S3 rides.
- `docs/architecture/rv-methodology.md` — cell × row × modifiers.
- `crates/brain/src/insights.rs`, `crates/theory/src/intonation.rs`,
  `crates/brain/src/learner.rs`, `crates/variations/src/lib.rs`.
