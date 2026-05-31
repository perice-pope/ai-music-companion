# Story — Tone-Quality Model (on-device timbre assessment): Design Proposal

**Status:** Draft — pending founder + CTO review
**Author:** Design proposal generated for review
**Target story:** *No GitHub issue yet. Suggested title: "Phase 3: Tone-Quality Model — on-device timbre descriptors." Suggested labels: `story`, `phase-3`, `ml`.*
**Phase:** 3, track 1 (recommended first — see `story-phase3-overview.md`).

**Dependencies landed:**
- **ONNX Runtime sidecar pattern (Phase 2)** — `ort` `load-dynamic`, the bundled-runtime seam (`apps/desktop/src-tauri/src/runtime.rs`, `scripts/fetch-onnxruntime.sh`, `tauri.conf.json` resource map). The tone model is a second ONNX model shipped the same way. **This is the big one — the hard infra already exists.**
- **Ears layer** — real-time capture and the analysis path that already produces `AudioEvent`s and phrase-length audio buffers (`crates/ears`).
- **Phrase aggregation** — `PhraseSummary` (`crates/brain/src/phrase.rs`) is the natural carrier for a per-phrase tone descriptor; it already anchors LLM coaching and the recap.
- **Session persistence** — `StoredSession` / `SessionStore` (`crates/brain/src/store.rs`) for trend tracking across sessions.
- **Instrument profiles** — `profiles/*.json`; tone expectations differ by family, so the profile is the natural place for per-instrument tone priors.

---

## 1. Product framing

### What it is (musician's POV)

Today the app hears *what* you played (pitch, rhythm, dynamics, stability). The tone model adds *how it sounded* — the thing a teacher reacts to before they mention a wrong note: "that was bright and a little airy," "your sound got thin as you went up," "nice warm core today." Output is a **multi-dimensional descriptor**, never a single grade:

```
{ brightness: 0.7, warmth: 0.5, air_noise: 0.1, core_clarity: 0.8, vibrato_quality: 0.6 }
```

These feed three places already built: the **live coach** (a between-phrase whisper can now reference tone), the **session recap** ("your tone was warmer today than Tuesday"), and — later — the **Teacher Dashboard** (a teacher sees tone trajectory per student).

### Why now / why it fits Phase 3 first

- **It reuses the Phase 2 ONNX infrastructure almost for free** — same `ort` `load-dynamic` runtime, same bundling seam, same offline-on-device posture. We pay the model cost, not the infra cost.
- **It's the one piece of proprietary ML in the whole product** (architecture-v2 §5) — the clearest competitive moat.
- **It's fully offline and carries no privacy/legal gate** (unlike the Teacher Dashboard), so it's unblocked.

### Three "coach, don't judge" decisions (consistent with Score Mode / Free Play)

1. **Descriptors, not a score.** We output dimensions a teacher would name, never "tone: 7/10." The UI shows them as gentle, labelled qualities.
2. **Relative to your own baseline, in your own room** — not against an abstract "perfect tone." "Warmer than Tuesday" is both more useful and more *robust to room acoustics* than an absolute rating (architecture-v2 §5 mitigation #2).
3. **Honest about limits.** Early versions work best with a consistent setup (same room, same mic). We say so, and we lean on relative tracking to stay useful despite that.

### The honest hard problem: training data + room acoustics

This is the real risk, and we name it up front (architecture-v2 §5):
- **Training data** — a learned model needs ~500–1000 annotated phrases per instrument family, rated by teachers on these dimensions. We don't have that yet. **This gates the *learned* model, not the architecture.**
- **Room acoustics** — reverb, mic, placement all colour timbre. Mitigations: a **room-calibration step**, **relative-to-baseline** tracking, and **training-data diversity / synthetic RIR augmentation**.

The slicing below is built so we ship value and a stable interface *before* the learned model exists, then swap the model in.

---

## 2. Architecture

```
phrase audio (2–10s, from Ears)
        │
        ▼
 feature extraction  ── mel spectrogram + spectral features
 (centroid, rolloff, flux, MFCC)         │
        │                                 │   room calibration profile
        ▼                                 │   (conditioning input)
   tone model  ◄──────────────────────────┘
   (ONNX, <5M params, on-device, no GPU)
        │
        ▼
 ToneDescriptor { brightness, warmth, air_noise, core_clarity, vibrato_quality }
        │
        ├──► PhraseSummary.tone (live coach + recap)
        └──► baseline tracker (relative-to-your-room deltas, persisted per instrument+room)
```

### Where the code lives

Tone analysis is audio-domain and runs **off** the real-time path (per-phrase, not per-sample), so — mirroring the `crates/transcribe` decision — a **dedicated crate `crates/tone`** keeps the second heavy ONNX dependency out of the real-time `ears` build.

```rust
// crates/tone
pub struct ToneDescriptor { /* the 5 dimensions, each 0..1 */ }

pub struct RoomProfile { /* reverb estimate, spectral coloring, noise floor */ }

/// Extract timbre features from a phrase-length mono buffer at the model rate.
pub fn features(samples: &[f32], sample_rate: u32) -> ToneFeatures;

/// Score tone (ONNX model), conditioned on the room profile.
pub fn assess(features: &ToneFeatures, room: &RoomProfile) -> Result<ToneDescriptor, ToneError>;
```

### Model

- **<5M params** CNN or small Transformer encoder (architecture-v2 §5), targeting on-device inference well under a phrase's worth of time (we have seconds, not the <25 ms real-time budget — this is *not* on the hot path).
- Shipped as a bundled ONNX resource, **identical mechanism to `nmp.onnx`** — embedded or under the `onnxruntime`-style resource seam, run via `ort`.
- **Room profile as a conditioning input** so the same tone reads consistently across rooms.

### Integration points (all already exist)

- `PhraseSummary` gains `tone: Option<ToneDescriptor>` (additive; `None` when the model/feature isn't available — same pattern as `score_position`).
- The recap and live-coach prompt builders gain tone context (they already take `PhraseSummary`).
- `StoredSession` persists tone per phrase for trend charts; the baseline tracker stores per-(instrument, room) rolling baselines.
- Instrument `profiles/*.json` gain optional tone priors (expected brightness range, vibrato expectations) — **no code change to add an instrument**, consistent with the profile philosophy.

---

## 3. The bootstrap path (how we ship before the dataset exists)

We do **not** block the whole story on collecting 1000 annotated phrases. Staged honesty:

1. **Heuristic descriptors first.** Several dimensions are *computable from DSP today* without a learned model: brightness ≈ normalised spectral centroid, air/noise ≈ high-band noise energy / harmonic-to-noise ratio, vibrato quality ≈ regularity of the f0 contour (we already track pitch). These give a real, useful (if rough) descriptor immediately and define the `ToneDescriptor` contract.
2. **Room calibration + relative tracking** make even rough descriptors useful ("warmer than your baseline").
3. **Learned model swaps in behind the same `assess()` interface** once data exists — the UI, persistence, and coaching don't change.

This means the *learned-model* gate (training data) only blocks the final slice, not the feature.

---

## 4. Testing strategy

| Test | Covers |
|---|---|
| Feature extraction on synthesised tones | A bright synthetic tone yields higher centroid/brightness than a dark one; an airy (noise-added) tone yields higher `air_noise`. Deterministic, no model. |
| Heuristic descriptors monotonicity | Adding noise raises `air_noise`; widening the spectrum raises `brightness`. |
| Room calibration | A reverb-convolved vs dry version of the same tone produces room profiles that, used as conditioning, reduce the descriptor gap (robustness check). |
| Baseline tracker | Relative deltas computed correctly across a sequence of sessions; "warmer than last time" fires on a real centroid shift. |
| ONNX model load + inference (when the learned model lands) | Gated on the runtime exactly like `crates/transcribe`'s tests (`ORT_DYLIB_PATH`, `TRANSCRIBE_REQUIRE_ORT`-style gate). |
| `PhraseSummary.tone` round-trips through `StoredSession` | Additive schema persists and reloads. |

The synthesised-tone fixtures are the crux — deterministic timbre tests in CI without shipping copyrighted/teacher-rated audio.

---

## 5. PR slicing

Target each PR <600 lines, testable and mergeable alone.

### PR 1 — `crates/tone` scaffold + feature extraction (~400 lines)
- New crate; `ToneFeatures`, mel + spectral feature extraction (pure Rust DSP, reuse `ears` FFT primitives where sensible).
- Tests: synthesised bright/dark/airy tones → expected feature ordering.
- **Merge criterion:** features are deterministic and discriminate timbre on fixtures. No model, no UI.

### PR 2 — Heuristic `ToneDescriptor` + room calibration (~450 lines)
- `assess()` with a DSP-heuristic implementation; `RoomProfile` capture + a calibration command (play a sustained tone + scale).
- `PhraseSummary.tone` field (additive) + persistence in `StoredSession`.
- Tests: descriptor monotonicity, room-robustness, schema round-trip.
- **Merge criterion:** a phrase yields a sensible descriptor; relative-to-baseline works; nothing requires a learned model.

### PR 3 — Recap + live-coach integration + UI (~400 lines)
- Tone descriptors surfaced in the recap and (optionally) a between-phrase whisper; gentle labelled UI (no traffic lights).
- Instrument-profile tone priors (optional fields).
- Tests: recap renders tone; coach prompt includes tone context; profile priors parsed.
- **Merge criterion:** "your tone was warmer today" appears in a recap from real session data.

### PR 4 — Learned ONNX model (gated on dataset) (~500 lines)
- Bundle the trained tone model; `assess()` switches to ONNX inference behind the same interface; room profile as conditioning input.
- Tests: model load + inference gated on the runtime (Phase 2 pattern); learned descriptors on a labelled holdout fixture within tolerance.
- **Merge criterion:** the learned model replaces the heuristic with no change to callers. **Blocked on training data (Open Question 1).**

---

## 6. Cut lines — NOT in this story

- **Per-note tone coloring / traffic lights** — rejected by the product principle.
- **Absolute "tone score"** — rejected; descriptors + relative tracking only.
- **Cross-student tone comparison** — that's a Teacher Dashboard concern, not here.
- **Tone for polyphonic / ensemble input** — single-line first (consistent with the single-mic ensemble decision in the decisions log).
- **Collecting/operating the annotation pipeline** — a data-ops effort tracked separately; this story consumes a dataset, it doesn't build the labeling tool.

---

## 7. Open questions for the founder

1. **Training data (the real gate).** How do we get ~500–1000 teacher-annotated phrases per instrument family? Options: recruit from a teacher network, partner with a school, pay annotators, or bootstrap from public datasets + teacher validation. *Needed before PR 4, not before PR 1–3.*
2. **Phase tag.** v2 tool-table says tone is "Phase 2." Confirm we're treating it as Phase 3 track 1 (this doc).
3. **Dimensions — are these five right** (brightness, warmth, air/noise, core clarity, vibrato quality), or does a teacher advisor want different/more axes (e.g. projection, attack quality)?
4. **Calibration UX intrusiveness.** Mandatory first-run calibration vs optional-but-encouraged? Relative tracking degrades gracefully without it, but is less robust.
5. **Retention of tone audio for training.** Do we (opt-in) keep phrase audio to grow the dataset? This is a privacy decision that should be set consistently with the Teacher Dashboard's posture.

---

**End of design doc.**
