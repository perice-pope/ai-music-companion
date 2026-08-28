# Spec: Boss moments — assemble drills into a musical payoff + backing band (#259)

> Part of epic #252. The capstone of the practice loop: it turns a run of reps into a short piece of
> *music*. Builds on **F1** (RV generator), **F2** (Learner Model), and the **#212** follow-me band.

## 1. Summary
After a player drills a few RV variations well, the AI occasionally assembles what they just practiced
into a short, named **musical moment** — "play these three in a row: that's a ii–V–I in three keys" —
composed from the recent RV material with concrete target notes, scored against their play, with the
existing backing band (#212) joining in the detected key and tempo. It is the reward for the reps and
the proof that drills add up to music. Rate-limited so it stays special; degrades gracefully with no
band when accompaniment is unavailable.

## 2. Problem / why
The adaptive coach (#254) gives the player drills to grind, but a drill is a means, never the payoff —
nothing in the loop yet says "all those reps were *for* something." There's no moment where the
practiced material resolves into a phrase that sounds like music, and the one asset that would make it
feel like music — the follow-me band (#212) — never fires inside a guided drill. This feature closes
that gap: it harvests the just-drilled RV material (F2), composes a musical moment from it (F1), drops
the band in underneath, and marks the win in the Learner Model for the collection/identity surfaces
(#256/#258). It is the dopamine cap on the epic's core loop.

## 3. Non-goals
- No new RV theory: moments are **composed from F1 output**, not a second generation engine. A moment
  re-orders / re-keys / concatenates `VariationSpec`s the user already drilled; it never invents new
  variation *types*.
- No new audio engine or backing-band sounds — reuse #212's accompaniment synth/controls verbatim.
- No new score renderer — reuse `ScoreView`; moment `ticks` are the shape it already consumes.
- No LLM dependence: trigger, composition, and scoring are pure Rust core and work fully offline. (The
  card's celebratory *copy* may be enriched later behind the existing coaching opt-in; out of scope here.)
- Not a leaderboard / sharing feature; the only persistence is the F2 "moment achieved" marker.
- No more than one boss moment on screen, and never one mid-moment.

## 4. Contract / interface

### Backend — `brain::coach` (new `moments` submodule)
Trigger + composition are **pure and deterministic** (seeded; no I/O), mirroring F1/F2.

```rust
/// The recent, already-scored RV material a moment can be built from. Sourced from the
/// session/Learner Model (F2); most-recent-first. Carries the F1 spec so composition can re-key it.
pub struct DrillHistory {
    pub recent: Vec<DrilledItem>,
}
pub struct DrilledItem {
    pub spec: VariationSpec,   // the F1 spec the user just drilled
    pub key: KeyScale,         // the key/scale it was drilled in
    pub accuracy: f32,         // 0..1, from `scoring`
    pub completed_at: Timestamp,
}

pub struct MomentConfig {
    pub min_qualifying_drills: u32, // how many well-drilled items must precede a moment (default 3)
    pub min_accuracy: f32,          // an item "qualifies" only at/above this accuracy (default 0.7)
    pub window: Duration,           // rate-limit: at most one moment per window (default e.g. 10 min)
}

pub struct BossMoment {
    pub label: String,             // human title, e.g. "ii–V–I in three keys"
    pub sequence: GeneratedSequence, // F1 shape: ticks (ScoreView) + target_notes (grading) + label
    pub key: KeyScale,             // key the band plays in
    pub tempo_bpm: f32,            // tempo the band locks to
    pub concept: ConceptKey,       // stable collection key, e.g. "ii-V-i-three-keys"
}

/// Pure + deterministic. Returns `Some` only when BOTH the trigger condition is met
/// (≥ `min_qualifying_drills` qualifying items available to compose a coherent moment)
/// AND the rate limit allows it (`now - last_moment_at >= window`, or no prior moment).
/// Returns `None` otherwise (not enough material, low accuracy, or within the window).
pub fn maybe_compose_moment(
    history: &DrillHistory,
    last_moment_at: Option<Timestamp>,
    now: Timestamp,
    cfg: &MomentConfig,
    seed: u64,
) -> Option<BossMoment>;
```

### F2 — Learner Model transition (`brain::learner`)
```rust
/// Additive, pure transition. Records a "moment achieved" entry into the collection
/// (deduped by `concept`, count incremented) for the collection (#256) / identity (#258)
/// surfaces. `score` is the user's grade on the moment (0..1).
pub fn apply_moment_achieved(
    m: &LearnerModel,
    moment: &BossMoment,
    score: f32,
    now: Timestamp,
) -> LearnerModel;
```

> **S2 shipping note.** The landed signature is
> `apply_moment_achieved(model, concept: &str, label: &str, score: f32, now_epoch_secs: i64)` —
> the sketch's `&BossMoment` unbundles into the two fields the marker persists, so S2 lands in
> parallel with S1 (disjoint footprints, no dependency on the unmerged type); S3 passes
> `moment.concept` / `moment.label`. The marker lives in its own additive
> `LearnerModel.moments: BTreeMap<concept, MomentAchieved>` map (label, best score,
> first-achieved, count), NOT inside the reveal `collection` — reveals key by
> `(concept, connection)` and feed `collection_size()`, so a moment entry there would inflate
> the "N in your collection" surfaces (#256). Repeat semantics: count bumps, `first_achieved`
> kept, `best_score` = best ever (a worse retry can't lower it), `label` refreshed to the
> latest achievement.

### IPC / commands (`apps/desktop/src-tauri/src/commands.rs`)
- New event `boss-moment` (payload: `label`, MusicXML or `ticks` for `ScoreView`, `key`, `tempo_bpm`,
  `concept`, `band_available: bool`) emitted at most once per window when `maybe_compose_moment`
  returns `Some`.
- New command `start_boss_moment` — begins scoring the moment's `target_notes` (via the existing
  score-follow/scoring path) and **best-effort** starts the band in `key`/`tempo_bpm` by reusing the
  #212 accompaniment start + `set_key` / `set_clock`. Band failure must not abort the moment.
- New command `complete_boss_moment` (or a `moment-result` event) — on finish, computes the moment
  score, stops the band, and applies `apply_moment_achieved` to the Learner Model.
- Reuses `accompaniment-status` for band state; no new band IPC philosophy.

### Frontend (`apps/desktop/src/components/BossMomentCard.tsx`)
- Subscribes to `boss-moment`; renders one restrained, celebratory card: the `label`, the moment in a
  reused `ScoreView` (cursor follows `score_position`), and the existing band controls
  (`AccompanimentToggle`). Auto-dismiss on completion; never stacks.
- When `band_available` is `false`, the card renders and the moment plays/scores normally with the band
  control shown disabled (with a calm "band unavailable" hint) — never blocks the moment.

## 5. Acceptance criteria (numbered, testable)
1. Given a `DrillHistory` with ≥ `min_qualifying_drills` items at/above `min_accuracy` and no prior
   moment, `maybe_compose_moment` returns `Some(BossMoment)` whose `sequence.target_notes` is non-empty
   and whose `sequence.ticks` are derived from the supplied drilled `VariationSpec`s (composed from
   recent RV material, not invented).
2. The returned moment carries a concrete musical `label` and a stable `concept`, and its `key`/
   `tempo_bpm` are derived from the recent material / detected clock (so the band can play it).
3. Given fewer than `min_qualifying_drills` qualifying items (too few, or accuracy below
   `min_accuracy`), `maybe_compose_moment` returns `None` (no moment).
4. Given `last_moment_at` such that `now - last_moment_at < cfg.window`, `maybe_compose_moment` returns
   `None` even when material qualifies — at most one moment per window.
5. Given `now - last_moment_at >= cfg.window` (or `last_moment_at == None`) with qualifying material, a
   moment is returned (window has elapsed).
6. `maybe_compose_moment` is deterministic for a fixed `(history, last_moment_at, now, cfg, seed)`:
   same inputs → identical `BossMoment` (same `ticks`, `target_notes`, `label`, `key`, `tempo_bpm`).
7. Starting a boss moment with accompaniment available starts the band in the moment's `key` and
   `tempo_bpm` (asserted via the accompaniment control: `set_key`/`set_clock` receive those values) and
   scores the user's play against `sequence.target_notes`.
8. If accompaniment is unavailable (start fails or output device absent), the moment still emits, the
   card still renders, and the moment is still scored — no band, no error to the user (`band_available`
   is `false`; the moment is not aborted).
9. On completing a moment, `apply_moment_achieved` adds/updates exactly one collection entry keyed by
   `concept` (first time: inserted with `count = 1`; repeat of the same concept: `count` incremented,
   not duplicated) and the transition preserves all other Learner Model fields (additive).
10. Frontend: on a `boss-moment` event a single `BossMomentCard` renders the `label`, a `ScoreView` of
    the moment, and band controls; a second event replaces (does not stack) the card.

## 6. Edge cases & failure modes
- **Empty / first-run history** (no drills yet) → `maybe_compose_moment` returns `None`; no card.
- **All recent drills below `min_accuracy`** → `None` (a moment celebrates *well*-drilled material).
- **Exactly `min_qualifying_drills`** qualifying items → boundary fires (`>=`, tested).
- **Rate-limit boundary** `now - last_moment_at == window` → allowed (`>=`); one tick under → blocked.
- **Accompaniment unavailable** (no output device, band busy, start error) → graceful degradation per
  AC8; band failure is swallowed and surfaced as a disabled control, never an exception.
- **Repeated identical concept** (same moment achieved twice) → collection dedupes by `concept`,
  increments `count` (AC9) — no duplicate entries.
- **Tempo/key not yet locked** (live clock unlocked, no confident key) → use the most-recent drilled
  item's key and a sane default tempo for the moment; the band still follows once the clock locks.
- **Mid-moment trigger** → no new moment is composed while one is active (single-moment invariant).
- **Offline** → trigger, composition, scoring, band (local synth), and the F2 marker all work with zero
  network; nothing here depends on the LLM.
- **Schema drift** → the new collection marker rides the existing versioned, preserve-unknown-fields
  blob (F2 invariant); a roundtrip test guards it.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::coach::moments::tests::composes_from_recent_material` | `Some`, non-empty `target_notes`, ticks derived from supplied specs |
| AC2 | `tests::moment_carries_label_concept_key_tempo` | label/concept present; key+tempo derived for band |
| AC3 | `tests::too_few_or_low_accuracy_returns_none` | `None` below qualifying threshold |
| AC4 | `tests::rate_limited_within_window` | `None` when within `window` despite qualifying material |
| AC5 | `tests::fires_after_window_elapsed` | `Some` at/after window (and when no prior moment) |
| AC6 | `tests::moment_is_deterministic` | same `(inputs, seed)` → identical `BossMoment` |
| AC7 | integration `start_boss_moment_starts_band_in_key_tempo` | `set_key`/`set_clock` get moment key+tempo; scoring runs vs `target_notes` |
| AC8 | integration `moment_degrades_without_band` | band-unavailable → moment emits/renders/scores; `band_available=false`; no abort |
| AC9 | `brain::learner::tests::moment_marker_dedupes_and_counts` | first insert `count=1`; repeat increments; other fields preserved |
| AC9 (drift) | `brain::learner::tests::roundtrip_preserves_unknown` (extends F2) | marker survives version roundtrip |
| AC10 | `BossMomentCard.test.tsx` | renders label + ScoreView + band controls; second event replaces |
| Edge: empty history | `tests::empty_history_returns_none` | no moment on first run |
| Edge: boundary count | `tests::exact_threshold_fires` | `>=` boundary fires |
| Edge: window boundary | `tests::window_boundary_inclusive` | `== window` allowed; one under blocked |
| Manual | checklist | drill 3 keys → moment card appears with band; pull audio device → moment still plays bandless |

## 8. Architecture / approach
All decision-making stays in Rust core per CLAUDE.md. **Trigger + composition** live in `brain::coach`
(new `moments` submodule, next to #254's coach logic) and are pure/deterministic: they read a
`DrillHistory` assembled from the session / Learner Model (F2), call **F1** `generate` to realize the
composed `VariationSpec`s into a `GeneratedSequence`, and emit a `BossMoment`. The rate limiter is a
single `last_moment_at` timestamp compared against `cfg.window` — no second cadence source. **Scoring**
reuses the existing score-follow + `scoring` path against `target_notes`. The **band** reuses #212's
accompaniment verbatim: `start_boss_moment` calls the existing accompaniment start and pushes the
moment's key/tempo via `AccompanimentSender::set_key` / `set_clock`; the start is best-effort and any
failure is caught so the moment proceeds bandless (graceful degradation). The **F2 marker** is an
additive transition (`apply_moment_achieved`) writing one deduped collection entry, riding the existing
versioned JSONB blob + Supabase sync — no migration, no breaking change. The frontend is render-only:
`BossMomentCard` reuses `ScoreView` and `AccompanimentToggle`; no business logic crosses into TS. Fully
offline-first — no new outbound call, nothing to add to the network allowlist.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | `brain::coach::moments`: `DrillHistory`/`MomentConfig`/`BossMoment` types + `maybe_compose_moment` (trigger + rate limit + F1-backed composition), pure & deterministic, unit-tested | `crates/brain/src/coach/moments.rs`, `crates/brain/src/coach/mod.rs` | F1, F2 | yes |
| S2 | F2 marker: `apply_moment_achieved` (dedup + count, additive, roundtrip-safe) | `crates/brain/src/learner*` | F2, S1 | no |
| S3 | IPC + band wiring: `boss-moment` event, `start_boss_moment` / `complete_boss_moment` commands, best-effort band start reusing #212 (`set_key`/`set_clock`), graceful degradation | `apps/desktop/src-tauri/src/commands.rs`, `main.rs`/`lib.rs` registration | S1, S2, #212 | yes |
| S4 | `BossMomentCard` frontend: subscribe to `boss-moment`, reuse `ScoreView` + `AccompanimentToggle`, restrained celebratory styling, single-card / replace behavior, disabled-band hint | `apps/desktop/src/components/BossMomentCard.tsx`, store + `App.tsx` wiring | S3 | no |

Suggested waves: S1 and S2 can land in parallel behind F1/F2 (S2 touches `learner`, S1 touches
`coach`); S3 then S4 serialize on the IPC contract.

## 10. Risks / open questions
- **What makes a *coherent* moment** vs an arbitrary concatenation — the composition rule (e.g. "same
  variation type across N keys" → ii–V–I-style payoff) needs a small, curated set of moment *shapes*;
  start with one or two well-formed shapes and expand. Tracked here.
- **Window length + qualifying threshold** are tuning values; defaults in `MomentConfig` are a starting
  point, to be felt out in manual practice (too frequent kills the specialness; too rare and it never
  fires).
- **Key/tempo for the band** when the live clock is unlocked or key is low-confidence — fallback policy
  (most-recent drilled key + default tempo) needs confirmation against #212's follow behavior.
- **"Achieved" semantics** — is a moment "achieved" on *attempt* or only above a score bar? Current
  contract records the marker on completion with the achieved `score`; whether a low score still counts
  toward identity (#258) is an open product call.

## 11. References
- Epic + foundations: `docs/specs/252-rv-practice-coach.md` (F1 `generate`/`GeneratedSequence`,
  F2 `LearnerModel`/`apply_*`/collection), `docs/specs/253-reveal-loop.md` (rate-limit + card pattern).
- Issues: #259 (this), #252 (epic), #212 (follow-me accompaniment), #254 (adaptive coach — shares
  `brain::coach`), #256/#258 (collection / "your sound" — consume the moment marker).
- Existing code: `crates/brain/src/accompaniment.rs` (`AccompanimentSynth`, `AccompanimentDriver`,
  `AccompanimentSender::set_key`/`set_clock`, `accompaniment_control_channel`),
  `apps/desktop/src-tauri/src/commands.rs` (`start_accompaniment`/`stop_accompaniment`,
  `accompaniment-status` event, `AppState`), `crates/brain/src/scoring.rs` + `follower.rs` (scoring /
  score-follow), `apps/desktop/src/components/ScoreView.tsx`, `AccompanimentToggle.tsx`.
