# Spec: Follow / Handoff — the listening metronome (#421 S2, E3)

## 1. Summary
The Pocket earns its name. Three modes: **Anchor** (S1, unchanged),
**Follow** (the click locks to YOUR measured pulse), and **Handoff**
(follows for a spell, then freezes — "I've got your 96. Now hold it" —
with an honest drift line while you hold). No new threads, no new
dependencies: the perception stream already measures tempo at ~8 Hz;
the Metronome's update_config is already phase-preserving.

## 2. Contract
- Backend: `set_pocket_tempo(tempo_bpm: f64)` — clamps via the S1
  clamp, forwards through a lock-free SPSC control channel into the
  pocket's render source, which applies `update_config` (the
  phase-preserving path — the grid shifts smoothly, never pops).
  No-op when the Pocket is silent. The render source wraps Metronome +
  receiver; the alloc law holds (try_pop in render, no heap).
- Frontend (the S2b routed-measurement pattern — orchestration policy
  here, measurement stays backend):
  - Mode chips on PocketControl: anchor | follow | handoff (exclusive,
    anchor default, disabled off-session, reset on session end).
  - FOLLOW: while playing, confident perception tempo
    (locked/confidence gate + 40..=220 range) updates the click via
    set_pocket_tempo, throttled to ≥1 s between sends and ≥2 BPM
    change (no jitter-chasing).
  - HANDOFF: follows identically for HANDOFF_FOLLOW_SECS = 8, then
    FREEZES (stops sending) and shows the drift line: "holding your
    {frozen} — you're {+N/-N} BPM {ahead/behind}" from live perception
    tempo vs the frozen value; within ±2 BPM it reads "in the pocket".
    Rule 0: the line holds and updates in place, never blinks.
- The pulse keeps breathing at the CURRENT tempo (label + animation
  re-time in place — S1's identity discipline).

## 3. ACs
1. set_pocket_tempo re-times a playing click smoothly (update_config
   path pinned; clamp shared with S1 — played == reported).
2. Silent pocket → set_pocket_tempo is a calm no-op.
3. Follow sends only on confident, changed (≥2 BPM), throttled (≥1 s)
   readings; wobbly/unconfident perception sends nothing.
4. Handoff stops sending after the follow window and the drift line
   derives from frozen-vs-live; "in the pocket" within ±2 BPM; the
   line updates in place (node identity).
5. Mode chips exclusive; anchor default; mode resets at session end;
   switching modes mid-play behaves (follow→anchor stops sending).
6. The render path still never allocates (alloc test extended to the
   channel-fed source).
7. S1 behavior byte-identical when mode=anchor (existing suites).

## 4. Test map
| AC | Test |
|---|---|
| 1,2 | commands: set_pocket_tempo clamp + silent no-op (+ channel unit in ears) |
| 3 | store/control test: throttle, delta gate, confidence gate |
| 4 | PocketControl: freeze after window, drift line text + identity |
| 5 | PocketControl: chip exclusivity, reset, mid-play switch |
| 6 | ears pocket_alloc_test: channel-fed source, zero alloc |
| 7 | existing pocket suites unchanged |
