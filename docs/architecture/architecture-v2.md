# AI Music Companion — Architecture & Strategic Choice v2

**Author:** Architecture team
**Date:** April 16, 2026
**Status:** Proposed v2.0 (supersedes v1.0)
**Phase 0:** COMPLETE

---

## Executive summary

We are building an AI Music Companion that thinks like a good teacher, not a robotic note-checker. The app assembles best-in-class open-source audio tools (Aubio, PESTO, Matchmaker, OSMD) into a Tauri 2.0 + Rust desktop app with a React frontend, and layers LLM-powered coaching (Claude/GPT-4 API) on top to deliver phrase-level musical feedback, real-time "whispered tips" between phrases, and session recaps that read like a teacher's handwritten notes. Music enters from anywhere: MusicXML files, MIDI, photos of sheet music (Audiveris OMR), or YouTube links (yt-dlp + basic-pitch). A custom tone-quality model (Phase 2) is the technical differentiator no competitor has crossed. The core practice loop runs fully offline; cloud is optional for sync and, in Phase 3, a teacher dashboard (Supabase + React) that lets instructors monitor students remotely. Each instrument (brass, voice, strings, woodwinds, piano) is defined by a JSON profile, not custom code.

---

## Part 1 — The Strategic Choice (updated)

### The Decision: Assemble best-in-class tools, add an LLM coaching layer, train one narrow custom model.

### Layman's overview (read this first)

> Think of it like opening a restaurant. We could grow our own wheat, raise our own cows, and press our own olive oil — or we could buy world-class ingredients from trusted suppliers and focus on being the best *chef*. We're choosing to be the chef.
>
> The music-AI world already has excellent, free, battle-tested "ingredients" (Aubio for pitch, Matchmaker for score-following, Demucs for separating instruments, OSMD for drawing sheet music). What was missing in v1 was the *sous-chef who talks to the student* — an LLM that reads the raw analysis data and translates it into the kind of feedback a great private teacher would give. Not "note 47 was 12 cents flat" but "your phrasing in the second line lost direction — try breathing earlier and pushing through the apex."
>
> We also realized that musicians get their music from *everywhere*: a PDF on a stand, a YouTube video, a MIDI file from a friend. So we're adding an import pipeline that accepts all of those and converts them into something the app can follow along with.
>
> Our differentiator isn't the AI perception — it's the **coaching voice, the tone-quality assessment, and the seamless import experience**. That's where we win.

### Technical reasoning (updated from v1)

1. **Commoditized perception, uncommoditized coaching.** Pitch detection, onset detection, score alignment, and source separation are all open-source and at or near state-of-the-art. Duplicating them is pointless. But *nobody* is using LLMs to synthesize audio analysis into musical coaching. That's the gap.
2. **"Coach, don't judge" changes the product category.** Competitors (Tonestro, SmartMusic) grade notes green/yellow/red. Users describe this as "robotic" and "demoralizing." A teacher doesn't stop you every time a note is slightly flat — they listen to a phrase and say "that's better, now try connecting the D to the F more smoothly." LLM coaching lets us do this.
3. **Cost & time to market.** Assembling tools + calling an LLM API ships in months. Training a coaching model from scratch would take years and require annotated datasets of teacher feedback that don't exist.
4. **Music input is a solved problem — in pieces.** OMR (Audiveris), audio extraction (yt-dlp), audio-to-MIDI (basic-pitch), and MIDI-to-MusicXML (music21/partitura) are all mature open-source tools. We just need to wire them into a pipeline.
5. **Where we *will* train a custom model.** Phase 2 adds a narrow tone-quality assessment model — the frontier no competitor has crossed. This is one small model on top of a proven stack, focused on timbre characteristics (brightness, warmth, air, resonance) rather than pitch accuracy.

---

## Part 2 — Architecture for the AI Music Companion (v2)

### Core design principle: "Coach, don't judge"

Every architectural decision flows from this principle:

- **Phrase-level assessment, not per-note grading.** The system segments performances into musical phrases and evaluates musicality across each phrase: direction, dynamics, articulation consistency, rhythmic feel, and tonal quality. Individual note deviations are data points feeding into phrase-level analysis, never surfaced as standalone verdicts.
- **The AI considers expression.** A slightly flat note at the top of a crescendo might be musically intentional. A rushed passage might be stylistically appropriate. The LLM coaching layer has the context to make these judgments.
- **Feedback reads like a teacher's notes.** Not "Note 14: -8 cents, Note 15: +3 cents" but "Your second phrase had a nice shape, but the top of the line lost support — try more air through the A-flat."
- **Real-time whispered tips between phrases.** During natural breathing points, the app can surface a single short suggestion. Not during playing — never interrupt the music.

### Layman's overview (read this first)

> Our app is a **desktop program** (Windows, Mac, Linux — mobile coming in Phase 3) that listens to a musician practice and responds like a thoughtful teacher.
>
> It has three layers:
>
> 1. **The Ears** — listens through the mic or MIDI cable. Converts sound into notes, timing, dynamics, and tone characteristics in under 20 milliseconds.
> 2. **The Brain** — follows along with the score, analyzes musical phrases (not just individual notes), and asks an AI coach (Claude or GPT-4) to generate feedback the way a real teacher would. Also handles importing music from anywhere: sheet music photos, YouTube links, MIDI files, MusicXML.
> 3. **The Face** — two views. The **Student View** shows scrolling sheet music, a practice session with coaching tips, and end-of-session recaps. The **Teacher Dashboard** (Phase 3) lets instructors see how their students are doing across sessions.

### High-level diagram (v2)

```
┌─────────────────────────────────────────────────────────────────────┐
│                  THE FACE  (Web Frontend)                           │
│                                                                     │
│  ┌─────────────────────────┐    ┌─────────────────────────────┐    │
│  │    Student View         │    │   Teacher Dashboard (Ph 3)  │    │
│  │  React + TypeScript     │    │   React + Supabase          │    │
│  │  OSMD scrolling score   │    │   Student roster & progress │    │
│  │  Coaching tips overlay  │    │   Assignment management     │    │
│  │  Session recap panel    │    │   Session replay & notes    │    │
│  │  Import wizard          │    │                             │    │
│  └────────────▲────────────┘    └──────────────▲──────────────┘    │
│               │                                │                    │
└───────────────┼────────────────────────────────┼────────────────────┘
                │ Tauri IPC (JSON)               │ Supabase API
┌───────────────┴────────────────────────────────┼────────────────────┐
│                  THE BRAIN  (Rust Core)         │                    │
│                                                 │                    │
│  ┌──────────────────┐  ┌─────────────────┐  ┌──┴───────────────┐   │
│  │ Phrase Analyzer   │  │ LLM Coach       │  │ Cloud Sync       │   │
│  │ Score follower    │  │ Claude/GPT-4 API│  │ Supabase client  │   │
│  │ Musical segmenter │  │ Whispered tips  │  │ Teacher data feed│   │
│  │ Expression eval   │  │ Session recaps  │  │                  │   │
│  └────────▲─────────┘  └────────▲────────┘  └──────────────────┘   │
│           │                     │                                    │
│  ┌────────┴─────────┐  ┌───────┴─────────────────────────────┐     │
│  │ Scoring Engine    │  │ Import Pipeline                     │     │
│  │ Phrase-level eval │  │ MusicXML (native)                   │     │
│  │ Dynamics/artic    │  │ MIDI (midir + partitura)            │     │
│  │ Tone quality (P2) │  │ Photo → OMR (Audiveris)            │     │
│  │ SQLite history    │  │ YouTube → audio (yt-dlp)            │     │
│  └────────▲─────────┘  │ Audio → MIDI (basic-pitch)          │     │
│           │             │ MIDI → MusicXML (music21/partitura) │     │
│           │             └─────────────────────────────────────┘     │
└───────────┼─────────────────────────────────────────────────────────┘
            │ Lock-free ring buffer
┌───────────┴─────────────────────────────────────────────────────────┐
│                  THE EARS  (Rust Audio Thread)                       │
│   cpal (mic) · midir (MIDI) · Aubio yinfft · PESTO (ONNX)          │
│   madmom-lite onset detect · RMS envelope · Attack shape analysis   │
│   Instrument profile selector                                       │
└─────────────────────────────────────────────────────────────────────┘
```

### Section 1 — Platform: Tauri 2.0 + Rust backend (unchanged from v1)

**Why:** Tauri gives us one codebase that ships Windows, Mac, Linux, iOS, and Android with a web UI and a Rust core. Tauri apps are ~96% smaller than Electron and hit 2-5 ms audio round-trip using native APIs (CoreAudio/WASAPI/ALSA) — essential for real-time feedback. Rust is memory-safe, stable, and hiring-friendly.

**Phase 0 validated this choice.** The spike proved cpal mic capture, YIN pitch detection, Zustand-based state management, and Tauri IPC all work together within our latency budget.

### Section 2 — The Ears (audio + MIDI capture and analysis) — Layer 1

*Phase 1 proved this layer works. Implementation diverged from spec in Phase 2 spike; below is reality as of 2026-06-16.*

| Job | Tool | License | Status | Notes |
|---|---|---|---|---|
| Mic capture | **cpal** (Rust) | Apache-2.0 | ✅ Built Phase 0 | Cross-platform, actively maintained, used by Bevy. Validates within latency budget. |
| MIDI capture | **midir** (Rust) | MIT | ❌ Absent | Spec'd in Phase 1 plan, not yet implemented. MIDI import (score files) exists; live MIDI input does not. Consider for Phase 2 if group ensemble features are prioritized. |
| Fast pitch (real-time) | **Pure-Rust YIN** (not Aubio FFI) | MIT | ✅ Built Phase 0 | Avoids GPL-3 dependency entirely. ~6 ms hop per latency audit. No external binary required. |
| Accurate pitch (optional) | **PESTO via ONNX Runtime** | MIT | ⚠️ In transcription only | Integrated into `crates/transcribe` for audio→MIDI (Phase 2 Smart Import). NOT wired into real-time Ears layer — audio transcription is offline, not live performance path. |
| Onset detection | **Custom SuperFlux (Rust port)** | BSD | ✅ Built Phase 1 | Homegrown Rust port of madmom SuperFlux (not a madmom binary dependency). Vibrato-robust, critical for phrase segmentation. ~3 ms per audit. |
| Dynamics capture | **RMS envelope** (custom Rust) | N/A | ✅ Built Phase 1 | Standard DSP, feeds into phrase-level scoring. |
| Attack shape analysis | **Custom transient analyzer** (Rust) | N/A | ❌ Absent | Doc-claimed in v1 spec; not yet implemented. Would characterize articulation (legato, staccato, accented). Deferred pending expression-eval completion (§3a). |
| Audio buffer | Lock-free SPSC ring buffer (`ringbuf` crate) | MIT | ✅ Built Phase 0 | Standard real-time audio pattern. Zero-allocation in audio thread. |

**Instrument-agnostic design (unchanged):** a JSON **Instrument Profile** (`profiles/trumpet.json`, `profiles/voice.json`, `profiles/violin.json` ...) tells the Ears layer which detector settings to use — frequency range, vibrato tolerance, attack expectation, tuning corrections. Adding a new instrument = adding a JSON file. No code changes.

**New in v2:** The Ears now also capture dynamics (RMS envelope per frame) and attack shape (transient characteristics), feeding richer data into the Brain's phrase-level analysis. These are lightweight DSP operations that stay within the latency budget.

### Section 3 — The Brain (phrase analysis, LLM coaching, import pipeline) — Layer 2

This is where v2 diverges most significantly from v1. The Brain is no longer a per-note scoring engine with an "adaptive practice planner" bolted on. It is a **phrase-level musical intelligence layer** with an LLM coaching engine at its center.

#### 3a. Phrase analysis engine

| Job | Tool | Status | Notes |
|---|---|---|---|
| Real-time score following | **Custom Rust Online DTW** (not Matchmaker) | ✅ Built Phase 1 | Homegrown implementation of Online DTW. Matches score measure-by-measure to performance audio in real-time. ~3 ms per audit. Handles both audio and MIDI input. |
| Score parsing | **Native Rust MusicXML parser** (not partitura) | ✅ Built Phase 0-1 | `crates/score/musicxml.rs` handles `.musicxml` / `.mxl` files. MIDI parser also exists (`score/midi.rs`). No Python dependency. Dynamically parsed on import. |
| Phrase segmentation | **Silence-gap + measure-boundary** (custom Rust) | ✅ Built Phase 1 | Segments by detected silence (breath points) or measure boundaries. Does NOT yet parse breath marks, slurs, or harmonic cadences from score. Full score-aware segmentation deferred to Phase 2 musical-relevance work. |
| Phrase-level scoring | **Custom Rust scoring engine** | ⚠️ Partial | Intonation + dynamic arc implemented and real. **Rhythmic stability is hardcoded `None`** (see hotspot tracking). Articulation consistency not yet implemented. Tone quality v0 exists (heuristic DSP, not neural). |
| Expression evaluation | **Score parsing exists; comparison missing** | ❌ Partial | Score parsing extracts dynamic markings (crescendo, diminuendo, forte, piano). These are NEVER compared to played dynamics. The infrastructure exists; the comparison logic is missing. Deferred pending prioritization. |
| MusicalFingerprint unification | **Custom Rust module** (`crates/theory`, `crates/groove`, `crates/idiom`, `fingerprint.rs`) | ✅ Built Phase 4 (spiked) | Unifies intonation, groove, tone, idiom into a single `MusicalFingerprint` struct. This entire layer is UNSPEC'D in v2 but shipped in v2.0.0. See note below. |
| Local storage | **SQLite via `rusqlite`** | ✅ Built Phase 1 | Session history, phrase summaries, completed sessions. Migrations managed via `crates/store`. |

#### 3b. LLM coaching engine

The core product differentiator. The LLM takes structured analysis data from the phrase analyzer and generates feedback in a human teacher's voice.

| Job | Tool | Status | Notes |
|---|---|---|---|
| Coaching LLM | **Claude API** (primary) / **GPT-4 API** (fallback) | ✅ Built Phase 1 | Default model is `claude-opus-4-8` (was `claude-opus-4-7`, fixed in v2.0.1). Best-in-class instruction following. Maintains pedagogical persona per instrument. |
| Whispered tips | **LLM with constrained output** (1 sentence, <15 words) | ✅ Built Phase 1 | Between phrases during natural pauses. Latency goal <500ms. Real-time tips are NOT wired on the frontend (hotspot #106). Tips queued but never displayed in UI. |
| Session recaps | **LLM with structured prompt** | ✅ Built Phase 1 | End-of-session summary: strengths first, areas to improve, next steps. Reads like handwritten teacher notes. Part of `CompletedSession` recap. |
| Offline fallback | **Grounded on-device recap** (no LLM) | ✅ Built Phase 1 | When no API key: `grounded_offline_recap()` analyzes fingerprint and emits structured feedback using only local data. Never fabricates; aligns with "coaching-off-with-banner" decision. 191-line function, low-risk but review-hostile per hotspot. |
| Practice planning | **Absent** | ❌ Not built | Spec'd but not implemented. Would use cross-session trends + spaced repetition. Deferred to Phase 2-3. |
| Prompt engineering | **Custom system prompts per instrument** | ✅ Built Phase 1 | Trumpet/voice/piano/violin prompts in `coaching.rs:481-550`. Encodes instrument-specific pedagogy. |

**How it works:**

1. The phrase analyzer produces a structured JSON summary of each phrase: `{pitch_tendency, rhythmic_stability, dynamic_shape, articulation_score, tone_quality, comparison_to_score, comparison_to_previous_attempts}`.
2. Between phrases (detected by breath marks, rests, or silence), the coaching engine sends this data to the LLM with a system prompt encoding the instrument's teaching style.
3. The LLM returns a coaching response: either a whispered tip (real-time, 1 sentence) or a phrase assessment (stored for the session recap).
4. At session end, all phrase assessments are compiled and sent to the LLM for a session recap.

**Offline fallback:** When no internet is available, the coaching engine falls back to a rule-based tip generator (the v1 approach, essentially). The experience degrades gracefully — you still get phrase-level analysis and basic tips, just not the natural-language coaching voice.

**Cost control:** LLM calls are batched at phrase boundaries, not per-note. A typical 30-minute practice session might generate 40-80 phrase analyses and 1 session recap — roughly $0.02-0.05 in API costs at current Claude pricing. Whispered tips use cached responses for common patterns to minimize API calls.

#### 3c. Import pipeline

Musicians get their music from everywhere. The import pipeline normalizes all sources into MusicXML, the app's internal format.

| Input source | Tool | Status | Notes |
|---|---|---|---|
| **MusicXML file** (.musicxml, .mxl) | Native Rust parser | ✅ Built Phase 1 | `crates/score/musicxml.rs`. Fast, zero-dependency. Direct load. |
| **MIDI file** (.mid) | **Native Rust MIDI parser** (not music21) | ✅ Built Phase 1 | `crates/score/midi.rs`. Converts MIDI to internal representation. Parses on import, no Python subprocess. |
| **Audio → MIDI transcription** (.mp3, .wav) | **basic-pitch via ONNX Runtime** | ✅ Built Phase 2 (Smart Import) | `crates/transcribe` uses ONNX basic-pitch inference. Monophonic input (single instrument) is the sweet spot. On-device, no GPU. Bundled in installers via Phase 2 release. |
| **OMR (photo/PDF of sheet music)** | **oemer, not Audiveris** | ⚠️ Spiked | `crates/omr/sidecar.rs` exists with oemer (Python, AGPL-3.0). Gated behind `AMC_ENABLE_PDF_OMR` env flag. Binary not bundled yet. Audiveris was the spec choice; oemer was used for spike. Unresolved: which engine, where's the binary, what's the UI message on quality limits? |
| **YouTube import** | **yt-dlp + basic-pitch + MIDI→MusicXML** | ❌ Absent | Spec'd Phase 2; not implemented. Would require yt-dlp + orchestration. Deferred. |

**Key divergence from v2 spec:** Audio→MIDI is fully integrated (Phase 2 Smart Import shipped); OMR is spiked with oemer but not shipped; YouTube import is absent.

The import pipeline runs asynchronously. File drop/paste triggers import; score appears when ready. Errors surface with recovery suggestions.

### Section 4 — The Face (UI) — Layer 3

The Face now has two distinct views: the Student View (Phase 1) and the Teacher Dashboard (Phase 3).

#### 4a. Student View

| Job | Tool | Why |
|---|---|---|
| Framework | **React 18 + TypeScript** | Largest talent pool, most stable |
| Styling | **Tailwind CSS** | Low-maintenance, well-understood |
| Score rendering | **OpenSheetMusicDisplay (OSMD)** | BSD license, cursor API for score following, SVG phrase highlighting |
| State | **Zustand** | Simpler than Redux. **Validated in Phase 0.** |
| Coaching overlay | Custom React component | Displays whispered tips in a non-intrusive overlay at phrase boundaries |
| Session recap | Custom React component | End-of-session view with phrase-by-phrase coaching notes, trend charts, suggested next steps |
| Import wizard | Custom React component | Drag-and-drop or paste interface for all supported input types |

**Key UI changes from v1:**

- **No per-note color coding as the primary feedback.** Notes are *not* colored green/yellow/red during playback. Instead, phrase regions are softly highlighted after completion to indicate overall phrase quality. The emphasis is on the coaching text, not a traffic-light display.
- **Coaching tips panel.** A sidebar or overlay that shows the LLM's feedback in real time (between phrases) and accumulates during the session.
- **Session recap screen.** After the musician stops, a recap screen shows: (1) a phrase-by-phrase summary with the LLM's coaching notes, (2) trend data vs. previous sessions, (3) suggested exercises for next time.
- **Two modes:** "Free Play" (no score, just listens and coaches on tone/dynamics/intonation) and "Score Mode" (follows along with imported sheet music).

#### 4b. Teacher Dashboard (Phase 3)

| Job | Tool | Why |
|---|---|---|
| Backend | **Supabase** (managed Postgres + Auth + Realtime) | Off-the-shelf, generous free tier, real-time subscriptions for live session monitoring |
| Frontend | **React + TypeScript** (same stack as student view) | Code sharing, talent pool |
| Student roster | Supabase tables + Row Level Security | Teachers see only their students |
| Session feed | Supabase Realtime subscriptions | Teacher sees live session recaps as students practice |
| Assignment management | Custom module | Teacher assigns pieces, sets goals, leaves notes |
| Progress analytics | Custom charts (Recharts) | Cross-session trends per student: intonation improvement, phrase quality trajectory, practice consistency |

The teacher dashboard is a web app (not embedded in Tauri) — teachers access it from any browser. Student data syncs to Supabase when the student is online; the teacher dashboard reads from Supabase.

### Section 5 — Tone quality assessment (v0 heuristic, neural v1 planned) — Phase 2

Our tone-quality assessment exists as a heuristic DSP analysis; the planned custom ML model is deferred.

#### What we're measuring

"Tone quality" for a musician means: brightness vs. warmth, resonance vs. thinness, air/noise in the sound, core clarity, vibrato quality (speed, width, consistency). These are the things a teacher listens for beyond pitch and rhythm.

#### Current implementation (v0 heuristic)

- **Input:** Short audio segments (phrase-length) from the Ears layer. Mel spectrograms + spectral features (centroid, rolloff, flux, MFCC).
- **Feature extraction:** `crates/tone/mel.rs` and `crates/tone/spectral.rs`. Computes mel-spectrogram features + spectral descriptors.
- **Tone descriptor:** `ToneDescriptor` struct (`{brightness, warmth, air_noise, core_clarity, vibrato_quality}`) — multi-dimensional, not a single score. Computed via heuristic thresholds on spectral stats.
- **Room calibration:** `crates/tone/room.rs` captures room acoustic signature on first use; `baseline.rs` tracks relative tone quality vs. user's own baseline (not absolute).
- **Status:** ✅ Shipped Phase 2. Honest heuristic approach, not a neural model. Documented in code as "heuristic DSP, no neural model" — users know this is not trained intelligence.

#### Planned neural model (v1, deferred)

- **Model:** Small CNN or Transformer encoder, <5M parameters, on-device inference, no GPU.
- **Training data:** ~500-1000 annotated phrases per instrument family, rated by music teachers on tone dimensions.
- **Blocker:** No labeled tone-quality corpus exists. Building one requires partnership with teachers and time. Deferred pending product priorities.

#### Room acoustics: a known hard problem

Room acoustics significantly affect perceived tone quality. Reverb, room modes, mic placement, and microphone quality all color the timbre the app hears. A student playing in a tiled bathroom sounds brighter than the same student in a carpeted bedroom — but their actual tone hasn't changed.

**Mitigation strategy (three-pronged):**

1. **Room calibration step.** On first use (and periodically), the app asks the musician to play a sustained tone and a short scale. This captures the room's acoustic signature — approximate reverb time, frequency coloring, noise floor. The tone model uses this as a conditioning input to factor out room effects.
2. **Relative quality tracking, not absolute grading.** The app compares your tone *to your own baseline in the same room*, not to an abstract "perfect tone." This sidesteps the worst room-acoustics confounds. "Your tone was warmer today than Tuesday" is more useful (and more robust) than "your tone is 7/10."
3. **Training data diversity.** The training set must include recordings from diverse acoustic environments (practice rooms, bedrooms, studios, rehearsal halls) with varying mic quality. Augmentation with synthetic room impulse responses (convolution reverb) expands coverage cheaply.

This is genuinely hard, and we should be honest about the limits. Early versions will work best with consistent setups (same room, same mic, same position). Accuracy will improve over time as we collect more diverse training data.

### Section 6 — Cloud services (expanded from v1)

| Job | Tool | Phase | Why |
|---|---|---|---|
| Auth + user data + sync | **Supabase** (managed Postgres) | 2 | Off-the-shelf, generous free tier |
| Storage (recordings, MusicXML library) | Supabase Storage / S3 | 2 | Standard |
| Teacher dashboard backend | Supabase + Row Level Security | 3 | Real-time subscriptions for live monitoring |
| LLM coaching API calls | **Claude API** / **GPT-4 API** | 1 | Routed through app, keyed per user |
| Heavy offline processing (Demucs backing tracks) | Background worker on modest GPU instance | 2 | Not in the real-time path |
| OMR processing (Audiveris) | Local sidecar (preferred) or cloud worker | 2 | Runs on import, not real-time |

**Core principle unchanged: no cloud dependency for the core practice loop.** The app works fully offline after install. LLM coaching degrades to rule-based tips. Import pipeline runs locally. Sync is optional.

### Section 7 — Complete tool table (v2)

| Layer | Job | Tool | License | Phase |
|---|---|---|---|---|
| Ears | Mic capture | **cpal** (Rust) | Apache-2.0 | 0 (done) |
| Ears | MIDI capture | **midir** (Rust) | MIT | 1 |
| Ears | Fast pitch | **Aubio yinfft** via FFI | GPL-3 | 0 (done) |
| Ears | Neural pitch | **PESTO** via ONNX Runtime | MIT | 1 |
| Ears | Onset detection | **madmom SuperFlux** (Rust port) | BSD | 1 |
| Ears | Audio buffer | **ringbuf** (Rust) | MIT | 0 (done) |
| Brain | Score following | **Matchmaker** (Online DTW) | MIT | 1 |
| Brain | Score parsing | **partitura** / **musicxml-rs** | MIT | 1/2 |
| Brain | LLM coaching | **Claude API** / **GPT-4 API** | Commercial | 1 |
| Brain | OMR (sheet music photos) | **Audiveris** (Java) | AGPL-3.0 | 2 |
| Brain | YouTube audio extraction | **yt-dlp** | Unlicense | 2 |
| Brain | Audio-to-MIDI | **basic-pitch** (Spotify) | Apache-2.0 | 2 |
| Brain | MIDI-to-MusicXML | **music21** / **partitura** | BSD/MIT | 2 |
| Brain | Tone quality model | Custom CNN/Transformer | Proprietary | 2 |
| Brain | Source separation | **Demucs v4** (Meta) | MIT | 2 |
| Brain | Local storage | **SQLite** via `rusqlite` | Public domain | 1 |
| Face | UI framework | **React 18 + TypeScript** | MIT | 0 (done) |
| Face | Styling | **Tailwind CSS** | MIT | 0 (done) |
| Face | Score rendering | **OSMD** | BSD | 1 |
| Face | State management | **Zustand** | MIT | 0 (done) |
| Face | Charts | **Recharts** | MIT | 1 |
| Cloud | Auth + sync | **Supabase** | Apache-2.0 | 2 |
| Cloud | Teacher dashboard | **Supabase + React** | Apache-2.0/MIT | 3 |

### Section 8 — What we explicitly are NOT building (updated)

- **No per-note grading as the primary feedback.** We do not color individual notes green/yellow/red during performance. That's the Tonestro/SmartMusic approach and users hate it. We assess phrases, not notes.
- **No custom foundation model.** We use existing audio analysis tools + LLM APIs.
- **No real-time ensemble source separation.** Research shows it's not viable yet. We do offline separation for backing tracks only.
- **No browser-only deployment as primary surface.** Latency + Safari Web MIDI gaps make this untenable.
- **No gamification.** No points, no streaks, no Guitar Hero. Serious musicians want a practice tool that respects their musicianship. The app's tone is warm and encouraging, like a good teacher, not a video game.
- **No auto-grading for auditions or assessments.** The app is a practice companion, not an examiner. We explicitly avoid the "judge" framing.

### Section 9 — Roadmap (updated)

| Phase | Timeline | Status | Deliverable & Reality Check |
|---|---|---|---|
| **0. Spike** | 4 weeks | ✅ **COMPLETE** (Feb–Mar 2026) | Tauri 2.0 shell + cpal mic capture + YIN pitch detection + live pitch display + Zustand state + instrument profiles (JSON) + CI pipeline. Validated platform, latency, IPC. All as spec'd. |
| **1. Practice Companion MVP** | ~4 months | ✅ **COMPLETE** (v2.0.0 shipped Jun 2026) | **Built:** Phrase-level scoring, LLM coaching (Claude API), session recaps, OSMD score rendering, Free Play mode, Score Mode (MusicXML import), 9 instrument profiles, SQLite history, offline fallback. **Ahead of spec:** Custom Online DTW score follower (not Matchmaker), native Rust MusicXML/MIDI parsers, unified MusicalFingerprint (theory/groove/idiom layer UNSPEC'D). **Behind spec:** Whispered tips queued but not displayed in UI (#106), expression evaluation missing, attack-shape analysis absent. |
| **2. Smart Import + Tone** | ~3 months | ⚠️ **Partial** (v2.0.0–2.0.1) | **Built:** Audio→MIDI transcription via ONNX basic-pitch (bundled in Phase 2 release installers). Tone quality v0 (heuristic DSP, not neural). Room calibration + relative baseline. Strings + woodwinds profiles. **Missing:** OMR (spiked with oemer, not shipped). YouTube import (absent). Demucs backing tracks (absent). **Unresolved:** Which OMR engine (Audiveris spec vs. oemer spike)? Is the binary bundled? What's the UI messaging? |
| **3. Teacher Platform + Mobile** | ~6 months | ❌ **Not started** (spec only) | Teacher dashboard RLS migrations exist but UI absent. Student roster, assignment management, session replay missing. iOS/Android via Tauri 2.0 not yet attempted. Cross-session intelligence (practice planning) absent. |
| **4. Musical Relevance (spiked, UNSPEC'D in v2)** | N/A | ✅ **Shipped in Phase 1** | `crates/theory`, `crates/groove`, `crates/idiom`, `fingerprint.rs` unify analysis into a `MusicalFingerprint`. This is a full architecture layer that shipped in Phase 1 but appears nowhere in v2 spec. Reflects founder's "Phase 4 honesty" direction (grounded analysis, never judge). Does not ship a feature; enables Phase 3-4 features. |

### Section 10 — Latency budget (unchanged, validated in Phase 0)

Total mic-to-screen must be **<25 ms**.

| Stage | Budget | Notes |
|---|---|---|
| Audio capture (cpal buffer) | ~5 ms | Validated in Phase 0 |
| Pitch detection (Aubio yinfft hop) | ~6 ms | Validated in Phase 0 |
| Score alignment | ~3 ms | Phase 1 |
| IPC + render | ~5 ms | Validated in Phase 0 |
| Headroom | ~6 ms | |

**LLM coaching is NOT in the latency-critical path.** Coaching calls happen at phrase boundaries (during rests/breaths) and are allowed up to 500ms for whispered tips, 2-3 seconds for phrase assessments. Session recaps have no latency constraint.

### Section 11 — Risk table (updated)

| Risk | Severity | Mitigation |
|---|---|---|
| **Aubio is GPL-3** | Medium | Option A: ship as separately-licensed sidecar process. Option B: swap to PESTO + ONNX (MIT) once accuracy is validated. Phase 0 used YIN (no GPL dependency) for the spike. |
| **Matchmaker is Python-only** | Medium | Phase 1 embeds via PyO3; Phase 2 ports the Online DTW loop to Rust (~2 weeks work). |
| **LLM API latency for whispered tips** | Medium | Cache common tip patterns locally. Use smaller/faster model for real-time tips. Degrade to rule-based tips if API is slow (>500ms) or offline. |
| **LLM API cost at scale** | Low | ~$0.02-0.05/session at current pricing. Can reduce with caching, prompt optimization, and batching. If costs spike, switch to smaller/cheaper model for tips, keep full model for recaps. |
| **LLM hallucination / bad coaching advice** | Medium | Constrain LLM output with structured prompts and instrument-specific system prompts. Validate musical claims against analysis data. Log all coaching output for review. Include "flag this tip" button for user feedback. |
| **Tone quality model accuracy** | High | Start with relative tracking (compare to your own baseline) before absolute assessment. Be transparent about confidence levels. Room calibration mitigates environment variance. Launch as "beta" feature with explicit accuracy disclaimers. |
| **Room acoustics confound tone assessment** | High | Three-pronged mitigation: room calibration step, relative (not absolute) quality tracking, and diverse training data with synthetic augmentation. See Section 5 for details. |
| **Audiveris OMR quality on poor scans** | Medium | Clear UI messaging about scan quality requirements. Preview step where user can correct obvious errors before committing. Fallback: manual MusicXML entry or MIDI import. |
| **yt-dlp legal/ToS concerns** | Medium | Personal practice use only. Do not store/redistribute extracted audio. Document this clearly in terms of service. If YouTube cracks down on yt-dlp, this feature degrades gracefully — other import paths remain. |
| **Latency regressions** | Medium | Continuous bench suite: measures mic-to-screen round-trip on every PR; fails build if >25 ms. Validated in Phase 0. |
| **Instrument-specific edge cases** | Low | Profiles are data, not code. Music teachers can tune them without engineering changes. |
| **Teacher dashboard data privacy (FERPA/COPPA)** | High (Phase 3) | Supabase Row Level Security. End-to-end encryption for session data. Age verification. Consult education privacy counsel before Phase 3 launch. |

---

## Appendix A — Phase 0 retrospective (what we built and learned)

Phase 0 (the spike) is complete. Here's what was built and what it proved:

**Built:**
- Tauri 2.0 desktop shell with React + TypeScript frontend
- cpal-based microphone capture with real-time audio streaming
- YIN pitch detection algorithm running in the Rust audio thread
- Live pitch display with Zustand store and Tauri IPC events
- JSON instrument profiles (trumpet, voice, violin, etc.)
- Full CI pipeline: `cargo fmt`, `cargo clippy --deny warnings`, `cargo test`, `cargo audit`, `pnpm lint`, `pnpm test`, `pnpm build`
- Conventional commits, atomic changes, no `unsafe` without `// SAFETY:` comments

**Proved:**
- Tauri 2.0 + cpal delivers audio capture well within the 5ms budget
- Rust-to-frontend IPC via Tauri events is fast enough (~2ms) for real-time pitch display
- Zustand handles real-time state updates without frame drops
- The instrument profile pattern (JSON, no code changes) works cleanly
- The CI pipeline catches regressions before merge

**What we didn't prove yet (deferred to Phase 1):**
- Score following latency (Matchmaker integration)
- LLM coaching round-trip time
- OSMD rendering performance with real-time cursor updates

---

## Appendix B — "Coach, don't judge" in practice: example feedback comparison

To make the design principle concrete, here's how v1 (note-checking) and v2 (coaching) would handle the same performance:

**Scenario:** Student plays a 4-bar phrase from Haydn Trumpet Concerto. Notes 3 and 7 are slightly flat (8 and 12 cents). The dynamic shape is mostly flat (no crescendo where marked). The tone is good.

**v1 approach (note-checking):**
```
Note 3 (E5): -8 cents [YELLOW]
Note 7 (G5): -12 cents [YELLOW]
Notes 1,2,4,5,6,8: [GREEN]
Score: 75% (6/8 notes in tune)
```

**v2 approach (coaching):**
```
Whispered tip (between phrases): "Nice tone! Try pushing
more air through the crescendo."

Session recap: "Your sound in the Haydn was warm and centered
today. The phrase starting at measure 5 could use more
direction — there's a crescendo marked that wants to bloom
toward the G. Your intonation is solid; just watch the tendency
to sag on sustained notes in the upper register. Try playing
that phrase on a single breath with a clear dynamic arc."
```

The v2 feedback mentions the same pitch issues but in context, focuses on musicality, acknowledges what went well, and gives an actionable next step. This is what a good teacher does.

---

## Appendix C — Phase 4 "Musical Honesty" Architecture Layer (Shipped Phase 1, Unspec'd until now)

During Phase 1 development, the team spiked a **unified analysis layer** that synthesizes intonation, groove, tone, and idiom into a single `MusicalFingerprint` struct. This layer ships in v2.0.0 but is not mentioned in the v2 spec (which focuses on Phase 1-2 scope). It is the foundation for Phase 3-4 "musical relevance" features and deserves documentation.

### Purpose

Shift from "robotic per-note feedback" to **grounded, contextual coaching**. Instead of asking "how many cents flat was this note?", the fingerprint asks "what does this performance tell us about the musician's current understanding of phrasing, groove, timbre, and idiom?" This is the "coach, don't judge" principle made operational.

### Components

| Module | Location | What it does | Phase |
|---|---|---|---|
| **Intonation Analyzer** | `crates/theory/src/intonation.rs` | Computes mean/variance of cents deviations per phrase, context (high notes often flat), tendency (sharp or flat overall). Raw cents feedback is never surfaced; only tendencies and contextual observations. | 1 |
| **Groove Analyzer** | `crates/groove/src/lib.rs` | Detects swing, triplet feel, rubato, or strict metronomic timing in a phrase. Answers: "Is the rhythm stylistically appropriate to the tune?" | 1 |
| **Tone Descriptor** | `crates/tone/descriptor.rs` | Heuristic DSP assessment of brightness, warmth, air, clarity, vibrato quality. Room-calibrated (relative to player's own baseline). Never absolute judgment. | 1 |
| **Idiom Recognizer** | `crates/idiom/src/lib.rs` | Identifies musical idiom (jazz swing, classical legit, contemporary gospel, folk, etc.) from performance fingerprint. Enables instrument-appropriate coaching and genre-specific suggestions. | 1 (spiked, not yet wired to coaching) |
| **Fingerprint Unifier** | `crates/brain/src/fingerprint.rs` | Combines all four dimensions into a single `MusicalFingerprint` struct. This is what the LLM coaching engine receives; phrase-level feedback is grounded in all four dimensions, not isolated measures. | 1 |

### Why this matters for "coach, don't judge"

1. **Intonation in context.** "Your high F-sharps tend sharp" (contextual) vs. "Note 7: +15 cents" (robotic).
2. **Groove as a feature, not a fault.** A jazz swing feel is not a "timing error"; it's an idiom choice. The analyzer recognizes this.
3. **Tone relative to musician's baseline.** "Your tone was warmer today than Tuesday" is honest and actionable. "Your tone is 6/10" is meaningless without context.
4. **Idiom-aware coaching.** A classical trumpet teacher gives different feedback than a jazz coach. The idiom recognizer enables this.

### Data flow (Phase 1+)

```
Raw audio performance
       ↓
   Ears layer (pitch, onset, dynamics, tone spectrum)
       ↓
   Brain: phrase segmentation & analysis
       ├── intonation.rs → {mean_cents, variance, tendency, context}
       ├── groove.rs → {feel: swing|triplet|rubato|metronomic}
       ├── tone/descriptor.rs → {brightness, warmth, air, clarity, vibrato}
       ├── idiom.rs → {idiom_class: jazz|classical|contemporary|folk|...}
       └── fingerprint.rs → unified MusicalFingerprint
            ↓
       LLM Coaching Engine (reads MusicalFingerprint, generates feedback)
            ↓
       Session recap (grounded in fingerprint, never fabricated)
```

### Status as of v2.0.1

- ✅ All modules built and wired (crates/theory, groove, tone, idiom, fingerprint).
- ✅ Fingerprint persisted in `SessionRecap.fingerprint` (forward-compatible JSONB).
- ⚠️ Idiom recognizer exists but is not yet wired to the coaching LLM. Idiom classification is computed but doesn't influence teacher prompts yet.
- ✅ Grounded offline recap falls back to this layer when no API key is set — coaching is grounded in the fingerprint, never hallucinates.

### Next steps (Phase 3-4)

1. **Idiom → prompt personalization:** Wire idiom classification into the LLM system prompt. "I'm coaching a classical/jazz/folk musician" changes the tone and suggestions.
2. **Cross-session trends:** Compare fingerprints across sessions to spot improvement (intonation tightening, groove settling, tone warming) and plateaus (where the musician is stuck).
3. **Listening Coach (Phase 4 "Listening" module):** Use the idiom recognizer + fingerprint to suggest reference recordings. "Your swing feel is close, but a bit straighter. Try listening to this recording of Miles Davis at this timestamp to hear the pocket."
4. **Taste Profile integration (Phase 4):** Combine musician's taste preferences (genres, artists) with fingerprint to tailor coaching voice and suggestions.
