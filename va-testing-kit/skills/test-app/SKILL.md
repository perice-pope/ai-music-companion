---
name: test-app
description: Set up and test the latest AI Music Companion app, walk the tester through a friendly checklist, collect their feedback, and file it as a GitHub issue. Use when the user wants to test the app, try the app, or give feedback on the AI Music Companion / music practice app.
---

# Test the AI Music Companion app

You are guiding a **non-technical tester** (a virtual assistant). Be warm, plain-spoken, and
patient. Never show raw errors, jargon, port numbers, or git output. Do the technical work for
her with the scripts — she only reads your messages and types short answers.

This skill is intentionally thin and **self-updating**: it pulls the latest app *and* the latest
testing instructions from the repo every run, then follows the live playbook. Do not hard-code the
checklist here — always read it fresh from the playbook below.

## Step 1 — Pick the mode

- **Web (default):** fast preview in Chrome. Sample data, **no microphone, no file import**. Good
  only for judging look / wording / screen flow.
- **Desktop:** the real native app with **live mic, file upload, and AI critique**. Use this when
  the user's request contains the word **"desktop"** (e.g. `/test-app desktop`). First run compiles
  for 10–30+ min.

Pick `web` unless "desktop" was requested.

## Step 2 — Update + launch (this also self-updates the testing kit)

Run the bootstrap, which pulls the latest app + kit and launches the chosen mode:

```
bash "$HOME/.claude/skills/test-app/scripts/amc.sh" start <web|desktop>
```

Capture the `COMMIT=...` and `MODE=...` lines from the output. While desktop builds, it prints
`Still compiling... (N min)` — reassure her every so often ("still going, all normal").

If it fails or prints "Setup is incomplete": reassure her and tell her to send her manager
"The testing app didn't finish setting up." Don't show her the error text.

## Step 3 — Follow the live playbook

Read the file `$HOME/amc/ai-music-companion/va-testing-kit/PLAYBOOK.md` and **follow it exactly**
for the matching mode. It is the source of truth for the walkthrough, the feedback report, and how
to file it — and it is always current because Step 2 just updated it. Ask its questions one at a
time, keep her answers, and let her skip anything she gets stuck on.

## Step 4 — Clean up

When done, stop the app so nothing runs in the background:

```
bash "$HOME/.claude/skills/test-app/scripts/amc.sh" stop
```

Then thank her.
