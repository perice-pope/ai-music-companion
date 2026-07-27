# Spec: First-launch prompt — ask once whether to enable update checks (#465)

## 1. Summary
A once-only, dismissible prompt in the update pill's bottom-left spot that asks a new
user whether to turn on automatic update checks. Either answer is final (the question
never returns); changing your mind lives in Connections & Privacy, which the prompt names.

## 2. Problem / why
The #58 auto-check is strictly opt-in and OFF by default — correctly. But nothing ever
*tells* a user the toggle exists, so a fresh install sits on a stale version with the
pill silent and testers think updates are broken. The founder hit this on 2026-07-21: a
fresh install on v2.40.2 with v2.43.0 published, discovered the switch only by a
debugging session ending in Connections & Privacy. Issue #465 (founder-authored) is the
spec source; the copy below is quoted from it.

## 3. Non-goals
- No change to what the check sends, when it runs, or how the pill behaves — the
  heartbeat in `App.tsx` keeps reacting to `autoUpdateCheckEnabled` exactly as today.
- No new network call and no disclosure-table change (the check is already enumerated;
  the prompt only surfaces the existing choice — it is itself fully offline).
- No re-prompting, snoozing, or "remind me later" state machine.

## 4. Contract / interface
`connectionsStore` (additive):
- `updatePromptAnswered: boolean` — persisted (`localStorage`
  `ai-music-companion:update-prompt-answered`), default `false`.
- `answerUpdatePrompt(enable: boolean)` — marks answered (persisted); `enable=true`
  also enables + persists `autoUpdateCheckEnabled`; `enable=false` changes nothing else.
- `setAutoUpdateCheckEnabled` additionally marks the question answered: an explicit
  choice in Connections & Privacy makes the prompt moot forever.

New component `FirstRunUpdatePrompt.tsx`, rendered by `App` beside `UpdatePill`.
Visible iff `!updatePromptAnswered && !autoUpdateCheckEnabled && updateStore.phase === "idle"`.

## 5. Acceptance criteria (numbered, testable)
1. On a fresh install (nothing answered, toggle off, pill idle) the prompt renders in
   the pill's spot with the issue's copy and both buttons.
2. "Yes, keep me current" enables `autoUpdateCheckEnabled`, persists both flags, and
   removes the prompt; the existing heartbeat then performs its first check — with no
   restart needed.
3. "No thanks" leaves `autoUpdateCheckEnabled` false and its persisted value untouched,
   persists `updatePromptAnswered`, and removes the prompt.
4. Once answered (either way), the prompt never renders again — including after the
   user later toggles auto-check on and back off in Connections & Privacy.
5. No update request happens while the prompt is unanswered (the network gate stays
   the store flag, which stays false until "Yes").
6. Flipping the Connections & Privacy toggle directly (either direction) counts as the
   answer: the prompt never shows after that.
7. The prompt yields the slot to the pill: any non-idle update phase hides it (no
   stacked bottom-left elements, e.g. after a manual check finds an update).

## 6. Edge cases & failure modes
- **localStorage unavailable:** flags fall back to in-session state (same behavior as
  every existing connections flag) — the prompt may re-ask next launch, and never
  crashes. Matches `loadFlag`/`saveFlag`'s existing contract.
- **Existing installs** updating to this build see the question once too — deliberate:
  stale-version testers are exactly the motivating audience. If they already enabled
  the toggle, AC6/the visibility gate keeps the prompt away.
- **Manual check while unanswered** (via Connections & Privacy button): a found update
  raises the pill → AC7 hides the prompt; dismissing the pill brings it back (still
  unanswered, still honest).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `FirstRunUpdatePrompt.test.tsx` "asks the question on a fresh install" | copy + both buttons render |
| AC2 | ".. yes enables and persists the toggle" | store flag true, both localStorage keys "true", prompt gone |
| AC3 | ".. no thanks changes nothing but the answer" | toggle false, auto-update key untouched, answered persisted, prompt gone |
| AC4 | ".. never asks twice" | answered → null; answered + toggle cycled off → still null |
| AC5+AC2 | `App.test.tsx` "first-launch prompt: no check before the answer, first check right after yes" | updater `check` uncalled while prompt up; called once after clicking Yes |
| AC6 | ".. the Connections & Privacy toggle answers the question too" | `setAutoUpdateCheckEnabled(false)` → prompt null |
| AC7 | ".. yields the slot to the pill" | phase "available" → prompt null |

## 8. Architecture / approach
Face-layer only; no Rust, no IPC. The store stays the single opt-in source of truth and
the heartbeat's gate is untouched, so the offline-first promise ("no update request on
launch or in the background" until opted in) is structurally preserved — the prompt can
only flip the same switch the settings row flips. Styling mirrors the pill (sky palette,
fixed bottom-left) per the sleek/slim/simple bar.

## 9. Slice breakdown
Single slice (one PR): store flag + prompt component + App mount + tests.

## 10. Risks / open questions
- Copy is quoted from the founder's issue verbatim — no open questions.
- The prompt shows regardless of screen (it lives at App level like the pill). Accepted:
  first-launch users are on the instrument picker, and the element is small and calm.

## 11. References
- Issue #465 (founder proposal, 2026-07-21 — the spec source)
- #58 update pill + toggle (`UpdatePill.tsx`, `updateStore.ts`, `App.tsx` heartbeat)
- `docs/architecture/offline-first-and-network-transparency.md` (App auto-update row)
