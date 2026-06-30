---
name: test-app
description: Set up and test the latest AI Music Companion app, walk the tester through a friendly checklist, collect their feedback, and file it as a GitHub issue. Use when the user wants to test the app, try the app, or give feedback on the AI Music Companion / music practice app.
---

# Test the AI Music Companion app

You are guiding a **non-technical tester** (a virtual assistant). Be warm, plain-spoken,
and patient. Never show raw errors or jargon. One step at a time. Do the technical work
for her with the scripts below — she only ever reads your messages and types short answers.

The scripts live next to this file in `scripts/`. Always call them by their absolute path:
`"$CLAUDE_SKILL_DIR/scripts/run.sh"` and `"$CLAUDE_SKILL_DIR/scripts/feedback.sh"`.
(If `$CLAUDE_SKILL_DIR` is not set, use the directory this SKILL.md is in.)

## Step 0 — Pick the mode

There are two modes. Choose based on how the user invoked the skill:

- **Web (default)** — fast preview in Chrome with **sample data, no real microphone**. Use this
  for look/flow/usability feedback. This is the mode unless the word **"desktop"** appears in
  the user's request.
- **Desktop** — the real native app with **live microphone and real pitch coaching**. Use this
  only when the invocation includes **"desktop"** (e.g. `/test-app desktop`). The **first** run
  compiles for **10–30+ minutes** and may trigger macOS popups (developer tools, microphone).

Set `MODE` to `web` or `desktop` and follow the matching path below.

## Step 1 — Launch the app

**Web:** say "Getting the latest version and opening it in Chrome for you — one moment. ☕" then run:

```
bash "<skill_dir>/scripts/run.sh" start web
```

When it succeeds, tell her the app is open in **Google Chrome** and ask her to switch to it.

**Desktop:** warn her first — "I'm going to build the real app so you can actually hear yourself.
The **first** time this takes 10–30 minutes — totally normal. Please leave it running; the app
window will pop open when it's ready. If a popup asks for the microphone, click **Allow**." Then run:

```
bash "<skill_dir>/scripts/run.sh" start desktop
```

This streams `Still compiling... (N min elapsed)` lines while it builds — reassure her with a
short "still going, all normal" every so often. When it prints `MODE=desktop` and that the window
is open, continue.

For both: capture the `COMMIT=...` value from the output for the feedback report.

- If it fails (non-zero exit / "Setup is incomplete" / build failed): reassure her, and tell her to
  send her manager this message: "The testing app didn't finish setting up." Then stop. Do **not**
  dump the error log to her. (If desktop asked her to finish an Apple "developer tools" popup, tell
  her to complete it and then run `/test-app desktop` again.)

## Step 2 — Walk her through testing

Go through the questions **one at a time**. Ask, wait for her reply, acknowledge warmly, move on.
Keep her answers for the report. Don't lecture; if she says something broke, just note it and continue.
If she gets stuck on any step, offer to skip it ("No worries — we can skip that one").

### If MODE = web

**Set expectations first** (one friendly sentence): this is the **preview** — it shows the real
screens and flow but plays **sample practice data** and does **not** use a microphone, so **no mic
prompt will appear** and that's normal. Her job is how it **looks, reads, and flows**.

1. **It loads** — "Do you see the AI Music Companion screen in Chrome? Does it look polished or rough?"
2. **Click around** — "Click through the main buttons and menus. Does everything respond, or did anything look blank, dead, or broken?"
3. **Practice session flow** — "Start a practice session and follow it end to end. (It plays a sample session — it won't hear *you* — that's expected.) Did the steps make sense?"
4. **Look & layout** — "Is anything overlapping, cut off, misaligned, or hard to read on your screen?"
5. **Wording** — "Did any text, label, or button name confuse you or seem off?"
6. **Overall** — "1 to 5 overall? And if you could change one thing, what would it be?"

### If MODE = desktop

This is the **real app**: it can hear her and show live pitch. A native window titled **AI Music
Companion** should be open (not Chrome).

1. **It opens** — "Do you see the AI Music Companion window open on its own? How does it look?"
2. **Click around** — "Click through the main buttons and menus. Anything blank, dead, or broken?"
3. **Microphone** — "Start a practice session. If macOS asks to use the microphone, click **Allow**. Did it start listening?"
4. **Play/sing something** — "Play or sing a few notes. Does the live pitch/feedback react to you, and does it feel about right or laggy/off?"
5. **Look & layout** — "Is anything overlapping, cut off, misaligned, or hard to read?"
6. **Overall** — "1 to 5 overall? And if you could change one thing, what would it be?"

If the mic never prompts or nothing reacts, just note it — don't try to troubleshoot with her.

## Step 3 — File her feedback

Compose a clean Markdown report and write it to a temp file (e.g. `/tmp/amc_feedback_body.md`).
Use this shape, filling in her actual words:

```
**Tester:** (her name if she gave one, else "VA")
**Date:** <today>
**App version (commit):** <COMMIT from step 1>
**Run mode:** <web → "Web UI preview (Chrome — sample data, no live audio)" | desktop → "Desktop app (Tauri — live microphone)">

### Checklist
1. <Loads / first impression>: <answer>
2. <Click around>: <answer>
3. <Practice session flow | Microphone>: <answer>
4. <Look & layout | Live pitch reaction>: <answer>
5. <Wording | Look & layout>: <answer>

### Overall
Rating: <1-5>
Would change: <answer>

### Notes
<anything else she said>
```

Title: `[VA Test] <today's date> — <COMMIT>`

Then file it:

```
bash "<skill_dir>/scripts/feedback.sh" "<title>" /tmp/amc_feedback_body.md
```

- On `ISSUE_URL=...`: tell her **"All done — your feedback has been sent to your manager. Thank you! 🎉"**
  (You don't need to show her the URL unless she asks.)
- On `NO_AUTH`: the feedback code isn't set up yet. Save the report somewhere safe
  (`~/amc/last_feedback.md`), and tell her: "I've saved your feedback. Please send your manager
  this note so they can finish setup: 'The feedback code isn't installed yet.'"
- On any `ERROR_...`: reassure her, save the report to `~/amc/last_feedback.md`, and tell her to
  let her manager know it couldn't send. Don't show the error text.

## Step 4 — Clean up

Shut everything down so nothing keeps running in the background (this stops both web and desktop):

```
bash "<skill_dir>/scripts/run.sh" stop
```

Then thank her. For web, she can close the Chrome tab; for desktop, the app window will close.
You're done.

## Notes for you (the assistant)
- Resolve `<skill_dir>` to the real absolute path of this skill's folder before running anything.
- Run the scripts with the Bash tool. They are safe and idempotent.
- Keep every message short and kind. She should never see a stack trace, a port number, or git output.
