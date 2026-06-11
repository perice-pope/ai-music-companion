# Test the App — Simple Pass/Fail Checklist

**For:** anyone (no tech experience needed). You'll open the app, look at the
screen, and mark each step **PASS** or **FAIL**.

**You'll need:** the app installed on the Mac, and someone nearby to play an
instrument or sing when a step asks for sound.

Go one step at a time. After each, decide **PASS** (it did what it should) or
**FAIL** (it didn't, or something looked broken).

---

## 1. The app opens

- Double-click **AI Music Companion** (in the Dock, or the Applications folder).

**PASS** if a window appears within a few seconds and there's no error message.
**FAIL** if nothing opens, it crashes, or an error pops up.

---

## 2. It asks to use the microphone

- Pick an instrument and press **Start** (or the listen/record button).
- A small box should ask permission to use the microphone — click **Allow** / **OK**.

**PASS** if it asks and you clicked Allow (or it was already allowed before).
**FAIL** if it never asks and the app can't hear anything, or it crashes.

---

## 3. It reacts to sound right away

- With listening started, ask the player to play **a few short, sharp notes**.
- Watch the moving pitch display / needle / meter on screen.

**PASS** if the screen reacts **instantly** — no delay you can notice between the
sound and the screen moving.
**FAIL** if there's a clear lag, the screen trails behind, or fast notes get
missed.

---

## 4. It shows sheet music (skip if there's no music file handy)

- Open or drag a music file into the app.

**PASS** if the sheet music appears on screen with no error.
**FAIL** if it errors, freezes, or nothing shows.
*(If you don't have a music file, skip this and mark it N/A.)*

---

## 5. The moving line follows the music (skip if you skipped step 4)

- With sheet music on screen, ask the player to **play along** with it.
- Watch the little moving line/cursor that follows the notes.

**PASS** if the line stays **smooth** and stays roughly where they're playing —
not jerky, not stuck, not far behind.
**FAIL** if it stutters badly, freezes, or falls way behind.

---

## 6. It gives a summary at the end

- Press **Stop** / **End session**.

**PASS** if a summary / recap screen appears (a short message is fine).
**FAIL** if it crashes or nothing happens.

---

## 7. It closes cleanly

- Quit the app: click the red dot, or press **⌘ Q**.

**PASS** if it fully closes with no error and nothing hangs.
**FAIL** if it freezes, errors, or seems stuck shutting down.

---

## Results

Tell whoever's reviewing the result of each step:

| Step | What it checks | PASS / FAIL |
|---|---|---|
| 1 | App opens | |
| 2 | Asks for microphone | |
| 3 | Reacts to sound instantly | |
| 4 | Shows sheet music | |
| 5 | Moving line follows along | |
| 6 | Gives a summary at the end | |
| 7 | Closes cleanly | |

**All good** if every step is PASS (steps 4–5 may be N/A if you had no music file).

---

> **Want an exact timing number?** (optional, for an engineer) Film step 3 with a
> phone in slow-motion at 240 fps, with the instrument and screen both in frame.
> Count the frames between the sound and the screen reacting; each frame is about
> 4 ms. Aim for under 25 ms (about 6 frames). Deeper hardware checks for other
> operating systems live in this file's git history.
