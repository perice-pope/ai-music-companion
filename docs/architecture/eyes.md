# RFC: Eyes — Computer Vision for Technique Analysis

**Status:** Exploration → proposed v1 scope
**Author:** Perice Pope (+ Claude)
**Last updated:** 2026-04-23
**Related:** [architecture-v2.md](./architecture-v2.md), [mobile.md](./mobile.md)

---

## Why Eyes

A 1-on-1 teacher diagnoses through **two** channels at once:

- **What came out** — ears. Pitch, rhythm, intonation, timbre.
- **How it was produced** — eyes. Embouchure, posture, hand shape, bow angle, breathing, tension.

Audio alone tells us a note is flat. Only vision tells us *why*: jaw locked, shoulders raised, bow pulling off-angle toward the fingerboard, wrist collapsed, fingers flying off the keys. Without the "how", our coaching reduces to pitch-correction feedback — the Tonestro/SmartMusic pattern the v2 arch explicitly rejects (§4a, §8).

Adding a vision channel is the highest-leverage thing we can do for coaching quality after the DTW follower lands.

## High-signal targets per family

| Family     | What a teacher watches for                                                                 |
|------------|--------------------------------------------------------------------------------------------|
| Brass      | Embouchure corners, mouthpiece angle, puffed cheeks, jaw tension, shoulder rise, posture   |
| Voice      | Jaw drop, raised larynx, facial tension, shoulder/chest posture, breath mechanics          |
| Strings    | Bow hold, bow-to-bridge angle, bow travel straightness, left-hand thumb, wrist collapse    |
| Woodwind   | Embouchure, finger height (no slapping), hand tension, posture, breath                     |
| Piano      | Hand arch (flat = tension), wrist level, finger independence, shoulder/forearm tension     |

Most of what a teacher catches most of the time is a **geometric relationship between body landmarks**, not a rich scene-understanding problem. This matters for the tech choice below.

## Non-negotiable design constraints

These are product-design constraints, not technical ones. Write them down before engineering starts or we'll paint ourselves into a corner.

1. **Coach, don't judge — applied harder.** The v2 §4a/§8 rule "no per-note green/yellow/red" applies **tenfold** to video. A red X on a 10-year-old's posture will make them quit in a week. Vision observations must route through the same coaching layer as audio observations — never spawn a new scorekeeper. No stoplight colors on body parts, ever.
2. **On-device by default, always.** No frame leaves the device without an explicit "save/share this take" action from the user. The default path is camera → landmarks → discarded.
3. **Rolling buffer only.** No persistent video storage unless the user asks. We keep the last N seconds in a ring buffer for "show me that last attempt" review; older frames are overwritten.
4. **Parental consent flow** before the camera ever turns on for an under-13 account. COPPA applies. No camera access without an adult-account confirmation in the setup flow.
5. **Respect the hardware indicator.** No hidden recording. If the OS shows a camera-on LED/dot, we earned it and we're doing something visible in the UI.
6. **FERPA review** before the first school pilot. Schools have a different consent model and stricter data-handling requirements than consumers.
7. **Opt-in, not opt-out.** Ears works fine without eyes. The app must be fully usable with the camera permission denied.

## Tech stack

Split the workload by latency budget:

### Real-time landmarks (the 90% case) — MediaPipe Tasks

Google's MediaPipe Tasks is the answer for almost everything a teacher's eye catches in the moment. It runs on-device via TFLite, ~10–20 ms/frame on a modern phone, cross-platform (iOS, Android, desktop, web).

Three models we'd use immediately:

- **Hand Landmarker** — 21 points per hand. Enough to compute valve depression, finger curl, bow grip, piano arch.
- **Face Landmarker** — 468 points including mouth/jaw. Enough for embouchure shape, jaw drop, facial tension proxies.
- **Pose Landmarker** — 33 points. Shoulders, spine, arm angles → posture, shoulder rise on inhale.

Our code consumes **landmarks**, not frames. Teacher heuristics become geometric rules:

```rust
// "Wrist is collapsing" = angle(elbow, wrist, knuckle) < threshold
// "Shoulders rising on inhale" = shoulder_y_delta > threshold over breath window
// "Bow drifting to fingerboard" = bow_angle_vs_bridge out of [80°, 100°]
```

This is deterministic, explainable, and cheap. It also means the rule set is **data**, like instrument profiles — we can ship new technique checks by editing JSON, not code.

### Async VLM coaching (the 10% case) — open multimodal LLMs

For "review this take" feedback that needs actual scene understanding — "your bow is drifting toward the fingerboard on the upstroke, and you're pulling your shoulder up with it" — we want a vision-language model. Candidates:

| Model          | Size        | Latency target                       | Notes                                                    |
|----------------|-------------|--------------------------------------|----------------------------------------------------------|
| Moondream 2    | ~1.8B       | ~1–2 s per clip on laptop            | Tiny, fits on a phone. Good fit for on-device review.    |
| Qwen2-VL       | 2B / 7B     | 2–5 s per clip                       | Stronger reasoning. 7B is desktop-only.                  |
| PaliGemma      | 3B          | 1–3 s per clip                       | Google's open VLM. Good for fine-tuning on music data.   |
| LLaVA-NeXT     | 7B+         | 3–8 s per clip                       | Desktop/server only. Reference-quality open baseline.    |

Too slow for per-frame feedback; perfect for **post-phrase or post-session review** — feed a 5s clip after the student finishes a line, get a paragraph of technique notes.

### Not in scope

- **Gemini Live / Project Astra** — streaming hosted VLM video of minors is a non-starter on privacy, COPPA, schools. Also cost-per-session.
- **OpenPose / MoveNet** — superseded by MediaPipe Pose for our uses.
- **Custom ML training from scratch** — premature. Landmark heuristics will cover the first hundred technique checks; revisit only if we hit a ceiling.

## Architecture

Add `crates/eyes` as the third sensor, mirroring the Ears → Brain → Face pattern.

```
┌─────────────────────────────────────────────────────────────────┐
│                         crates/eyes                             │
│                                                                 │
│  CameraInput ──► MediaPipe Tasks ──► LandmarkStream ──┐         │
│  (trait)        (Hand/Face/Pose)    (SPSC ring buf)   │         │
│                                                       │         │
│                                          Technique rules         │
│                                          (JSON profiles)         │
│                                                       │         │
│                                                       ▼         │
│                                              TechniqueEvent     │
└───────────────────────────────────────────────────┬─────────────┘
                                                    │
                                                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                         crates/brain                            │
│                                                                 │
│  AudioEvent ─┐                                                  │
│              ├──► Coach ──► CoachingCue ──► UI                  │
│ TechniqueEvent                                                  │
│              │                                                  │
│  Async VLM review (on SessionRecap request) ──► recap.notes    │
└─────────────────────────────────────────────────────────────────┘
```

Key properties:

- **Landmark stream is a hot path.** Same SPSC / ring-buffer discipline as audio. No allocations per frame. Consumer runs at video framerate (30 fps) or lower if the device can't keep up.
- **Technique rules are data.** Following the `profiles/` pattern, checks live in JSON: `{ "name": "shoulders_raised_on_inhale", "applies_to": ["brass", "voice", "woodwind"], "signal": { ... }, "threshold": ... }`. Adding a new technique check = adding a JSON file.
- **Coaching is unified.** Ears and Eyes both emit events into the same coach. The coach decides what's worth surfacing to the user and when — no direct path from `TechniqueEvent` to UI.
- **VLM review is a cold path.** Runs on SessionRecap generation (end of session or end of phrase), not in the live loop. Output lands in the recap blob alongside ears-derived notes.

## What this changes elsewhere

- **`architecture-v2.md` §1 three-layer diagram** gets a third sensor box (Eyes), feeding Brain alongside Ears.
- **`profiles/`** convention extends: JSON files can describe technique rules in addition to instruments.
- **`crates/brain/session.rs`** `SessionRecap` grows an optional `technique_observations: Vec<TechniqueNote>` field, populated from TechniqueEvent stream aggregates + VLM review output.
- **Permission flow** in the Tauri shell needs a camera-permission setup step, gated on parental-consent screen for under-13 accounts.
- **Mobile** changes the camera story — iPad front camera on a tripod is a *better* angle than a laptop webcam. See [mobile.md](./mobile.md).

## Sequencing

Not now. Finish the audio spine first — eyes on an unstable ears isn't useful.

### Phase 0 — prereqs (before writing any eyes code)

- DTW follower landed (story #34)
- Tracing + eprintln! retirement (story #67)
- History UI MVP shipped
- Parental-consent / FERPA product-design decisions committed to this RFC

### Phase 1 — Eyes v0, desktop, one instrument

Pick **trumpet** as the pilot. Embouchure + posture are highest-signal and land cleanly as landmark heuristics. Concrete deliverables:

1. `crates/eyes` crate with `CameraInput` trait + MediaPipe Tasks wrapper (via TFLite bindings).
2. 3–5 landmark-heuristic technique checks encoded as JSON in `profiles/techniques/`:
   - `puffed_cheeks` (face landmark: cheek point distance from jaw centerline)
   - `shoulders_rising_on_inhale` (pose landmark: shoulder_y vs breath window)
   - `slumped_posture` (pose landmark: spine line angle)
   - `head_tilt_forward` (pose landmark: ear-shoulder line angle)
   - `mouthpiece_angle_off_axis` (face landmark: mouth-center vs mouthpiece-detection — may need a lightweight mouthpiece detector)
4. Integration into the Coach: TechniqueEvents flow into the same cue queue as AudioEvents, throttled and de-duplicated.
5. One end-to-end user story: "as a trumpet student, if I puff my cheeks during a phrase, at the end of the phrase I see a coaching note about it."

### Phase 2 — Eyes v1, more instruments + async VLM

- Extend the technique-check JSON catalog to strings (bow angle, wrist), piano (hand arch), woodwinds (finger height).
- Wire Moondream for post-session review on desktop.
- Add "save this take" flow that promotes the ring-buffer frames to persistent storage with explicit user action.

### Phase 3 — Mobile eyes

See [mobile.md](./mobile.md). iPad-first for schools, then iPhone/Android. MediaPipe Tasks already runs on mobile; the work is in the Tauri mobile integration and the permission/consent flow.

## Open questions

- **Power budget on mobile.** Running camera + landmark inference + audio analysis + TFT pitch detection is non-trivial thermal load. Need to measure on actual hardware before committing to 30 fps vision in the mobile build. Likely answer: drop to 15 fps on battery, 30 fps on charge.
- **Mouthpiece / bow detection.** Body landmarks won't tell us where the mouthpiece or bow is. Options: (a) tiny YOLO model fine-tuned on instrument parts, (b) ArUco / AprilTag sticker on the mouthpiece (crude but 100% reliable), (c) hand-position inference as proxy. Revisit when Phase 1 lands; don't prematurely optimize.
- **How much technique feedback is too much.** A student hearing about embouchure, posture, breathing, and fingering every phrase will feel nagged. The coach layer needs a per-session budget (max N technique notes surfaced per minute). This is a coaching problem, not an eyes problem — but eyes expands the space of things the coach could say, so the budget needs re-examining.
- **Teacher-in-the-loop.** Long-term, real teachers should be able to annotate student sessions with their own notes keyed to timestamps. Video review is the natural UI for that. Out of scope for v1 but worth holding space for.

## Explicit non-goals

- Real-time per-note green/red/yellow feedback on any body part. See constraint #1.
- Streaming video to a cloud service in the live loop. See constraint #2.
- Identifying students by face. We use face landmarks for embouchure only; no face recognition, no cross-session identity via face.
- Detecting emotion / engagement / attention. We're a music coach, not a surveillance product. This sensor is for technique, period.
