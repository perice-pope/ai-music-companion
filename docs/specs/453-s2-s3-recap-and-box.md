# Spec: recap integration + the coaching box (#453 S2 + S3)

## 1. Summary
S1 shipped the analyzer: evidence-cited `PracticeSuggestion`s over the
timed exercise log + `key_mastery`, exposed by the `practice_suggestions`
command. S2 weaves AT MOST ONE of them into the post-session recap
(offline generator appends it; the LLM generator receives them as
GROUNDED INPUT, mirroring the idiom block). S3 gives free play a NEW
persistent surface — the coaching box — that functions exactly like the
amber reveal box (rule 0) but in a calm muted violet, showing at most
one suggestion, refreshed at session start and explore begin.

## 2. Contract

**S2 — recap:**
- `RecapInput.history_suggestions: Vec<insights::PracticeSuggestion>`
  (`serde(default)` — stored inputs load unchanged). Threaded by the
  command layer via a new `generate_recap_with_context` parameter; the
  recorder stays history-agnostic (same posture as `note_verdicts`).
- `end_practice_session_impl` computes the suggestions from the store
  (the S1 command's exact read path, refactored into a shared
  `practice_suggestions_core`) and hands them to `build_recap`. Store
  failure → empty list, never an error.
- Offline generator (`grounded_offline_recap`, which also backs the
  LLM engine's `fallback_recap`): appends AT MOST ONE suggestion — the
  FIRST by the analyzer's pinned order — to `next_session_suggestions`.
  Its `text` already embeds the citation numbers; nothing is rephrased.
- LLM generator: `insights::history_prompt_block` renders ALL
  suggestions (each `text` + `evidence`) into `build_recap_user_prompt`
  as GROUNDED INPUT — the idiom-block posture: the model may narrate,
  hedged, but must keep the numbers as given and never invent history
  claims. Empty suggestions → no block (byte-identical prompt).
- Thin sessions (#445-6b): BOTH generators bail into
  `thin_session_recap` before any history is woven — a quick touch
  keeps its single suggestion; history lines never stack onto it.

**S3 — the coaching box:**
- `CoachingBox.tsx`, mounted directly below `RevealCard` in all three
  `PracticeSession` mounts (explore / score / free play). Testids
  `coaching-box` / `coaching-box-text` / `coaching-box-dismiss`.
- Calm palette: `bg-violet-950/40` + `border-violet-800` +
  `text-violet-200` family — clearly NOT the reveal's amber alarm.
- Store (`practiceStore`): `coachingSuggestion` (`{kind, text,
  evidence} | null`) + `coachingQuieted` + `refreshCoachingSuggestion`
  + `dismissCoachingSuggestion`. Refresh invokes the
  `practice_suggestions` command (fire-and-forget, errors silent) at
  session start and at explore begin (the cheap exercise-log-writing
  hook). Session-scoped: both reset in `endSession`'s tail (and the
  `openMatchedScore` session-teardown mirror).
- Rule 0: a newer fetch's FIRST suggestion replaces the shown one in
  place; an empty (or failed) fetch NEVER clears a shown suggestion;
  dismissal quiets the box for the rest of the session (later fetches
  stay quiet). AT MOST ONE suggestion is ever shown.
- No empty chrome: with no suggestion to show the component renders
  nothing at all — in particular, an empty reveal + an empty box add
  zero extra chrome below the reveal's spacer.

## 3. Non-goals
Habit-shape analysis, the taste/goal tie-in (S4), refreshing on every
exercise-log write (openers, drill submits — no cheap common hook
exists; explore begin + session start are the pinned cadence), any
dim-on-contradiction state (a history claim has no live signal to
contradict it; "dims, never vanishes" maps to "empty never clears"),
new analyzer rules, teacher surfaces (#449).

## 4. ACs
1. The offline full recap appends exactly ONE history line — the first
   suggestion by pinned order, verbatim `text` (citation included) —
   to `next_session_suggestions`; a second suggestion never appears;
   no suggestions → no history line.
2. A thin session's recap gains NO history line — it keeps its single
   #445-6b suggestion even when suggestions exist.
3. The LLM user prompt carries the suggestions as GROUNDED INPUT
   (marked, text + evidence present, invention forbidden) when they
   exist, and no history block when they don't.
4. The real end-session path over a real store fixture (seeded log +
   mastery) weaves exactly the one earned, cited line into the recap;
   the same path with a thin session weaves none.
5. The coaching box surfaces the FIRST fetched suggestion (only one),
   fetched via the `practice_suggestions` command by name; fetch
   errors are silent.
6. Rule 0: the box holds through an empty fetch result; a newer
   suggestion replaces it in place; dismissal quiets the box for the
   session (later fetches stay quiet).
7. Calm palette pin: the box renders violet-family classes and no
   amber class anywhere in its markup.
8. No suggestion → the box renders nothing (no empty chrome).
9. Store wiring: session start triggers a `practice_suggestions`
   fetch; explore begin triggers another; session end resets the box
   state (suggestion cleared, quiet lifted).

## 5. Test map
| AC | Test |
|---|---|
| 1 | `coaching::tests::offline_recap_appends_at_most_one_history_suggestion` |
| 2 | `coaching::tests::thin_recap_gains_no_history_suggestion` |
| 3 | `coaching::tests::recap_prompt_carries_history_as_grounded_input` |
| 4 | `commands::tests::end_session_weaves_one_cited_history_line`, `commands::tests::thin_end_session_weaves_no_history_line` |
| 5 | CoachingBox.test.tsx `a fetched suggestion surfaces — first only, routed by command name`, `fetch errors are silent — never a crash` |
| 6 | CoachingBox.test.tsx `holds through an empty fetch — rule 0`, `a newer suggestion replaces in place`, `dismissal quiets the box for the session` |
| 7 | CoachingBox.test.tsx `calm palette: muted violet, never the amber alarm` |
| 8 | CoachingBox.test.tsx `renders nothing when history has nothing to say` |
| 9 | practiceStore.test.ts `session start fetches a coaching suggestion (#453 S3)`, `explore begin refreshes; empty results never clear; session end resets` |

## 6. Architecture
S2 rides the existing recap choke points: the thin gate stays a single
early return per generator (#445-6b), the prompt grounding mirrors
`idiom_prompt_block` (grounded facts in, hedged narration out — the
honesty rule: the LLM may NARRATE, the FACTS come from local
analysis), and the command layer joins store data to the recap at
coaching time exactly like `taste_profile`. S3 mirrors the
`pieceMatch` slice of `practiceStore` (rule-0 hold/replace/quiet,
session-scoped resets) and the `RevealCard` surface conventions.
Offline-first: everything reads sessions.db; no new network. See
issue #453, docs/specs/453-s1-history-analyzer.md,
docs/specs/445-thin-recap.md.
