# AMC Tester Playbook (live — pulled fresh every run)

This is the source of truth for the `/test-app` walkthrough. The skill reads it after updating, so
edits here reach the tester on her next run with no re-install. Keep it warm and non-technical.

Paths (on her machine):
- Feedback script: `$HOME/amc/ai-music-companion/va-testing-kit/skills/test-app/scripts/feedback.sh`
- Sample files: `$HOME/amc/ai-music-companion/va-testing-kit/samples/`
  - `sample-score-c-major-scale.musicxml` (sheet-music file)
  - `sample-recording-c-major-scale.wav` (audio recording)
  - `sample-band-c-major.mid` (multi-track band MIDI: Trumpet + Bass + Drums)

Ask one question at a time. Acknowledge warmly. Keep her answers for the report. Offer to skip
anything she gets stuck on. Never show errors/jargon.

**If a login or password screen ever blocks the app:** the app has **no login, no account, no
password** — practicing never requires signing in (the only sign-in anywhere is the optional
"sync" box on the History page, which can be ignored). A password screen standing between her and
the instrument picker means the window is showing **some other program by mistake** — the launcher
now detects this and explains it, but if one slips through: don't type anything into it, note the
exact window title and what the screen says, take a screenshot for the manager, and file the
report as blocked with those details.

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

### ⭐ RUN PLAN — multiple tests back to back (2026-07-12 edition)

This week shipped SIX releases (the app should say **v2.26.0** or later — check and record it).
Instead of one marathon session, run **five focused runs, back to back**, each with a FRESH
session (fully **End Session** between runs — the recap must appear each time — then start anew).
One combined report at the end covers all runs; note per run anything that felt off. If a run
blocks completely, screenshot it, skip forward, and keep going — a blocked run is a finding, not
a failure.

**RUN 1 — "Did the fixes land?" (Voice, ~15 min).** The quick health pass over everything fixed
this week: steps **1–5** (privacy switch, pitch meter, 'I hear' strip, coaching tips, and
ESPECIALLY **reveals — step 5**: they broke and were fixed this week, so a steady melody MUST
produce a card again; if no card after ~60–90 seconds of clean playing in one key, that's a
headline finding). Then steps **6–9** (practice-this-sound, note editing, work-on-my-last-lick,
mode difference). End the session and check the recap + step **13b(a)** (the confident-positive
key line on a steady melody).

**RUN 2 — "Jazz ears" (Piano, at a real piano — or chordal music played from your phone,
~20 min).** The week's headline. Steps **11c** (live chord labels: C major → labeled ~within half
a second; G7; C/E inversion; single note → NO label; mashed cluster → 'hearing several notes…'),
then **11e** (Listen to the room: the rolling chord lane, tap-a-chip-to-row, the honest
'several notes…' chips, and after ending — 'What the room played' in the recap with timestamps),
then **11g** (Work on my last progression: the chord SEQUENCE dealt through the keys). Feel-notes
matter more than pass/fail here: does grabbing harmony out of the air feel like magic or like a
gimmick? Say why.

**RUN 3 — "Sheet music" (any instrument you can play a line on, ~20 min).** Steps **10 → 10b**
(score + band-MIDI upload with the Trumpet/Bass picker), **11** (the cursor follows you — it was
overhauled this week: it should track through the WHOLE piece, not stall at measure 1; also
deliberately skip a measure and confirm it catches up), **11b** (live verdicts ✓/~/✗, phrase
cards, the score recap, and the 🎲 row-through-12-keys button), **11f** (tap any measure right on
the notation — NEW), and **12** (the .wav upload; watch for the loading message — a known bug
hunt: tell me EXACTLY what text appears, or doesn't, the moment you drop the file).

**RUN 4 — "Lessons" (Piano, ~15 min).** Section **B** (steps 1–5: start, play, grade, adapt,
recap, escape hatch) plus **11d** (chord drills — the lesson's second drill on Piano should say
'block chords' and show stacked notes; grade honestly, then deliberately fumble). And the **#327
hunt**: a big notation-rendering fix landed this week that may have cured the intermittent blank
lesson sheet — run 4+ lessons across different keys and report ANY render error with its exact
key name (or happily report none).

**RUN 5 — "The stranger test" (installer, ~15 min).** The INSTALLER smoke section below, exactly
as written — download from the public site like a stranger, right-click-open, quick pass. This is
the pilot launch gate: a failure here is a SHIP BLOCKER headline.

Between runs: 10 deep breaths, sip of water — fatigue findings ("I stopped noticing X by run 4")
are real findings too; note them.


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

**Bug hunt (#327 — please do this one deliberately):** the lesson notation sometimes fails with
"Could not render this score" — it depends on the KEY. Run at least **4 separate lessons** (end
and restart to get different keys). For every lesson, jot the key name shown in the colored cell
(like "C#", "G", "Eb"). If the error appears, we need: the exact key name, whether it happened on
drill 1 or later, and whether ending + starting a new lesson cleared it. If 4+ lessons all render
clean, say that too — "didn't reproduce in N tries" is real data.

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
10b. **Upload a BAND file (NEW — MIDI with several instruments).** "Now drag in
   **sample-band-c-major.mid**. The app should ask **which part you want to practice** and list
   exactly two: **Trumpet (8 notes)** and **Bass (4 notes)** — the drum track must NOT be offered
   (drums aren't notes you can practice). Pick Trumpet: does sheet music appear, titled Trumpet?"
   Flag hard: a 'Drums' option appearing, extra garbage notes in the music, or no picker at all.
11. **Practice with it [#279 — THE FIX IS IN, this is the big retest]** — "Click **Start Practice
   with This Score**. You should see the sheet music, and — for the first time in four runs — a
   **translucent highlight cursor** should appear over measure 1 within a moment of playing and
   **follow you down the line** as you play or hum. Also check: **(a)** scroll the score away from
   the cursor mid-take — it must NOT snap back on every beat; **(b)** end the session — the
   highlight disappears (no stale cursor); **(c)** it should NOT appear on guided-lesson drill
   notation (that stays static on purpose)." If it still doesn't move, run
   `grep -iE "follower|score-position" ~/amc/.desktop.log | tail -5` and paste the output.
11b. **The app listens while you practice (NEW — the big one this run).** With the score
   session from step 11 still going: **(a) Live verdicts:** "As you play along, a small tally
   should appear above the music — green ✓ for clean notes, amber ~ for rough ones, red ✗ for
   missed — with little colored dots trailing. Play a few notes right, then play one obviously
   wrong: does the tally react honestly? It should NOT count anything while you're just
   noodling something else entirely." **(b) Phrase cards:** "Pause between phrases — a card
   like 'Measures 1–2 — 4 clean' should appear naming the measures you just played. Does the
   measure range match where you actually were?" **(c) The score recap:** "End the session.
   The recap should have a new section: your piece's name, 'X% of N notes clean, as the app
   followed along', and up to 3 'Measure N — …' rows for the spots that need work. Do those
   measures match where you actually struggled?" **(d) 🎲 THE BUTTON:** "Each worst-measure row
   has 'Row it through 12 keys'. Tap one: you should land back at the instrument picker with a
   green note saying your measure is queued (there's an ✕ to dismiss it). Start a new session —
   the measure should open as colored dots rowed through different keys, exactly like 'Work on
   my last lick'. This is the flagship: tell me how it FEELS, not just whether it works."
11f. **Tap the sheet music itself (NEW this run — RUN 3).** While the score session is running
   (cursor following you), **tap any measure right on the notation** — a subtle highlight
   appears when you hover. "The app should immediately deal THAT measure through 12 keys as
   colored cells — without ending your session ('Back to listening' returns you to the score).
   Tap a measure that's all rests: it should calmly say there's nothing to row, and you stay
   right where you were. Does grabbing a measure straight off the page feel natural?" **Detail
   to watch:** hover before tapping — a faint highlight should outline exactly ONE measure, the
   one under your pointer (if the highlight covers the wrong measure or drifts after you resize
   the window, that's a finding: screenshot with your cursor visible). Note the ✓/~/✗ tally
   BEFORE you tap; when you come back with 'Back to listening', it should be exactly where you
   left it — your row practice must never have counted as score practice.
11c. **Jazz ears — it can hear CHORDS now (NEW this run — RUN 2).** Start a free-play session on
   **Piano** (or Guitar — it's in the instrument list now) near a real piano/guitar, or play a
   chord recording out loud. "**(a)** Play a plain **C major chord** and hold it: within about
   half a second the 'I hear' strip at the top should show a **chord name with a small colored
   dot** — 'C' with a red dot. Play a **G7** — does it say 'G7'? **(b)** Play the same C chord
   but put an **E in the bass** (lowest note): it should say **'C/E'** — the slash is the app
   noticing your bass note. **(c)** Play a **single note** — the chord name should NOT appear
   (one note is not a chord; if it invents one, flag it hard). **(d)** Mash 4–5 random
   neighboring keys at once: it should say **'hearing several notes…'** rather than making up a
   chord name — that honesty is the feature. Tell me: did the labels feel right and steady, or
   did they flicker/lie?" **Detail to watch:** the label should appear within about half a second
   of a clean chord and then hold STEADY while it rings (it may firm up its little slash — 'C'
   becoming 'C/E' — that's correct). A label that flip-flops between two names on one held chord
   is a bug: screenshot it and note both names. Sharp/flat spelling should match the key you're
   in ('Db', never 'C#', when you're playing in flat keys).
11d. **Chord DRILLS — the lesson deals block chords on piano/guitar (NEW this run — RUN 4).** Stay on
   **Piano** (or Guitar) with the real instrument at hand, then start a **Guided Lesson**. "The
   second drill's label should say **'… block chords'** and the notation should show **stacked
   notes** (whole chords, one per measure) instead of a note-by-note arpeggio. **(a)** Play each
   chord the colored cells ask for, one per measure, then tap grade: does the score feel fair?
   **(b)** Do one drill where you deliberately play a couple of WRONG chords — the grade should
   drop honestly, and a 'close' chord (right letter, wrong flavor — like C7 where Cmaj7 was
   asked) should hurt less than a totally wrong one. **(c)** Tap grade once WITHOUT playing
   anything: it should calmly say it didn't hear you yet — never a 0% for silence. On Voice or
   Trumpet the lesson should still deal the old note-by-note arpeggio — chords are only dealt to
   instruments that can play them." **Detail to watch:** grading is forgiving in the right ways
   — noodling a wrong chord BEFORE finding the right one shouldn't tank the score, and a close
   chord (right letter, wrong flavor) should cost about half. If a drill you played essentially
   perfectly grades badly, that's a headline finding: note exactly what you played.
11e. **Listen to the room — jam-along (NEW this run — RUN 2).** In a free-play session, tap the
   **"🎧 Listen to the room"** button under the pitch meter, then play music AT the app —
   a recording on your phone speaker works great (pick something chordal: pop, gospel, jazz).
   "**(a)** Chord chips should roll across the lane as the song plays — each with a colored dot
   and 1–3 little confidence dots. Do the chips roughly match the song's chords changing?
   **(b) THE MOVE:** tap any chip — the app should deal THAT chord through 12 keys as block
   chords, exactly like 'Work on my last lick'. Tell me how that moment FEELS — hearing a song,
   grabbing a chord out of the air, and drilling it is the whole idea. **(c)** Play something
   dense or messy: the lane should show italic 'several notes…' chips instead of inventing
   chord names. **(d)** End the session: the recap should include **'What the room played'** —
   the chord sequence with timestamps. **(e)** Check the fine print on the lane: it should say
   labels only, nothing recorded or sent anywhere." **Detail to watch:** chips should arrive a
   beat or two AFTER each chord change (that small lag is normal — it's being careful, not slow);
   each chip carries 1–3 little dots (more dots = more confident — do hesitant chips really look
   hesitant?); the lane holds about the last 8 things heard. If a chip ever names a chord that
   was flatly wrong (not just simplified — e.g. it said F when the song was clearly on C), note
   the song + the moment.
11g. **Work on my last progression (NEW this run — RUN 2).** After playing (or playing a recording of)
   a few chords in any session — the jam lane is perfect for this — tap **"🎲 Work on my last
   progression"**. "The app should deal the chord SEQUENCE it just heard (like 'Dm7 → G7 →
   Cmaj7') through the keys (3 or more, growing as you level up) — each key plays all the
   chords in a row as block chords. Tap it
   before playing any chords: it should calmly ask you to play a couple first. Does drilling a
   whole progression through the keys feel like the RV idea leveled up?" **Detail to watch:**
   the header should name YOUR chords in order with arrows (like 'your progression · Dm7 → G7 →
   Cmaj7') and the spelling should stay consistent even after tapping the little variation chips
   ('Make it spicy' etc.) — if any chord name suddenly changes its sharp/flat spelling after a
   chip tap, screenshot before/after.
12. **Upload a recording (bonus)** — "Back at the drop area, drag in
   **sample-recording-c-major-scale.wav**. Does it show 'Listening for notes… / Building the
   score…' and turn into sheet music? (If it errors, just note it — this part is newer.)"

### C. The AI critique
13. **Recap** — "End the session. Read me the recap. Does it actually reflect what you played —
   things like tone, how in-tune you were (a % or 'in tune'), tempo, plus **Strengths**, **Areas to
   work on**, and **Next time, try**? Is it specific and useful, or generic?"
13b. **The key line is honest now — AND finally says yes (your #334 'one thing')** — the recap's
   "Key:" line should be exactly as confident as the app actually was. Two quick sessions:
   **(a) Stable — IMPORTANT: sing a MELODY, not one held note.** "Sing a simple scale or tune
   that stays in one key — do-re-mi-fa-sol up and down a few times is perfect — for 30–60
   seconds, then end. One held note can't name a key (that's why your A# test showed nothing —
   that was correct), but a melody can. This time the recap should state the key **plainly**
   ('Key: G major'), it must MATCH what the 'I hear' strip showed, and — new — look for the
   **confident positive** you asked for: a Strengths line like **'Clear tonal center — the
   session sat firmly in G major.'** That line existing at all is the fix; tell me if it
   doesn't appear on a genuinely steady melodic session."
   **(b) Wandering:** "Now noodle around — drift between keys on purpose, maybe settle somewhere
   only right at the end, then end the session. The recap should NOT state a key flatly. It should
   either say **'leaning … toward the end'** or **'kept moving — normal for exploratory
   playing'**, and NO Strengths line may claim it 'sat firmly' anywhere." Flag hard: a flat key
   claim that differs from the strip's end state (the original half-step bug), or a hedge on a
   genuinely steady melodic session (the bug just fixed).
14. **The big question** — "Did it feel like the app truly **heard you** and gave **helpful,
    specific feedback on the music**? What was missing or wrong?"
15. **Overall** — "1 to 5 overall, and the single thing you'd most want changed?"

If the mic never reacts, or upload/critique errors out, just note it plainly in the report — don't
troubleshoot with her.

---

## INSTALLER smoke test (run when the manager asks — simulates a real pilot user)

This is the exact path a stranger takes, and it has NEVER been human-tested: the packaged app is a
different build than the dev mode the kit runs (bundled audio engine, no dev server). ~15 minutes
plus a download.

1. **Download like a stranger:** open https://perice-pope.github.io/ai-music-companion-pitch/ in a
   browser, scroll to Download, click **Download for macOS**. Note anything scary the browser says.
2. **Install & first launch:** open the .dmg, drag the app to Applications, then — important —
   **right-click the app → Open** (double-clicking will be blocked; the site says so — did YOU
   find that instruction where you needed it?). Approve the microphone prompt.
3. **Does the packaged app actually work?** Quick pass, no deep testing: pick Voice → start a
   session → hum — does the pitch meter move? Play steadily — does a reveal appear? Drag in
   **sample-score-c-major-scale.musicxml** — does it render, does the cursor follow? Drag in
   **sample-recording-c-major-scale.wav** — does the beta transcription produce anything (this
   uses the BUNDLED audio engine, the part most likely to differ from dev mode)?
4. **Report:** every difference from the dev version, every scary dialog, and the version shown
   anywhere in the app. If the app won't open or crashes at launch, that's a SHIP BLOCKER —
   screenshot + exact message.

## Filing her feedback (both modes)

Write a Markdown report to `/tmp/amc_feedback_body.md` using her actual words:

```
**Tester:** <name or "VA">
**Date:** <today>
**App version (commit):** <COMMIT from launch>
**Run mode:** <web → "Web preview (look/wording only)" | desktop → "Desktop (live mic, upload, critique)">
**App version shown:** <from the app — should be v2.26.0+>

### Run log (back-to-back plan)
- Run 1 (fixes check, Voice): <done / skipped / blocked — one line>
- Run 2 (jazz ears, Piano): <same>
- Run 3 (sheet music): <same>
- Run 4 (lessons + chord drills): <same>
- Run 5 (installer smoke): <same>
- Fatigue notes (what you stopped noticing, when): <answer>

### Practice modes & what the user sees
<her answers about instruments, the 3 modes, the session view, pitch/'I hear'/tips>

### Reveals 🎵 (desktop only — new feature)
- A card appeared on a steady, clearly-pitched passage: <yes/no>
- Felt accurate / cool: <answer>
- A new reveal replaced the old (never stacked two): <answer>
- Frequency felt right (occasional, ~1 per few phrases, not spammy): <answer>
- No card on vague/atonal playing (expected): <answer>

### Upload music & practice with it   (desktop only)
- **[NEW] Band MIDI picker:** offered exactly Trumpet + Bass (no Drums); chosen part rendered
  with its own name: <answer>
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

### Tap-the-page measure bridge 🎼 (desktop only — NEW)
- Tapping a measure on the notation dealt it through 12 keys mid-session: <answer>
- "Back to listening" returned to the score session: <answer>
- Rest-only measure refused calmly (no crash, no navigation): <answer>
- Scrolling the score still works normally (the overlay never blocks it): <answer>

### Progression lift 🎲 (desktop only — NEW)
- The heard chord sequence dealt through 12 keys as block chords: <answer>
- Label named the actual progression (e.g. Dm7 → G7 → Cmaj7): <answer>
- Tapping with no chords heard refused calmly: <answer>
- Her words on how progression-through-12-keys FELT: <answer>

### Jam-along — Listen to the room 🎧 (desktop only — NEW)
- Chord chips rolled by and roughly tracked the song's changes: <answer>
- Tapping a chip dealt that chord through 12 keys (block chords): <answer>
- How the grab-a-chord-from-a-song moment FELT (her words): <answer>
- Dense/messy audio showed "several notes…", never invented names: <answer>
- Recap included "What the room played" with timestamps: <answer>
- Privacy line present (labels only, nothing recorded/sent): <answer>

### Chord drills 🎹 (desktop only — NEW, piano/guitar lessons)
- Lesson on Piano/Guitar dealt a "block chords" drill with stacked notation: <answer>
- Playing the right chords graded fairly: <answer>
- Wrong chords dropped the grade; a close chord (right root) hurt less: <answer>
- Grading silence said "didn't hear you yet", not 0%: <answer>
- Voice/Trumpet lessons still deal the note-by-note arpeggio: <answer>

### Guided Lesson 🎓 (desktop only — new feature)
- **[#327 hunt — a big notation-render fix landed this week; this may be CURED] Keys tried
  across 4+ lessons; any render error with its exact key name, or happily none:** <answer>
- Lesson started; sheet music + drill header shown: <answer>
- NEW look: colored cells + transparent notation + a real key signature: <answer>
- Grades felt roughly fair for how she played: <answer>
- Difficulty adapted (easier after a bad drill, harder after nailed ones): <answer>
- Recap listed all drills + difficulty movement; 2nd lesson started at the new step: <answer>
- "End lesson" exits calmly: <answer>
- Reveal collection counter grows on new reveals only: <answer>

### Jazz ears — live chord labels 🎹 (desktop only — NEW)
- C major chord → chord name + colored dot appeared in the "I hear" strip within ~½ s: <answer>
- G7 named correctly: <answer>
- C chord with E in the bass → "C/E" slash label: <answer>
- Single note → NO chord label (flag hard if it invents one): <answer>
- Random cluster → "hearing several notes…" instead of a made-up name: <answer>
- Labels steady vs flickering; her words on how it felt: <answer>

### AI critique / recap   (desktop only)
- **[#277 re-test]** Recap key matches what the live "I hear" strip actually showed during the
  session (watch the strip while playing; compare at the end): <answer>
- **[#316 re-test — the 'coin flip' fix]** Stable one-key session → recap states the key plainly
  and it matches the strip's end state: <answer>
- **[#316 re-test]** Wandering session → recap hedges ("leaning … toward the end") or says "kept
  moving" — never flatly names a key, and no Strengths line claims it "sat firmly": <answer>
- **[#277 re-test]** The "Flavour" line changes between two different sessions (do one swung/jazzy,
  one plain — read both lines aloud): <answer>
- Did the recap reflect her actual playing: <answer>
- Helpful & specific vs generic: <answer>

### Installer smoke test (only when run)
- Download + right-click-Open flow worked as the site describes: <answer>
- Packaged app: mic / reveals / score+cursor / .wav each worked: <answer>
- Differences vs the dev version, scary dialogs, version string: <answer>

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
