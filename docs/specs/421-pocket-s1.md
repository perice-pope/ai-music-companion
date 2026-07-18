# Spec: The Pocket S1 — Anchor click + count-in (#421)

## 1. Summary
The listening metronome's parity floor: a strict Anchor click at a chosen
tempo with a one-bar count-in, played through the existing accompaniment
audio engine, controlled from the session header, visualized as a
BREATHING pulse (rule 0: swells and dims on a continuous curve — never
blinks). Follow/Handoff (S2), Coach's Rhythm (S3), gaps (S4) build on
this floor.

## 2. Problem / why
#421: RV's metronome reimagined. S1 must exist before any listening
personality can — and the pieces are already here: `ears::output::Metronome`
(BPM, meter, accent, decaying sine clicks, hot-path safe) has been
shipped-but-unwired since #212, and `ears::output_engine::AudioOutput`
owns the device lifecycle with a zero-alloc render thread.

## 3. Non-goals
- No Follow/Handoff (no LiveClock wiring), no Coach's Rhythm, no gaps,
  no subdivision journey, no tempo ladder.
- No simultaneous band + click: S1 keeps one audio output owner —
  starting the Pocket stops the band and vice versa (same teardown +
  cmd-lock serialization start_accompaniment uses).
- No beat-accurate visual sync: the pulse animates at the beat period
  from the start timestamp (CSS clock); audible-visual drift over long
  runs is accepted and documented (S2's clock wiring can tighten it).

## 4. Contract / interface
- `Metronome` gains count-in support: `with_count_in(bars: u8)` — during
  the count-in bars every click uses the ACCENT voice (the classic
  "1-2-3-4!" call-off), then normal accent-on-downbeat. Pure, unit-tested
  in ears (no allocation changes to next_sample).
- New commands (commands.rs, mirroring accompaniment's):
  - `start_pocket(tempo_bpm: f64, beats_per_bar: u8, count_in: bool)` —
    clamps tempo to 40..=220, beats 2..=7; tears down any band; starts
    AudioOutput with a Metronome render source; emits `pocket-status
    { playing: true, tempo_bpm }`.
  - `stop_pocket()` — teardown + `pocket-status { playing: false }`.
  - `start_accompaniment` also tears down the Pocket (exclusivity both
    ways).
- Store: `pocketPlaying`, `pocketTempo` (persisted default 90),
  `startPocket()`, `stopPocket()`, event listener for `pocket-status`.
- UI: `PocketControl` in the session header next to AccompanimentToggle —
  BPM stepper (40-220), count-in toggle (default on), start/stop.
  While playing: the breathing pulse (a circle whose scale/opacity
  follows a continuous ease curve at the beat period) + tempo label.

## 5. Acceptance criteria
1. `Metronome::with_count_in(1)`: first bar's every click is the accent
   voice; bar 2 onward accents only beat 1 (sample-level unit test).
2. `start_pocket` at 96 BPM emits pocket-status {playing:true,
   tempo_bpm:96}; stop emits playing:false (mock-Tauri event tests).
3. Tempo/beats clamp: 20→40, 300→220, beats 1→2, 9→7 — no panic, the
   clamped value is what plays AND what the event reports.
4. Starting the Pocket while the band plays stops the band (and vice
   versa) — both statuses emitted; no device contention (cmd-lock).
5. The render path allocates nothing (extend the accompaniment alloc
   test pattern to the pocket source).
6. Frontend: start sends the store's tempo/count-in; the pulse element
   is ONE persistent node whose animation style derives from tempo —
   no mount/unmount per beat (rule 0), asserted via testid identity.
7. Pocket controls disabled outside status==="listening" (same rule as
   the band toggle).
8. Session end stops the Pocket (teardown parity with the band).

## 6. Edge cases
- start_pocket twice: second call restarts cleanly (teardown first).
- Device unavailable (no output device): calm error string, no crash,
  status stays false.
- Tempo NaN/negative over the wire: clamp handles (serde f32; NaN →
  clamp to 40 via .max/.min chain — pin it).
- App with mocks (tests): commands behave without a real device
  (AudioOutput::start error path returns the calm error).

## 7. Test plan
| AC | Test |
|---|---|
| 1 | ears output.rs unit: count-in accent pattern by sample inspection |
| 2 | commands mock-Tauri: start/stop event payloads |
| 3 | commands: clamp table test |
| 4 | commands: the DETERMINISTIC half — pocket start empties the band slot + emits its stopped status before any device work (mock runtime); full audible exclusivity is the ignored audible test + manual verify (the boundary the band's tests drew) |
| 5 | brain tests: pocket_alloc_test (mirror accompaniment_alloc_test) |
| 6 | PocketControl.test: wire shape, pulse node identity across state |
| 7 | PocketControl.test: disabled off-session |
| 8 | commands: end_practice_session stops pocket (status event) |

## 8. Architecture
Click synthesis stays in ears (the Metronome, pre-allocated); device
lifecycle reuses AudioOutput; commands own exclusivity; the frontend
sends semantic settings only. Fully offline, no new dependencies.
