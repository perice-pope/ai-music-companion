# Spec: startLesson failure surfaces in the UI (#495)

## 1. Summary
When "Give me a lesson" fails, the player sees a calm error message next to the button
instead of nothing. Kills the dead-button pattern flagged by two consecutive CTO audits
(#363 item 9, #494 item 5).

## 2. Problem / why
`PracticeSession.tsx:212` runs `startLesson().catch(console.error)`. Any backend/store
failure (IPC error, "start a practice session before starting a lesson") goes only to the
console — the user clicks and nothing visibly happens. The codebase already fought and
fixed this exact class on the score screen (#184, `ScorePicker.tsx`'s `startError`) and
for the instrument switcher (`switchError` in this same component).

## 3. Non-goals
- No global toast system — the house pattern is a component-local `role="alert"` line.
- No change to the `startLesson` store action's contract (it keeps throwing; callers decide
  how to surface).
- No retry/backoff logic; the button itself is the retry.

## 4. Contract / interface
`PracticeSession` gains local state `lessonStartError: string | null`. No store, IPC, or
type changes. The alert renders only while the start button renders (a successful start
unmounts both with the button).

## 5. Acceptance criteria (numbered, testable)
1. Given `start_lesson` rejects, clicking "Give me a lesson" renders a `role="alert"`
   element containing the backend's message.
2. Clicking the button again clears the previous error before the new attempt; if the
   retry succeeds, no alert remains and the lesson takes the stage.
3. A successful first start renders no alert.

## 6. Edge cases & failure modes
- Non-`Error` rejection values (Tauri invoke rejects with strings): stringified, not
  `[object Object]`.
- Error while a previous error is showing: replaced, not appended.
- Success after failure: the lesson panel mounts and the alert is gone (AC2).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `PracticeSession.test.tsx` "a failed lesson start shows the error instead of a dead button" | alert with backend message appears after rejected invoke |
| AC2 | same file, "retrying after a failed start clears the error" | alert gone after successful retry, lesson panel mounted |
| AC3 | existing "the start-lesson button fires start_lesson" + new assert | no alert on success path |
| string rejection | AC1 test rejects with a plain string | message shown verbatim |

## 8. Architecture / approach
Frontend-only, matching `ScorePicker.tsx`'s #184 handling: local state set in a
try/catch around the awaited store action, rendered as a small `role="alert"` paragraph
anchored under the button (absolute, like the header's dropdown surfaces, so the header
row doesn't reflow). Offline-first: no network. Real-time: nowhere near the audio thread.

## 9. Slice breakdown
Single slice — one component + its test.

## 10. Risks / open questions
Open PR #490 (warmup UI) edits the adjacent line (wraps the same button in a
`!warmupActive` condition); the merge conflict is one-line and mechanical, whichever
lands second.

## 11. References
#495, #494 (audit), #363 (prior audit), #184 (`ScorePicker.tsx` pattern),
`apps/desktop/src/components/PracticeSession.tsx`.
