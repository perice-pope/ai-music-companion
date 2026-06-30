# Spec: "One more variation" — ambient suggester chips + RV shuffle (#255)

> Part of epic #252. Builds behind the **F1** RV generator and reads the **F2** Learner Model. The
> first feature to make play *interactive*: the AI's reaction grows tappable chips, each a concrete
> RV parameter change, and tapping pulls the next always-fresh variation. No typing, ever.

## 1. Summary
In free play, after the AI reacts to a phrase it offers ≤3 tappable **chips** drawn from the RV
parameter space (`[New keys]` reshuffle roots, `[Make it spicy]` +1 difficulty, `[Different scale]`
swap scale, …). Tapping a chip generates the next RV variation via F1 reflecting **exactly** that
delta, renders it on the staff (playable + consistently scored), and — because RV reshuffles roots —
the next rep is always slightly new. Chips adapt to the player's context read from F2.

## 2. Problem / why
Free play (see #253) now *reacts* and *reveals*, but still offers nothing to **do next** — the loop
ends at the reaction. RV's whole appeal is "one more pull": each tap reshuffles into a fresh-but-
related rep. Today there is no bridge from "the AI heard you" to "here is the next thing to play."
This slice adds the smallest interactive surface — a finger-friendly chip row under the Coach card —
that turns a passive reaction into a self-driven, endless-but-fresh practice session with zero typing.

## 3. Non-goals
- No guided/adaptive lesson routine, no scoring-driven difficulty *automation* (that's #254). Chips
  change difficulty by **one step on explicit tap only**; nothing auto-escalates.
- No free-text / voice chip input — chips are AI-offered, the user only taps.
- No new generation, scoring, or theory logic in the frontend — F1 owns generation; the frontend
  renders chips and sends the chosen `VariationDelta`. (CLAUDE.md: no business logic in the frontend.)
- No new outbound network call. Chip selection + generation are fully local/offline.
- No heavy exploration analytics; an exploration log entry is optional and lightweight (AC-gated off).
- Not more than 3 chips, and not more than one chip row on screen.

## 4. Contract / interface

### Backend (Rust core)
Chip suggestion is a **pure, deterministic** function over the just-played context, the current spec,
and the F2 Learner Model. It lives next to F1 so it can name the parameter space, but reads F2 by value.

```rust
/// A concrete, named change to the active VariationSpec. The frontend never
/// constructs these — it echoes back the exact one attached to a tapped chip.
pub enum VariationDelta {
    ReshuffleRoots,                 // [New keys]  — RV signature: new root order (keep first fixed)
    BumpDifficulty(i8),             // [Spicy] = +1, [Simpler] = -1  (clamped 0..=MAX)
    SwapScale(ScaleModifier),       // [Different scale] — replace scale, keep difficulty/roots policy
    SwapChord(ChordModifier),       // [Different chord]
    ToggleDirection,                // [Reverse it] — forward <-> reversed
}

pub struct ChipSpec {
    pub label: String,              // finger-friendly, e.g. "New keys", "Make it spicy"
    pub delta: VariationDelta,      // the exact change tapping applies
}

/// Pure: same (ctx, spec, model, seed) -> same ordered ≤3 chips. No I/O, no network.
pub fn suggest_chips(
    ctx: &MusicalContext,           // from #253: key/mode/confidence of what was just played
    spec: &VariationSpec,           // the spec that produced the current rep
    model: &LearnerModel,           // F2: difficulty + recent accuracy drive which chips appear
    seed: u64,
) -> Vec<ChipSpec>;                 // length 0..=3, stable order

/// Apply a tapped delta to the active spec, then generate via F1.
/// `apply_delta` is pure; `generate` is F1's deterministic generator.
pub fn apply_delta(spec: &VariationSpec, delta: &VariationDelta) -> VariationSpec;
```

- **IPC.** Two thin JSON commands extend the existing command pattern (no new IPC philosophy):
  - `suggest_chips(...) -> Vec<ChipSpec>` (or chips ride along on the existing reaction event payload
    as `chips: ChipSpec[]`, mirroring the `CoachCard { reaction?, reveal?, chips }` contract from #252).
  - `apply_variation_delta(spec, delta) -> GeneratedSequence` — applies the delta and returns the next
    rep's `GeneratedSequence` (the `ticks` shape `ScoreView` already consumes; `target_notes` for
    scoring). The active spec is held in the session so successive taps compound.
- **F2 reads (no heavy write).** `suggest_chips` reads `model.difficulty` and recent accuracy
  (e.g. an EWMA over recent reps / the relevant `key_mastery` entry) to gate chips:
  never offer a difficulty-raising chip at `MAX`; offer `[Simpler]` when recent accuracy is below a
  struggling threshold. An optional lightweight exploration-log write (which delta was tapped) is the
  only write, and is behind a flag — no `apply_drill_result`-style mastery mutation here.

### Frontend (`apps/desktop/src/components/CoachCard.tsx` + `ChipRow.tsx`)
- `Chip = { label: string; action: VariationDelta }` and `CoachCard { reaction?, reveal?, chips: Chip[] }`
  per the #252 interaction contract. `ChipRow` renders ≤3 chips as a compact, finger-friendly row
  **under** the Coach card (reusing `CoachingTipPanel` card styling / the free-play layout slot in
  `PracticeSession.tsx`).
- Tapping a chip invokes `apply_variation_delta` with the chip's `action`, and feeds the returned
  `GeneratedSequence` into the existing `ScoreView` (`musicXml`/ticks) + scoring path. The frontend
  performs **no** theory; it only sends the delta and renders the result.
- Empty chips array → no row rendered (defined empty state, no layout jump).

## 5. Acceptance criteria (numbered, testable)
1. After a phrase with a valid `MusicalContext`, `suggest_chips` returns **at most 3** `ChipSpec`s,
   each carrying a concrete `VariationDelta` (no label without a delta).
2. Tapping a chip applies **exactly** that delta and generates the next variation via F1: a
   `[New keys]` tap yields a sequence whose root order differs from the current rep while the first
   root is unchanged (RV shuffle rule); a `[Spicy]` tap yields a spec with `difficulty` exactly one
   step higher.
3. The generated variation is rendered on the staff from its `GeneratedSequence.ticks` and is
   playable; the notes displayed equal `target_notes`, i.e. what is shown is exactly what is scored.
4. Chips adapt to context read from F2: at `difficulty == MAX`, **no** difficulty-raising chip is
   offered; when recent accuracy is below the struggling threshold, a `[Simpler]` chip **is** offered.
5. `[New keys]` (`ReshuffleRoots`) always produces a **next rep that differs** from the current one
   (root order changes) — the "always-fresh next rep" guarantee — given a roots set of length ≥ 2.
6. `suggest_chips` and `apply_delta`+`generate` are **deterministic**: identical `(ctx, spec, model,
   seed)` produce identical chips and identical next sequence (seed-reproducible).
7. `BumpDifficulty` is clamped: `+1` at `MAX` is a no-op spec (and per AC4 the chip isn't offered);
   `-1` at `0` is a no-op spec — never out of range.
8. Frontend: given `chips: Chip[]` of length 3, `ChipRow` renders 3 tappable controls under the
   Coach card; tapping control _i_ invokes `apply_variation_delta` with `chips[i].action`. Given an
   empty array, no row is rendered.
9. No outbound network call is made by chip suggestion, delta application, or generation (all local).

## 6. Edge cases & failure modes
- **First run / empty Learner Model** → `suggest_chips` uses defaults (difficulty 0, no struggling
  signal) and still returns a sane chip set (e.g. `[New keys]`, `[Make it spicy]`, `[Different scale]`);
  never crashes, never offers `[Simpler]` with no evidence.
- **Low-confidence / no context** (silence) → return an empty chip vec (no row), matching #253's
  "no guess" stance; never fabricate a context just to show chips.
- **Difficulty at bounds** → `MAX`: drop `[Spicy]`, may surface `[Simpler]`; `0`: drop `[Simpler]`.
  Covered by AC4/AC7.
- **Single-root spec** (`roots.len() == 1`) → `ReshuffleRoots` can't change order; either omit the
  `[New keys]` chip or fall back to a different fresh delta — never offer a chip that is a guaranteed
  no-op.
- **Rapid taps** → each tap applies to the *latest* spec (taps compound deterministically given the
  seed stream); no lost/duplicated rep, no concurrent generation race.
- **Generation fails / spec invalid after delta** → surface nothing new (keep current rep), no crash;
  the delta application is total (clamping, valid swaps) so this should be unreachable, but is guarded.
- **Offline** → unaffected; chips + generation are local-only (AC9).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `variations::chips::tests::at_most_three_chips_each_with_delta` | len ≤ 3; every chip has a `VariationDelta` |
| AC2 (keys) | `variations::chips::tests::new_keys_reshuffles_keeps_first` | next rep root order differs, first root fixed |
| AC2 (spicy) | `variations::chips::tests::spicy_bumps_difficulty_one_step` | applied spec difficulty == prev+1 |
| AC3 | `variations::tests::ticks_match_target_notes` | rendered ticks == `target_notes` (display == scored) |
| AC4 (max) | `variations::chips::tests::no_harder_chip_at_max` | no difficulty-raising chip when difficulty==MAX |
| AC4 (struggle) | `variations::chips::tests::offers_simpler_when_struggling` | `[Simpler]` present below accuracy threshold |
| AC5 | `variations::chips::tests::reshuffle_always_fresh` | `ReshuffleRoots` next rep ≠ current (roots ≥ 2) |
| AC6 | `variations::chips::tests::suggest_and_generate_deterministic` | same (ctx,spec,model,seed) → identical chips + seq |
| AC7 | `variations::tests::bump_difficulty_clamped` | +1@MAX and -1@0 are no-op specs |
| AC8 | `ChipRow.test.tsx` | renders 3 controls; tap _i_ invokes delta `chips[i].action`; empty → no row |
| AC9 | `variations::chips::tests::no_network_in_chip_path` | connections/LLM client never invoked |
| edge: empty model | `variations::chips::tests::empty_model_returns_sane_default_chips` | first-run chip set, no `[Simpler]` |
| edge: no context | `variations::chips::tests::low_confidence_returns_no_chips` | empty vec on weak/absent context |
| edge: single root | `variations::chips::tests::single_root_omits_dead_reshuffle` | no guaranteed no-op `[New keys]` |
| manual | free-play click-through | tap each chip, confirm staff updates + scores against shown notes |

## 8. Architecture / approach
Chip suggestion + delta application are **pure Rust** in `crates/variations` (the F1 crate) so they
sit next to the parameter space they name and stay leaf-level (no `brain` dependency for generation).
`suggest_chips` takes the `LearnerModel` (F2) **by value/ref** — it reads `difficulty` + recent
accuracy but does not own or mutate F2, keeping the F1↔F2 seam one-directional. The session holds the
active `VariationSpec`; `apply_variation_delta` mutates that held spec via the pure `apply_delta` then
calls F1's `generate(spec, seed)`, returning a `GeneratedSequence` in the **same `ticks` shape**
`ScoreView` + score-follow already consume — so rendering and scoring reuse existing paths unchanged.
Determinism comes from F1's explicit seed; the session advances the seed per tap so each "pull" is
fresh yet reproducible. Frontend is render-only: `CoachCard` + `ChipRow` echo the chosen
`VariationDelta` back over a thin JSON command. No network is touched, so nothing new is added to the
allowlist or `ConnectionsPrivacy.tsx` (offline-first holds by construction).

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | `VariationDelta` + `apply_delta` (pure, clamped, total) + unit tests | `crates/variations/src/delta.rs` | F1 | no |
| S2 | `suggest_chips` (context + F2-aware gating, deterministic, ≤3) + tests | `crates/variations/src/chips.rs` | F1, F2, S1 | no |
| S3 | IPC: `apply_variation_delta` command + session-held spec + seed advance | `apps/desktop/src-tauri/src/commands.rs`, session state | S1, S2 | no |
| S4 | Frontend `CoachCard` + `ChipRow` under the card, wired to `ScoreView`/scoring | `apps/desktop/src/components/CoachCard.tsx`, `ChipRow.tsx`, `PracticeSession.tsx` | S3 | no |
| S5 | (optional) lightweight exploration log of tapped deltas (flag-gated) | `crates/variations` log hook / `brain::learner` | S2 | no |

**First slice to build now: S1** (pure delta application) — it is self-contained and unblocks S2/S3.

## 10. Risks / open questions
- **Chip vocabulary & labels.** Which 3 chips to surface by default, and copy that reads friendly to
  kids ("Make it spicy" vs "Harder"). Start with `[New keys] [Make it spicy] [Different scale]`; expand.
- **"Recent accuracy" source in F2.** Whether to read a global EWMA, the active key's `key_mastery`,
  or a short in-session window — affects when `[Simpler]` appears. Resolve against F2's final shape.
- **Seed-stream ownership.** Where the per-tap seed lives (session vs Learner Model) so taps stay
  deterministic across a session and a reload. Likely session-local; confirm with #254's needs.
- **Single-root / tiny-roots specs.** Confirm the fallback delta when `[New keys]` would be a no-op.

## 11. References
- Epic + foundations: `docs/specs/252-rv-practice-coach.md` (F1 `generate(spec, seed)`, F2
  `LearnerModel`, `CoachCard`/`Chip` interaction contract). Sibling: `docs/specs/253-reveal-loop.md`
  (`MusicalContext`, same free-play surface, opt-out/offline stance).
- RV source: `perice-pope/random-variations` (`src/musicUtils.ts` root shuffle, `src/types.ts`,
  `src/data/{scales,chords,enclosures}.json`).
- Existing UI: `apps/desktop/src/components/CoachingTipPanel.tsx` (card styling), `PracticeSession.tsx`
  (free-play layout slot), `ScoreView.tsx` (consumes `ticks`/MusicXML + cursor).
- GitHub issue #255.
</content>
</invoke>
