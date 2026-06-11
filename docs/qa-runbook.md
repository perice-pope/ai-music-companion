# How to Test AI Music Companion on a Real Computer (QA Runbook)

**Who this is for:** anyone giving a build a thumbs-up before it ships. **You do
not need to be a programmer to do this** — if you can play your instrument and
use a phone camera, you can run every check here.

**When to do it:** before each public release, and any time the audio, the app
window, or the moving sheet-music cursor has changed.

---

## The short version — 3 things you're checking

Our automated tests already prove the app's "thinking" is fast and that it works
with no internet. But a test robot **can't plug in a real microphone, can't open
the real app window, and can't watch a screen actually draw**. Only a human on a
real machine can check those. That's what this document is for.

You're checking three things:

1. **Does it feel instant?** When you play a note, does the screen react with no
   delay you can notice?
2. **Does it run properly on each computer you care about?** (Mac, Windows,
   Linux) — does it open, hear your microphone, and close cleanly?
3. **Does the moving cursor keep up?** When you play along to sheet music, does
   the little line that follows the notes stay glued to where you are, smoothly?

If all three look good on the computers you care about, it's good to ship.

> **Why a human has to do this (the nerdy bit, optional):** the automated speed
> test only times the part of the app that runs as pure math — pitch detection
> and lining your playing up with the score. It deliberately does **not** cover
> three things, because they only exist on real hardware: the **microphone**,
> the **bridge** that carries data from the engine to the screen, and the
> **drawing** of the moving cursor on a real display. This runbook is exactly
> those three.

---

## Before you start (5 minutes of setup)

- [ ] **Use a real "release" build of the app**, not a developer/preview build.
      (A preview build runs slower and would give you misleading results. Ask
      whoever hands you the build to confirm it's a release build, or see the
      Release runbook to make one.)
- [ ] **Plug in your microphone or audio interface with a cable** — *not*
      Bluetooth. Bluetooth headphones/mics add a big delay of their own (a tenth
      of a second or more) that would ruin the timing test.
- [ ] **Find a quiet room**, have your instrument or voice ready, and a tuner
      handy.
- [ ] **Write down what computer you're on:** which one (Mac/Windows/Linux) and
      its version, and which microphone. Speed depends on the machine, so notes
      only make sense compared to the same kind of computer.

---

## Check 1 — Does it feel instant?

**What this proves:** the whole path from your instrument to the screen is fast
enough that you can't catch it lagging. Our target is **under 25 milliseconds**
— about one-fortieth of a second, faster than the eye can notice.

### 1a. The "feel" test (quick, subjective)

1. Open the app, pick your instrument, and start a free-play session (you don't
   need any sheet music for this).
2. Play short, sharp notes — a tongued note on a wind/brass instrument, a
   plucked string, or a crisp "ta" if you're singing.
3. Watch the live pitch display while you do it.

✅ **Pass:** the screen reacts *immediately*. There's no gap you can feel between
hearing your note and seeing the screen move. You shouldn't be able to "beat" the
display by playing fast.

❌ **Fail:** you can feel a lag, the display visibly trails behind your playing,
or fast passages smear together / skip.

> The "feel" test is judgment-based on purpose: people reliably notice a lag
> bigger than about 1/30th of a second, so this catches any big problem fast. For
> an actual number, do test 1b.

### 1b. The slow-motion camera test (gives you a real number)

This is the honest way to measure the true delay from "sound leaves your
instrument" to "screen reacts." It only needs a phone:

1. Put your phone in **slow-motion video mode** (use the highest frame rate it
   offers — 240 frames per second is ideal).
2. Frame the shot so you can see **both** your instrument (or mouth) **and** the
   screen at the same time.
3. Record yourself making one sharp, sudden note (a clap works too).
4. Play the video back **frame by frame.** Count how many frames pass between the
   moment you physically make the sound and the moment the screen first reacts.
5. Turn frames into milliseconds: at 240 fps, each frame is about **4
   milliseconds**. So `number of frames × 4 = the delay in ms`.
6. Do it 5 times and write down the **typical** number and the **worst** one.

✅ **Pass:** the typical delay is **25 ms or less** (about 6 frames or fewer at
240 fps), and the worst is no more than ~35 ms.

❌ **Fail:** the typical delay is over 25 ms. If it fails, tell the engineers
whether the automated speed test (in CI) was still passing — if the math was fast
but the real delay isn't, the slowdown is in the microphone or the
engine-to-screen bridge, not the music analysis.

**Write down:** `Check 1b — typical ___ ms, worst ___ ms, microphone ___`.

---

## Check 2 — Does it run properly on each computer?

**What this proves:** the app actually opens, hears your microphone, and behaves
on each operating system. (The robot tests run "headless" — with no real window —
so they can't catch a problem that only shows up when the real app launches.)

Run this whole list **on each computer you support — Mac, Windows, and Linux** —
using a release build. Mark each one pass/fail on its own, because problems are
often specific to one operating system.

For each computer:

- [ ] **It opens.** The installed app launches (not a developer preview), the
      window appears, and nothing errors out on startup.
- [ ] **It asks for the microphone.** The first time it listens, the operating
      system's "allow microphone?" prompt appears (especially on Mac). Saying
      **yes** starts live audio. Saying **no** shows a polite message — it does
      **not** crash.
- [ ] **It hears you.** Start a session and play — the pitch display updates live.
      (This proves the engine is talking to the screen on this computer.)
- [ ] **It loads sheet music.** Drag in or open a MusicXML file; it draws on
      screen without error.
- [ ] **It gives you a recap.** End a session — you get a summary (or a calm
      "couldn't do the online recap" message if there's no internet).
- [ ] **It closes cleanly.** Closing the window fully quits the app — no leftover
      process still running in the background (check Activity Monitor / Task
      Manager).

✅ **Pass (for that computer):** every box checked, no crash, no freeze, no
leftover process.

❌ **Fail (for that computer):** any crash on opening, no microphone prompt, a
dead pitch display, a drawing error, or a process still running after you closed
it.

**Write down:** `Check 2 — Mac ☐  Windows ☐  Linux ☐` (with each version).

---

## Check 3 — Does the moving cursor keep up?

**What this proves:** when you play along to sheet music, the moving line (the
"cursor") that follows the notes stays smooth and glued to your place — even on a
busy page. (A robot test already checks the cursor's *logic* is efficient, but it
can't watch the screen actually draw at full speed. That's why a person has to
look.)

### Setup

1. Load a **long, busy piece** — at least ~100 measures, ideally a full movement
   with lots of fast notes (a Bach partita, a busy etude). The busier the page,
   the harder the app has to work, which is the point.
2. If you can, turn on a frame-rate / FPS meter for the app window (or just point
   a high-frame-rate phone camera at the screen and watch closely).

### What to do

1. Play (or play a recording) through the piece so the cursor keeps moving
   through the fast passages.
2. While it's following, **make it work harder**: resize the window, scroll, and
   let it hit a phrase ending (which pops up a tip/recap) so the drawing has to
   compete with other work.
3. Watch through at least one **page turn** and one **repeat / jump backward**
   (where the cursor jumps back and re-finds your place).

✅ **Pass:**
- The motion stays smooth — roughly **55–60 frames per second**, no visible
  stutter.
- The cursor stays right on your spot — it never falls more than about one
  measure behind.
- When the music repeats and the cursor jumps backward, it re-finds the spot
  smoothly, with no long freeze.

❌ **Fail:**
- Visible stutter or jerky motion during normal following.
- The cursor lags more than ~1 measure behind, or freezes when it jumps backward.

> If this fails but the engineers' cursor logic test still passes, the problem is
> in the actual on-screen drawing, not the cursor math — point them at the
> sheet-music rendering, not the cursor-position code.

**Write down:** `Check 3 — frames per second ___, piece ___ measures, computer ___`.

---

## Sign-off sheet

Fill this in and attach it to the release. **Ship only when every row you're
responsible for says PASS.**

| Check | In plain English | Result | Number / notes |
|---|---|---|---|
| 1a — feel | Screen reacts instantly to a note | ☐ pass ☐ fail | |
| 1b — camera | Measured delay is under 25 ms | ☐ pass ☐ fail | typical ___ ms |
| 2 — Mac | Opens, hears mic, closes clean on Mac | ☐ pass ☐ fail | |
| 2 — Windows | Opens, hears mic, closes clean on Windows | ☐ pass ☐ fail | |
| 2 — Linux | Opens, hears mic, closes clean on Linux | ☐ pass ☐ fail | |
| 3 — cursor | Moving cursor stays smooth and on-time | ☐ pass ☐ fail | ___ fps |

**The release is a GO only when every row that applies passes.** If Check 1b
fails while the automated speed test was green, the slowdown is in the parts only
real hardware can show (microphone, engine-to-screen bridge, or drawing) — which
means this runbook just did its job and caught something the robots can't.
