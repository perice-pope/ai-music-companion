# Spec: Strict recap parse — no narration flag without LLM-authored text (#470)

## 1. Summary
`parse_recap_json` stops accepting a valid-JSON LLM response that carries no
`overall_assessment`: such a response is a parse failure, the engine serves the
grounded offline fallback, and `recap_used_llm()` stays `false`. This is issue
#470's option (a) — option (b) (documenting the weaker flag semantics at the
projection sites) shipped with #449 T2; this slice retires the caveat.

## 2. Problem / why
Reviewer finding on #449 T1 (issue #470): the recap parser is
all-defaults-forgiving per field, so a valid-JSON-wrong-keys response
"parses" — `recap_used_llm` flips `true` and `practice_events` journals
`narration_used {"kind":"recap"}` while **every user-visible field is a canned
default with zero LLM-authored text**. Since T2, that event is projected to
teacher dashboards, so the overcount is externally visible. The product's
standing bar ("silence beats a lie"; the offline path "never fabricates") wants
the flag to mean *the shown headline was LLM-authored*, not *a response
happened to be JSON*.

## 3. Non-goals
- No change to secondary-field forgiveness: a response WITH a real
  `overall_assessment` but missing `strengths` / `areas_to_improve` /
  `next_session_suggestions` still parses, with canned defaults for the
  missing lists (pinned by AC4). Tightening those is not asked for by #470.
- No change to `get_tip` parsing (out of #470's scope).
- No change to the response-envelope parsing (`parse_recap_from_response`),
  the airplane switch, thin-session gating, or the fallback generator.
- No schema/DTO shape change — only doc-comment semantics at the projection
  sites.

## 4. Contract / interface
- `CoachingEngine::parse_recap_json` (private): returns
  `Err(SessionError::RecapFailed(..))` when `overall_assessment` is absent,
  not a string, or empty/whitespace-only. On success the stored
  `overall_assessment` is the trimmed LLM text. All other behavior unchanged.
- `generate_recap` is externally unchanged (`Ok` either way): a parse failure
  already routes to `fallback_recap` with `recap_llm_fired == false`.
- Flag semantics upgrade (documented at the three projection sites and their
  grep-pin test): `narration_used {"kind":"recap"}` now means the recap's
  headline text was LLM-authored.

## 5. Acceptance criteria (numbered, testable)
1. A syntactically valid JSON recap body with wrong keys (the issue's original
   `{"invalid": "json", "structure": true}`) produces the grounded offline
   fallback recap and `recap_used_llm() == false`.
2. A recap body whose `overall_assessment` is an empty or whitespace-only
   string behaves as AC1 (fallback, no flag).
3. A recap body whose `overall_assessment` is present but not a string
   (e.g. a number) behaves as AC1.
4. A recap body with a non-empty `overall_assessment` and NO other recognized
   keys parses: the shown assessment is the LLM text, the missing lists get
   the existing canned defaults, and `recap_used_llm() == true`.
5. Existing behavior pins stay green: a prose (non-JSON) body, an API failure,
   and the offline policy all still serve fallbacks with the flag `false`; a
   complete parsed response still sets it `true`.
6. The projection-site caveats (`syncStore.ts`, `commands.rs`,
   `types/brain.ts`) state the new semantics, and the grep-pin test in
   `syncStore.dashboard.test.ts` asserts the updated wording.

## 6. Edge cases & failure modes
- `overall_assessment: null` → `as_str()` is `None` → parse failure (AC3 class).
- Markdown-fenced JSON (already stripped) with a good assessment → parses.
- Leading/trailing whitespace in the assessment → trimmed, still parses.
- The parse failure must not surface an error to the user: `generate_recap`
  converts it to the fallback recap (existing `Err(_) => Ok(fallback)` arm).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `coaching::tests::wrong_keys_json_recap_falls_back_without_flag` | fallback assessment text == `grounded_offline_recap`'s; flag false |
| AC2 | `coaching::tests::blank_overall_assessment_falls_back_without_flag` | same, for `"   "` |
| AC3 | `coaching::tests::blank_overall_assessment_falls_back_without_flag` (numeric case) | same, for `42` |
| AC4 | `coaching::tests::partial_recap_with_real_assessment_still_parses` | LLM headline shown, canned list defaults, flag true |
| AC5 | existing `recap_used_llm_true_only_after_parsed_network_recap`, `generate_recap_handles_malformed_response` | unchanged behavior |
| AC6 | `syncStore.dashboard.test.ts` grep-pin test (updated) | caveat wording matches implementation |

## 8. Architecture / approach
One guard at the top of the field extraction in
`crates/brain/src/coaching.rs::parse_recap_json`; doc-comment updates at the
three projection sites plus the pin test; two one-line supersession notes in
`docs/specs/449-t2-sync-projection.md` where option (b) is recorded. Fully
offline-neutral — no network behavior changes.

## 9. Slice breakdown
Single slice (< 200 changed lines).

## 10. Risks / open questions
- An LLM that answers with a good recap under a differently-named headline key
  now falls back. That is the intended trade: the fallback is honest and
  grounded, and the prompt explicitly demands `overall_assessment`.

## 11. References
- Issue #470; #449 T1 (flag), T2 (projection + option (b));
  `docs/specs/449-t2-sync-projection.md`; decisions-log 2026-04-20
  ("Coaching-off-with-banner" — honesty over filler).
