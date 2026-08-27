# AI Music Companion — Architecture & Strategic Choice v2

**Author:** Architecture team
**Date:** April 16, 2026
**Status:** HISTORICAL (stamped 2026-08-27) — kept, like v1, for decision tracking. Not the current spec.
**Phase 0:** COMPLETE

> **⚠️ Historical document — do not build your mental model from this file.**
> This spec describes the April 2026 plan, and consecutive CTO audits
> ([#363](https://github.com/perice-pope/ai-music-companion/issues/363),
> [#494](https://github.com/perice-pope/ai-music-companion/issues/494),
> [#508](https://github.com/perice-pope/ai-music-companion/issues/508)) found it
> "describes a different app": the RV method (cells rowed through 12 keys — now the
> dominant product surface) is absent here, and several named tools were never used or
> were replaced by better in-house choices (Aubio/PESTO → pure-Rust YIN + SuperFlux,
> Matchmaker/PyO3 → a pure-Rust Online DTW follower, Audiveris → an oemer sidecar;
> yt-dlp and midir were never built).
>
> **Where current truth lives:**
> [`rv-methodology.md`](./rv-methodology.md) (the product north star) ·
> [`../../CLAUDE.md`](../../CLAUDE.md) (house rules and layout) ·
> [`offline-first-and-network-transparency.md`](./offline-first-and-network-transparency.md)
> (the network contract) ·
> [`../design/decisions-log.md`](../design/decisions-log.md) (settled calls) ·
> the latest `cto-audit`-labeled issue (what is actually built, verified against the code).

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

*Largely unchanged from v1. Phase 0 proved this layer works.*

| Job | Tool | License | Why it's safe |
|---|---|---|---|
| Mic capture | **cpal** (Rust) | Apache-2.0 | Cross-platform, actively maintained, used by Bevy. **Validated in Phase 0.** |
| MIDI capture | **midir** (Rust) | MIT | Stable API, wraps RtMidi patterns |
| Fast pitch (real-time) | **Aubio yinfft** via FFI | GPL-3 (sidecar option for permissive licensing) | 15+ years old, ~5.8 ms hop |
| Accurate pitch (optional) | **PESTO** via ONNX Runtime | MIT | 130K params, <10 ms inference |
| Onset detection | **madmom SuperFlux** (Rust reimplementation) or Aubio onset | as above | Vibrato-robust, critical for brass/voice |
| Dynamics capture | **RMS envelope** (custom Rust) | N/A | Standard DSP, feeds into phrase-level dynamics analysis |
| Attack shape | **Custom transient analyzer** (Rust) | N/A | Characterizes articulation (legato, staccato, accented) for expression feedback |
| Audio buffer | Lock-free SPSC ring buffer (`ringbuf` crate) | MIT | Standard real-time audio pattern |

**Instrument-agnostic design (unchanged):** a JSON **Instrument Profile** (`profiles/trumpet.json`, `profiles/voice.json`, `profiles/violin.json` ...) tells the Ears layer which detector settings to use — frequency range, vibrato tolerance, attack expectation, tuning corrections. Adding a new instrument = adding a JSON file. No code changes.

**New in v2:** The Ears now also capture dynamics (RMS envelope per frame) and attack shape (transient characteristics), feeding richer data into the Brain's phrase-level analysis. These are lightweight DSP operations that stay within the latency budget.

### Section 3 — The Brain (phrase analysis, LLM coaching, import pipeline) — Layer 2

This is where v2 diverges most significantly from v1. The Brain is no longer a per-note scoring engine with an "adaptive practice planner" bolted on. It is a **phrase-level musical intelligence layer** with an LLM coaching engine at its center.

#### 3a. Phrase analysis engine

| Job | Tool | Why |
|---|---|---|
| Real-time score following | Port **Matchmaker** (Online DTW) to Rust, or call Python lib via PyO3 in Phase 1 | 20 ms median alignment error, handles audio or MIDI input |
| Score parsing | **partitura** (Python) in Phase 1; **musicxml-rs** in Phase 2 | MusicXML is the universal standard |
| Phrase segmentation | Custom Rust module | Segments the score into musical phrases using breath marks, rests, slurs, and harmonic cadence points. This is the fundamental unit of analysis. |
| Phrase-level scoring | Custom Rust module | Per-phrase: intonation tendency (mean/variance of cent deviations), rhythmic stability, dynamic shape (did the phrase have direction?), articulation consistency, tone quality score (Phase 2). NOT per-note green/yellow/red. |
| Expression evaluation | Custom Rust module | Compares the dynamic arc of a played phrase against the score's markings (crescendo, diminuendo, forte, piano). Did the musician *shape* the phrase? |
| Local storage | **SQLite** via `rusqlite` | Session history, phrase-level analytics, cross-session trends |

#### 3b. LLM coaching engine (NEW)

This is the core product differentiator. The LLM takes structured analysis data from the phrase analyzer and generates feedback in a human teacher's voice.

| Job | Tool | Why |
|---|---|---|
| Coaching LLM | **Claude API** (primary) / **GPT-4 API** (fallback) | Best-in-class instruction following; can maintain pedagogical persona. Claude preferred for nuanced, non-robotic tone. |
| Whispered tips | LLM with constrained output (1 sentence, <15 words) | Between phrases during natural pauses. Must be fast (<500ms round-trip). Uses a smaller/faster model or cached common tips for latency. |
| Session recaps | LLM with structured prompt | End-of-session summary: what went well, what to focus on next, specific exercises to try. Reads like a teacher's handwritten notes. |
| Practice planning | LLM + spaced repetition data | Cross-session intelligence: "You've been working on this etude for a week — your phrasing in the middle section has improved a lot, but the interval leaps in measure 12 still need attention." |
| Prompt engineering | Custom system prompts per instrument family | A trumpet teacher talks differently than a voice teacher. System prompts encode instrument-specific pedagogy. |

**How it works:**

1. The phrase analyzer produces a structured JSON summary of each phrase: `{pitch_tendency, rhythmic_stability, dynamic_shape, articulation_score, tone_quality, comparison_to_score, comparison_to_previous_attempts}`.
2. Between phrases (detected by breath marks, rests, or silence), the coaching engine sends this data to the LLM with a system prompt encoding the instrument's teaching style.
3. The LLM returns a coaching response: either a whispered tip (real-time, 1 sentence) or a phrase assessment (stored for the session recap).
4. At session end, all phrase assessments are compiled and sent to the LLM for a session recap.

**Offline fallback:** When no internet is available, the coaching engine falls back to a rule-based tip generator (the v1 approach, essentially). The experience degrades gracefully — you still get phrase-level analysis and basic tips, just not the natural-language coaching voice.

**Cost control:** LLM calls are batched at phrase boundaries, not per-note. A typical 30-minute practice session might generate 40-80 phrase analyses and 1 session recap — roughly $0.02-0.05 in API costs at current Claude pricing. Whispered tips use cached responses for common patterns to minimize API calls.

#### 3c. Import pipeline (NEW)

Musicians get their music from everywhere. The import pipeline normalizes all sources into MusicXML, the app's internal format.

| Input source | Tool | Pipeline | Phase |
|---|---|---|---|
| **MusicXML file** (.musicxml, .mxl) | Native parser | Direct load | 1 |
| **MIDI file** (.mid) | **music21** or **partitura** (Python) | MIDI -> MusicXML conversion | 1 |
| **Photo of sheet music** | **Audiveris** (open-source OMR, Java) | Image -> OMR -> MusicXML | 2 |
| **PDF of sheet music** | **Audiveris** | PDF -> OMR -> MusicXML | 2 |
| **YouTube link** | **yt-dlp** (audio extraction) -> **basic-pitch** (Spotify, audio-to-MIDI) -> **music21** (MIDI-to-MusicXML) | URL -> audio -> MIDI -> MusicXML | 2 |
| **Audio recording** (.mp3, .wav) | **basic-pitch** (Spotify) -> **music21** | Audio -> MIDI -> MusicXML | 2 |

**Audiveris** (AGPL-3.0, Java, ~1.5K GitHub stars) is the most capable open-source OMR engine. It handles printed music well; handwritten is unreliable (acknowledged limitation — we'll state this in the UI). It runs as a sidecar process, invoked on import, not in the real-time path.

**basic-pitch** (Apache-2.0, Spotify) is a lightweight neural audio-to-MIDI converter. Monophonic input (single instrument) is its sweet spot — exactly our use case. It runs offline, no GPU required.

**yt-dlp** (Unlicense) extracts audio from YouTube (and 1000+ other sites). Legal note: we extract audio for personal practice use only. The app does not store or redistribute extracted audio.

The import pipeline runs asynchronously. The user drops in a file/link, sees a progress indicator, and the score appears in the app when ready. Errors (bad OMR quality, polyphonic YouTube audio) surface clearly with suggestions ("This scan didn't convert cleanly — try a higher-resolution photo" or "This recording has multiple instruments — results may be approximate").

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

### Section 5 — Tone quality assessment model (NEW) — Phase 2

This is our one custom ML model and our primary technical differentiator. No competitor assesses tone quality for any instrument.

#### What we're measuring

"Tone quality" for a musician means: brightness vs. warmth, resonance vs. thinness, air/noise in the sound, core clarity, vibrato quality (speed, width, consistency), and projection. These are the things a teacher listens for beyond pitch and rhythm.

#### Architecture

- **Input:** Short audio segments (phrase-length, 2-10 seconds) from the Ears layer.
- **Feature extraction:** Mel spectrogram + spectral features (centroid, rolloff, flux, MFCC). These capture timbre characteristics that distinguish "good tone" from "thin tone" or "airy tone."
- **Model:** Small CNN or Transformer encoder. We're targeting <5M parameters to keep inference fast (must run on-device, no GPU required).
- **Output:** Multi-dimensional tone descriptor: `{brightness: 0.7, warmth: 0.5, air_noise: 0.1, core_clarity: 0.8, vibrato_quality: 0.6}` — not a single "good/bad" score.
- **Training data:** Annotated recordings from music teachers rating tone quality on these dimensions. We'll need ~500-1000 annotated phrases per instrument family to start.

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

| Phase | Timeline | Status | Deliverable |
|---|---|---|---|
| **0. Spike** | 4 weeks | **COMPLETE** | Tauri 2.0 shell + cpal mic capture + YIN pitch detection + live pitch display + Zustand state + instrument profiles (JSON) + CI pipeline (fmt, clippy, test, audit, lint). Validated the platform choice, latency budget, and real-time IPC pattern. |
| **1. Practice Companion MVP** | ~4 months | Next | Phrase-level analysis engine. LLM coaching (Claude API) with whispered tips and session recaps. Score following (Matchmaker). OSMD score rendering. Free Play mode (no score) and Score Mode (MusicXML import). Brass + voice + piano profiles. SQLite session history. Cross-session trend tracking. Offline fallback (rule-based tips when no internet). |
| **2. Smart Import + Tone** | ~3 months | Planned | OMR import (Audiveris: photo/PDF of sheet music). YouTube import pipeline (yt-dlp + basic-pitch + music21). Tone quality model v1 (custom, on-device). Demucs backing track generation. Supabase cloud sync (optional). Strings + woodwinds profiles. Room calibration step for tone model. |
| **3. Teacher Platform + Mobile** | ~6 months | Planned | Teacher dashboard (Supabase + React web app). Student roster, assignment management, session replay, progress analytics. iOS and Android via Tauri 2.0 mobile. Cross-session intelligence ("you've been struggling with this passage for 3 sessions — here's a new approach"). Monetization: freemium student app + paid teacher subscription. |

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
