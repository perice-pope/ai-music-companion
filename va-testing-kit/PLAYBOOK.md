# AMC Tester Playbook (live — pulled fresh every run)

This is the source of truth for the `/test-app` walkthrough. The skill reads it after updating, so
edits here reach the tester on her next run with no re-install. Keep it warm and non-technical.

Paths (on her machine):
- Feedback script: `$HOME/amc/ai-music-companion/va-testing-kit/skills/test-app/scripts/feedback.sh`
- Sample files: `$HOME/amc/ai-music-companion/va-testing-kit/samples/`
  - `sample-score-c-major-scale.musicxml` (sheet-music file)
  - `sample-recording-c-major-scale.wav` (audio recording)

Ask one question at a time. Acknowledge warmly. Keep her answers for the report. Offer to skip
anything she gets stuck on. Never show errors/jargon.

---

## WEB walkthrough (MODE = web)

**Before anything — check what today's test is about.** The web preview **cannot** test **sound,
playing or uploading music, the AI's live feedback, or the new "reveals" feature** — those need the
real app. So ask her first: *"Quick check before we start — is today about testing the sound, playing
or uploading music, the AI feedback, or the new 'reveals'? If so, I should run the desktop version
instead so it actually works. Want me to switch?"* If she says yes — or she's unsure and her manager
asked her to test a **new feature** — **stop and run `/test-app desktop` instead** (call
`amc.sh start desktop` and follow the DESKTOP walkthrough). Only continue here for a plain
look-and-feel/wording pass.

Then say, in one friendly sentence: *"This is the quick preview — it shows the real screens but
plays sample data. It won't use your microphone and file upload is turned off here, so we're just
judging how it looks and reads. The real sound testing happens in a separate mode."*

1. **Instruments** — "Do you see a grid of instruments (trumpet, voice, violin, piano…) each with
   an emoji and a family label like Brass or Strings? Pick one — does selecting it feel clear?"
2. **Practice modes** — "Below the instruments there are three modes: **Warm-up**, **Practice**,
   **Run-through**. Read the little description under each. Do they make sense to you?"
3. **Start a session** — "Click **Start Practice**. Up top you should see a timer counting, the
   mode name, a **Play with me** button, and **End Session**. Does that bar look right? (The note
   area will say 'Listening…' and won't move — that's expected in the preview.)"
4. **Score screen** — "Go back and click **Practice with Score**. You'll see a drop area that says
   'Drag a score or recording here.' Read it — is it clear what you could add? (If you try to drop a
   file it'll show an error — that's expected here; just judge the screen and wording.)"
5. **Recap** — "End the session. You should land on a recap titled 'Nice session' with **Strengths**
   listed before **Areas to work on**, and **Practice again** / **Done** buttons. Look right?"
6. **Look & wording** — "Anything overlapping, cut off, or hard to read? Any text or button name
   that confused you?"
7. **Overall** — "1 to 5 overall, and one thing you'd change?"

Note for the report: web mode can't test mic, upload, or AI critique.

---

## DESKTOP walkthrough (MODE = desktop) — the real test

A native window titled **AI Music Companion** is open. This mode tests the two things that matter
most: **what the user sees in the practice modes**, and **uploading music + the AI critiquing your
playing**. She'll need to make sound — humming or singing a few notes is totally fine.

### A. Practice modes & what she sees
1. **Open it** — "Pick an instrument (or **Voice** if you'll hum/sing). Choose the **Practice** mode
   for full coaching. Start a session — if macOS asks for the **microphone**, click **Allow**."
2. **It hears you** — "Hum or play a few steady notes. Does the big note + the pitch meter move and
   roughly match what you're singing? Does it feel quick or laggy?"
3. **'I hear' strip** — "Look for an 'I hear' line. Does it show a tempo (like '120 BPM') and a key
   (like 'G major') while you play?"
4. **Coaching tips** — first check the switch: "Open **Connections & Privacy** and tell me whether
   **'AI coaching narration'** is ON." If it's OFF, tip cards are **expected to be absent** (that's
   the offline mode, not a bug) — note the toggle state and move on. If it's ON: "Do small tip cards
   slide in while you play? Read one to me — does it sound like helpful, sensible feedback?"
   *(Heads-up: the amber **'In the wild'** cards are music **reveals**, not coaching tips — report
   them in the Reveals section, not here.)*
5. **Reveals 🎵 (the new feature)** — "Keep playing in one clear key for a little while — a steady
   tune, nothing random. A small **'In the wild'** card should pop up naming real music that uses
   that sound (like *G Dorian → Miles Davis – 'So What'*). When one appears, tell me:
   **(a)** does it feel **accurate and cool**?  **(b)** when a new one comes, does it **replace** the
   old (never two cards stacked)?  **(c)** is it **occasional** — about once every few phrases, not
   spammy? **(d)** under the card, is there a little counter like **'2 in your collection'** that
   grows only when a *new* reveal appears (a repeat shouldn't grow it)? Then noodle vaguely /
   atonally and confirm **no** card pops up (that's expected)." (If a card never appears even on a
   steady, clearly-pitched passage, note that.)
6. **Mode difference** — "Switch the mode to **Warm-up** and play again. It should go quiet (no
   tips). Does it?"

### A2. The Guided Lesson 🎓 (the BIG new feature — "a private teacher")
While the session is running:
1. **Start it** — "Up top there's a **'Give me a lesson'** button — tap it. Sheet music should
   appear with a header like *'Lesson · drill 1 of 4 · step 0'* and a description of what to play
   (something easy, like one major scale, slow)."
2. **Play a drill** — "Hum or play the line as best you can, then tap **'I played it — grade me'**.
   You should get a percentage and the next drill appears. Does the grade feel roughly fair for how
   you did?"
3. **It adapts** — "On one drill, deliberately do *badly* (or stay silent) before grading. The next
   drill should get **easier** (fewer keys / slower). On drills you nail, later drills should get
   **harder**. Did you notice it adapting?"
4. **The recap** — "After 4 drills you get a **'Lesson complete'** card listing each drill's
   percentage and a line like *'Difficulty: step 0 → step 1'*. Tap Done, then start a **second
   lesson** — it should begin at the step the first one ended on (it remembered you!)."
5. **Escape hatch** — "Start one more lesson and tap **'End lesson'** mid-way — it should calmly
   return you to free play."

### B. Upload music & practice with it
Open the samples folder for her so she can drag a file:
`open "$HOME/amc/ai-music-companion/va-testing-kit/samples"`

6. **Upload a score** — "Go back, click **Practice with Score**, and drag
   **sample-score-c-major-scale.musicxml** from the folder I just opened onto the drop area. Does
   sheet music appear? (If it asks which part, pick the only one.)"
7. **Practice with it** — "Click **Start Practice with This Score**. You should see the sheet music
   with a moving cursor. Play or hum along — does the cursor follow you down the line?"
8. **Upload a recording (bonus)** — "Back at the drop area, drag in
   **sample-recording-c-major-scale.wav**. Does it show 'Listening for notes… / Building the
   score…' and turn into sheet music? (If it errors, just note it — this part is newer.)"

### C. The AI critique
9. **Recap** — "End the session. Read me the recap. Does it actually reflect what you played —
   things like tone, how in-tune you were (a % or 'in tune'), tempo, plus **Strengths**, **Areas to
   work on**, and **Next time, try**? Is it specific and useful, or generic?"
10. **The big question** — "Did it feel like the app truly **heard you** and gave **helpful, specific
    feedback on the music**? What was missing or wrong?"
11. **Overall** — "1 to 5 overall, and the single thing you'd most want changed?"

If the mic never reacts, or upload/critique errors out, just note it plainly in the report — don't
troubleshoot with her.

---

## Filing her feedback (both modes)

Write a Markdown report to `/tmp/amc_feedback_body.md` using her actual words:

```
**Tester:** <name or "VA">
**Date:** <today>
**App version (commit):** <COMMIT from launch>
**Run mode:** <web → "Web preview (look/wording only)" | desktop → "Desktop (live mic, upload, critique)">

### Practice modes & what the user sees
<her answers about instruments, the 3 modes, the session view, pitch/'I hear'/tips>

### Reveals 🎵 (desktop only — new feature)
- A card appeared on a steady, clearly-pitched passage: <yes/no>
- Felt accurate / cool: <answer>
- A new reveal replaced the old (never stacked two): <answer>
- Frequency felt right (occasional, ~1 per few phrases, not spammy): <answer>
- No card on vague/atonal playing (expected): <answer>

### Upload music & practice with it   (desktop only)
- Score (.musicxml) import & render: <answer>
- Cursor follows while playing: <answer>
- Audio (.wav) import & transcription: <answer>

### Guided Lesson 🎓 (desktop only — new feature)
- Lesson started; sheet music + drill header shown: <answer>
- Grades felt roughly fair for how she played: <answer>
- Difficulty adapted (easier after a bad drill, harder after nailed ones): <answer>
- Recap listed all drills + difficulty movement; 2nd lesson started at the new step: <answer>
- "End lesson" exits calmly: <answer>
- Reveal collection counter grows on new reveals only: <answer>

### AI critique / recap   (desktop only)
- Did the recap reflect her actual playing: <answer>
- Helpful & specific vs generic: <answer>

### Look, layout & wording
<answer>

### Overall
Rating: <1-5>
One thing to change: <answer>

### Notes
<anything else>
```

Title: `[VA Test] <today> — <mode> — <COMMIT>`

Then file it:

```
bash "$HOME/amc/ai-music-companion/va-testing-kit/skills/test-app/scripts/feedback.sh" "<title>" /tmp/amc_feedback_body.md
```

- `ISSUE_URL=...` → "All done — your feedback went straight to your manager. Thank you! 🎉"
- `NO_AUTH` → save report to `~/amc/last_feedback.md`; tell her to send her manager: "The feedback
  code isn't installed yet."
- `ERROR_...` → save report to `~/amc/last_feedback.md`; tell her it couldn't send and to let her
  manager know. Don't show the error text.
