# Spec: The key strip settles honestly — "finding the key…" on key-less material (#404 finding 2)

> Part of #404 (VA run 2026-07-16, #401). Finding 1 (recap vs strip contradiction) shipped in
> #433; this is the remaining finding: **warm-up key cycling** — "the live key detection kept
> cycling throughout — never settled."

## 1. Summary

When the material the player is producing is nearly key-less (long-tone warm-ups, chromatic
wandering), the "I hear" strip must say **"finding the key…"** instead of showing a
confident-looking key name that is stale, wrong, or churning. Steady in-key playing and real
modulations keep today's behavior.

## 2. Problem / why

Two defects conspire, both in `theory::KeyTracker` (measured via a trace of warm-up shapes):

1. **Early commit, no ambiguity gate on the *display*:** three long tones in, the tracker commits
   (e.g. chromatic descending long tones → "A Phrygian", confidence 0.41, margin 0.00; chromatic
   noodling → "F Dorian", 0.67) and the strip prints the name.
2. **Stale held fit:** `update()` only refreshes the held estimate's confidence when the top
   candidate *is* the held key. When the rolling window wanders off the held key (exactly what
   key-less material does), the displayed confidence freezes at its commit-time value; the strip
   keeps asserting a reading the profile no longer supports. On real (noisy) input the near-tie
   aliases churn — the VA's carousel.

RV rule (north star): key detection is **display honesty only** — a rotating or stale
confident-looking name on key-less material is precisely the dishonesty it forbids.

## 3. Non-goals

- No change to *which* key is estimated, the commit bar, or the switch margin/dwell rules.
- No margin gating of the display (the 2026-07-11 decision: genuinely ambiguous steady material
  — do-re-mi-fa-sol — must keep its reading; the relative-alternative button is the ambiguity UX).
- No change to the reveal gate (`REVEAL_MIN_CONFIDENCE`), recap aggregation (#433), or key pinning.
- Not touching the chord tracker or #382's chord label pipeline.

## 4. Contract / interface

- `theory::KeyTracker`:
  - `update()` refreshes the held estimate's `confidence`/`margin` against the **current** profile
    every observation (via `correlation_for` when the held key is not the top candidate).
  - New `pub fn is_settled(&self) -> bool` — `false` once the held key's live fit has failed the
    commit bar (`min_confidence` / `min_pitch_classes`) for `unsettle_dwell` **consecutive**
    observations; `true` again when the fit recovers or a (re)commit/switch happens.
  - `KeyTrackerConfig` gains `unsettle_dwell: u8` (default documented in code).
- `brain::perception::KeySnapshot` gains `settled: bool` (serde: defaults `true` when absent —
  wire-additive; `KeySnapshot` is live-wire only, never persisted).
- Frontend `KeySnapshot` mirror (`types/brain.ts`) gains `settled: boolean`.
- `PerceptionPanel`: an unsettled, unpinned key renders **"finding the key…"** (no name, no
  "maybe", no relative-alternative or lock buttons). Pinned display is unchanged.

## 5. Acceptance criteria (numbered, testable)

1. Chromatic long-tone warm-up (the VA's shape, fed as notes): after the early tentative commit,
   the tracker reports **unsettled** and stays unsettled while the material stays key-less.
2. The held estimate's confidence is **live**: with C major held and wandering material following,
   `current().confidence` decreases even while the held identity stays C (fails on today's code).
3. Steady one-key material never goes unsettled — from first commit through 6+ scale passes,
   `is_settled()` is `true` at every observation (no false "finding…" flash).
4. A sustained modulation still lands and asserts: C major → 12 reps of F# major ends held on F#,
   settled. (A transient unsettled window *during* the ambiguous transition is acceptable and
   honest; the end state is pinned.)
5. End-to-end at the strip's layer: warm-up frames through `PerceptionTracker` produce
   `snapshot.key.settled == false`; steady scale frames produce `settled == true`.
6. `PerceptionPanel` with `settled: false` (unpinned) renders "finding the key…" and neither the
   key name nor the alternative/lock buttons; with `settled: true` renders exactly today's UI;
   a pinned key renders the pinned name regardless.
7. Existing reveal trigger pins stay green: steady material fires, noodling never fires
   (`crates/brain/tests/reveal_trigger_test.rs` unchanged and passing).

## 6. Edge cases & failure modes

- **No held key yet** → `is_settled()` true (vacuous); strip already shows nothing.
- **Fit oscillating around the bar** → the `unsettle_dwell` streak requirement is the hysteresis;
  one below-bar observation never blanks the name.
- **Recovery** → same-key material after mush re-asserts the name (streak resets when the fit
  clears the bar; a switch/commit also resets).
- **`reset()`** clears the streak with the rest of the state.
- **Old wire consumers** → `settled` absent deserializes as `true` (today's behavior).

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `theory::tracker::tests::keyless_warmup_reads_unsettled_not_a_confident_name` | unsettled during chromatic long-tone feed |
| AC2 | `theory::tracker::tests::held_confidence_tracks_the_live_profile` | confidence strictly lower after wandering |
| AC3 | `theory::tracker::tests::steady_material_never_flashes_unsettled` | settled at every observation |
| AC4 | `theory::tracker::tests::a_modulation_lands_settled` | end state F#, settled |
| AC5 | `brain::perception::tests::warmup_snapshot_is_unsettled_steady_is_settled` | `key.settled` through the real frame path |
| AC6 | `PerceptionPanel.test.tsx` (new cases) | "finding the key…" rendering + button absence; settled/pinned unchanged |
| AC7 | existing `reveal_trigger_test.rs` | unchanged, green |
| reset edge | extend `theory::tracker::tests::reset_clears_state` | settled true after reset |

## 8. Architecture / approach

All logic in the Rust core (`crates/theory`, surfaced through `crates/brain`); the frontend only
renders the flag (no business logic in the Face). No network, no allocation-sensitive path (the
tracker runs on the processing thread). The recap path is untouched: phrases snapshot
`theory::KeyEstimate` as before — they simply inherit the (more honest) live confidence, which
only strengthens #316/#433's evidence gates.

## 9. Slice breakdown

Single slice (~8 files, well under 400 lines): tracker + perception + FE types/panel + tests +
this spec + a one-line VA playbook note so the next run recognizes "finding the key…" as the
fix, not a regression.

## 10. Risks / open questions

- The `unsettle_dwell` default and the live-fit bar reuse `min_confidence` (0.4) — calibrated by
  the trace shapes in the tests. If a future perception change shifts the bands, the AC1/AC3
  tests go red in-tree (same philosophy as the reveal trigger pins).
- Draft PR #423 (fix/387) touches `tracker.rs`/`perception.rs` for key *naming*; changes here are
  additive and orthogonal (settling, not spelling) — a trivial merge either way.

## 11. References

#404 (finding 2), #401 (VA run), #433 (finding 1), #313/#277/#321/#325 (key signal honesty
history), `docs/architecture/rv-methodology.md` (display honesty), decisions log 2026-07-11
(reveal gate — why no margin gating).
