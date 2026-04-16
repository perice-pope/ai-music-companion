# AI Music Companion -- User Testing Guide

This document tells you exactly what to test at each phase of the app, from a musician's perspective. You do not need to be an engineer. You need your instrument, a microphone, and a willingness to play a few notes and tell us what happened.

---

## How to Use This Guide

Each phase below has:

1. **Prerequisites** -- what you need before you sit down to test.
2. **Test scenarios** -- numbered checklists. Do them in order. Check them off as you go.
3. **What to look for** -- how you know it is working (or not).
4. **User notes template** -- copy this, fill it in, and send it back.
5. **Known limitations** -- things that are *supposed* to not work yet. Do not file bugs for these.

When something feels wrong -- even if you cannot articulate why -- write it down. "The tuner felt laggy" is useful. "The display was jittery when I played above the staff" is even better.

---

## Phase 0: Live Pitch Display (COMPLETE)

**What exists:** A desktop window that listens to your microphone, detects pitch in real time, and shows you the note name, frequency, cents deviation, and a tuning meter bar.

### Prerequisites

- [ ] macOS computer (Apple Silicon or Intel)
- [ ] A working microphone -- built-in laptop mic is fine for testing, but an external USB mic or audio interface will give cleaner results
- [ ] Your instrument (trumpet, voice, violin, clarinet, or piano)
- [ ] A quiet room -- background noise confuses pitch detection
- [ ] The app built and running locally (ask a developer to help you launch it if needed: `pnpm tauri dev` from the `apps/desktop` directory)

### Test Scenarios

#### 0.1 -- App Launch and Idle State

- [ ] 1. Open the app.
- [ ] 2. Confirm you see the title "AI Music Companion" and the subtitle "Phase 0 -- Spike."
- [ ] 3. Confirm you see "Backend says:" followed by a message in green text (this proves the Rust backend is connected).
- [ ] 4. Confirm the pitch display area shows "Listening..." (meaning the mic is active but you have not played anything yet).

#### 0.2 -- Single Long Tone

- [ ] 1. Pick up your instrument (or warm up your voice).
- [ ] 2. Play or sing a comfortable, sustained note -- for trumpet, try a middle G (concert F, ~349 Hz). For voice, try an A4 (~440 Hz). For violin, try open A string.
- [ ] 3. Hold the note steady for at least 3 seconds.
- [ ] 4. Watch the display: you should see a note name appear (e.g., "F", "A", "G"), the octave number next to it, the frequency in Hz below, and the cents deviation.
- [ ] 5. Observe the tuning meter bar. The indicator dot should sit near center if you are in tune, drift left if flat, drift right if sharp.
- [ ] 6. Observe the color: green means within 10 cents, yellow means 10-25 cents off, red means more than 25 cents off.
- [ ] 7. Stop playing. The display should return to showing "Listening..." or stop updating within a moment.

#### 0.3 -- Chromatic Walk

- [ ] 1. Play a slow chromatic scale across roughly one octave of your comfortable range.
- [ ] 2. Hold each note for about 1-2 seconds.
- [ ] 3. Watch the note name update as you move between pitches. Each note name should change to match what you are playing.
- [ ] 4. Pay attention to how quickly the display reacts when you move to a new note. It should feel nearly instant -- no perceptible lag.

#### 0.4 -- Extreme Registers

- [ ] 1. Play the lowest note you can comfortably produce on your instrument.
- [ ] 2. Does the app detect it? Write down the note and whether detection worked.
- [ ] 3. Play the highest note you can comfortably produce.
- [ ] 4. Does the app detect it? Write down the note and whether detection worked.
- [ ] 5. Note: the pitch detector covers roughly 60 Hz to 2000 Hz. If you are a soprano singing above C6 (~1047 Hz), or a violinist in very high positions, detection may drop out. That is expected -- record what happens.

#### 0.5 -- Dynamics

- [ ] 1. Play a long tone starting pianissimo (as soft as you can) and crescendo to fortissimo.
- [ ] 2. Does the pitch display appear at the soft end? There is a silence gate -- very quiet playing (below a certain threshold) will show no pitch. Note the dynamic level where detection kicks in.
- [ ] 3. Does the pitch stay stable at fortissimo, or does it jump around?
- [ ] 4. Play a note at mezzo-forte and add vibrato (if your instrument/voice naturally does this). Does the note name stay stable, or does it flicker between two note names?

#### 0.6 -- Confidence Indicator

- [ ] 1. While playing a clear, sustained tone, look at the "confidence" percentage at the bottom of the pitch display.
- [ ] 2. For a clean tone, confidence should be above 80%.
- [ ] 3. Try singing or playing with a breathy, unfocused tone. Does confidence drop?
- [ ] 4. Clap your hands near the mic. The display should briefly show no pitch or very low confidence, since a clap is not a pitched sound.

#### 0.7 -- Background Noise

- [ ] 1. Without playing, talk near the microphone. Does the app try to show pitch for your speaking voice? (It might -- speech has pitch. This is fine, just note the behavior.)
- [ ] 2. Turn on a fan or AC near the mic. Does it cause false pitch readings?
- [ ] 3. Play your instrument with the fan still running. Does pitch detection still work reasonably?

### What to Look For

| Quality | Good | Concerning |
|---|---|---|
| **Latency** | Note appears the instant you start playing -- feels like a hardware tuner | Visible delay between playing a note and seeing it on screen (more than ~50ms feels "sluggish") |
| **Accuracy** | Matches a trusted tuner (e.g., your clip-on tuner or a tuner app you trust) within 5 cents | Consistently reads a different note name, or cents reading disagrees with your trusted tuner by more than 15 cents |
| **Stability** | Holds steady on a sustained tone with minimal jitter | Flickers rapidly between two note names, or cents value jumps around by more than 10 cents on a steady tone |
| **Onset response** | When you tongue/bow/sing a new note, the display updates within one beat at 120 bpm (~500ms) | Takes more than a second to recognize you changed notes |
| **Visual clarity** | You can read the note name from arm's length while playing | Text is too small, colors are hard to distinguish, meter bar is hard to see |

### User Notes Template -- Phase 0

```
Tester name:
Date:
Instrument:
Microphone used:
Room conditions (quiet/noisy):

--- 0.1 App Launch ---
Did it launch?                [ ] Yes  [ ] No
Backend message appeared?     [ ] Yes  [ ] No
"Listening..." shown?         [ ] Yes  [ ] No
Notes:

--- 0.2 Single Long Tone ---
Note played:
Displayed note name:
Displayed Hz:
Displayed cents:
Does it match your tuner?     [ ] Yes  [ ] Close  [ ] No
Tuning meter felt:            [ ] Accurate  [ ] Jumpy  [ ] Wrong
Color coding correct?         [ ] Yes  [ ] No
Notes:

--- 0.3 Chromatic Walk ---
Notes tracked correctly?      [ ] All  [ ] Most  [ ] Few  [ ] None
Latency feel:                 [ ] Instant  [ ] Slight delay  [ ] Laggy
Any notes that were misread?  List them:
Notes:

--- 0.4 Extreme Registers ---
Lowest note tried:            Detected? [ ] Yes  [ ] No
Highest note tried:           Detected? [ ] Yes  [ ] No
Notes:

--- 0.5 Dynamics ---
Softest dynamic detected:     (pp / p / mp / mf / etc.)
Stable at fortissimo?         [ ] Yes  [ ] No
Vibrato handled well?         [ ] Yes  [ ] Flickered  [ ] Lost note
Notes:

--- 0.6 Confidence ---
Confidence on clean tone:     ___%
Confidence on breathy tone:   ___%
Clap caused false pitch?      [ ] Yes  [ ] No
Notes:

--- 0.7 Background Noise ---
Speech caused readings?       [ ] Yes  [ ] No
Fan caused false readings?    [ ] Yes  [ ] No
Playing with fan worked?      [ ] Yes  [ ] Partially  [ ] No
Notes:

--- Overall Impressions ---
What worked well:

What felt wrong or frustrating:

Suggestions:
```

### Known Limitations -- Phase 0

- **No instrument profile selection in the UI.** The pitch detector uses a default frequency range (60-2000 Hz). You cannot yet pick "trumpet" or "violin" to tailor it. The JSON profiles exist in the codebase but are not connected to the frontend.
- **No practice feedback.** The app only shows what note you are playing. It does not tell you if you are playing correctly, give coaching advice, or follow a score.
- **No session history.** Nothing is saved. When you close the app, all data is gone.
- **Monophonic only.** If you play a chord on piano, the detector will try to pick one pitch out of the cluster. Results will be unpredictable.
- **No MIDI input.** Only microphone input works.
- **Single-channel detection.** Even if your mic sends stereo, the app processes one channel.
- **macOS only.** Windows and Linux builds are not tested yet.

---

## Phase 1: Practice Companion MVP

**What will exist:** The core practice loop. The app listens to you play, understands musical phrases, gives you AI coaching feedback between phrases, follows along with a score (or just listens in free play mode), and remembers your sessions.

### Prerequisites

- [ ] Everything from Phase 0, plus:
- [ ] A stable internet connection (the AI coaching engine calls Claude or GPT-4 -- it is not local)
- [ ] At least one MusicXML or MIDI file of a piece you know well (for Score Mode testing). Good candidates:
  - A simple etude or method book exercise (Arban, Schradieck, Vaccai, Klos&eacute;, etc.)
  - A slow melody you can play from memory (for comparing free play vs. score mode)
- [ ] 15-20 minutes per test session (the AI needs time to observe your playing before it can give useful feedback)

### Test Scenarios

#### 1.1 -- Instrument Profile Selection

- [ ] 1. Open the app.
- [ ] 2. Look for an instrument selection control (dropdown, sidebar, or onboarding screen).
- [ ] 3. Select your instrument (Trumpet, Soprano Voice, Violin, Clarinet, or Piano).
- [ ] 4. Confirm the app acknowledges your selection.
- [ ] 5. Play a note in your instrument's normal range. Pitch detection should work as before.
- [ ] 6. Switch to a different instrument profile (e.g., switch from Trumpet to Violin). Does the app update?
- [ ] 7. If you are a trumpet player: play a low F# (1-2-3 valve combination). Does the app account for the known intonation tendency of that fingering? (Phase 1 stretch goal -- it may not yet.)

#### 1.2 -- Free Play Mode (No Score)

- [ ] 1. Select Free Play mode (no score loaded).
- [ ] 2. Play a phrase of 4-8 notes -- something lyrical, like the first line of a song you know.
- [ ] 3. Pause at the end of the phrase (lift the instrument, take a breath, or simply stop).
- [ ] 4. Wait for AI feedback. The app should recognize your phrase ended and present coaching text.
- [ ] 5. Read the feedback. It should reference what you just played -- pitch accuracy, timing, dynamics, or musical suggestions. It should feel like a comment from a knowledgeable teacher, not a generic platitude.
- [ ] 6. Play another phrase. Does the feedback evolve? Does it reference the previous phrase or notice improvement?
- [ ] 7. Play something intentionally out of tune (lip a note sharp, sing flat on purpose). Does the AI catch it?
- [ ] 8. Play something intentionally rushed (speed through a passage). Does the AI comment on timing?

#### 1.3 -- Free Play: Extended Session (5+ Minutes)

- [ ] 1. Play freely for at least 5 minutes -- scales, long tones, a melody, whatever you would do in a normal warm-up.
- [ ] 2. Does the AI coaching stay relevant throughout? Or does it start repeating itself?
- [ ] 3. Does the feedback become more specific over time as the AI "learns" your playing?
- [ ] 4. At the end of 5 minutes, stop playing.
- [ ] 5. Look for a "Session Recap" or summary screen. It should appear automatically or be accessible via a button.
- [ ] 6. Read the recap. Does it accurately summarize what happened? Does it highlight your strengths and areas to work on?

#### 1.4 -- Score Mode: Import and Setup

- [ ] 1. Switch to Score Mode.
- [ ] 2. Import a MusicXML or MIDI file.
- [ ] 3. Confirm the app loads the score without errors.
- [ ] 4. Look for a visual representation of the music (this might be a simplified note display, not full notation rendering in Phase 1 -- note what you see).
- [ ] 5. Confirm the app shows the key signature, time signature, and tempo (or at least some of these).

#### 1.5 -- Score Mode: Playing Along

- [ ] 1. With a score loaded, start playback/practice mode.
- [ ] 2. Play the first phrase of the piece.
- [ ] 3. Watch for score following: does the app track where you are in the music? Look for a cursor, highlight, or indicator that moves with you.
- [ ] 4. Play in tempo. Does the follower keep up?
- [ ] 5. Slow down deliberately (practice tempo). Does the follower adjust?
- [ ] 6. Skip ahead by 4 bars and start playing from there. Does the score follower recover and find your position?
- [ ] 7. Play a wrong note on purpose. Does the AI notice? Does it wait until the phrase ends to tell you, or does it interrupt?
- [ ] 8. At the end of the piece (or a section), check for AI feedback. Does it reference specific measures or passages by location?

#### 1.6 -- Score Mode: Difficult Passages

- [ ] 1. Load a piece that has at least one passage you find technically difficult.
- [ ] 2. Play through the piece, including the hard part.
- [ ] 3. After the difficult passage, does the AI feedback specifically address it?
- [ ] 4. Play the difficult passage again. Does the AI compare your second attempt to the first?
- [ ] 5. Does the AI suggest specific practice strategies (slow it down, isolate the interval, use a metronome, etc.)?

#### 1.7 -- Practice History

- [ ] 1. After completing a practice session (at least 5 minutes), close the session.
- [ ] 2. Look for a practice history or session log somewhere in the app.
- [ ] 3. Confirm your session appears with the date, duration, and instrument.
- [ ] 4. Open the session detail. Can you see the recap and any feedback from that session?
- [ ] 5. Complete a second session on a different day (or at a different time).
- [ ] 6. Confirm both sessions appear in the history.
- [ ] 7. Close the app completely and reopen it. Is your history still there?

#### 1.8 -- AI Coaching Quality

This is the most important subjective test. Play normally for 10-15 minutes and evaluate:

- [ ] 1. Does the AI sound like it understands your instrument? (A trumpet player should not get bowing advice.)
- [ ] 2. Is the feedback musically literate? Does it use correct terminology for your instrument?
- [ ] 3. Is the feedback actionable? ("Try supporting that D with more air" is actionable. "Good job!" is not.)
- [ ] 4. Does the feedback match what a decent private teacher would say? Would you trust it?
- [ ] 5. Is the feedback encouraging without being sycophantic? Does it find real positives, not just filler praise?
- [ ] 6. Does it notice patterns? (E.g., "You tend to rush when ascending" or "Your intonation drops in the upper register.")
- [ ] 7. Is there anything the AI said that was flat-out wrong? Write down the exact feedback and what you were playing.

### What to Look For

| Quality | Good | Concerning |
|---|---|---|
| **Phrase detection** | AI knows when your phrase ended without you pressing a button | AI cuts you off mid-phrase, or waits forever after you stop |
| **Feedback relevance** | Comments are specific to what you just played | Generic advice that could apply to anyone playing anything |
| **Score following** | Cursor stays within 1 beat of your position, even when you slow down or pause | Cursor gets lost, jumps around, or stalls |
| **Coaching tone** | Sounds like a supportive but honest teacher | Sounds like a chatbot, or is overly critical, or is empty praise |
| **Session recap** | Accurate summary that you would agree with | Mentions things you did not play, misses obvious issues |
| **Latency** | Feedback appears within 2-3 seconds of finishing a phrase | More than 5 seconds feels like the app froze |
| **History persistence** | Sessions survive app restart | Data disappears, or sessions are missing details |

### User Notes Template -- Phase 1

```
Tester name:
Date:
Instrument:
Instrument profile selected:
Microphone used:
Piece(s) used (if score mode):

--- 1.1 Profile Selection ---
Instrument selector found?    [ ] Yes  [ ] No
Profile switch worked?        [ ] Yes  [ ] No
Notes:

--- 1.2 Free Play ---
Phrase detection worked?      [ ] Yes  [ ] Sometimes  [ ] No
Feedback appeared after pause? [ ] Yes  [ ] No
Feedback was relevant?        [ ] Yes  [ ] Somewhat  [ ] No
AI caught intentional errors? [ ] Yes  [ ] No
Most useful feedback received (quote it):

Most unhelpful/wrong feedback (quote it):

--- 1.3 Extended Session ---
Duration played:              ___ minutes
Feedback stayed fresh?        [ ] Yes  [ ] Became repetitive
Session recap appeared?       [ ] Yes  [ ] No
Recap was accurate?           [ ] Yes  [ ] Partially  [ ] No
Notes:

--- 1.4-1.6 Score Mode ---
File imported:                (filename and format)
Import succeeded?             [ ] Yes  [ ] No  [ ] Error: ____________
Score following accuracy:     [ ] Spot on  [ ] Close  [ ] Lost often
Recovered after skipping?     [ ] Yes  [ ] No
Feedback referenced measures? [ ] Yes  [ ] No
Difficult passage addressed?  [ ] Yes  [ ] No
Practice strategy suggested?  [ ] Yes  [ ] No
Notes:

--- 1.7 Practice History ---
Session saved?                [ ] Yes  [ ] No
Survives app restart?         [ ] Yes  [ ] No
Notes:

--- 1.8 Coaching Quality ---
Instrument-appropriate?       [ ] Yes  [ ] No  (give example if no)
Musically literate?           [ ] Yes  [ ] No  (give example if no)
Actionable?                   [ ] Yes  [ ] Mostly generic
Matches a real teacher?       [ ] Yes  [ ] Somewhat  [ ] No
Noticed patterns?             [ ] Yes  [ ] No
Anything flat-out wrong?      [ ] Yes  [ ] No
If yes, describe:

--- Overall Impressions ---
Would you use this in a real practice session?  [ ] Yes  [ ] Maybe  [ ] No
What worked well:

What felt wrong or frustrating:

What a private teacher does that this app does not:

Suggestions:
```

### Known Limitations -- Phase 1

- **Phrase-level analysis, not note-level.** The AI evaluates groups of notes, not every individual note. It will not tell you "the third note of measure 7 was 12 cents flat" -- it will say "your intonation in that passage was a bit low."
- **No sheet music rendering.** The app imports scores for analysis but may not display traditional notation. You will likely see a simplified visual or just a cursor position.
- **Internet required for coaching.** The LLM feedback requires an API call. No internet means no coaching -- pitch detection still works offline.
- **Limited score format support.** MusicXML and MIDI only. No PDF sheet music import yet (that is Phase 2). No hand-written notation.
- **Monophonic instruments only for accurate analysis.** Piano chords will confuse the pitch detector. Single-line piano melodies should work.
- **No backing tracks or accompaniment.** You play alone.
- **English-only coaching.** AI feedback is in English.
- **Cloud sync not available.** Practice history is stored locally in SQLite. If you switch computers, your data stays behind.

---

## Phase 2: Smart Import and Tone Quality

**What will exist:** Photo import of paper sheet music, YouTube-to-score extraction, tone quality assessment, backing track separation, and cloud sync across devices.

### Prerequisites

- [ ] Everything from Phase 1, plus:
- [ ] A smartphone or webcam (for photo import of sheet music)
- [ ] Physical sheet music to photograph -- ideally both printed and handwritten, to test limits
- [ ] A YouTube URL of a piece you know (for YouTube-to-score testing)
- [ ] A recording of yourself or an ensemble (MP3, WAV, or similar -- for backing track separation)
- [ ] A Supabase account (for cloud sync testing -- the team will provide sign-up instructions)
- [ ] A second device (another Mac, or eventually an iPad/phone) to test cloud sync
- [ ] 20-30 minutes per test session

### Test Scenarios

#### 2.1 -- Photo Import of Sheet Music

- [ ] 1. Find a piece of printed sheet music -- something with clear notation, not too dense. A single page from a method book works well.
- [ ] 2. Use the app's import feature to photograph or upload the sheet music.
- [ ] 3. Wait for processing (optical music recognition can take 10-30 seconds per page).
- [ ] 4. Compare the imported score to the original. Check for:
  - Correct note pitches
  - Correct rhythms
  - Correct key signature
  - Correct time signature
  - Accidentals (sharps, flats, naturals)
  - Dynamics markings (if the app captures them)
- [ ] 5. Count the errors. Write down specific measures where the import got it wrong.
- [ ] 6. Now try a piece with more complexity: two staves, dense rhythms, or small print. How does it handle it?
- [ ] 7. Try photographing handwritten music (if you have any). Does it work at all?
- [ ] 8. Try a poor-quality photo: bad lighting, angled, partially obscured. At what point does it fail?

#### 2.2 -- YouTube Import

- [ ] 1. Find a YouTube video of a solo performance of a piece you know well -- ideally a clean recording without too much reverb or accompaniment.
- [ ] 2. Paste the URL into the app's YouTube import feature.
- [ ] 3. Wait for processing (audio extraction + transcription). This may take 1-2 minutes.
- [ ] 4. Examine the resulting score/MIDI. Does it capture the melody accurately?
- [ ] 5. Try playing along with the imported score. Does the score follower work with it?
- [ ] 6. Try a YouTube video with accompaniment (piano + solo instrument, for example). Does the app isolate the solo part?
- [ ] 7. Try a video with poor audio quality or audience noise. How does it degrade?

#### 2.3 -- Tone Quality Assessment

- [ ] 1. Play a long tone with your best, most focused sound. Hold for 4-5 seconds.
- [ ] 2. Look for a tone quality indicator or rating. The app should give you feedback beyond just pitch -- something about the color, clarity, or richness of your sound.
- [ ] 3. Now deliberately play with a thin, pinched tone. Does the assessment change?
- [ ] 4. Play with a spread, unfocused tone. Does the assessment catch that?
- [ ] 5. If you are a brass player: compare an open tone to a muted tone. Does the app distinguish them?
- [ ] 6. If you are a string player: compare arco to pizzicato. Does the app handle both?
- [ ] 7. If you are a vocalist: compare a chest-voice tone to a head-voice tone. Does the assessment adapt?
- [ ] 8. Play the same note at different dynamics (pp, mf, ff). Does the tone rating change with volume, or does it appropriately focus on quality independent of loudness?

#### 2.4 -- Backing Track Separation (Demucs)

- [ ] 1. Upload a recording that contains multiple instruments (a band recording, an ensemble piece, or an accompaniment track).
- [ ] 2. Wait for the separation to complete (this is computationally intensive -- could take a few minutes).
- [ ] 3. Listen to the separated tracks. Can you hear your instrument's part isolated?
- [ ] 4. Listen to the "minus one" track (everything except your instrument). Is it clean? Can you practice over it?
- [ ] 5. Play along with the minus-one track. Does the app's pitch detection still work while the backing track is playing? (This depends on audio routing.)
- [ ] 6. Rate the separation quality: are there artifacts, ghosting, or missing parts?

#### 2.5 -- Cloud Sync

- [ ] 1. Log in with your Supabase account on your primary device.
- [ ] 2. Complete a practice session.
- [ ] 3. Confirm the session appears in your local history.
- [ ] 4. Log in on a second device with the same account.
- [ ] 5. Check practice history on the second device. Does your session appear?
- [ ] 6. Complete a session on the second device. Go back to the first device and confirm it synced.
- [ ] 7. Disconnect from the internet. Complete a session. Reconnect. Does it sync afterward?
- [ ] 8. Log out and log back in. Is your data still there?

### What to Look For

| Quality | Good | Concerning |
|---|---|---|
| **Photo import accuracy** | 95%+ notes correct on clean printed music | More than 1 error per line of music, or wrong key/time signature |
| **YouTube import** | Captures the melody and rhythm of a clear solo recording | Misses large sections, wrong key, or rhythm is unrecognizable |
| **Tone assessment** | Distinguishes your best tone from a deliberately poor tone; feedback matches what your teacher would say | Rates a pinched tone the same as a resonant tone, or gives random/inconsistent ratings |
| **Track separation** | You can practice over the minus-one track and it sounds musical | Severe artifacts, missing instruments, or your part bleeds through heavily |
| **Cloud sync** | Seamless -- you forget it is even happening | Data is missing, duplicated, or takes more than 30 seconds to appear |

### User Notes Template -- Phase 2

```
Tester name:
Date:
Instrument:

--- 2.1 Photo Import ---
Sheet music used:             (title, printed or handwritten)
Photo quality:                [ ] Good  [ ] Fair  [ ] Poor
Processing time:              ___ seconds
Notes correct?                [ ] All  [ ] Most  [ ] Many errors
Rhythms correct?              [ ] All  [ ] Most  [ ] Many errors
Key/time signature correct?   [ ] Yes  [ ] No
Specific errors found (list measure numbers):

Handwritten music attempted?  [ ] Yes -- worked  [ ] Yes -- failed  [ ] No
Notes:

--- 2.2 YouTube Import ---
YouTube URL used:
Recording quality:            [ ] Clean  [ ] Some noise  [ ] Poor
Processing time:              ___ minutes
Melody captured accurately?   [ ] Yes  [ ] Mostly  [ ] No
Rhythm captured accurately?   [ ] Yes  [ ] Mostly  [ ] No
Accompaniment isolated?       [ ] Yes  [ ] Partially  [ ] No
Notes:

--- 2.3 Tone Quality ---
Best tone rating:
Pinched tone rating:
Spread tone rating:
Feedback felt accurate?       [ ] Yes  [ ] Sometimes  [ ] No
Feedback was instrument-specific? [ ] Yes  [ ] Generic
Notes:

--- 2.4 Backing Tracks ---
Recording used:               (describe the source)
Processing time:              ___ minutes
Separation quality:           [ ] Clean  [ ] Some artifacts  [ ] Unusable
Minus-one track musical?      [ ] Yes  [ ] Barely  [ ] No
Could practice over it?       [ ] Yes  [ ] No
Notes:

--- 2.5 Cloud Sync ---
Sync worked across devices?   [ ] Yes  [ ] Partially  [ ] No
Offline session synced later? [ ] Yes  [ ] No
Notes:

--- Overall Impressions ---
Most impressive new feature:

Most frustrating new feature:

Suggestions:
```

### Known Limitations -- Phase 2

- **OMR is imperfect.** Optical music recognition (via Audiveris) works best on clean, printed, single-staff music. Dense orchestral scores, handwritten music, and poor-quality scans will produce errors. Always verify the import against the original.
- **YouTube extraction depends on audio quality.** A clean solo recording will transcribe much better than a live concert with reverb, audience noise, and multiple instruments.
- **Tone quality ML model is new.** It has been trained primarily on trumpet, voice, and violin samples. Clarinet and piano assessments may be less nuanced in early releases.
- **Track separation takes time.** Demucs is not real-time. Expect 1-5 minutes per song depending on length and your hardware.
- **Cloud sync requires account creation.** Local-only mode still works without an account.
- **No iOS or Android app yet.** Cloud sync is between desktop machines only.
- **Backing track playback and pitch detection simultaneously may require careful audio routing.** If the backing track comes through your speakers and the mic picks it up, pitch detection will be confused. Use headphones when practicing with backing tracks.

---

## Phase 3: Teacher Platform and Mobile

**What will exist:** A teacher dashboard for managing students, iOS/Android apps, cross-session intelligence, assignment workflows, and AI-generated progress reports.

### Prerequisites

- [ ] Everything from Phase 2, plus:
- [ ] An iPhone or Android phone (for mobile app testing)
- [ ] A second person to act as "teacher" (or test both roles yourself with two accounts)
- [ ] Teacher account credentials (the team will provide setup instructions)
- [ ] Student account credentials (separate from teacher)
- [ ] 30+ minutes per test session
- [ ] Ideally, 2-3 weeks of accumulated practice history (to test cross-session intelligence and progress reports)

### Test Scenarios

#### 3.1 -- Mobile App: Basic Functionality

- [ ] 1. Install the app on your phone (iOS App Store or Android Play Store).
- [ ] 2. Log in with your existing account.
- [ ] 3. Confirm your practice history from desktop appears on mobile.
- [ ] 4. Start a practice session on mobile. Does pitch detection work using the phone's microphone?
- [ ] 5. Play a few phrases and check for AI feedback. Does coaching work on mobile?
- [ ] 6. Complete the session. Does it appear in your history on desktop afterward?
- [ ] 7. Test in different locations: a practice room, a bedroom, outdoors. How does the phone mic handle different acoustics?
- [ ] 8. Check battery usage after a 15-minute session. Is it reasonable?
- [ ] 9. Test with the phone on a music stand versus handheld. Which works better for mic pickup?

#### 3.2 -- Mobile App: Score Mode

- [ ] 1. Open a score you previously imported (from desktop, via cloud sync).
- [ ] 2. Play along on mobile. Does score following work on the phone?
- [ ] 3. Import a new score directly on mobile (photo or file). Does it work?
- [ ] 4. Is the score display readable on a phone screen? Can you see enough notes ahead to play comfortably?
- [ ] 5. Try landscape orientation. Is it better for score display?

#### 3.3 -- Teacher Dashboard: Student Setup

- [ ] 1. Log in with a teacher account on desktop.
- [ ] 2. Navigate to the teacher dashboard.
- [ ] 3. Add a student to your roster (by email or invite link).
- [ ] 4. Confirm the student appears in your roster.
- [ ] 5. On the student's device/account, accept the teacher invitation.
- [ ] 6. Have the student complete a practice session.
- [ ] 7. Return to the teacher dashboard. Can you see the student's session?

#### 3.4 -- Teacher Dashboard: Session Review

- [ ] 1. From the teacher dashboard, open a student's recent practice session.
- [ ] 2. Can you see what the student practiced, for how long, and what feedback the AI gave?
- [ ] 3. Can you see pitch accuracy, timing, and dynamics data for the session?
- [ ] 4. Can you add your own notes or comments to the session?
- [ ] 5. Does the student see your comments on their device?

#### 3.5 -- Assignment System

- [ ] 1. From the teacher dashboard, create an assignment for a student: assign a specific piece or exercise.
- [ ] 2. Set a due date and any practice instructions (e.g., "Work on measures 17-24, focus on intonation in the chromatic passage").
- [ ] 3. Confirm the assignment appears on the student's device.
- [ ] 4. Have the student open the assignment and start practicing the assigned piece.
- [ ] 5. After the student's session, check the teacher dashboard. Can you see that the student worked on the assignment?
- [ ] 6. Does the AI feedback reference the teacher's instructions? (e.g., does it focus on intonation in mm. 17-24 as the teacher requested?)

#### 3.6 -- Cross-Session Intelligence

This requires accumulated data. Ideally, test after 2-3 weeks of regular use.

- [ ] 1. After multiple sessions over several days or weeks, look for long-term observations from the AI.
- [ ] 2. Does the AI reference previous sessions? ("Last Tuesday you struggled with this passage -- today it was much cleaner.")
- [ ] 3. Does it identify recurring patterns? ("You consistently rush accelerando passages" or "Your intonation on sustained notes above the staff tends to go sharp.")
- [ ] 4. Does it track improvement? ("Your tone quality in the middle register has improved over the past two weeks.")
- [ ] 5. Does it suggest what to work on next based on your history?

#### 3.7 -- Weekly Progress Reports

- [ ] 1. After at least one week of practice data, look for a weekly progress report (it may be emailed or available in-app).
- [ ] 2. Does the report accurately summarize the week? (Total practice time, pieces worked on, areas of focus.)
- [ ] 3. Does it highlight improvements and persistent challenges?
- [ ] 4. Is the tone constructive? Would a student feel motivated reading it?
- [ ] 5. **Teacher view:** From the teacher dashboard, view the progress report for a student. Does it give you enough information to plan the next lesson?
- [ ] 6. Would you share this report with a parent? Is it clear to a non-musician?

#### 3.8 -- Multi-Student Teacher Workflow

- [ ] 1. Add 3+ students to your roster.
- [ ] 2. Assign different pieces to different students.
- [ ] 3. Can you quickly scan your roster and see who has practiced this week and who has not?
- [ ] 4. Can you compare progress across students? (Not to rank them -- to understand your studio's needs.)
- [ ] 5. Can you send a group assignment to all students at once?
- [ ] 6. When a new student joins mid-semester, is the onboarding flow smooth?

### What to Look For

| Quality | Good | Concerning |
|---|---|---|
| **Mobile mic quality** | Usable pitch detection from phone mic in a quiet room | Phone mic is too noisy to get reliable readings |
| **Mobile UX** | Easy to start a session with one hand while holding your instrument | Requires too many taps, tiny buttons, confusing navigation |
| **Cross-device sync** | Practice on phone at school, review on desktop at home -- seamless | Missing sessions, duplicates, or long sync delays |
| **Teacher dashboard** | Scan 10 students in under 2 minutes, drill into any session in one click | Slow to load, buried navigation, too much clicking |
| **Cross-session intelligence** | AI references your actual history and gets it right | AI fabricates details about past sessions, or never references history |
| **Progress reports** | You would forward it to a parent or put it in a student's file | Vague, inaccurate, or not useful for lesson planning |
| **Assignment workflow** | Teacher assigns in 30 seconds, student sees it immediately | Assignments get lost, wrong piece shows up, or due dates are confusing |

### User Notes Template -- Phase 3

```
Tester name:
Date:
Role:                         [ ] Student  [ ] Teacher  [ ] Both
Instrument(s):
Devices used:

--- 3.1-3.2 Mobile App ---
Platform:                     [ ] iOS  [ ] Android
Phone model:
Pitch detection on phone:     [ ] Good  [ ] Usable  [ ] Unreliable
Score readable on phone?      [ ] Yes  [ ] Cramped  [ ] No
Cloud sync worked?            [ ] Yes  [ ] Partially  [ ] No
Battery impact after 15 min:  ___ % drain
Best phone position:          (stand, handheld, propped on piano, etc.)
Notes:

--- 3.3-3.4 Teacher Dashboard ---
Student setup smooth?         [ ] Yes  [ ] Confusing
Session data visible?         [ ] Yes  [ ] Partially  [ ] No
Could add comments?           [ ] Yes  [ ] No
Student saw comments?         [ ] Yes  [ ] No
Notes:

--- 3.5 Assignments ---
Assignment creation easy?     [ ] Yes  [ ] No
Student received it?          [ ] Yes  [ ] No
AI used teacher instructions? [ ] Yes  [ ] No
Notes:

--- 3.6 Cross-Session Intelligence ---
Weeks of data accumulated:    ___
AI referenced past sessions?  [ ] Yes  [ ] No
Identified recurring patterns?[ ] Yes  [ ] No
Tracked improvement?          [ ] Yes  [ ] No
Anything fabricated/wrong?    [ ] Yes  [ ] No
If yes, describe:

--- 3.7 Progress Reports ---
Report was accurate?          [ ] Yes  [ ] Partially  [ ] No
Useful for lesson planning?   [ ] Yes  [ ] Somewhat  [ ] No
Appropriate for parents?      [ ] Yes  [ ] No
Notes:

--- 3.8 Multi-Student (Teachers Only) ---
Number of students managed:   ___
Roster overview useful?       [ ] Yes  [ ] No
Time to scan all students:    ___ minutes
Group assignments worked?     [ ] Yes  [ ] No
Notes:

--- Overall Impressions ---
Compared to your current teaching/practice tools, this is:
[ ] Much better  [ ] Somewhat better  [ ] About the same  [ ] Worse

What worked well:

What felt wrong or frustrating:

What would make you switch from your current workflow:

Suggestions:
```

### Known Limitations -- Phase 3

- **Phone microphones vary widely.** An iPhone 15 in a quiet room will give dramatically different results than a budget Android phone in a noisy apartment. Test results are specific to the device and environment.
- **Cross-session intelligence needs data.** If the AI has only seen you play twice, its long-term observations will be shallow. Give it 2-3 weeks of regular use before judging this feature.
- **Progress reports are AI-generated.** They may occasionally misattribute a pattern or miss context. Teachers should review them before sharing with parents.
- **The teacher dashboard is desktop-first.** A mobile teacher dashboard is a future consideration. For Phase 3, teacher features are designed for desktop browsers.
- **Assignment system assumes score availability.** If a piece has not been imported into the system, the teacher cannot assign it. Build a shared library first.
- **Internet required for all teacher/student sync features.** Offline practice still works, but the teacher will not see it until the student's device syncs.
- **Privacy and data considerations.** Student practice recordings may be processed by cloud LLM APIs. The team should clarify data handling policies before deploying in schools or studios with minors.

---

## General Testing Tips

**For all phases:**

- **Use headphones** when the app plays any audio back to you (metronome, backing tracks). Otherwise the mic picks up the playback and pitch detection goes haywire.
- **Warm up first.** The app does not know if you are cold. Your first five minutes of testing will reflect your chops, not the app's accuracy. Warm up, then test.
- **Compare against a trusted tuner.** Keep your clip-on tuner or a tuner app running alongside the AI Music Companion. If they disagree, write down what each one says.
- **Test across your full range.** Do not just play in the comfortable middle -- push the extremes. Apps often break at the edges.
- **Test with different dynamics.** Pianissimo and fortissimo are harder for pitch detection than mezzo-forte. Try all of them.
- **Take screenshots.** If you see something wrong or confusing, screenshot it before it disappears.
- **Be honest.** "This is not useful to me" is valuable feedback. We would rather hear it now than after launch.

**For musicians who are not tech-savvy:**

- You do not need to understand how pitch detection works. You just need to know if the note name on screen matches what your ear and your tuner say.
- If the app crashes, freezes, or shows an error, write down what you were doing right before it happened. That is the most useful thing you can tell us.
- If something takes too long, time it with your phone. "The AI took about 8 seconds to respond" is more useful than "it was slow."

---

## Reporting Issues

When you find a bug or something that does not feel right:

1. **What were you doing?** ("I was playing a chromatic scale starting on low C.")
2. **What did you expect?** ("I expected the display to show C, C#, D, D#, etc.")
3. **What actually happened?** ("It showed C, then jumped to E, skipped D entirely.")
4. **Can you reproduce it?** ("It happens every time I play D4 softly.")
5. **Screenshots or screen recordings** are extremely helpful.

Send your completed notes templates and any bug reports to the development team. Every single observation helps -- even the ones that feel minor.
