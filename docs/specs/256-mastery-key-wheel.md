# Spec: Mastery — the 12-key wheel (#256)

> Part of epic #252. A **read-only** view over the F2 Learner Model. Builds nothing new in the write
> path; its whole job is to make invisible progress visible. Depends on F2 (`brain::learner`).

## 1. Summary
A circle-of-fifths wheel that lights each of the 12 keys by its mastery state (none / learning /
owned) derived from F2 `key_mastery`, shows which scales the player has unlocked, and trends
intonation/tone over time from stored fingerprints. Tap a key for its detail. It is the "come back
tomorrow" surface — for the practicing musician, beginner→advanced.

## 2. Problem / why
F2 accumulates `key_mastery` (attempts, EWMA accuracy, `owned`) and per-session `Fingerprint`s, but
nothing surfaces that growth. Progress is real and stored yet invisible, so there's no felt reason to
return. #256 is the visualization: a calm wheel that says "you own these keys now, here's what's
opening up, your tone is steadying." It is a pure read — it must never change mastery or write the
Learner Model (writes belong to #254/#255/#257 per epic #252).

## 3. Non-goals
- **No writes.** This feature never mutates `key_mastery`, `collection`, or any Learner Model field.
- No new perception/detection, no scoring, no drill logic — it consumes F2 outputs only.
- No re-derivation of accuracy: the wheel reads F2's `Mastery`; it does not recompute EWMA from raw
  attempts.
- No animation/celebration choreography beyond a simple state render (a "just owned" reveal moment is
  a later polish, not this slice).
- No change to the `owned` threshold rule itself — that rule lives in F2 (`apply_drill_result`); the
  wheel's classifier reads the same constants but does not own them.

## 4. Contract / interface

### Backend — pure selector in Rust core (`brain::learner::wheel`)
All derivation (state classification, scales-unlocked, trend direction) is a **pure, deterministic,
read-only** function over an F2 `LearnerModel` plus the user's stored `Fingerprint` history. No I/O,
no clock reads inside the classifier (any "now" is passed in).

```rust
/// Mastery state for one key, derived from F2 `Mastery`.
pub enum KeyState { None, Learning, Owned }

pub struct KeyCell {
    pub key: KeyScale,            // F2 key identity (root pitch-class + scale)
    pub state: KeyState,
    pub attempts: u32,            // mirrored from F2 Mastery.attempts
    pub accuracy_ewma: f32,       // mirrored from F2 Mastery.accuracy_ewma (0..1)
    pub scales_unlocked: Vec<String>, // scale names seen owned/learning for this root
}

pub enum Trend { Improving, Steady, Slipping, Unknown } // Unknown = too few fingerprints

pub struct WheelView {
    pub cells: [KeyCell; 12],     // one per chromatic root, ordered circle-of-fifths in the view layer
    pub intonation_trend: Trend,  // from stored fingerprints' IntonationSummary.mean_abs_cents
    pub tone_trend: Trend,        // from stored fingerprints' ToneDescriptor (core_clarity)
    pub total_owned: u8,          // 0..=12, convenience for the empty/summary copy
}

/// Pure read. `now` only feeds recency display, never classification.
pub fn build_wheel(model: &LearnerModel, fingerprints: &[Fingerprint], now: Timestamp) -> WheelView;

/// The single classification rule, isolated for unit testing. Reuses F2's owned threshold/min-attempts.
pub fn classify(m: &Mastery) -> KeyState;
```

**Classification rule (the one testable rule):**
- `Owned` ⇔ `m.owned` is true in F2 **and** `m.attempts >= MIN_ATTEMPTS` **and**
  `m.accuracy_ewma >= OWNED_THRESHOLD` (the same `MIN_ATTEMPTS` / `OWNED_THRESHOLD` constants F2 uses
  to flip `owned`, imported, not redefined — the wheel must agree with F2, never drift).
- `Learning` ⇔ `m.attempts >= 1` and not `Owned`.
- `None` ⇔ `m.attempts == 0` (or the key is absent from `key_mastery`).

**Trend rule:** over the last `K` fingerprints that carry the dimension, compare the mean of the
older half vs. the newer half. Intonation improves when `mean_abs_cents` **decreases**; tone improves
when `core_clarity` **increases**. Difference within `EPS` ⇒ `Steady`; fewer than `MIN_TREND_POINTS`
qualifying fingerprints ⇒ `Unknown`.

### IPC
- One read command `get_mastery_wheel() -> WheelView` (no event stream; the wheel is a snapshot the
  page fetches on mount and on return to the screen). Thin JSON, snake_case, mirrored in
  `apps/desktop/src/types/brain.ts` with a roundtrip assertion (per the brain.ts drift rule).

### Frontend — `apps/desktop/src/components/KeyWheel.tsx` (+ `KeyWheelDetail.tsx`)
- Renders a clean SVG: 12 segments laid out in **circle-of-fifths** order (C, G, D, A, E, B, F#/Gb,
  Db, Ab, Eb, Bb, F), each filled by `KeyState` (3 calm fills + a `none` resting fill).
- Below/around the wheel: a "scales unlocked" count and the two trend glyphs (intonation, tone) with
  hedged copy; `Unknown` trend renders as a quiet "not enough yet", never a fake arrow.
- Tap a segment → `KeyWheelDetail` panel for that key: state label, attempts, accuracy, scales
  unlocked, recency. Keyboard-focusable; `aria-label` per segment.
- Empty/first-run: all 12 cells `None`, `total_owned == 0` → a defined empty state ("Your keys light
  up as you own them") — no crash, no fabricated progress.
- Presentation only — no thresholds, no trend math, no classification in TS (all from `WheelView`).

## 5. Acceptance criteria (numbered, testable)
1. Given a `LearnerModel` whose `key_mastery` has a key with `owned==true`, `attempts >= MIN_ATTEMPTS`
   and `accuracy_ewma >= OWNED_THRESHOLD`, `classify` returns `Owned` and that cell renders the owned
   fill.
2. Given a key with `attempts >= 1` but not meeting the owned rule, `classify` returns `Learning`.
3. Given a key with `attempts == 0`, or a key absent from `key_mastery`, `classify` returns `None`.
4. Given an owned `Mastery` whose `attempts < MIN_ATTEMPTS` (or `accuracy_ewma < OWNED_THRESHOLD`),
   `classify` returns `Learning`, not `Owned` (the wheel never over-claims past F2's gate).
5. `build_wheel` returns exactly 12 `KeyCell`s, one per chromatic root, and `total_owned` equals the
   count of `Owned` cells.
6. The frontend lays the 12 cells out in circle-of-fifths order (C at top, then clockwise by fifths),
   independent of the `cells` array order.
7. `scales_unlocked` for a cell lists exactly the scale names that key has at `Owned`/`Learning`
   state in F2 (deduped, stable order); a key with none shows an empty list.
8. Given ≥ `MIN_TREND_POINTS` fingerprints with decreasing `mean_abs_cents` newer-vs-older,
   `intonation_trend == Improving`; with increasing, `Slipping`; within `EPS`, `Steady`.
9. Given fewer than `MIN_TREND_POINTS` qualifying fingerprints, the corresponding trend is `Unknown`
   and the UI renders the "not enough yet" state (no arrow).
10. Given an **empty** `LearnerModel` (cold start) and no fingerprints, `build_wheel` returns 12
    `None` cells, `total_owned == 0`, both trends `Unknown`, and the page renders the defined empty
    state without error.
11. `build_wheel`/`classify` are deterministic and **read-only**: called twice on the same inputs
    they return equal `WheelView`s and leave the `LearnerModel` unchanged (no write path touched).
12. Tapping a segment opens `KeyWheelDetail` for that exact key showing its state, attempts, accuracy
    and scales-unlocked; the wheel never shows more than the tapped key's detail at once.

## 6. Edge cases & failure modes
- **First run / empty model** → AC10: all `None`, empty state, no guess.
- **Key present but `attempts == 0`** (initialized, never drilled) → `None`, same as absent.
- **`owned==true` but stats below the gate** (e.g. stale/migrated blob) → `Learning`; the wheel
  trusts the numeric gate over a possibly-stale flag, and never renders owned without the evidence.
- **Forward-compat blob** (unknown F2 fields, newer version) → ignored by the read; the wheel renders
  from the fields it knows (relies on F2's preserve-unknown roundtrip).
- **Fingerprints missing a dimension** (no intonation or no tone on some sessions) → only qualifying
  fingerprints count toward that trend; below `MIN_TREND_POINTS` → `Unknown` (AC9).
- **NaN/garbage `accuracy_ewma`** → treated as not meeting the owned threshold → `Learning`/`None`,
  never `Owned` (defensive: a NaN comparison is false, so it fails the `>=` gate by design — assert).
- **All 12 owned** → `total_owned == 12`, summary copy handles the maxed state.
- **Offline** → entirely local read; no network anywhere in this feature (nothing to gate).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::learner::wheel::tests::owned_classifies_owned` | meets gate → `Owned` |
| AC2 | `wheel::tests::attempted_classifies_learning` | ≥1 attempt, not owned → `Learning` |
| AC3 | `wheel::tests::zero_or_absent_classifies_none` | 0 attempts / absent → `None` |
| AC4 | `wheel::tests::owned_below_gate_is_learning` | `owned` flag but under min/threshold → `Learning` |
| AC5 | `wheel::tests::build_wheel_has_12_cells_and_owned_count` | 12 cells; `total_owned` correct |
| AC6 | `KeyWheel.test.tsx::lays_out_circle_of_fifths` | segment order C→G→D…→F regardless of input order |
| AC7 | `wheel::tests::scales_unlocked_lists_owned_and_learning` | exact deduped scale names per key |
| AC8 | `wheel::tests::intonation_trend_direction` | decreasing/increasing/flat cents → Improving/Slipping/Steady |
| AC9 | `wheel::tests::sparse_fingerprints_trend_unknown` + `KeyWheel.test.tsx::renders_not_enough_yet` | `Unknown` + UI no-arrow |
| AC10 | `wheel::tests::empty_model_all_none` + `KeyWheel.test.tsx::empty_state` | cold start render |
| AC11 | `wheel::tests::build_wheel_is_pure_and_readonly` | equal output twice; input model unchanged |
| AC12 | `KeyWheelDetail.test.tsx::tap_opens_single_key_detail` | tap → that key's detail; only one |
| NaN edge | `wheel::tests::nan_accuracy_never_owned` | NaN EWMA fails the gate |
| brain.ts drift | `brain.spec.ts` roundtrip | TS `WheelView` matches Rust serde shape |
| Manual | — | Wheel reads calm, segments legible, tap detail readable, empty state on a fresh profile |

## 8. Architecture / approach
Derivation is a pure leaf in `brain::learner::wheel`, beside F2's transitions, importing F2's
`MIN_ATTEMPTS` / `OWNED_THRESHOLD` so the wheel can never disagree with the flag F2 wrote (single
source of truth for the rule). `build_wheel` reads the in-memory `LearnerModel` (already loaded by
the brain store) and the existing stored `Fingerprint`s — no new persistence, no migration, no event
stream. One thin `get_mastery_wheel` command returns the snapshot; the frontend is render-only,
matching the repo rule that business logic stays in Rust core and the frontend just draws (like
`PerceptionPanel` consuming a snapshot). SVG is hand-rolled in `KeyWheel.tsx` (the existing SVG in
the app is OSMD-rendered score, not a reusable chart primitive, so this is the first hand-built SVG
view — kept small, static, Tailwind-styled, no chart dependency). No audio-thread interaction;
nothing real-time. Fully offline; no networked feature to disclose.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | Pure read-model: `classify` + `build_wheel` + `WheelView`/`KeyCell`/`KeyState`/`Trend` types, importing F2 constants; full unit coverage (AC1–5, 7, 11, NaN) | `crates/brain/src/learner/wheel.rs` | F2 | no |
| S2 | Trend derivation from stored fingerprints (intonation + tone), `Unknown` gating (AC8–9) | `crates/brain/src/learner/wheel.rs` (trend fns) | S1 | no |
| S3 | `get_mastery_wheel` IPC command + `WheelView` mirror in `brain.ts` + roundtrip test | `apps/desktop/src-tauri/src/commands.rs`, `apps/desktop/src/types/brain.ts` | S1, S2 | no |
| S4 | `KeyWheel.tsx` SVG (circle-of-fifths layout, 3+1 fills, empty state) (AC6, 10) | `apps/desktop/src/components/KeyWheel.tsx`, page wiring | S3 | no |
| S5 | `KeyWheelDetail.tsx` tap-for-detail panel + a11y (AC12) | `apps/desktop/src/components/KeyWheelDetail.tsx` | S4 | no |

**First slice to build now: S1** — self-contained, offline, proves the classification rule that the
whole view rests on.

## 10. Risks / open questions
- **Constant sharing with F2:** the wheel must import F2's `MIN_ATTEMPTS`/`OWNED_THRESHOLD` rather
  than copy them — confirm F2 exposes them `pub`. If not, S1 adds a tiny `pub` re-export in F2.
- **Trend window `K` / `MIN_TREND_POINTS` / `EPS`:** chosen for stability over twitchiness; tune with
  real fingerprint history. Tone trend axis (`core_clarity` vs a blend) — start single-axis, revisit.
- **Circle-of-fifths enharmonics** (F#/Gb, etc.): pick display spelling in the view layer; doesn't
  affect the pitch-class-keyed `KeyScale`.
- **Owned-vs-flag conflict** (AC4 edge): decided to trust the numeric gate; confirm that matches the
  product intent ("never over-claim") with the user.

## 11. References
- Epic + F2 contract: `docs/specs/252-rv-practice-coach.md` (F2 `LearnerModel`, `Mastery`,
  `key_mastery: BTreeMap<KeyScale, Mastery>`, `owned` flips at threshold over ≥M attempts,
  `derive_sound_profile`, stored fingerprints).
- Style/precedent: `docs/specs/253-reveal-loop.md` (pure-selector-in-Rust + render-only frontend).
- `apps/desktop/src/types/brain.ts` (serde mirror + drift rule; `MusicalFingerprint`,
  `IntonationSummary.mean_abs_cents`, `ToneDescriptor.core_clarity`).
- `apps/desktop/src/components/PerceptionPanel.tsx` (snapshot-consuming, render-only component
  pattern), `ScoreView.tsx` (existing SVG is OSMD-rendered, not a chart primitive).
