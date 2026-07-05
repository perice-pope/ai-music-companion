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

> **Monday session (#277 re-test):** last time several things didn't pass — this run re-checks each
> of them explicitly, plus everything new from the weekend (the app looks and behaves differently in
> places; that's expected). Where a step says **[#277 re-test]**, compare against her last report.

A native window titled **AI Music Companion** is open. This mode tests the two things that matter
most: **what the user sees in the practice modes**, and **uploading music + the AI critiquing your
playing**. She'll need to make sound — humming or singing a few notes is totally fine.

### A. Practice modes & what she sees
1. **Open it** — "FIRST, from the picker screen, open **Connections & Privacy** and tell me whether
   **'AI coaching narration'** is ON (the Back button there lands on the History page — just tap
   back to the picker). Then pick an instrument (or **Voice** if you'll hum/sing), choose the
   **Practice** mode, and start a session — if macOS asks for the **microphone**, click **Allow**.
   Keep the app window **big / full-screen** — some cards live in a side column that hides when the
   window is narrow."
2. **It hears you** — "Hum or play a few steady notes. Does the big note + the pitch meter move and
   roughly match what you're singing? Does it feel quick or laggy?"
3. **'I hear' strip** — "Look for an 'I hear' line. Does it show a tempo (like '120 BPM') and a key
   (like 'G major') while you play?"
4. **Coaching tips** — (you checked the switch in step 1) If **'AI coaching narration'** was OFF,
   tip cards are **expected to be absent** (that's the offline mode, not a bug) — note the toggle
   state and move on. If it was ON: "Do small tip cards slide in while you play? Read one to me —
   does it sound like helpful, sensible feedback?"
   *(Heads-up: the amber **'In the wild'** cards are music **reveals**, not coaching tips — report
   them in the Reveals section, not here.)*
5. **Reveals 🎵 (the new feature)** — "Keep playing in one clear key for a little while — a steady
   tune, nothing random. A small **'In the wild'** card should pop up naming real music that uses
   that sound (like *G Dorian → Miles Davis – 'So What'*). When one appears, tell me:
   **(a)** does it feel **accurate and cool**?  **(b)** when a new one comes, does it **replace** the
   old (never two cards stacked)?  **(c)** is it **occasional** — about once every few phrases, not
   spammy? **(d) [#277 re-test]** can you actually READ each card now — does every card stay up at
   least a few seconds even while the key reading wobbles? **(e)** under the card, is there a little
   counter like **'2 in your collection'** that grows only when a *new* reveal appears? Then noodle
   vaguely / atonally and confirm **no** card pops up (that's expected)." (If a card never appears
   even on a steady, clearly-pitched passage, note that.)
6. **🎲 Practice this sound (NEW)** — "When a reveal card is up, tap **'Practice this sound'** under
   it. A slim staff of **colored dots (no stems!)** should appear right in free play, with a row of
   **colored note cells** showing the keys in order, and up to three buttons like **'New keys 🎲'**,
   **'Make it spicy'**, **'Different scale'**. Tap each one: does the music visibly change to match?
   Does **'Back to listening'** return you to normal free play?"
7. **Edit the notes (NEW)** — "While exploring: **tap any dot** — a gold ring appears with little
   buttons (8va ↑/↓, ♯, ♭, ✕). **Drag a dot up or down** — it should snap to lines and spaces, and
   when you let go, THE SAME change appears in **every key section** of the exercise. Try **↩ undo**
   — does it bring back exactly what was there? Also tap **'♪ rhythms'** — stems appear on the same
   dots without anything moving around; tap again, they vanish."
8. **🎲 Work on my last lick (NEW — the big one)** — "Back in normal free play, play any short
   melody you make up (5–10 notes), pause a beat, then hit **'Work on my last lick'**. Your OWN
   melody should appear as colored dots, repeated through several different keys. Is it really what
   you played? If a note is wrong, drag it to fix it — in every key at once. If you tap the button
   before playing anything, it should politely say to play a phrase first (not crash)."
9. **Mode difference** — "Switch the mode to **Warm-up** and play again. It should go quiet (no
   tips). Does it?"

### A2. The Guided Lesson 🎓 (the BIG new feature — "a private teacher")
While the session is running:
1. **Start it** — "Up top there's a **'Give me a lesson'** button — tap it. Sheet music should
   appear with a header like *'Lesson · drill 1 of 4 · step 0'* and a description of what to play.
   **(NEW look)** The notation should be smaller and see-through (no big white page), with a row of
   **colored note cells** above it showing the drill's keys in play order — do the colors and cells
   show up? **(founder check)** Does the sheet music show a proper **key signature** (sharps/flats
   at the start of the staff) instead of accidentals on every note?"
2. **Play a drill** — "Hum or play the line as best you can, then tap **'I played it — grade me'**.
   You should get a percentage and the next drill appears. Does the grade feel roughly fair for how
   you did?"
3. **It adapts** — "On one drill, deliberately play *wrong notes* before grading (don't just stay
   silent — if it heard nothing it will say 'I didn't catch that yet' instead of grading). The next
   drill should get **easier** (fewer keys / slower). On drills you nail, later drills should get
   **harder**. Did you notice it adapting?"
4. **The recap** — "After 4 drills you get a **'Lesson complete'** card listing each drill's
   percentage and a line like *'Difficulty: step 0 → step 1'*. Tap Done, then start a **second
   lesson** — it should begin at the step the first one ended on (it remembered you!)."
5. **Escape hatch** — "Start one more lesson and tap **'End lesson'** mid-way — it should calmly
   return you to free play."

### A3. Your Keys wheel 🎡 (NEW)
Back on the **instrument picker screen** (end the session or tap Done first):
1. "Below the instruments there's a **colorful wheel of 12 keys**. When you first opened the app
   today it was dim, saying **'play to light up'** (if you've practiced on this machine before,
   some keys may already glow — that's your history, it remembers). After your lesson, do the keys
   you drilled show **brighter** than the rest? Tap a bright one — does a little card show your
   drills, best %, and the scales you worked?"
2. "Do a second lesson, come back, and check the wheel again — did anything change?"

### B. Upload music & practice with it
Open the samples folder for her so she can drag a file:
`open "$HOME/amc/ai-music-companion/va-testing-kit/samples"`

10. **Upload a score** — "Go back, click **Practice with Score**, and drag
   **sample-score-c-major-scale.musicxml** from the folder I just opened onto the drop area. Does
   sheet music appear? (If it asks which part, pick the only one.)"
11. **Practice with it [#277 re-test]** — "Click **Start Practice with This Score**. You should see
   the sheet music with a moving cursor. Play or hum along — does the cursor follow you down the
   line?" **If the cursor doesn't move**, don't stop: keep the session going a moment, then run
   `grep -iE "follower|score-position" ~/amc/.desktop.log | tail -5` and paste the output into the
   report — the app now logs exactly which part went quiet (#279).
12. **Upload a recording (bonus)** — "Back at the drop area, drag in
   **sample-recording-c-major-scale.wav**. Does it show 'Listening for notes… / Building the
   score…' and turn into sheet music? (If it errors, just note it — this part is newer.)"

### C. The AI critique
13. **Recap** — "End the session. Read me the recap. Does it actually reflect what you played —
   things like tone, how in-tune you were (a % or 'in tune'), tempo, plus **Strengths**, **Areas to
   work on**, and **Next time, try**? Is it specific and useful, or generic?"
14. **The big question** — "Did it feel like the app truly **heard you** and gave **helpful,
    specific feedback on the music**? What was missing or wrong?"
15. **Overall** — "1 to 5 overall, and the single thing you'd most want changed?"

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

### Explore / "Practice this sound" 🎲 (desktop only — NEW)
- Tapping the reveal's button opened a variation in free play: <answer>
- Colored STEMLESS dots on a slim staff (no white page): <answer>
- Each chip visibly changed the music as named: <answer>
- "Back to listening" returned to normal free play: <answer>

### Editing + My Lick 🎼 (desktop only — NEW)
- Tap-select showed the gold ring + little buttons: <answer>
- Dragging a dot fixed the note in EVERY key section: <answer>
- Undo restored exactly the previous version: <answer>
- "♪ rhythms" added stems without moving anything: <answer>
- "Work on my last lick" showed my own melody through the keys: <answer>
- Tapping it before playing gave a polite message (no crash): <answer>

### Your Keys wheel 🎡 (desktop only — NEW)
- Wheel visible on the picker; dim + "play to light up" before practicing: <answer>
- Keys brightened after lessons; tap-detail showed drills/%/scales: <answer>

### Guided Lesson 🎓 (desktop only — new feature)
- Lesson started; sheet music + drill header shown: <answer>
- NEW look: colored cells + transparent notation + a real key signature: <answer>
- Grades felt roughly fair for how she played: <answer>
- Difficulty adapted (easier after a bad drill, harder after nailed ones): <answer>
- Recap listed all drills + difficulty movement; 2nd lesson started at the new step: <answer>
- "End lesson" exits calmly: <answer>
- Reveal collection counter grows on new reveals only: <answer>

### AI critique / recap   (desktop only)
- **[#277 re-test]** Recap key matches what the live "I hear" strip actually showed during the
  session (watch the strip while playing; compare at the end): <answer>
- **[#277 re-test]** The "Flavour" line changes between two different sessions (do one swung/jazzy,
  one plain — read both lines aloud): <answer>
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
