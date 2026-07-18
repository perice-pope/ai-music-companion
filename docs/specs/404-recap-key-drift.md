# Spec: recap key drift honesty — no end-state claim the strip never showed (#404, finding 1)

> VA run 2026-07-16 (#401 → #404): the recap concluded "leaning F Phrygian toward the end"
> while the live strip was cycling "E Locrian / F major" at session end.

## 1. Summary
When the session's dominant key disagrees with the strip's closing state and the closing key
held only one phrase (a cycling/blip end), the recap currently claims the dominant key
"**toward the end**" — an end-state claim about a key the strip was not showing at the end.
This slice makes that claim honest: the dominant key is reported as a whole-session fact
("mostly F Phrygian — wandering by the end"), never as where the session ended.

## 2. Problem / why
`aggregate_key` (crates/brain/src/coaching.rs) resolves a winner-vs-final-reading
contradiction with a single-phrase closing run by leaning the whole-session mass winner —
correct per #313 (a blip must not hijack the recap). But it returns
`KeyClaimStrength::Leaning`, and **every** `Leaning` consumer phrases the claim as
"leaning X toward the end" (recap prompt line, `SessionRecap.tsx`). For the settled-late
case that copy is honest; for the contradiction-blip case it is exactly backwards — the
winner carried the *middle* of the session and the end wandered off it. The RV bar: key
detection is display honesty only; the recap must never visibly contradict the strip.

## 3. Non-goals
- Key-chip hysteresis / "finding the key…" live state (#404 finding 2) — same subsystem as
  in-flight PR #423 (Locrian demotion changes what the tracker reports); deferred until it
  lands.
- Naming both keys of a cycling end ("between E Locrian and F major") — richer shape,
  carries two keys; not needed to remove the contradiction.
- Any change to which key is claimed or to the #313 blip rule — only how firmly/where the
  claim is anchored.

## 4. Contract / interface
- `brain::fingerprint::KeyClaimStrength` grows a variant: `Drifted` (serde `"drifted"`) —
  "the key that carried the session's tracking mass, but the live reading had wandered off
  it by session close; a whole-session claim, never an end-state claim."
- `KeyClaimStrength::hedged()` — `true` for `Leaning | Drifted`; the existing
  "not-Leaning ⇒ firm" checks route through it.
- Frontend mirror `MusicalFingerprint.key_claim` union adds `"drifted"`.
- Persisted recap JSON is forward-compatible: old blobs never contain `"drifted"`;
  new blobs with it are only read by code that knows it.

## 5. Acceptance criteria (numbered, testable)
1. A session whose vote winner ≠ final reading with a single-phrase closing run (cycling
   end) yields `Claimed(winner, Drifted)` — including the VA shape: F Phrygian dominant,
   closing alternation E Locrian / F major.
2. The winner ≠ final case where the closing key held ≥ 2 phrases still yields
   `Claimed(final_reading, Leaning)` (unchanged, pinned).
3. The recap prompt line for a `Drifted` key says the key carried most of the session and
   explicitly forbids phrasing it as where the session ended; it does not contain
   "toward the end".
4. `SessionRecap.tsx` renders a drifted key as "mostly <key> — wandering by the end";
   leaning/asserted/legacy renderings are unchanged.
5. A `Drifted` key is hedged everywhere `Leaning` is: it does not drive a mode-named
   flavour line, does not produce the "sat firmly" strength, and its next-session
   suggestion says the key carried the session, not "the key you ended on".
6. `Drifted` does not anchor degree tendencies (`aggregate_intonation` remains
   Asserted-only, pinned by existing test).
7. `KeyClaimStrength::Drifted` serializes to/from `"drifted"`.

## 6. Edge cases & failure modes
- Closing reading that IS the winner (winner == final) → unchanged: `Asserted` when it
  carried ≥ 60% of the mass across ≥ 2 counted readings, else `Leaning`; the strip ended on
  the winner, so no end-state dishonesty arises and `final_run` is not consulted. A
  single-reading session is never firm (the two-sighting floor).
- Legacy fingerprint (`key_claim: null`) → still reads as asserted/flat (unchanged).
- Old frontend receiving `"drifted"` cannot happen (frontend + backend ship together), but
  the TSX falls through to the flat form rather than crashing if it ever did.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `coaching::tests::a_cycling_session_end_reads_mostly_the_dominant_key` (new) | VA shape → `Drifted` + winner named |
| AC1 | `coaching::tests::a_single_phrase_closing_blip_does_not_hijack_the_recap_key` (updated) | blip branch → `Drifted`, not `Leaning` |
| AC2 | `coaching::tests::recap_defers_to_the_final_live_reading_on_contradiction` (existing) | held close → `Leaning(final)` |
| AC3 | `coaching::tests::recap_prompt_marks_hedged_and_unsettled_keys` (extended) | "mostly", no "toward the end" |
| AC4 | `SessionRecap.test.tsx` (extended) | "mostly G# major — wandering by the end" |
| AC5 | `coaching::tests::a_hedged_key_does_not_drive_a_mode_named_flavour` (extended) + fallback-recap tests (extended) | flavour degrades; no "sat firmly"; suggestion copy |
| AC6 | `coaching::tests::degree_tendencies_require_an_asserted_key` (existing) | Asserted-only anchor |
| AC7 | `fingerprint::tests` (new case) | `"drifted"` round-trip |

## 8. Architecture / approach
Pure local Rust + display copy; no network, no new IPC, no schema change. Files:
`crates/brain/src/fingerprint.rs`, `crates/brain/src/coaching.rs`,
`apps/desktop/src/types/brain.ts`, `apps/desktop/src/components/SessionRecap.tsx` + tests.

## 9. Slice breakdown
Single slice (this PR). Follow-up: #404 finding 2 (key-chip settling state) after #423.

## 10. Risks / open questions
- Overlaps `coaching.rs` with in-flight drafts #423/#416 in different regions; small,
  mergeable either order.

## 11. References
#404, #401, #316 (claim strengths), #313 (blip rule), `docs/architecture/rv-methodology.md`.
