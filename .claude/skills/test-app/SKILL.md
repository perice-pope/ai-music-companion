---
name: test-app
description: Guided pass/fail walkthrough to test the AI Music Companion desktop app on a Mac. Launches the app and steps a non-technical tester through simple checks one at a time, asking the musician to play when sound is needed, and records the results. Use when someone says "test the app", types "/test-app", or wants to run QA on the desktop app.
---

# Guided app test (Mac)

You are walking a **non-technical tester** through testing the AI Music Companion
desktop app **on a Mac**. They may be controlling the Mac remotely (AnyDesk); a
musician is nearby and will play sound when you ask. Be warm, plain, and **slow —
one step at a time, and WAIT for their reply before moving on**. No jargon ever.

## Step 0 — make sure you can actually drive the app

You can only launch the app if you are running **on the Mac** (Claude Code in the
Mac's Terminal). If there is no display / you cannot run GUI apps (e.g. a cloud
session), say so plainly and switch to **guide-only mode**: you give each step,
they click on their screen, they tell you PASS/FAIL. Do not pretend to open it.

If you can drive it, launch the app:

- Try the built app first: `open -a "AI Music Companion"`
- If that fails, run it in dev mode (tell them "this may take a minute to build
  the first time"): `cd apps/desktop && pnpm tauri dev`
- Wait until they confirm the window is open before starting the steps.

## The 7 checks — ask ONE at a time, then wait for "pass" or "fail"

Send each step as a short message. After each, ask **"PASS or FAIL?"** and stop
until they answer. Record each answer.

1. **It opens** — "The app window should be open now. Do you see it, with no error
   message? (PASS / FAIL)"
2. **Microphone** — "Pick an instrument and press Start. If a box asks to use the
   microphone, click Allow. Did it ask (or was it already allowed)? (PASS / FAIL)"
3. **Reacts instantly** — "🎵 Musician: play a few short, sharp notes now. Tester:
   does the pitch display react instantly, with no delay you can notice? (PASS / FAIL)"
4. **Shows sheet music** *(optional)* — "If you have a music file handy, open or
   drag it in. Does the sheet music appear with no error? (PASS / FAIL / SKIP)"
5. **Moving line follows** *(skip if step 4 was skipped)* — "🎵 Musician: play
   along with the music. Tester: does the moving line stay smooth and roughly on
   their spot? (PASS / FAIL)"
6. **Summary at the end** — "Press Stop / End session. Does a summary screen
   appear? (PASS / FAIL)"
7. **Closes cleanly** — "Quit the app (red dot, or ⌘Q). Does it fully close with
   no freeze or error? (PASS / FAIL)"

## Finish

Show a small results table (step → PASS/FAIL) and an overall verdict:
**GO** if every applicable step passed, otherwise **NO-GO**. For anything that
failed, say in one plain sentence what looked wrong, and offer to look into it.

Keep every message short and friendly.
