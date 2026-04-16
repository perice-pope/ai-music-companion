# Real-time music input processing for an AI practice companion

**Building an AI-powered music practice companion is technically feasible today, but the optimal approach depends critically on choosing the right pitch detection library, platform architecture, and score alignment strategy.** For brass instruments specifically, the technology landscape offers a compelling set of open-source tools — Aubio for sub-20ms pitch detection, Matchmaker for real-time score following, and OSMD for interactive notation rendering — that can be assembled into a cohesive practice system. The competitive landscape reveals a significant gap: no comprehensive AI-powered brass practice app exists, and the nearest competitors rely on basic DSP rather than modern ML. A Tauri 2.0 application with a Rust audio backend and web-based UI emerges as the strongest platform choice for 2025–2026, delivering near-native audio latency with cross-platform reach.

---

## Pitch detection: Aubio wins for real-time brass, PESTO for neural accuracy

The choice of pitch detection engine determines whether a practice app can deliver sub-20ms feedback — the threshold where musicians perceive latency as "instantaneous." Brass instruments produce strong harmonic signals with a clear fundamental, making them relatively tractable for pitch trackers, but challenges include embouchure-based pitch bending, wide vibrato (±50 cents), lip slurs, and characteristic attack transients.

**Aubio** (C library, ~3K GitHub stars, version 0.4.9) stands out as the best option for real-time brass feedback. Its `yinfft` algorithm uses spectral-domain autocorrelation with a default hop size of **256 samples at 44.1 kHz — just 5.8ms per frame**. The library is designed for causal/real-time use with no memory allocation during processing, supports JACK audio natively, and compiles to WebAssembly via Emscripten. Its `aubionotes` command can emit MIDI-like output (onset, pitch, duration) directly from live audio, making it an immediate audio-to-MIDI converter for monophonic instruments.

**PESTO** (Sony CSL Paris, 2023) represents the most promising neural approach for real-time use. With only **~130K parameters** and a self-supervised training objective (no annotated data required), it processes audio **55× faster than CREPE** while achieving competitive accuracy (~94.8% raw pitch accuracy on MIR-1K). It now supports streaming audio processing. Model inference takes under 10ms, though the Variable-Q Transform input computation adds some latency. For a brass practice app, PESTO offers the best balance of neural accuracy and speed.

**CREPE** (ICASSP 2018, ~1.5K GitHub stars) remains the accuracy benchmark — a 6-layer CNN operating on 1024 samples at 16 kHz — but its inherent **64ms analysis window** plus CNN inference time makes it unsuitable for real-time feedback. The TorchCREPE reimplementation (~481 stars) and ONNX export paths exist for integration, but CREPE is best reserved for post-performance analysis. **PENN** (2023, Northwestern) optimizes CREPE's architecture to run **11.2× faster than real-time on CPU** with 5-cent resolution, making it a viable near-real-time neural option.

For intonation analysis, the math is straightforward: `cents = 1200 × log₂(f_detected / f_target)`. Best pitch detectors achieve **±1–2 cents accuracy** on clean sustained tones. Brass-specific considerations matter significantly: trumpet valve combinations introduce inherent intonation errors of **10–40 cents** (1+3 valve combination tends sharp, requiring third-valve slide adjustment), the 5th partial is ~14 cents flat, and professional brass players routinely adjust major thirds narrow by 14 cents for just intonation. A practice app needs to account for these instrument-specific tendencies rather than treating equal temperament as the sole reference.

| Library | Sub-20ms? | Approach | Best use case |
|---------|-----------|----------|---------------|
| Aubio (yinfft) | **Yes** (5.8ms hop) | DSP | Real-time feedback |
| PESTO | **Yes** (<10ms inference) | Neural (130K params) | Streaming with accuracy |
| pYIN (librosa) | Borderline (~23ms) | DSP+HMM | Near-real-time analysis |
| CREPE | No (64ms+) | Neural (CNN) | Offline gold standard |
| PENN | Possible on GPU | Optimized neural | Near-real-time with GPU |

---

## Rhythm analysis and score following have mature open-source solutions

Onset detection and score alignment are essential for rhythm feedback. **madmom** (CPJKU, ~1.3K GitHub stars) provides state-of-the-art onset detection using bidirectional LSTMs at **100 fps (10ms resolution)**. Its SuperFlux algorithm is particularly relevant for brass — it applies maximum-filter vibrato suppression, preventing the wide vibrato common in trumpet playing from triggering false onsets. madmom consistently ranks at or near the top of MIREX onset detection evaluations.

For real-time score following — aligning a live performance to a reference score as it unfolds — **Matchmaker** (ISMIR 2025, Python 3.12) is the most current open-source solution. It implements both Online DTW (Dixon and Arzt variants) and HMM-based algorithms, accepting audio (microphone) or MIDI (keyboard) input and supporting MusicXML and MIDI score formats via the `partitura` library. Published accuracy for Online DTW score following shows **median alignment error of 20ms** on piano recordings, with over 97% of onsets detected within 100ms tolerance.

**Antescofo** (IRCAM Paris, since 2008) represents the research gold standard — a real-time listening machine used by the Berlin Philharmonic and NY Philharmonic — but it's tied to the Max/MSP ecosystem rather than being a standalone library. For a practice app, Matchmaker provides the most practical starting point with its modular Python architecture and both audio and MIDI input modes.

---

## MIDI input is well-supported everywhere except Safari

The **Web MIDI API** covers ~78% of global browser usage: full support in Chrome (v43+), Edge (v79+), Firefox (v108+), and Opera, but **zero support in Safari on any platform**. Since all iOS browsers use WebKit, this means Web MIDI is completely unavailable on iPhones and iPads — a critical limitation for any browser-only approach. The API itself adds minimal latency (USB MIDI round-trip is typically **1–5ms**), but MIDI messages arrive on the main thread via event callbacks rather than in the audio thread, introducing potential jitter under heavy UI load. **WEBMIDI.js** (djipco/webmidi) is the standard high-level wrapper, providing `playNote()`, event listeners for `noteon`/`pitchbend`/`controlchange`, and Node.js compatibility.

For desktop MIDI, the ecosystem is mature across all languages. **RtMidi** (C++) is the foundational cross-platform library that most others wrap: **python-rtmidi** for Python, **node-midi** for Node.js, and **midir** for Rust (inspired by RtMidi, supporting ALSA, JACK, CoreMIDI, WinMM, WinRT, and even Web MIDI). For real-time MIDI analysis against a reference score, the core algorithm pattern is: determine score position via score following, match incoming notes against expected notes at that position, compute timing deviation from expected onset, and classify as correct/wrong pitch/wrong timing/missed/extra.

Several open-source projects demonstrate this pattern in practice. **PianoBooster** (~1.5K GitHub stars, C++/Qt, GPL) is the most mature — a MIDI-based piano teaching tool where accompaniment follows the student's tempo, with color-coded right/wrong notes. **piano-trainer** (Rust/Tauri) offers interactive scale and chord practice with MIDI keyboard support. The **sightreading** project (TypeScript) provides sight-reading practice with MIDI input using Verovio for rendering.

---

## Tauri 2.0 with Rust audio backend is the optimal platform for 2025–2026

The platform decision hinges on one question: can browser-based audio processing achieve low enough latency for real-time music practice feedback? The answer is **yes, but with caveats that make a native audio backend strongly preferable**.

Measured browser latency (Jeff Kaufman's benchmarks, MacBook Pro) shows optimized round-trip audio at **14–19ms in Firefox, 19–67ms in Chrome** depending on configuration. With `latencyHint: 0` and disabled echo cancellation/noise suppression, a browser can achieve ~20ms input-to-processing latency. This is usable for visual pitch/timing feedback (where the critical path is mic→pitch detection→screen update, not audio monitoring). Soundtrap (Spotify's browser DAW) reports **~30ms best-case round-trip**, calling it "passable but not great." By contrast, native apps with ASIO/CoreAudio achieve **2–5ms round-trip**.

**Tauri 2.0** (released late 2024, **70K+ GitHub stars**) offers the strongest hybrid architecture: a Rust backend handling audio capture and ML inference with near-native performance, paired with a web-based frontend for UI rendering. Key advantages over Electron include **96% smaller binary size** (2–10MB vs 60–150MB), **~50% less RAM** (30–40MB vs 100–300MB idle), and **40% faster startup**. Tauri 2.0 adds iOS and Android support alongside desktop platforms.

The recommended architecture uses `cpal` (Rust, Apache 2.0) for cross-platform audio I/O with direct access to CoreAudio/WASAPI/ALSA, `midir` for MIDI I/O, and ONNX Runtime (via the `ort` or `tract` crate) for ML model inference — running pitch detection models **2–5ms per frame** versus 10–20ms for TensorFlow.js in a browser. The web frontend handles score rendering (OSMD or VexFlow), UI feedback, and visualization. A Tauri app using `cpal` for native mic capture has demonstrated **300–400ms lower latency** than browser `getUserMedia`.

```
┌────────────────────────────────────────────┐
│         Web Frontend (Svelte/React)        │
│  OSMD (score) + Canvas (pitch feedback)    │
├────────────────────────────────────────────┤
│            Tauri IPC Bridge                │
├────────────────────────────────────────────┤
│           Rust Backend                     │
│  cpal → FFT/YIN → ONNX pitch model        │
│  midir → MIDI input                        │
│  Onset detection → Score alignment (DTW)   │
└────────────────────────────────────────────┘
```

If a web-only deployment is required (avoiding native distribution), AudioWorklet + WebAssembly (Rust compiled to WASM) + TensorFlow.js CREPE provides a viable path for Chrome/Firefox desktop users, accepting **20–40ms input-to-visual latency**. Progressive Web Apps can access microphones and Web MIDI (where supported) with offline caching via Service Workers, but iOS limitations (no Web MIDI, restricted background audio) make PWAs unsuitable as the sole deployment target.

Every major commercial music learning app — Yousician (which uses **JUCE** and Unity), SmartMusic, Tonestro, Simply Piano — uses **native platforms** for core audio processing. This is not a coincidence.

---

## The competitive landscape has a clear gap for AI-powered brass practice

**No comprehensive AI-powered brass instrument practice app exists today.** This represents the single most important market finding from this research.

**Tonestro** (Austrian, iOS/Android) is the closest competitor, supporting trumpet, trombone, French horn, tuba, and euphonium with real-time pitch/rhythm/intonation feedback via microphone. However, it uses basic DSP rather than ML, suffers from frequent bugs (freezing during playback, audio interference without headphones), and user reviews describe its feedback as "robotic" — stopping music when notes aren't played precisely enough. Its content skews beginner, with limited material for advanced players.

**SmartMusic/MakeMusic Cloud** is the gold standard for institutional music education, supporting all orchestral and band instruments including brass. Its assessment engine color-codes pitch accuracy (green/red/yellow) and reports cents deviation per note, but it only assesses pitch, rhythm, and duration — **no dynamics, articulation, tone quality, or musical expression**. It's web-based (Chrome only), designed for K-12 classrooms rather than self-directed adult learners, and sits behind school/district subscription pricing.

**Yousician** (25M+ users, Helsinki) dominates the self-directed learning market for guitar, piano, ukulele, bass, and voice, but **does not support brass or woodwind instruments at all**. Its gamified Guitar Hero-style interface works for beginners but is "fundamentally incompatible with advanced musical concepts."

**Modacity** occupies a unique niche beloved by professional brass players (endorsed by the World Trumpet Society president). But it's purely a practice organization tool — timer, recorder, playlist, MetroDrone — with **no real-time pitch or performance assessment**.

The broader AI music education space is growing rapidly (the online music education market reached **$3.9B in 2025**, growing at 15.23% CAGR). AI tutoring startups like Studio Music School (GPT-4 powered curriculum) and Wondera (conversational AI for music creation) are emerging, but none focus on real-time performance assessment for acoustic instruments.

---

## Score rendering and digital format choices are well-defined

**MusicXML** is the clear primary format for a practice companion. It's the de facto interchange standard supported by virtually all notation software (Finale, Sibelius, MuseScore, Dorico), carries rich notation information (note names, dynamics, articulations, lyrics) essential for practice feedback, and renders directly in both major web score libraries. Compressed `.mxl` files (ZIP archive, ~1/20th uncompressed size) are web-friendly. **MIDI serves as a secondary format** — ideal for playback, score following alignment, and real-time input handling, but it loses enharmonic spelling, beaming, dynamics markings, and other visual notation data.

**OpenSheetMusicDisplay (OSMD)** is the recommended score renderer for a web-based practice app. Built on VexFlow (TypeScript, MIT license), OSMD natively renders MusicXML with responsive line breaks, a built-in **cursor API for score following** (tracking the current note position), SVG note coloring for real-time correct/wrong feedback, and part selection for multi-instrument scores. Its BSD license permits commercial use. For highest engraving quality or MEI format support, **Verovio** (C++20 with JS/Python/Swift bindings, LGPL) produces publication-quality SVG using SMuFL fonts and generates MIDI output and time maps from scores.

---

## Source separation works offline but real-time isolation remains impractical for ensemble use

Source separation technology has advanced dramatically, with **BS-RoFormer reaching 9.80 dB SDR** on MUSDB18-HQ — a quality level where separated stems sound clean enough for practice backing tracks. **Demucs v4** (HTDemucs, Meta/MIT license) achieves ~9.0 dB SDR with a hybrid time-frequency Transformer architecture. Both produce four standard stems: vocals, drums, bass, and "other." The "other" category lumps brass with guitar, piano, synths, and strings — **standard models cannot isolate brass specifically**.

Commercial services have begun addressing this gap. **Moises** (65M+ users, Apple's 2024 iPad App of the Year) offers dedicated wind/brass instrument separation at premium tiers. **LALAL.AI** provides a dedicated "wind instruments" stem detecting trumpets, trombones, horns, and woodwinds. **Klangio Wind2Notes** can isolate and transcribe individual wind instruments from ensemble recordings.

Real-time source separation exists but with severe quality trade-offs. **HS-TasNet** (L-Acoustics, 2024) achieves **23ms algorithmic latency** but only **4.65–5.55 dB SDR** — roughly half the quality of offline models. The fundamental problem is that high-quality models like Demucs require ~7.8 seconds of lookahead (future audio context), making them architecturally incompatible with low-latency streaming.

**Isolating one specific player from an ensemble of same-family instruments — such as one trumpet from a brass section — is not feasible with any current technology.** Source separation models distinguish by instrument class, not by individual player. Two trumpets playing in unison occupy the same frequency space with the same timbre. The most practical approaches for ensemble practice remain:

- **Pre-processing recordings offline** with high-quality separation to create minus-one backing tracks by instrument family
- **Individual microphones per player** for real-time isolation (the most reliable method)
- **MIDI-based accompaniment** (Band-in-a-Box, iReal Pro) for complete control over each part
- **Beamforming with microphone arrays** combined with AI separation for spatial+spectral source isolation

Demucs does run in the browser via WebAssembly (the `free-music-demixer` project and `demucs-rs` with WebGPU), enabling client-side offline separation without server infrastructure — viable for an upload→process→practice workflow.

---

## Conclusion: a clear technical path exists for a differentiated product

The technology stack for an AI-powered brass practice companion is ready to be assembled. **Aubio provides sub-6ms pitch detection frames**, PESTO adds neural-grade accuracy at streaming speeds, Matchmaker delivers real-time score following, and OSMD renders interactive notation — all open source. The Tauri 2.0 + Rust backend architecture delivers near-native audio performance with web UI flexibility and cross-platform deployment including mobile.

The market opportunity is equally clear. Brass players currently choose between Tonestro (buggy, basic DSP), SmartMusic (institutional, pitch/rhythm only), and Modacity (no assessment) — none offering the kind of intelligent, adaptive practice feedback that piano players enjoy through apps like Flowkey or Piano Marvel. No existing app assesses tone quality, dynamics, articulation, or musical expression for any instrument. No app uses knowledge tracing or spaced repetition for instrumental practice. The online music education market is growing at over 15% annually with private lessons costing $50–100/hour.

Three capabilities would most differentiate a new entrant: **intonation analysis tuned to brass-specific tendencies** (valve combination corrections, just intonation adjustments, overtone series deviations), **adaptive practice recommendations** using performance history to suggest what to work on next, and **AI-driven tone quality assessment** — the frontier that no current app has crossed, but which QMUL and MTG Barcelona are actively researching. Offline source separation (Demucs/BS-RoFormer) could enable a "play along with minus-one" feature using real recordings rather than MIDI accompaniment, while the Matchmaker library's score following could power a scrolling notation display that stays synchronized with the performer through tempo changes and mistakes.