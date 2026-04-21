# AI Music Companion — Architecture & Strategic Choice

**Author:** Architecture team
**Date:** April 15, 2026
**Status:** Proposed v1.0

---

## Part 1 — The Strategic Choice

### The Decision: Assemble best-in-class tools. Do **not** train a new AI model from scratch.

### Layman's overview (read this first)

> Think of it like opening a restaurant. We could grow our own wheat, raise our own cows, and press our own olive oil — or we could buy world-class ingredients from trusted suppliers and focus on being the best *chef*. We're choosing to be the chef.
>
> The music-AI world already has excellent, free, battle-tested "ingredients" (Aubio for pitch, Matchmaker for score-following, Demucs for separating instruments, OSMD for drawing sheet music). Training our own models would cost millions of dollars, take 18–24 months, and likely end up *worse* than what's already public. Instead, we combine these tools into something nobody else has built: a single, polished practice companion that works for **every** instrument — brass, voice, strings, woodwinds, anything.
>
> Our differentiator isn't the AI — it's the **experience, the tuning for each instrument family, and the feedback loop**. That's where we win.

### Technical reasoning

1. **Commoditized perception, uncommoditized product.** Pitch detection (Aubio/PESTO), onset detection (madmom), score alignment (Matchmaker), and source separation (Demucs/BS-RoFormer) are all open-source and at or near state-of-the-art. Duplicating any of them is an 18-month research project for no moat.
2. **Cost & time to market.** Training a CREPE-equivalent model requires tens of thousands of GPU-hours plus annotated datasets we don't own. Assembly ships in months, not years.
3. **The research confirms the market gap is UX + instrument coverage**, not model accuracy. No competitor (Tonestro, SmartMusic, Yousician, Modacity) is held back by bad ML — they're held back by product focus, latency, and lack of expressive feedback.
4. **Where we *will* train small custom models — later.** Phase 2 will add a **narrow** fine-tune for tone-quality assessment (the frontier no competitor has crossed). That's one small model on top of a proven stack, not a foundation model.

---

## Part 2 — Architecture for the AI Music Companion

### Layman's overview (read this first)

> Our app is a **desktop program** (Windows, Mac, Linux) with a matching **mobile version**, built so a musician can plug in a microphone or MIDI keyboard and get instant, accurate feedback while they practice.
>
> It has three layers:
>
> 1. **The Ears** — listens to the musician through their mic or MIDI cable. Converts sound into notes and timing in under 20 milliseconds (faster than a human can notice).
> 2. **The Brain** — compares what the musician played against the sheet music, scores them, and decides what they should work on next.
> 3. **The Face** — shows sheet music that scrolls along, colors notes green/yellow/red, and gives human-readable tips ("your high F was 12 cents flat").
>
> Each "section" below has a 🎯 pointer telling you *which of the three layers it belongs to* so you can skip around.

### High-level diagram

```
┌─────────────────────────────────────────────────────────────┐
│                  THE FACE  (Web Frontend)                   │  🎯 Layer 3
│   React + TypeScript · OSMD sheet music · Tailwind UI       │
│   Scrolling score · Pitch meter · Coach feedback · History  │
└───────────────────────────▲─────────────────────────────────┘
                            │ Tauri IPC (JSON)
┌───────────────────────────┴─────────────────────────────────┐
│                  THE BRAIN  (Rust Core)                     │  🎯 Layer 2
│   Score follower (Matchmaker-rs port) · Feedback engine     │
│   Adaptive practice planner · SQLite history · Sync client  │
└───────────────────────────▲─────────────────────────────────┘
                            │ Lock-free ring buffer
┌───────────────────────────┴─────────────────────────────────┐
│                  THE EARS  (Rust Audio Thread)              │  🎯 Layer 1
│   cpal (mic) · midir (MIDI) · Aubio yinfft · PESTO (ONNX)   │
│   madmom-lite onset detect · Instrument profile selector    │
└─────────────────────────────────────────────────────────────┘
                            │
          ┌─────────────────┴─────────────────┐
          │   Cloud (optional, Phase 2)       │
          │   Supabase (auth + sync + storage)│
          │   Demucs worker (backing tracks)  │
          └───────────────────────────────────┘
```

### Section 1 — Platform: Tauri 2.0 + Rust backend 🎯 spans all three layers

**Why:** Tauri gives us one codebase that ships Windows, Mac, Linux, iOS, and Android with a web UI and a Rust core. The research shows Tauri apps are ~96% smaller than Electron and hit 2–5 ms audio round-trip using native APIs (CoreAudio/WASAPI/ALSA) — essential for real-time feedback. Rust is memory-safe, stable, and hiring-friendly.

**Stability pick justification:** Tauri 2.0 GA (late 2024), Rust stable toolchain, no bleeding-edge dependencies. Every library below is 1.0+ and widely deployed.

### Section 2 — The Ears (audio + MIDI capture and analysis) 🎯 Layer 1

| Job | Tool | License | Why it's safe |
|---|---|---|---|
| Mic capture | **cpal** (Rust) | Apache-2.0 | Cross-platform, actively maintained, used by Bevy |
| MIDI capture | **midir** (Rust) | MIT | Stable API, wraps RtMidi patterns |
| Fast pitch (real-time) | **Aubio yinfft** via FFI | GPL-3 (we ship as separate process if we need permissive licensing, else accept GPL) | 15+ years old, ~5.8 ms hop |
| Accurate pitch (optional) | **PESTO** via ONNX Runtime | MIT | 130K params, <10 ms inference |
| Onset detection | **madmom SuperFlux** (re-implemented in Rust) or Aubio onset | as above | Vibrato-robust, critical for brass/voice |
| Audio buffer | Lock-free SPSC ring buffer (`ringbuf` crate) | MIT | Standard real-time audio pattern |

**Instrument-agnostic design:** a JSON **Instrument Profile** (`profiles/trumpet.json`, `profiles/voice.json`, `profiles/violin.json` …) tells the Ears layer which detector settings to use — frequency range, vibrato tolerance, attack expectation, tuning corrections (e.g., trumpet 1+3 valve combo runs 10–40 cents sharp; the profile knows this). Adding a new instrument = adding a JSON file. No code changes.

### Section 3 — The Brain (score following, scoring, coaching) 🎯 Layer 2

| Job | Tool | Why |
|---|---|---|
| Real-time score following | Port **Matchmaker** (Online DTW) to Rust, or call the Python lib via embedded interpreter in Phase 1 | 20 ms median alignment error, published 2025, handles audio or MIDI input |
| Score parsing | **partitura** (Python) in Phase 1; **musicxml-rs** in Phase 2 | MusicXML is the universal standard |
| Scoring engine | Custom Rust module | Per-note: pitch error (cents), onset error (ms), duration error, dynamics (RMS envelope), articulation (attack shape). Color-codes green/yellow/red |
| Adaptive practice planner | Custom — simple spaced-repetition (SM-2) + rule-based weakness detection in Phase 1; LLM-powered coach in Phase 2 | Keep it boring & debuggable early |
| Local storage | **SQLite** via `rusqlite` | Bulletproof, embedded, no server needed |

### Section 4 — The Face (UI) 🎯 Layer 3

| Job | Tool | Why |
|---|---|---|
| Framework | **React 18 + TypeScript** | Largest talent pool, most stable |
| Styling | **Tailwind CSS** | Low-maintenance, well-understood |
| Score rendering | **OpenSheetMusicDisplay (OSMD)** | BSD license, cursor API for live score following, SVG note coloring for instant feedback |
| Charts / pitch meter | **Recharts** + custom canvas for real-time pitch trace | Stable, minimal deps |
| State | **Zustand** | Simpler than Redux, fewer footguns |

The UI receives JSON events from the Rust core (`{note_index, cents_deviation, timing_ms, verdict}`) over Tauri's IPC bridge at ~30 Hz — plenty fast for visual feedback, low enough to stay out of the audio hot path.

### Section 5 — Cloud services (kept minimal) 🎯 optional, Phase 2

| Job | Tool | Why |
|---|---|---|
| Auth + user data + sync | **Supabase** (managed Postgres) | Off-the-shelf, generous free tier, one less thing to operate |
| Storage (audio recordings, MusicXML library) | Supabase Storage / S3 | Standard |
| Heavy offline processing (backing-track generation with Demucs v4 / BS-RoFormer) | Background worker on modest GPU instance | Not in the real-time path, so cost-controlled |

**No cloud dependency for the core practice loop** — the app works fully offline after install. This is a deliberate stability choice.

### Section 6 — What we explicitly are NOT building 🎯 scope guard

- No custom foundation model.
- No real-time ensemble source separation (research shows it's not viable yet; we do offline separation for backing tracks only).
- No browser-only deployment as primary surface (latency + Safari Web MIDI gaps).
- No social / gamification features in v1 — serious musicians want accuracy, not Guitar Hero.

### Section 7 — Roadmap (boring on purpose) 🎯 delivery plan

| Phase | Timeline | Deliverable |
|---|---|---|
| 0. Spike | 4 weeks | Tauri shell + cpal mic capture + Aubio pitch displayed on screen |
| 1. MVP | 4 months | Brass + voice profiles, OSMD score follow, per-note scoring, SQLite history |
| 2. Expand | +3 months | Strings, woodwinds, piano (MIDI) profiles; Supabase sync; backing-track generation |
| 3. Differentiate | +6 months | Tone-quality fine-tune (our one custom model), adaptive LLM coach, iOS/Android release |

### Section 8 — Risk table 🎯 for the PM

| Risk | Mitigation |
|---|---|
| Aubio is GPL-3 | Option A: ship Aubio as a separately-licensed sidecar process. Option B: swap to PESTO + ONNX (MIT) once accuracy is validated. |
| Matchmaker is Python-only today | Phase 1 embeds PyO3; Phase 2 ports the Online DTW loop to Rust (≈2 weeks work). |
| Latency regressions | Continuous bench suite: measures mic→screen round-trip on every PR; fails build if >25 ms. |
| Instrument-specific edge cases | Profiles are data, not code. Music teachers can tune them without engineering changes. |

---

## One-paragraph summary for the exec deck

We are building the AI Music Companion by **assembling the best existing open-source music-AI tools** (Aubio, PESTO, Matchmaker, OSMD) into a **Tauri 2.0 + Rust desktop app** with a **React frontend**, where each supported instrument (brass, voice, strings, woodwinds, piano) is defined by a **JSON profile** rather than custom code. This lets us ship a cross-instrument, sub-20 ms-latency practice companion in roughly four months, using only stable, widely-adopted libraries, and keeps our one opportunity for proprietary ML — tone-quality assessment — as a clean Phase-3 addition on top of a proven stack.
