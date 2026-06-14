# Spec: <feature name> (#<issue>)

> Copy this file to `docs/specs/<issue>-<slug>.md` and fill it in **before writing code**.
> A spec with vague or untestable acceptance criteria produces vague, untestable code.

## 1. Summary
One or two sentences: what this delivers and for whom.

## 2. Problem / why
What's broken or missing today. Link the issue and any evidence (logs, a screenshot,
a failing case). Be concrete — "the recap is identical every session because X".

## 3. Non-goals
What this slice explicitly does **not** do. (Prevents scope creep and over-building.)

## 4. Contract / interface
The shape that other code depends on: function signatures, types, events, DB schema,
IPC commands, UI props. If it changes an existing contract, say how (and whether it's breaking).

## 5. Acceptance criteria (numbered, testable)
Each must be checkable by a test or a manual step. Bad: "feedback works." Good:
1. Given a session with a swung Dorian fingerprint, the flavour reads "modal-jazz feel".
2. Given a straight diatonic session, no flavour line is shown.
3. ...

## 6. Edge cases & failure modes
The inputs that break naive code: empty/zero, very large, malformed, concurrent,
offline, missing permission, first-run vs migrated DB. For each, the expected behavior.

## 7. Test plan
Map **each acceptance criterion and edge case → a specific test** (unit/integration/e2e),
asserting behavior, able to fail for a real reason. Note anything that needs a manual check.
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `crate::module::tests::name` | flavour string for swung+Dorian |

## 8. Architecture / approach
How it fits what exists (name the files/crates). Offline-first: if it touches network,
how it's opt-in + disclosed. Real-time: if it's near the audio thread, the allocation story.

## 9. Slice breakdown (ordered, each a shippable PR)
For an epic, the small PRs. Each < ~400 lines, independently testable. Record each slice's
**file/module footprint** and **depends-on** so `/epic` can schedule non-overlapping slices into
parallel waves (disjoint footprints + merged deps = same wave). Define shared **interfaces/
contracts** as their own early slice — that's the seam others build behind in parallel.
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | e.g. audio-output engine interface + stub | `crates/audio/`, `commands.rs` | — | yes |
| S2 | local synth behind the interface | `crates/audio/synth.rs` | S1 | yes |
| S3 | frontend "play with me" button | `apps/desktop/src/...` | S1 | no |

## 10. Risks / open questions
What could go wrong, what you're unsure about, what needs a human decision.

## 11. References
Files, prior issues/PRs, design docs.
